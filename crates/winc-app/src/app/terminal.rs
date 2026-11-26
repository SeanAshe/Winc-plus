//! 终端会话：停靠在编辑区下方或右侧面板，或显示为编辑器中的标签。
//!
//! 第一次打开终端时启动辅助进程 `winc-pty`，每个终端是其中的一个 shell。帧存成可见的一屏，
//! 往上翻时按需请求回滚缓冲中的行（[`tab`]）。只为可见的标签发帧额度；隐藏标签切回时请求最新帧，
//! 模块关闭时不启动进程。编辑器标签归属、移动与拖放在 [`area`]，鼠标在 [`mod@pointer`]，查找在
//! [`find`]，命令块在 [`blocks`]，链接在 [`link`]，编辑器交互在 [`editor`]，进程在 [`helper`]。

mod area;
mod blocks;
mod editor;
mod find;
pub(crate) mod groups;
mod helper;
mod keys;
pub(crate) mod link;
pub(crate) mod local_paste;
mod locate;
mod pointer;
pub(crate) mod run;
mod session;
pub(crate) mod shells;
mod shortcuts;
mod split;
pub(crate) mod strip;
mod tab;
pub(crate) mod upload;

use helper::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use winc_config::{OptionAsMeta, TerminalGroup, TerminalSession};
use winc_i18n::t;
use winc_pty::client::{Client, Poll};
use winc_pty::input::{self, Key as TermKey, Mods};
use winc_pty::protocol::{Event, FromHelper, MAGIC, Modes, Size, Theme, ToHelper, VERSION};
use winc_render::{Color, Rect};

use super::animation::{Clock, SIDEBAR_ANIMATION};
use super::{App, CloseRequest, Focus, UserEvent};
use crate::theme::{Palette, TOP_BAR, TOP_BAR_END};

pub(crate) use blocks::{ErrorNav, format_duration};
#[cfg(any(target_os = "macos", all(test, unix)))]
pub(crate) use editor::shell_quote;
pub(crate) use find::{FIND_OPTIONS_MIN, FindPart, TermFind};
pub(crate) use link::TermLink;
pub(crate) use pointer::BlockHit;
#[cfg(all(test, unix))]
pub(crate) use tab::{Point, Selection, Unit};
pub(crate) use tab::{SETTLE, TabEnd, TermSplit, TermTab, ViewRow};

/// 面板在下面时的默认高度（逻辑像素，含标签栏）。
pub(crate) const DEFAULT_HEIGHT: f32 = 242.0;
/// 面板在下面时的最矮高度（逻辑像素，含标签栏）：标签栏加几行。
pub(crate) const MIN_HEIGHT: f32 = 96.0;
/// 在下面拖高面板时给编辑区至少留的高度（逻辑像素）。
pub(crate) const EDITOR_MIN: f32 = 120.0;
/// 面板在编辑区右边时的默认宽度（逻辑像素，取自调参）。
pub(crate) const DEFAULT_WIDTH: f32 = 480.0;
/// 面板在编辑区右边时的最窄宽度（逻辑像素）：再窄时一行放不下几个字。
pub(crate) const MIN_WIDTH: f32 = 280.0;
/// 面板在右边时至少给编辑区留的宽度（逻辑像素，取自调参）。
pub(crate) const EDITOR_MIN_WIDTH: f32 = 360.0;
/// 面板在下面时标签栏的高度（逻辑像素）：`TAB_HEIGHT` 上下各留 5。
const HEADER: f32 = crate::theme::TAB_HEIGHT + 10.0;
/// 网格右边和下方的留白（逻辑像素）。
pub(crate) const PAD: f32 = 8.0;
/// 网格左边的留白（逻辑像素），命令块的色带画在这里。
pub(crate) const GUTTER: f32 = 14.0;
/// 色带离面板左边的距离（逻辑像素）。
pub(crate) const BAND_X: f32 = 5.0;
/// 发出新的尺寸后等待该尺寸一帧的最长时间（100 ms，取自调参）。
///
/// 超过这个时间不再等待，新的尺寸仍然会发：程序开着同步输出时，辅助进程会先不发帧。
const RESIZE_WAIT: Duration = Duration::from_millis(100);
/// 换位置的动画总时长（360 ms）：前一半在原来的位置收起，后一半在新位置出来。
const MOVE_ANIMATION: Duration = Duration::from_millis(360);
/// 一次向辅助进程要多少行历史（行）：整份回滚缓冲可以设到一百万行，一次要会超出单条消息的
/// 长度上限，因此分段要。
const REQUEST_LINES: u64 = 10_000;
/// 一条输入消息最多多少字节（1 MiB）：粘贴很大的内容时分开发。
const INPUT_CHUNK: usize = 1 << 20;
/// 回滚缓冲默认的行数（行，`terminal.scrollback` 没有设置时使用）。
const SCROLLBACK: u32 = 10_000;
/// 对比度保护默认的最低对比度（`terminal.minimumContrast`，0 表示关）：3 是 WCAG 2.1 对
/// 大字号的 AA 要求。
pub(crate) const MINIMUM_CONTRAST: f32 = 3.0;

/// 向辅助进程要历史行的用途，结果回来时按它处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    /// 往上翻时显示。
    View,
    /// 复制选区。
    Copy,
    /// 查找：整个回滚缓冲。
    Find,
    /// F8：失败命令的输出。
    Errors,
}

