//! 终端命令块（shell 集成）的测试：色带、折起输出、⌘↑ ⌘↓ 跳转、吸顶的命令行、长命令结束时
//! 的提醒、F8 在报错之间跳转。
//!
//! 屏幕内容由本地终端仿真读入带 OSC 133 标记的输出生成，命令块从仿真中取出，与辅助进程发来的
//! 数据相同。

use super::*;
use super::{NONCE, add_tab, cell_center, click, serve, show_terminal};
use crate::app::terminal::{self, BlockHit, ViewRow};
use winc_pty::protocol::Block;

/// 一条命令：提示符、命令行、输出的几行、退出码，与集成脚本发出的一致。
fn command(cmd: &str, output: &[&str], exit: i32) -> String {
    let mut out = format!(
        "\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C;nonce={NONCE};cmdline_url={}\x07",
        cmd.replace(' ', "%20")
    );
    for line in output {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("\x1b]133;D;{exit}\x07"));
    out
}

/// 已按回车执行 `sleep 2`、尚无输出时的屏幕：光标停在下一行行首。
fn sleeping() -> String {
    format!(
        "\x1b]133;A\x07$ \x1b]133;B\x07sleep 2\r\n\x1b]133;C;nonce={NONCE};cmdline_url=sleep%202\x07"
    )
}

/// 当前目录（OSC 7）。
fn cwd(dir: &std::path::Path) -> String {
    format!("\x1b]7;file://host{}\x07", dir.display())
}

/// 把仿真中的命令块交给正在看的标签，等同于收到辅助进程的消息。
fn sync_blocks(app: &mut App, term: &mut winc_pty::Terminal) {
    let id = app.terminal.tab().unwrap().id;
    if let Some(blocks) = term.take_blocks() {
        app.terminal_blocks(id, blocks, false);
    }
}

fn tab(app: &App) -> &terminal::TermTab {
    app.terminal.tab().unwrap()
}

/// 网格上第 `row` 行的色带的位置。
fn band_at(app: &App, row: usize) -> (f32, f32) {
    let card = app.layout.terminal;
    let (_, y) = cell_center(app, row, 0);
    (card.x + terminal::BAND_X + 1.0, y)
}

