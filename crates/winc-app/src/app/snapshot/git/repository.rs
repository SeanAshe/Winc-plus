//! 仓库和远程管理的测试：分支重命名、变基拉取、标签的推送和删除、初始化、克隆和远程仓库。

use winc_editor::git::Remote;

use crate::app::git::branch::{BranchOp, RemoteOp, TagOp};
use crate::app::git::form::FormKind;
use crate::app::input::context_menu::MenuCommand;
use crate::layout::DialogPart;

use super::*;

/// 打开 `dir` 并启用 Git 模块；git 不可用时返回 `None`。
fn git_app(dir: &Path) -> Option<App> {
    let mut app = app_with(dir, false);
    app.settings_toggle(Setting::GitModule);
    (!git_unusable(&app)).then_some(app)
}

/// 以 `dir/bare.git` 为远程，在 `dir/work` 中建立带上游的仓库，返回工作目录。
fn repo_with_remote(dir: &Path) -> Option<PathBuf> {
    let bare = dir.join("bare.git");
    let work = dir.join("work");
    let run = |at: &Path, args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    std::fs::create_dir_all(&work).unwrap();
    let ready = run(dir, &["init", "-q", "--bare", "-b", "main", "bare.git"])
        && run(&work, &["init", "-q", "-b", "main"])
        && run(&work, &["commit", "-q", "--allow-empty", "-m", "first"])
        && run(&work, &["remote", "add", "origin", bare.to_str().unwrap()])
        && run(&work, &["push", "-q", "-u", "origin", "main"]);
    if !ready {
        skip_without_git("无法用 git 建测试仓库");
        return None;
    }
    Some(work)
}

fn output_of(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output();
    String::from_utf8(output.unwrap().stdout).unwrap()
}

