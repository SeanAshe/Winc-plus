//! 数据与配置格式的词法：块标量、节标题和键。

use super::*;

impl Lexer<'_> {
    /// 返回这一行开头的 YAML 块标量所属键的列（字符偏移，从 0 起）。
    ///
    /// 这一行的值不是 `|`、`>` 时返回 `None`。列是键开头的位置（`- name: |` 中 `name` 的位置）；
    /// 值直接写在列表项上（`- |`）时是 `-` 的位置。之后缩进更深的行才是内容，与键同列的
    /// 下一个键结束块标量。
    pub(super) fn block_scalar_start(&self) -> Option<u8> {
        let chars = self.chars;
        // 行尾注释之前的部分。
        let end = (0..chars.len())
            .find(|&k| chars[k] == '#' && k > 0 && chars[k - 1].is_whitespace())
            .unwrap_or(chars.len());
        let end = chars[..end]
            .iter()
            .rposition(|c| !c.is_whitespace())
            .map(|k| k + 1)?;
        // 指示符：`|` 或 `>`，之后可以有 `-`、`+` 和缩进数字。
        let indicator = chars[..end]
            .iter()
            .rposition(|&c| !matches!(c, '-' | '+' | '0'..='9'))?;
        if !matches!(chars[indicator], '|' | '>') {
            return None;
        }
        let before = chars[..indicator].iter().rposition(|c| !c.is_whitespace());

        let mut column = scan(chars, 0, |c| c == ' ');
        let mut dash = None;
        while chars.get(column) == Some(&'-')
            && chars.get(column + 1).is_some_and(|c| c.is_whitespace())
        {
            dash = Some(column);
            column = scan(chars, column + 1, |c| c == ' ');
        }
        let key = match before.map(|k| chars[k]) {
            Some(':') if before.is_some_and(|k| k >= column) => column,
            Some('-') => dash?,
            None => 0,
            _ => return None,
        };
        u8::try_from(key).ok()
    }

    /// 处理 YAML 块标量中的一行，返回下一行开头的状态。
    ///
    /// 缩进比键（列 `key`）深的行和空行是内容，按字符串着色；否则块标量已经结束，返回 `Ok(0)`，
    /// 这一行继续按普通行分析。
    pub(super) fn block_scalar_line(&mut self, key: u8) -> Result<usize, LineState> {
        let chars = self.chars;
        match chars.iter().position(|c| !c.is_whitespace()) {
            None => Err(LineState::BlockScalar(key)),
            Some(first) if first > usize::from(key) => {
                self.push(first, chars.len(), TokenKind::String);
                Err(LineState::BlockScalar(key))
            }
            Some(_) => Ok(0),
        }
    }

    /// 识别行首的节标题（`[name]`、`[[name]]`），记成标题，返回其后的位置。
    ///
    /// 这一行不是节标题时返回 `None`。
    ///
    /// 括号内只能是键名（字母、数字、`_`、`-`、`.`、引号和空格），多行数组中以 `[` 开头的元素行
    /// （`[1, 2],`）不算。
    pub(super) fn section_header(&mut self) -> Option<usize> {
        let chars = self.chars;
        let start = scan(chars, 0, char::is_whitespace);
        if chars.get(start) != Some(&'[') {
            return None;
        }
        let close = start + chars[start..].iter().position(|&c| c == ']')?;
        let end = scan(chars, close, |c| c == ']');
        let name = &chars[start..end];
        if name.iter().any(|&c| {
            !(is_name(c) || matches!(c, '[' | ']' | '.' | '-' | '"' | '\'' | ' ' | '/' | ':'))
        }) {
            return None;
        }
        let rest = scan(chars, end, char::is_whitespace);
        if rest < chars.len() && !self.at_line_comment(rest) {
            return None;
        }
        self.push(start, end, TokenKind::Heading);
        Some(end)
    }

    /// 识别行首的键：跳过缩进和 YAML 列表的 `- `，之后是带引号或不带引号的键，再往后（可以隔着
    /// 空格）是分隔符。
    ///
    /// 是键时把键记成 [`TokenKind::Property`] 并返回键末尾的位置；不是键时返回 `None`。
    pub(super) fn key(&mut self) -> Option<usize> {
        let chars = self.chars;
        let n = chars.len();
        let mut i = scan(chars, 0, char::is_whitespace);
        while chars.get(i) == Some(&'-') && chars.get(i + 1).is_some_and(|c| c.is_whitespace()) {
            i = scan(chars, i + 1, char::is_whitespace);
        }
        let start = i;

        let end = match *chars.get(start)? {
            q @ ('"' | '\'') => {
                let mut j = start + 1;
                while j < n && chars[j] != q {
                    j += if q == '"' && chars[j] == '\\' { 2 } else { 1 };
                }
                (j < n).then_some(j + 1)?
            }
            c if is_ident(c) || matches!(c, '.' | '-' | '@' | '/' | '%') => {
                scan(chars, start, |c| {
                    !c.is_whitespace()
                        && !self.language.key_separators.contains(&c)
                        && !matches!(
                            c,
                            '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '#'
                        )
                })
            }
            _ => return None,
        };

        let sep = scan(chars, end, |c| c == ' ' || c == '\t');
        let c = *chars.get(sep)?;
        if !self.language.key_separators.contains(&c) {
            return None;
        }
        // 引号括起的键（JSON）后面的冒号可以紧跟值；其余情况冒号后面必须是空白或行尾。
        let quoted = matches!(chars[start], '"' | '\'');
        if c == ':' && !quoted && chars.get(sep + 1).is_some_and(|c| !c.is_whitespace()) {
            return None;
        }

        self.push(start, end, TokenKind::Property);
        Some(end)
    }
}
