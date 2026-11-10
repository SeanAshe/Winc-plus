//! `vte` 切分出的字符、控制字符和序列的处理：C0 控制字符、ESC、CSI、OSC、DCS。
//!
//! 序列的含义按 xterm 控制序列实现，不认识的序列一律忽略而不报错，程序输出不会使终端出错。

use std::fmt::Write as _;

use vte::{Params, Perform};

use super::{Dcs, Emulator, MAX_LINK_BYTES, Pos};
use crate::protocol::{Attrs, Color, CommandMark, CursorShape, Event, Marks, Modes, Underline};

/// Kitty 键盘协议支持的标志位，只包含第 1 级「消除歧义」。
const KEYBOARD_SUPPORTED: u8 = 0b1;
/// Kitty 键盘协议标志位栈的深度上限（层）。
///
/// 超过时丢弃最底下的层。程序嵌套推入数十层没有实际用途，16 层已覆盖实际用法，并限制栈的
/// 内存占用。
const KEYBOARD_STACK: usize = 16;
/// DCS 序列记录的字节上限（字节，1 KiB）。
///
/// XTGETTCAP、DECRQSS 的请求都很短，1 KiB 足以覆盖。
const MAX_DCS: usize = 1024;
/// OSC 52 写剪贴板的上限（解码前的字节，1 MiB）。
///
/// 覆盖复制大段文本的常见情况；超出时丢弃请求，避免为一条序列分配大块内存。
const MAX_CLIPBOARD: usize = 1 << 20;
/// 标题保留的最大字符数（字符，按码点个数计）。
///
/// 超出的部分截掉：标签栏一项无法完整显示更长的标题，该上限同时限制程序刷出的超长标题。
const MAX_TITLE: usize = 1024;
/// 通知（OSC 9、777）标题与正文各自保留的最大字符数（字符，按码点个数计）。
///
/// 通知只显示一两行，与标题用同一上限；超长正文不会整段留在内存里，也不会整段发给主程序。
const MAX_NOTIFICATION: usize = 1024;
/// 单个超链接地址的上限（字节，2 KiB）。
///
/// 更长的地址不当作链接，避免为每个地址记录并发送一条事件。
const MAX_LINK: usize = 2048;

/// CSI 序列的参数，每个参数只取第一个子参数。
///
/// 子参数只有 SGR 使用，单独处理。最多读取 16 个参数，多出的参数直接忽略。
struct Args {
    /// 各参数的第一个子参数，下标不小于 `len` 的项没有意义。
    values: [u16; 16],
    /// 实际的参数个数，不超过 16。
    len: usize,
}

impl Args {
    fn new(params: &Params) -> Self {
        let mut args = Args {
            values: [0; 16],
            len: 0,
        };
        for param in params.iter().take(16) {
            args.values[args.len] = param[0];
            args.len += 1;
        }
        args
    }

    /// 返回第 `i` 个参数，参数不存在或为 0 时返回 `default`。
    fn get(&self, i: usize, default: u16) -> u16 {
        match self.values[..self.len].get(i) {
            Some(&v) if v != 0 => v,
            _ => default,
        }
    }

    /// 返回第 `i` 个参数作为次数或位置的取值，最小为 1。
    fn count(&self, i: usize) -> usize {
        usize::from(self.get(i, 1))
    }

    fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.values[..self.len].iter().copied()
    }
}

