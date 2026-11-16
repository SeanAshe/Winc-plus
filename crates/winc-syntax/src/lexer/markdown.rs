//! Markdown（CommonMark 与 GFM）的规则：代码块、行首的块结构和行内结构。
//!
//! 只按单行判断，不建立文档树：缩进代码块和跨行的强调不识别；Setext 标题只给下划线行着色，
//! 上面的标题文字要看下一行才能确定，不着色。

use super::{Lexer, LineState, TokenKind, compact, expand, language_at, scan, starts_with};
use crate::lang::fence_language;

impl Lexer<'_> {
    /// 处理 Markdown 代码块的开始行、内部行和结束行，返回下一行开头的状态。
    ///
    /// - 开始行与结束行：缩进之后三个以上反引号或 `~`，结束行用与开始行相同的字符，之后只能有空白。
    /// - 结束行的长度和缩进不检查（CommonMark 要求不短于开始行、缩进不超过 3 个空格），列表项中缩进
    ///   较深的代码块同样能识别。
    /// - 信息字符串指明认得的语言（```` ```rust ````）时，内容按该语言着色，否则整块按字符串着色。
    ///
    /// 这一行不是代码块时返回 `None`。
    pub(super) fn fence(&mut self, state: LineState) -> Option<LineState> {
        let chars = self.chars;
        let n = chars.len();
        let indent = scan(chars, 0, char::is_whitespace);
        let marker = chars.get(indent).copied().filter(|&c| c == '`' || c == '~');
        let run_end = marker.map_or(indent, |m| scan(chars, indent, |c| c == m));
        let is_fence = run_end - indent >= 3;

        if let LineState::Fence {
            tilde,
            language,
            inner,
        } = state
        {
            let close = if tilde { '~' } else { '`' };
            if is_fence
                && marker == Some(close)
                && chars[run_end..].iter().all(|c| c.is_whitespace())
            {
                self.push(indent, n, TokenKind::String);
                return Some(LineState::Normal);
            }
            let Some(code) = language_at(language) else {
                self.push(0, n, TokenKind::String);
                return Some(state);
            };
            let next =
                Lexer::new(code, chars, self.tokens.as_deref_mut(), self.base).run(expand(inner));
            return Some(LineState::Fence {
                tilde,
                language,
                inner: compact(next),
            });
        }

        if !is_fence || state != LineState::Normal {
            return None;
        }
        // 反引号代码块开始行的信息字符串（语言名）中不能有反引号，否则是行内代码。
        if marker == Some('`') && chars[run_end..].contains(&'`') {
            return None;
        }
        self.push(indent, n, TokenKind::String);
        let info: String = chars[run_end..].iter().collect();
        Some(LineState::Fence {
            tilde: marker == Some('~'),
            language: fence_language(&info).unwrap_or(u8::MAX),
            inner: 0,
        })
    }

    /// 处理行首的块结构，返回行内结构开始的位置；整行已处理完时返回下一行开头的状态。
    ///
    /// - ATX 标题：1 到 6 个 `#`，之后是空白或行尾，整行是标题。
    /// - 分隔线：3 个以上相同的 `-`、`*` 或 `_`，可以夹空格；Setext 标题的下划线 `===`。
    /// - 表格：分隔行（`|---|:--:|`）整行按关键字着色；以 `|` 开头的行中 `|` 是列分隔符。
    /// - 链接定义：`[label]: url`。
    /// - 引用的 `>`、列表的 `-`、`*`、`+`、`1.`、`1)`，以及任务列表的 `[ ]`、`[x]`，按关键字着色。
    pub(super) fn markdown_block(&mut self) -> Result<usize, LineState> {
        let chars = self.chars;
        let n = chars.len();
        let mut i = scan(chars, 0, char::is_whitespace);

        loop {
            // 引用可以嵌套（`> > a`），其中可以再有标题和列表。
            if chars.get(i) == Some(&'>') {
                self.push(i, i + 1, TokenKind::Keyword);
                i = scan(chars, i + 1, char::is_whitespace);
                continue;
            }
            break;
        }

        let hashes = scan(chars, i, |c| c == '#') - i;
        if (1..=6).contains(&hashes) && chars.get(i + hashes).is_none_or(|c| c.is_whitespace()) {
            self.push(i, n, TokenKind::Heading);
            return Err(LineState::Normal);
        }

        if chars.get(i) == Some(&'=')
            && chars[i..]
                .iter()
                .rposition(|&c| c != ' ' && c != '\t')
                .is_some_and(|last| chars[i..=i + last].iter().all(|&c| c == '='))
        {
            self.push(i, n, TokenKind::Heading);
            return Err(LineState::Normal);
        }

        if chars[i..].contains(&'|')
            && chars[i..].contains(&'-')
            && chars[i..]
                .iter()
                .all(|&c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
        {
            self.push(i, n, TokenKind::Keyword);
            return Err(LineState::Normal);
        }
        self.table_row = chars.get(i) == Some(&'|');

        if let Some(&mark @ ('-' | '*' | '_')) = chars.get(i)
            && chars[i..]
                .iter()
                .all(|&c| c == mark || c == ' ' || c == '\t')
            && chars[i..].iter().filter(|&&c| c == mark).count() >= 3
        {
            self.push(i, n, TokenKind::Keyword);
            return Err(LineState::Normal);
        }

        if chars.get(i) == Some(&'[')
            && let Some(close) = chars[i..].iter().position(|&c| c == ']')
            && chars.get(i + close + 1) == Some(&':')
        {
            let label_end = i + close + 1;
            self.push(i, label_end, TokenKind::Link);
            let url = scan(chars, label_end + 1, char::is_whitespace);
            let url_end = scan(chars, url, |c| !c.is_whitespace());
            self.push(url, url_end, TokenKind::String);
            return Ok(url_end);
        }

        let marker_end = match chars.get(i) {
            Some('-' | '*' | '+') => Some(i + 1),
            Some(c) if c.is_ascii_digit() => {
                let digits = scan(chars, i, |c| c.is_ascii_digit());
                (digits - i <= 9 && matches!(chars.get(digits), Some('.' | ')')))
                    .then_some(digits + 1)
            }
            _ => None,
        };
        if let Some(end) = marker_end
            && chars.get(end).is_none_or(|c| c.is_whitespace())
        {
            self.push(i, end, TokenKind::Keyword);
            i = scan(chars, end, char::is_whitespace);
            if chars.get(i) == Some(&'[')
                && matches!(chars.get(i + 1), Some(' ' | 'x' | 'X'))
                && chars.get(i + 2) == Some(&']')
            {
                self.push(i, i + 3, TokenKind::Keyword);
                i += 3;
            }
        }

        Ok(i)
    }

    /// 识别 `i` 处的行内结构，返回其后的位置；`i` 处不是行内结构时返回 `None`。
    ///
    /// 依次识别行内代码、反斜杠转义、强调、链接、自动链接、行内 HTML 标签和裸网址。
    pub(super) fn markdown_inline(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        match chars[i] {
            '`' => Some(self.code_span(i)),
            // CommonMark：反斜杠只转义 ASCII 标点，`\*` 不开始强调。
            '\\' if chars.get(i + 1).is_some_and(char::is_ascii_punctuation) => Some(i + 2),
            '*' | '_' | '~' => Some(self.emphasis(i)),
            '=' => Some(self.highlight(i)),
            '[' => self.link(i, i),
            '!' if chars.get(i + 1) == Some(&'[') => self.link(i, i + 1),
            '<' => self.angle(i),
            'h' | 'w' => self.bare_url(i),
            '|' if self.table_row => {
                self.push(i, i + 1, TokenKind::Keyword);
                Some(i + 1)
            }
            _ => None,
        }
    }

    /// 识别 `i` 处开始的 Markdown 行内代码，返回其末尾之后的位置。
    ///
    /// 依据 CommonMark：行内代码以若干个连续反引号开始，以同样个数的连续反引号结束，内部没有
    /// 反斜杠转义。本行内找不到等长的收尾时，开头的反引号串是普通文本。
    fn code_span(&mut self, i: usize) -> usize {
        let chars = self.chars;
        let open = scan(chars, i, |c| c == '`');
        let len = open - i;
        let mut j = open;
        while let Some(k) = chars[j..].iter().position(|&c| c == '`') {
            let run = j + k;
            let run_end = scan(chars, run, |c| c == '`');
            if run_end - run == len {
                self.push(i, run_end, TokenKind::String);
                return run_end;
            }
            j = run_end;
        }
        open
    }

    /// 识别 `i` 处开始的强调（`*a*`、`**a**`、`_a_`、`~~a~~`），返回其后的位置。
    ///
    /// 开头的标记串之后不能是空白，收尾的标记串与开头等长、之前不能是空白；`_` 在单词内部
    /// （`snake_case`）不算。本行内找不到收尾时，标记串是普通文本。
    fn emphasis(&mut self, i: usize) -> usize {
        let chars = self.chars;
        let mark = chars[i];
        let open = scan(chars, i, |c| c == mark);
        let len = open - i;
        let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric());
        if chars.get(open).is_none_or(|c| c.is_whitespace())
            || (mark == '_' && i > 0 && word(chars.get(i - 1)))
        {
            return open;
        }

        let mut j = open;
        while let Some(k) = chars[j..].iter().position(|&c| c == mark) {
            let run = j + k;
            let run_end = scan(chars, run, |c| c == mark);
            if run_end - run == len
                && !chars[run - 1].is_whitespace()
                && !(mark == '_' && word(chars.get(run_end)))
            {
                let kind = match (mark, len) {
                    ('~', _) | (_, 1) => TokenKind::Emphasis,
                    _ => TokenKind::Strong,
                };
                self.push(i, run_end, kind);
                return run_end;
            }
            j = run_end;
        }
        open
    }

    /// 识别 `i` 处开始的 `==高亮==`，返回其后的位置。
    ///
    /// 规则与预览相同：标记是恰好两个 `=`，行内代码中的和被反斜杠转义的不是标记；开头的标记与
    /// 其后最近的标记配对，中间的文字不为空且两端不是空白。不成对时 `=` 是普通文本，后一个标记
    /// 仍可作为开头。只在本行内查找收尾。
    fn highlight(&mut self, i: usize) -> usize {
        let chars = self.chars;
        let run = |at: usize, c: char| chars[at..].iter().take_while(|&&x| x == c).count();
        let marks = run(i, '=');
        if marks != 2 {
            return i + marks;
        }
        let mut j = i + 2;
        while j < chars.len() {
            match chars[j] {
                '\\' if chars.get(j + 1).is_some_and(char::is_ascii_punctuation) => j += 2,
                // 行内代码以等长的反引号串收尾；没有收尾时反引号是普通文本。
                '`' => {
                    let ticks = run(j, '`');
                    let mut k = j + ticks;
                    while k < chars.len() && (chars[k] != '`' || run(k, '`') != ticks) {
                        k += if chars[k] == '`' { run(k, '`') } else { 1 };
                    }
                    j = if k < chars.len() {
                        k + ticks
                    } else {
                        j + ticks
                    };
                }
                '=' if run(j, '=') == 2 => {
                    let inner = &chars[i + 2..j];
                    let blank = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace());
                    if blank(inner.first()) || blank(inner.last()) {
                        break;
                    }
                    self.push(i, j + 2, TokenKind::Emphasis);
                    return j + 2;
                }
                '=' => j += run(j, '='),
                _ => j += 1,
            }
        }
        i + 2
    }

    /// 识别 `bracket` 处 `[` 开始的链接（`[a](url)`、`[a][ref]`），返回其后的位置。
    ///
    /// `start` 是链接的开头，图片包括之前的 `!`；`bracket` 处不是链接时返回 `None`。
    fn link(&mut self, start: usize, bracket: usize) -> Option<usize> {
        let chars = self.chars;
        let mut depth = 0usize;
        let close = (bracket..chars.len()).find(|&k| {
            match chars[k] {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            depth == 0
        })?;
        let (open, closer) = match chars.get(close + 1) {
            Some('(') => ('(', ')'),
            Some('[') => ('[', ']'),
            _ => return None,
        };
        let mut depth = 0usize;
        let end = (close + 1..chars.len()).find(|&k| {
            if chars[k] == open {
                depth += 1;
            } else if chars[k] == closer {
                depth -= 1;
            }
            depth == 0
        })?;
        self.push(start, close + 1, TokenKind::Link);
        self.push(close + 1, end + 1, TokenKind::String);
        Some(end + 1)
    }

    /// 识别 `i` 处 `<` 开始的自动链接（`<https://…>`、`<a@b.c>`）或行内 HTML 标签，返回其后的
    /// 位置；都不是时返回 `None`。
    fn angle(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        let close = i + chars[i..].iter().position(|&c| c == '>')?;
        let inner = &chars[i + 1..close];
        let first = *inner.first()?;
        if !(first.is_ascii_alphabetic() || first == '/' || first == '!') {
            return None;
        }
        // 自动链接中没有空白，含 `:`（网址）或 `@`（邮件地址）；其余是标签，属性之间可以有空白。
        let url =
            !inner.iter().any(|c| c.is_whitespace()) && inner.iter().any(|&c| c == ':' || c == '@');
        let kind = if url { TokenKind::Link } else { TokenKind::Tag };
        self.push(i, close + 1, kind);
        Some(close + 1)
    }

    /// 识别 `i` 处开始的裸网址（GFM 的扩展自动链接：`https://…`、`http://…`、`www.…`），返回其后
    /// 的位置；不是网址时返回 `None`。
    ///
    /// 网址到空白为止，末尾的标点（`.`、`,`、`)` 等）不属于网址。
    fn bare_url(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        if i > 0 && chars[i - 1].is_alphanumeric() {
            return None;
        }
        if !["https://", "http://", "www."]
            .iter()
            .any(|p| starts_with(chars, i, p))
        {
            return None;
        }
        let mut end = scan(chars, i, |c| !c.is_whitespace() && c != '<');
        while end > i
            && matches!(
                chars[end - 1],
                '.' | ',' | ':' | ';' | '!' | '?' | ')' | '*' | '_'
            )
        {
            end -= 1;
        }
        self.push(i, end, TokenKind::Link);
        Some(end)
    }
}
