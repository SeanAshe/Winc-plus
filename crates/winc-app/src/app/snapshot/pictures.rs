//! 图片预览的缩放、滚动和拖动的测试。
//!
//! 操作方式与 PDF 预览相同：滚动平移，按住主修饰键滚动、捏合或 `⌘+`、`⌘−` 缩放。

use super::*;
use winit::event::TouchPhase;
use winit::window::CursorIcon;

/// 图片四周的留白（逻辑像素），与 `app::preview::images` 的 `MARGIN` 相同。
const MARGIN: f32 = 24.0;

/// 打开一张 8 × 8 的位图：按原大小显示，远小于预览区域。
fn small_image(name: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("image.jpg");
    std::fs::write(
        &path,
        include_bytes!("../../../../winc-preview/testdata/red-8x8.jpg"),
    )
    .unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_images();
    paint(&mut app);
    (app, dir)
}

/// 一张 1200 × 800 的 SVG 的源码：适应预览区域时已经缩小，放大后超出预览区域。
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 800">
<rect width="1200" height="800" fill="#e8f0fc"/>
<rect x="40" y="40" width="500" height="320" fill="#367df2"/>
<circle cx="870" cy="390" r="220" fill="#ec704a"/>
<path d="M80 700L1120 100M80 100L1120 700" stroke="#19334d" stroke-width="12"/>
</svg>"##;

/// 以只看预览的方式打开 [`SVG`]。
fn large_image(name: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("sample.svg");
    std::fs::write(&path, SVG).unwrap();
    let mut app = app_with(&path, false);
    app.set_preview_mode(winc_editor::PreviewMode::Preview);
    app.wait_for_images();
    paint(&mut app);
    (app, dir)
}

/// 返回图片当前的位置和预览区域。
fn placed(app: &App) -> (Rect, Rect) {
    let (picture, area) = app.shown_picture().expect("正在显示图片");
    (app.picture_view.rect(picture, area).unwrap(), area)
}

/// 触控板滚动 (`dx`, `dy`) 逻辑像素并重绘。
fn scroll(app: &mut App, dx: f32, dy: f32) {
    let delta = PhysicalPosition::new(f64::from(dx * SCALE), f64::from(dy * SCALE));
    app.on_wheel_event(MouseScrollDelta::PixelDelta(delta), TouchPhase::Moved);
    paint(app);
}

/// 按住主修饰键把滚轮滚动一格：放大（`up`）或缩小一档，之后等待 SVG 按新的大小绘制完成。
fn zoom(app: &mut App, up: bool) {
    app.modifiers = command_modifiers();
    let y = if up { 1.0 } else { -1.0 };
    app.on_wheel_event(MouseScrollDelta::LineDelta(0.0, y), TouchPhase::Moved);
    app.modifiers = Default::default();
    app.wait_for_images();
    paint(app);
}

/// 把鼠标移到图片中相对位置 `at`（宽、高的比例）处。
fn point_at(app: &mut App, at: (f32, f32)) {
    let (rect, _) = placed(app);
    app.on_mouse_move(rect.x + rect.w * at.0, rect.y + rect.h * at.1);
}

/// 返回鼠标在图片中的相对位置（宽、高的比例）。
fn pointed(app: &App) -> (f32, f32) {
    let (rect, _) = placed(app);
    (
        (app.mouse.0 - rect.x) / rect.w,
        (app.mouse.1 - rect.y) / rect.h,
    )
}

