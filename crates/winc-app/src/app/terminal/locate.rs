//! 认出输出里的「文件:行」和网址：F8 在失败命令的报错之间跳、按住 ⌘ 时的链接都用它。
//!
//! 认这几种写法：`路径:行:列`、`路径:行`（`rustc` 的 `--> src/x.rs:3:5`，`gcc`、`grep` 的
//! `文件:行:列`）、`路径(行,列)`（`tsc`、`msbuild`）、`File "路径", line 行`（Python 调用栈）。
//! 路径里至少要有一个 `/` 或 `.`，避免把时间（`12:30:45`）、`localhost:8080` 当成文件；文件是否
//! 存在由调用方按命令运行时的目录查，链接还认不带行号的路径。

use winc_pty::protocol::Line;
use winc_pty::width::cells as char_cells;

/// 一行里认出来的一处位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Location {
    /// 在这一行里从第几格开始。
    pub start: usize,
    /// 结束格（不含）。
    pub end: usize,
    /// 路径，原样（相对路径按命令运行时的目录解析）。
    pub path: String,
    /// 行号，从 1 起；不带行号的路径为 `None`。
    pub line: Option<usize>,
    /// 列号，从 1 起；没写时为 `None`。
    pub col: Option<usize>,
}

/// 路径里可以有的字符（不含空格：带空格的路径认不出来，宁可漏掉也不乱认）。
fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '+' | '@' | '\\')
}

/// 一行里的字和各自的起始格。
fn cells(line: &Line) -> (Vec<char>, Vec<usize>, usize) {
    let mut text = Vec::with_capacity(line.text.len());
    let mut cols = Vec::with_capacity(line.text.len());
    let mut col = 0;
    for (c, w) in char_cells(&line.text) {
        if w == 0 {
            continue;
        }
        text.push(c);
        cols.push(col);
        col += w;
    }
    (text, cols, col)
}

/// 一行里所有的位置，按出现的顺序。
///
/// `bare` 为真时也认不带行号的路径。
pub(crate) fn locations(line: &Line, bare: bool) -> Vec<Location> {
    let (text, cols, cell_end) = cells(line);
    let col_of = |i: usize| cols.get(i).copied().unwrap_or(cell_end);

    let mut out: Vec<Location> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if !path_char(text[i]) || (i > 0 && path_char(text[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < text.len() && path_char(text[end]) {
            end += 1;
        }
        // 句末的点不算路径（`see a.rs.`）。
        let path_end = trim_dots(&text, start, end);
        let path: String = text[start..path_end].iter().collect();
        if path_end == end
            && (path.contains('/') || path.contains('.'))
            && !path.ends_with('/')
            && let Some((line_no, col_no, stop)) = position(&text, end)
        {
            out.push(Location {
                start: col_of(start),
                end: col_of(stop),
                path,
                line: Some(line_no),
                col: col_no,
            });
            i = stop;
            continue;
        }
        if bare && (path.contains('/') || path.contains('.')) && path != "." && path != ".." {
            out.push(Location {
                start: col_of(start),
                end: col_of(path_end),
                path,
                line: None,
                col: None,
            });
        }
        i = end;
    }
    // 扫出来的本来就按位置排好；Python 的按位置插进去，同一处（不带行号的路径）换成它。
    for found in python(&text, &col_of) {
        let at = out.partition_point(|l| l.start < found.start);
        match out.get_mut(at) {
            Some(same) if same.start == found.start => *same = found,
            _ => out.insert(at, found),
        }
    }
    out
}

/// 一行里的网址：（起始格，结束格，网址）。
///
/// 认 `http://`、`https://`、`file://` 开头的，到空白、
/// 引号、尖括号为止；末尾的标点和没配对的右括号不算。
pub(crate) fn urls(line: &Line) -> Vec<(usize, usize, String)> {
    let (text, cols, cell_end) = cells(line);
    let col_of = |i: usize| cols.get(i).copied().unwrap_or(cell_end);
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let starts = ["https://", "http://", "file://"]
            .iter()
            .any(|p| rest.iter().take(p.len()).copied().eq(p.chars()));
        if !starts || (i > 0 && text[i - 1].is_alphanumeric()) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < text.len()
            && !text[end].is_whitespace()
            && !matches!(text[end], '"' | '\'' | '<' | '>' | '`')
        {
            end += 1;
        }
        while let Some(&last) = text[i..end].last() {
            let unmatched = |open: char, close: char| {
                last == close
                    && text[i..end].iter().filter(|&&c| c == close).count()
                        > text[i..end].iter().filter(|&&c| c == open).count()
            };
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?')
                || unmatched('(', ')')
                || unmatched('[', ']')
            {
                end -= 1;
            } else {
                break;
            }
        }
        let url: String = text[i..end].iter().collect();
        if url.len() > 8 {
            out.push((col_of(i), col_of(end), url));
        }
        i = end.max(i + 1);
    }
    out
}

fn trim_dots(text: &[char], start: usize, mut end: usize) -> usize {
    while end > start + 1 && text[end - 1] == '.' {
        end -= 1;
    }
    end
}

/// 路径后面紧跟着的行号和列号：`:3:5`、`:3`、`(3,5)`、`(3)`。
///
/// 返回（行，列，到哪里为止）。
fn position(text: &[char], at: usize) -> Option<(usize, Option<usize>, usize)> {
    let number = |from: usize| -> Option<(usize, usize)> {
        let digits = text[from.min(text.len())..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let value: String = text[from..from + digits].iter().collect();
        let value = value.parse().ok().filter(|&v: &usize| v > 0)?;
        Some((value, from + digits))
    };
    match text.get(at)? {
        ':' => {
            let (line, after) = number(at + 1)?;
            if text.get(after) == Some(&':')
                && let Some((col, stop)) = number(after + 1)
            {
                return Some((line, Some(col), stop));
            }
            Some((line, None, after))
        }
        '(' => {
            let (line, after) = number(at + 1)?;
            match text.get(after)? {
                ')' => Some((line, None, after + 1)),
                ',' => {
                    let (col, stop) = number(after + 1)?;
                    (text.get(stop) == Some(&')')).then_some((line, Some(col), stop + 1))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Python 的 `File "路径", line 3`。
fn python(text: &[char], col_of: &impl Fn(usize) -> usize) -> Vec<Location> {
    const HEAD: &str = "File \"";
    const LINE: &str = "\", line ";
    let s: String = text.iter().collect();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = s[from..].find(HEAD) {
        let head = from + found;
        let path_start = head + HEAD.len();
        let Some(close) = s[path_start..].find(LINE) else {
            break;
        };
        let path = &s[path_start..path_start + close];
        let digits_start = path_start + close + LINE.len();
        let digits: String = s[digits_start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        from = digits_start;
        if let Ok(line) = digits.parse::<usize>()
            && line > 0
            && !path.is_empty()
            && !path.starts_with('<')
        {
            // 字节偏移换成第几个字，再换成格。
            let index = |byte: usize| s[..byte].chars().count();
            out.push(Location {
                start: col_of(index(path_start)),
                end: col_of(index(path_start) + path.chars().count()),
                path: path.to_string(),
                line: Some(line),
                col: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests;
