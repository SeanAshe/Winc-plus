//! 标记语言（HTML、XML、SVG、Vue、Svelte、Astro、Liquid、Nunjucks）的规则：标签、属性、字符
//! 实体，以及 HTML 中内嵌的脚本与样式；Astro 还有文件开头 `---` 之间的 TypeScript frontmatter，
//! Liquid 与 Nunjucks 还有 `{{ … }}`、`{% … %}` 模板标签。
//!
//! 正文中的引号按标点处理，只有标签内的属性值才记成字符串。

use super::{
    Lexer, LineState, TokenKind, compact, expand, is_name, literal_span, scan, starts_with,
    starts_with_ignore_case,
};
use crate::lang::{Markup, embedded, template};

/// `LineState::Tag` 的值：标签结束后是普通内容。
const CONTENT: u8 = 0;
/// `LineState::Tag` 的值：标签结束后是 `<script>` 的内容。
const SCRIPT: u8 = 1;
/// `LineState::Tag` 的值：标签结束后是 `<style>` 的内容。
const STYLE: u8 = 2;
/// `LineState::Tag` 的值：处在跨行的模板输出 `{{ … }}` 中。
const OUTPUT: u8 = 3;
/// `LineState::Tag` 的值：处在跨行的模板语句 `{% … %}` 中。
const STATEMENT: u8 = 4;

