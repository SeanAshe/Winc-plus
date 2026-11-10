//! 最近打开的目录（`⌃R`）：在快速打开的卡片里列出最近列表中的目录，按目录名或上级目录筛选。
//!
//! 回车在此窗口打开（窗口里开着别的目录时按 `editor.openFolderInNewWindow` 决定），按住 `⌥`
//! 在新窗口打开，与开始页、项目名菜单中 `⌥` 的含义相同。

use super::*;
use crate::app::input::palette::{matched, needle, rate};

/// 返回最近目录 `folder` 在列表中显示的两部分：目录名，以及淡色显示的上级目录（用户主目录
/// 缩写为 `~`）。
pub(crate) fn folder_label(folder: &Path) -> (String, String) {
    let name = winc_editor::display_name(folder).into_owned();
    let parent = folder
        .parent()
        .map(|p| winc_editor::display_path(p, None))
        .unwrap_or_default();
    (name, parent)
}

/// 返回非空查询 `needle` 与目录的匹配等级，越小越靠前；不匹配时为 `None`。
///
/// 目录名命中的排在上级目录命中的前面；命中位置相同时，连续出现的排在分散出现的前面（见
/// [`rate`]）。
fn folder_rank(needle: &[char], name: &str, parent: &str) -> Option<u8> {
    let tier = |score: u32| u8::from(score >= 1000);
    rate(needle, name)
        .map(tier)
        .or_else(|| rate(needle, parent).map(|score| 2 + tier(score)))
}

impl App {
    /// 命令「打开最近的目录」（`⌃R`）：在面板中列出最近列表中的目录，当前目录除外。
    ///
    /// 第一条是上一次打开的目录，按 `⌃R` 再按回车即可切回；列表已经打开时，再按一次选中下一条。
    /// 没有打开目录时同样可用。
    pub(in crate::app) fn open_recent_folders(&mut self) {
        if self.picker.open && self.picker.folders {
            self.picker_move(true);
            return;
        }
        self.finish_name_input();
        self.picker.places = None;
        self.picker.branch_rows = None;
        self.picker.location_rows = None;
        self.picker.folders = true;
        self.picker.open = true;
        self.picker.doc = self.editor.active().map(|d| d.id);
        self.picker.input.set_text("");
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按查询筛选最近的目录，按 [`folder_rank`] 排列，同一等级内保持最近列表的顺序；没有查询
    /// 时按最近列表的顺序全部列出。
    pub(in crate::app) fn folder_picker_refresh(&mut self) {
        let needle = needle(self.picker.input.text());
        let current = self.editor.root();
        let mut ranked: Vec<(u8, usize)> = (self.state.recent.iter().enumerate())
            .filter(|(_, folder)| Some(folder.as_path()) != current)
            .filter_map(|(i, folder)| {
                if needle.is_empty() {
                    return Some((0, i));
                }
                let (name, parent) = folder_label(folder);
                folder_rank(&needle, &name, &parent).map(|rank| (rank, i))
            })
            .collect();
        ranked.sort_unstable();
        self.picker.hits = ranked.into_iter().map(|(_, i)| i).collect();
    }

    /// 返回第 `row` 行（`Picker::hits` 中的下标）的目录名、上级目录中命中查询的字符下标，绘制时
    /// 标出这些字符。与 [`folder_rank`] 一致：目录名命中时只标目录名。
    pub(crate) fn folder_matched(&self, row: usize) -> (Vec<usize>, Vec<usize>) {
        let needle = needle(self.picker.input.text());
        let folder = (self.picker.hits.get(row)).and_then(|&i| self.state.recent.get(i));
        let Some(folder) = folder.filter(|_| !needle.is_empty()) else {
            return (Vec::new(), Vec::new());
        };
        let (name, parent) = folder_label(folder);
        let in_name = matched(&needle, &name);
        if in_name.is_empty() {
            (Vec::new(), matched(&needle, &parent))
        } else {
            (in_name, Vec::new())
        }
    }

    /// 打开选中的目录：按住 `⌥` 时在新窗口打开，否则与在开始页上点击相同。目录已不存在时从
    /// 最近列表中移除并提示。
    pub(in crate::app) fn folder_picker_accept(&mut self) {
        let Some(&index) = self.picker.hits.get(self.picker.selected) else {
            return;
        };
        let new_window = self.modifiers.alt_key();
        self.close_picker();
        if new_window {
            self.open_recent_in_new_window(index);
        } else {
            self.run_start_action(StartAction::Recent(index));
        }
    }

    /// 点击第 `row` 行行尾的「✕」：把该目录从最近列表中移除。面板保持打开，选中项留在原来的
    /// 位置。
    pub(in crate::app) fn forget_picker_folder(&mut self, row: usize) {
        let folder = (self.picker.hits.get(row)).and_then(|&i| self.state.recent.get(i));
        let Some(folder) = folder.cloned() else {
            return;
        };
        let selected = self.picker.selected;
        self.update_state(|state| state.forget(&folder));
        self.picker_refresh();
        self.picker.selected = selected.min(self.picker.hits.len().saturating_sub(1));
        self.relayout();
        self.request_redraw();
    }
}
