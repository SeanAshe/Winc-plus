//! 拖放编辑的测试：拖动选中的文字移动或复制、撤销一次即恢复、没有拖动时按单击放置光标、放回
//! 选区之内不改动，以及关闭设置后按拖选处理。

use super::*;
use crate::keymap::Action;
use winit::keyboard::ModifiersState;

const SAMPLE: &str = "let alpha = 1;\nlet beta = 2;\n";

/// 打开内容为 [`SAMPLE`] 的文件，选中第一行的 `alpha`（第 4 至 9 列）；`dark` 为是否用深色主题。
fn drag_app(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    std::fs::write(dir.join("a.txt"), SAMPLE).unwrap();
    let mut app = app_with(&dir.join("a.txt"), dark);
    app.focus = Focus::Editor;
    paint(&mut app);
    app.editor.active_mut().unwrap().selection = Selection::new(4, 9);
    (app, dir)
}

/// 第 `line` 行第 `col` 列那个字符正中的窗口坐标（行、列从 0 起）。
fn over(app: &App, line: usize, col: usize) -> (f32, f32) {
    let t = app.layout.text;
    let doc = app.editor.active().unwrap();
    (
        t.x + TEXT_PAD - doc.scroll_x + (col as f32 + 0.5) * app.metrics.cell,
        t.y + ((line as f64 - doc.scroll_line) as f32 + 0.5) * app.fonts.line_height,
    )
}

/// 第 `line` 行第 `col` 列之前的字符边界的窗口坐标。
fn before(app: &App, line: usize, col: usize) -> (f32, f32) {
    let (x, y) = over(app, line, col);
    (x - app.metrics.cell / 2.0, y)
}

/// 在 `from` 按下左键，移到 `to`，按住修饰键 `mods` 松开。
fn drag(app: &mut App, from: (f32, f32), to: (f32, f32), mods: ModifiersState) {
    app.pointer.last_click = None;
    app.on_mouse_move(from.0, from.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(to.0, to.1);
    app.pointer.modifiers = mods;
    app.on_mouse_up(MouseButton::Left);
    app.pointer.modifiers = ModifiersState::empty();
}

fn text(app: &App) -> String {
    app.editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap()
}

fn selected(app: &App) -> (usize, usize) {
    let s = app.editor.active().unwrap().selection;
    (s.from(), s.to())
}

/// 当前系统上松开时复制所按的键。
fn copy_key() -> ModifiersState {
    if cfg!(target_os = "macos") {
        ModifiersState::ALT
    } else {
        ModifiersState::CONTROL
    }
}

#[test]
fn dragging_the_selection_moves_it_and_undo_puts_it_back() {
    let (mut app, dir) = drag_app("text-drag-move", false);
    // 拖动途中显示将放下的位置，指针为箭头。
    app.pointer.last_click = None;
    let (from, to) = (over(&app, 0, 6), before(&app, 1, 4));
    app.on_mouse_move(from.0, from.1);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(selected(&app), (4, 9), "按下时不收起选区");
    app.on_mouse_move(to.0, to.1);
    assert_eq!(app.text_drop(), Some(19));
    paint(&mut app);
    app.on_mouse_up(MouseButton::Left);

    assert_eq!(text(&app), "let  = 1;\nlet alphabeta = 2;\n");
    assert_eq!(selected(&app), (14, 19), "选区落在移过去的文字上");
    assert_eq!(app.text_drop(), None);
    // 移动是一次撤销。
    app.run_action(Action::Undo);
    assert_eq!(text(&app), SAMPLE);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn holding_the_copy_key_on_release_copies_the_selection() {
    let (mut app, dir) = drag_app("text-drag-copy", false);
    let (from, to) = (over(&app, 0, 6), before(&app, 1, 4));
    drag(&mut app, from, to, copy_key());
    assert_eq!(text(&app), "let alpha = 1;\nlet alphabeta = 2;\n");
    assert_eq!(selected(&app), (19, 24));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 在选区上按下后没有移动就松开：按单击放置光标；放回选区之内：文字不动，选区保留。
#[test]
fn a_click_or_a_drop_inside_the_selection_changes_nothing() {
    let (mut app, dir) = drag_app("text-drag-click", false);
    let at = before(&app, 0, 6);
    drag(&mut app, at, at, ModifiersState::empty());
    assert_eq!(text(&app), SAMPLE);
    assert_eq!(selected(&app), (6, 6));

    app.editor.active_mut().unwrap().selection = Selection::new(4, 9);
    let (from, to) = (over(&app, 0, 5), before(&app, 0, 8));
    drag(&mut app, from, to, ModifiersState::empty());
    assert_eq!(text(&app), SAMPLE);
    assert_eq!(selected(&app), (4, 9));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 关闭拖放编辑后，在选区上按下拖动是重新拖选。
#[test]
fn with_drag_and_drop_off_pressing_on_the_selection_selects_anew() {
    let (mut app, dir) = drag_app("text-drag-off", false);
    app.config.drag_and_drop = Some(false);
    let (from, to) = (before(&app, 0, 6), before(&app, 1, 4));
    drag(&mut app, from, to, ModifiersState::empty());
    assert_eq!(text(&app), SAMPLE);
    assert_eq!(selected(&app), (6, 19));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：拖动选中的文字途中，将放下的位置显示一条竖线，浅色、深色各一张。运行方式：
/// `cargo test -p winc-app shoot_text_drag -- --ignored`，图片写到 `WINC_SNAPSHOT_DIR`
/// （默认是系统临时目录下的 `winc-snapshots`）。
#[test]
#[ignore = "截图"]
fn shoot_text_drag() {
    for (tag, dark) in [("light", false), ("dark", true)] {
        let (mut app, dir) = drag_app(&format!("text-drag-shot-{tag}"), dark);
        let (from, to) = (over(&app, 0, 6), before(&app, 1, 4));
        app.on_mouse_move(from.0, from.1);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(to.0, to.1);
        shoot(&mut app, &format!("text-drag-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
