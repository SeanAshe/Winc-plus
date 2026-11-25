//! 在文件中查找的对话框：标题行是标题、结果统计，右端是范围（整个项目或当前文件所在的目录）、「在底
//! 部显示」和关闭按钮；下面依次是查询框（右端是搜索选项和「过滤」）、文件掩码、各处匹配和选中处的上
//! 下文。拖动标题行的空白处移动、拖动边缘改变大小，按 Esc、关闭按钮或打开一处结果时关闭。
//!
//! 「在底部显示」把结果放进底部面板的「搜索」标签：顶部一行是查询和结果统计，左边是各处匹配，右边是
//! 选中处的上下文，与对话框共用查询、结果和选中项；面板标题栏上的「在对话框中打开」回到对话框。

mod rows;

use std::path::PathBuf;
use std::time::Instant;

use winc_editor::{ProjectSearch, Query};
use winc_i18n::t;
use winc_render::Rect;
use winc_syntax::LineState;

use super::global::SEARCH_DELAY;
use super::picker::PICKER_ROW;
use super::*;
use crate::app::editor::definition::{Place, Places};
use crate::layout::{PanelButton, PickerParts};

/// 对话框中输入框和预览标题行的高度（逻辑像素）。
const FIELD_H: f32 = 28.0;
const PREVIEW_HEAD: f32 = 30.0;
/// 卡片顶边到标题行里按钮的距离（逻辑像素）；标题行下面、输入框下面也留这么多。
const INSET: f32 = 12.0;
/// 卡片左右两边到内容的距离（逻辑像素），同其他对话框：输入框的边框、列表和预览中选中一行的
/// 底色、标题的左边、关闭按钮里叉的右边都在这条线上。
pub(crate) const SIDE: f32 = crate::theme::SETTINGS_PAD;
/// 快速打开面板的一行里，底色比行缩进的距离（逻辑像素，见 `draw_rows`）。
const ROW_INSET: f32 = 6.0;
/// 输入的文字、列表和预览的文件图标共用的左边（离卡片左边，逻辑像素）：输入框的边框往里
/// 8，列表的图标在行里缩进 `PICKER_ICON_X`。
const TEXT_X: f32 = SIDE - ROW_INSET + crate::app::PICKER_ICON_X;
/// 关闭按钮里叉的边长（逻辑像素）。
pub(crate) const CLOSE_MARK: f32 = 7.0;
/// 范围分段控件中一段的宽度（逻辑像素）。
const SEGMENT_W: f32 = 72.0;
/// 预览区上下的留白（逻辑像素）。
pub(crate) const PREVIEW_PAD: f32 = 4.0;
/// 边缘上可以拖动改变大小的宽度（逻辑像素）。
const EDGE: f32 = 6.0;
/// 对话框的默认大小和最小大小（逻辑像素）。
const DEFAULT_SIZE: (f32, f32) = (760.0, 600.0);
const MIN_SIZE: (f32, f32) = (480.0, 360.0);

/// 对话框中可以点击的部位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    /// 标题行的空白处：按住拖动。
    Title,
    /// 右上角的关闭按钮。
    Close,
    /// 关闭按钮左边的「在底部显示」。
    Dock,
    /// 查询输入框。
    Query,
    /// 查询框右端的「过滤」：显示或隐藏文件掩码。
    MaskToggle,
    /// 文件掩码输入框。
    Mask,
    /// 范围：0 整个项目，1 当前文件所在的目录。
    Scope(usize),
    /// 搜索选项开关：0 区分大小写，1 全字匹配，2 正则。
    Option(usize),
    /// 结果列表中的一行。
    Row(usize),
    /// 其余部分（预览区）。
    Body,
    /// 边缘：横向、纵向各自是 -1（左、上）、0 或 1（右、下）。
    Edge(i8, i8),
}

/// 对话框各部件的位置。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DialogParts {
    pub card: Rect,
    /// 标题行：按住可以拖动对话框，从卡片顶边到 `bar` 的下沿。
    pub header: Rect,
    /// 标题行里放标题、结果统计和按钮的一条：左边是标题和结果统计，右端依次是范围、「在底部
    /// 显示」和关闭按钮。
    pub bar: Rect,
    pub close: Rect,
    pub dock: Rect,
    /// 范围分段控件的底槽和其中的两段（整个项目、当前文件所在的目录）。
    pub track: Rect,
    pub scopes: [Rect; 2],
    /// 查询框、其中查询文字的区域、搜索选项开关和「过滤」。
    pub query_row: Rect,
    pub query: Rect,
    pub options: [Rect; 3],
    pub mask_toggle: Rect,
    /// 文件掩码的输入框；隐藏时高度为 0。
    pub mask: Rect,
    /// 结果列表，按快速打开面板的行排布。
    pub list: PickerParts,
    /// 预览的标题行（文件名和所在目录）和代码区。
    pub preview_head: Rect,
    pub preview: Rect,
}

