//! 主程序和辅助进程之间的协议：stdio 上的二进制分帧，小端；长度不含自己，含类型字节。
//!
//! ```text
//! [u32 长度][u8 类型][载荷……]
//! ```
//!
//! 辅助进程启动后先发 [`FromHelper::Hello`]（带 [`MAGIC`] 和 [`VERSION`]），主程序核对不上时不再
//! 继续发送。路径、环境变量和输入按原样的字节传输（不一定是 UTF-8），文字字段是 UTF-8。

mod codec;
mod wire;

use wire::*;

use std::fmt;

/// 握手时核对的魔数。
pub const MAGIC: [u8; 8] = *b"winc-pty";
/// 协议版本号。
///
/// 消息的格式每变化一次加一。
pub const VERSION: u32 = 6;
/// 一条消息的长度上限（字节，64 MiB）。
///
/// 超过上限的消息视为对方出错并断开连接，避免一个错误的长度使对端分配数 GB 内存。
/// 经验值：取 64 MiB 是为了容纳整屏加回滚缓冲的分段内容（见 [`FromHelper::History`]）。
const MAX_MESSAGE: usize = 64 << 20;
/// 收发缓冲空闲时保留的最大容量（字节，1 MiB）。
///
/// 一次回滚缓冲的回答可达数 MB，之后的消息通常只有几 KB；缓冲用空时容量超过该值即收缩到该值，
/// 两个进程都不再长期占用峰值容量。经验值：平时的帧和输入都远小于该值，不会反复收缩、扩张。
pub const IDLE_BUFFER: usize = 1 << 20;

/// 颜色：默认色、256 色表里的一项（0–15 是主题的 16 色），或真彩色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    /// 前景或背景的默认色，由主题决定。
    #[default]
    Default,
    /// 256 色表里的一项。
    Indexed(u8),
    /// 真彩色。
    Rgb(u8, u8, u8),
}

/// 单元格的属性位，下划线样式占第 12–14 位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attrs(pub u16);

impl Attrs {
    /// 粗体。
    pub const BOLD: u16 = 1 << 0;
    /// 淡色。
    pub const DIM: u16 = 1 << 1;
    /// 斜体。
    pub const ITALIC: u16 = 1 << 2;
    /// 闪烁（只记下来，画不画由主程序定）。
    pub const BLINK: u16 = 1 << 3;
    /// 前景背景对调。
    pub const INVERSE: u16 = 1 << 4;
    /// 隐藏（画成背景色）。
    pub const HIDDEN: u16 = 1 << 5;
    /// 删除线。
    pub const STRIKE: u16 = 1 << 6;
    /// 宽字符因放不下而换到下一行时，留在该行末尾的填充空格。
    ///
    /// 重排时移除。
    pub const FILLER: u16 = 1 << 7;
    const UNDERLINE_SHIFT: u16 = 12;
    const UNDERLINE_MASK: u16 = 0b111 << Self::UNDERLINE_SHIFT;

    /// 是否带有 `bit` 里的所有位。
    pub fn has(self, bit: u16) -> bool {
        self.0 & bit == bit
    }

    /// 设置或清除 `bit`。
    pub fn set(&mut self, bit: u16, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    /// 下划线样式。
    pub fn underline(self) -> Underline {
        match (self.0 & Self::UNDERLINE_MASK) >> Self::UNDERLINE_SHIFT {
            1 => Underline::Single,
            2 => Underline::Double,
            3 => Underline::Curly,
            4 => Underline::Dotted,
            5 => Underline::Dashed,
            _ => Underline::None,
        }
    }

    /// 设置下划线样式。
    pub fn set_underline(&mut self, underline: Underline) {
        self.0 = (self.0 & !Self::UNDERLINE_MASK) | ((underline as u16) << Self::UNDERLINE_SHIFT);
    }
}

/// 下划线样式（SGR `4:n`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u16)]
pub enum Underline {
    /// 没有下划线。
    #[default]
    None = 0,
    /// 单线。
    Single = 1,
    /// 双线。
    Double = 2,
    /// 波浪线（诊断信息常用）。
    Curly = 3,
    /// 点线。
    Dotted = 4,
    /// 虚线。
    Dashed = 5,
}

/// 一段文字的样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    /// 前景色。
    pub fg: Color,
    /// 背景色。
    pub bg: Color,
    /// 下划线颜色（SGR 58）；`Default` 表示与前景色相同。
    pub underline_color: Color,
    /// 属性位和下划线样式。
    pub attrs: Attrs,
    /// 超链接编号（OSC 8），0 表示没有。
    ///
    /// 链接地址见 [`Event::Link`]。
    pub link: u16,
}

