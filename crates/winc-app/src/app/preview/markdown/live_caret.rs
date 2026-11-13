//! Markdown 预览中实时编辑的光标：源位置与预览位置的换算、水平与垂直移动、点击和拖选。

use super::*;

impl App {
    /// 可编辑的预览中文字光标的原文位置（字节偏移，按当前文字计算）。
    pub(crate) fn live_caret_source(&self) -> Option<usize> {
        let rope = self.editor.active()?.text().rope()?;
        let head = self.editor.active()?.selection.head;
        Some(rope.char_to_byte(head.min(rope.len_chars())))
    }

    /// 返回可编辑的预览中光标所在的叶子块，见 [`LiveLeaf`]；尚未解析或没有叶子块时为 `None`。
    ///
    /// 光标在第一个叶子块之前时取第一个。拖动选择文字时取按下处（选区的起点）所在的块：
    /// 拖到其他块上时该块不切换回排版，鼠标下方的内容不移位。
    pub(crate) fn live_leaf(&self) -> Option<LiveLeaf> {
        let doc = self.editor.active()?;
        let parsed = self.preview.markdown.parsed.as_ref()?;
        let rope = doc.text().rope()?;
        let len = rope.len_bytes();
        // 按住鼠标期间按按下之前的光标查找，显示原文的块和标记保持不变，鼠标下方的文字不移动；
        // 否则按下时该块切换为原文，文字移位，单击会变成选中一段。未开始编辑时全部按排版显示。
        let caret = match self.preview.markdown.live_drag {
            Some((.., before)) if self.live_dragging() => before?,
            _ => self.live_caret_source()?,
        };
        let delta = len as i64 - parsed.len as i64;
        let shift = |at: usize| usize::try_from(at as i64 + delta).unwrap_or(0).min(len);
        let i = parsed.leaves.partition_point(|l| l.range.start <= caret);
        let line = rope.byte_to_line(caret);
        let line_start = rope.line_to_byte(line);
        let line_end = rope
            .try_line_to_byte(line + 1)
            .map_or(len, |b| b.saturating_sub(1).max(caret))
            .min(len);
        let block_of = |at: usize| {
            parsed
                .ranges
                .iter()
                .rposition(|r| r.start <= at && at < r.end.max(r.start + 1))
        };
        let prev = i.checked_sub(1).map(|k| &parsed.leaves[k]);
        let next = parsed.leaves.get(i);
        let (found, leaf, gap) = match (prev.map(|l| &l.range), next.map(|l| &l.range)) {
            (Some(p), _) if caret <= shift(p.end).max(p.start) => (p, p.start..shift(p.end), false),
            // 光标在块的标记上（引用的 `>`、提示块的 `[!NOTE]`、列表的序号），尚未到达其第一个
            // 叶子块：取该叶子块，从光标所在行显示原文。
            (_, Some(n)) if block_of(n.start).is_some() && block_of(n.start) == block_of(caret) => {
                (n, line_start.min(n.start)..shift(n.end).max(n.start), false)
            }
            // 光标在叶子块之后的空行上：范围延伸到光标所在行，使光标在原文的空行上有位置。
            (Some(p), _) => (p, p.start..line_end, true),
            (None, Some(n)) => (n, n.start..shift(n.end).max(n.start), false),
            (None, None) => return None,
        };
        let kind = prev
            .into_iter()
            .chain(next)
            .find(|l| std::ptr::eq(&l.range, found));
        // 块之间的空行视为正文：回车另起一段。
        let prose = gap || kind.is_some_and(|l| l.prose);
        let table = !gap && kind.is_some_and(|l| l.table);
        let before = rope.byte_slice(line_start..caret.max(line_start));
        let blank = line_start > found.start
            && kind.is_some_and(|l| l.prose)
            && before.chars().all(char::is_whitespace);
        let block = block_of(found.start)?;
        let range = parsed.ranges[block].start..shift(parsed.ranges[block].end).max(leaf.end);
        // 输入之后、重新解析之前，范围来自上一版文档：起点可能已超出文档，也可能不在字符边界上；
        // 先将范围限制在当前文档内，之后才能按范围截取原文。
        let snap = |at: usize| rope.char_to_byte(rope.byte_to_char(at.min(len)));
        let leaf = snap(leaf.start)..snap(leaf.end).max(snap(leaf.start));
        let range = snap(range.start)..snap(range.end).max(snap(range.start));
        let last = rope.byte_to_line(leaf.end.min(len));
        let edge =
            (!prose && !table && (line <= rope.byte_to_line(leaf.start) || line + 1 >= last))
                .then_some(line);
        Some(LiveLeaf {
            block,
            range,
            leaf,
            parsed_end: found.end,
            revision: doc.revision,
            prose,
            table,
            blank,
            at: caret,
            edge,
        })
    }