fn near(a: (f32, f32), b: (f32, f32)) {
    assert!(
        (a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001,
        "{a:?} != {b:?}"
    );
}

#[test]
fn command_wheel_zooms_an_image_around_the_pointer() {
    let (mut app, dir) = large_image("picture-zoom");
    let (fit, area) = placed(&app);
    let font = app.fonts.line_height;
    // 适应预览区域：四周留白，图片居中。
    assert_eq!(fit.w, area.w - MARGIN * 2.0);
    near(
        (fit.x + fit.w / 2.0, fit.y + fit.h / 2.0),
        (area.x + area.w / 2.0, area.y + area.h / 2.0),
    );

    // 在图片中心放大到 2 倍，之后鼠标所指的位置在继续放大时保持不动。
    point_at(&mut app, (0.5, 0.5));
    for _ in 0..3 {
        zoom(&mut app, true);
    }
    near((placed(&app).0.w / fit.w, 0.0), (2.0, 0.0));
    point_at(&mut app, (0.3, 0.7));
    let at = pointed(&app);
    zoom(&mut app, true);
    near((placed(&app).0.w / fit.w, 0.0), (3.0, 0.0));
    near(pointed(&app), at);
    // 触控板按住主修饰键滚动满一格同样缩放一档；状态栏显示倍数。
    app.modifiers = command_modifiers();
    let notch = PhysicalPosition::new(0.0, 48.0 * f64::from(SCALE));
    app.on_wheel_event(MouseScrollDelta::PixelDelta(notch), TouchPhase::Started);
    app.modifiers = Default::default();
    near((placed(&app).0.w / fit.w, 0.0), (4.0, 0.0));
    near(pointed(&app), at);
    assert_eq!(app.status.message.as_deref(), Some("图片缩放 400%"));
    // SVG 的显示大小不超过光栅化的上限（长边 8192 物理像素）：下一档为 6 倍，不可选。
    assert!(fit.w * 6.0 * SCALE > 8192.0);
    zoom(&mut app, true);
    near((placed(&app).0.w / fit.w, 0.0), (4.0, 0.0));

    // `⌘+`、`⌘−`、`⌘0` 缩放图片，捏合满一档缩放一次；字号不变。
    app.step_font_size(None);
    assert_eq!(placed(&app).0, fit);
    app.step_font_size(Some(true));
    near((placed(&app).0.w / fit.w, 0.0), (1.25, 0.0));
    app.step_font_size(Some(false));
    app.step_font_size(Some(false));
    near((placed(&app).0.w / fit.w, 0.0), (0.8, 0.0));
    app.picture_pinch(0.1);
    near((placed(&app).0.w / fit.w, 0.0), (0.8, 0.0));
    app.picture_pinch(0.1);
    assert_eq!(placed(&app).0, fit);
    assert_eq!(app.fonts.line_height, font);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn zoom_stops_at_the_first_and_last_steps() {
    let (mut app, dir) = small_image("picture-zoom-limits");
    let (fit, area) = placed(&app);
    assert_eq!((fit.w, fit.h), (8.0, 8.0));
    point_at(&mut app, (0.5, 0.5));
    for _ in 0..20 {
        zoom(&mut app, true);
    }
    // 最大 32 倍；小图放大后仍不超出预览区域，保持居中，滚动和拖动都不改变位置。
    let (rect, _) = placed(&app);
    assert_eq!((rect.w, rect.h), (256.0, 256.0));
    near(
        (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0),
        (area.x + area.w / 2.0, area.y + area.h / 2.0),
    );
    scroll(&mut app, 40.0, 40.0);
    assert_eq!(placed(&app).0, rect);
    assert_eq!(app.cursor_icon, CursorIcon::Default);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.drag, Drag::None);
    app.on_mouse_up(MouseButton::Left);
    // 放大的位图直接取最近的像素绘制，不另存放大后的位图。
    assert!(app.picture_cache.is_none());
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let center = (
        ((area.x + area.w / 2.0) * SCALE) as usize,
        ((area.y + area.h / 2.0) * SCALE) as usize,
    );
    let pixel = image.pixels[center.1 * image.width as usize + center.0];
    assert!(
        (pixel >> 16) & 0xff > 150 && pixel & 0xff < 80,
        "{pixel:06x}"
    );

    for _ in 0..20 {
        zoom(&mut app, false);
    }
    assert_eq!((placed(&app).0.w, placed(&app).0.h), (4.0, 4.0));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn scrolling_pans_a_zoomed_image_within_its_content() {
    let (mut app, dir) = large_image("picture-scroll");
    let (fit, area) = placed(&app);
    point_at(&mut app, (0.5, 0.5));
    // 适应预览区域时内容不超出区域，滚动不改变位置。
    scroll(&mut app, 0.0, -60.0);
    assert_eq!(placed(&app).0, fit);
    assert_eq!(app.cursor_icon, CursorIcon::Default);

    for _ in 0..3 {
        zoom(&mut app, true);
    }
    let (rect, _) = placed(&app);
    assert_eq!(app.cursor_icon, CursorIcon::Grab);
    // 滚动只平移，不缩放；与其他内容相同，只按位移较大的方向滚动。
    scroll(&mut app, 0.0, -60.0);
    near(
        (placed(&app).0.x, placed(&app).0.y),
        (rect.x, rect.y - 60.0),
    );
    scroll(&mut app, 50.0, 5.0);
    near(
        (placed(&app).0.x, placed(&app).0.y),
        (rect.x + 50.0, rect.y - 60.0),
    );
    assert_eq!(placed(&app).0.w, rect.w);
    // 滚动到四边时图片的边缘与预览区域的边缘相距留白的宽度。
    scroll(&mut app, 0.0, -100_000.0);
    scroll(&mut app, -100_000.0, 0.0);
    let (rect, _) = placed(&app);
    near(
        (rect.right(), rect.bottom()),
        (area.right() - MARGIN, area.bottom() - MARGIN),
    );
    scroll(&mut app, 0.0, 100_000.0);
    scroll(&mut app, 100_000.0, 0.0);
    let (rect, _) = placed(&app);
    near((rect.x, rect.y), (area.x + MARGIN, area.y + MARGIN));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dragging_pans_a_zoomed_image_and_double_click_restores_fit() {
    let (mut app, dir) = large_image("picture-drag");
    let (fit, area) = placed(&app);
    let center = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_move(center.0, center.1);
    // 图片没有超出预览区域时不能拖动。
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.drag, Drag::None);
    app.on_mouse_up(MouseButton::Left);

    for _ in 0..3 {
        zoom(&mut app, true);
    }
    let (rect, _) = placed(&app);
    app.last_click = None;
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.cursor_icon, CursorIcon::Grabbing);
    app.on_mouse_move(center.0 + 70.0, center.1 - 30.0);
    near(
        (placed(&app).0.x, placed(&app).0.y),
        (rect.x + 70.0, rect.y - 30.0),
    );
    // 拖动中缩放后继续拖动，图片仍随鼠标移动相同的距离。
    zoom(&mut app, true);
    let (rect, _) = placed(&app);
    app.on_mouse_move(app.mouse.0 + 10.0, app.mouse.1 + 20.0);
    near(
        (placed(&app).0.x, placed(&app).0.y),
        (rect.x + 10.0, rect.y + 20.0),
    );
    // 拖过内容的边缘后停在留白处。
    app.on_mouse_move(app.mouse.0 + 100_000.0, app.mouse.1 + 100_000.0);
    near(
        (placed(&app).0.x, placed(&app).0.y),
        (area.x + MARGIN, area.y + MARGIN),
    );
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.drag, Drag::None);
    // 松开后移动鼠标不再平移。
    let (rect, _) = placed(&app);
    app.on_mouse_move(center.0, center.1);
    assert_eq!(placed(&app).0, rect);
    assert_eq!(app.cursor_icon, CursorIcon::Grab);

    // 双击还原为适应预览区域。
    app.last_click = None;
    click(&mut app, center);
    click(&mut app, center);
    assert_eq!(placed(&app).0, fit);
    assert_eq!(app.cursor_icon, CursorIcon::Default);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_view_resets_for_another_document_and_survives_a_reload() {
    let (mut app, dir) = large_image("picture-documents");
    let (fit, area) = placed(&app);
    let center = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_move(center.0, center.1);
    for _ in 0..3 {
        zoom(&mut app, true);
    }
    scroll(&mut app, 0.0, -80.0);
    let (rect, _) = placed(&app);

    // 同一文档的图片重新加载：缩放与滚动位置保留。
    let reloaded = (app.editor.active().unwrap().picture().unwrap())
        .as_ref()
        .clone();
    app.editor.active_mut().unwrap().picture = Some(winc_editor::PictureState::Ready(
        std::sync::Arc::new(reloaded),
    ));
    app.wait_for_images();
    assert_eq!(placed(&app).0, rect);

    // 换到其他标签时结束拖动；回到图片时还原为适应预览区域。
    app.on_mouse_down(MouseButton::Left);
    assert!(matches!(app.drag, Drag::Picture { .. }));
    app.new_file();
    app.advance_images();
    assert_eq!(app.drag, Drag::None);
    assert!(app.shown_picture().is_none());
    // 没有显示图片时 `⌘+` 调整字号。
    assert!(!app.zoom_picture(Some(true), None));
    app.on_mouse_up(MouseButton::Left);
    app.activate_tab(0);
    app.wait_for_images();
    assert_eq!(placed(&app).0, fit);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_zoomed_svg_is_rasterized_only_where_it_is_seen() {
    let (mut app, dir) = large_image("picture-region");
    let (_, area) = placed(&app);
    point_at(&mut app, (0.5, 0.5));
    let before = (app.renderer.as_ref().unwrap().image().unwrap().pixels).clone();
    for _ in 0..5 {
        zoom(&mut app, true);
    }
    // 4 倍时整张图约 7300 × 4900 物理像素；位图只有预览区域加四周多绘制的部分。
    let (rect, _) = placed(&app);
    let cache = app.picture_cache.as_ref().unwrap();
    let sharp = cache.sharp();
    assert_eq!(sharp.size.0, (rect.w * SCALE).round() as u32);
    assert!(sharp.size.0 > 7000, "{:?}", sharp.size);
    // 整张图另有一份低分辨率的位图，长边 1024 像素。
    let base = cache.base.as_ref().unwrap();
    assert_eq!(base.width, 1024);
    let most = |room: f32| ((room + 64.0 * 2.0) * SCALE).ceil() as u32 + 1;
    assert!(cache.width() >= (area.w * SCALE) as u32 && cache.width() <= most(area.w));
    assert!(cache.height() >= (area.h * SCALE) as u32 && cache.height() <= most(area.h));

    // 平移不超过多绘制的宽度时沿用同一份位图，超过后重新光栅化可见的部分。
    let (origin, pixels) = (sharp.origin, sharp.pixels.rgba.as_ptr());
    scroll(&mut app, -40.0, 0.0);
    app.wait_for_images();
    let sharp = app.picture_cache.as_ref().unwrap().sharp();
    assert_eq!((sharp.origin, sharp.pixels.rgba.as_ptr()), (origin, pixels));

    // 超过后先显示低分辨率的位图，不留空白：右端新露出的部分已有图片的颜色。
    scroll(&mut app, -400.0, 0.0);
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let edge = (
        ((area.right() - 8.0) * SCALE) as usize,
        ((area.y + area.h / 2.0) * SCALE) as usize,
    );
    let pixel = image.pixels[edge.1 * image.width as usize + edge.0] & 0xff_ffff;
    assert_eq!(pixel, 0xec704a, "{pixel:06x}");
    // 之后重新光栅化可见的部分。
    app.wait_for_images();
    let sharp = app.picture_cache.as_ref().unwrap().sharp();
    assert_eq!(sharp.origin.0, origin.0 + 440 * SCALE as u32);

    // 图片只绘制在预览区域内：侧栏的像素与放大之前相同（底部的状态栏显示倍数，不比较）。
    let after = app.renderer.as_ref().unwrap().image().unwrap();
    let width = after.width as usize;
    let sidebar = (app.layout.sidebar.w * SCALE) as usize;
    let rows = (TOP_BAR_END * SCALE) as usize..((SIZE.1 - 50.0) * SCALE) as usize;
    for row in rows {
        let range = row * width..row * width + sidebar;
        assert_eq!(before[range.clone()], after.pixels[range], "{row}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_zoom_transition_rasterizes_only_the_final_size() {
    use crate::app::animation::{Clock, PDF_MOTION};
    let (mut app, dir) = large_image("picture-motion");
    let (fit, _) = placed(&app);
    point_at(&mut app, (0.5, 0.5));
    for _ in 0..3 {
        zoom(&mut app, true);
    }
    let two = app.picture_cache.as_ref().unwrap().sharp().size;
    assert_eq!(two.0, (fit.w * 2.0 * SCALE).round() as u32);

    // 从 2 倍放大到 3 倍，过渡进行到一半：这一帧的大小介于两者之间，仍绘制 2 倍时的位图。
    app.step_font_size(Some(true));
    app.replay_picture_zoom(2.0, Clock::stopped(PDF_MOTION, PDF_MOTION / 2));
    paint(&mut app);
    let (shown, _) = placed(&app);
    assert!(
        shown.w > fit.w * 2.0 && shown.w < fit.w * 3.0,
        "{}",
        shown.w
    );
    assert_eq!(app.picture_cache.as_ref().unwrap().sharp().size, two);
    // 后台只按过渡结束后的大小光栅化，不为中间的各帧光栅化。
    app.wait_for_images();
    let three = app.picture_cache.as_ref().unwrap().sharp().size;
    assert_eq!(three.0, (fit.w * 3.0 * SCALE).round() as u32);
    assert_eq!(placed(&app).0, shown);

    app.picture_view.zoom_motion = None;
    paint(&mut app);
    assert_eq!(placed(&app).0.w, fit.w * 3.0);
    assert_eq!(app.picture_cache.as_ref().unwrap().sharp().size, three);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_shrunk_bitmap_is_resampled_once_per_size() {
    let dir = sample_folder("picture-shrink");
    let path = dir.join("wide.png");
    let image = Image {
        pixels: vec![0xff_ffff; 2000 * 4],
        width: 2000,
        height: 4,
    };
    write_png(&path, &image).unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_images();
    paint(&mut app);
    // 2000 像素宽的图片缩小到预览区域内：使用缩小后的位图，大小不超过解码后保留的像素。
    let (rect, _) = placed(&app);
    let cache = app.picture_cache.as_ref().unwrap();
    assert_eq!(cache.width(), (rect.w * SCALE).round() as u32);
    assert!(cache.width() < 2000);
    let pixels = cache.sharp().pixels.rgba.as_ptr();
    paint(&mut app);
    let cache = app.picture_cache.as_ref().unwrap();
    assert_eq!(cache.sharp().pixels.rgba.as_ptr(), pixels);
    // 放大到不小于保留的像素后直接绘制，释放缩小后的位图。
    point_at(&mut app, (0.5, 0.5));
    zoom(&mut app, true);
    assert!(placed(&app).0.w * SCALE >= 2000.0);
    assert!(app.picture_cache.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn svg_split_preview_zooms_without_touching_the_source() {
    use winc_editor::PreviewMode;
    let dir = sample_folder("picture-svg-split");
    let path = dir.join("sample.svg");
    std::fs::write(&path, SVG).unwrap();
    let mut app = app_with(&path, false);
    app.set_preview_mode(PreviewMode::Split);
    app.wait_for_split();
    app.wait_for_images();
    paint(&mut app);
    let (fit, area) = placed(&app);
    assert_eq!(area, app.layout.split);
    point_at(&mut app, (0.5, 0.5));
    let font = app.fonts.line_height;
    let top = app.editor.active().unwrap().scroll_line;
    for _ in 0..3 {
        zoom(&mut app, true);
    }
    point_at(&mut app, (0.35, 0.65));
    let at = pointed(&app);
    zoom(&mut app, true);
    near((placed(&app).0.w / fit.w, 0.0), (3.0, 0.0));
    near(pointed(&app), at);
    scroll(&mut app, 0.0, -30.0);
    assert_eq!(app.fonts.line_height, font);
    assert_eq!(app.editor.active().unwrap().scroll_line, top);

    // 修改文字后右侧的图重新绘制，缩放与滚动位置保留。
    let (rect, _) = placed(&app);
    app.run(winc_editor::Command::Insert(" ".into()));
    app.wait_for_split();
    app.wait_for_images();
    assert_eq!(placed(&app).0, rect);

    // 换回源代码后不再显示图片，滚动回到文字。
    app.set_preview_mode(PreviewMode::Source);
    app.wait_for_images();
    assert!(app.shown_picture().is_none());
    assert!(app.editor.active().unwrap().shows_text());
    std::fs::remove_dir_all(dir).unwrap();
}
