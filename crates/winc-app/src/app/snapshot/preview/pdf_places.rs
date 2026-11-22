//! PDF 预览的阅读位置：切换标签后保留，记入各文件上次的位置，再次打开时恢复。

use winit::keyboard::{Key, NamedKey};

use super::*;

/// 绘制并等待渲染线程，直到打开和排版完成。
fn settle(app: &mut App) {
    for _ in 0..3 {
        paint(app);
        app.wait_for_pdf();
    }
    paint(app);
}

/// 返回文件 `path` 此刻的阅读位置；按文件名查找，不受临时目录符号链接的影响。
fn place(app: &App, path: &Path) -> Option<(usize, f32)> {
    let places = app.pdf_places_now();
    let found = (places.iter()).find(|(file, _)| file.file_name() == path.file_name());
    found.and_then(|(_, place)| *place)
}

/// 两个阅读位置是否为同一处：页号相同，页内比例之差不超过千分之一。
fn same(a: Option<(usize, f32)>, b: Option<(usize, f32)>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.0 == b.0 && (a.1 - b.1).abs() < 1e-3)
}

/// 从 PDF 切换到其他标签再切回，以及关闭标签后重新打开，都回到原来的位置；位于开头时不记录。
#[test]
fn the_pdf_preview_returns_to_where_it_was_left() {
    let dir = sample_folder("pdf-places-tabs");
    let (first, second) = (dir.join("手册.pdf"), dir.join("附录.pdf"));
    std::fs::write(&first, sample_pdf(30)).unwrap();
    std::fs::write(&second, sample_pdf(8)).unwrap();
    let mut app = open_pdf(&first, false);
    assert_eq!(place(&app, &first), None, "位于开头时不记录");

    app.scroll_pdf(-4321.0);
    settle(&mut app);
    let (scroll, left) = (app.preview.pdf.scroll, place(&app, &first));
    assert!(left.is_some_and(|(page, _)| page > 0), "{left:?}");

    // 切换到文本标签：预览关闭，位置仍在现场中。
    app.open_path(&dir.join("a.txt"));
    settle(&mut app);
    assert!(app.preview.pdf.pages.is_none());
    assert!(same(place(&app, &first), left));

    // 另一个 PDF 从开头显示，不沿用前一个文件的位置。
    app.open_path(&second);
    settle(&mut app);
    assert_eq!(app.preview.pdf.scroll, 0.0);
    app.scroll_pdf(-700.0);
    settle(&mut app);
    let other = app.preview.pdf.scroll;

    app.open_path(&first);
    settle(&mut app);
    assert!(
        (app.preview.pdf.scroll - scroll).abs() < 0.5,
        "{}",
        app.preview.pdf.scroll
    );

    // 关闭标签后重新打开，仍回到原处。
    app.close_tab(app.editor.active.unwrap());
    settle(&mut app);
    app.open_path(&first);
    settle(&mut app);
    assert!(
        (app.preview.pdf.scroll - scroll).abs() < 0.5,
        "{}",
        app.preview.pdf.scroll
    );
    app.open_path(&second);
    settle(&mut app);
    assert!(
        (app.preview.pdf.scroll - other).abs() < 0.5,
        "{}",
        app.preview.pdf.scroll
    );

    // 回到开头后去掉它的记录。
    app.open_path(&first);
    settle(&mut app);
    assert!(app.pdf_key(&Key::Named(NamedKey::Home)));
    settle(&mut app);
    app.remember_session();
    let read = |(_, place): &&(PathBuf, winc_config::FilePlace)| place.view.is_some();
    let saved: Vec<_> = app.state.places.iter().filter(read).collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].0.file_name(), second.file_name());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 阅读位置记在状态里：重新启动后以另一个缩放倍数打开，显示的仍是同一页的同一处；没有打开
/// 目录时同样记得。
#[test]
fn the_pdf_place_is_restored_with_the_session() {
    let dir = sample_folder("pdf-places-session");
    let path = dir.join("手册.pdf");
    std::fs::write(&path, sample_pdf(30)).unwrap();
    let mut app = open_pdf(&path, false);
    app.scroll_pdf(-6000.0);
    settle(&mut app);
    let left = place(&app, &path);
    app.remember_session();

    let mut next = blank_app(false);
    next.state = app.state.clone();
    next.preview.pdf.zoom = 1.5;
    next.open_path(&dir);
    next.open_path(&path);
    settle(&mut next);
    assert!(
        next.preview.pdf.scroll > app.preview.pdf.scroll,
        "放大后同一处的滚动位置更大"
    );
    assert!(same(place(&next, &path), left), "{left:?}");

    // 不打开目录、只打开这个文件：同样回到原处，看到的新位置也记得下来。
    let mut alone = blank_app(false);
    alone.state = app.state.clone();
    alone.open_path(&path);
    settle(&mut alone);
    assert!(alone.editor.root().is_none());
    assert!(same(place(&alone, &path), left), "{left:?}");
    alone.scroll_pdf(-900.0);
    settle(&mut alone);
    let further = place(&alone, &path);
    alone.remember_session();
    let saved = alone
        .state
        .places
        .iter()
        .find(|(file, _)| file.file_name() == path.file_name());
    assert!(same(saved.and_then(|(_, place)| place.view), further));

    // 记录的页已不存在（文件被替换为较短的文档）时从开头显示。文件仍被前几个实例打开，部分
    // 系统不允许此时覆盖它，因此改写记录的页码。
    let mut short = blank_app(false);
    short.state = app.state.clone();
    for (_, place) in &mut short.state.places {
        place.view = place.view.map(|(_, at)| (999, at));
    }
    short.open_path(&dir);
    short.open_path(&path);
    settle(&mut short);
    assert_eq!(short.preview.pdf.scroll, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}
