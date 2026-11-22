//! 源代码中链接的测试：识别、打开本地文件和维基链接、跳到标题、悬停和右键菜单。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use winc_preview::markdown::syntax::WIKI;

/// 把光标放在当前文档中 `text` 第一次出现处之后 1 个字符的位置。
fn caret_in(app: &mut App, text: &str) {
    let all = app.editor.active().unwrap();
    let all = all.text_range(0, usize::MAX).unwrap();
    let at = all.find(text).unwrap();
    let pos = all[..at].chars().count() + 1;
    app.select(pos, pos);
}

fn active_name(app: &App) -> String {
    app.editor.active().unwrap().name().into_owned()
}

/// 建一个笔记目录并打开 `index.md`：同名的 `note.md` 一个在 `near` 中，一个在 `far/deep` 中。
fn notes(name: &str) -> (App, PathBuf) {
    let dir = winc_config::files::canonicalize(sample_folder(name)).unwrap();
    for folder in ["near", "far/deep"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
        std::fs::write(dir.join(folder).join("note.md"), "# 笔记\n").unwrap();
    }
    std::fs::write(dir.join("near/data.csv"), "a,b\n").unwrap();
    std::fs::write(dir.join("near/v1.2 notes.md"), "# 说明\n").unwrap();
    let index = concat!(
        "# 开头\n\n见 https://winc.si/zh-cn/ 和 [数据](data.csv)。\n\n",
        "[[note]] [[data.csv|表]] [[missing]] [到结尾](#结尾)\n\n",
        "```\n# 不是标题\n```\n\n## 结尾\n",
    );
    std::fs::write(dir.join("near/index.md"), index).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("near/index.md"));
    (app, dir)
}

#[test]
fn links_in_markdown_source_open_from_the_caret() {
    let (mut app, dir) = notes("source-links");
    let url_at = |app: &mut App, text: &str| {
        caret_in(app, text);
        app.link_at_caret().map(|link| link.url)
    };
    // 网址只识别，不在测试中打开浏览器。
    let url = "https://winc.si/zh-cn/";
    assert_eq!(url_at(&mut app, "https://").as_deref(), Some(url));
    assert_eq!(
        url_at(&mut app, "[[data.csv"),
        Some(format!("{WIKI}data.csv"))
    );
    assert_eq!(url_at(&mut app, "见 "), None);

    // 相对路径的文件在编辑区中打开。
    caret_in(&mut app, "[数据]");
    assert!(app.open_link_at_caret());
    assert_eq!(active_name(&app), "data.csv");

    // 维基链接按文件名查找，同名时取离当前文档近的；没有扩展名时按 `.md`。
    app.open_path(&dir.join("near/index.md"));
    caret_in(&mut app, "[[note");
    assert!(app.open_link_at_caret());
    let opened = app.editor.active().unwrap().path.clone();
    assert_eq!(opened, Some(dir.join("near/note.md")));

    // 名字中有 `.` 而没有扩展名：按补上 `.md` 的名字找到。
    let dotted = app.wiki_target("v1.2 notes");
    assert_eq!(dotted, Some(dir.join("near/v1.2 notes.md")));

    // 找不到的文件只提示，不新建。
    app.open_path(&dir.join("near/index.md"));
    caret_in(&mut app, "[[missing");
    assert!(app.open_link_at_caret());
    assert_eq!(active_name(&app), "index.md");
    assert!(app.status.message.is_some());
    assert!(!dir.join("near/missing.md").exists());

    // 预览中同样按名字检查：找不到的标为失效。
    use crate::app::preview::markdown::links::Broken;
    let broken = app.check_markdown_link(&format!("{WIKI}missing"));
    assert_eq!(broken, Some(Broken::File));
    assert_eq!(app.check_markdown_link(&format!("{WIKI}note")), None);

    // `#标题` 跳到源代码中的标题行，代码块中的 `#` 除外。
    caret_in(&mut app, "[到结尾]");
    assert!(app.open_link_at_caret());
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (10, 0));

    // 下一行是 `=`、`-` 的标题同样计数。
    let setext = dir.join("near/setext.md");
    std::fs::write(
        &setext,
        "标题\n===\n\n正文\n\n第二节\n---\n\n[跳](#第二节)\n",
    )
    .unwrap();
    app.open_path(&setext);
    caret_in(&mut app, "[跳]");
    assert!(app.open_link_at_caret());
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().line_of(doc.selection.head), 5);
    app.open_path(&dir.join("near/index.md"));

    // 不在链接上：命令提示，`open_link_at_caret` 返回假。
    caret_in(&mut app, "见 ");
    assert!(!app.open_link_at_caret());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_editor_menu_offers_the_link_under_the_caret() {
    let (mut app, dir) = notes("source-link-menu");
    paint(&mut app);
    let first = |app: &mut App, text: &str| {
        caret_in(app, text);
        let id = app.editor.active().unwrap().id;
        app.open_context_menu(MenuTarget::Editor(id), (0.0, 0.0));
        let first = menu_commands(app)[0];
        app.overlays.context_menu = None;
        first
    };
    assert_eq!(first(&mut app, "[[note"), (MenuCommand::OpenLink, true));
    assert_eq!(first(&mut app, "见 "), (MenuCommand::GoToDefinition, true));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 其他语言中只认网址；`[文字](地址)` 和 `[[名字]]` 是代码。
#[test]
fn only_urls_are_links_outside_markdown() {
    let dir = sample_folder("source-links-code");
    let code = "// https://winc.si\nlet a = b[i](x); let c = [[1]];\n";
    std::fs::write(dir.join("a.rs"), code).unwrap();
    let mut app = app_with(&dir.join("a.rs"), false);
    let link = |app: &mut App, text: &str| {
        caret_in(app, text);
        app.link_at_caret().map(|link| link.url)
    };
    assert_eq!(link(&mut app, "https").as_deref(), Some("https://winc.si"));
    assert_eq!(link(&mut app, "[i]"), None);
    assert_eq!(link(&mut app, "[[1"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 按住修饰键时鼠标下的链接记下来（绘制下划线），单击打开；不按修饰键时单击只放置光标。
#[test]
fn clicking_with_the_modifier_opens_the_link_under_the_mouse() {
    use winit::keyboard::ModifiersState;
    let (mut app, dir) = notes("source-link-click");
    paint(&mut app);
    caret_in(&mut app, "[[note");
    let pos = app.editor.active().unwrap().selection.head;
    // 文字区域中落在 `pos` 上的一点。
    let y = app.layout.text.y + 4.5 * app.fonts.line_height;
    let x = (0..600)
        .map(|i| app.layout.text.x + i as f32)
        .find(|&x| app.pos_at(x, y) == Some(pos))
        .expect("这一行上有这个位置");
    app.select(0, 0);

    app.on_mouse_move(x + 1.0, y);
    assert_eq!(app.pointer.link, None);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(active_name(&app), "index.md");

    app.pointer.modifiers = if cfg!(target_os = "macos") {
        ModifiersState::SUPER
    } else {
        ModifiersState::CONTROL
    };
    app.update_text_link();
    let link = app.pointer.link.clone().expect("鼠标在链接上");
    assert_eq!(link.url, format!("{WIKI}note"));
    paint(&mut app);
    // 与上一次单击隔开，避免记为双击。
    app.pointer.last_click = None;
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let opened = app.editor.active().unwrap().path.clone();
    assert_eq!(opened, Some(dir.join("near/note.md")));
    let _ = std::fs::remove_dir_all(&dir);
}
