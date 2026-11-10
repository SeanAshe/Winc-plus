//! 开始页与现场的测试：最近目录、现场的保存与恢复、切换目录、未保存内容的备份、
//! 临时打开的文件，以及目录锁在多个实例间的行为。

use super::*;
use winc_config::Tab;

#[test]
fn no_history_starts_on_the_start_page() {
    let mut app = blank_app(false);
    paint(&mut app);

    // 没有目录时侧栏收起，编辑区卡片占满整个宽度；开始页上有应用设置、新建文件和打开操作，没有
    // 最近列表。只有 macOS 的选择器能同时选文件和目录，其他平台上打开文件和打开目录是两项。
    assert!(app.layout.sidebar.right() <= 0.0);
    assert_eq!(app.layout.editor.x, ISLAND_MARGIN);
    let actions: Vec<StartAction> = app.hits.start.iter().map(|(_, a)| *a).collect();
    let mut expected = vec![
        StartAction::Settings,
        StartAction::NewFile,
        StartAction::Open,
    ];
    if !cfg!(target_os = "macos") {
        expected.push(StartAction::OpenFolder);
    }
    assert_eq!(actions, expected);

    // 点「应用设置」打开设置。
    let (row, _) = app.hits.start[0];
    app.on_mouse_move(row.x + 10.0, row.y + 10.0);
    assert_eq!(app.hover, Hover::Start(StartAction::Settings));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.settings.open);
    app.close_settings();
    paint(&mut app);

    // 点「新建文件」开一个未命名标签，开始页换成编辑区。
    let (row, _) = app.hits.start[1];
    app.on_mouse_move(row.x + 10.0, row.y + 10.0);
    assert_eq!(app.hover, Hover::Start(StartAction::NewFile));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.documents.len(), 1);
    paint(&mut app);
    assert!(app.hits.start.is_empty());
}

