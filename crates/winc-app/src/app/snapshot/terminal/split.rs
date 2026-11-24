//! 拆分的终端标签：标签栏上的拆分按钮，拖动、双击分隔线调整各终端的大小。

use super::*;
use crate::app::terminal::HeaderHit;

/// 标签栏上的拆分按钮请求一个拆分出的终端；拖动分隔线按位置分配两侧的宽度，靠边时停在最小
/// 宽度；双击分隔线恢复平分。
#[test]
fn the_split_button_and_dragged_dividers_size_split_terminals() {
    let dir = sample_folder("terminal-split-resize");
    let mut app = app_with(&dir.join("short.rs"), false);
    let _term = show_terminal(&mut app, "left\r\n");
    let first = app.terminal.tab().unwrap().id;
    paint(&mut app);

    // 标签栏上的拆分按钮：新终端排进当前标签（测试里没有辅助进程建它，撤掉请求后自己放一个）。
    let button = (app.hits.terminal.iter())
        .find(|(_, hit)| *hit == HeaderHit::Split)
        .map(|(rect, _)| *rect)
        .expect("标签栏上有拆分按钮");
    let wanted = app.terminal.want.len();
    click(&mut app, (button.x + 4.0, button.y + 4.0));
    assert_eq!(app.terminal.want_into[wanted..], [Some(first)]);
    app.terminal.want.truncate(wanted);
    app.terminal.want_shells.truncate(wanted);
    app.terminal.want_into.truncate(wanted);
    let size = app.terminal_size();
    crate::app::terminal::insert_pane(&mut app.terminal.tabs, 0, TermTab::new(99, size));
    app.focus_terminal_pane(0);
    paint(&mut app);

    let whole = app.terminal_whole_card();
    // 第一个终端的宽度：分隔线就在它的右边缘。
    let first_width = |app: &App| app.terminal_pane_dividers()[0].x - whole.x;
    let divider = app.terminal_pane_dividers()[0];
    let y = divider.y + divider.h / 2.0;
    app.on_mouse_move(divider.x + 1.0, y);
    assert_eq!(app.pointer.hover, Hover::TerminalSplit(0));

    // 拖到三成处：第一个终端占三成，前台的终端随之变窄。
    let drag_to = |app: &mut App, x: f32| {
        app.pointer.last_click = None;
        app.on_mouse_move(divider.x + 1.0, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x, y);
        app.on_mouse_up(MouseButton::Left);
    };
    drag_to(&mut app, whole.x + whole.w * 0.3);
    assert!(
        (first_width(&app) - whole.w * 0.3).abs() <= 1.0,
        "{}",
        first_width(&app)
    );
    // 按住分隔线不换前台的终端：仍是第一个，它的区域随之变窄。
    assert_eq!(app.terminal.tab().unwrap().pane_at(), 0);
    assert!((app.terminal_content_card().w - whole.w * 0.3).abs() <= 1.0);

    // 拖到最左：停在最小宽度，不会把终端挤没。
    let divider = app.terminal_pane_dividers()[0];
    let y = divider.y + divider.h / 2.0;
    app.pointer.last_click = None;
    app.on_mouse_move(divider.x + 1.0, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(whole.x + 5.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert!(
        (first_width(&app) - 80.0).abs() <= 1.0,
        "{}",
        first_width(&app)
    );

    // 双击分隔线：两侧恢复各占一半。
    let divider = app.terminal_pane_dividers()[0];
    let at = (divider.x + 1.0, divider.y + divider.h / 2.0);
    app.pointer.last_click = None;
    click(&mut app, at);
    click(&mut app, at);
    let shares = app.terminal.tab().unwrap().pane_shares();
    assert!((shares[0] - shares[1]).abs() < 1e-3, "{shares:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拆分经辅助进程建出新终端：之后各终端平分，拖动过的大小不保留；画完一帧给每个终端额度，
/// 不在前台的终端同样能收到新的画面。
#[test]
fn splitting_shares_the_tab_equally_and_every_pane_gets_new_frames() {
    use winc_pty::protocol::ToHelper;
    let dir = sample_folder("terminal-split-equal");
    let mut app = app_with(&dir.join("short.rs"), false);
    let _term = show_terminal(&mut app, "first\r\n");
    let log = app.attach_recording_helper(&dir);
    let split = |app: &mut App| {
        app.split_terminal();
        app.relayout();
        paint(app);
    };
    let equal = |app: &App, n: usize| {
        let shares = app.terminal.tab().unwrap().pane_shares();
        assert_eq!(shares.len(), n);
        assert!(
            shares.iter().all(|&s| (s - shares[0]).abs() < 1e-3),
            "{shares:?}"
        );
    };

    split(&mut app);
    equal(&app, 2);
    // 拖动分隔线后再拆分：三个终端仍然平分。
    let whole = app.terminal_whole_card();
    let divider = app.terminal_pane_dividers()[0];
    let y = divider.y + divider.h / 2.0;
    app.on_mouse_move(divider.x + 1.0, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(whole.x + whole.w * 0.3, y);
    app.on_mouse_up(MouseButton::Left);
    split(&mut app);
    equal(&app, 3);
    let widths: Vec<f32> = (0..3)
        .map(|k| app.with_terminal_pane(k, |app| app.terminal_content_card().w))
        .collect();
    assert!(
        widths.iter().all(|w| (w - widths[0]).abs() <= 2.0),
        "{widths:?}"
    );

    // 三个终端都收到了一帧：画完以后每个都拿到额度。
    let ids: Vec<u16> = (app.terminal.tab().unwrap().split_order().iter())
        .map(|t| t.id)
        .collect();
    let (all, at) = app.terminal.take_group(0);
    let all = all.into_iter().map(|mut t| {
        t.credit_due = true;
        t
    });
    app.terminal.put_group(0, all.collect(), at);
    app.after_terminal_frame();
    let credited: Vec<u16> = (app.sent_to_helper(&log).iter())
        .filter_map(|m| match m {
            ToHelper::Credit { tab } => Some(*tab),
            _ => None,
        })
        .collect();
    for id in ids {
        assert!(credited.contains(&id), "{id} 没有额度：{credited:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 面板在右侧时上下排：每个终端的内容画在自己那一块里，不在前台的终端不会画到最上面那一块。
#[test]
fn stacked_split_terminals_draw_inside_their_own_part() {
    let dir = sample_folder("terminal-split-stacked");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_right = Some(true);
    let _term = show_terminal(&mut app, "top\r\n");
    let size = app.terminal_size();
    crate::app::terminal::insert_pane(&mut app.terminal.tabs, 0, TermTab::new(99, size));
    app.relayout();
    paint(&mut app);

    let divider = app.terminal_pane_dividers()[0];
    // 上面一块的终端区止于分隔线，下面一块的从分隔线下面开始。
    let view = |app: &mut App, k: usize| app.with_terminal_pane(k, |app| app.terminal_view());
    let (top, bottom) = (view(&mut app, 0), view(&mut app, 1));
    assert!(top.bottom() <= divider.y + 0.5, "{top:?} {divider:?}");
    assert!(bottom.y >= divider.bottom() - 0.5, "{bottom:?} {divider:?}");
    let origin = app.with_terminal_pane(1, |app| app.terminal_origin().1);
    assert!((origin - bottom.y).abs() <= 1.0, "{origin} {bottom:?}");
    // 网格和它那一块一样高：不比标签栏以下的部分高，也不会盖住分隔线。
    let grid = app.with_terminal_pane(1, |app| app.terminal_grid());
    assert!(grid.y >= divider.bottom() - 0.5, "{grid:?} {divider:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
