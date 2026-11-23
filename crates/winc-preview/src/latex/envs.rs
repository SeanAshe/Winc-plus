//! LaTeX 环境的转换：列表、引用块、代码、公式、表格、定理类的环境和文献列表。

use super::{Conv, List, ListKind, MAX_DEPTH, Until, escape, names};

/// 原样保留的公式环境。
const MATH: &[&str] = &[
    "equation", "align", "alignat", "gather", "multline", "flalign", "eqnarray",
];

impl<'a> Conv<'a> {
    /// 转换环境 `name` 到 `\end{name}` 为止；`start` 是 `\begin` 的位置。
    pub(super) fn environment(&mut self, name: &'a str, start: usize) {
        let base = name.strip_suffix('*').unwrap_or(name);
        if self.depth >= MAX_DEPTH {
            self.skip_env(name);
            return;
        }
        match base {
            "document" => {
                if std::mem::take(&mut self.preamble) {
                    self.bufs.truncate(1);
                }
            }
            "itemize" => self.list(name, ListKind::Bullet),
            "enumerate" => self.list(name, ListKind::Number),
            "description" => self.list(name, ListKind::Description),
            "thebibliography" => {
                self.group_raw();
                self.blank_line();
                self.push_raw("## References");
                self.newline_out();
                self.list(name, ListKind::Bibliography);
            }
            "quote" | "quotation" | "verse" => self.quote_block(name),
            "abstract" => {
                self.blank_line();
                self.push_raw("**Abstract**");
                self.newline_out();
                self.blank_line();
                self.body(name);
                self.blank_line();
            }
            "verbatim" | "Verbatim" | "lstlisting" | "minted" | "BVerbatim" => {
                let options = self.opt_raw().unwrap_or_default();
                let lang = match base {
                    "minted" => self.group_raw().unwrap_or_default().trim().to_owned(),
                    "lstlisting" => listing_language(options),
                    _ => String::new(),
                };
                self.code(name, &lang);
            }
            "comment" => self.skip_env(name),
            _ if MATH.contains(&base) => self.math_env(name, start),
            "displaymath" | "math" => {
                let close = format!("\\end{{{name}}}");
                let Some(end) = self.rest().find(&close).map(|i| self.pos + i) else {
                    return;
                };
                let (l, r) = if base == "math" {
                    ("\\(", "\\)")
                } else {
                    ("\\[", "\\]")
                };
                self.math(l, end, r);
                self.advance_to(end + close.len());
            }
            "tabular" | "tabularx" | "tabulary" | "longtable" | "tabu" => {
                self.skip_opts();
                // `tabular*`、`tabularx` 先写表格的宽度。
                if name == "tabular*" || base == "tabularx" || base == "tabulary" {
                    self.group_raw();
                }
                let spec = self.group_raw().unwrap_or_default();
                self.table(name, spec);
            }
            "frame" => {
                self.skip_opts();
                if self.next_is('{') {
                    self.heading(2);
                }
                self.body(name);
            }
            _ => {
                let title = self
                    .theorems
                    .get(base)
                    .cloned()
                    .or_else(|| names::THEOREMS.contains(&base).then(|| capitalize(base)));
                match title {
                    Some(title) => self.theorem(name, &title, false),
                    None if base == "proof" => self.theorem(name, "Proof", true),
                    None => {
                        self.skip_opts();
                        for _ in 0..names::env_args(name) {
                            self.group_raw();
                        }
                        self.body(name);
                    }
                }
            }
        }
    }

    /// 转换环境的内容，直到 `\end{name}`。
    fn body(&mut self, name: &'a str) {
        self.depth += 1;
        self.run(Until::Env(name));
        self.depth -= 1;
    }

    /// 跳过环境的内容，直到 `\end{name}`。
    fn skip_env(&mut self, name: &str) {
        let close = format!("\\end{{{name}}}");
        let end = self
            .rest()
            .find(&close)
            .map_or(self.src.len(), |i| self.pos + i + close.len());
        self.advance_to(end);
    }

    /// 转换列表，内容按标记的宽度缩进。
    fn list(&mut self, name: &'a str, kind: ListKind) {
        self.skip_opts();
        self.blank_line();
        self.lists.push(List { kind, count: 0 });
        self.prefix.push(String::new());
        self.body(name);
        self.prefix.pop();
        self.lists.pop();
        self.blank_line();
    }

