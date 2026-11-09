//! 转到文件中的符号（`⇧⌘O`，或在快速打开中输入 `@`）。
//!
//! 在快速打开的卡片中列出当前文件的函数、类型、标题等符号，输入内容可筛选。上下移动选中项时，
//! 编辑区预览滚动到该符号；回车跳转到该符号，`Esc` 恢复打开前的滚动位置。

use winc_editor::DocumentId;
use winc_editor::symbols::{self, Symbol};
use winc_i18n::t;

use super::*;

/// 当前文档的符号列表，进入 `@` 模式时提取一次。
pub struct SymbolList {
    /// 符号所属的文档。
    id: DocumentId,
    /// 提取到的符号，按在文件中出现的顺序排列。
    pub items: Vec<Symbol>,
    /// 文档是否为大文件（全文没有读入内存）；为真时不提取符号，`items` 为空。
    pub too_large: bool,
    /// 打开列表时编辑区的滚动位置：首行（行，从 0 起）和横向偏移（逻辑像素）。
    ///
    /// 未选择符号就离开时据此恢复。
    scroll: (f64, f32),
}

impl App {
    /// 打开符号列表（`⇧⌘O`）：在快速打开的卡片中列出当前文档的符号，不需要先打开目录。
    ///
    /// 没有打开的文档，或当前标签不显示文本时，在状态栏提示并返回。
    pub(in crate::app) fn open_symbols(&mut self) {
        if self.editor.active.is_none() || !self.active_shows_text() {
            self.flash(t!("editor-no-open-file"));
            return;
        }
        self.finish_name_input();
        self.picker_places = None;
        self.picker_branch_rows = None;
        self.picker_location_rows = None;
        self.picker_folders = false;
        self.picker_open = true;
        self.picker_doc = self.editor.active().map(|d| d.id);
        self.picker_input.set_text("@");
        self.picker_input.set_caret(1, false);
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 是否正在列出符号，即输入框以 `@` 开头且不在「位置」列表中。
    pub(crate) fn picker_lists_symbols(&self) -> bool {
        self.picker_places.is_none()
            && !self.picker_lists_branches()
            && !self.picker_lists_locations()
            && !self.picker_folders
            && self.picker_input.text().starts_with('@')
    }

    /// 按当前查询刷新符号列表。
    ///
    /// 查询为空时保持符号在文件中的顺序，并选中光标所在的符号；
    /// 否则按名字的匹配度（见 [`input::palette::rate`]）排序，匹配度相同时保持文件中的顺序。
    pub(in crate::app) fn symbols_refresh(&mut self) {
        let Some(doc) = self.editor.active() else {
            self.picker_symbols = None;
            self.picker_hits.clear();
            return;
        };
        if self.picker_symbols.as_ref().is_none_or(|l| l.id != doc.id) {
            let items = doc.text().rope().map_or_else(Vec::new, |rope| {
                symbols::symbols(rope, doc.highlighter.language())
            });
            self.picker_symbols = Some(SymbolList {
                id: doc.id,
                items,
                too_large: doc.is_large(),
                scroll: (doc.scroll_line, doc.scroll_x),
            });
        }
        let Some(list) = &self.picker_symbols else {
            return;
        };

        let needle = input::palette::needle(self.picker_input.text().trim_start_matches('@'));
        let caret_line = self
            .editor
            .active()
            .map(|d| d.text().pos_to_line_col(d.selection.head).0);
        let mut scored: Vec<(u32, usize)> = list
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let score = if needle.is_empty() {
                    0
                } else {
                    input::palette::rate(&needle, &s.name)?
                };
                Some((score, i))
            })
            .collect();
        scored.sort();
        // 查询为空时选中光标所在的符号，即起始行不晚于光标行的最后一个符号，
        // 使列表打开时就定位在当前位置。
        if needle.is_empty()
            && let Some(caret) = caret_line
        {
            self.picker_selected = list
                .items
                .iter()
                .rposition(|s| s.line <= caret)
                .unwrap_or(0);
        }
        self.picker_hits = scored.into_iter().map(|(_, i)| i).collect();
        self.symbols_preview();
    }

    /// 将编辑区滚动到选中的符号并预览，光标位置不变；该行已经可见时不滚动。
    pub(in crate::app) fn symbols_preview(&mut self) {
        let Some(line) = self.selected_symbol().map(|s| s.line) else {
            return;
        };
        self.scroll_to_line(line, 1.0 / 3.0, true);
        self.request_redraw();
    }

    fn selected_symbol(&self) -> Option<&Symbol> {
        let list = self.picker_symbols.as_ref()?;
        list.items.get(*self.picker_hits.get(self.picker_selected)?)
    }

    /// 跳转到选中的符号并选中其名字，同时关闭面板。
    pub(in crate::app) fn symbols_accept(&mut self) {
        let Some(symbol) = self.selected_symbol().cloned() else {
            return;
        };
        // 先清空 `picker_symbols`，使关闭面板时 `drop_symbols` 不恢复打开前的滚动位置。
        self.picker_symbols = None;
        self.close_picker();
        self.jump_to(symbol.line, symbol.col, symbol.name.chars().count());
    }

    /// 离开 `@` 模式（关闭面板或删除 `@`）时清理符号列表。
    ///
    /// 未跳转到符号时，编辑区恢复到打开列表时的滚动位置。
    pub(in crate::app) fn drop_symbols(&mut self) {
        let Some(list) = self.picker_symbols.take() else {
            return;
        };
        if let Some(doc) = self.editor.document_mut(list.id) {
            (doc.scroll_line, doc.scroll_x) = list.scroll;
        }
    }
}
