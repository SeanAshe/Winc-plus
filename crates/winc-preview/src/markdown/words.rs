//! Markdown 原文的字数：中日韩文字每个字算一个，其余文字按词算，词中的撇号和连字符不断开；
//! 链接和图片的地址、HTML 标签和行首有序列表的序号不算。
//!
//! 按片段读入，可以直接读 rope 的各块，不必把整篇拼成一个字符串。

#[cfg(test)]
mod tests;

/// 数 `chunks` 拼起来的原文有多少字。
pub fn count<'a>(chunks: impl IntoIterator<Item = &'a str>) -> usize {
    let mut counter = Counter::default();
    for chunk in chunks {
        for c in chunk.chars() {
            counter.push(c);
        }
    }
    counter.finish()
}

/// 跳过不算的内容时在等什么。
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Skip {
    #[default]
    None,
    /// 链接地址 `](…)` 里，数着括号的层数。
    Destination(u32),
    /// HTML 标签 `<…>` 里。
    Tag,
}

#[derive(Default)]
struct Counter {
    words: usize,
    /// 正在读一个词。
    in_word: bool,
    /// 这一行到目前为止只有空白和引用的 `>`。
    head: bool,
    /// 正在读的词从行首开始、全是数字：可能是有序列表的序号。
    number: bool,
    /// 读到序号之后的 `.`、`)`：后面是空白时不算这个词。
    marker: bool,
    /// 数字之后的 `.`、`,`：后面还是数字时（`1.5`、`1,000`）同一个词继续。
    joiner: Option<char>,
    skip: Skip,
    prev: char,
    started: bool,
}

impl Counter {
    fn push(&mut self, c: char) {
        if !self.started {
            (self.started, self.head) = (true, true);
        }
        if let Some(joiner) = self.joiner.take() {
            if c.is_ascii_digit() {
                self.number = false;
                return self.next(c);
            }
            self.end_word(joiner);
        }
        if self.marker {
            self.marker = false;
            if !c.is_whitespace() {
                self.words += 1;
            }
        }
        match self.skip {
            Skip::Destination(depth) => {
                self.skip = match c {
                    '\n' => Skip::None,
                    '(' => Skip::Destination(depth + 1),
                    ')' if depth == 0 => Skip::None,
                    ')' => Skip::Destination(depth - 1),
                    _ => self.skip,
                };
                return self.next(c);
            }
            Skip::Tag => {
                if matches!(c, '>' | '\n') {
                    self.skip = Skip::None;
                }
                return self.next(c);
            }
            Skip::None => {}
        }
        if c == '(' && self.prev == ']' {
            self.end_word(c);
            self.skip = Skip::Destination(0);
            return self.next(c);
        }
        if self.prev == '<' && (c.is_ascii_alphabetic() || matches!(c, '/' | '!')) {
            self.end_word(c);
            self.skip = Skip::Tag;
            return self.next(c);
        }
        if cjk(c) {
            self.end_word(c);
            self.words += 1;
        } else if c.is_alphanumeric() {
            if !self.in_word {
                self.in_word = true;
                self.number = self.head && c.is_ascii_digit();
            } else if !c.is_ascii_digit() {
                self.number = false;
            }
        } else if self.in_word && matches!(c, '.' | ',') && self.prev.is_ascii_digit() {
            self.joiner = Some(c);
            return self.next(c);
        } else if self.in_word && matches!(c, '\'' | '’' | '-') {
            // 词中的撇号、连字符：后面还是字母时同一个词继续，否则这个词在这里结束。
            self.number = false;
        } else {
            self.end_word(c);
        }
        if !(c.is_whitespace() || c == '>') {
            self.head = false;
        }
        if c == '\n' {
            self.head = true;
        }
        self.next(c);
    }

    /// 正在读的词到 `c` 之前结束。
    fn end_word(&mut self, c: char) {
        if !self.in_word {
            return;
        }
        self.in_word = false;
        if self.number && matches!(c, '.' | ')') {
            self.marker = true;
        } else {
            self.words += 1;
        }
        self.number = false;
    }

    fn next(&mut self, c: char) {
        self.prev = c;
    }

    fn finish(mut self) -> usize {
        let last = self.joiner.take().unwrap_or('\n');
        self.end_word(last);
        if self.marker {
            self.words += 1;
        }
        self.words
    }
}

/// 是否为按字计数的中日韩文字：汉字、假名和谚文。
fn cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2FA1F}'
    )
}
