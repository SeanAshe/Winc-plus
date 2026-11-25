//! 表格着色的测试。

use super::*;

fn kinds(line: &str, separator: char) -> Vec<(usize, usize, TokenKind)> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    column_tokens(&chars, separator, &mut tokens);
    tokens.iter().map(|t| (t.start, t.end, t.kind)).collect()
}

#[test]
fn each_column_has_its_own_kind_and_separators_are_dimmed() {
    assert_eq!(
        kinds("ab,c,,d", ','),
        [
            (0, 2, COLUMN_KINDS[0]),
            (2, 3, TokenKind::Separator),
            (3, 4, COLUMN_KINDS[1]),
            (4, 5, TokenKind::Separator),
            (5, 6, TokenKind::Separator),
            (6, 7, COLUMN_KINDS[3]),
        ]
    );
    // 第 7 列从头循环。
    let wide = kinds("0\t1\t2\t3\t4\t5\t6", '\t');
    assert_eq!(wide.last().unwrap().2, COLUMN_KINDS[0]);
}

/// 引号内的分隔符不分列。
#[test]
fn separators_inside_quotes_stay_in_the_field() {
    assert_eq!(
        kinds("\"a, b\",c", ','),
        [
            (0, 6, COLUMN_KINDS[0]),
            (6, 7, TokenKind::Separator),
            (7, 8, COLUMN_KINDS[1]),
        ]
    );
}