#[test]
fn bands_fold_and_unfold_a_command_output() {
    let dir = sample_folder("terminal-fold");
    let mut app = app_with(&dir.join("short.rs"), false);
    let screen = [
        command("ls", &["a", "b", "c"], 0),
        command("false", &[], 1),
        "\x1b]133;A\x07$ \x1b]133;B\x07".to_string(),
    ]
    .concat();
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);
    paint(&mut app);
    let t = tab(&app);
    assert_eq!(t.blocks.len(), 3);
    assert_eq!(t.blocks[0].output, Some(1));
    assert_eq!(t.blocks[1].exit, Some(1));

    // 色带上的悬停、点击：折起 ls 的三行输出，网格上只剩一行「⋯ 3 行」。
    let at = band_at(&app, 2);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(app.hover, Hover::TerminalBlock(BlockHit::Band(0)));
    let before = tab(&app).display_len();
    click(&mut app, at);
    let t = tab(&app);
    assert!(t.folded.contains(&t.blocks[0].id));
    assert_eq!(t.display_len(), before - 2);
    assert_eq!(t.view()[1], ViewRow::Fold(0, 3));
    assert_eq!(
        t.view()[2],
        ViewRow::Line(4),
        "折起的下一行是 false 的提示符"
    );
    // 光标仍在提示符上：光标所在的行向上移动了两行。
    assert_eq!(
        t.view().iter().position(|r| *r == ViewRow::Line(5)),
        Some(3)
    );
    paint(&mut app);

    // 点折起的那一行展开。
    let (x, y) = cell_center(&app, 1, 3);
    click(&mut app, (x, y));
    assert!(tab(&app).folded.is_empty());
    assert_eq!(tab(&app).display_len(), before);

    // ⌥⌘[ 看着最新输出时折起最后一条结束了、有输出的命令；false 没有输出，折的是 ls。
    app.fold_terminal_output(true);
    assert_eq!(tab(&app).folded.len(), 1);
    app.fold_terminal_output(false);
    assert!(tab(&app).folded.is_empty());

    // ⇧ 点色带选中整块输出。
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    let at = band_at(&app, 1);
    click(&mut app, at);
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(
        tab(&app).selected_text().unwrap().as_deref(),
        Some("a\nb\nc")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_menu_on_a_command_copies_it_and_its_output() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let dir = sample_folder("terminal-block-menu");
    let mut app = app_with(&dir.join("short.rs"), false);
    let screen = [
        command("echo hi", &["hi", "there"], 0),
        "\x1b]133;A\x07$ \x1b]133;B\x07".to_string(),
    ]
    .concat();
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);
    paint(&mut app);
    let id = tab(&app).blocks[0].id;

    // 右键点在第 `row` 行：菜单是哪个块的，前面几项是什么。
    let right_click = |app: &mut App, row: usize| {
        let (x, y) = cell_center(app, row, 1);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Right);
        app.on_mouse_up(MouseButton::Right);
        let menu = app.context_menu.as_ref().unwrap();
        let items = menu
            .entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item(c, on) => Some((*c, *on)),
                _ => None,
            })
            .take(3)
            .collect::<Vec<_>>();
        (menu.target.clone(), items)
    };
    let (target, items) = right_click(&mut app, 1);
    assert_eq!(target, MenuTarget::Terminal(Some(id)));
    assert_eq!(
        items,
        [
            (MenuCommand::CopyCommand, true),
            (MenuCommand::CopyOutput, true),
            (MenuCommand::Rerun, true),
        ]
    );
    app.run_menu_command(MenuCommand::CopyCommand);
    assert_eq!(app.copied.as_deref(), Some("echo hi"));
    right_click(&mut app, 2);
    app.run_menu_command(MenuCommand::CopyOutput);
    assert_eq!(app.copied.as_deref(), Some("hi\nthere"));

    // 提示符那一行的块还没运行：没有块的几项。
    let (target, items) = right_click(&mut app, 3);
    assert_eq!(target, MenuTarget::Terminal(None));
    assert_eq!(items[0].0, MenuCommand::Copy);
    app.context_menu = None;

    // 有命令正在执行时不能重新运行。
    app.terminal.tab_mut().unwrap().blocks[1].output = Some(4);
    assert!(!app.terminal_can_rerun(id));
    let (_, items) = right_click(&mut app, 1);
    assert_eq!(items[2], (MenuCommand::Rerun, false));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_running_command_band_stops_at_the_output() {
    let dir = sample_folder("terminal-running-band");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut term = show_terminal(&mut app, &sleeping());
    sync_blocks(&mut app, &mut term);
    let t = tab(&app);
    assert!(t.blocks[0].running());
    // 色带只在命令那一行，不顺着下面的空行画到面板底。
    assert_eq!(t.block_at(0), Some(0));
    assert_eq!(t.block_at(1), None);
    assert_eq!(t.block_at(5), None);

    // 有了输出：跟着长到输出的最后一行。
    term.feed(b"a\r\nb");
    let frame = term.take_frame().unwrap();
    app.terminal.tab_mut().unwrap().apply(frame);
    let t = tab(&app);
    assert_eq!(t.block_at(2), Some(0));
    assert_eq!(t.block_at(3), None);

    // 启动全屏程序（`top`）：备用屏的行不属于任何块，旁边不画色带和结果，也点不到。
    term.feed(b"\x1b[?1049h\x1b[Hfull screen");
    let frame = term.take_frame().unwrap();
    app.terminal.tab_mut().unwrap().apply(frame);
    let t = tab(&app);
    assert_eq!(t.block_at(0), None);
    assert_eq!(t.block_at(2), None);
    let grid = app.terminal_grid();
    assert_eq!(app.terminal_block_hit(grid.x - 4.0, grid.y + 2.0), None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_running() {
    let dir = sample_folder("terminal-running-shot");
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        let screen = [command("ls", &["a", "b"], 0), sleeping()].concat();
        let mut term = show_terminal(&mut app, &screen);
        sync_blocks(&mut app, &mut term);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-running-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn command_jumps_walk_the_history_and_the_command_sticks_on_top() {
    let dir = sample_folder("terminal-jump");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut screen = String::new();
    for i in 0..6 {
        let lines: Vec<String> = (0..12).map(|j| format!("out {i}.{j}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        screen.push_str(&command(&format!("step {i}"), &lines, 0));
    }
    screen.push_str("\x1b]133;A\x07$ ");
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);
    let rows = tab(&app).rows.len() as u64;
    assert!(tab(&app).history.1 > rows, "要有回滚缓冲");

    // ⌘↑ 跳到最后一条命令，再往上一条；提示符在网格第一行。
    app.jump_terminal_command(false);
    serve(&mut app, &term);
    let t = tab(&app);
    let last = &t.blocks[5];
    assert_eq!(t.jump, Some(last.id));
    assert_eq!(t.top(), last.prompt);
    app.jump_terminal_command(false);
    serve(&mut app, &term);
    let t = tab(&app);
    assert_eq!(t.top(), t.blocks[4].prompt);
    assert!(t.scroll > 0);
    paint(&mut app);

    // ⌘↓ 往下，过了最后一条回到最新的输出。
    app.jump_terminal_command(true);
    app.jump_terminal_command(true);
    assert_eq!((tab(&app).scroll, tab(&app).jump), (0, None));

    // 往上翻到第 3 条命令的输出中间：它的命令行吸在顶上，点它跳回那条命令。
    let target = tab(&app).blocks[2].output.unwrap() + 3;
    app.terminal.tab_mut().unwrap().scroll_to(target, 0);
    serve(&mut app, &term);
    assert_eq!(tab(&app).sticky(), Some(2));
    paint(&mut app);
    let (x, y) = cell_center(&app, 0, 4);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::TerminalBlock(BlockHit::Sticky));
    click(&mut app, (x, y));
    assert_eq!(tab(&app).top(), tab(&app).blocks[2].prompt);
    assert_eq!(tab(&app).sticky(), None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_sticky_command_survives_a_width_change() {
    let dir = sample_folder("terminal-sticky-width");
    let mut app = app_with(&dir.join("short.rs"), false);
    // 还在运行的命令，输出比一屏多：看着最新的输出时它的命令行吸在顶上。
    let mut screen = command("cargo build", &[], 0);
    screen.push_str("\x1b]133;A\x07$ \x1b]133;B\x07cargo build\r\n\x1b]133;C\x07");
    for i in 0..60 {
        screen.push_str(&format!("out {i}\r\n"));
    }
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);
    serve(&mut app, &term);
    let i = tab(&app).sticky().expect("命令吸在顶上");
    assert!(
        tab(&app)
            .sticky_line(i)
            .unwrap()
            .text
            .contains("cargo build")
    );

    // 改宽度：辅助进程重排，命令块与整屏的一帧一起到达，缓存失效；取回之前吸顶的仍是那一行。
    let id = tab(&app).id;
    let mut size = tab(&app).size;
    size.cols -= 5;
    term.resize(size);
    sync_blocks(&mut app, &mut term);
    let frame = term.take_frame().unwrap();
    assert!(frame.full);
    app.terminal_message(winc_pty::protocol::FromHelper::Frame { tab: id, frame });
    let i = tab(&app).sticky().expect("命令还吸在顶上");
    assert!(
        tab(&app)
            .sticky_line(i)
            .unwrap()
            .text
            .contains("cargo build")
    );
    serve(&mut app, &term);
    assert!(
        tab(&app)
            .sticky_line(i)
            .unwrap()
            .text
            .contains("cargo build")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn folding_while_scrolled_keeps_the_view_still() {
    let dir = sample_folder("terminal-fold-scroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut screen = String::new();
    for i in 0..4 {
        let lines: Vec<String> = (0..12).map(|j| format!("{i}.{j}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        screen.push_str(&command(&format!("c{i}"), &lines, 0));
    }
    screen.push_str("\x1b]133;A\x07$ ");
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);

    // 翻到第 2 条命令的提示符，折起它下面那条（在可见范围以下）：网格第一行不动。
    let prompt = tab(&app).blocks[1].prompt;
    app.terminal.tab_mut().unwrap().scroll_to(prompt, 0);
    serve(&mut app, &term);
    let id = tab(&app).blocks[2].id;
    app.terminal.tab_mut().unwrap().set_folded(id, true);
    assert_eq!(tab(&app).top(), prompt);
    // 折起网格第一行所在的那条（⌥⌘[ 往上翻着时折的就是它）：折起的那一行停在顶上。
    app.fold_terminal_output(true);
    let t = tab(&app);
    assert!(t.folded.contains(&t.blocks[1].id));
    assert_eq!(t.top(), prompt);
    assert!(matches!(t.view()[1], ViewRow::Fold(1, 12)));

    // 查找翻到折起的输出里：先展开。
    let hidden = tab(&app).blocks[2].output.unwrap() + 4;
    app.terminal.tab_mut().unwrap().reveal(hidden);
    let t = tab(&app);
    assert!(!t.folded.contains(&t.blocks[2].id));
    assert!(t.view().contains(&ViewRow::Line(hidden)));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn f8_walks_the_errors_of_the_last_failed_command() {
    let dir = sample_folder("terminal-errors");
    let mut app = app_with(&dir.join("a.txt"), false);
    let screen = [
        cwd(&dir),
        command(
            "cargo build",
            &["error[E0425]: oops", "  --> src/main.rs:2:5", "   |"],
            101,
        ),
        command(
            "cargo test",
            &[
                "error: first",
                "  --> short.rs:3:1",
                "warning: missing.rs:1:1 is not there",
                "error: second",
                "  --> src/main.rs:1:4",
            ],
            101,
        ),
        // 没有找到结果的 `lsof` 也退出 1，但没有输出：F8 跳过它，仍找上面的 cargo test。
        command("lsof -i :30030", &[], 1),
        command("echo ok", &["ok"], 0),
        "\x1b]133;A\x07$ ".to_string(),
    ]
    .concat();
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);

    // F8：最后一条失败的是 cargo test，不存在的文件跳过。
    app.step_terminal_error(true);
    let nav = app.terminal.errors.as_ref().unwrap();
    assert_eq!(nav.spots.len(), 2);
    assert_eq!(nav.current, Some(0));
    assert_eq!(
        tab(&app).selected_text().unwrap().as_deref(),
        Some("short.rs:3:1")
    );
    app.step_terminal_error(true);
    assert_eq!(
        tab(&app).selected_text().unwrap().as_deref(),
        Some("src/main.rs:1:4")
    );
    // ⇧F8 往回，绕到头。
    app.step_terminal_error(false);
    app.step_terminal_error(false);
    assert_eq!(app.terminal.errors.as_ref().unwrap().current, Some(1));

    // ↩ 打开那个文件的那一行那一列。
    assert!(app.terminal_error_key(true));
    let doc = app.editor.active().unwrap();
    let opened = winc_config::files::canonicalize(doc.path.as_deref().unwrap()).unwrap();
    assert_eq!(
        opened,
        winc_config::files::canonicalize(dir.join("src/main.rs")).unwrap()
    );
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (0, 3));
    assert_eq!(app.focus, Focus::Editor);

    // 其他按键结束跳转，并照常交给 shell。
    app.focus = Focus::Terminal;
    assert!(!app.terminal_error_key(false));
    assert!(app.terminal.errors.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_long_command_finishing_in_the_background_is_noticed() {
    let dir = sample_folder("terminal-notice");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    let _ = add_tab(&mut app, "zsh", "$ ");
    let running = Block {
        id: 1,
        prompt: 0,
        command: 0,
        output: Some(1),
        text: "cargo build".into(),
        ..Block::default()
    };
    let done = |exit, duration| Block {
        end: Some(5),
        exit: Some(exit),
        duration: Some(duration),
        ..running.clone()
    };

    // 不在看的标签里结束：标签上亮点，失败的是红点。
    app.terminal_blocks(1, vec![running.clone()], false);
    app.terminal_blocks(1, vec![done(1, 300)], false);
    assert_eq!(app.terminal.tabs[0].unseen, Some(true));
    assert!(
        app.terminal.pending_notice.is_none(),
        "窗口在前台、命令也不长"
    );
    // 退出 1 但没有输出（`lsof`、`grep` 未找到）：显示普通的点，不是红点。
    let mut quiet = running.clone();
    quiet.id = 3;
    app.terminal_blocks(1, vec![quiet.clone()], false);
    app.terminal_blocks(
        1,
        vec![Block {
            end: Some(1),
            exit: Some(1),
            ..quiet
        }],
        false,
    );
    assert_eq!(app.terminal.tabs[0].unseen, Some(false));
    app.activate_terminal_tab(0);
    assert_eq!(app.terminal.tabs[0].unseen, None);

    // 窗口不在前台时结束的长命令：回到窗口时提示。
    app.window.focused = false;
    let mut again = running.clone();
    again.id = 2;
    app.terminal_blocks(1, vec![again.clone()], false);
    app.terminal_blocks(
        1,
        vec![Block {
            id: 2,
            ..done(0, 12_400)
        }],
        false,
    );
    assert_eq!(app.terminal.tabs[0].unseen, None, "回来时看到的就是它");
    app.window.focused = true;
    app.terminal_window_focused();
    assert_eq!(
        app.notice.text.as_deref(),
        Some("「cargo build」已完成（12.4 s）")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 后台标签重排（宽度改变）后命令块先到、整屏的那一帧要等该标签成为正在看的标签才到：命令
/// 结束时仍然标记，块等那一帧到达后再替换，避免用新行号对应旧屏幕。
#[test]
fn blocks_ahead_of_their_frame_still_mark_a_background_tab() {
    let dir = sample_folder("terminal-early-blocks");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut term = show_terminal(&mut app, "$ ");
    let _ = add_tab(&mut app, "zsh", "$ ");
    let running = Block {
        id: 1,
        output: Some(1),
        text: "make".into(),
        ..Block::default()
    };
    app.terminal_blocks(1, vec![running.clone()], false);
    let done = Block {
        end: Some(5),
        exit: Some(0),
        ..running.clone()
    };
    app.terminal_blocks(1, vec![done.clone()], true);
    let tab = &app.terminal.tabs[0];
    assert_eq!(tab.unseen, Some(false));
    assert_eq!(tab.blocks, [running]);
    // 同一批命令块再来一次（与那一帧一起）：不再计为一次结束。
    app.terminal.tabs[0].unseen = None;
    app.terminal_blocks(1, vec![done.clone()], true);
    assert_eq!(app.terminal.tabs[0].unseen, None);

    term.feed(b"x");
    let frame = term.take_frame().unwrap();
    app.terminal_message(winc_pty::protocol::FromHelper::Frame { tab: 1, frame });
    let tab = &app.terminal.tabs[0];
    assert_eq!(tab.blocks, [done]);
    assert!(tab.early_blocks.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_blocks() {
    let dir = sample_folder("terminal-blocks-shot");
    let mut screen = cwd(&dir);
    screen.push_str(&command(
        "cargo build",
        &["   Compiling winc v0.1.0", "    Finished `dev` profile"],
        0,
    ));
    screen.push_str(&command(
        "cargo test",
        &[
            "error[E0425]: cannot find value `x`",
            "  --> src/main.rs:2:5",
            "test result: FAILED",
        ],
        101,
    ));
    screen.push_str(&command(
        "git status",
        &["On branch main", "nothing to commit"],
        0,
    ));
    screen.push_str("\x1b]133;A\x07$ \x1b]133;B\x07");
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.terminal.height = 320.0;
        let mut term = show_terminal(&mut app, &screen);
        app.terminal.height = 320.0;
        app.relayout();
        sync_blocks(&mut app, &mut term);
        serve(&mut app, &term);
        // 时长由本地仿真测出，接近于 0：改成可读的数值。
        if let Some(tab) = app.terminal.tab_mut() {
            tab.blocks[0].duration = Some(12_400);
            tab.blocks[1].duration = Some(800);
            let id = tab.blocks[2].id;
            tab.set_folded(id, true);
        }
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-blocks-{theme}"));
        // 鼠标位于折起的那一行上：该行高亮，点击即可展开。
        let row = tab(&app)
            .view()
            .iter()
            .position(|r| matches!(r, ViewRow::Fold(..)));
        let (x, y) = cell_center(&app, row.unwrap(), 2);
        app.on_mouse_move(x, y);
        shoot(&mut app, &format!("terminal-fold-hover-{theme}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rows_hidden_under_the_sticky_command_or_a_fold_are_not_shown() {
    let dir = sample_folder("terminal-hidden-rows");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut screen = String::new();
    for i in 0..4 {
        let lines: Vec<String> = (0..12).map(|j| format!("https://e.com/{i}/{j}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        screen.push_str(&command(&format!("c{i}"), &lines, 0));
    }
    screen.push_str("\x1b]133;A\x07$ ");
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);

    // 翻到第 2 条命令的输出中间：它的命令行吸在顶上，盖着网格第一行。
    let output = tab(&app).blocks[1].output.unwrap();
    app.terminal.tab_mut().unwrap().scroll_to(output + 4, 0);
    serve(&mut app, &term);
    assert_eq!(tab(&app).sticky(), Some(1));
    assert!(!app.terminal_shows_line(cell_center(&app, 0, 0).1));
    assert!(app.terminal_shows_line(cell_center(&app, 1, 0).1));

    // ⌘ 悬停在吸顶的命令行上：底下那一行的网址看不见，不认。
    app.modifiers = command_modifiers();
    let (x, y) = cell_center(&app, 0, 3);
    app.on_mouse_move(x, y);
    app.update_terminal_link();
    assert_eq!(app.terminal.link, None);
    let (x, y) = cell_center(&app, 1, 3);
    app.on_mouse_move(x, y);
    app.update_terminal_link();
    assert_eq!(app.terminal.link.as_ref().map(|l| l.line), Some(output + 5));
    app.modifiers = winit::keyboard::ModifiersState::empty();

    // 要露出的正是被盖着的那一行：翻过去。
    app.terminal.tab_mut().unwrap().reveal(output + 4);
    let t = tab(&app);
    let at = t
        .view()
        .iter()
        .position(|r| *r == ViewRow::Line(output + 4));
    assert!(at.is_some_and(|at| at > 0 || t.sticky().is_none()));

    // 折起的那一行：底下的字看不见。
    let id = tab(&app).blocks[1].id;
    app.terminal.tab_mut().unwrap().set_folded(id, true);
    serve(&mut app, &term);
    let t = tab(&app);
    let row = t
        .view()
        .iter()
        .position(|r| matches!(r, ViewRow::Fold(1, _)));
    let row = row.expect("折起的那一行在网格上");
    assert!(!app.terminal_shows_line(cell_center(&app, row, 0).1));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_full_screen_program_starts_at_the_bottom_and_ends_f8_elsewhere() {
    let dir = sample_folder("terminal-alt-scroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    let mut screen = String::new();
    for i in 0..4 {
        let lines: Vec<String> = (0..12).map(|j| format!("{i}.{j}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        screen.push_str(&command(&format!("c{i}"), &lines, 0));
    }
    screen.push_str("\x1b]133;A\x07$ ");
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);

    // 向上翻看时启动全屏程序：显示的是它的备用屏，不留在主屏的回滚缓冲中。
    app.terminal.tab_mut().unwrap().scroll_to(3, 0);
    assert!(tab(&app).scroll > 0);
    term.feed(b"\x1b[?1049h\x1b[Hvim");
    let frame = term.take_frame().unwrap();
    app.terminal.tab_mut().unwrap().apply(frame);
    assert_eq!(tab(&app).scroll, 0);

    // F8 的跳转是这个标签的：换到别的标签就结束，那边的 ↩ 照常给 shell。
    term.feed(b"\x1b[?1049l");
    term.feed(command("cargo build", &["error: x", "  --> short.rs:3:1"], 101).as_bytes());
    term.feed(b"\x1b]133;A\x07$ ");
    let frame = term.take_frame().unwrap();
    app.terminal.tab_mut().unwrap().apply(frame);
    sync_blocks(&mut app, &mut term);
    app.step_terminal_error(true);
    assert!(app.terminal.errors.is_some());
    let _other = add_tab(&mut app, "zsh", "$ ");
    app.activate_terminal_tab(0);
    assert!(app.terminal.errors.is_some(), "回到同一个标签不结束");
    app.activate_terminal_tab(1);
    assert!(app.terminal.errors.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_single_line_of_output_is_not_folded() {
    let dir = sample_folder("terminal-fold-one");
    let mut app = app_with(&dir.join("short.rs"), false);
    let screen = command("ls", &["only line"], 0);
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);

    app.click_terminal_band(0);
    app.fold_terminal_output(true);
    assert!(tab(&app).folded.is_empty());

    // 按住 ⇧ 选中输出不受影响。
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;
    app.click_terminal_band(0);
    assert!(tab(&app).selection.is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_already_folded_block_can_still_be_unfolded() {
    let dir = sample_folder("terminal-unfold-one");
    let mut app = app_with(&dir.join("short.rs"), false);
    let screen = command("ls", &["only line"], 0);
    let mut term = show_terminal(&mut app, &screen);
    sync_blocks(&mut app, &mut term);
    let id = tab(&app).blocks[0].id;
    app.terminal.tab_mut().unwrap().set_folded(id, true);

    app.click_terminal_band(0);
    assert!(tab(&app).folded.is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
