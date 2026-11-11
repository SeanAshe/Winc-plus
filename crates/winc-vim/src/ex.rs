//! 命令行（`:`）的解析：范围、命令名、`!` 与参数。
//!
//! 解析只依赖文本：地址中的标记、查找与偏移在执行时求值。命令名按最短缩写识别。

use crate::host::Message;

/// 范围中一个地址的起点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Base {
    /// 当前行（`.`，或只写了偏移）。
    Current,
    /// 最后一行（`$`）。
    Last,
    /// 行号，从 1 起；0 表示第一行之前。
    Number(usize),
    /// 标记所在的行。
    Mark(char),
    /// 之后第一个匹配模式的行（`/模式/`）；模式为空时沿用上一次查找的模式。
    Forward(String),
    /// 之前第一个匹配模式的行（`?模式?`）。
    Backward(String),
}

/// 范围中的一个地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    /// 起点。
    pub base: Base,
    /// 相对起点的行数。
    pub offset: isize,
    /// 该地址之后的分隔符是否为 `;`：求值下一个地址之前先把当前行移到该地址。
    pub moves: bool,
}

/// 命令行中的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmd {
    /// 只有范围：光标移到范围的末行。
    Goto,
    /// `:d`。
    Delete,
    /// `:y`。
    Yank,
    /// `:pu`。
    Put,
    /// `:m`。
    Move,
    /// `:t`、`:co`。
    Copy,
    /// `:j`。
    Join,
    /// `:>`；值为 `>` 的个数。
    ShiftRight(usize),
    /// `:<`；值为 `<` 的个数。
    ShiftLeft(usize),
    /// `:normal`。
    Normal,
    /// `:sort`。
    Sort,
    /// `:k`、`:mark`。
    Mark,
    /// `:noh`。
    NoHighlight,
    /// `:reg`、`:di`。
    Registers,
    /// `:marks`。
    Marks,
    /// `:ls`、`:files`、`:buffers`。
    Buffers,
    /// `:set`。
    Set,
    /// `:s`。
    Substitute,
    /// `:&`、`:&&`。
    RepeatSubstitute,
    /// `:g`。
    Global,
    /// `:v`。
    GlobalInverse,
    /// `:p`、`:#`、`:l`：没有输出窗口，不执行任何操作。
    Print,
    /// `:u`。
    Undo,
    /// `:red`。
    Redo,
    /// `:w`、`:up`。
    Write,
    /// `:wa`。
    WriteAll,
    /// `:q`、`:clo`。
    Quit,
    /// `:qa`。
    QuitAll,
    /// `:wq`、`:x`。
    WriteQuit,
    /// `:wqa`、`:xa`。
    WriteQuitAll,
    /// `:e`。
    Edit,
    /// `:bn`。
    BufferNext,
    /// `:bp`、`:bN`。
    BufferPrevious,
    /// `:bd`。
    BufferDelete,
    /// `:b`。
    Buffer,
    /// `:tabn`。
    TabNext,
    /// `:tabp`、`:tabN`。
    TabPrevious,
    /// `:tabc`。
    TabClose,
    /// `:tabo`。
    TabOnly,
    /// `:tabnew`、`:tabe`。
    TabNew,
    /// `:h`。
    Help,
}

impl Cmd {
    /// 命令是否把参数中的 `|` 当作参数的一部分，而不是下一条命令的开始。
    pub(crate) fn takes_bar(self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Global
                | Self::GlobalInverse
                | Self::Substitute
                | Self::RepeatSubstitute
        )
    }

    /// 命令的参数是否为文件路径，供补全使用。
    fn takes_path(self) -> bool {
        matches!(self, Self::Edit | Self::Write | Self::TabNew)
    }
}

