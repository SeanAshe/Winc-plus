//! 终端仿真：字节进来，屏幕网格、回滚缓冲、帧和事件出去。
//!
//! [`Terminal::feed`] 把字节交给 `vte` 切分，切出来的字和序列由 `perform` 模块作用到网格上。
//! 主程序要用的数据都从这里取：[`Terminal::take_frame`]（上一帧以来的变化）、
//! [`Terminal::take_events`]、[`Terminal::take_responses`]（要写回伪终端的回答，例如光标位置
//! 查询）。模块不碰 IO，测试里直接喂字节、读取屏幕文本。

mod blocks;
mod control;
mod editing;
mod history;
mod perform;
mod reflow;
mod row;
pub mod width;

use std::collections::HashMap;

use blocks::Blocks;
use history::History;
use row::Row;

use crate::protocol::{
    Block, Color, Cursor, CursorShape, Event, Frame, Line, Marks, Modes, Size, Style, Theme,
};

/// OSC 字符串的最大长度（字节，2 MiB），超出的部分丢弃。
///
/// `vte` 开启 `std` 时把整个 OSC 攒在不限长的缓冲里，等到结束符才交出来，程序输出不结束的 OSC
/// （`printf '\e]0;'; yes`）时内存占用随输出持续增长，容量也不释放。最长的合法用途是 OSC 52 写
/// 剪贴板（解码前 1 MiB，见 `MAX_CLIPBOARD`），2 MiB 留有余量。
const MAX_OSC: usize = 2 << 20;

/// 触发更换 `vte` 解析器的 OSC 长度阈值（字节，64 KiB）。
///
/// `vte` 攒 OSC 的缓冲只增不减，一次 OSC 52 粘贴的 1 MiB 会长期占用内存；OSC 结束后换用新的
/// 解析器即可释放缓冲。普通的 OSC（标题、目录、链接）远小于该阈值，不受影响。
const SHRINK_OSC: usize = 64 << 10;

/// 读到 OSC 时的处理阶段。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Osc {
    /// 不在 OSC 中。
    Outside,
    /// 上一段以 ESC 结尾，下一个字节为「]」时进入 OSC。
    Escaped,
    /// 超过 [`SHRINK_OSC`] 的 OSC 以 ESC 结尾：下一个字节为「\」时 OSC 完整结束并更换解析器。
    LongEscaped,
    /// 处于 OSC 中，附带已交给 `vte` 的字节数。
    Inside(usize),
    /// OSC 超过 [`MAX_OSC`] 已截断，丢弃剩余字节直到结束符。
    Discard,
    /// 丢弃期间遇到 ESC：下一个字节为「\」时是字符串结束符（ST）的后一半，同样丢弃。
    DiscardEscaped,
}

/// 一个终端。
pub struct Terminal {
    /// `vte` 的解析器，交给它的 OSC 过长后换成新的解析器。
    parser: vte::Parser,
    /// 屏幕、回滚缓冲、命令块、模式和待发数据。
    emu: Emulator,
    /// OSC 的读取阶段。
    osc: Osc,
}

