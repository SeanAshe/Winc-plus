//! 粘贴的测试：Markdown 中的链接与图片、其他文档中的路径、文件树中的复制粘贴（文件，以及
//! 粘贴为新文件的文本与图片）。

use super::*;
use crate::keymap::KeyCode::Char;
use navigation::press;

/// 在 `dir` 的 `docs` 子目录中写入内容为 `text` 的 `notes.md` 并打开，光标放在末尾。
fn open_markdown(app: &mut App, dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("docs/notes.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    app.open_path(&path);
    let end = text.chars().count();
    app.select(end, end);
    path
}

fn text(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    doc.text_range(0, doc.text().end_pos()).unwrap()
}

fn selected(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    let range = doc.selection.range();
    doc.text_range(range.start, range.end).unwrap()
}

#[test]
fn files_pasted_into_markdown_become_links_and_outside_images_are_copied() {
    let dir = search_folder("paste-files");
    let mut app = app_with(&dir, false);
    open_markdown(&mut app, &dir, "看这里：");

    // 项目之外的图片复制到文档旁边再引用；项目里的文件按相对路径引用，多项之间用空格隔开。
    let outside = std::env::temp_dir().join(format!("winc-paste-{}.png", std::process::id()));
    std::fs::write(&outside, b"png").unwrap();
    app.copied_files = vec![dir.join("a.txt"), outside.clone()];
    app.paste();
    let photo = outside.file_name().unwrap().to_string_lossy().into_owned();
    let stem = photo.trim_end_matches(".png");
    assert_eq!(
        text(&app),
        format!("看这里：[a.txt](../a.txt) ![{stem}]({photo})")
    );
    assert_eq!(
        std::fs::read(dir.join("docs").join(&photo)).unwrap(),
        b"png"
    );

    // 只粘贴一项时选中链接文字，直接键入即可替换。
    app.copied_files = vec![dir.join("src/main.rs")];
    app.paste();
    assert_eq!(selected(&app), "main.rs");
    assert!(text(&app).ends_with("[main.rs](../src/main.rs)"));

    let _ = std::fs::remove_file(&outside);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_image_pasted_into_markdown_is_saved_beside_the_document() {
    let dir = search_folder("paste-image");
    let mut app = app_with(&dir, false);
    open_markdown(&mut app, &dir, "");

    app.copied_image = Some(b"first".to_vec());
    app.paste();
    assert_eq!(text(&app), "![image](image.png)");
    assert_eq!(selected(&app), "image");
    assert_eq!(std::fs::read(dir.join("docs/image.png")).unwrap(), b"first");

    // 再粘贴一张：不覆盖已有的图片。
    app.select(19, 19);
    app.paste();
    assert!(text(&app).ends_with("![image-1](image-1.png)"));
    assert_eq!(
        std::fs::read(dir.join("docs/image-1.png")).unwrap(),
        b"first"
    );

    // 剪贴板中同时有文本和图片时（例如从表格中复制）粘贴文本。
    let end = text(&app).chars().count();
    app.select(end, end);
    app.copied = Some("单元格".into());
    app.paste();
    assert!(text(&app).ends_with("单元格"));
    assert!(!dir.join("docs/image-2.png").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_documents_get_paths_and_refuse_images() {
    let dir = search_folder("paste-plain");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    app.select(0, 0);

    // 不是 Markdown：文件写成绝对路径，每行一个。
    app.copied_files = vec![dir.join("a.txt"), dir.join("Cargo.toml")];
    app.paste();
    let expected = format!(
        "{}\n{}fn main()",
        dir.join("a.txt").display(),
        dir.join("Cargo.toml").display()
    );
    assert!(text(&app).starts_with(&expected), "{}", text(&app));

    // 图片不能粘贴到其他文档：提示原因，不在磁盘上留下文件。
    app.copied_files.clear();
    app.copied_image = Some(b"png".to_vec());
    let before = text(&app);
    app.paste();
    assert_eq!(text(&app), before);
    assert!(
        app.status
            .message
            .as_deref()
            .is_some_and(|m| m.contains("Markdown"))
    );
    assert!(!dir.join("src/image.png").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_tree_copies_files_and_pastes_text_and_images_as_new_files() {
    let folder = search_folder("paste-tree");
    let mut app = app_with(&folder, false);
    // 文件树中的路径以工作区的根目录为准（可能是解析过符号链接的真实路径）。
    let dir = app.editor.root().unwrap().to_path_buf();
    let select = |app: &mut App, path: &Path| {
        let ws = app.editor.workspace.as_mut().unwrap();
        ws.reveal(path);
        app.focus = Focus::Tree;
    };
    let selected_path = |app: &App| {
        let ws = app.editor.workspace.as_ref().unwrap();
        ws.selected_entry().map(|e| e.path.clone())
    };

    // ⌘C 复制选中的文件本身，⌘V 粘贴到同一目录中，得到副本并选中该副本。
    select(&mut app, &dir.join("a.txt"));
    assert!(press(&mut app, &["cmd"], Char('c')));
    assert_eq!(app.copied_files, [dir.join("a.txt")]);
    assert!(press(&mut app, &["cmd"], Char('v')));
    let copy = dir.join("a 副本.txt");
    assert_eq!(
        std::fs::read(&copy).unwrap(),
        std::fs::read(dir.join("a.txt")).unwrap()
    );
    assert_eq!(selected_path(&app), Some(copy));

    // 选中目录时粘贴到该目录中，目录连同内容一起复制；粘贴的正是选中的目录时，
    // 在旁边复制出副本。
    std::fs::create_dir(dir.join("src/inner")).unwrap();
    select(&mut app, &dir.join("src"));
    app.copied_files = vec![dir.join("Cargo.toml"), dir.join("src")];
    app.paste();
    assert!(dir.join("src/Cargo.toml").is_file());
    assert!(dir.join("src 副本/main.rs").is_file());
    assert_eq!(selected_path(&app), Some(dir.join("src 副本")));
    // 目录不能粘贴到该目录内部更深的位置：选中 `src/inner` 后再粘贴 `src`。
    select(&mut app, &dir.join("src/inner"));
    app.copied_files = vec![dir.join("src")];
    app.paste();
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|m| m.contains("自身内部"))
    );

    // 剪贴板中只有文本：粘贴为新文件。名字输入行里填好按内容猜的名字，选中主名，直接键入即
    // 替换；按 ↩ 新建并打开，内容就是剪贴板中的文本。
    select(&mut app, &dir.join("Cargo.toml"));
    app.copied_files.clear();
    app.copied = Some("{\"name\": \"winc\"}".into());
    app.paste();
    let entry = app.name_input.as_ref().expect("打开名字输入行");
    assert_eq!(entry.dir, dir);
    assert_eq!(entry.field.text(), "untitled.json");
    assert_eq!(entry.field.selected_text(), "untitled");
    assert_eq!(app.focus, Focus::Prompt);
    app.edit_field(|f| f.insert("package"));
    app.submit_prompt();
    assert!(app.name_input.is_none());
    assert_eq!(
        std::fs::read_to_string(dir.join("package.json")).unwrap(),
        "{\"name\": \"winc\"}"
    );
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.path.as_deref(),
        Some(dir.join("package.json").as_path())
    );

    // 剪贴板中只有图片：同样粘贴为新文件，建议的名字是 `image.png`。按 Esc 不新建。
    app.copied = None;
    app.copied_image = Some(b"png".to_vec());
    select(&mut app, &dir.join("Cargo.toml"));
    app.paste();
    assert_eq!(app.name_input.as_ref().unwrap().field.text(), "image.png");
    app.close_name_input();
    assert!(!dir.join("image.png").exists());
    select(&mut app, &dir.join("Cargo.toml"));
    app.paste();
    app.submit_prompt();
    assert_eq!(std::fs::read(dir.join("image.png")).unwrap(), b"png");
    select(&mut app, &dir.join("Cargo.toml"));
    app.paste();
    assert_eq!(app.name_input.as_ref().unwrap().field.text(), "image-1.png");
    app.close_name_input();

    // 右键菜单：剪贴板中有文件、文本或图片时可以粘贴，剪贴板为空时「粘贴」灰显。
    assert!(app.can_paste_into_tree());
    app.copied_image = None;
    app.copied = Some("文本".into());
    assert!(app.can_paste_into_tree());
    app.copied = None;
    assert!(!app.can_paste_into_tree());

    let _ = std::fs::remove_dir_all(&folder);
}
