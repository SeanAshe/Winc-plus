//! Markdown 写作命令的测试：源代码和可编辑的预览中设标题、切换列表和引用、插入块和链接，以及
//! `markdown.shortcuts` 关掉后源代码不再使用这些键。

use super::*;
use crate::keymap::{Action, Context};

/// 打开内容为 `text` 的 Markdown 文件，只看源代码。
fn source_doc(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("NOTES.md");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    (app, dir)
}

/// 返回在光标处插入 `|`（有选区时在两端插入 `[`、`]`）后的文档文字。
fn marked(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    let rope = doc.text().rope().unwrap();
    let (a, b) = (
        rope.char_to_byte(doc.selection.from()),
        rope.char_to_byte(doc.selection.to()),
    );
    let mut text = rope.to_string();
    if a == b {
        text.insert(a, '|');
    } else {
        text.insert(b, ']');
        text.insert(a, '[');
    }
    text
}

/// 选中第一处 `needle`。
fn select(app: &mut App, needle: &str) {
    let at = doc_text(app).find(needle).unwrap();
    let doc = app.editor.active_mut().unwrap();
    let rope = doc.text().rope().unwrap();
    let (a, b) = (rope.byte_to_char(at), rope.byte_to_char(at + needle.len()));
    doc.selection = winc_core::Selection::new(a, b);
}

/// 在可编辑的预览中选中第一处 `needle`，显示格式工具栏并重绘。
fn select_with_toolbar(app: &mut App, needle: &str) {
    select(app, needle);
    app.start_live_editing();
    app.offer_selection_toolbar();
    paint(app);
}

