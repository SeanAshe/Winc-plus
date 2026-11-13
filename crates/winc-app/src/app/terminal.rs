//! 终端面板：编辑区卡片下方的一张卡片，可以有几个终端标签。
//!
//! 第一次打开面板时启动辅助进程 `winc-pty`，每个标签是它里面的一个登录 shell。帧存成可见的一屏，
//! 往上翻时按需向它要回滚缓冲里的行（[`tab`]）。只给开着的标签额度：后面的标签不画、不收帧，
//! 切过去时再要一帧最新的；模块关着时什么都不起。鼠标在 [`mod@pointer`]，查找在 [`find`]，命令块
//! 在 [`blocks`]，⌘ 悬停链接在 [`link`]，与编辑器配合在 [`editor`]，进程在 [`helper`]。

mod blocks;
mod editor;
mod find;
mod helper;
mod keys;
pub(crate) mod link;
mod locate;
mod pointer;
pub(crate) mod run;
pub(crate) mod shells;
pub(crate) mod strip;
mod tab;

use helper::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use winc_config::{OptionAsMeta, TerminalSession};
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
pub(crate) use tab::{SETTLE, TabEnd, TermTab, ViewRow};

/// 面板在下面时的默认高度（逻辑像素，含标签栏）。
pub(crate) const DEFAULT_HEIGHT: f32 = 242.0;
/// 面板在下面时的最矮高度（逻辑像素，含标签栏）：标签栏加几行。
pub(crate) const MIN_HEIGHT: f32 = 96.0;
/// 在下面拖高面板时给编辑区至少留的高度（逻辑像素）。
pub(crate) const EDITOR_MIN: f32 = 120.0;
/// 面板在编辑区右边时的默认宽度（逻辑像素，经验值）。
pub(crate) const DEFAULT_WIDTH: f32 = 480.0;
/// 面板在编辑区右边时的最窄宽度（逻辑像素，经验值）：再窄时一行放不下几个字。
pub(crate) const MIN_WIDTH: f32 = 280.0;
/// 面板在右边时至少给编辑区留的宽度（逻辑像素，经验值）。
pub(crate) const EDITOR_MIN_WIDTH: f32 = 360.0;
/// 面板在下面时标签栏的高度（逻辑像素）：`TAB_HEIGHT` 上下各留 5。
const HEADER: f32 = crate::theme::TAB_HEIGHT + 10.0;
/// 网格右边和下方的留白（逻辑像素）。
pub(crate) const PAD: f32 = 8.0;
/// 网格左边的留白（逻辑像素），命令块的色带画在这里。
pub(crate) const GUTTER: f32 = 14.0;
/// 色带离面板左边的距离（逻辑像素）。
pub(crate) const BAND_X: f32 = 5.0;
/// 发出新的尺寸后等待该尺寸一帧的最长时间（100 ms，经验值）。
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
    /// 新建标签的「+」。
    New,
    /// 「重新启动」：shell 退出了、或者辅助进程退出了时。
    Restart,
    /// 收起面板（和状态栏上的终端按钮一样）。
    Hide,
    /// 「+」右边选 shell 的按钮。
    ShellMenu,
    /// 把面板挪到编辑区右边，或者挪回下面。
    Position,
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
    /// 最大化（⌘⇧↩）：面板占到排版允许的最大，编辑区只留最小的高度（宽度）。
    pub maximized: bool,
    /// 最大化、还原时大小的渐变：（开始时的高度或宽度，时钟）。
    pub grow: Option<(f32, Clock)>,
    /// 测试里没有窗口、没有时钟：设置后面板的动画都停在这一时刻。
    #[cfg(test)]
    pub frozen: Option<Duration>,
    client: Option<Client>,
    /// 辅助进程握过手了，可以新建终端。
    ready: bool,
    /// 各终端标签。
    pub tabs: Vec<TermTab>,
    /// 正在看的标签。
    pub active: usize,
    /// 等辅助进程准备好、知道面板尺寸以后要新建的标签：各自的工作目录（空的是默认目录）。
    pub want: Vec<Vec<u8>>,
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
    /// 标签名和结束状态只显示在面板里，面板收起时不计入；未读标记同时显示在状态栏。
    pub(in crate::app) fn visible_state(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.open, self.active, self.crashed, &self.error).hash(&mut hasher);
        for tab in &self.tabs {
            tab.unseen.hash(&mut hasher);
            if self.open {
                (tab.id, tab.label(), tab.ended).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// 消息是否作用于正在显示的内容。标题、进程、未读标记等的变化由 [`Self::visible_state`] 判断。
    pub(in crate::app) fn message_is_shown(&self, message: &FromHelper) -> bool {
        let shows = |id: u16| self.open && self.tab().is_some_and(|tab| tab.id == id);
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

    fn index_of(&self, id: u16) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn by_id(&mut self, id: u16) -> Option<&mut TermTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
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
            self.focus = Focus::Terminal;
        }
        self.reset_blink();
        self.relayout();
        self.request_redraw();
    }

    /// 打开面板、焦点放进去；还没有标签时开一个。
    fn open_terminal_panel(&mut self) {
        self.set_terminal_open(true);
        let t = &mut self.terminal;
        if t.height <= 0.0 {
            t.height = DEFAULT_HEIGHT;
        }
        if t.tabs.is_empty() && t.want.is_empty() {
            t.want.push(Vec::new());
        }
        self.focus = Focus::Terminal;
        self.start_terminal();
    }

    /// 现场里要记下的终端：各标签的工作目录、正在看的、面板开着没有、大小。
    ///
    /// 没有标签时 `None`。
    pub(super) fn terminal_session(&self) -> Option<TerminalSession> {
        let t = &self.terminal;
        // 还没建出来的（面板收着时恢复的）也算。
        // 运行配置的标签不恢复。
        let shells = || t.tabs.iter().filter(|tab| tab.run.is_none());
        let tabs: Vec<PathBuf> = shells()
            .map(|tab| bytes_path(&tab.cwd))
            .chain(t.want.iter().map(|cwd| bytes_path(cwd)))
            .collect();
        let pending = (0..t.want.len()).map(|i| t.want_names.get(i).cloned().unwrap_or_default());
        let names = shells().map(|tab| tab.name.clone()).chain(pending);
        let pending = (0..t.want.len()).map(|i| t.want_shells.get(i).cloned().flatten());
        let commands = shells().map(|tab| tab.shell.clone()).chain(pending);
        let before = t.tabs.iter().take(t.active).filter(|tab| tab.run.is_some());
        let active = t.active - before.count();
        (!tabs.is_empty()).then(|| TerminalSession {
            active: t.want_active.unwrap_or(active),
            tabs,
            names: names.collect(),
            shells: commands.map(Option::unwrap_or_default).collect(),
            open: t.open,
            height: t.height,
            width: t.width,
        })
    }

    /// 打开目录时按现场重开上次的终端标签，各在原来的目录里（程序不恢复）。
    ///
    /// 终端里已经有标签了不动；面板上次收着时等打开面板再建。
    pub(super) fn restore_terminal(&mut self, session: &TerminalSession) {
        let t = &mut self.terminal;
        let enabled = self.config.terminal_enabled.unwrap_or(false);
        if !enabled || !t.tabs.is_empty() || !t.want.is_empty() {
            return;
        }
        let tabs = session.tabs.iter();
        t.want = tabs.map(|p| os_bytes(p.as_os_str())).collect();
        t.want_names.clone_from(&session.names);
        let saved = session.shells.iter();
        t.want_shells = saved
            .map(|command| {
                Some(command.clone()).filter(|c| !c.is_empty() && shells::still_usable(c))
            })
            .collect();
        t.want_active = Some(session.active);
        if session.height > 0.0 {
            t.height = session.height;
        }
        if session.width > 0.0 {
            t.width = session.width;
        }
        if session.open {
            self.set_terminal_open(true);
            self.start_terminal();
        }
    }

    /// 窗口建好时按现场恢复终端：启动时恢复的目录在窗口创建之前打开，当时还不能启动辅助进程。
    ///
    /// 面板直接到位，没有打开动画；测量启动时间时不恢复。
    pub(in crate::app) fn restore_terminal_for_window(&mut self) {
        if self.frame.bench.silent() {
            return;
        }
        let session = self
            .editor
            .root()
            .and_then(|root| self.state.session_for(root));
        if let Some(terminal) = session.and_then(|session| session.terminal.clone()) {
            self.restore_terminal(&terminal);
            self.terminal.motion = None;
        }
    }

    /// 换目录或关闭目录时关掉所有终端标签，挂断其中的 shell，不确认、不收起面板。
    ///
    /// 调用方须先确认运行中的程序（[`Self::confirm_terminal_quit`]），并把标签记入原目录的
    /// 现场。
    pub(super) fn close_all_terminal_tabs(&mut self) {
        let t = &mut self.terminal;
        let ids: Vec<u16> = t.tabs.iter().map(|tab| tab.id).collect();
        if let Some(client) = &t.client {
            for &tab in &ids {
                client.send(&ToHelper::Close { tab });
            }
        }
        t.tabs.clear();
        t.want.clear();
        t.want_names.clear();
        t.want_shells.clear();
        t.want_active = None;
        t.active = 0;
        t.client = None;
        t.ready = false;
        for id in ids {
            self.forget_terminal_tab(id);
        }
        if self.focus.in_terminal() {
            self.focus = Focus::Editor;
        }
    }

    /// 换到新目录后的终端：按新目录的现场重开它的标签；新目录没有记下终端、面板开着时在
    /// 新目录里开一个标签；面板开着而新目录上次是收着的，收起面板。
    pub(super) fn switch_terminal(&mut self, session: Option<&TerminalSession>) {
        match session {
            Some(session) => {
                if !session.open && self.terminal.open {
                    self.set_terminal_open(false);
                }
                self.restore_terminal(session);
            }
            None if self.terminal.open && self.terminal.tabs.is_empty() => {
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
            && self.terminal.tabs.iter().any(|t| t.busy().is_some())
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
        let busy: Vec<&str> = self.terminal.tabs.iter().filter_map(|t| t.busy()).collect();
        let (name, count) = match &busy[..] {
            [] => return false,
            &[one] => (Some(one), 1),
            many => (None, many.len()),
        };
        let message = match (&request, name) {
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
            CloseRequest::CloseWindow | CloseRequest::Tabs(_) => (
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
        if !open {
            self.finish_terminal_rename(true);
        }
        if !open && self.focus.in_terminal() {
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

    /// 状态栏上的终端按钮、标签栏右端的收起按钮：面板开着就收起，收着就打开（没有标签时开一个）。
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
    /// 大小逐渐变化，shell 逐帧重排，与拖动分隔条时相同；面板收着
    /// 时打开并直接最大化。
    pub(super) fn toggle_terminal_maximized(&mut self) {
        if !self.terminal.open {
            self.terminal.maximized = true;
            self.open_terminal_panel();
        } else {
            let l = self.layout.terminal;
            let from = if self.terminal_right() { l.w } else { l.h };
            self.terminal.maximized = !self.terminal.maximized;
            self.terminal.grow = self.panel_clock(SIDEBAR_ANIMATION).map(|c| (from, c));
        }
        self.relayout();
        self.request_redraw();
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
        if !self.terminal.open {
            // 打开面板时没有标签、也没有恢复的标签要建，会先放一个默认的：换成这个默认标签。
            // 恢复的那些标签（面板收着时退出的）留着，这个默认标签加在后面。
            let empty = self.terminal.tabs.is_empty() && self.terminal.want.is_empty();
            self.open_terminal_panel();
            if empty {
                self.terminal.want.clear();
                self.terminal.want_names.clear();
                self.terminal.want_shells.clear();
            }
        }
        let t = &mut self.terminal;
        t.want_shells.resize(t.want.len(), None);
        t.want_shells.push(shell);
        t.want.push(cwd);
        // 新建的这一个就是要看的，不再切回恢复时记着的那一个。
        self.terminal.want_active = None;
        self.focus = Focus::Terminal;
        self.start_terminal();
        self.relayout();
        self.request_redraw();
    }

    /// 关掉一个标签（⌘W、标签上的 ×）。
    ///
    /// 里面还有程序在运行时先确认（`terminal.confirmClose`）。
    pub(super) fn close_terminal_tab(&mut self, index: usize) {
        let Some(tab) = self.terminal.tabs.get(index) else {
            return;
        };
        let busy = tab
            .busy()
            .filter(|_| self.config.terminal_confirm_close != Some(false));
        if let Some(busy) = busy
            && self.window.handle.is_some()
        {
            let id = tab.id;
            let message = t!("terminal-close-confirm", name = busy);
            let detail = t!("terminal-close-confirm-detail");
            self.show_confirm(&message, detail, t!("common-close"), move |ok| {
                ok.then_some(UserEvent::TerminalClose(id))
            });
            return;
        }
        self.close_terminal_tab_now(index);
    }

    /// 确认框上确认了关掉编号是 `id` 的标签。
    pub(crate) fn confirmed_terminal_close(&mut self, id: u16) {
        if let Some(index) = self.terminal.index_of(id) {
            self.close_terminal_tab_now(index);
        }
    }

    /// 关掉一个标签：挂断它的 shell。
    ///
    /// 最后一个也关了时收起面板，焦点回编辑区。
    fn close_terminal_tab_now(&mut self, index: usize) {
        if index >= self.terminal.tabs.len() {
            return;
        }
        self.animate_terminal_close(index);
        let t = &mut self.terminal;
        let tab = t.tabs.remove(index);
        if let Some(client) = &t.client {
            client.send(&ToHelper::Close { tab: tab.id });
        }
        // 最后一个关掉了（也没有等着建的）：辅助进程自己会退出，连接马上放掉（关掉它的
        // stdin）。之后新建的标签由下一个辅助进程建，不发给正在退出的这一个（发了也收不到，
        // 还会被当成辅助进程中止）。
        let last = t.tabs.is_empty() && t.want.is_empty() && t.run_queue.is_empty();
        if last {
            t.client = None;
            t.ready = false;
        }
        self.forget_terminal_tab(tab.id);
        let t = &mut self.terminal;
        if t.active > index || t.active >= t.tabs.len() {
            t.active = t.active.saturating_sub(1);
        }
        if last {
            self.set_terminal_open(false);
            self.focus = Focus::Editor;
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
            // F8 的跳转属于原来那个标签：如果保留，在这里按 ↩ 会去打开那边的报错，不交给 shell。
            let id = self.terminal.tabs[index].id;
            if self.terminal.errors.as_ref().is_some_and(|e| e.tab != id) {
                self.terminal.errors = None;
            }
            self.terminal.link = None;
            self.terminal.find = None;
            self.terminal.requests.retain(|r| r.2 != Request::Find);
            if self.focus == Focus::TerminalFind {
                self.focus = Focus::Terminal;
            }
            self.terminal_activated();
            self.request_redraw();
        }
    }

    /// 切到相邻的标签（⌘⇧[、⌘⇧]），到头了绕回去。
    pub(super) fn cycle_terminal_tab(&mut self, forward: bool) {
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

    /// 换了正在看的标签：把它攒着的额度发出去，马上拿到最新的一帧。
    fn terminal_activated(&mut self) {
        let t = &mut self.terminal;
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get_mut(t.active)) else {
            return;
        };
        if tab.credit_due {
            tab.credit_due = false;
            client.send(&ToHelper::Credit { tab: tab.id });
        }
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

    /// 画完一帧以后：给正在看的标签额度，焦点变了时告诉终端里的程序。
    pub(super) fn after_terminal_frame(&mut self) {
        let focused = self.focus == Focus::Terminal && self.window.focused;
        let t = &mut self.terminal;
        if !t.open {
            // 收起面板时焦点去了编辑区：程序也要知道（1004），否则一直以为自己有焦点。
            self.blur_terminal_tab();
            return;
        }
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get_mut(t.active)) else {
            return;
        };
        if tab.credit_due {
            tab.credit_due = false;
            client.send(&ToHelper::Credit { tab: tab.id });
        }
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