/// 在文件中查找的对话框的状态；关闭后保留，下次打开沿用位置、大小、查询、选项和结果。
#[derive(Default)]
pub struct SearchDialog {
    pub(crate) open: bool,
    /// 拖动或改过大小后的位置；`None` 时在编辑区中居中。
    pub(crate) frame: Option<Rect>,
    pub(crate) query: crate::field::TextField,
    /// 文件掩码：逗号分隔的 glob；显示时只搜索匹配的文件。
    pub(crate) mask: crate::field::TextField,
    pub(crate) mask_on: bool,
    /// 只在当前文件所在的目录中搜索；目录在打开对话框时记下。
    pub(crate) in_dir: bool,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) engine: ProjectSearch,
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    /// 查询变化后开始搜索的时刻。
    pub(crate) due: Option<Instant>,
    /// 列出的各处匹配，沿用位置浮窗的条目。
    pub(crate) places: Option<Places>,
    /// 列出的条目序号（全部条目）。
    pub(crate) hits: Vec<usize>,
    /// 各条目匹配的长度（字符）。
    pub(crate) lens: Vec<usize>,
    version: Option<u64>,
    pub(crate) selected: usize,
    pub(crate) scroll: f32,
    /// 预览区的内容，见 [`Context`]。
    context: Option<Context>,
    /// 结果各行的高亮状态，见 [`rows::RowStates`]。
    pub(crate) rows: rows::RowStates,
    /// 新的搜索还没有结果，列出的仍是上一次的结果（淡色显示）。
    pub(crate) stale: bool,
    /// 上一次的结果换成新的结果时回到第一条。
    reset_pending: bool,
    /// 结果放在底部面板的「搜索」标签中。
    pub(crate) docked: bool,
    /// 底部面板正在显示「搜索」标签（而不是终端）。
    pub(crate) in_panel: bool,
}

/// 缓存的预览内容。
struct Context {
    /// 选中处的路径和行号。
    at: (PathBuf, usize),
    /// 要取的行数；文件较短时多于 `lines` 的行数。
    taken: usize,
    /// 首行的行号。
    first: usize,
    lines: Vec<String>,
    /// 首行行首的高亮状态：跨行的结构（块注释、Vue 的 `<script>`）中的行据此着色。
    state: LineState,
}

/// 预览没有打开的文件时，从文件开头推算高亮状态的行数上限（行）：与编辑区每帧推算的上限相同，
/// 约一次屏幕刷新的耗时。更靠后的行从上方几百行处推算
/// （见 [`winc_editor::line_highlight_states`]）。
pub(super) const CONTEXT_STATE_BUDGET: usize = 4_000;

/// 底部面板「搜索」标签中各部件的位置。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DockParts {
    /// 顶部一行，与快速打开的输入行相同：放大镜、查询，右端是结果统计和三个搜索选项开关；
    /// `query` 是查询文字最多能占的区域。
    pub summary: Rect,
    pub query: Rect,
    pub options: [Rect; 3],
    /// 结果列表，按快速打开面板的行排布。
    pub list: PickerParts,
    /// 右边的预览区。
    pub preview: Rect,
}

impl SearchDialog {
    /// 测试用：丢掉预览区的缓存，下一次重新读取。
    #[cfg(test)]
    pub(crate) fn context_forget(&mut self) {
        self.context = None;
    }

    fn query_of(&self) -> Query {
        Query {
            text: self.query.text().to_owned(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include: if self.mask_on {
                self.mask.text().to_owned()
            } else {
                String::new()
            },
            ..Query::default()
        }
    }
}

impl App {
    /// 「在文件中查找」打开在哪里：设置中没写时按键位方案，`idea` 为对话框。
    pub(crate) fn search_style(&self) -> winc_config::SearchStyle {
        use crate::keymap::Preset;
        use winc_config::SearchStyle;
        self.config.find_in_files.unwrap_or_else(|| {
            let keymap = self.config.keymap.as_deref().and_then(Preset::from_key);
            match keymap.unwrap_or_default() {
                Preset::Idea => SearchStyle::Dialog,
                Preset::VsCode => SearchStyle::Sidebar,
            }
        })
    }

    /// 打开对话框：焦点在编辑区、有合适的选区（没有选区时为光标所在的词）时用它作为查询，否则
    /// 沿用上次的查询（见 [`Self::files_search_seed`]）；查询全选，焦点放进输入框。
    pub(in crate::app) fn open_search_dialog(&mut self) {
        let seed = self.files_search_seed();
        self.open_search_dialog_with(seed);
    }

    /// 从侧栏的搜索面板打开对话框，带上面板的查询和搜索选项；面板没有查询时与
    /// [`Self::open_search_dialog`] 相同。
    pub(in crate::app) fn open_search_dialog_from_panel(&mut self) {
        let g = &self.panels.global;
        if g.query.text().is_empty() {
            return self.open_search_dialog();
        }
        let seed = g.query.text().to_owned();
        let (case, word, regex) = (g.case_sensitive, g.whole_word, g.regex);
        let d = &mut self.search_dialog;
        let options = (d.case_sensitive, d.whole_word, d.regex);
        (d.case_sensitive, d.whole_word, d.regex) = (case, word, regex);
        if options != (case, word, regex) {
            d.engine.clear();
        }
        self.open_search_dialog_with(Some(seed));
    }

