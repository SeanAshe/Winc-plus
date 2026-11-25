//! 遍历工作区目录：哪些文件收录、按什么顺序排列，以及依赖目录在哪里。
//!
//! 文件树、快速打开的文件索引和搜索共用这里的规则，看到的文件相同；本机与远端主机上的服务进程
//! 也共用，远端的目录在远端遍历。

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use ignore::WalkBuilder;

/// 文件树中不显示的目录和文件名（按名字精确匹配），快速打开的文件索引同样跳过。
pub const HIDDEN: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    ".idea",
    ".DS_Store",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
];

/// 返回遍历工作区目录的配置，快速打开的文件索引和全局搜索共用，两者看到的文件相同。
///
/// 遵守各级 `.gitignore`、`.ignore` 和全局的忽略规则，不进入指向目录的符号链接，跳过
/// [`HIDDEN`] 中的名字；指向文件的符号链接是否收录由 [`is_listed_file`] 判断。
///
/// 在版本库内时 `.gitignore` 只到版本库的根目录为止，与 Git 相同，上级目录的规则不影响版本库。
/// 不在版本库内时 `.gitignore` 同样生效：下载的源码包也不会列出 `node_modules` 这类目录。
pub fn walker(dir: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(dir);
    // `hidden(false)` 关闭 `ignore` 对以 `.` 开头的名字的过滤：`.github`、`.editorconfig`
    // 仍然收录，只有 [`HIDDEN`] 中的名字由 `filter_entry` 跳过。
    builder
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(in_repository(dir))
        .follow_links(false)
        .filter_entry(|entry| !HIDDEN.contains(&entry.file_name().to_string_lossy().as_ref()));
    builder
}

/// 同 [`walker`]，但不遵守忽略规则，只跳过 [`HIDDEN`] 中的名字。
pub fn walker_with_ignored(dir: &Path) -> WalkBuilder {
    let mut builder = walker(dir);
    builder
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);
    builder
}

/// `dir` 或它的某一级上级目录是否是版本库的根目录。
fn in_repository(dir: &Path) -> bool {
    dir.ancestors().any(is_repository)
}

/// 目录 `dir` 本身是否是版本库的根目录：其中有 `.git` 或 `.jj`。
pub fn is_repository(dir: &Path) -> bool {
    winc_backend::fs::exists(dir.join(".git")) || winc_backend::fs::exists(dir.join(".jj"))
}

/// 逐个目录列出时更换忽略规则匹配器的间隔（个）：每列出该数量的目录就换一个新的匹配器。
///
/// 取自调参：匹配器缓存走过的每个目录的规则，列出家目录这样的大目录时能占一百多 MB；按该间隔
/// 换掉后释放，重新读取规则的开销可以忽略。
pub const MATCHER_DIRS: usize = 1024;

/// 返回 `dir` 之下的嵌套版本库，按路径排序：被上级目录的忽略规则整个略过、自身又是版本库根目录
/// 的目录。
///
/// 上级版本库的忽略规则止于嵌套版本库的根目录，这样的目录按自身的忽略规则收录：
/// 快速打开和搜索视其为另一个根目录。只列出不被忽略的目录，不进入被忽略的目录，也不读文件；
/// 嵌套版本库之中再嵌套的一并找出。`cancel` 置位时提前返回，结果不全。
pub fn nested_repositories(dir: &Path, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut roots = vec![dir.to_path_buf()];
    while let Some(root) = roots.pop() {
        let fresh = || walker(&root).build_matchers().into_iter().next();
        let (mut matcher, mut listed) = (fresh(), 0usize);
        let mut frontier = vec![String::new()];
        while let Some(rel) = frontier.pop() {
            if cancel.load(Relaxed) {
                return found;
            }
            listed += 1;
            if listed.is_multiple_of(MATCHER_DIRS) {
                matcher = fresh();
            }
            let Ok(entries) = winc_backend::fs::read_dir(root.join(&rel)) else {
                continue;
            };
            for entry in entries.flatten() {
                let Some(name) = entry.name.to_str().filter(|n| !HIDDEN.contains(n)) else {
                    continue;
                };
                // 指向目录的符号链接不进入，与遍历相同。
                if !entry.kind.is_dir() {
                    continue;
                }
                let child = match rel.is_empty() {
                    true => name.to_owned(),
                    false => format!("{rel}/{name}"),
                };
                let ignored =
                    (matcher.as_mut()).is_some_and(|m| m.matched(&child, true).is_ignore());
                if !ignored {
                    frontier.push(child);
                } else if is_repository(&root.join(&child)) {
                    found.push(root.join(&child));
                    roots.push(root.join(&child));
                }
            }
        }
    }
    found.sort();
    found
}

