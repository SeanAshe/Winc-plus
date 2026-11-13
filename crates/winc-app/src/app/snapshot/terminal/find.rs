//! 终端中查找的测试：查找条、匹配的高亮与跳转。

use super::*;

#[test]
fn find_walks_up_from_the_newest_match() {
    let dir = sample_folder("terminal-find");
    let mut app = app_with(&dir.join("short.rs"), false);
    let output: String = (1..=40)
        .map(|i| {
            if i % 10 == 0 {
                format!("Warning {i}\r\n")
            } else {
                format!("ok {i}\r\n")
            }
        })
        .collect();
    let term = show_terminal(&mut app, &output);

    assert!(press(&mut app, &["term"], Char('f')));
    assert_eq!(app.focus, Focus::TerminalFind);
    // 没有辅助进程时取不到回滚缓冲：把本地仿真中的内容交给查找，等同于收到回答。
    let tab = app.terminal.tab().unwrap();
    let (start, end) = tab.history;
    let id = tab.id;
    let (start, lines) = term.history(start, end);
    app.terminal_find_loaded(id, start, lines, true);

    app.edit_field(|f| {
        f.set_text("warning");
        true
    });
    let find = app.terminal.find.as_ref().unwrap();
    assert_eq!(find.matches.len(), 4);
    assert_eq!(find.current, Some(3));
    let text = |app: &App| {
        let tab = app.terminal.tab().unwrap();
        tab.selected_text().unwrap().unwrap()
    };
    assert_eq!(text(&app), "Warning");

    // ↩ 往上一处，翻到能看见它；绕到最下面。
    app.terminal_find_step(true);
    app.terminal_find_step(true);
    app.terminal_find_step(true);
    serve(&mut app, &term);
    let tab = app.terminal.tab().unwrap();
    let first = app.terminal.find.as_ref().unwrap().matches[0].0;
    assert_eq!(app.terminal.find.as_ref().unwrap().current, Some(0));
    assert!(first.line >= tab.top() && first.line < tab.top() + tab.rows.len() as u64);
    paint(&mut app);
    app.terminal_find_step(true);
    assert_eq!(app.terminal.find.as_ref().unwrap().current, Some(3));

    // 有大写字母时区分大小写。
    app.edit_field(|f| {
        f.set_text("Warning 2");
        true
    });
    assert_eq!(app.terminal.find.as_ref().unwrap().matches.len(), 1);
    app.close_terminal_find();
    assert_eq!(app.focus, Focus::Terminal);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn find_scans_in_slices_from_the_bottom() {
    let dir = sample_folder("terminal-find-slices");
    let mut app = app_with(&dir.join("short.rs"), false);
    let output: String = (1..=40)
        .map(|i| {
            if i % 10 == 0 {
                format!("Warning {i}\r\n")
            } else {
                format!("ok {i}\r\n")
            }
        })
        .collect();
    let term = show_terminal(&mut app, &output);
    assert!(press(&mut app, &["term"], Char('f')));
    let tab = app.terminal.tab().unwrap();
    let (id, (start, end)) = (tab.id, tab.history);
    let (start, lines) = term.history(start, end);
    app.terminal_find_loaded(id, start, lines, true);

    // 回归：每改一次查找的词都同步找完整个回滚缓冲。时间预算为零时每次只找一行，
    // 先找到的是最下面的那一处，选中它，其余的在之后的时间片里补上。
    app.terminal
        .find
        .as_mut()
        .unwrap()
        .field
        .set_text("warning");
    app.restart_terminal_find();
    let count = |app: &App| app.terminal.find.as_ref().unwrap().matches.len();
    while count(&app) == 0 {
        app.continue_terminal_find(std::time::Duration::ZERO);
    }
    let selected = |app: &App| {
        let tab = app.terminal.tab().unwrap();
        tab.selected_text().unwrap().unwrap()
    };
    assert_eq!(count(&app), 1);
    assert_eq!(app.terminal.find.as_ref().unwrap().current, Some(0));
    let newest = app.terminal.tab().unwrap().selection.unwrap().anchor;
    // 找完以后当前的仍是同一处，序号随前面补上的匹配后移。
    while app.terminal_find_tick() {}
    let find = app.terminal.find.as_ref().unwrap();
    assert_eq!(find.matches.len(), 4);
    assert_eq!(find.current, Some(3));
    assert_eq!(find.matches[3].0, newest);
    assert_eq!(selected(&app), "Warning");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 查找条关掉前都能点回去，按钮可用，点击不落到下面的终端。
#[test]
fn the_find_bar_takes_clicks_and_closes() {
    use crate::app::terminal::{FindPart, HeaderHit};
    let mut app = blank_app(true);
    let output: String = (1..=5).map(|i| format!("Warning {i}\r\n")).collect();
    show_terminal(&mut app, &output);
    assert!(press(&mut app, &["term"], Char('f')));
    app.edit_field(|field| {
        field.set_text("warning");
        true
    });
    paint(&mut app);

    // 和第一行垂直居中，高度与标签相同。
    let bar = app.terminal_find_bar().unwrap();
    let row = app.terminal_view().y + app.fonts.terminal_line_height / 2.0;
    assert_eq!(bar.y + bar.h / 2.0, row);
    assert_eq!(bar.h, crate::theme::TAB_HEIGHT);

    // 点到终端：焦点离开，查找条不关闭。
    let (x, y) = cell_center(&app, 4, 0);
    click(&mut app, (x, y));
    assert_eq!(app.focus, Focus::Terminal);
    assert!(app.terminal.find.is_some());
    paint(&mut app);

    // 点输入框：焦点回来，查找词还在。
    let area = app.terminal.find.as_ref().unwrap().field.view.area;
    click(&mut app, (area.x + 4.0, area.y + area.h / 2.0));
    assert_eq!(app.focus, Focus::TerminalFind);
    assert_eq!(app.terminal.find.as_ref().unwrap().field.text(), "warning");
    paint(&mut app);

    let center_of = |app: &App, part| {
        let hits = &app.hits.terminal;
        let rect = (hits.iter().find(|(_, hit)| *hit == HeaderHit::Find(part)))
            .unwrap()
            .0;
        (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    };
    // 上一处、下一处：焦点留在输入框。
    let current = |app: &App| app.terminal.find.as_ref().unwrap().current;
    let newest = current(&app).unwrap();
    let (x, y) = center_of(&app, FindPart::Previous);
    click(&mut app, (x, y));
    assert_eq!(current(&app), Some(newest - 1));
    assert_eq!(app.focus, Focus::TerminalFind);
    let (x, y) = center_of(&app, FindPart::Next);
    click(&mut app, (x, y));
    assert_eq!(current(&app), Some(newest));

    // 点在按钮之外的空白：不在下面的终端里开始选字。
    let bar = app.terminal_find_bar().unwrap();
    let area = app.terminal.find.as_ref().unwrap().field.view.area;
    app.focus = Focus::Terminal;
    let selection = app.terminal.tab().unwrap().selection;
    click(&mut app, (area.right() + 2.0, bar.y + 2.0));
    assert_eq!(app.focus, Focus::TerminalFind);
    assert_eq!(app.terminal.tab().unwrap().selection, selection);

    let (x, y) = center_of(&app, FindPart::Close);
    click(&mut app, (x, y));
    assert!(app.terminal.find.is_none());
    assert_eq!(app.focus, Focus::Terminal);
}

/// 查找条四边都有细边框，外面没有阴影。
#[test]
fn the_find_bar_has_a_full_border_and_no_shadow() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "");
    assert!(press(&mut app, &["term"], Char('f')));
    let mut scale = 1.0;
    let renderer = app.frame.renderer.as_mut().unwrap();
    renderer.draw(|frame| scale = frame.scale());
    paint(&mut app);
    let bar = app.terminal_find_bar().unwrap();
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * scale) as usize, (y * scale) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let x = bar.x + bar.w / 2.0;
    let y = bar.y + bar.h / 2.0;
    let background = pixel(x, bar.bottom() + 40.0);
    // 每条边外侧 1 个逻辑像素以内是边框，紧贴着的外面就是面板的底色。
    let half = 0.5;
    for (edge, outside) in [
        (pixel(x, bar.y - half), pixel(x, bar.y - 1.0 - half)),
        (
            pixel(x, bar.bottom() + half),
            pixel(x, bar.bottom() + 1.0 + half),
        ),
        (pixel(bar.x - half, y), pixel(bar.x - 1.0 - half, y)),
        (
            pixel(bar.right() + half, y),
            pixel(bar.right() + 1.0 + half, y),
        ),
    ] {
        assert_ne!(edge, background);
        assert_eq!(outside, background);
    }
}