    /// 打开对话框，`seed` 有值时作为查询。
    pub(in crate::app) fn open_search_dialog_with(&mut self, seed: Option<String>) {
        if self.editor.workspace.is_none() {
            self.flash(t!("workspace-open-folder-first"));
            return;
        }
        let dir = (self.editor.active()).and_then(|doc| {
            doc.path
                .as_deref()?
                .parent()
                .map(std::path::Path::to_path_buf)
        });
        let d = &mut self.search_dialog;
        d.open = true;
        let moved = d.in_dir && d.dir != dir;
        d.dir = dir;
        let changed = moved
            | seed.is_some_and(|seed| {
                let changed = seed != d.query.text();
                if changed {
                    d.query.remember();
                    d.query.set_text(&seed);
                }
                changed
            });
        d.query.select_all();
        self.focus = Focus::SearchDialog;
        if changed || d.engine.query().is_none() {
            self.search_dialog_changed(true);
        }
        // 重新滚动，让选中的一行在对话框的列表中可见。
        let selected = self.search_dialog.selected;
        self.search_dialog_select(selected);
        self.reset_blink();
        self.request_redraw();
    }

    /// 关闭对话框，焦点回到编辑区。
    pub(in crate::app) fn close_search_dialog(&mut self) {
        self.search_dialog.open = false;
        self.search_dialog.query.remember();
        if matches!(self.focus, Focus::SearchDialog | Focus::SearchMask) {
            self.focus = Focus::Editor;
        }
        self.request_redraw();
    }

    /// 查询或选项变化：`now` 为 `true` 时立即搜索，否则等待 [`SEARCH_DELAY`]。
    pub(in crate::app) fn search_dialog_changed(&mut self, now: bool) {
        if now || self.search_dialog.query.text().is_empty() {
            self.run_search_dialog();
        } else {
            self.search_dialog.due = Some(Instant::now() + SEARCH_DELAY);
        }
        self.request_redraw();
    }

    /// 按对话框中的查询从头搜索整个目录；查询为空时清空结果。
    fn run_search_dialog(&mut self) {
        self.search_dialog.rows.reset();
        let Some(mut root) = self.editor.root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let mut extra = self.extra_folders();
        if self.search_dialog.in_dir
            && let Some(dir) = &self.search_dialog.dir
        {
            root = dir.clone();
            extra.clear();
        }
        let dirty = self.dirty_ropes();
        let proxy = self.link.proxy.clone();
        let d = &mut self.search_dialog;
        d.due = None;
        if d.query.text().is_empty() {
            d.engine.clear();
        } else {
            let wake = move || {
                if let Some(proxy) = &proxy {
                    let _ = proxy.send_event(UserEvent::Searched);
                }
            };
            let query = d.query_of();
            d.engine.set_extra_folders(extra);
            d.engine.start(&root, &query, dirty, wake);
        }
        self.sync_search_dialog(true);
    }

    /// 处理延迟的搜索和后台搜索的进展；返回下一次需要唤醒的时刻。
    pub(in crate::app) fn poll_search_dialog(&mut self, now: Instant) -> Option<Instant> {
        let d = &mut self.search_dialog;
        if let Some(due) = d.due {
            if now < due {
                return Some(due);
            }
            self.run_search_dialog();
            return None;
        }
        let running = d.engine.is_running();
        d.engine.poll();
        self.sync_search_dialog(false);
        // 列表显示在对话框里或底部面板中时，推算看得到的各行的高亮状态。
        let list = match self.docked_parts() {
            Some(dock) if self.search_dialog.docked => Some(dock.list.list),
            _ => self
                .search_dialog
                .open
                .then(|| self.search_dialog_parts().list.list),
        };
        if let Some(list) = list {
            self.advance_row_states(list.h);
        }
        running.then_some(now + crate::app::GLOBAL_POLL)
    }

    /// 结果变化后重新生成列表；`reset` 为 `true` 时回到第一条。
    fn sync_search_dialog(&mut self, reset: bool) {
        let d = &mut self.search_dialog;
        let version = d.engine.version();
        if d.version == Some(version) && !reset {
            return;
        }
        d.version = Some(version);
        // 新的搜索还没有结果时留着上一次的结果，避免列表先变空再出现。
        let waiting = d.engine.is_running() && d.engine.results.is_empty();
        if waiting && !d.hits.is_empty() {
            d.stale = true;
            d.reset_pending |= reset;
            return self.request_redraw();
        }
        let reset = reset || std::mem::take(&mut d.reset_pending);
        d.stale = false;
        let mut items = Vec::new();
        d.lens.clear();
        for file in &d.engine.results {
            for m in &file.matches {
                let start = m.preview[..m.highlight.start].chars().count();
                let len = m.preview[m.highlight.clone()].chars().count();
                items.push(Place {
                    path: file.path.clone(),
                    relative: file.relative.clone(),
                    line: m.line,
                    col: Some(m.cols.start),
                    preview: m.preview.clone(),
                    marks: std::iter::once(start..start + len).collect(),
                });
                d.lens.push(m.cols.len());
            }
        }
        d.hits = (0..items.len()).collect();
        d.places = Some(Places::search(d.query.text().to_owned(), items));
        if reset {
            d.selected = 0;
            d.scroll = 0.0;
        } else {
            d.selected = d.selected.min(d.hits.len().saturating_sub(1));
        }
        self.request_redraw();
    }