impl Terminal {
    /// 创建 `cols` × `rows` 格、回滚缓冲最多 `scrollback` 行的终端。
    ///
    /// `cols`、`rows` 小于 1 时按 1 计算。
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        Terminal {
            parser: vte::Parser::new(),
            emu: Emulator::new(cols.max(1), rows.max(1), scrollback),
            osc: Osc::Outside,
        }
    }

    /// 处理程序输出的字节。
    ///
    /// OSC 超过 `MAX_OSC` 时截断：已交给 `vte` 的部分用 CAN 结束，各 OSC 按自身的上限丢弃；剩余
    /// 字节直到结束符为止都不再交给 `vte`。
    pub fn feed(&mut self, mut bytes: &[u8]) {
        const ESC: u8 = 0x1b;
        const CAN: u8 = 0x18;
        // 处于 OSC 中时能结束 OSC 的字节：BEL、ESC（ST 的前一半或其他序列的开头）、CAN、SUB。
        let ends = |b: &u8| matches!(*b, 0x07 | ESC | CAN | 0x1a);
        while !bytes.is_empty() {
            match self.osc {
                Osc::Outside => {
                    let Some(at) = bytes.iter().position(|&b| b == ESC) else {
                        self.parser.advance(&mut self.emu, bytes);
                        return;
                    };
                    self.parser.advance(&mut self.emu, &bytes[..=at]);
                    bytes = &bytes[at + 1..];
                    self.osc = Osc::Escaped;
                }
                Osc::Escaped => {
                    if bytes[0] == b']' {
                        self.parser.advance(&mut self.emu, &bytes[..1]);
                        bytes = &bytes[1..];
                        self.osc = Osc::Inside(0);
                    } else {
                        self.osc = Osc::Outside;
                    }
                }
                Osc::Inside(len) => {
                    let end = bytes.iter().position(ends);
                    let take = end.unwrap_or(bytes.len());
                    if len + take > MAX_OSC {
                        let room = MAX_OSC - len;
                        self.parser.advance(&mut self.emu, &bytes[..room]);
                        self.parser.advance(&mut self.emu, &[CAN]);
                        // CAN 让 `vte` 结束截断的 OSC 并回到初始状态，新解析器随即释放攒下的缓冲。
                        self.parser = vte::Parser::new();
                        bytes = &bytes[room..];
                        self.osc = Osc::Discard;
                        continue;
                    }
                    self.parser.advance(&mut self.emu, &bytes[..take]);
                    bytes = &bytes[take..];
                    // 结束符照常交给 `vte`：ESC 也可能是下一个序列的开头。
                    self.osc = match end {
                        Some(_) if len + take > SHRINK_OSC && bytes[0] == ESC => {
                            self.parser.advance(&mut self.emu, &bytes[..1]);
                            bytes = &bytes[1..];
                            Osc::LongEscaped
                        }
                        Some(_) if len + take > SHRINK_OSC => {
                            // BEL、CAN、SUB 交出后 OSC 已经结束，解析器回到初始状态。
                            self.parser.advance(&mut self.emu, &bytes[..1]);
                            bytes = &bytes[1..];
                            self.parser = vte::Parser::new();
                            Osc::Outside
                        }
                        Some(_) => Osc::Outside,
                        None => Osc::Inside(len + take),
                    };
                }
                Osc::LongEscaped => {
                    if bytes[0] == b'\\' {
                        self.parser.advance(&mut self.emu, &bytes[..1]);
                        bytes = &bytes[1..];
                        self.parser = vte::Parser::new();
                        self.osc = Osc::Outside;
                    } else {
                        // 不是 ST：ESC 已经交给 `vte`，后续字节按普通 ESC 之后的情形处理。
                        self.osc = Osc::Escaped;
                    }
                }
                Osc::Discard => {
                    let Some(at) = bytes.iter().position(ends) else {
                        return;
                    };
                    self.osc = if bytes[at] == ESC {
                        Osc::DiscardEscaped
                    } else {
                        Osc::Outside
                    };
                    bytes = &bytes[at + 1..];
                }
                Osc::DiscardEscaped => {
                    if bytes[0] == b'\\' {
                        bytes = &bytes[1..];
                        self.osc = Osc::Outside;
                    } else {
                        // 不是 ST：该 ESC 是下一个序列的开头，补交给 `vte`。
                        self.parser.advance(&mut self.emu, &[ESC]);
                        self.osc = Osc::Escaped;
                    }
                }
            }
        }
    }

    /// 修改终端尺寸。
    ///
    /// 主屏按新宽度重排，备用屏只截断或补齐。
    pub fn resize(&mut self, size: Size) {
        self.emu.pixels = (size.width, size.height);
        self.emu
            .resize(usize::from(size.cols).max(1), usize::from(size.rows).max(1));
    }

    /// 清空屏幕和回滚缓冲，光标所在行（一般是提示符）保留并移到第一行（⌘K）。
    pub fn clear(&mut self) {
        self.emu.clear_all();
    }

    /// 设置回答颜色查询用的主题；为 `None` 时不回答颜色查询。
    ///
    /// 程序用模式 2031 要求过通知、明暗发生变化时，回答新的明暗。
    pub fn set_theme(&mut self, theme: Option<Theme>) {
        let before = self.emu.theme.as_ref().map(Theme::is_dark);
        self.emu.theme = theme;
        if self.emu.scheme_reports && self.emu.theme.as_ref().map(Theme::is_dark) != before {
            self.emu.report_color_scheme();
        }
    }

    /// 设置 shell 集成脚本报告命令行时附带的随机数。
    ///
    /// 只记录附带该随机数的命令行（OSC 133 `C`），程序输出中伪造的命令行不记录；没有设置随机数
    /// 时不记录任何命令行。
    pub fn set_nonce(&mut self, nonce: String) {
        self.emu.nonce = Some(nonce);
    }

    /// 列数。
    pub fn cols(&self) -> usize {
        self.emu.cols
    }

    /// 行数。
    pub fn rows(&self) -> usize {
        self.emu.rows
    }

    /// 上一帧以来的变化；什么都没变时返回 `None`。
    pub fn take_frame(&mut self) -> Option<Frame> {
        self.emu.take_frame()
    }

    /// 命令块（shell 集成）；与上次取出时相同则返回 `None`。
    pub fn take_blocks(&mut self) -> Option<Vec<Block>> {
        let emu = &mut self.emu;
        let base = emu.history.range().1;
        let screen: Vec<(u64, Marks)> = emu
            .primary
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !row.marks.is_empty())
            .map(|(i, row)| (base + i as u64, row.marks))
            .collect();
        emu.blocks
            .take(emu.history.range().0, &screen, emu.history.marks())
    }

    /// 取出积攒的事件，取走后列表清空。
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.emu.events)
    }

    /// 取出要写回伪终端的字节（对查询的回答），取走后清空。
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.emu.responses)
    }

    /// 是否因重排或清空回滚缓冲改变了行号，且变化尚未随帧发出。
    pub fn renumbered(&self) -> bool {
        self.emu.damage.reset
    }

    /// 程序是否开启了同步输出（模式 2026），即正在绘制一帧，暂不发送帧。
    pub fn syncing(&self) -> bool {
        self.emu.sync
    }

    /// 返回回滚缓冲中绝对行号在 `start..end` 内的行，以及实际第一行的行号。
    pub fn history(&self, start: u64, end: u64) -> (u64, Vec<Line>) {
        self.emu.history.get(start, end)
    }

    /// 返回屏幕上第 `row` 行的内容。
    ///
    /// # Panics
    ///
    /// `row` 不小于行数时 panic。
    pub fn line(&self, row: usize) -> Line {
        self.emu.screen().rows[row].to_line()
    }

    /// 返回屏幕上的文字：每行去掉行尾空格后用换行连接，去掉末尾的空行。
    ///
    /// 供测试和调试使用。
    pub fn screen_text(&self) -> String {
        let lines: Vec<String> = (0..self.emu.rows)
            .map(|r| self.line(r).trimmed().to_string())
            .collect();
        lines.join("\n").trim_end_matches('\n').to_string()
    }

    /// 返回光标位置（行，格），均从 0 起。
    pub fn cursor(&self) -> (usize, usize) {
        let cursor = self.emu.screen().cursor;
        (cursor.row, cursor.col)
    }

    /// 返回主程序用到的当前模式。
    pub fn modes(&self) -> Modes {
        self.emu.modes()
    }
}

