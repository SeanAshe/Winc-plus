//! 差异标签中的逐行追溯：一个开关（行号栏、文字的右键菜单，命令「Git：逐行追溯」）同时开关
//! 两侧，每一侧追溯它记下的来源（见 `DiffSide::blame`），在那一侧的行号与中间的连接带之间多一栏。

use super::*;
use winc_editor::diff_view::{LEFT, RIGHT};

impl App {
    /// 差异 `id` 第 `side` 侧逐行追溯的键：「未提交的更改」中换进来的一段按段记，其余按标签记。
    pub(in crate::app) fn diff_blame_key(&self, id: DocumentId, side: usize) -> BlameKey {
        match self.combined_inside_id() {
            Some(section) => BlameKey::Section(id, section, side),
            None => BlameKey::Side(id, side),
        }
    }

    /// 开关当前差异标签的逐行追溯：两侧中能追溯的一起打开，开着时一起关闭。两侧都不是 Git 中的
    /// 版本（比较两个文件、合并冲突）时说明原因。
    pub(in crate::app) fn toggle_diff_blame(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        if !self.diff_blamable() {
            self.flash(t!("git-blame-diff-unavailable"));
            return;
        }
        let on = !view.blamed.contains(&true);
        let able = [&view.old, &view.new].map(|half| half.blame.is_some());
        if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) {
            view.blamed = able.map(|able| able && on);
        }
        for side in [LEFT, RIGHT] {
            if !on {
                self.git.blame.drop_key(self.diff_blame_key(id, side));
            }
            self.update_blame_of(self.diff_blame_key(id, side));
        }
        self.relayout();
        self.request_redraw();
    }

    /// 当前差异能否逐行追溯：至少一侧是 Git 中的版本。
    pub(in crate::app) fn diff_blamable(&self) -> bool {
        let view = self.editor.active().and_then(|d| d.diff.as_deref());
        view.is_some_and(|view| view.old.blame.is_some() || view.new.blame.is_some())
    }

    /// 窗口坐标 (`x`, `y`) 是否在差异一侧的行号栏或逐行追溯一栏上；在时返回差异标签。
    pub(in crate::app) fn diff_gutter_at(&self, x: f32, y: f32) -> Option<DocumentId> {
        let doc = self.editor.active()?;
        let view = doc.diff.as_deref()?;
        let parts = crate::app::diff_parts(self.layout.editor, view, self.metrics.cell);
        if y < parts.body.y {
            return None;
        }
        let over = |side: usize| {
            let pane = parts.panes[side];
            pane.gutter.contains(x, y) || pane.blame.contains(x, y)
        };
        (over(LEFT) || over(RIGHT)).then_some(doc.id)
    }

    /// 关闭差异一侧 `key` 的逐行追溯（追溯失败时）。
    pub(super) fn close_diff_blame(&mut self, key: BlameKey) {
        let BlameKey::Side(id, side) = key else {
            return;
        };
        self.git.blame.drop_key(key);
        let doc = self.editor.documents.iter_mut().find(|d| d.id == id);
        if let Some(view) = doc.and_then(|d| d.diff.as_deref_mut()) {
            view.blamed[side] = false;
        }
    }

    /// 在绘制前更新第 `index` 个文档（差异）两侧的逐行追溯，见 [`App::update_blame_of`]。
    pub(crate) fn update_diff_blame(&mut self, index: usize) {
        let Some(id) = self.editor.documents.get(index).map(|d| d.id) else {
            return;
        };
        for side in [LEFT, RIGHT] {
            self.update_blame_of(self.diff_blame_key(id, side));
        }
    }

    /// 当前差异第 `side` 侧最后修改第 `line` 行的提交。
    fn diff_blame_commit(&self, side: usize, line: usize) -> Option<BlameCommit> {
        let id = self.editor.active()?.id;
        let blame = self.blame_of_key(self.diff_blame_key(id, side))?;
        let index = *blame.lines.get(line)?;
        blame.commits.get(index as usize).cloned()
    }

    /// 悬停在当前差异第 `side` 侧逐行追溯第 `line` 行上时的提示。
    pub(crate) fn diff_blame_tip(&self, side: usize, line: usize) -> Option<String> {
        Some(blame_label(&self.diff_blame_commit(side, line)?))
    }

    /// 点击当前差异第 `side` 侧逐行追溯的第 `line` 行：在日志标签中选中对应的提交。
    pub(in crate::app) fn click_diff_blame(&mut self, side: usize, line: usize) {
        let Some(commit) = self.diff_blame_commit(side, line) else {
            return;
        };
        let view = self.editor.active().and_then(|d| d.diff.as_deref());
        let half = view.map(|v| v.side(side));
        let root = half.and_then(|h| h.blame.as_ref()).map(|b| b.root.clone());
        self.show_blamed_commit(&commit, root);
    }
}