/// 查找词比输入框长时，滚轮能左右滚动文字，终端不跟着滚；输入以后回到光标处。
#[test]
fn the_wheel_scrolls_a_long_find_query_sideways() {
    use winit::event::MouseScrollDelta::LineDelta;
    let mut app = blank_app(true);
    let output: String = (1..=60).map(|i| format!("line {i}\r\n")).collect();
    let term = show_terminal(&mut app, &output);
    serve(&mut app, &term);
    assert!(press(&mut app, &["term"], Char('f')));
    app.edit_field(|field| {
        field.set_text(&"long query ".repeat(20));
        true
    });
    paint(&mut app);
    fn find(app: &App) -> &crate::field::TextField {
        &app.terminal.find.as_ref().unwrap().field
    }
    let end = find(&app).view.scroll;
    assert!(end > 0.0, "光标在末尾时文字已经滚动");
    let caret = find(&app).caret();
    let area = find(&app).view.area;
    app.on_mouse_move(area.x + 8.0, area.y + area.h / 2.0);
    let lines = app.terminal.tab().unwrap().scroll;

    // 往回滚：光标不动，可以滚出可见范围，终端不动。
    app.on_wheel(LineDelta(0.0, 1.0));
    paint(&mut app);
    let back = find(&app).view.scroll;
    assert!(back < end, "{back} < {end}");
    assert_eq!(find(&app).caret(), caret);
    assert_eq!(app.terminal.tab().unwrap().scroll, lines);
    // 下一帧保持在滚到的位置。
    paint(&mut app);
    assert_eq!(find(&app).view.scroll, back);

    // 滚到头为止：触控板的横向滑动也一样。
    app.on_wheel(LineDelta(1000.0, 0.0));
    paint(&mut app);
    assert_eq!(find(&app).view.scroll, 0.0);
    app.on_wheel(LineDelta(-1000.0, 0.0));
    paint(&mut app);
    assert_eq!(find(&app).view.scroll, end);

    // 滚开以后输入：回到光标处。
    app.on_wheel(LineDelta(0.0, 1000.0));
    paint(&mut app);
    assert_eq!(find(&app).view.scroll, 0.0);
    app.edit_field(|field| {
        // `set_text` 之后文字处于全选状态，先取消选区，否则输入会替换全部文字。
        field.set_caret(field.text().len(), false);
        field.insert("x")
    });
    paint(&mut app);
    assert!(find(&app).view.scroll > end);
}

