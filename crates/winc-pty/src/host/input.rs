//! 主程序一侧：把按键、粘贴和焦点变化编码成写给伪终端的字节。
//!
//! 按 xterm 控制序列的规则编码，包括 Kitty 键盘协议的第 1 级。本模块不依赖窗口库：主程序把
//! 自己的按键事件转换成 [`Key`] 和 [`Mods`] 后交给 [`encode`]。

use crate::protocol::Modes;

/// 一个按键（不含 ⌘ 组合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key<'a> {
    /// 键入的文字（已按键盘布局和 ⇧ 换算，不含 ⌃ 和 Meta）。
    Text(&'a str),
    /// 与 ⌃ 或 Meta 一起按下的字符键：键上的字符（已按 ⇧ 换算），用于计算控制字符或加 `ESC`
    /// 前缀。
    Char(char),
    /// 回车。
    Enter,
    /// 制表键。
    Tab,
    /// 退格。
    Backspace,
    /// Esc。
    Escape,
    /// ↑。
    Up,
    /// ↓。
    Down,
    /// ←。
    Left,
    /// →。
    Right,
    /// Home。
    Home,
    /// 功能键 F1–F12。
    End,
    /// Page Up。
    PageUp,
    /// Page Down。
    PageDown,
    /// Insert。
    Insert,
    /// 向前删除（⌦）。
    Delete,
    /// 功能键 F1–F12。
    F(u8),
}

/// 修饰键集合。
///
/// macOS 的 ⌘ 不包含在内：带 ⌘ 的组合由主程序处理（快捷键、行首行尾）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// ⇧。
    pub shift: bool,
    /// Meta：⌥ 作为 Meta 使用时为真（`terminal.optionAsMeta`）。
    pub alt: bool,
    /// ⌃。
    pub ctrl: bool,
}

impl Mods {
    /// 返回 xterm 的修饰参数：1 加上 ⇧ 的 1、Alt 的 2、⌃ 的 4。
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }

    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }
}

/// Kitty 键盘协议的「消除歧义」标志位（第 1 级，位值 1）。
pub const DISAMBIGUATE: u8 = 0b1;

/// 把按键编码成字节，追加到 `out` 末尾。
///
/// `keyboard` 是程序打开的 Kitty 键盘协议标志位，即帧里的 [`crate::protocol::Frame::keyboard`]。
/// 该按键在终端中没有意义时返回 `false`，不追加任何字节。
pub fn encode(key: Key<'_>, mods: Mods, modes: Modes, keyboard: u8, out: &mut Vec<u8>) -> bool {
    if keyboard & DISAMBIGUATE != 0 && kitty(key, mods, out) {
        return true;
    }
    let start = out.len();
    let app_cursor = modes.has(Modes::APP_CURSOR);

    match key {
        Key::Text(text) => {
            if mods.alt {
                out.push(0x1b);
            }
            out.extend_from_slice(text.as_bytes());
        }
        Key::Char(c) => return encode_char(c, mods, out),
        Key::Enter => {
            if mods.alt {
                out.push(0x1b);
            }
            out.extend_from_slice(if modes.has(Modes::NEWLINE) {
                b"\r\n"
            } else {
                b"\r"
            });
        }
        Key::Tab if mods.shift => out.extend_from_slice(b"\x1b[Z"),
        Key::Tab => {
            if mods.alt {
                out.push(0x1b);
            }
            out.push(b'\t');
        }
        Key::Backspace => {
            if mods.alt {
                out.push(0x1b);
            }
            // ⌃⌫ 按 xterm 的规则发 ^H；普通退格发 DEL。
            out.push(if mods.ctrl { 0x08 } else { 0x7f });
        }
        Key::Escape => {
            if mods.alt {
                out.push(0x1b);
            }
            out.push(0x1b);
        }
        Key::Up => cursor_key(b'A', mods, app_cursor, out),
        Key::Down => cursor_key(b'B', mods, app_cursor, out),
        Key::Right => cursor_key(b'C', mods, app_cursor, out),
        Key::Left => cursor_key(b'D', mods, app_cursor, out),
        Key::Home => cursor_key(b'H', mods, app_cursor, out),
        Key::End => cursor_key(b'F', mods, app_cursor, out),
        Key::Insert => tilde_key(2, mods, out),
        Key::Delete => tilde_key(3, mods, out),
        Key::PageUp => tilde_key(5, mods, out),
        Key::PageDown => tilde_key(6, mods, out),
        Key::F(n @ 1..=4) => {
            let last = b'P' + (n - 1);
            if mods.any() {
                out.extend_from_slice(format!("\x1b[1;{}", mods.param()).as_bytes());
                out.push(last);
            } else {
                out.extend_from_slice(&[0x1b, b'O', last]);
            }
        }
        Key::F(n @ 5..=12) => {
            // xterm 给 F5–F12 的编号是 15、17、18、19、20、21、23、24（没有 16 和 22）。
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde_key(CODES[usize::from(n - 5)], mods, out);
        }
        Key::F(_) => return false,
    }
    out.len() > start
}

