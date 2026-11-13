//! 改动列表：行、勾选、选中、滚动、固定在顶部的行和键盘。

use super::*;
use crate::app::ScrollTarget;

impl App {
    /// 提交视图列表现在的各行（见 [`CommitRows`]）。
    ///
    /// 按状态的版本（`GitState::generation`，换仓库时也变）和收起、平铺的样子缓存：每帧绘制、
    /// 滚动、悬停都要用，不再每次从头排。
    pub(crate) fn commit_rows(&self) -> Rc<CommitRows> {
        rows_of(
            &self.git.commit,
            self.git.status.as_ref(),
            self.git.root.as_deref(),
            self.git.generation,
        )
    }

    /// 状态里第 `i` 个文件；状态里没有这一项时为 `None`。
    pub(super) fn commit_entry(&self, i: usize) -> Option<&Entry> {
        self.git.status.as_ref()?.entries.get(i)
    }

    /// 切换第 `i` 个文件的勾选；该文件在多选的选区里时，选中的各个文件一起勾上或取消。
    pub(super) fn toggle_commit_check(&mut self, i: usize) {
        let Some(entry) = self.commit_entry(i).cloned() else {
            return;
        };
        let on = !self.git.commit.checked(&entry);
        if !(self.commit_multi_selected() && self.commit_is_selected(&entry.path)) {
            self.git.commit.set_checked(&entry, on);
            return;
        }
        let paths = self.commit_selection();
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let entries: Vec<Entry> = status
            .entries
            .iter()
            .filter(|e| paths.contains(&e.path))
            .cloned()
            .collect();
        for entry in &entries {
            self.git.commit.set_checked(entry, on);
        }
    }

    /// 只选中 `path`（清掉多选），在预览标签里打开它的差异；列表滚到让它露出来。
    ///
    /// 浏览历史提交的文件时（`commit.browse`），在该提交的差异标签中打开这个文件。
    pub(in crate::app::git) fn select_commit_file(&mut self, path: PathBuf) {
        self.git.commit.marked.clear();
        self.git.commit.anchor = Some(path.clone());
        self.git.commit.selected = Some(path.clone());
        self.reveal_commit_selection();
        if self.git.commit.browse {
            let index = (self.git.status.as_ref())
                .and_then(|status| status.entries.iter().position(|e| e.path == path));
            if let Some(index) = index {
                self.open_log_diff_file(index);
            }
            return;
        }
        self.git_preview_changes(&path);
    }

    /// 浏览历史提交的文件时点击了第 `i` 个文件：选中它，并在差异标签中打开。
    pub(super) fn browse_commit_file(&mut self, i: usize) {
        let Some(path) = self.commit_entry(i).map(|e| e.path.clone()) else {
            return;
        };
        self.focus = Focus::CommitList;
        self.diff.land = None;
        self.select_commit_file(path);
    }

    /// 列表里看得见的文件，按显示顺序。
    pub(crate) fn commit_files(&self) -> Vec<PathBuf> {
        let Some(status) = self.git.status.as_ref() else {
            return Vec::new();
        };
        self.commit_rows()
            .iter()
            .filter_map(|r| match r {
                CommitRow::File { index, .. } => Some(status.entries[*index].path.clone()),
                _ => None,
            })
            .collect()
    }

