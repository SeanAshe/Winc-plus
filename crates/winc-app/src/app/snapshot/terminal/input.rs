//! 终端输入的测试：修饰键的编码、剪贴板中没有文字时的粘贴，以及右键单击复制或粘贴。
//!
//! 写给程序的字节由记录用的辅助进程截取（`attach_recording_helper`）。

use super::*;
use super::{cell_center, show_terminal};
use std::path::Path;
use winc_pty::protocol::ToHelper;

/// 返回截至目前写给程序的全部字节。
fn sent_input(app: &App, log: &Path) -> Vec<u8> {
    let sent = app.sent_to_helper(log).into_iter();
    let bytes = sent.filter_map(|message| match message {
        ToHelper::Input { bytes, .. } => Some(bytes),
        _ => None,
    });
    bytes.flatten().collect()
}

/// 剪贴板中没有文字时粘贴：程序开启了括号粘贴时发送一次空的括号粘贴，供程序读取剪贴板中的
/// 图片；未开启时不发送任何内容。
///
/// 回归：剪贴板中只有图片时，粘贴不向程序发送任何内容。
#[test]
fn pasting_without_text_sends_an_empty_bracketed_paste() {
    let dir = sample_folder("terminal-empty-paste");
    let mut app = blank_app(true);
    show_terminal(&mut app, "");
    let log = app.attach_recording_helper(&dir);
    app.clipboard.copied = None;

    assert!(press(&mut app, &["term"], Char('v')));
    assert_eq!(sent_input(&app, &log), b"");
    show_terminal(&mut app, "\x1b[?2004h");
    assert!(press(&mut app, &["term"], Char('v')));
    assert_eq!(sent_input(&app, &log), b"\x1b[200~\x1b[201~");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 剪贴板中的图片存为本机文件后粘贴：程序收到一次括号粘贴，内容是文件的路径（按需加引号），
/// 文件是剪贴板中的 PNG；粘贴期间换了标签时不粘贴，提示存到了哪里。只在 Windows 上由粘贴触发，
/// 其他平台上粘贴仍是一次空的括号粘贴，由程序自己读取剪贴板。
#[test]
fn clipboard_images_saved_as_files_paste_their_path() {
    let dir = sample_folder("terminal image paste");
    *crate::app::terminal::local_paste::DIR.lock().unwrap() = Some(dir.join("saved"));
    let mut app = blank_app(true);
    show_terminal(&mut app, "\x1b[?2004h");
    let log = app.attach_recording_helper(&dir);
    let id = app.terminal.tab().unwrap().id;
    let png = winc_editor::paste::encode_png(1, 1, &[255, 0, 0, 255]).unwrap();
    app.clipboard.copied = None;
    app.clipboard.copied_image = Some(png.clone());

    if !cfg!(windows) {
        app.terminal_paste();
        assert_eq!(sent_input(&app, &log), b"\x1b[200~\x1b[201~");
    }
    let before = sent_input(&app, &log).len();
    assert!(app.save_clipboard_image(id));
    let sent = String::from_utf8(sent_input(&app, &log)[before..].to_vec()).unwrap();
    let quoted = (sent.strip_prefix("\x1b[200~"))
        .and_then(|text| text.strip_suffix("\x1b[201~"))
        .unwrap_or_else(|| panic!("{sent:?}"));
    // 存放目录的名字里有空格，路径加了引号。
    let path = quoted.trim_matches(['\'', '"']);
    assert_ne!(path, quoted);
    assert!(
        path.starts_with(dir.join("saved").to_str().unwrap()),
        "{path}"
    );
    assert_eq!(std::fs::read(path).unwrap(), png);

    let before = sent_input(&app, &log).len();
    app.terminal_image_saved(id + 1, Ok(path.into()));
    assert_eq!(sent_input(&app, &log).len(), before);
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("已存为"), "{notice}");
    app.clipboard.copied_image = None;
    assert!(!app.save_clipboard_image(id));

    *crate::app::terminal::local_paste::DIR.lock().unwrap() = None;
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Alt 是否视为 Meta：macOS 上按 `terminal.optionAsMeta` 判断，其他平台一律视为 Meta。
///
/// winit 的 `KeyEvent` 无法在其 crate 之外构造，因此通过 SGR 鼠标上报检查：Meta 对应按钮编号
/// 中值为 8 的位。回归：Windows 与 Linux 上按 Alt 组合键时，程序收到的按键不带 Meta。
#[test]
fn alt_is_meta_unless_the_platform_types_with_it() {
    let dir = sample_folder("terminal-alt-meta");
    let mut app = blank_app(true);
    show_terminal(&mut app, "\x1b[?1000h\x1b[?1006h");
    let log = app.attach_recording_helper(&dir);
    paint(&mut app);
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    let (x, y) = cell_center(&app, 0, 0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(winit::event::MouseButton::Left);
    app.on_mouse_up(winit::event::MouseButton::Left);

    // 左右 ⌥ 均未记录为按下；macOS 按默认设置只把左 ⌥ 视为 Meta，因此上报中不带 Meta。
    let button = if cfg!(target_os = "macos") { 0 } else { 8 };
    let report = format!("\x1b[<{button};1;1M\x1b[<{button};1;1m");
    assert_eq!(String::from_utf8_lossy(&sent_input(&app, &log)), report);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 终端中组字时候选框随输入法报告的光标移动；输入法要求隐藏光标时对齐组字文字的左边。
#[test]
fn the_terminal_composition_caret_follows_the_input_method() {
    use winit::event::Ime;

    let mut app = blank_app(true);
    show_terminal(&mut app, "");
    let at = |app: &mut App, cursor: Option<(usize, usize)>| {
        app.on_ime(Ime::Preedit("abc".into(), cursor));
        paint(app);
        app.ime.cursor.unwrap().x
    };
    let (start, end) = (at(&mut app, Some((0, 0))), at(&mut app, Some((3, 3))));
    let cell = app.metrics.terminal_cell;
    assert!((end - start - 3.0 * cell).abs() < 0.5, "{start} {end}");
    assert!((at(&mut app, None) - start).abs() < 0.5);
}

/// 右键单击：默认弹出终端菜单。设为「复制或粘贴」后，有所选内容时复制并取消选择，没有时粘贴；
/// 开启了选中时复制时所选内容已复制过，直接粘贴；按住 Alt（macOS 上为 ⌥）仍弹出菜单。
#[test]
fn right_click_copies_or_pastes_instead_of_the_menu() {
    use crate::app::terminal::{Point, Selection, Unit};
    let dir = sample_folder("terminal-right-click");
    let mut app = blank_app(true);
    show_terminal(&mut app, "hello\r\n");
    paint(&mut app);
    let log = app.attach_recording_helper(&dir);
    let at = cell_center(&app, 0, 2);
    let select = |app: &mut App| {
        let tab = app.terminal.tab_mut().unwrap();
        let line = tab.top();
        tab.selection = Some(Selection {
            anchor: Point { line, col: 0 },
            head: Point { line, col: 5 },
            unit: Unit::Char,
        });
    };

    right_click(&mut app, at);
    assert!(app.overlays.context_menu.is_some());
    app.overlays.context_menu = None;

    app.config.terminal_right_click = Some(winc_config::TerminalRightClick::CopyOrPaste);
    select(&mut app);
    app.clipboard.copied = None;
    right_click(&mut app, at);
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(app.clipboard.copied.as_deref(), Some("hello"));
    assert!(app.terminal.tab().unwrap().selection.is_none());
    assert_eq!(sent_input(&app, &log), b"");
    right_click(&mut app, at);
    assert_eq!(sent_input(&app, &log), b"hello");

    app.config.terminal_copy_on_select = Some(true);
    select(&mut app);
    right_click(&mut app, at);
    assert_eq!(sent_input(&app, &log), b"hellohello");

    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    right_click(&mut app, at);
    assert!(app.overlays.context_menu.is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}
