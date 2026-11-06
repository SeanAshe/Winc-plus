//! 勾选和折叠的状态，以及按目录分层排好的改动列表。

use super::*;

impl CommitState {
    /// 返回当前按下「提交」不会提交的原因；可以提交（或正在提交）时为 `None`。
    ///
    /// 检查项与 [`App::git_commit`] 相同。提交按钮据此置灰，悬停提示显示原因。
    /// 冲突标记按状态中读取的结果判断：编辑器中刚移除、尚未保存时按钮仍置灰，
    /// 按下后先保存再重新检查。
    pub fn blocker(&self, status: &Status) -> Option<String> {
        if self.busy {
            return None;
        }
        let rebasing = status.operation == Some(Operation::Rebase);
        let mut checked = self.checked_entries(status).peekable();
        if checked.peek().is_none() && !self.amend && !rebasing {
            return Some(t!("git-commit-nothing-checked").to_owned());
        }
        let unresolved = checked
            .filter(|e| status.unresolved.contains(&e.path))
            .count();
        if unresolved > 0 {
            return Some(t!("git-commit-blocker-conflicts", count = unresolved));
        }
        if self.message.text().trim().is_empty() && !rebasing {
            return Some(t!("git-commit-message-required").to_owned());
        }
        None
    }

    /// 把正在写的提交信息记到位置 `root` 名下；什么都没写时清掉记录。
    pub(in crate::app) fn keep_draft(&mut self, root: PathBuf) {
        let text = if self.amend {
            self.draft.clone().unwrap_or_default()
        } else {
            self.message.text().to_owned()
        };
        if text.trim().is_empty() {
            self.drafts.remove(&root);
        } else {
            self.drafts.insert(root, text);
        }
    }

    /// 填回位置 `root` 名下记着的提交信息。
    pub(in crate::app) fn restore_draft(&mut self, root: &Path) {
        if let Some(text) = self.drafts.remove(root) {
            self.message.set_text(&text);
        }
    }

    /// 文件 `entry` 是否勾上。
    pub fn checked(&self, entry: &Entry) -> bool {
        if entry.untracked() {
            self.included.contains(&entry.path)
        } else {
            !self.excluded.contains(&entry.path)
        }
    }

    /// 文件的勾选框：`Some(true)` 整个提交，`Some(false)` 只提交一部分改动，`None` 不提交。
    ///
    /// 只暂存了一部分的文件（[`Entry::partly_staged`]）勾着时只提交已暂存的内容，也是一部分。
    pub fn file_state(&self, entry: &Entry) -> Option<bool> {
        if !self.checked(entry) {
            None
        } else if self.skips_hunks(entry) || entry.partly_staged() {
            Some(false)
        } else {
            Some(true)
        }
    }

    /// 文件 `entry` 是否在差异里勾掉了其中几处改动。
    pub fn skips_hunks(&self, entry: &Entry) -> bool {
        self.skipped.get(&entry.path).is_some_and(|s| !s.is_empty())
    }

    /// 文件 `entry` 里的一处改动是否提交：`key` 是它的内容，`old` 是它在 `HEAD` 里的行。
    pub fn hunk_on(&self, entry: &Entry, key: u64, old: &Range<usize>) -> bool {
        self.checked(entry)
            && !self
                .skipped
                .get(&entry.path)
                .is_some_and(|s| s.iter().any(|s| s.matches(key, old)))
    }