    /// 是否正在可编辑的预览上拖动选择文字。
    pub(super) fn live_dragging(&self) -> bool {
        self.pointer.drag == crate::app::Drag::MarkdownText
            && self.preview.markdown.live_drag.is_some()
    }

    /// ←→ 移出光标所在的块、落到块之间的空行上时，跳过这些空行，到下一块的开头或上一块的
    /// 末尾：排版后的预览不显示空行，光标不停留在不可见的位置。
    ///
    /// 后方（前方）没有块时停在空行上。落到不显示的块标记（列表记号、引用的 `>`、续行的
    /// 缩进）中时同样跳过，见 [`App::live_visible`]。其余情形返回 `false`，交给编辑区逐字移动。
    pub(super) fn live_horizontal(&mut self, right: bool) -> bool {
        self.preview.markdown.line_end = None;
        let extend = self.pointer.modifiers.shift_key();
        let Some(live) = self.live_leaf() else {
            return false;
        };
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (sel, chars) = (doc.selection, rope.len_chars());
        if !sel.is_empty() && !extend {
            return false;
        }
        let next = match right {
            true if sel.head < chars => sel.head + 1,
            false if sel.head > 0 => sel.head - 1,
            _ => return false,
        };
        // 正文中没有空行，落到空行上即表示离开了该块；代码块中的空行是代码的一部分。
        let byte = rope.char_to_byte(next);
        let outside = live.prose
            || if right {
                byte >= live.leaf.end
            } else {
                byte < live.leaf.start
            };
        let blank = |line: usize| rope.line(line).chars().all(char::is_whitespace);
        let mut line = rope.char_to_line(next);
        let mut target = next;
        if outside && blank(line) {
            if right {
                while line < rope.len_lines() && blank(line) {
                    line += 1;
                }
                if line < rope.len_lines() {
                    target = rope.line_to_char(line);
                }
            } else {
                while blank(line) && line > 0 {
                    line -= 1;
                }
                if !blank(line) {
                    target = line_end(rope, line);
                }
            }
        }
        let target = self.live_visible(target, right);
        if target == next {
            return false;
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.goal_cell = None;
            doc.collapse_selections();
            let anchor = if extend { doc.selection.anchor } else { target };
            doc.selection = winc_core::Selection::new(anchor, target);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 原文第 `at` 个字节所在的叶子块是否为正文（段落、标题）；不在任何叶子块中时为
    /// `Some(false)`，解析结果不是当前版本时为 `None`。
    pub(super) fn live_prose_at(&self, at: usize) -> Option<bool> {
        let doc = self.editor.active()?;
        if self.preview.markdown.checked != Some(doc.revision) {
            return None;
        }
        let leaves = &self.preview.markdown.parsed.as_ref()?.leaves;
        let i = leaves.partition_point(|l| l.range.start <= at);
        Some(
            i.checked_sub(1)
                .map(|i| &leaves[i])
                .is_some_and(|l| l.prose && at <= l.range.end),
        )
    }

    /// 返回第 `line` 行开头在预览中不显示的块标记的长度（字符）。
    ///
    /// 叶子块的第一行取叶子块之前的部分（引用的 `>`、列表记号、任务框），正文的续行取引用
    /// 标记和缩进。解析结果不是当前版本时为 0。
    pub(super) fn live_hidden_prefix(&self, line: usize) -> usize {
        let Some(doc) = self.editor.active() else {
            return 0;
        };
        if self.preview.markdown.checked != Some(doc.revision) {
            return 0;
        }
        let (Some(parsed), Some(rope)) = (self.preview.markdown.parsed.as_ref(), doc.text().rope())
        else {
            return 0;
        };
        if line >= rope.len_lines() {
            return 0;
        }
        let start = rope.line_to_byte(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let i = parsed
            .leaves
            .partition_point(|l| l.range.start <= start + text.len());
        let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i]) else {
            return 0;
        };
        let hidden = if leaf.range.start >= start {
            leaf.range.start - start
        } else if leaf.prose && start <= leaf.range.end {
            winc_preview::markdown::enter::indent(text)
        } else {
            0
        };
        rope.byte_to_char((start + hidden).min(rope.len_bytes())) - rope.line_to_char(line)
    }

    /// 光标位置 `pos`（字符偏移）落在不显示的块标记中时，返回移动方向上最近的可见位置；
    /// 否则返回 `pos`。
    ///
    /// 向后（`forward`）时取该行内容的开头，向前时取上方最近一个非空行的末尾；上方没有内容时
    /// 留在该行内容的开头。
    pub(super) fn live_visible(&self, pos: usize, forward: bool) -> usize {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return pos;
        };
        let line = rope.char_to_line(pos.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let hidden = self.live_hidden_prefix(line);
        if pos - start >= hidden {
            return pos;
        }
        let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
        match (0..line).rev().find(|&l| !blank(l)) {
            Some(above) if !forward => line_end(rope, above),
            _ => start + hidden,
        }
    }

    /// 编辑区的命令移动光标之后调用：光标落在不显示的块标记中时移到可见位置。
    ///
    /// `before` 是移动之前光标的位置（字符偏移），据此判断移动方向。按词移动、移到行首等
    /// 命令按原文计算位置，结果可能落在列表记号或续行的缩进中。
    pub(in crate::app) fn live_settle_caret(&mut self, before: usize) {
        if !self.markdown_live() {
            return;
        }
        self.parse_markdown_inline();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let sel = doc.selection;
        if sel.head == before || !doc.extra_selections().is_empty() {
            return;
        }
        let head = self.live_visible(sel.head, sel.head > before);
        if head == sel.head {
            return;
        }
        if let Some(doc) = self.editor.active_mut() {
            let anchor = if sel.is_empty() { head } else { sel.anchor };
            doc.selection = winc_core::Selection::new(anchor, head);
        }
        self.request_redraw();
    }

    /// 将光标移到所在显示行的开头或末尾（`end`），按住 ⇧ 时扩展选区。
    ///
    /// 正文按预览中折行后的行计算，一段原文折成多行时不会移到整段的开头、末尾。光标位于
    /// 代码块、公式等按原文显示的块中时返回 `false`，按编辑区的方式处理。
    pub(super) fn live_line_edge(&mut self, end: bool) -> bool {
        if self.live_leaf().is_none_or(|l| !l.prose && !l.table) {
            return false;
        }
        let extend = self.pointer.modifiers.shift_key();
        let Some(source) = self.live_caret_source() else {
            return false;
        };
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return false;
        };
        let Some((_, y, h)) = laid.caret_point(source) else {
            return false;
        };
        // 横坐标取内容一栏之外：命中的是该行最左、最右的字缝。
        let x = if end {
            laid.left + laid.width() + self.preview.markdown.area.w
        } else {
            laid.left - self.preview.markdown.area.w
        };
        let sy = self.preview.markdown.area.y - self.preview.markdown.scroll() + y + h / 2.0;
        let Some(found) = crate::app::preview::markdown::laid::source_at(self, x, sy) else {
            return false;
        };
        // 折行处的位置按下一行的开头定位：纵坐标与当前行不同时，光标应画在本行的末尾。
        laid.set_upstream(None);
        let wrapped = end
            && laid
                .caret_point(found)
                .is_some_and(|(_, fy, _)| (fy - y).abs() >= 0.5);
        if wrapped {
            laid.set_upstream(Some(found));
        }
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let pos = rope.byte_to_char(found.min(rope.len_bytes()));
        let revision = doc.revision;
        doc.goal_cell = None;
        doc.collapse_selections();
        let anchor = if extend { doc.selection.anchor } else { pos };
        doc.selection = winc_core::Selection::new(anchor, pos);
        self.preview.markdown.line_end = wrapped.then_some((pos, revision));
        self.preview.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 原文位置 `source` 位于折行处时，判断窗口纵坐标 `y` 是否更接近上一显示行，并让排版结果
    /// 按该结论定位光标（见 `Laid::set_upstream`）。
    ///
    /// 折行处的位置同时是上一行的末尾和下一行的开头：在上一行的右端点击、从上方按 ↓ 移到
    /// 行尾时，光标应留在上一行。
    pub(super) fn live_row_affinity(&self, source: usize, y: f32) -> bool {
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return false;
        };
        let y = y - (self.preview.markdown.area.y - self.preview.markdown.scroll());
        let far = |p: Option<(f32, f32, f32)>| p.map(|(_, top, h)| (top + h / 2.0 - y).abs());
        laid.set_upstream(None);
        let below = laid.caret_point(source);
        laid.set_upstream(Some(source));
        let above = laid.caret_point(source);
        let upstream = match (far(above), far(below)) {
            (Some(a), Some(b)) => a + 0.5 < b,
            _ => false,
        };
        if !upstream {
            laid.set_upstream(None);
        }
        upstream
    }

