//! 临时打开模式的测试：标准输入标签的读取、读完后的编辑和另存为。

use super::*;
use crate::bench::Bench;

/// 测试中标准输入标签的标题：测试以源语言运行，与 [`stdin_title`] 的结果相同。
const STDIN_TITLE: &str = "标准输入";

/// 返回打开了空的标准输入标签、正在接收的应用，以及标签和向它发送内容的通道。
fn reading_app() -> (App, DocumentId, mpsc::Sender<Piece>) {
    let mut app = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    let index = app.editor.show_output(STDIN_TITLE, "");
    let id = app.editor.documents[index].id;
    let (send, receive) = mpsc::channel();
    app.one_off.stdin = Some((id, receive));
    (app, id, send)
}

#[test]
fn closing_the_stdin_tab_stops_reading() {
    let (mut app, id, send) = reading_app();

    send.send(Piece::Text("one\n".to_owned())).unwrap();
    app.poll_stdin();
    assert_eq!(app.editor.documents[0].text().line_count(), 2);

    // 关闭标签后 `poll_stdin` 丢弃接收端，发送端再发送会返回错误，后台线程据此退出。
    app.remove_tabs(&[id]);
    let _ = send.send(Piece::Text("two\n".to_owned()));
    app.poll_stdin();
    assert!(app.one_off.stdin.is_none());
    assert!(send.send(Piece::Text("three\n".to_owned())).is_err());
}

#[test]
fn finished_stdin_becomes_an_editable_untitled_document() {
    let (mut app, id, send) = reading_app();
    send.send(Piece::Text("one\n".to_owned())).unwrap();
    app.poll_stdin();
    let doc = app.editor.document(id).unwrap();
    assert_eq!(
        doc.unwritable_reason(),
        Some(winc_i18n::t!("editor-readonly-tab"))
    );

    send.send(Piece::Done(None)).unwrap();
    app.poll_stdin();
    assert!(!app.reading_stdin(id));
    let doc = app.editor.document(id).unwrap();
    assert_eq!(doc.name(), STDIN_TITLE);
    assert!(doc.title.is_none() && doc.unwritable_reason().is_none());
    // 读入的内容是保存基线：不修改时关闭不询问。
    assert!(!doc.is_dirty());
    assert_eq!(app.save_name(id), format!("{STDIN_TITLE}.txt"));
}

#[test]
fn saving_stdin_while_reading_turns_the_tab_into_the_file() {
    let dir = std::env::temp_dir().join(format!("winc-stdin-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut app, id, send) = reading_app();
    send.send(Piece::Text("one\n".to_owned())).unwrap();
    app.poll_stdin();

    // 上级是普通文件，保存失败：仍是只读的输出标签，继续接收。
    let file = dir.join("file");
    std::fs::write(&file, "").unwrap();
    assert!(!app.save_picked(id, file.join("out.txt")));
    assert!(app.reading_stdin(id));
    let doc = app.editor.document(id).unwrap();
    assert_eq!(doc.title.as_deref(), Some(STDIN_TITLE));
    assert!(doc.unwritable_reason().is_some());

    send.send(Piece::Text("two\n".to_owned())).unwrap();
    app.poll_stdin();
    let path = dir.join("out.txt");
    assert!(app.save_picked(id, path.clone()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    let doc = app.editor.document(id).unwrap();
    assert_eq!(doc.name(), "out.txt");
    assert!(doc.unwritable_reason().is_none() && !doc.is_dirty());
    // 保存后不再接收：接收端已丢弃，后台线程下次发送时退出。
    assert!(!app.reading_stdin(id));
    assert!(send.send(Piece::Text("three\n".to_owned())).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}