/// 光标位置。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pos {
    /// 屏幕上的行号（从 0 起）。
    row: usize,
    /// 行内的格号（从 0 起）。
    col: usize,
    /// 刚在最后一格写了字：下一个字先换行再写（xterm 的「延迟换行」）。
    pending_wrap: bool,
}

/// DECSC 保存的状态。
#[derive(Debug, Clone, Copy, Default)]
struct Saved {
    /// 保存的光标。
    pos: Pos,
    /// 保存的画笔。
    pen: Style,
    /// 是否处于原点模式。
    origin: bool,
    /// G0、G1 是否切换到 DEC 制图字符集。
    charsets: [bool; 2],
    /// 是否处于 SO 移位状态，即使用 G1。
    shift_out: bool,
}

/// 主屏或备用屏。
#[derive(Debug)]
struct Screen {
    /// 各行，元素个数是终端的行数。
    rows: Vec<Row>,
    /// 光标。
    cursor: Pos,
    /// DECSC 保存的光标、画笔和字符集状态；没有保存过时为 `None`。
    saved: Option<Saved>,
    /// Kitty 键盘协议的标志位栈，主屏与备用屏各一个。
    keyboard: Vec<u8>,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        Screen {
            rows: (0..rows).map(|_| Row::new(cols, Color::Default)).collect(),
            cursor: Pos::default(),
            saved: None,
            keyboard: Vec::new(),
        }
    }
}

