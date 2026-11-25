//! 竖向终端标签的布局、滚动与拖动，及终端内容区域的隔离。

use super::*;
use crate::app::scroll::target::ScrollTarget;
use crate::app::terminal::HeaderHit;

fn tab_hit(app: &App, index: usize) -> Rect {
    app.hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::Tab(index))
        .unwrap()
        .0
}

/// 固定标签列表的展开、收起进度；收起的时间包含内容先淡出的阶段。
fn freeze_tab_position(app: &mut App, right: bool, at: std::time::Duration) {
    use crate::app::animation::{Clock, MODULE_FADE};
    let position = &mut app.terminal.strip.position;
    position.track(!right, None);
    position.track(right, Some(Clock::frozen(at, MODULE_FADE)));
}

#[test]
fn vertical_tabs_share_the_panel_without_covering_terminal_content() {
    let dir = sample_folder("terminal-vertical-layout");
    for right in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), false);
        app.config.terminal_right = Some(right);
        app.config.terminal_vertical_tabs = Some(true);
        show_terminal(&mut app, "$ ");
        paint(&mut app);
        let full = app.terminal_size();
        let first = tab_hit(&app, 0);
        assert!(first.y >= app.terminal_view().y);
        let new = app
            .hits
            .terminal
            .iter()
            .find(|(_, hit)| *hit == HeaderHit::New)
            .unwrap()
            .0;

        add_tab(&mut app, "python", "$ ");
        paint(&mut app);
        let (first, second) = (tab_hit(&app, 0), tab_hit(&app, 1));
        assert_eq!(first.x, second.x);
        assert_eq!(second.y - first.bottom(), 2.0);
        assert!(first.y >= app.terminal_view().y);
        assert!(app.terminal_grid().right() <= first.x);
        assert_eq!(app.terminal_size().cols, full.cols);
        assert_eq!(app.terminal_size().rows, full.rows);
        assert_eq!(
            app.hits
                .terminal
                .iter()
                .find(|(_, hit)| *hit == HeaderHit::New)
                .unwrap()
                .0,
            new
        );

        app.open_terminal_find();
        paint(&mut app);
        let find = app.terminal_find_bar().unwrap();
        assert!(find.right() <= first.x);
        let blank = (first.x + 12.0, second.bottom() + 20.0);
        app.focus = Focus::Editor;
        click(&mut app, blank);
        assert_eq!(app.focus, Focus::Editor);

        app.close_terminal_tab(1);
        paint(&mut app);
        assert_eq!(app.terminal_size(), full);
        assert!(tab_hit(&app, 0).y >= app.terminal_view().y);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tab_position_changes_use_motion_only_when_animations_are_enabled() {
    use std::time::Duration;

    let dir = sample_folder("terminal-position-motion");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    paint(&mut app);
    app.terminal.strip.frozen = Some(Duration::ZERO);
    app.config.terminal_vertical_tabs = Some(true);
    paint(&mut app);
    assert!(app.terminal.strip.animating());
    let size = app.terminal_size();
    app.config.ui_animations = Some(false);
    paint(&mut app);
    assert!(!app.terminal.strip.animating());
    assert!(app.terminal_size().cols < size.cols);
    app.config.terminal_vertical_tabs = Some(false);
    paint(&mut app);
    assert!(!app.terminal.strip.animating());
    app.config.ui_animations = Some(true);
    app.config.terminal_vertical_tabs = Some(true);
    paint(&mut app);
    assert!(app.terminal.strip.animating());
    let mut startup = app_with(&dir.join("short.rs"), false);
    startup.config.terminal_vertical_tabs = Some(true);
    startup.terminal.strip.frozen = Some(Duration::ZERO);
    show_terminal(&mut startup, "$ ");
    paint(&mut startup);
    assert!(!startup.terminal.strip.animating());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tab_position_motion_moves_layout_continuously_and_reverses_without_jumping() {
    use crate::app::animation::{EXIT_FADE, MODULE_FADE};
    use crate::app::terminal::run::RunButton;
    use std::time::Duration;

    let dir = sample_folder("terminal-position-reverse");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "python", "$ ");
    paint(&mut app);
    let size = app.terminal_size();
    let run_x = |app: &App| {
        app.hits
            .terminal
            .iter()
            .find(|(_, hit)| *hit == HeaderHit::Run(RunButton::Start))
            .unwrap()
            .0
            .x
    };
    let right = run_x(&app);
    app.terminal.strip.frozen = Some(Duration::ZERO);
    app.config.terminal_vertical_tabs = Some(true);
    freeze_tab_position(&mut app, true, MODULE_FADE / 3);
    paint(&mut app);
    let list = app.terminal_tab_list().unwrap();
    assert!(list.x < app.layout.terminal.right());
    assert!(list.x > app.layout.terminal.right() - list.w);
    assert!(run_x(&app) < right && run_x(&app) > app.layout.terminal.x + 16.0);
    assert!(app.terminal_size().cols < size.cols);
    assert!(
        app.hits
            .terminal
            .iter()
            .filter(|(rect, _)| rect.w > 0.0)
            .all(|(rect, _)| rect.right() <= app.layout.terminal.right())
    );
    let middle_size = app.terminal_size();
    let middle_run = run_x(&app);
    app.config.terminal_vertical_tabs = Some(false);
    paint(&mut app);
    assert_eq!(app.terminal_tab_list().unwrap().x, list.x);
    assert_eq!(app.terminal_size(), middle_size);
    assert_eq!(run_x(&app), middle_run);
    freeze_tab_position(&mut app, false, EXIT_FADE + MODULE_FADE);
    paint(&mut app);
    assert!(app.terminal_tab_list().is_none());
    assert!(!app.terminal.strip.animating());
    assert_eq!(app.terminal_size(), size);
    assert_eq!(run_x(&app), right);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn vertical_tabs_scroll_separately_and_reveal_the_active_terminal() {
    let dir = sample_folder("terminal-vertical-scroll");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    for _ in 0..20 {
        add_tab(&mut app, "zsh", "$ ");
    }
    paint(&mut app);
    let last = tab_hit(&app, 20);
    assert!(last.h > 0.0);
    assert_eq!(tab_hit(&app, 0).h, 0.0);
    let scroll = app.terminal.tab().unwrap().scroll;
    let at = (last.x + 12.0, last.y + last.h / 2.0);
    assert_eq!(
        app.scroll_target_at(at.0, at.1),
        ScrollTarget::TerminalStrip
    );
    app.scroll_target_by(ScrollTarget::TerminalStrip, at, false, (0.0, 1000.0));
    paint(&mut app);
    assert!(tab_hit(&app, 0).h > 0.0);
    assert_eq!(app.terminal.active, 20);
    assert_eq!(app.terminal.tab().unwrap().scroll, scroll);

    app.activate_terminal_tab(0);
    paint(&mut app);
    app.activate_terminal_tab(20);
    paint(&mut app);
    assert!(tab_hit(&app, 20).h > 0.0);
    assert_eq!(tab_hit(&app, 0).h, 0.0);
    app.terminal.height /= 2.0;
    paint(&mut app);
    let last = tab_hit(&app, 20);
    assert_eq!(last.h, crate::theme::TREE_ROW);
    assert!(last.bottom() <= app.layout.terminal.bottom());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn vertical_tabs_reorder_vertically_and_keep_rename_and_close() {
    let dir = sample_folder("terminal-vertical-drag");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "python", "$ ");
    add_tab(&mut app, "cargo", "$ ");
    paint(&mut app);
    let ids: Vec<_> = app.terminal.tabs.iter().map(|tab| tab.id).collect();
    let first = tab_hit(&app, 0);
    let at = (first.x + 16.0, first.y + first.h / 2.0);
    app.on_mouse_move(at.0, at.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(at.0 + 20.0, at.1);
    assert_eq!(app.terminal.active, 0);
    app.on_mouse_move(at.0, at.1 + first.h);
    paint(&mut app);
    assert_eq!(
        app.terminal
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [ids[1], ids[0], ids[2]]
    );
    assert_eq!(app.terminal.active, 1);
    app.on_mouse_up(MouseButton::Left);
    app.open_terminal_rename(1);
    paint(&mut app);
    let caret = app.ime.cursor.unwrap();
    let renamed = tab_hit(&app, 1);
    assert!(caret.x >= renamed.x && caret.right() <= renamed.right());
    app.edit_field(|field| field.insert("server"));
    app.finish_terminal_rename(true);
    assert_eq!(app.terminal.tabs[1].name, "server");
    app.close_terminal_tab(1);
    paint(&mut app);
    assert_eq!(app.terminal.tabs.len(), 2);
    assert!(app.dragged_terminal_tab().is_none());
    let last = tab_hit(&app, 1);
    app.on_mouse_move(last.x + 16.0, last.y + last.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(last.x + 16.0, last.bottom() + last.h);
    assert_eq!(app.dragged_terminal_tab().unwrap().1, last.y);
    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changing_tab_orientation_preserves_terminals_and_ends_the_old_drag() {
    let dir = sample_folder("terminal-tabs-orientation");
    let mut app = app_with(&dir.join("short.rs"), false);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "python", "$ ");
    paint(&mut app);
    let size = app.terminal_size();
    let ids: Vec<_> = app.terminal.tabs.iter().map(|tab| tab.id).collect();
    app.set_terminal_tab_position(true);
    paint(&mut app);
    assert!(app.terminal_size().cols < size.cols);
    let tab = tab_hit(&app, 0);
    app.on_mouse_move(tab.x + 16.0, tab.y + tab.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(tab.x + 16.0, tab.y + tab.h / 2.0 + 6.0);
    assert!(app.dragged_terminal_tab().is_some());

    let mut config = app.config.clone();
    config.terminal_vertical_tabs = Some(false);
    app.adopt_config(config);
    paint(&mut app);
    assert!(app.dragged_terminal_tab().is_none());
    assert_eq!(app.terminal_size(), size);
    assert_eq!(
        app.terminal
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(tab_hit(&app, 0).y, tab_hit(&app, 1).y);
    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn list_close_actions_appear_on_hover_and_keep_the_new_entry_fixed() {
    let dir = sample_folder("terminal-list-close");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    add_tab(&mut app, "python", "$ ");
    paint(&mut app);
    let new = app
        .hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::New)
        .unwrap()
        .0;
    assert!(
        app.hits
            .terminal
            .iter()
            .all(|(_, hit)| !matches!(hit, HeaderHit::Close(_)))
    );
    let row = tab_hit(&app, 0);
    app.on_mouse_move(row.x + 16.0, row.y + row.h / 2.0);
    paint(&mut app);
    let close = app
        .hits
        .terminal
        .iter()
        .find(|(_, hit)| *hit == HeaderHit::Close(0))
        .unwrap()
        .0;
    click(&mut app, (close.x + close.w / 2.0, close.y + close.h / 2.0));
    paint(&mut app);
    assert_eq!(app.terminal.tabs.len(), 1);
    assert_eq!(app.terminal.tabs[0].label(), "python");
    assert_eq!(
        app.hits
            .terminal
            .iter()
            .find(|(_, hit)| *hit == HeaderHit::New)
            .unwrap()
            .0,
        new
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_new_terminal_and_shell_menu_share_a_control_with_separate_actions() {
    use crate::app::MenuButton;
    use crate::app::terminal::shells::ShellChoice;

    let dir = sample_folder("terminal-list-new-menu");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "(.venv) project ❯ ");
    app.terminal.shells = ["zsh", "bash"]
        .into_iter()
        .map(|name| ShellChoice {
            name: name.into(),
            command: name.into(),
        })
        .collect();
    paint(&mut app);
    let hit = |app: &App, target| {
        app.hits
            .terminal
            .iter()
            .find(|(_, hit)| *hit == target)
            .unwrap()
            .0
    };
    let new = hit(&app, HeaderHit::New);
    let menu = hit(&app, HeaderHit::ShellMenu);
    assert_eq!(new.right(), menu.x);
    assert_eq!((new.y, new.h), (menu.y, menu.h));
    let row = tab_hit(&app, 0);
    assert_eq!((new.x, menu.right()), (row.x, row.right()));
    let pixel = |app: &App, x: f32, y: f32| {
        let renderer = app.frame.renderer.as_ref().unwrap();
        let scale = renderer.scale_factor();
        let image = renderer.image().unwrap();
        image.pixels[(y * scale) as usize * image.width as usize + (x * scale) as usize]
    };
    let sample = (new.right() - 6.0, new.y + new.h / 2.0);
    let idle = pixel(&app, sample.0, sample.1);
    assert_eq!(idle, pixel(&app, sample.0, row.bottom() + 20.0));
    app.on_mouse_move(new.x + 12.0, sample.1);
    paint(&mut app);
    let hovered = pixel(&app, sample.0, sample.1);
    assert_ne!(idle, hovered);
    app.on_mouse_move(menu.x + menu.w / 2.0, sample.1);
    paint(&mut app);
    assert_eq!(pixel(&app, sample.0, sample.1), hovered);
    let waiting = app.terminal.want.len();
    click(&mut app, (menu.x + menu.w / 2.0, menu.y + menu.h / 2.0));
    assert_eq!(app.overlays.menu_button, Some(MenuButton::TerminalShells));
    assert_eq!(app.terminal.want.len(), waiting);
    paint(&mut app);
    assert_eq!(pixel(&app, sample.0, sample.1), hovered);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_offscreen_rename_does_not_intercept_visible_terminal_tabs() {
    let dir = sample_folder("terminal-hidden-rename");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    for _ in 0..20 {
        add_tab(&mut app, "zsh", "$ ");
    }
    app.open_terminal_rename(0);
    paint(&mut app);
    let field = app.terminal.rename.as_ref().unwrap().1.view.area;
    let at = (field.x + 10.0, field.y + field.h / 2.0);
    app.scroll_target_by(ScrollTarget::TerminalStrip, at, false, (0.0, -1000.0));
    paint(&mut app);
    assert_eq!(tab_hit(&app, 0).h, 0.0);
    assert_ne!(app.field_at(at.0, at.1), Some(Focus::TerminalRename));
    let expected = app
        .hits
        .terminal
        .iter()
        .find_map(|(rect, hit)| match hit {
            HeaderHit::Tab(index) if rect.contains(at.0, at.1) => Some(*index),
            _ => None,
        })
        .unwrap();
    click(&mut app, at);
    assert_eq!(app.terminal.active, expected);
    assert!(app.terminal.rename.is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_partially_visible_rename_is_clipped_and_does_not_cover_the_new_entry() {
    let dir = sample_folder("terminal-partial-rename");
    for (dark, right) in [(false, false), (true, true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_right = Some(right);
        app.config.terminal_vertical_tabs = Some(true);
        show_terminal(&mut app, "$ ");
        for _ in 0..40 {
            add_tab(&mut app, "zsh", "$ ");
        }
        app.activate_terminal_tab(0);
        paint(&mut app);
        let rows = app.terminal_tab_rows().unwrap();
        let at = (rows.x + 30.0, rows.y + 10.0);
        // 保留行底部 5 个逻辑像素，输入框主体位于固定的新建入口中。
        let delta = -(crate::theme::TREE_ROW - 5.0);
        app.scroll_target_by(ScrollTarget::TerminalStrip, at, false, (0.0, delta));
        paint(&mut app);
        assert_eq!(tab_hit(&app, 0).h, 5.0);
        let pixels = |app: &App| {
            let renderer = app.frame.renderer.as_ref().unwrap();
            let scale = renderer.scale_factor();
            let image = renderer.image().unwrap();
            let mut pixels = Vec::new();
            for y in ((rows.y - crate::theme::TREE_ROW) * scale) as usize..(rows.y * scale) as usize
            {
                let start = y * image.width as usize + (rows.x * scale) as usize;
                let end = y * image.width as usize + (rows.right() * scale) as usize;
                pixels.extend_from_slice(&image.pixels[start..end]);
            }
            pixels
        };
        let before = pixels(&app);
        app.open_terminal_rename(0);
        paint(&mut app);
        assert!(pixels(&app) == before, "重命名输入框不应覆盖新建入口");
        let field = app.terminal.rename.as_ref().unwrap().1.view.area;
        let hidden = (field.x + 10.0, rows.y - 10.0);
        assert_ne!(
            app.field_at(hidden.0, hidden.1),
            Some(Focus::TerminalRename)
        );
        let visible = tab_hit(&app, 0);
        let at = (field.x + 10.0, visible.y + 1.0);
        assert_eq!(app.field_at(at.0, at.1), Some(Focus::TerminalRename));
        let caret = app.ime.cursor.unwrap();
        assert!(caret.y >= rows.y && caret.bottom() <= visible.bottom());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_faded_rename_does_not_intercept_the_collapsing_list() {
    use crate::app::animation::EXIT_FADE;

    let dir = sample_folder("terminal-faded-rename");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    app.open_terminal_rename(0);
    paint(&mut app);
    let field = app.terminal.rename.as_ref().unwrap().1.view.area;
    let at = (field.x + 10.0, field.y + field.h / 2.0);
    assert_eq!(app.field_at(at.0, at.1), Some(Focus::TerminalRename));
    app.config.terminal_vertical_tabs = Some(false);
    freeze_tab_position(&mut app, false, EXIT_FADE);
    paint(&mut app);
    assert!(app.terminal_tab_list().is_some());
    assert_ne!(app.field_at(at.0, at.1), Some(Focus::TerminalRename));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 绘制竖向终端列表，覆盖浅色、深色主题及新建入口的悬停和菜单状态。
///
/// ```text
/// cargo test -p winc-app snapshot_terminal_vertical_tabs -- --ignored
/// ```
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_vertical_tabs() {
    use crate::app::terminal::shells::ShellChoice;
    let dir = sample_folder("terminal-vertical-shot");
    std::fs::create_dir_all(dir.join(".winc")).unwrap();
    std::fs::write(
        dir.join(".winc/run.json"),
        r#"{"configurations": [{"name": "test", "command": "cargo test"}]}"#,
    )
    .unwrap();
    for dark in [false, true] {
        for right in [false, true] {
            let mut app = app_with(&dir.join("short.rs"), dark);
            app.config.terminal_enabled = Some(true);
            app.config.terminal_right = Some(right);
            app.config.terminal_vertical_tabs = Some(true);
            app.reload_run_configs();
            assert_eq!(app.run.current().unwrap().name, "test");
            show_terminal(
                &mut app,
                "(.venv) project ❯ cargo test\r\n   Compiling winc-app\r\n",
            );
            app.terminal.shells = ["zsh", "bash"]
                .into_iter()
                .map(|name| ShellChoice {
                    name: name.into(),
                    command: format!("/bin/{name}"),
                })
                .collect();
            for name in ["python", "cargo watch", "ssh server"] {
                add_tab(&mut app, name, "$ ");
            }
            app.activate_terminal_tab(0);
            let theme = if dark { "dark" } else { "light" };
            let position = if right { "right" } else { "bottom" };
            shoot(&mut app, &format!("terminal-vertical-{position}-{theme}"));
            let new = app
                .hits
                .terminal
                .iter()
                .find(|(_, hit)| *hit == HeaderHit::New)
                .unwrap()
                .0;
            app.on_mouse_move(new.x + new.w / 2.0, new.y + new.h / 2.0);
            shoot(
                &mut app,
                &format!("terminal-vertical-{position}-{theme}-hover"),
            );
            let menu = app
                .hits
                .terminal
                .iter()
                .find(|(_, hit)| *hit == HeaderHit::ShellMenu)
                .unwrap()
                .0;
            click(&mut app, (menu.x + menu.w / 2.0, menu.y + menu.h / 2.0));
            shoot(
                &mut app,
                &format!("terminal-vertical-{position}-{theme}-menu"),
            );
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 绘制标签位置切换的开始、中间及结束状态。
///
/// ```text
/// cargo test -p winc-app snapshot_terminal_tab_position_motion -- --ignored
/// ```
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_terminal_tab_position_motion() {
    use crate::app::animation::{EXIT_FADE, MODULE_FADE};
    use std::time::Duration;

    let dir = sample_folder("terminal-position-motion-shot");
    for (dark, right) in [(false, false), (true, true)] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_right = Some(right);
        show_terminal(
            &mut app,
            "(.venv) project ❯ cargo test\r\n   Compiling winc-app\r\n",
        );
        add_tab(&mut app, "python", "$ ");
        add_tab(&mut app, "cargo watch", "$ ");
        app.activate_terminal_tab(0);
        paint(&mut app);
        let placement = if right { "right-dark" } else { "bottom-light" };
        for (on, at, phase) in [
            (true, Duration::ZERO, "in-start"),
            (true, MODULE_FADE / 3, "in-middle"),
            (true, MODULE_FADE, "in-end"),
            (false, EXIT_FADE / 2, "out-start"),
            (false, EXIT_FADE + MODULE_FADE / 2, "out-middle"),
            (false, EXIT_FADE + MODULE_FADE, "out-end"),
        ] {
            app.config.terminal_vertical_tabs = Some(on);
            freeze_tab_position(&mut app, on, at);
            shoot(&mut app, &format!("terminal-position-{placement}-{phase}"));
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn find_and_error_state_follow_the_tab_they_belong_to() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "error one");
    add_tab(&mut app, "zsh", "two");
    app.activate_terminal_tab(0);
    app.focus = Focus::Terminal;
    app.open_terminal_find();
    assert!(app.terminal.find.is_some());

    // 点正在看的标签不关查找条；换到另一个标签时查找条不跟过去。
    app.activate_terminal_tab(0);
    assert!(app.terminal.find.is_some());
    assert_eq!(app.focus, Focus::TerminalFind);
    app.activate_terminal_tab(1);
    assert!(app.terminal.find.is_none());
    assert_eq!(app.focus, Focus::Terminal);
}

#[test]
fn a_new_terminal_after_the_helper_exits_is_not_shown_as_exited() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    add_tab(&mut app, "zsh", "two");
    app.terminal_helper_gone();
    assert!(app.terminal.crashed);

    // 新建终端时先恢复原有标签，新标签排在后面，按键不再被当作已退出的标签处理。
    app.attach_idle_helper();
    assert!(app.run_action(crate::keymap::Action::NewTerminal));
    assert!(!app.terminal.crashed);
    assert_eq!(app.terminal.tabs.len() + app.terminal.want.len(), 3);
    assert!(app.terminal.tabs.iter().all(|tab| tab.ended.is_none()));
}

#[test]
fn confirming_an_unchanged_rename_keeps_the_automatic_name() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "$ ");
    app.terminal.tabs[0].title = "~/winc".into();
    app.open_terminal_rename(0);
    app.finish_terminal_rename(true);
    assert!(app.terminal.tabs[0].name.is_empty());
    app.terminal.tabs[0].title = "~/other".into();
    assert_eq!(app.terminal.tabs[0].label(), "~/other");

    // 已有自定名字时输入相同的名字仍然保留。
    app.terminal.tabs[0].name = "web".into();
    app.open_terminal_rename(0);
    app.finish_terminal_rename(true);
    assert_eq!(app.terminal.tabs[0].name, "web");
}

#[test]
fn a_python_environment_follows_the_recorded_answer() {
    let dir = sample_folder("terminal-python-choice");
    let mut app = app_with(&dir.join("short.rs"), false);
    let dir = app.editor.root().expect("打开了目录").to_path_buf();
    app.terminal.want = vec![Vec::new()];
    // 目录里没有环境：没有可加载的，也不需要询问。
    assert_eq!(app.python_choices(), Some(vec![true]));

    let venv = dir.join(".venv");
    std::fs::create_dir_all(&venv).unwrap();
    std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
    // 测试中没有窗口，无法询问：按同意处理。
    assert_eq!(app.python_choices(), Some(vec![true]));

    // 拒绝只在本次运行中有效，不写入状态；同意长期记录，并覆盖此前的拒绝。
    app.answered_python_environment(venv.clone(), false);
    assert_eq!(app.python_choices(), Some(vec![false]));
    assert_eq!(app.state.trusted_repo(&venv), None);
    app.terminal.python_declined.clear();
    assert_eq!(app.python_choices(), Some(vec![true]), "下次启动时重新询问");
    app.answered_python_environment(venv.clone(), false);
    app.answered_python_environment(venv.clone(), true);
    assert_eq!(app.python_choices(), Some(vec![true]));
    assert_eq!(app.state.trusted_repo(&venv), Some(1));

    // 正在询问时等着建的标签不建。
    app.terminal.python_asking = Some(venv.clone());
    assert_eq!(app.python_choices(), None);
    app.terminal.python_asking = None;

    // 关掉 shell 集成时不加载。
    app.config.terminal_shell_integration = Some(false);
    assert_eq!(app.python_choices(), Some(vec![false]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shell_that_exits_cleanly_closes_its_tab() {
    let mut app = blank_app(false);
    show_terminal(&mut app, "one");
    add_tab(&mut app, "zsh", "two");
    let (first, second) = (app.terminal.tabs[0].id, app.terminal.tabs[1].id);

    // 异常退出：标签留着，用户要看退出码和输出。
    app.terminal_message(FromHelper::Exited {
        tab: second,
        code: 1,
    });
    assert_eq!(app.terminal.tabs.len(), 2);
    assert!(app.terminal.tabs[1].ended.is_some());

    // 正常退出：标签直接关闭；最后一个关闭时面板收起。
    app.terminal_message(FromHelper::Exited {
        tab: first,
        code: 0,
    });
    assert_eq!(app.terminal.tabs.len(), 1);
    assert_eq!(app.terminal.tabs[0].id, second);
}

#[test]
fn scrolling_to_the_ends_and_closing_the_other_tabs() {
    let mut app = blank_app(false);
    let mut lines = String::new();
    for i in 0..200 {
        lines.push_str(&format!("line {i}\r\n"));
    }
    show_terminal(&mut app, &lines);
    assert!(app.run_action(crate::keymap::Action::ScrollTerminalTop));
    let tab = app.terminal.tab().unwrap();
    assert!(tab.scroll > 0 && tab.scroll == tab.max_scroll());
    assert!(app.run_action(crate::keymap::Action::ScrollTerminalBottom));
    assert_eq!(app.terminal.tab().unwrap().scroll, 0);

    add_tab(&mut app, "zsh", "two");
    add_tab(&mut app, "zsh", "three");
    let keep = app.terminal.tabs[1].id;
    app.close_other_terminal_tabs(keep);
    assert_eq!(app.terminal.tabs.len(), 1);
    assert_eq!(app.terminal.tabs[0].id, keep);
}

#[test]
fn the_palette_offers_the_terminal_module_while_it_is_off() {
    use crate::keymap::Action;
    let mut app = blank_app(false);
    // 模块关着：内置终端的命令不列出，只有启用它的入口。
    app.config.terminal_enabled = Some(false);
    assert!(app.action_available(Action::EnableTerminal));
    assert!(!app.action_available(Action::NewTerminal));
    assert!(app.run_action(Action::EnableTerminal));
    assert_eq!(app.config.terminal_enabled, Some(true));
    // 启用之后入口消失，内置终端的命令列出。
    assert!(!app.action_available(Action::EnableTerminal));
    assert!(app.action_available(Action::NewTerminal));
}

/// 拆分过的终端标签：各终端并排，点哪个哪个到前台，关掉前台的由相邻的补上，现场里各记成一个标签。
#[test]
fn a_split_terminal_tab_shows_its_terminals_side_by_side() {
    let dir = sample_folder("terminal-split");
    let mut app = app_with(&dir.join("short.rs"), false);
    let _term = show_terminal(&mut app, "left\r\n");
    let whole = app.terminal_content_card();
    let first = app.terminal.tab().unwrap().id;

    // 标签和终端里的右键菜单都能拆分：新终端排进这个标签。
    for target in [MenuTarget::TerminalTab(first), MenuTarget::Terminal(None)] {
        app.open_context_menu(target, (whole.x + 20.0, whole.y + 20.0));
        assert!(menu_commands(&app).contains(&(MenuCommand::SplitTerminal, true)));
        app.overlays.context_menu = None;
    }
    app.open_context_menu(MenuTarget::TerminalTab(first), (whole.x, whole.y));
    let wanted = app.terminal.want.len();
    app.run_menu_command(MenuCommand::SplitTerminal);
    // 再拆一次：两个请求各记着要加进的标签。
    app.split_terminal();
    assert_eq!(app.terminal.want.len(), wanted + 2);
    assert_eq!(app.terminal.want_into[wanted..], [Some(first), Some(first)]);
    // 测试里没有辅助进程建它们：撤掉这两个请求，下面自己放一个终端。
    app.terminal.want.truncate(wanted);
    app.terminal.want_shells.truncate(wanted);
    app.terminal.want_into.truncate(wanted);

    let size = app.terminal_size();
    let (mut all, at) = app.terminal.take_group(0);
    all.push(TermTab::new(99, size));
    app.terminal.put_group(0, all, at + 1);
    app.relayout();
    paint(&mut app);
    assert_eq!(app.terminal.tabs.len(), 1);
    let card = app.terminal_content_card();
    assert!(
        card.x > whole.x && card.w <= whole.w / 2.0 + 1.0,
        "{card:?} {whole:?}"
    );
    assert_eq!(app.terminal_pane_dividers().len(), 1);
    // 各终端按自己的宽度：前台那个的列数是整块的一半左右。
    assert!(app.terminal_size().cols <= size.cols / 2 + 1);

    click(&mut app, (whole.x + 30.0, card.bottom() - 30.0));
    assert_eq!(app.terminal.tab().unwrap().id, first);
    app.run_action(crate::keymap::Action::NextTerminalPane);
    assert_eq!(app.terminal.tab().unwrap().id, 99);

    app.save_session();
    let root = app.editor.root().unwrap().to_path_buf();
    let session = app.state.session_for(&root).cloned().unwrap();
    assert_eq!(session.terminal.unwrap().tabs.len(), 2);

    // ⌘W 只关前台的那个终端，标签留着。
    app.run_action(crate::keymap::Action::CloseTerminal);
    let tab = app.terminal.tab().unwrap();
    assert_eq!((tab.id, tab.pane_count()), (first, 1));

    // 标签上的 × 关掉标签里的全部终端。
    let (mut all, at) = app.terminal.take_group(0);
    all.push(TermTab::new(98, size));
    app.terminal.put_group(0, all, at);
    app.close_terminal_tab(0);
    assert!(app.terminal.tabs.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 竖向列表里拆分的标签展开成一组：一行写着有几个终端，下面按排列顺序每个终端一行。点一行把它
/// 换到前台；那一行的关闭按钮和中键只关掉这一个终端，标签和其余终端留着。
#[test]
fn split_tabs_list_their_terminals_in_the_vertical_list() {
    use crate::app::terminal::HeaderHit;

    let dir = sample_folder("terminal-split-list");
    let mut app = app_with(&dir.join("short.rs"), false);
    app.config.terminal_vertical_tabs = Some(true);
    show_terminal(&mut app, "$ ");
    let size = app.terminal_size();
    add_tab(&mut app, "python", "$ ");
    let (mut all, _) = crate::app::terminal::take_group(&mut app.terminal.tabs, 0);
    all.push(TermTab::new(98, size));
    all.push(TermTab::new(99, size));
    app.terminal.put_group(0, all, 0);
    app.activate_terminal_tab(0);
    paint(&mut app);

    let hit = |app: &App, want: HeaderHit| {
        (app.hits.terminal.iter())
            .find(|(_, h)| *h == want)
            .map(|(r, _)| *r)
    };
    let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    // 组名一行在上，三个终端依次在下面，同一列、同样高。
    let head = hit(&app, HeaderHit::Tab(0)).unwrap();
    let rows: Vec<Rect> = (0..3)
        .map(|at| hit(&app, HeaderHit::Pane(0, at)).unwrap())
        .collect();
    assert!(rows[0].y >= head.bottom() - 0.5);
    assert!(
        rows.windows(2)
            .all(|w| (w[1].y - w[0].bottom()).abs() < 0.5)
    );
    // python 标签排在整组下面。
    assert!(hit(&app, HeaderHit::Tab(1)).unwrap().y > rows[2].bottom());

    // 点第三行：它换到前台。
    click(&mut app, center(rows[2]));
    assert_eq!(app.terminal.tab().unwrap().pane_at(), 2);
    assert_eq!(app.terminal.tab().unwrap().id, 99);
    assert_eq!(app.focus, Focus::Terminal);

    // 悬停第二行出现关闭按钮，点它只关掉这个终端。
    paint(&mut app);
    let second = hit(&app, HeaderHit::Pane(0, 1)).unwrap();
    app.on_mouse_move(second.x + 30.0, second.y + second.h / 2.0);
    paint(&mut app);
    let close = hit(&app, HeaderHit::ClosePane(0, 1)).expect("悬停时有关闭按钮");
    click(&mut app, center(close));
    let tab = app.terminal.tab().unwrap();
    assert_eq!(tab.pane_count(), 2);
    assert!(tab.panes().all(|pane| pane.id != 98));
    assert_eq!(app.terminal.tabs.len(), 2);

    // 中键点一行：同样只关掉它；剩一个终端时标签回到普通的一行。
    paint(&mut app);
    let first = hit(&app, HeaderHit::Pane(0, 0)).unwrap();
    app.on_mouse_move(first.x + 30.0, first.y + first.h / 2.0);
    app.on_mouse_down(winit::event::MouseButton::Middle);
    app.on_mouse_up(winit::event::MouseButton::Middle);
    paint(&mut app);
    assert_eq!(app.terminal.tab().unwrap().pane_count(), 1);
    assert!(hit(&app, HeaderHit::Pane(0, 0)).is_none());
    assert_eq!(app.terminal.tabs.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拆分的标签在竖向列表里的样子：浅色、深色，鼠标停在一个终端上。
///
/// 运行：`cargo test -p winc-app shoot_terminal_split_list -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_terminal_split_list() {
    use crate::app::terminal::HeaderHit;

    let dir = sample_folder("terminal-split-list-shot");
    for dark in [false, true] {
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.terminal_vertical_tabs = Some(true);
        show_terminal(
            &mut app,
            "~/project ❯ cargo test\r\n   Compiling winc-app\r\n",
        );
        let size = app.terminal_size();
        add_tab(&mut app, "python", "$ ");
        add_tab(&mut app, "ssh server", "$ ");
        let (mut all, _) = crate::app::terminal::take_group(&mut app.terminal.tabs, 0);
        all.push(TermTab::new(98, size));
        all.push(TermTab::new(99, size));
        app.terminal.put_group(0, all, 1);
        app.activate_terminal_tab(0);
        paint(&mut app);
        let row = (app.hits.terminal.iter())
            .find(|(_, h)| *h == HeaderHit::Pane(0, 2))
            .unwrap()
            .0;
        app.on_mouse_move(row.x + 30.0, row.y + row.h / 2.0);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("terminal-split-list-{theme}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
