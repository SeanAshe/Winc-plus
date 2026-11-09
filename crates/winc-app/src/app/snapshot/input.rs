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
    app.modifiers = ModifiersState::CONTROL;
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
    app.preedit = "ni".to_owned();
    assert_eq!(stroke(&app), None);
    app.preedit.clear();
    app.field_preedit = "ni".to_owned();
    assert_eq!(stroke(&app), None);
    app.field_preedit.clear();
    assert_eq!(stroke(&app), Some(comma));

    let _ = std::fs::remove_dir_all(&dir);
}
