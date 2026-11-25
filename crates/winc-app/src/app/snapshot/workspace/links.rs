//! 经符号链接打开的目录：工作区、标签、现场与最近列表、附加的目录、Git 都按链接下的路径记录，
//! 判断是否为同一个目录时按真实路径。

use super::*;

/// 建一个测试目录和指向它的符号链接 `<name>-link`，返回（真实路径，链接）。
fn linked_folder(name: &str) -> (PathBuf, PathBuf) {
    let real = sample_folder(name);
    let link = real.with_file_name(format!("{name}-link"));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();
    (real, link)
}

/// 删除测试目录和指向它的链接。
fn remove(real: &Path, link: &Path) {
    let _ = std::fs::remove_file(link);
    let _ = std::fs::remove_dir_all(real);
}

#[test]
fn a_folder_opened_through_a_link_keeps_the_link_path() {
    let (real, link) = linked_folder("links-open");
    let mut app = app_with(&link, false);
    paint(&mut app);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(app.editor.root(), Some(link.as_path()));
    assert_eq!(ws.name, "links-open-link");
    assert!(ws.rows().iter().all(|row| row.path.starts_with(&link)));
    assert_eq!(app.session_key().as_deref(), Some(link.as_path()));
    assert_eq!(app.state.recent.first(), Some(&link));

    // 快速打开和全局搜索在链接下查找，结果是链接下的路径。
    app.open_picker();
    app.overlays.picker.input.set_text("main.rs");
    app.picker_refresh();
    let picker = &app.overlays.picker;
    let hit = picker.index.absolute(picker.hits[0]).unwrap();
    assert_eq!(hit, link.join("src/main.rs"));
    app.close_picker();
    app.set_sidebar_view(SidebarView::Search);
    app.panels.global.query.insert("hello");
    app.run_global_search();
    finish_search(&mut app);
    let results = &app.panels.global.engine.results;
    assert!(!results.is_empty());
    assert!(results.iter().all(|file| file.path.starts_with(&link)));

    // 再从真实路径打开同一个目录：认作已打开的目录，不重新打开。
    app.switch_folder(&real);
    assert_eq!(app.editor.root(), Some(link.as_path()));
    remove(&real, &link);
}

#[test]
fn a_file_reached_through_either_path_is_one_tab_under_the_link() {
    let (real, link) = linked_folder("links-tabs");
    let mut app = app_with(&link, false);
    let file = link.join("src/main.rs");
    app.open_path(&file);
    let tabs = app.editor.documents.len();
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(file.as_path())
    );

    // 外部给出的真实路径（命令行、拖入）换回链接下的写法，切到已有的标签。
    app.open_path(&real.join("src/main.rs"));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(file.as_path())
    );
    assert_eq!(
        app.editor.document_path(&real.join("src")),
        link.join("src")
    );
    remove(&real, &link);
}

#[test]
fn records_kept_under_the_real_path_move_to_the_link() {
    let (real, link) = linked_folder("links-records");
    // 以前按真实路径打开时记下的现场和最近列表。
    let mut app = app_with(&real, false);
    app.open_path(&real.join("src/main.rs"));
    app.remember_session();
    let state = app.state.clone();
    assert_eq!(state.recent.first(), Some(&real));

    // 改为经链接打开：现场、最近列表中的位置改到链接下，标签按链接下的路径恢复。
    let mut app = blank_app(false);
    app.state = state;
    app.open_folder_now(&link);
    let file = link.join("src/main.rs");
    let paths: Vec<_> = (app.editor.documents.iter())
        .filter_map(|d| d.path.clone())
        .collect();
    assert_eq!(paths, [file]);
    assert_eq!(app.state.recent.first(), Some(&link));
    assert!(!app.state.recent.contains(&real));
    assert!(app.state.session_for(&real).is_none());
    remove(&real, &link);
}

#[test]
fn a_link_and_its_target_are_one_attached_folder() {
    let main = sample_folder("links-main");
    let (real, link) = linked_folder("links-extra");
    let mut app = app_with(&main, false);
    app.add_folder(&link);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&link));
    let ws = app.editor.workspace.as_ref().unwrap();
    // 链接和它指向的目录是同一个目录，不能再加一次。
    assert!(!ws.can_add_folder(&real));
    assert!(!ws.can_add_folder(&link.join("src")));
    // 附加的目录中的文件同样按链接下的路径打开。
    app.open_path(&real.join("src/main.rs"));
    let opened = app.editor.active().unwrap().path.clone();
    assert_eq!(opened, Some(link.join("src/main.rs")));
    remove(&real, &link);
    let _ = std::fs::remove_dir_all(&main);
}

#[test]
fn git_works_under_the_link() {
    let (real, link) = linked_folder("links-git");
    if !super::super::git::make_repo(&real) {
        return remove(&real, &link);
    }
    let mut app = app_with(&link, false);
    app.settings_toggle(Setting::GitModule);
    if super::super::git::git_unusable(&app) {
        return remove(&real, &link);
    }
    app.git_refresh();
    app.poll_git();
    assert_eq!(app.git.repositories, std::slice::from_ref(&link));
    assert_eq!(app.git.root.as_deref(), Some(link.as_path()));
    let status = app.git.status.as_ref().unwrap();
    assert!(!status.entries.is_empty());
    assert!(status.entries.iter().all(|e| e.path.starts_with(&link)));
    // 有附属工作树时列出各工作树：当前的一项按链接下的路径认出，链接之外的按真实路径。
    let side = real.with_file_name("links-git-side");
    let _ = std::fs::remove_dir_all(&side);
    let side_arg = side.to_string_lossy().into_owned();
    super::super::git::git_in(&real, &["worktree", "add", "-q", "-b", "side", &side_arg]);
    app.git_refresh();
    app.poll_git();
    let trees: Vec<_> = app.git.worktrees.iter().map(|w| w.path.clone()).collect();
    assert_eq!(trees, [link.clone(), side.clone()]);
    assert_eq!(app.git.current_worktree(), Some(0));
    let _ = std::fs::remove_dir_all(&side);
    remove(&real, &link);
}

/// 未保存的内容按链接下的路径备份，下次启动时恢复到链接下的同一个标签。
#[test]
fn unsaved_text_comes_back_under_the_link() {
    let (real, link) = linked_folder("links-backup");
    let backups = super::session::backup_root("links-backup");
    let file = link.join("src/main.rs");
    let mut app = app_with(&link, false);
    app.backup.root = Some(backups.clone());
    app.open_path(&file);
    app.run(Command::Insert("// 还没存\n".into()));
    app.request_quit();
    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    assert_eq!(session.folder, link);
    drop(app);

    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    let restored = crate::app::restore_at_launch(&mut editor, Some(&session), Some(&backups));
    let mut app = App::new(editor, None, Bench::from_env(), Config::default());
    app.backup.root = Some(backups.clone());
    app.adopt_restored(restored);
    let docs: Vec<_> = (app.editor.documents.iter())
        .filter(|doc| doc.path.is_some())
        .collect();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].path.as_deref(), Some(file.as_path()));
    assert!(docs[0].is_dirty());
    let text = docs[0].text().rope().unwrap().to_string();
    assert!(text.starts_with("// 还没存"), "{text}");
    let _ = std::fs::remove_dir_all(&backups);
    remove(&real, &link);
}
