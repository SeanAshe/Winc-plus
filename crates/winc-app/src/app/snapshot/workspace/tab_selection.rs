//! 标签栏多选的测试：选中的几个标签一起拖动，关闭左侧、右侧以选区为界。

use winit::keyboard::ModifiersState;

use crate::app::input::context_menu::MenuCommand;

use super::*;

fn names(app: &App) -> Vec<String> {
    app.editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect()
}

/// 返回第 `index` 个标签（从 0 起）内的一点：距标签左边缘 20 逻辑像素，垂直居中。
fn tab_point(app: &App, index: usize) -> (f32, f32) {
    let rect = app.hits.tabs.iter().find(|h| h.index == index).unwrap().tab;
    (rect.x + 20.0, rect.y + rect.h / 2.0)
}

fn click_tab(app: &mut App, index: usize, modifiers: ModifiersState) {
    app.modifiers = modifiers;
    let at = tab_point(app, index);
    click(app, at);
    app.modifiers = ModifiersState::empty();
    paint(app);
}

fn selected(app: &App) -> Vec<String> {
    let ids = app.tab_selection();
    app.editor
        .documents
        .iter()
        .filter(|d| ids.contains(&d.id))
        .map(|d| d.name().into_owned())
        .collect()
}

#[test]
fn dragging_a_selected_tab_moves_the_whole_selection() {
    let dir = search_folder("tabs-multi-drag");
    let mut app = app_with(&dir, false);
    for name in [
        "src/main.rs",
        "src/config.rs",
        "Cargo.toml",
        "a.txt",
        "short.rs",
    ] {
        app.open_path(&dir.join(name));
    }
    paint(&mut app);

    click_tab(&mut app, 0, ModifiersState::empty());
    click_tab(&mut app, 2, command_modifiers());
    assert_eq!(selected(&app), ["main.rs", "Cargo.toml"]);

    // 按在选中的 Cargo.toml 上往右拖到底：main.rs 先靠拢到它左边，两个一起移到最右。
    let (x, y) = tab_point(&app, 2);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(selected(&app), ["main.rs", "Cargo.toml"]);
    app.on_mouse_move(x + 10.0, y);
    assert_eq!(
        names(&app),
        ["config.rs", "main.rs", "Cargo.toml", "a.txt", "short.rs"]
    );
    app.on_mouse_move(2000.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        names(&app),
        ["config.rs", "a.txt", "short.rs", "main.rs", "Cargo.toml"]
    );
    assert_eq!(selected(&app), ["main.rs", "Cargo.toml"]);
    paint(&mut app);

    // 再往左拖到底：仍是一起。
    let (x, y) = tab_point(&app, 3);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(0.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        names(&app),
        ["main.rs", "Cargo.toml", "config.rs", "a.txt", "short.rs"]
    );
    paint(&mut app);

    // 在选中的标签上按下又松开（没有拖动）：只选它。
    click_tab(&mut app, 1, ModifiersState::empty());
    assert_eq!(selected(&app), ["Cargo.toml"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn close_left_and_right_stop_at_the_selection() {
    let dir = search_folder("tabs-multi-close-sides");
    let mut app = app_with(&dir, false);
    for name in [
        "src/main.rs",
        "src/config.rs",
        "Cargo.toml",
        "a.txt",
        "short.rs",
    ] {
        app.open_path(&dir.join(name));
    }
    paint(&mut app);

    // 选中 config.rs 和 a.txt，右键点击 config.rs：关闭右侧从 a.txt 之后开始。
    click_tab(&mut app, 1, ModifiersState::empty());
    click_tab(&mut app, 3, command_modifiers());
    let at = tab_point(&app, 1);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseRight);
    assert_eq!(names(&app), ["main.rs", "config.rs", "Cargo.toml", "a.txt"]);

    // 关闭左侧：config.rs 左边的 main.rs。
    paint(&mut app);
    let at = tab_point(&app, 3);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseLeft);
    assert_eq!(names(&app), ["config.rs", "Cargo.toml", "a.txt"]);

    // 只点一个标签时以它自己为界。
    paint(&mut app);
    click_tab(&mut app, 2, ModifiersState::empty());
    let at = tab_point(&app, 2);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseLeft);
    assert_eq!(names(&app), ["a.txt"]);

    std::fs::remove_dir_all(&dir).unwrap();
}
