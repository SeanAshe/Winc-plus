//! 最近打开的目录（`⌃R`）：在快速打开的卡片里列出最近列表中的目录，按目录名或上级目录筛选。
//!
//! 回车在此窗口打开（窗口里开着别的目录时按 `editor.openFolderInNewWindow` 决定），按住 `⌥`
//! 在新窗口打开，与开始页、项目名菜单中 `⌥` 的含义相同。开始页和此面板中的同名目录按
//! [`parents`] 区分。

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

/// 最近目录在列表中淡色显示的上级目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parent {
    /// 上级目录的显示写法（用户主目录缩写为 `~`，远端目录写成 `目标:/路径`）。
    pub(crate) text: String,
    /// 最近列表中另有同名目录时，[`Self::text`] 中区分各同名目录所需的末尾一段的起点（字节
    /// 下标，见 [`distinct_start`]）；没有同名目录时为 `None`。
    pub(crate) keep: Option<usize>,
}

/// 返回 `folders` 各目录在列表中显示的上级目录，顺序与 `folders` 相同。
pub(crate) fn parents(folders: &[PathBuf]) -> Vec<Parent> {
    let names: Vec<_> = folders
        .iter()
        .map(|f| winc_editor::display_name(f))
        .collect();
    let texts: Vec<String> = (folders.iter())
        .map(|f| {
            f.parent()
                .map(|p| winc_editor::display_path(p, None))
                .unwrap_or_default()
        })
        .collect();
    (0..folders.len())
        .map(|i| {
            let others: Vec<&str> = (0..folders.len())
                .filter(|&j| j != i && names[j] == names[i])
                .map(|j| texts[j].as_str())
                .collect();
            let keep = (!others.is_empty()).then(|| distinct_start(&texts[i], &others));
            Parent {
                text: texts[i].clone(),
                keep,
            }
        })
        .collect()
}

/// 返回上级目录 `parent` 末尾能与 `others`（其他同名目录的上级目录）区分开的一段的起点（字节
/// 下标）。
///
/// 从最后一级目录起，不足以区分时逐级向前添加，直到与每一个都不同，如 `a/src` 与 `b/src`
/// 只需保留 `a/src`；无法区分时为 0，即整个路径。
pub(crate) fn distinct_start(parent: &str, others: &[&str]) -> usize {
    fn starts(p: &str) -> Vec<usize> {
        std::iter::once(0)
            .chain(p.match_indices(['/', '\\']).map(|(i, _)| i + 1))
            .collect()
    }
    fn tail<'a>(p: &'a str, starts: &[usize], k: usize) -> Option<&'a str> {
        starts.len().checked_sub(k).map(|j| &p[starts[j]..])
    }
    let mine = starts(parent);
    let theirs: Vec<Vec<usize>> = others.iter().map(|other| starts(other)).collect();
    (1..=mine.len())
        .find(|&k| {
            let own = tail(parent, &mine, k);
            (others.iter().zip(&theirs)).all(|(other, starts)| tail(other, starts, k) != own)
        })
        .map_or(0, |k| mine[mine.len() - k])
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
        if self.overlays.picker.open && self.overlays.picker.folders {
            self.picker_move(true);
            return;
        }
        self.finish_name_input();
        self.overlays.picker.places = None;
        self.overlays.picker.branch_rows = None;
        self.overlays.picker.location_rows = None;
        self.overlays.picker.remote = None;
        self.overlays.picker.languages = false;
        self.overlays.picker.folders = true;
        self.overlays.picker.open = true;
        self.overlays.picker.doc = self.editor.active().map(|d| d.id);
        self.overlays.picker.input.set_text("");
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按查询筛选最近的目录，按 [`folder_rank`] 排列，同一等级内保持最近列表的顺序；没有查询
    /// 时按最近列表的顺序全部列出。
    pub(in crate::app) fn folder_picker_refresh(&mut self) {
        let needle = needle(self.overlays.picker.input.text());
        let current = self.editor.root();
        let mut ranked: Vec<(u8, usize)> = (self.state.recent.iter().enumerate())
            .filter(|(_, folder)| Some(folder.as_path()) != current)
            .filter_map(|(i, folder)| {
                if needle.is_empty() {
                    return Some((0, i));
                }
                // 按显示的名字（组名）查找，也可以按目录本身的名字查找。
                let (name, parent) = folder_label(folder);
                let title = self.recent_title(folder);
                folder_rank(&needle, &title, &parent)
                    .or_else(|| folder_rank(&needle, &name, &parent))
                    .map(|rank| (rank, i))
            })
            .collect();
        ranked.sort_unstable();
        self.overlays.picker.hits = ranked.into_iter().map(|(_, i)| i).collect();
    }

    /// 返回第 `row` 行（`Picker::hits` 中的下标）的目录名、上级目录中命中查询的字符下标，绘制时
    /// 标出这些字符。与 [`folder_rank`] 一致：目录名命中时只标目录名。
    pub(crate) fn folder_matched(&self, row: usize) -> (Vec<usize>, Vec<usize>) {
        let needle = needle(self.overlays.picker.input.text());
        let folder = (self.overlays.picker.hits.get(row)).and_then(|&i| self.state.recent.get(i));
        let Some(folder) = folder.filter(|_| !needle.is_empty()) else {
            return (Vec::new(), Vec::new());
        };
        let (_, parent) = folder_label(folder);
        let in_name = matched(&needle, &self.recent_title(folder));
        if in_name.is_empty() {
            (Vec::new(), matched(&needle, &parent))
        } else {
            (in_name, Vec::new())
        }
    }

    /// 打开选中的目录：按住 `⌥` 时在新窗口打开，否则与在开始页上点击相同。目录已不存在时从
    /// 最近列表中移除并提示。
    pub(in crate::app) fn folder_picker_accept(&mut self) {
        let Some(&index) = self.overlays.picker.hits.get(self.overlays.picker.selected) else {
            return;
        };
        let new_window = self.pointer.modifiers.alt_key();
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
        let folder = (self.overlays.picker.hits.get(row)).and_then(|&i| self.state.recent.get(i));
        let Some(folder) = folder.cloned() else {
            return;
        };
        let selected = self.overlays.picker.selected;
        self.update_state(|state| state.forget(&folder));
        self.picker_refresh();
        self.overlays.picker.selected =
            selected.min(self.overlays.picker.hits.len().saturating_sub(1));
        self.relayout();
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
