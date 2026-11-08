//! `inline` 的测试：折行时不能出现在行首的标点不放在行首。

use super::*;

/// 把 `text` 拆成每个字宽 1 逻辑像素的原子，可断处按 [`can_break`] 确定。
fn atoms(text: &str) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::new();
    let mut prev = None;
    for c in text.chars() {
        out.push(Atom {
            kind: AtomKind::Char(c),
            width: 1.0,
            run: 0,
            brk: prev.is_some_and(|p| can_break(p, c)),
            space: is_space(c),
            source: None,
        });
        prev = Some(c);
    }
    out
}

/// 返回按宽度 `width`（逻辑像素）折行后各行的文字，去掉首尾空格。
fn lines(text: &str, width: f32) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    break_lines(&atoms(text), width)
        .into_iter()
        .map(|r| chars[r].iter().collect::<String>().trim().to_owned())
        .collect()
}

#[test]
fn middle_dots_stay_with_the_word_before_them() {
    assert_eq!(lines("规则表 · 规则包", 5.0), ["规则表 ·", "规则包"]);
}

#[test]
fn forced_breaks_keep_closing_marks_off_the_line_start() {
    // 括号内的字母串没有可断处，只能在字与字之间强制断行；右括号随前一个字换到下一行。
    assert_eq!(lines("（abcdef）", 7.0), ["（abcde", "f）"]);
}
