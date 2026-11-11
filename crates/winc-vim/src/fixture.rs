//! 对照测试用例与录制结果的文件格式。
//!
//! 用例文件（`.cases`）手工编写，只有输入；录制文件（`.rec`）由 `cargo xtask vim-fixtures`
//! 生成，在每个用例之后记录 `vim` 执行的结果，文件开头记录 `vim` 的版本。两种文件的语法相同：
//!
//! ```text
//! # 注释
//! vim 9.1.1752
//! case delete-two-chars
//! text "abc def"
//! cursor 0 1
//! keys "a2x
//! > text "a def"
//! > cursor 0 1
//! > reg " v "bc"
//! > reg a v "bc"
//! ```
//!
//! 用例可以带 `tag 标记` 行（写在 `keys` 之前）。行为有意与 `vim` 不同的用例在 `keys` 之后以
//! `= ` 开头的行手写期望的结果，语法与 `> ` 开头的录制结果相同。多个用例共用的初始文本可以
//! 先以 `buffer 名字` 加若干 `text` 行定义，用例中以 `use 名字` 引用；结果的文本与初始文本
//! 相同时写作 `> unchanged`。

use std::fmt::Write as _;

/// 录制结果中无法表示的寄存器内容（`vim` 内部的字节，不是 UTF-8）所记的文本；对照测试不比较
/// 内容为该文本的寄存器。
pub const OPAQUE: &str = "\u{fffd}";

/// 文本中的位置：行号与列号，都从 0 起；列号按字符（`char`）计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// 行号。
    pub line: usize,
    /// 行内的字符偏移。
    pub col: usize,
}

/// 寄存器内容的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterKind {
    /// 按字符，记作 `v`。
    Charwise,
    /// 按行，记作 `V`。
    Linewise,
    /// 按块，记作 `b` 加块的宽度（显示列，取 `vim` 报告的值），例如 `b3`。
    Blockwise(usize),
}

/// 一个寄存器的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    /// 寄存器名。
    pub name: char,
    /// 内容的类型。
    pub kind: RegisterKind,
    /// 内容的各行，不含行尾的换行符。
    pub lines: Vec<String>,
}

/// 执行按键之后的状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    /// 文本的各行，不含换行符；至少一行。
    pub text: Vec<String>,
    /// 光标位置。
    pub cursor: Position,
    /// 非空的寄存器，按录制时查询的顺序排列。
    pub registers: Vec<Register>,
}

/// 一个用例：初始文本、光标、按键，以及录制的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Case {
    /// 名字，在文件内唯一，不含空白字符。
    pub name: String,
    /// 初始文本的各行，不含换行符；至少一行。
    pub text: Vec<String>,
    /// 初始的光标位置。
    pub cursor: Position,
    /// 标记，按文件中的顺序排列；`later` 表示所用的命令尚未实现，对照测试跳过该用例。
    pub tags: Vec<String>,
    /// 在普通模式下依次输入的按键（按键记法）。
    pub keys: String,
    /// 手写的期望结果：行为有意与 `vim` 不同时给出，对照测试以该结果为准。
    pub expected: Option<State>,
    /// 录制的结果；用例文件中为 `None`。
    pub result: Option<State>,
}

/// 一个用例文件或录制文件的内容。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    /// 录制所用 `vim` 的版本，例如 `9.1.1752`；用例文件中为 `None`。
    pub vim: Option<String>,
    /// 全部用例，按文件中的顺序排列。
    pub cases: Vec<Case>,
}

/// 解析用例文件或录制文件。
///
/// # Errors
///
/// 语法错误、用例重名或用例缺少按键时返回说明，以「第 N 行」开头（行号从 1 起）。
pub fn parse(source: &str) -> Result<File, String> {
    let mut file = File::default();
    let mut buffers = Buffers::default();
    for (number, line) in source.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        parse_line(&mut file, &mut buffers, line)
            .map_err(|e| format!("第 {} 行：{e}", number + 1))?;
    }
    if let Some(case) = file.cases.iter().find(|c| c.keys.is_empty()) {
        return Err(format!("用例 {} 缺少 keys", case.name));
    }
    for case in &mut file.cases {
        if case.text.is_empty() {
            case.text.push(String::new());
        }
    }
    Ok(file)
}

/// 解析过程中已定义的共用文本，以及正在定义的一个。
#[derive(Default)]
struct Buffers {
    /// 名字与各行。
    defined: Vec<(String, Vec<String>)>,
    /// 最近的 `buffer` 行之后是否还没有出现 `case` 行：此时 `text` 行属于该共用文本。
    open: bool,
}

