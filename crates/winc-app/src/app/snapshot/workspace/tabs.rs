//! 标签栏的测试：宽度与横向滚动、拖动调整顺序、多选与关闭，图片和 SVG 的预览标签，以及
//! 网页的「在浏览器中打开」按钮。

use std::time::Instant;

use super::*;

fn names(app: &App) -> Vec<String> {
    app.editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect()
}

/// 返回第 `index` 个标签（从 0 起）内的一点：距标签左边缘 20 逻辑像素，垂直居中。
fn tab_point(app: &App, index: usize) -> (f32, f32) {
    let rect = app.hits.tabs.iter().find(|h| h.index == index).unwrap().tab;
    (rect.x + 20.0, rect.y + rect.h / 2.0)
}

fn tab_rect(app: &App, index: usize) -> Rect {
    app.hits.tabs.iter().find(|h| h.index == index).unwrap().tab
}

#[test]
fn tabs_fit_their_names() {
    let dir = search_folder("tabs-width");
    let long = "a-much-longer-file-name-than-the-others.rs";
    let names = ["a.rs", long, "b.rs"];
    for name in names {
        std::fs::write(dir.join(name), "x\n").unwrap();
    }
    let mut app = app_with(&dir.join(names[0]), false);
    app.open_path(&dir.join(names[1]));
    app.open_path(&dir.join(names[2]));
    paint(&mut app);

    // 宽度贴合文件名：短名字的标签窄，长名字的标签到最大宽度为止；关闭按钮都在标签右端。
    let widths = |app: &App| -> Vec<f32> { app.hits.tabs.iter().map(|h| h.tab.w).collect() };
    let fitted = widths(&app);
    assert!(fitted[0] < fitted[1], "{fitted:?}");
    assert_eq!(fitted[1], TAB_WIDTH_MAX);
    assert!(
        fitted
            .iter()
            .all(|w| (TAB_WIDTH_MIN..=TAB_WIDTH_MAX).contains(w))
    );
    for hit in &app.hits.tabs {
        assert_eq!(hit.close.right(), hit.tab.right() - 6.0);
    }

    // 用鼠标关掉第一个：移过来的长名字标签暂用被关闭标签的宽度，关闭按钮正好在鼠标下面。
    let close = app.hits.tabs.iter().find(|h| h.index == 0).unwrap().close;
    let (x, y) = (close.x + close.w / 2.0, close.y + close.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.documents.len(), names.len() - 1);
    paint(&mut app);
    assert_eq!(widths(&app)[0], fitted[0]);
    assert_eq!(app.tab_at(x, y), Some((0, true)));

    // 鼠标离开顶栏后解除锁定，宽度回到贴合文件名的样子。
    app.on_mouse_move(x, TOP_BAR_END + 100.0);
    paint(&mut app);
    assert_eq!(widths(&app)[0], TAB_WIDTH_MAX);

    // 标签多了放不下：不缩窄，横向滚动。
    for i in 0..8 {
        let name = format!("{i}-{long}");
        std::fs::write(dir.join(&name), "x\n").unwrap();
        app.open_path(&dir.join(name));
    }
    paint(&mut app);
    assert!(app.tabs.max_scroll() > 0.0);
    assert!(app.tabs.widths.iter().skip(2).all(|&w| w == TAB_WIDTH_MAX));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_tab_reorders_the_tabs() {
    let dir = search_folder("tabs-drag");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    app.open_path(&dir.join("src/config.rs"));
    app.open_path(&dir.join("Cargo.toml"));
    paint(&mut app);
    assert_eq!(names(&app), ["main.rs", "config.rs", "Cargo.toml"]);

    // 按下即切换到该标签；鼠标移动 3 像素仍算单击，顺序不变。
    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.editor.active, Some(0));
    app.on_mouse_move(x + 3.0, y);
    assert!(app.dragged_tab().is_none());
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(names(&app), ["main.rs", "config.rs", "Cargo.toml"]);

    // 向右拖过 `config.rs` 的中线时与 `config.rs` 交换位置；被拖动的标签跟随鼠标，所有悬停
    // 效果关闭。
    let config = tab_rect(&app, 1);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + config.w / 2.0 + 10.0, y);
    app.on_mouse_move(x + config.w / 2.0 + 10.0, y);
    assert_eq!(names(&app), ["config.rs", "main.rs", "Cargo.toml"]);
    assert_eq!(app.editor.active().unwrap().name(), "main.rs");
    assert_eq!(app.hover, Hover::None);
    // 在中线附近稍微往回移动，不会换回原位。
    app.on_mouse_move(x + config.w / 2.0 + 5.0, y);
    assert_eq!(names(&app), ["config.rs", "main.rs", "Cargo.toml"]);
    app.on_mouse_move(x + config.w / 2.0 + 10.0, y);
    paint(&mut app);
    let lifted = tab_rect(&app, 1);
    assert_eq!(lifted.x + 20.0, x + config.w / 2.0 + 10.0);
    // 鼠标下命中的是正在拖动的标签，而不是被遮挡的标签。
    assert_eq!(app.tab_at(x + config.w / 2.0 + 10.0, y), Some((1, false)));

    // 拖到最右边之外也只能到最后一个位置，不会拖出标签栏。
    app.on_mouse_move(app.layout.window.w - 4.0, y);
    assert_eq!(names(&app), ["config.rs", "Cargo.toml", "main.rs"]);
    paint(&mut app);
    let last = tab_rect(&app, 2);
    assert!(last.right() <= app.tabs.view.right());

    // 松开后停留在新位置，滑动动画结束时恰好落在标签位置。
    app.on_mouse_up(MouseButton::Left);
    assert!(app.dragged_tab().is_none());
    assert_eq!(names(&app), ["config.rs", "Cargo.toml", "main.rs"]);
    assert_eq!(app.editor.active().unwrap().name(), "main.rs");
    app.tabs.slides.clear();
    paint(&mut app);
    let (a, b) = (tab_rect(&app, 1), tab_rect(&app, 2));
    assert_eq!(b.x, a.right() + TAB_GAP);

    // 一次向左拖动越过两个标签。
    let (x, y) = tab_point(&app, 2);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(4.0, y);
    assert_eq!(names(&app), ["main.rs", "config.rs", "Cargo.toml"]);

    // 拖动过程中该标签被关闭（快捷键）：不留下任何拖动状态。
    app.close_tab(0);
    app.on_mouse_move(300.0, y);
    assert!(app.tabs.drag.is_none());
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(names(&app), ["config.rs", "Cargo.toml"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn switching_apps_mid_drag_drops_the_tab() {
    let dir = search_folder("tabs-focus");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    app.open_path(&dir.join("src/config.rs"));
    paint(&mut app);
    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 40.0, y);
    assert!(app.dragged_tab().is_some());

    // 按住鼠标时切换到其他应用，窗口收不到松开事件：此处结束拖动，切回后标签不再跟随鼠标。
    app.drop_drags();
    assert!(app.tabs.drag.is_none());
    app.on_mouse_move(x + 200.0, y);
    assert!(app.dragged_tab().is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_wheel_scrolls_a_full_tab_bar_sideways() {
    let dir = search_folder("tabs-wheel");
    let mut app = app_with(&dir, false);
    for _ in 0..20 {
        app.new_file();
    }
    app.activate_tab(0);
    paint(&mut app);
    assert_eq!(app.tabs.scroll, 0.0);
    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);

    // 鼠标滚轮的竖直一格、水平一格都横向滚动标签栏。
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    let after_vertical = app.tabs.scroll;
    assert!(after_vertical > 0.0);
    app.on_wheel(MouseScrollDelta::LineDelta(-1.0, 0.0));
    assert!(app.tabs.scroll > after_vertical);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dragging_to_the_end_of_a_full_tab_bar_scrolls_it() {
    let dir = search_folder("tabs-scroll");
    let mut app = app_with(&dir, false);
    for _ in 0..20 {
        app.new_file();
    }
    app.activate_tab(0);
    paint(&mut app);
    assert!(app.tabs.max_scroll() > 0.0);
    assert_eq!(app.tabs.scroll, 0.0);

    // 拖到右端后保持不动：标签栏自动向后滚动，被拖动的标签依次越过其他标签。
    let first = app.editor.documents[0].id;
    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(app.tabs.view.right() - 2.0, y);
    for _ in 0..1000 {
        if app.tab_drag_tick(Instant::now()).is_none() {
            break;
        }
    }
    assert_eq!(app.tabs.scroll, app.tabs.max_scroll());
    assert_eq!(app.editor.documents.last().unwrap().id, first);
    app.on_mouse_up(MouseButton::Left);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn images_open_as_previews_decoded_in_the_background() {
    let dir = sample_folder("image-preview");
    let path = dir.join("red.jpg");
    std::fs::write(
        &path,
        include_bytes!("../../../../../winc-preview/testdata/red-8x8.jpg"),
    )
    .unwrap();
    let mut app = app_with(&path, false);
    // 标签先打开，图片在后台解码。
    assert!(matches!(
        app.editor.active().unwrap().picture,
        Some(winc_editor::PictureState::Loading)
    ));
    app.wait_for_images();
    paint(&mut app);

    let doc = app.editor.active().unwrap();
    let picture = doc.picture().expect("打开成图片预览");
    assert_eq!((picture.width, picture.height), (8, 8));
    // 图片没有文本，不能编辑，也不能查找。
    assert!(doc.readonly);
    app.run_action(crate::keymap::Action::Find);
    assert!(!app.find.open);
    // 小图按原大小绘制：8 个逻辑像素，在缩放倍数 `SCALE` 为 2 时是 16 个物理像素。
    let cache = app.picture_cache.as_ref().unwrap();
    assert_eq!(
        (cache.width(), cache.height()),
        ((8.0 * SCALE) as u32, (8.0 * SCALE) as u32)
    );

    // 其他程序替换了图片文件：重新解码。
    std::fs::write(&path, image_bytes_16()).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let root = app.editor.root().unwrap().to_path_buf();
    app.apply_changes(winc_editor::Changes {
        paths: vec![root.join("red.jpg")],
        rescan: false,
        git: false,
    });
    app.wait_for_images();
    let picture = app.editor.active().unwrap().picture().unwrap();
    assert_eq!((picture.width, picture.height), (16, 4));

    // 损坏的图片：显示原因，可以重试。
    std::fs::write(dir.join("broken.png"), b"\x89PNG\r\n\x1a\nbroken").unwrap();
    app.open_path(&dir.join("broken.png"));
    app.wait_for_images();
    let doc = app.editor.active().unwrap();
    assert!(doc.picture.is_none());
    assert!(matches!(
        &doc.unsupported,
        Some(winc_editor::Unsupported::Unreadable(r)) if r.contains("无法解码")
    ));

    // 切换到其他标签：缩放后的缓存图不保留。
    app.new_file();
    paint(&mut app);
    assert!(app.picture_cache.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn svg_opens_as_source_and_toggles_to_a_preview() {
    use crate::keymap::Action;

    let dir = sample_folder("svg-preview");
    let path = dir.join("dot.svg");
    std::fs::write(
        &path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="red"/><path d="M0 0L9 9" stroke="red" marker-end="url(#m)"/></svg>"#,
    )
    .unwrap();
    let mut app = app_with(&path, false);
    let doc = app.editor.active().unwrap();
    assert!(doc.shows_text() && doc.can_preview() && !doc.readonly);
    // 「源代码 / 分栏 / 预览」三段切换在标签栏右端，标签栏给它让出位置。
    paint(&mut app);
    let button = app.layout.preview;
    assert!(button.w > 0.0);
    assert!(app.layout.tabs.right() < button.x);
    assert_eq!(button.right(), app.layout.top_bar.right() - 8.0);

    // 点三段切换的「预览」：文档只读，状态栏显示图片大小、未能绘制的特性（箭头标记）。
    let [_, _, eye] = crate::layout::preview_segments(button);
    crate::app::snapshot::click(&mut app, (eye.x + 4.0, eye.y + 4.0));
    app.wait_for_images();
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let picture = doc.picture().expect("SVG 预览");
    assert_eq!(
        (picture.width, picture.height, picture.format),
        (24, 24, "SVG")
    );
    assert_eq!(picture.missing(), ["箭头标记"]);
    assert!(doc.readonly && !doc.shows_text());
    assert_eq!(
        app.layout.preview, button,
        "预览时三段切换还在，点「源代码」回去"
    );
    // 小图标按长边 256 逻辑像素放大显示。
    let cache = app.picture_cache.as_ref().unwrap();
    assert_eq!(cache.width(), (256.0 * SCALE) as u32);
    app.run(winc_editor::Command::Insert("x".into()));
    assert!(!app.editor.active().unwrap().is_dirty());

    // 切回源代码：仍然可以编辑；编辑后再预览，绘制的是修改后的内容。
    app.run_action(Action::TogglePreview);
    let doc = app.editor.active().unwrap();
    assert!(doc.shows_text() && !doc.readonly);
    app.run_action(Action::SelectAll);
    app.run(winc_editor::Command::Insert(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"/>"#.into(),
    ));
    app.run_action(Action::TogglePreview);
    app.wait_for_images();
    let picture = app.editor.active().unwrap().picture().unwrap();
    assert_eq!((picture.width, picture.height), (10, 5));

    // 无法绘制时显示原因，并提供「显示源代码」。
    app.run_action(Action::TogglePreview);
    app.run_action(Action::SelectAll);
    app.run(winc_editor::Command::Insert("<svg".into()));
    app.run_action(Action::TogglePreview);
    app.wait_for_images();
    paint(&mut app);
    assert!(matches!(
        &app.editor.active().unwrap().picture,
        Some(winc_editor::PictureState::Failed(r)) if r.contains("XML")
    ));
    assert!(
        app.hits
            .placeholder
            .iter()
            .any(|(_, a)| *a == crate::app::PlaceholderAction::ShowSource)
    );

    // 其他类型的文件不能预览。
    app.new_file();
    paint(&mut app);
    assert_eq!(app.layout.preview.w, 0.0, "不能预览的文件没有这个按钮");
    app.run_action(Action::TogglePreview);
    assert_eq!(
        app.status.message.as_deref(),
        Some("仅支持预览 SVG、Markdown、CSV 和 TSV 文件")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回一张 16×4 像素的全白 PNG 的字节。
fn image_bytes_16() -> Vec<u8> {
    // 手写最小的 PNG：IHDR、一个 IDAT 块（deflate 存储块，不压缩）、IEND。
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let (w, h) = (16u32, 4u32);
    let mut raw = Vec::new();
    for _ in 0..h {
        raw.push(0);
        raw.extend(std::iter::repeat_n(255u8, (w * 3) as usize));
    }
    let mut z = vec![0x78, 0x01, 0x01];
    z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
    z.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    // IHDR 的后五个字节：位深 8、颜色类型 2（RGB）、压缩方法 0、滤波方法 0、不隔行扫描（0）。
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn close_tab_key_closes_the_window_once_no_tab_is_left_without_quitting() {
    use crate::app::snapshot::editor::navigation::press;
    use crate::keymap::KeyCode::Char;

    let dir = search_folder("close-tab-key");
    let mut app = app_with(&dir.join("Cargo.toml"), false);
    assert_eq!(app.editor.documents.len(), 1);

    // 有标签时 ⌘W 关闭标签，窗口不关。
    assert!(press(&mut app, &["cmd"], Char('w')));
    assert!(app.editor.documents.is_empty());
    assert!(!app.window.hidden && !app.closing.quit_requested);

    // 命令面板里的「关闭标签页」在没有标签时什么也不做。
    app.run_action(crate::keymap::Action::CloseTab);
    assert!(!app.window.hidden && !app.closing.quit_requested);

    // 没有标签时再按 ⌘W：macOS 上关闭窗口，应用不退出（留在程序坞中）；其他系统上什么也不做。
    press(&mut app, &["cmd"], Char('w'));
    assert_eq!(app.window.hidden, cfg!(target_os = "macos"));
    assert!(!app.closing.quit_requested);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：关闭中间一个标签、内容淡出到一半时（`tab-close-light`、`tab-close-dark`）。
///
/// 此时宽度尚未开始收起，右侧的标签仍在原位。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_tab_close() {
    use crate::app::animation::{Clock, EXIT_FADE};

    for (tag, dark) in [("light", false), ("dark", true)] {
        let dir = search_folder(&format!("tab-close-{tag}"));
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.open_path(&dir.join("a.txt"));
        app.open_path(&dir.join("Cargo.toml"));
        app.activate_tab(1);
        paint(&mut app);
        app.animate_tab_close_with(1, Clock::frozen(EXIT_FADE / 2, EXIT_FADE));
        app.editor.close(1);
        shoot(&mut app, &format!("tab-close-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_newly_opened_tab_grows_in_and_pushes_the_rest_aside() {
    use crate::app::animation::{Clock, TAB_SLIDE};

    let dir = search_folder("tab-open-grow");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.open_path(&dir.join("Cargo.toml"));
    paint(&mut app);
    // 在两个标签中间打开（新标签排在当前标签后面）：先切到第一个。
    app.activate_tab(0);
    app.open_path(&dir.join("a.txt"));
    paint(&mut app);
    let full = app.tabs.widths.clone();
    let plus = app.tabs.new_tab.x;
    let id = app.editor.active().unwrap().id;
    let index = app.editor.active.unwrap();

    // 长到三分之一的那一帧：它窄，右边的标签和新建按钮还没让到位。
    app.tabs
        .opening
        .push((id, Clock::frozen(TAB_SLIDE / 3, TAB_SLIDE)));
    paint(&mut app);
    assert!(app.tabs.widths[index] < full[index] * 0.9);
    assert!(app.tabs.widths[index] > 0.0);
    assert!(app.tabs.new_tab.x < plus);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：新打开的标签长到三分之一（`tab-open-light`、`tab-open-dark`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_tab_open() {
    use crate::app::animation::{Clock, TAB_SLIDE};

    for (tag, dark) in [("light", false), ("dark", true)] {
        let dir = search_folder(&format!("tab-open-{tag}"));
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.open_path(&dir.join("Cargo.toml"));
        app.activate_tab(0);
        app.open_path(&dir.join("a.txt"));
        paint(&mut app);
        let id = app.editor.active().unwrap().id;
        app.tabs
            .opening
            .push((id, Clock::frozen(TAB_SLIDE / 3, TAB_SLIDE)));
        shoot(&mut app, &format!("tab-open-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：SVG 的源代码，标签栏右端是预览按钮，鼠标停在上面。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_preview_button() {
    for dark in [false, true] {
        let name = if dark {
            "preview-button-dark"
        } else {
            "preview-button-light"
        };
        let dir = sample_folder(name);
        let path = dir.join("dot.svg");
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="red"/></svg>"#).unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        let b = app.layout.preview;
        app.on_mouse_move(b.x + 4.0, b.y + 4.0);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 把三段切换的动画固定在出现（`on`）或消失途中的 `at` 处。
///
/// 测试中没有窗口，`track_modules` 不播放动画，因此直接设置冻结的时钟。
fn freeze_preview_buttons(app: &mut App, on: bool, at: std::time::Duration) {
    use crate::app::animation::{Clock, EXIT_FADE, MODULE_FADE};
    // 消失时宽度在内容淡出结束后才开始收起，`at` 从收起开始计时。
    let at = if on { at } else { at + EXIT_FADE };
    let fx = &mut app.module_fx.preview;
    fx.track(!on, None);
    fx.track(on, Some(Clock::frozen(at, MODULE_FADE)));
}

/// 打开一个文本文件和一个 SVG 文件，SVG 文件为当前文档。
fn svg_and_text(name: &str, dark: bool) -> (App, std::path::PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("dot.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="red"/></svg>"#).unwrap();
    let mut app = app_with(&dir.join("a.txt"), dark);
    app.open_path(&path);
    paint(&mut app);
    (app, dir)
}

/// 切换到不支持预览的文件时，三段切换向右收窄（途中不响应点击），标签栏随之变宽；
/// 切换回来时三段切换展开，标签栏让出位置。
#[test]
fn preview_buttons_grow_and_shrink_with_the_tab() {
    use crate::app::animation::MODULE_FADE;
    let (mut app, dir) = svg_and_text("preview-fade", false);
    let (group, tabs) = (app.layout.preview, app.layout.tabs);
    assert!(group.w > 0.0);

    app.open_path(&dir.join("a.txt"));
    freeze_preview_buttons(&mut app, false, MODULE_FADE / 3);
    paint(&mut app);
    assert!(app.animating());
    let now = app.layout.preview;
    assert!(now.w > 0.0 && now.w < group.w, "{now:?}");
    assert_eq!(now.right(), group.right());
    assert!(app.layout.tabs.w > tabs.w);
    app.on_mouse_move(now.x + 2.0, now.y + 4.0);
    assert!(!matches!(app.hover, crate::app::Hover::Preview(_)));
    freeze_preview_buttons(&mut app, false, MODULE_FADE);
    app.track_modules();
    paint(&mut app);
    assert!(app.module_fx.preview_mode.is_none());
    assert_eq!(app.layout.preview.w, 0.0);

    app.open_path(&dir.join("dot.svg"));
    freeze_preview_buttons(&mut app, true, MODULE_FADE / 3);
    paint(&mut app);
    let now = app.layout.preview;
    assert!(now.w > 0.0 && now.w < group.w, "{now:?}");
    assert!(app.layout.tabs.w > tabs.w);
    freeze_preview_buttons(&mut app, true, MODULE_FADE);
    app.track_modules();
    paint(&mut app);
    assert_eq!((app.layout.preview, app.layout.tabs), (group, tabs));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截取三段切换收窄、展开到三分之一时的画面（`preview-fade-out-*`、`preview-fade-in-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_preview_buttons_fading() {
    use crate::app::animation::MODULE_FADE;
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let (mut app, dir) = svg_and_text(&format!("preview-fade-{theme}"), dark);
        app.open_path(&dir.join("a.txt"));
        freeze_preview_buttons(&mut app, false, MODULE_FADE / 3);
        shoot(&mut app, &format!("preview-fade-out-{theme}"));
        app.open_path(&dir.join("dot.svg"));
        freeze_preview_buttons(&mut app, true, MODULE_FADE / 3);
        shoot(&mut app, &format!("preview-fade-in-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 三段切换的界面测试：分栏时左侧仍可编辑、右侧随文字重画；文本解析失败时仍显示上一张图
/// 并在下方写出原因；只看预览时只读；⇧⌘V 从分栏直接切到只看预览。
#[test]
fn svg_split_preview_follows_the_text() {
    use winc_editor::PreviewMode;

    let dir = sample_folder("svg-split");
    let path = dir.join("box.svg");
    let svg =
        |w: u32| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="10"/>"#);
    std::fs::write(&path, svg(20)).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let [_, split, preview] = crate::layout::preview_segments(app.layout.preview);

    crate::app::snapshot::click(&mut app, (split.x + 4.0, split.y + 4.0));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    assert!(doc.shows_text() && !doc.readonly);
    let pane = app.layout.split;
    assert!(pane.w > 0.0 && app.layout.text.right() <= pane.x);
    app.wait_for_split();
    assert_eq!(app.split.picture.as_ref().unwrap().width, 20);

    // 改文字：右边跟着重画。
    app.run_action(crate::keymap::Action::SelectAll);
    app.run(winc_editor::Command::Insert(svg(30)));
    app.wait_for_split();
    assert_eq!(app.split.picture.as_ref().unwrap().width, 30);
    // 写到一半：还是上一张，另外记下原因。
    app.run(winc_editor::Command::Insert("<".into()));
    app.wait_for_split();
    assert_eq!(app.split.picture.as_ref().unwrap().width, 30);
    assert!(app.split.error.is_some());
    paint(&mut app);

    crate::app::snapshot::click(&mut app, (preview.x + 4.0, preview.y + 4.0));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Preview);
    assert!(doc.readonly);
    assert_eq!(app.layout.split.w, 0.0);

    app.set_preview_mode(PreviewMode::Split);
    app.run_action(crate::keymap::Action::TogglePreview);
    assert_eq!(
        app.editor.active().unwrap().preview_mode(),
        PreviewMode::Preview
    );
    app.run_action(crate::keymap::Action::TogglePreview);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Source);
    assert!(!doc.readonly);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 分栏和预览之间来回切，文字没改时直接用对方已经解析好的图，不再解析一遍。
#[test]
fn switching_between_split_and_preview_reuses_the_parsed_svg() {
    use std::sync::Arc;
    use winc_editor::{PictureState, PreviewMode};

    let dir = sample_folder("svg-split-reuse");
    let path = dir.join("box.svg");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"/>"#;
    std::fs::write(&path, svg).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);

    app.set_preview_mode(PreviewMode::Split);
    app.wait_for_split();
    let parsed = Arc::clone(app.split.picture.as_ref().unwrap());

    // 分栏 → 预览：马上就有图，是同一张。
    app.set_preview_mode(PreviewMode::Preview);
    let doc = app.editor.active().unwrap();
    assert!(matches!(&doc.picture, Some(PictureState::Ready(p)) if Arc::ptr_eq(p, &parsed)));

    // 换一张已解析好的预览图再切回分栏：右侧直接复用它，不在后台重新解析。
    let fresh = Arc::new(winc_preview::svg::load(svg, None, None).unwrap());
    app.editor.active_mut().unwrap().picture = Some(PictureState::Ready(Arc::clone(&fresh)));
    app.split = crate::app::preview::images::SplitPreview::default();
    app.set_preview_mode(PreviewMode::Split);
    app.advance_images();
    assert!(Arc::ptr_eq(app.split.picture.as_ref().unwrap(), &fresh));
    assert!(!app.split.is_pending());

    // 文字改动后切到预览：按新文字重新解析。
    app.run(winc_editor::Command::Insert(" ".into()));
    app.set_preview_mode(PreviewMode::Preview);
    let doc = app.editor.active().unwrap();
    assert!(matches!(doc.picture, Some(PictureState::Loading)));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：SVG 分栏预览，鼠标停在三段切换的「分栏」上。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_split_preview() {
    for dark in [false, true] {
        let name = if dark { "split-dark" } else { "split-light" };
        let dir = sample_folder(name);
        let path = dir.join("logo.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#3296FC"/>
      <stop offset="1" stop-color="#0035D2"/>
    </linearGradient>
  </defs>
  <rect x="5" y="5" width="90" height="90" rx="22" fill="url(#g)"/>
  <circle cx="50" cy="50" r="22" fill="#fff"/>
</svg>
"##,
        )
        .unwrap();
        let mut app = app_with(&path, dark);
        paint(&mut app);
        app.set_preview_mode(winc_editor::PreviewMode::Split);
        paint(&mut app);
        app.wait_for_split();
        let [_, split, _] = crate::layout::preview_segments(app.layout.preview);
        app.on_mouse_move(split.x + 4.0, split.y + 4.0);
        paint(&mut app);
        app.wait_for_images();
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn command_and_shift_clicks_select_tabs_to_close_together() {
    use winit::keyboard::ModifiersState;

    use crate::app::input::context_menu::MenuCommand;

    let dir = search_folder("tabs-multi-select");
    let mut app = app_with(&dir, false);
    for name in [
        "src/main.rs",
        "src/config.rs",
        "Cargo.toml",
        "a.txt",
        "short.rs",
    ] {
        app.open_path(&dir.join(name));
    }
    paint(&mut app);
    let click_tab = |app: &mut App, index: usize, modifiers: ModifiersState| {
        app.modifiers = modifiers;
        let at = tab_point(app, index);
        click(app, at);
        app.modifiers = ModifiersState::empty();
        paint(app);
    };
    let selected = |app: &App| -> Vec<String> {
        let ids = app.tab_selection();
        app.editor
            .documents
            .iter()
            .filter(|d| ids.contains(&d.id))
            .map(|d| d.name().into_owned())
            .collect()
    };

    // ⌘ 单击加选并切到它；再 ⌘ 单击当前的那个把它移出，切到选区里剩下的。
    click_tab(&mut app, 0, ModifiersState::empty());
    click_tab(&mut app, 2, command_modifiers());
    assert_eq!(selected(&app), ["main.rs", "Cargo.toml"]);
    assert_eq!(app.editor.active().unwrap().name(), "Cargo.toml");
    click_tab(&mut app, 3, command_modifiers());
    click_tab(&mut app, 3, command_modifiers());
    assert_eq!(selected(&app), ["main.rs", "Cargo.toml"]);
    assert_eq!(app.editor.active().unwrap().name(), "Cargo.toml");

    // ⇧ 单击从上一次单击的标签选到这一个。
    click_tab(&mut app, 1, ModifiersState::SHIFT);
    assert_eq!(selected(&app), ["config.rs", "Cargo.toml", "a.txt"]);
    assert_eq!(app.editor.active().unwrap().name(), "config.rs");

    // 右键点在选区里的标签上，「关闭」一起关掉。
    let at = tab_point(&app, 2);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::Close);
    assert_eq!(names(&app), ["main.rs", "short.rs"]);
    assert!(!app.tabs_multi_selected());

    // 普通单击回到只选一个；通过其他方式切换标签后也不再算多选。
    click_tab(&mut app, 0, ModifiersState::empty());
    click_tab(&mut app, 1, command_modifiers());
    assert!(app.tabs_multi_selected());
    click_tab(&mut app, 0, ModifiersState::empty());
    assert_eq!(selected(&app), ["main.rs"]);
    click_tab(&mut app, 1, command_modifiers());
    app.open_path(&dir.join("Dockerfile"));
    assert_eq!(selected(&app), ["Dockerfile"]);

    // 「关闭其他」留下选中的各个标签。
    click_tab(&mut app, 0, ModifiersState::empty());
    click_tab(&mut app, 2, command_modifiers());
    let at = tab_point(&app, 2);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseOthers);
    assert_eq!(names(&app), ["main.rs", "Dockerfile"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn same_name_tabs_add_folders_until_they_differ() {
    use std::path::Path;

    use crate::view::tests::distinct_folder;

    let folder = |path: &str, others: &[&str]| {
        let others: Vec<&Path> = others.iter().map(Path::new).collect();
        distinct_folder(Path::new(path), &others)
    };
    // 上一级目录不同时只标注上一级；相同时逐级向上添加，直到能够区分。
    assert_eq!(
        folder("/w/api/mod.rs", &["/w/web/mod.rs"]).as_deref(),
        Some("api")
    );
    assert_eq!(
        folder("/w/a/src/mod.rs", &["/w/b/src/mod.rs"]).as_deref(),
        Some("a/src")
    );
    // 须与每个同名文件都能区分：三个文件中有两个的上一级目录相同。
    assert_eq!(
        folder("/w/a/src/mod.rs", &["/w/b/src/mod.rs", "/w/a/lib/mod.rs"]).as_deref(),
        Some("a/src")
    );
    // 一个文件位于另一个文件的上级目录中时，路径较短的一方使用全部上级目录。
    assert_eq!(folder("/w/mod.rs", &["/w/x/mod.rs"]).as_deref(), Some("w"));
    assert_eq!(folder("/w/x/mod.rs", &["/w/mod.rs"]).as_deref(), Some("x"));
}

#[test]
fn hovering_a_tab_shows_its_path_and_the_close_shortcut() {
    let dir = search_folder("tabs-tooltip");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.open_path(&dir.join("a.txt"));
    paint(&mut app);

    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);
    assert!(app.hover.has_tooltip());
    let (rect, text) = crate::view::tests::tooltip(&app).unwrap();
    assert_eq!(rect, tab_rect(&app, 0));
    let path = app.editor.documents[0].path.clone().unwrap();
    assert!(path.ends_with("src/main.rs"));
    let folder = winc_editor::display_path(path.parent().unwrap(), None);
    assert_eq!(text, format!("main.rs\n{folder}"));

    let close = app.hits.tabs.iter().find(|h| h.index == 1).unwrap().close;
    app.on_mouse_move(close.x + close.w / 2.0, close.y + close.h / 2.0);
    let (rect, text) = crate::view::tests::tooltip(&app).unwrap();
    assert_eq!(rect, close);
    assert!(text.starts_with("关闭"), "{text}");
    // Windows 上关闭标签页的快捷键写作 Ctrl+F4，与系统约定的做法一致。
    let key = if cfg!(windows) { "F4" } else { "W" };
    assert!(text.contains(key), "{text}");

    // 按住标签拖动时不显示提示。
    let (x, y) = tab_point(&app, 0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 40.0, y);
    assert!(app.tabs.drag.is_some());
    assert_eq!(crate::view::tests::tooltip(&app), None);
    app.on_mouse_up(MouseButton::Left);
}

#[test]
fn tabs_without_a_file_show_a_tooltip_only_when_the_name_is_shortened() {
    let mut app = blank_app(false);
    app.new_file();
    paint(&mut app);
    let index = app.editor.active.unwrap();
    let (x, y) = tab_point(&app, index);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::Tab(index, false));
    assert_eq!(
        crate::view::tests::tooltip(&app),
        None,
        "名称完整显示时不提示"
    );

    // 名称超出标签宽度被截短时，提示显示完整名称。

    let long = "很长的标签名称".repeat(12);
    app.editor.documents[index].title = Some(long.clone());
    paint(&mut app);
    let (x, y) = tab_point(&app, index);
    app.on_mouse_move(x, y);
    assert_eq!(
        crate::view::tests::tooltip(&app),
        Some((tab_rect(&app, index), long))
    );
}

#[test]
fn the_tab_list_lists_every_tab_when_they_overflow() {
    use crate::app::MenuCommand;

    let dir = search_folder("tabs-list");
    for p in ["a/mod.rs", "b/mod.rs"] {
        std::fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
        std::fs::write(dir.join(p), "x\n").unwrap();
    }
    let mut app = app_with(&dir.join("a/mod.rs"), false);
    app.open_path(&dir.join("b/mod.rs"));
    paint(&mut app);
    // 标签全部放得下时不显示「⌄」。
    assert_eq!(app.tabs.list, None);

    for i in 0..12 {
        let name = format!("{i}-a-much-longer-file-name.rs");
        std::fs::write(dir.join(&name), "x\n").unwrap();
        app.open_path(&dir.join(name));
    }
    paint(&mut app);
    // 放不下时「⌄」位于最右端，新建按钮在其左侧，标签的可见范围位于新建按钮左侧。
    let list = app.tabs.list.expect("放不下时有「⌄」");
    assert_eq!(list.right(), app.layout.tabs.right());
    assert!(app.tabs.new_tab.right() <= list.x);
    assert!(app.tabs.view.right() < app.tabs.new_tab.x);

    let (x, y) = (list.x + list.w / 2.0, list.y + list.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::TabList);
    assert_eq!(
        crate::view::tests::tooltip(&app)
            .map(|(_, text)| text)
            .as_deref(),
        Some("显示所有标签页")
    );

    // 点击「⌄」后按标签顺序列出全部标签，当前标签带勾选标记；
    // 两个同名文件的副标题为标签上标注的上级目录。
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.menu_button, Some(crate::app::MenuButton::TabList));
    let entries = &app.context_menu.as_ref().expect("弹出了标签列表").entries;
    let count = app.editor.documents.len();
    assert_eq!(entries.len(), count);
    let active = app.editor.active.unwrap();
    assert_eq!(active, count - 1);
    for i in 0..count {
        assert_eq!(app.menu_checked(MenuCommand::ShowTab(i)), i == active);
        let label = app.menu_label(MenuCommand::ShowTab(i));
        assert_eq!(label.title, app.editor.documents[i].name());
        let path = app.editor.documents[i].path.as_deref().unwrap();
        assert_eq!(label.tooltip, Some(winc_editor::display_path(path, None)));
    }
    let subtitle = |i| app.menu_label(MenuCommand::ShowTab(i)).subtitle;
    assert_eq!(subtitle(0).as_deref(), Some("a"));
    assert_eq!(subtitle(1).as_deref(), Some("b"));
    assert_eq!(subtitle(2), None);
    // 菜单项的图标与标签上的图标相同，绘制在 16×16 的正方形内。
    let icon = app.menu_icon(MenuCommand::ShowTab(0)).expect("有图标");
    assert_eq!(icon.width, icon.height);

    // 选择第一项后切换到对应标签，并滚动标签栏使其可见。
    assert!(app.tabs.scroll > 0.0);
    app.run_menu_command(MenuCommand::ShowTab(0));
    paint(&mut app);
    assert_eq!(app.editor.active, Some(0));
    assert_eq!(app.tabs.scroll, 0.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn web_pages_get_an_open_in_browser_button() {
    let dir = search_folder("browser-button");
    let page = dir.join("index.HTML");
    std::fs::write(&page, "<h1>hi</h1>\n").unwrap();
    let mut app = app_with(&page, false);
    paint(&mut app);

    // 网页：标签栏右端有一个按钮，标签栏让出该处；没有预览切换。
    let button = app.layout.browser;
    assert_eq!(button.w, crate::theme::BUTTON);
    assert!(app.layout.tabs.right() <= button.x);
    assert_eq!(app.layout.preview.w, 0.0);
    let at = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(app.hover, crate::app::Hover::Browser);
    shoot(&mut app, "browser-button");

    // 没有未保存的修改：直接打开，不提示；有修改时提示浏览器中是上次保存的内容。
    app.status.message = None;
    click(&mut app, at);
    assert_eq!(app.status.message, None);
    app.run(winc_editor::Command::Insert("<p>new</p>".into()));
    click(&mut app, at);
    assert_eq!(
        app.status.message.as_deref(),
        Some(winc_i18n::t!("preview-browser-unsaved"))
    );

    // 换到不是网页的文件：按钮向右收起，收起途中也不响应。
    use crate::app::animation::{Clock, EXIT_FADE, MODULE_FADE};
    app.open_path(&dir.join("a.txt"));
    let fx = &mut app.module_fx.browser;
    fx.track(true, None);
    let at = EXIT_FADE + MODULE_FADE / 3;
    fx.track(false, Some(Clock::frozen(at, MODULE_FADE)));
    paint(&mut app);
    assert!(app.browser_page().is_none());
    let now = app.layout.browser;
    assert!(now.w > 0.0 && now.w < button.w, "{now:?}");
    assert_eq!(now.right(), button.right());
    app.on_mouse_move(now.x + 2.0, now.y + 4.0);
    assert_ne!(app.hover, crate::app::Hover::Browser);

    let _ = std::fs::remove_dir_all(&dir);
}
