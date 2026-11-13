//! `.editorconfig`：项目约定的缩进、行尾和保存时的清理，按 `EditorConfig` 规范解析。
//!
//! 缩进与制表宽度在打开文件时生效；行尾、删除行尾空白、末尾补换行在保存时执行。从文件所在目录
//! 起逐级向上查找 `.editorconfig`，遇到 `root = true` 为止：离文件越近的优先级越高，同一个文件
//! 内写在后面的小节优先。

use std::ops::Range;
use std::path::{Component, Path};

use regex::Regex;
use winc_core::indent::Indent;
use winc_core::text::LineEnding;

/// 一个文件适用的 `EditorConfig` 设置。
///
/// 未设置的项为 `None`，由编辑器按文件内容或语言的默认值判断。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileConfig {
    indent_style: Option<IndentStyle>,
    indent_size: Option<IndentSize>,
    /// 一个 Tab 显示的列数（列）。
    tab_width: Option<usize>,
    /// 保存时使用的行尾风格。
    pub end_of_line: Option<LineEnding>,
    /// 保存时是否删除行尾空白。
    pub trim_trailing_whitespace: Option<bool>,
    /// 保存时是否在文件末尾补一个换行。
    pub insert_final_newline: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndentStyle {
    Tab,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndentSize {
    Columns(usize),
    /// `indent_size = tab`：缩进宽度取 Tab 的显示宽度。
    Tab,
}

impl FileConfig {
    /// 返回一级缩进；`indent_style` 和 `indent_size` 都没有设置时返回 `None`。
    ///
    /// `fallback` 是从文件内容检测到的缩进（或语言的默认值），只设置了一项时另一项取 `fallback`。
    /// 空格缩进的宽度限制在 1 到 8 列：下限 1 保证每次缩进都产生可辨认的宽度，上限 8 与缩进宽度
    /// 菜单列出的取值范围 `1` 到 `8` 相同；超出范围时按上下限处理，挡住异常的设置值。
    pub fn indent(&self, fallback: Indent) -> Option<Indent> {
        if self.indent_style.is_none() && self.indent_size.is_none() {
            return None;
        }
        let style = self.indent_style.unwrap_or(match fallback {
            Indent::Tabs => IndentStyle::Tab,
            Indent::Spaces(_) => IndentStyle::Space,
        });
        Some(match style {
            IndentStyle::Tab => Indent::Tabs,
            IndentStyle::Space => {
                let width = match self.indent_size {
                    Some(IndentSize::Columns(n)) => n,
                    Some(IndentSize::Tab) => self.tab_width.unwrap_or(4),
                    None => match fallback {
                        Indent::Spaces(n) => n,
                        Indent::Tabs => self.tab_width.unwrap_or(4),
                    },
                };
                Indent::Spaces(width.clamp(1, 8))
            }
        })
    }

    /// 返回一个 Tab 显示的列数（列），限制在 1 到 16 列；没有可用设置时返回 `None`。
    ///
    /// 没有设置 `tab_width` 时取 `indent_size` 的列数；`indent_size = tab` 没有数值，返回 `None`。
    /// 宽度限制在 1 到 16 列：下限 1 保证制表位之间有间隔，上限 16 避免一个 Tab 占满较窄的窗口；
    /// 超出范围时按上下限处理，挡住异常的设置值。
    pub fn tab_width(&self) -> Option<usize> {
        let width = self.tab_width.or(match self.indent_size {
            Some(IndentSize::Columns(n)) => Some(n),
            _ => None,
        })?;
        Some(width.clamp(1, 16))
    }

    /// 是否有保存时需要执行的清理：统一行尾、删除行尾空白或补末尾换行。
    pub fn tidies(&self) -> bool {
        self.end_of_line.is_some()
            || self.trim_trailing_whitespace == Some(true)
            || self.insert_final_newline == Some(true)
    }

    /// 设置一项；`value` 不区分大小写，`unset` 清除该项，无效的值被忽略。
    fn set(&mut self, key: &str, value: &str) {
        let value = value.to_ascii_lowercase();
        let unset = value == "unset";
        let flag = match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        let number = value.parse::<usize>().ok().filter(|&n| n > 0);
        match key {
            "indent_style" => {
                self.indent_style = match value.as_str() {
                    "tab" => Some(IndentStyle::Tab),
                    "space" => Some(IndentStyle::Space),
                    _ if unset => None,
                    _ => return,
                }
            }
            "indent_size" => {
                self.indent_size = match (value.as_str(), number) {
                    ("tab", _) => Some(IndentSize::Tab),
                    (_, Some(n)) => Some(IndentSize::Columns(n)),
                    _ if unset => None,
                    _ => return,
                }
            }
            "tab_width" if number.is_some() || unset => self.tab_width = number,
            "end_of_line" => {
                self.end_of_line = match value.as_str() {
                    "lf" => Some(LineEnding::Lf),
                    "crlf" => Some(LineEnding::CrLf),
                    "cr" => Some(LineEnding::Cr),
                    _ if unset => None,
                    _ => return,
                }
            }
            "trim_trailing_whitespace" if flag.is_some() || unset => {
                self.trim_trailing_whitespace = flag
            }
            "insert_final_newline" if flag.is_some() || unset => self.insert_final_newline = flag,
            _ => {}
        }
    }
}

/// 返回适用于 `path` 的设置。
///
/// 从 `path` 所在目录起逐级向上读取 `.editorconfig`，遇到 `root = true` 为止。无法读取的
/// `.editorconfig` 视为不存在，格式有误的行或小节被忽略。
pub fn lookup(path: &Path) -> FileConfig {
    // 远端的文件只在那台主机的根目录以内查找。各级目录中有没有 `.editorconfig` 一次查完，
    // 只读取存在的：远端通常只需一两次往返，而不是每级目录一次。
    let remote = winc_backend::is_remote(path);
    let dirs: Vec<&Path> = (path.ancestors().skip(1))
        .take_while(|dir| !remote || winc_backend::is_remote(dir))
        .collect();
    let candidates: Vec<std::path::PathBuf> =
        dirs.iter().map(|dir| dir.join(".editorconfig")).collect();
    let present = winc_backend::fs::metadata_many(&candidates);
    let mut files = Vec::new();
    for ((dir, file), present) in dirs.into_iter().zip(&candidates).zip(present) {
        if present.is_err() {
            continue;
        }
        let Ok(text) = winc_backend::fs::read_to_string(file) else {
            continue;
        };
        let parsed = parse(&text);
        let root = parsed.root;
        files.push((dir, parsed));
        if root {
            break;
        }
    }

    let mut config = FileConfig::default();
    for (dir, file) in files.iter().rev() {
        let Some(relative) = relative_path(path, dir) else {
            continue;
        };
        for section in &file.sections {
            if section.matches(&relative) {
                for (key, value) in &section.pairs {
                    config.set(key, value);
                }
            }
        }
    }
    config
}

/// 返回 `path` 相对 `dir` 的路径，以 `/` 分隔。
///
/// `path` 不在 `dir` 之下，或含有非 UTF-8 的部分时返回 `None`。
fn relative_path(path: &Path, dir: &Path) -> Option<String> {
    let rest = path.strip_prefix(dir).ok()?;
    let parts: Option<Vec<&str>> = rest
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}

struct Parsed {
    root: bool,
    sections: Vec<Section>,
}

struct Section {
    glob: Option<Glob>,
    pairs: Vec<(String, String)>,
}

impl Section {
    fn matches(&self, relative: &str) -> bool {
        self.glob.as_ref().is_some_and(|g| g.matches(relative))
    }
}

fn parse(text: &str) -> Parsed {
    let mut root = false;
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push(Section {
                glob: Glob::new(name),
                pairs: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim().to_owned());
        match sections.last_mut() {
            Some(section) => section.pairs.push((key, value)),
            None if key == "root" => root = value.eq_ignore_ascii_case("true"),
            None => {}
        }
    }
    Parsed { root, sections }
}

/// 小节名中的通配模式，按 `EditorConfig` 规范解释。
///
/// - `*` 匹配不含 `/` 的任意字符串，`**` 可以跨目录，`?` 匹配单个非 `/` 字符；
/// - `[abc]`、`[!abc]` 是字符集合及其取反；
/// - `{a,b}` 是备选项，`{1.3}` 是整数范围。
///
/// 不含 `/` 的模式匹配任意一层目录下的文件（如 `*.py`），含 `/` 的模式从 `.editorconfig` 所在
/// 目录起匹配。
struct Glob {
    regex: Regex,
    /// 每个 `{m..n}` 对应正则中的一个捕获组；正则匹配成功后，再检查捕获的整数是否在该范围内。
    ranges: Vec<(i64, i64)>,
}

impl Glob {
    fn new(glob: &str) -> Option<Glob> {
        let mut ranges = Vec::new();
        let body = translate(glob.strip_prefix('/').unwrap_or(glob), &mut ranges);
        let anywhere = !has_separator(glob);
        let pattern = if anywhere {
            format!("^(?:.*/)?{body}$")
        } else {
            format!("^{body}$")
        };
        Some(Glob {
            regex: Regex::new(&pattern).ok()?,
            ranges,
        })
    }

    fn matches(&self, path: &str) -> bool {
        let Some(caps) = self.regex.captures(path) else {
            return false;
        };
        self.ranges.iter().enumerate().all(|(i, &(low, high))| {
            caps.get(i + 1)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .is_some_and(|n| (low..=high).contains(&n))
        })
    }
}

/// 是否在方括号之外含有 `/`。
fn has_separator(glob: &str) -> bool {
    let mut in_class = false;
    for c in glob.chars() {
        match c {
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => return true,
            _ => {}
        }
    }
    false
}

/// 把通配模式转换为正则表达式的主体，`{m.n}` 的范围按顺序追加到 `ranges`。
fn translate(glob: &str, ranges: &mut Vec<(i64, i64)>) -> String {
    let chars: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    // 尚未闭合的 `{` 的栈：元素表示是否为 `{a,b}` 形式的备选项（否则按字面字符处理）。
    let mut braces: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                out.push_str(&regex::escape(&chars[i + 1].to_string()));
                i += 2;
                continue;
            }
            // `**/` 匹配零层或多层目录：`**/` 可以匹配空串，前后的部分因此可以直接相连。
            '*' if chars.get(i + 1) == Some(&'*') && chars.get(i + 2) == Some(&'/') => {
                out.push_str("(?:.*/)?");
                i += 3;
                continue;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push_str(".*");
                i += 2;
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' => match chars[i + 1..].iter().position(|&c| c == ']') {
                Some(len) if !chars[i + 1..i + 1 + len].contains(&'/') => {
                    let inner = &chars[i + 1..i + 1 + len];
                    let (negated, inner) = match inner.split_first() {
                        Some(('!', rest)) => (true, rest),
                        _ => (false, inner),
                    };
                    out.push_str(if negated { "[^/" } else { "[" });
                    for &c in inner {
                        if c == '-' {
                            out.push('-');
                        } else {
                            out.push_str(&regex::escape(&c.to_string()));
                        }
                    }
                    out.push(']');
                    i += len + 2;
                    continue;
                }
                _ => out.push_str(r"\["),
            },
            '{' => match closing_brace(&chars, i) {
                Some(close) => {
                    let inner: String = chars[i + 1..close].iter().collect();
                    if let Some(range) = number_range(&inner) {
                        out.push_str("([+-]?[0-9]+)");
                        ranges.push(range);
                        i = close + 1;
                        continue;
                    }
                    let alternation = top_level_comma(&chars[i + 1..close]);
                    out.push_str(if alternation { "(?:" } else { r"\{" });
                    braces.push(alternation);
                }
                None => out.push_str(r"\{"),
            },
            '}' => match braces.pop() {
                Some(true) => out.push(')'),
                _ => out.push_str(r"\}"),
            },
            ',' if braces.last() == Some(&true) => out.push('|'),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out
}