/// 解析一行并记入 `file`。
fn parse_line(file: &mut File, buffers: &mut Buffers, line: &str) -> Result<(), String> {
    if line.trim().is_empty() || line.starts_with('#') {
        return Ok(());
    }
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    match word {
        "buffer" => {
            let name = rest.trim();
            if name.is_empty() || buffers.defined.iter().any(|(n, _)| n == name) {
                return Err(format!("共用文本的名字为空或重复：{name}"));
            }
            buffers.defined.push((name.to_owned(), Vec::new()));
            buffers.open = true;
        }
        "text" if buffers.open => {
            if let Some((_, lines)) = buffers.defined.last_mut() {
                lines.push(parse_string(rest)?.0);
            }
        }
        "use" => {
            let case = file.cases.last_mut().ok_or("缺少 case 行")?;
            let found = buffers.defined.iter().find(|(name, _)| name == rest);
            let (_, lines) = found.ok_or_else(|| format!("没有名为 {rest} 的共用文本"))?;
            if !case.text.is_empty() || !case.keys.is_empty() {
                return Err("use 写在 keys 之前，且不与 text 同时使用".into());
            }
            case.text.clone_from(lines);
        }
        "vim" if file.cases.is_empty() => file.vim = Some(rest.trim().to_owned()),
        "case" => {
            let name = rest.trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err("用例名不能为空，也不能含空白字符".into());
            }
            if file.cases.iter().any(|c| c.name == name) {
                return Err(format!("用例 {name} 重名"));
            }
            buffers.open = false;
            file.cases.push(Case {
                name: name.to_owned(),
                ..Case::default()
            });
        }
        "text" | "cursor" | "keys" | "tag" | ">" | "=" => {
            let case = file.cases.last_mut().ok_or("缺少 case 行")?;
            parse_field(case, word, rest)?;
        }
        _ => return Err(format!("无法识别的行：{line}")),
    }
    Ok(())
}