/// 标签栏上可以点的东西，画的时候记下位置（[`TerminalPanel::hits`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderHit {
    /// 第几个标签。
    Tab(usize),
    /// 第几个标签的关闭按钮。
    Close(usize),
    /// 竖向列表里拆分的标签下的一个终端：（第几个标签，在排列中的位置）。
    Pane(usize, usize),
    /// 竖向列表里拆分的标签下一个终端的关闭按钮，参数同 `Pane`。
    ClosePane(usize, usize),
    /// 新建标签的「+」。
    New,
    /// 「重新启动」：shell 退出了、或者辅助进程退出了时。
    Restart,
    /// 收起面板（和状态栏上的终端按钮一样）。
    Hide,
    /// 「+」右边选 shell 的按钮。
    ShellMenu,
    /// 「搜索」标签（在文件中查找的结果）。
    SearchTab,
    /// 「搜索」标签的关闭按钮。
    SearchClose,
    /// 显示「搜索」标签时的「在对话框中打开」。
    SearchDialog,
    /// 把面板挪到编辑区右边，或者挪回下面。
    Position,
    /// 最大化面板，或者还原（⌘⇧↩）。
    Maximize,
    /// 把当前终端移到编辑器标签。
    Editor,
    /// 把编辑器终端移回面板。
    Panel,
    /// 拆分当前的终端（见 `App::split_terminal`）。
    Split,
    /// 运行配置的按钮。
    Run(run::RunButton),
    /// 查找条的一部分。
    Find(FindPart),
}

/// 面板出现、收起、换位置的动画（见 [`App::terminal_shown`]）。
///
/// 动画里面板一直是原来的大小，只是滑出、滑进窗口边上（下面的滑到状态栏后面，右边的滑出
/// 窗口），不改 shell 的尺寸。
#[derive(Clone, Copy)]
pub(crate) struct PanelMotion {
    /// 开始时露出的比例。
    from: f32,
    /// 换位置：前一半在原来的位置收起，后一半在新位置出来。
    moved: bool,
    /// 动画的时钟。
    pub clock: Clock,
}

/// 终端面板的状态。
#[derive(Default)]
pub(crate) struct TerminalPanel {
    /// 面板开着。
    pub open: bool,
    /// 面板在编辑区下面时的高度（逻辑像素，含标签栏）；0 表示还没打开过，用默认值。
    pub height: f32,
    /// 面板在编辑区右边时的宽度（逻辑像素）；0 表示还没拖过，用默认值。
    pub width: f32,
    /// 出现、收起、换位置的动画。
    pub motion: Option<PanelMotion>,
    /// 最大化（⌘⇧↩）：面板铺满编辑区一栏，编辑区不占地方。随现场保存。
    pub maximized: bool,
    /// 最大化、还原的过渡：面板在平常的大小和整栏之间伸缩。
    pub grow: Option<Clock>,
    /// 测试里没有窗口、没有时钟：设置后面板的动画都停在这一时刻。
    #[cfg(test)]
    pub frozen: Option<Duration>,
    client: Option<Client>,
    /// 辅助进程握过手了，可以新建终端。
    ready: bool,
    /// 各终端标签；分栏时是正在用的一栏的（见 [`groups`]）。
    pub tabs: Vec<TermTab>,
    /// 正在看的标签。
    pub active: usize,
    /// 分栏：各栏按排列顺序，正在用的一栏只占位置、记份额，它的标签在 `tabs` 里；没有分栏时
    /// 为空。
    pub groups: Vec<groups::TermGroup>,
    /// 正在用的一栏在 `groups` 中的位置。
    pub group_at: usize,
    /// 上一次正在用的面板栏；焦点切到编辑器终端后仍保留。
    pub panel_at: usize,
    /// 拖动编辑器终端时临时打开了原本收起的面板。
    pub drag_opened: bool,
    /// 临时换进了别的栏（[`App::with_terminal_group`]）：此时不整理空栏、不建标签。
    borrowed: bool,
    /// 临时换进别的栏时要过排版（[`App::sync_terminal`]），换回来以后补上。
    resync: bool,
    /// `want` 里各标签要放进第几栏（恢复现场、辅助进程退出后重开时），和 `want` 一一对应；
    /// 数量不足的部分放进正在用的一栏。
    want_groups: Vec<usize>,
    /// 等辅助进程准备好、知道面板尺寸以后要新建的标签：各自的工作目录（空的是默认目录）。
    pub want: Vec<Vec<u8>>,
    /// 下一个排进 `want` 的是拆分出的终端：加进前台终端编号为它的标签（见 `want_into`）。
    pub split_into: Option<u16>,
    /// `want` 里各个拆分出的终端要加进的标签（前台终端的编号），和 `want` 一一对应；`None` 或
    /// 数量不足的部分各成一个标签。
    pub want_into: Vec<Option<u16>>,
    /// `want` 里各标签的名字（恢复现场、重新启动时带上用户起的名字），和 `want` 一一对应；
    /// 数量不足的部分视为没有名字。
    want_names: Vec<String>,
    /// `want` 里各标签的 shell，和 `want` 一一对应；`None` 或数量不足的部分用默认 shell。
    pub want_shells: Vec<Option<String>>,
    /// 本机可选的 shell（[`shells`]），检测完成前为空。
    pub shells: Vec<shells::ShellChoice>,
    /// 是否已经开始检测本机的 shell。
    shells_detected: bool,
    /// `want` 里的标签都建好以后看第几个（恢复上次的现场时）；`None` 是最后建的那个。
    want_active: Option<usize>,
    /// 等着启动的运行配置。
    run_queue: Vec<run::RunStart>,
    /// 新建的标签起来以后先写给它的输入（⌃⇧↩ 发代码时还没有终端）。
    pub pending_input: Option<Vec<u8>>,
    /// 辅助进程意外退出了：标签和最后一屏还留着，等用户重新启动。
    pub crashed: bool,
    /// 正在询问是否加载的 Python 环境目录；回答之前等着建的标签不建。
    pub python_asking: Option<PathBuf>,
    /// 本次运行中回答了不加载的 Python 环境目录；不写入状态文件，下次启动时重新询问。
    pub python_declined: Vec<PathBuf>,
    /// 按住 ⌘ 时鼠标下的链接。
    pub link: Option<TermLink>,
    /// 有标签在等新尺寸的一帧、面板又变了时：什么时候不再等（见 [`App::sync_terminal`]）。
    pub resize_due: Option<Instant>,
    /// 启动不了时给用户看的原因，画在面板里。
    pub error: Option<String>,
    next_id: u16,
    next_request: u32,
    /// 还没回来的取行请求：（请求编号，标签编号，用途）。
    ///
    /// 同时只有几个，用列表。
    requests: Vec<(u32, u16, Request)>,
    /// 上一次告诉终端里的程序的焦点状态（焦点上报，模式 1004）。
    focus_sent: bool,
    /// 查找条。
    pub find: Option<TermFind>,
    /// 正在改名字的标签（编号）和标签上的输入框。
    pub rename: Option<(u16, crate::field::TextField)>,
    /// 上一次上报鼠标时在哪一格（同一格里移动不重复报）。
    last_mouse_cell: Option<(u16, u16)>,
    /// F8 在报错之间跳。
    pub errors: Option<ErrorNav>,
    /// 窗口不在前台时结束的长命令，回到窗口时提示。
    pub pending_notice: Option<String>,
    /// 标签栏的动画和拖动。
    pub strip: strip::Strip,
}

