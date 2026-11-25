//! 字符串类的词法：引号与原始字符串的开始和结束、插值、正则字面量、长括号和 `%` 字面量。

use super::*;

impl Lexer<'_> {
    /// 返回 `i` 处字符串之前紧邻的前缀的效果（见 [`Prefix`]）。
    fn prefix(&self, i: usize) -> Prefix {
        let language = self.language;
        let start = self.chars[..i]
            .iter()
            .rposition(|&c| !(is_ident(c) || c == '@'))
            .map_or(0, |s| s + 1);
        if start == i {
            return Prefix::plain(language);
        }
        let mut buf = [0u8; 24];
        // 前缀都是短的 ASCII 单词；更长或含其他字符的单词不是前缀。
        let word = ascii_word(&self.chars[start..i], &mut buf, false).unwrap_or("");
        let raw = language.raw_prefixes.contains(&word);
        let interpolated = language.interpolation.is_some_and(|it| {
            if it.prefixes.is_empty() {
                !raw
            } else {
                it.prefixes.contains(&word)
            }
        });
        Prefix { raw, interpolated }
    }

    /// 返回 `i` 处开始的字符串的开头定界符长度（字符）和结束规则。
    ///
    /// `i` 处不是字符串开头时返回 `None`。
    pub(super) fn opening(&self, i: usize) -> Option<(usize, Span)> {
        let language = self.language;
        let c = self.chars[i];

        if let Some(index) = language
            .block_strings
            .iter()
            .position(|b| starts_with(self.chars, i, b.open))
        {
            let len = language.block_strings[index].open.chars().count();
            return Some((len, block_string_span(language, u8::try_from(index).ok()?)?));
        }

        if language.triple_quotes {
            for (open, single, state) in [
                ("\"\"\"", false, LineState::TripleDouble),
                ("'''", true, LineState::TripleSingle),
            ] {
                if starts_with(self.chars, i, open) {
                    let span = quoted_span(language, open, single, self.prefix(i), state);
                    return Some((3, span));
                }
            }
        }

        if matches!(language.raw_strings, RawStrings::Rust | RawStrings::Swift)
            && let Some((open, state)) = self.raw_opening(i)
        {
            return Some((open, state.span(language)?));
        }

        match c {
            '"' if language.quoted_identifiers => Some((
                1,
                Span {
                    doubled: true,
                    ..literal_span("\"", TokenKind::Property, LineState::Normal)
                },
            )),
            '"' if !language.double_quote_string => None,
            '"' | '\'' if language.prose_quotes && self.after_word(i) => None,
            '"' => {
                let carry = if language.multiline_strings {
                    LineState::DoubleQuote
                } else {
                    LineState::Normal
                };
                Some((1, quoted_span(language, "\"", false, self.prefix(i), carry)))
            }
            '\'' if language.single_quote_string && !self.apostrophe(i) => {
                let carry = if language.multiline_strings {
                    LineState::SingleQuote
                } else {
                    LineState::Normal
                };
                Some((1, quoted_span(language, "'", true, self.prefix(i), carry)))
            }
            '`' if language.backtick_string => Some((1, backtick_span(language))),
            _ => None,
        }
    }

    /// `i` 处的引号之前（隔着空白）是否是普通单词，而不是之后可以跟字符串的关键字（见
    /// [`Language::prose_quotes`]）。
    fn after_word(&self, i: usize) -> bool {
        let chars = self.chars;
        let Some(end) = chars[..i].iter().rposition(|c| !c.is_whitespace()) else {
            return false;
        };
        if !is_ident(chars[end]) {
            return false;
        }
        let start = chars[..end]
            .iter()
            .rposition(|&c| !is_ident(c))
            .map_or(0, |k| k + 1);
        let mut buf = [0u8; 24];
        !ascii_word(&chars[start..=end], &mut buf, false)
            .is_some_and(|w| STRING_AFTER_KEYWORDS.contains(&w))
    }

    /// 从 `from` 开始查找 `span` 的结束符，把 `start` 到结束符记成 token，返回结束位置。
    ///
    /// 转义序列、插值和变量单独记成 token。到行尾仍未结束时，剩下的部分整个记成 token，
    /// 返回 `None`；其中的插值延续到下一行时（见 [`Lexer::interpolation`]）同样返回 `None`。
    pub(super) fn close(&mut self, start: usize, from: usize, span: &Span) -> Option<usize> {
        let n = self.chars.len();
        let escape = self.language.escape_char;
        let close_len = span.close.chars().count();
        let mut start = start;
        let mut j = from;

        while j < n {
            let c = self.chars[j];
            if span.block_escapes == BlockEscapes::Nix
                && let Some(len) = nix_escape_len(self.chars, j)
            {
                let end = (j + len).min(n);
                self.push(start, j, span.kind);
                self.push(j, end, TokenKind::Escape);
                start = end;
                j = end;
                continue;
            }
            if let Some(it) = span.interpolation {
                // 连写的开头首字符（`{{`、`$$`）是字面字符，不开始插值。
                if it.doubled && it.open.starts_with(c) && self.chars.get(j + 1) == Some(&c) {
                    j += 2;
                    continue;
                }
                let open = j + it.open.chars().count();
                if starts_with(self.chars, j, it.open)
                    && let Some(end) = self.interpolation(start, open, it.close, span.carry)
                {
                    // 收尾的括号与紧随其后的字符串内容一起着色。
                    start = end - 1;
                    j = end;
                    continue;
                }
                // 插值跨行：这一行剩下的部分已经按代码扫完。
                if self.suspended.is_some() {
                    return None;
                }
            }

            if span.escapes && c == escape {
                let len = escape_len(self.chars, j);
                if span.escape_tokens {
                    self.push(start, j, span.kind);
                    self.push(j, (j + len).min(n), TokenKind::Escape);
                    start = (j + len).min(n);
                }
                j += len.max(2);
            } else if span.variables
                && matches!(c, '$' | '%' | '!')
                && let Some((end, kind)) = self.variable(j, true)
            {
                self.push(start, j, span.kind);
                self.push(j, end, kind);
                start = end;
                j = end;
            } else if starts_with(self.chars, j, span.close) {
                if span.doubled && starts_with(self.chars, j + close_len, span.close) {
                    j += 2 * close_len;
                    continue;
                }
                let end = j + close_len;
                self.push(start, end, span.kind);
                return Some(end);
            } else {
                j += 1;
            }
        }

        self.push(start, n.max(start), span.kind);
        None
    }

    /// `i` 处的单引号是否是撇号。
    ///
    /// 语言区分撇号（[`Language::apostrophes`]），且单引号紧跟在字母、数字或 `_` 之后时是撇号。
    fn apostrophe(&self, i: usize) -> bool {
        self.language.apostrophes
            && i > 0
            && (self.chars[i - 1].is_alphanumeric() || self.chars[i - 1] == '_')
    }

    /// 返回 `i` 处原始字符串（Rust 的 `r"`、`r#"`、`br"`，Swift 的 `#"`）开头的长度
    /// （字符）和对应的状态。
    ///
    /// `i` 处不是原始字符串开头时返回 `None`。
    ///
    /// `i` 之前紧邻标识符字符时不算，例如 `bar"` 中的 `r`。
    fn raw_opening(&self, i: usize) -> Option<(usize, LineState)> {
        let chars = self.chars;
        if i > 0 && is_ident(chars[i - 1]) {
            return None;
        }
        let rust = self.language.raw_strings == RawStrings::Rust;
        let mut j = i;
        if rust {
            if chars.get(j) == Some(&'b') {
                j += 1;
            }
            if chars.get(j) != Some(&'r') {
                return None;
            }
            j += 1;
        }
        let hashes = scan(chars, j, |c| c == '#') - j;
        if !rust && hashes == 0 {
            return None;
        }
        let state = *RAW_STATES.get(hashes)?;
        (chars.get(j + hashes) == Some(&'"')).then_some((j + hashes + 1 - i, state))
    }

    /// 处理 `from` 处开始的插值（开头之后的部分），返回收尾的 `closer` 之后的位置。
    ///
    /// 插值之前的字符串从 `start` 起记成一个 token；插值内部按代码着色，其中的字符串（包括嵌套
    /// 的模板字符串）整个跳过，与 `closer` 配对的括号计数。可以跨行的插值里还识别注释。
    ///
    /// `carry` 是所在字符串跨行时的状态。本行内没有闭合时返回 `None`：所在的字符串可以跨行、
    /// 续行时仍然识别插值，且 `outer` 还有空位时，插值延续到下一行（`suspended` 有值），本行
    /// 剩下的部分已经按代码着色；否则不产生 token，开头按字符串内容处理。嵌套超过
    /// [`MAX_TEMPLATE_DEPTH`] 层，或本行已有 [`MAX_UNCLOSED`] 个没有闭合的插值时同样不产生 token。
    pub(super) fn interpolation(
        &mut self,
        start: usize,
        from: usize,
        closer: char,
        carry: LineState,
    ) -> Option<usize> {
        if self.depth >= MAX_TEMPLATE_DEPTH || self.unclosed >= MAX_UNCLOSED {
            return None;
        }
        if self.carrying
            && let Some(frame) = self.outer.iter().position(|&f| f == 0)
            && carry
                .span(self.language)
                .is_some_and(|span| span.interpolation.is_some())
        {
            // 没有闭合时 token 留给下一行接着扫，不需要试扫。
            self.push(start, from, TokenKind::String);
            // 字符串状态的紧凑编码只占高 5 位：可以跨行插值的字符串，编码都小于 32。
            let code = compact(carry);
            debug_assert!(code < 32, "{carry:?}");
            self.outer[frame] = code << 3 | 1;
            let end = self.interpolation_body(from, closer, Some(frame), 0);
            if end.is_some() {
                self.outer[frame] = 0;
            }
            return end;
        }
        let end = self.interpolation_to_close(start, from, closer, carry);
        if end.is_none() {
            self.unclosed += 1;
        }
        end
    }

    /// [`Lexer::interpolation`] 中不跨行的插值的扫描本身，不计没有闭合的次数。
    fn interpolation_to_close(
        &mut self,
        start: usize,
        from: usize,
        closer: char,
        carry: LineState,
    ) -> Option<usize> {
        // 先不产生 token 试扫一遍，确认在本行闭合后再正式扫描，否则已经产生的 token 无法撤回。
        // 试扫成功后内层的插值也一定闭合，用 `closes_known` 跳过内层的试扫，扫描总量与
        // 嵌套层数无关。
        if self.tokens.is_some() && !self.closes_known {
            let tokens = self.tokens.take();
            let end = self.interpolation(start, from, closer, carry);
            self.tokens = tokens;
            end?;
            self.closes_known = true;
            self.push(start, from, TokenKind::String);
            let end = self.interpolation_body(from, closer, None, 0);
            self.closes_known = false;
            return end;
        }

        self.push(start, from, TokenKind::String);
        self.interpolation_body(from, closer, None, 0)
    }

    /// 扫描插值内部的代码，返回收尾的 `closer` 之后的位置；本行内没有闭合时返回 `None`。
    ///
    /// `nested` 是进入时尚未配对的括号个数。`frame` 是跨行的插值在 `outer` 中的下标：没有闭合
    /// 时把括号个数和行尾所处的状态记下来，留给下一行；不跨行的插值为 `None`。
    pub(super) fn interpolation_body(
        &mut self,
        from: usize,
        closer: char,
        frame: Option<usize>,
        nested: u8,
    ) -> Option<usize> {
        let opener = if closer == ')' { '(' } else { '{' };
        let carrying = std::mem::replace(&mut self.carrying, frame.is_some());
        self.depth += 1;
        let mut nested = nested;
        let mut k = from;
        // 行尾停在插值的代码里，或停在其中没有结束的字符串或块注释里。
        let mut carry = LineState::Normal;
        let end = loop {
            let Some(&c) = self.chars.get(k) else {
                break None;
            };
            match c {
                c if c == closer && nested == 0 => break Some(k + 1),
                c if c == opener => {
                    nested = nested.saturating_add(1);
                    k += 1;
                }
                c if c == closer => {
                    nested -= 1;
                    k += 1;
                }
                _ => {
                    // 跨行的插值里是完整的代码，注释照常识别。不跨行的插值里不识别：`#` 可以是
                    // 格式说明（Python 的 `{n:#x}`），行注释又会吞掉收尾。
                    if frame.is_some() {
                        if let Some((index, len)) = self.comment_opening(k) {
                            match self.comment_close(k, k + len, index, 1) {
                                Ok(end) => {
                                    k = end;
                                    continue;
                                }
                                Err(next) => {
                                    carry = next;
                                    break None;
                                }
                            }
                        }
                        if self.at_line_comment(k) {
                            self.push(k, self.chars.len(), TokenKind::Comment);
                            break None;
                        }
                    }
                    match self.opening(k) {
                        Some((open, span)) => match self.close(k, k + open, &span) {
                            Some(end) => k = end,
                            None => {
                                carry = span.carry;
                                break None;
                            }
                        },
                        None => k = self.token(k),
                    }
                }
            }
        };
        if end.is_none()
            && let Some(frame) = frame
        {
            // 括号个数只有 3 位，更深的嵌套按 6 层记。
            self.outer[frame] = self.outer[frame] & !7 | (nested.min(6) + 1);
            self.suspended.get_or_insert(carry);
        }
        self.depth -= 1;
        self.carrying = carrying;
        end
    }

    /// 返回 `i` 处斜杠开始的正则表达式字面量（连同标志）的末尾之后的位置。
    ///
    /// 斜杠之前最近的非空白字符是值的结尾（标识符、数字、右括号、引号）时是除号；该标识符是
    /// `return`、`typeof` 这类关键字时仍是正则。斜杠之前是 `<` 的是 JSX 的结束标签。
    pub(super) fn regex(&self, i: usize) -> Option<usize> {
        let chars = self.chars;
        // `//` 与 `/*` 是注释（字符串插值内部不先识别注释，在这里排除）。
        if matches!(chars.get(i + 1), Some('/' | '*')) {
            return None;
        }
        if let Some(p) = chars[..i].iter().rposition(|c| !c.is_whitespace()) {
            let prev = chars[p];
            if is_ident(prev) {
                let start = chars[..p]
                    .iter()
                    .rposition(|&c| !is_ident(c))
                    .map_or(0, |s| s + 1);
                let mut buf = [0u8; 24];
                let keyword = ascii_word(&chars[start..=p], &mut buf, false)
                    .is_some_and(|w| REGEX_AFTER_KEYWORDS.contains(&w));
                if !keyword {
                    return None;
                }
            } else if prev == ')' && self.after_condition(p) {
                // `if (x) /re/.test(s)`：条件括号之后是语句的开头，斜杠开始正则。
            } else if matches!(prev, ')' | ']' | '}' | '"' | '\'' | '`' | '<') {
                return None;
            }
        }

        // 字符类 `[…]` 中的斜杠不收尾（`/[/]/`）。
        let mut in_class = false;
        let mut j = i + 1;
        while j < chars.len() {
            match chars[j] {
                '\\' => j += 1,
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => return Some(scan(chars, j + 1, |c| c.is_ascii_alphabetic())),
                _ => {}
            }
            j += 1;
        }
        None
    }

    /// `close` 处的 `)` 是否收尾 `if`、`while`、`for`、`with` 的条件括号。
    ///
    /// 向前找到与 `close` 处 `)` 配对的 `(`，检查 `(` 之前的单词；本行内找不到配对时返回 `false`。
    pub(super) fn after_condition(&self, close: usize) -> bool {
        let chars = self.chars;
        let mut depth = 0usize;
        let Some(open) = (0..=close).rev().find(|&k| {
            match chars[k] {
                ')' => depth += 1,
                '(' => depth -= 1,
                _ => {}
            }
            depth == 0
        }) else {
            return false;
        };
        let end = chars[..open]
            .iter()
            .rposition(|c| !c.is_whitespace())
            .map_or(0, |k| k + 1);
        let start = chars[..end]
            .iter()
            .rposition(|&c| !is_ident(c))
            .map_or(0, |k| k + 1);
        let mut buf = [0u8; 24];
        ascii_word(&chars[start..end], &mut buf, false)
            .is_some_and(|w| matches!(w, "if" | "while" | "for" | "with"))
    }

    /// 返回 `i` 处 C++ 原始字符串开头（`R"delim(`）之后的位置和定界符的散列值。
    ///
    /// `i` 处不是 C++ 原始字符串开头时返回 `None`。
    ///
    /// 依据 C++ 标准：定界符最多 16 个字符，不含空白、括号、反斜杠和双引号。
    pub(super) fn cpp_raw_opening(&self, i: usize) -> Option<(usize, u16)> {
        let chars = self.chars;
        if i > 0 && is_ident(chars[i - 1]) {
            return None;
        }
        let prefix = ["R", "LR", "uR", "UR", "u8R"]
            .iter()
            .find(|p| starts_with(chars, i, p))?;
        let quote = i + prefix.len();
        if chars.get(quote) != Some(&'"') {
            return None;
        }
        let delimiter = quote + 1;
        let paren = (delimiter..chars.len().min(delimiter + 17)).find(|&k| chars[k] == '(')?;
        let word = &chars[delimiter..paren];
        if word
            .iter()
            .any(|&c| c.is_whitespace() || matches!(c, ')' | '\\' | '"'))
        {
            return None;
        }
        Some((paren + 1, delimiter_hash(word)))
    }

    /// 从 `from` 开始查找 C++ 原始字符串的收尾（`)`、定界符、`"`），把 `start` 到收尾记成字符串
    /// token，返回收尾之后的位置。
    ///
    /// 到行尾仍未结束时，剩下的部分整个记成 token，返回 `None`。
    pub(super) fn cpp_raw_close(&mut self, start: usize, from: usize, hash: u16) -> Option<usize> {
        let chars = self.chars;
        let n = chars.len();
        let mut j = from;
        while let Some(k) = chars[j..].iter().position(|&c| c == ')') {
            let paren = j + k;
            // 定界符中没有双引号，`)` 之后的第一个双引号就是可能的收尾。
            if let Some(quote) = (paren + 1..n.min(paren + 18)).find(|&q| chars[q] == '"')
                && delimiter_hash(&chars[paren + 1..quote]) == hash
            {
                self.push(start, quote + 1, TokenKind::String);
                return Some(quote + 1);
            }
            j = paren + 1;
        }
        self.push(start, n, TokenKind::String);
        None
    }

    /// 返回 `i` 处 Ruby 百分号字面量开头之后的位置和开头的定界符；`i` 处不是开头时返回 `None`。
    ///
    /// `%` 之后是可选的类型字母（`q`、`Q`、`w`、`W`、`i`、`I`、`r`、`s`、`x`）和定界符。没有类型
    /// 字母时只认括号定界符，且 `%` 之前不能是值（`a % (b)` 是取模）。
    pub(super) fn percent_opening(&self, i: usize) -> Option<(usize, u8)> {
        let chars = self.chars;
        if chars[i] != '%' {
            return None;
        }
        let typed = chars.get(i + 1).is_some_and(|c| "qQwWiIrsx".contains(*c));
        let at = i + 1 + usize::from(typed);
        let open = *chars.get(at)?;
        if typed {
            if !open.is_ascii_punctuation() || open == '=' {
                return None;
            }
        } else {
            let prev = chars[..i].iter().rev().find(|c| !c.is_whitespace());
            if !matches!(open, '(' | '[' | '{' | '<')
                || prev.is_some_and(|&c| is_ident(c) || matches!(c, ')' | ']' | '}'))
            {
                return None;
            }
        }
        Some((at + 1, u8::try_from(u32::from(open)).ok()?))
    }

    /// 从 `from` 开始查找百分号字面量的收尾，把 `start` 到收尾记成字符串，返回收尾之后的位置；
    /// 到行尾仍未结束时返回下一行开头的状态。
    ///
    /// 括号定界符在内部配对计数（`%r{a{2}}`），反斜杠转义下一个字符。
    pub(super) fn percent_close(
        &mut self,
        start: usize,
        from: usize,
        open: u8,
        depth: u8,
    ) -> Result<usize, LineState> {
        let chars = self.chars;
        let n = chars.len();
        let open_char = char::from(open);
        let close_char = match open_char {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '<' => '>',
            c => c,
        };
        let mut depth = depth;
        let mut j = from;
        while j < n {
            let c = chars[j];
            if c == '\\' {
                j += 2;
                continue;
            }
            if c == close_char {
                depth -= 1;
                if depth == 0 {
                    self.push(start, j + 1, TokenKind::String);
                    return Ok(j + 1);
                }
            } else if c == open_char {
                depth = depth.saturating_add(1);
            }
            j += 1;
        }
        self.push(start, n, TokenKind::String);
        Err(LineState::Percent { open, depth })
    }

    /// 返回 `i` 处长括号开头之后的位置、`=` 的个数和是否为注释。
    ///
    /// `comment` 是所在语言的注释符；`i` 处不是长括号开头时返回 `None`。
    pub(super) fn long_opening(&self, i: usize, comment: &str) -> Option<(usize, u8, bool)> {
        let chars = self.chars;
        let marker = comment.chars().count();
        let (bracket, comment) =
            if starts_with(chars, i, comment) && chars.get(i + marker) == Some(&'[') {
                (i + marker, true)
            } else if chars[i] == '[' {
                (i, false)
            } else {
                return None;
            };
        let equals = scan(chars, bracket + 1, |c| c == '=');
        if chars.get(equals) != Some(&'[') {
            return None;
        }
        let level = u8::try_from(equals - bracket - 1).ok()?;
        Some((equals + 1, level, comment))
    }

    /// 从 `from` 开始查找长括号的收尾（`]`、`level` 个 `=`、`]`），把 `start` 到收尾记成
    /// token，返回收尾之后的位置；到行尾仍未结束时返回下一行开头的状态。
    pub(super) fn long_close(
        &mut self,
        start: usize,
        from: usize,
        level: u8,
        comment: bool,
    ) -> Result<usize, LineState> {
        let chars = self.chars;
        let n = chars.len();
        let kind = if comment {
            TokenKind::Comment
        } else {
            TokenKind::String
        };
        let level = usize::from(level);
        let mut j = from;
        while let Some(k) = chars[j..].iter().position(|&c| c == ']') {
            let bracket = j + k;
            let equals = scan(chars, bracket + 1, |c| c == '=');
            if equals - bracket - 1 == level && chars.get(equals) == Some(&']') {
                self.push(start, equals + 1, kind);
                return Ok(equals + 1);
            }
            j = bracket + 1;
        }
        self.push(start, n, kind);
        let level = level as u8;
        Err(if comment {
            LineState::LongComment(level)
        } else {
            LineState::LongString(level)
        })
    }
}

