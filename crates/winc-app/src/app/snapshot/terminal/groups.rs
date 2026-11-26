//! 终端分栏：标签移到新的一栏，各栏有自己的标签栏和标签，点哪一栏用哪一栏，标签拖到别的栏或
//! 拖到一栏的边上拆出新的一栏，分隔线调整大小，随现场保存。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry};
use crate::app::terminal::HeaderHit;
use winc_pty::protocol::ToHelper;

/// 面板开着、接着记录消息的辅助进程，有两个标签（编号 1、2），看着第 2 个；返回应用、目录和
/// 记录的文件。
fn grouped(name: &str) -> (App, std::path::PathBuf, std::path::PathBuf) {
    let dir = sample_folder(name);
    let mut app = app_with(&dir.join("short.rs"), false);
    let _term = show_terminal(&mut app, "left\r\n");
    let _ = add_tab(&mut app, "cargo", "right\r\n");
    let log = app.attach_recording_helper(&dir);
    paint(&mut app);
    (app, dir, log)
}

/// 把正在看的标签移到新的一栏，再画一帧。
fn move_to_group(app: &mut App) {
    app.move_terminal_to_new_group();
    paint(app);
}

/// 各栏的标签编号，按排列顺序。
fn tab_ids(app: &App) -> Vec<Vec<u16>> {
    let t = &app.terminal;
    (0..t.group_count())
        .map(|at| t.group_tabs(at).iter().map(|tab| tab.id).collect())
        .collect()
}

/// 第 `at` 栏标签栏上 `hit` 的位置。
fn group_hit(app: &App, at: usize, hit: HeaderHit) -> Option<Rect> {
    let hits = &app.hits.terminal_groups[at];
    hits.iter().find(|(_, h)| *h == hit).map(|(rect, _)| *rect)
}

/// 右键点在 `(x, y)`，返回菜单里「移到右侧新分栏」能不能用；菜单留着。
fn move_item(app: &mut App, (x, y): (f32, f32)) -> Option<bool> {
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Right);
    app.on_mouse_up(MouseButton::Right);
    let menu = app.overlays.context_menu.as_ref()?;
    menu.entries.iter().find_map(|e| match e {
        MenuEntry::Item(MenuCommand::MoveTerminalRight, on) => Some(*on),
        _ => None,
    })
}

