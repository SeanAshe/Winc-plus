//! 收集工作区的文件：整体遍历，以及按目录中的变化增量更新。

use super::*;

/// 遍历时更换忽略规则匹配器的目录间隔（个）：每列出该数量的目录就换一个新的匹配器。
///
/// 依据见 [`collect`] 的说明。
const MATCHER_DIRS: usize = 1024;

/// 遍历 `root` 和各个附加的目录 `extra`（路径前缀、目录），合成一张文件表。
///
/// 附加的目录中的路径以前缀加 `/` 开头；总数超过 `limit` 时先收根目录的。
pub(super) fn collect_all(
    root: &Path,
    extra: &[(String, PathBuf)],
    limit: usize,
    cancel: &AtomicBool,
) -> Files {
    let main = collect(root, limit, cancel);
    if extra.is_empty() {
        return main;
    }
    let mut paths: Packed = main.paths.iter().collect();
    let mut dirs: Vec<String> = main.dirs.iter().map(str::to_owned).collect();
    let mut scratch = String::new();
    let mut failed = main.failed;
    for (label, folder) in extra {
        let part = collect(folder, limit.saturating_sub(paths.len()), cancel);
        failed |= part.failed;
        dirs.push(label.clone());
        dirs.extend(part.dirs.iter().map(|dir| format!("{label}/{dir}")));
        for path in part.paths.iter() {
            scratch.clear();
            scratch.push_str(label);
            scratch.push('/');
            scratch.push_str(path);
            paths.push(&scratch);
        }
    }
    let mut table = Files::sorted(&paths, limit);
    table.layers = main.layers;
    table.failed = failed;
    table.dirs = DirSet::new(dirs.iter().map(String::as_str));
    table
}

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

/// 一次后台更新的结果：新的文件表（不需要变化时为 `None`），以及确认已不存在的路径（相对路径，
/// 以 `/` 分隔），调用方据此清理别处记着的这些文件。
pub(super) struct Patched {
    pub(super) files: Option<Files>,
    pub(super) missing: Vec<String>,
}

/// 按变化的路径更新文件表；任务被取消，或远端的目录没能查看时返回 `None`，文件表保持原样。
///
/// 只看已遍历过的目录里的变化：被忽略或因太深而未收录的目录与文件表无关。表里已有就不动，
/// 没有就加上（目录则收录它下面的内容）；不存在或被忽略就从表里去掉。变化的路径各自的样子按与
/// 遍历相同的规则查看（[`winc_search::probe`]）；远端的目录由远端的服务进程一次查看完。
///
/// `.gitignore`、`.ignore` 变化时重新收录它所在的目录（在根目录时整个重新遍历）；更新后的文件
/// 数超过 [`MAX_FILES`] 时也整个重新遍历。
pub(super) fn patch(
    root: &Path,
    base: &Files,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Option<Patched> {
    let whole = |cancel| {
        Some(Patched {
            files: Some(collect(root, MAX_FILES, cancel)),
            missing: Vec::new(),
        })
    };
    let mut wanted: Vec<String> = Vec::new();
    for path in paths {
        let Some(rel) = relative_in(root, path) else {
            continue;
        };
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if !base.walked(parent) {
            continue;
        }
        if name == ".gitignore" || name == ".ignore" {
            if parent.is_empty() {
                return whole(cancel);
            }
            wanted.push(parent.to_owned());
        } else {
            wanted.push(rel);
        }
    }
    // 按路径的层级排序：目录之下的路径紧随其后，重新收录过的目录下的变化不必再看。
    wanted.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
    wanted.dedup();
    if wanted.is_empty() {
        return Some(Patched {
            files: None,
            missing: Vec::new(),
        });
    }
    let probes = match winc_backend::is_remote(root) {
        true => probe_remote(root, &wanted, base.layers, cancel)?,
        false => winc_search::probe(root, &wanted, base.layers, cancel),
    };
    if cancel.load(Ordering::Relaxed) || probes.len() != wanted.len() {
        return None;
    }

    let mut removed: HashSet<String> = HashSet::new();
    let (mut added, mut dirs, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for (rel, probe) in wanted.into_iter().zip(probes) {
        let was_file = base.position(&rel).is_some();
        let was_dir = base.dirs.contains(rel.as_str());
        match probe {
            Probe::File if was_file && !was_dir => {}
            Probe::File => {
                if was_dir {
                    removed.insert(rel.clone());
                }
                added.push(rel);
            }
            // 目录可能被整个换掉（删除后把另一个目录改名移入），此时只有这一条通知，里面的
            // 文件没有各自的通知，因此与磁盘对照；没有变化时不更新文件表。
            Probe::Dir {
                mut files,
                dirs: mut found,
            } => {
                // 文件没变、也没有新的目录时不更新：其下的变化已在这次查看之内（见
                // `Probe::Covered`），新建的空目录也要记为遍历过。
                let known = found.iter().all(|dir| base.dirs.contains(dir.as_str()));
                if was_dir && !was_file && known && same_files(base, &rel, &mut files) {
                    continue;
                }
                if was_file || was_dir {
                    removed.insert(rel);
                }
                added.append(&mut files);
                dirs.append(&mut found);
            }
            Probe::Missing => {
                if was_file || was_dir {
                    removed.insert(rel.clone());
                }
                missing.push(rel);
            }
            Probe::Unlisted if was_file || was_dir => {
                removed.insert(rel);
            }
            Probe::Unlisted | Probe::Covered => {}
        }
    }

    if removed.is_empty() && added.is_empty() && dirs.is_empty() {
        return Some(Patched {
            files: None,
            missing,
        });
    }
    let files = merge(base, &removed, added, dirs);
    if files.len() > MAX_FILES {
        return whole(cancel);
    }
    Some(Patched {
        files: Some(files),
        missing,
    })
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
        failed: base.failed,
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

/// 由远端的服务进程查看根目录 `root` 下的各个路径（见 [`winc_search::probe`]），一次往返；任务
/// 被取消、无法启动或连接中断时返回 `None`。
fn probe_remote(
    root: &Path,
    paths: &[String],
    layers: Option<usize>,
    cancel: &AtomicBool,
) -> Option<Vec<Probe>> {
    let at = winc_backend::locate(root)?;
    let listing = Listing {
        root: at.path,
        paths: paths.to_vec(),
        layers,
    };
    let exec = Exec::this_server().arg(host::FILES).stdin(listing.encode());
    let mut input = std::io::BufReader::new(winc_backend::remote::spawn(root, &exec).ok()?);
    let mut found = Vec::with_capacity(paths.len());
    while found.len() < paths.len() {
        // 放弃读取时远端的进程随之结束。
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        found.push(host::read_probe(&mut input).ok()??);
    }
    Some(found)
}

/// 收录远端主机上目录 `root` 中的文件：由远端的服务进程遍历（规则与本机的遍历相同），一次
/// 往返。超过 `limit` 个时按路径的顺序收录前 `limit` 个。没能列出时文件表为空，并记下失败。
fn collect_remote(root: &Path, limit: usize, cancel: &AtomicBool) -> Files {
    let listed = probe_remote(root, &[String::new()], None, cancel);
    let Some(Probe::Dir { files, dirs }) = listed.and_then(|mut found| found.pop()) else {
        return Files {
            failed: !cancel.load(Ordering::Relaxed),
            ..Files::default()
        };
    };
    let files: Packed = files.iter().map(String::as_str).collect();
    let mut table = Files::sorted(&files, limit);
    table.dirs = DirSet::new(dirs.iter().map(String::as_str));
    table
}