/// 命令名、最短缩写的长度与对应的命令；同一前缀下靠前的一条优先。
const NAMES: &[(&str, usize, Cmd)] = &[
    ("substitute", 1, Cmd::Substitute),
    ("set", 2, Cmd::Set),
    ("sort", 3, Cmd::Sort),
    ("delete", 1, Cmd::Delete),
    ("display", 2, Cmd::Registers),
    ("yank", 1, Cmd::Yank),
    ("put", 2, Cmd::Put),
    ("print", 1, Cmd::Print),
    ("move", 1, Cmd::Move),
    ("mark", 2, Cmd::Mark),
    ("marks", 5, Cmd::Marks),
    ("copy", 2, Cmd::Copy),
    ("t", 1, Cmd::Copy),
    ("join", 1, Cmd::Join),
    ("normal", 4, Cmd::Normal),
    ("k", 1, Cmd::Mark),
    ("nohlsearch", 3, Cmd::NoHighlight),
    ("number", 2, Cmd::Print),
    ("list", 1, Cmd::Print),
    ("ls", 2, Cmd::Buffers),
    ("registers", 3, Cmd::Registers),
    ("redo", 3, Cmd::Redo),
    ("files", 5, Cmd::Buffers),
    ("buffers", 7, Cmd::Buffers),
    ("global", 1, Cmd::Global),
    ("vglobal", 1, Cmd::GlobalInverse),
    ("undo", 1, Cmd::Undo),
    ("update", 2, Cmd::Write),
    ("write", 1, Cmd::Write),
    ("wall", 2, Cmd::WriteAll),
    ("wq", 2, Cmd::WriteQuit),
    ("wqall", 3, Cmd::WriteQuitAll),
    ("quit", 1, Cmd::Quit),
    ("qall", 2, Cmd::QuitAll),
    ("quitall", 5, Cmd::QuitAll),
    ("close", 3, Cmd::Quit),
    ("xit", 1, Cmd::WriteQuit),
    ("exit", 3, Cmd::WriteQuit),
    ("xall", 2, Cmd::WriteQuitAll),
    ("edit", 1, Cmd::Edit),
    ("buffer", 1, Cmd::Buffer),
    ("bnext", 2, Cmd::BufferNext),
    ("bprevious", 2, Cmd::BufferPrevious),
    ("bNext", 2, Cmd::BufferPrevious),
    ("bdelete", 2, Cmd::BufferDelete),
    ("tabnext", 4, Cmd::TabNext),
    ("tabprevious", 4, Cmd::TabPrevious),
    ("tabNext", 4, Cmd::TabPrevious),
    ("tabclose", 4, Cmd::TabClose),
    ("tabonly", 4, Cmd::TabOnly),
    ("tabnew", 6, Cmd::TabNew),
    ("tabedit", 4, Cmd::TabNew),
    ("help", 1, Cmd::Help),
];

/// 返回各命令的全名与最短缩写的长度，按表中的顺序；供帮助页列出。
pub fn command_names() -> impl Iterator<Item = (&'static str, usize)> {
    NAMES.iter().map(|(name, min, _)| (*name, *min))
}

/// 解析之后的一条命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line<'a> {
    /// 范围中的地址，按书写的顺序；没有范围时为空。
    pub addresses: Vec<Address>,
    /// 命令。
    pub cmd: Cmd,
    /// 命令名之后是否有 `!`。
    pub bang: bool,
    /// 参数：命令名与 `!` 之后的文本，已去掉开头的空白。
    pub args: &'a str,
}

/// 返回命令名 `name` 对应的命令。
fn lookup(name: &str) -> Option<Cmd> {
    NAMES
        .iter()
        .find(|(full, min, _)| name.len() >= *min && full.starts_with(name))
        .map(|(_, _, cmd)| *cmd)
}

/// 从 `text` 的开头读出一个不带符号的整数，返回其值与余下的文本。
pub(crate) fn number(text: &str) -> Option<(usize, &str)> {
    let digits = text.len() - text.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    // 位数过多时取上限，不报错。
    let value = text[..digits].parse().unwrap_or(usize::MAX);
    Some((value, &text[digits..]))
}

/// 从 `text`（分隔符之后）读出到下一个未转义的 `separator` 为止的模式，返回模式与余下的文本。
///
/// 余下的文本不含结尾的分隔符。向前文查找（分隔符为 `?`）时去掉转义分隔符的反斜杠。
pub(crate) fn delimited(text: &str, separator: char) -> (String, &str) {
    let mut pattern = String::new();
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some((_, next)) if next == separator && separator != '/' => pattern.push(next),
                Some((_, next)) => {
                    pattern.push(c);
                    pattern.push(next);
                }
                None => pattern.push(c),
            }
        } else if c == separator {
            return (pattern, &text[at + c.len_utf8()..]);
        } else {
            pattern.push(c);
        }
    }
    (pattern, "")
}

