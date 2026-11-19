//! 十六进制视图的测试：从二进制文件的占位页进入，点选、拖选、键盘移动、复制、转到偏移，文件
//! 变化后重新读取，以及从编码菜单切回文本。

use super::*;
use crate::app::PlaceholderAction;
use crate::app::input::context_menu::MenuCommand;
use winc_editor::Encoding;
use winc_editor::encoding::CHOICES;
use winit::keyboard::{Key, NamedKey};

/// 可执行文件开头那样的一段字节：前两行有 NUL，第 5 行起是一句话。
fn sample() -> Vec<u8> {
    let mut bytes = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xFF\xFF\x00\x00".to_vec();
    bytes.extend_from_slice(&[0xB8, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x1A, 0, 0, 0, 0, 0]);
    bytes.extend(std::iter::repeat_n(0u8, 32));
    bytes.extend_from_slice(b"This program cannot be run in DOS mode.\r\r\n$");
    bytes.extend(std::iter::repeat_n(0xCCu8, 4000));
    bytes
}

/// 打开 `name` 里的二进制文件，在占位页上点「以十六进制打开」。
fn hex_app(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("app.exe");
    std::fs::write(&path, sample()).unwrap();
    let mut app = app_with(&path, dark);
    app.run_placeholder_action(PlaceholderAction::ShowHex);
    app.editor.load_active();
    paint(&mut app);
    (app, dir)
}

fn view(app: &App) -> &winc_editor::hex::HexView {
    app.editor.active().unwrap().hex.as_deref().unwrap()
}

/// 十六进制一栏里第 `row` 行第 `i` 个字节的中心（窗口坐标）。
fn byte_point(app: &App, row: u64, i: u64) -> (f32, f32) {
    let parts = app.hex_parts();
    let lh = app.fonts.line_height;
    (
        parts.hex_at(i) + parts.cell,
        parts.body.y + row as f32 * lh + lh / 2.0,
    )
}

