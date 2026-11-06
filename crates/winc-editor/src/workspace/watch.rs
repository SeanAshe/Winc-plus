//! 监视打开的目录：其他程序在目录中新建、删除、重命名或改写文件时，系统发出通知（macOS 为
//! FSEvents，Linux 为 inotify，Windows 为 ReadDirectoryChangesW，均经由 `notify` crate 接入）。
//!
//! 文件树、⌘P 的文件表和已打开的文档据此按路径各自更新，不必重新遍历整个目录。收集在 `notify`
//! 自己的线程上进行，第一条变化到达时唤醒界面线程一次，界面线程稍后用 [`Watcher::take`] 一次取走
//! 全部变化，因此密集改动只处理一次。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::workspace::tree::HIDDEN;

/// 一批变化最多记录的路径数（个）。
///
/// 超过该数量时不再逐个记录，改为重新扫描整个目录（切换分支、`npm install` 等一次改动成千上万个
/// 文件的情况）。该值为经验值。
const MAX_PATHS: usize = 4096;

/// 一批累积的文件系统变化。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// 发生变化的路径（绝对路径），对应的文件可能已被删除。
    pub paths: Vec<PathBuf>,
    /// 是否需要重新扫描整个目录：系统报告通知丢失、监视出错，或变化的路径超过 `MAX_PATHS` 个。
    ///
    /// 为 `true` 时 `paths` 不完整（此后的路径不再记录），调用方应当整个重新扫描，不依赖 `paths`。
    pub rescan: bool,
    /// 某个 `.git` 里的 `HEAD`、`index`、`packed-refs` 或 `refs/` 是否变了：提交、暂存、
    /// 切换分支等操作，Git 模块据此刷新。
    ///
    /// 这些路径不在 `paths` 里。
    pub git: bool,
}

impl Changes {
    /// 是否没有任何变化：没有路径，`.git` 没变，也不需要重新扫描。
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && !self.rescan && !self.git
    }
}

#[derive(Default)]
struct Pending {
    paths: HashSet<PathBuf>,
    rescan: bool,
    git: bool,
    /// 在打开的目录之外另外监视的公共 Git 目录。
    git_dir: Option<PathBuf>,
    /// 收集到第一条变化后是否已唤醒过界面线程；在 [`Watcher::take`] 取走之前不再重复唤醒。
    woken: bool,
}

/// 目录监视器：后台线程累积变化，界面线程一次取走。
///
/// 释放（drop）`Watcher` 即停止监视。
pub struct Watcher {
    root: PathBuf,
    pending: Arc<Mutex<Pending>>,
    /// 底层的系统监视器，释放时停止监视。
    inner: RecommendedWatcher,
}

impl Watcher {
    /// 开始递归监视 `root` 及其所有子目录。
    ///
    /// 有变化时在 `notify` 的线程上调用 `wake`，同一批变化只调用一次。
    ///
    /// # Errors
    ///
    /// 系统无法创建监视器或无法监视该目录时返回 `notify::Error`。
    pub fn start(root: &Path, wake: impl Fn() + Send + 'static) -> notify::Result<Watcher> {
        let pending = Arc::new(Mutex::new(Pending::default()));
        let shared = Arc::clone(&pending);
        let base = root.to_path_buf();

        let mut inner =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let mut pending = shared.lock().unwrap_or_else(PoisonError::into_inner);
                match event {
                    Ok(event) if event.need_rescan() => pending.rescan = true,
                    // 打开、读取文件不算变化。
                    Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
                    Ok(event) => {
                        // 已经要重新扫描时不再记录路径（反正用不上），只看 `.git` 是否变化。
                        for path in event.paths {
                            if git_state(&path) {
                                pending.git = true;
                            } else if (pending.git_dir.as_ref())
                                .is_some_and(|dir| path.starts_with(dir))
                            {
                                // 公共 Git 目录里的其他文件不是打开的目录里的东西。
                            } else if !pending.rescan && !hidden(&base, &path) {
                                pending.paths.insert(path);
                            }
                        }
                        if pending.paths.len() > MAX_PATHS {
                            pending.paths.clear();
                            pending.rescan = true;
                        }
                    }
                    Err(_) => pending.rescan = true,
                }
                if !pending.woken && (pending.rescan || pending.git || !pending.paths.is_empty()) {
                    pending.woken = true;
                    drop(pending);
                    wake();
                }
            })?;
        inner.watch(root, RecursiveMode::Recursive)?;
        Ok(Watcher {
            root: root.to_path_buf(),
            pending,
            inner,
        })
    }

    /// 让监视跟上当前仓库的公共 Git 目录 `dir`：不在打开的目录里时（附属工作树、打开的是仓库的
    /// 子目录）另外监视它，提交、分支这些状态都记在那里。监视不了时打开的目录照常监视。
    pub fn watch_git_dir(&mut self, dir: Option<&Path>) {
        let dir = dir.filter(|dir| !dir.starts_with(&self.root));
        // 系统监视器改动时要等监视线程回复，而监视线程送通知时要拿这把锁：改动之前先放开，否则两边
        // 互相等待。
        let old = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.git_dir.as_deref() == dir {
                return;
            }
            std::mem::replace(&mut pending.git_dir, dir.map(Path::to_path_buf))
        };
        if let Some(old) = old {
            let _ = self.inner.unwatch(&old);
        }
        if let Some(dir) = dir
            && self.inner.watch(dir, RecursiveMode::Recursive).is_err()
        {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending.git_dir = None;
        }
    }

    /// 返回被监视的根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 取走并清空累积的变化，其中的 `paths` 已排序。
    pub fn take(&self) -> Changes {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let mut paths: Vec<PathBuf> = pending.paths.drain().collect();
        paths.sort();
        let rescan = std::mem::take(&mut pending.rescan);
        let git = std::mem::take(&mut pending.git);
        pending.woken = false;
        Changes { paths, rescan, git }
    }
}

/// `path` 是不是某个 `.git` 目录（打开的目录里可能有多个仓库）里记录仓库状态的文件：`HEAD`、
/// `index`、`packed-refs`、`refs/` 下的文件，以及 `worktrees/<名字>/` 下各工作树自己的 `HEAD`、
/// `index`。
///
/// `index.lock` 这类临时文件不算，操作完成时正式的文件会再变一次。
fn git_state(path: &Path) -> bool {
    let Some(git_dir) = path.ancestors().find(|dir| dir.ends_with(".git")) else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(git_dir) else {
        return false;
    };
    let mut parts = relative.components().map(|c| c.as_os_str());
    match parts.next().and_then(|c| c.to_str()) {
        Some("HEAD" | "index" | "packed-refs") => parts.next().is_none(),
        Some("worktrees") => {
            let file = parts.nth(1).and_then(|c| c.to_str());
            matches!(file, Some("HEAD" | "index")) && parts.next().is_none()
        }
        Some("refs") => !relative.to_string_lossy().ends_with(".lock"),
        _ => false,
    }
}

/// `path` 是否位于文件树、⌘P 和全局搜索都忽略的目录中（如 `.git`，见 `HIDDEN`）。
///
/// 版本库操作会在这些目录中一次改动大量文件，因此在收集阶段直接丢弃。
fn hidden(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative
            .components()
            .any(|c| HIDDEN.contains(&c.as_os_str().to_string_lossy().as_ref()))
    })
}

#[cfg(test)]
mod tests;
