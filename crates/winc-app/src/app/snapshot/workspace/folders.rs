//! 附加目录的测试：添加和移除、文件树的顶层行、菜单、相对路径、随会话恢复。

use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};
use crate::app::{MenuIcon, PopupRow};

/// 文件树各行的名字和缩进层级。
fn rows(app: &App) -> Vec<(String, usize)> {
    let ws = app.editor.workspace.as_ref().unwrap();
    (ws.rows().iter())
        .map(|entry| (entry.label().into_owned(), entry.depth))
        .collect()
}

/// 在 `path` 上打开文件树菜单，返回列出的命令。
fn listed(app: &mut App, path: Option<PathBuf>) -> Vec<MenuCommand> {
    app.open_context_menu(MenuTarget::Tree(path), (10.0, 10.0));
    let menu = app.overlays.context_menu.take().unwrap();
    (menu.entries.into_iter())
        .filter_map(|entry| match entry {
            MenuEntry::Item(command, _) => Some(command),
            _ => None,
        })
        .collect()
}

#[test]
fn a_folder_can_be_added_to_the_window_and_removed() {
    let main = winc_config::files::canonicalize(sample_folder("folders-main")).unwrap();
    let other = winc_config::files::canonicalize(search_folder("folders-other")).unwrap();
    let mut app = app_with(&main.join("short.rs"), false);
    paint(&mut app);
    let single = rows(&app);
    assert!(single.iter().all(|(_, depth)| *depth == 0));
    // 空白处的菜单可以添加，没有可移除的。
    let blank = listed(&mut app, None);
    assert!(blank.contains(&MenuCommand::AddFolder));
    assert!(!blank.contains(&MenuCommand::RemoveFolder));

    // 项目菜单里「打开目录…」的下一项是「添加目录到窗口…」；没有打开目录时没有这一项。
    let project = |app: &mut App| -> Vec<MenuCommand> {
        app.open_context_menu(MenuTarget::Project, (10.0, 10.0));
        let menu = app.overlays.context_menu.take().unwrap();
        (menu.entries.into_iter())
            .filter_map(|entry| match entry {
                MenuEntry::Item(command, _) => Some(command),
                _ => None,
            })
            .collect()
    };
    let menu = project(&mut app);
    let open = menu
        .iter()
        .position(|c| *c == MenuCommand::OpenFolder)
        .unwrap();
    assert_eq!(menu.get(open + 1), Some(&MenuCommand::AddFolder));
    assert!(!project(&mut blank_app(false)).contains(&MenuCommand::AddFolder));

    app.add_folder(&other);
    paint(&mut app);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&other));
    let both = rows(&app);
    let tops: Vec<&str> = (both.iter().filter(|(_, depth)| *depth == 0))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(tops, ["folders-main", "folders-other"]);
    assert!(both.iter().filter(|(_, depth)| *depth == 1).count() >= single.len());
    // 窗口的名字和打开的目录不变。
    assert_eq!(app.editor.root(), Some(main.as_path()));

    // 目录自己的一行：不能改名、删除、剪切；附加的那个可以移除。
    let on_main = listed(&mut app, Some(main.clone()));
    let on_other = listed(&mut app, Some(other.clone()));
    for menu in [&on_main, &on_other] {
        assert!(menu.contains(&MenuCommand::AddFolder));
        assert!(!menu.contains(&MenuCommand::Rename) && !menu.contains(&MenuCommand::Delete));
        assert!(!menu.contains(&MenuCommand::Cut));
    }
    assert!(!on_main.contains(&MenuCommand::RemoveFolder));
    assert!(on_other.contains(&MenuCommand::RemoveFolder));
    // 其中的文件仍然：可以改名、删除，没有移除。
    let inside = other.join("src/main.rs");
    app.editor.workspace.as_mut().unwrap().reveal(&inside);
    let file = listed(&mut app, Some(inside));
    assert!(file.contains(&MenuCommand::Rename) && file.contains(&MenuCommand::Delete));
    assert!(!file.contains(&MenuCommand::RemoveFolder));

    // 相对路径相对于文件所在的目录。
    assert_eq!(
        app.relative_path(&other.join("src/main.rs")).as_deref(),
        Some("src/main.rs")
    );
    assert_eq!(
        app.relative_path(&main.join("short.rs")).as_deref(),
        Some("short.rs")
    );
    assert_eq!(app.relative_path(&other), None);

    // 删除、改名对目录自己的一行不起作用。
    app.ask_delete(&other);
    app.start_rename(&other);
    assert!(app.panels.name_input.is_none() && other.is_dir());

    // 重复添加、添加其中的子目录：说明原因，不改变列表。
    app.add_folder(&other.join("src"));
    let notice = app.notice.text.take();
    assert_eq!(
        notice.as_deref(),
        Some(winc_i18n::t!("workspace-add-folder-overlaps"))
    );
    assert_eq!(app.extra_folders().len(), 1);
    shoot(&mut app, "folders-two");

    // 有附加的目录时项目菜单里可以给这一组起名：输入框里是现在的名字，留空恢复为目录名。
    assert!(project(&mut app).contains(&MenuCommand::RenameGroup));
    app.ask_rename_group();
    assert!(app.overlays.form_dialog.is_some());
    app.focused_field().unwrap().insert("  数据平台  ");
    assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(app.editor.workspace.as_ref().unwrap().name, "数据平台");
    app.ask_rename_group();
    assert_eq!(app.focused_field().unwrap().text(), "数据平台");
    app.close_form_dialog();

    // 改动当时就记进会话，不等退出；也不改变下次启动时打开哪个目录。
    let saved = app.state.session_for(&main).unwrap();
    assert_eq!(saved.folders, std::slice::from_ref(&other));
    assert_eq!(saved.name.as_deref(), Some("数据平台"));
    let state = app.state.clone();
    let mut again = blank_app(false);
    again.state = state.clone();
    again.open_folder_now(&main);
    assert_eq!(again.extra_folders(), std::slice::from_ref(&other));
    assert_eq!(again.editor.workspace.as_ref().unwrap().name, "数据平台");

    // 最近列表里这一组用起的名字和叠放的目录图标；没起名时在目录名后注明附加的个数。
    let mut elsewhere = blank_app(false);
    elsewhere.state = state;
    elsewhere.open_folder_now(&other);
    let index = (elsewhere.state.recent.iter())
        .position(|folder| *folder == main)
        .unwrap();
    let recent = MenuCommand::OpenRecent(index);
    assert_eq!(elsewhere.menu_label(recent).title, "数据平台");
    let group_icon = crate::app::input::context_menu::MenuIcon::Folders;
    assert_eq!(elsewhere.menu_icon_kind(recent), Some(group_icon));
    // 「打开最近」面板同样显示组名，按组名和目录本身的名字都能找到。
    for (query, marked) in [("数据", true), ("folders-main", false)] {
        elsewhere.open_recent_folders();
        elsewhere.overlays.picker.input.set_text(query);
        elsewhere.picker_refresh();
        assert_eq!(elsewhere.overlays.picker.hits, [index], "{query}");
        assert_eq!(!elsewhere.folder_matched(0).0.is_empty(), marked, "{query}");
        elsewhere.close_picker();
    }
    // 项目菜单里这一组是子菜单：第一项打开这一组，其后列出其中的各个目录。
    paint(&mut elsewhere);
    elsewhere.open_project_menu();
    let menu = elsewhere.overlays.context_menu.as_ref().unwrap();
    let group = (menu.entries.iter()).find_map(|entry| match entry {
        MenuEntry::Submenu(title, items) if title == "数据平台" => Some(items.clone()),
        _ => None,
    });
    let group = group.expect("一组目录在项目菜单中是子菜单");
    assert_eq!(
        group,
        [
            MenuEntry::Item(MenuCommand::OpenGroup(index), true),
            MenuEntry::Separator,
            MenuEntry::Item(MenuCommand::GroupFolder(index, 0), true),
            MenuEntry::Item(MenuCommand::GroupFolder(index, 1), true),
        ]
    );
    let inner = MenuCommand::GroupFolder(index, 1);
    assert_eq!(elsewhere.menu_label(inner).title, "folders-other");
    assert_eq!(
        elsewhere.group_folders(index),
        [main.clone(), other.clone()]
    );
    // 这一行和单个目录一样带图标，用的是目录组的图标。
    let popup = elsewhere.popup().unwrap();
    let icon = (popup.levels[0].rows.iter()).find_map(|row| match row {
        PopupRow::Submenu { label, icon, .. } if label.title == "数据平台" => Some(*icon),
        _ => None,
    });
    assert_eq!(icon, Some(Some(MenuIcon::Folders)));
    // 展开子菜单再截图。
    paint(&mut elsewhere);
    let row = elsewhere
        .popup()
        .and_then(|popup| popup.levels[0].row_rects.first().copied());
    if let Some(row) = row {
        elsewhere.on_mouse_move(row.x + 30.0, row.y + row.h / 2.0);
    }
    paint(&mut elsewhere);
    shoot(&mut elsewhere, "folders-recent");
    elsewhere.overlays.context_menu = None;

    // 子菜单里点打开的目录：单独打开它，不带其他目录；这一组仍记在会话里。
    elsewhere.open_group_folder(index, 0);
    assert_eq!(elsewhere.editor.root(), Some(main.as_path()));
    assert!(elsewhere.extra_folders().is_empty() && elsewhere.disk.solo);
    assert_eq!(
        elsewhere.editor.workspace.as_ref().unwrap().name,
        "folders-main"
    );
    elsewhere.remember_session();
    let kept = elsewhere.state.session_for(&main).unwrap();
    assert_eq!(kept.folders, std::slice::from_ref(&other));
    assert_eq!(kept.name.as_deref(), Some("数据平台"));
    // 这时项目菜单里可以就地恢复这一组。
    let here = (elsewhere.state.recent.iter())
        .position(|folder| *folder == main)
        .unwrap();
    assert!(project(&mut elsewhere).contains(&MenuCommand::OpenGroup(here)));
    elsewhere.open_group(here);
    assert_eq!(elsewhere.extra_folders(), std::slice::from_ref(&other));
    assert!(!elsewhere.disk.solo);
    assert_eq!(
        elsewhere.editor.workspace.as_ref().unwrap().name,
        "数据平台"
    );
    // 这一组开着时点其中打开的目录：移除其他目录，这一组仍然保留。
    elsewhere.open_group_folder(here, 0);
    assert!(elsewhere.extra_folders().is_empty() && elsewhere.disk.solo);
    assert_eq!(
        elsewhere.state.session_for(&main).unwrap().folders,
        std::slice::from_ref(&other)
    );
    // 单独打开着时再添加目录：先恢复这一组，再加入新的。
    let extra = main.with_file_name("folders-extra");
    std::fs::create_dir_all(&extra).unwrap();
    let extra = winc_config::files::canonicalize(&extra).unwrap();
    elsewhere.add_folder(&extra);
    assert_eq!(elsewhere.extra_folders(), [other.clone(), extra.clone()]);
    let _ = std::fs::remove_dir_all(&extra);
    again.rename_group("");
    again.remember_session();
    assert_eq!(again.recent_title(&main), "folders-main +1");
    assert_eq!(
        again.editor.workspace.as_ref().unwrap().name,
        "folders-main"
    );

    // 上次附加的目录已不存在：不恢复，说明是哪一个；这一组随之不再成组。
    let gone = main.with_file_name("folders-gone");
    std::fs::create_dir_all(&gone).unwrap();
    let gone = winc_config::files::canonicalize(&gone).unwrap();
    again.add_folder(&gone);
    again.remove_folder(&other);
    assert_eq!(again.extra_folders(), std::slice::from_ref(&gone));
    let state = again.state.clone();
    std::fs::remove_dir_all(&gone).unwrap();
    let mut later = blank_app(false);
    later.state = state;
    later.open_folder_now(&main);
    assert!(later.extra_folders().is_empty());
    let notice = later.notice.text.take().unwrap_or_default();
    assert!(notice.contains("folders-gone"), "{notice}");

    // 移除：只从窗口中移除，磁盘上的内容不动，文件树回到只列打开的目录。
    app.run_tree_command(MenuCommand::RemoveFolder, Some(other.clone()));
    paint(&mut app);
    assert!(app.extra_folders().is_empty() && other.join("src/main.rs").is_file());
    // 名字属于这一组：只剩打开的目录时取消，菜单里不再有重命名。
    assert_eq!(app.editor.workspace.as_ref().unwrap().name, "folders-main");
    assert!(!project(&mut app).contains(&MenuCommand::RenameGroup));
    assert!(app.recent_group(&main).is_none());
    assert_eq!(rows(&app), single);
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&other);
}