    /// `\item`、`\bibitem`：列表的一项。
    pub(super) fn item(&mut self, bib: bool) {
        let label = if bib {
            let label = self.opt_raw().map(str::to_owned);
            let key = self.group_raw().unwrap_or_default().trim().to_owned();
            Some(label.unwrap_or(key))
        } else {
            self.opt_raw().map(|raw| self.fragment(raw))
        };
        let Some(list) = self.lists.last_mut() else {
            // 列表外的 `\item` 当作无序列表的一项。
            self.line_start();
            self.push_raw("- ");
            return;
        };
        list.count += 1;
        let (kind, count) = (list.kind, list.count);
        let marker = match (kind, &label) {
            (ListKind::Number, None) => format!("{count}. "),
            _ => "- ".to_owned(),
        };
        // 标记写在外层的缩进处。
        if let Some(top) = self.prefix.last_mut() {
            top.clear();
        }
        self.line_start();
        self.push_raw(&marker);
        if let Some(top) = self.prefix.last_mut() {
            *top = " ".repeat(marker.chars().count());
        }
        match (kind, label) {
            (ListKind::Description, Some(label)) => self.push_raw(&format!("**{label}** ")),
            (ListKind::Bibliography, Some(label)) => {
                self.push_raw(&format!("&#91;{}&#93; ", escape(&label)));
            }
            (_, Some(label)) if !label.is_empty() => self.push_raw(&format!("{label} ")),
            _ => {}
        }
        let buf = self.buf();
        (buf.fresh, buf.marker) = (true, true);
    }

    /// 引用块：每行前面加 `> `。
    fn quote_block(&mut self, name: &'a str) {
        self.blank_line();
        self.prefix.push("> ".to_owned());
        self.line_start();
        self.body(name);
        self.prefix.pop();
        self.blank_line();
    }

    /// 把内容原样写进语言为 `lang` 的围栏代码块。
    fn code(&mut self, name: &str, lang: &str) {
        let close = format!("\\end{{{name}}}");
        let end = self
            .rest()
            .find(&close)
            .map_or(self.src.len(), |i| self.pos + i);
        let mut body = &self.src[self.pos..end];
        // 内容从 `\begin{…}` 的下一行开始，到 `\end{…}` 的上一行为止。
        if let Some(first) = body.find('\n').filter(|&i| body[..i].trim().is_empty()) {
            self.advance_to(self.pos + first + 1);
            body = &self.src[self.pos..end];
        }
        if let Some(last) = body.rfind('\n').filter(|&i| body[i..].trim().is_empty()) {
            body = &body[..last];
        }
        let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        self.blank_line();
        self.push_raw(&format!("{fence}{lang}"));
        for line in body.split('\n') {
            self.newline_out();
            let to = self.pos + line.len();
            let buf = self.buf();
            buf.text.push_str(line);
            buf.fresh = false;
            self.advance_to((to + 1).min(end));
        }
        self.newline_out();
        self.push_raw(&fence);
        self.advance_to((end + close.len()).min(self.src.len()));
        self.blank_line();
    }

    /// 连同 `\begin`、`\end` 原样写入公式环境。
    fn math_env(&mut self, name: &str, start: usize) {
        let close = format!("\\end{{{name}}}");
        let Some(end) = self.rest().find(&close).map(|i| self.pos + i) else {
            return;
        };
        let begin = &self.src[start..self.pos];
        self.math(begin, end, &close);
        self.advance_to(end + close.len());
    }

    /// 转换定理类的环境：加粗的标题之后是内容，证明以 ∎ 结尾。
    fn theorem(&mut self, name: &'a str, title: &str, proof: bool) {
        let note = self.opt_raw().map(|raw| self.fragment(raw));
        self.blank_line();
        let head = match (proof, note) {
            (true, Some(note)) => format!("*{note}.* "),
            (true, None) => format!("*{title}.* "),
            (false, Some(note)) => format!("**{title} ({note}).** "),
            (false, None) => format!("**{title}.** "),
        };
        self.push_raw(&head);
        self.buf().fresh = true;
        self.body(name);
        if proof {
            self.join_line();
            self.space();
            self.push_raw("∎");
        }
        self.blank_line();
    }

