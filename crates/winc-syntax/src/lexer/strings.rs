//! 字符串类的词法：引号与原始字符串的开始和结束、插值、正则字面量、长括号和 `%` 字面量。

use super::*;

impl Lexer<'_> {
    /// `i` 处的单引号是否是撇号。
    ///
    /// 语言区分撇号（[`Language::apostrophes`]），且单引号紧跟在字母、数字或 `_` 之后时是撇号。
    pub(super) fn apostrophe(&self, i: usize) -> bool {
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
    pub(super) fn raw_opening(&self, i: usize) -> Option<(usize, LineState)> {
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
    /// 本行内没有闭合，或嵌套超过 [`MAX_TEMPLATE_DEPTH`] 层时返回 `None`，不产生 token。插值之前
    /// 的字符串从 `start` 起记成一个 token；插值内部按代码着色，其中的字符串（包括嵌套的模板
    /// 字符串）整个跳过，与 `closer` 配对的括号计数。
    pub(super) fn interpolation(
        &mut self,
        start: usize,
        from: usize,
        closer: char,
    ) -> Option<usize> {
        if self.depth >= MAX_TEMPLATE_DEPTH {
            return None;
        }

        // 先不产生 token 试扫一遍，确认在本行闭合后再正式扫描，否则已经产生的 token 无法撤回。
        // 试扫成功后内层的插值也一定闭合，用 `closes_known` 跳过内层的试扫，扫描总量与
        // 嵌套层数无关。
        if self.tokens.is_some() && !self.closes_known {
            let tokens = self.tokens.take();
            let end = self.interpolation(start, from, closer);
            self.tokens = tokens;
            end?;
            self.closes_known = true;
            self.push(start, from, TokenKind::String);
            let end = self.interpolation_body(from, closer);
            self.closes_known = false;
            return end;
        }

        self.push(start, from, TokenKind::String);
        self.interpolation_body(from, closer)
    }

    /// 扫描插值内部的代码，返回收尾的 `closer` 之后的位置；本行内没有闭合时返回 `None`。
    pub(super) fn interpolation_body(&mut self, from: usize, closer: char) -> Option<usize> {
        let opener = if closer == ')' { '(' } else { '{' };
        self.depth += 1;
        let mut nested = 0usize;
        let mut k = from;
        let end = loop {
            let Some(&c) = self.chars.get(k) else {
                break None;
            };
            match c {
                c if c == closer && nested == 0 => break Some(k + 1),
                c if c == opener => {
                    nested += 1;
                    k += 1;
                }
                c if c == closer => {
                    nested -= 1;
                    k += 1;
                }
                _ => match self.opening(k) {
                    Some((open, span)) => match self.close(k, k + open, &span) {
                        Some(end) => k = end,
                        None => break None,
                    },
                    None => k = self.token(k),
                },
            }
        };
        self.depth -= 1;
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
