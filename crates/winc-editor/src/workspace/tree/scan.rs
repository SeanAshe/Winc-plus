//! 在后台重读文件树里被其他程序改过的目录。
//!
//! 文件监视通知目录变了以后，界面线程上只挑出要重读哪些目录（[`Workspace::scan`]），读目录
//! 交给后台（[`DirScan::run`]），读好后再换进缓存、重新拍平（[`Workspace::apply_scan`]）：大
//! 目录读一遍要十几毫秒，网络盘上可能长时间阻塞。

use super::*;

/// 要在后台重读的目录，见 [`Workspace::scan`]。
#[derive(Debug)]
pub struct DirScan {
    root: PathBuf,
    dirs: Vec<PathBuf>,
    expanded: Vec<PathBuf>,
}

/// 后台读好的目录内容，见 [`DirScan::run`]，交给 [`Workspace::apply_scan`]。
#[derive(Debug)]
pub struct DirListing {
    root: PathBuf,
    /// 各目录读取的结果，顺序同 [`DirScan::dirs`]。
    dirs: Vec<(PathBuf, Read)>,
    /// 已展开、但已经不是目录的路径。
    gone: Vec<PathBuf>,
}

/// 读取一个目录的结果。
#[derive(Debug)]
enum Read {
    /// 读到的内容。
    Listed(Vec<DirEntry>),
    /// 目录已不存在。
    Absent,
    /// 当前线程不能等待读取（界面线程上的远端目录）：内容未知，留待后台重读。
    Blocked,
    /// 因其他原因读不出来（没有权限、连接中断）：内容未知，已有的缓存保留。
    Failed,
}

impl DirScan {
    /// 要重读的目录，按读的顺序；[`DirListing::retain`] 按同样的顺序给出。
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// 读各目录，并检查已展开的目录还是不是目录。
    ///
    /// 通常在后台线程上调用。在不能等待读取的线程上调用时，读不了的目录不当作已不存在，
    /// 由 [`Workspace::apply_scan`] 记下，留待后台重读。
    pub fn run(self) -> DirListing {
        let read = |dir: &Path| match list_dir(dir) {
            Ok(fresh) => Read::Listed(fresh),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Read::Blocked,
            Err(error) if winc_backend::fs::is_absent(&error) => Read::Absent,
            Err(_) => Read::Failed,
        };
        // 无法确定的（连接中断、不能等待）不算：只有确实不是目录或已不存在的才收起。各个展开的
        // 目录的文件信息一次取回，远端的目录只需一次往返。
        let expanded = self.expanded;
        let gone = || {
            let found = winc_backend::fs::metadata_many(&expanded);
            (expanded.iter().zip(found))
                .filter(|(_, metadata)| match metadata {
                    Ok(metadata) => !metadata.kind.is_dir(),
                    Err(error) => winc_backend::fs::is_absent(error),
                })
                .map(|(path, _)| path.clone())
                .collect()
        };
        // 远端的各个目录互不依赖，每个要等一次往返：与上面的检查同时读取。
        // 当前线程不能等待时照常逐个读，各自立即返回 `Blocked`。
        let remote = winc_backend::is_remote(&self.root) && winc_backend::remote::may_block();
        let dirs = &self.dirs;
        let list = || match remote {
            true => read_together(dirs, &read),
            false => dirs.iter().map(|dir| read(dir)).collect(),
        };
        let (gone, read): (_, Vec<Read>) = crate::background::both(remote, gone, list);
        DirListing {
            dirs: self.dirs.into_iter().zip(read).collect(),
            gone,
            root: self.root,
        }
    }
}

/// 同时等待读取的远端目录数（个）：与读取仓库的分支、储藏等各项时同时发出的请求数相当。
/// 请求在一条连接上传输，这里的线程只是各自等待应答。
const TOGETHER: usize = 8;

/// 读 `dirs` 中的各个目录，最多 [`TOGETHER`] 个同时进行，结果的顺序同 `dirs`。
fn read_together(dirs: &[PathBuf], read: &(impl Fn(&Path) -> Read + Sync)) -> Vec<Read> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let take = || {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Some(dir) = dirs.get(index) else {
                return done;
            };
            done.push((index, read(dir)));
        }
    };
    let mut done: Vec<(usize, Read)> = std::thread::scope(|scope| {
        let helpers: Vec<_> = (1..TOGETHER.min(dirs.len()))
            .map(|_| scope.spawn(take))
            .collect();
        let mut done = take();
        for helper in helpers {
            done.extend(
                helper
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e)),
            );
        }
        done
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, read)| read).collect()
}

impl DirListing {
    /// 只留下 `keep` 返回 `true` 的目录（按 [`DirScan::dirs`] 的顺序逐个询问），用来丢弃
    /// 期间又被重读过的、已经过时的目录。
    pub fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        self.dirs.retain(|(dir, _)| keep(dir));
    }
}