#[test]
fn writing_commands_edit_the_markdown_source() {
    let (mut app, dir) = source_doc("md-writing", "标题\n\n一\n二\n\n正文\n");
    assert_eq!(app.key_context(), Context::Markdown);
    put_caret(&mut app, "题", false);
    assert!(app.run_action(Action::MarkdownHeading2));
    assert_eq!(marked(&app), "## 标|题\n\n一\n二\n\n正文\n");
    // 同一级再按一次改回正文，一次撤销回到原样。
    assert!(app.run_action(Action::MarkdownHeading2));
    assert_eq!(marked(&app), "标|题\n\n一\n二\n\n正文\n");
    app.run(winc_editor::Command::Undo);
    app.run(winc_editor::Command::Undo);
    assert_eq!(doc_text(&app), "标题\n\n一\n二\n\n正文\n");

    // 选中两行：改成有序列表，再改成引用。
    select(&mut app, "一\n二");
    assert!(app.run_action(Action::MarkdownOrderedList));
    assert_eq!(marked(&app), "标题\n\n[1. 一\n2. 二]\n\n正文\n");
    assert!(app.run_action(Action::MarkdownQuote));
    assert_eq!(marked(&app), "标题\n\n[> 1. 一\n> 2. 二]\n\n正文\n");

    // 有字的一行上插入代码块和表格：插在下面，光标在可以继续写的地方。
    put_caret(&mut app, "正文", true);
    assert!(app.run_action(Action::MarkdownCodeBlock));
    assert!(marked(&app).ends_with("正文\n\n```|\n\n```\n"));
    app.run(winc_editor::Command::Undo);
    put_caret(&mut app, "正文", true);
    assert!(app.run_action(Action::MarkdownTable));
    assert!(marked(&app).ends_with("正文\n\n| | |  |\n| --- | --- |\n|  |  |\n"));

    // 选中文字插入链接：剪贴板里是网址时作为地址。
    app.run(winc_editor::Command::Undo);
    app.clipboard.copied = Some("https://winc.si".into());
    select(&mut app, "正文");
    assert!(app.run_action(Action::MarkdownLink));
    assert!(marked(&app).ends_with("[正文](https://winc.si)|\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_setting_keeps_markdown_keys_to_the_editable_preview() {
    let (mut app, dir) = source_doc("md-writing-off", "正文\n");
    app.config.markdown_shortcuts = Some(false);
    assert_eq!(app.key_context(), Context::Editor);
    assert!(!app.action_available(Action::MarkdownHeading1));
    // 可编辑的预览中同样使用。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    assert_eq!(app.key_context(), Context::Markdown);
    put_caret(&mut app, "正文", true);
    assert!(app.run_action(Action::MarkdownHeading1));
    settle(&mut app);
    assert_eq!(doc_text(&app), "# 正文\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 状态栏上 Markdown 文档的字数：整篇，有选区时另有选中部分；其他文件不显示。
#[test]
fn the_status_bar_counts_words_in_markdown() {
    let (mut app, dir) = source_doc(
        "md-words",
        "# 标题\n\n中文和 English 混排，见 [文档](https://x)。\n",
    );
    // 测试中没有事件循环，后台的计数在当前线程上数完。
    app.track_words();
    assert_eq!(app.word_count(), Some((11, 0)));
    select(&mut app, "中文和");
    app.track_words();
    assert_eq!(app.word_count(), Some((11, 3)));
    shoot(&mut app, "md-words");
    // 没有选区时字数与光标位置无关。
    let doc = app.editor.active().unwrap();
    let key = (doc.id, doc.revision, 0, 0);
    for at in [0, 3] {
        app.editor.active_mut().unwrap().selection = winc_core::Selection::point(at);
        app.track_words();
        assert_eq!(app.status.words.done, Some((key, 11, 0)));
    }
    app.open_path(&dir.join("short.rs"));
    app.editor.load_active();
    app.track_words();
    assert_eq!(app.word_count(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拖动把手或使用右键菜单调整图片大小时，只改写该图片的原文，光标位置和显示方式不变。
#[test]
fn images_resize_by_dragging_the_handle_or_from_the_menu() {
    use winit::event::MouseButton;
    let dir = sample_folder("md-image-resize");
    let png = winc_editor::paste::encode_png(400, 200, &[90; 400 * 200 * 4]).unwrap();
    std::fs::write(dir.join("wide.png"), png).unwrap();
    let path = dir.join("LIVE.md");
    let text = "<p align=\"center\">\n  <img src=\"wide.png\" alt=\"logo\" width=\"128\" />\n</p>\n\n\
        前文\n\n<p align=\"center\">\n  <a href=\"https://winc.si\"><img src=\"wide.png\" \
        alt=\"下载\" width=\"300\"></a>\n</p>\n\n![图](wide.png)\n\n后文\n";
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let [_, button, _] = app.hits.markdown_images[..] else {
        panic!("画出了三张可调整的图片");
    };
    assert_eq!(button.rect.w, 300.0);

    // 居中的图片两侧同时改变：把手左移 80，宽度减少 160。
    let handle = button.handle().unwrap();
    let (x, y) = (handle.x + handle.w / 2.0, handle.y + handle.h / 2.0);
    app.on_mouse_move(x, y);
    paint(&mut app);
    shoot(&mut app, "md-image-handle");
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 80.0, y);
    paint(&mut app);
    shoot(&mut app, "md-image-resizing");
    app.on_mouse_up(MouseButton::Left);
    let changed = "<img src=\"wide.png\" alt=\"下载\" width=\"140\">";
    assert!(doc_text(&app).contains(changed), "{}", doc_text(&app));
    assert!(doc_text(&app).contains("alt=\"logo\" width=\"128\""));
    settle(&mut app);
    assert!(!app.live_caret_shown());
    assert_eq!(app.hits.markdown_images[1].rect.w, 140.0);

    // 右键菜单写入百分比宽度，「原始大小」恢复 Markdown 写法，每次占一步撤销。
    let image = app.hits.markdown_images[2].rect;
    let center = (image.x + image.w / 2.0, image.y + image.h / 2.0);
    right_click(&mut app, center);
    use crate::app::input::context_menu::MenuCommand;
    assert!(menu_commands(&app).contains(&(MenuCommand::ImageWidth(50), true)));
    app.run_menu_command(MenuCommand::ImageWidth(50));
    assert!(doc_text(&app).contains("<img src=\"wide.png\" alt=\"图\" width=\"50%\">"));
    settle(&mut app);
    right_click(&mut app, center);
    app.run_menu_command(MenuCommand::ImageWidth(0));
    assert!(doc_text(&app).contains("\n![图](wide.png)\n"));
    app.run(winc_editor::Command::Undo);
    assert!(doc_text(&app).contains("width=\"50%\""));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 粘贴的图片按设置的目录和名字保存，链接写成相对文档的路径。
#[test]
fn pasted_images_follow_the_folder_and_name_settings() {
    let (mut app, dir) = source_doc("md-paste-image", "");
    app.paste_image(b"one");
    assert_eq!(marked(&app), "![[image]](image.png)");
    assert!(dir.join("image.png").is_file());

    app.config.markdown_image_folder = Some("${filename}.assets".into());
    app.config.markdown_image_name = Some("${filename}-shot".into());
    put_caret(&mut app, "png)", true);
    app.run(winc_editor::Command::Insert("\n".into()));
    app.paste_image(b"two");
    let pasted = "![image](image.png)\n![[NOTES-shot]](NOTES.assets/NOTES-shot.png)";
    assert_eq!(marked(&app), pasted);
    let saved = dir.join("NOTES.assets/NOTES-shot.png");
    assert_eq!(std::fs::read(saved).unwrap(), b"two");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 在 `](` 之后输入时列出目录里的文件：按输入筛选，图片只列图片；回车写入相对路径并补上 `)`，
/// Esc 关掉后同一个地址里不再列出。
#[test]
fn link_paths_are_suggested_while_typing() {
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = source_doc("md-suggest", "");
    std::fs::create_dir_all(dir.join("docs/img")).unwrap();
    std::fs::write(dir.join("docs/guide.md"), "# 指南\n").unwrap();
    std::fs::write(dir.join("docs/img/shot.png"), b"png").unwrap();
    // 文件是测试里新建的：先让文件表重新建起来。
    app.overlays.picker.stale = true;
    app.track_suggest();
    let typed = |app: &mut App, text: &str| {
        app.run(winc_editor::Command::Insert(text.into()));
        app.track_suggest();
    };
    use crate::app::preview::markdown::suggest::Hit;
    let listed = |app: &App| -> Vec<String> {
        let hits = &app.preview.markdown.suggest.hits;
        (hits.iter())
            .map(|hit| match hit {
                Hit::File(path, _) => path.clone(),
                Hit::Heading(_, _, slug) => format!("#{slug}"),
                Hit::Command(command) => format!("/{}", command.label()),
            })
            .collect()
    };

    typed(&mut app, "见 [指南](gui");
    assert_eq!(listed(&app), ["docs/guide.md"]);
    paint(&mut app);
    shoot(&mut app, "md-suggest");
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert_eq!(marked(&app), "见 [指南](docs/guide.md)|");

    // 图片只列图片。列表绘制之前按键交给编辑区。
    typed(&mut app, "\n![](");
    assert_eq!(listed(&app), ["docs/img/shot.png"]);
    assert!(!app.suggest_key(&Key::Named(NamedKey::Escape)));
    paint(&mut app);
    // 带修饰键的按键不接管：⇧Tab、⇧↩ 交给编辑区。
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(!app.suggest_key(&Key::Named(NamedKey::Tab)));
    app.pointer.modifiers = Default::default();
    // Esc 关掉以后，在这个地址里继续输入也不再列出。
    assert!(app.suggest_key(&Key::Named(NamedKey::Escape)));
    typed(&mut app, "d");
    assert!(app.preview.markdown.suggest.at.is_none());
    assert!(!app.suggest_key(&Key::Named(NamedKey::Enter)));
    // 只是把光标移进已有的地址里不列出。
    typed(&mut app, ")");
    put_caret(&mut app, "guide", true);
    app.track_suggest();
    assert!(app.preview.markdown.suggest.at.is_none());

    // 地址以 `#` 开头时列出本文的标题，写入锚点。
    typed(&mut app, "\n\n## 安装步骤\n\n见 [安装](#安装");
    assert_eq!(listed(&app), ["#安装步骤"]);
    paint(&mut app);
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert!(
        marked(&app).contains("见 [安装](#安装步骤)|"),
        "{}",
        marked(&app)
    );

    // 输入改变后选中回到第一项。
    typed(&mut app, "\n[](");
    assert!(listed(&app).len() > 1);
    paint(&mut app);
    assert!(app.suggest_key(&Key::Named(NamedKey::ArrowDown)));
    assert_eq!(app.preview.markdown.suggest.selected, 1);
    typed(&mut app, "d");
    assert_eq!(app.preview.markdown.suggest.selected, 0);

    // 换到别的标签再回来：没有输入，不再列出。
    let notes = app.editor.active().unwrap().path.clone().unwrap();
    app.open_path(&dir.join("short.rs"));
    app.editor.load_active();
    app.track_suggest();
    app.open_path(&notes);
    app.track_suggest();
    assert!(app.preview.markdown.suggest.at.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 悬停在预览的链接上提示地址，在脚注的编号上提示脚注的内容。
#[test]
fn hovering_links_and_footnotes_shows_where_they_go() {
    let (mut app, dir) = live_doc(
        "md-hover",
        "见 [文档](https://winc.si/docs) 和脚注[^a]。\n\n[^a]: 脚注里的说明。\n",
    );
    paint(&mut app);
    let link = |app: &App, url: &str| {
        let (rect, _) = (app.hits.markdown_links.iter())
            .find(|(_, u)| u == url)
            .unwrap()
            .clone();
        rect
    };
    for (url, name) in [
        ("#fn-1", "md-hover-note"),
        ("https://winc.si/docs", "md-hover-link"),
    ] {
        let rect = link(&app, url);
        app.on_mouse_move(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        assert!(app.pointer.hover.has_tooltip());
        app.overlays.tooltip.since = Some(
            std::time::Instant::now()
                - crate::theme::TOOLTIP_DELAY
                - crate::app::animation::TOOLTIP_FADE,
        );
        shoot(&mut app, name);
    }
    assert_eq!(
        app.preview
            .markdown
            .parsed
            .as_ref()
            .unwrap()
            .footnote_text("1")
            .as_deref(),
        Some("脚注里的说明。")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 专注模式淡化光标所在段落以外的文字，打字机模式让光标所在的行停在中间；F8、F9 切换。
#[test]
fn focus_and_typewriter_modes_follow_the_caret() {
    use crate::app::preview::markdown::paragraph_lines;
    let text: String = (0..80)
        .map(|i| format!("第 {i} 段的第一行\n第 {i} 段的第二行\n\n"))
        .collect();
    let (mut app, dir) = source_doc("md-focus", &text);
    let rope = app.editor.active().unwrap().text().rope().unwrap().clone();
    let at = rope.line_to_char(4);
    assert_eq!(paragraph_lines(&rope, at), (3, 4));
    assert_eq!(paragraph_lines(&rope, rope.line_to_char(2)), (2, 2));

    // F8、F9 在 Markdown 文件里切换两种模式。
    assert!(app.run_action(Action::MarkdownFocusMode));
    assert!(app.run_action(Action::MarkdownTypewriterMode));
    assert!(app.markdown_focus() && app.markdown_typewriter());

    // 打字机模式：点击放光标时不滚动，免得拖成选区。
    app.scroll.keyed = false;
    put_caret(&mut app, "第 40 段的第一行", true);
    app.ensure_caret_visible();
    app.scroll.glide = None;
    let clicked = app.editor.active().unwrap().scroll_line;
    // 用键盘移动光标后，这一行在编辑区中间。
    app.scroll.keyed = true;
    app.ensure_caret_visible();
    app.scroll.glide = None;
    let doc = app.editor.active().unwrap();
    let row = doc.row_at(doc.selection.head) as f64;
    let visible = app.visible_lines() as f64;
    assert!((doc.scroll_line - (row + 0.5 - visible / 2.0)).abs() < 1.0);
    assert!(clicked < doc.scroll_line);
    shoot(&mut app, "md-focus-source");

    // 可编辑的预览里同样淡化其余的块。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    app.start_live_editing();
    settle(&mut app);
    shoot(&mut app, "md-focus-preview");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：打字机模式下在文末编辑时，光标所在行居中的滚动位置超过普通的上限；自动保存后文件监视
/// 报告变化，限制滚动范围时滚回上限，编辑区上下跳动。
#[test]
fn typewriter_scroll_past_the_end_survives_file_changes() {
    let text: String = (0..80).map(|i| format!("第 {i} 段\n\n")).collect();
    let (mut app, dir) = source_doc("md-typewriter-end", &text);
    assert!(app.run_action(Action::MarkdownTypewriterMode));
    app.scroll.keyed = true;
    put_caret(&mut app, "第 79 段", true);
    app.ensure_caret_visible();
    app.scroll.glide = None;
    let scroll = app.editor.active().unwrap().scroll_line;
    assert!(scroll > app.max_scroll_line(), "文末的行居中");

    let path = dir.join("NOTES.md");
    app.apply_changes(winc_editor::Changes {
        paths: vec![path],
        rescan: false,
        git: false,
    });
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    // 滚轮向上滚一行同样不先滚回上限。
    let id = app.editor.active().unwrap().id;
    app.scroll_editor_by_pixels(id, 0.0, app.fonts.line_height);
    let wheeled = app.editor.active().unwrap().scroll_line;
    assert!((wheeled - (scroll - 1.0)).abs() < 1e-6, "{wheeled}");

    // 关闭打字机模式后按普通的上限限制。
    assert!(app.run_action(Action::MarkdownTypewriterMode));
    app.clamp_scroll();
    assert_eq!(
        app.editor.active().unwrap().scroll_line,
        app.max_scroll_line()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点标题左边的箭头收起下面的一节，再点一次展开；收起时光标在这一节里的移到标题末尾。
#[test]
fn headings_fold_their_sections_in_the_preview() {
    let (mut app, dir) = live_doc(
        "md-fold",
        "# 甲\n\n第一段\n\n## 子\n\n第二段\n\n# 乙\n\n第三段\n",
    );
    let shown = |app: &App| crate::app::preview::markdown::laid::full_text(app).unwrap();
    let full = app.preview.markdown.height;
    assert!(shown(&app).contains("第一段"));
    // 光标放进要收起的一节里，再收起：光标回到标题末尾，这一节不再显示。
    put_caret(&mut app, "第二段", true);
    app.start_live_editing();
    app.toggle_heading_fold("甲");
    settle(&mut app);
    assert_eq!(app.live_caret_source(), doc_text(&app).find("\n"));
    let text = shown(&app);
    assert!(!text.contains("第一段") && !text.contains("第二段") && text.contains("第三段"));
    assert!(app.preview.markdown.height < full);

    // 鼠标停在标题上时显示箭头，单击展开。
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let area = app.preview.markdown.area;
    let top = area.y - app.preview.markdown.scroll() + laid.pad + laid.tops[0];
    app.on_mouse_move(area.x + area.w / 2.0, top + 4.0);
    paint(&mut app);
    shoot(&mut app, "md-fold");
    let (arrow, _) = (app.hits.markdown_links.iter())
        .find(|(_, url)| url.ends_with("fold:甲"))
        .unwrap()
        .clone();
    super::click(&mut app, (arrow.x + arrow.w / 2.0, arrow.y + arrow.h / 2.0));
    settle(&mut app);
    assert!(shown(&app).contains("第一段"));

    // 分栏时左边的光标在标题「乙」上：箭头与标出光标所在行的竖条同一列，竖条不再另画。
    app.set_preview_mode(PreviewMode::Split);
    app.focus = crate::app::Focus::Editor;
    put_caret(&mut app, "# 乙", true);
    settle(&mut app);
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let area = app.preview.markdown.area;
    let heading = laid.sections.iter().rposition(|s| s.is_some()).unwrap();
    let top = area.y - app.preview.markdown.scroll() + laid.pad + laid.tops[heading];
    app.on_mouse_move(area.x + area.w / 2.0, top + 4.0);
    paint(&mut app);
    shoot(&mut app, "md-fold-split");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 可编辑的预览中在两处之间来回单击时内容不漂移。
#[test]
fn clicking_back_and_forth_does_not_drift() {
    // 和常见的 README 开头一样：几个居中的 HTML 块，下面是正文。
    let centered = |inner: &str| format!("<p align=\"center\">\n  {inner}\n</p>\n\n");
    let text = [
        centered("<img src=\"icon.svg\" alt=\"icon\" width=\"128\" />"),
        "<h1 align=\"center\">Winc</h1>\n\n".to_owned(),
        centered("A lighter, faster, more comfortable desktop editor."),
        centered("English | <a href=\"./README.zh-CN.md\">简体中文</a>"),
        centered("<img alt=\"a\" src=\"a.svg\" />\n  <img alt=\"b\" src=\"b.svg\" />"),
        centered("<a href=\"https://winc.si\"><img alt=\"Download\" src=\"d.svg\" /></a>"),
        format!("## Overview\n\n{}\n", "Some text here. ".repeat(300)),
    ]
    .concat();
    let (mut app, dir) = live_doc("md-drift", &text);
    let area = app.preview.markdown.area;
    let mut seen = Vec::new();
    for k in 0..8 {
        // 每次都点那一块现在所在的位置，和来回点击时一样。
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let block = if k % 2 == 0 { 1 } else { 5 };
        let h = laid.blocks[block].1.height;
        let y = area.y - app.preview.markdown.scroll() + laid.pad + laid.tops[block] + h - 10.0;
        super::click(&mut app, (area.x + area.w / 2.0, y));
        settle(&mut app);
        seen.push((app.preview.markdown.scroll(), y));
    }
    assert_eq!(seen[0..2], seen[2..4], "{seen:?}");
    assert_eq!(seen[0..2], seen[6..8], "{seen:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 高于上限的 mermaid 图收起显示，可以展开和收起。
#[test]
fn tall_diagrams_shrink_then_fold_until_expanded() {
    let nodes: String = (0..60)
        .map(|i| format!("    n{i}[第 {i} 步] --> n{}\n", i + 1))
        .collect();
    let text = format!("# 流程\n\n```mermaid\nflowchart TD\n{nodes}```\n\n尾\n");
    let (mut app, dir) = live_doc("md-tall", &text);
    let height = |app: &App| {
        app.preview.markdown.layout.as_ref().unwrap().blocks[1]
            .1
            .height
    };
    let cap = (app.preview.markdown.area.h * 0.66).round().max(320.0);
    let folded = height(&app);
    assert!(folded <= cap + 40.0, "{folded} > {cap}");
    shoot(&mut app, "md-tall");
    let link = |app: &App| {
        (app.hits.markdown_links.iter())
            .find(|(_, url)| url.starts_with(winc_typeset::DETAILS))
            .map(|(rect, _)| *rect)
            .unwrap()
    };
    let rect = link(&app);
    super::click(&mut app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    settle(&mut app);
    let open = height(&app);
    assert!(open > folded * 2.0, "{open} <= {folded}");
    // 展开后点击「收起」恢复收起时的高度。
    app.preview.markdown.scroll_to(open);
    settle(&mut app);
    shoot(&mut app, "md-tall-open");
    let rect = link(&app);
    super::click(&mut app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    settle(&mut app);
    assert_eq!(height(&app), folded);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 视口上方的图片加载完成后，视口顶边的内容位置不变。
#[test]
fn an_image_loading_above_keeps_the_view_still() {
    let dir = sample_folder("md-image-load");
    let png = winc_editor::paste::encode_png(40, 300, &[200; 40 * 300 * 4]).unwrap();
    std::fs::write(dir.join("tall.png"), png).unwrap();
    let paragraphs: String = (0..80).map(|i| format!("第 {i} 段\n\n")).collect();
    let path = dir.join("LIVE.md");
    std::fs::write(&path, format!("![图](tall.png)\n\n{paragraphs}")).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let image = app.preview.markdown.images.keys().next().cloned().unwrap();
    // 先把图片高度设为 1 像素，之后恢复实际高度。
    let set_height = |app: &mut App, h: u32| {
        if let Some(winc_typeset::ImageSlot::Ready { height, .. }) =
            app.preview.markdown.images.get_mut(&image)
        {
            *height = h;
        }
        app.preview.markdown.loaded.push(image.clone());
        app.preview.markdown.images_version += 1;
    };
    set_height(&mut app, 1);
    settle(&mut app);
    let top_of = |app: &App, i: usize| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        laid.tops[i] - app.preview.markdown.scroll()
    };
    app.preview
        .markdown
        .scroll_to(top_of(&app, 40) + app.preview.markdown.scroll());
    settle(&mut app);
    let before = top_of(&app, 40);
    // 上方的图片高度增加 300 后，第 40 块的位置不变。
    set_height(&mut app, 300);
    settle(&mut app);
    assert!(
        (top_of(&app, 40) - before).abs() < 1.0,
        "{} != {before}",
        top_of(&app, 40)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 收起、展开视口中的标题时，视口顶边的内容位置不变。
#[test]
fn folding_a_heading_keeps_the_view_still() {
    let sections: String = (0..12)
        .map(|i| format!("## 第 {i} 节\n\n{}\n\n", "正文。".repeat(80)))
        .collect();
    let (mut app, dir) = live_doc("md-fold-still", &sections);
    let top_of = |app: &App, i: usize| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        laid.tops[i] - app.preview.markdown.scroll()
    };
    // 第 3 节的正文位于视口顶边，第 4 节的标题位于视口中。
    app.preview
        .markdown
        .scroll_to(top_of(&app, 7) + app.preview.markdown.scroll() + 20.0);
    settle(&mut app);
    let before = top_of(&app, 7);
    app.toggle_heading_fold("第-4-节");
    settle(&mut app);
    assert!((top_of(&app, 7) - before).abs() < 1.0);
    app.toggle_heading_fold("第-4-节");
    settle(&mut app);
    assert!((top_of(&app, 7) - before).abs() < 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 指向不存在的文件或标题的链接标为失效，网址不检查。
#[test]
fn broken_links_are_marked() {
    use crate::app::preview::markdown::links::Broken;
    let text = "# 标题\n\n[好](short.rs) [坏](missing.md) [锚](#标题) [无](#没有) \
        [网](https://winc.si/x)\n";
    let (mut app, dir) = live_doc("md-broken-links", text);
    paint(&mut app);
    shoot(&mut app, "md-broken-links");
    let check = |app: &mut App, url: &str| app.check_markdown_link(url);
    assert_eq!(check(&mut app, "short.rs"), None);
    assert_eq!(check(&mut app, "missing.md"), Some(Broken::File));
    assert_eq!(check(&mut app, "#标题"), None);
    assert_eq!(check(&mut app, "#没有"), Some(Broken::Heading));
    assert_eq!(check(&mut app, "https://winc.si/x"), None);
    // 文件创建并处理磁盘变化后，链接不再标为失效。
    std::fs::write(dir.join("missing.md"), "").unwrap();
    app.preview.markdown.links.clear();
    assert_eq!(check(&mut app, "missing.md"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 右键菜单「在新标签中打开图片」打开本地图片，包括包在链接中的图片。
#[test]
fn images_open_in_a_tab_from_the_menu() {
    use crate::app::input::context_menu::MenuCommand;
    let dir = sample_folder("md-image-open");
    let png = winc_editor::paste::encode_png(200, 120, &[90; 200 * 120 * 4]).unwrap();
    std::fs::write(dir.join("shot.png"), png).unwrap();
    let path = dir.join("LIVE.md");
    std::fs::write(&path, "[![图](shot.png)](https://winc.si)\n\n后文\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let [(image, _)] = app.hits.markdown_pictures.clone()[..] else {
        panic!("画出了一张本地图片");
    };
    right_click(&mut app, (image.x + 20.0, image.y + 20.0));
    assert!(menu_commands(&app).contains(&(MenuCommand::OpenImage, true)));
    app.run_menu_command(MenuCommand::OpenImage);
    settle(&mut app);
    let opened = app.editor.active().and_then(|d| d.path.clone()).unwrap();
    assert!(
        opened.ends_with("md-image-open/shot.png"),
        "{}",
        opened.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// HTML 块显示原文和恢复排版时，渲染结果在过渡的第一帧位于原来的位置。
#[test]
fn source_and_rendering_swap_without_a_jump() {
    let dir = sample_folder("md-swap-slide");
    let png = winc_editor::paste::encode_png(200, 80, &[90; 200 * 80 * 4]).unwrap();
    std::fs::write(dir.join("logo.png"), png).unwrap();
    let path = dir.join("LIVE.md");
    let text =
        "前文\n\n<p align=\"center\">\n  <img src=\"logo.png\" alt=\"logo\" />\n</p>\n\n后文\n";
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let image_y = |app: &App| app.hits.markdown_pictures[0].0.y;
    let rendered = image_y(&app);

    // 点击图片后块显示原文：第一帧图片位于原处，过渡结束时位于代码框下方。
    app.preview.markdown.frozen = Some(std::time::Duration::ZERO);
    let image = app.hits.markdown_pictures[0].0;
    super::click(&mut app, (image.x + 10.0, image.y + 10.0));
    paint(&mut app);
    assert_eq!(image_y(&app), rendered);
    // 冻结的时钟不前进：移除过渡后检查最终位置。
    app.preview.markdown.frozen = None;
    app.preview.markdown.motion = None;
    settle(&mut app);
    let below = image_y(&app);
    assert!(below > rendered + 20.0, "{below} <= {rendered}");

    // 光标移到别处后恢复排版：第一帧图片位于代码框下方，过渡结束时回到原处。
    app.preview.markdown.frozen = Some(std::time::Duration::ZERO);
    put_caret(&mut app, "后文", true);
    paint(&mut app);
    assert_eq!(image_y(&app), below);
    // 冻结的时钟不前进：移除过渡后检查最终位置。
    app.preview.markdown.frozen = None;
    app.preview.markdown.motion = None;
    settle(&mut app);
    assert_eq!(image_y(&app), rendered);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：HTML 块中前一张图片无法加载时，调整的仍是右键点击的图片。
#[test]
fn resizing_picks_the_clicked_image_when_an_earlier_one_fails() {
    use crate::app::input::context_menu::MenuCommand;
    let dir = sample_folder("md-image-resize-failed");
    let png = winc_editor::paste::encode_png(200, 80, &[90; 200 * 80 * 4]).unwrap();
    std::fs::write(dir.join("logo.png"), png).unwrap();
    let path = dir.join("LIVE.md");
    let text = "<p align=\"center\">\n  <img src=\"missing.png\" alt=\"无\">\n  \
        <img src=\"logo.png\" alt=\"标志\">\n</p>\n\n后文\n";
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let [image] = app.hits.markdown_images[..] else {
        panic!("只画出了读得出的那张");
    };
    let center = (image.rect.x + 20.0, image.rect.y + 20.0);
    right_click(&mut app, center);
    app.run_menu_command(MenuCommand::ImageWidth(50));
    let changed =
        "<img src=\"missing.png\" alt=\"无\">\n  <img src=\"logo.png\" alt=\"标志\" width=\"50%\">";
    assert!(doc_text(&app).contains(changed), "{}", doc_text(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 指向收起的一节中标题的链接不标为失效，跳转时展开该节。
#[test]
fn links_into_folded_sections_unfold_them() {
    let text = "# 总\n\n见[子](#子)\n\n## 节\n\n### 子\n\n内容\n\n## 后\n\n尾\n";
    let (mut app, dir) = live_doc("md-fold-anchor", text);
    app.toggle_heading_fold("节");
    settle(&mut app);
    assert!(crate::app::preview::markdown::laid::anchor_box(&app, "子").is_none());
    assert_eq!(app.check_markdown_link("#子"), None);
    app.jump_to_anchor("子");
    settle(&mut app);
    let id = app.editor.active().unwrap().id;
    assert!(!app.preview.markdown.folds.contains_key(&id));
    assert!(crate::app::preview::markdown::laid::anchor_box(&app, "子").is_some());
    // 在标题列表中点收起的一节里的标题同样展开。
    app.toggle_heading_fold("节");
    settle(&mut app);
    assert_eq!(app.hits.markdown_toc_blocks.len(), 4);
    app.press_toc(crate::app::preview::markdown::TocTarget::Heading(2));
    settle(&mut app);
    assert!(!app.preview.markdown.folds.contains_key(&id));
    assert!(crate::app::preview::markdown::laid::anchor_box(&app, "子").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 内容相同的两张高图各自记录开合状态。
#[test]
fn identical_tall_diagrams_expand_separately() {
    let nodes: String = (0..60)
        .map(|i| format!("    n{i}[第 {i} 步] --> n{}\n", i + 1))
        .collect();
    let chart = format!("```mermaid\nflowchart TD\n{nodes}```\n");
    let text = format!("# 流程\n\n{chart}\n{chart}\n尾\n");
    let (mut app, dir) = live_doc("md-tall-twins", &text);
    let height = |app: &App, i: usize| {
        app.preview.markdown.layout.as_ref().unwrap().blocks[i]
            .1
            .height
    };
    let folded = height(&app, 1);
    assert_eq!(height(&app, 2), folded);
    let link = (app.hits.markdown_links.iter())
        .find(|(_, url)| url.starts_with(winc_typeset::DETAILS))
        .map(|(rect, _)| *rect)
        .unwrap();
    super::click(&mut app, (link.x + link.w / 2.0, link.y + link.h / 2.0));
    settle(&mut app);
    assert!(height(&app, 1) > folded * 2.0);
    assert_eq!(height(&app, 2), folded);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 行首或空白之后输入 `/` 时列出写作命令，回车执行并删去 `/命令名`。
#[test]
fn slash_lists_writing_commands() {
    use crate::app::preview::markdown::slash::Slash;
    use crate::app::preview::markdown::suggest::Hit;
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = source_doc("md-slash", "");
    app.track_suggest();
    let typed = |app: &mut App, text: &str| {
        app.run(winc_editor::Command::Insert(text.into()));
        app.track_suggest();
    };
    let first = |app: &App| app.preview.markdown.suggest.hits.first().cloned();
    typed(&mut app, "/");
    assert!(app.preview.markdown.suggest.hits.len() >= 8);
    paint(&mut app);
    // 选中第一行、悬停第二行，两行的底色之间留有间隙。
    let (second, _) = app.hits.suggest[1];
    app.on_mouse_move(second.x + 40.0, second.y + second.h / 2.0);
    paint(&mut app);
    shoot(&mut app, "md-slash");
    // 空行上的标题命令写入记号。
    typed(&mut app, "h2");
    assert_eq!(first(&app), Some(Hit::Command(Slash::Heading(2))));
    paint(&mut app);
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert_eq!(marked(&app), "## |");
    // 有文字的行后面插入代码块，光标在开头的围栏之后，可以输入语言。
    typed(&mut app, "标题\n\n文字 /daima");
    assert_eq!(first(&app), Some(Hit::Command(Slash::Code)));
    paint(&mut app);
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert_eq!(marked(&app), "## 标题\n\n文字 \n\n```|\n\n```");
    // 路径中的 `/` 不呼出命令。
    typed(&mut app, "见 /usr/bin");
    assert!(
        app.preview.markdown.suggest.hits.is_empty() || app.preview.markdown.suggest.at.is_none()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：源代码中写作命令的列表覆盖在文字上方，鼠标位于列表上时显示箭头。
#[test]
fn the_text_cursor_leaves_the_slash_list_in_the_source() {
    use winit::window::CursorIcon;
    let (mut app, dir) = source_doc("md-slash-cursor", "");
    app.track_suggest();
    app.run(winc_editor::Command::Insert("/".into()));
    app.track_suggest();
    paint(&mut app);
    let card = app.hits.suggest_card.expect("列出写作命令");
    app.on_mouse_move(card.x + card.w / 2.0, card.y + card.h / 2.0);
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Default);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：源代码中写作命令的列表覆盖在文字上方，在列表上右键不弹出编辑区的菜单，光标不动，
/// 列表保持打开。
#[test]
fn right_clicks_on_the_slash_list_reach_no_menu() {
    let (mut app, dir) = source_doc("md-slash-menu", "");
    app.track_suggest();
    app.run(winc_editor::Command::Insert("/".into()));
    app.track_suggest();
    paint(&mut app);
    let card = app.hits.suggest_card.expect("列出写作命令");
    let caret = app.editor.active().unwrap().selection;
    right_click(&mut app, (card.x + card.w / 2.0, card.y + card.h / 2.0));
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(app.editor.active().unwrap().selection, caret);
    paint(&mut app);
    assert!(app.hits.suggest_card.is_some(), "列表保持打开");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 可编辑的预览中选中文字后显示格式工具栏，按钮执行写作命令；选区改变或关闭设置后不显示。
#[test]
fn selecting_text_shows_the_format_toolbar() {
    use crate::app::preview::markdown::toolbar::Tool;
    let (mut app, dir) = live_doc("md-toolbar", "前文\n\n这是一段要强调的文字。\n");
    select_with_toolbar(&mut app, "强调");
    assert!(app.hits.selection_card.is_some());
    shoot(&mut app, "md-toolbar");
    let bold = (app.hits.selection_tools.iter())
        .find(|(_, t)| *t == Tool::Bold)
        .map(|(r, _)| *r)
        .unwrap();
    super::click(&mut app, (bold.x + bold.w / 2.0, bold.y + bold.h / 2.0));
    paint(&mut app);
    assert!(
        doc_text(&app).contains("要**强调**的"),
        "{}",
        doc_text(&app)
    );
    assert!(app.hits.selection_card.is_some());
    assert_eq!(app.active_tools(), [Tool::Bold]);
    // 选区改变后不再显示。
    put_caret(&mut app, "文字", true);
    paint(&mut app);
    assert!(app.hits.selection_card.is_none());
    // 关闭设置后不显示。
    app.config.markdown_selection_toolbar = Some(false);
    select_with_toolbar(&mut app, "前文");
    assert!(app.hits.selection_card.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：连续点击格式工具栏的按钮时，选区随标记的增删和字形的变化改变宽度，工具栏跟着选区的
/// 中点左右移动。
#[test]
fn the_format_toolbar_stays_put_while_its_buttons_are_clicked() {
    use crate::app::preview::markdown::toolbar::Tool;
    let (mut app, dir) = live_doc("md-toolbar-steady", "前文\n\n这是一段要强调的文字。\n");
    select_with_toolbar(&mut app, "强调");
    let card = app.hits.selection_card.unwrap();
    let tools = [
        Tool::Bold,
        Tool::Bold,
        Tool::Code,
        Tool::Highlight,
        Tool::Bold,
    ];
    for tool in tools {
        let (button, _) = *(app.hits.selection_tools.iter())
            .find(|(_, t)| *t == tool)
            .unwrap();
        super::click(
            &mut app,
            (button.x + button.w / 2.0, button.y + button.h / 2.0),
        );
        paint(&mut app);
        let now = app.hits.selection_card.expect("点击后仍显示工具栏");
        assert_eq!(
            now.x,
            card.x,
            "点击 {tool:?} 后工具栏移动：{}",
            doc_text(&app)
        );
    }
    // 重新选中文字后不再沿用固定的位置，按新的选区放置。
    select_with_toolbar(&mut app, "前文");
    assert!(app.hits.selection_card.is_some());
    let toolbar = app.preview.markdown.toolbar.unwrap();
    assert_eq!(toolbar.left, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 可编辑的预览中同样可以用 `/` 呼出写作命令。
#[test]
fn slash_commands_work_in_the_editable_preview() {
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = live_doc("md-slash-live", "前文\n\n");
    put_caret(&mut app, "前文\n\n", true);
    app.start_live_editing();
    app.track_suggest();
    app.run(winc_editor::Command::Insert("/yinyong".into()));
    app.track_suggest();
    assert!(!app.preview.markdown.suggest.hits.is_empty());
    paint(&mut app);
    assert!(app.suggest_key(&Key::Named(NamedKey::Enter)));
    assert_eq!(doc_text(&app), "前文\n\n> ");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 命令列表随选中项滚动，最近用过的命令排在最前；行内公式和日期写在光标处。
#[test]
fn slash_list_scrolls_and_remembers_recent_commands() {
    use crate::app::preview::markdown::slash::Slash;
    use crate::app::preview::markdown::suggest::Hit;
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = source_doc("md-slash-more", "");
    app.track_suggest();
    let typed = |app: &mut App, text: &str| {
        app.run(winc_editor::Command::Insert(text.into()));
        app.track_suggest();
    };
    let press = |app: &mut App, key: NamedKey| {
        paint(app);
        assert!(app.suggest_key(&Key::Named(key)));
    };
    typed(&mut app, "/");
    for _ in 0..12 {
        press(&mut app, NamedKey::ArrowDown);
    }
    paint(&mut app);
    assert_eq!(app.preview.markdown.suggest.first, 3);
    shoot(&mut app, "md-slash-scroll");
    // 滚轮在列表上滚动列表，选中项不变。
    let card = app.hits.suggest_card.unwrap();
    let inside = (card.x + 20.0, card.y + 20.0);
    let target = app.scroll_target_at(inside.0, inside.1);
    app.scroll_target_by(target, inside, false, (0.0, -1000.0));
    assert_eq!(app.preview.markdown.suggest.first, 15);
    assert_eq!(app.preview.markdown.suggest.selected, 12);
    typed(&mut app, "hangnei");
    press(&mut app, NamedKey::Enter);
    assert_eq!(marked(&app), "$|$");
    typed(&mut app, " /riqi");
    press(&mut app, NamedKey::Enter);
    let (y, m, d, ..) = crate::system::local_time();
    let date = format!("$ {y:04}-{m:02}-{d:02}|$");
    assert!(marked(&app).ends_with(&date), "{}", marked(&app));
    // 最近用过的日期、行内公式排在最前。
    typed(&mut app, "\n\n/");
    let hits = &app.preview.markdown.suggest.hits;
    assert_eq!(
        hits[..2],
        [Hit::Command(Slash::Date), Hit::Command(Slash::InlineMath)]
    );
    // 改写整行的命令同样记为最近使用。
    typed(&mut app, "biaoti");
    press(&mut app, NamedKey::Enter);
    typed(&mut app, "\n\n/");
    let hits = &app.preview.markdown.suggest.hits;
    assert_eq!(hits[0], Hit::Command(Slash::Heading(1)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 粘贴为纯文本：选中文字后粘贴网址不写成链接，原样插入；整行复制的内容仍插入到当前行上方，
/// 同普通的粘贴。源代码的右键菜单里「粘贴」下面就是它，写着它的快捷键。
///
/// 回归：整行复制以换行符结尾的文件的最后一行，曾多带一个换行符，粘贴后多出一个空行。
#[test]
fn pasting_as_plain_text_skips_the_markdown_rewriting() {
    use crate::app::PopupRow;
    use crate::app::input::context_menu::MenuCommand;

    let (mut app, dir) = source_doc("md-paste-plain", "说明\n甲\n乙\n");
    select(&mut app, "说明");
    app.clipboard.copied = Some("https://winc.si".into());
    assert!(app.run_action(Action::MarkdownPastePlain));
    assert_eq!(doc_text(&app), "https://winc.si\n甲\n乙\n");

    // 整行复制「乙」，光标放到「甲」上：插入到「甲」上方。
    put_caret(&mut app, "乙", false);
    app.copy(false);
    // 「乙」是最后一行、文件以换行符结尾：复制的就是「乙」一行，不多一个空行。
    assert_eq!(app.clipboard.copied.as_deref(), Some("乙\n"));
    put_caret(&mut app, "甲", false);
    assert!(app.run_action(Action::MarkdownPastePlain));
    assert_eq!(doc_text(&app), "https://winc.si\n乙\n甲\n乙\n");

    // 右键菜单：「粘贴」之后是「粘贴为纯文本」，带快捷键。
    let (text, lh) = (app.layout.text, app.fonts.line_height);
    right_click(&mut app, (text.x + 20.0, text.y + lh / 2.0));
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    let paste = commands
        .iter()
        .position(|&c| c == MenuCommand::Paste)
        .unwrap();
    assert_eq!(commands.get(paste + 1), Some(&MenuCommand::PastePlain));
    let plain = app.menu_label(MenuCommand::PastePlain).title;
    let key = (app.popup().unwrap().levels[0].rows.iter()).find_map(|r| match r {
        PopupRow::Item { label, key, .. } if label.title == plain => Some(key.clone()),
        _ => None,
    });
    assert!(key.is_some_and(|k| !k.is_empty()), "菜单上没有写快捷键");
    let _ = std::fs::remove_dir_all(&dir);
}
