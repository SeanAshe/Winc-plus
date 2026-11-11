//! 寄存器：复制、删除的文本及其类型（按字符、按行）。
//!
//! 无名寄存器不单独保存内容，指向最近一次写入的寄存器。文本中的换行统一记为 `\n`，粘贴时由
//! 调用方换成文档的换行符。

use std::collections::BTreeMap;

/// 寄存器内容的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 按字符。
    Charwise,
    /// 按行：内容是若干整行，粘贴到光标所在行的下方或上方。
    Linewise,
    /// 按块：内容的各行是矩形区域在各行上的部分，值为块的宽度减一（显示列）。
    Blockwise(usize),
}

/// 一个寄存器的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    /// 类型。
    pub kind: Kind,
    /// 文本，换行记为 `\n`；按行的内容不含最后一行之后的换行符。
    pub text: String,
}

impl Content {
    /// 返回内容的各行。
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }
}

/// 系统剪贴板与无名寄存器的同步方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clipboard {
    /// 只有 `"+`、`"*` 使用系统剪贴板。
    #[default]
    Register,
    /// 复制时同时写入系统剪贴板。
    Yank,
    /// 无名寄存器与系统剪贴板同步。
    Always,
}

/// 全部寄存器占用的上限（字节）；超出时从 `"9` 起清空编号寄存器。
///
/// 取自调参：与撤销栈的上限同一量级，足以保存多次整文件的复制。
const MAX_BYTES: usize = 64 << 20;

/// 写入寄存器的来源，决定除指定的寄存器之外还写入哪些寄存器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// 复制：未指定寄存器时写入 `"0`。
    Yank,
    /// 删除：`numbered` 为真时（按行、跨行，或由指定的动作产生）进入 `"1` 并后移编号寄存器，
    /// 否则未指定寄存器时进入 `"-`。
    Delete {
        /// 是否不论长短都进入编号寄存器。
        numbered: bool,
    },
}

/// 一组寄存器。
#[derive(Debug, Default)]
pub struct Registers {
    /// 各寄存器的内容，以名字为键：`0` 至 `9`、`-`、`a` 至 `z`、`.`、`:`、`/`。
    slots: BTreeMap<char, Content>,
    /// 无名寄存器指向的寄存器名；尚未写入过任何寄存器时为 `None`。
    unnamed: Option<char>,
    /// 最近一次写入系统剪贴板的内容，用于在读回相同的文本时恢复类型。
    clip: Option<Content>,
}

impl Registers {
    /// 返回寄存器 `name` 的内容；`None` 与 `"` 表示无名寄存器，大写字母读取对应的小写寄存器。
    ///
    /// 不读取系统剪贴板：`+`、`*` 由调用方处理。
    pub fn get(&self, name: Option<char>) -> Option<&Content> {
        let name = match name {
            None | Some('"') => self.unnamed?,
            Some(name) => name.to_ascii_lowercase(),
        };
        self.slots.get(&name)
    }

    /// 返回各寄存器的名字与内容，按名字排序；用于核对与列出。
    pub fn iter(&self) -> impl Iterator<Item = (char, &Content)> {
        self.slots.iter().map(|(name, content)| (*name, content))
    }

    /// 返回无名寄存器指向的寄存器名。
    pub fn unnamed(&self) -> Option<char> {
        self.unnamed
    }

    /// 把只读寄存器 `name`（`.`、`:`、`/`）的内容设为按字符的 `text`。
    pub(crate) fn set_special(&mut self, name: char, text: String) {
        let kind = Kind::Charwise;
        self.slots.insert(name, Content { kind, text });
    }

    /// 把宏 `content` 存入寄存器 `name`，无名寄存器与编号寄存器不变；大写字母表示追加。
    pub(crate) fn set_macro(&mut self, name: char, content: Content) {
        let slot = name.to_ascii_lowercase();
        match self.slots.get_mut(&slot) {
            Some(existing) if name.is_ascii_uppercase() => existing.text.push_str(&content.text),
            _ => {
                self.slots.insert(slot, content);
            }
        }
    }