/// 上一帧以来发生变化的行。
#[derive(Debug, Default)]
struct Damage {
    /// 各行是否要重发，下标为屏幕上的行号。
    rows: Vec<bool>,
    /// 是否需要重发整屏。
    all: bool,
    /// 是否需要主程序丢弃缓存的行（重排、清空回滚缓冲）。
    reset: bool,
    /// 整屏向上滚动的行数，仅在 `all` 为假时有效。
    scroll: usize,
}

impl Damage {
    fn row(&mut self, row: usize) {
        if let Some(dirty) = self.rows.get_mut(row) {
            *dirty = true;
        }
    }

    fn range(&mut self, start: usize, end: usize) {
        let end = end.min(self.rows.len());
        if start < end {
            self.rows[start..end].fill(true);
        }
    }

    fn all(&mut self) {
        self.all = true;
    }

    /// 整屏向上滚动 `n` 行：已记下的脏行随之上移，底部空出的行记为脏。
    fn scroll_up(&mut self, n: usize) {
        if self.all {
            return;
        }
        let len = self.rows.len();
        if n >= len || self.scroll + n >= len {
            self.all = true;
            return;
        }
        self.rows.copy_within(n.., 0);
        self.rows[len - n..].fill(true);
        self.scroll += n;
    }

    fn any(&self) -> bool {
        self.all || self.reset || self.rows.contains(&true)
    }

    fn clear(&mut self, rows: usize) {
        self.rows.clear();
        self.rows.resize(rows, false);
        self.all = false;
        self.reset = false;
        self.scroll = 0;
    }
}

/// 超链接地址到编号的映射。
#[derive(Debug, Default)]
struct Links {
    /// 地址到编号；编号从 1 起，0 表示没有链接。
    ids: HashMap<String, u16>,
    /// 已记录地址的总字节数，达到 [`MAX_LINK_BYTES`] 后不再记录新地址。
    bytes: usize,
}

/// 一个终端记录的超链接地址总字节数上限（字节，4 MiB）。
///
/// 程序刷出成千上万个不同地址时，超出上限的地址不再当作链接，内存占用不再增长。
const MAX_LINK_BYTES: usize = 4 << 20;

/// 接收中的 DCS 序列。
#[derive(Debug)]
enum Dcs {
    /// XTGETTCAP（`DCS + q`）请求，附带已收到的字节。
    Termcap(Vec<u8>),
    /// DECRQSS（`DCS $ q`）请求，附带已收到的字节。
    Setting(Vec<u8>),
}

