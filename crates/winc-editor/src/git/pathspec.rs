//! 作用于一组文件的 `git` 命令的路径参数：按字面匹配，过长时分批或由标准输入传入。

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::slashed;

/// 一次 `git` 调用中路径参数的总长度上限（字节）。
///
/// Windows 的命令行最长 32767 个字符，其余部分留给 `git` 的路径和 `-c` 选项。
pub(super) const PATHS_PER_CALL: usize = 24_000;

/// 返回 `paths` 对应的 `git` 路径参数（用于 `--` 之后）：位于 `root` 之下的路径换算为相对路径，
/// 各段以 `/` 连接（[`slashed`]）。
///
/// 每个都加上 `:(literal)`，按字面匹配：`app/[slug]/page.tsx` 里的 `[`、`*`、`?` 不当通配符，
/// 回滚、提交时不会连带改动名字碰巧匹配上的其他文件。不用 `GIT_LITERAL_PATHSPECS`：
/// `git stash push -u` 自己也要用路径匹配，开启它以后不再收录未跟踪的文件。
pub(super) fn relative_all<'a>(
    root: &'a Path,
    paths: &'a [PathBuf],
) -> impl Iterator<Item = OsString> + 'a {
    paths.iter().map(move |p| {
        let mut spec = OsString::from(":(literal)");
        match p.strip_prefix(root) {
            Ok(relative) => spec.push(slashed(relative)),
            Err(_) => spec.push(p),
        }
        spec
    })
}

/// 把 `paths` 的路径参数（[`relative_all`]）按顺序分成几批，每批的总长度不超过
/// [`PATHS_PER_CALL`]；没有路径时返回一个空批。
pub(super) fn path_batches(root: &Path, paths: &[PathBuf]) -> Vec<Vec<OsString>> {
    let mut batches = vec![Vec::new()];
    let mut length = 0;
    for spec in relative_all(root, paths) {
        // 每个参数另有分隔的空格和两侧的引号。
        let size = spec.len() + 3;
        if length + size > PATHS_PER_CALL && batches.last().is_some_and(|b| !b.is_empty()) {
            batches.push(Vec::new());
            length = 0;
        }
        length += size;
        if let Some(batch) = batches.last_mut() {
            batch.push(spec);
        }
    }
    batches
}

/// 把 `chosen` 中整个选中的目录合并成一项：某个目录下有变化的路径（`changed`）全部在
/// `chosen` 中时，以该目录代替其中的各个路径，取最上层的这种目录，`root` 本身除外。
///
/// 路径都是绝对路径；结果按 `chosen` 原来的先后，不重复。
pub(super) fn merge_folders(root: &Path, chosen: &[PathBuf], changed: &[PathBuf]) -> Vec<PathBuf> {
    let dirs = |path: &Path| -> Vec<PathBuf> {
        (path.ancestors().skip(1))
            .take_while(|d| *d != root && d.starts_with(root))
            .map(Path::to_path_buf)
            .collect()
    };
    let picked: HashSet<&Path> = chosen.iter().map(PathBuf::as_path).collect();
    // 各目录下有变化的路径数，和其中选中的路径数。
    let mut counts: HashMap<PathBuf, (usize, usize)> = HashMap::new();
    for path in changed {
        let hit = usize::from(picked.contains(path.as_path()));
        for dir in dirs(path) {
            let count = counts.entry(dir).or_default();
            count.0 += 1;
            count.1 += hit;
        }
    }
    let full = |dir: &PathBuf| counts.get(dir).is_some_and(|&(all, hit)| all == hit);
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for path in chosen {
        let top = dirs(path).into_iter().rfind(full);
        let path = top.unwrap_or_else(|| path.clone());
        if seen.insert(path.clone()) {
            merged.push(path);
        }
    }
    merged
}

/// 返回 `specs` 以 NUL 分隔的字节，供 `--pathspec-from-file=- --pathspec-file-nul` 从标准输入读取。
pub(super) fn nul_separated(specs: &[OsString]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for spec in specs {
        #[cfg(unix)]
        bytes.extend_from_slice(std::os::unix::ffi::OsStrExt::as_bytes(spec.as_os_str()));
        #[cfg(not(unix))]
        bytes.extend_from_slice(spec.to_string_lossy().as_bytes());
        bytes.push(0);
    }
    bytes
}