/// 判断类型为 `kind` 的目录项 `path` 是否作为文件收录：普通文件，或指向普通文件的符号链接。
///
/// 只有符号链接才多做一次 stat。指向目录的符号链接不收录、不进入，避免链接成环时无限遍历，
/// 以及同一棵目录被重复收录。
pub fn is_listed_file(kind: impl Into<winc_backend::fs::Kind>, path: &Path) -> bool {
    let kind = kind.into();
    kind.is_file() || kind.is_symlink() && winc_backend::fs::is_file(path)
}

/// 一个变化了的路径现在的样子，见 [`probe`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// 已不存在。
    Missing,
    /// 存在，但不收录：被忽略、在 [`HIDDEN`] 中，或不是文件也不是目录。
    Unlisted,
    /// 收录的文件。
    File,
    /// 目录：其下收录的文件和遍历过的目录（相对于根目录，以 `/` 分隔，含它自己）。
    Dir {
        /// 其下收录的文件。
        files: Vec<String>,
        /// 遍历过的目录。
        dirs: Vec<String>,
    },
    /// 在前面刚收录过的目录之下，不必再看。
    Covered,
}

/// 查看根目录 `root` 下变化了的各个路径（相对路径，以 `/` 分隔，按 [`Path`] 的顺序排好）现在的
/// 样子；`layers` 是收录的层数（从 1 起，为 `None` 时不限），目录只收录到这一层。
///
/// 是否收录按所在目录的忽略规则，与遍历时相同；空的相对路径是根目录本身。`cancel` 置位时提前
/// 返回，结果不全。
pub fn probe(
    root: &Path,
    paths: &[String],
    layers: Option<usize>,
    cancel: &AtomicBool,
) -> Vec<Probe> {
    // 各目录下收录的名字，同一目录里的多个变化只列一次。
    let mut listed: HashMap<&str, HashSet<String>> = HashMap::new();
    // 上一个收录的目录：路径排好了序，其下的路径紧随其后。
    let mut gathered: Option<&str> = None;
    let mut found = Vec::with_capacity(paths.len());
    for rel in paths {
        if cancel.load(Relaxed) {
            break;
        }
        let covered = gathered.is_some_and(|dir| {
            dir.is_empty() || rel.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
        });
        if covered {
            found.push(Probe::Covered);
            continue;
        }
        let path = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        let Ok(metadata) = winc_backend::fs::symlink_metadata(&path) else {
            found.push(Probe::Missing);
            continue;
        };
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let allowed = rel.is_empty()
            || listed
                .entry(parent)
                .or_insert_with(|| listed_names(&root.join(parent)))
                .contains(name);
        let kind = metadata.kind;
        found.push(if !allowed {
            Probe::Unlisted
        } else if is_listed_file(kind, &path) {
            Probe::File
        } else if kind.is_dir() {
            let (mut files, mut dirs) = (Vec::new(), Vec::new());
            gather(root, rel, layers, (&mut files, &mut dirs), cancel);
            gathered = Some(rel);
            Probe::Dir { files, dirs }
        } else {
            Probe::Unlisted
        });
    }
    found
}