/// 按 Kitty 键盘协议第 1 级编码传统编码无法区分的键，返回是否已编码。
///
/// 范围是 Esc、带 ⌃ 或 Meta 的字符键、带修饰键的回车、制表和退格，编成 `CSI 码点; 修饰 u`；
/// 其余键（打字、方向键、功能键）保持原编码并返回 `false`。
fn kitty(key: Key<'_>, mods: Mods, out: &mut Vec<u8>) -> bool {
    let code = match key {
        Key::Escape => 27,
        Key::Enter | Key::Tab | Key::Backspace if !mods.any() => return false,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        // 协议中用不带 ⇧ 的字符（小写字母），⇧ 记在修饰位里。
        Key::Char(c) if mods.ctrl || mods.alt => u32::from(c.to_ascii_lowercase()),
        _ => return false,
    };
    if mods.any() {
        out.extend_from_slice(format!("\x1b[{code};{}u", mods.param()).as_bytes());
    } else {
        out.extend_from_slice(format!("\x1b[{code}u").as_bytes());
    }
    true
}

/// 编码方向键、Home 和 End：没有修饰键时按光标键模式发 `ESC [ A` 或 `ESC O A`，有修饰键时发
/// `ESC [ 1; m A`。
fn cursor_key(last: u8, mods: Mods, app_cursor: bool, out: &mut Vec<u8>) {
    if mods.any() {
        out.extend_from_slice(format!("\x1b[1;{}", mods.param()).as_bytes());
        out.push(last);
    } else if app_cursor {
        out.extend_from_slice(&[0x1b, b'O', last]);
    } else {
        out.extend_from_slice(&[0x1b, b'[', last]);
    }
}

/// 编码成 `ESC [ n ~`，有修饰键时编成 `ESC [ n; m ~`。
fn tilde_key(code: u8, mods: Mods, out: &mut Vec<u8>) {
    if mods.any() {
        out.extend_from_slice(format!("\x1b[{code};{}~", mods.param()).as_bytes());
    } else {
        out.extend_from_slice(format!("\x1b[{code}~").as_bytes());
    }
}

/// 编码 ⌃ 或 Meta 加字符键，返回是否已编码。
///
/// ⌃ 字母、⌃@、⌃[ 有对应的控制字符；Meta 在字节前加 `ESC`。没有控制字符的组合不编码。
fn encode_char(c: char, mods: Mods, out: &mut Vec<u8>) -> bool {
    let start = out.len();
    if mods.alt {
        out.push(0x1b);
    }
    if mods.ctrl {
        let Some(control) = control_char(c) else {
            out.truncate(start);
            return false;
        };
        out.push(control);
    } else {
        let mut buf = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    true
}

/// 返回 ⌃ 加上 `c` 对应的控制字符；没有对应控制字符时返回 `None`。
fn control_char(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// 鼠标按键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// 左键。
    Left,
    /// 中键。
    Middle,
    /// 右键。
    Right,
    /// 滚轮向上一格。
    WheelUp,
    /// 滚轮向下一格。
    WheelDown,
    /// 没有按键（仅移动）。
    None,
}

/// 鼠标事件的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// 按下（滚轮也算按下）。
    Press,
    /// 松开。
    Release,
    /// 移动（按住键时为拖动）。
    Motion,
}

/// 是否要上报鼠标事件（模式 1000、1002、1003 之一已打开）。
pub fn wants_mouse(modes: Modes) -> bool {
    modes.0 & (Modes::MOUSE_PRESS | Modes::MOUSE_DRAG | Modes::MOUSE_MOTION) != 0
}