    /// 可编辑的预览中光标应画在上一显示行末尾时，返回光标的原文位置（字节偏移），见
    /// `MarkdownPreview::line_end`。
    pub(crate) fn live_upstream(&self) -> Option<usize> {
        let doc = self.editor.active()?;
        let (pos, revision) = self.preview.markdown.line_end?;
        (doc.selection.head == pos && doc.revision == revision)
            .then(|| self.live_caret_source())
            .flatten()
    }

    /// 将光标向下（`down`）或向上移动一个显示行，或 `by` 逻辑像素；移出第一行、最后一行时
    /// 移到文档的开头、末尾。
    pub(super) fn live_vertical(&mut self, down: bool, by: Option<f32>) {
        let extend = self.pointer.modifiers.shift_key();
        let Some(source) = self.live_caret_source() else {
            return;
        };
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let Some((x, y, h)) = laid.caret_point(source) else {
            return;
        };
        let head = self.editor.active().map_or(0, |d| d.selection.head);
        let goal = match self.preview.markdown.goal {
            Some((goal, at)) if at == head => goal,
            _ => x,
        };
        let step = by.unwrap_or(h);
        let sign = if down { 1.0 } else { -1.0 };
        let height = laid.height();
        let screen = |laid: &crate::app::preview::markdown::laid::Laid, v: f32| {
            (
                laid.left + goal,
                self.preview.markdown.area.y - self.preview.markdown.scroll() + v,
            )
        };
        // 块间留白高于一行时，下一行的目标位置落在留白中，取到的最近一行仍是原来的行；
        // 因此每次再前进半行，直到到达其他行或超出内容范围。
        let mut target = y + h / 2.0 + sign * step;
        let mut moved = None;
        let mut upstream = false;
        while (0.0..=height).contains(&target) {
            let (sx, sy) = screen(laid, target);
            let found = crate::app::preview::markdown::laid::source_at(self, sx, sy);
            moved = found.filter(|&s| {
                upstream = self.live_row_affinity(s, sy);
                let point = self
                    .preview
                    .markdown
                    .layout
                    .as_ref()
                    .and_then(|l| l.caret_point(s));
                point.is_none_or(|(_, py, _)| (py - y).abs() >= 0.5)
            });
            if moved.is_some() || by.is_some() {
                break;
            }
            target += sign * h / 2.0;
        }
        // 文档以代码块、表格、分隔线等结束：再向下时在其后另起一段，光标有处可去。
        if moved.is_none() && down && !extend && self.live_append_paragraph() {
            return;
        }
        // 仍在原来的行上（已是第一行或最后一行）时，移到文档的开头或末尾。
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = match moved {
            Some(s) => rope.byte_to_char(s.min(rope.len_bytes())),
            None if down => rope.len_chars(),
            None => 0,
        };
        doc.goal_cell = None;
        doc.collapse_selections();
        let anchor = if extend { doc.selection.anchor } else { pos };
        doc.selection = winc_core::Selection::new(anchor, pos);
        let revision = doc.revision;
        self.preview.markdown.line_end = (moved.is_some() && upstream).then_some((pos, revision));
        self.preview.markdown.goal = Some((goal, pos));
        self.preview.markdown.selection = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
    }

