//! 解析 `git` 的输出：状态、分支、日志、逐行追溯、提交的文件，以及出错时给用户看的说明。
//!
//! 各命令的输出格式见各函数的文档；本模块不启动进程，输入由 [`Git`] 的方法取回。

use std::collections::HashSet;

use super::*;

/// 从 `git` 的输出里挑出失败的原因，用作给用户的说明。
///
/// 优先取 `error:`、`fatal:`、`! [rejected]`、`CONFLICT` 开头的行，没有这样的行时取第一行
/// 非空的内容。
pub(super) fn failure(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let key = lines.iter().find(|l| {
        l.starts_with("error:")
            || l.starts_with("fatal:")
            || l.starts_with("! [")
            || l.starts_with("CONFLICT")
    });
    match key.or(lines.first()) {
        Some(line) => line
            .trim_start_matches("fatal: ")
            .trim_start_matches("error: ")
            .to_owned(),
        None => winc_i18n::t!("git-run-failed").to_owned(),
    }
}

/// 返回 `git cat-file <rev>:<路径>` 以 128 退出时是否说明该版本里没有该文件。
///
/// 依据是报错 `stderr`（`LC_ALL=C`）的内容：「does not exist」「exists on disk, but not in」
/// 「is in the index, but not at stage 0」（冲突中的文件）；还没有提交过时 `HEAD` 本身认不出，
/// 文件同样不在 `HEAD` 里。其余以 128 退出的都是真正的失败：版本不存在、对象损坏、必需的
/// 过滤器失败。
pub(super) fn absent(stderr: &[u8], rev: &str) -> bool {
    let text = String::from_utf8_lossy(stderr);
    let path = text.lines().any(|line| {
        line.starts_with("fatal: path '")
            && [
                "' does not exist",
                "' exists on disk, but not in",
                "' is in the index, but not at stage",
            ]
            .iter()
            .any(|tail| line.contains(tail))
    });
    path || (rev == "HEAD" && text.contains("fatal: invalid object name 'HEAD'"))
}

/// 返回成功时 `git` 给出的结论，即输出中最后一行非空的内容（例如「Already up to date.」
/// 「Fast-forward」）。
pub(super) fn summary(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// 解析引用日志里的 `checkout: moving from A to B`：`B` 比 `A` 新，较新的行在前，返回不重复的
/// 分支名，最近的在前。
pub(super) fn parse_recent_branches(output: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines() {
        let Some(moved) = line.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((from, to)) = moved.rsplit_once(" to ") else {
            continue;
        };
        for name in [to, from] {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_owned());
            }
        }
    }
    names
}

/// 解析 `git for-each-ref` 的输出（每行的引用名、`*` 或空格、上游、提交号、提交时间、本机时区的
/// 提交时间之间用 NUL 隔开）。
///
/// 本地的分支在前，远程的在后，各自按名字排列；远程的 `HEAD` 指针不列。上游引用已不存在的（远程
/// 分支被删除后获取时清掉了）算作没有上游：`--prune` 之后没有什么可以和它同步。
pub(super) fn parse_branches(output: &str) -> Vec<Branch> {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in output.lines() {
        let mut fields = line.split('\0');
        let (Some(name), Some(head)) = (fields.next(), fields.next()) else {
            continue;
        };
        let upstream = fields.next().filter(|u| !u.is_empty()).map(str::to_owned);
        let oid = fields.next().unwrap_or_default().to_owned();
        let time = fields
            .next()
            .and_then(|t| t.parse().ok())
            .unwrap_or_default();
        let date = fields.next().unwrap_or_default().to_owned();
        if let Some(name) = name.strip_prefix("refs/heads/") {
            local.push(Branch {
                name: name.to_owned(),
                remote: false,
                current: head == "*",
                upstream,
                gone: false,
                oid,
                time,
                local: date,
            });
        } else if let Some(name) = name.strip_prefix("refs/remotes/") {
            if name.ends_with("/HEAD") {
                continue;
            }
            remote.push(Branch {
                name: name.to_owned(),
                remote: true,
                current: false,
                upstream: None,
                gone: false,
                oid,
                time,
                local: date,
            });
        }
    }
    local.sort_by(|a, b| a.name.cmp(&b.name));
    remote.sort_by(|a, b| a.name.cmp(&b.name));
    local.extend(remote);
    let known: HashSet<String> = local.iter().map(|b| b.name.clone()).collect();
    for branch in &mut local {
        // 和本地分支重名时 `git` 给上游加上 `remotes/`、`heads/`。
        let gone = branch.upstream.as_deref().is_some_and(|upstream| {
            let name = upstream
                .strip_prefix("remotes/")
                .or_else(|| upstream.strip_prefix("heads/"))
                .unwrap_or(upstream);
            !known.contains(name)
        });
        if gone {
            branch.upstream = None;
            branch.gone = true;
        }
    }
    local
}

/// 返回运行 `git` 失败的原因说明，用于展示给用户。
pub(super) fn describe(error: &RunError) -> String {
    match error {
        RunError::Spawn(e) => winc_i18n::t!("git-run-spawn-failed", error = e.to_string()),
        other => format!("git {other}"),
    }
}

