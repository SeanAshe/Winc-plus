//! PDF 预览的测试：翻页、缩放、查找和位置的记录。

use super::*;

#[test]
fn clicking_table_cells_once_keeps_the_page_still() {
    let filler = "前文\n\n".repeat(40);
    let (mut app, dir) = live_doc(
        "md-table-click",
        &format!("{filler}| a | b |\n|---|---|\n| c |  |\n|  |  |\n\n{filler}"),
    );
    // 将表格滚动到屏幕中部。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let a = shown.chars().position(|s| s == 'a').unwrap();
    let top = crate::app::preview::markdown::laid::offset_top(&app, a).unwrap();
    app.preview
        .markdown
        .scroll_to(top - app.preview.markdown.area.h / 2.0);
    settle(&mut app);
    let at = |c: char| shown.chars().position(|s| s == c).unwrap();
    let (a, b, c) = (
        point_of(&app, at('a')),
        point_of(&app, at('b')),
        point_of(&app, at('c')),
    );
    let row = c.1 - a.1;
    app.preview.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 2);
    let table = |caret: &str| format!("{filler}{caret}\n\n{filler}");
    // 模拟实际点击：按住鼠标期间绘制数帧并小幅移动指针，松开后再绘制。
    // 单击一次即进入该单元格，页面不滚动。
    let scroll = app.preview.markdown.scroll();
    let mut click = |(x, y): (f32, f32), want: &str| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        paint(&mut app);
        app.on_mouse_move(x + 1.0, y + 1.0);
        paint(&mut app);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        assert_eq!(with_caret(&app), table(want));
        assert_eq!(app.preview.markdown.scroll(), scroll, "{want}");
    };
    click(
        (b.0 + 8.0, c.1 + 4.0),
        "| a | b |\n|---|---|\n| c | | |\n|  |  |",
    );
    click(
        (c.0 + 8.0, c.1 + row + 4.0),
        "| a | b |\n|---|---|\n| c |  |\n| | |  |",
    );
    click(
        (c.0 + 1.0, c.1 + 4.0),
        "| a | b |\n|---|---|\n| |c |  |\n|  |  |",
    );
    click(
        (b.0 + 8.0, c.1 + row + 4.0),
        "| a | b |\n|---|---|\n| c |  |\n|  | | |",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_edits_do_not_scroll_the_page() {
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    let filler = "前文\n\n".repeat(60);
    let (mut app, dir) = live_doc(
        "md-table-still",
        &format!("{filler}| a | b |\n|---|---|\n| c | d |\n\n{filler}"),
    );
    put_caret(&mut app, "c", true);
    // 将表格滚动到屏幕中部：光标不会移出可见区域，页面无需随之滚动。
    let view = app.preview.markdown.area.h;
    app.preview
        .markdown
        .scroll_to(app.preview.markdown.scroll() + view / 2.0);
    settle(&mut app);
    let scroll = app.preview.markdown.scroll();
    assert!(scroll > view, "表格在第一屏以下");
    // 插入行、删除行、删除表格都是文字修改：上方内容不移动，页面不滚动。
    let key = |app: &mut App, mods: ModifiersState, key: NamedKey| {
        app.pointer.modifiers = mods;
        assert!(app.markdown_key(&Key::Named(key), true));
        app.pointer.modifiers = ModifiersState::empty();
        // 先绘制一帧（解析结果尚未返回），再等待解析完成。
        paint(app);
        settle(app);
    };
    key(&mut app, command_modifiers(), NamedKey::Enter);
    assert!(doc_text(&app).contains("|  |  |"));
    assert_eq!(app.preview.markdown.scroll(), scroll, "加行");
    let shift = command_modifiers() | ModifiersState::SHIFT;
    key(&mut app, shift, NamedKey::Backspace);
    assert_eq!(app.preview.markdown.scroll(), scroll, "删行");
    assert!(app.live_table_tool("delete"));
    paint(&mut app);
    settle(&mut app);
    assert!(!doc_text(&app).contains("| c |"));
    assert_eq!(app.preview.markdown.scroll(), scroll, "删表格");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wide_diagrams_shrink_to_fit_the_page() {
    let (mut app, dir) = live_doc(
        "md-mermaid-wide",
        &format!("前文\n\n{WIDE_DIAGRAM}\n后文\n"),
    );
    // 只显示预览和分栏（较窄）时都完整显示整张图：缩小字号重排，左侧的空白不计入宽度。
    for mode in [PreviewMode::Preview, PreviewMode::Split] {
        app.set_preview_mode(mode);
        settle(&mut app);
        let over = crate::view::preview::markdown::tests::overflow(&app);
        assert!(over <= 0.5, "{mode:?} 超出 {over}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// PDF 打开后逐页显示：只渲染视口附近的页，缓存中只保留这几页；滚动到结尾后，前面的页从缓存中
/// 移除。文件损坏时显示原因，以及「重试」「用默认应用打开」两个按钮。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn pdf_files_render_only_the_pages_in_view() {
    use winit::keyboard::{Key, NamedKey};

    let dir = sample_folder("pdf-pages");
    let path = dir.join("手册.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.unsupported, Some(winc_editor::Unsupported::Pdf));
    assert!(doc.readonly);
    assert_eq!(app.preview.pdf.area, app.layout.editor);
    assert_eq!(
        app.preview
            .pdf
            .pages
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .len(),
        30
    );
    let first: Vec<usize> = app.preview.pdf.rendered.keys().copied().collect();
    assert!(first.contains(&0), "{first:?}");
    assert!(first.len() <= 4, "{first:?}");

    assert!(app.pdf_key(&Key::Named(NamedKey::End)));
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(app.preview.pdf.scroll, app.preview.pdf.max_scroll());
    let last: Vec<usize> = app.preview.pdf.rendered.keys().copied().collect();
    assert!(last.contains(&29) && !last.contains(&0), "{last:?}");
    assert!(last.len() <= 4, "{last:?}");

    // 回归：页面滚出预览顶部时，超出预览区域的部分（包括边框）不绘制到标签栏上。
    let above = |app: &mut App| {
        paint(app);
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
        let scale = app.frame.renderer.as_ref().unwrap().scale_factor();
        let rows = (app.preview.pdf.area.y * scale).floor() as usize;
        image.pixels[..rows * image.width as usize].to_vec()
    };
    app.preview.pdf.scroll = 0.0;
    let before = above(&mut app);
    app.preview.pdf.scroll = 300.0;
    assert_eq!(above(&mut app), before);

    let broken = dir.join("损坏.pdf");
    std::fs::write(&broken, b"not a pdf").unwrap();
    app.open_path(&broken);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(
        app.preview
            .pdf
            .pages
            .as_ref()
            .unwrap()
            .as_ref()
            .err()
            .map(String::as_str),
        Some(winc_i18n::t!("preview-pdf-invalid"))
    );
    assert_eq!(app.hits.placeholder.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// PDF 预览有可以拖动的滚动条：滚动后显示，按住滑块拖到底即滚到结尾，途中不选中任何内容。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn the_pdf_preview_scrolls_by_its_scrollbar() {
    use crate::layout::{Scrollbar, scrollbar_thumb};

    let dir = sample_folder("pdf-scrollbar");
    let path = dir.join("长文档.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let area = app.preview.pdf.area;
    assert!(app.preview.pdf.max_scroll() > area.h);

    // 滚动后滚动条显示。
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    assert_eq!(app.scrollbar_shown(ScrollArea::Pdf), 0.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Pdf), 1.0);

    // 按住滑块拖到底：滚到结尾；拖回顶部：回到开头。
    let thumb = scrollbar_thumb(area, app.preview.pdf.scroll, app.preview.pdf.height).unwrap();
    app.on_mouse_move(area.right() - 3.0, thumb.y + thumb.h / 2.0);
    assert!(matches!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::Pdf)
    ));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(area.right() - 3.0, area.bottom() + 50.0);
    assert_eq!(app.preview.pdf.scroll, app.preview.pdf.max_scroll());
    app.on_mouse_move(area.right() - 3.0, area.y - 50.0);
    assert_eq!(app.preview.pdf.scroll, 0.0);
    app.on_mouse_up(MouseButton::Left);

    let _ = std::fs::remove_dir_all(&dir);
}

/// PDF 预览左侧有缩略图栏：只渲染栏内附近的页，点击缩略图跳到该页，栏随当前页滚动，有自己的
/// 滚动条；右下角工具栏上的开关显示或隐藏它，并记入状态。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn the_pdf_preview_lists_page_thumbnails() {
    use crate::app::preview::pdf::PdfTool;

    let dir = sample_folder("pdf-thumbnails");
    let path = dir.join("长文档.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let (area, strip) = (app.preview.pdf.area, app.preview.pdf.strip);
    assert!(strip.w > 0.0 && strip.h == area.h && strip.x == area.x);
    // 只渲染栏内及其上下各一项的缩略图，不是全部 30 页；正文的页不受影响。
    let listed = app.preview.pdf.thumbs.len();
    assert!((3..30).contains(&listed), "{listed}");
    assert!(app.preview.pdf.thumbs.contains_key(&0) && app.preview.pdf.rendered.contains_key(&0));

    // 点击第 3 页的缩略图：正文滚动到该页，栏内该页成为当前页。
    let (third, to) = app.hits.pdf_thumbs[2];
    assert!(to > 0.0);
    click(&mut app, (third.x + third.w / 2.0, third.y + third.h / 2.0));
    assert_eq!(app.preview.pdf.scroll, to);
    paint(&mut app);
    assert_eq!(app.preview.pdf.current, 2);

    // 在栏上滚动只滚动栏，正文不动；在正文上滚动到结尾后，栏跟到最后一页。
    app.on_mouse_move(strip.x + strip.w / 2.0, strip.y + strip.h / 2.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -400.0,
    )));
    assert!(app.preview.pdf.strip_scroll > 0.0);
    assert_eq!(app.preview.pdf.scroll, to);
    app.scroll_pdf(-f32::MAX);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(app.preview.pdf.current, 29);
    assert!(app.preview.pdf.thumbs.contains_key(&29) && !app.preview.pdf.thumbs.contains_key(&0));
    assert!(
        app.hits
            .pdf_thumbs
            .iter()
            .any(|(rect, _)| rect.bottom() >= strip.bottom() - 40.0)
    );

    // 栏内有滚动条：按住滑块拖到底，栏滚到最后；正文不动。
    let scroll = app.preview.pdf.scroll;
    let strip = app.preview.pdf.strip;
    let (at, content) = (app.preview.pdf.strip_scroll, app.preview.pdf.strip_height);
    let thumb = crate::layout::scrollbar_thumb(strip, at, content).unwrap();
    app.on_mouse_move(strip.right() - 3.0, thumb.y + thumb.h / 2.0);
    assert!(matches!(
        app.pointer.hover,
        Hover::Scrollbar(crate::layout::Scrollbar::PdfStrip)
    ));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(strip.right() - 3.0, strip.y - 50.0);
    assert_eq!(app.preview.pdf.strip_scroll, 0.0);
    app.on_mouse_move(strip.right() - 3.0, strip.bottom() + 50.0);
    assert_eq!(app.preview.pdf.strip_scroll, content - strip.h);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.preview.pdf.scroll, scroll);

    // 点击工具栏上的开关：隐藏缩略图栏，正文占满预览区域，缩略图释放；再点一次恢复。
    let tool = |app: &App, tool: PdfTool| {
        let (rect, _) = *app.hits.pdf_tools.iter().find(|(_, t)| *t == tool).unwrap();
        (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    };
    let press = |app: &mut App, which: PdfTool| {
        let at = tool(app, which);
        click(app, at);
    };
    let toggle = tool(&app, PdfTool::Thumbnails);
    app.on_mouse_move(toggle.0, toggle.1);
    assert_eq!(app.pointer.hover, Hover::PdfTool(PdfTool::Thumbnails));
    click(&mut app, toggle);
    assert!(app.state.pdf_thumbnails_hidden);
    paint(&mut app);
    assert_eq!(app.preview.pdf.strip.w, 0.0);
    assert!(app.preview.pdf.thumbs.is_empty() && app.hits.pdf_thumbs.is_empty());
    press(&mut app, PdfTool::Thumbnails);
    paint(&mut app);
    assert!(!app.state.pdf_thumbnails_hidden && app.preview.pdf.strip.w > 0.0);

    // 工具栏上的缩放按钮：放大、缩小各一档，点击倍数还原。
    press(&mut app, PdfTool::ZoomIn);
    assert_eq!(app.preview.pdf.zoom, 1.25);
    paint(&mut app);
    press(&mut app, PdfTool::ZoomOut);
    press(&mut app, PdfTool::ZoomOut);
    assert_eq!(app.preview.pdf.zoom, 0.8);
    paint(&mut app);
    press(&mut app, PdfTool::ZoomReset);
    assert_eq!(app.preview.pdf.zoom, 1.0);

    // 只有一页的文档没有缩略图栏，工具栏上也没有它的开关。
    let single = dir.join("单页.pdf");
    std::fs::write(&single, sample_pdf(1)).unwrap();
    app.open_path(&single);
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(app.preview.pdf.strip.w, 0.0);
    assert!(
        app.hits
            .pdf_tools
            .iter()
            .all(|(_, t)| *t != PdfTool::Thumbnails)
    );
    assert_eq!(app.hits.pdf_tools.len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

/// PDF 可以缩放：快捷键和主修饰键加滚轮各缩放一档，鼠标处的内容保持在原位；放大后可以横向
/// 滚动，位图过大的倍数不可选。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn the_pdf_preview_zooms_around_the_pointer() {
    use crate::keymap::Action;

    let dir = sample_folder("pdf-zoom");
    let path = dir.join("长文档.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let settle = |app: &mut App| {
        for _ in 0..3 {
            paint(app);
            app.wait_for_pdf();
        }
    };
    let size = app.config.editor_size;
    let fit = app.preview.pdf.rendered[&0].width;
    assert_eq!((app.preview.pdf.zoom, app.preview.pdf.scroll_x), (1.0, 0.0));

    // 字号的快捷键在 PDF 上缩放 PDF，不改字号；还原回到适应宽度。
    app.run_action(Action::DecreaseFontSize);
    settle(&mut app);
    assert_eq!(app.preview.pdf.zoom, 0.8);
    assert!(app.preview.pdf.rendered[&0].width < fit);
    app.run_action(Action::ResetFontSize);
    assert_eq!((app.preview.pdf.zoom, app.config.editor_size), (1.0, size));

    // 主修饰键加滚轮放大：鼠标处的内容保持在原位，页面比区域宽后可以横向滚动。
    app.scroll_pdf_to(app.preview.pdf.max_scroll() / 2.0);
    settle(&mut app);
    let body = app.preview.pdf.area.x + app.preview.pdf.strip.w;
    let at = (body + 200.0, app.preview.pdf.area.y + 300.0);
    app.on_mouse_move(at.0, at.1);
    let under = |app: &App| {
        let pages = app.preview.pdf.pages.clone().unwrap().unwrap();
        let width = app.preview.pdf.area.w - app.preview.pdf.strip.w;
        let (rects, ..) = crate::app::preview::pdf::page_rects(&pages, width, app.preview.pdf.zoom);
        let (x, y) = (
            app.preview.pdf.scroll_x + at.0 - body,
            app.preview.pdf.scroll + at.1 - app.preview.pdf.area.y,
        );
        let page = rects.iter().rposition(|r| r.y <= y).unwrap();
        let r = rects[page];
        (page, (x - r.x) / r.w, (y - r.y) / r.h)
    };
    let before = under(&app);
    app.pointer.modifiers = if cfg!(target_os = "macos") {
        winit::keyboard::ModifiersState::SUPER
    } else {
        winit::keyboard::ModifiersState::CONTROL
    };
    let notch = MouseScrollDelta::LineDelta(0.0, 1.0);
    for _ in 0..2 {
        app.on_wheel_event(notch, winit::event::TouchPhase::Moved);
    }
    settle(&mut app);
    assert_eq!((app.preview.pdf.zoom, app.config.editor_size), (1.5, size));
    let after = under(&app);
    assert_eq!(after.0, before.0);
    assert!((after.1 - before.1).abs() < 0.01 && (after.2 - before.2).abs() < 0.01);
    assert!(app.preview.pdf.width > app.preview.pdf.area.w - app.preview.pdf.strip.w);
    assert!(app.preview.pdf.scroll_x > 0.0);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    let x = app.preview.pdf.scroll_x;
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        -40.0, 0.0,
    )));
    assert!(app.preview.pdf.scroll_x > x);

    // 放大到最大的一档（或位图大小的上限）后不再变化。
    app.run_action(Action::IncreaseFontSize);
    let most = app.preview.pdf.zoom;
    app.run_action(Action::IncreaseFontSize);
    assert_eq!(app.preview.pdf.zoom, most);
    settle(&mut app);
    let px = &app.preview.pdf.rendered.values().next().unwrap();
    assert!(u64::from(px.width) * u64::from(px.height) <= 17_000_000);
    // 缩小到最小的一档后不再变化，内容不超出区域，横向不能滚动。
    for _ in 0..10 {
        app.run_action(Action::DecreaseFontSize);
    }
    settle(&mut app);
    assert_eq!((app.preview.pdf.zoom, app.preview.pdf.scroll_x), (0.5, 0.0));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 缩放和缩略图栏的显示、隐藏都有过渡：缩放期间倍数逐帧变化，锚点处的内容不动，请求的始终是
/// 最终倍数的位图，旧位图留到新的渲染完成；缩略图栏滑入期间只露出一部分。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn the_pdf_preview_animates_zooming_and_the_thumbnail_strip() {
    use crate::app::animation::PDF_MOTION;
    use crate::app::preview::pdf::{ZoomMotion, page_rects};

    let dir = sample_folder("pdf-motion");
    let path = dir.join("长文档.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    let pages = app.preview.pdf.pages.clone().unwrap().unwrap();
    let body = app.preview.pdf.area.w - app.preview.pdf.strip.w;
    let fit = app.preview.pdf.rendered[&0].width;

    // 放大到 1.5 倍，过渡走到一半：页面介于两种大小之间，第 1 页的指定位置停在锚点上。
    let anchor = (body / 2.0, 200.0);
    app.preview.pdf.zoom = 1.5;
    app.preview.pdf.zoom_motion = Some(ZoomMotion {
        from: 1.0,
        clock: Clock::stopped(PDF_MOTION, PDF_MOTION / 2),
        page: 0,
        at: (0.5, 0.25),
        anchor,
    });
    paint(&mut app);
    let (small, _, least) = page_rects(&pages, body, 1.0);
    let (large, _, most) = page_rects(&pages, body, 1.5);
    let zoom = Clock::stopped(PDF_MOTION, PDF_MOTION / 2).between(1.0, 1.5);
    let (now, _, height) = page_rects(&pages, body, zoom);
    assert!(least < height && height < most && small[0].w < now[0].w);
    assert_eq!(app.preview.pdf.height, height);
    let center = now[0].y + now[0].h * 0.25;
    assert!(
        (center - app.preview.pdf.scroll - anchor.1).abs() < 2.0,
        "{center}"
    );
    // 旧位图还在（缩放显示），已请求的是最终倍数的位图。
    assert_eq!(app.preview.pdf.rendered[&0].width, fit);
    app.preview.pdf.zoom_motion = None;
    for _ in 0..3 {
        paint(&mut app);
        app.wait_for_pdf();
    }
    assert_eq!(
        app.preview.pdf.rendered[&0].width,
        (large[0].w * SCALE).round() as u32
    );

    // 缩略图栏滑出到一半：只露出一部分，命中范围限制在露出的部分之内。
    app.preview.pdf.strip_motion = Some((false, Clock::stopped(PDF_MOTION, PDF_MOTION / 4)));
    paint(&mut app);
    let strip = app.preview.pdf.strip;
    assert!(strip.w > 0.0 && strip.w < 136.0, "{strip:?}");
    assert!(
        app.hits
            .pdf_thumbs
            .iter()
            .all(|(rect, _)| rect.right() <= strip.right())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归（#105）：某一页渲染失败时在页面中央显示原因，不再空白；失败的页不在每一帧重新请求。
#[cfg(any(target_os = "macos", windows))]
#[test]
fn a_page_that_fails_to_render_shows_why_instead_of_staying_blank() {
    let dir = sample_folder("pdf-failure");
    let path = dir.join("sample.pdf");
    std::fs::write(&path, sample_pdf(2)).unwrap();
    let mut app = open_pdf(&path, false);
    let width = app.preview.pdf.rendered[&0].width;
    let height = app.preview.pdf.rendered[&0].height;

    // 第一页渲染失败：记下原因，不再请求。
    app.preview.pdf.rendered.remove(&0);
    app.fail_pdf_page(0, width, "无法渲染此页（错误代码 0x80004005）");
    assert!(app.preview.pdf.failed.contains_key(&(0, width)));
    paint(&mut app);
    assert!(!app.preview.pdf.is_busy(), "失败的页被重新请求");
    assert!(!app.preview.pdf.rendered.contains_key(&0));

    // 页面可见部分不再全是白色：在中央画出了失败的原因（页面比视口高时画在可见部分的中央）。
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let scale = app.frame.renderer.as_ref().unwrap().scale_factor();
    let area = app.preview.pdf.area;
    let top = ((area.y + 32.0) * scale) as usize + 2;
    let bottom = ((area.bottom() * scale) as usize).min(top + height as usize) - 2;
    let left = ((area.x + area.w / 2.0) * scale) as usize - width as usize / 2 + 2;
    let row = image.width as usize;
    let white = image.pixels[top * row + left];
    let drawn = (top..bottom)
        .any(|y| (left..left + width as usize - 4).any(|x| image.pixels[y * row + x] != white));
    assert!(drawn, "失败的页仍然是空白");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(any(target_os = "macos", windows))]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_pdf() {
    for dark in [false, true] {
        let dir = sample_folder("pdf-shot");
        let path = dir.join("手册.pdf");
        std::fs::write(&path, sample_pdf(5)).unwrap();
        let mut app = open_pdf(&path, dark);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("pdf-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
