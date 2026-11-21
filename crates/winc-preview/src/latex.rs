//! 把 LaTeX 文档转换成 Markdown，并记下原文各行在结果中的位置（[`LineMap`]）。

mod envs;
mod names;

use std::collections::HashMap;

/// 转换的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Converted {
    /// 转换出的 Markdown 文本。
    pub markdown: String,
    /// 原文各行在 `markdown` 中的位置。
    pub map: LineMap,
}

/// 原文各行在转换结果中开始的行。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineMap {
    /// 原文第 `i` 行（从 0 起）在结果中开始的行。
    pub lines: Vec<usize>,
    /// 结果的行数。
    pub count: usize,
}

impl LineMap {
    /// 原文第 `line` 行（带小数，按行内的比例）对应结果中的第几行（带小数）。
    pub fn line_of(&self, line: f64) -> f64 {
        let last = self.count.saturating_sub(1);
        let at = |i: usize| self.lines.get(i).copied().unwrap_or(last) as f64;
        let i = line.max(0.0).floor();
        let (a, b) = (at(i as usize), at(i as usize + 1));
        a + (b - a).max(0.0) * (line - i)
    }

    /// [`LineMap::line_of`] 的逆运算。
    pub fn source_of(&self, line: f64) -> f64 {
        let line = line.max(0.0);
        // 结果中开始的行不超过 `line` 的最后一行原文。
        let k = self.lines.partition_point(|&l| l as f64 <= line).max(1) - 1;
        let a = self.lines.get(k).copied().unwrap_or(0) as f64;
        let b = (self.lines.get(k + 1).copied()).unwrap_or(self.count.saturating_sub(1)) as f64;
        let within = if b > a {
            ((line - a) / (b - a)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        k as f64 + within
    }
}

/// 转换 LaTeX 原文 `text`；有 `\begin{document}` 时导言区只取标题、作者、日期和定理名。
pub fn to_markdown(text: &str) -> Converted {
    let mut conv = Conv::new(text, 0);
    if text.contains("\\begin{document}") {
        conv.preamble = true;
        conv.bufs.push(Buf::default());
    }
    conv.run(Until::End);
    conv.finish()
}

/// 嵌套层数的上限（层），超过的部分跳过。
const MAX_DEPTH: usize = 100;

/// 一段正文在哪里结束。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Until<'a> {
    /// 原文结束。
    End,
    /// 配对的 `}`。
    Group,
    /// 名为这个的环境的 `\end{…}`。
    Env(&'a str),
}

/// 输出的一层缓冲区。
#[derive(Debug, Default)]
struct Buf {
    text: String,
    /// 当前行是否还没有写正文。
    fresh: bool,
    /// 当前行是否刚写了列表项的标记。
    marker: bool,
    /// 当前行到目前为止是否只有数字。
    digits: bool,
}

/// 列表的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Number,
    Description,
    Bibliography,
}

/// 正在转换的列表：种类和已经写了几项。
#[derive(Debug, Clone, Copy)]
struct List {
    kind: ListKind,
    count: usize,
}

/// 转换的状态。
struct Conv<'a> {
    src: &'a str,
    pos: usize,
    /// 已经记下行号的原文位置。
    seen: usize,
    /// 输出：最底下一层是结果，上面是正在收集的参数。
    bufs: Vec<Buf>,
    /// 结果已写的换行数，即当前是第几行。
    out_lines: usize,
    lines: Vec<usize>,
    /// 结果中每行开头的前缀：列表项内容的缩进、引用块的 `> `。
    prefix: Vec<String>,
    lists: Vec<List>,
    depth: usize,
    /// 是否还在导言区。
    preamble: bool,
    /// 遇到了 `\end{document}`，后面的不再转换。
    stopped: bool,
    title: Option<String>,
    author: Option<String>,
    date: Option<String>,
    notes: Vec<String>,
    /// `\newtheorem` 定义的环境和它的标题。
    theorems: HashMap<String, String>,
}

