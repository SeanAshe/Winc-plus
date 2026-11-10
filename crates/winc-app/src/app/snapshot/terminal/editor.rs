//! 终端与编辑器配合的测试：按住 ⌘ 的链接、在终端中打开、拖入路径、把选中的代码发送到终端，
//! 以及 shell 退出、伪终端断开与辅助进程退出之后的重启、关闭仍在运行的标签前的确认。

use super::*;
use super::{cell_center, click, show_terminal};
use crate::app::input::context_menu::MenuCommand;
use crate::app::terminal::{HeaderHit, TabEnd, TermLink};
use winc_pty::protocol::{Block, FromHelper, ToHelper};

fn cwd(dir: &std::path::Path) -> String {
    format!("\x1b]7;file://host{}\x07", dir.display())
}

/// 第 0 行第 `col` 格，按住 ⌘ 移过去，返回认出的链接。
fn hover_link(app: &mut App, row: usize, col: usize) -> Option<TermLink> {
    app.modifiers = command_modifiers();
    let (x, y) = cell_center(app, row, col);
    app.on_mouse_move(x, y);
    app.terminal.link.clone()
}

#[test]
fn command_hover_finds_urls_osc8_links_and_files() {
    let dir = sample_folder("terminal-links");
    let mut app = app_with(&dir.join("a.txt"), false);
    let line = concat!(
        "see https://example.com/x and src/main.rs:2:5 ",
        "\x1b]8;;https://osc8.example/\x07here\x1b]8;;\x07 Cargo.toml missing.rs"
    );
    let mut term = show_terminal(&mut app, &format!("{}{line}", cwd(&dir)));
    // 测试里没有辅助进程转发事件：直接把链接地址交给标签。
    for event in term.take_events() {
        if let winc_pty::protocol::Event::Link(id, uri) = event {
            app.terminal.tab_mut().unwrap().set_link(id, uri);
        }
    }
    paint(&mut app);
    // 不按 ⌘ 时没有链接。
    let (x, y) = cell_center(&app, 0, 6);
    app.on_mouse_move(x, y);
    assert!(app.terminal.link.is_none());

    use crate::app::terminal::link::Target;
    let url = hover_link(&mut app, 0, 6).unwrap();
    assert_eq!(url.target, Target::Url("https://example.com/x".into()));
    assert_eq!((url.start, url.end), (4, 25));
    paint(&mut app);

    let file = hover_link(&mut app, 0, 32).unwrap();
    let Target::File { path, line, col } = &file.target else {
        panic!("{file:?}");
    };
    assert!(path.ends_with("src/main.rs"));
    assert_eq!((*line, *col), (Some(2), Some(5)));

    let osc8 = hover_link(&mut app, 0, 47).unwrap();
    assert_eq!(osc8.target, Target::Url("https://osc8.example/".into()));
    // 不带行号的路径要文件存在。
    assert!(hover_link(&mut app, 0, 54).is_some());
    assert!(hover_link(&mut app, 0, 66).is_none());

    // 松开 ⌘，链接没了。
    app.modifiers = winit::keyboard::ModifiersState::empty();
    app.update_terminal_link();
    assert!(app.terminal.link.is_none());

    // ⌘ 单击文件：打开到那一行那一列。
    hover_link(&mut app, 0, 32);
    let at = cell_center(&app, 0, 32);
    click(&mut app, at);
    let doc = app.editor.active().unwrap();
    assert!(doc.path.as_deref().unwrap().ends_with("src/main.rs"));
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (1, 4));

    // ⌘⇧ 单击：在文件树里定位。
    app.focus = Focus::Terminal;
    hover_link(&mut app, 0, 55);
    app.modifiers = command_modifiers() | winit::keyboard::ModifiersState::SHIFT;
    let at = cell_center(&app, 0, 55);
    click(&mut app, at);
    let ws = app.editor.workspace.as_ref().unwrap();
    let selected = &ws.rows()[ws.selected.unwrap()];
    assert!(selected.path.ends_with("Cargo.toml"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_editor_sends_code_and_folders_to_the_terminal() {
    let dir = sample_folder("terminal-send");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_enabled = Some(true);

    // ⌃⇧↩ 没有终端时开一个，代码等它起来以后写进去；焦点留在编辑区。没有选区时发光标所在的行。
    app.run_action(crate::keymap::Action::SendToTerminal);
    assert!(app.terminal.open);
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.terminal.want, [Vec::<u8>::new()]);
    assert_eq!(
        app.terminal.pending_input.as_deref(),
        Some(&b"fn main() {\r"[..])
    );

    // 在终端中打开：模块开着时在那个目录新建一个终端标签，不开外部终端。
    app.terminal = Default::default();
    app.run_tree_command(MenuCommand::OpenInTerminal, Some(dir.join("src")));
    assert!(app.opened_terminal.is_none());
    assert_eq!(app.terminal.want.len(), 1);
    assert!(app.terminal.want[0].ends_with(b"/src"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_dropped_on_the_terminal_are_typed_not_opened() {
    let dir = sample_folder("terminal-drop");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    app.focus = Focus::Editor;
    let tabs = app.editor.documents.len();
    let at = cell_center(&app, 0, 4);
    assert_eq!(app.drop_zone(at.0, at.1), DropZone::Terminal);
    app.drop_file_at(&dir.join("a.txt"), at.0, at.1);
    assert_eq!(app.editor.documents.len(), tabs, "没有打开文件");
    assert_eq!(app.focus, Focus::Terminal);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn exited_shells_and_a_crashed_helper_restart() {
    let dir = sample_folder("terminal-restart");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ last screen");

    // 辅助进程没了：标签和最后一屏还在，标签栏上有「重新启动」。
    app.terminal_helper_gone();
    assert!(app.terminal.crashed);
    assert_eq!(app.terminal.tabs.len(), 1);
    app.terminal.tabs[0].cwd = b"/usr".to_vec();
    paint(&mut app);
    let button = app
        .hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::Restart)
        .map(|(rect, _)| *rect)
        .expect("有重新启动按钮");
    click(&mut app, (button.x + 4.0, button.y + 4.0));
    // 每个标签在自己最后的目录里重开（测试里找不到辅助进程，停在等它的这一步）。
    assert!(!app.terminal.crashed);
    assert!(app.terminal.tabs.is_empty());
    assert_eq!(app.terminal.want, [b"/usr".to_vec()]);

    // shell 退出了：↩ 只重开这一个。
    app.terminal = Default::default();
    show_terminal(&mut app, "$ exit");
    app.terminal.tabs[0].ended = Some(TabEnd::Exited(1));
    app.terminal.tabs[0].cwd = b"/tmp".to_vec();
    app.terminal.tabs[0].name = "server".into();
    paint(&mut app);
    app.restart_terminal_tab();
    assert_eq!(app.terminal.want, [b"/tmp".to_vec()]);
    // 起的名字跟着等着重开的标签走。
    assert_eq!(app.terminal_session().unwrap().names, ["server"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_lost_pseudo_console_stops_input_and_offers_a_restart() {
    let dir = sample_folder("terminal-disconnected");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ last screen");
    let log = app.attach_recording_helper(&dir);
    let id = app.terminal.tabs[0].id;
    app.terminal.tabs[0].process = "codex".into();

    // 程序还在运行，但已经够不着：输入不再发出去，关标签时仍然要确认。
    app.terminal_message(FromHelper::Disconnected { tab: id });
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.ended, Some(TabEnd::Disconnected));
    assert_eq!(tab.busy(), Some("codex"));
    assert_eq!(tab.unseen, None);
    app.terminal_input(b"a".to_vec());
    let sent = app.sent_to_helper(&log);
    assert!(!sent.iter().any(|m| matches!(m, ToHelper::Input { .. })));

    paint(&mut app);
    let restart = |app: &App| (app.hits.terminal.iter()).any(|(_, hit)| *hit == HeaderHit::Restart);
    assert!(restart(&app));

    // shell 随后退出了：说明换成退出码，仍然可以重新启动。
    app.terminal_message(FromHelper::Exited { tab: id, code: 1 });
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.ended, Some(TabEnd::Exited(1)));
    assert_eq!(tab.busy(), None);
    paint(&mut app);
    assert!(restart(&app));

    // 重新启动时关掉旧的终端，其中还在运行的进程一并结束。
    app.terminal_message(FromHelper::Disconnected { tab: id });
    app.request_terminal_restart();
    let sent = app.sent_to_helper(&log);
    assert!(
        sent.iter()
            .any(|m| matches!(m, ToHelper::Close { tab } if *tab == id))
    );
    assert_eq!(app.terminal.tab().unwrap().ended, None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_lost_pseudo_console_out_of_view_leaves_a_red_dot() {
    let dir = sample_folder("terminal-disconnected-unseen");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "zsh", "$ ");
    let (first, second) = (app.terminal.tabs[0].id, app.terminal.tabs[1].id);

    // 在看第二个标签时第一个断开了。
    app.terminal_message(FromHelper::Disconnected { tab: first });
    assert_eq!(app.terminal.tabs[0].unseen, Some(true));

    // 面板收着时正在看的标签断开了。
    app.terminal.open = false;
    app.terminal_message(FromHelper::Disconnected { tab: second });
    assert_eq!(app.terminal.tabs[1].unseen, Some(true));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn busy_tabs_name_what_is_running() {
    let dir = sample_folder("terminal-busy");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    let tab = app.terminal.tab_mut().unwrap();
    tab.process = "-zsh".into();
    assert_eq!(tab.busy(), None);
    tab.process = "vim".into();
    assert_eq!(tab.busy(), Some("vim"));
    tab.process = "zsh".into();
    tab.blocks = vec![Block {
        output: Some(1),
        text: "cargo build".into(),
        ..Block::default()
    }];
    assert_eq!(tab.busy(), Some("cargo build"));
    tab.ended = Some(TabEnd::Exited(0));
    assert_eq!(tab.busy(), None);

    // 测试里没有窗口弹确认框，直接关。
    app.terminal.tabs[0].ended = None;
    app.close_terminal_tab(0);
    assert!(app.terminal.tabs.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn running_programs_are_asked_about_before_quitting() {
    let dir = sample_folder("terminal-quit");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    // 只有 shell 在等待输入：⌘Q 直接退出。
    app.terminal.tab_mut().unwrap().process = "zsh".into();
    assert!(!app.asks_before_quit());
    // 有程序在运行：AppKit 的 ⌘Q 回调必须回到事件循环询问，不能直接退出。
    app.terminal.tab_mut().unwrap().process = "vim".into();
    assert!(app.asks_before_quit());
    app.config.terminal_confirm_close = Some(false);
    assert!(!app.asks_before_quit());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_links_and_restart() {
    let dir = sample_folder("terminal-links-shot");
    let screen = format!(
        "{}$ cargo build\r\nerror: oops\r\n  --> src/main.rs:2:5\r\nsee https://doc.rust-lang.org/error_codes\r\n$ ",
        cwd(&dir)
    );
    let mut app = app_with(&dir.join("short.rs"), true);
    show_terminal(&mut app, &screen);
    hover_link(&mut app, 2, 8);
    shoot(&mut app, "terminal-link-dark");
    app.terminal_helper_gone();
    shoot(&mut app, "terminal-crashed-dark");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_prompt() {
    let dir = sample_folder("terminal-prompt-shot");
    let prompt = |dir: &str, ok: bool| {
        format!(
            "\x1b[34m\x1b[1m{dir}\x1b[22m\x1b[39m \x1b[35mmain\x1b[39m \x1b[{}m❯\x1b[39m ",
            if ok { 32 } else { 31 }
        )
    };
    let screen = [
        prompt("~/project/winc", true),
        "cargo build\r\n".into(),
        "\x1b[1m\x1b[32m   Compiling\x1b[0m winc v0.1.0\r\n".into(),
        "\x1b[1m\x1b[32m    Finished\x1b[0m `dev` profile in 3.2s\r\n".into(),
        prompt("~/project/winc", true),
        "lsof -i :30030\r\n".into(),
        prompt("~/project/winc", false),
        "ls\r\n".into(),
        "Cargo.toml  \x1b[1;34mcrates\x1b[0m  \x1b[1;34mdocs\x1b[0m  README.md\r\n".into(),
        prompt("~/project/winc", true),
    ]
    .concat();
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        show_terminal(&mut app, &screen);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-prompt-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 标签栏上某个按钮的位置。
///
/// # Panics
///
/// 最近一帧没有画出该按钮时 panic，调用前须先 [`paint`]。
fn header_hit(app: &App, hit: HeaderHit) -> winc_render::Rect {
    app.hits
        .terminal
        .iter()
        .find(|(_, h)| *h == hit)
        .map(|(rect, _)| *rect)
        .unwrap()
}

#[test]
fn terminal_tabs_are_renamed_in_place() {
    use super::add_tab;
    use crate::app::input::context_menu::MenuTarget;

    let dir = sample_folder("terminal-rename");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    app.terminal.tabs[0].title = "~/winc".into();
    add_tab(&mut app, "zsh", "$ ");
    paint(&mut app);
    let id = app.terminal.tabs[0].id;

    // 右键第一个标签：它成为正在看的，菜单是这个标签的。
    let tab = header_hit(&app, HeaderHit::Tab(0));
    let at = (tab.x + 20.0, tab.y + tab.h / 2.0);
    app.on_mouse_move(at.0, at.1);
    app.on_mouse_down(MouseButton::Right);
    app.on_mouse_up(MouseButton::Right);
    assert_eq!(app.terminal.active, 0);
    let menu = app.context_menu.as_ref().unwrap();
    assert_eq!(menu.target, MenuTarget::TerminalTab(id));
    app.run_menu_command(MenuCommand::RenameTerminal);

    // 标签上换成输入框：填好现在的名字、全选，输入框跟着标签走。
    assert_eq!(app.focus, Focus::TerminalRename);
    let (_, field) = app.terminal.rename.as_ref().unwrap();
    assert_eq!(field.selected_text(), "~/winc");
    paint(&mut app);
    let field = app.terminal.rename.as_ref().unwrap().1.view.area;
    assert!(tab.contains(field.x, field.y));

    // 回车：名字盖过程序的标题，焦点回到终端。
    app.edit_field(|f| f.insert("  server "));
    app.finish_terminal_rename(true);
    assert_eq!(app.terminal.tabs[0].label(), "server");
    assert_eq!(app.focus, Focus::Terminal);

    // 双击再改，Esc 不改。
    paint(&mut app);
    click(&mut app, at);
    click(&mut app, at);
    assert_eq!(app.focus, Focus::TerminalRename);
    app.edit_field(|f| f.insert("x"));
    app.finish_terminal_rename(false);
    assert_eq!(app.terminal.tabs[0].label(), "server");

    // 编辑中途点击其他位置：按输入内容提交；留空则恢复自动的名称。
    app.run_action(crate::keymap::Action::RenameTerminal);
    app.edit_field(|f| f.delete(crate::field::Unit::Char, false));
    paint(&mut app);
    let (x, y) = cell_center(&app, 1, 1);
    click(&mut app, (x, y));
    assert!(app.terminal.rename.is_none());
    assert_eq!(app.terminal.tabs[0].label(), "~/winc");

    // 面板收着时从命令面板改名：先把面板打开。
    app.toggle_terminal();
    assert!(!app.terminal.open);
    app.run_action(crate::keymap::Action::RenameTerminal);
    assert!(app.terminal.open);
    assert_eq!(app.focus, Focus::TerminalRename);
    // 编辑中途去重命名另一个标签：前一个按输入内容提交。
    app.edit_field(|f| f.insert("one"));
    app.open_terminal_rename(1);
    assert_eq!(app.terminal.tabs[0].label(), "one");

    // 改名过程中关闭该标签（⌘W，其他系统 Ctrl+Shift+W）：输入框随之消失，焦点回到终端，不留在
    // 已隐藏的输入框里。
    assert_eq!(app.terminal.active, 1);
    crate::app::snapshot::editor::navigation::press(
        &mut app,
        &["term"],
        crate::keymap::KeyCode::Char('w'),
    );
    assert!(app.terminal.rename.is_none());
    assert_eq!(app.focus, Focus::Terminal);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn terminal_tabs_grow_shrink_and_reorder_by_dragging() {
    use super::add_tab;
    use crate::app::terminal::strip::{TAB_GAP, TAB_MIN};
    use std::time::Duration;

    let dir = sample_folder("terminal-strip");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    paint(&mut app);
    // 名字很短的标签也有一定宽度。
    let first = header_hit(&app, HeaderHit::Tab(0));
    assert_eq!(first.w, TAB_MIN);
    let plus = header_hit(&app, HeaderHit::New);

    // 新建：宽度从零开始增长，刚开始时「+」仍在原处，增长结束后位于其右方。
    app.terminal.strip.frozen = Some(Duration::ZERO);
    add_tab(&mut app, "zsh", "$ ");
    paint(&mut app);
    assert_eq!(header_hit(&app, HeaderHit::New).x, plus.x);
    // 测试里的时钟停在开头不会结束：去掉动画再画。
    app.terminal.strip = Default::default();
    paint(&mut app);
    let after = header_hit(&app, HeaderHit::New).x;
    assert_eq!(after, plus.x + TAB_MIN + TAB_GAP);
    add_tab(&mut app, "zsh", "$ ");
    paint(&mut app);
    let ids: Vec<u16> = app.terminal.tabs.iter().map(|t| t.id).collect();

    // 按住第一个向右拖过第二个的中线：两者换位，正在看的仍是拖动的那个标签。
    let tab = header_hit(&app, HeaderHit::Tab(0));
    let (x, y) = (tab.x + 20.0, tab.y + tab.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.terminal.active, 0);
    app.on_mouse_move(x + TAB_MIN * 0.7, y);
    paint(&mut app);
    let order: Vec<u16> = app.terminal.tabs.iter().map(|t| t.id).collect();
    assert_eq!(order, [ids[1], ids[0], ids[2]]);
    assert_eq!(app.terminal.active, 1);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.dragged_terminal_tab().is_none());

    // 关闭中间那个标签：在原位收窄，刚开始时右侧标签和「+」仍在原处。
    paint(&mut app);
    let plus = header_hit(&app, HeaderHit::New).x;
    let third = header_hit(&app, HeaderHit::Tab(2)).x;
    app.terminal.strip.frozen = Some(Duration::ZERO);
    app.close_terminal_tab(1);
    paint(&mut app);
    assert_eq!(app.terminal.tabs.len(), 2);
    assert_eq!(header_hit(&app, HeaderHit::Tab(1)).x, third);
    assert_eq!(header_hit(&app, HeaderHit::New).x, plus);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_tabs() {
    use super::add_tab;
    use std::time::Duration;
    let dir = sample_folder("terminal-tabs-shot");
    let mut app = app_with(&dir.join("short.rs"), true);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "cargo", "$ ");
    add_tab(&mut app, "zsh", "$ ");
    app.terminal.tabs[1].title = "cargo watch -x test".into();
    paint(&mut app);
    shoot(&mut app, "terminal-tabs-dark");
    // 新建的长到一半，拖着第一个往右走了一段。
    app.terminal.strip.frozen = Some(Duration::from_millis(30));
    add_tab(&mut app, "zsh", "$ ");
    let tab = header_hit(&app, HeaderHit::Tab(0));
    let (x, y) = (tab.x + 20.0, tab.y + tab.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 50.0, y);
    shoot(&mut app, "terminal-tabs-moving-dark");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 应用绘制标题栏、终端面板在右边时：面板的标签栏在顶栏那一行，右端的两个按钮排在窗口按钮
/// 左边，不叠在一起。
#[test]
fn a_right_terminal_leaves_room_for_the_window_buttons() {
    let dir = sample_folder("terminal-right-controls");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.window.controls = true;
    show_terminal(&mut app, "$ ");
    app.config.terminal_right = Some(true);
    app.relayout();
    paint(&mut app);
    let controls = app.layout.controls;
    assert!(controls.w > 0.0 && controls.x > app.layout.terminal.x);
    let hide = header_hit(&app, HeaderHit::Hide);
    assert!(hide.right() <= controls.x, "{hide:?} {controls:?}");
    assert_eq!(
        app.hover_at(hide.x + 4.0, hide.y + 4.0),
        Hover::TerminalHeader(HeaderHit::Hide)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_terminal_can_sit_right_of_the_editor() {
    use super::add_tab;
    use crate::theme::ISLAND_GAP;

    let dir = sample_folder("terminal-right");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    let bottom = app.layout;
    let rows = app.terminal_size().rows;

    // 移到右侧：编辑区卡片（顶栏、标签栏、编辑区）变窄，面板从顶栏那一行延伸到状态栏上方。
    app.config.terminal_right = Some(true);
    app.relayout();
    let l = app.layout;
    assert_eq!(l.editor.h, bottom.editor.h + bottom.terminal.h + ISLAND_GAP);
    assert_eq!(l.terminal.w, crate::app::terminal::DEFAULT_WIDTH);
    assert_eq!(l.terminal.x, l.editor.right() + ISLAND_GAP);
    assert_eq!(l.top_bar.right(), l.editor.right());
    assert!(l.tabs.right() < l.terminal.x);
    assert_eq!(
        (l.terminal.y, l.terminal.bottom()),
        (l.top_bar.y, l.editor.bottom())
    );
    assert!(app.terminal_size().rows > rows);

    // 标签栏在顶栏那一行里，点得到。
    add_tab(&mut app, "zsh", "$ ");
    paint(&mut app);
    let tab = header_hit(&app, HeaderHit::Tab(0));
    assert!(tab.y < crate::theme::TOP_BAR_END);
    click(&mut app, (tab.x + 20.0, tab.y + tab.h / 2.0));
    assert_eq!(app.terminal.active, 0);

    // 左右拖间距改宽度。
    let gap = l.terminal_divider;
    let (x, y) = (gap.x + gap.w / 2.0, gap.y + 200.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        app.layout.terminal.w,
        crate::app::terminal::DEFAULT_WIDTH + 100.0
    );

    // 右端的位置按钮把面板移回下方。
    let button = header_hit(&app, HeaderHit::Position);
    click(&mut app, (button.x + 4.0, button.y + 4.0));
    assert_eq!(app.config.terminal_right, Some(false));
    assert_eq!(app.layout.terminal.w, bottom.terminal.w);

    // 最右端的按钮收起面板，在位置按钮右边。
    paint(&mut app);
    let hide = header_hit(&app, HeaderHit::Hide);
    assert!(hide.x > header_hit(&app, HeaderHit::Position).right());
    click(&mut app, (hide.x + 4.0, hide.y + 4.0));
    assert!(!app.terminal.open);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_status_bar_button_slides_the_panel_out_and_back() {
    use crate::app::StatusItem;
    use crate::theme::ISLAND_GAP;
    use std::time::Duration;

    let dir = sample_folder("terminal-motion");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    let open = app.layout;
    let size = app.terminal_size();
    let button = |app: &mut App| {
        paint(app);
        let (pill, _) = *app
            .hits
            .status
            .iter()
            .find(|(_, item)| *item == StatusItem::Terminal)
            .unwrap();
        (pill.x + pill.w / 2.0, pill.y + pill.h / 2.0)
    };

    // 收起到一半：编辑区让回来一部分，面板还是原来的大小，贴着编辑区往下滑，shell 不改尺寸。
    app.terminal.frozen = Some(Duration::from_millis(60));
    let at = button(&mut app);
    click(&mut app, at);
    assert!(!app.terminal.open);
    let l = app.layout;
    assert!(open.editor.h < l.editor.h);
    assert!(l.editor.h < open.editor.h + open.terminal.h + ISLAND_GAP);
    assert_eq!(l.terminal.h, open.terminal.h);
    assert_eq!(l.terminal.y, l.editor.bottom() + ISLAND_GAP);
    assert_eq!(app.terminal_size(), size);

    // 到头：编辑区占满，面板不画。
    app.terminal.frozen = Some(Duration::from_secs(1));
    app.terminal.motion = None;
    app.relayout();
    assert_eq!(
        app.layout.editor.h,
        open.editor.h + open.terminal.h + ISLAND_GAP
    );

    // 再次点击滑回，停在原来的位置。
    let at = button(&mut app);
    click(&mut app, at);
    assert!(app.terminal.open);
    assert!(app.terminal.motion.is_some());
    assert_eq!(app.layout.editor, open.editor);
    assert_eq!(app.layout.terminal, open.terminal);

    // 切换位置：前半段在下方收起，后半段从右侧展开。
    app.terminal.frozen = Some(Duration::from_millis(90));
    paint(&mut app);
    assert!(app.run_action(crate::keymap::Action::ToggleTerminalPosition));
    app.relayout();
    assert_eq!(app.config.terminal_right, Some(true));
    let l = app.layout;
    assert_eq!(l.terminal.w, open.terminal.w);
    assert!(open.editor.h < l.editor.h);
    app.terminal.frozen = Some(Duration::from_millis(270));
    app.terminal.motion = None;
    app.toggle_terminal_position();
    app.toggle_terminal_position();
    let l = app.layout;
    assert_eq!(l.terminal.x, l.editor.right() + ISLAND_GAP);
    assert_eq!(l.terminal.w, crate::app::terminal::DEFAULT_WIDTH);
    assert!(l.editor.w > open.editor.w - l.terminal.w - ISLAND_GAP);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_terminal_menu_maximizes_and_the_options_apply() {
    use crate::app::input::context_menu::{MenuEntry, MenuTarget};
    use crate::app::terminal::{DEFAULT_HEIGHT, EDITOR_MIN};
    use crate::keymap::Action;

    let dir = sample_folder("terminal-menu");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ echo hi\r\nhi\r\n$ ");
    let full = app.layout.editor.h + app.layout.terminal.h;

    // 右键点在网格上：终端的菜单，没有选区时「复制」灰着。
    let right_click = |app: &mut App| {
        let (x, y) = cell_center(app, 1, 0);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Right);
        app.on_mouse_up(MouseButton::Right);
        let menu = app.context_menu.as_ref().unwrap();
        assert_eq!(menu.target, MenuTarget::Terminal(None));
        menu.entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(c, on) => Some((*c, *on)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        right_click(&mut app),
        [
            (MenuCommand::Copy, false),
            (MenuCommand::Paste, true),
            (MenuCommand::SelectAll, true),
            (MenuCommand::TerminalFind, true),
            (MenuCommand::TerminalClear, true),
            (MenuCommand::TerminalMaximize, true),
            (MenuCommand::TerminalDock, true),
            (MenuCommand::NewTerminal, true),
            (MenuCommand::CloseTerminal, true),
        ]
    );
    app.run_menu_command(MenuCommand::SelectAll);
    assert!(right_click(&mut app).contains(&(MenuCommand::Copy, true)));
    app.run_menu_command(MenuCommand::Copy);
    assert!(app.copied.as_deref().is_some_and(|t| t.contains("echo hi")));

    // 菜单里切换停靠位置，文字说的是切换后的位置。
    assert_eq!(
        app.menu_label(MenuCommand::TerminalDock).title,
        winc_i18n::t!("terminal-dock-right")
    );
    right_click(&mut app);
    app.run_menu_command(MenuCommand::TerminalDock);
    assert_eq!(app.config.terminal_right, Some(true));
    assert_eq!(
        app.menu_label(MenuCommand::TerminalDock).title,
        winc_i18n::t!("terminal-dock-bottom")
    );
    app.toggle_terminal_position();
    app.terminal.motion = None;
    app.relayout();

    // 最大化：编辑区只留最小的高度；再来一次还原。
    right_click(&mut app);
    app.run_menu_command(MenuCommand::TerminalMaximize);
    assert!(app.terminal.maximized);
    assert_eq!(app.layout.editor.h, EDITOR_MIN);
    assert_eq!(app.layout.editor.h + app.layout.terminal.h, full);
    app.run_action(Action::MaximizeTerminal);
    assert!(!app.terminal.maximized);
    assert_eq!(app.layout.terminal.h, DEFAULT_HEIGHT);

    // 最大化之后拖动分隔条：从当前大小继续拖动，不再是最大化状态。
    app.run_action(Action::MaximizeTerminal);
    let gap = app.layout.terminal_divider;
    let (x, y) = (gap.x + 200.0, gap.y + gap.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y + 50.0);
    app.on_mouse_up(MouseButton::Left);
    assert!(!app.terminal.maximized);
    assert_eq!(app.layout.editor.h, EDITOR_MIN + 50.0);

    // 选中松开就复制（`copyOnSelect`）。
    app.copied = None;
    app.config.terminal_copy_on_select = Some(true);
    let (from, to) = (cell_center(&app, 1, 0), cell_center(&app, 1, 1));
    app.on_mouse_move(from.0 - app.terminal_cell / 2.0 + 0.5, from.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(to.0 + app.terminal_cell / 2.0 - 0.5, to.1);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.copied.as_deref(), Some("hi"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_terminal_scrollbar_can_be_dragged() {
    let dir = sample_folder("terminal-scrollbar");
    let mut app = app_with(&dir.join("short.rs"), false);
    let lines: String = (0..300).map(|i| format!("line {i}\r\n")).collect();
    let term = show_terminal(&mut app, &(lines + "$ "));
    paint(&mut app);
    let (track, scroll, content) = app.terminal_scrollbar().unwrap();
    let max = app.terminal.tab().unwrap().max_scroll();
    assert!(max > 100);
    assert_eq!(scroll, content - track.h);

    // 按在轨道上面，拖到顶：翻到最早的一行；拖到底：回到最新的。
    let x = track.right() - 3.0;
    app.on_mouse_move(x, track.y + 1.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, track.y - 20.0);
    assert_eq!(app.terminal.tab().unwrap().scroll, max);
    super::serve(&mut app, &term);
    paint(&mut app);
    app.on_mouse_move(x, track.y + track.h / 2.0);
    let middle = app.terminal.tab().unwrap().scroll;
    assert!(middle > 0 && middle < max, "{middle}");
    app.on_mouse_move(x, track.bottom() + 20.0);
    assert_eq!(app.terminal.tab().unwrap().scroll, 0);
    app.on_mouse_up(MouseButton::Left);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn terminal_tabs_come_back_with_the_folder() {
    use super::add_tab;

    let dir = sample_folder("terminal-session");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "zsh", "$ ");
    app.terminal.tabs[0].cwd = b"/tmp/a".to_vec();
    app.terminal.tabs[1].cwd = b"/tmp/b".to_vec();
    app.terminal.tabs[1].name = "server".into();
    app.terminal.active = 1;
    app.terminal.height = 300.0;
    app.remember_session();
    let root = app.editor.root().unwrap().to_path_buf();
    let session = app.state.session_for(&root).unwrap();
    let saved = session.terminal.clone().unwrap();
    assert_eq!(saved.tabs, [Path::new("/tmp/a"), Path::new("/tmp/b")]);
    assert_eq!((saved.active, saved.open, saved.height), (1, true, 300.0));
    assert_eq!(saved.names, ["", "server"]);

    // 重新打开该目录：两个标签在各自的目录中重开，仍显示第二个，面板高度不变。
    let mut app = blank_app(false);
    app.config.terminal_enabled = Some(true);
    app.attach_idle_helper();
    app.restore_terminal(&saved);
    assert_eq!(app.terminal.want, [b"/tmp/a".to_vec(), b"/tmp/b".to_vec()]);
    assert!(app.terminal.open);
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 2);
    assert_eq!(app.terminal.active, 1);
    assert_eq!(app.terminal.tabs[1].label(), "server");
    assert_eq!(app.layout.terminal.h, 300.0);
    // 终端里已经有标签了：不再加。
    app.restore_terminal(&saved);
    assert!(app.terminal.want.is_empty());

    // 上次面板收着：等打开面板时才建，也不多开一个空的。
    let mut app = blank_app(false);
    app.config.terminal_enabled = Some(true);
    app.attach_idle_helper();
    app.restore_terminal(&winc_config::TerminalSession {
        open: false,
        ..saved.clone()
    });
    assert!(!app.terminal.open && app.terminal.tabs.is_empty());
    app.toggle_terminal();
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 2);

    // 面板收着时按 ⌘T（或「在终端中打开」）：恢复的两个标签仍然创建，新的加在后面。
    let mut app = blank_app(false);
    app.config.terminal_enabled = Some(true);
    app.attach_idle_helper();
    app.restore_terminal(&winc_config::TerminalSession {
        open: false,
        ..saved.clone()
    });
    app.new_terminal_tab_in(b"/tmp/c".to_vec());
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 3);
    assert_eq!(app.terminal.tabs[1].label(), "server");
    assert_eq!(app.terminal.active, 2, "看的是新建的那一个");

    // 辅助进程崩了、重新开始：还看着原来看的那一个；正在改的名字定下来，跟着重开的标签走。
    app.activate_terminal_tab(1);
    let mut field = crate::field::TextField::default();
    field.set_text("web");
    app.terminal.rename = Some((app.terminal.tabs[1].id, field));
    app.focus = Focus::TerminalRename;
    app.terminal_helper_gone();
    app.attach_idle_helper();
    app.restart_terminal_tab();
    assert_eq!(app.terminal.tabs.len(), 3);
    assert_eq!(app.terminal.active, 1);
    assert_eq!(app.terminal.tabs[1].label(), "web");
    assert!(app.terminal.rename.is_none());
    assert_eq!(app.focus, Focus::Terminal);

    // 终端模块关着时不恢复。
    let mut app = blank_app(false);
    app.restore_terminal(&saved);
    assert!(app.terminal.want.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn switching_folders_swaps_the_terminals() {
    // 回归：换目录以后终端还留着原目录的标签，工作目录也还是原目录。
    use super::add_tab;

    let first = sample_folder("terminal-switch-a");
    let second = sample_folder("terminal-switch-b");
    let mut app = app_with(&first.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "zsh", "$ ");
    app.terminal.tabs[0].cwd = b"/tmp/a".to_vec();
    app.terminal.tabs[1].cwd = b"/tmp/b".to_vec();

    // 新目录没有记下终端：原来的标签关掉，面板开着，在新目录里开一个。
    app.switch_folder(&second);
    assert!(app.terminal.tabs.is_empty());
    assert_eq!(app.terminal.want, [Vec::<u8>::new()]);
    assert!(app.terminal.open);
    // 原目录的标签记进了它的现场，换回去时重开。
    app.terminal.want.clear();
    app.switch_folder(&first);
    assert_eq!(app.terminal.want, [b"/tmp/a".to_vec(), b"/tmp/b".to_vec()]);

    // 关闭目录：终端一起关掉，面板收起。
    add_tab(&mut app, "zsh", "$ ");
    app.close_folder();
    assert!(app.terminal.tabs.is_empty() && app.terminal.want.is_empty());
    assert!(!app.terminal.open);

    std::fs::remove_dir_all(&first).unwrap();
    std::fs::remove_dir_all(&second).unwrap();
}

#[test]
fn bells_and_notifications_reach_the_user() {
    use super::add_tab;
    use winc_pty::protocol::{Event, FromHelper};

    let dir = sample_folder("terminal-bell");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "zsh", "$ ");
    app.terminal.active = 0;
    app.window.focused = true;
    let (first, second) = (app.terminal.tabs[0].id, app.terminal.tabs[1].id);
    let event = |app: &mut App, tab: u16, event: Event| {
        app.terminal_message(FromHelper::Event { tab, event });
    };

    // 正在看的标签响铃：什么也不做。其他标签响铃：标签上亮点。
    event(&mut app, first, Event::Bell);
    assert_eq!(app.terminal.tabs[0].unseen, None);
    event(&mut app, second, Event::Bell);
    assert_eq!(app.terminal.tabs[1].unseen, Some(false));

    // 程序发的通知：窗口在前台时直接提示。
    event(
        &mut app,
        first,
        Event::Notify {
            title: "构建".into(),
            body: "完成".into(),
        },
    );
    assert_eq!(app.notice.text.as_deref(), Some("构建：完成"));

    // 窗口不在前台：回到窗口时再提示。
    app.notice.text = None;
    app.window.focused = false;
    event(
        &mut app,
        first,
        Event::Notify {
            title: String::new(),
            body: "测试通过".into(),
        },
    );
    assert_eq!(app.notice.text, None);
    app.terminal_window_focused();
    assert_eq!(app.notice.text.as_deref(), Some("测试通过"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_maximized() {
    let dir = sample_folder("terminal-max-shot");
    let lines: String = (0..300).map(|i| format!("line {i}\r\n")).collect();
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        let term = show_terminal(&mut app, &(lines.clone() + "$ "));
        app.toggle_terminal_maximized();
        paint(&mut app);
        // 向上翻一段：显示滚动条。
        app.scroll_terminal(120);
        super::serve(&mut app, &term);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-maximized-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn crowded_terminal_tabs_shrink_and_keep_the_active_one_in_view() {
    use super::add_tab;
    use crate::app::terminal::strip::TAB_SQUEEZED;

    let dir = sample_folder("terminal-crowded");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_right = Some(true);
    show_terminal(&mut app, "$ ");
    for name in [
        "cargo watch",
        "npm run dev",
        "htop",
        "ssh server",
        "python",
        "make",
        "node",
        "git log",
    ] {
        add_tab(&mut app, name, "$ ");
    }
    paint(&mut app);

    // 都窄到最窄还放不下：正在看的（最后一个）和「+」都在右边的按钮左边，看得见、点得到。
    assert!(app.terminal.strip.widths.iter().all(|&w| w == TAB_SQUEEZED));
    let active = header_hit(&app, HeaderHit::Tab(8));
    let plus = header_hit(&app, HeaderHit::New);
    assert_eq!(active.w, TAB_SQUEEZED);
    let position = header_hit(&app, HeaderHit::Position);
    assert!(active.right() < plus.x && plus.right() <= position.x);
    // 移出可视区的第一个标签被裁掉，点击不到。
    assert_eq!(header_hit(&app, HeaderHit::Tab(0)).w, 0.0);

    // 滚轮停在标签栏上：横向滚动标签，终端内容不动，也不切换标签；竖直和水平滚轮都可以。
    let header = header_hit(&app, HeaderHit::Tab(8));
    app.on_mouse_move(header.x + 8.0, header.y + 8.0);
    let lines = app.terminal.tab().unwrap().scroll;
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 40.0));
    paint(&mut app);
    assert_eq!(header_hit(&app, HeaderHit::Tab(0)).w, TAB_SQUEEZED);
    assert_eq!(app.terminal.active, 8);
    assert_eq!(app.terminal.tab().unwrap().scroll, lines);
    app.on_wheel(MouseScrollDelta::LineDelta(-40.0, 0.0));
    paint(&mut app);
    assert_eq!(header_hit(&app, HeaderHit::Tab(8)).w, TAB_SQUEEZED);

    // 切换到第一个标签：标签栏滚回最左边。
    app.activate_terminal_tab(0);
    paint(&mut app);
    assert_eq!(header_hit(&app, HeaderHit::Tab(0)).w, TAB_SQUEEZED);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_many_tabs() {
    use super::add_tab;
    let dir = sample_folder("terminal-many-shot");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_right = Some(true);
    show_terminal(&mut app, "$ ");
    for name in ["cargo watch", "npm run dev", "htop", "ssh server", "python"] {
        add_tab(&mut app, name, "$ ");
    }
    paint(&mut app);
    shoot(&mut app, "terminal-many-tabs");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_rename() {
    use super::add_tab;
    let dir = sample_folder("terminal-rename-shot");
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        show_terminal(&mut app, "$ ");
        add_tab(&mut app, "zsh", "$ ");
        app.open_terminal_rename(0);
        paint(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-rename-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_right() {
    use super::add_tab;
    let dir = sample_folder("terminal-right-shot");
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_right = Some(true);
        show_terminal(
            &mut app,
            "\x1b[34m\x1b[1m~/project/winc\x1b[0m \x1b[35mmain\x1b[0m \x1b[32m❯\x1b[0m ls\r\nCargo.toml  crates  docs\r\n\x1b[34m\x1b[1m~/project/winc\x1b[0m \x1b[32m❯\x1b[0m ",
        );
        add_tab(&mut app, "cargo", "$ ");
        app.terminal.active = 0;
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-right-{theme}"));
        // 收起到一半，再到收好：状态栏的开关弹起，后台的命令失败了时带红点。
        app.terminal.frozen = Some(std::time::Duration::from_millis(50));
        app.set_terminal_open(false);
        shoot(&mut app, &format!("terminal-right-hiding-{theme}"));
        app.terminal.motion = None;
        app.terminal.tabs[1].unseen = Some(true);
        shoot(&mut app, &format!("terminal-right-hidden-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_program_hears_when_the_terminal_loses_focus() {
    let dir = sample_folder("terminal-focus-out");
    let mut app = app_with(&dir.join("short.rs"), false);
    // 程序要了焦点上报（1004）。
    show_terminal(&mut app, "\x1b[?1004h$ ");
    let log = app.attach_recording_helper(&dir);
    app.window.focused = true;
    app.after_terminal_frame();
    assert_eq!(app.input_sent_to(&log, 1), b"\x1b[I");

    // 收起面板：焦点回了编辑区。
    app.set_terminal_open(false);
    app.after_terminal_frame();
    assert_eq!(app.input_sent_to(&log, 1), b"\x1b[I\x1b[O");

    // 打开、回到终端，再开一个新标签：看的换成了新的，原来那个也失去焦点。
    app.set_terminal_open(true);
    app.focus = Focus::Terminal;
    app.after_terminal_frame();
    app.new_terminal_tab();
    app.relayout();
    assert_ne!(app.terminal.tab().unwrap().id, 1);
    assert_eq!(app.input_sent_to(&log, 1), b"\x1b[I\x1b[O\x1b[I\x1b[O");

    // 关掉最后一个标签：连接跟着放掉，下次新建时启动新的辅助进程，不发给正在退出的这一个。
    app.close_terminal_tab(1);
    assert!(app.terminal_connected());
    app.close_terminal_tab(0);
    assert!(!app.terminal_connected());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn code_sent_to_an_exited_shell_starts_it_again() {
    let dir = sample_folder("terminal-send-exited");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ exit");
    let log = app.attach_recording_helper(&dir);
    app.terminal.tabs[0].ended = Some(TabEnd::Exited(0));
    app.focus = Focus::Editor;

    // ⌃⇧↩：标签重开，光标那一行等新的 shell 启动后送出，不会丢弃。
    app.send_to_terminal();
    let tab = app.terminal.tab().unwrap();
    assert!(tab.ended.is_none());
    assert_ne!(tab.id, 1);
    assert_eq!(app.input_sent_to(&log, tab.id), b"fn main() {\r");
    assert_eq!(app.focus, Focus::Editor);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 启动时恢复的目录在窗口创建之前打开，窗口建好以后按现场恢复终端：开着的面板、标签和目录。
#[test]
fn the_first_window_restores_the_terminal_panel() {
    let dir = sample_folder("terminal-first-window");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    app.terminal.tabs[0].cwd = b"/tmp/a".to_vec();
    app.terminal.height = 300.0;
    app.remember_session();
    let state = app.state.clone();
    let root = app.editor.root().unwrap().to_path_buf();

    // 上次开着：恢复后开着，没有打开动画，高度不变，标签在原来的目录里。
    let open = |open: bool| {
        let mut session = state.session_for(&root).unwrap().clone();
        session.terminal.as_mut().unwrap().open = open;
        let mut state = state.clone();
        state.set_session(session);
        let mut app = app_with(&dir.join("short.rs"), false);
        app.state = state;
        app.config.terminal_enabled = Some(true);
        app.attach_idle_helper();
        app.restore_terminal_for_window();
        app
    };
    let mut app = open(true);
    assert!(app.terminal.open);
    assert!(app.terminal.motion.is_none());
    assert_eq!(app.terminal.height, 300.0);
    assert_eq!(app.terminal.want, [b"/tmp/a".to_vec()]);
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 1);

    // 上次收着：恢复后收着，打开面板时才建标签。
    let mut app = open(false);
    assert!(!app.terminal.open);
    app.relayout();
    assert!(app.terminal.tabs.is_empty());
    app.toggle_terminal();
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 停靠位置写进配置文件，下次启动读回来。
#[test]
fn the_panel_position_survives_a_restart() {
    let dir = sample_folder("terminal-position");
    let path = dir.join("config.json");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config_path = Some(path.clone());
    show_terminal(&mut app, "$ ");
    app.toggle_terminal_position();
    let (config, _) = winc_config::Config::load_from(&path);
    assert_eq!(config.terminal_right, Some(true));
    app.toggle_terminal_position();
    let (config, _) = winc_config::Config::load_from(&path);
    assert_eq!(config.terminal_right, Some(false));
    std::fs::remove_dir_all(&dir).unwrap();
}
