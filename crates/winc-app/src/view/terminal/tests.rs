//! 终端绘制的测试：粗体亮色的开关、样式段按格切分、折叠行数的分组、方块元素的形状、组字文字的位置。

use super::*;

#[test]
fn bold_is_bright_only_when_asked() {
    let p = &crate::theme::DARK;
    let bold_red = Style {
        fg: TermColor::Indexed(1),
        attrs: Attrs(Attrs::BOLD),
        ..Style::default()
    };
    let colors = |bright| Colors {
        p,
        reverse: false,
        min: 0.0,
        bright,
    };
    assert_eq!(colors(false).style(&bold_red).fg, p.ansi[1]);
    assert_eq!(colors(true).style(&bold_red).fg, p.ansi[9]);
    // 不是粗体、已经是亮色的不变。
    let red = Style {
        attrs: Attrs::default(),
        ..bold_red
    };
    assert_eq!(colors(true).style(&red).fg, p.ansi[1]);
}

/// 对比度保护只调整文字：分隔符、制表符和方块元素保留程序指定的字色，与相邻格子的底色衔接。
#[test]
fn contrast_protection_leaves_shape_characters_alone() {
    let colors = Colors {
        p: &crate::theme::LIGHT,
        reverse: false,
        min: 3.0,
        bright: false,
    };
    // 前一段的底色作字色，后一段的底色作底色：两者接近，对比度不足。
    let (yellow, green) = ((0xd7, 0x99, 0x21), (0x68, 0x9d, 0x6a));
    let style = Style {
        fg: TermColor::Rgb(yellow.0, yellow.1, yellow.2),
        bg: TermColor::Rgb(green.0, green.1, green.2),
        ..Style::default()
    };
    let paint = colors.style(&style);
    assert_eq!(paint.shape, Color::hex(0xd79921));
    assert_eq!(paint.bg, Some(Color::hex(0x689d6a)));
    assert_ne!(paint.fg, paint.shape, "文字的颜色仍按对比度调整");

    // 淡色按底色调和后同样用于图形字符；隐藏的文字两种颜色都等于底色。
    let dim = Style {
        attrs: Attrs(Attrs::DIM),
        ..style
    };
    assert_eq!(colors.style(&dim).shape, colors.style(&dim).fg);
    let hidden = Style {
        attrs: Attrs(Attrs::HIDDEN),
        ..style
    };
    assert_eq!(colors.style(&hidden).shape, Color::hex(0x689d6a));

    for c in [
        '\u{e0b0}', '\u{e0b2}', '\u{e0b4}', '\u{e0d7}', '─', '█', '▟',
    ] {
        assert!(is_shape(c), "{c:?}");
    }
    // 图标（分支、锁）和普通文字仍受保护。
    for c in ['\u{e0a0}', '\u{e0a2}', '\u{f418}', 'a', '中', '■'] {
        assert!(!is_shape(c), "{c:?}");
    }
}

#[test]
fn a_light_theme_lifts_near_black_backgrounds() {
    let p = &crate::theme::LIGHT;
    let colors = Colors {
        p,
        reverse: false,
        min: 0.0,
        bright: false,
    };
    let input = Style {
        fg: TermColor::Rgb(0xff, 0xff, 0xff),
        bg: TermColor::Rgb(0x24, 0x24, 0x28),
        ..Style::default()
    };
    let bg = colors.style(&input).bg.unwrap();
    assert!(crate::theme::luminance(bg) > 0.7, "{bg:?}");
    let diff = Style {
        bg: TermColor::Rgb(0x2b, 0x3f, 0x2b),
        ..Style::default()
    };
    let bg = colors.style(&diff).bg.unwrap();
    assert!(bg.g >= bg.r && bg.g >= bg.b, "{bg:?}");
    assert!(crate::theme::luminance(bg) > 0.7, "{bg:?}");
    // 本来就亮的底色不动。
    let pale = Style {
        bg: TermColor::Rgb(0xe6, 0xf4, 0xea),
        ..Style::default()
    };
    assert_eq!(colors.style(&pale).bg, Some(Color::hex(0xe6f4ea)));
}

#[test]
fn a_light_theme_keeps_the_inverted_cursor_black() {
    let p = &crate::theme::LIGHT;
    let colors = Colors {
        p,
        reverse: false,
        min: 0.0,
        bright: false,
    };
    let cursor = Style {
        attrs: Attrs(Attrs::INVERSE),
        ..Style::default()
    };
    let paint = colors.style(&cursor);
    assert_eq!(paint.bg, Some(p.caret));
    assert!(crate::theme::luminance(paint.fg) > 0.7, "{:?}", paint.fg);
}