impl<'a> Conv<'a> {
    fn new(src: &'a str, depth: usize) -> Conv<'a> {
        Conv {
            src,
            pos: 0,
            seen: 0,
            bufs: vec![Buf {
                fresh: true,
                ..Buf::default()
            }],
            out_lines: 0,
            lines: vec![0],
            prefix: Vec::new(),
            lists: Vec::new(),
            depth,
            preamble: false,
            stopped: false,
            title: None,
            author: None,
            date: None,
            notes: Vec::new(),
            theorems: HashMap::new(),
        }
    }

    /// 把一小段原文（表格的单元格）转换成一行文字。
    fn fragment(&self, text: &str) -> String {
        if self.depth >= MAX_DEPTH {
            return escape(text);
        }
        let mut conv = Conv::new(text, self.depth + 1);
        conv.theorems.clone_from(&self.theorems);
        conv.run(Until::End);
        one_line(&conv.bufs.swap_remove(0).text)
    }

    fn finish(mut self) -> Converted {
        self.bufs.truncate(1);
        self.prefix.clear();
        if !self.notes.is_empty() {
            self.blank_line();
            for (i, note) in std::mem::take(&mut self.notes).iter().enumerate() {
                self.push_raw(&format!("[^{}]: {note}", i + 1));
                self.newline_out();
            }
        }
        let total = self.src.matches('\n').count() + 1;
        self.lines.resize(total, self.out_lines);
        let markdown = self.bufs.swap_remove(0).text;
        let count = markdown.matches('\n').count() + 1;
        Converted {
            markdown,
            map: LineMap {
                lines: self.lines,
                count,
            },
        }
    }

    // ---- 读原文 ----

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.advance_to(self.pos + c.len_utf8());
        Some(c)
    }

    /// 读到 `to` 为止，经过的换行记下原文这一行在结果中开始的行。
    fn advance_to(&mut self, to: usize) {
        let to = to.clamp(self.pos, self.src.len());
        for (i, b) in self.src.as_bytes()[self.pos..to].iter().enumerate() {
            let at = self.pos + i;
            if *b == b'\n' && at >= self.seen {
                self.seen = at + 1;
                self.lines.push(self.out_lines);
            }
        }
        self.pos = to;
    }

    fn skip_blank(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }

    /// 跳过空格和制表符之后是不是 `c`。
    fn next_is(&self, c: char) -> bool {
        self.rest().trim_start_matches([' ', '\t']).starts_with(c)
    }

    /// `open` 处的 `{` 配对的 `}` 之后的位置。
    fn group_end(&self, open: usize) -> Option<usize> {
        self.close_of(open, b'{', b'}')
    }

    /// 返回 `open` 处的开括号 `l` 配对的 `r` 之后的位置。
    fn close_of(&self, open: usize, l: u8, r: u8) -> Option<usize> {
        let bytes = self.src.as_bytes();
        let (mut depth, mut braces, mut i) = (0usize, 0usize, open);
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 1,
                b'%' => {
                    i = self.src[i..].find('\n').map_or(bytes.len(), |n| i + n);
                    continue;
                }
                b'{' if l != b'{' => braces += 1,
                b'}' if l != b'{' => braces = braces.saturating_sub(1),
                b if b == l && braces == 0 => depth += 1,
                b if b == r && braces == 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// 读一个 `[…]` 参数的原文；后面不是 `[` 时为 `None`。
    fn opt_raw(&mut self) -> Option<&'a str> {
        if !self.next_is('[') {
            return None;
        }
        self.skip_blank();
        let end = self.close_of(self.pos, b'[', b']')?;
        let raw = &self.src[self.pos + 1..end - 1];
        self.advance_to(end);
        Some(raw)
    }