    /// 处理可编辑预览上的鼠标按下：将文档的光标放到点中的字上；双击选中一个词，三击选中
    /// 原文的一行，按住 ⇧ 时将选区扩展到该处。
    pub(super) fn click_live(&mut self, x: f32, y: f32) {
        // 按下之前的光标位置：未开始编辑时为 `None`，按住鼠标期间整篇按排版显示。
        let editing = self.editor.active().map(|d| d.id) == self.preview.markdown.editing;
        let before = self.live_caret_source().filter(|_| editing);
        self.ime.preedit.clear();
        self.preview.markdown.selection = None;
        self.preview.markdown.goal = None;
        self.preview.markdown.line_end = None;
        self.request_redraw();
        let Some(source) = crate::app::preview::markdown::laid::source_at(self, x, y) else {
            // 没有可定位的文字（空文档）：仍然进入编辑，显示光标。
            self.start_live_editing();
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.pointer.modifiers.shift_key();
        let upstream = count == 1 && self.live_row_affinity(source, y);
        // 三击选中一行时不含预览中不显示的块标记：随后输入的文字不会替换掉列表记号。
        let hidden = self
            .editor
            .active()
            .and_then(|d| d.text().rope())
            .map_or(0, |rope| {
                let line = rope.byte_to_line(source.min(rope.len_bytes()));
                self.live_hidden_prefix(line)
            });
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        self.preview.markdown.line_end = upstream.then_some((pos, doc.revision));
        doc.goal_cell = None;
        doc.record_cursors();
        doc.collapse_selections();
        let (origin, by_word) = match count {
            2 => (winc_core::movement::word_at(doc.text(), pos), true),
            3 => {
                let line = doc.text().pos_to_line_col(pos).0;
                let span = doc.text().line_span(line);
                (
                    (span.start + hidden.min(span.end - span.start), span.end),
                    true,
                )
            }
            _ if shift => ((doc.selection.anchor, doc.selection.anchor), false),
            _ => ((pos, pos), false),
        };
        doc.selection = if by_word {
            winc_core::Selection::new(origin.0, origin.1)
        } else {
            winc_core::Selection::new(origin.0, pos)
        };
        self.preview.markdown.live_drag = Some((origin, by_word, before));
        self.pointer.drag = crate::app::Drag::MarkdownText;
        self.start_live_editing();
        self.reset_blink();
    }

    /// 处理可编辑预览上的拖动选择：选区从按下处延伸到鼠标下方的字。
    pub(super) fn drag_live(&mut self, x: f32, y: f32) {
        let Some((origin, by_word, _)) = self.preview.markdown.live_drag else {
            return;
        };
        let Some(source) = crate::app::preview::markdown::laid::source_at(self, x, y) else {
            return;
        };
        let upstream = !by_word && self.live_row_affinity(source, y);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let pos = rope.byte_to_char(source.min(rope.len_bytes()));
        self.preview.markdown.line_end = upstream.then_some((pos, doc.revision));
        doc.selection = if !by_word {
            winc_core::Selection::new(origin.0, pos)
        } else if pos < origin.0 {
            winc_core::Selection::new(origin.1, winc_core::movement::word_at(doc.text(), pos).0)
        } else {
            winc_core::Selection::new(origin.0, winc_core::movement::word_at(doc.text(), pos).1)
        };
        self.reset_blink();
    }

    /// 可编辑的预览在本帧是否将光标所在的块显示为原文。
    ///
    /// 在预览上点击、移动光标或输入之后才显示原文；刚打开、仅阅读时整篇均按排版显示，
    /// 不会有某一块突然切换为原文。排版之前调用。
    pub(crate) fn live_editing(&mut self) -> bool {
        if !self.markdown_live() {
            return false;
        }
        let (Some(source), Some(doc)) = (self.live_caret_source(), self.editor.active()) else {
            return false;
        };
        let now = (doc.id, source, doc.revision);
        if self
            .preview
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now)
        {
            self.preview.markdown.editing = Some(doc.id);
        }
        self.preview.markdown.editing == Some(doc.id)
    }

