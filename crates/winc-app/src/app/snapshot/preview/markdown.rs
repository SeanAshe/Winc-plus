//! Markdown 预览的测试：各类块的排版、目录、链接和滚动。

use super::*;

#[test]
fn markdown_files_get_the_three_way_switch() {
    let (mut app, dir) = readme("md-modes", false, PreviewMode::Split);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    assert!(doc.shows_text() && !doc.readonly);
    assert!(app.layout.preview.w > 0.0);
    // 右边是预览：绘制在分栏的右半边，已完成排版并记录链接。
    assert_eq!(app.preview.markdown.area, app.layout.split);
    assert!(app.preview.markdown.height > app.preview.markdown.area.h);
    assert!(app.hits.markdown_links.len() >= 2);
    // 图片已读取，并按显示大小绘制。
    assert!(matches!(
        app.preview.markdown.images.values().next(),
        Some(crate::app::preview::markdown::ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));

    // 只看预览：整个编辑区，不显示源代码，但可以直接编辑；切回源代码后仍可正常编辑。
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(!doc.readonly && !doc.shows_text() && doc.edits_text());
    assert!(app.markdown_live());
    assert_eq!(app.preview.markdown.area, app.layout.editor);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().readonly);
    assert_eq!(app.preview.markdown.area, Rect::default());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tab_nests_list_items_in_the_preview() {
    use winit::keyboard::{Key, NamedKey};
    let tab = Key::Named(NamedKey::Tab);

    let (mut app, dir) = live_doc("md-tab", "- 一\n- 二\n\n段落\n");
    put_caret(&mut app, "二", true);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(with_caret(&app), "- 一\n  - 二|\n\n段落\n");
    settle(&mut app);
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&tab, false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(with_caret(&app), "- 一\n- 二|\n\n段落\n");
    settle(&mut app);
    // 第一项无法缩进，也不向文字中插入空格。
    put_caret(&mut app, "一", false);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(with_caret(&app), "- |一\n- 二\n\n段落\n");
    // 不是列表项时交给编辑区处理。
    put_caret(&mut app, "段落", true);
    assert!(!app.markdown_key(&tab, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn arrows_skip_the_blank_lines_between_blocks() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = live_doc("md-gap", "甲\n\n\n乙\n");
    put_caret(&mut app, "甲", true);
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowRight), false));
    assert_eq!(with_caret(&app), "甲\n\n\n|乙\n");
    settle(&mut app);
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));
    assert_eq!(with_caret(&app), "甲|\n\n\n乙\n");
    settle(&mut app);
    // 块内的逐字移动交给编辑区。
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));
    // 文末之后没有块：停在空行上（交给编辑区）。
    put_caret(&mut app, "乙", true);
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowRight), false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pasting_a_url_over_selected_text_makes_a_link() {
    let (mut app, dir) = live_doc("md-paste-link", "见文档。\n\n`代码`\n\n```\n块\n```\n");
    let select = |app: &mut App, needle: &str| {
        let text = doc_text(app);
        let byte = text.find(needle).unwrap();
        let doc = app.editor.active_mut().unwrap();
        let rope = doc.text().rope().unwrap();
        let (a, b) = (
            rope.byte_to_char(byte),
            rope.byte_to_char(byte + needle.len()),
        );
        doc.selection = winc_core::Selection::new(a, b);
    };
    select(&mut app, "文档");
    assert_eq!(
        app.markdown_link_paste(" https://a.dev/x\n").as_deref(),
        Some("[文档](https://a.dev/x)")
    );
    assert_eq!(
        app.markdown_link_paste("https://a.dev/(1)").as_deref(),
        Some("[文档](<https://a.dev/(1)>)")
    );
    // 不是网址、没有选区、在行内代码或代码块中时按普通方式粘贴。
    assert!(app.markdown_link_paste("文字").is_none());
    assert!(app.markdown_link_paste("https://a.dev x").is_none());
    select(&mut app, "代码");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    select(&mut app, "块");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    put_caret(&mut app, "见", true);
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    // 切换回源代码：预览已释放解析结果，按围栏判断代码块。
    app.set_preview_mode(PreviewMode::Source);
    settle(&mut app);
    assert!(app.preview.markdown.parsed.is_none() && app.preview.markdown.layout.is_none());
    select(&mut app, "块");
    assert!(app.markdown_link_paste("https://a.dev").is_none());
    select(&mut app, "文档");
    assert!(app.markdown_link_paste("https://a.dev").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_a_task_box_toggles_it() {
    let (mut app, dir) = live_doc("md-task", "开头\n\n- [ ] 甲\n- [x] 乙\n");
    let task = crate::app::preview::markdown::laid::TASK;
    let boxes = |app: &App| -> Vec<winc_render::Rect> {
        app.hits
            .markdown_links
            .iter()
            .filter(|(_, url)| url.starts_with(task))
            .map(|(r, _)| *r)
            .collect()
    };
    let caret = app.editor.active().unwrap().selection;
    let first = boxes(&app)[0];
    super::click(&mut app, (first.x + first.w / 2.0, first.y + first.h / 2.0));
    settle(&mut app);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [x] 乙\n");
    // 光标不移动，也不因此进入编辑状态：列表仍按排版显示，勾选框保留。
    assert_eq!(app.editor.active().unwrap().selection, caret);
    let second = boxes(&app)[1];
    super::click(&mut app, (second.x + 2.0, second.y + 2.0));
    settle(&mut app);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [ ] 乙\n");
    app.run(winc_editor::Command::Undo);
    assert_eq!(doc_text(&app), "开头\n\n- [x] 甲\n- [x] 乙\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_across_blocks_keeps_the_layout_still() {
    let (mut app, dir) = live_doc("md-drag", "第一段文字\n\n第二段文字\n\n第三段文字\n");
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let at = |s: &str| shown[..shown.find(s).unwrap()].chars().count();
    let (x, y) = point_of(&app, at("第一段") + 1);
    let (x2, y2) = point_of(&app, at("第三段") + 1);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    paint(&mut app);
    // 按住鼠标拖动期间排版不变（尚未开始编辑，全部按排版显示），鼠标下方的内容不移位。
    assert!(app.live_leaf().is_none());
    app.on_mouse_move(x2, y2 + 1.0);
    paint(&mut app);
    assert!(app.live_leaf().is_none());
    assert_eq!(
        crate::app::preview::markdown::laid::full_text(&app).unwrap(),
        shown
    );
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let text = doc_text(&app);
    let rope = doc.text().rope().unwrap();
    let selected =
        &text[rope.char_to_byte(doc.selection.from())..rope.char_to_byte(doc.selection.to())];
    assert!(
        selected.starts_with("一段") && selected.contains("第二段文字"),
        "{selected}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn math_and_diagrams_show_the_result_below_the_source() {
    let height = |text: &str| {
        let (mut app, dir) = live_doc("md-below", text);
        put_caret(&mut app, "A", false);
        app.run(winc_editor::Command::Insert(" ".into()));
        settle(&mut app);
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        // 光标不在围栏行上时只显示围栏之间的原文，不显示围栏。
        assert!(shown.contains(" A") && !shown.contains("```"), "{shown}");
        assert!(!shown.contains("$$"), "{shown}");
        let height = app.preview.markdown.height;
        let _ = std::fs::remove_dir_all(&dir);
        (height, shown)
    };
    // 原文下方绘制渲染后的图，因此比只显示原文的代码块更高。
    let (diagram, _) = height("```mermaid\ngraph TD\nA-->B\n```\n");
    let (plain, _) = height("```text\ngraph TD\nA-->B\n```\n");
    assert!(diagram > plain + 40.0, "{diagram} {plain}");
    // 公式的原文下面是排好的公式（复制的文字是公式的原文，跟在原文后面）。
    let (_, shown) = height("$$\nA^2 + b\n$$\n");
    assert_eq!(shown.matches("A^2").count(), 2, "{shown}");
}

#[test]
fn clicking_an_anchor_link_scrolls_to_the_heading() {
    let (mut app, dir) = readme("md-anchor", false, PreviewMode::Preview);
    let index = app
        .hits
        .markdown_links
        .iter()
        .position(|(_, url)| url == "#安装")
        .expect("有转到「安装」的链接");
    let (rect, _) = app.hits.markdown_links[index].clone();
    // 可编辑的预览中单击链接只放光标。
    super::click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    assert_eq!(app.preview.markdown.scroll(), 0.0);
    command_click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    let top = crate::app::preview::markdown::laid::anchor_top(&app, "安装").unwrap();
    assert!(top > 0.0);
    // 标题滚动到顶部；其后的内容不足一屏时滚动到底为止。
    let max = app.preview.markdown.height - app.preview.markdown.area.h;
    assert_eq!(app.preview.markdown.scroll(), top.min(max));
    // 脚注的编号和末尾的 ↩ 能找到彼此。
    let note = crate::app::preview::markdown::laid::anchor_top(&app, "fn-1").expect("脚注有锚点");
    let back =
        crate::app::preview::markdown::laid::anchor_top(&app, "fnref-1").expect("能跳回引用处");
    assert!(back > top && note > back);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn code_blocks_offer_a_copy_button_on_hover() {
    let (mut app, dir) = readme("md-copy-code", false, PreviewMode::Preview);
    let copy = crate::app::preview::markdown::laid::COPY;
    assert!(
        !app.hits
            .markdown_links
            .iter()
            .any(|(_, url)| url.starts_with(copy))
    );
    let code = *app.hits.markdown_codes.first().expect("画出了代码块");
    app.on_mouse_move(code.x + 20.0, code.y + 20.0);
    paint(&mut app);
    let (rect, _) = app
        .hits
        .markdown_links
        .iter()
        .find(|(_, url)| url.starts_with(copy))
        .cloned()
        .expect("鼠标在代码块上时有复制按钮");
    super::click(&mut app, (rect.x + 4.0, rect.y + 4.0));
    let copied = app.clipboard.copied.clone().unwrap();
    assert!(copied.starts_with("fn main() {\n"), "{copied:?}");
    assert!(copied.trim_end().ends_with('}'));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn links_to_headings_in_other_files_open_there() {
    let dir = sample_folder("md-other-file");
    let path = dir.join("README.md");
    std::fs::write(&path, "见 [用法](docs/guide.md#用法)。\n").unwrap();
    let mut guide = String::from("# 指南\n\n");
    for i in 0..80 {
        guide.push_str(&format!("第 {i} 段。\n\n"));
    }
    guide.push_str("## 用法\n\n照着做。\n");
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/guide.md"), guide).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let (rect, _) = app.hits.markdown_links[0].clone();
    command_click(&mut app, (rect.x + 2.0, rect.y + 2.0));
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.path.as_ref().unwrap().ends_with("docs/guide.md"));
    assert_eq!(doc.preview_mode(), PreviewMode::Preview);
    let top = crate::app::preview::markdown::laid::anchor_top(&app, "用法").unwrap();
    assert!(top > 0.0);
    assert!(app.preview.markdown.pending_anchor.is_none());
    let max = app.preview.markdown.height - app.preview.markdown.area.h;
    assert_eq!(app.preview.markdown.scroll(), top.min(max));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headings_get_github_anchors() {
    use crate::view::preview::markdown::tests::slug;
    assert_eq!(slug("Getting Started!"), "getting-started");
    assert_eq!(slug("安装 winc"), "安装-winc");
    assert_eq!(slug("a_b-c"), "a_b-c");
}

/// 截图：分栏（左侧源代码、右侧预览）和只看预览，浅色、深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_preview() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        for (mode, name) in [
            (PreviewMode::Split, "split"),
            (PreviewMode::Preview, "preview"),
        ] {
            let name = format!("md-{name}-{theme}");
            let (mut app, dir) = readme(&name, dark, mode);
            shoot(&mut app, &name);
            if mode == PreviewMode::Preview {
                let area = app.preview.markdown.area;
                app.pointer.mouse = (area.x + 50.0, area.y + 50.0);
                app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    0.0, -700.0,
                )));
                settle(&mut app);
                shoot(&mut app, &format!("{name}-scrolled"));
                // 选中一段、查找「预览」。
                app.markdown_select_all();
                app.preview.markdown.selection = Some((10, 40));
                app.open_search(false);
                app.bars.find.input.set_text("预览");
                app.bars.search.query = "预览".into();
                app.search_changed();
                settle(&mut app);
                shoot(&mut app, &format!("{name}-find"));
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_rich_markdown() {
    for dark in [false, true] {
        let dir = sample_folder("md-rich");
        let path = dir.join("RICH.md");
        std::fs::write(&path, RICH).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        shoot(
            &mut app,
            &format!("md-rich-{}", if dark { "dark" } else { "light" }),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 可绘制的 mermaid 图在悬停时显示全屏按钮；不支持的图按代码块显示，只有复制按钮。
#[test]
fn mermaid_blocks_draw_diagrams_and_fall_back_to_code() {
    let dir = sample_folder("md-mermaid-fallback");
    let path = dir.join("M.md");
    let drawn = "```mermaid\ngraph TD\n  A --> B\n```\n";
    let unknown = "```mermaid\njourney\n  title 步骤\n```\n";
    let expand = crate::app::preview::markdown::laid::EXPAND;
    for (source, diagram) in [(drawn, true), (unknown, false)] {
        std::fs::write(&path, source).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let [block] = app.hits.markdown_codes[..] else {
            panic!("只有一个代码块或图：{source}");
        };
        app.on_mouse_move(block.x + block.w / 2.0, block.y + block.h / 2.0);
        paint(&mut app);
        let has_expand = app.hits.markdown_links.iter().any(|(_, url)| url == expand);
        assert_eq!(has_expand, diagram, "{source}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 类图、实体关系图、思维导图、Git 提交图、甘特图都画成图，不按代码块显示。
#[test]
fn more_diagram_kinds_draw_as_diagrams() {
    for source in MORE_DIAGRAMS {
        let (mut app, dir) = live_doc("md-mermaid-more", source);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let [block] = app.hits.markdown_codes[..] else {
            panic!("只有一张图：{source}");
        };
        app.on_mouse_move(block.x + block.w / 2.0, block.y + block.h / 2.0);
        paint(&mut app);
        let expand = crate::app::preview::markdown::laid::EXPAND;
        assert!(
            app.hits.markdown_links.iter().any(|(_, url)| url == expand),
            "{source}"
        );
        let over = crate::view::preview::markdown::tests::overflow(&app);
        assert!(over <= 0.5, "超出 {over}：{source}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_more_mermaid_diagrams() {
    for dark in [false, true] {
        for (k, source) in MORE_DIAGRAMS.iter().enumerate() {
            let dir = sample_folder("md-mermaid-more");
            let path = dir.join("MERMAID.md");
            std::fs::write(&path, source).unwrap();
            let mut app = app_with(&path, dark);
            paint(&mut app);
            app.set_preview_mode(PreviewMode::Preview);
            settle(&mut app);
            let theme = if dark { "dark" } else { "light" };
            shoot(&mut app, &format!("md-mermaid-more-{k}-{theme}"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_mermaid_diagrams() {
    for dark in [false, true] {
        for (k, source) in DIAGRAMS.iter().enumerate() {
            let dir = sample_folder("md-mermaid");
            let path = dir.join("MERMAID.md");
            std::fs::write(&path, source).unwrap();
            let mut app = app_with(&path, dark);
            paint(&mut app);
            app.set_preview_mode(PreviewMode::Preview);
            settle(&mut app);
            let theme = if dark { "dark" } else { "light" };
            shoot(&mut app, &format!("md-mermaid-{k}-{theme}"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// 全屏查看：图覆盖编辑区并按适应查看区域的倍数显示；工具栏按钮、按键与按住 `⌘` 滚动可以
/// 缩放，拖动可以平移；`Esc`、关闭按钮或切回源代码时关闭，查看期间输入不写入文档。
#[test]
fn diagrams_open_full_screen_and_zoom() {
    use winit::keyboard::{Key, NamedKey};

    use crate::app::preview::markdown::DiagramTool;
    let (mut app, dir) = diagram_view_app("md-diagram-view", 0, false);
    let view = app
        .preview
        .markdown
        .diagram
        .as_ref()
        .expect("打开了全屏查看");
    assert_eq!(view.area, app.layout.editor);
    assert!(view.zoom.is_none());
    let fit = view.scale();
    assert!(fit > 1.0, "小图放大到适应窗口：{fit}");
    let button = |app: &App, tool| {
        let view = app.preview.markdown.diagram.as_ref().unwrap();
        let (r, _) = view.buttons.iter().find(|(_, t)| *t == tool).unwrap();
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    };
    let scale = |app: &App| app.preview.markdown.diagram.as_ref().unwrap().scale();

    let at = button(&app, DiagramTool::ZoomIn);
    click(&mut app, at);
    paint(&mut app);
    assert!((scale(&app) - fit * 1.25).abs() < 1e-3);
    assert!(app.diagram_view_key(&Key::Character("-".into()), "-", false));
    assert!((scale(&app) - fit).abs() < 1e-3);

    // 按住 `⌘` 滚动：以鼠标处为中心放大，鼠标处的内容位置不变。
    let area = app.preview.markdown.diagram.as_ref().unwrap().area;
    let at = (area.x + area.w / 4.0, area.y + area.h / 3.0);
    app.on_mouse_move(at.0, at.1);
    app.pointer.modifiers = command_modifiers();
    // 滚动量以物理像素给出；离屏窗口的缩放倍数为 2，换算为 100 逻辑像素。
    app.on_wheel(MouseScrollDelta::PixelDelta((0.0, 200.0).into()));
    app.pointer.modifiers = Default::default();
    paint(&mut app);
    let view = app.preview.markdown.diagram.as_ref().unwrap();
    let k = view.scale() / fit;
    assert!(k > 1.5, "{k}");
    let (cx, cy) = (at.0 - area.x - area.w / 2.0, at.1 - area.y - area.h / 2.0);
    assert!((view.pan.0 - (cx - cx * k)).abs() < 0.5);
    assert!((view.pan.1 - (cy - cy * k)).abs() < 0.5);

    // 拖动平移；点击倍数后恢复适应查看区域的倍数，图居中。
    let before = view.pan;
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(at.0 + 30.0, at.1 - 10.0);
    app.on_mouse_up(MouseButton::Left);
    let view = app.preview.markdown.diagram.as_ref().unwrap();
    assert!((view.pan.0 - (before.0 + 30.0)).abs() < 1e-3);
    assert!((view.pan.1 - (before.1 - 10.0)).abs() < 1e-3);
    assert!(view.grab.is_none());
    let at = button(&app, DiagramTool::Fit);
    click(&mut app, at);
    let view = app.preview.markdown.diagram.as_ref().unwrap();
    assert_eq!((view.zoom, view.pan), (None, (0.0, 0.0)));

    // 全屏查看期间的输入不写入被遮住的文档。
    let revision = app.editor.active().unwrap().revision;
    assert!(app.diagram_view_key(&Key::Character("x".into()), "x", false));
    assert_eq!(app.editor.active().unwrap().revision, revision);

    assert!(app.diagram_view_key(&Key::Named(NamedKey::Escape), "", false));
    assert!(app.preview.markdown.diagram.is_none());

    // 关闭按钮关闭全屏查看；切回源代码同样关闭。
    let (mut app, dir2) = diagram_view_app("md-diagram-view-close", 0, false);
    let at = button(&app, DiagramTool::Close);
    click(&mut app, at);
    assert!(app.preview.markdown.diagram.is_none());
    let (mut app, dir3) = diagram_view_app("md-diagram-view-source", 0, false);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert!(app.preview.markdown.diagram.is_none());
    for dir in [dir, dir2, dir3] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_diagram_view() {
    for dark in [false, true] {
        let (mut app, dir) = diagram_view_app("md-diagram-shot", 3, dark);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("md-diagram-view-{theme}"));
        // 关闭全屏查看后鼠标仍在图上，截取图右上角的全屏、复制按钮。
        app.close_diagram_view();
        let diagram = *app.hits.markdown_codes.first().unwrap();
        app.on_mouse_move(diagram.right() - 40.0, diagram.y + 20.0);
        shoot(&mut app, &format!("md-diagram-buttons-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 计时：只有一个很大的顶层块（长列表、长表格、长代码块）时，分栏滚动时每帧的耗时。
#[test]
#[ignore = "计时，手动运行"]
fn time_huge_blocks() {
    let list: String = (0..8000)
        .map(|i| format!("- 第 {i} 项，**加粗** 和 `代码`\n"))
        .collect();
    let table: String = "| a | b | c |\n|---|---|---|\n".to_owned()
        + &(0..8000)
            .map(|i| format!("| {i} | 第 {i} 行 | 说明 |\n"))
            .collect::<String>();
    let code = format!(
        "```rust\n{}```\n",
        (0..20000)
            .map(|i| format!("let x{i} = {i}; // 注释\n"))
            .collect::<String>()
    );
    for (name, text) in [("list", list), ("table", table), ("code", code)] {
        let dir = sample_folder(&format!("md-huge-{name}"));
        let path = dir.join("BIG.md");
        std::fs::write(&path, &text).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        // 对照：不分栏时滚动到同样的位置。
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.editor.active_mut().unwrap().scroll_line = (i * 97) as f64;
            paint(&mut app);
        }
        eprintln!("{name} 不分栏滚 50 次 {:?}", started.elapsed());
        app.set_preview_mode(PreviewMode::Split);
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!(
            "{name} {} KB 第一次排版 {:?}",
            text.len() / 1024,
            started.elapsed()
        );
        let started = std::time::Instant::now();
        for i in 0..50 {
            app.editor.active_mut().unwrap().scroll_line = (i * 97) as f64;
            paint(&mut app);
        }
        eprintln!("{name} 滚左边 50 次 {:?}", started.elapsed());
        // 在中间某一项（行）中修改一个字符。
        app.go_to(4000, 3, 0);
        paint(&mut app);
        app.run(winc_editor::Command::Insert("字".into()));
        app.wait_for_markdown();
        let started = std::time::Instant::now();
        paint(&mut app);
        eprintln!("{name} 改一个字 {:?}", started.elapsed());
        // 只看预览时在同一处输入：光标所在的块显示原文，每输入一个字符重排该块（不等待解析）。
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        app.run(winc_editor::Command::Insert("字".into()));
        paint(&mut app);
        let started = std::time::Instant::now();
        for _ in 0..10 {
            app.run(winc_editor::Command::Insert("字".into()));
            paint(&mut app);
        }
        eprintln!("{name} 预览中打 10 个字 {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 测量约 1 MB 的文档在预览中执行各步骤后增加的堆内存（MB）和排版缓存的条目数。
#[test]
#[ignore = "测量，手动运行"]
// 按全局分配器（`large_alloc`，仅 macOS）记录的大块内存计算。
#[cfg(target_os = "macos")]
fn measure_markdown_memory() {
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    let section = SAMPLE.replace("logo.svg", "missing.svg");
    let text: String = (0..900)
        .map(|i| format!("## 第 {i} 节\n\n{section}\n"))
        .collect();
    let dir = sample_folder("md-memory");
    let path = dir.join("BIG.md");
    std::fs::write(&path, &text).unwrap();
    std::fs::write(dir.join("other.rs"), "fn main() {}\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let report = |app: &App, step: &str, base: isize| {
        let memo = &app.preview.markdown.memo;
        eprintln!(
            "{step:<16} {:>7.1} MB  缓存 {} 条",
            mb(
                (crate::large_alloc::LIVE.load(std::sync::atomic::Ordering::Relaxed) - base).max(0)
                    as u64
            ),
            memo.len()
        );
    };
    let base = crate::large_alloc::LIVE.load(std::sync::atomic::Ordering::Relaxed);
    report(&app, "打开源代码", base);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    report(&app, "只看预览", base);
    if std::env::var_os("WINC_MD_BREAKDOWN").is_some() {
        let laid = app.preview.markdown.layout.take();
        drop(laid);
        report(&app, "去掉排版结果", base);
        app.preview.markdown.memo = Default::default();
        report(&app, "去掉排版缓存", base);
        heap_breakdown(app.preview.markdown.parsed.as_ref().unwrap());
        app.preview.markdown.parsed = None;
        report(&app, "去掉解析结果", base);
        return;
    }
    // 从开头滚动到末尾。
    let height = app.preview.markdown.height;
    let mut y = 0.0;
    while y < height {
        app.preview.markdown.scroll_to(y);
        paint(&mut app);
        y += app.preview.markdown.area.h;
    }
    report(&app, "滚到底", base);
    // 在中间的段落中输入 300 个字符，每个字符绘制一帧并等待一次解析。
    let at = text.len() / 2;
    let at = text[at..].find("轻量").map_or(at, |i| at + i);
    let doc = app.editor.active_mut().unwrap();
    let pos = doc.text().rope().unwrap().byte_to_char(at);
    doc.selection = winc_core::Selection::point(pos);
    for _ in 0..300 {
        app.run(winc_editor::Command::Insert("字".into()));
        app.wait_for_markdown();
        paint(&mut app);
    }
    report(&app, "打 300 个字", base);
    app.set_preview_mode(PreviewMode::Source);
    settle(&mut app);
    report(&app, "换回源代码", base);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    app.open_path(&dir.join("other.rs"));
    settle(&mut app);
    report(&app, "换到别的标签", base);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 计时：用约 1 MB 的文档测量首次排版与修改一个字符后重排的耗时。
#[test]
#[ignore = "计时，手动运行"]
fn time_large_markdown() {
    let section = SAMPLE.replace("logo.svg", "missing.svg");
    let text: String = (0..900)
        .map(|i| format!("## 第 {i} 节\n\n{section}\n"))
        .collect();
    eprintln!("{} KB", text.len() / 1024);
    let dir = sample_folder("md-large");
    let path = dir.join("BIG.md");
    std::fs::write(&path, &text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    let started = std::time::Instant::now();
    app.wait_for_markdown();
    eprintln!("解析 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("第一次排版加两帧 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("再画两帧 {:?}", started.elapsed());
    settle(&mut app);
    app.run(winc_editor::Command::Insert("字".into()));
    app.wait_for_markdown();
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("改一个字后重排加两帧 {:?}", started.elapsed());
    // 一张图片读取完成：只重排用到该图片的块。
    app.preview.markdown.loaded.push(dir.join("other.png"));
    let started = std::time::Instant::now();
    paint(&mut app);
    eprintln!("读好一张图片后重排加两帧 {:?}", started.elapsed());
    // 分栏时滚动左侧文字（右侧随之滚动）、滚动右侧预览（左侧随之滚动），每次一帧。
    let started = std::time::Instant::now();
    for i in 0..100 {
        app.editor.active_mut().unwrap().scroll_line = (i * 37) as f64;
        paint(&mut app);
    }
    eprintln!("滚左边 100 帧 {:?}", started.elapsed());
    let started = std::time::Instant::now();
    for _ in 0..100 {
        app.scroll_markdown(-300.0);
        paint(&mut app);
    }
    eprintln!("滚右边 100 帧 {:?}", started.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：可编辑的预览中光标位于粗体和链接中，只显示光标接触到的标记。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_reveal() {
    let (mut app, dir) = readme("md-reveal-shot", false, PreviewMode::Preview);
    for (needle, name) in [
        ("Rust", "bold"),
        ("GitHub", "link"),
        ("winc 编辑器", "heading"),
        ("cargo xtask", "code"),
    ] {
        put_caret(&mut app, needle, false);
        app.run(winc_editor::Command::MoveRight {
            extend: false,
            word: false,
        });
        app.run(winc_editor::Command::MoveLeft {
            extend: false,
            word: false,
        });
        settle(&mut app);
        shoot(&mut app, &format!("md-reveal-{name}"));
    }
    // 标题中的输入法组字使用标题的字号和粗体。
    put_caret(&mut app, "编辑器", false);
    app.ime.preedit = "中文".into();
    shoot(&mut app, "md-reveal-preedit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：可编辑的预览中光标分别位于代码、mermaid、公式、HTML、图片、表格中。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_markdown_blocks() {
    let text = "# 块\n\n```rust\nfn main() {}\n```\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n\
                $$\nx^2 + y^2\n$$\n\n<div align=\"center\">\n<b>粗</b> 和 <i>斜</i>\n</div>\n\n\
                ![标志](logo.svg)\n\n| 名 | 值 |\n|---|---|\n| **甲** | `1` |\n| 乙 | 2 |\n\n尾\n";
    let (mut app, dir) = live_doc("md-blocks-shot", text);
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    settle(&mut app);
    for (needle, name) in [
        ("fn main", "code"),
        ("```rust", "code-fence"),
        ("A -->", "mermaid"),
        ("x^2", "math"),
        ("<b>", "html"),
        ("标志", "image"),
        ("甲", "table"),
    ] {
        put_caret(&mut app, needle, false);
        shoot(&mut app, &format!("md-block-{name}"));
    }
    // 删除表格的一行，过渡进行到一半：后文正在向上滑动。
    app.preview.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 3);
    put_caret(&mut app, "乙", false);
    app.live_table_tool("delete-row");
    settle(&mut app);
    shoot(&mut app, "md-block-table-motion");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn markdown_images_render_one_size_at_a_time() {
    use crate::app::preview::markdown::ImageSlot;

    let mut app = blank_app(false);
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
    let picture = winc_preview::svg::load(svg, None, None).unwrap();
    let path = PathBuf::from("/tmp/winc-shot.svg");
    let slot = ImageSlot::Ready {
        width: picture.width,
        height: picture.height,
        picture: Some(std::sync::Arc::new(picture)),
        pixels: Vec::new(),
        drawing: Vec::new(),
    };
    app.preview.markdown.images.insert(path.clone(), slot);
    // 拖动改变窗口大小：每帧需要一个新尺寸，正在绘制时不再提交，只有第一个在后台绘制。
    for width in 100..110 {
        app.request_markdown_pixels(&path, (width, 50));
    }
    let Some(ImageSlot::Ready { drawing, .. }) = app.preview.markdown.images.get(&path) else {
        panic!("图片不在");
    };
    assert_eq!(drawing, &[(100, 50)]);
}

/// 只看预览时左侧有标题指示条，每个标题一条；鼠标停在指示条上时标题列表在原处展开，点击一项
/// 滚动到该标题。分栏时预览变窄，左侧留白容纳不下指示条，不绘制。
#[test]
fn the_preview_shows_a_heading_indicator() {
    use crate::app::preview::markdown::TocTarget;

    let (mut app, dir) = readme("md-toc", false, PreviewMode::Preview);
    assert_eq!(app.hits.markdown_toc_tops.len(), 2);
    let find = |app: &App, target| {
        app.hits
            .markdown_toc
            .iter()
            .find(|(_, t)| *t == target)
            .map(|h| h.0)
    };
    let bars = find(&app, TocTarget::Bars).unwrap();
    assert!(find(&app, TocTarget::Heading(0)).is_none());

    app.on_mouse_move(bars.x + 20.0, bars.y + 10.0);
    paint(&mut app);
    assert!(matches!(app.pointer.hover, Hover::MarkdownToc(_)));
    // 展开时第一行的横线落在指示条第一条的位置上（尚未滚动到任何标题），横线不跳动。指示条的
    // 范围比第一条高出 8，第一条粗 2。
    let first = find(&app, TocTarget::Heading(0)).unwrap();
    assert_eq!(first.y + first.h / 2.0, bars.y + 8.0 + 1.0);
    // 列表覆盖指示条展开；鼠标停在列表里两行之间的空白处时列表不收起。
    let row = find(&app, TocTarget::Heading(1)).unwrap();
    let card = app.hits.markdown_toc.iter().rev().nth(1).unwrap().0;
    assert!(card.contains(bars.x + 20.0, bars.y + 10.0));
    app.on_mouse_move(card.x + 1.0, row.y);
    paint(&mut app);
    assert!(find(&app, TocTarget::Heading(1)).is_some());

    click(&mut app, (row.x + 20.0, row.y + row.h / 2.0));
    let max = (app.preview.markdown.height - app.preview.markdown.area.h).max(0.0);
    assert_eq!(
        app.preview.markdown.scroll(),
        app.hits.markdown_toc_tops[1].min(max)
    );
    paint(&mut app);
    assert_eq!(app.preview.markdown.toc_current, Some(1));

    // 指示条贴在正文左边，随正文居中的位置移动。
    let left = app.preview.markdown.layout.as_ref().unwrap().left;
    // 列表展开着时卡片也记作 `Bars`，指示条本身总是最后一项。
    let bars = app.hits.markdown_toc.last().unwrap().0;
    assert!(
        bars.right() <= left && bars.right() >= left - 24.0,
        "{bars:?} {left}"
    );

    // 点击开头的标题后滚动到底：文末的标题无法滚动到顶部，当前所在的一节取屏幕里最后一个标题。
    app.press_toc(TocTarget::Heading(0));
    paint(&mut app);
    assert_eq!(app.preview.markdown.toc_current, Some(0));
    app.scroll_markdown_to(max);
    paint(&mut app);
    assert_eq!(app.preview.markdown.toc_current, Some(1));

    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert!(app.hits.markdown_toc.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：本仓库的 README 只看预览，滚动到第三个标题，鼠标停在标题指示条上；点击最后一个
/// 标题后正文中的底色。深浅两套配色。
#[test]
#[ignore]
fn shoot_preview_heading_indicator() {
    use crate::app::preview::markdown::TocTarget;

    for dark in [false, true] {
        let dir = sample_folder(if dark { "md-toc-dark" } else { "md-toc-light" });
        let path = dir.join("README.md");
        std::fs::write(&path, include_str!("../../../../../../README.md")).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(PreviewMode::Preview);
        settle(&mut app);
        let top = app.hits.markdown_toc_tops[2];
        app.scroll_markdown_to(top);
        paint(&mut app);
        shoot(&mut app, if dark { "md-toc-dark" } else { "md-toc-light" });
        let bars = app
            .hits
            .markdown_toc
            .iter()
            .find(|(_, t)| *t == TocTarget::Bars)
            .unwrap()
            .0;
        app.on_mouse_move(bars.x + 20.0, bars.y + 10.0);
        paint(&mut app);
        let open = if dark {
            "md-toc-open-dark"
        } else {
            "md-toc-open-light"
        };
        shoot(&mut app, open);
        // 展开到一半：横线正从指示条滑向各行，卡片和文字淡入到一半。
        let at = crate::app::animation::TOC_FADE;
        app.preview.markdown.toc_fade = Some(crate::app::animation::Clock::frozen(at / 2, at));
        let half = if dark {
            "md-toc-opening-dark"
        } else {
            "md-toc-opening-light"
        };
        shoot(&mut app, half);
        app.preview.markdown.toc_fade = None;
        // 点击最后一个标题：滚动到底，它成为当前所在的一节，正文里的底色淡出到三成。
        let last = app.hits.markdown_toc_tops.len() - 1;
        app.press_toc(TocTarget::Heading(last));
        let at = crate::app::animation::TOC_FLASH;
        let clock = crate::app::animation::Clock::frozen(at * 3 / 10, at);
        app.preview.markdown.toc_flash = Some((last, clock));
        app.on_mouse_move(900.0, 400.0);
        paint(&mut app);
        let picked = if dark {
            "md-toc-picked-dark"
        } else {
            "md-toc-picked-light"
        };
        shoot(&mut app, picked);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// ←→ 不停留在预览中不显示的列表记号、引用标记里：从项的开头向左到上一行的末尾，向右到
/// 下一块内容的开头。
#[test]
fn arrows_skip_the_hidden_block_markers() {
    use winit::keyboard::{Key, NamedKey};
    let left = Key::Named(NamedKey::ArrowLeft);
    let right = Key::Named(NamedKey::ArrowRight);
    let (mut app, dir) = live_doc("md-hidden-markers", "- 一\n- 二\n  - 三\n\n> 引\n");
    for (needle, before, after) in [
        ("二", "- 一|\n- 二\n", "- 一\n- |二\n"),
        ("三", "- 二|\n  - 三\n", "- 二\n  - |三\n"),
        ("引", "  - 三|\n\n> 引\n", "  - 三\n\n> |引\n"),
    ] {
        put_caret(&mut app, needle, false);
        assert!(app.markdown_key(&left, false), "{needle}");
        settle(&mut app);
        assert!(with_caret(&app).contains(before), "{}", with_caret(&app));
        assert!(app.markdown_key(&right, false), "{needle}");
        settle(&mut app);
        assert!(with_caret(&app).contains(after), "{}", with_caret(&app));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 块的开头按删除键、块的末尾按向前删除键：隔着空行的两段正文合并为一段；上一块是代码块时
/// 不合并，光标移到代码的末尾。
#[test]
fn deleting_at_a_block_edge_joins_the_neighbouring_block() {
    use winit::keyboard::{Key, NamedKey};
    let text = "甲\n\n乙\n\n```\nc\n```\n\n丙\n";
    let (mut app, dir) = live_doc("md-join-blocks", text);
    put_caret(&mut app, "乙", false);
    assert!(app.markdown_key(&Key::Named(NamedKey::Backspace), false));
    settle(&mut app);
    assert!(
        with_caret(&app).starts_with("甲|乙\n\n```"),
        "{}",
        with_caret(&app)
    );
    app.run(winc_editor::Command::Undo);
    assert_eq!(doc_text(&app), text);
    put_caret(&mut app, "甲", true);
    assert!(app.markdown_key(&Key::Named(NamedKey::Delete), false));
    settle(&mut app);
    assert!(
        with_caret(&app).starts_with("甲|乙\n\n```"),
        "{}",
        with_caret(&app)
    );
    app.run(winc_editor::Command::Undo);
    put_caret(&mut app, "丙", false);
    assert!(app.markdown_key(&Key::Named(NamedKey::Backspace), false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "甲\n\n乙\n\n```\nc|\n```\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 列表记号、标题和引用的标记之后输入空格才生效：只输入 `-`、`#`、`>` 时按文字显示。
#[test]
fn block_markers_take_effect_after_a_space() {
    let (mut app, dir) = live_doc("md-bare-markers", "前文\n\n\n\n后文\n");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(4);
    settle(&mut app);
    let shown = |app: &App| crate::app::preview::markdown::laid::full_text(app).unwrap();
    for marker in ["-", "1.", "#", ">"] {
        app.run(winc_editor::Command::Insert(marker.into()));
        paint(&mut app);
        assert!(
            shown(&app).contains(&format!("前文\n{marker}\n")),
            "{}",
            shown(&app)
        );
        let y = caret_screen_y(&app);
        app.run(winc_editor::Command::Insert(" ".into()));
        paint(&mut app);
        // 成为列表项、引用之后记号不再按文字显示，光标仍在同一行上；标题在光标位于开头时
        // 显示 `#`，字号随之变大。
        if marker != "#" {
            assert!(!shown(&app).contains(marker), "{marker}：{}", shown(&app));
            assert!((caret_screen_y(&app) - y).abs() < 4.0, "{marker}");
        } else {
            let parsed = app.preview.markdown.parsed.clone().unwrap();
            let heading = |b: &winc_preview::markdown::Block| {
                matches!(b, winc_preview::markdown::Block::Heading(1, _))
            };
            assert!(parsed.blocks.iter().any(heading));
        }
        app.run(winc_editor::Command::Undo);
        app.run(winc_editor::Command::Undo);
        settle(&mut app);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Tab、⇧Tab 调整列表项的层级时，子项和续行随该项移动。
#[test]
fn tab_moves_a_list_item_with_its_children() {
    use winit::keyboard::{Key, NamedKey};
    let tab = Key::Named(NamedKey::Tab);
    let text = "- 一\n- 二\n  - 子\n\n    续行\n- 三\n";
    let (mut app, dir) = live_doc("md-tab-subtree", text);
    put_caret(&mut app, "二", true);
    assert!(app.markdown_key(&tab, false));
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "- 一\n  - 二|\n    - 子\n\n      续行\n- 三\n"
    );
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&tab, false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    settle(&mut app);
    assert_eq!(with_caret(&app), text.replace("二", "二|"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 代码块的新建与删除：输入第三个反引号即补上闭围栏；回车进入代码块；空的代码块按一次删除键
/// 整块删除；有内容时在开围栏上删除只移除两行围栏；⌘↩ 在代码块之后另起一段。
#[test]
fn code_blocks_are_created_and_removed_as_a_whole() {
    use winit::keyboard::{Key, NamedKey};
    let tick = Key::Character("`".into());
    let back = Key::Named(NamedKey::Backspace);
    let (mut app, dir) = live_doc("md-code-whole", "前文\n\n\n\n后文\n");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(4);
    settle(&mut app);
    for _ in 0..2 {
        assert!(!app.markdown_key(&tick, false));
        app.run(winc_editor::Command::Insert("`".into()));
        paint(&mut app);
    }
    assert!(app.markdown_key(&tick, false));
    paint(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n```|\n```\n\n后文\n");
    // 后文没有成为代码。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(
        shown.ends_with("后文\n") || shown.ends_with("后文"),
        "{shown}"
    );
    assert!(!app.markdown_key(&Key::Named(NamedKey::Enter), false));
    app.run(winc_editor::Command::Newline);
    settle(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n```\n|\n```\n\n后文\n");
    // 有内容：在内容开头按删除键不删除；⌘↩ 到代码块之后。
    app.run(winc_editor::Command::Insert("x".into()));
    app.run(winc_editor::Command::MoveLeft {
        word: false,
        extend: false,
    });
    settle(&mut app);
    assert!(app.markdown_key(&back, false));
    assert_eq!(with_caret(&app), "前文\n\n```\n|x\n```\n\n后文\n");
    app.pointer.modifiers = command_modifiers();
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), true));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    settle(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n```\nx\n```\n\n|\n\n后文\n");
    app.run(winc_editor::Command::Undo);
    // 在开围栏上删除：移除两行围栏，保留内容。
    put_caret(&mut app, "```", true);
    assert!(app.markdown_key(&back, false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n|x\n\n后文\n");
    app.run(winc_editor::Command::Undo);
    // 空的代码块：按一次删除键整块删除。
    put_caret(&mut app, "x\n", false);
    app.run(winc_editor::Command::Delete { word: false });
    settle(&mut app);
    assert!(app.markdown_key(&back, false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n|\n\n后文\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文档以代码块结束时再按 ↓：在其后另起一段，光标移到该段。
#[test]
fn down_after_a_trailing_code_block_adds_a_paragraph() {
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = live_doc("md-trailing-code", "前文\n\n```\nx\n```");
    put_caret(&mut app, "x", true);
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "前文\n\n```\nx\n```\n\n|");
    let _ = std::fs::remove_dir_all(&dir);
}