    /// 打开选中的一处并选中匹配的文字；在对话框中时关闭对话框，在底部面板中时结果留着。新的搜索
    /// 还没有结果时不打开。
    pub(in crate::app) fn search_dialog_accept(&mut self) {
        let d = &self.search_dialog;
        // 列出的还是上一次的结果，不打开。
        if d.stale {
            return;
        }
        let Some(place) = d.places.as_ref().and_then(|p| p.items.get(d.selected)) else {
            return;
        };
        let (path, line, col) = (place.path.clone(), place.line, place.col.unwrap_or(0));
        let len = d.lens.get(d.selected).copied().unwrap_or(0);
        if self.search_dialog.open {
            self.close_search_dialog();
        }
        self.open_path_then(&path, move |app| app.jump_to(line, col, len));
    }

    /// 正在显示的结果列表：对话框打开时是对话框中的，否则是底部面板中的。
    fn shown_list(&self) -> PickerParts {
        match self.docked_parts() {
            Some(dock) if !self.search_dialog.open => dock.list,
            _ => self.search_dialog_parts().list,
        }
    }

    /// 把结果放进底部面板的「搜索」标签并关闭对话框（`⌘↩`，Windows、Linux 上为 `Ctrl+↩`）。
    pub(in crate::app) fn dock_search_results(&mut self) {
        self.close_search_dialog();
        // 「搜索」标签在第一栏的标签栏上。
        let group = self.ensure_terminal_panel_group();
        self.focus_terminal_group(group);
        let d = &mut self.search_dialog;
        d.docked = true;
        d.in_panel = true;
        if !self.terminal.open {
            self.set_terminal_open(true);
            if self.terminal.height <= 0.0 {
                self.terminal.height = crate::app::terminal::DEFAULT_HEIGHT;
            }
        }
        self.focus = Focus::SearchDock;
        self.relayout();
        let selected = self.search_dialog.selected;
        self.search_dialog_select(selected);
        self.request_redraw();
    }

    /// 在底部面板中显示「搜索」标签，焦点放进结果列表。
    pub(in crate::app) fn show_docked_search(&mut self) {
        let group = self.ensure_terminal_panel_group();
        self.focus_terminal_group(group);
        self.search_dialog.in_panel = true;
        self.focus = Focus::SearchDock;
        self.request_redraw();
    }

    /// 关闭底部面板的「搜索」标签；面板里没有终端时一起收起。
    pub(in crate::app) fn close_docked_search(&mut self) {
        let d = &mut self.search_dialog;
        d.docked = false;
        d.in_panel = false;
        if self.focus_in_docked_search() {
            self.focus = Focus::Editor;
        }
        if self.terminal.panel_tab_count() == 0 && self.terminal.want.is_empty() {
            self.set_terminal_open(false);
        }
        self.relayout();
        self.request_redraw();
    }

    /// 底部面板正在显示「搜索」标签。
    pub(crate) fn docked_search_shown(&self) -> bool {
        let d = &self.search_dialog;
        d.docked && d.in_panel && self.terminal.open
    }

    /// 返回底部面板「搜索」标签中各部件的位置；没有显示时返回 `None`。
    pub(crate) fn docked_parts(&self) -> Option<DockParts> {
        if !self.docked_search_shown() {
            return None;
        }
        let card = self.layout.terminal;
        let top = card.y + crate::app::terminal::header_height(card);
        let body = Rect::new(card.x, top, card.w, (card.bottom() - top).max(0.0));
        use crate::app::{PICKER_PAD, PICKER_TEXT_X};
        use crate::theme::{BAR_HEIGHT, BUTTON_GAP, BUTTON_SMALL};
        let summary = Rect::new(body.x, body.y, body.w, BAR_HEIGHT);
        let side = BUTTON_SMALL;
        let square = |right: f32| {
            let y = summary.y + (summary.h - side) / 2.0;
            Rect::new(right - side, y, side, side)
        };
        let regex = square(summary.right() - 12.0);
        let word = square(regex.x - BUTTON_GAP);
        let case = square(word.x - BUTTON_GAP);
        let x = body.x + PICKER_TEXT_X;
        let query = Rect::new(x, summary.y, (case.x - 12.0 - x).max(0.0), summary.h);
        let split = (body.x + body.w * 0.55).round();
        // 列表放整数行，第一行与预览的第一行齐平。
        let top = summary.bottom();
        let room = (body.bottom() - top - 2.0 * PICKER_PAD).max(0.0);
        let rows = (room / PICKER_ROW).floor().max(1.0);
        let list = PickerParts {
            bar: summary,
            actions: Rect::new(body.x, top, 0.0, 0.0),
            list: Rect::new(body.x, top + PICKER_PAD, split - body.x, rows * PICKER_ROW),
            footer: Rect::new(body.x, body.bottom(), 0.0, 0.0),
        };
        let preview = Rect::new(
            split,
            top,
            body.right() - split,
            (body.bottom() - top).max(0.0),
        );
        Some(DockParts {
            summary,
            query,
            options: [case, word, regex],
            list,
            preview,
        })
    }