#[test]
fn a_branch_can_be_renamed_including_the_current_one() {
    let dir = search_folder("git-rename");
    if !make_repo(&dir) {
        return;
    }
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    let root = app.git.root.clone().unwrap();
    let branch = (app.git.branches.iter())
        .find(|b| b.current)
        .cloned()
        .unwrap();
    app.run_branch_op(BranchOp::Rename, branch.clone());
    assert_eq!(app.focus, Focus::Prompt);
    assert_eq!(
        app.bars.goto_input.as_ref().unwrap().selected_text(),
        branch.name
    );
    app.bars.goto_input.as_mut().unwrap().set_text("renamed");
    app.submit_prompt();
    let head = output_of(&root, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(head.trim(), "renamed");

    // 名字不变和名字为空时不做任何操作。
    app.git_rename_branch("renamed".into(), "renamed");
    app.git_rename_branch("renamed".into(), " ");
    assert!(app.git.branch.running.is_none());
    assert_eq!(
        output_of(&root, &["symbolic-ref", "--short", "HEAD"]).trim(),
        "renamed"
    );
}

#[test]
fn pulling_with_rebase_keeps_local_commits_on_top() {
    let dir = search_folder("git-pull-rebase");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    // 另一份克隆向远程推送一个提交，本地另有一个提交。
    let other = dir.join("other");
    let author = [
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
    ];
    git_in(&dir, &["clone", "-q", "bare.git", "other"]);
    let commit = |at: &Path, message: &str| {
        let mut args = author.to_vec();
        args.extend(["commit", "-q", "--allow-empty", "-m", message]);
        git_in(at, &args);
    };
    commit(&other, "remote");
    git_in(&other, &["push", "-q"]);
    commit(&work, "local");

    let Some(mut app) = git_app(&work) else {
        return;
    };
    // `pull --rebase` 产生的提交需要身份，测试机不一定配置。
    git_in(&work, &["config", "user.name", "t"]);
    git_in(&work, &["config", "user.email", "t@t"]);
    app.run_branch_command(MenuCommand::GitPullRebase);
    let subjects = output_of(&work, &["log", "--format=%s"]);
    assert_eq!(
        subjects.lines().collect::<Vec<_>>(),
        ["local", "remote", "first"]
    );
}

#[test]
fn tags_can_be_pushed_and_deleted_locally_and_remotely() {
    let dir = search_folder("git-tags");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    git_in(&work, &["tag", "v1"]);
    let Some(mut app) = git_app(&work) else {
        return;
    };
    let bare = dir.join("bare.git");
    let tags = |at: &Path| output_of(at, &["tag", "--list"]);

    app.run_tag_op(TagOp::Push, "v1".into());
    assert_eq!(tags(&bare).trim(), "v1");

    // 从远程删除后本地保留，反之同样。没有窗口时确认直接通过。
    app.run_tag_op(TagOp::DeleteRemote, "v1".into());
    assert_eq!(tags(&bare).trim(), "");
    assert_eq!(tags(&work).trim(), "v1");

    app.run_tag_op(TagOp::Delete, "v1".into());
    assert_eq!(tags(&work).trim(), "");
}

#[test]
fn a_commit_menu_lists_its_tags() {
    use crate::app::input::context_menu::{MenuEntry, log_commit_menu};
    let menu = log_commit_menu(true, "main", &["v1", "v2"]);
    let submenus: Vec<&str> = (menu.iter())
        .filter_map(|entry| match entry {
            MenuEntry::Submenu(title, _) if title.contains('v') => Some(title.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(submenus.len(), 2);
}

#[test]
fn initializing_makes_the_open_folder_a_repository() {
    let dir = search_folder("git-init");
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    assert!(app.git.root.is_none());
    assert!(app.action_available(Action::GitInit));
    app.run_action(Action::GitInit);
    assert!(dir.join(".git").is_dir());
    assert!(app.git.root.is_some());
    assert!(!app.action_available(Action::GitInit));
    assert!(app.git.branch.running.is_none());
}

#[test]
fn cloning_checks_the_target_and_creates_the_repository() {
    let dir = search_folder("git-clone");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let bare = dir.join("bare.git");
    let target = dir.join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.open_clone_dialog();
    paint(&mut app);
    let plan = |app: &App| app.form_plan(app.overlays.form_dialog.as_ref().unwrap());
    assert!(!plan(&app).enabled);

    // 地址无法得到目录名、目标目录不为空时不能克隆。
    let url = bare.to_str().unwrap().to_owned();
    let form = app.overlays.form_dialog.as_mut().unwrap();
    form.folder = Some(target.clone());
    form.name.set_text("--upload-pack=x");
    assert!(plan(&app).problem.is_some());
    std::fs::create_dir(target.join("bare")).unwrap();
    std::fs::write(target.join("bare/file"), "").unwrap();
    app.overlays
        .form_dialog
        .as_mut()
        .unwrap()
        .name
        .set_text(&url);
    assert!(plan(&app).problem.is_some());
    std::fs::remove_dir_all(target.join("bare")).unwrap();

    assert!(plan(&app).enabled);
    assert_eq!(plan(&app).dir, Some(target.join("bare")));
    paint(&mut app);
    app.submit_form_dialog();
    assert!(target.join("bare/.git").is_dir());
    assert!(app.overlays.form_dialog.is_none());
}

#[test]
fn a_failed_clone_stays_in_the_dialog_with_the_reason() {
    let dir = search_folder("git-clone-fail");
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    app.open_clone_dialog();
    let form = app.overlays.form_dialog.as_mut().unwrap();
    form.folder = Some(dir.clone());
    form.name
        .set_text(dir.join("missing.git").to_str().unwrap());
    app.submit_form_dialog();
    let form = app.overlays.form_dialog.as_ref().unwrap();
    assert!(form.error.is_some());
    assert!(form.running.is_none());
    paint(&mut app);
}

#[test]
fn remotes_can_be_added_changed_and_removed() {
    let dir = search_folder("git-remotes");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let Some(mut app) = git_app(&work) else {
        return;
    };
    let names = |app: &App| -> Vec<Remote> { app.git.remote_list.clone() };
    assert_eq!(names(&app)[0].name, "origin");

    // 添加：名称和地址都要填，名称不能重复。
    app.run_branch_command(MenuCommand::GitAddRemote);
    paint(&mut app);
    let plan = |app: &App| app.form_plan(app.overlays.form_dialog.as_ref().unwrap());
    assert!(!plan(&app).enabled);
    let form = app.overlays.form_dialog.as_mut().unwrap();
    form.name.set_text("origin");
    form.second.set_text("https://example.com/a.git");
    assert!(plan(&app).problem.is_some());
    app.overlays
        .form_dialog
        .as_mut()
        .unwrap()
        .name
        .set_text("up");
    assert!(plan(&app).enabled);
    assert!(
        !app.overlays
            .form_dialog
            .as_ref()
            .unwrap()
            .second
            .is_masked()
    );
    paint(&mut app);
    app.submit_form_dialog();
    assert!(app.overlays.form_dialog.is_none());
    let list = names(&app);
    assert!(list.iter().any(|r| r.name == "up"), "{list:?}");

    // 修改地址：输入框里是现在的地址。
    let index = list.iter().position(|r| r.name == "up").unwrap();
    app.run_branch_command(MenuCommand::GitRemote(RemoteOp::Edit, index));
    let form = app.overlays.form_dialog.as_mut().unwrap();
    assert!(matches!(form.kind, FormKind::Remote { .. }));
    assert_eq!(form.focus, DialogPart::Branch);
    assert_eq!(form.name.text(), "https://example.com/a.git");
    form.name.set_text("https://example.com/b.git");
    paint(&mut app);
    app.submit_form_dialog();
    assert_eq!(
        output_of(&work, &["remote", "get-url", "up"]).trim(),
        "https://example.com/b.git"
    );

    // 移除。没有窗口时确认直接通过。
    let index = (names(&app).iter()).position(|r| r.name == "up").unwrap();
    app.run_branch_command(MenuCommand::GitRemote(RemoteOp::Remove, index));
    assert!(!output_of(&work, &["remote"]).contains("up"));
    assert!(names(&app).iter().all(|r| r.name != "up"));
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_repository_dialogs() {
    let dir = search_folder("git-repository-shots");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.open_clone_dialog();
    let form = app.overlays.form_dialog.as_mut().unwrap();
    form.name.set_text("https://github.com/DEEIX-AI/Winc.git");
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-clone");
    app.close_form_dialog();
    app.run_branch_command(MenuCommand::GitAddRemote);
    let form = app.overlays.form_dialog.as_mut().unwrap();
    form.name.set_text("upstream");
    form.second.set_text("https://github.com/DEEIX-AI/Winc.git");
    std::thread::sleep(std::time::Duration::from_millis(400));
    shoot(&mut app, "git-remote-add");
}