    /// 读一个 `{…}` 参数的原文；后面不是 `{` 时为 `None`。
    fn group_raw(&mut self) -> Option<&'a str> {
        if !self.next_is('{') {
            return None;
        }
        self.skip_blank();
        let end = self.group_end(self.pos)?;
        let raw = &self.src[self.pos + 1..end - 1];
        self.advance_to(end);
        Some(raw)
    }

    fn skip_opts(&mut self) {
        while self.opt_raw().is_some() {}
    }

    /// 跳过后面所有的 `[…]` 和 `{…}` 参数。
    fn skip_args(&mut self) {
        while self.opt_raw().is_some() || self.group_raw().is_some() {}
    }

    /// 转换一个 `{…}` 参数，返回转换出的文字；后面不是 `{` 时为 `None`。
    fn group(&mut self) -> Option<String> {
        if !self.next_is('{') {
            return None;
        }
        self.skip_blank();
        self.bump();
        self.bufs.push(Buf::default());
        self.nested(Until::Group);
        self.bufs.pop().map(|b| b.text)
    }

    // ---- 写结果 ----

    fn buf(&mut self) -> &mut Buf {
        self.bufs.last_mut().expect("至少有结果这一层")
    }

    fn fresh(&self) -> bool {
        self.bufs.last().is_some_and(|b| b.fresh)
    }

    /// 换行，并写上每行的前缀。
    fn newline_out(&mut self) {
        let main = self.bufs.len() == 1;
        let prefix = if main {
            self.prefix.concat()
        } else {
            String::new()
        };
        let buf = self.buf();
        buf.text.push('\n');
        buf.text.push_str(&prefix);
        (buf.fresh, buf.marker, buf.digits) = (true, false, false);
        if main {
            self.out_lines += 1;
        }
    }

    /// 原样写入 `s`，换行按 [`Conv::newline_out`]。
    fn push_raw(&mut self, s: &str) {
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                self.newline_out();
            }
            if !part.is_empty() {
                let buf = self.buf();
                buf.text.push_str(part);
                (buf.fresh, buf.marker, buf.digits) = (false, false, false);
            }
        }
    }

    /// 写一个正文的字：Markdown 的记号字符转义。
    fn push_char(&mut self, c: char) {
        let buf = self.buf();
        let fresh = buf.fresh;
        match c {
            // `\[` 会被当作公式，方括号写成字符引用。
            '[' => buf.text.push_str("&#91;"),
            ']' => buf.text.push_str("&#93;"),
            '*' | '_' | '`' | '<' | '>' | '|' | '#' | '$' | '~' | '&' | '\\' => {
                buf.text.push('\\');
                buf.text.push(c);
            }
            '-' | '+' | '=' if fresh => {
                buf.text.push('\\');
                buf.text.push(c);
            }
            '.' | ')' if buf.digits => {
                buf.text.push('\\');
                buf.text.push(c);
            }
            _ => buf.text.push(c),
        }
        buf.digits = c.is_ascii_digit() && (fresh || buf.digits);
        (buf.fresh, buf.marker) = (false, false);
    }

    fn push_text(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\n' {
                self.newline_out();
            } else {
                self.push_char(c);
            }
        }
    }

    /// 写一个空格：行首和已有空白之后不写。
    fn space(&mut self) {
        let buf = self.buf();
        if !buf.fresh && !buf.text.ends_with([' ', '\n', '\u{a0}']) {
            buf.text.push(' ');
        }
    }

    /// 回到行首：已有正文时换行，否则按现在的前缀重写这一行。
    fn line_start(&mut self) {
        if !self.fresh() || self.bufs.last().is_some_and(|b| b.marker) {
            self.newline_out();
            return;
        }
        let main = self.bufs.len() == 1;
        let prefix = if main {
            self.prefix.concat()
        } else {
            String::new()
        };
        let buf = self.buf();
        let start = buf.text.rfind('\n').map_or(0, |i| i + 1);
        buf.text.truncate(start);
        buf.text.push_str(&prefix);
    }

    /// 当前行还没有正文时并回上一行的末尾。
    fn join_line(&mut self) {
        if !self.fresh() {
            return;
        }
        let main = self.bufs.len() == 1;
        let buf = self.buf();
        let Some(nl) = buf.text.rfind('\n') else {
            return;
        };
        buf.text.truncate(nl);
        (buf.fresh, buf.marker) = (false, false);
        if main {
            self.out_lines -= 1;
        }
    }

    /// 另起一段：回到行首，不在列表里时前面空一行。
    fn blank_line(&mut self) {
        self.line_start();
        if !self.lists.is_empty() {
            return;
        }
        let text = &self.bufs.last().expect("至少有结果这一层").text;
        let Some(nl) = text.rfind('\n') else {
            return;
        };
        let previous = text[..nl].rsplit('\n').next().unwrap_or_default();
        if !previous
            .trim_matches(|c: char| c.is_whitespace() || c == '>')
            .is_empty()
        {
            self.newline_out();
        }
    }

    // ---- 正文 ----

    /// 转换正文，直到 `until`。
    fn run(&mut self, until: Until<'a>) {
        while let Some(c) = self.peek() {
            if self.stopped {
                return;
            }
            match c {
                '}' => {
                    self.bump();
                    if until == Until::Group {
                        return;
                    }
                }
                '{' => {
                    self.bump();
                    self.nested(Until::Group);
                }
                '%' => self.comment(),
                '\\' => {
                    if self.control(until) {
                        return;
                    }
                }
                '$' => self.dollar(),
                '\n' => {
                    // 只有空行才分段。
                    let from = self.src[..self.pos].rfind('\n').map_or(0, |i| i + 1);
                    let blank = self.src[from..self.pos].trim().is_empty();
                    let marker = self.bufs.last().is_some_and(|b| b.marker);
                    if !self.fresh() || (blank && !marker) {
                        self.newline_out();
                    }
                    self.bump();
                }
                ' ' | '\t' | '\r' => {
                    self.bump();
                    self.space();
                }
                '~' => {
                    self.bump();
                    self.push_char('\u{a0}');
                }
                '`' | '\'' => self.quote(c),
                '-' => self.dash(),
                // 表格外的对齐符和宏的参数不印出。
                '&' | '#' => {
                    self.bump();
                }
                _ => {
                    self.bump();
                    self.push_char(c);
                }
            }
        }
    }

    /// 转换嵌套的一层（`{` 已读过）；超过 [`MAX_DEPTH`] 时跳过这一层。
    fn nested(&mut self, until: Until<'a>) {
        if self.depth >= MAX_DEPTH {
            let end = match until {
                Until::Group => self.group_end(self.pos - 1),
                _ => None,
            };
            self.advance_to(end.unwrap_or(self.src.len()));
            return;
        }
        self.depth += 1;
        self.run(until);
        self.depth -= 1;
    }

    /// 跳过 `%` 注释及其后的换行；下一行是空行时保留换行。
    fn comment(&mut self) {
        let eol = self
            .rest()
            .find('\n')
            .map_or(self.src.len(), |i| self.pos + i);
        let next = self.src[eol..].strip_prefix('\n').unwrap_or_default();
        let blank_next = next
            .split('\n')
            .next()
            .unwrap_or_default()
            .trim()
            .is_empty();
        if eol == self.src.len() || blank_next {
            self.advance_to(eol);
            return;
        }
        self.advance_to(eol + 1);
        self.skip_blank();
    }

    /// 引号：两个 `` ` `` 和两个 `'` 是双引号，一个是单引号。
    fn quote(&mut self, c: char) {
        self.bump();
        let double = self.peek() == Some(c);
        if double {
            self.bump();
        }
        let shown = match (c, double) {
            ('`', true) => "“",
            ('`', false) => "‘",
            (_, true) => "”",
            _ => "’",
        };
        self.push_text(shown);
    }

    /// 连字符：`--` 是短破折号，`---` 是长破折号。
    fn dash(&mut self) {
        let run = self.rest().bytes().take_while(|&b| b == b'-').count();
        let (shown, used) = match run {
            1 => ("-", 1),
            2 => ("–", 2),
            _ => ("—", 3),
        };
        self.advance_to(self.pos + used);
        self.push_text(shown);
    }

    /// 把 `$…$`、`$$…$$` 写成 `\(…\)`、`\[…\]`。
    fn dollar(&mut self) {
        let display = self.rest().starts_with("$$");
        let (open, close) = if display { (2, "$$") } else { (1, "$") };
        match self.find_close(self.pos + open, close) {
            Some(end) if !self.src[self.pos + open..end].trim().is_empty() => {
                self.advance_to(self.pos + open);
                let (l, r) = if display {
                    ("\\[", "\\]")
                } else {
                    ("\\(", "\\)")
                };
                self.math(l, end, r);
                self.advance_to(end + close.len());
            }
            _ => {
                self.bump();
                self.push_char('$');
            }
        }
    }

    /// 从 `from` 起找收尾的 `close`：跳过转义的字，遇到空行（公式没有收尾）时为 `None`。
    fn find_close(&self, from: usize, close: &str) -> Option<usize> {
        let bytes = self.src.as_bytes();
        let mut i = from;
        while i < bytes.len() {
            if self.src.get(i..).is_some_and(|s| s.starts_with(close)) {
                return Some(i);
            }
            match bytes[i] {
                b'\\' => i += 2,
                b'\n' if blank_from(&self.src[i + 1..]) => return None,
                _ => i += 1,
            }
        }
        None
    }

    /// 写一个公式：`open`，原文到 `end` 为止（去掉 `%` 注释），`close`。
    fn math(&mut self, open: &str, end: usize, close: &str) {
        self.push_raw(open);
        while self.pos < end {
            let rest = &self.src[self.pos..end];
            let cut = rest.find(['\n', '%', '\\']).map_or(end, |i| self.pos + i);
            self.push_raw(&self.src[self.pos..cut]);
            self.advance_to(cut);
            match self.peek().filter(|_| self.pos < end) {
                Some('\n') => {
                    self.newline_out();
                    self.bump();
                }
                Some('%') => {
                    let eol = self.src[self.pos..end]
                        .find('\n')
                        .map_or(end, |i| self.pos + i);
                    self.advance_to(eol);
                }
                Some('\\') => {
                    let next = self.src[self.pos + 1..]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                    let to = (self.pos + 1 + next).min(end);
                    self.push_raw(&self.src[self.pos..to]);
                    self.advance_to(to);
                }
                _ => {}
            }
        }
        self.push_raw(close);
    }

    /// 读到 `\` 开始的控制序列；返回是否到了 `until` 的结尾。
    fn control(&mut self, until: Until<'a>) -> bool {
        let start = self.pos;
        self.bump();
        let Some(c) = self.peek() else {
            return false;
        };
        if c.is_ascii_alphabetic() {
            let len = self
                .rest()
                .bytes()
                .take_while(u8::is_ascii_alphabetic)
                .count();
            let mut end = self.pos + len;
            if self.src[end..].starts_with('*') {
                end += 1;
            }
            let name = &self.src[self.pos..end];
            self.advance_to(end);
            return self.command(name, until, start);
        }
        self.bump();
        match c {
            '\\' => self.hard_break(),
            '(' | '[' => {
                let (open, close) = if c == '(' {
                    ("\\(", "\\)")
                } else {
                    ("\\[", "\\]")
                };
                match self.find_close(self.pos, close) {
                    Some(end) => {
                        self.math(open, end, close);
                        self.advance_to(end + close.len());
                    }
                    None => self.push_char(c),
                }
            }
            '\'' | '`' | '^' | '"' | '~' | '=' | '.' => self.accent(c),
            ',' => self.push_char('\u{2009}'),
            ' ' | ';' | ':' | '>' => self.space(),
            '\n' => self.space(),
            '!' | '/' | '-' | '@' => {}
            _ => self.push_char(c),
        }
        false
    }

    /// 写一个硬换行（`\\`、`\newline`）。
    fn hard_break(&mut self) {
        if self.peek() == Some('*') {
            self.bump();
        }
        self.skip_opts();
        if self.fresh() {
            return;
        }
        self.push_raw("  ");
        let rest = self.rest().trim_start_matches([' ', '\t', '\r']);
        if !(rest.is_empty() || rest.starts_with('\n')) {
            self.newline_out();
            self.skip_blank();
        }
    }

    /// 重音：`\'e`、`\"{o}`。
    fn accent(&mut self, mark: char) {
        let base = match self.group_raw() {
            Some(raw) => raw.to_owned(),
            None => match self.bump() {
                Some('\\') => {
                    let len = self
                        .rest()
                        .bytes()
                        .take_while(u8::is_ascii_alphabetic)
                        .count();
                    let name = &self.src[self.pos..self.pos + len];
                    self.advance_to(self.pos + len);
                    format!("\\{name}")
                }
                Some(c) => c.to_string(),
                None => String::new(),
            },
        };
        self.push_text(&names::accent(mark, &base));
    }

    /// 用 `open`、`close` 包住 `inner` 写入，两头的空白留在外面。
    fn wrap(&mut self, inner: &str, open: &str, close: &str) {
        let trimmed = inner.trim();
        if trimmed.is_empty() {
            if !inner.is_empty() {
                self.space();
            }
            return;
        }
        if inner.starts_with(char::is_whitespace) {
            self.space();
        }
        self.push_raw(open);
        self.push_raw(trimmed);
        self.push_raw(close);
        if inner.ends_with(char::is_whitespace) {
            self.space();
        }
    }

    /// 写一个标题：另起一行，标题写成一行。
    fn heading(&mut self, level: usize) {
        self.skip_opts();
        let title = self.group().unwrap_or_default();
        self.blank_line();
        self.push_raw(&"#".repeat(level));
        self.push_raw(" ");
        self.push_raw(&one_line(&title));
        self.newline_out();
    }

    /// 处理命令 `\name`（`start` 是 `\` 的位置）；返回是否到了 `until` 的结尾。
    fn command(&mut self, name: &'a str, until: Until<'a>, start: usize) -> bool {
        match name {
            "begin" => {
                if let Some(env) = self.group_raw() {
                    self.environment(env.trim(), start);
                }
            }
            "end" => {
                let env = self.group_raw().unwrap_or_default().trim();
                if until == Until::Env(env) {
                    return true;
                }
                if env == "document" {
                    self.stopped = true;
                    self.advance_to(self.src.len());
                    return true;
                }
            }
            "part" | "part*" | "chapter" | "chapter*" => self.heading(1),
            "section" | "section*" => self.heading(2),
            "subsection" | "subsection*" => self.heading(3),
            "subsubsection" | "subsubsection*" => self.heading(4),
            "paragraph" | "paragraph*" | "subparagraph" | "subparagraph*" => {
                self.skip_opts();
                let title = self.group().unwrap_or_default();
                self.blank_line();
                self.wrap(&one_line(&title), "**", "**");
                self.space();
            }
            "frametitle" => self.heading(2),
            "title" | "author" | "date" => {
                self.skip_opts();
                let text = self.group().map(|t| one_line(&t));
                match name {
                    "title" => self.title = text,
                    "author" => self.author = text,
                    _ => self.date = text,
                }
            }
            "maketitle" => self.title_block(),
            "textbf" | "mathbf" | "textbf*" => self.styled("**", "**"),
            "textit" | "emph" | "textsl" | "mathit" => self.styled("*", "*"),
            "texttt" => {
                if let Some(inner) = self.group() {
                    self.push_raw(&code_span(&unescape(&one_line(&inner))));
                }
            }
            "underline" | "uline" => self.styled("<u>", "</u>"),
            "sout" | "st" | "xout" => self.styled("~~", "~~"),
            "textsuperscript" => self.styled("<sup>", "</sup>"),
            "textsubscript" => self.styled("<sub>", "</sub>"),
            "textcolor" | "colorbox" | "fcolorbox" => {
                self.skip_opts();
                if name == "fcolorbox" {
                    self.group_raw();
                }
                self.group_raw();
                self.plain();
            }
            "footnote" => {
                self.skip_opts();
                if let Some(text) = self.group() {
                    self.notes.push(one_line(&text));
                    self.push_raw(&format!("[^{}]", self.notes.len()));
                }
            }
            "href" => {
                let url = self.group_raw().map(unescape_url).unwrap_or_default();
                let text = self.group().unwrap_or_default();
                self.push_raw(&format!("[{}](<{url}>)", one_line(&text)));
            }
            "url" | "nolinkurl" => {
                let url = self.group_raw().map(unescape_url).unwrap_or_default();
                self.push_raw(&format!("[{}](<{url}>)", escape(&url)));
            }
            "cite" | "citep" | "parencite" | "autocite" | "cite*" | "citep*" | "footcite" => {
                self.skip_opts();
                let keys = self.group_raw().map(keys).unwrap_or_default();
                self.push_raw(&format!("&#91;{}&#93;", escape(&keys)));
            }
            "citet" | "textcite" | "citet*" | "citeauthor" | "citeyear" => {
                self.skip_opts();
                let keys = self.group_raw().map(keys).unwrap_or_default();
                self.push_text(&keys);
            }
            "ref" | "autoref" | "cref" | "Cref" | "pageref" | "nameref" | "vref" => {
                let key = self.group_raw().unwrap_or_default();
                self.push_text(key.trim());
            }
            "eqref" => {
                let key = self.group_raw().unwrap_or_default();
                self.push_text(&format!("({})", key.trim()));
            }
            "includegraphics" | "includegraphics*" => {
                self.skip_opts();
                if let Some(path) = self.group_raw() {
                    self.push_raw(&format!("![](<{}>)", path.trim()));
                }
            }
            "caption" | "caption*" => {
                self.skip_opts();
                let text = self.group().unwrap_or_default();
                self.blank_line();
                self.wrap(&one_line(&text), "*", "*");
                self.blank_line();
            }
            "item" => self.item(false),
            "bibitem" => self.item(true),
            "verb" | "verb*" => self.verb(),
            "input" | "include" | "subfile" | "subimport" => {
                if name == "subimport" {
                    self.group_raw();
                }
                if let Some(file) = self.group_raw() {
                    let file = file.trim();
                    let target = match std::path::Path::new(file).extension() {
                        Some(_) => file.to_owned(),
                        None => format!("{file}.tex"),
                    };
                    self.push_raw(&format!("[{}](<{target}>)", escape(file)));
                }
            }
            "newtheorem" | "newtheorem*" => {
                let env = self.group_raw().unwrap_or_default().trim().to_owned();
                self.skip_opts();
                let title = self.group_raw().unwrap_or_default();
                self.skip_opts();
                let title = self.fragment(title);
                self.theorems.insert(env, title);
            }
            // 作者之间的 `\and` 写成逗号。
            "and" => {
                let buf = self.buf();
                buf.text
                    .truncate(buf.text.trim_end_matches([' ', '\t']).len());
                self.push_raw(", ");
            }
            "newline" | "linebreak" => self.hard_break(),
            "par" => self.blank_line(),
            "def" | "gdef" | "edef" | "xdef" => {
                // `\def\name#1#2{…}`：名字、参数格式和定义都不印出。
                let end = self.rest().find('{').map(|i| self.pos + i);
                if let Some(open) = end {
                    let close = self.group_end(open).unwrap_or(self.src.len());
                    self.advance_to(close);
                }
            }
            _ if names::DROP.contains(&name) => self.skip_args(),
            _ if names::SILENT.contains(&name) => {}
            _ => {
                if let Some(text) = names::symbol(name) {
                    // 跳过紧跟的 `{}`。
                    if self.rest().starts_with("{}") {
                        self.advance_to(self.pos + 2);
                    }
                    self.push_text(text);
                } else if let Some(mark) = names::letter_accent(name).filter(|_| self.next_is('{'))
                {
                    self.accent(mark);
                } else {
                    self.unknown();
                }
            }
        }
        false
    }

    /// 带样式的命令（`\textbf{…}`）：转换参数，用 Markdown 的记号包起来。
    fn styled(&mut self, open: &str, close: &str) {
        self.skip_opts();
        if let Some(inner) = self.group() {
            self.wrap(&inner, open, close);
        }
    }

    /// 只留参数文字的命令（`\mbox{…}`）。
    fn plain(&mut self) {
        if self.next_is('{') {
            self.skip_blank();
            self.bump();
            self.nested(Until::Group);
        }
    }

    /// 识别不了的命令：只留最后一个 `{…}` 参数的文字。
    fn unknown(&mut self) {
        let (mut at, mut last) = (self.pos, None);
        loop {
            let skipped =
                self.src[at..].len() - self.src[at..].trim_start_matches([' ', '\t']).len();
            let open = at + skipped;
            let end = match self.src.as_bytes().get(open) {
                Some(b'[') => self.close_of(open, b'[', b']'),
                Some(b'{') => {
                    last = Some(open);
                    self.group_end(open)
                }
                _ => None,
            };
            match end {
                Some(end) => at = end,
                None => break,
            }
        }
        if let Some(open) = last {
            self.advance_to(open + 1);
            self.nested(Until::Group);
        }
    }

    /// `\verb|…|`：行内代码，定界符是 `verb` 后面的第一个字。
    fn verb(&mut self) {
        let Some(delim) = self.bump() else {
            return;
        };
        let rest = self.rest();
        let Some(len) = rest.find(delim).filter(|&i| !rest[..i].contains('\n')) else {
            return;
        };
        let code = &rest[..len];
        self.advance_to(self.pos + len + delim.len_utf8());
        self.push_raw(&code_span(code));
    }

    /// `\maketitle`：标题、作者和日期各成一段，标题是一级标题。
    fn title_block(&mut self) {
        let (title, author, date) = (self.title.clone(), self.author.clone(), self.date.clone());
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            self.blank_line();
            self.push_raw(&format!("# {title}"));
            self.newline_out();
        }
        for line in [author, date]
            .into_iter()
            .flatten()
            .filter(|t| !t.is_empty())
        {
            self.blank_line();
            self.push_raw(&line);
            self.newline_out();
        }
    }
}

