//! Markdown 的写作命令：设标题、切换引用和列表、插入代码块、公式块、表格、链接和图片。
//!
//! 修改落在源文档上，源代码和可编辑的预览共用；文字怎样改见 `winc_preview::markdown::paragraph`。

use std::ops::Range;

use winc_preview::markdown::paragraph::{self, Kind};

use super::*;

impl App {
    /// 当前标签是否在编辑 Markdown：可以修改的 Markdown 文件，显示着源代码，或者是可编辑的预览。
    pub(crate) fn markdown_writing(&self) -> bool {
        self.markdown_live()
            || self.editor.active().is_some_and(|d| {
                d.preview_kind() == Some(PreviewKind::Markdown)
                    && d.edits_text()
                    && d.readonly_reason().is_none()
                    && d.preview_mode() != PreviewMode::Preview
            })
    }

    /// 是否淡化光标所在段落以外的文字（专注模式，`markdown.focusMode`）。
    pub(crate) fn markdown_focus(&self) -> bool {
        self.config.markdown_focus == Some(true) && self.markdown_writing()
    }

    /// 是否让光标所在的行停在编辑区中间（打字机模式，`markdown.typewriterMode`）。
    pub(crate) fn markdown_typewriter(&self) -> bool {
        self.config.markdown_typewriter == Some(true) && self.markdown_writing()
    }

    /// 编辑区是否使用写作命令的快捷键：可编辑的预览中总是使用，Markdown 文件的源代码在
    /// `markdown.shortcuts` 开着时使用。
    pub(crate) fn markdown_keys(&self) -> bool {
        self.markdown_live()
            || (self.config.markdown_shortcuts != Some(false) && self.markdown_writing())
    }

    /// 可编辑的预览中光标在代码块、公式、表格等按原文显示的块里，写作命令不改动它。
    fn in_raw_block(&mut self) -> bool {
        if !self.markdown_live() {
            return false;
        }
        self.parse_markdown_inline();
        self.live_leaf().is_some_and(|l| !l.prose)
    }

