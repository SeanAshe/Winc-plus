//! 按分隔符分列的表格（CSV、TSV）的着色：每一列一种颜色，循环使用 [`COLUMN_KINDS`]，分隔符
//! 颜色较淡（[`TokenKind::Separator`]）。
//!
//! 按原文排布时由 [`column_tokens`] 生成词法单元；按列对齐时由应用层逐字符着色，使用同样的类别。

use crate::{Token, TokenKind};

/// 各列依次使用的着色类别，超过 6 列时从头循环。
pub const COLUMN_KINDS: [TokenKind; 6] = [
    TokenKind::Text,
    TokenKind::Keyword,
    TokenKind::String,
    TokenKind::Function,
    TokenKind::Number,
    TokenKind::Builtin,
];

/// 为以 `separator` 分列的一行 `chars` 着色，结果写入 `tokens`（先清空）。
///
/// 第 `i` 列（从 0 起）按 `COLUMN_KINDS[i % 6]` 着色，分隔符按 [`TokenKind::Separator`] 着色。
/// 引号内的分隔符不分列，`"a, b"` 是一个字段；引号的状态不跨行，与按列对齐的规则相同。
pub fn column_tokens(chars: &[char], separator: char, tokens: &mut Vec<Token>) {
    tokens.clear();
    let (mut field, mut start, mut quoted) = (0, 0, false);
    let push = |tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind| {
        if end > start {
            tokens.push(Token { start, end, kind });
        }
    };
    for (i, &c) in chars.iter().enumerate() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == separator && !quoted {
            push(tokens, start, i, COLUMN_KINDS[field % COLUMN_KINDS.len()]);
            push(tokens, i, i + 1, TokenKind::Separator);
            field += 1;
            start = i + 1;
        }
    }
    push(
        tokens,
        start,
        chars.len(),
        COLUMN_KINDS[field % COLUMN_KINDS.len()],
    );
}

#[cfg(test)]
mod tests;
