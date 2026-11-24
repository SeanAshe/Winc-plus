//! 可编辑的预览中块之间多出的空行：每行占一行高度，光标可以停在上面，删除时逐行去掉；
//! 标题开头的回车和任务项的 ⌘↩。

use winit::keyboard::{Key, NamedKey};

use super::*;

/// 在可编辑的预览中按下 `key`（`cmd` 为真时同时按住 ⌘），未处理时按编辑区的处理执行，
/// 之后等待排版。
fn press(app: &mut App, key: NamedKey, cmd: bool) {
    let key = Key::Named(key);
    if !app.markdown_key(&key, cmd) {
        match key {
            Key::Named(NamedKey::Backspace) => app.run(Command::Backspace { word: false }),
            Key::Named(NamedKey::Delete) => app.run(Command::Delete { word: false }),
            // 编辑区逐字移动。
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                let doc = app.editor.active_mut().unwrap();
                let head = doc.selection.head;
                let head = if key == Key::Named(NamedKey::ArrowLeft) {
                    head - 1
                } else {
                    head + 1
                };
                doc.selection = winc_core::Selection::point(head);
            }
            _ => panic!("{key:?} 未处理"),
        }
    }
    settle(app);
}

#[test]
fn extra_blank_lines_show_and_hold_the_caret() {
    let (mut app, dir) = live_doc("md-blank-lines", "甲\n\n乙\n\n\n\n丙\n");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert_eq!(laid.blocks.len(), 5);
    // 「乙」与「丙」之间多出两个空行，比「甲」「乙」之间多两行的高度。
    let (one, three) = (laid.tops[1] - laid.tops[0], laid.tops[4] - laid.tops[1]);
    assert!(three - one > laid.key.base * 2.0, "{one} {three}");

    put_caret(&mut app, "乙", true);
    let mut ys = vec![caret_screen_y(&app)];
    for expected in ["乙\n\n|\n\n丙", "乙\n\n\n|\n丙", "乙\n\n\n\n|丙"] {
        press(&mut app, NamedKey::ArrowRight, false);
        assert!(with_caret(&app).contains(expected), "{}", with_caret(&app));
        ys.push(caret_screen_y(&app));
    }
    assert!(
        ys.is_sorted() && ys.windows(2).all(|w| w[1] > w[0]),
        "{ys:?}"
    );
    press(&mut app, NamedKey::ArrowLeft, false);
    assert!(
        with_caret(&app).contains("乙\n\n\n|\n丙"),
        "{}",
        with_caret(&app)
    );
    press(&mut app, NamedKey::ArrowUp, false);
    assert!(
        with_caret(&app).contains("乙\n\n|\n\n丙"),
        "{}",
        with_caret(&app)
    );

    // 退格逐行去掉空行：第一个空行上时光标回到上一段的末尾。
    put_caret(&mut app, "丙", false);
    press(&mut app, NamedKey::Backspace, false);
    assert!(
        with_caret(&app).contains("乙\n\n\n|丙"),
        "{}",
        with_caret(&app)
    );
    press(&mut app, NamedKey::ArrowLeft, false);
    press(&mut app, NamedKey::Backspace, false);
    assert!(
        with_caret(&app).contains("乙|\n\n丙"),
        "{}",
        with_caret(&app)
    );
    // 段落末尾回车：新的空行上有光标，其后不多出空行。
    press(&mut app, NamedKey::Enter, false);
    assert!(
        with_caret(&app).contains("乙\n\n|\n丙"),
        "{}",
        with_caret(&app)
    );
    assert_eq!(
        app.preview.markdown.layout.as_ref().unwrap().blocks.len(),
        4
    );
    press(&mut app, NamedKey::Delete, false);
    assert!(
        with_caret(&app).contains("乙\n\n|丙"),
        "{}",
        with_caret(&app)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enter_at_the_start_of_a_heading_keeps_it() {
    let (mut app, dir) = live_doc("md-heading-enter", "段落\n\n## 标题\n");
    put_caret(&mut app, "标题", false);
    press(&mut app, NamedKey::Enter, false);
    assert_eq!(with_caret(&app), "段落\n\n\n## |标题\n");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert_eq!(laid.blocks.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_enter_toggles_task_items() {
    let (mut app, dir) = live_doc("md-task-toggle", "- [ ] 甲\n- [x] 乙\n- 丙\n");
    put_caret(&mut app, "甲", true);
    press(&mut app, NamedKey::Enter, true);
    assert!(
        doc_text(&app).starts_with("- [x] 甲\n"),
        "{}",
        doc_text(&app)
    );
    // 选中的各项有未勾选的时全部勾选，否则全部取消；一次撤销恢复。
    let rope = app.editor.active().unwrap().text().rope().unwrap().clone();
    let end = rope.len_chars();
    app.editor.active_mut().unwrap().selection = winc_core::Selection::new(0, end);
    press(&mut app, NamedKey::Enter, true);
    assert_eq!(doc_text(&app), "- [ ] 甲\n- [ ] 乙\n- 丙\n");
    app.run(Command::Undo);
    assert_eq!(doc_text(&app), "- [x] 甲\n- [x] 乙\n- 丙\n");
    // 不是任务项时不处理。
    put_caret(&mut app, "丙", true);
    assert!(!app.markdown_key(&Key::Named(NamedKey::Enter), true));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 光标停在空行上、在空行上输入第一个字时，各块的位置和高度不变。
#[test]
fn the_empty_line_keeps_its_height_under_the_caret() {
    let (mut app, dir) = live_doc("md-blank-height", "前文\n\n\n- 一\n");
    let shape = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let heights: Vec<f32> = laid.blocks.iter().map(|b| b.1.height).collect();
        (laid.tops.clone(), heights)
    };
    let away = shape(&app);
    assert_eq!(away.0.len(), 3);
    let at = doc_text(&app).find("\n- ").unwrap();
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(doc.text().rope().unwrap().byte_to_char(at));
    settle(&mut app);
    assert_eq!(shape(&app), away);
    app.run(Command::Insert("新".into()));
    settle(&mut app);
    assert_eq!(shape(&app), away);
    let _ = std::fs::remove_dir_all(&dir);
}
