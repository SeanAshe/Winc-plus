//! 终端输入策略的回归测试：主屏程序、备用屏、未知状态、组合键和数字直达。

use super::*;
use crate::keymap::{Action, KeyCode, Lookup, Mods, Os, Stroke};
use winc_pty::protocol::{Block, Modes};

fn key(mods: Mods, key: KeyCode) -> Stroke {
    Stroke { mods, key }
}

fn cmd() -> Mods {
    if Os::CURRENT == Os::Mac {
        Mods::META
    } else {
        Mods::CTRL
    }
}

/// 直达第 N 个终端的按键：macOS 上是 ⌃ 加数字，其他系统上是 Ctrl+Shift 加数字。
fn digit(ch: char) -> Stroke {
    let shift = if Os::CURRENT == Os::Mac {
        Mods::NONE
    } else {
        Mods::SHIFT
    };
    key(Mods::CTRL | shift, KeyCode::Char(ch))
}

/// 已完成一条命令，并明确回到 shell 提示符的快照。
fn at_prompt(app: &mut App) {
    app.terminal.tab_mut().unwrap().blocks = vec![
        Block {
            id: 1,
            prompt: 0,
            output: Some(1),
            end: Some(2),
            exit: Some(0),
            ..Block::default()
        },
        Block {
            id: 2,
            prompt: 2,
            ..Block::default()
        },
    ];
}

#[test]
fn command_navigation_requires_a_prompt_and_available_history() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "output");
    let previous = key(cmd(), KeyCode::Up);
    assert!(!app.on_shortcut(previous), "没有 shell 集成标记时不能吞键");
    at_prompt(&mut app);
    assert!(app.terminal_at_prompt());
    assert!(app.on_shortcut(previous));
    assert_eq!(app.terminal.tab().unwrap().jump, Some(1));

    app.terminal.tab_mut().unwrap().blocks.remove(0);
    assert!(!app.on_shortcut(previous), "没有可跳转命令时交还按键");
    assert!(!app.binds(previous));
}

/// 程序能收到的命令块按键：F8 在各系统上都会发送给程序。
fn next_error() -> Stroke {
    key(Mods::NONE, KeyCode::F(8))
}

/// 终端自己的查找键。
fn find() -> Stroke {
    let shift = if Os::CURRENT == Os::Mac {
        Mods::NONE
    } else {
        Mods::SHIFT
    };
    key(cmd() | shift, KeyCode::Char('f'))
}

#[test]
fn running_programs_and_interaction_modes_receive_navigation_keys() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "output");
    let mac = Os::CURRENT == Os::Mac;
    let previous = key(cmd(), KeyCode::Up);
    for mode in [
        Modes::ALT_SCREEN,
        Modes::MOUSE_PRESS,
        Modes::MOUSE_DRAG,
        Modes::MOUSE_MOTION,
    ] {
        at_prompt(&mut app);
        app.terminal.tab_mut().unwrap().modes.set(mode, true);
        assert!(!app.terminal_at_prompt());
        assert!(!app.binds(next_error()));
        // macOS 上带 ⌘ 的组合不发送给程序，让出只会使按键失效。
        assert_eq!(app.binds(previous), mac);
        app.terminal.tab_mut().unwrap().modes.set(mode, false);
    }
    at_prompt(&mut app);
    app.terminal.tab_mut().unwrap().keyboard = 1;
    assert!(
        !app.binds(next_error()),
        "主屏程序请求键盘协议后也应收到按键"
    );
    assert_eq!(app.binds(previous), mac);
    app.terminal.tab_mut().unwrap().keyboard = 0;
    app.terminal
        .tab_mut()
        .unwrap()
        .blocks
        .last_mut()
        .unwrap()
        .output = Some(3);
    assert!(!app.binds(next_error()), "运行中的命令和主屏程序应收到按键");
    assert_eq!(app.binds(previous), mac);
    app.config.terminal_shell_integration = Some(false);
    at_prompt(&mut app);
    assert!(!app.binds(next_error()));
}

#[test]
fn clearing_and_paging_stay_available_without_a_known_prompt() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "output");
    let term = if Os::CURRENT == Os::Mac {
        Mods::META
    } else {
        Mods::CTRL | Mods::SHIFT
    };
    let clear = key(term, KeyCode::Char('k'));
    // 没有 shell 集成标记、命令正在运行或进程已结束时，清屏和回滚翻页仍可使用。
    assert!(!app.terminal_at_prompt());
    assert!(app.binds(clear));
    assert!(app.terminal_scroll_keys());
    app.config.terminal_shell_integration = Some(false);
    assert!(app.binds(clear));
    assert!(app.terminal_scroll_keys());

    // 备用屏没有回滚缓冲，屏幕内容由程序管理。
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::ALT_SCREEN, true);
    assert!(!app.terminal_scroll_keys());
    assert_eq!(app.binds(clear), Os::CURRENT == Os::Mac);
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::ALT_SCREEN, false);

    app.config.terminal_application_priority = Some(true);
    assert!(!app.terminal_scroll_keys());
    assert_eq!(app.binds(clear), Os::CURRENT == Os::Mac);
}