    /// 把选中的各行（没有选区时是光标所在的一行）改成 `kind`，见 [`paragraph::change`]。
    pub(in crate::app) fn markdown_paragraph(&mut self, kind: Kind) {
        if self.in_raw_block() {
            return;
        }
        let Some((first, last, caret)) = self.selected_lines() else {
            return;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let lines: Vec<String> = (first..=last).map(|l| line_text(rope, l)).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let changed = paragraph::change(&refs, kind);
        let text: Vec<&str> = changed.iter().map(|l| l.text.as_str()).collect();
        let text = text.join("\n");
        // 只有一行、没有选区时光标随内容移动，否则选中改过的各行。
        let select = match caret {
            Some(at) => {
                let line = &changed[0];
                let at = if at >= line.old_start {
                    at - line.old_start + line.new_start
                } else {
                    line.new_start
                };
                at.min(line.text.len())..at.min(line.text.len())
            }
            None => 0..text.len(),
        };
        self.replace_lines(first, last, &text, select);
    }

    /// 把选中的各行放进 `fence` 围起的块里（代码块、公式块）；光标在空行上、没有选区时插入
    /// 一个空的块。见 [`paragraph::fenced`]。
    pub(in crate::app) fn markdown_fenced(&mut self, fence: &str) {
        if self.in_raw_block() {
            return;
        }
        let Some((first, last, caret)) = self.selected_lines() else {
            return;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let content: Vec<String> = (first..=last).map(|l| line_text(rope, l)).collect();
        let content = content.join("\n");
        // 光标在有字的一行上、没有选区时，块插在这一行下面。
        if caret.is_some() && !content.trim().is_empty() {
            let (block, at) = paragraph::fenced(fence, "");
            self.insert_below(first, &block, at);
            return;
        }
        let content = if content.trim().is_empty() {
            ""
        } else {
            &content
        };
        let (text, at) = paragraph::fenced(fence, content);
        self.replace_lines(first, last, &text, at..at);
    }

    /// 插入两列的表格：光标在空行上时换掉这一行，否则插在这一行下面。
    pub(in crate::app) fn markdown_table(&mut self) {
        if self.in_raw_block() {
            return;
        }
        let Some((first, _, _)) = self.selected_lines() else {
            return;
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let (table, at) = paragraph::table();
        if line_text(rope, first).trim().is_empty() {
            self.replace_lines(first, first, &table, at..at);
        } else {
            self.insert_below(first, &table, at);
        }
    }

    /// 把选中的文字（一行之内）做成链接或图片（`image`）；剪贴板里是网址时作为地址。
    /// 见 [`paragraph::link`]。
    pub(in crate::app) fn markdown_link(&mut self, image: bool) {
        if self.in_raw_block() {
            return;
        }
        let url = self.clipboard_text().filter(|t| is_url(t.trim()));
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
        let text = rope.slice(from..to).to_string();
        if text.contains('\n') {
            return;
        }
        let (link, select) = paragraph::link(&text, url.as_deref().map(str::trim), image);
        let crlf = doc.line_ending == winc_core::LineEnding::CrLf;
        let at = |byte: usize| from + chars_in(&link, byte, crlf);
        let select = (at(select.start), at(select.end));
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(from, to);
        }
        self.run(winc_editor::Command::Paste(link));
        self.finish_writing(select);
    }

    /// 选中的各行（首行、末行）；只有光标、没有选区时另给出光标在行中的位置（字节偏移）。
    /// 有多个光标时为 `None`。
    pub(super) fn selected_lines(&self) -> Option<(usize, usize, Option<usize>)> {
        let doc = self.editor.active()?;
        let rope = doc.text().rope()?;
        if !doc.extra_selections().is_empty() {
            return None;
        }
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let first = rope.char_to_line(from);
        let mut last = rope.char_to_line(to);
        // 选区停在下一行的开头时不算那一行。
        if last > first && to == rope.line_to_char(last) {
            last -= 1;
        }
        let caret = (from == to).then(|| rope.char_to_byte(from) - rope.line_to_byte(first));
        Some((first, last, caret))
    }

    /// 把第 `first` 到 `last` 行（不含末行的换行符）换成 `text`，之后选中 `select`（`text` 中的
    /// 字节偏移）。一次撤销即可恢复。
    pub(super) fn replace_lines(
        &mut self,
        first: usize,
        last: usize,
        text: &str,
        select: Range<usize>,
    ) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let start = rope.line_to_char(first);
        let end = start_of_break(rope, last);
        let crlf = doc.line_ending == winc_core::LineEnding::CrLf;
        let at = |byte: usize| start + chars_in(text, byte, crlf);
        let select = (at(select.start), at(select.end));
        if let Some(doc) = self.editor.active_mut() {
            doc.collapse_selections();
            doc.selection = winc_core::Selection::new(start, end);
        }
        self.run(winc_editor::Command::Paste(text.to_owned()));
        self.finish_writing(select);
    }

    /// 在第 `line` 行下面隔一个空行插入块 `block`，光标放在块中的 `at`（字节偏移）。
    pub(super) fn insert_below(&mut self, line: usize, block: &str, at: usize) {
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return;
        };
        let text = format!("{}\n\n{block}", line_text(rope, line));
        let head = text.len() - block.len();
        self.replace_lines(line, line, &text, head + at..head + at);
    }

    /// 写作命令改完之后：选中 `select`（字符位置），可编辑的预览中显示光标所在块的原文。
    fn finish_writing(&mut self, (a, b): (usize, usize)) {
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = winc_core::Selection::new(a, b);
        }
        if self.markdown_live() {
            self.preview.markdown.goal = None;
            self.preview.markdown.line_end = None;
            self.start_live_editing();
            self.reset_blink();
        }
        self.ensure_caret_visible();
        self.request_redraw();
    }
}

/// 光标 `at`（字符位置）所在的段落：前后到空行为止的首行和末行，各方向最多看
/// [`PARAGRAPH_LOOK`] 行。
pub(crate) fn paragraph_lines(rope: &ropey::Rope, at: usize) -> (usize, usize) {
    let line = rope.char_to_line(at.min(rope.len_chars()));
    let blank = |l: usize| line_text(rope, l).trim().is_empty();
    if blank(line) {
        return (line, line);
    }
    let lines = rope.len_lines();
    let first = (line.saturating_sub(PARAGRAPH_LOOK)..line)
        .rev()
        .find(|&l| blank(l))
        .map_or(line.saturating_sub(PARAGRAPH_LOOK), |l| l + 1);
    let last = (line + 1..lines.min(line + 1 + PARAGRAPH_LOOK))
        .find(|&l| blank(l))
        .map_or((lines - 1).min(line + PARAGRAPH_LOOK), |l| l - 1);
    (first, last)
}

