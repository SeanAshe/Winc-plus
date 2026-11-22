//! 更改文件的语言（`Ctrl+K M`、命令面板）：在快速打开的卡片里列出全部语言，键入文字筛选。
//!
//! 结果列表的各行是 [`winc_syntax::languages`] 中的下标；在它之后的两个下标分别表示「自动检测」
//! 和「纯文本」（见 [`LanguageRow`]）。选中后与状态栏语言菜单中的同一项相同。状态栏上点击语言
//! 仍弹出菜单。

use super::*;
use crate::app::input::context_menu::MenuCommand;
use crate::app::input::palette::{matched, needle, rate};

/// 语言列表中的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanguageRow {
    /// 按文件名重新识别语言（只在文档有文件名时列出）。
    Detect,
    /// 纯文本，不着色。
    Plain,
    /// [`winc_syntax::languages`] 中的第几种语言。
    Language(usize),
}

impl LanguageRow {
    /// 结果列表中的下标对应的行：语言之后的两个下标是「自动检测」和「纯文本」。
    pub(crate) fn of(hit: usize) -> Option<LanguageRow> {
        let count = winc_syntax::languages().len();
        match hit {
            i if i < count => Some(LanguageRow::Language(i)),
            i if i == count => Some(LanguageRow::Detect),
            i if i == count + 1 => Some(LanguageRow::Plain),
            _ => None,
        }
    }

    /// 这一行在结果列表中的下标，见 [`LanguageRow::of`]。
    fn hit(self) -> usize {
        let count = winc_syntax::languages().len();
        match self {
            LanguageRow::Language(i) => i,
            LanguageRow::Detect => count,
            LanguageRow::Plain => count + 1,
        }
    }

