//! 写提交信息时的辅助的测试：提交过的信息、首行提示、提交前检查 TODO、共同作者。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

/// 最近一次提交的完整信息。
fn last_message(root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "-1", "--format=%B"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// 勾上全部更改并写好提交信息 `message`。
fn ready_to_commit(app: &mut App, message: &str) {
    let entries = app.git.status.iter().flat_map(|s| &s.entries);
    let paths: Vec<PathBuf> = entries.map(|e| e.path.clone()).collect();
    app.set_commit_checked(&paths, true);
    app.git.commit.message.set_text(message);
}

/// 提交过的信息记在状态文件里：换仓库（提交视图重建）后在空的信息框里按 `↑` 翻到它，提交按钮
/// 的菜单里也列着它，选了填回信息框。
#[test]
fn commit_messages_are_remembered_and_offered_again() {
    let Some((mut app, root)) = git_app("git-message-history", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    ready_to_commit(&mut app, "修复登录\n\n说明原因");
    app.git_commit();
    app.poll_commit();
    assert_eq!(app.state.commit_messages, ["修复登录\n\n说明原因"]);

    // 提交视图重建后输入历史是空的；按 `↑` 时从状态文件里取回。
    app.git.commit = Default::default();
    app.focus = Focus::CommitMessage;
    assert!(app.commit_message_key(&Key::Named(NamedKey::ArrowUp), false, false));
    assert_eq!(app.git.commit.message.text(), "修复登录\n\n说明原因");

    // 提交按钮的菜单：「最近的提交信息」里第一条的标题是首行，下一行小字是正文。
    app.git.commit.message.set_text("");
    let label = app
        .compose_menu_label(MenuCommand::CommitMessage(0))
        .unwrap();
    assert_eq!(label.title, "修复登录");
    assert_eq!(label.subtitle.as_deref(), Some("说明原因"));
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitMessage(0));
    assert_eq!(app.git.commit.message.text(), "修复登录\n\n说明原因");
    let _ = std::fs::remove_dir_all(&root);
}

/// 在终端里提交的信息也能翻到：仓库里自己写的提交信息补在本程序记住的后面，菜单里列出，空的
/// 信息框里按 `↑` 翻到；别人写的不在其中。
#[test]
fn commits_made_elsewhere_are_offered_too() {
    let Some((mut app, root)) = git_app("git-message-elsewhere", false) else {
        return;
    };
    git_in(
        &root,
        &["commit", "-q", "--allow-empty", "-m", "在终端里提交"],
    );
    let made = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "user.name=Alice",
            "-c",
            "user.email=alice@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "-q", "--allow-empty", "-m", "别人的提交"])
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(made);
    app.git_refresh();
    app.git_refresh();
    assert!(app.state.commit_messages.is_empty());
    assert_eq!(app.git.own_messages, ["在终端里提交", "first"]);
    let label = app
        .compose_menu_label(MenuCommand::CommitMessage(0))
        .unwrap();
    assert_eq!(label.title, "在终端里提交");
    app.run_action(Action::ShowCommit);
    app.focus = Focus::CommitMessage;
    assert!(app.commit_message_key(&Key::Named(NamedKey::ArrowUp), false, false));
    assert_eq!(app.git.commit.message.text(), "在终端里提交");
    let _ = std::fs::remove_dir_all(&root);
}

/// 首行过长、首行后没有空行时给出提示，不阻止提交。
#[test]
fn the_subject_line_gets_a_hint_when_it_breaks_the_convention() {
    let Some((mut app, root)) = git_app("git-message-hint", false) else {
        return;
    };
    app.git.commit.message.set_text("修复登录");
    assert_eq!(app.commit_message_hint(), None);
    // 按显示列计：40 个汉字占 80 列，超过 72；70 个英文字母不超过。
    app.git.commit.message.set_text(&"字".repeat(40));
    let hint = app.commit_message_hint().unwrap();
    assert!(hint.contains("80") && hint.contains("72"), "{hint}");
    app.git.commit.message.set_text(&"a".repeat(70));
    assert_eq!(app.commit_message_hint(), None);
    app.git.commit.message.set_text("修复登录\n说明原因");
    assert!(app.commit_message_hint().is_some());
    app.git.commit.message.set_text("修复登录\n\n说明原因");
    assert_eq!(app.commit_message_hint(), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// 开着提交前检查时，新增的内容里有 TODO 先确认：取消不提交，仍然提交才提交；没有 TODO 时
/// 直接提交。
#[test]
fn new_todos_are_confirmed_before_committing() {
    let Some((mut app, root)) = git_app("git-check-todo", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitCheckTodo);
    assert!(app.state.commit_check_todo);
    std::fs::write(root.join("src/main.rs"), "fn main() {} // TODO: 加日志\n").unwrap();
    app.git_refresh();
    ready_to_commit(&mut app, "带 TODO");
    app.git_commit();
    let alert = app.overlays.alert.as_ref().expect("有 TODO 时先确认");
    assert!(alert.message.contains('1'), "{}", alert.message);
    press_alert(&mut app, 1);
    app.poll_commit();
    assert_eq!(last_message(&root), "first");

    app.git_commit();
    press_alert(&mut app, 0);
    app.poll_commit();
    assert_eq!(last_message(&root), "带 TODO");

    // 没有新增 TODO：不问，直接提交。
    std::fs::write(root.join("src/main.rs"), "fn main() { run(); }\n").unwrap();
    app.git_refresh();
    ready_to_commit(&mut app, "没有 TODO");
    app.git_commit();
    app.poll_commit();
    assert!(app.overlays.alert.is_none());
    assert_eq!(last_message(&root), "没有 TODO");
    let _ = std::fs::remove_dir_all(&root);
}

/// 共同作者：从最近的提交里读出其他作者（不含自己），选了在信息末尾加一行附注；已有附注时
/// 接在那一段里，不重复加。
#[test]
fn co_authors_come_from_recent_commits() {
    let Some((mut app, root)) = git_app("git-co-author", false) else {
        return;
    };
    let made = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "user.name=Alice Chen",
            "-c",
            "user.email=alice@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "-q", "--allow-empty", "-m", "Alice 的提交"])
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(made);
    app.git_refresh();
    assert_eq!(app.git.authors, ["Alice Chen <alice@example.com>"]);
    let menu = app.commit_button_menu();
    let offered = menu.iter().any(|entry| match entry {
        MenuEntry::Submenu(_, items) => {
            items.contains(&MenuEntry::Item(MenuCommand::CommitCoAuthor(0), true))
        }
        _ => false,
    });
    assert!(offered);

    app.git.commit.message.set_text("修复登录");
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitCoAuthor(0));
    let added = "修复登录\n\nCo-authored-by: Alice Chen <alice@example.com>";
    assert_eq!(app.git.commit.message.text(), added);
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitCoAuthor(0));
    assert_eq!(app.git.commit.message.text(), added);
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交前格式化：开着时要提交的文件先用格式化工具整理再提交，勾选式和暂存视图都是；关着时
/// 照原样提交。
#[cfg(unix)]
#[test]
fn files_are_reformatted_before_committing() {
    let Some((mut app, root)) = git_app("git-reformat", false) else {
        return;
    };
    let committed = |path: &str| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", &format!("HEAD:{path}")])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // 用户设置里的命令不用询问；把小写换成大写，看得出整理过。
    let command = "tr a-z A-Z".to_owned();
    app.config
        .format_commands
        .insert("rust".to_owned(), command);
    app.run_action(Action::ShowCommit);
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitReformat);
    assert!(app.state.commit_reformat && app.menu_checked(MenuCommand::CommitReformat));

    ready_to_commit(&mut app, "勾选式");
    app.git_commit();
    app.poll_commit();
    assert_eq!(last_message(&root), "勾选式");
    assert_eq!(committed("src/main.rs"), "FN MAIN() {}\n");
    assert_eq!(committed("src/new.rs"), "\n");
    assert_eq!(
        std::fs::read_to_string(root.join("src/main.rs")).unwrap(),
        "FN MAIN() {}\n"
    );

    // 暂存视图：整个暂存的文件整理后重新暂存，提交的是整理过的内容。
    app.git.commit.set_staging(true);
    std::fs::write(root.join("src/main.rs"), "fn other() {}\n").unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    app.git_refresh();
    app.git.commit.message.set_text("暂存视图");
    app.git_commit();
    app.poll_commit();
    assert_eq!(last_message(&root), "暂存视图");
    assert_eq!(committed("src/main.rs"), "FN OTHER() {}\n");

    // 关掉以后照原样提交。
    app.run_commit_button_command(MenuCommand::CommitReformat);
    std::fs::write(root.join("src/main.rs"), "fn third() {}\n").unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    app.git_refresh();
    app.git.commit.message.set_text("原样");
    app.git_commit();
    app.poll_commit();
    assert_eq!(committed("src/main.rs"), "fn third() {}\n");
    let _ = std::fs::remove_dir_all(&root);
}

/// 「提交并同步」：确认后提交，再与上游同步：上游的新提交先拉下来，本地的提交再推上去。
#[test]
fn commit_and_sync_pulls_then_pushes() {
    let dir = sample_folder("git-commit-sync");
    let (work, other) = (dir.join("work"), dir.join("other"));
    let git = |at: &Path, args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let made = git(&dir, &["init", "-q", "--bare", "-b", "main", "bare.git"])
        && git(&dir, &["clone", "-q", "bare.git", "work"])
        && git(&work, &["commit", "-q", "--allow-empty", "-m", "first"])
        && git(&work, &["push", "-q", "-u", "origin", "main"])
        && git(&dir, &["clone", "-q", "bare.git", "other"])
        && git(&other, &["commit", "-q", "--allow-empty", "-m", "upstream"])
        && git(&other, &["push", "-q", "origin", "main"]);
    let configured = ["user.name=t", "user.email=t@t", "pull.rebase=true"]
        .iter()
        .all(|kv| {
            let (key, value) = kv.split_once('=').unwrap();
            git(&work, &["config", key, value])
        });
    if !made || !configured {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    std::fs::write(work.join("a.txt"), "a\n").unwrap();
    let mut app = app_with(&work, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    ready_to_commit(&mut app, "本地的提交");
    let menu = app.commit_button_menu();
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::CommitSync, true)));
    app.open_context_menu(MenuTarget::CommitButton, (0.0, 0.0));
    app.run_menu_command(MenuCommand::CommitSync);
    press_alert(&mut app, 0);
    app.poll_commit();
    app.poll_branch_ops();
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.join("bare.git"))
        .args(["log", "--format=%s", "main"])
        .output()
        .unwrap();
    let pushed: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(pushed, ["本地的提交", "upstream", "first"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：首行过长时按钮一行右侧的提示（`commit-hint`），以及提交按钮展开的菜单
/// （`commit-button-menu`），菜单里有最近的提交信息和共同作者。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_compose() {
    let Some((mut app, root)) = git_app("git-compose-shot", false) else {
        return;
    };
    let made = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "user.name=Alice Chen",
            "-c",
            "user.email=alice@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "-q", "--allow-empty", "-m", "Alice 的提交"])
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(made);
    app.update_state(|state| {
        state.remember_commit_message("日志刷新后保留选中的提交");
        state.remember_commit_message("修复登录页的焦点顺序\n\n按 Tab 时跳过隐藏的按钮");
    });
    app.git_refresh();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.layout.sidebar.w = 420.0;
    app.relayout();
    let long =
        "提交视图：把其他仓库的暂存内容和当前仓库一起提交，并在部分失败时写明哪些仓库已经提交";
    app.git.commit.message.set_text(long);
    paint(&mut app);
    shoot(&mut app, "commit-hint");
    app.git.commit.message.set_text("修复");
    paint(&mut app);
    let menu = app.commit_parts().commit_menu;
    app.open_context_menu(MenuTarget::CommitButton, (menu.x, menu.bottom()));
    shoot(&mut app, "commit-button-menu");
    let _ = std::fs::remove_dir_all(&root);
}
