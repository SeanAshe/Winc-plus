//! 预览中滚动、选择、查找以及与源代码相互定位的测试。

use super::*;

#[test]
fn keys_scroll_the_full_preview() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = readme("md-keys", false, PreviewMode::Preview);
    // 只读的文档：预览不能编辑，按键滚动预览。
    app.editor.active_mut().unwrap().readonly = true;
    assert!(!app.markdown_live());
    // 焦点在其他位置时点击预览，键盘焦点交给预览。
    app.focus = Focus::Tree;
    let area = app.preview.markdown.area;
    super::click(&mut app, (area.x + area.w / 2.0, area.y + area.h / 2.0));
    assert_eq!(app.focus, Focus::Editor);
    let max = app.preview.markdown.height - app.preview.markdown.area.h;
    assert!(max > 0.0);
    let key = |app: &mut App, key: Key, cmd: bool| {
        assert!(app.markdown_key(&key, cmd));
        app.preview.markdown.scroll()
    };
    let down = key(&mut app, Key::Named(NamedKey::ArrowDown), false);
    assert!(down > 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowUp), false), 0.0);
    // 翻页的距离大于一次 ↓ 的距离、小于预览高度，保留上一屏的部分内容。
    let page = key(&mut app, Key::Named(NamedKey::PageDown), false);
    assert!(page > down && page < app.preview.markdown.area.h);
    assert_eq!(key(&mut app, Key::Named(NamedKey::PageUp), false), 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::Space), false), page);
    assert_eq!(key(&mut app, Key::Named(NamedKey::End), false), max);
    assert_eq!(key(&mut app, Key::Named(NamedKey::Home), false), 0.0);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowDown), true), max);
    assert_eq!(key(&mut app, Key::Named(NamedKey::ArrowUp), true), 0.0);
    // 其他键交给编辑区。
    assert!(!app.markdown_key(&Key::Character("x".into()), false));

    // Esc 清除选区；没有选区时不处理。
    app.markdown_select_all();
    assert!(app.markdown_key(&Key::Named(NamedKey::Escape), false));
    assert!(app.preview.markdown.selection.is_none());
    assert!(!app.markdown_key(&Key::Named(NamedKey::Escape), false));

    // 分栏时焦点在左侧文字上：方向键仍然移动光标，不滚动预览。
    app.editor.active_mut().unwrap().readonly = false;
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert!(!app.markdown_key(&Key::Named(NamedKey::PageDown), false));
    // 点击右侧预览：键盘滚动预览（左侧同步滚动）；按键输入时焦点回到左侧。
    let area = app.preview.markdown.area;
    super::click(&mut app, (area.x + area.w / 2.0, area.y + area.h / 2.0));
    let before = app.preview.markdown.scroll();
    assert!(app.markdown_key(&Key::Named(NamedKey::PageDown), false));
    assert!(app.preview.markdown.scroll() > before);
    assert!(!app.markdown_key(&Key::Character("x".into()), false));
    assert!(!app.markdown_focused());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 预览的滚动条可以拖动：只看预览和分栏时均可，不会选中文字；分栏时左侧随之滚动。