/// 附加的目录参与查找：快速打开和全局搜索列出其中的文件（路径以目录名开头），其中的仓库进入
/// 仓库列表；移除后不再列出。
#[test]
fn extra_folders_take_part_in_quick_open_search_and_git() {
    let main = winc_config::files::canonicalize(sample_folder("folders-find-main")).unwrap();
    let other = winc_config::files::canonicalize(search_folder("folders-find-other")).unwrap();
    if !git_in_quiet(&other, &["init", "-q"]) {
        return;
    }
    let mut app = app_with(&main.join("short.rs"), false);
    app.config.git_enabled = Some(true);
    paint(&mut app);
    app.add_folder(&other);

    // 快速打开：按附加目录中的文件名能找到，打开的是那个目录里的文件。
    app.open_picker();
    app.overlays.picker.input.set_text("config.rs");
    app.picker_refresh();
    let picker = &app.overlays.picker;
    let hit = picker.index.file(picker.hits[0]).unwrap().path.to_owned();
    assert_eq!(hit, "folders-find-other/src/config.rs");
    let absolute = picker.index.absolute(picker.hits[0]).unwrap();
    assert_eq!(absolute, other.join("src/config.rs"));
    app.close_picker();

    // 全局搜索：结果里有附加目录中的文件，相对路径以目录名开头。
    app.set_sidebar_view(SidebarView::Search);
    app.panels.global.query.insert("load_config");
    app.run_global_search();
    finish_search(&mut app);
    let found: Vec<&str> = (app.panels.global.engine.results.iter())
        .map(|file| file.relative.as_str())
        .collect();
    let inside = |path: &&str| path.starts_with("folders-find-other/src/");
    assert!(found.iter().any(inside), "{found:?}");

    // Git：附加目录里的仓库进入仓库列表，名字是目录名。
    app.git_discover();
    let at = app.git.repositories.iter().position(|repo| *repo == other);
    let at = at.expect("附加目录里的仓库被发现");
    assert_eq!(app.git_repository_name(at), "folders-find-other");

    // 移除后三处都不再包含它；正在看的仓库是它时换走。
    app.git_set_root(Some(other.clone()));
    app.remove_folder(&other);
    assert!(!app.git.repositories.contains(&other));
    assert_ne!(app.git.root.as_deref(), Some(other.as_path()));
    app.open_picker();
    app.overlays.picker.input.set_text("config.rs");
    app.picker_refresh();
    assert!(app.overlays.picker.hits.is_empty());
    app.close_picker();
    app.run_global_search();
    finish_search(&mut app);
    assert_eq!(app.panels.global.engine.total(), 0);
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&other);
}