impl TerminalPanel {
    /// 正在看的标签。
    pub fn tab(&self) -> Option<&TermTab> {
        self.tabs.get(self.active)
    }

    /// 正在看的标签，可变引用。
    pub fn tab_mut(&mut self) -> Option<&mut TermTab> {
        self.tabs.get_mut(self.active)
    }

    /// 返回面板之外也可见的状态和标签栏内容的摘要，用于判断一批消息是否改变了界面。
    ///
    /// 标签名和结束状态显示在面板或编辑器标签栏里；未读标记同时显示在状态栏。
    pub(in crate::app) fn visible_state(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.open, self.active, self.crashed, &self.error).hash(&mut hasher);
        (self.group_at, self.groups.len()).hash(&mut hasher);
        for at in 0..self.group_count() {
            (self.editor_at(at), self.group_shown(at)).hash(&mut hasher);
            for tab in self.group_tabs(at) {
                tab.unseen.hash(&mut hasher);
                if self.open || self.editor_at(at).is_some() {
                    (tab.id, tab.label(), tab.ended).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    /// 消息是否作用于正在显示的内容。标题、进程、未读标记等的变化由 [`Self::visible_state`] 判断。
    pub(in crate::app) fn message_is_shown(&self, message: &FromHelper) -> bool {
        // 各栏正在看的标签都在眼前，拆分出的终端也是。
        let shows = |id: u16| {
            self.shown_tabs()
                .any(|tab| tab.panes().any(|pane| pane.id == id))
        };
        match message {
            FromHelper::Hello { .. } | FromHelper::History { .. } | FromHelper::Failed { .. } => {
                true
            }
            FromHelper::Frame { tab, .. } | FromHelper::Blocks { tab, .. } => shows(*tab),
            FromHelper::Event {
                tab,
                event: Event::Link(..),
            } => shows(*tab),
            FromHelper::Event { .. }
            | FromHelper::Process { .. }
            | FromHelper::Exited { .. }
            | FromHelper::Disconnected { .. } => false,
        }
    }

    /// 编号为 `id` 的终端所在的标签（含拆分出的终端）。
    fn index_of(&self, id: u16) -> Option<usize> {
        (self.tabs.iter()).position(|t| t.panes().any(|pane| pane.id == id))
    }

    /// 编号为 `id` 的终端，可以是拆分出的。
    fn by_id(&mut self, id: u16) -> Option<&mut TermTab> {
        let index = self.index_of(id)?;
        let tab = &mut self.tabs[index];
        if tab.id == id {
            return Some(tab);
        }
        let split = tab.split.as_mut()?;
        split.others.iter_mut().find(|t| t.id == id)
    }

    /// 取出第 `index` 个标签的各个终端，按排列顺序，以及前台终端的位置。
    pub(crate) fn take_group(&mut self, index: usize) -> (Vec<TermTab>, usize) {
        take_group(&mut self.tabs, index)
    }

    /// 把各个终端 `all` 放回第 `index` 个标签，第 `at` 个在前台。
    pub(crate) fn put_group(&mut self, index: usize, all: Vec<TermTab>, at: usize) {
        put_group(&mut self.tabs, index, all, at);
    }

    /// 去掉编号为 `id` 的终端；它所在的标签还有其他终端时由相邻的补上。返回它所在的标签序号，
    /// 以及标签是否整个没了。
    pub(crate) fn remove_terminal(&mut self, id: u16) -> Option<(usize, bool)> {
        let index = self.index_of(id)?;
        let (mut all, at) = self.take_group(index);
        let pos = all.iter().position(|t| t.id == id)?;
        all.remove(pos);
        if all.is_empty() {
            return Some((index, true));
        }
        let at = if pos < at {
            at - 1
        } else {
            at.min(all.len() - 1)
        };
        self.put_group(index, all, at);
        Some((index, false))
    }

    /// 向辅助进程要 `tab` 的历史行 `range`，记下要来干什么。
    ///
    /// 分成每次最多 [`REQUEST_LINES`] 行：
    /// 一条消息有长度上限，整个回滚缓冲（可以设到一百万行）一次要会超出。
    fn request(&mut self, tab: u16, (start, end): (u64, u64), purpose: Request) {
        let Some(client) = &self.client else { return };
        let mut from = start;
        while from < end {
            let to = end.min(from + REQUEST_LINES);
            self.next_request = self.next_request.wrapping_add(1);
            let id = self.next_request;
            self.requests.push((id, tab, purpose));
            client.send(&ToHelper::History {
                tab,
                id,
                start: from,
                end: to,
            });
            from = to;
        }
    }

    /// 还有没回来的、为 `purpose` 向 `tab` 要的行。
    fn waiting(&self, tab: u16, purpose: Request) -> bool {
        self.requests
            .iter()
            .any(|&(_, t, p)| (t, p) == (tab, purpose))
    }
}

/// 把标签 `tab` 里各终端攒着的额度发出去：拆分出的终端和前台的一样在眼前，都要拿到新的帧，
/// 否则它们停在拆分前的画面上，变窄后重排的屏幕也收不到。
pub(super) fn send_credit(client: &Client, tab: &mut TermTab) {
    let credit = |pane: &mut TermTab| {
        if std::mem::take(&mut pane.credit_due) {
            client.send(&ToHelper::Credit { tab: pane.id });
        }
    };
    credit(tab);
    if let Some(split) = &mut tab.split {
        split.others.iter_mut().for_each(credit);
    }
}

/// 取出 `tabs` 中第 `index` 个标签的各个终端，按排列顺序，以及前台终端的位置。
pub(crate) fn take_group(tabs: &mut Vec<TermTab>, index: usize) -> (Vec<TermTab>, usize) {
    let mut focused = tabs.remove(index);
    match focused.split.take() {
        None => (vec![focused], 0),
        Some(split) => {
            let TermSplit { mut others, at } = *split;
            others.insert(at, focused);
            (others, at)
        }
    }
}

/// 把各个终端 `all` 放回 `tabs` 的第 `index` 个标签，第 `at` 个在前台。
pub(crate) fn put_group(tabs: &mut Vec<TermTab>, index: usize, mut all: Vec<TermTab>, at: usize) {
    let at = at.min(all.len().saturating_sub(1));
    let mut focused = all.remove(at);
    if !all.is_empty() {
        focused.split = Some(Box::new(TermSplit { others: all, at }));
    }
    tabs.insert(index, focused);
}

/// 把拆分出的终端 `tab` 排进 `tabs` 的第 `index` 个标签：排在前台终端的后面并成为前台。
///
/// 拆分后各终端平分，之前拖动过的大小不再保留。
pub(crate) fn insert_pane(tabs: &mut Vec<TermTab>, index: usize, tab: TermTab) {
    let (mut all, at) = take_group(tabs, index);
    all.insert(at + 1, tab);
    all.iter_mut().for_each(|pane| pane.share = 1.0);
    put_group(tabs, index, all, at + 1);
}

/// 面板标签栏的高度（逻辑像素）：`card` 从顶栏那一行开始时（面板在右边）用 `TOP_BAR`，
/// 使终端标签和文件标签排在同一行；面板在下面时用 `HEADER`。
pub(crate) fn header_height(card: Rect) -> f32 {
    if card.y < TOP_BAR_END {
        TOP_BAR
    } else {
        HEADER
    }
}

/// 第一个标签离面板左边的距离（逻辑像素），与标签上下的留白相同。
pub(crate) fn tab_inset(card: Rect) -> f32 {
    (header_height(card) - crate::theme::TAB_HEIGHT) / 2.0
}

/// 面板放不下一整行的零头（行，`0.0..1.0`）。
pub(crate) fn grid_slack(card: Rect, line_height: f32) -> f32 {
    (grid_rect(card, line_height).y - card.y - header_height(card)) / line_height
}

/// 面板里放网格的区域：标签栏以下，左边留出画命令块色带的 [`GUTTER`]，右边和底下留 [`PAD`]，
/// 高度正好是整数行。
pub(crate) fn grid_rect(card: Rect, line_height: f32) -> Rect {
    // 放不下一整行的零头留在上面，网格贴着面板下边：拖动分隔条时动的是面板上边，内容不跟着移动。
    // 零头处显示网格上面一行的下半截，见 `TermTab::viewport`。
    let header = header_height(card);
    let h = (card.h - header - PAD).max(0.0);
    let rows = (h / line_height).floor();
    let rest = if rows >= 1.0 {
        h - rows * line_height
    } else {
        0.0
    };
    Rect::new(
        card.x + GUTTER,
        card.y + header + rest,
        (card.w - GUTTER - PAD).max(0.0),
        // 留一点余量：按高度算行数时（`h / line_height` 向下取整）不因为舍入少一行。
        h - rest + 0.01,
    )
}

fn rgb(color: Color) -> [u8; 3] {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(color.r), byte(color.g), byte(color.b)]
}

/// 回答程序颜色查询用的主题颜色（程序靠它判断深色或浅色界面）。
fn theme_colors(p: &Palette) -> Theme {
    let mut palette = [[0; 3]; 16];
    for (slot, &color) in palette.iter_mut().zip(&p.ansi) {
        *slot = rgb(color);
    }
    Theme {
        fg: rgb(p.syntax_text),
        bg: rgb(p.editor_bg),
        cursor: rgb(p.caret),
        palette,
    }
}

impl App {
    /// 终端模块开着没有。
    pub(crate) fn terminal_enabled(&self) -> bool {
        self.config.terminal_enabled.unwrap_or(false)
    }

    /// 对比度保护的最低对比度（`terminal.minimumContrast`，0 表示关）。
    pub(crate) fn terminal_contrast(&self) -> f32 {
        self.config
            .terminal_minimum_contrast
            .unwrap_or(MINIMUM_CONTRAST)
    }

    /// ⌃\`：面板关着时打开并把焦点放进去；开着、焦点在里面时收起，焦点回编辑区；开着但焦点在
    /// 面板外时把焦点放进去。
    pub(super) fn toggle_terminal(&mut self) {
        let in_terminal = self.focus.in_terminal();
        if !self.terminal.open {
            self.open_terminal_panel();
        } else if in_terminal {
            self.set_terminal_open(false);
        } else {
            self.search_dialog.in_panel = false;
            self.focus = Focus::Terminal;
        }
        self.reset_blink();
        self.relayout();
        self.request_redraw();
    }

    /// 打开面板、焦点放进去；还没有标签时开一个。
    fn open_terminal_panel(&mut self) {
        self.search_dialog.in_panel = false;
        let group = self.ensure_terminal_panel_group();
        self.blur_terminal_tab();
        self.rotate_terminal_group(group);
        self.set_terminal_open(true);
        let t = &mut self.terminal;
        if t.height <= 0.0 {
            t.height = DEFAULT_HEIGHT;
        }
        if t.panel_tab_count() == 0 && t.want.is_empty() {
            t.want.push(Vec::new());
        }
        self.focus = Focus::Terminal;
        self.start_terminal();
    }

    /// 换目录或关闭目录时关掉所有终端标签，挂断其中的 shell，不确认、不收起面板。
    ///
    /// 调用方须先确认运行中的程序（[`Self::confirm_terminal_quit`]），并把标签记入原目录的
    /// 现场。
    pub(super) fn close_all_terminal_tabs(&mut self) {
        let t = &mut self.terminal;
        let documents: Vec<_> = t.groups.iter().filter_map(|group| group.editor).collect();
        let ids: Vec<u16> = (t.all_tabs().flat_map(TermTab::panes))
            .map(|tab| tab.id)
            .collect();
        if let Some(client) = &t.client {
            for &tab in &ids {
                client.send(&ToHelper::Close { tab });
            }
        }
        t.tabs.clear();
        t.groups.clear();
        t.group_at = 0;
        t.panel_at = 0;
        t.drag_opened = false;
        t.want_groups.clear();
        t.want.clear();
        t.want_names.clear();
        t.want_shells.clear();
        t.want_into.clear();
        t.want_active = None;
        t.active = 0;
        t.client = None;
        t.ready = false;
        t.crashed = false;
        for id in ids {
            self.forget_terminal_tab(id);
        }
        for document in documents {
            self.detach_terminal_editor_document(document);
        }
        if self.focus.in_terminal() {
            self.focus = Focus::Editor;
        }
    }

    /// 换到新目录后的终端：按新目录的现场重开它的标签；新目录没有记下终端、面板开着时在
    /// 新目录里开一个标签；面板开着而新目录上次是保存的，收起面板。
    pub(super) fn switch_terminal(&mut self, session: Option<&TerminalSession>) {
        match session {
            Some(session) => {
                if !session.open && self.terminal.open {
                    self.set_terminal_open(false);
                }
                self.restore_terminal(session);
            }
            None if self.terminal.open && self.terminal.tab_count() == 0 => {
                self.terminal.want.push(Vec::new());
                self.start_terminal();
            }
            None => {}
        }
    }

    /// 是否已连接辅助进程且当前标签已创建，此时可以向终端写入。
    pub(crate) fn terminal_ready(&self) -> bool {
        let t = &self.terminal;
        t.client.is_some() && t.tabs.get(t.active).is_some()
    }

    /// 终端中是否有程序在运行、关闭前需要确认；设置中关闭了确认（`terminal.confirmClose`）时为假。
    pub(crate) fn terminal_busy(&self) -> bool {
        self.config.terminal_confirm_close != Some(false)
            && (self.terminal.all_tabs().flat_map(TermTab::panes)).any(|t| t.busy().is_some())
    }

    /// 退出、关闭窗口、换目录或关闭目录（`request`）前，终端中仍有程序在运行时先确认
    /// （`terminal.confirmClose`）。
    ///
    /// 已弹出确认框或正在确认时返回真。用户确认后，由 [`UserEvent::TerminalQuit`] 继续处理
    /// 关闭请求 `request`；用户取消时不执行任何操作。
    pub(super) fn confirm_terminal_quit(&mut self, request: CloseRequest) -> bool {
        if self.closing.confirming.is_some() {
            return true;
        }
        if self.config.terminal_confirm_close == Some(false) {
            return false;
        }
        let busy: Vec<&str> = (self.terminal.all_tabs().flat_map(TermTab::panes))
            .filter(|tab| match &request {
                CloseRequest::Tabs(ids) => self
                    .terminal
                    .group_of(tab.id)
                    .and_then(|at| self.terminal.editor_at(at))
                    .is_some_and(|id| ids.contains(&id)),
                _ => true,
            })
            .filter_map(|t| t.busy())
            .collect();
        let (name, count) = match &busy[..] {
            [] => return false,
            &[one] => (Some(one), 1),
            many => (None, many.len()),
        };
        let message = match (&request, name) {
            (CloseRequest::Tabs(_), _) => t!("terminal-close-confirm", name = busy.join(", ")),
            (CloseRequest::Quit, Some(name)) => t!("terminal-quit-confirm-one", name = name),
            (CloseRequest::Quit, None) => t!("terminal-quit-confirm-many", count = count),
            (CloseRequest::Switch(_), Some(name)) => {
                t!("terminal-switch-folder-confirm-one", name = name)
            }
            (CloseRequest::Switch(_), None) => {
                t!("terminal-switch-folder-confirm-many", count = count)
            }
            (CloseRequest::CloseFolder, Some(name)) => {
                t!("terminal-close-folder-confirm-one", name = name)
            }
            (CloseRequest::CloseFolder, None) => {
                t!("terminal-close-folder-confirm-many", count = count)
            }
            (_, Some(name)) => t!("terminal-close-window-confirm-one", name = name),
            (_, None) => t!("terminal-close-window-confirm-many", count = count),
        };
        let (detail, button) = match request {
            CloseRequest::Quit => (t!("terminal-quit-confirm-detail"), t!("cmd-quit")),
            CloseRequest::Switch(_) => (
                t!("terminal-switch-folder-confirm-detail"),
                t!("terminal-switch-folder-confirm-button"),
            ),
            CloseRequest::CloseFolder => (
                t!("terminal-close-folder-confirm-detail"),
                t!("cmd-close-folder"),
            ),
            CloseRequest::Tabs(_) => (t!("terminal-close-confirm-detail"), t!("common-close")),
            CloseRequest::CloseWindow => (
                t!("terminal-close-window-confirm-detail"),
                t!("cmd-close-window"),
            ),
        };
        if self.window.handle.is_some() {
            // macOS 上关闭最后一个窗口时窗口只是隐藏，终端继续运行：先显示窗口，确认框才可见。
            self.show_window();
            let shown = self.show_confirm(&message, detail, button, move |ok| {
                Some(UserEvent::TerminalQuit(ok))
            });
            // 没有弹出时不记「正在确认」：否则退出、关窗口一直等一个不会来的回答。这次关闭
            // 不进行，与取消相同。
            if shown {
                self.closing.confirming = Some(request);
            }
            return true;
        }
        false
    }

    /// 是否把面板放在编辑区右边（`terminal.position` 为 `"right"`）。
    pub(crate) fn terminal_right(&self) -> bool {
        self.config.terminal_right == Some(true)
    }

    fn panel_clock(&mut self, duration: Duration) -> Option<Clock> {
        #[cfg(test)]
        if let Some(at) = self.terminal.frozen {
            return Some(Clock::frozen(at, duration));
        }
        self.clock(duration)
    }

    /// 显示或收起面板，带动画；收起时焦点回到编辑区。
    ///
    /// 只改面板，不新建、不关标签。
    pub(super) fn set_terminal_open(&mut self, open: bool) {
        if open == self.terminal.open {
            return;
        }
        let from = self.terminal_shown().0;
        self.terminal.open = open;
        // 打开时正在看的标签就在眼前，它的未读点不需要保留。
        if open && let Some(tab) = self.terminal.tabs.get_mut(self.terminal.active) {
            tab.unseen = None;
        }
        if !open && !self.terminal_in_editor() {
            self.finish_terminal_rename(true);
        }
        if !open
            && ((!self.terminal_in_editor() && self.focus.in_terminal())
                || self.focus_in_docked_search())
        {
            self.focus = Focus::Editor;
        }
        self.terminal.motion = self
            .panel_clock(SIDEBAR_ANIMATION)
            .map(|clock| PanelMotion {
                from,
                moved: false,
                clock,
            });
    }

    /// 状态栏上的终端按钮、标签栏右端的收起按钮和 ⌘J：面板开着就收起，保存就打开（没有标签时
    /// 开一个）。与焦点位置无关，收起时焦点在面板外的留在原处。
    pub(super) fn click_terminal_button(&mut self) {
        if self.terminal.open {
            self.set_terminal_open(false);
        } else {
            self.open_terminal_panel();
        }
        self.reset_blink();
        self.relayout();
        self.request_redraw();
    }

    /// 面板露出的比例（0 到 1）和在哪一边（在右边为真）。
    pub(crate) fn terminal_shown(&self) -> (f32, bool) {
        let right = self.terminal_right();
        let target = if self.terminal.open { 1.0 } else { 0.0 };
        let Some(motion) = self.terminal.motion else {
            return (target, right);
        };
        if !motion.moved {
            return (motion.clock.between(motion.from, target), right);
        }
        let ease = |t: f32| 1.0 - (1.0 - t.min(1.0)).powi(3);
        let t = motion.clock.linear() * 2.0;
        if t < 1.0 {
            (1.0 - ease(t), !right)
        } else {
            (ease(t - 1.0), right)
        }
    }

    /// 最大化或还原面板（⌘⇧↩）。
    ///
    /// 大小逐渐变化，shell 逐帧重排，与拖动分隔条时相同；面板保存时打开并直接最大化。最大化后
    /// 编辑区看不见，焦点放进面板。
    pub(super) fn toggle_terminal_maximized(&mut self) {
        if !self.terminal.open {
            self.terminal.maximized = true;
            self.open_terminal_panel();
        } else {
            self.terminal.maximized = !self.terminal.maximized;
            self.start_terminal_grow();
            if self.terminal.maximized && !self.focus.in_terminal() {
                self.focus = Focus::Terminal;
            }
        }
        self.reset_blink();
        self.relayout();
        self.request_redraw();
    }

    /// 开始最大化、还原的过渡：面板在平常的大小和整栏之间伸缩，方向由 `maximized` 决定。
    ///
    /// 上一段过渡还没走完时（例如拖动时吸附又松开）从当前的位置往回走。
    pub(in crate::app) fn start_terminal_grow(&mut self) {
        self.terminal.grow = match self.terminal.grow.filter(|clock| !clock.done()) {
            Some(clock) => Some(clock.reversed()),
            None => self.panel_clock(SIDEBAR_ANIMATION),
        };
    }

    /// 要在编辑区里打开或切换文件：面板最大化着时先还原，让出编辑区。
    pub(in crate::app) fn make_room_for_editor(&mut self) {
        if self.terminal.open && self.terminal.maximized {
            self.toggle_terminal_maximized();
        }
    }

    /// 切换面板的位置（编辑区右边或下面），写回配置文件并重新排版。
    ///
    /// 开着时先在原来的位置收起，
    /// 再从新位置出来。
    pub(super) fn toggle_terminal_position(&mut self) {
        self.config.terminal_right = Some(!self.terminal_right());
        self.save_config();
        if self.terminal.open {
            self.terminal.motion = self.panel_clock(MOVE_ANIMATION).map(|clock| PanelMotion {
                from: 1.0,
                moved: true,
                clock,
            });
        }
        self.relayout();
        self.request_redraw();
    }

    /// 新建一个终端标签（⌘T）。
    pub(super) fn new_terminal_tab(&mut self) {
        self.new_terminal_tab_in(Vec::new());
    }

    /// 在 `cwd` 新建一个终端标签（空的是打开的目录）。
    pub(super) fn new_terminal_tab_in(&mut self, cwd: Vec<u8>) {
        self.new_terminal_tab_with(cwd, None);
    }

    /// 在 `cwd` 用 `shell` 新建一个终端标签（`None` 是默认 shell）。
    pub(super) fn new_terminal_tab_with(&mut self, cwd: Vec<u8>, shell: Option<String>) {
        // 辅助进程退出了：先恢复原有标签，否则新标签会被当作已退出的标签显示。
        if self.terminal.crashed {
            self.restart_terminal_tab();
        }
        if self.terminal_in_editor() && self.terminal.split_into.is_none() {
            let group = self.ensure_terminal_panel_group();
            self.blur_terminal_tab();
            self.rotate_terminal_group(group);
            self.set_terminal_open(true);
        }
        if !self.terminal.open && !self.terminal_in_editor() {
            // 打开面板时没有标签、也没有恢复的标签要建，会先放一个默认的：换成这个默认标签。
            // 恢复的那些标签（面板保存时退出的）留着，这个默认标签加在后面。
            let empty = self.terminal.panel_tab_count() == 0 && self.terminal.want.is_empty();
            self.open_terminal_panel();
            if empty {
                self.terminal.want.clear();
                self.terminal.want_names.clear();
                self.terminal.want_shells.clear();
                self.terminal.want_into.clear();
                self.terminal.want_groups.clear();
            }
        }
        let t = &mut self.terminal;
        t.want_shells.resize(t.want.len(), None);
        t.want_shells.push(shell);
        t.want_into.resize(t.want.len(), None);
        t.want_into.push(t.split_into.take());
        t.want_groups.resize(t.want.len(), t.group_at);
        t.want_groups.push(t.group_at);
        t.want.push(cwd);
        // 新建的这一个就是要看的，不再切回恢复时记着的那一个。
        self.terminal.want_active = None;
        self.focus = Focus::Terminal;
        self.start_terminal();
        self.relayout();
        self.request_redraw();
    }

    /// 关掉编号是 `keep` 的标签以外的各个标签；其中仍有程序在运行的同样先确认。
    pub(in crate::app) fn close_other_terminal_tabs(&mut self, keep: u16) {
        let others: Vec<u16> = (self.terminal.tabs.iter())
            .map(|tab| tab.id)
            .filter(|&id| id != keep)
            .collect();
        for id in others {
            if let Some(index) = self.terminal.index_of(id) {
                self.close_terminal_tab(index);
            }
        }
    }

    /// 关掉一个标签和它拆分出的各个终端（标签上的 ×、标签的菜单）。
    ///
    /// 里面还有程序在运行时先确认（`terminal.confirmClose`）。
    pub(super) fn close_terminal_tab(&mut self, index: usize) {
        let Some(tab) = self.terminal.tabs.get(index) else {
            return;
        };
        let busy = tab.panes().find_map(TermTab::busy).map(str::to_owned);
        self.confirm_terminal_close(tab.id, busy, true);
    }

    /// 关掉当前标签里前台的那个终端（⌘W、终端里的菜单）；标签里只有它时标签也关掉。
    pub(super) fn close_terminal_pane(&mut self) {
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        let busy = tab.busy().map(str::to_owned);
        self.confirm_terminal_close(tab.id, busy, false);
    }

    /// `busy` 是还在运行的程序：按设置先确认，再关掉终端 `id`（`whole` 为真时连同它所在的
    /// 整个标签）。
    fn confirm_terminal_close(&mut self, id: u16, busy: Option<String>, whole: bool) {
        let busy = busy.filter(|_| self.config.terminal_confirm_close != Some(false));
        if let Some(busy) = busy
            && self.window.handle.is_some()
        {
            let message = t!("terminal-close-confirm", name = busy);
            let detail = t!("terminal-close-confirm-detail");
            self.show_confirm(&message, detail, t!("common-close"), move |ok| {
                ok.then_some(UserEvent::TerminalClose(id, whole))
            });
            return;
        }
        self.confirmed_terminal_close(id, whole);
    }

    /// 确认框上确认了关掉终端 `id`；`whole` 为真时连同它所在的整个标签。
    pub(crate) fn confirmed_terminal_close(&mut self, id: u16, whole: bool) {
        let Some(at) = self.terminal.group_of(id) else {
            return;
        };
        self.with_terminal_group(at, |app| match app.terminal.index_of(id) {
            Some(index) if whole => app.close_terminal_tab_now(index),
            Some(_) => app.close_terminal_now(id),
            None => {}
        });
        self.tidy_terminal_groups();
    }

    /// 关掉第 `index` 个标签里的各个终端。
    fn close_terminal_tab_now(&mut self, index: usize) {
        let Some(tab) = self.terminal.tabs.get(index) else {
            return;
        };
        let ids: Vec<u16> = tab.panes().map(|pane| pane.id).collect();
        for id in ids {
            self.close_terminal_now(id);
        }
    }

    /// 关掉终端 `id`（可以是拆分出的），挂断它的 shell；标签里没有别的终端时标签也关掉。
    ///
    /// 最后一个也关了时收起面板，焦点回编辑区。
    fn close_terminal_now(&mut self, id: u16) {
        let Some(index) = self.terminal.index_of(id) else {
            return;
        };
        let editor = self.terminal.editor_document();
        if self.terminal.tabs[index].pane_count() == 1 {
            self.animate_terminal_close(index);
        }
        let t = &mut self.terminal;
        let Some((index, gone)) = t.remove_terminal(id) else {
            return;
        };
        if let Some(client) = &t.client {
            client.send(&ToHelper::Close { tab: id });
        }
        // 最后一个关掉了（也没有等着建的）：辅助进程自己会退出，连接马上放掉（关掉它的
        // stdin）。之后新建的标签由下一个辅助进程建，不发给正在退出的这一个（发了也收不到，
        // 还会被当成辅助进程中止）。
        let last = t.tab_count() == 0 && t.want.is_empty() && t.run_queue.is_empty();
        if last {
            t.client = None;
            t.ready = false;
            t.crashed = false;
        }
        self.forget_terminal_tab(id);
        let t = &mut self.terminal;
        if gone && (t.active > index || t.active >= t.tabs.len()) {
            t.active = t.active.saturating_sub(1);
        }
        let waiting_here = t
            .want
            .iter()
            .enumerate()
            .any(|(i, _)| t.want_groups.get(i).copied().unwrap_or(t.group_at) == t.group_at);
        if gone
            && !waiting_here
            && let Some(document) = editor
        {
            if let Some(group) = t.groups.get_mut(t.group_at) {
                group.editor = None;
                group.shown = false;
            }
            self.detach_terminal_editor_document(document);
        }
        if last {
            self.set_terminal_open(false);
            self.focus = Focus::Editor;
        } else {
            // 这一栏的最后一个关了：去掉这一栏，改用相邻的一栏。
            self.tidy_terminal_groups();
        }
        self.terminal_activated();
        self.relayout();
        self.request_redraw();
    }

    /// 清理已删除标签的相关状态：还没回来的取行请求（辅助进程不会再答）、查找、F8、链接、
    /// 改名字的输入框；焦点在它的输入框里时回到终端。
    pub(in crate::app::terminal) fn forget_terminal_tab(&mut self, id: u16) {
        let t = &mut self.terminal;
        t.requests.retain(|&(_, tab, _)| tab != id);
        if t.find.as_ref().is_some_and(|f| f.tab == id) {
            t.find = None;
            if self.focus == Focus::TerminalFind {
                self.focus = Focus::Terminal;
            }
        }
        if t.errors.as_ref().is_some_and(|e| e.tab == id) {
            t.errors = None;
        }
        if t.link.as_ref().is_some_and(|l| l.tab == id) {
            t.link = None;
        }
        if t.rename.as_ref().is_some_and(|(tab, _)| *tab == id) {
            t.rename = None;
            if self.focus == Focus::TerminalRename {
                self.focus = Focus::Terminal;
            }
        }
    }

    /// 切到第 `index` 个标签。
    pub(super) fn activate_terminal_tab(&mut self, index: usize) {
        if index < self.terminal.tabs.len() {
            if index != self.terminal.active {
                self.blur_terminal_tab();
            }
            self.terminal.active = index;
            self.terminal.tabs[index].unseen = None;
            self.terminal_activated();
            self.request_redraw();
        }
    }

    /// 切到相邻的标签（⌘⇧[、⌘⇧]），到头了绕回去。
    pub(super) fn cycle_terminal_tab(&mut self, forward: bool) {
        if self.terminal_in_editor() {
            self.cycle_tab(forward);
            return;
        }
        let n = self.terminal.tabs.len();
        if n > 1 {
            let i = self.terminal.active;
            self.activate_terminal_tab(if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            });
        }
    }

    /// 正在看的标签变了：丢掉属于其他标签的查找、报错跳转和链接。
    ///
    /// 保留它们时查找条会显示在另一个标签上，↩ 会去打开原标签的报错，不交给 shell。
    pub(in crate::app::terminal) fn drop_other_tab_state(&mut self) {
        let id = self.terminal.tab().map(|tab| tab.id);
        let t = &mut self.terminal;
        if t.find.as_ref().is_some_and(|find| Some(find.tab) != id) {
            t.find = None;
            t.requests.retain(|r| r.2 != Request::Find);
            if self.focus == Focus::TerminalFind {
                self.focus = Focus::Terminal;
            }
        }
        let t = &mut self.terminal;
        if t.errors
            .as_ref()
            .is_some_and(|errors| Some(errors.tab) != id)
        {
            t.errors = None;
        }
        if t.link.as_ref().is_some_and(|link| Some(link.tab) != id) {
            t.link = None;
        }
        // 面板开着时正在看的标签就在眼前，未读点不需要保留。
        if t.group_shown(t.group_at)
            && let Some(tab) = t.tabs.get_mut(t.active)
        {
            tab.unseen = None;
        }
    }

    /// 换了正在看的标签：把它攒着的额度发出去，马上拿到最新的一帧。
    fn terminal_activated(&mut self) {
        self.drop_other_tab_state();
        let t = &mut self.terminal;
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get_mut(t.active)) else {
            return;
        };
        send_credit(client, tab);
        t.focus_sent = false;
    }

