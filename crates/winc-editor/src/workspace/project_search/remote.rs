//! 远端主机上的目录：把条件交给远端的服务进程，由它遍历、读文件并匹配，这里只收取结果。
//!
//! 文件不经连接传回本机。远端与本机用同一套规则（`winc_search`）匹配、解码和分行，搜出的位置
//! 与打开后的文本一致；有未保存修改的文件由远端报告遍历到了，这里按内存中的内容搜索。

use std::io::{self, BufReader};
use std::sync::mpsc::Sender;

use winc_backend::remote::{self, Exec};
use winc_backend::{Location, Target};
use winc_search::host::{self, Mode, Record, Request};

use super::*;

/// 交给一台远端主机的一次搜索。
pub(super) struct Remote {
    /// 在远端运行搜索的目录，也用来找到连接。
    at: PathBuf,
    target: Target,
    request: Request,
    /// `request.skip` 中各文件在本机的路径（有未保存修改的文档）。
    skipped: Vec<PathBuf>,
}

impl Remote {
    /// 改为只搜索目录下的依赖目录。
    pub(super) fn dependencies(mut self) -> Self {
        self.request.mode = Mode::Dependencies;
        self
    }
}

impl Walk {
    /// 返回遍历远端目录 `dirs` 的搜索，每个目录一个；`ignored` 为是否也搜索被忽略的文件。
    ///
    /// 「包含」「排除」相对于工作区的根目录；根目录不在同一台主机上时相对于这个目录。
    pub(super) fn tree_requests(&self, dirs: &[PathBuf], ignored: bool) -> Vec<Remote> {
        let root = winc_backend::locate(&self.root);
        (dirs.iter())
            .filter_map(|dir| {
                let at = winc_backend::locate(dir)?;
                let base = match &root {
                    Some(root) if root.target == at.target => root.path.clone(),
                    _ => at.path.clone(),
                };
                let paths = vec![at.path.clone()];
                Some(self.remote(dir.clone(), &at, Mode::Tree { ignored }, base, paths))
            })
            .collect()
    }

    /// 返回重新搜索远端文件 `files` 的搜索，每台主机一个。
    pub(super) fn file_requests(&self, files: Vec<PathBuf>) -> Vec<Remote> {
        let mut hosts: Vec<(Location, Vec<Vec<u8>>)> = Vec::new();
        for file in &files {
            let Some(at) = winc_backend::locate(file) else {
                continue;
            };
            match hosts.iter_mut().find(|(host, _)| host.target == at.target) {
                Some((_, paths)) => paths.push(at.path),
                None => hosts.push((at.clone(), vec![at.path])),
            }
        }
        (hosts.into_iter())
            .map(|(at, paths)| {
                let dir = winc_backend::virtual_path(&at.target, b"/");
                self.remote(dir, &at, Mode::Files, b"/".to_vec(), paths)
            })
            .collect()
    }

    /// 构造在 `at` 所在的主机上搜索 `paths` 的请求；匹配数的上限在发出时再定。
    fn remote(
        &self,
        dir: PathBuf,
        at: &Location,
        mode: Mode,
        root: Vec<u8>,
        paths: Vec<Vec<u8>>,
    ) -> Remote {
        let (skip, skipped) = (self.dirty.keys())
            .filter_map(|path| {
                let place = winc_backend::locate(path)?;
                (place.target == at.target).then(|| (place.path, path.clone()))
            })
            .unzip();
        Remote {
            at: dir,
            target: at.target.clone(),
            request: Request {
                query: self.query.clone(),
                region: winc_search::text::region().map(str::to_owned),
                limit: MAX_RESULTS,
                mode,
                root,
                paths,
                skip,
            },
            skipped,
        }
    }

    /// 在远端搜索并把结果送给界面线程；远端没能搜完时记下结果不全。
    pub(super) fn search_remote(&self, remote: &Remote, sender: &Sender<Message>) {
        let Some(limit) = self.run.budget.remaining() else {
            return;
        };
        if self.run.budget.stopped() {
            return;
        }
        let searched = self.receive(remote, limit, sender);
        if searched.is_err() && !self.run.budget.stopped() {
            self.run.remote_failed.store(true, Relaxed);
        }
    }

    /// 发出请求，逐条收取远端送回的结果，直到远端报告搜完。
    ///
    /// # Errors
    ///
    /// 无法启动远端的搜索、连接中断、远端出错或送回的数据不合格式时返回对应的 `io::Error`。
    fn receive(&self, remote: &Remote, limit: usize, sender: &Sender<Message>) -> io::Result<()> {
        let request = Request {
            limit,
            ..remote.request.clone()
        };
        let exec = Exec::this_server()
            .arg(host::SEARCH)
            .stdin(request.encode());
        let running = remote::spawn(&remote.at, &exec)?;
        self.run.watch(running.stopper());
        let mut input = BufReader::new(running);
        loop {
            let found = match host::read_record(&mut input)? {
                // 没有送回结束的一条就断了：远端出错或被结束。
                None => return Err(io::ErrorKind::UnexpectedEof.into()),
                Some(Record::End { truncated, gave_up }) => {
                    if truncated {
                        self.run.budget.mark_truncated();
                    }
                    if gave_up {
                        self.matcher.mark_gave_up();
                    }
                    return Ok(());
                }
                Some(Record::File {
                    path,
                    matches,
                    lossy,
                }) => Some((
                    winc_backend::virtual_path(&remote.target, &path),
                    matches,
                    lossy,
                )),
                Some(Record::Skipped(index)) => {
                    let path = remote
                        .skipped
                        .get(index)
                        .ok_or(io::ErrorKind::InvalidData)?;
                    (self.dirty.get(path))
                        .and_then(|rope| Searcher::new(&self.matcher, &self.run.budget).text(rope))
                        .map(|matches| (path.clone(), matches, false))
                }
            };
            if let Some((path, matches, lossy)) = found
                && self.emit(path, matches, lossy, sender).is_break()
            {
                self.run.stop();
                return Ok(());
            }
        }
    }
}