    /// 勾上或勾掉文件 `entry` 里的一处改动（`key`、`old` 见 [`CommitState::hunk_on`]）；`all`
    /// 是该文件现在所有的改动。
    ///
    /// 文件没勾时勾上一处：文件勾上，只提交这一处；一处都不剩时文件也勾掉。
    pub fn toggle_hunk(
        &mut self,
        entry: &Entry,
        key: u64,
        old: &Range<usize>,
        all: &[(u64, Range<usize>)],
    ) {
        let skip = |(key, old): &(u64, Range<usize>)| Skip {
            key: *key,
            old: old.clone(),
        };
        if !self.checked(entry) {
            self.set_checked(entry, true);
            let others = all.iter().filter(|(k, _)| *k != key).map(skip).collect();
            self.skipped.insert(entry.path.clone(), others);
            return;
        }
        let skipped = self.skipped.entry(entry.path.clone()).or_default();
        // 换成现在的样子：认得出的留下（内容变了的换成新的 key），认不出的丢弃。
        let mut fresh: Vec<Skip> = all
            .iter()
            .filter(|(k, o)| skipped.iter().any(|s| s.matches(*k, o)))
            .map(skip)
            .collect();
        match fresh.iter().position(|s| s.matches(key, old)) {
            Some(i) => {
                fresh.remove(i);
            }
            None => fresh.push(skip(&(key, old.clone()))),
        }
        let none_left = all
            .iter()
            .all(|(k, o)| fresh.iter().any(|s| s.matches(*k, o)));
        *skipped = fresh;
        if none_left {
            self.set_checked(entry, false);
        }
    }

    /// `HEAD` 现在是 `oid`：变了时按块的勾选全部作废。
    pub fn head_is(&mut self, oid: Option<&str>) {
        if self.head.as_deref() != oid {
            self.head = oid.map(str::to_owned);
            self.skipped.clear();
        }
    }

    /// 组 `group` 是否收起。
    pub fn group_collapsed(&self, group: CommitGroup) -> bool {
        self.collapsed[group.index()]
    }

    /// 是否平铺显示（不按目录排成树）。
    pub fn flat(&self) -> bool {
        self.flat
    }

    /// 组 `group` 里的目录 `dir`（相对仓库根目录）是否收起。
    pub fn dir_collapsed(&self, group: CommitGroup, dir: &Path) -> bool {
        self.collapsed_dirs[group.index()].contains(dir)
    }

    /// 收起或展开组 `group`。
    pub(super) fn toggle_group(&mut self, group: CommitGroup) {
        let i = group.index();
        self.collapsed[i] = !self.collapsed[i];
        self.layout += 1;
    }

    /// 收起或展开组 `group` 里的目录 `dir`。
    pub(super) fn toggle_dir(&mut self, group: CommitGroup, dir: PathBuf) {
        let dirs = &mut self.collapsed_dirs[group.index()];
        if !dirs.remove(&dir) {
            dirs.insert(dir);
        }
        self.layout += 1;
    }

    /// 收起（`collapse` 为真）或展开组 `group` 里的目录 `dir` 和它下面的各级目录。
    pub(super) fn set_dir_tree_collapsed(
        &mut self,
        status: &Status,
        root: &Path,
        (group, dir): (CommitGroup, &Path),
        collapse: bool,
    ) {
        // 收起的目录下面的行不会排出来：先全部展开排一遍，才看得到各级目录。
        let collapsed = std::mem::take(&mut self.collapsed_dirs[group.index()]);
        let all = self.build_rows(status, root);
        self.collapsed_dirs[group.index()] = collapsed;
        let under = all.iter().filter_map(|row| match row {
            CommitRow::Dir {
                group: g, dir: d, ..
            } if *g == group && d.starts_with(dir) => Some(d.clone()),
            _ => None,
        });
        let dirs = &mut self.collapsed_dirs[group.index()];
        for path in under {
            if collapse {
                dirs.insert(path);
            } else {
                dirs.remove(&path);
            }
        }
        self.layout += 1;
    }

    /// 设定平铺（`true`）或按目录排成树。
    pub(crate) fn set_flat(&mut self, flat: bool) {
        if self.flat != flat {
            self.flat = flat;
            self.layout += 1;
        }
    }

    /// 全部收起，或者全部展开（连收起的目录一起展开）。
    pub(super) fn collapse_all(&mut self, collapse: bool) {
        self.collapsed = [collapse; CommitGroup::COUNT];
        if !collapse {
            self.forget_dirs();
        }
        self.layout += 1;
    }