impl App {
    /// Markdown 中一次修改增删了行之后，按显示的序号重排改动处有序列表的原文序号，并入这次
    /// 修改的撤销单元。`revision` 是修改之前的版本号。
    ///
    /// 显示时有序列表从第一项的序号起依次递增，原文中的其余序号不起作用；删除、粘贴、移动
    /// 列表项之后改写原文，使原文与显示一致。哪些项要改见 `enter::renumber_from`。
    pub(in crate::app) fn renumber_markdown_lists(&mut self, revision: u64) {
        use winc_preview::markdown::enter;
        if !self.markdown_keys() {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let line = doc.unchanged_lines_since(revision);
        if line >= rope.len_lines() || !doc.extra_selections().is_empty() {
            return;
        }
        let above: Vec<String> = (line.saturating_sub(PARAGRAPH_LOOK)..line)
            .rev()
            .map(|l| line_text(rope, l))
            .collect();
        let anchors =
            enter::renumber_from(&line_text(rope, line), above.iter().map(String::as_str));
        let mut numbers = Vec::new();
        for up in anchors {
            let at = line - up;
            let below = (at + 1..rope.len_lines()).map(|l| line_text(rope, l));
            for (i, range, number) in enter::renumber(&line_text(rope, at), below) {
                numbers.push((at + 1 + i, range, number));
            }
        }
        if numbers.is_empty() {
            return;
        }
        let top: Vec<String> = (0..line).map(|l| line_text(rope, l)).collect();
        if enter::inside_fence(top.iter().map(String::as_str)) {
            return;
        }
        // 从后往前修改，前面的位置不受影响；选区随其前方序号的长度变化而移动。
        numbers.sort_by_key(|n| std::cmp::Reverse(n.0));
        numbers.dedup_by_key(|n| n.0);
        let before = doc.selection;
        let mut moved = [before.anchor, before.head];
        let changes: Vec<winc_core::Change> = (numbers.into_iter())
            .map(|(l, range, number)| {
                let start = rope.byte_to_char(rope.line_to_byte(l) + range.start);
                let removed = rope.slice(start..start + range.len()).to_string();
                let inserted = number.to_string();
                for pos in &mut moved {
                    if *pos >= start + removed.len() {
                        *pos = *pos + inserted.len() - removed.len();
                    } else if *pos > start {
                        *pos = start + inserted.len().min(*pos - start);
                    }
                }
                winc_core::Change {
                    start,
                    removed,
                    inserted,
                }
            })
            .collect();
        let after = winc_core::Selection::new(moved[0], moved[1]);
        let transaction =
            winc_core::Transaction::new(changes, before, after, winc_core::EditKind::Other);
        if let Some(doc) = self.editor.active_mut() {
            doc.amend(transaction);
        }
    }
}

/// 找段落的边界时往前、往后各看多少行：很长的一段不再找下去。
const PARAGRAPH_LOOK: usize = 500;

/// 第 `line` 行的文字，不含换行符。
pub(super) fn line_text(rope: &ropey::Rope, line: usize) -> String {
    let text = rope.line(line).to_string();
    text.trim_end_matches(['\n', '\r']).to_owned()
}

/// 第 `line` 行换行符之前的位置（字符）。
fn start_of_break(rope: &ropey::Rope, line: usize) -> usize {
    let start = rope.line_to_char(line);
    start + line_text(rope, line).chars().count()
}

/// `text` 前 `byte` 个字节在文档里占的字符数：文档用 CRLF 时每个换行多一个字符。
fn chars_in(text: &str, byte: usize, crlf: bool) -> usize {
    let head = &text[..byte];
    head.chars().count() + if crlf { head.matches('\n').count() } else { 0 }
}

/// 网址 `url` 是否指向图片（按 `?`、`#` 之前的扩展名判断）。
pub(super) fn is_image_url(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let images = [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".avif", ".bmp",
    ];
    url.starts_with("http") && images.iter().any(|ext| path.ends_with(ext))
}

/// `text` 是否像一个网址。
pub(super) fn is_url(text: &str) -> bool {
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|p| text.len() > p.len() && text.starts_with(p))
        && !text.contains(char::is_whitespace)
}
