//! 收集工作区的文件：整体遍历，以及按目录中的变化增量更新。

use super::*;

/// 遍历时更换忽略规则匹配器的目录间隔（个）：每列出该数量的目录就换一个新的匹配器。
///
/// 依据见 [`collect`] 的说明。
const MATCHER_DIRS: usize = 1024;

/// 遍历 `root`，收齐文件，排序后放进文件表。
///
/// 按层（广度优先）遍历：文件超过 `limit` 个时只收录浅层，整层整层地收，收录结果只取决于目录
/// 内容。不用深度优先：它要先走完一棵很深的子树，才知道浅层放不放得下。忽略规则与全局搜索相同
/// （[`ignore::IncrementalIgnore`]），每列出 [`MATCHER_DIRS`] 个目录换一个匹配器以限制规则缓存
/// （家目录能占一百多 MB）；只用一个线程，因为 macOS 的分配器不回收短命线程用过的内存。
pub(super) fn collect(root: &Path, limit: usize, cancel: &AtomicBool) -> Files {
    if winc_backend::is_remote(root) {
        return collect_remote(root, limit, cancel);
    }
    let fresh = || walker(root).build_matchers().into_iter().next();
    let mut matcher = fresh();
    let mut listed = 0;
    let mut frontier: Packed = [""].into_iter().collect();
    // 路径都放进 `Packed`，拼路径用同一个 `scratch`，遍历中不为每个文件和目录单独分配内存。
    let (mut files, mut dirs, mut scratch) = (Packed::default(), Packed::default(), String::new());
    let mut layers = None;

    for depth in 1.. {
        if frontier.is_empty() {
            break;
        }
        // 这一层的文件直接加进 `files`；加进去会超过 `limit` 时，退回到这一层之前。
        let before = files.len();
        let mut next = Packed::default();
        let mut full = false;
        for dir in frontier.iter() {
            if cancel.load(Ordering::Relaxed) {
                return Files::default();
            }
            listed += 1;
            if listed % MATCHER_DIRS == 0 {
                matcher = fresh();
            }
            list(
                root,
                dir,
                matcher.as_mut(),
                &mut scratch,
                &mut files,
                &mut next,
            );
            // 第 1 层自己就超过 `limit` 时也要列完，排序后收录前 `limit` 个。
            if depth > 1 && files.len() > limit {
                full = true;
                break;
            }
        }
        if full {
            files.truncate(before);
            layers = Some(depth - 1);
            break;
        }
        for dir in frontier.iter().filter(|dir| !dir.is_empty()) {
            dirs.push(dir);
        }
        frontier = next;
    }

    let mut table = Files::sorted(&files, limit);
    table.layers = layers;
    table.dirs = DirSet::new(dirs.iter());
    table
}

/// 列出 `dir`（相对路径）中不被忽略的文件和目录，把它们的相对路径分别放进 `files`、`dirs`。
///
/// 不跟随符号链接，与 `ignore` 的遍历相同。`scratch` 是拼路径用的缓冲，内容不保留。
fn list(
    root: &Path,
    dir: &str,
    matcher: Option<&mut ignore::IncrementalIgnore>,
    scratch: &mut String,
    files: &mut Packed,
    dirs: &mut Packed,
) {
    let Ok(entries) = winc_backend::fs::read_dir(root.join(dir)) else {
        return;
    };
    let mut matcher = matcher;
    for entry in entries.flatten() {
        let Some(name) = entry.name.to_str() else {
            continue;
        };
        if HIDDEN.contains(&name) {
            continue;
        }
        let kind = entry.kind;
        // `entry.path()` 需要分配，只为符号链接计算。
        let file = kind.is_file() || kind.is_symlink() && is_listed_file(kind, &entry.path());
        if !file && !kind.is_dir() {
            continue;
        }
        scratch.clear();
        if !dir.is_empty() {
            scratch.push_str(dir);
            scratch.push('/');
        }
        scratch.push_str(name);
        if let Some(matcher) = matcher.as_deref_mut()
            && matcher.matched(scratch.as_str(), kind.is_dir()).is_ignore()
        {
            continue;
        }
        if file {
            files.push(scratch);
        } else {
            dirs.push(scratch);
        }
    }
}