    /// 转换表格，第一行作表头；`spec` 是列格式。
    fn table(&mut self, name: &str, spec: &str) {
        let close = format!("\\end{{{name}}}");
        let end = self
            .rest()
            .find(&close)
            .map_or(self.src.len(), |i| self.pos + i);
        let body = &self.src[self.pos..end];
        self.advance_to((end + close.len()).min(self.src.len()));
        let mut rows: Vec<Vec<String>> = Vec::new();
        for raw in split_rows(body) {
            let mut row = Vec::new();
            for cell in raw {
                let span = multicolumn(cell);
                row.push(self.fragment(cell).replace('|', "\\|"));
                row.extend(std::iter::repeat_n(String::new(), span.saturating_sub(1)));
            }
            if row.iter().any(|c| !c.is_empty()) {
                rows.push(row);
            }
        }
        if rows.is_empty() {
            return;
        }
        let aligns = alignments(spec);
        let columns = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(aligns.len())
            .max(1);
        self.blank_line();
        for (i, row) in rows.iter().enumerate() {
            let mut line = String::from("|");
            for c in 0..columns {
                line.push(' ');
                line.push_str(row.get(c).map_or("", String::as_str));
                line.push_str(" |");
            }
            self.push_raw(&line);
            self.newline_out();
            if i == 0 {
                let mut rule = String::from("|");
                for c in 0..columns {
                    rule.push_str(match aligns.get(c) {
                        Some('l') => " :-- |",
                        Some('c') => " :-: |",
                        Some('r') => " --: |",
                        _ => " --- |",
                    });
                }
                self.push_raw(&rule);
                self.newline_out();
            }
        }
        self.blank_line();
    }
}

/// 把表格的原文分成行和单元格。
fn split_rows(body: &str) -> Vec<Vec<&str>> {
    let bytes = body.as_bytes();
    let (mut rows, mut row, mut start, mut depth, mut i) = (Vec::new(), Vec::new(), 0, 0usize, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if bytes.get(i + 1) == Some(&b'\\') && depth == 0 => {
                row.push(&body[start..i]);
                rows.push(std::mem::take(&mut row));
                i += 2;
                // 跳过 `\\[2pt]` 写的行距。
                let rest = &body[i..];
                if rest.trim_start().starts_with('[') {
                    let skipped = rest.len() - rest.trim_start().len();
                    i += skipped + rest.trim_start().find(']').map_or(0, |n| n + 1);
                }
                start = i;
                continue;
            }
            b'\\' => i += 1,
            b'%' => {
                // 注释留给转换时去掉。
                i = body[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'&' if depth == 0 => {
                row.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    row.push(&body[start..]);
    rows.push(row);
    rows
}

/// 单元格 `\multicolumn{n}{…}{…}` 横跨的列数；不是时为 1。
fn multicolumn(cell: &str) -> usize {
    cell.trim()
        .strip_prefix("\\multicolumn{")
        .and_then(|rest| rest.split('}').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(1)
}

/// 返回列格式 `spec` 中各列的对齐（`l`、`c`、`r`，其余为 `-`）。
fn alignments(spec: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut chars = spec.chars().peekable();
    let group = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        let mut text = String::new();
        if chars.peek() != Some(&'{') {
            return text;
        }
        chars.next();
        let mut depth = 1;
        for c in chars.by_ref() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            text.push(c);
        }
        text
    };
    while let Some(c) = chars.next() {
        match c {
            'l' | 'c' | 'r' => out.push(c),
            'p' | 'm' | 'b' | 'X' | 'L' | 'C' | 'R' | 'S' => {
                group(&mut chars);
                out.push(match c {
                    'L' => 'l',
                    'C' => 'c',
                    'R' => 'r',
                    _ => '-',
                });
            }
            '@' | '!' | '>' | '<' => {
                group(&mut chars);
            }
            '*' => {
                let n: usize = group(&mut chars).trim().parse().unwrap_or(1);
                let inner = alignments(&group(&mut chars));
                for _ in 0..n.min(64) {
                    out.extend_from_slice(&inner);
                }
            }
            _ => {}
        }
    }
    out
}

/// `lstlisting` 的选项里写的语言（`language=Python`）。
fn listing_language(options: &str) -> String {
    options
        .split(',')
        .filter_map(|o| o.split_once('='))
        .find(|(k, _)| k.trim() == "language")
        .map(|(_, v)| v.trim().trim_matches(['{', '}']).to_lowercase())
        .unwrap_or_default()
}

/// 首字母大写。
fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