/// 返回从 `chars[j]` 的转义字符起算的转义序列长度（字符）。
///
/// `\x41` 取 2 位十六进制数，`\u0041` 取 4 位，`\U0001F600` 取 8 位，`\101` 取至多 3 位八进制数；
/// `\u{1F600}` 到 `}` 为止，花括号里不是十六进制数时只取 `\u`；其余取转义字符和紧随的一个字符。
fn escape_len(chars: &[char], j: usize) -> usize {
    let hex =
        |from: usize, max: usize| scan(chars, from, |c| c.is_ascii_hexdigit()).min(from + max) - j;
    match chars.get(j + 1) {
        None => 1,
        Some('x') => hex(j + 2, 2),
        Some('u') if chars.get(j + 2) == Some(&'{') => {
            // 花括号里只能是十六进制数（Rust 可以用 `_` 分隔，Ruby 可以用空格隔开多个）；否则
            // `\u` 之后是普通内容，不能一直找到 `}`，把收尾的引号也算进来。
            let end = scan(chars, j + 3, |c| {
                c.is_ascii_hexdigit() || c == '_' || c == ' '
            });
            if end > j + 3 && chars.get(end) == Some(&'}') {
                end + 1 - j
            } else {
                2
            }
        }
        Some('u') => hex(j + 2, 4),
        Some('U') => hex(j + 2, 8),
        Some(c) if c.is_digit(8) => scan(chars, j + 1, |c| c.is_digit(8)).min(j + 4) - j,
        Some(_) => 2,
    }
}

/// 返回 `i` 处开始的 Nix 缩进字符串转义的长度（字符）：`'''`、`''$` 为 3，`''\` 连同其后的
/// 一个字符为 4（行尾时可能超出 `chars`）；`i` 处不是转义时返回 `None`。
fn nix_escape_len(chars: &[char], i: usize) -> Option<usize> {
    if !starts_with(chars, i, "''") {
        return None;
    }
    match chars.get(i + 2) {
        Some('\'' | '$') => Some(3),
        Some('\\') => Some(4),
        _ => None,
    }
}