    /// 返回底部面板「搜索」标签中 (`x`, `y`) 处的部位；没有显示或不在其中时返回 `None`。
    pub(crate) fn docked_part_at(&self, x: f32, y: f32) -> Option<DialogPart> {
        let dock = self.docked_parts()?;
        if !self.layout.terminal.contains(x, y) || y < dock.summary.y {
            return None;
        }
        if let Some(i) = dock.options.iter().position(|r| r.contains(x, y)) {
            return Some(DialogPart::Option(i));
        }
        if dock.summary.contains(x, y) {
            return Some(DialogPart::Query);
        }
        let row = (dock.list.list.contains(x, y))
            .then(|| dock.list.row_at(y, self.search_dialog.scroll))
            .flatten()
            .filter(|&i| i < self.search_dialog.hits.len());
        Some(row.map_or(DialogPart::Body, DialogPart::Row))
    }

    /// 处理在底部面板「搜索」标签中部位 `part` 上按下鼠标左键：查询框放进焦点，开关切换，单击
    /// 一行选中，双击打开。
    pub(in crate::app) fn press_docked_search(&mut self, part: DialogPart, x: f32, y: f32) {
        match part {
            DialogPart::Query => {
                // 对话框与查询框共用同一段文字，先关掉对话框，点击按这里的框定位。
                if self.search_dialog.open {
                    self.close_search_dialog();
                }
                self.focus = Focus::SearchDialog;
                self.click_field(x, y);
                return self.request_redraw();
            }
            DialogPart::Option(i) => {
                let buttons = [PanelButton::Case, PanelButton::Word, PanelButton::Regex];
                self.toggle_search_dialog_option(buttons[i]);
                return;
            }
            DialogPart::Row(i) => {
                self.focus = Focus::SearchDock;
                let double = self.click_count(x, y) == 2;
                self.search_dialog_select(i);
                if double {
                    self.search_dialog_accept();
                }
            }
            _ => self.focus = Focus::SearchDock,
        }
        self.request_redraw();
    }

    /// 焦点在底部面板的「搜索」标签里：结果列表，或对话框关着时的查询。
    pub(in crate::app) fn focus_in_docked_search(&self) -> bool {
        match self.focus {
            Focus::SearchDock => true,
            Focus::SearchDialog | Focus::SearchMask => !self.search_dialog.open,
            _ => false,
        }
    }

    /// 底部面板中的查询框有焦点（对话框关着，焦点在查询中）。
    pub(crate) fn docked_query_focused(&self) -> bool {
        self.focus == Focus::SearchDialog && !self.search_dialog.open && self.docked_search_shown()
    }

    /// 选中第 `index` 条并滚动到可见；超出范围时停在两端。
    pub(in crate::app) fn search_dialog_select(&mut self, index: usize) {
        let count = self.search_dialog.hits.len();
        if count == 0 {
            return;
        }
        let index = index.min(count - 1);
        let list = self.shown_list();
        let d = &mut self.search_dialog;
        d.selected = index;
        let top = index as f32 * PICKER_ROW;
        if top < d.scroll {
            d.scroll = top;
        } else if top + PICKER_ROW > d.scroll + list.list.h {
            d.scroll = top + PICKER_ROW - list.list.h;
        }
        self.request_redraw();
    }

    /// 结果列表一页的行数。
    pub(in crate::app) fn search_dialog_page(&self) -> usize {
        let list = self.shown_list().list;
        ((list.h / PICKER_ROW).floor() as usize)
            .saturating_sub(1)
            .max(1)
    }

    /// 滚动结果列表 `dy`（逻辑像素，向下为正）。
    pub(in crate::app) fn scroll_search_dialog(&mut self, dy: f32) {
        let list = self.shown_list().list;
        let d = &mut self.search_dialog;
        let max = (d.hits.len() as f32 * PICKER_ROW - list.h).max(0.0);
        d.scroll = (d.scroll + dy).clamp(0.0, max);
        self.request_redraw();
    }

    /// 切换搜索选项（区分大小写、全字匹配、正则）。
    pub(in crate::app) fn toggle_search_dialog_option(&mut self, button: PanelButton) {
        let d = &mut self.search_dialog;
        match button {
            PanelButton::Case => d.case_sensitive ^= true,
            PanelButton::Word => d.whole_word ^= true,
            PanelButton::Regex => d.regex ^= true,
            _ => return,
        }
        self.search_dialog_changed(true);
    }

