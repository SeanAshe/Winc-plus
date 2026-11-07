//! 按键、鼠标、粘贴和焦点编码的测试：覆盖各修饰键组合、模式开关与坐标边界。

use super::*;

fn bytes(key: Key<'_>, mods: Mods, modes: Modes) -> Vec<u8> {
    let mut out = Vec::new();
    encode(key, mods, modes, 0, &mut out);
    out
}

const NONE: Mods = Mods {
    shift: false,
    alt: false,
    ctrl: false,
};
const CTRL: Mods = Mods { ctrl: true, ..NONE };
const ALT: Mods = Mods { alt: true, ..NONE };
const SHIFT: Mods = Mods {
    shift: true,
    ..NONE
};

#[test]
fn plain_keys() {
    let modes = Modes::default();
    assert_eq!(bytes(Key::Text("你好"), NONE, modes), "你好".as_bytes());
    assert_eq!(bytes(Key::Enter, NONE, modes), b"\r");
    assert_eq!(bytes(Key::Backspace, NONE, modes), b"\x7f");
    assert_eq!(bytes(Key::Tab, SHIFT, modes), b"\x1b[Z");
    assert_eq!(bytes(Key::Escape, NONE, modes), b"\x1b");
    assert_eq!(bytes(Key::Delete, NONE, modes), b"\x1b[3~");
    assert_eq!(bytes(Key::PageUp, CTRL, modes), b"\x1b[5;5~");
    assert_eq!(bytes(Key::F(1), NONE, modes), b"\x1bOP");
    assert_eq!(bytes(Key::F(5), SHIFT, modes), b"\x1b[15;2~");
    assert_eq!(bytes(Key::F(12), NONE, modes), b"\x1b[24~");
    assert_eq!(bytes(Key::Enter, NONE, Modes(Modes::NEWLINE)), b"\r\n");
}

#[test]
fn cursor_keys_follow_the_cursor_key_mode() {
    assert_eq!(bytes(Key::Up, NONE, Modes::default()), b"\x1b[A");
    assert_eq!(bytes(Key::Up, NONE, Modes(Modes::APP_CURSOR)), b"\x1bOA");
    // 带修饰键时不受光标键模式影响。
    let mods = Mods {
        shift: true,
        ctrl: true,
        alt: false,
    };
    assert_eq!(
        bytes(Key::Left, mods, Modes(Modes::APP_CURSOR)),
        b"\x1b[1;6D"
    );
    assert_eq!(bytes(Key::End, NONE, Modes::default()), b"\x1b[F");
}

#[test]
fn control_and_meta_characters() {
    let modes = Modes::default();
    assert_eq!(bytes(Key::Char('c'), CTRL, modes), [3]);
    assert_eq!(bytes(Key::Char('['), CTRL, modes), [0x1b]);
    assert_eq!(bytes(Key::Char(' '), CTRL, modes), [0]);
    assert_eq!(bytes(Key::Char('b'), ALT, modes), b"\x1bb");
    assert_eq!(bytes(Key::Char('B'), ALT, modes), b"\x1bB");
    let both = Mods {
        ctrl: true,
        alt: true,
        shift: false,
    };
    assert_eq!(bytes(Key::Char('x'), both, modes), b"\x1b\x18");
    // 没有对应控制字符的组合不写入任何字节。
    let mut out = Vec::new();
    assert!(!encode(Key::Char('.'), CTRL, modes, 0, &mut out));
    assert!(out.is_empty());
}

#[test]
fn kitty_level_one_only_changes_ambiguous_keys() {
    let k = |key, mods| {
        let mut out = Vec::new();
        encode(key, mods, Modes::default(), DISAMBIGUATE, &mut out);
        String::from_utf8(out).unwrap()
    };
    assert_eq!(k(Key::Escape, NONE), "\x1b[27u");
    assert_eq!(k(Key::Enter, NONE), "\r");
    assert_eq!(k(Key::Enter, SHIFT), "\x1b[13;2u");
    assert_eq!(k(Key::Tab, SHIFT), "\x1b[9;2u");
    assert_eq!(k(Key::Backspace, CTRL), "\x1b[127;5u");
    assert_eq!(k(Key::Char('i'), CTRL), "\x1b[105;5u");
    assert_eq!(
        k(
            Key::Char('A'),
            Mods {
                shift: true,
                alt: true,
                ctrl: false
            }
        ),
        "\x1b[97;4u"
    );
    // 打字和方向键保持原有编码。
    assert_eq!(k(Key::Text("a"), NONE), "a");
    assert_eq!(k(Key::Up, CTRL), "\x1b[1;5A");
}

