//! 终端面板的测试：与编辑区分配空间、焦点、拖动改高度、终端里生效的快捷键、多个标签、翻回滚、
//! 选字和复制、查找、绘制结果，以及（手动运行的）连上真实的辅助进程执行命令。

mod blocks;
mod editor;
mod run;

use super::editor::navigation::press;
use super::*;
use crate::app::terminal::{self, TermTab};
use crate::keymap::KeyCode::Char;
use winc_pty::protocol::{Event, FromHelper};

/// 本地仿真按开启 shell 集成处理时，集成脚本在 OSC 133 标记中带回的随机数。
///
/// 取 16 位十六进制字符：协议要求命令行带回集成脚本启动时收到的值，长度不影响解析，用固定值即可。
const NONCE: &str = "0123456789abcdef";

/// 打开终端面板，屏幕内容是本地终端仿真读入 `bytes` 之后的结果（不启动辅助进程）。
pub(super) fn show_terminal(app: &mut App, bytes: &str) -> winc_pty::Terminal {
    app.config.terminal_enabled = Some(true);
    app.terminal.open = true;
    app.terminal.height = terminal::DEFAULT_HEIGHT;
    app.relayout();
    let term = add_tab(app, "zsh", bytes);
    app.focus = Focus::Terminal;
    term
}

/// 新开一个本地仿真的标签并切换到它，返回该仿真。
///
/// 测试用返回的仿真响应取历史行的请求，见 [`serve`]。
pub(super) fn add_tab(app: &mut App, process: &str, bytes: &str) -> winc_pty::Terminal {
    let size = app.terminal_size();
    let mut term = winc_pty::Terminal::new(usize::from(size.cols), usize::from(size.rows), 1000);
    term.set_nonce(NONCE.into());
    term.feed(bytes.as_bytes());
    let id = app.terminal.tabs.len() as u16 + 1;
    let mut tab = TermTab::new(id, size);
    tab.apply(term.take_frame().unwrap());
    tab.process = process.into();
    app.terminal.tabs.push(tab);
    app.terminal.active = app.terminal.tabs.len() - 1;
    term
}

/// 测试里没有辅助进程：正在看的标签缺少历史行时，从本地仿真中取出补上，等同于收到辅助进程的
/// 回答。
pub(super) fn serve(app: &mut App, term: &winc_pty::Terminal) {
    let tab = app.terminal.tab_mut().unwrap();
    if let Some((start, end)) = tab.missing() {
        let (start, lines) = term.history(start, end);
        tab.store_history(start, lines);
    }
}

/// 网格上第 `row` 行第 `col` 格的中心。
pub(super) fn cell_center(app: &App, row: usize, col: usize) -> (f32, f32) {
    let grid = app.terminal_grid();
    (
        grid.x + (col as f32 + 0.5) * app.cell,
        grid.y + (row as f32 + 0.5) * app.fonts.line_height,
    )
}

pub(super) use super::click;

#[test]
fn only_messages_for_what_is_shown_need_a_repaint() {
    let dir = sample_folder("terminal-shown");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "");
    add_tab(&mut app, "bash", "");
    let tabs: Vec<u16> = app.terminal.tabs.iter().map(|tab| tab.id).collect();
    let (hidden, active) = (tabs[0], tabs[1]);
    let blocks = |tab| FromHelper::Blocks {
        tab,
        blocks: Vec::new(),
        early: false,
    };
    let t = &app.terminal;
    assert!(t.message_is_shown(&blocks(active)));
    assert!(!t.message_is_shown(&blocks(hidden)));
    let bell = FromHelper::Event {
        tab: active,
        event: Event::Bell,
    };
    assert!(!t.message_is_shown(&bell));

    // 面板收起后，正在看的标签也不显示。
    app.terminal.open = false;
    assert!(!app.terminal.message_is_shown(&blocks(active)));
}

#[test]
fn the_visible_state_follows_what_the_tab_strip_and_status_bar_show() {
    let dir = sample_folder("terminal-visible");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "");
    add_tab(&mut app, "bash", "");
    let hidden = app.terminal.tabs[0].id;
    let title = |title: &str| FromHelper::Event {
        tab: hidden,
        event: Event::Title(title.to_owned()),
    };

    app.terminal_message(title("build"));
    let before = app.terminal.visible_state();
    // 标题没有变化：不需要重绘。
    app.terminal_message(title("build"));
    assert_eq!(app.terminal.visible_state(), before);
    // 后台标签的名字显示在标签栏上。
    app.terminal_message(title("test"));
    assert_ne!(app.terminal.visible_state(), before);

    // 面板收起后，标签名不可见，未读标记仍然显示在状态栏上。
    app.terminal.open = false;
    let closed = app.terminal.visible_state();
    app.terminal_message(title("spinner"));
    assert_eq!(app.terminal.visible_state(), closed);
    app.terminal.tabs[0].unseen = Some(false);
    assert_ne!(app.terminal.visible_state(), closed);
}