#[test]
fn a_file_dropped_on_the_start_page_opens_without_its_folder() {
    let dir = search_folder("drop-file");
    let other = sample_folder("drop-file-other");
    let mut app = blank_app(false);
    paint(&mut app);

    // 拖进一个文件：只打开该文件，不把它所在的目录当作项目打开，侧栏仍然收起。
    let file = dir.join("src/main.rs");
    app.open_path(&file);
    assert!(app.editor.workspace.is_none());
    assert_eq!(app.editor.documents.len(), 1);
    assert_eq!(app.focus, Focus::Editor);
    paint(&mut app);
    assert!(app.hits.start.is_empty());
    assert!(app.layout.sidebar.right() <= 0.0);

    // 拖进目录才打开目录，刚才的文件留在标签里。
    app.open_path(&dir);
    let root = winc_config::files::canonicalize(&dir).unwrap();
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert_eq!(app.editor.documents.len(), 1);

    // 打开了目录以后，拖进目录之外的文件也只开一个标签，不换目录。
    app.open_path(&other.join("a.txt"));
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert_eq!(app.editor.documents.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn recent_folders_open_and_can_be_forgotten() {
    let dir = search_folder("recent");
    let other = sample_folder("recent-other");
    let missing = std::env::temp_dir().join("winc-recent-missing-folder-zzz");
    let mut app = blank_app(false);
    app.state.recent = vec![
        winc_config::files::canonicalize(&other).unwrap(),
        missing.clone(),
        winc_config::files::canonicalize(&dir).unwrap(),
    ];
    paint(&mut app);

    let rect = |app: &App, action| app.hits.start.iter().find(|(_, a)| *a == action).unwrap().0;

    // 悬停在一行上时，右端的 ✕ 才可用；点击 ✕ 只从列表中移除该项，不打开目录。
    let close = rect(&app, StartAction::Forget(0));
    app.on_mouse_move(close.x + 4.0, close.y + 4.0);
    assert_eq!(app.hover, Hover::Start(StartAction::Forget(0)));
    app.on_mouse_down(MouseButton::Left);
    assert!(app.editor.workspace.is_none());
    assert_eq!(app.state.recent.len(), 2);
    assert_eq!(app.state.recent[0], missing);

    // 点击已不存在的目录：从列表中移除，并弹出提示。
    paint(&mut app);
    let row = rect(&app, StartAction::Recent(0));
    app.on_mouse_move(row.x + 40.0, row.y + 10.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.editor.workspace.is_none());
    assert_eq!(app.state.recent.len(), 1);
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("已从最近列表中移除"))
    );

    // 点击仍然存在的目录：打开该目录，侧栏展开，并将其移到最近列表最前面。
    paint(&mut app);
    let row = rect(&app, StartAction::Recent(0));
    app.on_mouse_move(row.x + 40.0, row.y + 10.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.editor.workspace.is_some());
    assert_eq!(
        app.state.recent[0],
        winc_config::files::canonicalize(&dir).unwrap()
    );

    // 测试中没有状态文件路径，这些变更只保存在内存中。
    assert!(app.state_path.is_none());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn session_is_saved_on_exit_and_restored_next_time() {
    let dir = search_folder("session");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.open_path(&dir.join("Cargo.toml"));
    app.open_path(&dir.join("src/config.rs"));
    app.new_file();
    let doomed = dir.join("gone.txt");
    std::fs::write(&doomed, "x").unwrap();
    app.open_path(&doomed);
    std::fs::remove_file(&doomed).unwrap();
    // 将 `Cargo.toml` 设为当前标签。
    let cargo = app
        .editor
        .documents
        .iter()
        .position(|d| d.name() == "Cargo.toml")
        .unwrap();
    app.activate_tab(cargo);

    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    // 没有备份时，不记录未命名的新文档和磁盘上已不存在的文件。
    let names: Vec<String> = session
        .tabs
        .iter()
        .map(|tab| match tab {
            Tab::File(p) => p.file_name().unwrap().to_string_lossy().into_owned(),
            Tab::Untitled(name) => name.clone(),
        })
        .collect();
    assert_eq!(names, ["main.rs", "Cargo.toml", "config.rs"]);
    assert_eq!(session.active, Some(1));

    // 下次启动：目录、标签和当前标签都恢复。
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    crate::app::restore_at_launch(&mut editor, Some(&session), None);
    assert_eq!(
        editor.workspace.as_ref().map(|w| w.root.clone()),
        Some(session.folder.clone())
    );
    let reopened: Vec<String> = editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect();
    assert_eq!(reopened, ["main.rs", "Cargo.toml", "config.rs"]);
    assert_eq!(editor.active().unwrap().name(), "Cargo.toml");
    // 只读了当前标签，其余的切换过去时再读。
    let pending: Vec<bool> = editor.documents.iter().map(|d| d.is_pending()).collect();
    assert_eq!(pending, [true, false, true]);
    // 运行期间切换目录时恢复的标签都只占位，当前标签交给后台读取，不在事件循环中读文件。
    let mut switched = Editor::default();
    assert!(crate::open_session_folder(&mut switched, &session));
    crate::app::restore_tabs(&mut switched, Some(&session), None);
    assert!(switched.documents.iter().all(|d| d.is_pending()));
    let mut restored = blank_app(false);
    restored.editor = editor;
    restored.activate_tab(0);
    assert!(!restored.editor.documents[0].is_pending());
    assert!(restored.editor.documents[0].text().line_count() > 0);

    // 目录已不存在：不恢复任何内容，显示开始页。
    let mut editor = Editor::default();
    let gone = Session {
        folder: dir.join("no-such-dir"),
        ..session.clone()
    };
    assert!(!crate::open_session_folder(&mut editor, &gone));
    assert!(editor.workspace.is_none() && editor.documents.is_empty());

    // 未打开目录就退出，下次启动仍是开始页（该目录的现场仍然保留，从最近列表打开时使用）。
    let mut app = blank_app(false);
    app.state.set_session(session.clone());
    app.state.last = Some(session.folder.clone());
    app.save_session();
    assert!(app.state.last_session().is_none());
    assert!(app.state.session_for(&session.folder).is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_folders_keeps_each_folders_tabs() {
    let a = search_folder("switch-a");
    let b = sample_folder("switch-b");
    let mut app = app_with(&a, false);
    let root_a = app.editor.workspace.as_ref().unwrap().root.clone();
    app.open_path(&root_a.join("src/main.rs"));
    app.open_path(&root_a.join("Cargo.toml"));
    let names = |app: &App| -> Vec<String> {
        app.editor
            .documents
            .iter()
            .map(|d| d.name().into_owned())
            .collect()
    };

    // 切换到目录 B：A 的标签关闭；B 没有打开过，因此没有标签。
    app.open_path(&b);
    let root_b = app.editor.workspace.as_ref().unwrap().root.clone();
    assert_ne!(root_a, root_b);
    assert!(app.editor.documents.is_empty());
    app.open_path(&root_b.join("a.txt"));

    // 有未保存的修改时先询问；取消则仍停留在 B。
    app.run(Command::Insert("x".into()));
    app.open_path(&root_a);
    assert!(app.closing.prompt.is_some());
    app.answer_save_prompt(SaveChoice::Cancel);
    assert_eq!(app.editor.workspace.as_ref().unwrap().root, root_b);

    // 不保存：回到 A，A 的标签和当前标签都恢复。
    app.open_path(&root_a);
    app.answer_save_prompt(SaveChoice::Discard);
    assert_eq!(app.editor.workspace.as_ref().unwrap().root, root_a);
    assert_eq!(names(&app), ["main.rs", "Cargo.toml"]);
    assert_eq!(app.editor.active().unwrap().name(), "Cargo.toml");

    // 项目名菜单：最近的目录（不含当前目录）、新建窗口、打开目录、关闭目录。菜单开着时项目名
    // 按钮处于按下状态，关掉菜单后放开。
    app.open_project_menu();
    assert_eq!(app.menu_button, Some(crate::app::MenuButton::Project));
    let menu = app.context_menu.as_ref().unwrap();
    let commands: Vec<_> = menu
        .entries
        .iter()
        .filter_map(|e| match *e {
            crate::app::input::context_menu::MenuEntry::Item(c, _) => Some(c),
            _ => None,
        })
        .collect();
    let b_index = app.state.recent.iter().position(|p| *p == root_b).unwrap();
    assert_eq!(
        commands,
        [
            MenuCommand::OpenRecent(b_index),
            MenuCommand::NewWindow,
            MenuCommand::OpenFolder,
            MenuCommand::CloseFolder
        ]
    );

    // 最近的目录：目录图标加名字，占一行；完整路径显示在悬停提示中。
    let label = app.menu_label(MenuCommand::OpenRecent(b_index));
    let name = winc_editor::display_name(&root_b).into_owned();
    assert_eq!(label.title, name);
    assert_eq!(label.subtitle, None);
    let full = winc_editor::display_path(&root_b, None);
    assert_eq!(label.tooltip.as_deref(), Some(full.as_str()));

    // 图标与文件树中的目录图标相同，颜色也相同：线条风格，底边是实色的线，中间空着，右上角
    // （标签右边）透明。
    let folder = app.menu_icon(MenuCommand::OpenRecent(b_index)).unwrap();
    let (w, h) = (folder.width as usize, folder.height as usize);
    assert_eq!((w, h), (30, 24));
    let channel = |v: f32| (v * 255.0).round() as u32;
    let p = app.palette.folder;
    let solid = 0xff00_0000 | channel(p.r) << 16 | channel(p.g) << 8 | channel(p.b);
    assert_eq!(folder.pixels[(h - 2) * w + w / 2], solid);
    assert_eq!(folder.pixels[(h * 2 / 3) * w + w / 2] >> 24, 0);
    assert_eq!(folder.pixels[w - 1] >> 24, 0);
    // 没有系统菜单时窗口内的菜单同样带图标，只有最近的目录一项有，画出来是目录的颜色。
    paint(&mut app);
    let popup = app.popup().expect("窗口内的菜单").levels[0].clone();
    let icons: Vec<_> = (popup.rows.iter())
        .filter_map(|row| match row {
            crate::app::PopupRow::Item { icon, .. } => Some(*icon),
            _ => None,
        })
        .collect();
    let folder_icon = crate::app::input::context_menu::MenuIcon::Folder;
    assert_eq!(icons, [Some(folder_icon), None, None, None]);
    let row = popup.row_rects[0];
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let scale = image.width as f32 / app.layout.window.w;
    let (x0, x1) = ((row.x + 24.0) * scale, (row.x + 46.0) * scale);
    let (y0, y1) = (row.y * scale, row.bottom() * scale);
    let drawn = (y0 as usize..y1 as usize).any(|y| {
        (x0 as usize..x1 as usize).any(|x| image.pixels[y * image.width as usize + x] == solid)
    });
    assert!(drawn, "目录图标没有画出来");
    // 其余两项只有文字；用词统一为「目录」而不是「文件夹」，与开始页、文件树一致。
    for (command, title) in [
        (MenuCommand::OpenFolder, "打开目录…"),
        (MenuCommand::CloseFolder, "关闭目录"),
    ] {
        assert!(app.menu_icon(command).is_none(), "{command:?}");
        let label = app.menu_label(command);
        assert_eq!(
            (label.title.as_str(), label.subtitle, label.tooltip),
            (title, None, None)
        );
    }
    assert!(app.menu_icon(MenuCommand::Rename).is_none());
    // 项目名旁边的徽章：A 和 B 的颜色不同。
    assert_ne!(
        app.state.badge_color(&root_a),
        app.state.badge_color(&root_b)
    );

    // 两个最近的目录同名时无法区分，才在名字下方显示其所在目录。
    let twin = root_b
        .parent()
        .unwrap()
        .join("twin")
        .join(root_b.file_name().unwrap());
    app.state.remember(&twin);
    let index = app.state.recent.iter().position(|p| *p == root_b).unwrap();
    let label = app.menu_label(MenuCommand::OpenRecent(index));
    let parent = winc_editor::display_path(root_b.parent().unwrap(), None);
    assert_eq!(label.subtitle.as_deref(), Some(parent.as_str()));
    app.state.forget(&twin);

    // 从菜单切换到 B：B 的 `a.txt` 恢复（之前未保存的修改已放弃），项目名按钮放开。
    app.run_menu_command(MenuCommand::OpenRecent(b_index));
    assert_eq!(app.menu_button, None);
    assert_eq!(app.editor.workspace.as_ref().unwrap().root, root_b);
    assert_eq!(names(&app), ["a.txt"]);
    assert!(!app.editor.active().unwrap().is_dirty());

    // 关闭目录：回到开始页，侧栏收起。
    app.open_project_menu();
    app.run_menu_command(MenuCommand::CloseFolder);
    assert!(app.editor.workspace.is_none() && app.editor.documents.is_empty());
    assert!(!app.sidebar_open());
    paint(&mut app);
    assert!(!app.hits.start.is_empty());

    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// 返回存放备份的临时目录路径（名称含 `name` 和进程号），并清除已有的同名目录。
pub(in crate::app::snapshot) fn backup_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-backups-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 模拟时间流逝，使所有待写的备份立即写入完成。
pub(in crate::app::snapshot) fn back_up_now(app: &mut App) {
    let later = Instant::now() + Duration::from_secs(5);
    app.backup_tick(Instant::now());
    app.backup_tick(later);
    app.collect_backups(true);
    app.backup_tick(later);
    app.collect_backups(true);
}

#[cfg(unix)]
#[test]
fn quitting_rewrites_only_backups_whose_content_or_caret_changed() {
    use std::os::unix::fs::MetadataExt;

    // 写好一个备份文件，`moved` 为真时再移动光标，然后退出；返回退出前后备份文件是否是同一个文件。
    let quit = |name: &str, moved: bool| {
        let dir = search_folder(name);
        let root = backup_root(name);
        let mut app = app_with(&dir.join("src/main.rs"), false);
        app.backup.root = Some(root.clone());
        app.run(Command::Insert("// 还没存\n".into()));
        back_up_now(&mut app);
        let backups = winc_editor::backup::Backups::new(&root, app.editor.root());
        let file = backups.read_all()[0].file.clone();
        let before = std::fs::metadata(&file).unwrap().ino();
        if moved {
            app.editor.active_mut().unwrap().selection = winc_core::Selection::point(0);
        }
        app.request_quit();
        assert!(
            app.closing.quit_requested && app.closing.prompt.is_none(),
            "已备份，不询问"
        );
        let same = std::fs::metadata(&file).unwrap().ino() == before;
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&root);
        same
    };
    // 备份写入后没有任何变化：原样保留，不重写正文。
    assert!(quit("hot-exit-current", false));
    // 光标移动过：重写，下次启动时光标恢复到最后的位置。
    assert!(!quit("hot-exit-caret", true));
}

#[test]
fn closing_a_folder_lets_this_window_lock_it_again() {
    let dir = search_folder("close-relock");
    let root = backup_root("close-relock");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.run(Command::Insert("// 备份\n".into()));
    back_up_now(&mut app);
    assert!(app.holds_backup_lock());

    // 关闭目录（不保存）：锁放开；从开始页重新打开时本窗口重新拿到锁，不提示
    // 「已在另一个窗口中打开」。
    app.run(Command::Undo);
    app.close_folder();
    assert!(app.editor.root().is_none());
    assert!(!app.holds_backup_lock());
    app.open_path(&dir);
    assert!(app.holds_backup_lock(), "重新打开以后锁在本窗口");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn quitting_keeps_unsaved_changes_for_next_time() {
    let dir = search_folder("hot-exit");
    let root = backup_root("hot-exit");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.run(Command::Insert("// 还没存\n".into()));
    app.new_file();
    app.run(Command::Insert("草稿".into()));
    app.open_path(&dir.join("Cargo.toml"));
    let gone = dir.join("gone.txt");
    std::fs::write(&gone, "x").unwrap();
    app.open_path(&gone);
    app.run(Command::Insert("留着 ".into()));
    std::fs::remove_file(&gone).unwrap();
    app.activate_tab(1);

    // 退出：不询问是否保存，直接退出（未保存的内容由备份保留）。
    app.request_quit();
    assert!(app.closing.prompt.is_none() && app.closing.quit_requested);
    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    assert_eq!(
        session.tabs.len(),
        4,
        "未命名的、磁盘上没有了的也记着：{session:?}"
    );
    assert!(matches!(session.tabs[1], Tab::Untitled(_)));
    assert_eq!(session.active, Some(1));
    let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(!main.contains("还没存"), "文件本身没动");

    // 下次启动：标签、顺序、当前标签和未保存的内容都恢复，且仍处于未保存状态。
    drop(app);
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    let restored = crate::app::restore_at_launch(&mut editor, Some(&session), Some(&root));
    let mut next = App::new(editor, None, Bench::from_env(), Config::default());
    next.backup.root = Some(root.clone());
    next.adopt_restored(restored);
    let docs: Vec<(String, bool)> = next
        .editor
        .documents
        .iter()
        .map(|d| (d.name().into_owned(), d.is_dirty()))
        .collect();
    assert_eq!(
        docs,
        [
            ("main.rs".into(), true),
            ("untitled".into(), true),
            ("Cargo.toml".into(), false),
            ("gone.txt".into(), true),
        ]
    );
    assert_eq!(next.editor.active, Some(1));
    let text = |app: &App, i: usize| app.editor.documents[i].text().rope().unwrap().to_string();
    assert!(text(&next, 0).starts_with("// 还没存\n"));
    assert_eq!(text(&next, 1), "草稿");
    assert_eq!(text(&next, 3), "留着 x");
    // 刚恢复的内容无需立即重新备份。
    next.backup_tick(Instant::now() + Duration::from_secs(5));
    assert!(next.backup.job.is_none());

    // ⌘Z 撤销回磁盘上的内容后，文档不再有未保存的修改：对应的备份删除；保存后的备份同样删除。
    next.activate_tab(0);
    next.run(Command::Undo);
    assert!(!next.editor.documents[0].is_dirty());
    next.activate_tab(3);
    next.save_active();
    assert_eq!(std::fs::read_to_string(&gone).unwrap(), "留着 x");
    back_up_now(&mut next);
    let left = winc_editor::backup::Backups::new(&root, next.editor.root()).read_all();
    assert_eq!(left.len(), 1, "只剩未命名的那一个：{left:?}");

    // 关闭该标签（不保存）：备份连同备份目录一起删除。
    next.close_tab(1);
    next.answer_save_prompt(SaveChoice::Discard);
    back_up_now(&mut next);
    // 备份根目录下允许剩下锁文件（文件而非目录），其余子目录必须为空。
    assert!(
        !root.exists()
            || std::fs::read_dir(&root).unwrap().all(|e| {
                let path = e.unwrap().path();
                !path.is_dir() || std::fs::read_dir(path).unwrap().next().is_none()
            })
    );

    let _ = std::fs::remove_file(&gone);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn edits_made_after_agreeing_to_quit_are_backed_up_too() {
    let dir = search_folder("hot-exit-late");
    let root = backup_root("hot-exit-late");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.run(Command::Insert("先改的\n".into()));
    app.request_quit();
    assert!(app.closing.quit_requested);
    // 同意退出后、其他窗口仍在询问期间，本窗口又修改了一处。
    app.run(Command::Insert("后改的\n".into()));
    app.prepare_exit();

    let session = app.state.last_session().cloned().unwrap();
    drop(app);
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    let restored = crate::app::restore_at_launch(&mut editor, Some(&session), Some(&root));
    let mut next = App::new(editor, None, Bench::from_env(), Config::default());
    next.backup.root = Some(root.clone());
    next.adopt_restored(restored);
    let text = next
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.starts_with("先改的\n后改的\n"), "{text}");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn closing_the_window_closes_the_folder_but_keeps_its_tabs_and_changes() {
    let dir = search_folder("close-window");
    let root = backup_root("close-window");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.run(Command::Insert("// 还没存\n".into()));
    app.open_path(&dir.join("Cargo.toml"));
    let folder = app.editor.root().unwrap().to_path_buf();

    // 关闭窗口：与退出一样不询问，未保存的内容备份。
    app.request_close_window();
    assert!(app.closing.prompt.is_none());
    if !cfg!(target_os = "macos") {
        // 其他平台上关闭窗口就是退出。
        assert!(app.closing.quit_requested);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    // macOS：不退出应用。窗口隐藏，目录和标签关闭，回到开始页；目录的现场和备份都留着，
    // 之后退出时没有打开的目录，下次启动是开始页。
    assert!(!app.closing.quit_requested && app.window.hidden);
    assert!(app.editor.workspace.is_none() && app.editor.documents.is_empty());
    assert!(app.state.last.is_none());
    let session = app
        .state
        .session_for(&folder)
        .cloned()
        .expect("目录的现场记下");
    assert_eq!(session.tabs.len(), 2);
    let backups = winc_editor::backup::Backups::new(&root, Some(&folder));
    assert_eq!(backups.read_all().len(), 1, "未保存的内容留在磁盘上");
    // 标签已经关闭，但后续的备份不会删除该备份文件。
    back_up_now(&mut app);
    assert_eq!(backups.read_all().len(), 1);

    // 在同一个进程里从最近列表重新打开该目录：标签和未保存的内容都回来。
    app.open_folder_now(&folder);
    let names: Vec<_> = app
        .editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect();
    assert_eq!(names, ["main.rs", "Cargo.toml"]);
    let main = &app.editor.documents[0];
    assert!(main.is_dirty());
    assert!(
        main.text()
            .rope()
            .unwrap()
            .to_string()
            .starts_with("// 还没存\n")
    );

    // 这时用 ⌘Q 退出：下次启动打开该目录。
    app.request_quit();
    assert!(app.closing.quit_requested);
    app.save_session();
    assert_eq!(app.state.last.as_deref(), Some(folder.as_path()));

    // 设置中关掉「启动时打开上次的目录」：⌘Q 退出后也是开始页，现场仍然记下。
    app.config.restore_folder = Some(false);
    app.save_session();
    assert!(app.state.last.is_none());
    assert!(app.state.session_for(&folder).is_some());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unsaved_changes_survive_a_crash() {
    let dir = search_folder("crash");
    let root = backup_root("crash");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.save_session();
    let session = app.state.last_session().cloned().unwrap();

    // 修改后不立即写备份，约 1 秒（`BACKUP_DELAY`）后在后台写入。
    app.run(Command::Insert("a".into()));
    let now = Instant::now();
    let due = app.backup_tick(now).expect("一会儿再来写");
    assert!(due > now && due <= now + Duration::from_secs(1));
    assert!(app.backup.job.is_none());
    // 持续输入不会推迟已确定的到期时刻：到点即写。
    app.run(Command::Insert("b".into()));
    assert_eq!(app.backup_tick(now + Duration::from_millis(500)), Some(due));
    app.new_file();
    app.run(Command::Insert("新的".into()));
    back_up_now(&mut app);

    // 程序异常终止：来不及询问，也来不及记录现场。下次启动仍然能恢复，排在现场标签之后。
    drop(app);
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    crate::app::restore_at_launch(&mut editor, Some(&session), Some(&root));
    let texts: Vec<String> = editor
        .documents
        .iter()
        .map(|d| d.text().rope().unwrap().chars().take(2).collect())
        .collect();
    assert_eq!(texts, ["ab", "新的"]);
    assert!(editor.documents.iter().all(|d| d.is_dirty()));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_second_winc_on_the_same_folder_leaves_the_first_ones_backups_alone() {
    let dir = search_folder("two-wincs");
    let root = backup_root("two-wincs");
    let mut first = app_with(&dir.join("src/main.rs"), false);
    first.backup.root = Some(root.clone());
    first.run(Command::Insert("第一个 ".into()));
    back_up_now(&mut first);
    first.save_session();
    let session = first.state.last_session().cloned().unwrap();

    // 第二个实例打开同一个目录：第一个实例仍在使用的备份不恢复。
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    let restored = crate::app::restore_at_launch(&mut editor, Some(&session), Some(&root));
    assert!(editor.documents.iter().all(|d| !d.is_dirty()));
    let mut second = App::new(editor, None, Bench::from_env(), Config::default());
    second.backup.root = Some(root.clone());
    second.adopt_restored(restored);
    assert!(
        second
            .notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("另一个 Winc"))
    );

    // 第二个实例中修改并保存：不触碰第一个实例的备份；第二个实例自己不做备份，退出时仍然询问。
    second.run(Command::Insert("第二个 ".into()));
    back_up_now(&mut second);
    second.save_active();
    back_up_now(&mut second);
    let backups = winc_editor::backup::Backups::new(&root, first.editor.root());
    let left = backups.read_all();
    assert_eq!(left.len(), 1);
    assert!(left[0].text().unwrap().starts_with("第一个 "));
    // 第一个退出、放开锁以后，第二个下次备份时接手，未保存的修改从此也有备份。
    drop(first);
    // 其他测试启动外部程序时，子进程在 exec 之前短暂继承第一个实例的锁，等它释放。
    drop(backups.lock_within(std::time::Duration::from_secs(1)));
    second.run(Command::Insert("接手 ".into()));
    back_up_now(&mut second);
    // 同一个文件只有一个备份文件：以第二个实例中打开的内容为准。
    let left = backups.read_all();
    assert_eq!(left.len(), 1);
    let text = left[0].text().unwrap();
    // 断言失败时一并输出第二个实例是否持有锁、此刻能否取到锁，便于分辨是没取到锁还是取到了锁
    // 却没写。
    assert!(
        text.starts_with("第二个 接手 "),
        "备份内容：{text:?}；第二个持有锁：{}；此刻能取到锁：{}",
        second.holds_backup_lock(),
        backups.lock().is_some(),
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_a_place_for_backups_quitting_still_asks() {
    let dir = search_folder("no-backups");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.run(Command::Insert("x".into()));
    assert!(app.backup.root.is_none());
    app.request_quit();
    assert!(app.closing.prompt.is_some() && !app.closing.quit_requested);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_hot_exit_off_quitting_asks_and_then_drops_the_backups() {
    let dir = search_folder("hot-exit-off");
    let root = backup_root("hot-exit-off");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.backup.root = Some(root.clone());
    app.config.hot_exit = Some(false);
    app.run(Command::Insert("x".into()));

    // 关闭热退出（`hot_exit`）时仍然备份：程序异常终止后仍可恢复。
    back_up_now(&mut app);
    let backups = winc_editor::backup::Backups::new(&root, app.editor.root());
    assert_eq!(backups.read_all().len(), 1);

    // 退出时仍然询问；取消则不做任何改动。
    app.request_quit();
    assert!(app.closing.prompt.is_some() && !app.closing.quit_requested);
    app.answer_save_prompt(SaveChoice::Cancel);
    assert_eq!(backups.read_all().len(), 1);
    // 关闭窗口时同样询问；取消则窗口、目录和标签都不动。
    app.request_close_window();
    assert!(app.closing.prompt.is_some());
    app.answer_save_prompt(SaveChoice::Cancel);
    assert!(!app.window.hidden && !app.closing.quit_requested);
    assert!(app.editor.workspace.is_some() && !app.editor.documents.is_empty());

    // 不保存：退出并删除备份，下次启动不恢复。
    app.request_quit();
    app.answer_save_prompt(SaveChoice::Discard);
    assert!(app.closing.quit_requested);
    assert!(backups.read_all().is_empty());
    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    drop(app);
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    crate::app::restore_at_launch(&mut editor, Some(&session), Some(&root));
    assert!(editor.documents.iter().all(|d| !d.is_dirty()));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn folded_blocks_are_restored_with_the_session() {
    let dir = search_folder("session-folds");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let tab_width = app.editor.active_tab_width();
    let doc = app.editor.active_mut().unwrap();
    let start = doc
        .fold_ranges(tab_width)
        .first()
        .expect("有可折叠的块")
        .start;
    assert!(doc.toggle_fold_at(start, tab_width));
    let folded = doc.folded();

    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    assert_eq!(session.folds.len(), 1);
    assert_eq!(session.folds[0].1, folded);

    // 下次启动：当前标签读入后恢复折叠。
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    crate::app::restore_at_launch(&mut editor, Some(&session), None);
    assert_eq!(editor.active().unwrap().folded(), folded);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn waiting_quits_once_the_files_are_closed_and_leaves_no_trace() {
    let dir = search_folder("wait");
    let message = dir.join("COMMIT_EDITMSG");
    std::fs::write(&message, "\n# 请输入提交说明\n").unwrap();
    let args = crate::cli::parse([message.clone().into_os_string()]).unwrap();
    let args = crate::cli::Args { wait: true, ..args };

    let mut editor = Editor::default();
    assert!(crate::open_targets(&mut editor, &args).is_empty());
    // 临时打开模式不打开项目。
    assert!(editor.workspace.is_none());
    let mut app = App::new(editor, None, Bench::from_env(), Config::default());
    app.start_one_off(&args);
    let mut winc = crate::winc::Winc::new(None, None);
    let key = winc.add_detached(app);

    // 写入内容并保存，再关闭标签：退出。
    winc.app(key).run(Command::Insert("修好了".into()));
    winc.app(key).save_active();
    assert!(!winc.settle_for_test().1);
    winc.app(key).close_tab(0);
    assert!(winc.settle_for_test().1);
    assert!(
        std::fs::read_to_string(&message)
            .unwrap()
            .starts_with("修好了")
    );
    // 没有改动最近列表和现场。
    let app = winc.app(key);
    assert!(app.state.recent.is_empty() && app.state.sessions.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn waiting_asks_about_other_unsaved_files_before_quitting() {
    let dir = search_folder("wait-others");
    let message = dir.join("COMMIT_EDITMSG");
    std::fs::write(&message, "\n").unwrap();
    let args = crate::cli::parse([message.into_os_string()]).unwrap();
    let args = crate::cli::Args { wait: true, ..args };
    let mut editor = Editor::default();
    assert!(crate::open_targets(&mut editor, &args).is_empty());
    let mut app = App::new(editor, None, Bench::from_env(), Config::default());
    app.start_one_off(&args);
    let mut winc = crate::winc::Winc::new(None, None);
    let key = winc.add_detached(app);

    // 等待期间又新建了一个文件并写入内容，没有保存（临时打开时不写备份）。
    winc.app(key).new_file();
    winc.app(key).run(Command::Insert("草稿".into()));
    let message = winc.app(key).editor.documents[0].id;
    // 关闭等待的标签：先询问另一个文档是否保存，不直接退出而丢弃其内容。
    winc.app(key).close_tabs(vec![message]);
    let (_, exit) = winc.settle_for_test();
    assert!(!exit && winc.app(key).closing.prompt.is_some());

    // 取消后窗口保持打开；之后按正常流程退出，选择不保存同样可以退出。
    winc.app(key).answer_save_prompt(SaveChoice::Cancel);
    let (_, exit) = winc.settle_for_test();
    assert!(!exit && winc.app(key).closing.prompt.is_none());
    assert_eq!(winc.app(key).editor.documents.len(), 1);
    winc.app(key).request_quit();
    winc.app(key).answer_save_prompt(SaveChoice::Discard);
    assert!(winc.settle_for_test().1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_given_with_a_line_open_there() {
    let dir = search_folder("cli-line");
    let main = dir.join("src/main.rs");
    let arg = format!("{}:3:5", main.display());
    let args = crate::cli::parse([arg.into()]).unwrap();
    let mut editor = Editor::default();
    assert!(crate::open_targets(&mut editor, &args).is_empty());
    let doc = editor.active().unwrap();
    let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
    assert_eq!((line, col), (2, 4));
    // 命令行只给出文件时：不打开其所在的目录。
    assert!(editor.workspace.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 侧栏的开关按目录记住：重新打开、启动时恢复，没有记录过的目录仍然显示文件树。
#[test]
fn the_sidebar_state_is_remembered_per_folder() {
    let first = sample_folder("sidebar-first");
    let second = sample_folder("sidebar-second");
    let mut app = app_with(&first.join("short.rs"), false);
    assert!(app.sidebar.visible);
    app.toggle_sidebar();
    assert!(!app.sidebar.visible);
    app.remember_session();
    let state = app.state.clone();

    // 从开始页打开这个目录：侧栏收起，焦点不留在看不见的文件树上。
    let mut app = blank_app(false);
    app.state = state.clone();
    app.open_folder_now(&first);
    assert!(!app.sidebar.visible);
    assert!(!app.focus_in_sidebar());

    // 启动时目录在窗口创建之前打开：窗口建好时恢复，没有动画。
    let mut app = app_with(&first.join("short.rs"), false);
    app.state = state.clone();
    app.restore_sidebar_for_window();
    assert!(!app.sidebar.visible);
    assert!(app.sidebar.motion.is_none());

    // 没有记录过的目录显示文件树，上一个目录收起不影响它。
    let mut app = blank_app(false);
    app.state = state.clone();
    app.open_folder_now(&second);
    assert!(app.sidebar.visible);

    // 重新展开后记下的也是展开；关闭目录时侧栏收起是回到开始页，不算用户收起。
    let mut app = blank_app(false);
    app.state = state;
    app.open_folder_now(&first);
    app.toggle_sidebar();
    let root = app.editor.root().unwrap().to_path_buf();
    app.close_folder();
    assert!(!app.sidebar.visible);
    let session = app.state.session_for(&root).unwrap();
    assert!(!session.sidebar_hidden);

    std::fs::remove_dir_all(&first).unwrap();
    std::fs::remove_dir_all(&second).unwrap();
}
