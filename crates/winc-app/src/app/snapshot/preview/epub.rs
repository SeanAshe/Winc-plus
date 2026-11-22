//! EPUB 电子书的阅读视图：转换后排版、侧栏大纲列出书的目录并转到各章，阅读位置在切换标签、
//! 关闭后重新打开和重新启动后保留。

use crate::app::{Focus, OutlineEmpty, OutlineKind, OutlineTarget};
use crate::keymap::Action;

use super::*;

/// 三章的书，每章一个标题和 40 段正文；第二章有一节被目录指向。目录为两层。
fn sample_epub() -> Vec<u8> {
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0">
<manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/>
<item id="c2" href="text/c2.xhtml" media-type="application/xhtml+xml"/>
<item id="c3" href="text/c3.xhtml" media-type="application/xhtml+xml"/>
</manifest><spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#;
    let page = |body: String| {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\">\
             <head><title>t</title></head><body>{body}</body></html>"
        )
    };
    let paragraphs = |chapter: usize| -> String {
        (1..=40)
            .map(|k| format!("<p>第 {chapter} 章第 {k} 段：这一段正文用来撑开页面的高度。</p>"))
            .collect()
    };
    let c1 = page(format!("<h1>第一章 出发</h1>{}", paragraphs(1)));
    let c2 = page(format!(
        "<h1>第二章 路上</h1>{}<h2 id=\"rest\">歇脚</h2>{}",
        paragraphs(2),
        paragraphs(2)
    ));
    let c3 = page(format!("<h1>第三章 到达</h1>{}", paragraphs(3)));
    let nav = page(
        "<nav xmlns:epub=\"http://www.idpf.org/2007/ops\" epub:type=\"toc\"><ol>\
         <li><a href=\"text/c1.xhtml\">第一章 出发</a></li>\
         <li><a href=\"text/c2.xhtml\">第二章 路上</a><ol><li><a href=\"text/c2.xhtml#rest\">歇脚</a></li></ol></li>\
         <li><a href=\"text/c3.xhtml\">第三章 到达</a></li></ol></nav>"
            .to_owned(),
    );
    stored_zip(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/text/c1.xhtml", c1.as_bytes()),
        ("OEBPS/text/c2.xhtml", c2.as_bytes()),
        ("OEBPS/text/c3.xhtml", c3.as_bytes()),
    ])
}

/// 打开写有 [`sample_epub`] 的 `path`，展开大纲，等待转换和排版完成。
fn open_book(path: &Path, dark: bool) -> App {
    std::fs::write(path, sample_epub()).unwrap();
    let mut app = app_with(path, dark);
    app.run_action(Action::ShowOutline);
    settle(&mut app);
    app
}