/// 返回目录 `dir` 下收录的直接子项的名字。
fn listed_names(dir: &Path) -> HashSet<String> {
    walker(dir)
        .max_depth(Some(1))
        .build()
        .flatten()
        .filter(|entry| entry.depth() == 1)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// 收录 `dir`（相对路径，空串是根目录）下面的文件和遍历过的目录，最多到第 `layers` 层（为
/// `None` 时不限）；`dir` 自己也算遍历过的目录（根目录除外）。
///
/// `dir` 所在的层数是其路径段数（从 1 起），已到第 `layers` 层时什么也不收录。名字不是有效
/// UTF-8 的项不收录。`cancel` 置位时提前返回。
pub fn gather(
    root: &Path,
    dir: &str,
    layers: Option<usize>,
    (files, dirs): (&mut Vec<String>, &mut Vec<String>),
    cancel: &AtomicBool,
) {
    let depth = if dir.is_empty() {
        0
    } else {
        dir.split('/').count()
    };
    if layers.is_some_and(|layers| depth >= layers) {
        return;
    }
    if !dir.is_empty() {
        dirs.push(dir.to_owned());
    }
    let base = if dir.is_empty() {
        root.to_path_buf()
    } else {
        root.join(dir)
    };
    let walk = walker(&base)
        .max_depth(layers.map(|layers| layers - depth))
        .build();
    for entry in walk.flatten() {
        if cancel.load(Relaxed) {
            return;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(path) = entry.path().strip_prefix(root).ok().and_then(slashed) else {
            continue;
        };
        if is_listed_file(kind, entry.path()) {
            files.push(path);
        } else if kind.is_dir() && layers.is_none_or(|layers| depth + entry.depth() < layers) {
            dirs.push(path);
        }
    }
}

/// 把相对路径写成以 `/` 分隔的字符串；含非 UTF-8 的部分或路径为空时返回 `None`。
pub fn slashed(relative: &Path) -> Option<String> {
    let mut out = String::new();
    for part in relative.components() {
        let text = part.as_os_str().to_str()?;
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(text);
    }
    (!out.is_empty()).then_some(out)
}

/// 返回递归展开目录 `path` 时要展开的各级目录（含它自己，按名字逐层排序），以及是否因超过上限
/// 只找了一部分：最多 `dirs` 个目录、看 `entries` 个目录项。
///
/// 遵守忽略规则，不进入符号链接；`path` 本身被忽略时不往下找，返回空。
pub fn descend(path: &Path, dirs: usize, entries: usize) -> (Vec<PathBuf>, bool) {
    let listed = path.parent().is_some_and(|parent| {
        walker(parent)
            .max_depth(Some(1))
            .build()
            .filter_map(Result::ok)
            .any(|e| e.path() == path)
    });
    if !listed {
        return (Vec::new(), false);
    }

    let mut walk = walker(path);
    walk.sort_by_file_name(|a, b| a.cmp(b));
    let (mut found, mut seen) = (Vec::new(), 0);
    for item in walk.build().filter_map(Result::ok) {
        seen += 1;
        let dir = item.file_type().is_some_and(|kind| kind.is_dir());
        if seen > entries || (dir && found.len() == dirs) {
            return (found, true);
        }
        if dir {
            found.push(item.into_path());
        }
    }
    (found, false)
}

/// 依赖目录的名字：Node 的、Python 虚拟环境的、PHP 和 Go 的 `vendor`、iOS 的 `Pods`。
pub const DEPENDENCY_FOLDERS: [&str; 5] = ["node_modules", ".venv", "venv", "vendor", "Pods"];
/// 往下找依赖目录的层数：根目录下一层和两层（如 `frontend/node_modules`）。
const DEPENDENCY_DEPTH: usize = 2;

/// 返回 `dir` 下两层以内（如 `frontend/node_modules`）的依赖目录，按路径排序；不进依赖目录和隐藏的目录
/// （[`HIDDEN`]）里找，不跟随符号链接。
pub fn dependency_folders(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..DEPENDENCY_DEPTH {
        let mut next = Vec::new();
        for parent in &level {
            let entries = winc_backend::fs::read_dir(parent)
                .into_iter()
                .flatten()
                .flatten();
            for entry in entries.filter(|entry| entry.kind.is_dir()) {
                let path = entry.path();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                match name.as_deref() {
                    Some(name) if DEPENDENCY_FOLDERS.contains(&name) => found.push(path),
                    Some(name) if HIDDEN.contains(&name) => {}
                    Some(_) => next.push(path),
                    None => {}
                }
            }
        }
        level = next;
    }
    found.sort();
    found
}

/// 按自然顺序比较两个名字：连续数字按数值比较，其余字符不区分大小写，因此 `file2` 排在
/// `file10` 之前。
///
/// 这是全序，只有两个名字完全相同时才返回 `Equal`：按上述规则相同时，先比较第一处数值相同、
/// 位数不同的数字（位数少的在前，`file1` 排在 `file01` 之前），再按原样逐字节比较（`A.rs`
/// 排在 `a.rs` 之前）。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (whole_a, whole_b) = (a, b);
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    // 第一处数值相同、位数不同的数字的比较结果，规则相同时据此区分。
    let mut digits = Ordering::Equal;

    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return digits.then_with(|| whole_a.cmp(whole_b)),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let ((x, x_len), (y, y_len)) = (take_number(&mut a), take_number(&mut b));
                match x.cmp(&y) {
                    Ordering::Equal => digits = digits.then(x_len.cmp(&y_len)),
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                match x.to_lowercase().cmp(y.to_lowercase()) {
                    Ordering::Equal => {}
                    other => return other,
                }

                a.next();
                b.next();
            }
        }
    }
}

/// 读取 `chars` 开头的连续数字并前移迭代器，返回其数值（超过 `u128` 的范围时饱和）和位数。
fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (u128, usize) {
    let (mut n, mut len): (u128, usize) = (0, 0);

    while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(d as u128);
        len += 1;
        chars.next();
    }

    (n, len)
}

#[cfg(test)]
mod tests;
