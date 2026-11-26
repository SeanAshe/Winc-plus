//! 输入的测试：浮层的焦点和快捷键、右键菜单、输入法接管的按键。

use super::*;

pub(super) mod dialogs;
pub(super) mod menus;
mod touch;

#[test]
fn a_key_taken_by_the_input_method_is_no_shortcut_while_composing() {
    use crate::keymap::{KeyCode, Mods, Stroke};
    use winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey, PhysicalKey};

    let dir = sample_folder("ime-shortcut");
    let mut app = app_with(&dir, false);
    app.pointer.modifiers = ModifiersState::CONTROL;
    let stroke = |app: &App| {
        app.stroke_of(
            &Key::Named(NamedKey::Process),
            "",
            PhysicalKey::Code(winit::keyboard::KeyCode::Comma),
            KeyLocation::Standard,
        )
    };
    let comma = Stroke {
        key: KeyCode::Char(','),
        mods: Mods::CTRL,
    };

    assert_eq!(stroke(&app), Some(comma));
    app.ime.preedit = "ni".to_owned();
    assert_eq!(stroke(&app), None);
    app.ime.preedit.clear();
    app.ime.field_preedit = "ni".to_owned();
    assert_eq!(stroke(&app), None);
    app.ime.field_preedit.clear();
    assert_eq!(stroke(&app), Some(comma));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 组字文字只属于焦点所在的一处；焦点不在可以输入文字的位置时停用输入法并清空组字文字。
#[test]
fn composition_follows_the_focus() {
    use winit::event::Ime;

    let dir = sample_folder("ime-focus");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    assert!(app.takes_text());
    app.on_ime(Ime::Preedit("ni".into(), None));
    assert_eq!(app.preedit_at(Focus::Editor), "ni");
    assert_eq!(app.preedit_at(Focus::Terminal), "");

    // 焦点移到查找条后继续组字：文字只留在输入框里。
    app.open_search(false);
    assert!(app.focus.is_field() && app.takes_text());
    app.on_ime(Ime::Preedit("nih".into(), None));
    assert_eq!(
        (app.ime.preedit.as_str(), app.ime.field_preedit.as_str()),
        ("", "nih")
    );
    app.focus = Focus::Editor;
    app.on_ime(Ime::Preedit("niha".into(), None));
    assert_eq!(
        (app.ime.preedit.as_str(), app.ime.field_preedit.as_str()),
        ("niha", "")
    );

    // 文件树不能输入文字。
    app.focus = Focus::Tree;
    assert!(!app.takes_text());
    app.update_ime_area();
    assert!(app.ime.preedit.is_empty() && app.ime.field_preedit.is_empty());
    // 没有运行中的终端时，终端面板同样不能输入文字。
    app.focus = Focus::Terminal;
    assert!(!app.takes_text());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：组字时不显示光标，在输入法中移动光标后无法确认位置。
#[test]
fn the_composition_caret_follows_the_input_method() {
    use winit::event::Ime;

    let dir = sample_folder("ime-caret");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let at = |app: &mut App, cursor: Option<(usize, usize)>| {
        app.on_ime(Ime::Preedit("nihao".into(), cursor));
        paint(app);
        app.ime.cursor.unwrap().x
    };
    let (start, middle, end) = (
        at(&mut app, Some((0, 0))),
        at(&mut app, Some((2, 2))),
        at(&mut app, Some((5, 5))),
    );
    let cell = app.metrics.cell;
    assert!(
        (middle - start - 2.0 * cell).abs() < 0.5,
        "{start} {middle}"
    );
    assert!((end - start - 5.0 * cell).abs() < 0.5, "{start} {end}");
    // 输入法报告一段范围时取其终点；要求隐藏光标时候选框留在末尾。
    assert!((at(&mut app, Some((0, 2))) - middle).abs() < 0.5);
    assert!((at(&mut app, None) - end).abs() < 0.5);

    // 查找条的输入框：同样随输入法的光标移动。
    app.on_ime(Ime::Preedit(String::new(), None));
    app.open_search(false);
    let at = |app: &mut App, cursor: Option<(usize, usize)>| {
        app.on_ime(Ime::Preedit("nihao".into(), cursor));
        paint(app);
        app.ime.cursor.unwrap().x
    };
    let (start, middle, end) = (
        at(&mut app, Some((0, 0))),
        at(&mut app, Some((2, 2))),
        at(&mut app, Some((5, 5))),
    );
    assert!(start < middle && middle < end, "{start} {middle} {end}");
    assert!((at(&mut app, None) - end).abs() < 0.5);

    let _ = std::fs::remove_dir_all(&dir);
}