impl Workspace {
    /// 返回 `paths` 在磁盘上发生变化（被其他程序新建、删除或改名）以后要重读的目录：已缓存且
    /// 受影响的目录（路径所在的目录，以及路径本身是目录时的该目录）。
    ///
    /// `paths` 为 `None`（整个重新扫描）时是全部已缓存的目录。
    pub fn scan(&self, paths: Option<&[PathBuf]>) -> DirScan {
        let mut dirs: Vec<PathBuf> = match paths {
            Some(paths) => {
                let mut dirs: HashSet<&Path> = HashSet::new();
                for path in paths {
                    dirs.extend(path.parent());
                    dirs.insert(path);
                }
                dirs.into_iter()
                    .filter(|d| self.children.contains_key(*d))
                    .map(Path::to_path_buf)
                    .collect()
            }
            None => self.children.keys().cloned().collect(),
        };
        dirs.sort();
        DirScan {
            root: self.root.clone(),
            dirs,
            expanded: self.expanded.iter().cloned().collect(),
        }
    }

    /// 返回当前线程未能读取、需要交给后台读取的目录；没有时返回 `None`。
    ///
    /// 包括需要显示但尚未读取的目录（见 `cached_children`），以及重读时未能读取的目录（见
    /// [`DirScan::run`]）。同一个目录在读好之前只返回一次。读好的内容同样交给
    /// [`Workspace::apply_scan`]，换入后新露出的目录可能仍未读取，调用方应再次调用本函数。
    pub fn take_missing(&mut self) -> Option<DirScan> {
        if self.missing.wanted.is_empty() {
            return None;
        }
        let dirs = std::mem::take(&mut self.missing.wanted);
        self.missing.requested.extend(dirs.iter().cloned());
        Some(DirScan {
            root: self.root.clone(),
            dirs,
            expanded: self.expanded.iter().cloned().collect(),
        })
    }

    /// 换上后台读好的目录内容 `listing`；返回文件树是否发生了变化。
    ///
    /// 目录内容确实变化时才重新拍平，这时同时收起已经不是目录的展开项。选中项保持不变。读的是
    /// 另一个工作区（期间换了目录）时什么也不做。
    pub fn apply_scan(&mut self, listing: DirListing) -> bool {
        if listing.root != self.root {
            return false;
        }
        let mut changed = false;
        for (dir, read) in listing.dirs {
            if matches!(read, Read::Blocked) {
                self.missing.want(dir);
                continue;
            }
            let awaited = self.missing.requested.remove(&dir);
            match (self.children.get(&dir), read) {
                // 此前未能读取的目录：读到的内容（已不存在的按空目录）写入缓存。
                (None, read @ (Read::Listed(_) | Read::Absent)) if awaited => {
                    let fresh = match read {
                        Read::Listed(fresh) => fresh,
                        _ => Vec::new(),
                    };
                    self.children.insert(dir.clone(), fresh.into());
                    // 展开时没能读到它：现在补上对其下连续单子目录的展开。
                    if self.missing.chains.remove(&dir) && self.expanded.contains(&dir) {
                        self.expand_chain(dir);
                    }
                    changed = true;
                }
                // 读取失败（连接中断、没有权限）：不写入缓存，下次需要显示时再读。
                (None, Read::Failed) if awaited => {
                    self.missing.chains.remove(&dir);
                }
                // 没有缓存的目录不需要显示；读不出来的保留已有的内容。
                (None, _) | (Some(_), Read::Failed | Read::Blocked) => {}
                // 目录已不存在：其上级目录重读时不再包含它，同时丢弃它的缓存。
                (Some(_), Read::Absent) => {
                    self.children.remove(&dir);
                    changed = true;
                }
                (Some(cached), Read::Listed(fresh)) => {
                    let same = cached.len() == fresh.len()
                        && cached
                            .iter()
                            .zip(&fresh)
                            .all(|(a, b)| a.path == b.path && a.kind == b.kind);
                    if !same {
                        self.children.insert(dir, fresh.into());
                        changed = true;
                    }
                }
            }
        }

        if changed {
            for path in &listing.gone {
                self.expanded.remove(path);
            }
            self.rebuild();
        }
        // 此前要选中的路径：所在的目录读好后选中；待读的目录都已读完仍没有它时放弃。
        if let Some(path) = self.missing.reveal.take() {
            match self.row_of(&path) {
                Some(index) => self.select_segment(index, Some(&path)),
                None if self.missing.wanted.is_empty() && self.missing.requested.is_empty() => {}
                None => self.missing.reveal = Some(path),
            }
        }
        changed
    }
}
