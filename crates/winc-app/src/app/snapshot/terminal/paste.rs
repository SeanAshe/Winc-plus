//! 向终端粘贴剪贴板图片的测试。

use super::*;

/// 剪贴板里是图片时，粘贴把图片存成临时文件，把路径发给终端里的程序。
#[test]
fn pasting_a_clipboard_image_into_the_terminal_sends_its_path() {
    use winc_pty::protocol::{Modes, ToHelper};
    let dir = sample_folder("terminal-image-paste");
    let mut app = blank_app(true);
    show_terminal(&mut app, "$ ");
    let log = app.attach_recording_helper(&dir);
    app.terminal.tab_mut().unwrap().modes = Modes(Modes::BRACKETED_PASTE);
    app.clipboard.copied = None;
    app.clipboard.copied_image = Some(b"png-bytes".to_vec());
    assert!(app.run_action(crate::keymap::Action::Paste));

    let inputs = |app: &App| -> Vec<Vec<u8>> {
        app.sent_to_helper(&log)
            .into_iter()
            .filter_map(|message| match message {
                ToHelper::Input { bytes, .. } => Some(bytes),
                _ => None,
            })
            .collect()
    };
    let text = String::from_utf8(inputs(&app).concat()).unwrap();
    let path = text
        .strip_prefix("\u{1b}[200~")
        .and_then(|body| body.strip_suffix("\u{1b}[201~"))
        .unwrap_or(&text);
    assert!(path.contains("winc-paste"), "{path}");
    assert!(!path.contains('\r'), "{path}");
    assert_eq!(std::fs::read(path).unwrap(), b"png-bytes");
    std::fs::remove_file(path).unwrap();

    // 同时有文字时仍粘贴文字，不写图片文件。
    app.clipboard.copied = Some("hello".into());
    app.clipboard.copied_image = Some(b"other".to_vec());
    assert!(app.run_action(crate::keymap::Action::Paste));
    let sent = inputs(&app);
    assert_eq!(sent.last().unwrap(), b"\x1b[200~hello\x1b[201~");
    std::fs::remove_dir_all(&dir).unwrap();
}