    /// 返回对话框的位置：拖动或改过大小后用记下的位置，否则在编辑区中居中；都限制在窗口内。
    pub(crate) fn search_dialog_rect(&self) -> Rect {
        let window = self.layout.window;
        let editor = self.layout.editor;
        let rect = self.search_dialog.frame.unwrap_or_else(|| {
            let (w, h) = DEFAULT_SIZE;
            let (w, h) = (w.min(window.w - 48.0), h.min(window.h - 96.0));
            let x = editor.x + (editor.w - w) / 2.0;
            let y = (editor.y + 24.0).min(window.bottom() - h - 24.0);
            Rect::new(x, y, w, h)
        });
        clamp_dialog(rect, window)
    }

    /// 返回对话框各部件的位置。
    ///
    /// 左右两边对齐在 [`SIDE`]：输入框的边框、列表和预览中选中一行的底色、标题的左边和关闭按钮
    /// 里叉的右边；输入的文字、列表和预览的图标在它往里的同一列（[`TEXT_X`]）。顶边到标题行、
    /// 标题行到输入框、输入框到列表都隔 [`INSET`]。
    pub(crate) fn search_dialog_parts(&self) -> DialogParts {
        use crate::app::PICKER_PAD;
        use crate::theme::{BUTTON, BUTTON_GAP, DIALOG_GAP};
        let card = self.search_dialog_rect();
        let row = |y: f32, h: f32| Rect::new(card.x, y, card.w, h);
        // 列表、预览按快速打开面板的行排：行的底色从行内缩进 6，这里让行本身缩进 `SIDE - 6`。
        let lane = |y: f32, h: f32| {
            let inset = SIDE - ROW_INSET;
            Rect::new(card.x + inset, y, (card.w - 2.0 * inset).max(0.0), h)
        };
        let (x, w) = (card.x + SIDE, (card.w - 2.0 * SIDE).max(0.0));

        // 标题行（按住可以拖动）从卡片顶边算起，标题和按钮在离顶边 `INSET` 的一条里；右端从右
        // 往左：关闭、「在底部显示」、范围。
        let header = row(card.y, INSET + BUTTON);
        let bar = row(card.y + INSET, BUTTON);
        let button = |right: f32| Rect::new(right - BUTTON, bar.y, BUTTON, BUTTON);
        // 叉的右边对齐在 `SIDE`，按钮的悬停底色伸进边距。
        let close = button(card.right() - SIDE + (BUTTON - CLOSE_MARK) / 2.0);
        let dock = button(close.x - BUTTON_GAP);
        let track_w = SEGMENT_W * 2.0 + 4.0;
        let track = Rect::new(
            dock.x - 8.0 - track_w,
            bar.y + (bar.h - FIELD_H) / 2.0,
            track_w,
            FIELD_H,
        );
        let segment = |i: f32| {
            let x = track.x + 2.0 + i * SEGMENT_W;
            Rect::new(x, track.y + 2.0, SEGMENT_W, track.h - 4.0)
        };
        let scopes = [segment(0.0), segment(1.0)];

        // 查询框内靠右依次是三个开关和「过滤」，都是正方形。
        let query_row = Rect::new(x, bar.bottom() + DIALOG_GAP, w, FIELD_H);
        let side = FIELD_H - 6.0;
        let square = |right: f32| Rect::new(right - side, query_row.y + 3.0, side, side);
        let mask_toggle = square(query_row.right() - 3.0);
        let regex = square(mask_toggle.x - BUTTON_GAP);
        let word = square(regex.x - BUTTON_GAP);
        let case = square(word.x - BUTTON_GAP);
        let text_x = card.x + TEXT_X;
        let query = Rect::new(
            text_x,
            query_row.y,
            (case.x - 8.0 - text_x).max(0.0),
            FIELD_H,
        );
        let mask_h = if self.search_dialog.mask_on {
            FIELD_H
        } else {
            0.0
        };
        let mask = Rect::new(x, query_row.bottom() + DIALOG_GAP, w, mask_h);
        let fields = if mask_h > 0.0 { mask } else { query_row };

        // 输入框下面隔一条分隔线是各处匹配，再下面是预览，一直到卡片底边：预览同编辑区一样，
        // 放不下的最后一行露出一部分，底下不留成段的空白。
        let top = fields.bottom() + INSET;
        let bottom = card.bottom() - RADIUS_CARD / 2.0;
        // 列表和预览平分剩下的高度；列表放整数行，不露出半行，零头归预览。
        let rows = (((bottom - top) / 2.0 - 2.0 * PICKER_PAD) / PICKER_ROW)
            .floor()
            .max(1.0);
        let list_h = rows * PICKER_ROW + 2.0 * PICKER_PAD;
        let list = PickerParts {
            bar: lane(top, 0.0),
            actions: lane(top, 0.0),
            list: lane(top + PICKER_PAD, (list_h - 2.0 * PICKER_PAD).max(0.0)),
            footer: lane(top + list_h, 0.0),
        };
        let preview_head = lane(top + list_h, PREVIEW_HEAD);
        let preview = lane(
            preview_head.bottom(),
            (bottom - preview_head.bottom()).max(0.0),
        );
        DialogParts {
            card,
            header,
            bar,
            close,
            dock,
            track,
            scopes,
            query_row,
            query,
            options: [case, word, regex],
            mask_toggle,
            mask,
            list,
            preview_head,
            preview,
        }
    }