#[test]
fn the_preview_scrollbar_can_be_dragged() {
    use crate::layout::{Scrollbar, scrollbar_thumb};

    for (mode, name) in [
        (PreviewMode::Preview, "md-drag-full"),
        (PreviewMode::Split, "md-drag-split"),
    ] {
        let (mut app, dir) = readme(name, false, mode);
        let area = app.preview.markdown.area;
        let height = app.preview.markdown.height;
        assert!(height > area.h, "{height} {}", area.h);
        let thumb = scrollbar_thumb(area, 0.0, height).unwrap();
        let (x, y) = (thumb.x + thumb.w / 2.0, thumb.y + thumb.h / 2.0);

        app.on_mouse_move(x, y);
        assert_eq!(app.pointer.hover, Hover::Scrollbar(Scrollbar::Markdown));
        app.on_mouse_down(MouseButton::Left);
        assert!(matches!(
            app.pointer.drag,
            crate::app::Drag::Scrollbar { .. }
        ));

        // 拖到底部：滚动到末尾，滑块位于底端。
        app.on_mouse_move(x, area.bottom() + 100.0);
        let end = height - area.h;
        assert!((app.preview.markdown.scroll() - end).abs() < 1.0, "{name}");
        paint(&mut app);
        assert!(app.preview.markdown.selection.is_none());
        if mode == PreviewMode::Split {
            assert!(app.editor.active().unwrap().scroll_line > 0.0);
        }

        // 拖回顶部。
        app.on_mouse_move(x, area.y - 100.0);
        assert!(app.preview.markdown.scroll() < 1.0, "{name}");
        app.on_mouse_up(MouseButton::Left);
        assert!(matches!(app.pointer.drag, crate::app::Drag::None));

        // 点击轨道：滑块移到点击处。
        app.on_mouse_move(x, area.bottom() - 2.0);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        assert!(app.preview.markdown.scroll() > 0.0, "{name}");

        // 滚动条左侧仍然是选择文字。
        app.on_mouse_move(area.x + 50.0, area.y + 50.0);
        app.on_mouse_down(MouseButton::Left);
        assert!(matches!(app.pointer.drag, crate::app::Drag::MarkdownText));
        app.on_mouse_up(MouseButton::Left);

        // 回到代码模式：同一个位置是编辑区的滚动条。
        app.set_preview_mode(PreviewMode::Source);
        settle(&mut app);
        let track = app.scroll_track();
        app.on_mouse_move(track.right() - 2.0, track.y + 4.0);
        assert_ne!(app.pointer.hover, Hover::Scrollbar(Scrollbar::Markdown));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_modes_keeps_the_place() {
    let text: String = (0..60)
        .map(|i| format!("## 第 {i} 节\n\n第 {i} 节的正文。\n\n"))
        .collect();
    let (mut app, dir) = live_doc("md-switch", &text);
    // 预览滚动到第 30 节：切换为源代码后文字顶部是第 30 节的标题（第 120 行）。
    let y = crate::app::preview::markdown::laid::anchor_top(&app, "第-30-节").unwrap();
    app.scroll_markdown_to(y);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    let line = app.editor.active().unwrap().scroll_line;
    assert!((line - 120.0).abs() < 2.0, "{line}");
    // 文字滚动到第 10 节，切换回只看预览：预览顶部是第 10 节，光标所在的块不显示原文。
    app.editor.active_mut().unwrap().scroll_line = 40.0;
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let y = crate::app::preview::markdown::laid::anchor_top(&app, "第-10-节").unwrap();
    // 差值来自预览顶部的留白，分栏同步滚动的映射同样包含这段留白。
    let pad = app.preview.markdown.scroll() - y;
    assert!((0.0..=40.0).contains(&pad), "{pad}");
    assert!(
        !crate::app::preview::markdown::laid::full_text(&app)
            .unwrap()
            .contains("## ")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_text_can_be_selected_and_copied() {
    use super::editor::navigation::press;
    use crate::keymap::KeyCode::Char;

    // 分栏时点击了预览：选择的是排版后的文字。可编辑的预览中选的是原文，见
    // `the_full_preview_edits_the_document`。
    let (mut app, dir) = readme("md-select", false, PreviewMode::Split);
    let area = app.preview.markdown.area;
    super::click(&mut app, (area.x + 5.0, area.bottom() - 5.0));
    assert!(app.markdown_focused());
    // ⌘A ⌘C：复制整份文字，段落之间换行，中文软换行处不插入空格，代码保持原样。
    assert!(press(&mut app, &["cmd"], Char('a')));
    assert!(press(&mut app, &["cmd"], Char('c')));
    let all = app.clipboard.copied.clone().unwrap();
    assert!(
        all.starts_with("winc 编辑器\n一个用 Rust 写的轻量编辑器，启动快"),
        "{all}"
    );
    assert!(all.contains("混排时按字换行，链接比如 GitHub 和 安装。"));
    assert!(all.contains("fn main() {\n    // 打印一行\n"));
    assert!(all.contains("下载发行版\n解压到 /Applications\n"));

    // 拖动选择一段（从「Rust」的 R 之后开始）；双击选中一个词。
    let at = all[..all.find("Rust").unwrap()].chars().count() + 1;
    let (x, y) = point_of(&app, at);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    // 拖到「混排」之后：按字的位置查找，不按行数计算（各系统的字体宽度不同，折行的位置也不同）。
    let end = all[..all.find("混排").unwrap()].chars().count() + 2;
    let (end_x, end_y) = point_of(&app, end);
    app.on_mouse_move(end_x, end_y);
    app.on_mouse_up(MouseButton::Left);
    let dragged = app.markdown_selected_text().unwrap();
    assert!(
        dragged.starts_with("ust 写的") && dragged.contains("混排"),
        "{dragged}"
    );
    super::click(&mut app, (x, y));
    assert_eq!(app.markdown_selected_text().as_deref(), Some("Rust"));

    // 点击预览外部（文件树）：清除选区。
    let tree = app.layout.tree;
    super::click(&mut app, (tree.x + 10.0, tree.y + 10.0));
    assert!(app.preview.markdown.selection.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_and_source_point_at_each_other() {
    use super::editor::navigation::press;
    use crate::keymap::KeyCode::Char;

    let (mut app, dir) = readme("md-locate", false, PreviewMode::Split);
    assert!(press(&mut app, &["cmd"], Char('a')));
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    // 样式标记、嵌套列表、代码块、表格、引用中的软换行和脚注都能对应到原文；点击链接会打开
    // 链接，不在此列。
    for target in [
        "启动快",
        "旧的写法",
        "第一次打开",
        "打印一行",
        "隔行铺底色",
        "可以有多行",
        "这是脚注",
    ] {
        let at = shown[..shown.find(target).unwrap()].chars().count();
        // 先滚动到可见位置。
        app.select_in_markdown(at, at + 1);
        paint(&mut app);
        let (x, y) = point_of(&app, at);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        paint(&mut app);
        let expected = SAMPLE[..SAMPLE.find(target).unwrap()].chars().count();
        let doc = app.editor.active().unwrap();
        assert_eq!(doc.selection, Selection::point(expected), "{target}");

        // 反向：光标所在的字在预览上落在点中的那一行。
        let source = app.markdown_caret_source().unwrap();
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let (_, top, _) =
            crate::app::preview::markdown::laid::point_of(laid, source).expect(target);
        let line = crate::app::preview::markdown::laid::offset_top(&app, at).expect(target);
        assert_eq!(top, line, "{target}");
    }

    // 只看预览时右键「在源代码中显示」：改为分栏，光标放到点中的字上。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let at = shown[..shown.find("打印一行").unwrap()].chars().count();
    app.select_in_markdown(at, at + 1);
    paint(&mut app);
    let (x, y) = point_of(&app, at);
    assert!(app.markdown_context_target(x, y));
    app.reveal_markdown_source();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    let expected = SAMPLE[..SAMPLE.find("打印一行").unwrap()].chars().count();
    assert_eq!(doc.selection, Selection::point(expected));
    assert!(!app.preview.markdown.focused);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_f_searches_the_rendered_preview() {
    // 分栏时点击了预览：查找排版后的文字。
    let (mut app, dir) = readme("md-find", false, PreviewMode::Split);
    let area = app.preview.markdown.area;
    super::click(&mut app, (area.x + 5.0, area.bottom() - 5.0));
    assert!(app.markdown_focused());
    app.open_search(false);
    assert!(app.bars.find.open);
    app.bars.find.input.set_text("预览");
    app.bars.search.query = "预览".into();
    app.search_changed();
    paint(&mut app);
    // 「预览」在渲染后的文字里出现两次（表格一格、提示块一句），源码中的写法不计入。
    assert_eq!(app.bars.search.matches().len(), 2);
    assert_eq!(app.markdown_selected_text().as_deref(), Some("预览"));
    let first = app.preview.markdown.selection;
    app.find_next(false);
    assert_ne!(app.preview.markdown.selection, first);
    // 第二处在表格里，预览滚动到它的位置。
    assert!(app.preview.markdown.scroll() > 0.0);
    app.find_next(false);
    assert_eq!(app.preview.markdown.selection, first);
    // 预览没有「行」的概念：「只显示匹配的行」不生效。
    app.press_search_button(crate::layout::SearchButton::Filter);
    assert!(!app.bars.find.filter_lines);
    // 排版后的文字不可修改：不展开替换。
    app.close_search();
    app.open_search(true);
    assert!(!app.bars.find.replace_open);
    app.close_search();

    // 点击左侧文字后查找文字。
    let text = app.layout.text;
    super::click(&mut app, (text.x + 40.0, text.y + 5.0));
    assert!(!app.markdown_focused());
    app.open_search(false);
    app.search_changed();
    assert_eq!(
        app.bars.search.document(),
        app.editor.active().map(|d| d.id)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Home、End 按预览中折行后的显示行移动：一段原文折成多行时，End 停在本行的右端，光标不换行；
/// 再按 ↓ 到下一行的右端，Home 回到该行的开头。
#[test]
fn home_and_end_follow_the_wrapped_rows() {
    use winit::keyboard::{Key, NamedKey};
    let para = "折行的段落".repeat(40);
    let (mut app, dir) = live_doc("md-row-edges", &format!("前文\n\n{para}\n\n后文\n"));
    let start = doc_text(&app).find(&para).unwrap();
    put_caret(&mut app, "折行的段落折", false);
    let caret = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let (x, y, _) = laid.caret_point(app.live_caret_source().unwrap()).unwrap();
        (x, y)
    };
    let first = caret(&app);
    assert!(app.markdown_key(&Key::Named(NamedKey::End), false));
    settle(&mut app);
    let end = caret(&app);
    let at = app.live_caret_source().unwrap();
    assert!(at > start && at < start + para.len(), "{at}");
    assert!(
        (end.1 - first.1).abs() < 0.5 && end.0 > first.0 + 100.0,
        "{end:?}"
    );
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    settle(&mut app);
    let below = caret(&app);
    assert!(
        below.1 > end.1 + 10.0 && (below.0 - end.0).abs() < 20.0,
        "{below:?}"
    );
    assert!(app.markdown_key(&Key::Named(NamedKey::Home), false));
    settle(&mut app);
    let home = caret(&app);
    assert!((home.1 - below.1).abs() < 0.5 && home.0 < 1.0, "{home:?}");
    assert_eq!(app.live_caret_source().unwrap(), at);
    let _ = std::fs::remove_dir_all(&dir);
}
