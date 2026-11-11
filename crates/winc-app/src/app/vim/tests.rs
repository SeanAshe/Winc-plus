//! 按键归属与按键转换的测试。

use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use super::{claims, convert, copies};
use winc_vim::{Code, Key as VimKey, Mode, Mods, Named};

/// 没有修饰键、产生字符 `text` 的按键。
fn typed(text: &str, code: KeyCode, modifiers: ModifiersState, latin: bool) -> Option<VimKey> {
    let key = Key::Character(text.into());
    let plain = Key::Character(text.to_lowercase().into());
    convert(&key, &plain, PhysicalKey::Code(code), modifiers, latin)
}

#[test]
fn characters_are_recognized_by_what_they_produce() {
    let none = ModifiersState::empty();
    assert_eq!(
        typed("d", KeyCode::KeyD, none, true),
        Some(VimKey::char('d'))
    );
    // `Shift` 已体现在产生的字符中。
    let shift = ModifiersState::SHIFT;
    assert_eq!(
        typed("D", KeyCode::KeyD, shift, true),
        Some(VimKey::char('D'))
    );
    assert_eq!(
        typed("$", KeyCode::Digit4, shift, true),
        Some(VimKey::char('$'))
    );
    // `Alt` 产生的字符按字符本身识别，普通模式下也不按物理键位变成命令。
    let alt = ModifiersState::ALT;
    for latin in [false, true] {
        assert_eq!(
            typed("å", KeyCode::KeyA, alt, latin),
            Some(VimKey::char('å'))
        );
    }
    // 布局上需要 `Alt` 才能输入的 ASCII 字符照常识别。
    assert_eq!(
        typed("@", KeyCode::KeyL, alt, true),
        Some(VimKey::char('@'))
    );
}

#[test]
fn control_combinations_use_the_unmodified_character() {
    let ctrl = ModifiersState::CONTROL;
    assert_eq!(
        typed("r", KeyCode::KeyR, ctrl, true),
        Some(VimKey::ctrl('r'))
    );
    // 与 `[`、`i`、`m` 的组合等同 `Esc`、`Tab`、回车。
    assert_eq!(
        typed("[", KeyCode::BracketLeft, ctrl, true),
        Some(VimKey::named(Named::Esc))
    );
    // `Ctrl+Shift` 的组合保留 `Shift`，不与 `Ctrl` 的组合混同。
    let both = ctrl | ModifiersState::SHIFT;
    let key = typed("R", KeyCode::KeyR, both, true).unwrap();
    assert_eq!(key.mods(), Mods::CTRL.with(Mods::SHIFT));
    assert_ne!(key, VimKey::ctrl('r'));
    // `Ctrl` 与 `6` 的组合等同 `Ctrl-^`，按住 `Shift` 时也是。
    let six = |text: &str, modifiers: ModifiersState| {
        let (logical, plain) = (Key::Character(text.into()), Key::Character("6".into()));
        let physical = PhysicalKey::Code(KeyCode::Digit6);
        convert(&logical, &plain, physical, modifiers, true)
    };
    assert_eq!(six("6", ctrl), Some(VimKey::ctrl('^')));
    assert_eq!(six("^", both), Some(VimKey::ctrl('^')));
    // 带 `Command` 的组合带有相应的修饰键，之后归应用处理。
    let cmd = typed("s", KeyCode::KeyS, ModifiersState::SUPER, true).unwrap();
    assert!(cmd.mods().contains(Mods::CMD));
}