#[test]
fn mouse_reports_follow_the_requested_modes() {
    let report = |button, action, modes: u32| {
        let mut out = Vec::new();
        mouse(button, action, (4, 9), NONE, Modes(modes), &mut out).then_some(out)
    };
    let sgr = Modes::MOUSE_PRESS | Modes::MOUSE_SGR;
    assert_eq!(report(Button::Left, MouseAction::Press, 0), None);
    assert_eq!(
        report(Button::Left, MouseAction::Press, sgr).unwrap(),
        b"\x1b[<0;5;10M"
    );
    assert_eq!(
        report(Button::Left, MouseAction::Release, sgr).unwrap(),
        b"\x1b[<0;5;10m"
    );
    assert_eq!(
        report(Button::WheelDown, MouseAction::Press, sgr).unwrap(),
        b"\x1b[<65;5;10M"
    );
    // 只开 1000 时不上报拖动；开 1002 上报拖动但不上报无按键的移动；开 1003 两者都上报。
    assert_eq!(report(Button::Left, MouseAction::Motion, sgr), None);
    let drag = sgr | Modes::MOUSE_DRAG;
    assert_eq!(
        report(Button::Left, MouseAction::Motion, drag).unwrap(),
        b"\x1b[<32;5;10M"
    );
    assert_eq!(report(Button::None, MouseAction::Motion, drag), None);
    let all = sgr | Modes::MOUSE_MOTION;
    assert_eq!(
        report(Button::None, MouseAction::Motion, all).unwrap(),
        b"\x1b[<35;5;10M"
    );
    // 旧编码：每个数加 32 写成字节，松开时上报 3。
    let legacy = Modes::MOUSE_PRESS;
    assert_eq!(
        report(Button::Right, MouseAction::Press, legacy).unwrap(),
        b"\x1b[M\"%*"
    );
    assert_eq!(
        report(Button::Right, MouseAction::Release, legacy).unwrap(),
        b"\x1b[M#%*"
    );
    let mut out = Vec::new();
    assert!(!mouse(
        Button::Left,
        MouseAction::Press,
        (300, 0),
        NONE,
        Modes(legacy),
        &mut out
    ));
}

#[test]
fn pasting_keeps_the_text_inside_its_brackets() {
    let mut out = Vec::new();
    paste("a\nb\r\nc", Modes::default(), &mut out);
    assert_eq!(out, b"a\rb\rc");

    let mut out = Vec::new();
    paste(
        "ls\x1b[201~rm -rf x\n",
        Modes(Modes::BRACKETED_PASTE),
        &mut out,
    );
    assert_eq!(out, b"\x1b[200~lsrm -rf x\r\x1b[201~");

    // 去掉一个标记后拼出的另一个标记同样去掉。
    let mut out = Vec::new();
    paste(
        "a\x1b[201\x1b[200~~rm x\n",
        Modes(Modes::BRACKETED_PASTE),
        &mut out,
    );
    assert_eq!(out, b"\x1b[200~arm x\r\x1b[201~");
}

#[test]
fn focus_reports_only_when_asked() {
    let mut out = Vec::new();
    focus(true, Modes::default(), &mut out);
    assert!(out.is_empty());
    focus(false, Modes(Modes::FOCUS), &mut out);
    assert_eq!(out, b"\x1b[O");
}

#[test]
fn multi_line_pastes_need_confirming_without_bracketed_paste() {
    let plain = Modes::default();
    let bracketed = Modes(Modes::BRACKETED_PASTE);
    // 未开启括号粘贴：多行需要确认；单行、以及只在结尾多一个换行的单行不需要确认。
    assert!(paste_needs_confirm("ls\nrm -rf x", plain));
    assert!(paste_needs_confirm("ls\r\nrm\r\n", plain));
    assert!(!paste_needs_confirm("ls -la", plain));
    assert!(!paste_needs_confirm("ls -la\n", plain));
    assert!(!paste_needs_confirm("ls -la\r\n", plain));
    // 开启括号粘贴：程序把整段视为一次粘贴，不会逐行执行。
    assert!(!paste_needs_confirm("ls\nrm -rf x", bracketed));
}

/// 回归测试：嵌套的括号粘贴标记必须一遍去完。
///
/// 触发条件：内容中层层嵌套的标记（去掉里面一个拼出外面一个）；若每次移除后都从头重新查找，
/// 耗时是平方级，构造的剪贴板内容会长时间占用界面线程。
#[test]
fn nested_paste_markers_are_stripped_in_one_pass() {
    let depth = 20_000;
    let text = format!("a{}{}b", "\x1b[20".repeat(depth), "0~1~".repeat(depth / 2));
    let mut out = Vec::new();
    paste(&text, Modes(Modes::BRACKETED_PASTE), &mut out);
    assert_eq!(out, b"\x1b[200~ab\x1b[201~");
    // 标记之前不足 6 个字节时不越界。
    let mut out = Vec::new();
    paste("~", Modes(Modes::BRACKETED_PASTE), &mut out);
    assert_eq!(out, b"\x1b[200~~\x1b[201~");
}

#[test]
fn cursor_steps_count_characters_not_cells() {
    assert!(matches!(cursor_steps("hello", 5, 2), (Key::Left, 3)));
    assert!(matches!(cursor_steps("hello", 1, 4), (Key::Right, 3)));
    // 宽字符占两格，只按一次；组合字符不单独算。
    assert!(matches!(cursor_steps("中文ab", 6, 0), (Key::Left, 4)));
    assert!(matches!(cursor_steps("e\u{301}x", 2, 0), (Key::Left, 2)));
    // 点到文字后面：算到末尾。
    assert!(matches!(cursor_steps("ab", 1, 50), (Key::Right, 1)));
    assert!(matches!(cursor_steps("ab", 1, 1), (_, 0)));
}
