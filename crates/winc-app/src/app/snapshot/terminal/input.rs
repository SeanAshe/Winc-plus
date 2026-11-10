//! 终端输入的测试：修饰键的编码与剪贴板中没有文字时的粘贴。
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
    app.copied = None;

    assert!(press(&mut app, &["term"], Char('v')));
    assert_eq!(sent_input(&app, &log), b"");
    show_terminal(&mut app, "\x1b[?2004h");
    assert!(press(&mut app, &["term"], Char('v')));
    assert_eq!(sent_input(&app, &log), b"\x1b[200~\x1b[201~");
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
    app.modifiers = winit::keyboard::ModifiersState::ALT;
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