/// 解析用例 `case` 之内以 `word` 开头的一行，`rest` 是该行余下的部分。
fn parse_field(case: &mut Case, word: &str, rest: &str) -> Result<(), String> {
    match (word, &mut case.result) {
        ("text", None) if case.keys.is_empty() => case.text.push(parse_string(rest)?.0),
        ("cursor", None) if case.keys.is_empty() => case.cursor = parse_position(rest)?,
        ("keys", None) if case.keys.is_empty() => {
            if rest.is_empty() || rest != rest.trim() {
                return Err("keys 不能为空，首尾的空格写作 <Space>".into());
            }
            rest.clone_into(&mut case.keys);
        }
        ("tag", None) if case.keys.is_empty() => {
            if rest.is_empty() || rest.contains(char::is_whitespace) {
                return Err("标记不能为空，也不能含空白字符".into());
            }
            case.tags.push(rest.to_owned());
        }
        (">" | "=", result) if !case.keys.is_empty() => {
            let result = if word == "=" {
                &mut case.expected
            } else {
                result
            };
            let state = result.get_or_insert_with(State::default);
            let (word, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            match word {
                "unchanged" => {
                    if case.text.is_empty() {
                        state.text.push(String::new());
                    } else {
                        state.text.clone_from(&case.text);
                    }
                }
                "text" => state.text.push(parse_string(rest)?.0),
                "cursor" => state.cursor = parse_position(rest)?,
                "reg" => state.registers.push(parse_register(rest)?),
                _ => return Err(format!("无法识别的结果：{word}")),
            }
        }
        _ => {
            return Err(format!(
                "{word} 出现的位置不对：输入在 keys 之前，结果在 keys 之后"
            ));
        }
    }
    Ok(())
}

/// 解析 `行号 列号`。
fn parse_position(text: &str) -> Result<Position, String> {
    let mut numbers = text.split(' ').map(str::parse::<usize>);
    match (numbers.next(), numbers.next(), numbers.next()) {
        (Some(Ok(line)), Some(Ok(col)), None) => Ok(Position { line, col }),
        _ => Err(format!("位置应为「行号 列号」：{text}")),
    }
}

/// 解析 `名字 类型 "行"…`。
fn parse_register(text: &str) -> Result<Register, String> {
    let mut chars = text.chars();
    let (Some(name), Some(' ')) = (chars.next(), chars.next()) else {
        return Err(format!("寄存器应为「名字 类型 内容」：{text}"));
    };
    let (kind, mut rest) = chars
        .as_str()
        .split_once(' ')
        .unwrap_or((chars.as_str(), ""));
    let kind = match (kind, kind.strip_prefix('b').map(str::parse::<usize>)) {
        ("v", _) => RegisterKind::Charwise,
        ("V", _) => RegisterKind::Linewise,
        (_, Some(Ok(width))) => RegisterKind::Blockwise(width),
        _ => return Err(format!("寄存器类型应为 v、V 或 b 加宽度：{kind}")),
    };
    let mut lines = Vec::new();
    while !rest.is_empty() {
        let (line, tail) = parse_string(rest)?;
        lines.push(line);
        rest = tail.strip_prefix(' ').unwrap_or(tail);
    }
    Ok(Register { name, kind, lines })
}

/// 解析 `text` 开头的一个带引号的字符串，返回内容与余下的文本。
///
/// 转义：`\\`、`\"`、`\t`、`\r`、`\n`，以及 `\u{十六进制码位}`。
fn parse_string(text: &str) -> Result<(String, &str), String> {
    let invalid = || format!("字符串不完整或转义有误：{text}");
    let mut chars = text.strip_prefix('"').ok_or_else(invalid)?.chars();
    let mut value = String::new();
    loop {
        match chars.next().ok_or_else(invalid)? {
            '"' => return Ok((value, chars.as_str())),
            '\\' => value.push(match chars.next().ok_or_else(invalid)? {
                '\\' => '\\',
                '"' => '"',
                't' => '\t',
                'r' => '\r',
                'n' => '\n',
                'u' => {
                    let rest = chars.as_str().strip_prefix('{').ok_or_else(invalid)?;
                    let (digits, tail) = rest.split_once('}').ok_or_else(invalid)?;
                    chars = tail.chars();
                    u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(invalid)?
                }
                _ => return Err(invalid()),
            }),
            c => value.push(c),
        }
    }
}

/// 把 `file` 写成文本；[`parse`] 该文本得到与 `file` 相等的值。
///
/// `file` 中各用例的文本至少有一行、名字与按键符合 [`Case`] 的约定时成立。
pub fn write(file: &File) -> String {
    let mut out = String::new();
    // 向 `String` 写入不会失败。
    let _ = write_file(&mut out, file);
    out
}

/// 把 `file` 写入 `out`。
fn write_file(out: &mut String, file: &File) -> std::fmt::Result {
    if let Some(vim) = &file.vim {
        writeln!(out, "vim {vim}")?;
    }
    // 两个以上的用例使用的多行文本只写一次，按首次出现的顺序编号。
    let mut shared: Vec<&Vec<String>> = Vec::new();
    for (index, case) in file.cases.iter().enumerate() {
        let repeated = file.cases[index + 1..].iter().any(|c| c.text == case.text);
        if case.text.len() > 1 && repeated && !shared.contains(&&case.text) {
            shared.push(&case.text);
        }
    }
    for (index, text) in shared.iter().enumerate() {
        writeln!(out, "\nbuffer b{}", index + 1)?;
        for line in *text {
            writeln!(out, "text {}", quoted(line))?;
        }
    }
    for case in &file.cases {
        writeln!(out, "\ncase {}", case.name)?;
        match shared.iter().position(|text| **text == case.text) {
            Some(index) => writeln!(out, "use b{}", index + 1)?,
            None => write_text(out, "", &case.text)?,
        }
        writeln!(out, "cursor {} {}", case.cursor.line, case.cursor.col)?;
        for tag in &case.tags {
            writeln!(out, "tag {tag}")?;
        }
        writeln!(out, "keys {}", case.keys)?;
        for (prefix, state) in [("= ", &case.expected), ("> ", &case.result)] {
            if let Some(state) = state {
                write_state(out, prefix, state, &case.text)?;
            }
        }
    }
    Ok(())
}

/// 以 `prefix` 为每行的前缀写出各行文本。
fn write_text(out: &mut String, prefix: &str, text: &[String]) -> std::fmt::Result {
    for line in text {
        writeln!(out, "{prefix}text {}", quoted(line))?;
    }
    Ok(())
}

/// 以 `prefix` 为每行的前缀写出状态；文本与初始文本 `initial` 相同时写作 `unchanged`。
fn write_state(
    out: &mut String,
    prefix: &str,
    state: &State,
    initial: &[String],
) -> std::fmt::Result {
    if state.text == initial {
        writeln!(out, "{prefix}unchanged")?;
    } else {
        write_text(out, prefix, &state.text)?;
    }
    writeln!(
        out,
        "{prefix}cursor {} {}",
        state.cursor.line, state.cursor.col
    )?;
    for register in &state.registers {
        write!(out, "{prefix}reg {} ", register.name)?;
        match register.kind {
            RegisterKind::Charwise => out.push('v'),
            RegisterKind::Linewise => out.push('V'),
            RegisterKind::Blockwise(width) => write!(out, "b{width}")?,
        }
        for line in &register.lines {
            write!(out, " {}", quoted(line))?;
        }
        out.push('\n');
    }
    Ok(())
}

/// 返回带引号的 `text`，引号、反斜杠与控制字符写成转义。
fn quoted(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;