/// 查找的选项和编辑区相同：默认不区分大小写，可以区分、全字、用正则；正则有误时没有结果。
#[test]
fn find_options_change_what_matches() {
    use crate::app::terminal::{FindPart, HeaderHit};
    let mut app = blank_app(true);
    let term = show_terminal(&mut app, "Error errors error\r\nerr0r\r\n");
    assert!(press(&mut app, &["term"], Char('f')));
    let tab = app.terminal.tab().unwrap();
    let (start, end, id) = (tab.history.0, tab.history.1, tab.id);
    let (start, lines) = term.history(start, end);
    app.terminal_find_loaded(id, start, lines, true);
    app.edit_field(|field| {
        field.set_text("error");
        true
    });
    let count = |app: &App| app.terminal.find.as_ref().unwrap().matches.len();
    assert_eq!(count(&app), 3, "默认不区分大小写，大写开头的词也算");

    let press_part = |app: &mut App, part| {
        paint(app);
        let hits = &app.hits.terminal;
        let rect = hits
            .iter()
            .find(|(_, h)| *h == HeaderHit::Find(part))
            .unwrap()
            .0;
        click(app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    };
    press_part(&mut app, FindPart::Case);
    assert_eq!(count(&app), 2);
    press_part(&mut app, FindPart::Word);
    assert_eq!(count(&app), 1);
    press_part(&mut app, FindPart::Word);
    press_part(&mut app, FindPart::Case);
    assert_eq!(count(&app), 3);
    assert_eq!(app.focus, Focus::TerminalFind);

    press_part(&mut app, FindPart::Regex);
    app.edit_field(|field| {
        field.set_text("err.r$");
        true
    });
    assert_eq!(count(&app), 2);
    app.edit_field(|field| {
        field.set_text("(");
        true
    });
    assert_eq!(count(&app), 0);
    assert!(app.terminal.find.as_ref().unwrap().invalid);
    paint(&mut app);
}

/// 窄面板上放不下选项时不显示，也点不到。
#[test]
fn find_options_hide_on_a_narrow_panel() {
    use crate::app::terminal::{FindPart, HeaderHit};
    let mut app = blank_app(true);
    app.config.terminal_right = Some(true);
    show_terminal(&mut app, "$ ");
    app.terminal.width = 280.0;
    assert!(press(&mut app, &["term"], Char('f')));
    paint(&mut app);
    let hits = &app.hits.terminal;
    assert!(
        hits.iter()
            .all(|(_, h)| *h != HeaderHit::Find(FindPart::Case))
    );
    assert!(
        hits.iter()
            .any(|(_, h)| *h == HeaderHit::Find(FindPart::Close))
    );
}

/// 软换行的内容拼接后查找，匹配可以跨两行；查找条打开期间的新输出也会被查找，当前匹配保持不变。
#[test]
fn find_covers_wrapped_lines_and_new_output() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "");
    let cols = usize::from(app.terminal_size().cols);
    // 「needle」跨在第一行行尾和第二行行首。
    let wrapped = format!("{}needle\r\n", "x".repeat(cols - 3));
    let mut term = add_tab(&mut app, "zsh", &wrapped);
    assert!(press(&mut app, &["term"], Char('f')));
    app.terminal.find.as_mut().unwrap().loaded = true;
    app.edit_field(|field| {
        field.set_text("needle");
        true
    });
    let find = app.terminal.find.as_ref().unwrap();
    assert_eq!(find.matches.len(), 1);
    let (from, to) = find.matches[0];
    assert_eq!((to.line - from.line, from.col, to.col), (1, cols - 3, 3));
    paint(&mut app);

    // 新输出：重新查找后结果增加，当前匹配保持原位置。
    term.feed(b"needle again\r\n");
    let id = app.terminal.tab().unwrap().id;
    app.terminal_message(FromHelper::Frame {
        tab: id,
        frame: term.take_frame().unwrap(),
    });
    app.terminal_find_tick();
    let find = app.terminal.find.as_ref().unwrap();
    assert_eq!(find.matches.len(), 2);
    assert_eq!(find.matches[find.current.unwrap()], (from, to));

    // 间隔内再次输出：暂不查找，但继续请求绘制下一帧，以便间隔结束后查找。
    term.feed(b"needle more\r\n");
    app.terminal_message(FromHelper::Frame {
        tab: id,
        frame: term.take_frame().unwrap(),
    });
    assert!(app.terminal_find_tick());
    assert_eq!(app.terminal.find.as_ref().unwrap().matches.len(), 2);
}