impl Perform for Emulator {
    fn print(&mut self, c: char) {
        self.history_erased = false;
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        self.cluster = super::width::Cluster::None;
        match byte {
            0x07 => self.events.push(Event::Bell),
            0x08 => {
                let cursor = self.cursor();
                self.set_cursor(cursor.row, cursor.col.saturating_sub(1));
            }
            0x09 => self.tab(1),
            0x0a..=0x0c => {
                self.linefeed();
                if self.modes.has(Modes::NEWLINE) {
                    let row = self.cursor().row;
                    self.set_cursor(row, 0);
                }
            }
            0x0d => {
                let row = self.cursor().row;
                self.set_cursor(row, 0);
            }
            0x0e => self.shift_out = true,
            0x0f => self.shift_out = false,
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.cluster = super::width::Cluster::None;
        if ignore {
            return;
        }
        let args = Args::new(params);
        let cursor = self.cursor();

        match (intermediates, action) {
            ([], '@') => {
                let bg = self.pen.bg;
                self.screen_mut().rows[cursor.row].insert_blanks(cursor.col, args.count(0), bg);
                self.damage.row(cursor.row);
            }
            ([], 'A') => self.move_up(args.count(0)),
            ([], 'B' | 'e') => self.move_down(args.count(0)),
            ([], 'C' | 'a') => self.set_cursor(cursor.row, cursor.col + args.count(0)),
            ([], 'D') => self.set_cursor(cursor.row, cursor.col.saturating_sub(args.count(0))),
            ([], 'E') => {
                self.move_down(args.count(0));
                let row = self.cursor().row;
                self.set_cursor(row, 0);
            }
            ([], 'F') => {
                self.move_up(args.count(0));
                let row = self.cursor().row;
                self.set_cursor(row, 0);
            }
            ([], 'G' | '`') => self.set_cursor(cursor.row, args.count(0) - 1),
            ([], 'H' | 'f') => self.goto(args.count(0) - 1, args.count(1) - 1),
            ([], 'd') => self.goto_row(args.count(0) - 1),
            ([], 'I') => self.tab(args.count(0)),
            ([], 'Z') => self.back_tab(args.count(0)),
            // 带 `?` 的选择性擦除按整块擦除处理。
            ([] | [b'?'], 'J') => self.erase_display(args.get(0, 0)),
            ([] | [b'?'], 'K') => self.erase_line(args.get(0, 0)),
            ([], 'L') => self.insert_lines(args.count(0)),
            ([], 'M') => self.delete_lines(args.count(0)),
            ([], 'P') => {
                let bg = self.pen.bg;
                self.screen_mut().rows[cursor.row].delete_chars(cursor.col, args.count(0), bg);
                self.damage.row(cursor.row);
            }
            ([], 'X') => {
                self.erase_cells(cursor.row, cursor.col, cursor.col + args.count(0));
            }
            ([], 'S') => self.scroll_up(args.count(0)),
            // 带多个参数的 `T` 是旧的鼠标高亮跟踪，不是下滚。
            ([], 'T') if args.len <= 1 => self.scroll_down(args.count(0)),
            ([], 'b') => {
                // 重复最后一个字符（REP），次数不超过整屏格数，避免一次请求输出过多内容。
                if let Some(c) = self.last_char {
                    for _ in 0..args.count(0).min(self.cols * self.rows) {
                        self.write_char(c);
                    }
                }
            }
            // 主设备属性（DA1）：报告 VT220（62），支持 ANSI 颜色（22）。
            ([], 'c') if args.get(0, 0) == 0 => self.respond("\x1b[?62;22c"),
            // 次设备属性（DA2）：终端类型 1（VT220）、版本 10、没有 ROM 卡带（0）。
            ([b'>'], 'c') if args.get(0, 0) == 0 => self.respond("\x1b[>1;10;0c"),
            ([], 'g') => match args.get(0, 0) {
                0 => {
                    if let Some(tab) = self.tabs.get_mut(cursor.col) {
                        *tab = false;
                    }
                }
                3 => self.tabs.fill(false),
                _ => {}
            },
            ([], 'h' | 'l') => {
                for mode in args.iter() {
                    self.set_ansi_mode(mode, action == 'h');
                }
            }
            ([b'?'], 'h' | 'l') => {
                for mode in args.iter() {
                    self.set_private_mode(mode, action == 'h');
                }
            }
            ([], 'm') => self.sgr(params),
            // 设备状态报告（DSR）：5 表示就绪，6 请求光标位置。
            // 明暗主题查询：997 回答，1 为深色，2 为浅色。
            ([b'?'], 'n') if args.get(0, 0) == 996 => self.report_color_scheme(),
            ([], 'n') => match args.get(0, 0) {
                5 => self.respond("\x1b[0n"),
                6 => {
                    let (row, col) = self.report_position();
                    self.respond(&format!("\x1b[{row};{col}R"));
                }
                _ => {}
            },
            // 带 `?` 的私有光标位置报告（DECXCPR）。
            ([b'?'], 'n') if args.get(0, 0) == 6 => {
                let (row, col) = self.report_position();
                self.respond(&format!("\x1b[?{row};{col}R"));
            }
            ([b'?', b'$'], 'p') => {
                let mode = args.get(0, 0);
                let state = self.private_mode_state(mode);
                self.respond(&format!("\x1b[?{mode};{state}$y"));
            }
            ([b'$'], 'p') => {
                let mode = args.get(0, 0);
                let state = match mode {
                    4 => on_off(self.insert),
                    20 => on_off(self.modes.has(Modes::NEWLINE)),
                    _ => 0,
                };
                self.respond(&format!("\x1b[{mode};{state}$y"));
            }
            ([b'!'], 'p') => self.soft_reset(),
            ([b' '], 'q') => {
                let style = args.get(0, 0);
                self.shape = match style {
                    3 | 4 => CursorShape::Underline,
                    5 | 6 => CursorShape::Bar,
                    _ => CursorShape::Block,
                };
                self.blink = matches!(style, 0 | 1 | 3 | 5);
            }
            // 终端名称与版本报告（XTVERSION）。
            ([b'>'], 'q') => {
                self.respond(&format!("\x1bP>|winc {}\x1b\\", winc_version::VERSION));
            }
            // 设置滚动区域（DECSTBM），参数行号从 1 起。
            ([], 'r') => {
                let top = args.count(0) - 1;
                let bottom = usize::from(args.get(1, self.rows as u16)).min(self.rows);
                if top + 1 < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.goto(0, 0);
                }
            }
            ([], 's') => self.save_cursor(),
            ([], 'u') => self.restore_cursor(),
            ([b'?'], 'u') => {
                let flags = self.keyboard();
                self.respond(&format!("\x1b[?{flags}u"));
            }
            ([b'>'], 'u') => {
                // Kitty 键盘协议：把当前标志位压入栈顶。
                let flags = args.get(0, 0) as u8 & KEYBOARD_SUPPORTED;
                let stack = &mut self.screen_mut().keyboard;
                if stack.len() >= KEYBOARD_STACK {
                    stack.remove(0);
                }
                stack.push(flags);
            }
            // Kitty 键盘协议：从栈顶弹出 `n` 层标志位。
            ([b'<'], 'u') => {
                let stack = &mut self.screen_mut().keyboard;
                let n = args.count(0).min(stack.len());
                stack.truncate(stack.len() - n);
            }
            // Kitty 键盘协议：设置当前标志位，或按修饰位增减标志位。
            ([b'='], 'u') => {
                let flags = args.get(0, 0) as u8 & KEYBOARD_SUPPORTED;
                let stack = &mut self.screen_mut().keyboard;
                let current = stack.last().copied().unwrap_or(0);
                let next = match args.get(1, 1) {
                    2 => current | flags,
                    3 => current & !flags,
                    _ => flags,
                };
                match stack.last_mut() {
                    Some(top) => *top = next,
                    None => stack.push(next),
                }
            }
            ([], 't') => self.window_report(args.get(0, 0)),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        self.cluster = super::width::Cluster::None;
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.linefeed();
                let row = self.cursor().row;
                self.set_cursor(row, 0);
            }
            ([], b'H') => {
                let col = self.cursor().col;
                if let Some(tab) = self.tabs.get_mut(col) {
                    *tab = true;
                }
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.hard_reset(),
            ([], b'=') => self.modes.set(Modes::APP_KEYPAD, true),
            ([], b'>') => self.modes.set(Modes::APP_KEYPAD, false),
            ([b'('], set) => self.charsets[0] = set == b'0',
            ([b')'], set) => self.charsets[1] = set == b'0',
            ([b'#'], b'8') => {
                // 整屏填充 E 用于校准（DECALN），并把滚动区域复原、光标移回左上角。
                let pen = crate::protocol::Style::default();
                for r in 0..self.rows {
                    for c in 0..self.cols {
                        self.screen_mut().rows[r].write(c, 'E', &pen, false);
                    }
                }
                self.top = 0;
                self.bottom = self.rows;
                self.origin = false;
                self.screen_mut().cursor = Pos::default();
                self.damage.all();
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        let Some(&command) = params.first() else {
            return;
        };
        // 回答的结束符通常和查询相同。Windows 上回答经 ConPTY 到达程序时可能被分块；程序把分块后的
        // 尾部当作键盘输入时，BEL 等于按下 Ctrl+G，因此一律用 ST。
        let end = if bell_terminated && !cfg!(windows) {
            "\x07"
        } else {
            "\x1b\\"
        };

        match command {
            b"0" | b"2" => {
                let mut title = join(&params[1..]);
                if let Some((cut, _)) = title.char_indices().nth(MAX_TITLE) {
                    title.truncate(cut);
                }
                // 程序在循环中改标题时只保留最后一个，事件不会持续累积。
                match self.events.last_mut() {
                    Some(Event::Title(last)) => *last = title,
                    _ => self.events.push(Event::Title(title)),
                }
            }
            b"7" => {
                if let Some(path) = params.get(1).and_then(|url| file_url_path(url)) {
                    self.blocks.cwd = path.as_slice().into();
                    self.events.push(Event::Cwd(path));
                }
            }
            b"8" => {
                let uri = join(params.get(2..).unwrap_or_default());
                self.pen.link = self.link_id(uri);
            }
            b"52" => {
                let Some(&data) = params.get(2) else {
                    return;
                };
                // 「?」表示读取剪贴板，终端不回答，程序无法读取用户的剪贴板。
                if data != b"?"
                    && data.len() <= MAX_CLIPBOARD
                    && let Some(text) = base64(data).and_then(|b| String::from_utf8(b).ok())
                {
                    self.events.push(Event::Clipboard(text));
                }
            }
            b"133" => self.shell_mark(params),
            b"9" => {
                // `9;4;…` 是任务栏进度条扩展，不是通知。
                if params.get(1).is_some_and(|p| *p != b"4") {
                    let body = join(&params[1..]);
                    self.notify(String::new(), body);
                }
            }
            b"777" if params.get(1) == Some(&&b"notify"[..]) => {
                let title = params.get(2).map(|t| lossy(t)).unwrap_or_default();
                let body = join(params.get(3..).unwrap_or_default());
                self.notify(title, body);
            }
            b"10" | b"11" | b"12" if params.get(1) == Some(&&b"?"[..]) => {
                let Some(theme) = self.theme else {
                    return;
                };
                let color = match command {
                    b"10" => theme.fg,
                    b"11" => theme.bg,
                    _ => theme.cursor,
                };
                let which = lossy(command);
                self.respond(&format!("\x1b]{which};{}{end}", rgb_spec(color)));
            }
            b"4" => {
                let Some(theme) = self.theme else {
                    return;
                };
                for pair in params[1..].as_chunks::<2>().0 {
                    if pair[1] != b"?" {
                        continue;
                    }
                    let Some(index) = lossy(pair[0]).parse::<u8>().ok() else {
                        continue;
                    };
                    let color = theme.color(index);
                    self.respond(&format!("\x1b]4;{index};{}{end}", rgb_spec(color)));
                }
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.dcs = match (intermediates, action) {
            _ if ignore => None,
            ([b'+'], 'q') => Some(Dcs::Termcap(Vec::new())),
            ([b'$'], 'q') => Some(Dcs::Setting(Vec::new())),
            _ => None,
        };
    }

    fn put(&mut self, byte: u8) {
        if let Some(Dcs::Termcap(data) | Dcs::Setting(data)) = &mut self.dcs
            && data.len() < MAX_DCS
        {
            data.push(byte);
        }
    }

    fn unhook(&mut self) {
        match self.dcs.take() {
            Some(Dcs::Termcap(data)) => {
                // 不回答具体能力：TERM 已说明是 xterm-256color，对每个名字都答「不支持」，程序
                // 不会持续等待。回答写回伪终端即成为 shell 的输入，因此只有十六进制编码的名字
                // （XTGETTCAP 的规定）才原样带回，`cat` 一个文件不会让 shell 执行其中的命令。
                for name in data.split(|&b| b == b';') {
                    if hex_name(name) {
                        self.respond(&format!("\x1bP0+r{}\x1b\\", lossy(name)));
                    } else {
                        self.respond("\x1bP0+r\x1b\\");
                    }
                }
            }
            Some(Dcs::Setting(data)) => {
                let reply = match &data[..] {
                    b"m" => Some(format!("{}m", self.sgr_string())),
                    b"r" => Some(format!("{};{}r", self.top + 1, self.bottom)),
                    b" q" => {
                        let base = match self.shape {
                            CursorShape::Block => 1,
                            CursorShape::Underline => 3,
                            CursorShape::Bar => 5,
                        };
                        Some(format!("{} q", base + u8::from(!self.blink)))
                    }
                    _ => None,
                };
                match reply {
                    Some(reply) => self.respond(&format!("\x1bP1$r{reply}\x1b\\")),
                    None => self.respond("\x1bP0$r\x1b\\"),
                }
            }
            None => {}
        }
    }
}

impl Emulator {
    /// 追加一条要写回伪终端的回答。
    fn respond(&mut self, reply: &str) {
        self.responses.extend_from_slice(reply.as_bytes());
    }

    /// 返回光标位置（行、列，均从 1 起）；原点模式下行号从滚动区域顶边算起。
    fn report_position(&self) -> (usize, usize) {
        let cursor = self.cursor();
        let row = if self.origin {
            cursor.row.saturating_sub(self.top)
        } else {
            cursor.row
        };
        (row + 1, cursor.col + 1)
    }

    fn set_ansi_mode(&mut self, mode: u16, on: bool) {
        match mode {
            4 => self.insert = on,
            20 => self.modes.set(Modes::NEWLINE, on),
            _ => {}
        }
    }

    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.set(Modes::APP_CURSOR, on),
            5 => {
                self.modes.set(Modes::REVERSE, on);
                self.damage.all();
            }
            6 => {
                self.origin = on;
                self.goto(0, 0);
            }
            7 => {
                // 关闭自动换行时写到行尾不算等待折行（见 `write_char`），重新开启后不补折行。
                if on && !self.autowrap {
                    self.screen_mut().cursor.pending_wrap = false;
                }
                self.autowrap = on;
            }
            12 => self.blink = on,
            25 => self.cursor_visible = on,
            // 备用屏：模式 47 切换时不清空，1047 切换时清空，1049 另外保存和恢复光标。
            47 => self.set_alternate(on, false, false),
            1047 => self.set_alternate(on, false, true),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => self.set_alternate(on, true, true),
            66 => self.modes.set(Modes::APP_KEYPAD, on),
            9 | 1000 => self.modes.set(Modes::MOUSE_PRESS, on),
            1002 => self.modes.set(Modes::MOUSE_DRAG, on),
            1003 => self.modes.set(Modes::MOUSE_MOTION, on),
            1004 => self.modes.set(Modes::FOCUS, on),
            1006 => self.modes.set(Modes::MOUSE_SGR, on),
            1007 => self.modes.set(Modes::ALT_SCROLL, on),
            2004 => self.modes.set(Modes::BRACKETED_PASTE, on),
            2026 => self.sync = on,
            2031 => self.scheme_reports = on,
            _ => {}
        }
    }

    /// DECRQM 的回答：1 开着，2 关着，0 不认识。
    fn private_mode_state(&self, mode: u16) -> u8 {
        let modes = self.modes;
        match mode {
            1 => on_off(modes.has(Modes::APP_CURSOR)),
            5 => on_off(modes.has(Modes::REVERSE)),
            6 => on_off(self.origin),
            7 => on_off(self.autowrap),
            12 => on_off(self.blink),
            25 => on_off(self.cursor_visible),
            47 | 1047 | 1049 => on_off(self.alt),
            66 => on_off(modes.has(Modes::APP_KEYPAD)),
            9 | 1000 => on_off(modes.has(Modes::MOUSE_PRESS)),
            1002 => on_off(modes.has(Modes::MOUSE_DRAG)),
            1003 => on_off(modes.has(Modes::MOUSE_MOTION)),
            1004 => on_off(modes.has(Modes::FOCUS)),
            1006 => on_off(modes.has(Modes::MOUSE_SGR)),
            1007 => on_off(modes.has(Modes::ALT_SCROLL)),
            2004 => on_off(modes.has(Modes::BRACKETED_PASTE)),
            2026 => on_off(self.sync),
            // 字素簇宽度一直开启，不可关闭（3 表示永久开启）。
            2027 => 3,
            2031 => on_off(self.scheme_reports),
            _ => 0,
        }
    }

    /// 回答当前主题的明暗（`CSI ? 997 ; 1 n` 为深色，`2` 为浅色）；没有主题时不回答。
    pub(super) fn report_color_scheme(&mut self) {
        if let Some(dark) = self.theme.as_ref().map(|theme| theme.is_dark()) {
            let scheme = if dark { 1 } else { 2 };
            self.respond(&format!("\x1b[?997;{scheme}n"));
        }
    }

    /// 回答 `CSI t` 的查询：14 为窗口像素尺寸，16 为单元格像素尺寸，18 为行列数。
    ///
    /// 其余请求（修改窗口大小、标题栈）忽略。
    fn window_report(&mut self, op: u16) {
        let (width, height) = (usize::from(self.pixels.0), usize::from(self.pixels.1));
        match op {
            14 => self.respond(&format!("\x1b[4;{height};{width}t")),
            16 => {
                let (h, w) = (height / self.rows, width / self.cols);
                self.respond(&format!("\x1b[6;{h};{w}t"));
            }
            18 => {
                let (rows, cols) = (self.rows, self.cols);
                self.respond(&format!("\x1b[8;{rows};{cols}t"));
            }
            _ => {}
        }
    }

    /// 修改画笔（SGR）。
    fn sgr(&mut self, params: &Params) {
        let pen = &mut self.pen;
        if params.is_empty() {
            *pen = crate::protocol::Style {
                link: pen.link,
                ..Default::default()
            };
            return;
        }
        let mut iter = params.iter();
        while let Some(param) = iter.next() {
            let attrs = &mut pen.attrs;
            match param[0] {
                0 => {
                    *pen = crate::protocol::Style {
                        link: pen.link,
                        ..Default::default()
                    }
                }
                1 => attrs.set(Attrs::BOLD, true),
                2 => attrs.set(Attrs::DIM, true),
                3 => attrs.set(Attrs::ITALIC, true),
                4 => attrs.set_underline(match param.get(1).copied().unwrap_or(1) {
                    0 => Underline::None,
                    2 => Underline::Double,
                    3 => Underline::Curly,
                    4 => Underline::Dotted,
                    5 => Underline::Dashed,
                    _ => Underline::Single,
                }),
                5 | 6 => attrs.set(Attrs::BLINK, true),
                7 => attrs.set(Attrs::INVERSE, true),
                8 => attrs.set(Attrs::HIDDEN, true),
                9 => attrs.set(Attrs::STRIKE, true),
                21 => attrs.set_underline(Underline::Double),
                22 => attrs.set(Attrs::BOLD | Attrs::DIM, false),
                23 => attrs.set(Attrs::ITALIC, false),
                24 => attrs.set_underline(Underline::None),
                25 => attrs.set(Attrs::BLINK, false),
                27 => attrs.set(Attrs::INVERSE, false),
                28 => attrs.set(Attrs::HIDDEN, false),
                29 => attrs.set(Attrs::STRIKE, false),
                n @ 30..=37 => pen.fg = Color::Indexed((n - 30) as u8),
                38 => {
                    if let Some(color) = extended_color(param, &mut iter) {
                        pen.fg = color;
                    }
                }
                39 => pen.fg = Color::Default,
                n @ 40..=47 => pen.bg = Color::Indexed((n - 40) as u8),
                48 => {
                    if let Some(color) = extended_color(param, &mut iter) {
                        pen.bg = color;
                    }
                }
                49 => pen.bg = Color::Default,
                58 => {
                    if let Some(color) = extended_color(param, &mut iter) {
                        pen.underline_color = color;
                    }
                }
                59 => pen.underline_color = Color::Default,
                n @ 90..=97 => pen.fg = Color::Indexed((n - 90 + 8) as u8),
                n @ 100..=107 => pen.bg = Color::Indexed((n - 100 + 8) as u8),
                _ => {}
            }
        }
    }

    /// 当前画笔写成 SGR 参数（DECRQSS 的回答）。
    fn sgr_string(&self) -> String {
        let pen = self.pen;
        let mut out = String::from("0");
        let attrs = pen.attrs;
        for (bit, code) in [
            (Attrs::BOLD, 1),
            (Attrs::DIM, 2),
            (Attrs::ITALIC, 3),
            (Attrs::BLINK, 5),
            (Attrs::INVERSE, 7),
            (Attrs::HIDDEN, 8),
            (Attrs::STRIKE, 9),
        ] {
            if attrs.has(bit) {
                let _ = write!(out, ";{code}");
            }
        }
        if attrs.underline() != Underline::None {
            let _ = write!(out, ";4:{}", attrs.underline() as u16);
        }
        for (color, base) in [(pen.fg, 38), (pen.bg, 48), (pen.underline_color, 58)] {
            match color {
                Color::Default => {}
                Color::Indexed(i) => {
                    let _ = write!(out, ";{base}:5:{i}");
                }
                Color::Rgb(r, g, b) => {
                    let _ = write!(out, ";{base}:2::{r}:{g}:{b}");
                }
            }
        }
        out
    }

    /// 记录一条通知，标题和正文各截到 [`MAX_NOTIFICATION`] 个字符。
    fn notify(&mut self, mut title: String, mut body: String) {
        for text in [&mut title, &mut body] {
            if let Some((cut, _)) = text.char_indices().nth(MAX_NOTIFICATION) {
                text.truncate(cut);
            }
        }
        self.events.push(Event::Notify { title, body });
    }

    fn link_id(&mut self, uri: String) -> u16 {
        // 过长的地址不当作链接：每个链接地址都要记录并发送给主程序。
        if uri.is_empty() || uri.len() > MAX_LINK {
            return 0;
        }
        if let Some(&id) = self.links.ids.get(&uri) {
            return id;
        }
        let id = self.links.ids.len() + 1;
        if id >= usize::from(u16::MAX) || self.links.bytes + uri.len() > MAX_LINK_BYTES {
            return 0;
        }
        let id = id as u16;
        self.links.bytes += uri.len();
        self.links.ids.insert(uri.clone(), id);
        self.events.push(Event::Link(id, uri));
        id
    }

    /// 处理 shell 集成报告的命令边界（OSC 133）。
    fn shell_mark(&mut self, params: &[&[u8]]) {
        let Some(kind) = params.get(1).and_then(|p| p.first()) else {
            return;
        };
        let (flag, mark) = match kind {
            b'A' => (Marks::PROMPT, CommandMark::Prompt),
            b'B' => (Marks::COMMAND, CommandMark::Command),
            b'C' => (Marks::OUTPUT, CommandMark::Output),
            b'D' => {
                let exit = params.get(2).and_then(|p| lossy(p).parse::<i32>().ok());
                self.mark(Marks::DONE, exit, None);
                self.events.push(Event::Command(CommandMark::Done(exit)));
                return;
            }
            _ => return,
        };
        // `A` 之后可以带 `redraw=1` 或 `redraw=0`，表示 shell 改宽度时是否自行重画整个提示符。
        // 没有携带参数时不改动（提示符主题也可能发送 `A`）。命令运行期间收到的 `A` 来自命令的输出
        // （远程 shell、日志文件），不代表本地 shell 会重画。
        if flag == Marks::PROMPT && !self.blocks.running() {
            for param in params.get(2..).unwrap_or_default() {
                match *param {
                    b"redraw=1" => self.blocks.redraws_prompt = true,
                    b"redraw=0" => self.blocks.redraws_prompt = false,
                    _ => {}
                }
            }
        }
        // `C` 之后可以带命令行：`cmdline_url=` 为百分号转义形式，`cmdline=` 为原样形式；命令行
        // 中的分号被 OSC 切开，需要拼接还原。「重新运行」会把命令行原样写入 shell 的输入，因此
        // 只接受附带随机数的命令行（`nonce=`，见 [`Emulator::nonce`]），伪造的不记录。
        let mut text = None;
        if flag == Marks::OUTPUT
            && let Some((first, rest)) = params.get(2..).and_then(<[_]>::split_first)
            && let Some(nonce) = &self.nonce
            && first.strip_prefix(b"nonce=") == Some(nonce.as_bytes())
        {
            let rest = join(rest);
            text = Some(if let Some(url) = rest.strip_prefix("cmdline_url=") {
                lossy(&percent_decode(url.as_bytes()))
            } else if let Some(raw) = rest.strip_prefix("cmdline=") {
                raw.to_owned()
            } else {
                String::new()
            });
        }
        self.mark(flag, None, text);
        self.events.push(Event::Command(mark));
    }
}

fn on_off(on: bool) -> u8 {
    if on { 1 } else { 2 }
}

/// 解析 SGR 38、48、58 之后的颜色，无法解析时返回 `None`。
///
/// 冒号形式（`38:2:r:g:b`、`38:5:n`）的颜色在同一参数的子参数里，分号形式（`38;2;r;g;b`、
/// `38;5;n`）的颜色从后面的参数里取。
fn extended_color<'a>(param: &[u16], rest: &mut impl Iterator<Item = &'a [u16]>) -> Option<Color> {
    let byte = |v: u16| v.min(255) as u8;
    if param.len() > 1 {
        return match param[1] {
            5 => param.get(2).map(|&i| Color::Indexed(byte(i))),
            2 => {
                // 带色彩空间时为 `38:2:cs:r:g:b`（6 个分量），不带时为 `38:2:r:g:b`。
                let rgb = if param.len() >= 6 {
                    &param[3..6]
                } else {
                    param.get(2..5)?
                };
                Some(Color::Rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2])))
            }
            _ => None,
        };
    }
    match rest.next()?[0] {
        5 => Some(Color::Indexed(byte(rest.next()?[0]))),
        2 => {
            let r = rest.next()?[0];
            let g = rest.next()?[0];
            let b = rest.next()?[0];
            Some(Color::Rgb(byte(r), byte(g), byte(b)))
        }
        _ => None,
    }
}

