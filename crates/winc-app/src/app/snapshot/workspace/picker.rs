//! 快速打开面板（⌘P）的测试：匹配与排序、最近打开、`文件:行:列` 跳转、选中项的移动、
//! 滚动与卡片高度动画。

use super::*;

#[test]
fn picker_lists_matches_and_opens_the_selected_file() {
    let dir = search_folder("picker");
    let mut app = app_with(&dir, false);

    // `⌘P`：面板打开、焦点在输入框，列表中是该目录的文件。
    app.open_picker();
    assert!(app.picker.open);
    assert_eq!(app.focus, Focus::Picker);
    assert!(app.picker.hits.len() >= 3, "{:?}", app.picker.hits);

    // 输入 `cfg`，按文件名模糊匹配，`config.rs` 排在第一位。
    app.picker.input.set_text("cfg");
    app.picker_refresh();
    assert!(!app.picker.hits.is_empty());
    let first = app
        .picker
        .index
        .file(app.picker.hits[0])
        .unwrap()
        .path
        .to_owned();
    assert!(first.ends_with("config.rs"), "{first}");

    // ↩ 打开选中的文件。
    app.picker_accept();
    assert!(!app.picker.open);
    assert_eq!(app.focus, Focus::Editor);
    assert!(
        app.editor
            .active()
            .and_then(|d| d.path.as_ref())
            .is_some_and(|p| p.ends_with("config.rs"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn picker_lists_recent_files_first_and_jumps_to_a_line() {
    let dir = search_folder("picker-recent");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    app.open_path(&root.join("src/config.rs"));
    app.open_path(&root.join("Cargo.toml"));
    app.open_path(&root.join("src/main.rs"));

    // 查询为空时：最近打开的文件排在最前（当前文件除外），第一条是上一个文件，回车即可切回。
    app.open_picker();
    assert_eq!(app.picker.recent, 2);
    let path = |app: &App, i: usize| {
        app.picker
            .index
            .file(app.picker.hits[i])
            .unwrap()
            .path
            .to_owned()
    };
    assert_eq!(path(&app, 0), "Cargo.toml");
    assert_eq!(path(&app, 1), "src/config.rs");
    // 其后是其余文件，按路径排序，且不与最近列表重复。
    let rest: Vec<String> = (2..app.picker.hits.len()).map(|i| path(&app, i)).collect();
    assert!(!rest.contains(&"Cargo.toml".to_owned()));
    // 排序不区分大小写：`a.txt` 与 `Cargo.toml` 按字母顺序排列，大写开头的文件不会全部排在前面。
    assert!(
        rest.windows(2)
            .all(|w| w[0].to_lowercase() <= w[1].to_lowercase()),
        "{rest:?}"
    );
    app.picker_accept();
    assert_eq!(app.editor.active().unwrap().name(), "Cargo.toml");

    // 查询非空时按匹配度排序，不显示「最近打开」分组。
    app.open_picker();
    app.picker.input.set_text("main");
    app.picker_refresh();
    assert_eq!(app.picker.recent, 0);

    // `文件:行:列`：打开文件并跳转到该位置（输入的行号、列号从 1 起）。
    app.picker.input.set_text("main.rs:2:5");
    app.picker_refresh();
    assert_eq!(app.picker.goto.as_deref(), Some("2:5"));
    assert_eq!(path(&app, 0), "src/main.rs");
    app.picker_accept();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.name(), "main.rs");
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (1, 4));

    // 只写 `:行`：在当前文件中跳转。
    app.open_picker();
    app.picker.input.set_text(":3");
    app.picker_refresh();
    assert!(app.picker.hits.is_empty());
    app.picker_accept();
    assert!(!app.picker.open);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (2, 0));

    // 窗口失去焦点期间在外部新增了文件，回到窗口后按 ⌘P 即可搜到。
    std::fs::write(root.join("added.rs"), "").unwrap();
    app.check_disk();
    app.open_picker();
    app.picker.input.set_text("added");
    app.picker_refresh();
    assert_eq!(path(&app, 0), "added.rs");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn picker_moves_selection_and_stops_at_the_ends() {
    let dir = search_folder("picker-move");
    let mut app = app_with(&dir, false);
    app.open_picker();

    let total = app.picker.hits.len();
    assert!(total >= 2, "这个目录里应该有不止一个文件");

    assert_eq!(app.picker.selected, 0);
    app.picker_move(false);
    assert_eq!(app.picker.selected, 0, "已经在第一条，再往上不动");

    app.picker_move(true);
    assert_eq!(app.picker.selected, 1);

    for _ in 0..total + 5 {
        app.picker_move(true);
    }
    assert_eq!(app.picker.selected, total - 1, "到底就停住，不绕回开头");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn picker_closes_on_escape_and_refreshes_while_typing() {
    let dir = search_folder("picker-esc");
    let mut app = app_with(&dir, false);
    app.open_picker();

    app.picker.input.set_text("zzz-没有这个文件");
    app.picker_refresh();
    assert!(app.picker.hits.is_empty());

    // 没有结果时回车不执行任何操作，面板保持打开。
    app.picker_accept();
    assert!(app.picker.open);

    app.close_picker();
    assert!(!app.picker.open);
    assert_eq!(app.focus, Focus::Editor);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rows_beyond_the_card_are_not_drawn_below_it() {
    let dir = sample_folder("picker-overflow");
    for i in 0..30 {
        std::fs::write(dir.join(format!("f{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, false);
    app.open_picker();
    app.picker_refresh();
    let shown = PickerParts::new(app.layout.picker).list.h / PICKER_ROW;
    assert!(app.picker.hits.len() > shown as usize);

    // 卡片下方的一条像素带（位于阴影之外）：结果比列表能显示的多一行时，该像素带与结果刚好
    // 显示得下时完全相同，说明多出的行没有画到卡片之外。
    let below = |app: &mut App| {
        paint(app);
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let card = app.layout.picker;
        let y = ((card.bottom() + PICKER_ROW / 2.0) * SCALE) as u32;
        let (x0, x1) = ((card.x * SCALE) as u32, (card.right() * SCALE) as u32);
        let row = (y * image.width) as usize;
        image.pixels[row + x0 as usize..row + x1 as usize].to_vec()
    };
    let overflowing = below(&mut app);
    app.picker.hits.truncate(shown as usize);
    assert_eq!(below(&mut app), overflowing);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_card_slides_to_its_new_height() {
    use std::time::Duration;

    let at = |ms| Clock::frozen(Duration::from_millis(ms), PICKER_ANIMATION);
    let dir = sample_folder("picker-height");
    for i in 0..30 {
        std::fs::write(dir.join(format!("f{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, false);
    app.open_picker();
    app.relayout();
    let tall = app.layout.picker.h;

    // 没有窗口时不做动画：输入后直接变为一行的高度（截图逐像素比对依赖这一点）。
    app.picker.input.set_text("f07");
    app.picker_refresh();
    app.relayout();
    let short = app.layout.picker.h;
    assert!(short < tall);
    assert!(app.picker.motion.is_none());

    // 动画进行中：卡片高度介于起止高度之间（此处为从 `tall` 到 `short` 的 0.875 处），
    // 绘制和点击命中都以此高度为界。
    let shrinking = PickerMotion {
        from: tall,
        to: short,
        clock: at(80),
    };
    app.picker.motion = Some(shrinking);
    app.relayout();
    let shown = app.layout.picker.h;
    assert_eq!(shown, tall - (tall - short) * 0.875);
    paint(&mut app);

    // 滚动范围按动画终点的高度计算，不随动画过程变化：只有一行的列表不需要滚动。
    assert_eq!(
        crate::app::workspace::picker::picker_span(&app),
        PickerParts::new(app.picker_rect()).list.h
    );
    assert_eq!(app.picker_rect().h, short);

    // 动画进行中结果再次变化：从当前高度继续滑动，不跳回起点。
    let next = PickerMotion::toward(app.picker.motion, short, tall, at(0));
    assert_eq!((next.from, next.height()), (shown, shown));
    assert_eq!(PickerMotion::toward(None, short, tall, at(0)).from, short);

    // 窗口大小改变、面板关闭：卡片高度都直接到位，不做动画。
    app.picker.motion = Some(shrinking);
    app.picker.height = app.picker.height.map(|(h, _)| (h, (1.0, 1.0)));
    app.relayout();
    assert!(app.picker.motion.is_none());
    app.picker.motion = Some(shrinking);
    app.close_picker();
    app.relayout();
    assert!(app.picker.motion.is_none() && app.picker.height.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewalking_the_folder_keeps_listing_the_old_table() {
    let dir = sample_folder("picker-rewalk");
    for i in 0..30 {
        std::fs::write(dir.join(format!("f{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, false);
    app.open_picker();
    app.picker_refresh();
    assert!(!app.picker_indexing());

    // 回到窗口后再按 ⌘P：后台重新遍历目录，结果尚未取回（测试中没有事件循环，不会取回）。
    app.picker.index.refresh();
    app.picker_refresh();
    app.relayout();
    assert!(!app.picker.index.is_ready());
    assert!(!app.picker_indexing());
    assert!(app.picker.hits.len() > PICKER_MAX_ROWS);

    // 最后一个可见行的文件图标仍然绘制，而不是只显示一行「正在索引文件…」、下方留空。
    paint(&mut app);
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let parts = PickerParts::new(app.layout.picker);
    let last = parts.row(PICKER_MAX_ROWS - 1, 0.0);
    let mut colors = std::collections::HashSet::<u32>::new();
    for y in (last.y * SCALE) as u32..(last.bottom() * SCALE) as u32 {
        let row = (y * image.width) as usize;
        let x0 = ((last.x + PICKER_ICON_X) * SCALE) as usize;
        colors.extend(
            &image.pixels[row + x0..row + x0 + (crate::icons::FILE_WIDTH * SCALE) as usize],
        );
    }
    assert!(colors.len() > 3, "最后一行是空的");

    // 只有首次建立文件表、尚无内容可列时，才显示那一行提示文字。
    app.picker.index = winc_editor::FileIndex::spawn(&dir, || {});
    assert!(app.picker_indexing());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scrolling_the_list_keeps_rows_at_the_top() {
    let dir = sample_folder("picker-scroll");
    for i in 0..30 {
        std::fs::write(dir.join(format!("f{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, false);
    app.open_picker();
    app.picker_refresh();

    // 滚动 5.5 行：列表顶部露出第 5 条（序号从 0 起）的下半部分，紧接其后是第 6 条，
    // 而不是一片空白。
    app.picker.scroll = 5.5 * PICKER_ROW;
    paint(&mut app);
    let list = PickerParts::new(app.layout.picker).list;
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let band = |top: f32, bottom: f32| {
        let mut colors = std::collections::HashSet::<u32>::new();
        for y in (top * SCALE) as u32..(bottom * SCALE) as u32 {
            let row = (y * image.width) as usize;
            let (x0, x1) = (
                ((list.x + 12.0) * SCALE) as usize,
                ((list.x + 200.0) * SCALE) as usize,
            );
            colors.extend(&image.pixels[row + x0..row + x1]);
        }
        colors.len()
    };
    assert!(
        band(list.y, list.y + PICKER_ROW * 1.5) > 3,
        "列表顶上是空的"
    );

    // 点击第 6 条绘制的位置，命中的就是第 6 条。
    let sixth = PickerParts::new(app.layout.picker).row(6, app.picker.scroll);
    assert_eq!(
        PickerParts::new(app.layout.picker).row_at(sixth.y + sixth.h / 2.0, app.picker.scroll),
        Some(6)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