    /// 清除收起的目录记录（换了目录，它们是旧仓库的）。
    pub(in crate::app) fn forget_dirs(&mut self) {
        self.collapsed_dirs.iter_mut().for_each(HashSet::clear);
        self.layout += 1;
    }

    /// 一些文件的勾选状态：`None` 一个都没勾，`Some(true)` 全勾上，`Some(false)` 勾了一部分。
    fn tri_state<'a>(&self, entries: impl IntoIterator<Item = &'a Entry>) -> Option<bool> {
        let (mut on, mut off) = (false, false);
        for entry in entries {
            if self.checked(entry) {
                on = true;
            } else {
                off = true;
            }
            if on && off {
                return Some(false);
            }
        }
        on.then_some(true)
    }

    /// 一组的勾选状态，含义同 [`CommitState::members_state`]。
    pub fn group_state(&self, status: &Status, group: CommitGroup) -> Option<bool> {
        self.tri_state(status.entries.iter().filter(|e| group.holds(e)))
    }

    /// `members`（`Status::entries` 的下标，见 [`CommitRows::group`]、[`CommitRows::dir`]）的
    /// 勾选状态：`None` 一个都没勾，`Some(true)` 全勾上，`Some(false)` 勾了一部分。
    pub fn members_state(&self, status: &Status, members: &[usize]) -> Option<bool> {
        self.tri_state(members.iter().filter_map(|&i| status.entries.get(i)))
    }

    /// 排列表：每组一行标题，没收起的组下面按目录排成树（或者平铺）。
    ///
    /// 空的组不显示。`root` 是仓库的根目录，文件的路径按它算相对路径。
    pub(super) fn build_rows(&self, status: &Status, root: &Path) -> CommitRows {
        let mut out = CommitRows::default();
        for group in CommitGroup::ALL {
            let files: Vec<(usize, &Path)> = status
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| group.holds(e))
                .map(|(i, e)| (i, e.path.strip_prefix(root).unwrap_or(&e.path)))
                .collect();
            if files.is_empty() {
                continue;
            }
            out.rows.push(CommitRow::Group(group));
            let start = out.members.len();
            if self.collapsed[group.index()] {
                out.members.extend(files.iter().map(|&(index, _)| index));
            } else if self.flat {
                // 平铺：按路径排，每个文件一行。
                let mut files = files;
                files.sort_by(|a, b| natural_path_cmp(a.1, b.1));
                out.push_files(&files, 0);
            } else {
                self.push_level(group, Path::new(""), files, 0, &mut out);
            }
            out.groups[group.index()] = start..out.members.len();
        }
        out
    }

    /// 把 `dir` 下面的 `items`（下标、相对 `dir` 的路径）排成一层：先是子目录（未收起的继续
    /// 往下排），再是文件。
    fn push_level<'a>(
        &self,
        group: CommitGroup,
        dir: &Path,
        items: Vec<(usize, &'a Path)>,
        depth: usize,
        out: &mut CommitRows,
    ) {
        let mut files = Vec::new();
        let mut dirs: BTreeMap<NaturalName<'a>, Vec<(usize, &'a Path)>> = BTreeMap::new();
        for (index, path) in items {
            let mut parts = path.components();
            match (parts.next(), parts.as_path()) {
                (Some(first), rest) if !rest.as_os_str().is_empty() => {
                    dirs.entry(NaturalName(first.as_os_str()))
                        .or_default()
                        .push((index, rest));
                }
                _ => files.push((index, path)),
            }
        }
        for (NaturalName(name), mut sub) in dirs {
            // 只有一个子目录、没有文件：和子目录合成一行。
            let mut label = PathBuf::from(name);
            while let Some(next) = only_subdir(&sub) {
                label.push(next);
                for (_, p) in &mut sub {
                    let rest: &'a Path = p;
                    *p = rest.strip_prefix(next).unwrap_or(Path::new(""));
                }
            }
            let full = dir.join(&label);
            let start = out.members.len();
            out.rows.push(CommitRow::Dir {
                group,
                dir: full.clone(),
                label: label.to_string_lossy().into_owned(),
                depth,
            });
            if self.dir_collapsed(group, &full) {
                out.members.extend(sub.iter().map(|&(index, _)| index));
            } else {
                self.push_level(group, &full, sub, depth + 1, out);
            }
            out.dirs[group.index()].insert(full, start..out.members.len());
        }
        files.sort_by(|a, b| natural_path_cmp(a.1, b.1));
        out.push_files(&files, depth);
    }

    /// 勾上或勾掉文件 `entry`：`on` 为 `false` 时勾掉；之前按块勾掉的一处改动随之作废。
    pub(super) fn set_checked(&mut self, entry: &Entry, on: bool) {
        self.checks += 1;
        let path = entry.path.clone();
        // 整个文件勾上或勾掉，之前按块勾掉的作废。
        self.skipped.remove(&path);
        if on {
            self.excluded.remove(&path);
            self.included.insert(path);
        } else {
            self.included.remove(&path);
            self.excluded.insert(path);
        }
    }

    /// 状态里当前勾上的各个文件。
    pub(super) fn checked_entries<'a>(
        &'a self,
        status: &'a Status,
    ) -> impl Iterator<Item = &'a Entry> {
        status.entries.iter().filter(|e| self.checked(e))
    }
}