/// 一行中从第 `start` 格开始的样式段，延伸到下一个 `Run` 之前。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// 从第几格开始（格从 0 起，不是第几个字）。
    pub start: u16,
    /// 样式。
    pub style: Style,
}

/// shell 集成（OSC 133）记在行上的标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Marks {
    /// [`Marks::PROMPT`]、[`Marks::COMMAND`]、[`Marks::OUTPUT`]、[`Marks::DONE`] 的组合。
    pub flags: u8,
    /// 带 [`Marks::DONE`] 时上一条命令的退出码。
    pub exit: i32,
    /// 带 [`Marks::PROMPT`] 时这个提示符开始的命令块的编号（见 [`Block::id`]）。
    pub block: u32,
}

impl Marks {
    /// 提示符从这一行开始（`133;A`）。
    pub const PROMPT: u8 = 1 << 0;
    /// 用户输入的命令从这一行开始（`133;B`）。
    pub const COMMAND: u8 = 1 << 1;
    /// 命令的输出从这一行开始（`133;C`）。
    pub const OUTPUT: u8 = 1 << 2;
    /// 上一条命令在这一行结束（`133;D`），退出码在 [`Marks::exit`]。
    pub const DONE: u8 = 1 << 3;

    /// 是否没有任何标记。
    pub fn is_empty(self) -> bool {
        self.flags == 0
    }
}

/// 一行：文字和按格计量的样式段。
///
/// 宽字符的后半格和组合字符都不单独出现在 `text` 中：宽字符占两格，组合字符跟在前一个字符之后
/// 且不占格（宽度见 [`crate::term::width::char_width`]）。非软换行的行去掉行尾默认样式的空格，
/// 软换行的行保留全部格子，重排时逐格拼回。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    /// 这一行的文字。
    pub text: String,
    /// 样式段，按 `start` 递增；第一段从 0 开始。
    ///
    /// 空行可以没有。
    pub runs: Vec<Run>,
    /// 这一行写满后自动折到了下一行（软换行），重排时和下一行连起来。
    pub wrapped: bool,
    /// shell 集成的标记。
    pub marks: Marks,
}

impl Line {
    /// 返回这一行编进消息后占用的字节数，[`FromHelper::History`] 按它分段。
    pub fn encoded_len(&self) -> usize {
        let color = |c: Color| match c {
            Color::Default => 1,
            Color::Indexed(_) => 2,
            Color::Rgb(..) => 4,
        };
        let runs: usize = self
            .runs
            .iter()
            .map(|r| 6 + color(r.style.fg) + color(r.style.bg) + color(r.style.underline_color))
            .sum();
        // 字段顺序与 `Writer::line` 一一对应：4 字节文字长度 + 文字 + 4 字节样式段个数 +
        // 样式段 + 1 字节软换行标记 + 1 字节标记 + 4 字节退出码 + 4 字节块号。
        4 + self.text.len() + 4 + runs + 1 + 1 + 4 + 4
    }

    /// 返回这一行的纯文字，去掉行尾的空格（用于测试和复制）。
    pub fn trimmed(&self) -> &str {
        self.text.trim_end_matches(' ')
    }
}

/// 光标的形状（DECSCUSR）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// 方块。
    #[default]
    Block,
    /// 下划线。
    Underline,
    /// 竖线。
    Bar,
}