    /// 把 `content` 解释成从系统剪贴板读到的文本：与最近一次写入剪贴板的文本相同时沿用
    /// 当时的类型，否则以换行符结尾的是按行，其余按字符。
    pub(crate) fn read_clipboard(&self, text: &str) -> Content {
        let text = text.replace("\r\n", "\n");
        if let Some(last) = &self.clip
            && clipboard_text(last) == text
        {
            return last.clone();
        }
        match text.strip_suffix('\n') {
            Some(lines) => Content {
                kind: Kind::Linewise,
                text: lines.to_owned(),
            },
            None => Content {
                kind: Kind::Charwise,
                text,
            },
        }
    }

    /// 记录写入系统剪贴板的内容 `content`，并返回写入的文本。
    pub(crate) fn write_clipboard(&mut self, content: &Content) -> String {
        self.clip = Some(content.clone());
        clipboard_text(content)
    }

    /// 把 `content` 写入寄存器。
    ///
    /// `name` 是命令指定的寄存器名（已排除 `_`、`+`、`*` 与只读寄存器）；`None` 与 `"` 表示
    /// 未指定，大写字母表示追加到对应的小写寄存器。
    pub(crate) fn write(&mut self, name: Option<char>, content: Content, source: Source) {
        let name = name.filter(|name| *name != '"');
        let multiline = content.kind == Kind::Linewise || content.text.contains('\n');
        if let Some(name) = name {
            self.store(name, content.clone());
            self.unnamed = Some(name.to_ascii_lowercase());
        }
        match source {
            Source::Yank => {
                if name.is_none() {
                    self.store('0', content);
                    self.unnamed = Some('0');
                }
            }
            Source::Delete { numbered } => {
                let small = name.is_none() && !multiline;
                if name.is_some() || multiline || numbered {
                    self.shift_numbered();
                    self.store('1', content.clone());
                    self.unnamed = Some('1');
                }
                if small {
                    self.store('-', content);
                    self.unnamed = Some('-');
                }
            }
        }
        self.trim();
    }

    /// 把 `"1` 至 `"8` 的内容依次后移一位，原来的 `"9` 丢弃。
    fn shift_numbered(&mut self) {
        for digit in (b'1'..=b'8').rev() {
            match self.slots.remove(&char::from(digit)) {
                Some(content) => {
                    self.slots.insert(char::from(digit + 1), content);
                }
                None => {
                    self.slots.remove(&char::from(digit + 1));
                }
            }
        }
    }

    /// 写入名为 `name` 的寄存器；大写字母追加到对应的小写寄存器。
    fn store(&mut self, name: char, content: Content) {
        if !name.is_ascii_uppercase() {
            self.slots.insert(name, content);
            return;
        }
        let lower = name.to_ascii_lowercase();
        match self.slots.get_mut(&lower) {
            None => {
                self.slots.insert(lower, content);
            }
            Some(existing) => {
                // 追加按行的内容时，原有的内容也变为按行；向按行或按块的内容追加时另起一行。
                if content.kind == Kind::Linewise {
                    existing.kind = Kind::Linewise;
                }
                if existing.kind != Kind::Charwise {
                    existing.text.push('\n');
                }
                existing.text.push_str(&content.text);
            }
        }
    }

    /// 总占用超过 [`MAX_BYTES`] 时从 `"9` 起清空编号寄存器。
    fn trim(&mut self) {
        let mut total: usize = self.slots.values().map(|c| c.text.len()).sum();
        for digit in (b'1'..=b'9').rev() {
            if total <= MAX_BYTES {
                break;
            }
            if let Some(content) = self.slots.remove(&char::from(digit)) {
                total -= content.text.len();
            }
        }
    }
}

/// 返回 `content` 写入系统剪贴板时的文本：按行的内容以换行符结尾。
fn clipboard_text(content: &Content) -> String {
    match content.kind {
        Kind::Charwise | Kind::Blockwise(_) => content.text.clone(),
        Kind::Linewise => format!("{}\n", content.text),
    }
}