/// 解析 `git blame --porcelain` 的输出。
///
/// 每一行先是「提交号 原行号 现行号 [连着几行]」，同一提交第一次出现时后面跟着 `author`、
/// `author-time`、`author-tz`、`summary`，最后是 Tab 开头的该行内容。行号从 1 起。
pub(super) fn parse_blame(text: &str) -> Blame {
    let mut blame = Blame::default();
    let mut index: HashMap<String, u32> = HashMap::new();
    let mut current: Option<(u32, usize)> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            if let Some((commit, at)) = current.take() {
                if blame.lines.len() <= at {
                    blame.lines.resize(at + 1, commit);
                }
                blame.lines[at] = commit;
            }
            continue;
        }
        let mut words = line.split(' ');
        let first = words.next().unwrap_or("");
        if object_id(first) {
            let final_line = words
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);
            let next = blame.commits.len() as u32;
            let commit = *index.entry(first.to_owned()).or_insert_with(|| {
                blame.commits.push(BlameCommit {
                    oid: first.to_owned(),
                    uncommitted: first.bytes().all(|b| b == b'0'),
                    ..BlameCommit::default()
                });
                next
            });
            current = Some((commit, final_line.saturating_sub(1)));
            continue;
        }
        let Some((commit, _)) = current else {
            continue;
        };
        let Some(entry) = blame.commits.get_mut(commit as usize) else {
            continue;
        };
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => entry.author = value.to_owned(),
            "author-time" => entry.time = value.parse().unwrap_or(0),
            "author-tz" => entry.offset = parse_tz(value),
            "summary" => entry.summary = value.to_owned(),
            _ => {}
        }
    }
    blame
}

/// 把「+0800」「-0530」形式的时区换成相对 UTC 的偏移（秒）。
fn parse_tz(text: &str) -> i64 {
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    let hours: i64 = digits.get(..2).and_then(|h| h.parse().ok()).unwrap_or(0);
    let minutes: i64 = digits.get(2..4).and_then(|m| m.parse().ok()).unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

/// `text` 是否是完整的对象号：40 位十六进制数（SHA-1）或 64 位十六进制数（SHA-256 仓库）。
fn object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 解析 `git log` 的输出：每个提交一条记录，记录之间用 `0x1e` 隔开，字段之间是 `0x1f`。
///
/// 提交说明（`%s`）中也可以包含 `0x1e`、`0x1f`，从而伪造一条提交号为任意文字（例如
/// `--output=…`）的记录。因此提交号或父提交不是完整对象号的记录一律丢弃。
pub(super) fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\u{1e}')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').split('\u{1f}');
            let oid = fields.next()?;
            if !object_id(oid) {
                return None;
            }
            let oid = oid.to_owned();
            let parents: Vec<String> = fields
                .next()?
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            if !parents.iter().all(|p| object_id(p)) {
                return None;
            }
            let author = fields.next()?.to_owned();
            let email = fields.next()?.to_owned();
            let time = fields.next()?.parse().unwrap_or(0);
            let local = fields.next()?.to_owned();
            let refs = parse_refs(fields.next()?);
            let subject = fields.next().unwrap_or("").to_owned();
            Some(Commit {
                oid,
                parents,
                author,
                email,
                time,
                local,
                refs,
                subject,
            })
        })
        .collect()
}

/// 解析 `%D`（`--decorate=full`）给出的引用，例如
/// `HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1`。
///
/// 远程的 `HEAD` 指针不列。
pub(super) fn parse_refs(text: &str) -> Vec<Ref> {
    let mut refs = Vec::new();
    for item in text.split(", ").map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(branch) = item.strip_prefix("HEAD -> ") {
            let name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
            refs.push(Ref::Head(name.to_owned()));
        } else if item == "HEAD" {
            refs.push(Ref::Detached);
        } else if let Some(tag) = item.strip_prefix("tag: ") {
            refs.push(Ref::Tag(
                tag.strip_prefix("refs/tags/").unwrap_or(tag).to_owned(),
            ));
        } else if let Some(name) = item.strip_prefix("refs/heads/") {
            refs.push(Ref::Branch(name.to_owned()));
        } else if let Some(name) = item.strip_prefix("refs/remotes/")
            && !name.ends_with("/HEAD")
        {
            refs.push(Ref::Remote(name.to_owned()));
        }
    }
    refs
}