#[test]
fn application_priority_keeps_management_and_does_not_change_winc_fields() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    add_tab(&mut app, "agent", "two");
    at_prompt(&mut app);
    app.config.terminal_application_priority = Some(true);
    let mac = Os::CURRENT == Os::Mac;
    assert!(!app.binds(next_error()));
    assert!(!app.binds(key(Mods::SHIFT, KeyCode::F(8))));
    // 其他系统上 Ctrl 组合会发送给程序；macOS 上带 ⌘ 的组合仍由 Winc 处理。
    assert_eq!(app.binds(key(cmd(), KeyCode::Up)), mac);
    assert_eq!(app.binds(find()), mac);
    assert!(app.on_shortcut(digit('1')));
    assert_eq!(app.terminal.active, 0);
    assert_eq!(app.focus, Focus::Terminal);
    assert!(app.on_shortcut(key(cmd() | Mods::SHIFT, KeyCode::Char(']'))));
    assert_eq!(app.terminal.active, 1);

    app.focus = Focus::TerminalRename;
    assert!(app.on_shortcut(find()));
    assert_eq!(app.focus, Focus::TerminalFind);
    assert!(app.on_shortcut(find()));
    assert_eq!(app.focus, Focus::Terminal);
}

#[test]
fn terminal_priority_does_not_leave_a_pending_editor_chord() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "output");
    app.pending_chord = Some(key(cmd(), KeyCode::Char('k')));
    assert!(!app.on_shortcut(key(Mods::NONE, KeyCode::Char('x'))));
    assert_eq!(app.pending_chord, None);
}

#[test]
fn terminal_numbers_follow_display_order_and_do_not_create_missing_tabs() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    add_tab(&mut app, "agent", "two");
    add_tab(&mut app, "agent", "three");
    app.terminal.tabs.swap(0, 2);
    let id = app.terminal.tabs[0].id;
    assert!(app.on_shortcut(digit('1')));
    assert_eq!(app.terminal.tab().unwrap().id, id);
    let count = app.terminal.tabs.len();
    assert!(app.on_shortcut(digit('9')));
    assert_eq!(app.terminal.tab().unwrap().id, id);
    assert_eq!(app.terminal.tabs.len(), count);
    assert_eq!(app.focus, Focus::Terminal);
    app.focus = Focus::Editor;
    assert!(!app.on_shortcut(digit('9')));
}

#[test]
fn terminal_focus_commands_reveal_an_existing_tab_in_a_hidden_panel() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    add_tab(&mut app, "agent", "two");
    app.set_terminal_open(false);
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.run_action(Action::FocusTerminal9));
    assert!(!app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.run_action(Action::FocusTerminal1));
    assert!(app.terminal.open);
    assert_eq!(app.terminal.active, 0);
    assert_eq!(app.focus, Focus::Terminal);
    assert_eq!(app.terminal.tabs.len(), 2);
}

#[test]
fn the_panel_toggle_closes_in_one_press_wherever_the_focus_is() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    let toggle = key(cmd(), KeyCode::Char('j'));

    // 焦点在面板外：进出终端的键先把焦点放进去，面板开关直接收起，焦点留在原处。
    app.focus = Focus::Editor;
    assert!(app.run_action(Action::ToggleTerminal));
    assert!(app.terminal.open);
    assert_eq!(app.focus, Focus::Terminal);
    app.focus = Focus::Editor;
    assert!(app.on_shortcut(toggle));
    assert!(!app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);

    // 面板关着时打开并聚焦，标签不增加。
    assert!(app.on_shortcut(toggle));
    assert!(app.terminal.open);
    assert_eq!(app.focus, Focus::Terminal);
    assert_eq!(app.terminal.tabs.len(), 1);

    // 终端应用优先时仍保留面板开关；macOS 以外的系统上 Ctrl+J 在终端里交给 shell。
    app.config.terminal_application_priority = Some(true);
    assert_eq!(app.binds(toggle), Os::CURRENT == Os::Mac);
    assert!(app.run_action(Action::ToggleTerminalPanel));
    assert!(!app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn terminal_numbers_leave_the_tool_view_keys_alone() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    // 焦点在终端时，打开工具视图的数字键不被终端编号占用。
    for digit in ['7', '9'] {
        if let Lookup::Run(actions) = app.lookup_shortcut(key(cmd(), KeyCode::Char(digit)), None) {
            assert!(actions.iter().all(|a| a.terminal_index().is_none()));
        }
    }
}
