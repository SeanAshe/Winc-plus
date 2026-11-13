//! Markdown 预览中的实时编辑：按键、回车、退格、列表缩进、围栏代码块和表格工具。
//!
//! 编辑落在源文档上，预览随之重新解析；光标的定位与移动在 `live_caret` 中。

use super::*;

impl App {
    /// Markdown 中选中一段文字（一行之内）后粘贴网址 `content`：返回替换选区的链接
    /// `[文字](网址)`；不应生成链接时为 `None`，按普通粘贴处理。
    ///
    /// 以下情况不生成链接：选中的文字本身是网址（视为替换网址）、选区位于代码块或行内代码中
    /// （其中的链接语法不生效）、有多个选区。
    pub(in crate::app) fn markdown_link_paste(&self, content: &str) -> Option<String> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(PreviewKind::Markdown) || !doc.extra_selections().is_empty() {
            return None;
        }
        let url = content.trim();
        let is_url = |s: &str| {
            ["https://", "http://", "mailto:"]
                .iter()
                .any(|p| s.len() > p.len() && s.starts_with(p))
                && !s.contains(char::is_whitespace)
        };
        if !is_url(url) {
            return None;
        }
        let rope = doc.text().rope()?;
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let selected = rope.slice(from..to).to_string();
        if selected.is_empty() || selected.contains(['\n', '`']) || is_url(selected.trim()) {
            return None;
        }
        // 位于行内代码中：该行选区之前的反引号数量为奇数。
        let (from, line) = (rope.char_to_byte(from), rope.char_to_line(from));
        let before = rope.byte_slice(rope.line_to_byte(line)..from).to_string();
        if before.matches('`').count() % 2 == 1 {
            return None;
        }
        // 判断是否位于代码块、公式、HTML 块中时使用当前版本的解析结果；没有该结果时
        // （未显示预览或尚未解析到当前版本）只检查围栏代码块，统计之前的围栏行。
        if self.preview.markdown.checked == Some(doc.revision)
            && let Some(parsed) = self.preview.markdown.parsed.as_ref()
        {
            let i = parsed.leaves.partition_point(|l| l.range.start <= from);
            if let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i])
                && leaf.range.contains(&from)
                && !leaf.prose
            {
                return None;
            }
        } else if in_fence(rope.lines_at(0).take(line)) {
            return None;
        }
        // 网址含括号时用尖括号包裹，避免括号被视为链接的结尾。
        let url = if url.contains(['(', ')']) {
            format!("<{url}>")
        } else {
            url.to_owned()
        };
        Some(format!("[{selected}]({url})"))
    }

    /// 可编辑的预览中 ↑↓ 按显示行移动光标（加 ⇧ 扩展选区），`PageUp` / `PageDown` 移动一屏；
    /// 正文里的回车、⇧ 回车见 [`App::live_enter`]。
    ///
    /// 其余的键返回 `false`，交给编辑区按普通编辑处理（输入、删除、←→、⌘↑↓ 等都作用于文档）。
    pub(super) fn live_key(&mut self, key: &Key, cmd: bool, area: winc_render::Rect) -> bool {
        self.parse_markdown_inline();
        // macOS 上 ⌘← / ⌘→ 与 Home / End 相同，移到显示行的开头、末尾。
        if cfg!(target_os = "macos") && cmd && !self.pointer.modifiers.alt_key() {
            match key {
                Key::Named(NamedKey::ArrowLeft) => return self.live_line_edge(false),
                Key::Named(NamedKey::ArrowRight) => return self.live_line_edge(true),
                _ => {}
            }
        }
        // 代码块、公式中 ⌘↩ 在块之后另起一段。
        if cmd
            && !self.pointer.modifiers.alt_key()
            && !self.pointer.modifiers.shift_key()
            && *key == Key::Named(NamedKey::Enter)
            && self.live_leave_fenced()
        {
            return true;
        }
        // 表格中 ⌘↩ 在下方插入一行，⌘⇧⌫ 删除当前行。
        if cmd && !self.pointer.modifiers.alt_key() && self.live_leaf().is_some_and(|l| l.table) {
            let shift = self.pointer.modifiers.shift_key();
            match key {
                Key::Named(NamedKey::Enter) if !shift => return self.live_table_tool("row"),
                Key::Named(NamedKey::Backspace) if shift => {
                    return self.live_table_tool("delete-row");
                }
                _ => {}
            }
        }
        if cmd || self.pointer.modifiers.alt_key() {
            return false;
        }
        use winc_preview::markdown::cells::Step;
        let shift = self.pointer.modifiers.shift_key();
        match key {
            Key::Named(NamedKey::Tab) => {
                return self.live_table_step(if shift { Step::Previous } else { Step::Next })
                    || self.live_list_shift(shift);
            }
            Key::Named(NamedKey::Enter) if !shift && self.live_table_step(Step::Down) => {
                return true;
            }
            Key::Named(NamedKey::Enter) => return self.live_enter(shift),
            Key::Named(NamedKey::Backspace) => return self.live_backspace(),
            Key::Named(NamedKey::Delete) => return self.live_delete(),
            Key::Named(NamedKey::Home) => return self.live_line_edge(false),
            Key::Named(NamedKey::End) => return self.live_line_edge(true),
            Key::Named(NamedKey::ArrowLeft) => return self.live_horizontal(false),
            Key::Named(NamedKey::ArrowRight) => return self.live_horizontal(true),
            Key::Named(NamedKey::Space) if !shift && self.live_space() => return true,
            Key::Character(c) if c == " " && !shift && self.live_space() => return true,
            Key::Character(c) if (c == "`" || c == "~") && self.live_fence_char(c) => {
                return true;
            }
            _ => {}
        }
        let page = (area.h * 0.9).max(self.fonts.line_height);
        let (down, by) = match key {
            Key::Named(NamedKey::ArrowUp) => (false, None),
            Key::Named(NamedKey::ArrowDown) => (true, None),
            Key::Named(NamedKey::PageUp) => (false, Some(page)),
            Key::Named(NamedKey::PageDown) => (true, Some(page)),
            _ => return false,
        };
        self.live_vertical(down, by);
        true
    }

    /// 执行表格工具栏或右键菜单「表格」中的操作 `op`。
    ///
    /// `op` 可用的值见 `view::preview::markdown` 的 `table_tools`。修改规则见
    /// `winc_preview::markdown::cells::change`，`delete` 删除整张表，一次撤销即可恢复。
    /// 光标不在表格中、或操作无效（删除表头、删除最后一列）时返回 `false`。
    pub(crate) fn live_table_tool(&mut self, op: &str) -> bool {
        use winc_preview::markdown::Align;
        use winc_preview::markdown::cells::{self, Change};
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let leaf = live.leaf.start..live.leaf.end.min(rope.len_bytes());
        let text = rope.byte_slice(leaf.clone()).to_string();
        let change = match op {
            "row-above" => Change::RowAbove,
            "row" => Change::RowBelow,
            "column-left" => Change::ColumnLeft,
            "column" => Change::ColumnRight,
            "delete-row" => Change::DeleteRow,
            "delete-column" => Change::DeleteColumn,
            "left" => Change::Align(Align::Left),
            "center" => Change::Align(Align::Center),
            "right" => Change::Align(Align::Right),
            "none" => Change::Align(Align::None),
            "delete" => {
                // 删除表格后面的换行，避免留下空行。
                let end = text.trim_end().len();
                let (new, at) = (text[end..].trim_start_matches('\n').to_owned(), 0);
                return self.replace_live_text(leaf.start, &text, &new, at);
            }
            _ => return false,
        };
        let Some((new, at)) = cells::change(&text, caret.saturating_sub(leaf.start), change) else {
            return false;
        };
        self.replace_live_text(leaf.start, &text, &new, at)
    }

    /// 将文档中从 `start` 起的原文 `old` 替换为 `new`。
    ///
    /// 只替换不同的部分，光标移到 `new` 中的 `caret` 处（字节偏移）。随后的重排中各块滑动到
    /// 新位置。
    pub(super) fn replace_live_text(
        &mut self,
        start: usize,
        old: &str,
        new: &str,
        caret: usize,
    ) -> bool {
        self.preview.markdown.slide = true;
        let prefix = old
            .char_indices()
            .zip(new.chars())
            .find(|((_, a), b)| a != b)
            .map_or(old.len().min(new.len()), |((i, _), _)| i);
        let suffix = old[prefix..]
            .chars()
            .rev()
            .zip(new[prefix..].chars().rev())
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum::<usize>();
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(start + prefix),
            rope.byte_to_char(start + old.len() - suffix),
        );
        doc.collapse_selections();
        doc.selection = winc_core::Selection::new(a, b);
        self.run(winc_editor::Command::Insert(
            new[prefix..new.len() - suffix].to_owned(),
        ));
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let at = rope.byte_to_char((start + caret).min(rope.len_bytes()));
            doc.selection = winc_core::Selection::point(at);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 返回光标位于表格中时右键菜单的「表格」子菜单；不在表格中时为 `None`。
    pub(in crate::app) fn live_table_menu(
        &self,
    ) -> Option<Vec<crate::app::input::context_menu::MenuEntry>> {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry};
        self.live_leaf().filter(|l| l.table)?;
        let item = |op| MenuEntry::Item(MenuCommand::Table(op), true);
        Some(vec![
            item("row-above"),
            item("row"),
            item("column-left"),
            item("column"),
            MenuEntry::Separator,
            item("left"),
            item("center"),
            item("right"),
            MenuEntry::Separator,
            item("delete-row"),
            item("delete-column"),
            item("delete"),
        ])
    }

    /// 光标在表格的单元格中时，按 Tab、⇧Tab、回车切换单元格（见 `winc_preview::markdown::cells`）。
    ///
    /// 选中目标单元格的内容，随后的输入将替换该内容；在最后一行继续向下时新增一行。不在表格
    /// 中、或位于表头下方的分隔行时返回 `false`，交给编辑区处理（缩进、换行）。
    pub(super) fn live_table_step(&mut self, step: winc_preview::markdown::cells::Step) -> bool {
        use winc_preview::markdown::cells;
        let Some(live) = self.live_leaf().filter(|l| l.table) else {
            return false;
        };
        let Some(caret) = self.live_caret_source() else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        if !(live.leaf.start..=live.leaf.end).contains(&caret) {
            return false;
        }
        let text = rope.byte_slice(live.leaf.clone()).to_string();
        let Some(target) = cells::step(&text, caret - live.leaf.start, step) else {
            return false;
        };
        let mut select = target.select;
        if let Some((at, row)) = target.insert {
            let before = rope.len_bytes();
            let at = live.leaf.start + at;
            if let Some(doc) = self.editor.active_mut()
                && let Some(rope) = doc.text().rope()
            {
                let pos = rope.byte_to_char(at);
                doc.collapse_selections();
                doc.selection = winc_core::Selection::point(pos);
            }
            let len = row.len();
            self.run(winc_editor::Command::Insert(row));
            // 文档的行尾为 `\r\n` 时，插入的文字比 `row` 长。
            let after = self
                .editor
                .active()
                .and_then(|d| d.text().rope())
                .map_or(before, |r| r.len_bytes());
            let grown = (after - before).saturating_sub(len);
            select = select.start + grown..select.end + grown;
        }
        if let Some(doc) = self.editor.active_mut()
            && let Some(rope) = doc.text().rope()
        {
            let char_at = |b: usize| rope.byte_to_char((live.leaf.start + b).min(rope.len_bytes()));
            let (a, b) = (char_at(select.start), char_at(select.end));
            doc.goal_cell = None;
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 处理正文（段落、标题、块之间的空行）中的回车：另起一段、列表续一项、空项退出一层；
    /// 按住 ⇧ 时段内换行（见 `winc_preview::markdown::enter`）。
    ///
    /// 光标在代码块、公式、表格、HTML 块中时返回 `false`，按编辑区的方式换行并保持缩进。
    /// 有选区时先替换选区：光标之前的文字截至选区开头，之后的文字从选区末尾算起。
    pub(super) fn live_enter(&mut self, shift: bool) -> bool {
        use winc_preview::markdown::enter;
        if !shift && self.live_fence() {
            return true;
        }
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let (from, to) = (rope.char_to_byte(from), rope.char_to_byte(to));
        let line_of = |at: usize| rope.byte_to_line(at);
        let line_text = |line: usize| {
            let text = rope.line(line).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let start = rope.line_to_byte(line_of(from));
        let before = rope.byte_slice(start..from).to_string();
        let to_line = line_of(to);
        let rest = line_text(to_line);
        let after = rest[(to - rope.line_to_byte(to_line)).min(rest.len())..].to_owned();
        let edit = if shift {
            enter::Edit {
                before: 0,
                after: 0,
                text: enter::line_break(&before),
            }
        } else {
            let next = (to_line + 1 < rope.len_lines()).then(|| line_text(to_line + 1));
            let first = line_of(from);
            let above: Vec<String> = (first.saturating_sub(200)..first)
                .rev()
                .map(line_text)
                .collect();
            enter::enter(
                &before,
                &after,
                next.as_deref(),
                above.iter().map(String::as_str),
            )
        };
        let (a, b) = (from - edit.before, (to + edit.after).min(rope.len_bytes()));
        // 有序列表中续出一项：其后同一层各项的序号顺延，与新增的一项在同一次修改中完成，
        // 撤销一步即可恢复。
        let item = edit.text.rsplit('\n').next().unwrap_or_default();
        let below = (to_line + 1..rope.len_lines()).map(line_text);
        let changes = if shift || !edit.text.contains('\n') {
            Vec::new()
        } else {
            enter::renumber(item, below)
        };
        let mut text = edit.text.clone();
        let mut end = b;
        for (i, range, number) in changes {
            let start = rope.line_to_byte(to_line + 1 + i);
            text.push_str(&rope.byte_slice(end..start + range.start).to_string());
            text.push_str(&number.to_string());
            end = start + range.end;
        }
        let (a, b) = (rope.byte_to_char(a), rope.byte_to_char(end));
        // 光标停在新增的一项之后；换行符在插入时按文档的行尾风格转换，长度随之变化。
        let caret = (text.len() > edit.text.len()).then(|| {
            let ending = doc.line_ending.as_str().chars().count();
            let breaks = edit.text.matches('\n').count();
            a + edit.text.chars().count() + breaks * (ending - 1)
        });
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(text));
        if let Some(caret) = caret
            && let Some(doc) = self.editor.active_mut()
        {
            doc.selection = winc_core::Selection::point(caret);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 补全闭围栏：在围栏行（```` ```rust ````、`$$`）末尾按回车时，光标移到两行围栏之间的空行。
    ///
    /// 规则见 `winc_preview::markdown::enter::fence`。光标位于代码块中间的行、有选区、
    /// 或当前行不是围栏时返回 `false`。刚输入的围栏可能已解析为未闭合的代码块，
    /// 也可能仍是正文段落，因此两种情况都按原文判断。
    pub(super) fn live_fence(&mut self) -> bool {
        let leaf = self.live_leaf();
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(caret);
        if leaf.is_some_and(|l| !l.prose && rope.byte_to_line(l.leaf.start) != line) {
            return false;
        }
        let byte = rope.char_to_byte(caret);
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..byte).to_string();
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let after = text(line)[byte - start..].to_owned();
        if !before.contains(['`', '~', '$']) {
            return false;
        }
        let below: Vec<String> = (line + 1..rope.len_lines()).map(text).collect();
        let fence =
            winc_preview::markdown::enter::fence(&before, &after, below.iter().map(String::as_str));
        let Some((head, tail)) = fence else {
            return false;
        };
        // 分两次插入：换行符按文档的行尾风格生成，光标停在两段文字之间。
        self.run(winc_editor::Command::Insert(head));
        let at = self.editor.active().map(|d| d.selection.head);
        self.run(winc_editor::Command::Insert(tail));
        if let Some(doc) = self.editor.active_mut()
            && let Some(at) = at
        {
            doc.selection = winc_core::Selection::point(at);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 按 Tab、⇧Tab 调整列表项的层级：缩进为上一项的子项，或退出一层。
    ///
    /// 规则见 `winc_preview::markdown::enter::shift`，光标和选区随该行移动，子项和续行随该项
    /// 移动。不是列表项、有多个光标或选区跨行时返回 `false`，交给编辑区按普通缩进处理。
    pub(super) fn live_list_shift(&mut self, outdent: bool) -> bool {
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let chars = rope.len_chars();
        let sel = doc.selection;
        let line = rope.char_to_line(sel.head.min(chars));
        if !doc.extra_selections().is_empty() || rope.char_to_line(sel.anchor.min(chars)) != line {
            return false;
        }
        let line_text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let above: Vec<String> = (line.saturating_sub(200)..line)
            .rev()
            .map(line_text)
            .collect();
        let text = line_text(line);
        let Some((range, indent)) =
            winc_preview::markdown::enter::shift(&text, above.iter().map(String::as_str), outdent)
        else {
            return false;
        };
        if text[range.clone()] == indent {
            return true;
        }
        // 替换的范围只含空格、Tab 和有序列表的序号，字节数等于字符数。
        let start = rope.line_to_char(line);
        let (a, b) = (start + range.start, start + range.end);
        let moved = |c: usize| {
            if c >= b {
                (c + indent.len()).saturating_sub(range.len())
            } else {
                c.min(a + indent.len())
            }
        };
        let after = winc_core::Selection::new(moved(sel.anchor), moved(sel.head));
        // 子项和续行（缩进多于该项的各行）随该项移动同样的宽度，层级关系不变。
        use winc_preview::markdown::enter::margins;
        let (quote, old) = margins(&text);
        let new = indent.len() - indent.trim_start_matches([' ', '\t']).len();
        let mut lines = vec![format!(
            "{}{indent}{}",
            &text[..range.start],
            &text[range.end..]
        )];
        let (mut last, mut blanks) = (line, 0);
        for l in line + 1..rope.len_lines() {
            let child = line_text(l);
            if child.trim().is_empty() {
                blanks += 1;
                continue;
            }
            let (q, i) = margins(&child);
            if q != quote || i <= old {
                break;
            }
            lines.extend(std::iter::repeat_n(String::new(), blanks));
            blanks = 0;
            let (head, tail) = child.split_at(q);
            lines.push(if new >= old {
                format!("{head}{}{tail}", " ".repeat(new - old))
            } else {
                format!("{head}{}", &tail[(old - new).min(i)..])
            });
            last = l;
        }
        let (a, b, indent) = if last == line {
            (a, b, indent)
        } else {
            (start, line_end(rope, last), lines.join("\n"))
        };
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(indent));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = after;
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文中的删除键：移除光标前的一层块标记（列表记号、任务框、引用的 `>`、标题的 `#`）。
    ///
    /// 规则见 `winc_preview::markdown::enter::backspace`。光标位于块的开头时删到上一块的末尾，
    /// 两段正文合并为一段。有选区、有多个光标、位于代码块等块中、或光标前还有文字时返回
    /// `false`，按普通删除处理。
    pub(super) fn live_backspace(&mut self) -> bool {
        if let Some(done) = self.live_fenced_backspace() {
            return done;
        }
        if self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..caret).to_string();
        let Some(edit) = winc_preview::markdown::enter::backspace(&before) else {
            // 光标位于块的开头：预览中看不到光标前的缩进和块之间的空行，逐个删除时光标
            // 看起来不动。一次删到上一行的末尾；隔着空行时与上一块合并。
            if line == 0 || !before.trim().is_empty() {
                return false;
            }
            let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
            let to = rope.byte_to_char(caret);
            let from = if !blank(line - 1) {
                // 同一段的续行：没有缩进时按普通删除合并两行。
                if before.is_empty() {
                    return false;
                }
                line_end(rope, line - 1)
            } else {
                let Some(above) = (0..line).rev().find(|&l| !blank(l)) else {
                    return false;
                };
                let from = line_end(rope, above);
                let Some(prose) = self.live_prose_at(rope.char_to_byte(from)) else {
                    return false;
                };
                // 上一块是代码块、表格等时不合并：光标所在行有文字时只移动光标，移到上一块
                // 内容的末尾（闭围栏的上一行、最后一个单元格的文字之后）。
                if !prose && !blank(line) {
                    let last = rope.line(above).to_string();
                    let last = last.trim_end();
                    let from = if above > 0 && ["```", "~~~", "$$"].contains(&last.trim_start()) {
                        line_end(rope, above - 1)
                    } else if let Some(cell) = last.strip_suffix('|') {
                        from - (last.chars().count() - cell.trim_end().chars().count())
                    } else {
                        from
                    };
                    if let Some(doc) = self.editor.active_mut() {
                        doc.selection = winc_core::Selection::point(from);
                    }
                    self.preview.markdown.goal = None;
                    self.start_live_editing();
                    self.request_redraw();
                    return true;
                }
                from
            };
            if let Some(doc) = self.editor.active_mut() {
                doc.selection = winc_core::Selection::new(from, to);
            }
            self.run(winc_editor::Command::Insert(String::new()));
            self.preview.markdown.goal = None;
            self.start_live_editing();
            return true;
        };
        let (a, b) = (
            rope.byte_to_char(caret - edit.before),
            rope.byte_to_char(caret),
        );
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文末尾的向前删除键：光标位于块的最后一行末尾、其后隔着空行时，与下一块合并。
    ///
    /// 预览中不显示块之间的空行，逐个删除换行符时内容不变化。下一块不是正文（代码块、表格等）
    /// 时不合并，光标移到下一块的开头。有选区、有多个光标、光标不在行尾、或解析结果不是当前
    /// 版本时返回 `false`，按普通删除处理。
    pub(super) fn live_delete(&mut self) -> bool {
        if self.live_fenced_delete() {
            return true;
        }
        if self.live_leaf().is_none_or(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let blank = |l: usize| rope.line(l).chars().all(char::is_whitespace);
        if head != line_end(rope, line) || line + 1 >= rope.len_lines() || !blank(line + 1) {
            return false;
        }
        let Some(below) = (line + 1..rope.len_lines()).find(|&l| !blank(l)) else {
            return false;
        };
        let to = rope.line_to_char(below) + self.live_hidden_prefix(below);
        let Some(prose) = self.live_prose_at(rope.char_to_byte(to)) else {
            return false;
        };
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = if prose {
                winc_core::Selection::new(head, to)
            } else {
                winc_core::Selection::point(to)
            };
        }
        if prose {
            self.run(winc_editor::Command::Insert(String::new()));
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        self.request_redraw();
        true
    }

    /// 处理正文中输入的空格：行首的任务框简写（`[]`、`-[]`、`- [`）换成任务项 `- [ ] `。
    ///
    /// 规则见 `winc_preview::markdown::enter::task`。不是简写、有选区、有多个光标、输入法
    /// 正在组字、或光标位于代码块等块中时返回 `false`，按普通输入处理。
    pub(super) fn live_space(&mut self) -> bool {
        let composing = !self.preedit_at(Focus::Editor).is_empty();
        if composing || self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let at = (caret - rope.line_to_byte(line)).min(text.len());
        let Some(edit) = winc_preview::markdown::enter::task(&text[..at], &text[at..]) else {
            return false;
        };
        let (a, b) = (
            rope.byte_to_char(caret - edit.before),
            rope.byte_to_char(caret + edit.after),
        );
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(edit.text));
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 处理正文中输入的 `` ` ``、`~`（`mark`）：输入后该行成为未闭合的开围栏时，立即在下一行
    /// 补上闭围栏，光标留在开围栏之后，可以继续输入语言名。
    ///
    /// 未闭合的围栏会使其后的全部内容成为代码，补全之前整篇的外观都会改变。不构成开围栏、
    /// 下方已有闭围栏、有选区、或光标位于代码块等块中时返回 `false`，按普通输入处理。
    pub(super) fn live_fence_char(&mut self, mark: &str) -> bool {
        let composing = !self.preedit_at(Focus::Editor).is_empty();
        if composing || self.live_leaf().is_some_and(|l| !l.prose) {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        let line = rope.byte_to_line(caret);
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_end_matches(['\n', '\r']).to_owned()
        };
        let current = text(line);
        let at = (caret - rope.line_to_byte(line)).min(current.len());
        let typed = format!("{}{mark}", &current[..at]);
        // 只在恰好输入第三个字符时补全：更长的围栏由回车补全。
        if typed.len() - typed.trim_end_matches(mark).len() != 3 * mark.len() {
            return false;
        }
        let below: Vec<String> = (line + 1..rope.len_lines()).map(text).collect();
        let below = below.iter().map(String::as_str);
        let fence = winc_preview::markdown::enter::fence(&typed, &current[at..], below);
        let Some((_, tail)) = fence else {
            return false;
        };
        self.run(winc_editor::Command::Insert(mark.to_owned()));
        let at = self.editor.active().map(|d| d.selection.head);
        self.run(winc_editor::Command::Insert(tail));
        if let Some(doc) = self.editor.active_mut()
            && let Some(at) = at
        {
            doc.selection = winc_core::Selection::point(at);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 返回光标所在的围栏代码块或 `$$` 公式各行的位置；光标不在这类块中时为 `None`。
    pub(super) fn live_fenced(&self) -> Option<Fenced> {
        let live = self.live_leaf().filter(|l| !l.prose && !l.table)?;
        let rope = self.editor.active()?.text().rope()?;
        let len = rope.len_bytes();
        // 引用、列表中的代码块各行带有引用标记和缩进。
        let text = |l: usize| {
            let text = rope.line(l).to_string();
            text.trim_matches([' ', '\t', '>', '\n', '\r']).to_owned()
        };
        let first = rope.byte_to_line(live.leaf.start.min(len));
        let open = text(first);
        let mark = ["```", "~~~", "$$"]
            .into_iter()
            .find(|m| open.starts_with(m))?;
        let end = live
            .leaf
            .end
            .min(len)
            .saturating_sub(1)
            .max(live.leaf.start);
        let mut last = rope.byte_to_line(end);
        while last > first && text(last).is_empty() {
            last -= 1;
        }
        let close = text(last);
        let closed =
            last > first && close.starts_with(mark) && close.chars().all(|c| mark.starts_with(c));
        let content = first + 1..if closed { last } else { last + 1 };
        Some(Fenced {
            first,
            last,
            closed,
            bare: open.chars().all(|c| mark.starts_with(c)),
            empty: content.clone().all(|l| text(l).is_empty()),
            content,
        })
    }

    /// 将围栏代码块 `fenced` 的全部各行替换为 `text`，光标放在其开头。
    pub(super) fn replace_fenced(&mut self, fenced: &Fenced, text: String) {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let (a, b) = (rope.line_to_char(fenced.first), line_end(rope, fenced.last));
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        self.run(winc_editor::Command::Insert(text));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::point(a);
        }
        self.preview.markdown.goal = None;
        self.start_live_editing();
        self.request_redraw();
    }

    /// 处理围栏代码块、公式中的删除键，返回是否已处理；不属于以下情形时为 `None`。
    ///
    /// 逐字删除围栏会使代码块失去一侧的围栏，其后的全部内容成为代码。因此：
    /// - 光标在内容第一行的开头：空的代码块整块删除，有内容时不删除。
    /// - 光标在不带语言名的开围栏上：空的代码块整块删除，有内容时移除两行围栏、保留内容。
    /// - 光标在闭围栏上：空的代码块整块删除，有内容时光标移到内容的末尾。
    pub(super) fn live_fenced_backspace(&mut self) -> Option<bool> {
        let fenced = self.live_fenced()?;
        let doc = self.editor.active()?;
        let rope = doc.text().rope()?;
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return None;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let col = head - rope.line_to_char(line);
        let on_close = fenced.closed && line == fenced.last;
        let on_open = line == fenced.first && fenced.bare && col > 0;
        let at_start = line == fenced.first + 1 && col == 0;
        if !on_close && !on_open && !at_start {
            return None;
        }
        if fenced.empty {
            self.replace_fenced(&fenced, String::new());
        } else if on_close {
            let end = line_end(rope, fenced.last - 1);
            if let Some(doc) = self.editor.active_mut() {
                doc.selection = winc_core::Selection::point(end);
            }
            self.request_redraw();
        } else if on_open {
            let lines: Vec<String> = fenced
                .content
                .clone()
                .map(|l| {
                    let text = rope.line(l).to_string();
                    text.trim_end_matches(['\n', '\r']).to_owned()
                })
                .collect();
            self.replace_fenced(&fenced, lines.join("\n"));
        }
        Some(true)
    }

    /// 处理围栏代码块、公式中的向前删除键，返回是否已处理。
    ///
    /// 光标在开围栏的末尾、或内容最后一行的末尾时不删除：删除其后的换行符会使内容与围栏
    /// 连成一行，代码块失去一侧的围栏。
    pub(super) fn live_fenced_delete(&mut self) -> bool {
        let Some(fenced) = self.live_fenced() else {
            return false;
        };
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return false;
        }
        let head = doc.selection.head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let before_close = fenced.closed && line + 1 == fenced.last;
        head == line_end(rope, line) && (line == fenced.first || before_close)
    }

    /// 在光标所在的围栏代码块、公式之后另起一段，光标移到新的一段；光标不在已闭合的这类
    /// 块中时返回 `false`。
    pub(super) fn live_leave_fenced(&mut self) -> bool {
        let Some(fenced) = self.live_fenced().filter(|f| f.closed) else {
            return false;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let end = line_end(rope, fenced.last);
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::point(end);
        }
        self.run(winc_editor::Command::Insert("\n\n".to_owned()));
        self.preview.markdown.goal = None;
        self.start_live_editing();
        true
    }

    /// 文档的最后一行属于代码块、表格、分隔线等不是正文的块时，在文末另起一段并把光标移到
    /// 该段，返回是否已处理。
    ///
    /// 文档以换行符结束时光标可以直接移到末尾的空行，不需要处理。
    pub(super) fn live_append_paragraph(&mut self) -> bool {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return false;
        };
        let chars = rope.len_chars();
        if chars == 0 || rope.char(chars - 1) == '\n' {
            return false;
        }
        if self.live_prose_at(rope.len_bytes()) != Some(false) {
            return false;
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::point(chars);
        }
        self.run(winc_editor::Command::Insert("\n\n".to_owned()));
        self.preview.markdown.goal = None;
        self.preview.markdown.line_end = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 对选区切换行内格式的标记 `mark`（`**`、`*`、`` ` ``、`~~`），规则见
    /// `winc_preview::markdown::format::toggle`。
    ///
    /// 只处理一行之内的单个选区；选区跨行、有多个光标、或光标位于代码块、公式等按原文显示的
    /// 块中时不修改。
    pub(in crate::app) fn live_format(&mut self, mark: &str) {
        self.parse_markdown_inline();
        if self.live_leaf().is_some_and(|l| !l.prose && !l.table) {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        if !doc.extra_selections().is_empty() {
            return;
        }
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let line = rope.char_to_line(from);
        if rope.char_to_line(to) != line {
            return;
        }
        let start = rope.line_to_byte(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        let local = |c: usize| (rope.char_to_byte(c) - start).min(text.len());
        let t = winc_preview::markdown::format::toggle(text, local(from)..local(to), mark);
        // 修改只涉及这一行：行首之前的字符数不变，行内的位置按修改之后的文字换算。
        let first = rope.line_to_char(line);
        let chars_to = |text: &str, byte: usize| first + text[..byte].chars().count();
        let (a, b) = (chars_to(text, t.range.start), chars_to(text, t.range.end));
        let edited = format!(
            "{}{}{}",
            &text[..t.range.start],
            t.text,
            &text[t.range.end..]
        );
        let select = (
            chars_to(&edited, t.select.start),
            chars_to(&edited, t.select.end),
        );
        let changed = a != b || !t.text.is_empty();
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(a, b);
        }
        if changed {
            // 按粘贴处理：整段替换，不触发输入单个字符时的括号、引号配对。
            self.run(winc_editor::Command::Paste(t.text));
        }
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(select.0, select.1);
        }
        self.preview.markdown.goal = None;
        self.preview.markdown.line_end = None;
        self.start_live_editing();
        self.reset_blink();
        self.request_redraw();
    }
}