/// 从 `text` 的开头读出一个地址，返回地址与余下的文本；开头不是地址时返回 `None`。
fn address(text: &str) -> Result<(Option<Address>, &str), Message> {
    let text = text.trim_start_matches([' ', '\t']);
    let mut chars = text.chars();
    let (base, mut rest) = match chars.next() {
        Some('.') => (Some(Base::Current), chars.as_str()),
        Some('$') => (Some(Base::Last), chars.as_str()),
        Some('\'') => match chars.next() {
            Some(name) => (Some(Base::Mark(name)), chars.as_str()),
            None => return Err(Message::BadRange),
        },
        Some(c @ ('/' | '?')) => {
            let (pattern, rest) = delimited(chars.as_str(), c);
            let base = if c == '/' {
                Base::Forward(pattern)
            } else {
                Base::Backward(pattern)
            };
            (Some(base), rest)
        }
        Some('\\') => match chars.next() {
            Some('/' | '&') => (Some(Base::Forward(String::new())), chars.as_str()),
            Some('?') => (Some(Base::Backward(String::new())), chars.as_str()),
            _ => return Err(Message::BadRange),
        },
        Some(c) if c.is_ascii_digit() => {
            let (value, rest) = number(text).unwrap_or((0, text));
            (Some(Base::Number(value)), rest)
        }
        _ => (None, text),
    };
    // 偏移：`+`、`-` 之后的数省略时为 1，可以连写。
    let mut offset: isize = 0;
    let mut signed = false;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        let sign = match trimmed.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => break,
        };
        signed = true;
        let after = &trimmed[1..];
        let (amount, after) = number(after).unwrap_or((1, after));
        let amount = isize::try_from(amount).unwrap_or(isize::MAX);
        offset = offset.saturating_add(sign * amount);
        rest = after;
    }
    let address = match base {
        Some(base) => Some(Address {
            base,
            offset,
            moves: false,
        }),
        None if signed => Some(Address {
            base: Base::Current,
            offset,
            moves: false,
        }),
        None => None,
    };
    Ok((address, rest))
}

/// 从 `text` 的开头读出范围，返回各地址与余下的文本。
fn range(text: &str) -> Result<(Vec<Address>, &str), Message> {
    let trimmed = text.trim_start_matches([' ', '\t', ':']);
    let whole = |first: Base, last: Base| {
        let address = |base| Address {
            base,
            offset: 0,
            moves: false,
        };
        vec![address(first), address(last)]
    };
    if let Some(rest) = trimmed.strip_prefix('%') {
        return Ok((whole(Base::Number(1), Base::Last), rest));
    }
    if let Some(rest) = trimmed.strip_prefix('*') {
        return Ok((whole(Base::Mark('<'), Base::Mark('>')), rest));
    }
    let mut addresses = Vec::new();
    let mut rest = trimmed;
    loop {
        let (found, after) = address(rest)?;
        let after = after.trim_start_matches([' ', '\t']);
        let separator = after.chars().next().filter(|c| matches!(c, ',' | ';'));
        match (found, separator) {
            (None, None) => return Ok((addresses, after)),
            (found, separator) => {
                // 分隔符一侧省略的地址是当前行。
                let mut address = found.unwrap_or(Address {
                    base: Base::Current,
                    offset: 0,
                    moves: false,
                });
                address.moves = separator == Some(';');
                addresses.push(address);
                match separator {
                    Some(_) => {
                        rest = &after[1..];
                        // 分隔符之后没有地址时，范围止于当前行。
                        let next = rest.trim_start_matches([' ', '\t']);
                        if address_starts(next).is_none() {
                            addresses.push(Address {
                                base: Base::Current,
                                offset: 0,
                                moves: false,
                            });
                            return Ok((addresses, next));
                        }
                    }
                    None => return Ok((addresses, after)),
                }
            }
        }
    }
}

/// `text` 的开头是否为一个地址；是时返回其第一个字符。
fn address_starts(text: &str) -> Option<char> {
    text.chars().next().filter(|c| {
        matches!(c, '.' | '$' | '\'' | '/' | '?' | '\\' | '+' | '-') || c.is_ascii_digit()
    })
}

/// 返回由各部分组成的命令，参数去掉开头的空白。
fn line_of(addresses: Vec<Address>, cmd: Cmd, bang: bool, args: &str) -> Line<'_> {
    Line {
        addresses,
        cmd,
        bang,
        args: args.trim_start_matches([' ', '\t']),
    }
}