/// 勾掉的一处改动：按内容（[`winc_core::diff::hunk_key`]）认，内容改过了再按它在 `HEAD` 里的
/// 行认。
///
/// `HEAD` 一侧不会因为编辑工作区而变，改动的内容变了也还认得出来，不会因为内容又改动就被
/// 重新算回要提交的改动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    key: u64,
    /// 在 `HEAD` 里的行；纯新增时是插入位置上的空区间。
    old: Range<usize>,
}

impl Skip {
    /// 一处改动（内容 `key`，在 `HEAD` 里的行 `old`）是不是勾掉的这一处。
    pub(super) fn matches(&self, key: u64, old: &Range<usize>) -> bool {
        if self.key == key {
            return true;
        }
        if self.old.is_empty() || old.is_empty() {
            // 纯新增：插入位置挨着或落在另一段里。
            self.old.start <= old.end && old.start <= self.old.end
        } else {
            self.old.start < old.end && old.start < self.old.end
        }
    }
}

/// 两个相对路径按文件视图的自然顺序比（逐级比名字，`2.png` 在 `10.png` 前面）。
fn natural_path_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    a.components()
        .zip(b.components())
        .map(|(x, y)| {
            let (x, y) = (
                x.as_os_str().to_string_lossy(),
                y.as_os_str().to_string_lossy(),
            );
            winc_editor::natural_cmp(&x, &y)
        })
        .find(|o| o.is_ne())
        .unwrap_or_else(|| a.components().count().cmp(&b.components().count()))
}

/// `items`（下标、相对路径）都在同一个子目录里、这一层没有文件时，返回那个子目录的名字。
fn only_subdir<'a>(items: &[(usize, &'a Path)]) -> Option<&'a OsStr> {
    let first = |p: &'a Path| {
        let mut parts = p.components();
        let first = parts.next()?.as_os_str();
        (!parts.as_path().as_os_str().is_empty()).then_some(first)
    };
    let next = first(items.first()?.1)?;
    items
        .iter()
        .all(|&(_, p)| first(p) == Some(next))
        .then_some(next)
}

/// 按自然顺序排的目录名，用作 `BTreeMap` 的键。
#[derive(PartialEq, Eq)]
struct NaturalName<'a>(&'a OsStr);

impl Ord for NaturalName<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        winc_editor::natural_cmp(&self.0.to_string_lossy(), &other.0.to_string_lossy())
            .then_with(|| self.0.cmp(other.0))
    }
}

impl PartialOrd for NaturalName<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