#[test]
fn a_dark_theme_keeps_near_black_backgrounds() {
    let p = &crate::theme::DARK;
    let colors = Colors {
        p,
        reverse: false,
        min: 0.0,
        bright: false,
    };
    let input = Style {
        bg: TermColor::Rgb(0x24, 0x24, 0x28),
        ..Style::default()
    };
    assert_eq!(colors.style(&input).bg, Some(Color::hex(0x242428)));
}

#[test]
fn segments_follow_runs_by_cell_not_by_char() {
    let red = Style {
        fg: TermColor::Indexed(1),
        ..Style::default()
    };
    let line = Line {
        text: "中a\u{301}bc".into(),
        runs: vec![
            winc_pty::protocol::Run {
                start: 0,
                style: Style::default(),
            },
            winc_pty::protocol::Run {
                start: 3,
                style: red,
            },
        ],
        ..Line::default()
    };
    let parts = segments(&line);
    assert_eq!(parts.len(), 2);
    assert_eq!(
        (parts[0].start, parts[0].end, parts[0].text),
        (0, 3, "中a\u{301}")
    );
    assert_eq!((parts[1].start, parts[1].end, parts[1].text), (3, 5, "bc"));
    assert_eq!(width_at(&line, 0), 2);
    let cluster = cluster_at(&line, 3).unwrap();
    assert_eq!(cluster.base, 'b');
    assert_eq!(cluster.style.fg, TermColor::Indexed(1));
    // 组合字符跟着基本字符；宽字符的后半格和行尾没有字。
    let cluster = cluster_at(&line, 2).unwrap();
    assert_eq!((cluster.base, &cluster.marks[..]), ('a', &['\u{301}'][..]));
    assert!(cluster_at(&line, 1).is_none());
    assert!(cluster_at(&line, 9).is_none());
}

#[test]
fn hidden_line_counts_are_grouped() {
    assert_eq!(group_digits(7), "7");
    assert_eq!(group_digits(1284), "1 284");
    assert_eq!(group_digits(1_000_000), "1 000 000");
}

#[test]
fn block_elements_split_into_fractions_of_the_cell() {
    let parts = |c| {
        let mut found = Vec::new();
        let known = block_parts(c, |rect, opacity| found.push((rect, opacity)));
        known.then_some(found)
    };
    assert_eq!(parts('█').unwrap(), [([0.0, 0.0, 1.0, 1.0], 1.0)]);
    assert_eq!(parts('▀').unwrap(), [([0.0, 0.0, 1.0, 0.5], 1.0)]);
    assert_eq!(parts('▁').unwrap(), [([0.0, 0.875, 1.0, 1.0], 1.0)]);
    assert_eq!(parts('▏').unwrap(), [([0.0, 0.0, 0.125, 1.0], 1.0)]);
    assert_eq!(parts('▐').unwrap(), [([0.5, 0.0, 1.0, 1.0], 1.0)]);
    assert_eq!(parts('░').unwrap(), [([0.0, 0.0, 1.0, 1.0], 0.25)]);
    assert_eq!(parts('▓').unwrap(), [([0.0, 0.0, 1.0, 1.0], 0.75)]);
    assert_eq!(parts('▙').unwrap().len(), 3);
    assert_eq!(parts('a'), None);
    assert_eq!(parts('─'), None);
    // 整个区段都有形状。
    for c in '\u{2580}'..='\u{259F}' {
        assert!(parts(c).is_some_and(|found| !found.is_empty()), "{c}");
    }
}

#[test]
fn the_preedit_moves_left_only_when_it_would_overflow() {
    // 每格 10，右边界 100。
    assert_eq!(preedit_x(40.0, "", 10.0, 100.0), 40.0);
    assert_eq!(preedit_x(40.0, "abc", 10.0, 100.0), 40.0);
    assert_eq!(preedit_x(80.0, "abc", 10.0, 100.0), 70.0);
    // 宽字占两格。
    assert_eq!(preedit_x(80.0, "中文", 10.0, 100.0), 60.0);
    // 比可用宽度还长时保留结尾。
    assert_eq!(preedit_x(0.0, &"a".repeat(30), 10.0, 100.0), -200.0);
}
