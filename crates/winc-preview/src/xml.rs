//! 逐个读出 XML 中的标签和文字，不建立整棵树：工作表和共享字符串表可以有几百 MB，建树占用的
//! 内存是原文的数倍。
//!
//! 只支持常见的部分：标签名和属性名取冒号之后的本地名，不处理命名空间和 DTD。格式错误
//! （标签没有结束、开始与结束标签不配对）时读到出错处为止，并由 [`Tokens::well_formed`] 报告。

use std::borrow::Cow;

/// XML 中的一项。
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
    /// 开始标签。
    Open {
        /// 本地名。
        name: &'a str,
        /// 属性部分的原文，用 [`attribute`] 取值。
        attributes: &'a str,
        /// 是否为空元素（`<a/>`）。
        empty: bool,
    },
    /// 结束标签的本地名。
    Close(&'a str),
    /// 标签之间的文字，实体已还原。
    Text(Cow<'a, str>),
}

/// 依次读出 `text` 中的各项；注释、处理指令和文档类型声明跳过。
pub struct Tokens<'a> {
    rest: &'a str,
    /// 尚未结束的各层标签的名字（带前缀）。
    open: Vec<&'a str>,
    /// 是否遇到了格式错误。
    broken: bool,
}

impl<'a> Tokens<'a> {
    /// 创建读 `text` 的迭代器。
    pub fn new(text: &'a str) -> Self {
        Self {
            rest: text,
            open: Vec::new(),
            broken: false,
        }
    }

    /// 读完之后，文档的格式是否正确：没有遇到错误，各层标签都已结束。
    pub fn well_formed(&self) -> bool {
        !self.broken && self.rest.is_empty() && self.open.is_empty()
    }

    /// 记下格式错误并停止读取。
    fn fail(&mut self) -> Option<Token<'a>> {
        (self.broken, self.rest) = (true, "");
        None
    }

    /// 跳过以 `end` 结尾的一段，返回其中 `skip` 字节之后、`end` 之前的内容；没有 `end` 时读完。
    fn until(&mut self, skip: usize, end: &str) -> &'a str {
        let body = &self.rest[skip..];
        let (inner, rest) = match body.find(end) {
            Some(at) => (&body[..at], &body[at + end.len()..]),
            None => (body, ""),
        };
        self.rest = rest;
        inner
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            if self.rest.is_empty() {
                return None;
            }
            if !self.rest.starts_with('<') {
                let end = self.rest.find('<').unwrap_or(self.rest.len());
                let (text, rest) = self.rest.split_at(end);
                self.rest = rest;
                return Some(Token::Text(unescape(text)));
            }
            if self.rest.starts_with("<![CDATA[") {
                return Some(Token::Text(Cow::Borrowed(self.until(9, "]]>"))));
            }
            if self.rest.starts_with("<!--") {
                self.until(4, "-->");
            } else if self.rest.starts_with("<?") {
                self.until(2, "?>");
            } else if self.rest.starts_with("<!") {
                self.until(2, ">");
            } else if self.rest.starts_with("</") {
                let name = self.until(2, ">").trim();
                if self.open.pop() != Some(name) {
                    return self.fail();
                }
                return Some(Token::Close(local(name)));
            } else {
                // 属性值中可以有 `>`：在引号之外的第一个 `>` 处结束。
                let mut quote = None;
                let end = self.rest.bytes().position(|b| match (quote, b) {
                    (None, b'"' | b'\'') => {
                        quote = Some(b);
                        false
                    }
                    (Some(q), b) if q == b => {
                        quote = None;
                        false
                    }
                    (None, b'>') => true,
                    _ => false,
                });
                let Some(end) = end else {
                    return self.fail();
                };
                let inner = &self.rest[1..end];
                self.rest = &self.rest[end + 1..];
                let (inner, empty) = match inner.strip_suffix('/') {
                    Some(inner) => (inner, true),
                    None => (inner, false),
                };
                let split = inner.find(char::is_whitespace).unwrap_or(inner.len());
                let (name, attributes) = inner.split_at(split);
                if name.is_empty() || name.contains(['<', '"', '\'', '=']) {
                    return self.fail();
                }
                if !empty {
                    self.open.push(name);
                }
                return Some(Token::Open {
                    name: local(name),
                    attributes,
                    empty,
                });
            }
        }
    }
}

/// 返回带前缀的名字 `name`（如 `x:row`）冒号之后的部分。
fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

/// 返回开始标签的属性部分 `attributes` 中本地名为 `key` 的属性值，实体已还原。
pub fn attribute<'a>(attributes: &'a str, key: &str) -> Option<Cow<'a, str>> {
    let mut rest = attributes;
    loop {
        let (name, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let (value, next) = after[1..].split_once(quote)?;
        if local(name.trim()) == key {
            return Some(unescape(value));
        }
        rest = next;
    }
}

/// 还原 `text` 中的实体（`&lt;`、`&#10;`、`&#x4E2D;` 等）；不认识的实体原样保留。
fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest.find(';').map(|end| &rest[1..end]);
        let decoded = entity.and_then(|entity| match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let code = entity.strip_prefix('#')?;
                let code = match code.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => code.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (decoded, entity) {
            (Some(c), Some(entity)) => {
                out.push(c);
                rest = &rest[entity.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}