#[test]
fn non_latin_layouts_fall_back_to_the_physical_key_outside_insert_mode() {
    let none = ModifiersState::empty();
    // 西里尔布局上 `KeyJ` 产生「о」：普通模式按美式布局取 `j`，插入模式保持原字符。
    assert_eq!(
        typed("о", KeyCode::KeyJ, none, true),
        Some(VimKey::char('j'))
    );
    assert_eq!(
        typed("о", KeyCode::KeyJ, none, false),
        Some(VimKey::char('о'))
    );
    let shift = ModifiersState::SHIFT;
    assert_eq!(
        typed("О", KeyCode::KeyJ, shift, true),
        Some(VimKey::char('J'))
    );
    assert_eq!(
        typed("Ж", KeyCode::Semicolon, shift, true),
        Some(VimKey::char(':'))
    );
    assert_eq!(
        typed("ё", KeyCode::Backquote, none, true),
        Some(VimKey::char('`'))
    );
    // 带 `Ctrl` 时同样取物理键位。
    let ctrl = ModifiersState::CONTROL;
    assert_eq!(
        typed("к", KeyCode::KeyR, ctrl, true),
        Some(VimKey::ctrl('r'))
    );
}

#[test]
fn named_keys_keep_their_modifiers() {
    let named = |key: NamedKey, modifiers: ModifiersState| {
        let key = Key::Named(key);
        convert(
            &key,
            &key,
            PhysicalKey::Code(KeyCode::Escape),
            modifiers,
            true,
        )
    };
    let none = ModifiersState::empty();
    assert_eq!(
        named(NamedKey::Escape, none),
        Some(VimKey::named(Named::Esc))
    );
    assert_eq!(
        named(NamedKey::Enter, none),
        Some(VimKey::named(Named::Enter))
    );
    assert_eq!(named(NamedKey::Space, none), Some(VimKey::char(' ')));
    assert_eq!(
        named(NamedKey::ArrowLeft, ModifiersState::SHIFT),
        Some(VimKey::new(Code::Named(Named::Left), Mods::SHIFT))
    );
    // 单独的修饰键、功能键不交给 Vim 层。
    assert_eq!(named(NamedKey::Shift, ModifiersState::SHIFT), None);
    assert_eq!(named(NamedKey::F5, none), None);
    assert_eq!(
        convert(
            &Key::Dead(None),
            &Key::Dead(None),
            PhysicalKey::Code(KeyCode::KeyA),
            none,
            true
        ),
        None
    );
}

#[test]
fn ownership_follows_the_platform_rules_and_the_overrides() {
    let none: &[(String, bool)] = &[];
    // 没有 `Ctrl` 的按键都交给 Vim 层，带 `Command` 的都归应用。
    assert!(claims(VimKey::char('d'), false, none));
    assert!(claims(VimKey::named(Named::Esc), false, none));
    let cmd = VimKey::new(Code::Char('s'), Mods::CMD);
    assert!(!claims(cmd, true, none));

    // macOS 上 `Ctrl` 组合全部交给 Vim 层。
    for c in "cvxaszfnpwrdu".chars() {
        assert!(claims(VimKey::ctrl(c), true, none), "{c}");
    }
    // 其他系统上保存、撤销、新建、快速打开归应用，其余交给 Vim 层。
    for c in "sznp".chars() {
        assert!(!claims(VimKey::ctrl(c), false, none), "{c}");
    }
    for c in "cvxafwrdueoy".chars() {
        assert!(claims(VimKey::ctrl(c), false, none), "{c}");
    }
    // 可视模式下的 `Ctrl+C` 是复制，macOS 上与其余模式下按 Vim 的含义。
    assert!(copies(VimKey::ctrl('c'), Mode::Visual, false));
    assert!(!copies(VimKey::ctrl('c'), Mode::Normal, false));
    assert!(!copies(VimKey::ctrl('c'), Mode::Visual, true));
    assert!(!copies(VimKey::ctrl('v'), Mode::Visual, false));

    // 配置中的改写优先于默认的规则。
    let overrides = [("ctrl+f".to_owned(), true), ("ctrl+d".to_owned(), false)];
    assert!(claims(VimKey::ctrl('f'), false, &overrides));
    assert!(!claims(VimKey::ctrl('d'), false, &overrides));
    assert!(!claims(VimKey::ctrl('d'), true, &overrides));
    assert!(claims(VimKey::ctrl('r'), false, &overrides));
}