/// 光标位置与形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    /// 屏幕上第几行（从 0 起）。
    pub row: u16,
    /// 第几格（从 0 起，按格计）。
    pub col: u16,
    /// 形状。
    pub shape: CursorShape,
    /// 程序要求闪烁。
    pub blink: bool,
    /// 程序要求显示（DECTCEM）。
    pub visible: bool,
}

/// 程序打开的模式集合，主程序按它们编码按键、鼠标和粘贴。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes(pub u32);

impl Modes {
    /// 方向键发应用模式的序列（DECCKM）。
    pub const APP_CURSOR: u32 = 1 << 0;
    /// 小键盘发应用模式的序列（DECKPAM）。
    pub const APP_KEYPAD: u32 = 1 << 1;
    /// 粘贴时包上 `ESC[200~`、`ESC[201~`（2004）。
    pub const BRACKETED_PASTE: u32 = 1 << 2;
    /// 焦点变化时发 `ESC[I`、`ESC[O`（1004）。
    pub const FOCUS: u32 = 1 << 3;
    /// 上报鼠标按下和松开（1000）。
    pub const MOUSE_PRESS: u32 = 1 << 4;
    /// 上报按住拖动时的鼠标移动（1002）。
    pub const MOUSE_DRAG: u32 = 1 << 5;
    /// 上报所有鼠标移动，不论是否有按键按下（1003）。
    pub const MOUSE_MOTION: u32 = 1 << 6;
    /// 鼠标用 SGR 编码（1006）。
    pub const MOUSE_SGR: u32 = 1 << 7;
    /// 备用屏上把滚轮转换成方向键（1007，默认开启）。
    pub const ALT_SCROLL: u32 = 1 << 8;
    /// 正在备用屏上。
    pub const ALT_SCREEN: u32 = 1 << 9;
    /// 整屏反色（DECSCNM）。
    pub const REVERSE: u32 = 1 << 10;
    /// 回车发 CR LF（LNM）。
    pub const NEWLINE: u32 = 1 << 11;

    /// 是否带有 `bit`。
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit == bit
    }

    /// 设置或清除 `bit`。
    pub fn set(&mut self, bit: u32, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

/// 一帧：上一帧以来屏幕的变化。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frame {
    /// 列数。
    pub cols: u16,
    /// 行数。
    pub rows: u16,
    /// 主程序是否丢弃该标签缓存的全部行，`lines` 中给出整屏（尺寸变化、重排或清空回滚后为真）。
    pub full: bool,
    /// 先把整屏上移指定的行数（底部空出的行在 `lines` 中），再换上 `lines`。
    ///
    /// 滚动一行时只发送新出现的一行。
    pub scroll: u16,
    /// 变化的行：屏幕上的行号和内容。
    pub lines: Vec<(u16, Line)>,
    /// 光标位置与形状。
    pub cursor: Cursor,
    /// 模式。
    pub modes: Modes,
    /// Kitty 键盘协议当前的标志位（0 表示按 xterm 规则编码）。
    pub keyboard: u8,
    /// 回滚缓冲里现有的行：绝对行号 `history.0..history.1`，按 [`ToHelper::History`] 取。
    pub history: (u64, u64),
}

/// 命令块某一段开始的位置（OSC 133）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandMark {
    /// 提示符开始（`A`）。
    Prompt,
    /// 命令开始（`B`），用户正在输入。
    Command,
    /// 命令开始运行，后面是输出（`C`）。
    Output,
    /// 命令结束（`D`），带退出码（shell 未提供时为 `None`）。
    Done(Option<i32>),
}

/// 终端中发生、不属于屏幕内容的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 窗口标题（OSC 0、2）。
    Title(String),
    /// 当前目录（OSC 7），已去掉 `file://主机` 并解开百分号转义。
    Cwd(Vec<u8>),
    /// 响铃。
    Bell,
    /// 程序请求发送系统通知（OSC 9、777）。
    Notify {
        /// 标题，可能为空。
        title: String,
        /// 正文。
        body: String,
    },
    /// 程序请求写入剪贴板（OSC 52，已解开 base64）。
    ///
    /// 读剪贴板的请求一律不回答。
    Clipboard(String),
    /// 超链接编号对应的地址（OSC 8）；同一编号只发送一次。
    Link(u16, String),
    /// shell 集成的命令边界标记。
    Command(CommandMark),
}

