//! 最近打开的项目（`⌃R`）：在快速打开的卡片里先列出最近列表中的目录，再列出最近从窗口外打开的
//! 文件，按名字或上级目录筛选。
//!
//! 目录回车在此窗口打开（窗口里开着别的目录时按 `editor.openFolderInNewWindow` 决定），文件在
//! 此窗口的标签中打开；按住 `⌥` 在新窗口打开，与开始页、项目名菜单中 `⌥` 的含义相同。开始页和
//! 此面板中的同名目录按 [`parents`] 区分。

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

/// 「打开最近的项目」面板里的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recent<'a> {
    /// 最近的目录。
    Folder(&'a Path),
    /// 最近打开的文件。
    File(&'a Path),
}

impl App {
    /// 面板中第 `index` 项（`Picker::hits` 的元素）：最近目录之后继续是最近文件。
    pub(crate) fn recent_entry(&self, index: usize) -> Option<Recent<'_>> {
        let folders = &self.state.recent;
        match index.checked_sub(folders.len()) {
            None => folders.get(index).map(|f| Recent::Folder(f)),
            Some(i) => self.state.recent_files.get(i).map(|f| Recent::File(f)),
        }
    }

    /// 面板中第 `index` 项显示的名字和上级目录：目录的名字是它的组名（见
    /// [`App::recent_title`]），文件是文件名。
    fn recent_entry_label(&self, index: usize) -> Option<(String, String)> {
        match self.recent_entry(index)? {
            Recent::Folder(folder) => Some((self.recent_title(folder), folder_label(folder).1)),
            Recent::File(file) => Some(folder_label(file)),
        }
    }

    /// 命令「打开最近的项目」（`⌃R`）：在面板中列出最近列表中的目录（当前目录除外）和文件。
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

    /// 按查询筛选最近的目录和文件，按 [`folder_rank`] 排列，同一等级内目录在前、保持最近列表的
    /// 顺序；没有查询时按最近列表的顺序全部列出。
    pub(in crate::app) fn folder_picker_refresh(&mut self) {
        let needle = needle(self.overlays.picker.input.text());
        let current = self.editor.root();
        let count = self.state.recent.len() + self.state.recent_files.len();
        let mut ranked: Vec<(u8, usize)> = (0..count)
            .filter(
                |&i| !matches!(self.recent_entry(i), Some(Recent::Folder(f)) if Some(f) == current),
            )
            .filter_map(|i| {
                if needle.is_empty() {
                    return Some((0, i));
                }
                // 按显示的名字（目录的组名）查找，目录也可以按它本身的名字查找。
                let (title, parent) = self.recent_entry_label(i)?;
                let own = match self.recent_entry(i)? {
                    Recent::Folder(folder) => folder_label(folder).0,
                    Recent::File(_) => title.clone(),
                };
                folder_rank(&needle, &title, &parent)
                    .or_else(|| folder_rank(&needle, &own, &parent))
                    .map(|rank| (rank, i))
            })
            .collect();
        ranked.sort_unstable();
        self.overlays.picker.hits = ranked.into_iter().map(|(_, i)| i).collect();
    }

    /// 返回第 `row` 行（`Picker::hits` 中的下标）的名字、上级目录中命中查询的字符下标，绘制时
    /// 标出这些字符。与 [`folder_rank`] 一致：名字命中时只标名字。
    pub(crate) fn folder_matched(&self, row: usize) -> (Vec<usize>, Vec<usize>) {
        let needle = needle(self.overlays.picker.input.text());
        let label = (self.overlays.picker.hits.get(row)).and_then(|&i| self.recent_entry_label(i));
        let Some((name, parent)) = label.filter(|_| !needle.is_empty()) else {
            return (Vec::new(), Vec::new());
        };
        let in_name = matched(&needle, &name);
        if in_name.is_empty() {
            (Vec::new(), matched(&needle, &parent))
        } else {
            (in_name, Vec::new())
        }
    }

    /// 打开选中的目录或文件：按住 `⌥` 时在新窗口打开；否则目录与在开始页上点击相同，文件在
    /// 此窗口打开。已不存在时从最近列表中移除并提示。
    pub(in crate::app) fn folder_picker_accept(&mut self) {
        let Some(&index) = self.overlays.picker.hits.get(self.overlays.picker.selected) else {
            return;
        };
        let new_window = self.pointer.modifiers.alt_key();
        self.close_picker();
        if let Some(Recent::File(file)) = self.recent_entry(index) {
            let file = file.to_path_buf();
            return self.open_recent_file(&file, new_window);
        }
        if new_window {
            self.open_recent_in_new_window(index);
        } else {
            self.run_start_action(StartAction::Recent(index));
        }
    }

    /// 打开最近的文件 `file`：`new_window` 时在新窗口打开；已不存在时从最近列表中移除并提示。
    fn open_recent_file(&mut self, file: &Path, new_window: bool) {
        if !self.may_exist(file) {
            let gone = file.to_path_buf();
            self.update_state(move |state| state.forget_file(&gone));
            let path = winc_editor::display_path(file, None);
            self.notify(
                NoticeKind::Warning,
                winc_i18n::t!("workspace-recent-missing", path = path),
            );
            return;
        }
        if new_window {
            self.open_in_new_window(file);
        } else {
            self.open_path(file);
            self.note_opened_file(file);
        }
    }

    /// 点击第 `row` 行行尾的「✕」：把该目录或文件从最近列表中移除。面板保持打开，选中项留在原来
    /// 的位置。
    pub(in crate::app) fn forget_picker_folder(&mut self, row: usize) {
        let entry = (self.overlays.picker.hits.get(row)).and_then(|&i| self.recent_entry(i));
        let Some(entry) = entry.map(|e| match e {
            Recent::Folder(f) => (true, f.to_path_buf()),
            Recent::File(f) => (false, f.to_path_buf()),
        }) else {
            return;
        };
        let selected = self.overlays.picker.selected;
        self.update_state(move |state| match entry {
            (true, folder) => state.forget(&folder),
            (false, file) => state.forget_file(&file),
        });
        self.picker_refresh();
        self.overlays.picker.selected =
            selected.min(self.overlays.picker.hits.len().saturating_sub(1));
        self.relayout();
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
