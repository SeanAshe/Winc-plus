//! 差异标签中未改动内容的折叠与展开，以及折叠、展开时的过渡。

use super::*;

impl App {
    /// 开始第 `gap` 段未改动内容展开（`expanding` 为真）或折叠的过渡，返回是否已开始。
    ///
    /// 调用时差异中该段须为展开状态。以下情况不做过渡：动态效果已关闭；该段无法折叠为一个
    /// 连续的区间；该段的起点不在编辑区顶部之下（顶部的行需要保持不动）。
    pub(super) fn start_diff_fold(&mut self, gap: usize, expanding: bool) -> bool {
        (self.clock(FOLD_ANIMATION))
            .is_some_and(|clock| self.begin_diff_fold(gap, expanding, clock))
    }

    /// 同 [`Self::start_diff_fold`]，过渡按给定的时钟 `clock` 计时；测试中使用固定时刻的时钟。
    pub(in crate::app) fn begin_diff_fold(
        &mut self,
        gap: usize,
        expanding: bool,
        clock: Clock,
    ) -> bool {
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(view) = doc.diff.as_deref().filter(|view| view.is_open(gap)) else {
            return false;
        };
        let Some(lines) = view.fold_lines(gap) else {
            return false;
        };
        let rows = [LEFT, RIGHT]
            .map(|side| view.row_of(side, lines[side].start)..view.row_of(side, lines[side].end));
        let top = view.row_at(RIGHT, doc.scroll_line.max(0.0));
        if rows[RIGHT].start as f64 <= top || rows[LEFT].len() != rows[RIGHT].len() {
            return false;
        }
        self.motion.diff_fold = Some(DiffFoldMotion {
            doc: doc.id,
            gap,
            rows,
            expanding,
            scroll: doc.scroll_line,
            clock,
        });
        self.request_redraw();
        true
    }

    /// 结束进行中的展开或折叠过渡；折叠的过渡此时把折叠写入差异。
    ///
    /// `force` 为假时只在过渡的时钟走完后结束，每帧绘制之前调用；为真时立即结束，在修改折叠
    /// 状态或处理点击之前调用，使后续操作按最终的排版进行。
    pub(in crate::app) fn finish_diff_fold(&mut self, force: bool) {
        let Some(motion) = self.motion.diff_fold.take_if(|m| force || m.clock.done()) else {
            return;
        };
        if motion.expanding {
            return;
        }
        if self.editor.active().is_some_and(|d| d.id == motion.doc) {
            self.refold_diff(|view, row| view.fold(motion.gap, row));
        } else if let Some(view) = (self.editor.documents.iter_mut())
            .find(|d| d.id == motion.doc)
            .and_then(|d| d.diff.as_deref_mut())
        {
            view.fold(motion.gap, 0);
        }
    }

    /// 在差异标签中执行折叠命令 `command`，作用于未改动的长段；返回当前标签是否为差异标签。
    ///
    /// 「全部折叠」「全部展开」作用于所有未改动的长段；折叠、展开和切换作用于只读光标所在的
    /// 一段，没有只读光标时作用于编辑区顶部往下的第一段。按层级折叠在差异中没有对应的操作。
    pub(in crate::app) fn fold_diff(&mut self, command: FoldCommand) -> bool {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return false;
        };
        let top = self.editor.active().map_or(0.0, |d| d.scroll_line.max(0.0));
        let top = view.row_at(RIGHT, top) as usize;
        let gap = match view.read_caret() {
            Some((side, pos)) => {
                let text = &view.side(side).text;
                view.gap_of(side, text.char_to_line(pos.min(text.len_chars())))
            }
            None => view.rows[RIGHT].iter().skip(top).find_map(|row| match row {
                SideRow::Fold { gap, .. } => Some(*gap),
                SideRow::Line { line, hunk, .. } => {
                    hunk.is_none().then(|| view.gap_of(RIGHT, *line)).flatten()
                }
            }),
        };
        let open = gap.is_some_and(|gap| view.is_open(gap));
        match (command, gap) {
            (FoldCommand::FoldAll, _) => self.refold_diff(DiffView::fold_all),
            (FoldCommand::UnfoldAll, _) => self.refold_diff(DiffView::unfold_all),
            (FoldCommand::Fold | FoldCommand::FoldRecursively | FoldCommand::Toggle, Some(gap))
                if open =>
            {
                self.press_diff(DiffTarget::Refold(gap))
            }
            (
                FoldCommand::Unfold | FoldCommand::UnfoldRecursively | FoldCommand::Toggle,
                Some(gap),
            ) => self.press_diff(DiffTarget::Fold(gap)),
            _ => {}
        }
        true
    }

    /// 以 `change` 修改当前差异标签的折叠状态，右侧顶部的行保持在顶部。
    ///
    /// `change` 的参数为差异视图和修改前右侧顶部的显示行，返回该行的内容在修改后所在的显示行。
    pub(super) fn refold_diff(&mut self, change: impl FnOnce(&mut DiffView, usize) -> usize) {
        self.finish_diff_fold(true);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if let Some(view) = doc.diff.as_deref_mut() {
            let row = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
            let row = change(view, row);
            let row = view.pos_of(RIGHT, row as f64);
            set_scroll_line(doc, row);
        }
        self.clamp_scroll();
        self.request_redraw();
    }
}
