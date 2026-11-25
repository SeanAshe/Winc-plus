//! 查找仓库：目录所在的仓库、目录下面的仓库，以及读状态时夹在未跟踪的文件里的嵌套仓库。
//!
//! 只查文件系统（各目录的 `.git`），不运行 `git`；远端的目录经连接成批查询。

use std::path::{Path, PathBuf};

use super::Entry;
use crate::tools::shell::process::Cancel;

/// [`scan_repositories`] 最多查看的目录项数（个）：在后台运行，比 [`SCAN_ENTRIES`] 宽松，又不至于
/// 在放着海量文件的目录里长时间读磁盘。
const SCAN_DEEP_ENTRIES: usize = 65_536;
/// [`scan_repositories`] 不进入的目录：依赖和构建产物，里面不会有用户自己的仓库，文件却很多。
const SCAN_SKIPPED: [&str; 4] = ["node_modules", "target", "vendor", "__pycache__"];
/// [`find_repositories`] 最多查看的目录项数（个，取自调参）：放着大量文件的目录不拖慢打开。
const SCAN_ENTRIES: usize = 4096;

/// 从 `dir` 往上找仓库的根目录：第一个含有 `.git`（目录，或工作树和子模块里的文件）的目录。
///
/// 只查文件系统，不运行 `git`。
pub fn find_root(dir: &Path) -> Option<PathBuf> {
    // 远端的目录只在那台主机的根目录以内向上查找：再往上就是本机的文件系统。
    let remote = winc_backend::is_remote(dir);
    let ancestors: Vec<&Path> = dir
        .ancestors()
        .take_while(|d| !remote || winc_backend::is_remote(d))
        .collect();
    // 各级的 `.git` 一次查完：远端的目录只需一次往返。
    let markers: Vec<PathBuf> = ancestors.iter().map(|d| d.join(".git")).collect();
    let found = winc_backend::fs::metadata_many(&markers);
    (ancestors.into_iter().zip(found))
        .find(|(_, metadata)| metadata.is_ok())
        .map(|(dir, _)| dir.to_path_buf())
}

/// 返回打开目录 `dir` 时可用的仓库：`dir` 所在的仓库（如果有，排在最前面），之后是第一层子目录里的
/// 仓库，按名字排序。
///
/// 只查文件系统，不运行 `git`。目录项很多时只看前一部分（`SCAN_ENTRIES`）。
pub fn find_repositories(dir: &Path) -> Vec<PathBuf> {
    // 只有目录（和可能指向目录的符号链接）才可能是仓库；各项的 `.git` 一次查完。
    let candidates: Vec<PathBuf> = winc_backend::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .take(SCAN_ENTRIES)
        .filter(|entry| entry.kind.is_dir() || entry.kind.is_symlink())
        .map(|entry| entry.path())
        .collect();
    let markers: Vec<PathBuf> = candidates.iter().map(|path| path.join(".git")).collect();
    let found = winc_backend::fs::metadata_many(&markers);
    let mut children: Vec<PathBuf> = (candidates.into_iter().zip(found))
        .filter(|(_, metadata)| metadata.is_ok())
        .map(|(path, _)| path)
        .collect();
    children.sort_by_cached_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
    });
    find_root(dir).into_iter().chain(children).collect()
}

/// 在 `dir` 下面最多 `depth` 层里查找仓库，按路径排序，不含 `dir` 本身。
///
/// 按层往下找，较浅的目录优先用完额度（`SCAN_DEEP_ENTRIES`）。找到仓库后继续查找其中的目录，
/// 嵌套的仓库（不论是否为子模块）同样找到；跳过隐藏目录（含 `.git`）、依赖和构建产物目录
/// （`SCAN_SKIPPED`），不跟随符号链接。取消时返回已找到的部分。
pub fn scan_repositories(dir: &Path, depth: usize, cancel: &Cancel) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![dir.to_path_buf()];
    let mut budget = SCAN_DEEP_ENTRIES;
    for _ in 0..depth {
        let mut next = Vec::new();
        for parent in &level {
            if cancel.is_cancelled() || budget == 0 {
                found.sort();
                return found;
            }
            let Ok(entries) = winc_backend::fs::read_dir(parent) else {
                continue;
            };
            let mut candidates = Vec::new();
            for entry in entries.flatten().take(budget) {
                budget -= 1;
                let name = entry.name.to_string_lossy();
                let skipped = name.starts_with('.') || SCAN_SKIPPED.contains(&name.as_ref());
                if !skipped && entry.kind.is_dir() {
                    candidates.push(entry.path());
                }
            }
            // 这一个目录下各项的 `.git` 一次查完。
            let markers: Vec<PathBuf> = candidates.iter().map(|path| path.join(".git")).collect();
            let repositories = winc_backend::fs::metadata_many(&markers);
            for (path, repository) in candidates.into_iter().zip(repositories) {
                if repository.is_ok() {
                    found.push(path.clone());
                }
                next.push(path);
            }
        }
        level = next;
    }
    found.sort();
    found
}

/// 从 `entries` 中挑出嵌套的仓库并移除：未跟踪的目录中有 `.git` 时，`git status` 把该仓库当作
/// 一个未跟踪的目录列出（不进入其中）。返回这些仓库的根目录，按原来的顺序。
///
/// 只查未跟踪的目录，各项的 `.git` 一次查完：远端的仓库只需一次往返。在后台读取状态时调用。
pub fn nested_repositories(entries: &mut Vec<Entry>) -> Vec<PathBuf> {
    let dirs: Vec<usize> = (entries.iter().enumerate())
        .filter(|(_, e)| e.is_dir && e.untracked())
        .map(|(i, _)| i)
        .collect();
    if dirs.is_empty() {
        return Vec::new();
    }
    let markers: Vec<PathBuf> = dirs.iter().map(|&i| entries[i].path.join(".git")).collect();
    let found = winc_backend::fs::metadata_many(&markers);
    let nested: Vec<usize> = (dirs.into_iter().zip(found))
        .filter(|(_, metadata)| metadata.is_ok())
        .map(|(i, _)| i)
        .collect();
    let roots = nested.iter().map(|&i| entries[i].path.clone()).collect();
    let mut index = 0;
    entries.retain(|_| {
        let keep = !nested.contains(&index);
        index += 1;
        keep
    });
    roots
}