#[test]
fn the_panel_takes_its_room_from_the_editor_card() {
    let dir = sample_folder("terminal-layout");
    let mut app = app_with(&dir.join("short.rs"), false);
    let before = app.layout.editor;

    show_terminal(&mut app, "");
    let l = app.layout;
    assert_eq!(l.terminal.h, terminal::DEFAULT_HEIGHT);
    assert_eq!(l.terminal.bottom(), before.bottom());
    assert_eq!(l.terminal.x, before.x);
    assert_eq!(l.terminal.w, before.w);
    assert_eq!(l.editor.bottom() + ISLAND_GAP, l.terminal.y);
    assert_eq!(l.text.h, l.editor.h);
    assert_eq!(l.terminal_divider.y, l.editor.bottom());

    // 网格放得下的列数行数和面板一致。
    let size = app.terminal_size();
    let grid = terminal::grid_rect(l.terminal, app.fonts.line_height);
    assert_eq!(f32::from(size.cols), (grid.w / app.cell).floor());
    assert_eq!(
        f32::from(size.rows),
        (grid.h / app.fonts.line_height).floor()
    );

    // 窗口矮到放不下时面板不出现，编辑区保持原样。
    app.terminal.height = 10_000.0;
    app.relayout();
    assert!(app.layout.terminal.h <= l.editor.h + l.terminal.h);
    assert!(app.layout.editor.h >= terminal::EDITOR_MIN);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clicks_move_the_focus_and_the_gap_resizes_the_panel() {
    let dir = sample_folder("terminal-mouse");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    app.focus = Focus::Editor;
    paint(&mut app);

    let card = app.layout.terminal;
    app.on_mouse_move(card.x + 100.0, card.y + 60.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.focus, Focus::Terminal);

    let text = app.layout.text;
    app.on_mouse_move(text.x + 40.0, text.y + 10.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.focus, Focus::Editor);

    // 在两张卡片之间的间距上向上拖 50 逻辑像素：面板高度增加 50。
    let gap = app.layout.terminal_divider;
    let (x, y) = (gap.x + 200.0, gap.y + gap.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::TerminalDivider);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y - 50.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.layout.terminal.h, card.h + 50.0);
    assert_eq!(app.terminal.height, card.h + 50.0);

    // 拖过头时停在给编辑区留的最小高度上，记下的也是这个高度。
    app.on_mouse_move(x, y - 50.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, 0.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.layout.editor.h, terminal::EDITOR_MIN);
    assert_eq!(app.terminal.height, app.layout.terminal.h);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn only_command_shortcuts_reach_past_the_terminal() {
    let dir = sample_folder("terminal-keys");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");

    // 编辑类的命令改的是看不见的文档，交给终端：⌘Z（其他系统 Ctrl+Z）不撤销编辑区。
    assert!(!press(&mut app, &["cmd"], Char('z')));
    if cfg!(target_os = "macos") {
        // ⌃ 组合交给 shell：⌃⇧G（显示提交）在终端里不是快捷键。
        assert!(!press(&mut app, &["ctrl", "shift"], Char('g')));
        // 其余带 ⌘ 的全局快捷键仍然生效：⌘B 收起侧栏。
        let shown = app.panel_visible;
        assert!(press(&mut app, &["cmd"], Char('b')));
        assert_ne!(app.panel_visible, shown);
        assert_eq!(app.focus, Focus::Terminal, "收起侧栏不动终端的焦点");
    } else {
        // 单按 Ctrl 的组合交给 shell：Ctrl+B（收起侧栏）在终端里不是快捷键。
        assert!(!press(&mut app, &["ctrl"], Char('b')));
        // 带 Ctrl+Shift 的全局快捷键仍然生效：Ctrl+Shift+E 显示文件树（焦点随之到文件树）。
        app.sidebar_view = SidebarView::Search;
        assert!(press(&mut app, &["ctrl", "shift"], Char('e')));
        assert_eq!(app.sidebar_view, SidebarView::Files);
        app.focus = Focus::Terminal;
    }
    // ⌘K（其他系统 Ctrl+Shift+K）在终端里是清屏，不是组合键的第一段。
    assert!(press(&mut app, &["term"], Char('k')));
    assert!(app.pending_chord.is_none());

    // ⌃` 在终端里也认：焦点在终端时收起面板，焦点回编辑区；再按一次又打开。
    assert!(press(&mut app, &["ctrl"], Char('`')));
    assert!(!app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);
    assert!(press(&mut app, &["ctrl"], Char('`')));
    assert!(app.terminal.open);
    assert_eq!(app.focus, Focus::Terminal);

    // 模块关着时 ⌃` 不是快捷键，什么也不发生。
    app.stop_terminal();
    app.config.terminal_enabled = Some(false);
    assert!(!press(&mut app, &["ctrl"], Char('`')));
    assert!(!app.terminal.open);
    assert_eq!(app.layout.terminal.h, 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tabs_switch_close_and_take_the_panel_with_the_last_one() {
    let dir = sample_folder("terminal-tabs");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "one");
    let _ = add_tab(&mut app, "cargo", "two");
    let _ = add_tab(&mut app, "vim", "three");
    assert_eq!(app.terminal.active, 2);

    // ⌘⇧] 和 ⌘⇧[（其他系统 Ctrl+PageDown、Ctrl+PageUp）在终端里切终端的标签，到头绕回去；
    // 编辑区的标签不动。
    let step = |app: &mut App, next: bool| {
        if cfg!(target_os = "macos") {
            press(app, &["cmd", "shift"], Char(if next { ']' } else { '[' }))
        } else {
            let key = if next {
                crate::keymap::KeyCode::PageDown
            } else {
                crate::keymap::KeyCode::PageUp
            };
            press(app, &["ctrl"], key)
        }
    };
    let doc = app.editor.active;
    assert!(step(&mut app, true));
    assert_eq!(app.terminal.active, 0);
    assert!(step(&mut app, false));
    assert_eq!(app.terminal.active, 2);
    assert_eq!(app.editor.active, doc);

    // 点标签栏上的标签和关闭按钮。
    paint(&mut app);
    let hit = |app: &App, want: terminal::HeaderHit| {
        let rect = app
            .terminal
            .hits
            .iter()
            .find(|(_, h)| *h == want)
            .unwrap()
            .0;
        (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    };
    let at = hit(&app, terminal::HeaderHit::Tab(1));
    click(&mut app, at);
    assert_eq!(app.terminal.active, 1);
    paint(&mut app);
    let at = hit(&app, terminal::HeaderHit::Close(1));
    click(&mut app, at);
    let names: Vec<&str> = app.terminal.tabs.iter().map(|t| t.label()).collect();
    assert_eq!(names, ["zsh", "vim"]);
    assert_eq!(app.terminal.active, 1);

    // ⌘W（其他系统 Ctrl+Shift+W）关终端的标签，不关编辑区的；最后一个关了面板收起，焦点回
    // 编辑区。
    let documents = app.editor.documents.len();
    assert!(press(&mut app, &["term"], Char('w')));
    assert!(press(&mut app, &["term"], Char('w')));
    assert_eq!(app.editor.documents.len(), documents);
    assert!(app.terminal.tabs.is_empty());
    assert!(!app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.layout.terminal.h, 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_cursor_waits_for_the_prompt_after_return() {
    let dir = sample_folder("terminal-settle");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut term = show_terminal(&mut app, "~/winc $ ls");
    let mut step = |app: &mut App, bytes: &str| {
        term.feed(bytes.as_bytes());
        let frame = term.take_frame().unwrap();
        app.terminal.tab_mut().unwrap().apply(frame);
        app.terminal.tab().unwrap().settle.is_some()
    };

    // 回车（或 ⌃C）：shell 先换行，光标位于空行行首，此时不绘制光标；提示符出现以后再绘制。
    assert!(step(&mut app, "\r\n"));
    assert!(!step(&mut app, "~/winc $ "));

    // 等不到提示符：超过 `SETTLE` 以后仍然在行首绘制光标。
    assert!(step(&mut app, "\r\n"));
    let later = Instant::now() + terminal::SETTLE;
    assert_eq!(app.terminal_settle_tick(later), None);
    assert!(app.terminal.tab().unwrap().settle.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_wheel_scrolls_back_through_the_history() {
    let dir = sample_folder("terminal-scroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    let output: String = (1..=60).map(|i| format!("line {i}\r\n")).collect();
    let term = show_terminal(&mut app, &output);
    let rows = app.terminal.tab().unwrap().rows.len();
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.max_scroll(), 61 - rows);

    // 往上滚三行：看见的第一行往前三行，光标不画（不在可见范围）。
    let card = app.layout.terminal;
    app.on_mouse_move(card.x + 100.0, card.y + 80.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 1.0));
    serve(&mut app, &term);
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.scroll, 3);
    let first = tab.line(tab.top()).unwrap().trimmed().to_string();
    assert_eq!(first, format!("line {}", 61 - rows - 3 + 1));
    paint(&mut app);

    // 滚过头停在最早的一行；往下滚回到最新。
    for _ in 0..50 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, 1.0));
        serve(&mut app, &term);
    }
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.top(), tab.history.0);
    assert_eq!(tab.line(tab.top()).unwrap().trimmed(), "line 1");
    for _ in 0..50 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    }
    assert_eq!(app.terminal.tab().unwrap().scroll, 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dragging_selects_and_command_c_copies() {
    let dir = sample_folder("terminal-select");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(
        &mut app,
        "$ cargo build\r\nerror: src/main.rs:12:5 failed\r\n$ ",
    );
    paint(&mut app);

    // 从第 0 行第 2 格拖到第 1 行第 4 格（按两格之间的缝隙判定）。
    let (x0, y0) = cell_center(&app, 0, 2);
    let (x1, y1) = cell_center(&app, 1, 4);
    app.on_mouse_move(x0 - app.cell / 2.0, y0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x1 + app.cell / 2.0, y1);
    app.on_mouse_up(MouseButton::Left);
    assert!(press(&mut app, &["term"], Char('c')));
    assert_eq!(app.copied.as_deref(), Some("cargo build\nerror"));

    // 双击选中整个 `文件:行:列`。
    let at = cell_center(&app, 1, 12);
    click(&mut app, at);
    click(&mut app, at);
    assert!(press(&mut app, &["term"], Char('c')));
    assert_eq!(app.copied.as_deref(), Some("src/main.rs:12:5"));

    // 三击选中整行；单击清掉选区，⌘C 什么也不复制。
    click(&mut app, at);
    assert!(press(&mut app, &["term"], Char('c')));
    assert_eq!(
        app.copied.as_deref(),
        Some("error: src/main.rs:12:5 failed")
    );
    let blank = cell_center(&app, 2, 0);
    click(&mut app, blank);
    assert!(app.terminal.tab().unwrap().selection.is_none());

    // ⌘A 选中全部。
    assert!(press(&mut app, &["term"], Char('a')));
    assert!(press(&mut app, &["term"], Char('c')));
    assert_eq!(
        app.copied.as_deref(),
        Some("$ cargo build\nerror: src/main.rs:12:5 failed\n$")
    );
    paint(&mut app);

    std::fs::remove_dir_all(&dir).unwrap();
}

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

#[test]
fn the_panel_keeps_the_dragged_height_without_a_gap_under_the_tabs() {
    let dir = sample_folder("terminal-no-gap");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    let line = app.fonts.line_height;
    let height = app.layout.terminal.h + (line / 2.0).round();
    app.terminal.height = height;
    app.relayout();
    assert_eq!(app.layout.terminal.h, height);

    // 上面没有内容：内容贴着标签栏，零头留在下面。
    let top = app.terminal_view().y;
    assert_eq!(app.terminal_grid().y, top);

    // 上面有内容：网格贴着面板下边，零头处显示上一行的下半截。
    let output: String = (0..60).map(|i| format!("line {i}\r\n")).collect();
    let term = add_tab(&mut app, "zsh", &output);
    serve(&mut app, &term);
    let grid = app.terminal_grid();
    assert!(grid.y > top);
    assert!(grid.y - top < line);
    let tab = app.terminal.tab().unwrap();
    let above = tab.row_at(tab.view_top() - 1).unwrap();
    let terminal::ViewRow::Line(number) = above else {
        panic!("上一行应当是普通的行");
    };
    assert!(tab.line(number).is_some(), "上一行要先取回来");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_wheel_moves_rows_by_pixels_and_clicks_follow_them() {
    let dir = sample_folder("terminal-pixel-scroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "");
    app.terminal.height = terminal::DEFAULT_HEIGHT + 7.0;
    app.relayout();
    let output: String = (0..80).map(|i| format!("line {i}\r\n")).collect();
    let term = add_tab(&mut app, "zsh", &output);
    app.relayout();
    serve(&mut app, &term);
    let lh = app.fonts.line_height;

    // 每次滚动 3 逻辑像素：同一行每次都移动 3 逻辑像素。
    let y_of = |app: &App, row: u64| {
        let (first, origin) = app.terminal_origin();
        origin + (row as f32 - first as f32) * lh
    };
    let row = app.terminal.tab().unwrap().view_top();
    let mut last = y_of(&app, row);
    for _ in 0..40 {
        app.terminal_wheel(3.0);
        serve(&mut app, &term);
        let y = y_of(&app, row);
        assert!((y - last - 3.0).abs() < 1.01, "移动了 {}", y - last);
        last = y;
    }

    // 点在视口最上面只露出一部分的行上：选中的就是这一行。
    let tab = app.terminal.tab().unwrap();
    let (first, origin) = app.terminal_origin();
    let top = app.terminal_view().y;
    assert!(origin < top);
    let line = tab.row_at(first).unwrap().line(tab);
    let x = app.terminal_view().x + app.cell * 0.5;
    app.on_mouse_move(x, top + 0.5);
    app.on_mouse_down(MouseButton::Left);
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.selection.map(|s| s.anchor.line), Some(line));
    app.on_mouse_up(MouseButton::Left);

    // 翻到顶后不再动，第一行贴着标签栏；键入回到最新的输出，不留不足一行的滚动量。
    app.terminal_wheel(1.0e7);
    assert_eq!(app.terminal.tab().unwrap().viewport(), (0, 0.0));
    assert_eq!(app.terminal_origin().1, top);
    app.terminal_wheel(-lh * 0.5);
    let tab = app.terminal.tab().unwrap();
    assert!(tab.scroll_frac > 0.0 && tab.view_top() > 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_tab_bar_keeps_even_gaps_around_tabs_and_buttons() {
    let dir = sample_folder("terminal-tab-bar-gaps");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "");
    for _ in 0..20 {
        add_tab(&mut app, "zsh", "");
    }
    app.terminal.active = 0;
    paint(&mut app);
    let card = app.layout.terminal;
    let hit = |app: &App, wanted: terminal::HeaderHit| {
        (app.terminal.hits.iter())
            .find(|(_, hit)| *hit == wanted)
            .map(|(rect, _)| *rect)
            .unwrap()
    };

    // 标签放不下：第一个标签上、下、左的留白相同。
    let tab = hit(&app, terminal::HeaderHit::Tab(0));
    assert_eq!(tab.x - card.x, tab.y - card.y);
    assert_eq!(tab.x - card.x, app.terminal_view().y - tab.bottom());
    assert_eq!(tab.x - card.x, 5.0);

    // 「+」、运行、切换位置、收起四个按钮间距相同，尺寸和间距与编辑区标签栏一致。
    let new = hit(&app, terminal::HeaderHit::New);
    let run = hit(
        &app,
        terminal::HeaderHit::Run(terminal::run::RunButton::Start),
    );
    let position = hit(&app, terminal::HeaderHit::Position);
    let hide = hit(&app, terminal::HeaderHit::Hide);
    assert_eq!(run.x - new.right(), crate::theme::TAB_GAP);
    assert_eq!(position.x - run.right(), crate::theme::TAB_GAP);
    assert_eq!(hide.x - position.right(), crate::theme::TAB_GAP);
    assert_eq!(new.w, crate::theme::BUTTON);
    assert_eq!(card.right() - hide.right(), 8.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal() {
    let dir = sample_folder("terminal-shot");
    let screen = concat!(
        "\x1b[1;32m➜\x1b[0m  \x1b[36mwinc\x1b[0m \x1b[34mgit:(\x1b[31mmain\x1b[34m)\x1b[0m ls\r\n",
        "Cargo.toml  \x1b[35mlink\x1b[0m@  README.md  \x1b[31mrun.sh\x1b[0m*  \x1b[1;34msrc\x1b[0m/\r\n",
        "\x1b[1;31merror\x1b[0m\x1b[1m: 未找到文件\x1b[0m \x1b[4msrc/main.rs\x1b[0m:\x1b[33m12\x1b[0m:5\r\n",
        "\x1b[38;5;208m256 色\x1b[0m \x1b[38;2;120;200;255m真彩色\x1b[0m \x1b[7m 反色 \x1b[0m ",
        "\x1b[2m淡色\x1b[0m \x1b[3m斜体\x1b[0m \x1b[9m删除线\x1b[0m \x1b[4:3;58;5;1m波浪线\x1b[0m\r\n",
        "\x1b[42;30m 全部通过 \x1b[0m 731 passed\r\n",
        "\x1b[1;32m➜\x1b[0m  \x1b[36mwinc\x1b[0m echo 中文和 emoji 😀",
    );
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        show_terminal(&mut app, screen);
        let _ = add_tab(&mut app, "cargo", screen);
        let _ = add_tab(&mut app, "htop", screen);
        app.terminal.active = 0;
        if let Some(tab) = app.terminal.tab_mut() {
            use crate::app::terminal::{Point, Selection, Unit};
            let line = tab.top() + 1;
            tab.selection = Some(Selection {
                anchor: Point { line, col: 0 },
                head: Point { line, col: 10 },
                unit: Unit::Char,
            });
        }
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-{theme}"));
        app.focus = Focus::Editor;
        shoot(&mut app, &format!("terminal-unfocused-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 连上真实的辅助进程：在登录 shell 中执行一条命令，等待输出出现在面板里。
///
/// 需要先构建辅助进程，并用 `WINC_PTY` 指向它：
///
/// ```text
/// cargo build -p winc-pty
/// WINC_PTY=$PWD/target/debug/winc-pty cargo test -p winc-app terminal_runs -- --ignored
/// ```
#[test]
#[ignore = "要先构建 winc-pty，见上面的说明"]
fn terminal_runs_a_command_in_the_login_shell() {
    let dir = sample_folder("terminal-live");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_enabled = Some(true);
    app.toggle_terminal();
    assert!(app.terminal.error.is_none(), "{:?}", app.terminal.error);

    let screen = |app: &App| {
        app.terminal.tab().map_or_else(String::new, |tab| {
            let lines: Vec<&str> = tab.rows.iter().map(winc_pty::Line::trimmed).collect();
            lines.join("\n")
        })
    };
    let wait = |app: &mut App, done: &dyn Fn(&App) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "等太久了，屏幕上是：\n{}",
                screen(app)
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
            // 测试里没有事件循环：自行收取消息、绘制一帧、分配时间额度。
            app.poll_terminal();
            paint(app);
            app.after_terminal_frame();
        }
    };
    wait(&mut app, &|app| app.terminal.tab().is_some());
    app.terminal_input(b"echo winc-$((6*7))\r".to_vec());
    wait(&mut app, &|app| screen(app).contains("winc-42"));
    // shell 集成（登录 shell 是 zsh、bash、fish 时注入）：这条命令成了一个成功结束的块。
    let shell = std::env::var("SHELL").unwrap_or_default();
    if ["zsh", "bash", "fish"].iter().any(|s| shell.ends_with(s)) {
        wait(&mut app, &|app| {
            app.terminal
                .tab()
                .unwrap()
                .blocks
                .iter()
                .any(|b| b.text == "echo winc-$((6*7))" && b.exit == Some(0))
        });
    }
    // 刷出一屏以上的输出后往上翻：缺的历史行向辅助进程请求，返回后即可见。
    app.terminal_input(b"seq 1 300\r".to_vec());
    // 等单独一行的 300（命令本身也带着「300」）：回显不算，输出到了才往上翻。
    wait(&mut app, &|app| screen(app).lines().any(|l| l == "300"));
    app.scroll_terminal(150);
    wait(&mut app, &|app| {
        let tab = app.terminal.tab().unwrap();
        (0..tab.rows.len() as u64).all(|r| tab.line(tab.top() + r).is_some())
    });
    let tab = app.terminal.tab().unwrap();
    let top: u32 = tab.line(tab.top()).unwrap().trimmed().parse().unwrap();
    assert!(top < 300 - tab.rows.len() as u32, "{top}");

    // 第二个标签：新的 shell，切回第一个还在。
    app.new_terminal_tab();
    wait(&mut app, &|app| app.terminal.tabs.len() == 2);
    app.terminal_input(b"echo second-$((1+1))\r".to_vec());
    wait(&mut app, &|app| screen(app).contains("second-2"));
    app.activate_terminal_tab(0);
    app.terminal_input(b"exit\r".to_vec());
    wait(&mut app, &|app| {
        app.terminal.tab().is_some_and(|t| t.ended.is_some())
    });

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dragging_the_divider_resizes_live_without_jumping() {
    let dir = sample_folder("terminal-drag-resize");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut screen = String::new();
    for i in 0..40 {
        screen.push_str(&format!("line {i}\r\n"));
    }
    // 提示符下面还空着几行（清过屏下半截）：辅助进程重排时去掉这些空行。
    screen.push_str("\x1b[3A\x1b[J$ ");
    let mut term = show_terminal(&mut app, &screen);
    app.attach_idle_helper();
    paint(&mut app);
    let line = {
        let tab = app.terminal.tab().unwrap();
        tab.history.1 + u64::from(tab.cursor.row)
    };
    // 光标那一行在网格的第几行；网格上的历史行都已经有了（没有先空着的行）。
    let cursor_row = |app: &App| {
        let tab = app.terminal.tab().unwrap();
        let view = tab.view();
        for row in &view {
            if let terminal::ViewRow::Line(l) = row {
                assert!(tab.line(*l).is_some(), "第 {l} 行空着");
            }
        }
        view.iter()
            .position(|row| *row == terminal::ViewRow::Line(line))
            .expect("光标那一行在网格上")
    };

    // 辅助进程回答在路上的尺寸：重排好的一帧。回答前后光标那一行在网格的同一行。
    let answer = |app: &mut App, term: &mut winc_pty::Terminal| {
        let (size, id) = {
            let tab = app.terminal.tab().unwrap();
            tab.resizing.map(|_| (tab.size, tab.id))
        }?;
        serve(app, term);
        let before = cursor_row(app);
        term.resize(size);
        let frame = term.take_frame().unwrap();
        app.terminal_message(winc_pty::protocol::FromHelper::Frame { tab: id, frame });
        serve(app, term);
        assert!(app.terminal.tab().unwrap().resizing.is_none());
        assert_eq!(cursor_row(app), before, "帧回来时不跳");
        // 面板这期间又变了：马上发下一个。
        app.terminal_resize_tick(std::time::Instant::now());
        Some(size)
    };

    // 往上拖高：每移动一次就发送新的尺寸，同时只有一个在途中，返回后才发下一个。
    let gap = app.layout.terminal_divider;
    let (x, y) = (gap.x + 200.0, gap.y + gap.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    serve(&mut app, &term);
    let lh = app.fonts.line_height;
    // 网格贴着面板下边：拖的时候最后一行的位置不动（零头留在上面）。
    let bottom = |app: &App| {
        let grid = terminal::grid_rect(app.layout.terminal, lh);
        grid.y + (grid.h / lh).floor() * lh
    };
    let fixed = bottom(&app);
    let mut answered = 0;
    // 有尺寸在路上时又动了几次。
    let mut held = 0;
    for step in 1..=8 {
        app.on_mouse_move(x, y - 1.5 * lh * step as f32);
        assert!((bottom(&app) - fixed).abs() < 0.05, "第 {step} 步网格动了");
        // 拖高时上面露出来的历史行已经取来了。
        cursor_row(&app);
        let sent = app.terminal.tab().unwrap().size;
        let waiting = app.terminal.tab().unwrap().resizing.is_some();
        app.on_mouse_move(x, y - 1.5 * lh * step as f32 - 0.7 * lh);
        if waiting {
            assert_eq!(app.terminal.tab().unwrap().size, sent, "同时只有一个在路上");
            held += 1;
        }
        while answer(&mut app, &mut term).is_some() {
            answered += 1;
        }
        paint(&mut app);
    }
    assert!(answered >= 8, "拖着的时候就改了 {answered} 次");
    assert!(held >= 4, "有尺寸在路上时只动了 {held} 次");
    let tab = app.terminal.tab().unwrap();
    assert_eq!(usize::from(tab.size.rows), tab.grid_rows());
    assert_eq!(
        cursor_row(&app),
        tab.grid_rows() - 1,
        "空行去掉了，提示符在最下面"
    );
    app.on_mouse_up(MouseButton::Left);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shortcuts_in_the_terminal_fields_stay_with_the_terminal() {
    use super::editor::navigation::press;
    let dir = sample_folder("terminal-field-keys");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ echo hi\r\nhi\r\n$ ");
    app.open_terminal_find();
    app.edit_field(|f| f.insert("hi"));

    // 编辑用的快捷键作用于输入框：全选、复制以后清空，再粘贴回来。
    assert!(press(&mut app, &["term"], Char('a')));
    assert!(press(&mut app, &["term"], Char('c')));
    assert_eq!(app.copied.as_deref(), Some("hi"));
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    app.run_action(crate::keymap::Action::Paste);
    assert_eq!(app.terminal.find.as_ref().unwrap().field.text(), "hi");
    app.edit_field(|f| {
        f.select_all();
        false
    });
    // ⌘F 关闭查找条，焦点回到终端，不会转到编辑区的查找条。
    assert!(press(&mut app, &["term"], Char('f')));
    assert!(app.terminal.find.is_none());
    assert_eq!(app.focus, Focus::Terminal);
    // 再按一次打开。
    assert!(press(&mut app, &["term"], Char('f')));
    assert_eq!(app.focus, Focus::TerminalFind);
    // 查找条开着、焦点在终端时，⌘F 回到查找框并全选，不关闭。
    app.edit_field(|f| f.insert("hi"));
    app.focus = Focus::Terminal;
    assert!(press(&mut app, &["term"], Char('f')));
    assert_eq!(app.focus, Focus::TerminalFind);
    let find = app.terminal.find.as_ref().unwrap();
    assert_eq!(find.field.selection(), (0, 2));
    // ⌘W 关的是终端标签，编辑区的文件还开着。
    assert!(press(&mut app, &["term"], Char('w')));
    assert!(app.terminal.tabs.is_empty());
    assert_eq!(app.editor.documents.len(), 1);

    // 标签上改名字时也一样。
    show_terminal(&mut app, "$ ");
    app.open_terminal_rename(0);
    assert!(press(&mut app, &["term"], Char('a')));
    assert!(press(&mut app, &["term"], Char('w')));
    assert!(app.terminal.tabs.is_empty());
    assert_eq!(app.editor.documents.len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_hidden_panel_does_not_keep_the_resize_timer_spinning() {
    let dir = sample_folder("terminal-resize-spin");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    // 改了尺寸、还在等那一帧时收起面板：过了期限以后不再返回过去的时刻（事件循环会空转）。
    app.terminal.resize_due = Some(Instant::now());
    app.set_terminal_open(false);
    let later = Instant::now() + std::time::Duration::from_secs(1);
    assert_eq!(app.terminal_resize_tick(later), None);
    assert_eq!(app.terminal.resize_due, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn only_the_tab_in_view_reflows_when_the_width_changes() {
    let dir = sample_folder("terminal-resize-hidden");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "zsh", "$ ");
    let log = app.attach_recording_helper(&dir);
    let resized = |app: &App| {
        (app.sent_to_helper(&log).iter())
            .filter_map(|m| match m {
                winc_pty::protocol::ToHelper::Resize { tab, size } => Some((*tab, size.cols)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let (first, second) = (app.terminal.tabs[0].id, app.terminal.tabs[1].id);
    let before = app.terminal_size().cols;

    // 改宽度要重排整个回滚缓冲：只改正在看的第二个标签，第一个在后台不重排。
    app.toggle_sidebar();
    app.relayout();
    let cols = app.terminal_size().cols;
    assert_ne!(cols, before);
    assert_eq!(resized(&app), [(second, cols)]);
    assert_eq!(app.terminal.tabs[0].size.cols, before);

    // 切过去时再改。
    app.activate_terminal_tab(0);
    app.relayout();
    assert_eq!(resized(&app), [(second, cols), (first, cols)]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reverse_video_keeps_default_text_visible() {
    let image = |bytes: &str| {
        let mut app = blank_app(true);
        show_terminal(&mut app, bytes);
        paint(&mut app);
        let renderer = app.renderer.as_ref().unwrap();
        renderer.image().unwrap().pixels.clone()
    };
    let reversed = "\x1b[?25l\x1b[?5h";
    assert_ne!(image(&format!("{reversed}text")), image(reversed));
}

/// 方块元素铺满格子：上下两行之间不露出底色。
#[test]
fn block_elements_leave_no_gap_between_rows() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "\x1b[?25l█\r\n█");
    let mut scale = 1.0;
    let renderer = app.renderer.as_mut().unwrap();
    renderer.draw(|frame| scale = frame.scale());
    paint(&mut app);
    let (x, y) = cell_center(&app, 0, 0);
    let lh = app.fonts.line_height;
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let pixel = |y: f32| {
        let (x, y) = ((x * scale) as usize, (y * scale) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let mut at = y;
    while at < y + lh {
        assert_eq!(pixel(at), pixel(y), "第一行中线往下 {} 处", at - y);
        at += 1.0 / scale;
    }
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
    let row = app.terminal_view().y + app.fonts.line_height / 2.0;
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
        let hits = &app.terminal.hits;
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
    let renderer = app.renderer.as_mut().unwrap();
    renderer.draw(|frame| scale = frame.scale());
    paint(&mut app);
    let bar = app.terminal_find_bar().unwrap();
    let image = app.renderer.as_ref().unwrap().image().unwrap();
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

/// 能选的 shell 不止一个时才有下拉箭头；选中的 shell 用来启动，重新启动和现场都带着它。
#[test]
fn the_shell_menu_starts_the_chosen_shell_and_remembers_it() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};
    use crate::app::terminal::shells::ShellChoice;
    use winc_pty::protocol::ToHelper;
    let dir = sample_folder("terminal-shells");
    let mut app = app_with(&dir.join("short.rs"), true);
    show_terminal(&mut app, "$ ");
    let log = app.attach_recording_helper(&dir);
    let has_menu = |app: &App| {
        let hits = &app.terminal.hits;
        hits.iter()
            .any(|(_, h)| *h == terminal::HeaderHit::ShellMenu)
    };
    let choice = |name: &str| ShellChoice {
        name: name.to_owned(),
        command: format!("/bin/{name}"),
    };

    // 只有一个可选时不显示。
    app.terminal.shells = vec![choice("zsh")];
    paint(&mut app);
    assert!(!has_menu(&app));

    app.terminal.shells = vec![choice("zsh"), choice("bash")];
    paint(&mut app);
    assert!(has_menu(&app));

    // 菜单列出两个，第二个的副标题是启动命令。
    let menu = (app.terminal.hits.iter())
        .find(|(_, h)| *h == terminal::HeaderHit::ShellMenu)
        .unwrap()
        .0;
    // 尺寸和间距与标签栏上其他按钮相同。
    let new = (app.terminal.hits.iter())
        .find(|(_, h)| *h == terminal::HeaderHit::New)
        .unwrap()
        .0;
    assert_eq!(menu.w, crate::theme::BUTTON);
    assert_eq!(menu.x - new.right(), crate::theme::TAB_GAP);
    click(&mut app, (menu.x + menu.w / 2.0, menu.y + menu.h / 2.0));
    assert_eq!(
        app.menu_button,
        Some(crate::app::MenuButton::TerminalShells)
    );
    assert_eq!(
        app.context_menu.as_ref().expect("弹出了菜单").entries,
        [
            MenuEntry::Item(MenuCommand::NewTerminalShell(0), true),
            MenuEntry::Item(MenuCommand::NewTerminalShell(1), true),
        ]
    );
    let label = app.menu_label(MenuCommand::NewTerminalShell(1));
    assert_eq!(label.title, "bash");
    assert_eq!(label.subtitle.as_deref(), Some("/bin/bash"));

    // 选中第二个：用它启动。
    app.new_terminal_with_shell(1);
    app.relayout();
    let started = |app: &App, log: &std::path::Path, shell: &[u8]| {
        let sent = app.sent_to_helper(log);
        let spawns = sent
            .iter()
            .filter(|m| matches!(m, ToHelper::Spawn { program: Some(p), .. } if p == shell));
        spawns.count()
    };
    assert_eq!(started(&app, &log, b"/bin/bash"), 1);
    assert_eq!(
        app.terminal.tab().unwrap().shell.as_deref(),
        Some("/bin/bash")
    );

    // 现场记下：默认 shell 的是空的。
    let session = app.terminal_session().unwrap();
    assert_eq!(session.shells, ["", "/bin/bash"]);

    // 退出后重新启动：还是这个 shell。
    let id = app.terminal.tab().unwrap().id;
    app.terminal_message(FromHelper::Exited { tab: id, code: 0 });
    app.restart_terminal_tab();
    app.relayout();
    assert_eq!(started(&app, &log, b"/bin/bash"), 2);
    assert_eq!(
        app.terminal.tab().unwrap().shell.as_deref(),
        Some("/bin/bash")
    );

    // ⌘T 仍然是默认 shell。
    app.new_terminal_tab();
    app.relayout();
    assert_eq!(app.terminal.tab().unwrap().shell, None);

    // 恢复现场：卸载了的 shell 改用默认的，不报启动失败。
    let mut restored = blank_app(true);
    restored.config.terminal_enabled = Some(true);
    restored.restore_terminal(&winc_config::TerminalSession {
        tabs: vec!["/tmp".into(), "/tmp".into()],
        shells: vec!["/bin/winc-no-such-shell".into(), "/bin/sh".into()],
        ..winc_config::TerminalSession::default()
    });
    let shells = &restored.terminal.want_shells;
    assert_eq!(shells[0], None);
    assert_eq!(shells[1].as_deref(), Some("/bin/sh"));

    // `terminal.shell` 是默认的，排在最前面，同名的不重复。
    app.config.terminal_shell = Some("/usr/local/bin/bash --norc".into());
    let list = app.terminal_shell_list();
    let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["bash", "zsh"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 标签开始收窄之后再新建标签：「+」固定在最右端，不随新标签的动画移动，直到标签最窄。
#[test]
fn the_new_button_stays_put_once_the_tabs_narrow() {
    use crate::app::terminal::strip::{TAB_MIN, TAB_SQUEEZED};
    let mut app = blank_app(true);
    show_terminal(&mut app, "$ ");
    // 动画冻结在起点：新标签宽度为零，其他标签已经收窄。
    app.terminal.strip.frozen = Some(std::time::Duration::ZERO);
    let new = |app: &App| {
        let hits = &app.terminal.hits;
        let hit = hits.iter().find(|(_, h)| *h == terminal::HeaderHit::New);
        hit.unwrap().0.x
    };
    let mut pinned = None;
    let mut narrowed = 0;
    for _ in 0..30 {
        let _ = add_tab(&mut app, "cargo", "");
        paint(&mut app);
        let widths = &app.terminal.strip.widths;
        if widths.iter().any(|&w| w < TAB_MIN) {
            narrowed += 1;
            let at = *pinned.get_or_insert(new(&app));
            assert_eq!(new(&app), at, "{} 个标签", widths.len());
        }
    }
    assert!(narrowed > 5, "覆盖了变窄的整个过程：{narrowed}");
    let widths = &app.terminal.strip.widths;
    assert!(widths.iter().all(|&w| w == TAB_SQUEEZED));
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
        let hits = &app.terminal.hits;
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
    let hits = &app.terminal.hits;
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
        let renderer = app.renderer.as_mut().unwrap();
        renderer.draw(|frame| scale = frame.scale());
        paint(app);
        // 第一格左下角：字形碰不到的地方，只有底色。
        let grid = app.terminal_grid();
        let (x, y) = (grid.x + 0.5, grid.y + app.fonts.line_height - 1.0);
        let image = app.renderer.as_ref().unwrap().image().unwrap();
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
        let renderer = app.renderer.as_ref().unwrap();
        let image = renderer.image().unwrap();
        let (w, h) = (image.width as usize, image.height as usize);
        // 只看窗口右边一条（滚动条轨道），下半部分（内容区）。
        let right = |row: usize| image.pixels[row * w + w - 20..row * w + w].to_vec();
        (h / 2..h).flat_map(right).collect::<Vec<u32>>()
    };
    assert_ne!(image("needle"), image("no-such-text"));
}

/// 输入文字时清除选区，⌘C 不会复制旧内容。
#[test]
fn typing_clears_the_selection() {
    use crate::app::terminal::{Point, Selection, Unit};
    let mut app = blank_app(true);
    show_terminal(&mut app, "hello\r\n");
    let line = app.terminal.tab().unwrap().top();
    app.terminal.tab_mut().unwrap().selection = Some(Selection {
        anchor: Point { line, col: 0 },
        head: Point { line, col: 3 },
        unit: Unit::Char,
    });
    app.terminal_commit("中");
    assert!(app.terminal.tab().unwrap().selection.is_none());
}

/// ⇧ 单击保留选区起点并移动终点。
#[test]
fn shift_click_extends_the_selection() {
    let mut app = blank_app(true);
    show_terminal(&mut app, "one two three\r\nfour five\r\n");
    paint(&mut app);
    app.on_mouse_move(cell_center(&app, 0, 2).0, cell_center(&app, 0, 2).1);
    app.on_mouse_down(winit::event::MouseButton::Left);
    let (x, y) = cell_center(&app, 0, 6);
    app.on_mouse_move(x, y);
    app.on_mouse_up(winit::event::MouseButton::Left);
    let anchor = app.terminal.tab().unwrap().selection.unwrap().anchor;

    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    let (x, y) = cell_center(&app, 1, 3);
    app.on_mouse_move(x, y);
    app.on_mouse_down(winit::event::MouseButton::Left);
    app.on_mouse_up(winit::event::MouseButton::Left);
    let selection = app.terminal.tab().unwrap().selection.unwrap();
    assert_eq!(selection.anchor, anchor);
    assert_eq!(selection.head.line, anchor.line + 1);
}

/// 拖选超出内容区时持续滚动，鼠标静止时同样滚动，到达顶部后停止。
#[test]
fn dragging_past_the_edge_keeps_scrolling() {
    let mut app = blank_app(true);
    let output: String = (1..=80).map(|i| format!("line {i}\r\n")).collect();
    let term = show_terminal(&mut app, &output);
    serve(&mut app, &term);
    paint(&mut app);
    let (x, y) = cell_center(&app, 3, 1);
    app.on_mouse_move(x, y);
    app.on_mouse_down(winit::event::MouseButton::Left);
    assert!(!app.drag_autoscrolling());

    let top = app.terminal_view().y - 40.0;
    app.on_mouse_move(x, top);
    assert!(app.drag_autoscrolling());
    let (before, limit) = app.terminal.tab().unwrap().scroll_position();
    // 鼠标静止时下一帧继续滚动。
    app.continue_drag_scroll();
    let (after, _) = app.terminal.tab().unwrap().scroll_position();
    assert!(after > before, "{after} > {before}");

    // 到达顶部后不再请求绘制下一帧。
    app.terminal.tab_mut().unwrap().set_scroll_position(limit);
    assert!(!app.drag_autoscrolling());
    app.on_mouse_up(winit::event::MouseButton::Left);
}

/// ⌥ 单击只在 shell 等输入、点在光标那一行时生效，发相应次数的方向键。
#[test]
fn option_click_moves_the_shell_cursor_at_the_prompt() {
    use winc_pty::protocol::ToHelper;
    let dir = sample_folder("terminal-option-click");
    let mut app = blank_app(true);
    let prompt = format!("\x1b]133;A;{NONCE}\x07$ \x1b]133;B;{NONCE}\x07echo hi");
    show_terminal(&mut app, &prompt);
    let log = app.attach_recording_helper(&dir);
    paint(&mut app);
    app.terminal.tab_mut().unwrap().blocks = vec![winc_pty::protocol::Block {
        id: 1,
        ..Default::default()
    }];
    app.modifiers = winit::keyboard::ModifiersState::ALT;
    let inputs = |app: &App| -> Vec<Vec<u8>> {
        let sent = app.sent_to_helper(&log);
        let bytes = sent.into_iter().filter_map(|m| match m {
            ToHelper::Input { bytes, .. } => Some(bytes),
            _ => None,
        });
        bytes.collect()
    };

    // 光标在第 9 格（行尾），点第 2 格：左移 7 次。
    let (x, y) = cell_center(&app, 0, 2);
    app.on_mouse_move(x - app.cell / 2.0 + 0.5, y);
    app.on_mouse_down(winit::event::MouseButton::Left);
    app.on_mouse_up(winit::event::MouseButton::Left);
    assert_eq!(inputs(&app).concat(), b"\x1b[D".repeat(7));
    assert!(app.terminal.tab().unwrap().selection.is_none());

    // 命令在运行时不发。
    let tab = app.terminal.tab_mut().unwrap();
    tab.blocks[0].output = Some(1);
    let sent = inputs(&app).len();
    app.on_mouse_down(winit::event::MouseButton::Left);
    app.on_mouse_up(winit::event::MouseButton::Left);
    assert_eq!(inputs(&app).len(), sent);
    std::fs::remove_dir_all(&dir).unwrap();
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
    let y = origin + (display - first) as f32 * app.fonts.line_height;
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
    let renderer = app.renderer.as_mut().unwrap();
    renderer.draw(|frame| scale = frame.scale());
    // 标签栏从左边到「+」右边的像素。
    let tabs = |app: &mut App| {
        paint(app);
        let new = (app.terminal.hits.iter())
            .find(|(_, hit)| *hit == terminal::HeaderHit::New)
            .unwrap()
            .0;
        let image = app.renderer.as_ref().unwrap().image().unwrap();
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

#[test]
fn combining_marks_and_underline_styles_are_drawn() {
    // 同一行文字按 `bytes` 画好以后的画面。
    let image = |bytes: &str| {
        let mut app = blank_app(false);
        show_terminal(&mut app, bytes);
        paint(&mut app);
        app.renderer
            .as_ref()
            .unwrap()
            .image()
            .unwrap()
            .pixels
            .clone()
    };
    // NFD 的「é」（`e` 加组合重音符）：重音符画出来，与不带重音符的 `e` 不同。
    assert_ne!(image("cafe\u{301}"), image("cafe"));
    // 波浪线、点线、虚线各有各的画法，不都画成直线。
    let single = image("\x1b[4:1mtext\x1b[0m");
    for style in 3..=5 {
        let styled = image(&format!("\x1b[4:{style}mtext\x1b[0m"));
        assert_ne!(styled, single, "下划线样式 {style}");
    }
}