/// 在 `dir` 里运行 `git`，返回是否成功；没有 `git` 时返回 `false`。
fn git_in_quiet(dir: &Path, args: &[&str]) -> bool {
    #[allow(clippy::disallowed_methods, reason = "测试中直接运行本机的 git")]
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status();
    status.is_ok_and(|status| status.success())
}

/// 把目录拖到编辑区：按设置添加到窗口或打开它；已在窗口中的目录和文件直接打开。
#[test]
fn a_dropped_folder_is_added_or_opened_as_configured() {
    use winc_config::FolderDrop;

    let main = winc_config::files::canonicalize(sample_folder("folders-drop-main")).unwrap();
    let other = winc_config::files::canonicalize(search_folder("folders-drop-other")).unwrap();
    let third = winc_config::files::canonicalize(search_folder("folders-drop-third")).unwrap();
    let mut app = app_with(&main.join("short.rs"), false);
    paint(&mut app);
    let editor = app.layout.editor;
    let (x, y) = (editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);

    // 设为添加：拖入的目录成为附加目录，打开的目录不变。
    app.config.drop_folder = Some(FolderDrop::Add);
    app.drop_file_at(&other, x, y);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&other));
    assert_eq!(app.editor.root(), Some(main.as_path()));

    // 已在窗口中的目录不能再添加：和以前一样打开它。文件始终是打开。
    app.drop_file_at(&other.join("src/main.rs"), x, y);
    assert_eq!(app.editor.root(), Some(main.as_path()));
    assert_eq!(app.editor.active().unwrap().name(), "main.rs");

    // 询问中选择添加并记住：写入设置，之后不再询问。
    app.config.drop_folder = Some(FolderDrop::Ask);
    app.folder_drop_chosen(&third, None, true, true);
    assert_eq!(app.extra_folders(), [other.clone(), third.clone()]);
    assert_eq!(app.config.drop_folder, Some(FolderDrop::Add));

    // 拖到文件树上同样按设置处理：设为添加时成为附加目录，不复制进去。
    let fourth = winc_config::files::canonicalize(search_folder("folders-drop-fourth")).unwrap();
    let tree = app.layout.tree;
    app.drop_file_at(&fourth, tree.x + 20.0, tree.bottom() - 6.0);
    assert!(app.extra_folders().contains(&fourth));
    assert!(!main.join("folders-drop-fourth").exists());
    app.remove_folder(&fourth);
    // 询问中选择复制：复制进落点所在的目录，不成为附加目录。
    app.folder_drop_chosen(&fourth, Some(main.clone()), false, false);
    assert!(main.join("folders-drop-fourth/src/main.rs").is_file());
    assert!(!app.extra_folders().contains(&fourth));
    let _ = std::fs::remove_dir_all(&fourth);

    // 设为不添加：拖到编辑区的目录替换打开的目录。
    app.remove_folder(&third);
    app.config.drop_folder = Some(FolderDrop::Open);
    app.drop_file_at(&third, x, y);
    assert_eq!(app.editor.root(), Some(third.as_path()));
    assert!(app.extra_folders().is_empty());

    // 设置页的文件一页列出这一项，菜单里打勾的是当前的取值。
    assert_eq!(Setting::DropFolder.page(), SettingsPage::Files);
    assert_eq!(
        crate::layout::drop_folder_label(FolderDrop::Open),
        winc_i18n::t!("settings-drop-folder-open")
    );
    for dir in [&main, &other, &third] {
        let _ = std::fs::remove_dir_all(dir);
    }
}