/// 按住第 `group` 栏的第 `index` 个标签，拖出标签栏到 `to` 松开。
fn drag_tab(app: &mut App, group: usize, index: usize, to: (f32, f32)) {
    let tab = group_hit(app, group, HeaderHit::Tab(index)).unwrap();
    let from = (tab.x + 10.0, tab.y + tab.h / 2.0);
    app.pointer.last_click = None;
    app.on_mouse_move(from.0, from.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(from.0, from.1 + 30.0);
    app.on_mouse_move(to.0, to.1);
    paint(app);
    app.on_mouse_up(MouseButton::Left);
    paint(app);
}

/// 只有一个标签时谈不上分栏：标签和终端里的「移到右侧新分栏」都灰着，命令也不动。
#[test]
fn a_lone_tab_cannot_move_to_a_new_group() {
    let dir = sample_folder("terminal-groups-lone");
    let mut app = app_with(&dir.join("short.rs"), false);
    let _term = show_terminal(&mut app, "only\r\n");
    paint(&mut app);
    let tab = app
        .hits
        .terminal
        .iter()
        .find(|(_, h)| *h == HeaderHit::Tab(0));
    let tab = tab.map(|(rect, _)| *rect).unwrap();
    assert_eq!(
        move_item(&mut app, (tab.x + 10.0, tab.y + tab.h / 2.0)),
        Some(false)
    );
    app.overlays.context_menu = None;
    let grid = app.terminal_view();
    assert_eq!(
        move_item(&mut app, (grid.x + 20.0, grid.y + 10.0)),
        Some(false)
    );
    app.overlays.context_menu = None;
    app.move_terminal_to_new_group();
    assert!(app.terminal.groups.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 「移到右侧新分栏」把这个标签挪到右边新的一栏，不新开终端；各栏有自己的标签栏，面板的按钮
/// 只在最后一栏；两栏的终端都按自己那一栏变窄；点哪一栏就用哪一栏，一栏的最后一个标签关了只
/// 去掉这一栏。
#[test]
fn moving_a_tab_to_a_new_group_keeps_the_terminal() {
    let (mut app, dir, log) = grouped("terminal-groups-move");
    let whole = app.terminal_size();
    // 右键点在第一个标签上：移走的是它，不是正在看的那个。
    let tab = group_hit(&app, 0, HeaderHit::Tab(0)).unwrap();
    assert_eq!(
        move_item(&mut app, (tab.x + 10.0, tab.y + tab.h / 2.0)),
        Some(true)
    );
    app.run_menu_command(MenuCommand::MoveTerminalRight);
    paint(&mut app);
    assert_eq!(tab_ids(&app), [vec![2], vec![1]]);
    assert_eq!(app.terminal.group_at, 1);
    let cards = app.terminal_group_cards_in(app.layout.terminal);
    assert!(cards[1].x > cards[0].right(), "{cards:?}");

    // 两栏各有自己的标签；收起面板等按钮只在最后一栏，拆分每栏都有。
    assert!(group_hit(&app, 0, HeaderHit::Tab(0)).is_some());
    assert!(group_hit(&app, 1, HeaderHit::Tab(0)).is_some());
    assert!(group_hit(&app, 0, HeaderHit::Hide).is_none());
    assert!(group_hit(&app, 1, HeaderHit::Hide).is_some());
    assert!(group_hit(&app, 0, HeaderHit::Split).is_some());

    // 没有新开终端；大小稳定后两个终端都按自己那一栏变窄，告诉了辅助进程。
    settle_terminal_size(&mut app);
    let sent = app.sent_to_helper(&log);
    assert!(
        !sent.iter().any(|m| matches!(m, ToHelper::Spawn { .. })),
        "{sent:?}"
    );
    for id in [1, 2] {
        let resized = sent.iter().any(
            |m| matches!(m, ToHelper::Resize { tab, size } if *tab == id && size.cols < whole.cols),
        );
        assert!(resized, "{id} {sent:?}");
    }

    // 点第一栏的标签：改用第一栏。
    let tab = group_hit(&app, 0, HeaderHit::Tab(0)).unwrap();
    click(&mut app, (tab.x + 10.0, tab.y + tab.h / 2.0));
    assert_eq!(app.terminal.group_at, 0);
    assert_eq!(app.terminal.tab().map(|t| t.id), Some(2));

    // 关掉第二栏唯一的标签：第二栏去掉，面板还开着。
    app.focus_terminal_group(1);
    app.close_terminal_pane();
    assert_eq!(tab_ids(&app), [vec![2]]);
    assert!(app.terminal.open);
    assert!(app.terminal.groups.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拖出标签栏的标签：拖到一栏的边上拆出新的一栏，拖到别的栏的标签栏上插在松开处，拖到别的栏
/// 中部放进去；原来那一栏空了就去掉。一栏唯一的标签拖到它自己的边上不变。
#[test]
fn tabs_dragged_out_of_the_strip_split_or_join_groups() {
    let (mut app, dir, _log) = grouped("terminal-groups-drag");
    let card = app.terminal_group_cards_in(app.layout.terminal)[0];
    let body = |card: Rect, r: f32| (card.x + card.w * r, card.y + card.h * 0.6);
    // 贴着一栏左边或右边的一处（离边 16 逻辑像素，在拆分的一条之内）。
    let edge = |card: Rect, right: bool| {
        let x = if right {
            card.right() - 16.0
        } else {
            card.x + 16.0
        };
        (x, card.y + card.h * 0.6)
    };

    // 拖到右边：拆出新的一栏。
    drag_tab(&mut app, 0, 1, edge(card, true));
    assert_eq!(tab_ids(&app), [vec![1], vec![2]]);
    assert_eq!(app.terminal.group_at, 1);

    // 第一栏唯一的标签拖到它自己的左边：不变。
    let cards = app.terminal_group_cards_in(app.layout.terminal);
    drag_tab(&mut app, 0, 0, edge(cards[0], false));
    assert_eq!(tab_ids(&app), [vec![1], vec![2]]);

    // 拖到第一栏的标签栏上、第一个标签前面：插在那里，第二栏空了就去掉。
    let first = group_hit(&app, 0, HeaderHit::Tab(0)).unwrap();
    drag_tab(&mut app, 1, 0, (first.x + 2.0, first.y + first.h / 2.0));
    assert_eq!(tab_ids(&app), [vec![2, 1]]);
    assert_eq!(app.terminal.tab().map(|t| t.id), Some(2));

    // 再拆开，然后拖到另一栏中部：放进那一栏的最后。
    drag_tab(&mut app, 0, 0, edge(card, true));
    assert_eq!(tab_ids(&app), [vec![1], vec![2]]);
    let cards = app.terminal_group_cards_in(app.layout.terminal);
    drag_tab(&mut app, 1, 0, body(cards[0], 0.5));
    assert_eq!(tab_ids(&app), [vec![1, 2]]);
    assert!(!app.terminal_tab_dragged_out());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拖动分栏的分隔线按位置分配两栏的宽度，双击恢复平分。
#[test]
fn the_group_divider_resizes_groups() {
    let (mut app, dir, _log) = grouped("terminal-groups-divider");
    move_to_group(&mut app);
    let card = app.layout.terminal;
    let divider = app.terminal_group_dividers()[0];
    let y = divider.y + divider.h / 2.0;
    app.on_mouse_move(divider.x + 1.0, y);
    assert_eq!(app.pointer.hover, Hover::TerminalGroupSplit(0));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(card.x + card.w * 0.7, y);
    app.on_mouse_up(MouseButton::Left);
    let first = app.terminal_group_cards_in(card)[0];
    assert!((first.w - card.w * 0.7).abs() <= 1.0, "{first:?}");
    // 按住分隔线不改用哪一栏。
    assert_eq!(app.terminal.group_at, 1);

    let divider = app.terminal_group_dividers()[0];
    let at = (divider.x + 1.0, y);
    app.pointer.last_click = None;
    click(&mut app, at);
    click(&mut app, at);
    let shares = app.terminal.panel_shares();
    assert!((shares[0] - shares[1]).abs() < 1e-3, "{shares:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 不在用的一栏里的 shell 退出了：在那一栏关掉它，那一栏空了就去掉，正在用的一栏不变。
#[test]
fn a_shell_exiting_in_another_group_closes_it_there() {
    let (mut app, dir, _log) = grouped("terminal-groups-exit");
    move_to_group(&mut app);
    app.focus_terminal_group(0);
    app.terminal_message(FromHelper::Exited { tab: 2, code: 0 });
    assert_eq!(tab_ids(&app), [vec![1]]);
    assert_eq!(app.terminal.tab().map(|t| t.id), Some(1));
    assert!(app.terminal.open);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 现场记下各栏的标签数和份额；恢复时各标签回到原来那一栏，还看着原来看的那一个。
#[test]
fn groups_are_kept_in_the_session() {
    let (mut app, dir, _log) = grouped("terminal-groups-session");
    move_to_group(&mut app);
    let session = app.terminal_session_with_tabs(&[]).expect("有终端");
    assert_eq!(session.tabs.len(), 2);
    assert_eq!(session.active, 1);
    let counts: Vec<usize> = session.groups.iter().map(|g| g.tabs).collect();
    assert_eq!(counts, [1, 1]);

    app.close_all_terminal_tabs();
    assert!(app.terminal.groups.is_empty());
    app.restore_terminal(&session);
    // 测试里没有真的辅助进程：恢复时启动不了，换成记录消息的。
    app.terminal.error = None;
    let _log = app.attach_recording_helper(&dir);
    app.relayout();
    paint(&mut app);
    let ids = tab_ids(&app);
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.iter().all(|tabs| tabs.len() == 1), "{ids:?}");
    assert_eq!(app.terminal.group_at, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 分栏的样子：面板在下方时左右两栏，在右侧时上下两栏；再拖着一个标签到另一栏的末端。
///
/// 运行：`cargo test -p winc-app shoot_terminal_groups -- --ignored`，图片写到
/// `WINC_SNAPSHOT_DIR`（默认为临时目录下的 `winc-snapshots`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_terminal_groups() {
    let dir = sample_folder("terminal-groups-shot");
    let prompt = "\x1b[34m~/winc\x1b[0m \x1b[35mmain\x1b[0m ❯ ls\r\nCargo.toml  src\r\n\x1b[34m~/winc\x1b[0m ❯ ";
    for right in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), false);
        app.config.terminal_right = Some(right);
        show_terminal(&mut app, prompt);
        let _ = add_tab(&mut app, "cargo", prompt);
        let _ = add_tab(&mut app, "htop", prompt);
        app.move_terminal_to_new_group();
        app.focus_terminal_group(0);
        paint(&mut app);
        let side = if right { "right" } else { "bottom" };
        shoot(&mut app, &format!("terminal-groups-{side}"));

        // 按住第一栏的第一个标签，拖到第二栏的末端：标出要拆出的那一半。
        let tab = group_hit(&app, 0, HeaderHit::Tab(0)).unwrap();
        let card = app.terminal_group_cards_in(app.layout.terminal)[1];
        app.on_mouse_move(tab.x + 10.0, tab.y + tab.h / 2.0);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(tab.x + 10.0, tab.y + 60.0);
        app.on_mouse_move(card.right() - 30.0, card.bottom() - 30.0);
        shoot(&mut app, &format!("terminal-groups-drag-{side}"));
        app.on_mouse_up(MouseButton::Left);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 「终端中的下一个窗格」走过当前标签拆分出的各个终端后进下一栏，到最后一栏后回到第一栏；
/// 「上一个」反过来，进上一栏时从它最后一个终端开始。
#[test]
fn the_next_pane_continues_into_the_next_group() {
    let (mut app, dir, _log) = grouped("terminal-groups-cycle");
    move_to_group(&mut app);
    app.focus_terminal_group(0);
    let size = app.terminal_size();
    crate::app::terminal::insert_pane(&mut app.terminal.tabs, 0, TermTab::new(99, size));
    app.focus_terminal_pane(0);
    let here = |app: &App| (app.terminal.group_at, app.terminal.tab().map(|t| t.id));
    assert_eq!(here(&app), (0, Some(1)));
    app.cycle_terminal_pane(true);
    assert_eq!(here(&app), (0, Some(99)));
    app.cycle_terminal_pane(true);
    assert_eq!(here(&app), (1, Some(2)));
    app.cycle_terminal_pane(true);
    assert_eq!(here(&app), (0, Some(1)));
    // 往回：进上一栏（绕到最后一栏），再回到第一栏时从它最后一个终端开始。
    app.cycle_terminal_pane(false);
    assert_eq!(here(&app), (1, Some(2)));
    app.cycle_terminal_pane(false);
    assert_eq!(here(&app), (0, Some(99)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拖着标签切到别的应用（收不到松开）：拖动结束，标签留在原来那一栏。
#[test]
fn losing_focus_cancels_a_tab_drag() {
    let (mut app, dir, _log) = grouped("terminal-groups-cancel");
    let tab = group_hit(&app, 0, HeaderHit::Tab(1)).unwrap();
    let card = app.terminal_group_cards_in(app.layout.terminal)[0];
    app.on_mouse_move(tab.x + 10.0, tab.y + tab.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(card.right() - 20.0, card.bottom() - 20.0);
    assert!(app.terminal_tab_dragged_out());
    app.drop_drags();
    assert!(!app.terminal_tab_dragged_out());
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(tab_ids(&app), [vec![1, 2]]);
    let _ = std::fs::remove_dir_all(&dir);
}
