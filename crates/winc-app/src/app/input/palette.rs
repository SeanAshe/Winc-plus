//! 命令面板（`⇧⌘P`）。
//!
//! 命令面板与快速打开共用同一张卡片：输入框以 `>` 开头时列出命令，否则列出文件。
//! 命令列表和快捷键都来自键位表。本模块同时提供命令、符号两种列表共用的模糊匹配函数
//! [`needle`] 和 [`rate`]。

use super::*;
use crate::keymap::{self, Action};

/// 命令面板列出的命令列表。
///
/// 内容是键位表中 `in_palette` 为真的动作，顺序与键位表一致；快捷键取自键位表。
pub fn palette() -> &'static [Action] {
    static LIST: std::sync::OnceLock<Vec<Action>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        keymap::ACTIONS
            .iter()
            .copied()
            .filter(|a| a.in_palette())
            .collect()
    })
}

/// 按 `query` 搜索命令，返回 [`palette()`] 中的下标，匹配度高的在前。
///
/// 在标题或英文名中连续出现的匹配优于分散出现的匹配，位置越靠前越好。匹配不区分大小写，
/// 忽略空白字符。`query` 为空时按原顺序返回全部命令。
pub fn search_palette(query: &str) -> Vec<usize> {
    let needle = needle(query);
    if needle.is_empty() {
        return (0..palette().len()).collect();
    }

    let mut scored: Vec<(u32, usize)> = palette()
        .iter()
        .enumerate()
        .filter_map(|(i, action)| {
            let best = [action.title(), action.english()]
                .into_iter()
                .filter_map(|text| rate(&needle, text))
                .min()?;
            Some((best, i))
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, i)| i).collect()
}

/// 将查询串规范化为字符序列：转为小写，并去掉空白字符。
pub(in crate::app) fn needle(query: &str) -> Vec<char> {
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 返回 `needle` 与 `text` 的匹配得分，得分越小匹配度越高；不匹配时返回 `None`。
///
/// 得分规则：连续出现时为首个匹配在 `text`（去空白、转小写后）中的字符偏移；
/// 仅按顺序分散出现时为 1000 加上首个字符的偏移，因此总是排在连续匹配之后。
///
/// # Panics
///
/// `needle` 为空时 panic（`windows(0)` 不合法），调用方需要先排除空查询。
pub(in crate::app) fn rate(needle: &[char], text: &str) -> Option<u32> {
    let hay: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if let Some(at) = hay.windows(needle.len()).position(|w| w == needle) {
        return Some(at as u32);
    }
    // 分散出现：needle 的字符按顺序都能找到，得分加 1000，排在连续匹配之后。
    let mut rest = hay.iter();
    let mut first = None;
    for c in needle {
        let at = rest.position(|h| h == c)?;
        first.get_or_insert(at);
    }
    Some(1000 + first.unwrap_or(0) as u32)
}

/// 返回 `needle` 在 `text` 里命中的字符下标，面板据此把这些字符标出来。
///
/// 下标按 `text` 原文计算，升序排列。规则与 [`rate`] 相同：连续出现时是第一处那一段，否则
/// 是按顺序最先找到的几处；不命中时为空。
pub(in crate::app) fn matched(needle: &[char], text: &str) -> Vec<usize> {
    // 去空白、转小写后的每个字符，连同它在原文里是第几个字符。
    let hay: Vec<(char, usize)> = text
        .chars()
        .enumerate()
        .filter(|(_, c)| !c.is_whitespace())
        .flat_map(|(i, c)| c.to_lowercase().map(move |l| (l, i)))
        .collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let same = |w: &[(char, usize)]| w.iter().map(|h| h.0).eq(needle.iter().copied());
    let mut out: Vec<usize> = match hay.windows(needle.len()).position(same) {
        Some(at) => hay[at..at + needle.len()].iter().map(|h| h.1).collect(),
        None => {
            let mut rest = hay.iter();
            let found: Option<Vec<usize>> = needle
                .iter()
                .map(|c| rest.find(|h| h.0 == *c).map(|h| h.1))
                .collect();
            found.unwrap_or_default()
        }
    };
    out.dedup();
    out
}

impl App {
    /// 命令面板里查询（去掉开头的 `>`）在命令名 `text` 里命中的字符下标：画的时候标出来。
    pub(crate) fn palette_matched(&self, text: &str) -> Vec<usize> {
        matched(
            &needle(self.picker.input.text().trim_start_matches('>')),
            text,
        )
    }

    /// 打开命令面板（`⇧⌘P`）。
    ///
    /// 命令面板即预填 `>` 的快速打开；删除 `>` 后回到文件列表。
    pub(in crate::app) fn open_palette(&mut self) {
        self.finish_name_input();
        self.picker.places = None;
        self.picker.branch_rows = None;
        self.picker.location_rows = None;
        self.picker.folders = false;
        self.picker.open = true;
        self.picker.doc = self.editor.active().map(|d| d.id);
        self.picker.input.set_text(">");
        // 光标放在 `>` 之后，输入的字符作为命令查询，而不是替换 `>`。
        self.picker.input.set_caret(1, false);
        self.picker.selected = 0;
        self.picker.scroll = 0.0;
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 切换命令面板（`⇧⌘P`、F1）：已显示命令列表时关闭，否则打开。
    pub(super) fn toggle_palette(&mut self) {
        if self.picker.open && self.picker_lists_commands() {
            self.close_picker();
        } else {
            self.open_palette();
        }
    }

    /// 是否正在列出命令，即输入框以 `>` 开头且不在「位置」、分支、最近目录等列表中。
    pub(crate) fn picker_lists_commands(&self) -> bool {
        self.picker.places.is_none()
            && !self.picker_lists_branches()
            && !self.picker_lists_locations()
            && !self.picker.folders
            && self.picker.input.text().starts_with('>')
    }

    /// 按当前查询刷新命令列表；查询为空时，最近执行过的命令排在最前。
    pub(in crate::app) fn palette_refresh(&mut self) {
        let query = self.picker.input.text().trim_start_matches('>').to_owned();
        let mut hits = search_palette(&query);
        // 关着的模块的命令不列出来。
        hits.retain(|&i| self.action_available(palette()[i]));
        self.picker.recent = 0;
        if query.trim().is_empty() {
            let recent: Vec<usize> = self
                .recent_commands
                .iter()
                .filter(|&&a| self.action_available(a))
                .filter_map(|&a| palette().iter().position(|&e| e == a))
                .collect();
            hits.retain(|i| !recent.contains(i));
            self.picker.recent = recent.len();
            hits.splice(0..0, recent);
        }
        self.picker.hits = hits;
    }

    /// 执行选中的命令：关闭面板，并将该命令记为最近执行（最多保留 5 条）。
    pub(in crate::app) fn palette_accept(&mut self) {
        let Some(&action) = self
            .picker
            .hits
            .get(self.picker.selected)
            .and_then(|&i| palette().get(i))
        else {
            return;
        };
        self.close_picker();
        self.recent_commands.retain(|&a| a != action);
        self.recent_commands.insert(0, action);
        self.recent_commands.truncate(5);
        self.run_action(action);
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