    /// 选中的各个文件，按显示顺序；没有选中时为空。
    pub(crate) fn commit_selection(&self) -> Vec<PathBuf> {
        let files = self.commit_files();
        if self.git.commit.marked.is_empty() {
            return files
                .into_iter()
                .filter(|p| self.git.commit.selected.as_ref() == Some(p))
                .collect();
        }
        let mut selection: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| self.git.commit.marked.contains(p))
            .collect();
        // 收起的目录里选中的文件（⌘、⇧ 单击目录选上的）排在后面，按状态里的顺序。
        if selection.len() < self.git.commit.marked.len()
            && let Some(status) = self.git.status.as_ref()
        {
            let shown: HashSet<&PathBuf> = selection.iter().collect();
            let hidden: Vec<PathBuf> = status
                .entries
                .iter()
                .filter(|e| self.git.commit.marked.contains(&e.path) && !shown.contains(&e.path))
                .map(|e| e.path.clone())
                .collect();
            selection.extend(hidden);
        }
        selection
    }

    /// 第 `row` 行（目录）下面的各个文件，收起的子目录里的也算；不是目录时为空。
    pub(crate) fn commit_dir_files(&self, row: usize) -> Vec<PathBuf> {
        let rows = self.commit_rows();
        let (Some(CommitRow::Dir { group, dir, .. }), Some(status)) =
            (rows.get(row), self.git.status.as_ref())
        else {
            return Vec::new();
        };
        rows.dir(*group, dir)
            .iter()
            .map(|&i| status.entries[i].path.clone())
            .collect()
    }

    /// 只选中 `files`（不打开预览），第一个成为当前行和扩展选区的起点。
    pub(crate) fn select_commit_files(&mut self, files: Vec<PathBuf>) {
        let first = files.first().cloned();
        self.git.commit.marked = files.into_iter().collect();
        self.git.commit.selected = first.clone();
        self.git.commit.anchor = first;
    }

    /// ⌘ 单击第 `row` 行（目录）：它下面的文件都已选中时一起移出选区，否则一起加入，第一个
    /// 成为当前行和扩展选区的起点。
    pub(super) fn toggle_commit_dir_selected(&mut self, row: usize) {
        let files = self.commit_dir_files(row);
        let Some(first) = files.first().cloned() else {
            return;
        };
        let commit = &mut self.git.commit;
        if commit.marked.is_empty()
            && let Some(current) = commit.selected.clone()
        {
            commit.marked.insert(current);
        }
        if files.iter().all(|f| commit.marked.contains(f)) {
            for file in &files {
                commit.marked.remove(file);
            }
            self.git.commit.selected = self.commit_selection().first().cloned();
            self.git.commit.anchor = self.git.commit.selected.clone();
        } else {
            commit.marked.extend(files);
            commit.selected = Some(first.clone());
            commit.anchor = Some(first);
        }
    }

    /// 多选时选中了几个文件；没有多选时为 0。
    pub(crate) fn commit_marked_count(&self) -> usize {
        self.git.commit.marked.len()
    }

    /// 文件 `path` 是否选中。
    pub(crate) fn commit_is_selected(&self, path: &Path) -> bool {
        if self.git.commit.marked.is_empty() {
            return self.git.commit.selected.as_deref() == Some(path);
        }
        self.git.commit.marked.contains(path)
    }

    /// 是否选中了不止一个文件（还在改动里的，收起的目录里的也算）。
    ///
    /// 和 `commit_selection().len() > 1` 一样，但不排序、不克隆路径：绘制时每帧都要问。
    pub(crate) fn commit_multi_selected(&self) -> bool {
        let Some(status) = self.git.status.as_ref() else {
            return false;
        };
        self.git.commit.marked.len() > 1
            && status
                .entries
                .iter()
                .filter(|e| self.git.commit.marked.contains(&e.path))
                .nth(1)
                .is_some()
    }

    /// 把 `path` 加入或移出选区（⌘ 单击）；加入时该文件成为当前行和扩展选区的起点。
    pub(super) fn toggle_commit_selected(&mut self, path: PathBuf) {
        let commit = &mut self.git.commit;
        if commit.marked.is_empty()
            && let Some(current) = commit.selected.clone()
        {
            commit.marked.insert(current);
        }
        if !commit.marked.remove(&path) {
            commit.marked.insert(path.clone());
        }
        commit.selected = Some(path.clone());
        commit.anchor = Some(path);
        if commit.marked.is_empty() {
            commit.selected = None;
        }
    }

    /// 选中从扩展选区的起点到 `path` 之间看得见的各个文件（⇧ 单击、⇧↑↓）；`path` 成为当前行。
    pub(super) fn select_commit_range(&mut self, path: PathBuf) {
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let row = self.commit_rows().iter().position(|r| match r {
            CommitRow::File { index, .. } => status.entries[*index].path == path,
            _ => false,
        });
        if let Some(row) = row {
            self.select_commit_rows(row);
        }
    }

    /// 选中从扩展选区的起点到第 `to` 行之间的各行：文件行是它自己，目录行是它下面的所有文件
    /// （收起的也算）。
    ///
    /// 第 `to` 行的文件（目录时是它下面的第一个）成为当前行。
    pub(super) fn select_commit_rows(&mut self, to: usize) {
        let rows = self.commit_rows();
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let file_at = |r: usize| match rows.get(r) {
            Some(CommitRow::File { index, .. }) => Some(status.entries[*index].path.clone()),
            _ => None,
        };
        let from = [&self.git.commit.anchor, &self.git.commit.selected]
            .into_iter()
            .flatten()
            .find_map(|a| (0..rows.len()).find(|&r| file_at(r).as_ref() == Some(a)))
            .unwrap_or(to);
        let mut marked = HashSet::new();
        for r in from.min(to)..=from.max(to) {
            match file_at(r) {
                Some(path) => {
                    marked.insert(path);
                }
                None => marked.extend(self.commit_dir_files(r)),
            }
        }
        let current = file_at(to).or_else(|| self.commit_dir_files(to).into_iter().next());
        let Some(current) = current else {
            return;
        };
        self.git.commit.anchor = file_at(from).or(Some(current.clone()));
        self.git.commit.marked = marked;
        self.git.commit.selected = Some(current);
        self.reveal_commit_selection();
    }

    /// 让选中的文件露在列表里：盖在固定的组、目录下面、或者已滚出列表时，滚到它正好露出来。
    pub(in crate::app) fn reveal_commit_selection(&mut self) {
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let rows = self.commit_rows();
        let selected = self.git.commit.selected.as_deref();
        let Some(i) = rows.iter().position(|r| match r {
            CommitRow::File { index, .. } => {
                Some(status.entries[*index].path.as_path()) == selected
            }
            _ => false,
        }) else {
            return;
        };
        let list = self.commit_parts().list;
        let (top, bottom) = (i as f32 * COMMIT_ROW, (i + 1) as f32 * COMMIT_ROW);
        // 只在固定行盖住该行时才滚：滚到该行正好露在它所在的组、目录（层数对应的行数，不超过
        // 能固定的层数）下面。已经露着的不动，单击列表不会跳、顶上固定的行也不会变。
        let covered = self
            .commit_sticky(&rows)
            .iter()
            .map(|s| s.rect.bottom() - list.y)
            .fold(0.0, f32::max);
        let before = self.git.commit.scroll;
        if top - self.git.commit.scroll < covered {
            let pinned = rows[i].level().min(self.commit_sticky_max()) as f32 * COMMIT_ROW;
            self.git.commit.scroll = (top - pinned).max(0.0);
        } else if top < self.git.commit.scroll {
            self.git.commit.scroll = top;
        } else if bottom > self.git.commit.scroll + list.h {
            self.git.commit.scroll = bottom - list.h;
        }
        self.glide_pane(ScrollTarget::Commit, before);
    }

    /// 第 `parent` 行的组或目录刚收起或展开时，开始展开或收起的动画。
    ///
    /// `before` 是变化之前的各行。子行从第 `parent` 行下方逐渐显示（或收回），其后的各行随之移动。
    ///
    /// 动态效果关闭时直接到位。
    pub(super) fn start_commit_motion(&mut self, before: Rc<CommitRows>, parent: usize) {
        let after = self.commit_rows();
        let expanding = after.len() > before.len();
        let count = after.len().abs_diff(before.len());
        if count == 0 {
            self.git.commit.motion = None;
            return;
        }
        // 动画期间按行数较多的状态画：展开时画展开后的行，收起时画收起前的行（收回的行还在）。
        let rows = if expanding { after } else { before };
        self.git.commit.motion = self
            .clock(crate::app::animation::TREE_ANIMATION)
            .map(|clock| CommitMotion {
                rows,
                parent,
                count,
                expanding,
                clock,
            });
    }

    /// 最多固定几层：[`STICKY_MAX`]，也不超过列表高度的四成。
    fn commit_sticky_max(&self) -> usize {
        let list = self.commit_parts().list;
        ((list.h * 0.4 / COMMIT_ROW).floor() as usize).min(STICKY_MAX)
    }

    /// 固定在列表顶部的组、目录，外层在前（规则与 `tree_sticky` 相同）。
    ///
    /// 逐层往下找：已经固定了 n 层时，看紧挨着这些层下面露出的那一行。它在第 n 层的上级（组或
    /// 目录）是下一个固定的；它自己就是第 n 层的组、目录、而且上沿已经滚到固定的位置之上时，
    /// 固定它自己。这一层最后一行的下沿滚到固定的位置之下时，它被往上推，下面不再固定别的。
    pub(crate) fn commit_sticky(&self, rows: &[CommitRow]) -> Vec<CommitSticky> {
        let list = self.commit_parts().list;
        let scroll = self.git.commit.scroll;
        let max = self.commit_sticky_max();
        let mut stack: Vec<CommitSticky> = Vec::new();
        while stack.len() < max {
            let level = stack.len();
            let top = list.y + level as f32 * COMMIT_ROW;
            let first = ((scroll + level as f32 * COMMIT_ROW) / COMMIT_ROW).floor() as usize;
            let Some(row) = rows.get(first) else {
                break;
            };
            let row_top = list.y + first as f32 * COMMIT_ROW - scroll;
            let index = if row.level() > level {
                match rows[..first].iter().rposition(|r| r.level() <= level) {
                    Some(i) if rows[i].level() == level && rows[i].is_container() => i,
                    _ => break,
                }
            } else if row.level() == level && row.is_container() && row_top < top - 0.5 {
                first
            } else {
                break;
            };
            let count = rows[index + 1..]
                .iter()
                .take_while(|r| r.level() > level)
                .count();
            if count == 0 {
                break;
            }
            let bottom = list.y + (index + count + 1) as f32 * COMMIT_ROW - scroll;
            let y = top.min(bottom - COMMIT_ROW);
            stack.push(CommitSticky {
                row: index,
                level,
                rect: Rect::new(list.x, y, list.w, COMMIT_ROW),
            });
            if y < top {
                break;
            }
        }
        stack
    }

    /// 返回选中文件在 `rows` 中的行号。
    ///
    /// 选中文件位于收起的目录（或组）中而不可见时，返回包含该文件的最内层收起目录（或组）的行号。
    /// 没有选中文件时为 `None`。
    fn commit_anchor(&self, rows: &CommitRows) -> Option<usize> {
        let (status, root) = (self.git.status.as_ref()?, self.git.root.as_deref()?);
        let selected = self.git.commit.selected.as_deref()?;
        let index = status.entries.iter().position(|e| e.path == selected)?;
        let entry = &status.entries[index];
        // 目录排在其子目录之前，因此最后一个包含该文件的行即最内层。
        rows.iter().rposition(|row| match row {
            CommitRow::File { index: i, .. } => *i == index,
            CommitRow::Group(group) => group.holds(entry),
            CommitRow::Dir { group, dir, .. } => {
                group.holds(entry) && entry.path.starts_with(root.join(dir))
            }
        })
    }

    /// 以动画收起或展开第 `row` 行（组或目录），滚动位置不超过新的末尾。
    /// 递归收起或展开第 `row` 行的目录（`⌥` 单击）：展开的目录连同各级子目录一起收起，收起的
    /// 目录展开到底。只改内存中的记录，不访问磁盘。
    pub(super) fn toggle_commit_dir_recursively(&mut self, row: usize) {
        let before = self.commit_rows();
        let (Some(CommitRow::Dir { group, dir, .. }), Some(status), Some(root)) = (
            before.get(row),
            self.git.status.as_ref(),
            self.git.root.as_deref(),
        ) else {
            return;
        };
        let collapse = !self.git.commit.dir_collapsed(*group, dir);
        self.git
            .commit
            .set_dir_tree_collapsed(status, root, (*group, dir), collapse);
        self.start_commit_motion(Rc::clone(&before), row);
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
    }

    pub(super) fn toggle_commit_row(&mut self, row: usize) {
        let before = self.commit_rows();
        match before.get(row) {
            Some(CommitRow::Group(group)) => self.git.commit.toggle_group(*group),
            Some(CommitRow::Dir { group, dir, .. }) => {
                self.git.commit.toggle_dir(*group, dir.clone());
            }
            _ => return,
        }
        self.start_commit_motion(before, row);
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
    }

    /// 处理改动列表中的按键。
    ///
    /// `↑` `↓` 选择并预览，`Home` `End` 移到第一个、最后一个文件，`PageUp` `PageDown` 翻页，
    /// 以上按键配合 `⇧` 时扩展选区；`←` `→` 收起、展开选中文件所在的目录；空格切换勾选；
    /// `⌫` 回滚选中的文件；回车打开文件；`Tab` 移到提交信息；`Esc` 先缩小为单选，再回到编辑区。
    pub(in crate::app) fn on_key_commit_list(&mut self, event: &KeyEvent) {
        self.commit_list_key(&event.logical_key);
    }

    /// [`App::on_key_commit_list`] 按下的是 `key` 时做的事。
    ///
    /// 浏览历史提交的文件时只处理移动和展开收起：勾选、回滚、打开工作区文件都不适用。
    pub(in crate::app) fn commit_list_key(&mut self, key: &Key) {
        if self.git.commit.browse
            && matches!(
                key,
                Key::Named(
                    NamedKey::Space
                        | NamedKey::Backspace
                        | NamedKey::Delete
                        | NamedKey::Enter
                        | NamedKey::Tab
                )
            )
        {
            return;
        }
        let rows = self.commit_rows();
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        // 各文件在 `rows` 中的行号及其在状态中的下标。
        let files: Vec<(usize, usize)> = rows
            .iter()
            .enumerate()
            .filter_map(|(r, row)| match row {
                CommitRow::File { index, .. } => Some((r, *index)),
                _ => None,
            })
            .collect();
        let selected = self.git.commit.selected.as_deref();
        let at = files
            .iter()
            .position(|&(_, i)| Some(status.entries[i].path.as_path()) == selected);
        // 选中文件所在的行；文件被收起而不可见时为收起它的目录（或组）所在的行。
        let anchor = self.commit_anchor(&rows);
        // 要用到的那个文件的路径先取出来，下面改 `self` 时不再借着状态。
        let path = |i: usize| files.get(i).map(|&(_, f)| status.entries[f].path.clone());
        // 翻页距离为可见整行数减一，保留一行上下文；落在该行或其后第一个文件上，
        // 跳过组、目录行，到达两端时停在第一个、最后一个文件。
        let page = ((self.commit_parts().list.h / COMMIT_ROW).floor() as usize)
            .saturating_sub(1)
            .max(1);
        let last = files.len().checked_sub(1);
        let paged = |i: usize, down: bool| {
            let row = files[i].0;
            if down {
                let goal = row + page;
                files.iter().position(|&(r, _)| r >= goal).or(last)
            } else {
                let goal = row.saturating_sub(page);
                files.iter().rposition(|&(r, _)| r <= goal).or(Some(0))
            }
        };
        let target = match (key, at) {
            // 选中文件已被收起：从收起它的行开始向下或向上查找。
            (Key::Named(NamedKey::ArrowDown), None) if anchor.is_some() => files
                .iter()
                .position(|&(r, _)| Some(r) > anchor)
                .and_then(path),
            (Key::Named(NamedKey::ArrowUp), None) if anchor.is_some() => files
                .iter()
                .rposition(|&(r, _)| Some(r) < anchor)
                .and_then(path),
            (Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown), None) => path(0),
            (Key::Named(NamedKey::ArrowDown), Some(i)) => path(i + 1),
            (Key::Named(NamedKey::ArrowUp), Some(i)) => i.checked_sub(1).and_then(path),
            (Key::Named(NamedKey::Home), _) => path(0),
            (Key::Named(NamedKey::End), _) => last.and_then(path),
            (Key::Named(NamedKey::PageUp | NamedKey::PageDown), None) => path(0),
            (Key::Named(NamedKey::PageDown), Some(i)) => paged(i, true).and_then(path),
            (Key::Named(NamedKey::PageUp), Some(i)) => paged(i, false).and_then(path),
            (Key::Named(NamedKey::Enter), Some(i)) => path(i),
            _ => None,
        };

        match key {
            Key::Named(
                NamedKey::ArrowUp
                | NamedKey::ArrowDown
                | NamedKey::Home
                | NamedKey::End
                | NamedKey::PageUp
                | NamedKey::PageDown,
            ) => {
                if let Some(next) = target {
                    if self.pointer.modifiers.shift_key() && !self.git.commit.browse {
                        self.select_commit_range(next);
                    } else {
                        self.select_commit_file(next);
                    }
                }
            }
            // `←` 收起选中文件所在的目录，再次按下时收起上一级；`→` 逐级展开，直到文件重新可见。
            Key::Named(NamedKey::ArrowLeft) => {
                let parent = anchor.and_then(|a| {
                    let level = rows[a].level();
                    (0..a).rev().find(|&r| rows[r].level() < level)
                });
                if let Some(row) = parent {
                    self.toggle_commit_row(row);
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if let Some(row) = anchor.filter(|_| at.is_none()) {
                    self.toggle_commit_row(row);
                    self.reveal_commit_selection();
                }
            }
            Key::Named(NamedKey::Space) => {
                if let Some(i) = at {
                    self.toggle_commit_check(files[i].1);
                }
            }
            // `⌫` 确认后回滚选中的文件。
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => {
                let paths = self.commit_selection();
                if !paths.is_empty() {
                    self.confirm_rollback(paths);
                }
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(path) = target
                    && winc_backend::fs::is_file(&path)
                {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                }
            }
            // 没有提交卡片（没有更改）时提交信息不在屏幕上，焦点留在列表里。
            Key::Named(NamedKey::Tab) if self.commit_compose_shown() => {
                self.focus = Focus::CommitMessage;
            }
            Key::Named(NamedKey::Escape) if self.commit_multi_selected() => {
                if let Some(path) = self.git.commit.selected.clone() {
                    self.select_commit_file(path);
                }
            }
            Key::Named(NamedKey::Escape) if self.editor.active.is_some() => {
                self.focus = Focus::Editor;
            }
            _ => {}
        }
        self.request_redraw();
    }
}

/// 按 `state` 的收起、平铺设置排出 `status` 的各行，按（`generation`，排列设置的版本）缓存在
/// `state` 中；没有状态或仓库时返回空的列表。
pub(in crate::app::git) fn rows_of(
    state: &CommitState,
    status: Option<&Status>,
    root: Option<&Path>,
    generation: u64,
) -> Rc<CommitRows> {
    let (Some(status), Some(root)) = (status, root) else {
        return Rc::default();
    };
    let key = (generation, state.layout);
    let mut cache = state.rows.borrow_mut();
    if let Some((built, rows)) = cache.as_ref()
        && *built == key
    {
        return Rc::clone(rows);
    }
    let rows = Rc::new(state.build_rows(status, root));
    *cache = Some((key, Rc::clone(&rows)));
    rows
}
