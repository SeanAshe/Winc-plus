//! 单进程多窗口（`crate::winc`）的测试：关闭单个窗口、⌘Q 退出全部、切换到开着目录的窗口、
//! 窗口之间同步设置与被剪切的文件。
//!
//! 测试没有事件循环，每步之后手动执行一次应用层收尾
//! （`settle_for_test`）。

use super::*;
use crate::winc::{Request, Winc};

/// 两个窗口：第一个开着 `name` 目录里的 `src/main.rs`，第二个在开始页。
fn two_windows(
    name: &str,
) -> (
    Winc,
    crate::winc::WindowKey,
    crate::winc::WindowKey,
    PathBuf,
) {
    let dir = search_folder(name);
    let mut winc = Winc::new(None, None);
    let a = winc.add_detached(app_with(&dir.join("src/main.rs"), false));
    let b = winc.add_detached(blank_app(false));
    (winc, a, b, dir)
}

#[test]
fn closing_one_of_two_windows_drops_it_and_the_last_one_stays_hidden() {
    let (mut winc, a, b, dir) = two_windows("windows-close");
    winc.app(b).request_close_window();
    assert!(winc.app(b).window.hidden);
    winc.settle_for_test();
    assert_eq!(winc.keys(), [a], "还有别的窗口时关掉的窗口去掉");

    // 最后一个窗口：macOS 上关闭时保留（点击程序坞图标会带着开始页回来），不退出；其他系统上
    // 关闭最后一个窗口就是退出。
    winc.app(a).request_close_window();
    let (_, exit) = winc.settle_for_test();
    assert_eq!(winc.keys(), [a]);
    if cfg!(target_os = "macos") {
        assert!(winc.app(a).window.hidden && !exit);
    } else {
        assert!(winc.app(a).closing.quit_requested);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_a_window_without_a_folder_asks_about_unsaved_files() {
    let (mut winc, a, b, dir) = two_windows("windows-close-untitled");
    let root = super::workspace::session::backup_root("windows-close-untitled");
    for key in [a, b] {
        winc.app(key).backup.root = Some(root.clone());
    }
    // 没开目录的窗口中有未保存的笔记：备份没有之后恢复的时机，关闭前必须询问。
    winc.app(b).new_file();
    winc.app(b).run(Command::Insert("笔记".into()));
    winc.app(b).request_close_window();
    assert!(winc.app(b).closing.prompt.is_some() && !winc.app(b).window.hidden);
    winc.app(b).answer_save_prompt(SaveChoice::Cancel);
    winc.settle_for_test();
    assert_eq!(winc.keys(), [a, b], "取消了就留着");

    // 开着目录的窗口仍然不询问：内容写入备份，以后打开该目录时恢复。
    winc.app(a).run(Command::Insert("// 还没存\n".into()));
    winc.app(a).request_close_window();
    assert!(winc.app(a).closing.prompt.is_none() && winc.app(a).window.hidden);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn quitting_one_window_quits_all_of_them() {
    let (mut winc, a, b, dir) = two_windows("windows-quit");
    winc.set_active(a);
    winc.app(a).request_quit();
    let (_, exit) = winc.settle_for_test();
    assert!(winc.app(b).closing.quit_requested, "其他窗口也请它退出");
    assert!(exit, "都退了才结束事件循环");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_in_one_window_calls_the_whole_quit_off() {
    let (mut winc, a, b, dir) = two_windows("windows-quit-cancel");
    // 第二个窗口有不能热退出的修改：退出时要问。
    winc.app(b).config.hot_exit = Some(false);
    winc.app(b).new_file();
    winc.app(b).run(Command::Insert("x".into()));
    winc.set_active(b);

    winc.app(a).request_quit();
    let (_, exit) = winc.settle_for_test();
    assert!(
        !exit && winc.app(b).closing.prompt.is_some(),
        "第二个窗口在问"
    );
    assert!(winc.app(a).closing.quit_requested);

    winc.app(b).answer_save_prompt(SaveChoice::Cancel);
    let (_, exit) = winc.settle_for_test();
    assert!(!exit);
    assert!(
        !winc.app(a).closing.quit_requested,
        "取消了：已经同意退出的窗口照常开着"
    );
    assert!(!winc.app(b).closing.quit_requested);

    // 再退一次，这次不保存：都退了。
    winc.app(a).request_quit();
    winc.settle_for_test();
    winc.app(b).answer_save_prompt(SaveChoice::Discard);
    let (_, exit) = winc.settle_for_test();
    assert!(exit);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_a_folder_that_another_window_has_open_switches_to_that_window() {
    let (mut winc, a, b, dir) = two_windows("windows-focus");
    // 第一个窗口开着的是 `src`（打开文件时以它所在的目录为项目）。
    winc.app(b).switch_folder(&dir.join("src"));
    let (requests, _) = winc.settle_for_test();
    assert_eq!(requests, [Request::Focus(a)]);
    assert!(winc.app(b).editor.root().is_none(), "不在这里再开一份");

    // 提示框里选了切过去（移除开在另一个窗口里的工作树时）：同样只切过去；那个窗口已经关了时
    // 什么也不做。
    let src = winc_config::files::canonicalize(dir.join("src")).unwrap();
    assert!(winc.app(b).focus_window_with(&src));
    let (requests, _) = winc.settle_for_test();
    assert_eq!(requests, [Request::Focus(a)]);
    assert!(!winc.app(b).focus_window_with(&dir));
    let (requests, _) = winc.settle_for_test();
    assert!(requests.is_empty());

    // 新建窗口的请求交给应用层。
    winc.app(b).run_action(crate::keymap::Action::NewWindow);
    let (requests, _) = winc.settle_for_test();
    assert_eq!(requests, [Request::NewWindow(None)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_another_folder_follows_the_new_window_setting() {
    let (mut winc, a, _, dir) = two_windows("windows-open-in");
    winc.settle_for_test();
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let src = winc.app(a).editor.root().unwrap().to_path_buf();

    // 设成在新窗口打开：当前窗口的目录不变，请应用层新建窗口。
    winc.app(a).config.open_folder_in_new_window = Some(true);
    winc.app(a).open_path(&dir);
    let (requests, _) = winc.settle_for_test();
    assert_eq!(requests, [Request::NewWindow(Some(root.clone()))]);
    assert_eq!(winc.app(a).editor.root(), Some(src.as_path()));

    // 提示框上选了「此窗口」并勾选「不再询问」：在此窗口打开，记下这次的选择。
    winc.app(a).config.open_folder_in_new_window = None;
    winc.app(a).open_folder_in(&root, false, true);
    assert_eq!(winc.app(a).config.open_folder_in_new_window, Some(false));
    assert_eq!(winc.app(a).editor.root(), Some(root.as_path()));
    let (requests, _) = winc.settle_for_test();
    assert!(requests.is_empty());

    // 设成在此窗口打开：直接替换。
    winc.app(a).open_path(&src);
    assert_eq!(winc.app(a).editor.root(), Some(src.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropped_on_the_dock_icon_open_in_the_window_that_has_them_or_the_current_one() {
    let (mut winc, a, b, dir) = two_windows("windows-dock");
    winc.settle_for_test();
    let src = winc_config::files::canonicalize(dir.join("src")).unwrap();
    let target = |path: &Path| crate::cli::Target {
        path: path.to_path_buf(),
        line: None,
        col: None,
    };

    // 开着它所在目录的窗口收下。
    assert_eq!(winc.window_for_test(&[target(&src.join("main.rs"))]), Ok(a));

    // 项目外的文件：交给当前窗口，窗口打开的目录不变，也不打开文件所在的目录。
    let elsewhere = winc_config::files::canonicalize(&dir).unwrap();
    let file = elsewhere.join("a.txt");
    for (key, root) in [(a, Some(src.clone())), (b, None)] {
        winc.set_active(key);
        assert_eq!(winc.window_for_test(&[target(&file)]), Ok(key));
        winc.app(key).open_files(&[target(&file)]);
        assert_eq!(winc.app(key).editor.root(), root.as_deref());
    }
    winc.settle_for_test();

    // 目录不交给已有标签的窗口：没有空着的窗口时新建窗口打开。
    let other = search_folder("windows-dock-other");
    let other = winc_config::files::canonicalize(&other).unwrap();
    assert_eq!(
        winc.window_for_test(&[target(&other)]),
        Err(Some(other.clone()))
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn settings_changed_in_one_window_reach_the_others() {
    let (mut winc, a, b, dir) = two_windows("windows-config");
    winc.app(a).step_font_size(Some(true));
    let size = winc.app(a).config.editor_size;
    assert!(size.is_some());
    winc.settle_for_test();
    assert_eq!(winc.app(b).config.editor_size, size);
    assert_eq!(winc.app(b).fonts.code, size.unwrap(), "字号也用上了");

    winc.app(b).settings_toggle(Setting::WordWrap);
    winc.settle_for_test();
    assert_eq!(winc.app(a).config.word_wrap, Some(true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_written_back_keep_what_was_edited_in_the_file_meanwhile() {
    let (mut winc, a, b, dir) = two_windows("windows-config-edits");
    let path = dir.join("config.json");
    for key in [a, b] {
        winc.app(key).config_path = Some(path.clone());
    }
    // 程序运行期间，用户在 winc 之外手动修改了配置文件，之后在设置页中修改了另一项。
    std::fs::write(&path, r#"{"editor": {"size": 16}}"#).unwrap();
    winc.app(a).settings_toggle(Setting::WordWrap);
    winc.settle_for_test();

    let (config, _) = Config::load_from(&path);
    assert_eq!(
        (config.editor_size, config.word_wrap),
        (Some(16.0), Some(true))
    );
    // 手动修改的字号没有被覆盖，两个窗口都已应用。
    for key in [a, b] {
        assert_eq!(winc.app(key).config, config);
        assert_eq!(winc.app(key).fonts.code, 16.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_cut_in_one_window_are_moved_when_pasted_in_another() {
    let (mut winc, a, b, dir) = two_windows("windows-cut");
    let file = dir.join("a.txt");
    winc.app(a).cut_entries(std::slice::from_ref(&file));
    assert_eq!(*winc.app(b).shared.cut_files.borrow(), [file]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_go_to_the_innermost_window_that_contains_them() {
    let (mut winc, a, b, dir) = two_windows("windows-containing");
    winc.app(b).open_path(&dir);
    winc.settle_for_test();
    let shared = std::rc::Rc::clone(&winc.app(a).shared);
    let src = winc_config::files::canonicalize(dir.join("src")).unwrap();
    assert_eq!(
        shared.window_containing(&src.join("main.rs")),
        Some(a),
        "最里面的目录"
    );
    assert_eq!(
        shared.window_containing(&src.with_file_name("a.txt")),
        Some(b)
    );
    assert_eq!(shared.window_containing(Path::new("/elsewhere/x")), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_windows_are_remembered_at_quit_and_reopened_at_launch() {
    let first = search_folder("windows-restore-a");
    let second = search_folder("windows-restore-b");
    let state_path = first.with_file_name("windows-restore-state.json");
    let _ = std::fs::remove_file(&state_path);
    let template = crate::winc::Template {
        config_path: None,
        state_path: Some(state_path.clone()),
        backup_root: None,
    };
    let mut winc = Winc::new(None, Some(template.clone()));
    let a = winc.add_detached(app_with(&first, false));
    let b = winc.add_detached(app_with(&second, false));
    winc.set_active(a);
    winc.remember_windows_for_test();
    let root = |winc: &mut Winc, key| winc.app(key).editor.root().map(Path::to_path_buf);
    let others = State::load_from(&state_path).others;
    assert_eq!(
        others,
        [root(&mut winc, b).unwrap()],
        "当前窗口的由它自己记在 `last`"
    );

    // 下次启动：第一个窗口在开始页上时，第一个目录开在它里面，不多留一个空窗口。
    let mut winc = Winc::new(None, Some(template));
    let a = winc.add_detached(blank_app(false));
    winc.restore_windows(&others);
    assert_eq!(root(&mut winc, a), others.first().cloned());
    assert!(winc.settle_for_test().0.is_empty());

    // 第一个窗口开着别的目录：其余的各开一个窗口，最后切回第一个窗口。
    let mut winc = Winc::new(None, None);
    let a = winc.add_detached(app_with(&first, false));
    winc.restore_windows(&others);
    let (requests, _) = winc.settle_for_test();
    assert_eq!(
        requests,
        [
            Request::NewWindow(Some(others[0].clone())),
            Request::Focus(a)
        ]
    );
    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&second);
    let _ = std::fs::remove_file(&state_path);
}

/// 在一个窗口剪切、到另一个窗口的目录里粘贴。
///
/// 回归：此前的实现在一个窗口剪切、到另一个窗口粘贴时失败，只能移动本窗口目录里的文件；
/// 文件移过去以后，剪切那边打开着的文档要跟到新位置。
#[test]
fn files_cut_in_one_window_move_into_another_windows_folder() {
    let a_dir = search_folder("windows-move-a");
    let b_dir = search_folder("windows-move-b");
    let notes = a_dir.join("notes.txt");
    std::fs::write(&notes, "笔记\n").unwrap();
    let mut winc = Winc::new(None, None);
    let a = winc.add_detached(app_with(&notes, false));
    let b = winc.add_detached(app_with(&b_dir.join("src/main.rs"), false));

    winc.app(a).cut_entries(std::slice::from_ref(&notes));
    winc.app(b).copied_files = vec![notes.clone()];
    let target = winc.app(b).editor.workspace.as_ref().unwrap().target_dir();
    winc.app(b).paste_into_tree();
    winc.settle_for_test();

    let moved = target.join("notes.txt");
    assert!(moved.exists() && !notes.exists());
    let doc = winc.app(a).editor.active().unwrap();
    assert_eq!(doc.path.as_deref(), Some(moved.as_path()));
    assert!(winc.app(b).shared.cut_files.borrow().is_empty(), "只移一次");
    let _ = std::fs::remove_dir_all(&a_dir);
    let _ = std::fs::remove_dir_all(&b_dir);
}

/// 回归：退出时另一个窗口正在问「关标签要不要保存」，它答完以后整个退出被当作取消。
#[test]
fn a_window_busy_with_another_question_is_asked_again_after_it() {
    let (mut winc, a, b, dir) = two_windows("windows-quit-busy");
    winc.app(b).config.hot_exit = Some(false);
    winc.app(b).new_file();
    winc.app(b).run(Command::Insert("x".into()));
    let tab = winc.app(b).editor.active.unwrap();
    winc.app(b).close_tab(tab);
    assert!(winc.app(b).closing.prompt.is_some(), "在问关标签要不要保存");

    winc.set_active(a);
    winc.app(a).request_quit();
    let (_, exit) = winc.settle_for_test();
    assert!(!exit && !winc.app(b).closing.quit_requested, "先等它问完");

    // 关标签那件事答完：再请它退出，标签已关、没有要问的，整个退出完成。
    winc.app(b).answer_save_prompt(SaveChoice::Discard);
    let (_, exit) = winc.settle_for_test();
    assert!(exit && winc.app(b).closing.quit_requested);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 开着热退出时，已写进备份的文档在退出时不再询问。
///
/// 回归：此前的实现只要有未保存的文档就要询问，macOS 上取消退出会让系统注销和关机随之中断。
/// 没写过备份的文档、关了热退出的文档仍然要问。
#[test]
fn unsaved_files_already_backed_up_do_not_need_asking_at_quit() {
    let dir = search_folder("windows-quit-backed-up");
    let root = super::workspace::session::backup_root("windows-quit-backed-up");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    assert!(!app.asks_before_quit());

    app.run(Command::Insert("// 还没存\n".into()));
    assert!(app.asks_before_quit(), "还没写过备份");
    super::workspace::session::back_up_now(&mut app);
    assert!(
        !app.asks_before_quit(),
        "已写进备份，退出时再按当前内容写一次"
    );
    // 退出时按当前内容再写一次：备份之后又输入的内容也在里面。
    app.run(Command::Insert("// 又改了\n".into()));
    assert!(!app.asks_before_quit());
    app.prepare_exit();
    let folder = app.editor.workspace.as_ref().unwrap().root.clone();
    let backups = winc_editor::backup::Backups::new(&root, Some(&folder)).read_all();
    let text = backups[0].text().unwrap();
    assert!(text.starts_with("// 还没存\n// 又改了\n"), "{text:?}");

    app.config.hot_exit = Some(false);
    assert!(app.asks_before_quit());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

/// 应用绘制标题栏时（macOS 以外的平台）：顶栏右端是最小化、最大化、关闭三个按钮，标签栏
/// 让出该区域；窗口四边是改变大小的细边；点关闭按钮和系统标题栏上的关闭按钮一样关掉该窗口。
#[test]
fn a_drawn_title_bar_has_window_buttons_and_resize_edges() {
    use crate::app::WindowControl;
    use winit::window::ResizeDirection;

    let (mut winc, a, b, dir) = two_windows("windows-title-bar");
    let app = winc.app(a);
    paint(app);
    assert_eq!(app.layout.controls, Rect::default(), "默认用系统的标题栏");
    app.window.controls = true;
    app.relayout();
    paint(app);
    let controls = app.layout.controls;
    let window = app.layout.window;
    assert!(controls.w > 0.0 && controls.right() < window.right());
    assert!(controls.y >= 0.0 && controls.bottom() <= TOP_BAR_END);
    assert!(app.layout.tabs.right() <= controls.x, "标签栏让出窗口按钮");

    let rects = app.window_control_rects();
    let order: Vec<_> = rects.iter().map(|&(_, c)| c).collect();
    assert_eq!(order, WindowControl::ALL);
    for (rect, control) in rects {
        let (x, y) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        assert_eq!(app.hover_at(x, y), Hover::WindowControl(control));
    }

    // 四边和四角：改变大小的方向和对应的指针；稍微往内就是窗口中的内容。
    let (w, h) = (window.w, window.h);
    for ((x, y), direction) in [
        ((1.0, 1.0), ResizeDirection::NorthWest),
        ((w - 1.0, h - 1.0), ResizeDirection::SouthEast),
        ((w / 2.0, 1.0), ResizeDirection::North),
        ((1.0, h / 2.0), ResizeDirection::West),
        ((w - 1.0, h / 2.0), ResizeDirection::East),
        ((w / 2.0, h - 1.0), ResizeDirection::South),
    ] {
        assert_eq!(app.hover_at(x, y), Hover::ResizeEdge(direction));
    }
    assert!(!matches!(app.hover_at(w / 2.0, 12.0), Hover::ResizeEdge(_)));
    app.on_mouse_move(w - 1.0, h / 2.0);
    assert_eq!(app.cursor_icon, CursorIcon::EResize);

    // 关闭按钮：和系统标题栏上的一样关掉该窗口。
    let (close, _) = app.window_control_rects()[2];
    click(app, (close.x + close.w / 2.0, close.y + close.h / 2.0));
    winc.settle_for_test();
    assert_eq!(winc.keys(), [b]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 「使用系统标题栏」（macOS 以外的平台）：开关马上生效，顶栏右端的窗口按钮随之去掉或回来。
#[cfg(not(target_os = "macos"))]
#[test]
fn the_title_bar_setting_applies_at_once() {
    use crate::layout::Setting;

    let (mut winc, a, _, dir) = two_windows("windows-title-bar-setting");
    let app = winc.app(a);
    app.sync_title_bar();
    app.relayout();
    assert!(app.window.controls && app.layout.controls.w > 0.0);
    assert!(!app.switch_on(Setting::SystemTitleBar));
    app.settings_toggle(Setting::SystemTitleBar);
    assert_eq!(app.config.ui_native_title_bar, Some(true));
    assert!(!app.window.controls);
    assert_eq!(app.layout.controls, Rect::default());
    app.settings_toggle(Setting::SystemTitleBar);
    assert!(app.window.controls && app.layout.controls.w > 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 各窗口共用同一张背景图层：第二个窗口直接采用已有的，不再生成。
#[test]
fn windows_share_one_background_layer() {
    let (mut winc, a, b, dir) = two_windows("windows-wallpaper");
    let image = dir.join("bg.png");
    let red = winc_render::Image {
        pixels: vec![0xffff_0000; 64 * 48],
        width: 64,
        height: 48,
    };
    write_png(&image, &red).unwrap();
    for key in [a, b] {
        winc.app(key).config.ui_background_image = Some(image.to_string_lossy().into_owned());
    }
    paint(winc.app(a));
    winc.app(a).wait_for_wallpaper();
    paint(winc.app(a));
    let layer = |app: &App| app.renderer.as_ref().unwrap().wallpaper().cloned();
    let first = layer(winc.app(a)).expect("第一个窗口的图层");

    paint(winc.app(b));
    assert!(!winc.app(b).wallpaper.is_busy());
    let second = layer(winc.app(b)).expect("第二个窗口的图层");
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    drop(second);

    // 两个窗口都取消后，图层释放。
    for key in [a, b] {
        winc.app(key).config.ui_background_image = None;
        paint(winc.app(key));
    }
    assert_eq!(std::sync::Arc::strong_count(&first), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