/// 当前匹配绘制在选区之上，可以与其他匹配区分。
#[test]
fn the_current_match_shows_above_the_selection() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "needle\r\n");
    assert!(press(&mut app, &["term"], Char('f')));
    let pixel = |app: &mut App| {
        let mut scale = 1.0;
        let renderer = app.frame.renderer.as_mut().unwrap();
        renderer.draw(|frame| scale = frame.scale());
        paint(app);
        // 第一格左下角：字形碰不到的地方，只有底色。
        let grid = app.terminal_grid();
        let (x, y) = (grid.x + 0.5, grid.y + app.fonts.terminal_line_height - 1.0);
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
        image.pixels[(y * scale) as usize * image.width as usize + (x * scale) as usize]
    };
    app.edit_field(|field| {
        field.set_text("needle");
        true
    });
    let current = pixel(&mut app);
    // 去掉匹配，只留选区：颜色不一样。
    let find = app.terminal.find.as_mut().unwrap();
    find.matches.clear();
    find.current = None;
    assert_ne!(current, pixel(&mut app));
    assert!(app.terminal.tab().unwrap().selection.is_some());
}

/// 查找条开着时滚动条轨道上标出匹配。
#[test]
fn find_results_are_marked_on_the_scrollbar_track() {
    let image = |query: &str| {
        let mut app = blank_app(true);
        show_terminal(&mut app, "needle\r\nother\r\n");
        assert!(press(&mut app, &["term"], Char('f')));
        app.edit_field(|field| {
            field.set_text(query);
            true
        });
        paint(&mut app);
        let renderer = app.frame.renderer.as_ref().unwrap();
        let image = renderer.image().unwrap();
        let (w, h) = (image.width as usize, image.height as usize);
        // 只看窗口右边一条（滚动条轨道），下半部分（内容区）。
        let right = |row: usize| image.pixels[row * w + w - 20..row * w + w].to_vec();
        (h / 2..h).flat_map(right).collect::<Vec<u32>>()
    };
    assert_ne!(image("needle"), image("no-such-text"));
}