    /// 对话框和底部面板中较大的预览区放得下的行数。
    pub(crate) fn search_dialog_preview_lines(&self) -> usize {
        let dialog = self
            .search_dialog
            .open
            .then(|| self.search_dialog_parts().preview.h);
        let dock = self.docked_parts().map(|dock| dock.preview.h);
        let h = dialog.into_iter().chain(dock).fold(0.0, f32::max) - 2.0 * PREVIEW_PAD;
        // 最后一行可能只露出一部分，也要取来（见 `draw_context`）。
        (h / self.fonts.line_height).ceil().max(1.0) as usize
    }

    /// 返回 (`x`, `y`) 处的部位；不在对话框上时返回 `None`。
    pub(crate) fn search_dialog_part_at(&self, x: f32, y: f32) -> Option<DialogPart> {
        if !self.search_dialog.open {
            return None;
        }
        let parts = self.search_dialog_parts();
        let card = parts.card;
        let grow = Rect::new(
            card.x - EDGE,
            card.y - EDGE,
            card.w + 2.0 * EDGE,
            card.h + 2.0 * EDGE,
        );
        if !grow.contains(x, y) {
            return None;
        }
        let side = |at: f32, low: f32, high: f32| {
            if at < low + EDGE {
                -1
            } else if at > high - EDGE {
                1
            } else {
                0
            }
        };
        let edge = (
            side(x, card.x, card.right()),
            side(y, card.y, card.bottom()),
        );
        if edge != (0, 0) {
            return Some(DialogPart::Edge(edge.0, edge.1));
        }
        if parts.close.contains(x, y) {
            return Some(DialogPart::Close);
        }
        if let Some(i) = parts.options.iter().position(|r| r.contains(x, y)) {
            return Some(DialogPart::Option(i));
        }
        if parts.mask_toggle.contains(x, y) {
            return Some(DialogPart::MaskToggle);
        }
        if parts.dock.contains(x, y) {
            return Some(DialogPart::Dock);
        }
        if parts.query_row.contains(x, y) {
            return Some(DialogPart::Query);
        }
        if parts.mask.contains(x, y) {
            return Some(DialogPart::Mask);
        }
        if let Some(i) = parts.scopes.iter().position(|r| r.contains(x, y)) {
            return Some(DialogPart::Scope(i));
        }
        if parts.header.contains(x, y) {
            return Some(DialogPart::Title);
        }
        let row = parts.list.row_at(y, self.search_dialog.scroll);
        if let Some(i) = row.filter(|&i| i < self.search_dialog.hits.len()) {
            return Some(DialogPart::Row(i));
        }
        Some(DialogPart::Body)
    }

    /// 显示或隐藏文件掩码：显示时焦点移进掩码框，隐藏后不再按掩码过滤。
    pub(in crate::app) fn toggle_search_mask(&mut self) {
        let d = &mut self.search_dialog;
        d.mask_on ^= true;
        let filtered = !d.mask.text().is_empty();
        self.focus = if d.mask_on {
            Focus::SearchMask
        } else {
            Focus::SearchDialog
        };
        self.reset_blink();
        if filtered {
            self.search_dialog_changed(true);
        }
        self.request_redraw();
    }