/// 一个命令块：提示符、用户输入的命令和命令输出。
///
/// 行号都是绝对行号，与 [`Frame::history`] 的计数方式相同。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// 编号，同一终端内不重复，重排后也不变（主程序按它记录哪些块已折叠）。
    pub id: u32,
    /// 提示符的第一行。
    pub prompt: u64,
    /// 用户输入命令的那一行；shell 未报告 `B` 时取输出开始的前一行，即多行提示符的最后一行。
    pub command: u64,
    /// 输出的第一行；尚未回车运行时为 `None`。
    pub output: Option<u64>,
    /// 块的结束行：下一个提示符所在行，或命令结束标记（`D`）所在行；仍在运行时为 `None`。
    pub end: Option<u64>,
    /// 退出码；命令尚未结束，或者未运行即被下一个提示符取代时为 `None`。
    pub exit: Option<i32>,
    /// 运行时长（毫秒），辅助进程从开始运行量到命令结束。
    pub duration: Option<u32>,
    /// shell 报告的命令行（`cmdline_url=` 或 `cmdline=`），未报告时为空。
    pub text: String,
    /// 命令开始运行时的当前目录（最近一次 OSC 7），从未报告时为空。
    pub cwd: Vec<u8>,
}

impl Block {
    /// 命令是否正在运行：已经回车且尚未结束。
    pub fn running(&self) -> bool {
        self.output.is_some() && self.end.is_none()
    }

    /// 返回编码进消息后占用的字节数，用于限制 [`FromHelper::Blocks`] 的大小。
    pub fn encoded_len(&self) -> usize {
        // 字段顺序与 `Writer::block` 一一对应：4 字节编号 + 8 字节提示符 + 8 字节命令行 +
        // 9 字节输出 + 9 字节结束 + 1+4 字节退出码 + 1+4 字节时长 + 4 字节加文字 + 4 字节加目录。
        4 + 8 + 8 + 9 + 9 + 1 + 4 + 1 + 4 + 4 + self.text.len() + 4 + self.cwd.len()
    }
}

/// 新建终端时的尺寸，以及之后的尺寸变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    /// 列数。
    pub cols: u16,
    /// 行数。
    pub rows: u16,
    /// 网格区域的宽（物理像素，程序查询窗口像素尺寸时回答它）。
    pub width: u16,
    /// 网格区域的高（物理像素，程序查询窗口像素尺寸时回答它）。
    pub height: u16,
}

/// 主题的颜色，用于回答程序对颜色的查询（OSC 10、11、12、4）。
///
/// 程序根据这些颜色判断界面是深色还是浅色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Theme {
    /// 默认前景。
    pub fg: [u8; 3],
    /// 默认背景。
    pub bg: [u8; 3],
    /// 光标。
    pub cursor: [u8; 3],
    /// 16 色。
    pub palette: [[u8; 3]; 16],
}

impl Theme {
    /// 主题是否是深色：背景的相对亮度低于一半（亮度权重依据 ITU-R BT.601）。
    pub fn is_dark(&self) -> bool {
        let [r, g, b] = self.bg.map(u32::from);
        299 * r + 587 * g + 114 * b < 500 * 255
    }

    /// 返回 256 色表中第 `index` 个颜色：0–15 取主题色，16–231 是 6×6×6 色块，232–255 是灰阶
    /// （依据 xterm 的色表规则）。
    pub fn color(&self, index: u8) -> [u8; 3] {
        match index {
            0..=15 => self.palette[usize::from(index)],
            16..=231 => {
                let i = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                [level(i / 36), level(i / 6 % 6), level(i % 6)]
            }
            _ => {
                let v = 8 + 10 * (index - 232);
                [v, v, v]
            }
        }
    }
}

