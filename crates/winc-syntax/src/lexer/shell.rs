//! shell 的词法：变量与花括号展开、heredoc 的开始和各行。

use super::*;

impl Lexer<'_> {
    /// 返回 `i` 处变量引用的末尾之后的位置和着色类别；`i` 处不是变量引用时返回 `None`。
    ///
    /// `in_string` 表示在字符串中：[`Variables::InStrings`] 只在字符串中识别。变量的
    /// 写法见 [`Variables`]。
    pub(super) fn variable(&self, i: usize, in_string: bool) -> Option<(usize, TokenKind)> {
        let chars = self.chars;
        let sigil = chars[i];
        let next = *chars.get(i + 1)?;
        let name_start = |c: char| c.is_alphabetic() || c == '_';
        let braced = |from: usize, close: char| {
            chars[from..]
                .iter()
                .position(|&c| c == close)
                .map(|k| from + k + 1)
        };
        let variable = |end: usize| Some((end, TokenKind::Variable));

        match self.language.variables {
            Variables::None => None,
            Variables::Shell if sigil == '$' => match next {
                '{' => variable(self.shell_braced(i + 2)?),
                c if name_start(c) => variable(scan(chars, i + 2, is_name)),
                // 位置参数只取一位数字：`$10` 是 `$1` 后面跟着 `0`。
                c if c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '?' | '$' | '!' | '-') => {
                    variable(i + 2)
                }
                _ => None,
            },
            Variables::Make if sigil == '$' => match next {
                '(' | '{' => {
                    let close = if next == '(' { ')' } else { '}' };
                    let name_end = scan(chars, i + 2, |c| {
                        !c.is_whitespace() && !matches!(c, '(' | ')' | '{' | '}' | '$' | ':' | ',')
                    });
                    if name_end == i + 2 {
                        return None;
                    }
                    match *chars.get(name_end)? {
                        c if c == close => variable(name_end + 1),
                        // 替换引用：`$(SRC:.c=.o)`。
                        ':' => variable(braced(name_end, close)?),
                        c if c.is_whitespace() => Some((name_end, TokenKind::Function)),
                        _ => None,
                    }
                }
                // `$$` 把 `$` 传给 shell：`$$HOME`、`$${HOME}` 是 shell 变量。
                '$' => match *chars.get(i + 2)? {
                    '{' => variable(braced(i + 3, '}')?),
                    c if name_start(c) => variable(scan(chars, i + 3, is_name)),
                    _ => None,
                },
                c if c.is_alphanumeric()
                    || matches!(c, '@' | '<' | '^' | '*' | '?' | '+' | '|' | '%') =>
                {
                    variable(i + 2)
                }
                _ => None,
            },
            Variables::Dollar if sigil == '$' => match next {
                '{' => variable(braced(i + 2, '}')?),
                c if name_start(c) => {
                    // `$env:PATH`、`$script:x` 中的作用域前缀属于变量名。
                    let mut end = scan(chars, i + 2, is_name);
                    if chars.get(end) == Some(&':')
                        && chars.get(end + 1).is_some_and(|&c| name_start(c))
                    {
                        end = scan(chars, end + 1, is_name);
                    }
                    // CMake 的 `$ENV{X}`、`$CACHE{X}`。
                    if chars.get(end) == Some(&'{') {
                        end = braced(end + 1, '}')?;
                    }
                    variable(end)
                }
                _ => None,
            },
            Variables::InStrings if sigil == '$' && in_string => {
                name_start(next).then(|| (scan(chars, i + 2, is_name), TokenKind::Variable))
            }
            Variables::Ruby => match (sigil, next) {
                ('@', '@') if chars.get(i + 2).is_some_and(|&c| name_start(c)) => {
                    variable(scan(chars, i + 2, is_name))
                }
                ('@' | '$', c) if name_start(c) => variable(scan(chars, i + 1, is_name)),
                // 特殊全局变量：`$!`、`$~`、`` $` ``、`$'`、`$1` 等。
                ('$', c)
                    if c.is_ascii_digit()
                        || matches!(
                            c,
                            '!' | '@'
                                | '~'
                                | '&'
                                | '`'
                                | '\''
                                | '+'
                                | '/'
                                | '\\'
                                | ';'
                                | ','
                                | '.'
                                | '<'
                                | '>'
                                | '_'
                                | '*'
                                | '$'
                                | '?'
                                | ':'
                                | '"'
                        ) =>
                {
                    variable(i + 2)
                }
                _ => None,
            },
            Variables::Batch => match (sigil, next) {
                // `for` 的循环变量 `%%i`，参数 `%1`、`%*`，修饰过的参数 `%~dp0`。
                ('%', '%') if chars.get(i + 2).is_some_and(|c| c.is_alphabetic()) => {
                    variable(i + 3)
                }
                ('%', c) if c.is_ascii_digit() || c == '*' => variable(i + 2),
                ('%', '~') => variable(scan(chars, i + 2, char::is_alphanumeric)),
                ('%' | '!', c) if !c.is_whitespace() && c != sigil => {
                    let close = chars[i + 1..]
                        .iter()
                        .position(|&c| c == sigil || c.is_whitespace())?;
                    (chars[i + 1 + close] == sigil).then(|| (i + close + 2, TokenKind::Variable))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// 返回 Shell 的 `${…}` 中 `from`（`${` 之后）起到收尾 `}` 之后的位置。
    ///
    /// 本行内没有收尾时返回 `None`。
    ///
    /// 其中可以嵌套引号和 `${…}`（`${x#"${p}"/}`、`${a:-"b"}`），引号内的 `}` 不收尾。
    pub(super) fn shell_braced(&self, from: usize) -> Option<usize> {
        let chars = self.chars;
        let mut nested = 0usize;
        let mut j = from;
        while j < chars.len() {
            match chars[j] {
                '\\' => j += 1,
                q @ ('\'' | '"') => {
                    let mut k = j + 1;
                    while k < chars.len() && chars[k] != q {
                        k += if q == '"' && chars[k] == '\\' { 2 } else { 1 };
                    }
                    j = k;
                }
                '{' => nested += 1,
                '}' if nested == 0 => return Some(j + 1),
                '}' => nested -= 1,
                _ => {}
            }
            j += 1;
        }
        None
    }

    /// 返回 `i` 处 here 文档开头的末尾之后的位置和对应的状态值；`i` 处不是开头时返回 `None`。
    ///
    /// 各语言的写法见 [`Heredocs`]。
    pub(super) fn heredoc_opening(&self, i: usize) -> Option<(usize, u16)> {
        let chars = self.chars;
        let style = self.language.heredocs;
        let arrows = if style == Heredocs::Php { "<<<" } else { "<<" };
        if !starts_with(chars, i, arrows) || chars.get(i + arrows.len()) == Some(&'<') {
            return None;
        }
        if i > 0 && chars[i - 1] == '<' {
            return None;
        }
        // 算术展开和算术命令中的 `<<` 是左移（`$((a<<b))`、`(( x <<= 2 ))`）：之前有
        // 未配对的 `((`。
        if style == Heredocs::Shell {
            let before = &chars[..i];
            let pairs = |pattern: [char; 2]| before.windows(2).filter(|w| *w == pattern).count();
            if pairs(['(', '(']) > pairs([')', ')']) {
                return None;
            }
        }
        let mut j = i + arrows.len();
        let mut dash = style == Heredocs::Php;
        match (style, chars.get(j)) {
            (Heredocs::Shell, Some('-')) | (Heredocs::Compact, Some('-' | '~')) => {
                dash = true;
                j += 1;
            }
            _ => {}
        }
        if style != Heredocs::Compact {
            j = scan(chars, j, |c| c == ' ' || c == '\t');
        }
        let word_char = |c: char| {
            !c.is_whitespace() && !matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\'' | '"')
        };

        let (word, end, quoted) = match *chars.get(j)? {
            q @ ('\'' | '"') => {
                let close = j + 1 + chars[j + 1..].iter().position(|&c| c == q)?;
                // PHP 的 `<<<"EOT"` 与不带引号的写法相同，内容中展开变量。
                let quoted = !(style == Heredocs::Php && q == '"');
                (&chars[j + 1..close], close + 1, quoted)
            }
            '\\' if style == Heredocs::Shell => {
                let end = scan(chars, j + 1, word_char);
                (&chars[j + 1..end], end, true)
            }
            // POSIX shell 的结束标记可以含标点；其余语言的结束标记是标识符，之后可以紧跟方法
            // 调用或参数（Ruby 的 `<<~SQL.strip`、`<<-EOS, x`）。
            c if c.is_alphabetic() || c == '_' => {
                let end = if style == Heredocs::Shell {
                    scan(chars, j, word_char)
                } else {
                    scan(chars, j, is_name)
                };
                (&chars[j..end], end, false)
            }
            _ => return None,
        };
        if word.is_empty() {
            return None;
        }

        let mut tag = delimiter_hash(word) & HEREDOC_HASH;
        if dash {
            tag |= HEREDOC_DASH;
        }
        if quoted {
            tag |= HEREDOC_QUOTED;
        }
        Some((end, tag))
    }

    /// 处理 here 文档中的一行，返回下一行开头的状态；是结束行时返回 `Normal`，否则返回 `tag`。
    ///
    /// 结束行按整行的散列值判断（见 [`HEREDOC_HASH`]）；`<<-` 时先跳过行首的缩进，Shell
    /// 只允许制表符。
    ///
    /// 结束行按 [`TokenKind::Keyword`] 着色，其余行是内容，按字符串着色。结束标记不带引号时，内容
    /// 中的变量单独着色。
    pub(super) fn heredoc_line(&mut self, tag: u16) -> LineState {
        let chars = self.chars;
        let n = chars.len();
        let style = self.language.heredocs;
        let from = match (tag & HEREDOC_DASH != 0, style) {
            (false, _) => 0,
            (true, Heredocs::Shell) => scan(chars, 0, |c| c == '\t'),
            (true, _) => scan(chars, 0, char::is_whitespace),
        };
        let to = match style {
            // PHP 的结束标记之后可以紧跟 `;`、`,`、`)`。
            Heredocs::Php => scan(chars, from, is_name),
            Heredocs::Compact => n - chars.iter().rev().take_while(|c| c.is_whitespace()).count(),
            _ => n,
        };
        if to > from && delimiter_hash(&chars[from..to]) & HEREDOC_HASH == tag & HEREDOC_HASH {
            self.push(from, to, TokenKind::Keyword);
            return LineState::Normal;
        }

        if tag & HEREDOC_QUOTED != 0 || self.language.variables == Variables::None {
            self.push(0, n, TokenKind::String);
            return LineState::Heredoc(tag);
        }

        let mut start = 0;
        let mut j = 0;
        while j < n {
            if chars[j] == self.language.escape_char {
                j += 2;
            } else if matches!(chars[j], '$' | '%' | '!')
                && let Some((end, kind)) = self.variable(j, true)
            {
                self.push(start, j, TokenKind::String);
                self.push(j, end, kind);
                start = end;
                j = end;
            } else {
                j += 1;
            }
        }
        self.push(start, n, TokenKind::String);
        LineState::Heredoc(tag)
    }
}