/// 终端的全部状态。
///
/// 实现 `vte::Perform`，各序列的处理逻辑在 `perform` 模块。
struct Emulator {
    /// 列数（格）。
    cols: usize,
    /// 行数。
    rows: usize,
    /// 主屏。
    primary: Screen,
    /// 备用屏，供全屏程序使用。
    alternate: Screen,
    /// 正在显示备用屏。
    alt: bool,
    /// 滚出屏幕顶部的行。
    history: History,
    /// 回滚缓冲刚被程序清空（ED 3），之后还没有写入内容。
    ///
    /// 此时再擦除整屏是一次完整的清屏（`clear` 命令），屏幕内容不再移入回滚缓冲。
    history_erased: bool,
    /// 上一次清屏时移入回滚缓冲的行；之后有其他行滚入回滚缓冲时清除。
    last_kept: Option<Vec<Row>>,
    /// 当前画笔，写入字符时使用。
    pen: Style,
    /// 滚动区域的首末行 `top..bottom`。
    top: usize,
    bottom: usize,
    /// 原点模式（DECOM）：行号从滚动区域顶边算起。
    origin: bool,
    /// 自动换行（DECAWM）。
    autowrap: bool,
    /// 插入模式（IRM）。
    insert: bool,
    /// 主程序用到的模式（按键编码、鼠标、粘贴）；`ALT_SCREEN` 由 `alt` 推导。
    modes: Modes,
    /// 光标是否显示（DECTCEM）。
    cursor_visible: bool,
    /// 光标形状（DECSCUSR）。
    shape: CursorShape,
    /// 程序是否要求光标闪烁。
    blink: bool,
    /// 制表位，每列一项，`true` 表示该列是制表位。
    tabs: Vec<bool>,
    /// G0、G1 是否切换到 DEC 制图字符集（框线字符）。
    charsets: [bool; 2],
    /// 是否处于 SO 移位状态，即使用 G1。
    shift_out: bool,
    /// 最后写入的字符，供 `CSI b`（REP）重复。
    last_char: Option<char>,
    /// 前面写入的字符组成的 emoji 字素簇；移动光标等控制序列之后重新开始。
    cluster: width::Cluster,
    /// 程序是否要求在明暗主题变化时收到通知（模式 2031）。
    scheme_reports: bool,
    /// 记下的超链接地址。
    links: Links,
    /// 同步输出（模式 2026）是否打开。
    sync: bool,
    /// 回答颜色查询用的主题；为 `None` 时不回答颜色查询。
    theme: Option<Theme>,
    /// 网格区域的像素尺寸（宽，高），用于回答窗口尺寸查询。
    pixels: (u16, u16),
    /// 收到一半的 DCS 序列。
    dcs: Option<Dcs>,
    /// 上一帧以来变了的行。
    damage: Damage,
    /// 上一帧发出的光标、模式、键盘标志和回滚范围，均未变化时不必发帧。
    sent: Option<(Cursor, Modes, u8, (u64, u64))>,
    /// 待发给主程序的事件。
    events: Vec<Event>,
    /// 待写回伪终端的字节，即对查询的回答。
    responses: Vec<u8>,
    /// 从行上的标记切出的命令块。
    blocks: Blocks,
    /// shell 集成脚本附带的随机数（OSC 133 `C` 的 `nonce=`）；为 `None` 时不记录命令行。
    nonce: Option<String>,
}