/// 解析 `git diff-tree --raw --numstat -z` 的输出。
///
/// 先是每个文件一条 `--raw`：`:旧模式 新模式 旧对象 新对象状态\0路径\0`，改名、复制是
/// `…R087\0旧路径\0新路径\0`；之后按同样的顺序每个文件一条 `--numstat`：`加\t删\t路径\0`，
/// 改名是 `加\t删\t\0旧路径\0新路径\0`，二进制文件的行数是 `-`。第几条行数对应第几个文件。
pub(super) fn parse_diff_tree(output: &[u8], root: &Path) -> Vec<CommitFile> {
    let mut fields = output.split(|b| *b == 0).peekable();
    let path = |f: &[u8]| root.join(String::from_utf8_lossy(f).as_ref());
    let mut files = Vec::new();
    while let Some(raw) = fields.next_if(|f| f.first() == Some(&b':')) {
        let status = raw.rsplit(|b| *b == b' ').next().unwrap_or_default();
        let change = match status.first() {
            Some(b'A' | b'C') => Change::Added,
            Some(b'D') => Change::Deleted,
            Some(b'R') => Change::Renamed,
            _ => Change::Modified,
        };
        let Some(first) = fields.next() else { break };
        let (from, to) = if matches!(status.first(), Some(b'R' | b'C')) {
            let Some(to) = fields.next() else { break };
            (Some(path(first)), path(to))
        } else {
            (None, path(first))
        };
        files.push(CommitFile {
            path: to,
            from: from.filter(|_| change == Change::Renamed),
            change,
            lines: None,
        });
    }
    for file in &mut files {
        let Some(count) = fields.next() else { break };
        let mut parts = count.splitn(3, |b| *b == b'\t');
        let number = |part: Option<&[u8]>| std::str::from_utf8(part?).ok()?.parse().ok();
        let (added, removed) = (number(parts.next()), number(parts.next()));
        // 改名：这一条的路径是空的，后面跟着旧路径、新路径两段。
        if parts.next().is_some_and(<[u8]>::is_empty) {
            fields.next();
            fields.next();
        }
        file.lines = added.zip(removed);
    }
    files
}

/// 解析 `git status --porcelain=v2 -z --branch` 的输出。
///
/// 路径相对于 `root`，返回的是绝对路径。
pub(super) fn parse_status(output: &[u8], root: &Path) -> Option<Status> {
    let mut status = Status::default();
    let mut fields = output.split(|b| *b == 0).filter(|f| !f.is_empty());
    let path = |bytes: &[u8]| root.join(bytes_to_path(bytes));

    while let Some(field) = fields.next() {
        let text = field;
        match text.first()? {
            b'#' => parse_header(text, &mut status),
            b'1' => {
                // 1 XY sub mH mI mW hH hI path
                let mut parts = text.splitn(9, |b| *b == b' ');
                let xy = parts.nth(1)?;
                let rest = parts.nth(6)?;
                push_entry(&mut status, entry(path(rest), None, xy)?);
            }
            b'2' => {
                // 2 XY sub mH mI mW hH hI Xscore path，后面的一个字段是原来的路径。
                let mut parts = text.splitn(10, |b| *b == b' ');
                let xy = parts.nth(1)?;
                let rest = parts.nth(7)?;
                let from = fields.next()?;
                push_entry(&mut status, entry(path(rest), Some(path(from)), xy)?);
            }
            b'u' => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let mut parts = text.splitn(11, |b| *b == b' ');
                let rest = parts.nth(10)?;
                let conflict = Entry {
                    path: path(rest),
                    from: None,
                    staged: None,
                    unstaged: Some(Change::Conflicted),
                    is_dir: false,
                };
                push_entry(&mut status, conflict);
            }
            b'?' => {
                let rest = text.get(2..)?;
                // 未跟踪的目录以 `/` 结尾。
                let dir = rest.strip_suffix(b"/");
                let untracked = Entry {
                    path: path(dir.unwrap_or(rest)),
                    from: None,
                    staged: None,
                    unstaged: Some(Change::Untracked),
                    is_dir: dir.is_some(),
                };
                push_entry(&mut status, untracked);
            }
            // 忽略的文件和目录（目录以 `/` 结尾）。
            b'!' => {
                let rest = text.get(2..)?;
                status
                    .ignored
                    .push(path(rest.strip_suffix(b"/").unwrap_or(rest)));
            }
            // 跳过其他未知的行。
            _ => {}
        }
    }
    Some(status)
}

/// 把一条变化记进 `status`；已有 [`STATUS_LIMIT`] 条时不再记，并标记为截断。
fn push_entry(status: &mut Status, entry: Entry) {
    if status.entries.len() < STATUS_LIMIT {
        status.entries.push(entry);
    } else {
        status.truncated = true;
    }
}

fn parse_header(line: &[u8], status: &mut Status) {
    let line = String::from_utf8_lossy(line);
    let mut words = line.split(' ').skip(1);
    match words.next() {
        Some("branch.oid") => {
            if let Some(oid) = words.next().filter(|o| *o != "(initial)") {
                status.oid = Some(oid.to_owned());
                if status.head.is_none() {
                    status.head = Some(Head::Detached(oid.chars().take(7).collect()));
                }
            }
        }
        Some("branch.head") => match words.next() {
            Some("(detached)") | None => {}
            Some(name) => status.head = Some(Head::Branch(name.to_owned())),
        },
        Some("branch.ab") => {
            for word in words {
                if let Some(n) = word.strip_prefix('+') {
                    status.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = word.strip_prefix('-') {
                    status.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

fn entry(path: PathBuf, from: Option<PathBuf>, xy: &[u8]) -> Option<Entry> {
    let [x, y] = xy else { return None };
    Some(Entry {
        path,
        from,
        staged: Change::from_code(*x),
        unstaged: Change::from_code(*y),
        is_dir: false,
    })
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).replace('/', "\\"))
}