/// 主程序发给辅助进程的消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToHelper {
    /// 新建一个终端。
    Spawn {
        /// 标签号，由主程序分配，不复用。
        tab: u16,
        /// 要运行的程序；为 `None` 时依次取环境变量 `SHELL`、用户数据库中的登录 shell，最后用
        /// `/bin/sh`。
        program: Option<Vec<u8>>,
        /// 参数（不含程序名）。
        args: Vec<Vec<u8>>,
        /// 是否作为登录 shell 启动（程序名前加 `-`）。
        login: bool,
        /// 工作目录。
        cwd: Vec<u8>,
        /// 环境变量。
        ///
        /// 辅助进程在该环境基础上设置 `TERM` 等若干变量。
        env: Vec<(Vec<u8>, Vec<u8>)>,
        /// 回滚缓冲最多多少行。
        scrollback: u32,
        /// 尺寸。
        size: Size,
        /// shell 集成脚本所在的目录；为 `None` 时不注入。
        integration: Option<Vec<u8>>,
        /// 程序启动前先写进终端的内容，不经过程序（运行配置的命令行）；为空时不写。
        ///
        /// 不为空时，程序退出后辅助进程在终端里写入命令结束标记（OSC 133 `D`，带退出码），
        /// `banner` 中开始的命令块随之结束。
        banner: Vec<u8>,
    },
    /// 写进伪终端的字节（按键已经编码好）。
    Input {
        /// 标签号。
        tab: u16,
        /// 字节。
        bytes: Vec<u8>,
        /// 同一次输入（大段粘贴）分多条发送时，该条之后剩余的字节数；最后一条为 0。
        ///
        /// 辅助进程按整次输入决定是否接受，不会只丢弃中间的几条。
        rest: u64,
    },
    /// 尺寸变了。
    Resize {
        /// 标签号。
        tab: u16,
        /// 新尺寸。
        size: Size,
    },
    /// 主程序已完成绘制，可以再发送一帧。
    Credit {
        /// 标签号。
        tab: u16,
    },
    /// 请求回滚缓冲中绝对行号 `start..end` 的行。
    History {
        /// 标签号。
        tab: u16,
        /// 请求编号，回答时原样带回。
        id: u32,
        /// 起始行号（含，绝对行号）。
        start: u64,
        /// 结束行号（不含，绝对行号）。
        end: u64,
    },
    /// 清空屏幕和回滚缓冲（⌘K），保留光标所在行（一般是提示符）并移到第一行。
    Clear {
        /// 标签号。
        tab: u16,
    },
    /// 关闭一个终端：挂断它的进程组。
    Close {
        /// 标签号。
        tab: u16,
    },
    /// 主题变化。
    Theme(Theme),
    /// 结束终端里的程序但保留终端：挂断它的进程组，到期仍未退出时强制结束。
    ///
    /// 内容和回滚缓冲保留到 [`ToHelper::Close`]，退出时照常发 [`FromHelper::Exited`]。
    Stop {
        /// 标签号。
        tab: u16,
    },
}