/// 解析一条命令。
///
/// # Errors
///
/// 范围的写法有误时返回 [`Message::BadRange`]，命令名不存在时返回 [`Message::BadCommand`]。
pub(crate) fn parse(text: &str) -> Result<Line<'_>, Message> {
    let (addresses, rest) = range(text)?;
    let rest = rest.trim_start_matches([' ', '\t', ':']);
    let line = |cmd, bang, args| line_of(addresses.clone(), cmd, bang, args);
    let Some(first) = rest.chars().next() else {
        return Ok(line(Cmd::Goto, false, ""));
    };
    // 由符号组成的命令。
    match first {
        '>' | '<' => {
            let count = rest.len() - rest.trim_start_matches(first).len();
            let cmd = if first == '>' {
                Cmd::ShiftRight(count)
            } else {
                Cmd::ShiftLeft(count)
            };
            return Ok(line(cmd, false, &rest[count..]));
        }
        '&' => return Ok(line(Cmd::RepeatSubstitute, false, &rest[1..])),
        '#' => return Ok(line(Cmd::Print, false, &rest[1..])),
        '|' => return Ok(line(Cmd::Goto, false, rest)),
        _ => {}
    }
    let letters = rest.len()
        - rest
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    let name = &rest[..letters];
    // `:k` 之后可以直接跟标记名，`:s` 之后可以直接跟标志。
    let (cmd, used) = match lookup(name) {
        Some(cmd) => (cmd, letters),
        None if name.starts_with('k') && letters == 2 => (Cmd::Mark, 1),
        None => return Err(Message::BadCommand),
    };
    let rest = &rest[used..];
    let (bang, rest) = match rest.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    Ok(line(cmd, bang, rest))
}

/// 把参数 `args` 在第一个未转义的 `|` 处分开，返回参数与下一条命令。
///
/// 参数中转义的 `|` 去掉反斜杠。
pub(crate) fn split_bar(args: &str) -> (String, Option<&str>) {
    let mut out = String::new();
    let mut chars = args.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '\\' if chars.peek().is_some_and(|(_, next)| *next == '|') => {
                out.push('|');
                chars.next();
            }
            '|' => return (out, Some(&args[at + 1..])),
            c => out.push(c),
        }
    }
    (out, None)
}

/// 补全的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completing {
    /// 命令名。
    Command,
    /// 选项名。
    Option,
    /// 文件路径，由宿主列出。
    Path,
    /// 已打开的文档的名字，由宿主列出。
    Buffer,
}

/// 命令行中光标之前可以补全的部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// 种类。
    pub kind: Completing,
    /// 被补全的部分在命令行中的起点（字节偏移）。
    pub start: usize,
    /// 候选，按名字排序；补全路径时为空。
    pub candidates: Vec<String>,
}

/// 返回命令行 `text`（光标之前的部分）末尾可以补全的内容；没有可补全的内容时返回 `None`。
pub fn complete(text: &str) -> Option<Completion> {
    let (_, rest) = range(text).ok()?;
    let rest = rest.trim_start_matches([' ', '\t', ':']);
    let start = text.len() - rest.len();
    let letters = rest.len()
        - rest
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    if letters == rest.len() {
        // 光标仍在命令名之内。
        let mut candidates: Vec<String> = NAMES
            .iter()
            .filter(|(full, _, _)| full.starts_with(rest) && full.len() > 1)
            .map(|(full, _, _)| (*full).to_owned())
            .collect();
        candidates.sort();
        candidates.dedup();
        return (!candidates.is_empty()).then_some(Completion {
            kind: Completing::Command,
            start,
            candidates,
        });
    }
    let cmd = lookup(&rest[..letters])?;
    let args = rest[letters..].trim_start_matches('!');
    let word = args.rsplit([' ', '\t']).next().unwrap_or("");
    let start = text.len() - word.len();
    if cmd == Cmd::Set {
        let prefix = word.trim_start_matches("no").trim_start_matches("inv");
        let start = text.len() - prefix.len();
        let candidates: Vec<String> = crate::vim::option_names()
            .filter(|name| name.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        return (!candidates.is_empty()).then_some(Completion {
            kind: Completing::Option,
            start,
            candidates,
        });
    }
    let kind = match cmd {
        Cmd::Buffer => Completing::Buffer,
        cmd if cmd.takes_path() => Completing::Path,
        _ => return None,
    };
    Some(Completion {
        kind,
        start,
        candidates: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