/// 把鼠标事件编码成上报给程序的字节（xterm 的鼠标上报），返回是否已编码。
///
/// `col`、`row` 是格子坐标，从 0 起。程序未请求该类事件时返回 `false` 且不追加字节：未开
/// 1002 或 1003 不上报拖动，未开 1003 不上报没有按键的移动。开启 1006 时用 SGR 编码，否则用
/// 旧编码，坐标超过 222 时无法上报（每个数加 32 后必须放在一个字节里）。
pub fn mouse(
    button: Button,
    action: MouseAction,
    (col, row): (u16, u16),
    mods: Mods,
    modes: Modes,
    out: &mut Vec<u8>,
) -> bool {
    if !wants_mouse(modes) {
        return false;
    }
    if action == MouseAction::Motion {
        let wanted = if button == Button::None {
            modes.has(Modes::MOUSE_MOTION)
        } else {
            modes.0 & (Modes::MOUSE_DRAG | Modes::MOUSE_MOTION) != 0
        };
        if !wanted {
            return false;
        }
    }
    let mut code: u16 = match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::None => 3,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
    };
    if action == MouseAction::Motion {
        code += 32;
    }
    code += 4 * u16::from(mods.shift) + 8 * u16::from(mods.alt) + 16 * u16::from(mods.ctrl);

    if modes.has(Modes::MOUSE_SGR) {
        let last = if action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        out.extend_from_slice(format!("\x1b[<{code};{};{}{last}", col + 1, row + 1).as_bytes());
        return true;
    }
    // 旧编码：松开时不区分按键，统一上报 3；每个数加 32 后写成一个字节。
    if action == MouseAction::Release {
        code = 3 + (code & !3);
    }
    let byte = |v: u16| u8::try_from(v + 32).ok();
    let (Some(c), Some(x), Some(y)) = (byte(code), byte(col + 1), byte(row + 1)) else {
        return false;
    };
    out.extend_from_slice(&[0x1b, b'[', b'M', c, x, y]);
    true
}

/// 粘贴 `text` 之前是否需要确认。
///
/// 程序未开启括号粘贴且 `text` 不止一行时为真；只在结尾多一个换行的单行文本不算。没有括号
/// 粘贴时每个换行都相当于按一次回车，粘贴进 shell 的每一行都会立即执行。
pub fn paste_needs_confirm(text: &str, modes: Modes) -> bool {
    let body = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(['\n', '\r']))
        .unwrap_or(text);
    !modes.has(Modes::BRACKETED_PASTE) && body.contains(['\n', '\r'])
}

/// 编码一次粘贴，追加到 `out` 末尾。
///
/// 换行统一成回车（与在键盘上按回车相同）；程序开启括号粘贴时包上 `ESC[200~`、`ESC[201~`，
/// 并先去掉内容里自带的结束标记，避免粘贴内容提前结束括号粘贴，使后续文本被 shell 当作命令
/// 执行。
pub fn paste(text: &str, modes: Modes, out: &mut Vec<u8>) {
    let bracketed = modes.has(Modes::BRACKETED_PASTE);
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    let start = out.len();
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let b = match b {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => continue,
            b'\n' => b'\r',
            b => b,
        };
        out.push(b);
        // 每写入一个字节检查末尾是否为结束标记，是则移除。去掉一个标记可能拼出另一个
        // （`ESC[201ESC[200~~`），拼出的标记在写入其最后一个字节时同样移除。单遍完成：若每次
        // 移除后都从头重新查找，构造的内容会使粘贴长时间占用界面线程。
        if bracketed
            && b == b'~'
            && let Some(tail) = out.len().checked_sub(6).filter(|&t| t >= start)
            && out[tail..tail + 4] == *b"\x1b[20"
            && matches!(out[tail + 4], b'0' | b'1')
        {
            out.truncate(tail);
        }
    }
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
}

/// 返回把光标从第 `from` 格移到第 `to` 格要按的方向键和次数；一行文字是 `text`。
///
/// 按字符计数：宽字符占两格但只计一次，组合字符不计。`to` 超过文字末尾时按末尾计算。
pub fn cursor_steps(text: &str, from: usize, to: usize) -> (Key<'static>, usize) {
    let (mut col, mut left, mut right) = (0, 0, 0);
    for (_, width) in crate::width::cells(text) {
        if width == 0 {
            continue;
        }
        if col >= to && col < from {
            left += 1;
        } else if col >= from && col < to {
            right += 1;
        }
        col += width;
    }
    if left > 0 {
        (Key::Left, left)
    } else {
        (Key::Right, right)
    }
}

/// 编码焦点进出终端：程序开启焦点上报（1004）时发 `ESC [ I` 或 `ESC [ O`。
pub fn focus(focused: bool, modes: Modes, out: &mut Vec<u8>) {
    if modes.has(Modes::FOCUS) {
        out.extend_from_slice(if focused { b"\x1b[I" } else { b"\x1b[O" });
    }
}

#[cfg(test)]
mod tests;