/// 辅助进程发给主程序的消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromHelper {
    /// 握手，辅助进程启动后的第一条消息。
    Hello {
        /// 魔数，应等于 [`MAGIC`]。
        magic: [u8; 8],
        /// 协议版本，应等于 [`VERSION`]。
        version: u32,
        /// 辅助进程自己的版本号，报错时给用户看。
        build: String,
    },
    /// 屏幕内容变化。
    Frame {
        /// 标签号。
        tab: u16,
        /// 变化。
        frame: Frame,
    },
    /// 对 [`ToHelper::History`] 的回答。
    ///
    /// 已经丢弃的行不包含在内，`start` 是实际第一行的行号。行多或行宽（真彩色、叠加符号）时
    /// 分多条发送，避免单条超过消息上限；除最后一条外 `more` 为真。
    History {
        /// 标签号。
        tab: u16,
        /// 请求编号。
        id: u32,
        /// 第一行的绝对行号。
        start: u64,
        /// 行。
        lines: Vec<Line>,
        /// 同一请求是否还有后续的行。
        more: bool,
    },
    /// 终端事件。
    Event {
        /// 标签号。
        tab: u16,
        /// 事件。
        event: Event,
    },
    /// 命令块变化：全部的块，按位置从上到下。
    ///
    /// 与帧不同，该消息不等待额度，变化时立即发送：后台标签的命令结束时也必须获知。
    Blocks {
        /// 标签号。
        tab: u16,
        /// 块。
        blocks: Vec<Block>,
        /// 行号是否按尚未发出的那一帧（`Frame::full`：重排、清空回滚之后）计算：主程序据此等
        /// 那一帧到达后再更新命令栏和色带，避免闪烁。
        early: bool,
    },
    /// 前台进程变化（标签名取自它）。
    Process {
        /// 标签号。
        tab: u16,
        /// 进程名。
        name: String,
    },
    /// shell 已退出。
    ///
    /// 终端的内容仍然保留，可以继续获取回滚，直到主程序发送 [`ToHelper::Close`]。
    Exited {
        /// 标签号。
        tab: u16,
        /// 退出码；被信号终止时为 128 加信号编号。
        code: i32,
    },
    /// 伪终端断开了，shell 还在运行：输入到不了程序，也不会再有新的输出。
    ///
    /// 终端的内容仍然保留，可以继续获取回滚。其中的进程不会自动结束，直到主程序发送
    /// [`ToHelper::Close`]；shell 在此之前退出时随后还会发 [`FromHelper::Exited`]。
    Disconnected {
        /// 标签号。
        tab: u16,
    },
    /// 新建失败（无法打开伪终端、程序不存在等）。
    Failed {
        /// 标签号。
        tab: u16,
        /// 原因，给用户看。
        message: String,
    },
}

/// 解码失败：对方发来的数据不合法，只能断开连接。
///
/// 字段是给用户看的简短原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "终端协议数据有误：{}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// 协议中的一种消息。
pub trait Message: Sized {
    /// 把该消息编码成一条完整消息（带长度和类型），追加到 `out` 末尾。
    fn encode(&self, out: &mut Vec<u8>);

    /// 从一条消息的内容（类型字节加载荷，不含长度字段）解码。
    ///
    /// # Errors
    ///
    /// 类型未知、数据被截断、字段值不合法或者末尾有多余字节时返回 [`DecodeError`]。
    fn decode(data: &[u8]) -> Result<Self, DecodeError>;
}

/// 把收到的字节流拼成完整的消息。
#[derive(Debug, Default)]
pub struct Decoder {
    /// 收到的字节。
    buf: Vec<u8>,
    /// `buf` 中已经取走的字节数（下一条消息从这里开始）。
    start: usize,
}

impl Decoder {
    /// 追加读到的字节。
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// 取出下一条完整的消息；字节不足一条时返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// 长度为 0、超过单条消息长度上限（`MAX_MESSAGE`）或者消息解码失败时返回
    /// [`DecodeError`]，之后不应再使用该解码器。
    pub fn next_message<M: Message>(&mut self) -> Result<Option<M>, DecodeError> {
        let rest = &self.buf[self.start..];
        let Some(head) = rest.first_chunk::<4>() else {
            return Ok(None);
        };
        let len = u32::from_le_bytes(*head) as usize;

        if len == 0 || len > MAX_MESSAGE {
            return Err(DecodeError("消息长度无效"));
        }
        let Some(body) = rest.get(4..4 + len) else {
            return Ok(None);
        };
        let message = M::decode(body)?;
        self.start += 4 + len;

        // 已取走的部分超过一半时把剩余字节挪到开头，避免缓冲持续增长。
        if self.start > self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
            if self.buf.is_empty() {
                self.buf.shrink_to(IDLE_BUFFER);
            }
        }
        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests;