    /// 可编辑的预览中是否显示文字光标：在当前文档的预览上点击、移动光标或输入之后显示
    /// （见 [`App::live_editing`]）。
    pub(crate) fn live_caret_shown(&self) -> bool {
        let id = self.editor.active().map(|d| d.id);
        id.is_some() && self.preview.markdown.editing == id
    }

    /// 在可编辑的预览中开始编辑（点击、移动光标、组字）：光标所在的块显示原文。
    pub(in crate::app) fn start_live_editing(&mut self) {
        self.preview.markdown.editing = self.editor.active().map(|d| d.id);
    }

    /// 可编辑的预览中光标位置改变（输入、点击、移动、查找）后，滚动使光标可见并与边缘保留
    /// 一行的距离；光标未移动、仅滚动时不处理。
    ///
    /// 排版完成之后、绘制之前调用。
    pub(crate) fn follow_live_caret(&mut self) {
        let source = self
            .markdown_live()
            .then(|| self.live_caret_source())
            .flatten();
        let (Some(source), Some(doc)) = (source, self.editor.active()) else {
            self.preview.markdown.followed = None;
            return;
        };
        let now = (doc.id, source, doc.revision);
        let moved = self
            .preview
            .markdown
            .followed
            .is_some_and(|f| f.0 == doc.id && f != now);
        self.preview.markdown.followed = Some(now);
        if !moved || self.preview.markdown.editing != Some(doc.id) {
            return;
        }
        // 全选不滚动：选区的终点在文末，滚动过去会离开正在阅读的位置。
        let chars = doc.text().rope().map_or(0, ropey::Rope::len_chars);
        if doc.selection.from() == 0 && doc.selection.to() >= chars && chars > 0 {
            return;
        }
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let Some((_, y, h)) = laid.caret_point(source) else {
            return;
        };
        let md = &mut self.preview.markdown;
        let (scroll, view) = (md.scroll(), md.area.h);
        let margin = h.min(view / 4.0);
        if y - margin < scroll {
            md.scroll_to(y - margin);
        } else if y + h + margin > scroll + view {
            md.scroll_to(y + h + margin - view);
        }
    }
}