/// 返回 `dir`（相对路径）下不被忽略的直接子项的名字。
fn listing(root: &Path, dir: &str) -> HashSet<String> {
    walker(&root.join(dir))
        .max_depth(Some(1))
        .build()
        .flatten()
        .filter(|entry| entry.depth() == 1)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// 收录 `dir`（相对路径）下面的文件和遍历过的目录，最多到第 `layers` 层（为 `None` 时不限）；
/// `dir` 自己也算遍历过的目录。
///
/// 文件放进 `files`，目录放进 `dirs`；`cancel` 置位时提前返回。`dir` 所在的层数是其路径段数
/// （从 1 起）。
fn gather(
    root: &Path,
    dir: &str,
    layers: Option<usize>,
    files: &mut Vec<String>,
    dirs: &mut Vec<String>,
    cancel: &AtomicBool,
) {
    let depth = dir.split('/').count();
    if layers.is_some_and(|layers| depth >= layers) {
        return;
    }
    dirs.push(dir.to_owned());
    let walker = walker(&root.join(dir))
        .max_depth(layers.map(|layers| layers - depth))
        .build();
    for entry in walker.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(path) = entry.path().strip_prefix(root).ok().and_then(relative_path) else {
            continue;
        };
        if is_listed_file(kind, entry.path()) {
            files.push(path);
        } else if kind.is_dir() && layers.is_none_or(|layers| depth + entry.depth() < layers) {
            dirs.push(path);
        }
    }
}

/// 按变化的路径更新文件表，返回新表；表不需要变化或任务被取消时返回 `None`。
///
/// 只看已遍历过的目录里的变化：被忽略或因太深而未收录的目录与文件表无关。表里已有就不动，
/// 没有就加上（目录则收录它下面的内容）；不存在或被忽略就从表里去掉。
///
/// `.gitignore`、`.ignore` 变化时重新收录它所在的目录（在根目录时整个重新遍历）；更新后的文件
/// 数超过 [`MAX_FILES`] 时也整个重新遍历。
pub(super) fn patch(
    root: &Path,
    base: &Files,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Option<Files> {
    let mut removed: HashSet<String> = HashSet::new();
    let mut added: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut listings: HashMap<String, HashSet<String>> = HashMap::new();
    let mut redone: Option<String> = None;

    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Some(rel) = path.strip_prefix(root).ok().and_then(relative_path) else {
            continue;
        };
        let (parent, name) = rel
            .rsplit_once('/')
            .map_or(("", rel.as_str()), |(parent, name)| (parent, name));
        if !base.walked(parent) {
            continue;
        }

        if name == ".gitignore" || name == ".ignore" {
            if parent.is_empty() {
                return Some(collect(root, MAX_FILES, cancel));
            }
            removed.insert(parent.to_owned());
            gather(root, parent, base.layers, &mut added, &mut dirs, cancel);
            continue;
        }

        let was_file = base.position(&rel).is_some();
        let was_dir = base.dirs.contains(rel.as_str());
        let allowed = listings
            .entry(parent.to_owned())
            .or_insert_with(|| listing(root, parent))
            .contains(name);
        let kind = allowed
            .then(|| winc_backend::fs::symlink_metadata(path).ok())
            .flatten()
            .map(|m| m.kind);

        match kind {
            Some(kind) if is_listed_file(kind, path) => {
                if was_file && !was_dir {
                    continue;
                }
                if was_dir {
                    removed.insert(rel.clone());
                }
                added.push(rel);
            }
            Some(kind) if kind.is_dir() => {
                // 路径已排好序，上一级目录刚重新收录过时不用再收录一遍。
                if redone
                    .as_deref()
                    .is_some_and(|dir| rel.strip_prefix(dir).is_some_and(|r| r.starts_with('/')))
                {
                    continue;
                }
                // 目录可能被整个换掉（删除后把另一个目录改名移入），此时只有这一条通知，里面的
                // 文件没有各自的通知，因此与磁盘对照；没有变化时不更新文件表。
                let (mut files, mut found) = (Vec::new(), Vec::new());
                gather(root, &rel, base.layers, &mut files, &mut found, cancel);
                if was_dir && !was_file && same_files(base, &rel, &mut files) {
                    continue;
                }
                if was_file || was_dir {
                    removed.insert(rel.clone());
                }
                added.append(&mut files);
                dirs.append(&mut found);
                redone = Some(rel);
            }
            _ if was_file || was_dir => {
                removed.insert(rel);
            }
            _ => {}
        }
    }

    if removed.is_empty() && added.is_empty() && dirs.is_empty() {
        return None;
    }
    let files = merge(base, &removed, added, dirs);
    if files.len() > MAX_FILES {
        return Some(collect(root, MAX_FILES, cancel));
    }
    Some(files)
}

