//! 图片与 SVG 预览的测试：缩放、拖动和分栏预览。

use super::*;

#[test]
fn split_sides_scroll_together() {
    // 文档足够长，两侧都能滚动。
    let dir = sample_folder("md-sync");
    let path = dir.join("LONG.md");
    let text: String = (0..60)
        .map(|i| format!("## 第 {i} 节\n\n第 {i} 节的正文。\n\n"))
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    let heading_y = |app: &App, i: usize| {
        crate::app::preview::markdown::laid::anchor_top(app, &format!("第-{i}-节")).unwrap()
    };

    // 左侧滚到第 20 节的标题（第 80 行）：右侧的第 20 节也到达顶部。
    app.editor.active_mut().unwrap().scroll_line = 80.0;
    paint(&mut app);
    // 差值来自预览顶部的留白。
    let pad = app.preview.markdown.scroll() - heading_y(&app, 20);
    assert!((19.0..=33.0).contains(&pad), "{pad}");

    // 右侧滚到第 22 节：左侧同步到第 88 行。
    let target = heading_y(&app, 22) + pad;
    app.scroll_markdown(app.preview.markdown.scroll() - target);
    paint(&mut app);
    let line = app.editor.active().unwrap().scroll_line;
    assert!((line - 88.0).abs() < 0.5, "{line}");
    // 左侧没有再次滚动，右侧也不会被恢复到对齐位置。
    assert!((app.preview.markdown.scroll() - target).abs() < 1.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_an_image_shows_its_source_above_it() {
    let dir = sample_folder("md-image-click");
    std::fs::write(dir.join("logo.svg"), LOGO).unwrap();
    let path = dir.join("LIVE.md");
    std::fs::write(&path, "前\n\n![标志](logo.svg)\n\n后\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    // 点击图片中部：光标进入图片的原文，原文显示在图片上方，图片另起一行。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let (x, top) = point_of(&app, shown.find('前').unwrap());
    let (_, bottom) = point_of(&app, shown.chars().position(|c| c == '后').unwrap());
    app.on_mouse_move(x + 20.0, (top + bottom) / 2.0 + 20.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let at = app.live_caret_source().unwrap();
    let text = doc_text(&app);
    let image = text.find("![").unwrap();
    assert!((image..=text.find("后").unwrap() - 2).contains(&at), "{at}");
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("![标志](logo.svg)\n"), "{shown:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remote_images_come_from_the_cache_and_can_be_turned_off() {
    use crate::app::preview::markdown::{ImageSlot, image_path};
    use crate::layout::Setting;

    // 一个不会联网的地址：先将已下载的文件放入缓存，预览直接读取缓存。
    let url = format!("https://example.invalid/{}/badge", std::process::id());
    let cached = image_path(None, &url, true).unwrap();
    assert_eq!(image_path(None, &url, false), None);
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, LOGO).unwrap();

    let dir = sample_folder("md-remote");
    let path = dir.join("README.md");
    std::fs::write(&path, format!("![徽章]({url})\n")).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    assert!(matches!(
        app.preview.markdown.images.get(&cached),
        Some(ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));
    assert_eq!(app.preview.markdown.layout.as_ref().unwrap().chars(), 0);

    // 关闭联网：显示替代文字。
    app.settings_toggle(Setting::RemoteImages);
    assert_eq!(app.config.markdown_remote_images, Some(false));
    settle(&mut app);
    assert!(app.preview.markdown.images.is_empty());
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert_eq!(laid.text(0, laid.chars()), "徽章\n");

    let _ = std::fs::remove_file(&cached);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_image_cache_drops_old_and_partial_files() {
    use crate::app::preview::markdown::{CACHE_AGE, clean_cache};
    use std::time::{Duration, SystemTime};

    let dir = sample_folder("md-cache");
    std::fs::write(dir.join("used.png"), b"x").unwrap();
    std::fs::write(dir.join("old.png"), b"x").unwrap();
    std::fs::write(dir.join("half.part123"), b"x").unwrap();
    // 将时间推后 2 小时：未下载完的临时文件视为过期，图片仍保留。
    let later = SystemTime::now() + Duration::from_secs(2 * 3600);
    clean_cache(&dir, later);
    assert!(!dir.join("half.part123").exists());
    assert!(dir.join("old.png").exists());
    // 刚用过的文件更新过修改时间，保留；超过 30 天未用的删除。
    let file = std::fs::File::options()
        .write(true)
        .open(dir.join("used.png"))
        .unwrap();
    file.set_modified(later + CACHE_AGE).unwrap();
    clean_cache(&dir, later + CACHE_AGE + Duration::from_secs(60));
    assert!(dir.join("used.png").exists());
    assert!(!dir.join("old.png").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_the_split_divider_resizes_both_halves() {
    use winc_editor::PreviewMode;
    use winit::event::MouseButton;

    let dir = sample_folder("split-divider-drag");
    let path = dir.join("a.md");
    std::fs::write(&path, "# 标题\n\n正文\n").unwrap();
    let mut app = app_with(&path, false);
    app.choose_preview_mode(PreviewMode::Split);
    paint(&mut app);
    let (left, divider) = (app.layout.editor.w, app.layout.split_divider);
    assert!(divider.w > 0.0);
    let total = left + app.layout.split.w + 1.0;

    // 按住分隔线向右拖 100：源代码一侧变宽，预览一侧变窄，总宽度不变。
    let (x, y) = (divider.x + divider.w / 2.0, divider.y + divider.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.pointer.hover, Hover::SplitDivider);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!((app.layout.editor.w - (left + 100.0)).abs() <= 1.0);
    assert_eq!(app.layout.editor.w + app.layout.split.w + 1.0, total);

    // 拖到最左边：源代码一侧停在最小宽度，不再变窄。
    let divider = app.layout.split_divider;
    let x = divider.x + divider.w / 2.0;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(0.0, y);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert_eq!(app.layout.editor.w, 240.0);

    // 双击分隔线：恢复为左右各占一半。
    let divider = app.layout.split_divider;
    let x = divider.x + divider.w / 2.0;
    app.on_mouse_move(x, y);
    for _ in 0..2 {
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
    }
    paint(&mut app);
    assert_eq!(app.preview.split_share, None);
    assert_eq!(app.layout.editor.w, left);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn markdown_keeps_only_the_images_on_screen() {
    use crate::app::preview::markdown::ImageSlot;

    // 24 张 800 × 600 的图，每张解码后 1.9 MB；图与图之间有文字，一屏放不下几张。
    const COUNT: usize = 24;
    let dir = sample_folder("md-many-images");
    let rgba = vec![200u8; 800 * 600 * 4];
    let png = winc_editor::paste::encode_png(800, 600, &rgba).unwrap();
    let mut text = String::new();
    for i in 0..COUNT {
        std::fs::write(dir.join(format!("p{i}.png")), &png).unwrap();
        text.push_str(&format!(
            "第 {i} 张图之前的一段文字。\n\n![图 {i}](p{i}.png)\n\n"
        ));
    }
    let path = dir.join("IMAGES.md");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);

    // 返回（已知宽高的图片数，留有像素的图片，留有解码后图片的张数）。
    let held = |app: &App| {
        let (mut ready, mut drawn, mut decoded) = (0, Vec::new(), 0);
        for (path, slot) in &app.preview.markdown.images {
            if let ImageSlot::Ready {
                picture, pixels, ..
            } = slot
            {
                ready += 1;
                if !pixels.is_empty() {
                    drawn.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
                decoded += usize::from(picture.is_some());
            }
        }
        drawn.sort();
        (ready, drawn, decoded)
    };

    // 每张图的宽高都已知（排版需要），只有屏幕上的几张留有像素，解码后的图片全部释放。
    let (ready, top, decoded) = held(&app);
    assert_eq!(ready, COUNT);
    assert!(!top.is_empty() && top.len() < COUNT / 2, "{top:?}");
    assert!(top.contains(&"p0.png".to_owned()), "{top:?}");
    assert_eq!(decoded, 0);

    // 滚动到末尾：换成末尾的几张，开头的像素已经释放。
    let end = app.preview.markdown.height;
    app.preview.markdown.scroll_to(end);
    settle(&mut app);
    let (_, bottom, decoded) = held(&app);
    assert!(!bottom.is_empty() && bottom.len() < COUNT / 2, "{bottom:?}");
    assert!(
        bottom.contains(&format!("p{}.png", COUNT - 1)),
        "{bottom:?}"
    );
    assert!(!bottom.contains(&"p0.png".to_owned()), "{bottom:?}");
    assert_eq!(decoded, 0);

    // 滚回开头：已经释放的图片重新读取并绘制。
    app.preview.markdown.scroll_to(0.0);
    settle(&mut app);
    assert_eq!(held(&app).1, top);
}