impl Emulator {
    fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let mut emu = Emulator {
            cols,
            rows,
            primary: Screen::new(cols, rows),
            alternate: Screen::new(cols, rows),
            alt: false,
            history: History::new(scrollback),
            history_erased: false,
            last_kept: None,
            pen: Style::default(),
            top: 0,
            bottom: rows,
            origin: false,
            autowrap: true,
            insert: false,
            modes: Modes(Modes::ALT_SCROLL),
            cursor_visible: true,
            shape: CursorShape::Block,
            blink: true,
            tabs: Vec::new(),
            charsets: [false; 2],
            shift_out: false,
            last_char: None,
            cluster: width::Cluster::None,
            scheme_reports: false,
            links: Links::default(),
            sync: false,
            theme: None,
            pixels: (0, 0),
            dcs: None,
            damage: Damage::default(),
            sent: None,
            events: Vec::new(),
            responses: Vec::new(),
            blocks: Blocks::default(),
            nonce: None,
        };
        emu.reset_tabs();
        emu.damage.clear(rows);
        emu.damage.all();
        emu
    }

    fn screen(&self) -> &Screen {
        if self.alt {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn screen_mut(&mut self) -> &mut Screen {
        if self.alt {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn cursor(&self) -> Pos {
        self.screen().cursor
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        let (rows, cols) = (self.rows, self.cols);
        let cursor = &mut self.screen_mut().cursor;
        cursor.row = row.min(rows - 1);
        cursor.col = col.min(cols - 1);
        cursor.pending_wrap = false;
    }

    fn modes(&self) -> Modes {
        let mut modes = self.modes;
        modes.set(Modes::ALT_SCREEN, self.alt);
        modes
    }

    fn keyboard(&self) -> u8 {
        self.screen().keyboard.last().copied().unwrap_or(0)
    }

    fn reset_tabs(&mut self) {
        // 每 8 列一个制表位，第 0 列除外；8 列为终端的默认制表宽度。
        self.tabs = (0..self.cols).map(|c| c % 8 == 0 && c > 0).collect();
    }

    /// 在光标所在行记录标记并记入命令块，`text` 为 `C` 附带的可信命令行。
    ///
    /// 只在主屏上记录：备用屏上运行的程序不是 shell。
    fn mark(&mut self, flag: u8, exit: Option<i32>, text: Option<String>) {
        if self.alt {
            return;
        }
        let block = match flag {
            Marks::PROMPT => self.blocks.prompt(),
            Marks::OUTPUT => {
                self.blocks.output(text);
                0
            }
            Marks::DONE => {
                self.blocks.done();
                0
            }
            _ => 0,
        };
        let cursor = self.cursor();
        let row = cursor.row;
        // 提示符总是逻辑行的开头。上一行可能因 zsh 的 `PROMPT_SP` 写满而折行，不断开时重排会把它和
        // 提示符合成一行。
        if flag == Marks::PROMPT
            && cursor.col == 0
            && let Some(above) = row.checked_sub(1)
        {
            self.primary.rows[above].wrapped = false;
        }
        let marks = &mut self.primary.rows[row].marks;
        marks.flags |= flag;
        if flag == Marks::DONE {
            marks.exit = exit.unwrap_or(0);
        }
        if block != 0 {
            marks.block = block;
        }
        self.damage.row(row);
    }

    fn take_frame(&mut self) -> Option<Frame> {
        let cursor = self.cursor();
        let cursor = Cursor {
            row: cursor.row as u16,
            col: cursor.col as u16,
            shape: self.shape,
            blink: self.blink,
            visible: self.cursor_visible,
        };
        let state = (cursor, self.modes(), self.keyboard(), self.history.range());
        if !self.damage.any() && self.sent == Some(state) {
            return None;
        }

        let all = self.damage.all;
        let lines = (0..self.rows)
            .filter(|&r| all || self.damage.rows[r])
            .map(|r| (r as u16, self.screen().rows[r].to_line()))
            .collect();
        let frame = Frame {
            cols: self.cols as u16,
            rows: self.rows as u16,
            full: self.damage.reset,
            scroll: if all { 0 } else { self.damage.scroll as u16 },
            lines,
            cursor,
            modes: state.1,
            keyboard: state.2,
            history: state.3,
        };
        self.damage.clear(self.rows);
        self.sent = Some(state);
        Some(frame)
    }
}

/// 把字符映射为 DEC 制图字符集（`ESC ( 0`）中对应的框线字符，无对应字符时原样返回。
fn dec_graphics(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

#[cfg(test)]
mod tests;