impl Lexer<'_> {
    /// 按标记语言的规则分析整行，返回下一行开头的状态。
    pub(super) fn markup_line(&mut self, state: LineState) -> LineState {
        let n = self.chars.len();
        let mut tag = None;
        let mut i = match state {
            LineState::Start => {
                if self.language.frontmatter && self.frontmatter_fence() {
                    self.push(0, 3, TokenKind::Decorator);
                    return LineState::Frontmatter { inner: 0 };
                }
                0
            }
            LineState::Frontmatter { inner } => {
                // 多行模板字符串或块注释中的 `---` 是 TypeScript 的内容，不结束 frontmatter。
                if expand(inner) == LineState::Normal && self.frontmatter_fence() {
                    self.push(0, 3, TokenKind::Decorator);
                    return LineState::Normal;
                }
                let next = Lexer::new(
                    embedded(true),
                    self.chars,
                    self.tokens.as_deref_mut(),
                    self.base,
                )
                .run(expand(inner));
                return LineState::Frontmatter {
                    inner: compact(next),
                };
            }
            LineState::Tag(kind @ (OUTPUT | STATEMENT)) => {
                match self.template(0, kind == STATEMENT, false) {
                    Ok(end) => end,
                    Err(next) => return next,
                }
            }
            LineState::Tag(kind) => {
                tag = Some(kind);
                0
            }
            LineState::Embedded { script, inner } => match self.embedded(0, script, inner) {
                Ok(end) => end,
                Err(next) => return next,
            },
            _ => match self.resume(state) {
                Ok(end) => end,
                Err(next) => return next,
            },
        };

        while i < n {
            if let Some(kind) = tag {
                // 标签内的模板标签（`<input {% if a %}checked{% endif %}>`）；本行内没有收尾时，
                // 下一行仍按标签内部分析。
                if let Some(statement) = self.template_open(i) {
                    match self.template(i, statement, true) {
                        Ok(end) => i = end,
                        Err(_) => return LineState::Tag(kind),
                    }
                    continue;
                }
                match self.inside_tag(i) {
                    Ok(next) => i = next,
                    Err(end) => {
                        tag = None;
                        i = end;
                        if kind != CONTENT {
                            match self.embedded(i, kind == SCRIPT, 0) {
                                Ok(end) => i = end,
                                Err(next) => return next,
                            }
                        }
                    }
                }
                continue;
            }

            if let Some((index, len)) = self.comment_opening(i) {
                match self.comment_close(i, i + len, index, 1) {
                    Ok(end) => i = end,
                    Err(next) => return next,
                }
                continue;
            }

            if let Some((open, span)) = self.cdata(i) {
                match self.close(i, i + open, &span) {
                    Some(end) => i = end,
                    None => return span.carry,
                }
                continue;
            }

            if let Some(statement) = self.template_open(i) {
                match self.template(i, statement, true) {
                    Ok(end) => i = end,
                    Err(next) => return next,
                }
                continue;
            }

            match self.chars[i] {
                '<' => match self.tag_open(i) {
                    Some((end, kind)) => {
                        tag = Some(kind);
                        i = end;
                    }
                    None => i += 1,
                },
                '&' => i = self.entity(i).unwrap_or(i + 1),
                _ => i += 1,
            }
        }

        tag.map_or(LineState::Normal, LineState::Tag)
    }

    /// 本行是否只含 `---`（后面可以有空白），用于 Astro frontmatter 的开始与结束。
    fn frontmatter_fence(&self) -> bool {
        let end = self
            .chars
            .iter()
            .rposition(|c| !c.is_whitespace())
            .map_or(0, |last| last + 1);
        self.chars[..end] == ['-', '-', '-']
    }

    /// `i` 处是否开始模板标签：开始语句 `{%` 时为 `Some(true)`，开始输出 `{{` 时为 `Some(false)`；
    /// 语言不识别模板标签（[`crate::Language::template_tags`]）时为 `None`。
    fn template_open(&self, i: usize) -> Option<bool> {
        if !self.language.template_tags || self.chars[i] != '{' {
            return None;
        }
        match self.chars.get(i + 1)? {
            '%' => Some(true),
            '{' => Some(false),
            _ => None,
        }
    }

    /// 分析从 `from` 起的模板语句（`statement` 为真）或输出，返回收尾定界符之后的位置；本行内
    /// 没有收尾时返回下一行开头的状态。
    ///
    /// `opening` 为真时 `from` 处是开头的定界符，可以带去除空白的 `-`；语句的第一个名字是标签名。
    /// 引号中的 `%}`、`}}` 不算收尾。
    fn template(
        &mut self,
        from: usize,
        statement: bool,
        opening: bool,
    ) -> Result<usize, LineState> {
        let chars = self.chars;
        let n = chars.len();
        let mut i = from;
        if opening {
            let open = from + 2 + usize::from(chars.get(from + 2) == Some(&'-'));
            self.push(from, open, TokenKind::Decorator);
            i = open;
            if statement {
                let name = scan(chars, i, char::is_whitespace);
                i = scan(chars, name, is_name);
                self.push(name, i, TokenKind::Keyword);
            }
        }
        let close = if statement { '%' } else { '}' };
        let mut quote = None;
        let mut k = i;
        let end = loop {
            let Some(&c) = chars.get(k) else {
                break None;
            };
            match quote {
                Some(_) if c == '\\' => k += 1,
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if matches!(c, '"' | '\'') => quote = Some(c),
                None if c == close && chars.get(k + 1) == Some(&'}') => break Some(k),
                None => {}
            }
            k += 1;
        };
        let Some(end) = end else {
            self.template_expression(i, n);
            return Err(LineState::Tag(if statement { STATEMENT } else { OUTPUT }));
        };
        let body = if end > i && chars[end - 1] == '-' {
            end - 1
        } else {
            end
        };
        self.template_expression(i, body);
        self.push(body, end + 2, TokenKind::Decorator);
        Ok(end + 2)
    }

    /// 按 [`template`] 的规则分析 `from` 到 `to` 之间的模板表达式；`|` 之后的名字是过滤器，按
    /// 函数着色。
    fn template_expression(&mut self, from: usize, to: usize) {
        let chars = self.chars;
        let mut start = from;
        let mut quote = None;
        let mut k = from;
        while k <= to {
            let c = chars.get(k).copied().filter(|_| k < to);
            match (quote, c) {
                (Some(_), Some('\\')) => k += 1,
                (Some(q), Some(c)) if c == q => quote = None,
                (Some(_), Some(_)) => {}
                (None, Some(c @ ('"' | '\''))) => quote = Some(c),
                (None, Some('|')) if chars.get(k + 1) != Some(&'|') => {
                    self.template_segment(start, k);
                    let name = scan(chars, k + 1, char::is_whitespace).min(to);
                    let end = scan(chars, name, is_name).min(to);
                    self.push(name, end, TokenKind::Function);
                    start = end;
                    k = end;
                    continue;
                }
                (_, None) => {
                    self.template_segment(start, to);
                    break;
                }
                _ => {}
            }
            k += 1;
        }
    }

    /// 按 [`template`] 的规则分析 `from` 到 `to` 之间不含过滤器的一段表达式。
    fn template_segment(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        Lexer::new(
            template(),
            &self.chars[from..to],
            self.tokens.as_deref_mut(),
            self.base + from,
        )
        .run(LineState::Normal);
    }

    /// 返回 `i` 处 CDATA 段（`<![CDATA[…]]>`）的开头长度（字符）与结束规则。
    ///
    /// 不是 CDATA 时返回 `None`。
    fn cdata(&self, i: usize) -> Option<(usize, super::Span)> {
        let index = self
            .language
            .block_strings
            .iter()
            .position(|b| starts_with(self.chars, i, b.open))?;
        let block = self.language.block_strings[index];
        let span = literal_span(
            block.close,
            TokenKind::String,
            LineState::BlockString(u8::try_from(index).ok()?),
        );
        Some((block.open.chars().count(), span))
    }

    /// 识别 `i` 处 `<` 开始的标签（`<div`、`</div`、`<!DOCTYPE`、`<?xml`），把标签名记成 token，
    /// 返回标签名之后的位置和标签结束后的内容（见 [`CONTENT`]）；不是标签时返回 `None`。
    fn tag_open(&mut self, i: usize) -> Option<(usize, u8)> {
        let chars = self.chars;
        let (name, kind) = match chars.get(i + 1)? {
            '/' => (i + 2, TokenKind::Tag),
            '!' | '?' => (i + 2, TokenKind::Decorator),
            _ => (i + 1, TokenKind::Tag),
        };
        if !chars.get(name)?.is_alphabetic() {
            return None;
        }
        let end = scan(chars, name, |c| is_name(c) || matches!(c, '-' | ':' | '.'));
        let start = if kind == TokenKind::Decorator {
            i + 1
        } else {
            name
        };
        self.push(start, end, kind);

        let opening = name == i + 1;
        let content = if opening && self.language.markup == Markup::Html {
            let tag = |t: &str| end - name == t.len() && starts_with_ignore_case(chars, name, t);
            if self.language.script_tags.iter().any(|t| tag(t)) {
                SCRIPT
            } else if starts_with_ignore_case(chars, name, "style") && end - name == 5 {
                STYLE
            } else {
                CONTENT
            }
        } else {
            CONTENT
        };
        Some((end, content))
    }

    /// 分析标签内部 `i` 处的一个结构（属性名、属性值），返回其后的位置；标签在此结束时返回
    /// `Err`，值是 `>` 之后的位置。
    ///
    /// 自闭合的标签（`<script src="a" />`）之后没有内嵌内容，同样返回 `Err`，由调用方按标签结束
    /// 后的内容处理；这种写法在 HTML 中很少见，不单独区分。
    fn inside_tag(&mut self, i: usize) -> Result<usize, usize> {
        let chars = self.chars;
        let n = chars.len();
        match chars[i] {
            '>' => Err(i + 1),
            '/' | '?' if chars.get(i + 1) == Some(&'>') => Err(i + 2),
            '=' => Ok(self.attribute_value(scan(chars, i + 1, char::is_whitespace))),
            c if c.is_whitespace() || matches!(c, '/' | '?') => Ok(i + 1),
            '"' | '\'' | '{' => Ok(self.attribute_value(i)),
            _ => {
                // 属性名在模板标签之前结束（`checked{% endif %}`）。
                let end = (i..n)
                    .find(|&k| {
                        let c = chars[k];
                        c.is_whitespace()
                            || matches!(c, '=' | '>' | '/' | '"' | '\'')
                            || (k > i && self.template_open(k).is_some())
                    })
                    .unwrap_or(n);
                self.push(i, end, TokenKind::Attribute);
                Ok(end.max(i + 1).min(n))
            }
        }
    }

    /// 识别 `i` 处的属性值，返回其后的位置。
    ///
    /// 带引号的值是字符串，本行内没有收尾时到行尾为止；`{…}` 是表达式，不着色；
    /// 其余到空白或 `>` 为止。
    fn attribute_value(&mut self, i: usize) -> usize {
        let chars = self.chars;
        let n = chars.len();
        let Some(&c) = chars.get(i) else {
            return n;
        };
        match c {
            '"' | '\'' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&q| q == c)
                    .map_or(n, |k| i + k + 2);
                self.push(i, end, TokenKind::String);
                end
            }
            '{' => {
                let mut depth = 0usize;
                (i..n)
                    .find(|&k| {
                        match chars[k] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        depth == 0
                    })
                    .map_or(n, |k| k + 1)
            }
            _ => {
                let end = scan(chars, i, |c| !c.is_whitespace() && c != '>');
                self.push(i, end, TokenKind::String);
                end.max(i + 1)
            }
        }
    }

    /// 识别 `i` 处的字符实体（`&amp;`、`&#169;`、`&#x1F600;`），记成转义，返回其后的位置。
    ///
    /// 不是实体时返回 `None`。
    fn entity(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        let name = i + 1 + usize::from(chars.get(i + 1) == Some(&'#'));
        let end = scan(chars, name, char::is_alphanumeric);
        if end == name || chars.get(end) != Some(&';') {
            return None;
        }
        self.push(i, end + 1, TokenKind::Escape);
        Some(end + 1)
    }

    /// 按内嵌语言分析从 `from` 起的脚本（`script` 为真）或样式，到脚本标签的结束标签
    /// （`</script`，见 [`crate::lang::Language::script_tags`]）或 `</style` 为止，返回结束标签
    /// 的位置；本行内没有结束标签时返回下一行开头的状态。
    ///
    /// `inner` 是内嵌语言的行首状态，编码方式见 [`compact`]。
    fn embedded(&mut self, from: usize, script: bool, inner: u8) -> Result<usize, LineState> {
        let chars = self.chars;
        let tags: &[&str] = if script {
            self.language.script_tags
        } else {
            &["style"]
        };
        let closes = |k: usize| {
            starts_with_ignore_case(chars, k, "</")
                && tags
                    .iter()
                    .any(|t| starts_with_ignore_case(chars, k + 2, t))
        };
        let end = (from..chars.len()).find(|&k| closes(k));
        let stop = end.unwrap_or(chars.len());
        let base = self.base + from;
        let next = Lexer::new(
            embedded(script),
            &chars[from..stop],
            self.tokens.as_deref_mut(),
            base,
        )
        .run(expand(inner));
        end.ok_or(LineState::Embedded {
            script,
            inner: compact(next),
        })
    }
}