/// 当前的匹配落在查找条下面时翻开，不被盖住。
#[test]
fn the_find_bar_does_not_cover_the_current_match() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "");
    let cols = usize::from(app.terminal_size().cols);
    let mut output: String = (0..30).map(|i| format!("before {i}\r\n")).collect();
    output.push_str(&" ".repeat(cols - 8));
    output.push_str("needle\r\n");
    output.extend((0..30).map(|i| format!("after {i}\r\n")));
    let term = add_tab(&mut app, "zsh", &output);
    assert!(press(&mut app, &["term"], Char('f')));
    let tab = app.terminal.tab().unwrap();
    let (start, end) = tab.history;
    let id = tab.id;
    let (start, lines) = term.history(start, end);
    app.terminal_find_loaded(id, start, lines, true);
    app.edit_field(|field| {
        field.set_text("needle");
        true
    });
    paint(&mut app);
    let line = app.terminal.find.as_ref().unwrap().matches[0].0.line;

    // 这一行正好在第一行，再跳一次也是它。
    app.terminal.tab_mut().unwrap().scroll_to(line, 0);
    serve(&mut app, &term);
    app.terminal_find_step(true);
    let bar = app.terminal_find_bar().unwrap();
    let (first, origin) = app.terminal_origin();
    let display = app.terminal.tab().unwrap().display_of(line);
    let y = origin + (display - first) as f32 * app.fonts.terminal_line_height;
    assert!(y >= bar.bottom(), "{y} 被盖住，查找条下沿 {}", bar.bottom());
}

/// 查找条不占标签栏：打开并有了计数之后，标签和「+」都不动。
#[test]
fn the_find_bar_leaves_the_tab_bar_alone() {
    let mut app = blank_app(true);
    let output: String = (1..=40).map(|i| format!("Warning {i}\r\n")).collect();
    show_terminal(&mut app, &output);
    // 标签多到放不下：「+」紧贴右边的按钮。
    for _ in 0..20 {
        let _ = add_tab(&mut app, "cargo", "");
    }
    app.terminal.active = 0;
    let mut scale = 1.0;
    let renderer = app.frame.renderer.as_mut().unwrap();
    renderer.draw(|frame| scale = frame.scale());
    // 标签栏从左边到「+」右边的像素。
    let tabs = |app: &mut App| {
        paint(app);
        let new = (app.hits.terminal.iter())
            .find(|(_, hit)| *hit == terminal::HeaderHit::New)
            .unwrap()
            .0;
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
        let row = |y: f32| {
            let start = (y * scale) as usize * image.width as usize;
            let end = start + (new.right() * scale) as usize;
            image.pixels[start..end].to_vec()
        };
        [new.y + 2.0, new.y + new.h / 2.0, new.bottom() - 2.0].map(row)
    };
    let before = tabs(&mut app);
    assert!(press(&mut app, &["term"], Char('f')));
    app.edit_field(|field| {
        field.set_text("warning");
        true
    });
    assert_eq!(tabs(&mut app), before);
}
