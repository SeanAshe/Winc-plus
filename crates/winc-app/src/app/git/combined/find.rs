//! 「未提交的更改」中的查找：各段接成一份快照交给查找，选中的匹配交回它所在的那一段。

use super::*;

/// 查找用的快照：各段可查的一侧（右侧；右侧已删除时为左侧）依次接成的文字，段与段之间隔一个
/// 换行，以及每段在其中的起点。
pub(super) struct FindText {
    /// 按它接成时各段内容的代数（见 [`CombinedTab::generation`]）。
    generation: u64,
    /// 接成的文字。
    text: Rope,
    /// 每段：（第几段，查的哪一侧，在文字中的起点）。
    parts: Vec<(usize, usize, usize)>,
}

/// 查找快照交给查找时用的「一侧」编号的起点：与差异两侧的编号（0、1）分开，后面加上内容的
/// 代数，内容变了查找重新开始。
const FIND_SIDE: usize = 2;

impl App {
    /// 在「未提交的更改」中推进一个时间片的查找，返回（标签的标识，当前匹配的选区）；当前标签
    /// 不是它时返回 `None`。
    ///
    /// 查找的是各段接成的快照（见 [`FindText`]），段的内容变了重新接、重新开始查找。
    pub(in crate::app) fn update_combined_find(&mut self) -> Option<(DocumentId, Selection)> {
        if !self.combined_active() {
            return None;
        }
        let tab = self.git.combined.as_mut()?;
        if tab
            .find
            .as_ref()
            .is_none_or(|f| f.generation != tab.generation)
        {
            tab.find = Some(find_text(&tab.sections, tab.generation));
            tab.found = None;
        }
        let token = FIND_SIDE + tab.generation as usize;
        let selection = tab
            .found
            .map_or_else(Selection::default, |(a, b)| Selection::new(a, b));
        let tab = self.git.combined.as_ref()?;
        let doc = self.editor.document(tab.doc)?;
        if self.bars.search.snapshot_side(doc) != Some(token) {
            self.bars.find.pending_jump = None;
        }
        let text = &tab.find.as_ref()?.text;
        self.bars.search.update_snapshot(doc, token, text);
        Some((doc.id, selection))
    }

    /// 选中查找快照中 `start..end` 处的匹配：展开它所在的段和折叠的未更改内容，标为选区，看不见
    /// 时滚到视口上部三分之一处。
    pub(in crate::app) fn combined_reveal_match(&mut self, start: usize, end: usize) {
        // 匹配所在的段正在折叠：先结束过渡，下面再展开它。
        self.finish_combined_fold(true);
        let Some(tab) = self.git.combined.as_mut() else {
            return;
        };
        let Some(find) = tab.find.as_ref() else {
            return;
        };
        let at = find.parts.partition_point(|&(_, _, base)| base <= start);
        let Some(&(i, side, base)) = at.checked_sub(1).and_then(|at| find.parts.get(at)) else {
            return;
        };
        tab.found = Some((start, end));
        for section in &mut tab.sections {
            if let Body::Ready(view) = &mut section.body {
                view.read_selection = None;
            }
        }
        let section = &mut tab.sections[i];
        section.folded = false;
        let Body::Ready(view) = &mut section.body else {
            return;
        };
        let range = (start - base, end - base);
        view.read_selection = Some(winc_editor::diff_view::ReadSelection { side, range });
        let line = view
            .side(side)
            .text
            .char_to_line(range.0.min(view.side(side).text.len_chars()));
        view.reveal_line(side, line);
        let row = view.row_of(side, line) as f64;
        let map = view.unified_map();
        let local = if view.is_unified() {
            map.pos_of(side, row)
        } else {
            map.aligned_pos(side, row)
        };
        let pos = self.combined_starts()[i] + self.combined_header_rows() + local;
        let scroll = self.combined_scroll();
        let visible = f64::from(self.combined_parts().body.h / self.fonts.line_height);
        if pos < scroll || pos + 1.0 > scroll + visible {
            self.set_combined_scroll((pos - visible / 3.0).max(0.0));
        }
        self.request_redraw();
    }
}

/// 接成查找用的快照（见 [`FindText`]）：比较完的段按顺序接起来。
fn find_text(sections: &[Section], generation: u64) -> FindText {
    let mut text = Rope::new();
    let mut parts = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        let Body::Ready(view) = &section.body else {
            continue;
        };
        let side = if view.new.missing { LEFT } else { RIGHT };
        parts.push((i, side, text.len_chars()));
        let end = text.len_chars();
        text.insert(end, &view.side(side).text.to_string());
        let end = text.len_chars();
        text.insert(end, "\n");
    }
    FindText {
        generation,
        text,
        parts,
    }
}