/// 大纲中第 `i` 行的中心。
fn row(app: &App, i: usize) -> (f32, f32) {
    let (rect, _) = *(app.hits.outline.iter())
        .find(|(_, t)| *t == OutlineTarget::Row(i))
        .unwrap();
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

/// 点击大纲中的第 `i` 行，等待滚动结束。
fn click_row(app: &mut App, i: usize) {
    let at = row(app, i);
    click(app, at);
    settle(app);
}

/// EPUB 以只读的阅读视图打开，铺满编辑区；侧栏大纲列出书的目录。点击一条转到该章，读到的
/// 一条高亮，焦点交给编辑区。损坏的文件说明原因。
#[test]
fn epub_books_open_as_a_reading_view_with_their_contents() {
    let dir = sample_folder("epub-reading");
    let path = dir.join("旅行记.epub");
    let mut app = open_book(&path, false);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.unsupported, Some(winc_editor::Unsupported::Epub));
    assert!(doc.readonly);
    assert_eq!(app.preview.markdown.area, app.layout.editor);
    assert!(app.preview.markdown.failed.is_none());

    let outline = &app.panels.outline;
    assert!(matches!(outline.kind, OutlineKind::Anchors(_)));
    let tree: Vec<(&str, usize)> = (outline.items.iter())
        .map(|s| (s.name.as_str(), s.depth))
        .collect();
    assert_eq!(
        tree,
        [
            ("第一章 出发", 0),
            ("第二章 路上", 0),
            ("歇脚", 1),
            ("第三章 到达", 0)
        ]
    );
    assert_eq!(app.outline_current(&app.outline_rows()), Some(0));

    let mut last = app.preview.markdown.scroll();
    for i in 1..4 {
        click_row(&mut app, i);
        let scroll = app.preview.markdown.scroll();
        assert!(scroll > last, "第 {i} 条：{scroll} 应在 {last} 之下");
        last = scroll;
        assert_eq!(app.outline_current(&app.outline_rows()), Some(i));
        assert_eq!(app.focus, Focus::Editor);
    }
    click_row(&mut app, 0);
    assert_eq!(app.preview.markdown.scroll(), 0.0);

    let broken = dir.join("损坏.epub");
    std::fs::write(&broken, b"not an epub").unwrap();
    app.open_path(&broken);
    settle(&mut app);
    assert_eq!(
        app.preview.markdown.failed.as_deref(),
        Some(winc_i18n::t!("preview-epub-invalid"))
    );
    assert_eq!(app.panels.outline.empty, Some(OutlineEmpty::NotText));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 阅读位置在切换标签后保留，关闭后重新打开、重新启动后都回到原处；回到开头后不再记录。
#[test]
fn the_reading_place_is_kept_across_tabs_and_sessions() {
    let dir = sample_folder("epub-places");
    let path = dir.join("旅行记.epub");
    let mut app = open_book(&path, false);
    click_row(&mut app, 2);
    let scroll = app.preview.markdown.scroll();
    assert!(scroll > 0.0);

    app.open_path(&dir.join("a.txt"));
    settle(&mut app);
    app.open_path(&path);
    settle(&mut app);
    assert!((app.preview.markdown.scroll() - scroll).abs() < 0.5);

    app.close_tab(app.editor.active.unwrap());
    settle(&mut app);
    app.open_path(&path);
    settle(&mut app);
    assert!((app.preview.markdown.scroll() - scroll).abs() < 0.5);
    assert_eq!(app.outline_current(&app.outline_rows()), Some(2));

    app.remember_session();
    let mut next = blank_app(false);
    next.state = app.state.clone();
    next.open_path(&dir);
    next.open_path(&path);
    next.run_action(Action::ShowOutline);
    settle(&mut next);
    assert!((next.preview.markdown.scroll() - scroll).abs() < 0.5);

    click_row(&mut next, 0);
    next.remember_session();
    assert!(
        next.state
            .places
            .iter()
            .all(|(_, place)| place.view.is_none())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 转到目录中的一节、切换标签后切回，排版变化（调大字号）后仍停在那一节。
#[test]
fn contents_jumps_and_tab_switches_stay_put_when_the_layout_changes() {
    let dir = sample_folder("epub-hold");
    let path = dir.join("旅行记.epub");
    let mut app = open_book(&path, false);
    let anchor = match &app.panels.outline.kind {
        OutlineKind::Anchors(anchors) => anchors[2].clone().unwrap(),
        _ => panic!("EPUB 的大纲是目录"),
    };
    let top = |app: &App| {
        crate::app::preview::markdown::laid::anchor_box(app, &anchor)
            .unwrap()
            .0
    };

    // 目录：转到「歇脚」，排版变高之后仍对齐到那里。
    click_row(&mut app, 2);
    let before = app.preview.markdown.scroll();
    assert!((before - top(&app)).abs() < 0.5);
    app.run_action(Action::IncreaseFontSize);
    settle(&mut app);
    assert!(top(&app) > before + 1.0, "调大字号后「歇脚」应下移");
    assert!((app.preview.markdown.scroll() - top(&app)).abs() < 0.5);

    // 切到别的标签，排版又变了，切回来仍在「歇脚」。
    app.open_path(&dir.join("a.txt"));
    settle(&mut app);
    app.run_action(Action::IncreaseFontSize);
    app.open_path(&path);
    settle(&mut app);
    assert!((app.preview.markdown.scroll() - top(&app)).abs() < 0.5);
    assert_eq!(app.outline_current(&app.outline_rows()), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：EPUB 的阅读视图，侧栏大纲列出书的目录，当前一节高亮。深浅两套配色。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_epub() {
    for dark in [false, true] {
        let dir = sample_folder("epub-shot");
        let mut app = open_book(&dir.join("旅行记.epub"), dark);
        click_row(&mut app, 2);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("epub-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