fn closing_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn top_level_comma(chars: &[char]) -> bool {
    let mut depth = 0;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// 解析 `{m..n}` 中的整数范围，如 `3..12`、`-1..+5`；两端大小颠倒时取较小者为下限。
fn number_range(inner: &str) -> Option<(i64, i64)> {
    let (low, high) = inner.split_once("..")?;
    let parse = |s: &str| {
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<i64>().ok())
            .flatten()
    };
    let (low, high) = (parse(low)?, parse(high)?);
    Some((low.min(high), low.max(high)))
}

/// 按 `config` 清理保存前的文本。
///
/// 删除行尾的空格和 Tab、把行尾统一为 `end_of_line`、末尾缺少换行时补一个（取 `end_of_line`
/// 或 `fallback`）。`keep` 中的行号范围（从 0 起，按顺序排列、互不重叠）里的行不删除行尾空白，
/// 调用方借此保护光标所在的行。`\n`、`\r\n`、`\r` 之外的 Unicode 换行符不算行尾，原样保留。
pub fn tidy(
    text: &str,
    config: &FileConfig,
    fallback: LineEnding,
    keep: &[Range<usize>],
) -> String {
    let trim = config.trim_trailing_whitespace == Some(true);
    // 行号与范围都从小到大排列：游标一路往后移动即可，不必每行检查全部范围。
    let mut keep = keep.iter().peekable();
    let eol = config.end_of_line;
    let mut out = String::with_capacity(text.len() + 2);
    let mut rest = text;
    let mut line = 0;
    let mut last_ending = "";

    while !rest.is_empty() {
        let (content, ending, next) = match rest.find(['\n', '\r']) {
            Some(at) => {
                let len = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
                (&rest[..at], &rest[at..at + len], &rest[at + len..])
            }
            None => (rest, "", ""),
        };
        while keep.next_if(|r| r.end <= line).is_some() {}
        let kept = keep.peek().is_some_and(|r| r.contains(&line));
        let content = if trim && !kept {
            content.trim_end_matches([' ', '\t'])
        } else {
            content
        };
        out.push_str(content);
        let ending = match eol {
            Some(eol) if !ending.is_empty() => eol.as_str(),
            _ => ending,
        };
        out.push_str(ending);
        last_ending = ending;
        rest = next;
        line += 1;
    }

    if config.insert_final_newline == Some(true) && !out.is_empty() && last_ending.is_empty() {
        out.push_str(eol.unwrap_or(fallback).as_str());
    }
    out
}

#[cfg(test)]
mod tests;