    /// 要离开正在看的标签：告诉里面的程序失去了焦点（开了焦点上报时），否则它一直以为自己在前台。
    fn blur_terminal_tab(&mut self) {
        let t = &mut self.terminal;
        if !std::mem::take(&mut t.focus_sent) {
            return;
        }
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get(t.active)) else {
            return;
        };
        let mut bytes = Vec::new();
        input::focus(false, tab.modes, &mut bytes);
        if !bytes.is_empty() {
            client.send(&ToHelper::Input {
                tab: tab.id,
                bytes,
                rest: 0,
            });
        }
    }

    /// 画完一帧以后：给正在看的标签（拆分出的终端也在眼前）额度，焦点变了时告诉终端里的程序。
    pub(super) fn after_terminal_frame(&mut self) {
        let focused = self.focus == Focus::Terminal
            && self.window.focused
            && self.terminal.group_shown(self.terminal.group_at);
        let t = &mut self.terminal;
        let shown = t.group_shown(t.group_at);
        if let Some(client) = &t.client {
            for (at, group) in t.groups.iter_mut().enumerate() {
                if at == t.group_at
                    || !(if group.editor.is_some() {
                        group.shown
                    } else {
                        t.open
                    })
                {
                    continue;
                }
                if let Some(tab) = group.tabs.get_mut(group.active) {
                    send_credit(client, tab);
                }
            }
        }
        if !shown {
            self.blur_terminal_tab();
            return;
        }
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get_mut(t.active)) else {
            return;
        };
        send_credit(client, tab);
        if focused != t.focus_sent {
            t.focus_sent = focused;
            let mut bytes = Vec::new();
            input::focus(focused, tab.modes, &mut bytes);
            if !bytes.is_empty() {
                client.send(&ToHelper::Input {
                    tab: tab.id,
                    bytes,
                    rest: 0,
                });
            }
        }
    }
}
