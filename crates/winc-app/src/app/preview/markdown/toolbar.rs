//! 可编辑的预览中用鼠标选中文字后显示的格式工具栏（`markdown.selectionToolbar`）：选区不变时
//! 显示，按钮执行对应的写作命令。

use super::*;
use crate::keymap::Action;

/// 显示格式工具栏的选区，以及工具栏固定的位置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SelectionToolbar {
    /// 选区所在的文档。
    pub doc: DocumentId,
    /// 选区：改变后不再显示工具栏。
    pub selection: winc_core::Selection,
    /// 点击过按钮后工具栏固定的左边缘（逻辑像素，相对内容的左边缘）；为 `None` 时按选区放置。
    ///
    /// 按钮增删标记、改变字形之后选区的宽度随之改变，工具栏不再跟随选区的中点移动，连续点击时
    /// 按钮留在原处。
    pub left: Option<f32>,
}

/// 工具栏中的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Bold,
    Italic,
    Strike,
    Code,
    Highlight,
    Link,
}

impl Tool {
    /// 工具栏中的按钮，按显示顺序排列。
    pub(crate) const ALL: [Tool; 6] = [
        Tool::Bold,
        Tool::Italic,
        Tool::Strike,
        Tool::Highlight,
        Tool::Code,
        Tool::Link,
    ];

    /// 返回按钮对应的写作命令。
    pub(crate) fn action(self) -> Action {
        match self {
            Tool::Bold => Action::MarkdownBold,
            Tool::Italic => Action::MarkdownItalic,
            Tool::Strike => Action::MarkdownStrikethrough,
            Tool::Code => Action::MarkdownCode,
            Tool::Highlight => Action::MarkdownHighlight,
            Tool::Link => Action::MarkdownLink,
        }
    }

    /// 返回按钮的图标。
    pub(crate) fn glyph(self) -> crate::icons::WritingGlyph {
        use crate::icons::WritingGlyph as G;
        match self {
            Tool::Bold => G::Bold,
            Tool::Italic => G::Italic,
            Tool::Strike => G::Strike,
            Tool::Code => G::Code,
            Tool::Highlight => G::Highlight,
            Tool::Link => G::Link,
        }
    }

    /// 按钮之前是否有分隔线：粗体、斜体、删除线、高亮一组，行内代码一组，链接一组。
    pub(crate) fn starts_group(self) -> bool {
        matches!(self, Tool::Code | Tool::Link)
    }

    /// 返回按钮切换的标记；链接没有标记，为 `None`。
    fn mark(self) -> Option<&'static str> {
        Some(match self {
            Tool::Bold => "**",
            Tool::Italic => "*",
            Tool::Strike => "~~",
            Tool::Code => "`",
            Tool::Highlight => "==",
            Tool::Link => return None,
        })
    }

    /// 返回悬停提示中的名字。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tool::Bold => t!("preview-tool-bold"),
            Tool::Italic => t!("preview-tool-italic"),
            Tool::Strike => t!("preview-tool-strike"),
            Tool::Code => t!("preview-tool-code"),
            Tool::Highlight => t!("preview-tool-highlight"),
            Tool::Link => t!("preview-tool-link"),
        }
    }
}

impl App {
    /// 鼠标选中一行之内的文字后记录选区，之后选区不变时显示工具栏。
    pub(in crate::app) fn offer_selection_toolbar(&mut self) {
        let enabled = self.config.markdown_selection_toolbar != Some(false);
        // 格式命令只处理一行之内的选区：跨行时不显示。
        let one_line = |d: &winc_editor::Document| {
            let rope = d.text().rope();
            let line = |pos: usize| rope.map(|r| r.char_to_line(pos.min(r.len_chars())));
            line(d.selection.from()) == line(d.selection.to())
        };
        let picked = (self.editor.active())
            .filter(|d| !d.selection.is_empty() && d.extra_selections().is_empty() && one_line(d))
            .map(|d| SelectionToolbar {
                doc: d.id,
                selection: d.selection,
                left: None,
            });
        self.preview.markdown.toolbar = picked.filter(|_| enabled && self.markdown_live());
        self.request_redraw();
    }

    /// 返回显示工具栏的选区；选区已改变时为 `None`。
    pub(crate) fn selection_toolbar(&self) -> Option<winc_core::Selection> {
        let SelectionToolbar { doc, selection, .. } = self.preview.markdown.toolbar?;
        let doc = self.editor.active().filter(|d| d.id == doc)?;
        (doc.selection == selection && self.markdown_live()).then_some(selection)
    }

    /// 返回选区已有的格式：选区两侧或内部两端已有标记，或选区是链接文字。
    pub(crate) fn active_tools(&self) -> Vec<Tool> {
        let Some(selection) = self.selection_toolbar() else {
            return Vec::new();
        };
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()) else {
            return Vec::new();
        };
        let line = rope.char_to_line(selection.from());
        let start = rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let byte = |pos: usize| {
            let pos = pos.saturating_sub(start).min(text.chars().count());
            text.char_indices().nth(pos).map_or(text.len(), |(i, _)| i)
        };
        let (from, to) = (byte(selection.from()), byte(selection.to()));
        let text = text.trim_end_matches(['\n', '\r']);
        let (from, to) = (from.min(text.len()), to.min(text.len()));
        let link = text[..from].ends_with('[') && text[to..].starts_with("](");
        (Tool::ALL.into_iter())
            .filter(|tool| match tool.mark() {
                Some(mark) => winc_preview::markdown::format::active(text, from..to, mark),
                None => link,
            })
            .collect()
    }

    /// 执行工具栏中的按钮 `tool`；之后的选区上仍显示工具栏，左右位置不变。
    pub(in crate::app) fn run_selection_tool(&mut self, tool: Tool) {
        match tool.mark() {
            Some(mark) => {
                let content = self.preview.markdown.layout.as_ref().map(|laid| laid.left);
                let left = (self.hits.selection_card.zip(content)).map(|(card, x)| card.x - x);
                self.live_format(mark);
                self.offer_selection_toolbar();
                if let Some(toolbar) = &mut self.preview.markdown.toolbar {
                    toolbar.left = left;
                }
            }
            None => {
                self.markdown_link(false);
                self.preview.markdown.toolbar = None;
            }
        }
    }
}
