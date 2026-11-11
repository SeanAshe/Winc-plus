//! 按键：一次按键的规范形式，以及按键序列的文本记法（[`notation`]）。
//!
//! 同一个按键只有一种表示：控制字符、`Ctrl` 与字母、`Shift` 与字母的组合在 [`Key::new`] 中
//! 归并，命令表和解析器只比较规范形式。

pub mod notation;

use std::fmt;

/// 不产生字符的功能键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Named {
    /// `Esc`，记作 `<Esc>`。
    Esc,
    /// 回车，记作 `<CR>`。
    Enter,
    /// 制表键，记作 `<Tab>`。
    Tab,
    /// 退格，记作 `<BS>`。
    Backspace,
    /// 向后删除，记作 `<Del>`。
    Delete,
    /// 插入键，记作 `<Insert>`。
    Insert,
    /// 方向键上，记作 `<Up>`。
    Up,
    /// 方向键下，记作 `<Down>`。
    Down,
    /// 方向键左，记作 `<Left>`。
    Left,
    /// 方向键右，记作 `<Right>`。
    Right,
    /// 行首键，记作 `<Home>`。
    Home,
    /// 行尾键，记作 `<End>`。
    End,
    /// 上翻页，记作 `<PageUp>`。
    PageUp,
    /// 下翻页，记作 `<PageDown>`。
    PageDown,
    /// 功能键 `F1` 至 `F35`，记作 `<F1>`；序号从 1 起。
    F(u8),
}

/// 功能键序号的上限：键盘事件能区分的功能键为 `F1` 至 `F35`。
pub(crate) const MAX_FUNCTION_KEY: u8 = 35;

/// 按键的主体：产生的字符，或一个功能键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    /// 按键产生的字符，已包含 `Shift` 与键盘布局的作用（`D`、`$`）；不是控制字符。
    Char(char),
    /// 功能键。
    Named(Named),
}

/// 一组修饰键。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    /// 没有修饰键。
    pub const NONE: Self = Self(0);
    /// `Ctrl`，记作 `C-`。
    pub const CTRL: Self = Self(1);
    /// `Shift`，记作 `S-`。
    pub const SHIFT: Self = Self(1 << 1);
    /// `Alt`（macOS 上为 `Option`），记作 `M-`。
    pub const ALT: Self = Self(1 << 2);
    /// `Command`（macOS）或徽标键，记作 `D-`。
    pub const CMD: Self = Self(1 << 3);

    /// 是否包含 `other` 中的全部修饰键。
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// 是否没有任何修饰键。
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 返回加上 `other` 之后的修饰键。
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 返回去掉 `other` 之后的修饰键。
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl fmt::Debug for Mods {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mods(")?;
        notation::write_mods(f, *self)?;
        write!(f, ")")
    }
}

/// 一次按键的规范形式：主体加修饰键。
///
/// 只能由 [`Key::new`] 及其简写创建，相同的按键比较结果相等。[`fmt::Display`] 输出按键记法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// 按键的主体。
    code: Code,
    /// 归并之后仍然保留的修饰键。
    mods: Mods,
}

impl Key {
    /// 创建按键，并归并成规范形式。
    ///
    /// - 控制字符（U+0000 至 U+001F、U+007F）改为 `Ctrl` 与对应字符的组合，例如 U+0004 即
    ///   `Ctrl` 与 `d` 的组合。
    /// - `Ctrl` 与 ASCII 字母的组合不区分大小写，字母记为小写。
    /// - `Ctrl` 与 `[`、`i`、`m` 的组合分别等同于 `<Esc>`、`<Tab>`、`<CR>`，去掉 `Ctrl`。
    /// - 没有 `Ctrl` 时，`Shift` 与 ASCII 字母的组合改为大写字母，去掉 `Shift`。
    /// - 功能键的序号限制在 1 至 35 之内。
    pub fn new(code: Code, mods: Mods) -> Self {
        let (mut code, mut mods) = (code, mods);
        if let Code::Named(Named::F(number)) = code {
            code = Code::Named(Named::F(number.clamp(1, MAX_FUNCTION_KEY)));
        }
        if let Code::Char(c) = code
            && c.is_ascii_control()
        {
            // 控制字符与可见字符相差 0x40：U+0004 对应 `D`，U+007F 对应 `?`。
            code = Code::Char(char::from(c as u8 ^ 0x40));
            mods = mods.with(Mods::CTRL);
        }
        if let Code::Char(c) = code {
            if mods.contains(Mods::CTRL) {
                let lower = c.to_ascii_lowercase();
                let named = match lower {
                    '[' => Some(Named::Esc),
                    'i' => Some(Named::Tab),
                    'm' => Some(Named::Enter),
                    _ => None,
                };
                match named {
                    Some(named) => {
                        code = Code::Named(named);
                        mods = mods.without(Mods::CTRL);
                    }
                    None => code = Code::Char(lower),
                }
            } else if mods.contains(Mods::SHIFT) && c.is_ascii_alphabetic() {
                code = Code::Char(c.to_ascii_uppercase());
                mods = mods.without(Mods::SHIFT);
            }
        }
        Self { code, mods }
    }

    /// 创建产生字符 `c` 的按键，没有修饰键；控制字符的归并见 [`Key::new`]。
    pub fn char(c: char) -> Self {
        Self::new(Code::Char(c), Mods::NONE)
    }

    /// 创建 `Ctrl` 与字符 `c` 的组合。
    pub fn ctrl(c: char) -> Self {
        Self::new(Code::Char(c), Mods::CTRL)
    }

    /// 创建没有修饰键的功能键。
    pub fn named(named: Named) -> Self {
        Self::new(Code::Named(named), Mods::NONE)
    }

    /// 返回按键的主体。
    pub fn code(self) -> Code {
        self.code
    }

    /// 返回按键的修饰键。
    pub fn mods(self) -> Mods {
        self.mods
    }

    /// 返回按键产生的字符；带 `Ctrl`、`Alt`、`Command` 或是功能键时返回 `None`。
    pub fn as_char(self) -> Option<char> {
        match self.code {
            Code::Char(c) if self.mods.without(Mods::SHIFT).is_empty() => Some(c),
            _ => None,
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        notation::write_key(f, *self)
    }
}

#[cfg(test)]
mod tests;
