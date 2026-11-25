//! 仓库列表的测试：嵌套的仓库、手动添加、从列表中移除，以及附加目录中的仓库。

use std::time::{Duration, Instant};

use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::app::lifecycle::notice::NoticeKind;

use super::*;

/// 在 `dir` 下建一个只有 `src/main.rs` 的仓库，返回它经过符号链接解析后的路径。
fn repo_at(dir: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    make_repo(dir).then(|| winc_config::files::canonicalize(dir).unwrap())
}

/// 打开 `dir` 并开启 Git，只查找一层子目录；git 不可用时返回 `None`。
fn open_shallow(dir: &Path) -> Option<App> {
    let mut app = app_with(dir, false);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return None;
    }
    settle(&mut app, |app| !app.git_scanning());
    Some(app)
}

/// 收取后台结果，直到 `done` 成立或超时。
fn settle(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll_git();
        if done(app) || Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 在「添加仓库」对话框中输入 `text` 并提交，收下查找的结果。
fn add_by_hand(app: &mut App, text: &str) {
    app.open_add_repository_dialog();
    let form = app
        .overlays
        .form_dialog
        .as_mut()
        .expect("打开了添加仓库对话框");
    assert_eq!(form.kind, FormKind::AddRepository);
    form.name.set_text(text);
    app.submit_form_dialog();
    assert!(app.overlays.form_dialog.is_none());
    app.poll_git();
}

#[test]
fn nested_repositories_join_the_list_and_are_not_the_parent_s_changes() {
    let dir = search_folder("git-repo-list-nested");
    let Some(outer) = repo_at(&dir) else {
        return;
    };
    // 外层仓库提交以后才建的内层仓库：外层只把它看作一个未跟踪的目录。
    let Some(inner) = repo_at(&dir.join("libs/inner")) else {
        return;
    };
    let Some(mut app) = open_shallow(&outer) else {
        return;
    };
    // 只查找一层时找不到 `libs/inner`：它由读外层仓库的状态时发现。
    settle(&mut app, |app| app.git.repositories.contains(&inner));
    assert_eq!(app.git.repositories, [outer.clone(), inner.clone()]);
    assert_eq!(app.git.root.as_ref(), Some(&outer));
    let status = app.git.status.as_ref().unwrap();
    assert_eq!(status.nested, std::slice::from_ref(&inner));
    let entries = status.entries.iter();
    assert!(entries.clone().all(|e| !e.path.starts_with(&inner)));
    assert!(entries.clone().any(|e| e.path.ends_with("src/new.rs")));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_repository_added_by_hand_is_kept_for_the_folder() {
    let dir = search_folder("git-repo-list-add");
    let Some(repo) = repo_at(&dir.join("vendor/x/y/lib")) else {
        return;
    };
    let Some(mut app) = open_shallow(&dir) else {
        return;
    };
    assert!(app.git.repositories.is_empty());

    // 不在仓库中的目录：说明原因，列表不变。
    add_by_hand(&mut app, "vendor");
    assert_eq!(app.notice.kind, NoticeKind::Error);
    let text = app.notice.text.take().unwrap_or_default();
    assert!(text.contains("vendor"), "{text}");
    assert!(app.git.repositories.is_empty());

    // 仓库中的任意目录都可以：加入的是仓库的根目录，路径中的 `.`、`..` 消去，并切换到它。
    add_by_hand(&mut app, "./vendor/x/../x/y/lib/src/.");
    assert_eq!(app.git.repositories, std::slice::from_ref(&repo));
    assert_eq!(app.git.root.as_ref(), Some(&repo));
    let session = app.current_session().expect("记在目录的现场里");
    assert_eq!(session.repositories.added, std::slice::from_ref(&repo));

    // 同一目录的另一个窗口记下的调整：本窗口保存现场时不覆盖。
    let elsewhere = dir.join("elsewhere");
    let key = app.session_key().unwrap();
    let other = app.state.sessions.iter_mut().find(|s| s.folder == key);
    other.unwrap().repositories.removed.push(elsewhere.clone());
    app.remember_session();
    let kept = &app.current_session().unwrap().repositories;
    assert_eq!(kept.added, std::slice::from_ref(&repo));
    assert_eq!(kept.removed, [elsewhere]);

    // 再次打开目录：查找不到它，仍按记下的列出。
    app.git_folder_changed();
    settle(&mut app, |app| !app.git_scanning());
    assert_eq!(app.git.repositories, std::slice::from_ref(&repo));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_repository_removed_from_the_list_stays_out_until_added_again() {
    let dir = search_folder("git-repo-list-forget");
    let (Some(a), Some(b)) = (repo_at(&dir.join("a")), repo_at(&dir.join("b"))) else {
        return;
    };
    let Some(mut app) = open_shallow(&dir) else {
        return;
    };
    settle(&mut app, |app| app.git.repositories.len() == 2);
    app.git_set_root(Some(b.clone()));

    // 仓库选择器中每个仓库的菜单里有「从列表中移除」；移除当前仓库时换到第一个。
    app.open_places(true);
    app.open_context_menu(MenuTarget::Location(b.clone(), None), (0.0, 0.0));
    assert!(menu_commands(&app).contains(&(MenuCommand::LocationForget, true)));
    app.run_menu_command(MenuCommand::LocationForget);
    assert!(!app.overlays.picker.open);
    assert_eq!(app.git.repositories, std::slice::from_ref(&a));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    let removed = |app: &App| app.current_session().unwrap().repositories.removed.clone();
    assert_eq!(removed(&app), std::slice::from_ref(&b));

    // 只剩一个仓库时不能再移除。
    app.open_places(true);
    app.open_context_menu(MenuTarget::Location(a.clone(), None), (0.0, 0.0));
    assert!(menu_commands(&app).contains(&(MenuCommand::LocationForget, false)));
    app.overlays.context_menu = None;
    app.close_picker();

    // 再次查找、打开其中的文件都不会让它回到列表。
    app.git_folder_changed();
    settle(&mut app, |app| !app.git_scanning());
    assert_eq!(app.git.repositories, std::slice::from_ref(&a));
    app.open_path(&b.join("src/main.rs"));
    app.git_follow_active();
    assert_eq!(app.git.repositories, std::slice::from_ref(&a));

    // 手动添加后恢复，不再记为移除。
    add_by_hand(&mut app, "b");
    assert_eq!(app.git.repositories, [a.clone(), b.clone()]);
    assert!(removed(&app).is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn opening_a_file_in_an_attached_folder_adds_its_repository() {
    let dir = search_folder("git-repo-list-attached");
    let other = winc_config::files::canonicalize(search_folder("git-repo-list-other")).unwrap();
    let Some(repo) = repo_at(&other.join("pkg/deep/lib")) else {
        return;
    };
    let Some(mut app) = open_shallow(&dir) else {
        return;
    };
    app.add_folder(&other);
    assert!(!app.git.repositories.contains(&repo));

    // 附加目录里层数太深、没有查找到的仓库：打开其中的文件时加入并切换到它。
    app.open_path(&repo.join("src/main.rs"));
    app.git_follow_active();
    assert!(app.git.repositories.contains(&repo));
    assert_eq!(app.git.root.as_ref(), Some(&repo));

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&other).unwrap();
}