    /// 处理在部位 `part` 上按下鼠标左键。
    pub(in crate::app) fn press_search_dialog(&mut self, part: DialogPart, x: f32, y: f32) {
        let rect = self.search_dialog_rect();
        match part {
            DialogPart::Dock => return self.dock_search_results(),
            DialogPart::Close => return self.close_search_dialog(),
            DialogPart::Title => {
                self.pointer.drag = Drag::SearchDialog {
                    grab: (x - rect.x, y - rect.y),
                };
            }
            DialogPart::Edge(h, v) => {
                self.pointer.drag = Drag::SearchDialogEdge {
                    edge: (h, v),
                    from: (x, y),
                    rect,
                };
            }
            DialogPart::Option(i) => {
                let buttons = [PanelButton::Case, PanelButton::Word, PanelButton::Regex];
                self.toggle_search_dialog_option(buttons[i]);
            }
            DialogPart::Row(i) => {
                let double = self.click_count(x, y) == 2;
                self.search_dialog_select(i);
                if double {
                    self.focus = Focus::SearchDialog;
                    return self.search_dialog_accept();
                }
            }
            DialogPart::Query => {
                self.focus = Focus::SearchDialog;
                self.click_field(x, y);
            }
            DialogPart::Mask => {
                self.focus = Focus::SearchMask;
                self.click_field(x, y);
                return self.request_redraw();
            }
            DialogPart::MaskToggle => return self.toggle_search_mask(),
            DialogPart::Scope(i) => {
                let d = &mut self.search_dialog;
                let in_dir = i == 1 && d.dir.is_some();
                if d.in_dir != in_dir {
                    d.in_dir = in_dir;
                    self.search_dialog_changed(true);
                }
            }
            DialogPart::Body => {}
        }
        // 焦点已在查询或文件掩码中时留在原处。
        if !matches!(self.focus, Focus::SearchDialog | Focus::SearchMask) {
            self.focus = Focus::SearchDialog;
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 拖动标题行：左上角移到 (`x`, `y`) 减去按下处 `grab`。
    pub(in crate::app) fn move_search_dialog(&mut self, x: f32, y: f32, grab: (f32, f32)) {
        let rect = self.search_dialog_rect();
        let moved = Rect::new(x - grab.0, y - grab.1, rect.w, rect.h);
        self.search_dialog.frame = Some(clamp_dialog(moved, self.layout.window));
        self.request_redraw();
    }

    /// 拖动边缘改变大小：`edge` 是拖动的边，`start` 是按下时的位置和对话框。
    pub(in crate::app) fn resize_search_dialog(
        &mut self,
        (x, y): (f32, f32),
        edge: (i8, i8),
        (from, rect): ((f32, f32), Rect),
    ) {
        let (dx, dy) = (x - from.0, y - from.1);
        let (mut left, mut top, mut right, mut bottom) =
            (rect.x, rect.y, rect.right(), rect.bottom());
        match edge.0 {
            -1 => left = (left + dx).min(right - MIN_SIZE.0),
            1 => right = (right + dx).max(left + MIN_SIZE.0),
            _ => {}
        }
        match edge.1 {
            -1 => top = (top + dy).min(bottom - MIN_SIZE.1),
            1 => bottom = (bottom + dy).max(top + MIN_SIZE.1),
            _ => {}
        }
        let resized = Rect::new(left, top, right - left, bottom - top);
        self.search_dialog.frame = Some(clamp_dialog(resized, self.layout.window));
        self.request_redraw();
    }

    /// 返回选中的一处周围的几行（选中的行号、首行的行号、各行文字、首行行首的高亮状态），按需
    /// 从文档或磁盘读取并缓存。
    pub(crate) fn search_dialog_context(
        &mut self,
    ) -> Option<(usize, usize, Vec<String>, LineState)> {
        let d = &self.search_dialog;
        let place = d.places.as_ref()?.items.get(d.selected)?;
        let key = (place.path.clone(), place.line);
        let count = self.search_dialog_preview_lines();
        // 按要取的行数（而不是取到的行数）判断缓存是否够用。
        let cached = (d.context.as_ref()).filter(|c| c.at == key && c.taken >= count);
        if let Some(c) = cached {
            // 上次没有读到内容的也记着（各行为空）：不在每一帧重新读取。
            return (!c.lines.is_empty()).then(|| (c.at.1, c.first, c.lines.clone(), c.state));
        }
        let first = key.1.saturating_sub(count / 2);
        let open = (self.editor.documents.iter_mut()).find(|doc| doc.path.as_ref() == Some(&key.0));
        let (lines, state): (Vec<String>, LineState) = match open {
            Some(doc) => {
                let state = (doc.highlight_state(first, CONTEXT_STATE_BUDGET))
                    .unwrap_or_else(|| doc.approx_highlight_state(first));
                let text = doc.text();
                let lines = (first..(first + count).min(text.line_count()))
                    .map(|l| text.line_content(l).slice().to_string())
                    .collect();
                (lines, state)
            }
            // 过大的文件不为几行预览读进内存；读不到的与之相同，记为没有内容。
            None => {
                let small = |m: winc_backend::fs::Metadata| m.len <= winc_editor::LARGE_FILE;
                let bytes = (winc_backend::fs::metadata(&key.0).is_ok_and(small))
                    .then(|| winc_backend::fs::read(&key.0).ok())
                    .flatten()
                    .unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes);
                let lines = text.lines().skip(first).take(count);
                let lines = lines.map(str::to_owned).collect();
                let language = winc_syntax::detect(&winc_editor::display_name(&key.0));
                // 推算只用到首行之前的内容。
                let end = match first {
                    0 => 0,
                    n => (text.match_indices('\n').nth(n - 1)).map_or(text.len(), |(i, _)| i + 1),
                };
                let rope = ropey::Rope::from_str(&text[..end]);
                let state = winc_editor::line_highlight_states(
                    &rope,
                    language,
                    &[first],
                    CONTEXT_STATE_BUDGET,
                );
                (lines, state[0])
            }
        };
        let line = key.1;
        self.search_dialog.context = Some(Context {
            at: key,
            taken: count,
            first,
            lines: lines.clone(),
            state,
        });
        (!lines.is_empty()).then_some((line, first, lines, state))
    }
}

/// 把对话框限制在窗口 `window` 之内，并保证不小于最小大小（窗口放得下时）。
fn clamp_dialog(rect: Rect, window: Rect) -> Rect {
    let w = rect.w.max(MIN_SIZE.0).min(window.w - 16.0).max(0.0);
    let h = rect.h.max(MIN_SIZE.1).min(window.h - 16.0).max(0.0);
    let x = rect.x.clamp(
        window.x + 8.0,
        (window.right() - 8.0 - w).max(window.x + 8.0),
    );
    let y = rect.y.clamp(
        window.y + 8.0,
        (window.bottom() - 8.0 - h).max(window.y + 8.0),
    );
    Rect::new(x.round(), y.round(), w.round(), h.round())
}