/// 判断 XTGETTCAP 请求的能力名是否为十六进制编码：非空且由偶数个十六进制数字组成。
fn hex_name(name: &[u8]) -> bool {
    !name.is_empty() && name.len().is_multiple_of(2) && name.iter().all(u8::is_ascii_hexdigit)
}

/// 把字节按 UTF-8 解码，非法部分替换为 U+FFFD。
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 用分号连接 OSC 参数，还原标题、链接地址中本来的分号。
fn join(parts: &[&[u8]]) -> String {
    lossy(&parts.join(&b';'))
}

/// 取出 `file://主机/路径` 中的路径并解开百分号转义；任意协议名的 URL 都按相同规则取路径。
fn file_url_path(url: &[u8]) -> Option<Vec<u8>> {
    let scheme_end = url.windows(3).position(|w| w == b"://")?;
    let rest = &url[scheme_end + 3..];
    let path = &rest[rest.iter().position(|&b| b == b'/')?..];
    Some(percent_decode(path))
}

/// 解开百分号转义，`%` 之后不是两位十六进制数时原样保留。
fn percent_decode(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'%'
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(text[i]);
        i += 1;
    }
    out
}

/// 解码 base64，忽略空白和 `=`，遇到无法识别的字符时返回 `None`。
///
/// 不检查填充与末尾剩余的位。
fn base64(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &b in data {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// 颜色查询的回答格式：`rgb:rrrr/gggg/bbbb`，每个分量把 8 位值重复成 16 位（`0x37` → `3737`）。
fn rgb_spec([r, g, b]: [u8; 3]) -> String {
    let wide = |v: u8| u16::from(v) * 257;
    format!("rgb:{:04x}/{:04x}/{:04x}", wide(r), wide(g), wide(b))
}

#[cfg(test)]
mod tests;