    /// 显示的名字。
    pub(crate) fn title(self) -> &'static str {
        match self {
            LanguageRow::Language(i) => winc_syntax::languages().get(i).map_or("", |l| l.name),
            LanguageRow::Detect => MenuCommand::DetectLanguage.label(),
            LanguageRow::Plain => MenuCommand::UsePlainText.label(),
        }
    }

    /// 这一行选用的语言；「自动检测」没有。
    pub(crate) fn language(self) -> Option<&'static winc_syntax::Language> {
        match self {
            LanguageRow::Language(i) => winc_syntax::languages().get(i).copied(),
            LanguageRow::Plain => Some(winc_syntax::detect("")),
            LanguageRow::Detect => None,
        }
    }

    /// 名字后面淡色写出的扩展名（如 `.rs`）；不是具体语言或没有扩展名时为空。
    pub(crate) fn extensions(self) -> String {
        let LanguageRow::Language(i) = self else {
            return String::new();
        };
        let exts = winc_syntax::languages()
            .get(i)
            .map_or(&[][..], |l| l.extensions);
        (exts.iter().take(3))
            .map(|e| format!(".{e}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 非空查询 `needle` 与语言 `row` 的匹配等级，越小越靠前；不匹配时为 `None`。
///
/// 查询恰好是该语言的一个扩展名时排在最前（`ts`、`md`），其次是名字中连续出现，最后是分散出现
/// （见 [`rate`]）。
fn rank(needle: &[char], row: LanguageRow) -> Option<(u8, u32)> {
    if let LanguageRow::Language(i) = row
        && let Some(language) = winc_syntax::languages().get(i)
    {
        let query: String = needle.iter().collect();
        if language.extensions.iter().any(|e| *e == query) {
            return Some((0, 0));
        }
    }
    let score = rate(needle, row.title())?;
    Some((if score < 1000 { 1 } else { 2 }, score))
}

impl App {
    /// 命令「更改文件的语言」：在面板中列出全部语言，选中当前文档的语言。
    ///
    /// 输出标签（格式化的结果）没有这一项，不打开。
    pub(in crate::app) fn open_language_menu(&mut self) {
        if self.editor.active().is_none_or(|d| d.title.is_some()) {
            return;
        }
        self.finish_name_input();
        let picker = &mut self.overlays.picker;
        picker.places = None;
        picker.branch_rows = None;
        picker.location_rows = None;
        picker.folders = false;
        picker.vim_list = None;
        picker.remote = None;
        picker.languages = true;
        picker.open = true;
        picker.doc = self.editor.active().map(|d| d.id);
        picker.input.set_text("");
        self.focus = Focus::Picker;
        self.picker_refresh();
        // 停在当前的语言上（纯文本停在「纯文本」）：回车不改变，上下移动即可换到相邻的语言。
        let current = self.editor.active().map(|d| d.highlighter.language().id);
        let plain = current == Some(winc_syntax::detect("").id);
        let at = (self.overlays.picker.hits.iter()).position(|&hit| match LanguageRow::of(hit) {
            Some(LanguageRow::Plain) => plain,
            Some(LanguageRow::Language(i)) => {
                winc_syntax::languages().get(i).map(|l| l.id) == current
            }
            _ => false,
        });
        self.overlays.picker.selected = at.unwrap_or(0);
        self.relayout();
        self.picker_reveal();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按查询筛选语言：没有查询时先列「自动检测」（文档有文件名时）、「纯文本」和最近选择的
    /// 语言，之后按名字排列其余语言；有查询时按 [`rank`] 排列，同一等级内按名字排列。
    pub(in crate::app) fn language_picker_refresh(&mut self) {
        let needle = needle(self.overlays.picker.input.text());
        let named = self.editor.active().is_some_and(|d| d.path.is_some());
        let mut rows: Vec<LanguageRow> = named.then_some(LanguageRow::Detect).into_iter().collect();
        rows.push(LanguageRow::Plain);
        let all = winc_syntax::languages();
        let recent: Vec<usize> = (self.state.recent_languages.iter())
            .filter(|_| needle.is_empty())
            .filter_map(|id| all.iter().position(|l| l.id == id.as_str()))
            .collect();
        rows.extend(recent.iter().copied().map(LanguageRow::Language));
        let mut languages: Vec<LanguageRow> = (0..all.len())
            .filter(|i| !recent.contains(i))
            .map(LanguageRow::Language)
            .collect();
        languages.sort_by_key(|row| row.title().to_lowercase());
        rows.extend(languages);
        let hits = if needle.is_empty() {
            rows.into_iter().map(LanguageRow::hit).collect()
        } else {
            let mut ranked: Vec<((u8, u32), usize, LanguageRow)> = (rows.into_iter().enumerate())
                .filter_map(|(order, row)| Some((rank(&needle, row)?, order, row)))
                .collect();
            ranked.sort_by_key(|&(rank, order, _)| (rank, order));
            ranked.into_iter().map(|(_, _, row)| row.hit()).collect()
        };
        self.overlays.picker.hits = hits;
    }

    /// 第 `row` 行（`Picker::hits` 中的下标）的名字中命中查询的字符下标，绘制时标出。
    pub(crate) fn language_matched(&self, row: usize) -> Vec<usize> {
        let needle = needle(self.overlays.picker.input.text());
        (self.overlays.picker.hits.get(row))
            .and_then(|&hit| LanguageRow::of(hit))
            .map_or_else(Vec::new, |row| matched(&needle, row.title()))
    }

    /// 使用选中的语言，与状态栏语言菜单中选中同一项相同。
    pub(in crate::app) fn language_picker_accept(&mut self) {
        let hit = self.overlays.picker.hits.get(self.overlays.picker.selected);
        let Some(row) = hit.and_then(|&hit| LanguageRow::of(hit)) else {
            return;
        };
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return;
        };
        self.close_picker();
        self.focus = Focus::Editor;
        let command = match row {
            LanguageRow::Detect => MenuCommand::DetectLanguage,
            LanguageRow::Plain => MenuCommand::UsePlainText,
            LanguageRow::Language(i) => MenuCommand::UseLanguage(i),
        };
        self.run_status_command(command, id);
    }
}