/// `base` 中目录 `dir` 下的文件是否正好是 `files`；`files` 会被排序。
fn same_files(base: &Files, dir: &str, files: &mut [String]) -> bool {
    files.sort_by(|a, b| path_order(a, b));
    let mut inside = base
        .paths
        .iter()
        .filter(|path| path.strip_prefix(dir).is_some_and(|r| r.starts_with('/')));
    files
        .iter()
        .all(|file| inside.next() == Some(file.as_str()))
        && inside.next().is_none()
}

/// 返回新表：从 `base` 中去掉 `removed`（连同它们下面的内容），加上 `added` 和遍历过的 `dirs`，
/// 仍按 [`path_order`] 排序。
fn merge(
    base: &Files,
    removed: &HashSet<String>,
    mut added: Vec<String>,
    dirs: Vec<String>,
) -> Files {
    // 路径本身或它所在的某一级目录在 `removed` 中。
    let gone = |path: &str| {
        !removed.is_empty()
            && (removed.contains(path)
                || path
                    .match_indices('/')
                    .any(|(i, _)| removed.contains(&path[..i])))
    };

    added.sort_by(|a, b| path_order(a, b));
    added.dedup();
    // 按上限预留，建好后去掉多余的容量：新表会一直留着，不能带着去掉的路径和扩容的余量。
    let mut files = Files {
        layers: base.layers,
        dirs: DirSet::new(
            base.dirs
                .iter()
                .filter(|dir| !gone(dir))
                .chain(dirs.iter().map(String::as_str)),
        ),
        ..Files::with_capacity(
            base.paths.text.len() + added.iter().map(String::len).sum::<usize>(),
            base.len() + added.len(),
        )
    };

    let mut fresh = added.iter().peekable();
    for path in base.paths.iter() {
        if gone(path) {
            continue;
        }
        while let Some(next) = fresh.next_if(|a| path_order(a, path).is_lt()) {
            files.push(next);
        }
        // 重新收录的目录里原本就有的文件：以 `base` 中的为准，跳过 `added` 中的重复项。
        fresh.next_if(|a| *a == path);
        files.push(path);
    }
    for path in fresh {
        files.push(path);
    }
    files.paths.shrink_to_fit();
    files.names.shrink_to_fit();
    files
}

/// 比较两条路径在文件表中的先后顺序：先比较逐字符转小写后的路径（`a.txt` 和 `Cargo.toml`
/// 按字母排在一起），只差大小写时再按原样比较，因此是全序，二分查找能找到确切的那一项。
///
/// 建表时要比较几百万次，所以先逐字节比较 ASCII 的小写形式，碰到非 ASCII 的字节时两边都正好
/// 在字符边界上，从这里起改为逐字符比较，结果与全程逐字符比较相同。用 `char::to_lowercase`
/// 而不用 `str::to_lowercase`：后者会把词尾的 Σ 转成 ς，得到的顺序不一致。
pub(super) fn path_order(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    for (i, (&p, &q)) in x.iter().zip(y).enumerate() {
        if !p.is_ascii() || !q.is_ascii() {
            return a[i..]
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(b[i..].chars().flat_map(char::to_lowercase))
                .then_with(|| a.cmp(b));
        }
        let (p, q) = (p.to_ascii_lowercase(), q.to_ascii_lowercase());
        if p != q {
            return p.cmp(&q);
        }
    }
    x.len().cmp(&y.len()).then_with(|| a.cmp(b))
}

/// 把相对路径转成以 `/` 分隔的字符串；含非 UTF-8 的部分或路径为空时返回 `None`，
/// 调用方据此跳过该文件。
pub(super) fn relative_path(relative: &Path) -> Option<String> {
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

/// 收录远端主机上目录 `root` 中的文件：由远端的一条命令列出（见
/// [`remote_files`](crate::workspace::remote_files)），一次往返。超过 `limit` 个时按路径的
/// 顺序收录前 `limit` 个。
fn collect_remote(root: &Path, limit: usize, cancel: &AtomicBool) -> Files {
    let stop = || cancel.load(Ordering::Relaxed);
    let listing = crate::workspace::remote_files::list(root, false, None, &stop);
    let (mut files, mut dirs) = (Packed::default(), HashSet::new());
    for path in &listing {
        files.push(path);
        // 文件所在的各级目录都算遍历过：其中的变化要更新文件表。
        let mut dir = path.as_str();
        while let Some((parent, _)) = dir.rsplit_once('/') {
            if !dirs.insert(parent) {
                break;
            }
            dir = parent;
        }
    }
    let mut table = Files::sorted(&files, limit);
    table.dirs = DirSet::new(dirs);
    table
}
