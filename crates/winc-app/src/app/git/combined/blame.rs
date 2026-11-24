//! 「未提交的更改」中各段的逐行追溯：按段记结果，开关时各段一起开关。

use super::*;

impl App {
    /// 「未提交的更改」标签（文档 `doc`）中编号为 `section` 的一段的差异；没有时为 `None`。
    pub(crate) fn combined_view(&self, doc: DocumentId, section: u32) -> Option<&DiffView> {
        let tab = self.combined_tab().filter(|t| t.doc == doc)?;
        match &tab.sections.iter().find(|s| s.id == section)?.body {
            Body::Ready(view) => Some(view),
            _ => None,
        }
    }

    /// 第 `i` 段第 `side` 侧逐行追溯的键。
    fn combined_blame_key(&self, i: usize, side: usize) -> Option<BlameKey> {
        let tab = self.combined_tab()?;
        Some(BlameKey::Section(tab.doc, tab.sections.get(i)?.id, side))
    }

    /// 开关各段的逐行追溯（命令「Git：逐行追溯」）：能追溯的一侧一起打开，开着时一起关闭，
    /// 同差异标签。没有能追溯的段时说明原因。
    pub(in crate::app) fn toggle_combined_blame(&mut self) {
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let views = || {
            tab.sections.iter().filter_map(|s| match &s.body {
                Body::Ready(view) => Some(view),
                _ => None,
            })
        };
        if !views().any(|v| v.old.blame.is_some() || v.new.blame.is_some()) {
            self.flash(t!("git-blame-diff-unavailable"));
            return;
        }
        let on = !views().any(|v| v.blamed.contains(&true));
        let doc = tab.doc;
        let mut dropped = Vec::new();
        for section in &mut tab.sections {
            if let Body::Ready(view) = &mut section.body {
                let able = [view.old.blame.is_some(), view.new.blame.is_some()];
                view.blamed = able.map(|able| able && on);
                dropped.extend([LEFT, RIGHT].map(|side| BlameKey::Section(doc, section.id, side)));
            }
        }
        if !on {
            for key in dropped {
                self.git.blame.drop_key(key);
            }
        }
        self.request_redraw();
    }

    /// 绘制前更新第 `i` 段两侧的逐行追溯（开着时），见 [`App::update_blame_of`]。
    pub(crate) fn update_combined_blame(&mut self, i: usize) {
        for side in [LEFT, RIGHT] {
            if let Some(key) = self.combined_blame_key(i, side) {
                self.update_blame_of(key);
            }
        }
    }

    /// 一段一侧 `key` 的逐行追溯失败了：关掉这一侧。
    pub(in crate::app::git) fn close_combined_blame(&mut self, key: BlameKey) {
        let BlameKey::Section(_, id, side) = key else {
            return;
        };
        self.git.blame.drop_key(key);
        let sections = self
            .git
            .combined
            .as_mut()
            .map_or(&mut [][..], |t| &mut t.sections[..]);
        if let Some(Section {
            body: Body::Ready(view),
            ..
        }) = sections.iter_mut().find(|s| s.id == id)
        {
            view.blamed[side] = false;
        }
    }

    /// 第 `i` 段第 `side` 侧最后修改第 `line` 行的提交，以及它所在的仓库。
    fn combined_blame_commit(
        &self,
        i: usize,
        side: usize,
        line: usize,
    ) -> Option<(BlameCommit, PathBuf)> {
        let blame = self.blame_of_key(self.combined_blame_key(i, side)?)?;
        let index = *blame.lines.get(line)?;
        let commit = blame.commits.get(index as usize)?.clone();
        let Body::Ready(view) = &self.combined_tab()?.sections.get(i)?.body else {
            return None;
        };
        let root = view.side(side).blame.as_ref()?.root.clone();
        Some((commit, root))
    }

    /// 悬停在第 `i` 段逐行追溯的一格上时的提示。
    pub(crate) fn combined_blame_tip(&self, i: usize, side: usize, line: usize) -> Option<String> {
        Some(blame_label(&self.combined_blame_commit(i, side, line)?.0))
    }
}
