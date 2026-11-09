//! 勾选和折叠的状态，以及按目录分层排好的改动列表。

use winc_core::diff::{LineSet, Pick};
use winc_editor::diff_view::LEFT;

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
    /// 提交的是工作区里的内容，与暂存区无关：在终端里暂存过的文件与其他文件的处理相同。
    pub fn file_state(&self, entry: &Entry) -> Option<bool> {
        if !self.checked(entry) {
            None
        } else if self.skips_hunks(entry) {
            Some(false)
        } else {
            Some(true)
        }
    }

    /// 文件 `entry` 是否在差异里勾掉了其中几处改动。
    pub fn skips_hunks(&self, entry: &Entry) -> bool {
        self.skipped.get(&entry.path).is_some_and(|s| !s.is_empty())
    }

    /// 返回文件 `entry` 现在各段改动的提交方式，下标同 `all`（各段改动的键和在 `HEAD` 中的行
    /// 区间）。
    pub fn hunk_inclusions(&self, entry: &Entry, all: &[(u64, Range<usize>)]) -> Vec<Pick> {
        if !self.checked(entry) {
            return vec![Pick::Skip; all.len()];
        }
        let skips = self.skipped.get(&entry.path).map_or(&[][..], Vec::as_slice);
        picks(skips, all)
    }

    /// 勾上或勾掉文件 `entry` 的第 `hunk` 处改动；`all` 是该文件现在所有改动的键和在 `HEAD` 中的
    /// 行区间。
    ///
    /// 文件没勾时勾上一处：文件勾上，只提交这一处；一处都不剩时文件也勾掉。
    pub fn toggle_hunk(&mut self, entry: &Entry, hunk: usize, all: &[(u64, Range<usize>)]) {
        if hunk >= all.len() {
            return;
        }
        if !self.checked(entry) {
            self.set_checked(entry, true);
            let others = (all.iter().enumerate())
                .filter(|(h, _)| *h != hunk)
                .map(|(_, change)| Skip::whole(change))
                .collect();
            self.skipped.insert(entry.path.clone(), others);
            return;
        }
        let mut fresh = self.refreshed_skips(entry, all);
        fresh[hunk] = match fresh[hunk] {
            Some(_) => None,
            None => Some(Skip::whole(&all[hunk])),
        };
        self.store_skips(entry, fresh);
    }

    /// 勾上或勾掉文件 `entry` 的第 `hunk` 处改动中的一行。
    ///
    /// `new_len` 是这处改动的新行数，`line` 是（侧，相对这处改动开头的行号），`all` 同
    /// [`CommitState::toggle_hunk`]。文件没勾时勾上一行：文件勾上，只提交这一行。
    pub fn toggle_line(
        &mut self,
        entry: &Entry,
        (hunk, new_len): (usize, usize),
        (side, line): (usize, usize),
        all: &[(u64, Range<usize>)],
    ) {
        let Some((key, old)) = all.get(hunk).cloned() else {
            return;
        };
        let total = old.len() + new_len;
        let except = || {
            let mut set = full_set(old.len(), new_len);
            toggle_in(&mut set, side, line);
            set
        };
        if !self.checked(entry) {
            self.set_checked(entry, true);
            let mut fresh: Vec<Skip> = (all.iter().enumerate())
                .filter(|(h, _)| *h != hunk)
                .map(|(_, change)| Skip::whole(change))
                .collect();
            fresh.push(Skip::with_lines(key, &old, except(), total));
            fresh.retain(|s| !s.is_empty());
            self.skipped.insert(entry.path.clone(), fresh);
            return;
        }
        let mut fresh = self.refreshed_skips(entry, all);
        let lines = match fresh[hunk].take() {
            Some(Skip {
                lines: Some(mut set),
                ..
            }) => {
                toggle_in(&mut set, side, line);
                set
            }
            Some(_) => except(),
            None => {
                let mut set = LineSet::default();
                toggle_in(&mut set, side, line);
                set
            }
        };
        fresh[hunk] = Some(Skip::with_lines(key, &old, lines, total)).filter(|s| !s.is_empty());
        self.store_skips(entry, fresh);
    }

    /// 返回文件 `entry` 已勾掉的改动在现在的 `all` 上的记录，下标同 `all`（见 [`assign`]）。
    ///
    /// 内容相同的改动沿用原来的记录；内容变了的改动行号不再对应，按整处勾掉记录，不会被算回要
    /// 提交的改动；认不出的记录被丢弃。
    fn refreshed_skips(&self, entry: &Entry, all: &[(u64, Range<usize>)]) -> Vec<Option<Skip>> {
        let skips = self.skipped.get(&entry.path).map_or(&[][..], Vec::as_slice);
        (assign(skips, all).into_iter().zip(all))
            .map(|(found, (key, old))| {
                found.map(|found| {
                    if found.key == *key {
                        Skip {
                            old: old.clone(),
                            ..found.clone()
                        }
                    } else {
                        Skip::whole(&(*key, old.clone()))
                    }
                })
            })
            .collect()
    }

    /// 记下文件 `entry` 勾掉的改动（下标同各段改动）；所有改动都整处勾掉时文件也勾掉。
    fn store_skips(&mut self, entry: &Entry, skips: Vec<Option<Skip>>) {
        let none_left = skips
            .iter()
            .all(|s| s.as_ref().is_some_and(|s| s.lines.is_none()));
        self.skipped
            .insert(entry.path.clone(), skips.into_iter().flatten().collect());
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

    /// 展开 `dir`（相对仓库根目录）及其各级上级目录，以及所在的组，使其中的文件可见。
    pub(crate) fn expand_to(&mut self, dir: &Path) {
        for i in 0..CommitGroup::COUNT {
            self.collapsed[i] = false;
            self.collapsed_dirs[i].retain(|collapsed| !dir.starts_with(collapsed));
        }
        self.layout += 1;
    }

    /// 设定是否把只有一个子目录、没有文件的各级目录合成一行。
    pub(crate) fn set_compact(&mut self, compact: bool) {
        if self.compact != compact {
            self.compact = compact;
            self.layout += 1;
        }
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
    pub(in crate::app::git) fn build_rows(&self, status: &Status, root: &Path) -> CommitRows {
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
            // 只有一个子目录、没有文件：按设置和子目录合成一行。
            let mut label = PathBuf::from(name);
            while let Some(next) = only_subdir(&sub).filter(|_| self.compact) {
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
    /// 只勾掉其中几行时的这些行；`None` 表示整处勾掉。行号只在内容等于 `key` 时有效。
    lines: Option<LineSet>,
}

/// 把 `side` 侧第 `line` 行（相对改动开头）加入或移出 `set`。
fn toggle_in(set: &mut LineSet, side: usize, line: usize) {
    let lines = if side == LEFT {
        &mut set.old
    } else {
        &mut set.new
    };
    if !lines.remove(&line) {
        lines.insert(line);
    }
}

/// 返回包含一处改动全部行的集合。
fn full_set(old: usize, new: usize) -> LineSet {
    LineSet {
        old: (0..old).collect(),
        new: (0..new).collect(),
    }
}

impl Skip {
    /// 创建整处勾掉的记录。
    fn whole((key, old): &(u64, Range<usize>)) -> Skip {
        Skip {
            key: *key,
            old: old.clone(),
            lines: None,
        }
    }

    /// 创建勾掉 `lines` 的记录：`total` 是这处改动的总行数，全部勾掉时按整处记录。
    fn with_lines(key: u64, old: &Range<usize>, lines: LineSet, total: usize) -> Skip {
        let whole = lines.old.len() + lines.new.len() == total;
        Skip {
            key,
            old: old.clone(),
            lines: (!whole).then_some(lines),
        }
    }

    /// 是否没有勾掉任何行。
    fn is_empty(&self) -> bool {
        self.lines
            .as_ref()
            .is_some_and(|set| set.old.is_empty() && set.new.is_empty())
    }

    /// 返回内容为 `key` 的改动在这条记录下的提交方式。
    pub(super) fn pick(&self, key: u64) -> Pick {
        match &self.lines {
            Some(lines) if self.key == key => Pick::Lines(lines.clone()),
            _ => Pick::Skip,
        }
    }

    /// 内容变了的一处改动（在 `HEAD` 里的行 `old`）和这条记录的位置是否重叠：重叠时视为同一处。
    fn overlaps(&self, old: &Range<usize>) -> bool {
        if self.old.is_empty() || old.is_empty() {
            // 纯新增：插入位置挨着或落在另一段里。
            self.old.start <= old.end && old.start <= self.old.end
        } else {
            self.old.start < old.end && old.start < self.old.end
        }
    }
}

/// 把勾掉的记录 `skips` 对应到现在的各段改动 `all`（键、在 `HEAD` 中的行区间）上，返回每段
/// 改动对应的记录，下标同 `all`；一条记录最多对应一段改动。
///
/// 内容相同的改动可能有几处（例如两处加入了同一行），键相同，只按键对应会把一条记录同时算到这
/// 几处上：勾掉一处时几处一起显示为勾掉，再勾上时只去掉其中一份，无法勾回。因此依次按三种条件
/// 对应，每次只用还没对应上的记录：键和位置都相同；键相同、位置最近（上方增删了行时位置会变）；
/// 内容变了、位置重叠（见 [`Skip::overlaps`]）。
pub(super) fn assign<'a>(skips: &'a [Skip], all: &[(u64, Range<usize>)]) -> Vec<Option<&'a Skip>> {
    let mut taken = vec![false; skips.len()];
    let mut found: Vec<Option<usize>> = vec![None; all.len()];
    for round in 0..3 {
        for (h, (key, old)) in all.iter().enumerate() {
            if found[h].is_some() {
                continue;
            }
            let mut free = (0..skips.len()).filter(|&i| !taken[i]);
            let pick = match round {
                0 => free.find(|&i| skips[i].key == *key && skips[i].old == *old),
                1 => free
                    .filter(|&i| skips[i].key == *key)
                    .min_by_key(|&i| skips[i].old.start.abs_diff(old.start)),
                _ => free.find(|&i| skips[i].key != *key && skips[i].overlaps(old)),
            };
            if let Some(i) = pick {
                taken[i] = true;
                found[h] = Some(i);
            }
        }
    }
    found.into_iter().map(|i| i.map(|i| &skips[i])).collect()
}

/// 按勾掉的记录 `skips` 返回各段改动 `all` 的提交方式，下标同 `all`（见 [`assign`]）。
pub(super) fn picks(skips: &[Skip], all: &[(u64, Range<usize>)]) -> Vec<Pick> {
    (assign(skips, all).into_iter().zip(all))
        .map(|(skip, (key, _))| skip.map_or(Pick::Apply, |skip| skip.pick(*key)))
        .collect()
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