/// `text` 里一行（行首到下一个换行）是不是空行。
fn blank_from(text: &str) -> bool {
    text.split('\n')
        .next()
        .unwrap_or_default()
        .trim()
        .is_empty()
}

/// 把转换好的几行文字合成一行：换行和连续的空白都换成一个空格。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 按正文的规则转义 `text`（不在行首）。
fn escape(text: &str) -> String {
    let mut conv = Conv::new("", MAX_DEPTH);
    conv.buf().fresh = false;
    for c in text.chars() {
        conv.push_char(c);
    }
    conv.bufs.swap_remove(0).text
}

/// 去掉 [`escape`] 加的转义。
fn unescape(text: &str) -> String {
    let text = text.replace("&#91;", "[").replace("&#93;", "]");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            continue;
        }
        out.push(c);
    }
    out
}

/// `\cite{a, b}` 的键：逗号分隔，去掉空白。
fn keys(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 网址里 LaTeX 的转义（`\%`、`\#`、`\_`、`\&`、`\~`）还原成原来的字。
fn unescape_url(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|n| "%#_&~$".contains(*n)) {
            continue;
        }
        out.push(c);
    }
    out.replace(['<', '>', ' '], "")
}

/// 行内代码：反引号比代码里最长的一串反引号多一个。
fn code_span(code: &str) -> String {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

#[cfg(test)]
mod tests;
