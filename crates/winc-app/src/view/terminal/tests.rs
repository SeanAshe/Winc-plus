//! 终端绘制的测试：粗体亮色的开关、样式段按格切分、折叠行数的分组。

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
    assert_eq!(colors(false).style(&bold_red).0, p.ansi[1]);
    assert_eq!(colors(true).style(&bold_red).0, p.ansi[9]);
    // 不是粗体、已经是亮色的不变。
    let red = Style {
        attrs: Attrs::default(),
        ..bold_red
    };
    assert_eq!(colors(true).style(&red).0, p.ansi[1]);
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
    let bg = colors.style(&input).1.unwrap();
    assert!(crate::theme::luminance(bg) > 0.7, "{bg:?}");
    let diff = Style {
        bg: TermColor::Rgb(0x2b, 0x3f, 0x2b),
        ..Style::default()
    };
    let bg = colors.style(&diff).1.unwrap();
    assert!(bg.g >= bg.r && bg.g >= bg.b, "{bg:?}");
    assert!(crate::theme::luminance(bg) > 0.7, "{bg:?}");
    // 本来就亮的底色不动。
    let pale = Style {
        bg: TermColor::Rgb(0xe6, 0xf4, 0xea),
        ..Style::default()
    };
    assert_eq!(colors.style(&pale).1, Some(Color::hex(0xe6f4ea)));
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
    assert_eq!(colors.style(&input).1, Some(Color::hex(0x242428)));
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
    assert_eq!(char_at(&line, 3), Some('b'));
}

#[test]
fn hidden_line_counts_are_grouped() {
    assert_eq!(group_digits(7), "7");
    assert_eq!(group_digits(1284), "1 284");
    assert_eq!(group_digits(1_000_000), "1 000 000");
}