#[test]
fn bytes_are_selected_copied_and_found_by_offset() {
    let (mut app, dir) = hex_app("hex-select", false);
    assert!(app.hex_active());
    assert_eq!(view(&app).size(), sample().len() as u64);
    // 没有文本：Git 的比较不拿它当文件的内容，保存被拒绝并说明是十六进制视图。
    let doc = app.editor.active_mut().unwrap();
    assert!(!doc.holds_file_text());
    let error = doc.save().unwrap_err().to_string();
    assert_eq!(error, winc_i18n::t!("editor-readonly-hex"));

    // 点列标题一行不选字节。
    let header = app.hex_parts().header;
    let x = byte_point(&app, 0, 5).0;
    click(&mut app, (x, header.y + header.h / 2.0));
    assert_eq!(view(&app).selection(), 0..1);

    // 点第 0 行第 1 个字节，⇧ 点第 3 个：选中 3 个字节，两栏都铺底色。
    let (x, y) = byte_point(&app, 0, 1);
    click(&mut app, (x, y));
    assert_eq!(view(&app).selection(), 1..2);
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    let (x, y) = byte_point(&app, 0, 3);
    click(&mut app, (x, y));
    app.pointer.modifiers = Default::default();
    assert_eq!(view(&app).selection(), 1..4);
    app.copy(false);
    assert_eq!(app.clipboard.copied.as_deref(), Some("5A 90 00"));
    app.copy_hex(true);
    assert_eq!(app.clipboard.copied.as_deref(), Some("Z\u{FFFD}\0"));

    // 键盘：↓ 到下一行同一列，End 到行尾，⌘End 到最后一个字节并滚动到可见。
    app.hex_key(&Key::Named(NamedKey::ArrowDown), false);
    assert_eq!(view(&app).head, 19);
    app.hex_key(&Key::Named(NamedKey::End), false);
    assert_eq!(view(&app).head, 31);
    app.hex_key(&Key::Named(NamedKey::End), true);
    let last = view(&app).size() - 1;
    assert_eq!(view(&app).selection(), last..last + 1);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);

    // ⌃G 转到偏移，十六进制和十进制都行。
    app.open_goto();
    assert_eq!(app.bars.prompt_kind, PromptKind::GoToOffset);
    app.go_to_offset("0x40");
    assert_eq!(view(&app).head, 0x40);
    app.go_to_offset("17");
    assert_eq!(view(&app).head, 17);

    // 全选后复制超过上限的不会发生：这个文件很小，照常复制。
    app.select_all_hex();
    app.copy(false);
    assert!(
        app.clipboard
            .copied
            .as_deref()
            .unwrap()
            .starts_with("4D 5A 90 00")
    );
    paint(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_view_follows_the_file_and_returns_to_text() {
    let (mut app, dir) = hex_app("hex-follow", false);
    let path = dir.join("app.exe");
    let id = app.editor.active().unwrap().id;

    // 编码菜单：「十六进制」一项是选中的，文本编码都可选，不能按编码保存。
    paint(&mut app);
    app.open_status_menu(crate::app::status::status_bar::StatusItem::Encoding);
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::ReopenHex, false)));
    assert!(commands.contains(&(MenuCommand::ReopenEncoding(0), true)));
    assert!(commands.contains(&(MenuCommand::SaveEncoding(0), false)));
    assert!(app.menu_checked(MenuCommand::ReopenHex));
    app.overlays.context_menu = None;

    // 文件被改短：重新读取，选区收到新的大小以内。
    app.go_to_offset("0x60");
    std::fs::write(&path, b"MZ").unwrap();
    app.check_disk();
    assert_eq!((view(&app).size(), view(&app).head), (2, 1));

    // 选 UTF-8：换回文本（只读的二进制文本），不再是十六进制视图。
    let utf8 = (CHOICES.iter())
        .position(|(e, _)| *e == Encoding::Utf8)
        .unwrap();
    app.run_status_command(MenuCommand::ReopenEncoding(utf8), id);
    app.editor.load_active();
    let doc = app.editor.active().unwrap();
    assert!(doc.hex.is_none());
    assert_eq!(doc.text().rope().unwrap().to_string(), "MZ");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：从占位页进入的十六进制视图，选中第 0x4E 起的 4 个字节；浅色与深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_hex_view() {
    for dark in [false, true] {
        let (mut app, dir) = hex_app("hex-shot", dark);
        app.go_to_offset("0x4E");
        app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
        app.hex_key(&Key::Named(NamedKey::ArrowRight), false);
        app.hex_key(&Key::Named(NamedKey::ArrowRight), false);
        app.hex_key(&Key::Named(NamedKey::ArrowRight), false);
        app.pointer.modifiers = Default::default();
        let tag = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("hex-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 有未保存修改的文档切到十六进制之前要确认：没有窗口（测试中）时只在状态栏说明，修改留着。
#[test]
fn a_dirty_document_is_not_switched_to_hex_silently() {
    let dir = sample_folder("hex-dirty");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.run(winc_editor::Command::Insert("x".into()));
    app.show_hex();
    let doc = app.editor.active().unwrap();
    assert!(doc.hex.is_none() && doc.is_dirty());
    assert_eq!(
        app.status.message.as_deref(),
        Some(winc_i18n::t!("workspace-reopen-dirty"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 超过 16 MB 的文件按十六进制查看时，编码菜单里只有 UTF-8 能切回文本。
#[test]
fn a_large_hex_file_returns_to_text_only_as_utf8() {
    let dir = sample_folder("hex-large");
    let path = dir.join("big.bin");
    std::fs::write(&path, vec![0u8; winc_editor::LARGE_FILE as usize + 1]).unwrap();
    let mut app = app_with(&path, false);
    app.run_placeholder_action(PlaceholderAction::ShowHex);
    app.editor.load_active();
    paint(&mut app);
    app.open_status_menu(crate::app::status::status_bar::StatusItem::Encoding);
    let utf8 = (CHOICES.iter())
        .position(|(e, _)| *e == Encoding::Utf8)
        .unwrap();
    for (command, enabled) in menu_commands(&app) {
        if let MenuCommand::ReopenEncoding(i) = command {
            assert_eq!(enabled, i == utf8, "{i}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
