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
    // 输入文字的输入条比跳转到行的宽，长分支名写得下。
    let editor = app.layout.editor;
    assert_eq!(app.prompt_bar().w, 480.0_f32.min(editor.w - 40.0));
    shoot(&mut app, "git-rename-branch");
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

/// 提交菜单列出指向它的分支、标签：分支不超过两个、标签只有一个时各自一个子菜单，多了收进
/// 「分支」「标签」一个子菜单。
#[test]
fn a_commit_menu_lists_its_branches_and_tags() {
    use crate::app::input::context_menu::{CommitActions, MenuEntry, log_commit_menu};
    let all = CommitActions {
        enabled: true,
        undo: true,
        push: true,
        on_head: None,
        child: false,
        parent: true,
        changes: true,
        in_log: true,
    };
    let titles = |menu: &[MenuEntry]| -> Vec<String> {
        (menu.iter())
            .filter_map(|entry| match entry {
                MenuEntry::Submenu(title, _) => Some(title.clone()),
                _ => None,
            })
            .filter(|title| title.contains("分支") || title.contains("标签"))
            .collect()
    };
    let refs = |names: &[&str]| -> Vec<(String, Vec<MenuEntry>)> {
        names
            .iter()
            .map(|n| ((*n).to_owned(), Vec::new()))
            .collect()
    };
    let menu = log_commit_menu(all, "main", refs(&["main", "dev"]), &["v1"]);
    assert_eq!(titles(&menu), ["分支「main」", "分支「dev」", "标签「v1」"]);
    let menu = log_commit_menu(all, "main", refs(&["a", "b", "c"]), &["v1", "v2"]);
    assert_eq!(titles(&menu), ["分支", "标签"]);
    let grouped = menu.iter().find_map(|entry| match entry {
        MenuEntry::Submenu(title, items) if title == "分支" => Some(items),
        _ => None,
    });
    assert_eq!(titles(grouped.unwrap()).len(), 0);
    assert_eq!(grouped.unwrap().len(), 3);

    // 分组顺序：复制与拣选在最前，新建分支、标签在分支子菜单之后，转到在最后。
    let items: Vec<MenuCommand> = (menu.iter())
        .filter_map(|entry| match entry {
            MenuEntry::Item(command, _) => Some(*command),
            _ => None,
        })
        .collect();
    assert_eq!(items.first(), Some(&MenuCommand::LogCopyOid));
    assert_eq!(items.last(), Some(&MenuCommand::LogGoToParent));
    let at = |c: MenuCommand| items.iter().position(|&i| i == c).unwrap();
    assert!(at(MenuCommand::LogCherryPick) < at(MenuCommand::LogCheckout));
    assert!(at(MenuCommand::LogRevert) < at(MenuCommand::LogUndoCommit));
    assert!(at(MenuCommand::LogUndoCommit) < at(MenuCommand::LogReword));
    assert!(at(MenuCommand::LogReword) < at(MenuCommand::LogDropCommit));
    assert!(at(MenuCommand::LogDropCommit) < at(MenuCommand::LogInteractiveRebase));
    assert!(at(MenuCommand::LogInteractiveRebase) < at(MenuCommand::LogPushUpTo));
    assert!(at(MenuCommand::LogPushUpTo) < at(MenuCommand::LogNewBranch));
    // 复制提交号旁显示日志中复制的快捷键。
    let copy = MenuCommand::LogCopyOid.shortcut(crate::keymap::Context::Editor);
    assert_eq!(
        copy,
        crate::keymap::single_stroke(Action::Copy, crate::keymap::Context::Editor)
    );
    assert!(copy.is_some());
}

/// 日志里右键一个提交：从指向它的分支的子菜单把这个分支合并到当前分支。
#[test]
fn a_branch_on_a_commit_merges_from_the_commit_menu() {
    use crate::app::git::branch::BranchOp;
    use crate::app::input::context_menu::MenuTarget;
    let dir = search_folder("git-log-branch-merge");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["switch", "-q", "-c", "dev"]);
    std::fs::write(dir.join("dev.txt"), "dev\n").unwrap();
    git_in(&dir, &["add", "dev.txt"]);
    git_in(&dir, &["commit", "-q", "-m", "dev 上的提交"]);
    git_in(&dir, &["switch", "-q", "main"]);
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    app.git_refresh();
    assert!(show_all_log(&mut app));
    paint(&mut app);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    let dev_tip = view
        .commits
        .iter()
        .find(|c| c.subject == "dev 上的提交")
        .unwrap();
    let oid = dev_tip.oid.clone();
    let row = app.log_parts().list;
    app.open_context_menu(MenuTarget::LogCommit(oid), (row.x + 200.0, row.y + 40.0));
    paint(&mut app);
    shoot(&mut app, "git-log-commit-branch-menu");
    let dev = (app.git.branches.iter())
        .position(|b| !b.remote && b.name == "dev")
        .unwrap();
    let merge = MenuCommand::GitBranch(BranchOp::Merge, dev);
    assert!(menu_commands(&app).contains(&(merge, true)));
    app.run_menu_command(merge);
    app.poll_branch_ops();
    let merged = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["merge-base", "--is-ancestor", "dev", "main"])
        .status()
        .unwrap();
    assert!(merged.success());
    let _ = std::fs::remove_dir_all(&dir);
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

/// 分支选择器的「标签」一组：平时收起，展开后列出各个标签；行尾的菜单可以签出、推送和删除，
/// 回车签出它指向的提交。
#[test]
fn the_branch_picker_lists_tags_and_checks_one_out() {
    use crate::app::GitAnchor;
    use crate::app::git::branch::picker::BranchRow;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};
    let dir = search_folder("git-picker-tags");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let first = output_of(&work, &["rev-parse", "HEAD"]);
    git_in(&work, &["tag", "v1"]);
    std::fs::write(work.join("more.txt"), "more\n").unwrap();
    git_in(&work, &["add", "more.txt"]);
    // 身份写在命令行上：测试仓库没有配置身份，不依赖本机的全局配置。
    let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];
    let commit = ["-c", "commit.gpgsign=false", "commit", "-q", "-m", "more"];
    git_in(&work, &[&identity[..], &commit[..]].concat());
    git_in(&work, &["tag", "v2"]);
    let Some(mut app) = git_app(&work) else {
        return;
    };
    let mut names = app.git.tags.clone();
    names.sort();
    assert_eq!(names, ["v1", "v2"]);

    // 最后一行是收起的「标签」；确认它即展开，各个标签列在其后。
    app.open_branch_picker(GitAnchor::Branch);
    let rows = |app: &App| app.overlays.picker.branch_rows.clone().unwrap();
    let group = rows(&app).len() - 1;
    let closed = BranchRow::Tags {
        count: 2,
        open: false,
    };
    assert_eq!(rows(&app)[group], closed);
    app.overlays.picker.selected = group;
    app.branch_picker_accept();
    let v1 = app.git.tags.iter().position(|tag| tag == "v1").unwrap();
    let row = (rows(&app).iter()).position(|row| *row == BranchRow::Tag(v1));
    let row = row.unwrap();

    // 行尾的菜单：签出，新建分支、合并、推送、复制名字、删除、从远程删除。
    app.overlays.picker.selected = row;
    app.open_branch_row_menu(MenuTarget::TagOps(v1));
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    let ops = TagOp::ALL.map(|op| MenuCommand::PickerTag(Some(op), v1));
    assert_eq!(commands[0], MenuCommand::PickerTag(None, v1));
    assert_eq!(commands[1..], ops);
    app.run_menu_command(MenuCommand::PickerTag(Some(TagOp::CopyName), v1));
    assert_eq!(app.clipboard_text().as_deref(), Some("v1"));
    assert_eq!(app.status.message.as_deref(), Some("已复制标签名「v1」"));
    app.open_branch_picker(GitAnchor::Branch);
    app.overlays.picker.selected = row;
    app.open_branch_row_menu(MenuTarget::TagOps(v1));
    app.run_menu_command(MenuCommand::PickerTag(Some(TagOp::Push), v1));
    assert_eq!(
        output_of(&dir.join("bare.git"), &["tag", "--list"]).trim(),
        "v1"
    );

    // 回车签出标签指向的提交（没有窗口时确认直接通过），选择器随之关闭。
    app.open_branch_picker(GitAnchor::Branch);
    app.overlays.picker.selected = row;
    app.branch_picker_accept();
    assert!(!app.overlays.picker.open);
    assert_eq!(output_of(&work, &["rev-parse", "HEAD"]), first);

    // 删除后列表随刷新更新。
    app.run_tag_op(TagOp::Delete, "v1".into());
    assert_eq!(app.git.tags, ["v2"]);
}

/// 打开的目录不是仓库时，提交视图给出「初始化仓库」和「克隆仓库…」；初始化以后按项目类型生成
/// `.gitignore`，开着的 Git 输出标签不用重开就看得到这条命令。
#[test]
fn a_folder_without_a_repository_offers_to_initialize_one() {
    let dir = search_folder("git-no-repository");
    std::fs::write(dir.join("Cargo.toml"), "[package]\n").unwrap();
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let dir = app.editor.root().unwrap().to_path_buf();
    app.run_action(Action::ShowCommit);
    // 查找仓库的结果由事件循环收取；测试里没有事件循环，这里收一次。
    app.git_take_scan();
    paint(&mut app);
    assert!(app.git_no_repository());
    let hit = |app: &App, target| app.hits.commit.iter().any(|(_, t)| *t == target);
    assert!(hit(&app, CommitTarget::InitRepository));
    assert!(hit(&app, CommitTarget::CloneRepository));

    // 先打开输出标签，再初始化：标签里随后出现这条命令，仍是同一个标签。
    app.show_git_output();
    let tabs = app.editor.documents.len();
    let output = |app: &App| {
        let title = winc_i18n::t!("git-output-title");
        let doc = (app.editor.documents.iter()).find(|d| d.title.as_deref() == Some(title));
        doc.unwrap().text().rope().unwrap().to_string()
    };
    let ours = format!("{}\n$ git init", dir.display());
    assert!(!output(&app).contains(&ours));
    app.press_commit(CommitTarget::InitRepository);
    app.refresh_git_output();
    assert!(output(&app).contains(&ours), "{}", output(&app));
    assert_eq!(app.editor.documents.len(), tabs);

    // 成了仓库：不再是空状态；`.gitignore` 里有 Rust 项目的构建目录，已有的文件不会被覆盖。
    assert_eq!(app.git.root.as_deref(), Some(dir.as_path()));
    assert!(!app.git_no_repository());
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|line| line == "/target/"), "{ignore}");
    std::fs::write(dir.join(".gitignore"), "mine\n").unwrap();
    app.git_init();
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "mine\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 推送对话框列出会送出的提交：最新的在前，超过上限的只列最新的几个并记下总数；远程和目标
/// 分支先填上游。
#[test]
fn the_push_confirmation_lists_the_commits_it_will_send() {
    use crate::app::git::commit::PushIntent;
    let dir = sample_folder("git-push-preview");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let commit = |subject: &str| {
        let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let made = Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(identity)
            .args(["-c", "commit.gpgsign=false"])
            .args(["commit", "-q", "--allow-empty", "-m", subject])
            .output();
        assert!(made.is_ok_and(|o| o.status.success()));
    };
    commit("修复登录");
    commit("加上重试");
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.git_refresh();
    app.ask_push(PushIntent::Push);
    let form = |app: &App| match &app.overlays.form_dialog.as_ref().unwrap().kind {
        FormKind::Transfer(form) => form.clone(),
        kind => panic!("{kind:?}"),
    };
    let shown = form(&app);
    assert_eq!(shown.subjects, ["加上重试", "修复登录"]);
    assert_eq!((shown.total, shown.lines()), (2, 2));
    assert_eq!(
        (shown.remote.as_str(), shown.target.as_str()),
        ("origin", "main")
    );
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.name.text(), "main");
    assert_eq!(dialog.focus, DialogPart::Confirm);
    app.close_form_dialog();

    // 提交很多时只列最新的几个，最后一行写明其余的个数，两者加起来是全部。
    for i in 0..20 {
        commit(&format!("第 {i} 个"));
    }
    app.git_refresh();
    app.ask_push(PushIntent::Push);
    let shown = form(&app);
    let listed = shown.subjects.len();
    assert!(listed > 0 && listed < 22, "{listed}");
    assert_eq!(shown.subjects[0], "第 19 个");
    assert_eq!((shown.total, shown.lines()), (22, listed + 1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 推送对话框里改了目标分支、勾上「推送标签」：推到远程的那个分支，标签一并送出，上游
/// 不变；完成后对话框关闭。目标分支名无效时不能推送。
#[test]
fn the_push_dialog_pushes_to_another_branch_with_tags() {
    use crate::app::git::commit::PushIntent;
    let dir = sample_folder("git-push-target");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let git = |at: &Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        (out.status.success()).then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // 远程上 `main` 有「已推送」，`old` 停在它之前。
    git(&work, &["commit", "-q", "--allow-empty", "-m", "已推送"]).unwrap();
    git(&work, &["push", "-q", "origin", "main"]).unwrap();
    git(&work, &["push", "-q", "origin", "HEAD~1:refs/heads/old"]).unwrap();
    git(&work, &["commit", "-q", "--allow-empty", "-m", "待评审"]).unwrap();
    git(&work, &["tag", "v1"]).unwrap();
    let head = git(&work, &["rev-parse", "HEAD"]).unwrap();
    let bare = dir.join("bare.git");
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.git_refresh();
    app.ask_push(PushIntent::Push);
    let subjects = |app: &App| match &app.overlays.form_dialog.as_ref().unwrap().kind {
        FormKind::Transfer(form) => (form.subjects.clone(), form.total),
        kind => panic!("{kind:?}"),
    };
    assert_eq!(subjects(&app), (vec!["待评审".to_owned()], 1));

    // 改了目标分支，提交列表按新目标重读：推到 `old` 要送出两个提交；推到远程还没有的
    // `review`，送出的是远程哪个分支都没有的那一个。
    let retarget = |app: &mut App, name: &str| {
        app.overlays
            .form_dialog
            .as_mut()
            .unwrap()
            .name
            .set_text(name);
        app.refresh_transfer_list();
    };
    retarget(&mut app, "old");
    let both = vec!["待评审".to_owned(), "已推送".to_owned()];
    assert_eq!(subjects(&app), (both, 2));
    retarget(&mut app, "review");
    assert_eq!(subjects(&app), (vec!["待评审".to_owned()], 1));

    let dialog = app.overlays.form_dialog.as_mut().unwrap();
    dialog.name.set_text("-review");
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert!(!app.form_plan(dialog).enabled);

    let dialog = app.overlays.form_dialog.as_mut().unwrap();
    dialog.name.set_text("review");
    dialog.copy = true;
    app.submit_form_dialog();
    app.poll_branch_ops();
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(
        git(&bare, &["rev-parse", "refs/heads/review"]),
        Some(head.clone())
    );
    assert_eq!(
        git(&bare, &["rev-parse", "refs/tags/v1"]),
        Some(head.clone())
    );
    // 原来的上游没有收到这个提交，跟踪关系也没有变。
    assert_ne!(git(&bare, &["rev-parse", "refs/heads/main"]), Some(head));
    let upstream = git(&work, &["rev-parse", "--abbrev-ref", "@{upstream}"]);
    assert_eq!(upstream.as_deref(), Some("origin/main"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 有两个远程时，「拉取自…」从选的远程分支拉取，可以合并或变基；「推送到…」在关了推送确认
/// 时仍打开推送对话框，推到选的远程。
#[test]
fn pull_from_and_push_to_choose_another_remote() {
    use crate::app::git::form::Direction;
    let dir = sample_folder("git-two-remotes");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let git = |at: &Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        (out.status.success()).then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // 第二个远程 `mirror` 上另有提交，由另一个克隆推上去。
    let mirror = dir.join("mirror.git");
    let mirror_url = mirror.to_str().unwrap();
    git(&dir, &["init", "-q", "--bare", "-b", "main", "mirror.git"]).unwrap();
    git(&work, &["remote", "add", "mirror", mirror_url]).unwrap();
    git(&work, &["push", "-q", "mirror", "main"]).unwrap();
    let other = dir.join("other");
    git(&dir, &["clone", "-q", mirror_url, "other"]).unwrap();
    let push_mirror = |subject: &str| {
        git(&other, &["commit", "-q", "--allow-empty", "-m", subject]).unwrap();
        git(&other, &["push", "-q", "origin", "main"]).unwrap();
        git(&work, &["fetch", "-q", "mirror"]).unwrap();
    };
    push_mirror("镜像上的提交");
    // 变基时由 Winc 运行的 `git` 写提交，需要仓库里配置的身份。
    git(&work, &["config", "user.name", "t"]).unwrap();
    git(&work, &["config", "user.email", "t@t"]).unwrap();
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.git_refresh();
    let form = |app: &App| match &app.overlays.form_dialog.as_ref().unwrap().kind {
        FormKind::Transfer(form) => form.as_ref().clone(),
        kind => panic!("{kind:?}"),
    };

    // 两项在「⋯」菜单的远程仓库子菜单最上面。
    use crate::app::input::context_menu::MenuEntry;
    let remotes = (app.commit_more_menu().into_iter())
        .find_map(|entry| match entry {
            MenuEntry::Submenu(title, items) if title == "远程仓库" => Some(items),
            _ => None,
        })
        .unwrap();
    let transfer = [Action::GitPullFrom, Action::GitPushTo]
        .map(|action| MenuEntry::Item(MenuCommand::GitAction(action), true));
    assert_eq!(remotes[..2], transfer);

    // 默认是上游 `origin/main`，没有新提交；换到 `mirror` 后列出那边的提交。
    app.run_action(Action::GitPullFrom);
    let shown = form(&app);
    assert_eq!(shown.direction, Direction::Pull);
    assert!(!app.overlays.form_dialog.as_ref().unwrap().copy);
    assert_eq!((shown.remote.as_str(), shown.total), ("origin", 0));
    app.choose_form_base(Some("mirror".into()));
    assert_eq!(form(&app).subjects, ["镜像上的提交"]);
    // 本地没有记下的远程分支不能拉取。
    let dialog = app.overlays.form_dialog.as_mut().unwrap();
    dialog.name.set_text("nope");
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert!(!app.form_plan(dialog).enabled);
    app.overlays
        .form_dialog
        .as_mut()
        .unwrap()
        .name
        .set_text("main");
    app.submit_form_dialog();
    app.poll_branch_ops();
    assert!(app.overlays.form_dialog.is_none());
    let subject = |at: &Path, rev: &str| git(at, &["log", "-1", "--format=%s", rev]);
    assert_eq!(subject(&work, "HEAD").as_deref(), Some("镜像上的提交"));

    // 两边各有新提交时勾选「变基」：本地提交放到拉取的提交之后。
    // 本地提交改了文件：空提交在变基时可能被丢掉。
    push_mirror("镜像第二个");
    std::fs::write(work.join("local.txt"), "本地\n").unwrap();
    git(&work, &["add", "local.txt"]).unwrap();
    git(&work, &["commit", "-q", "-m", "本地提交"]).unwrap();
    // 仓库配置为变基拉取：「变基」默认勾上。
    git(&work, &["config", "pull.rebase", "true"]).unwrap();
    app.git_refresh();
    app.run_action(Action::GitPullFrom);
    assert!(app.overlays.form_dialog.as_ref().unwrap().copy);
    app.choose_form_base(Some("mirror".into()));
    app.submit_form_dialog();
    app.poll_branch_ops();
    assert_eq!(subject(&work, "HEAD").as_deref(), Some("本地提交"));
    assert_eq!(subject(&work, "HEAD~1").as_deref(), Some("镜像第二个"));

    // 关了推送确认，「推送到…」仍打开对话框；推到 `mirror`，上游不变。
    app.config.git_confirm_push = Some(false);
    app.git_refresh();
    app.run_action(Action::GitPushTo);
    assert!(matches!(form(&app).direction, Direction::Push(_)));
    app.choose_form_base(Some("mirror".into()));
    app.submit_form_dialog();
    app.poll_branch_ops();
    let head = git(&work, &["rev-parse", "HEAD"]);
    assert_eq!(git(&mirror, &["rev-parse", "refs/heads/main"]), head);
    let upstream = git(&work, &["rev-parse", "--abbrev-ref", "@{upstream}"]);
    assert_eq!(upstream.as_deref(), Some("origin/main"));

    // 游离的 `HEAD` 上没有分支：两个对话框都不打开，说明原因。
    git(&work, &["switch", "-q", "--detach"]).unwrap();
    app.git_refresh();
    for action in [Action::GitPullFrom, Action::GitPushTo] {
        app.run_action(action);
        assert!(app.overlays.form_dialog.is_none(), "{action:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：打开的目录不是仓库时的提交视图（`commit-no-repository`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_no_repository() {
    let dir = search_folder("commit-no-repository");
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.git_take_scan();
    shoot(&mut app, "commit-no-repository");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：推送对话框，列出会送出的提交（`push-preview`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_push_preview() {
    let dir = sample_folder("push-preview");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    for subject in [
        "修复登录页的焦点顺序",
        "日志刷新后保留选中的提交",
        "补上推送前的预览",
    ] {
        let made = Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(["commit", "-q", "--allow-empty", "-m", subject])
            .output();
        assert!(made.is_ok_and(|o| o.status.success()));
    }
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.ask_push(crate::app::git::commit::PushIntent::Push);
    shoot(&mut app, "push-preview");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `git` 版本低于最低要求时提示一次，之后不再重复；版本足够或读不出时不提示。
#[test]
fn old_git_is_reported_once() {
    let dir = search_folder("git-old-version");
    let mut app = app_with(&dir, false);
    let minimum = winc_editor::git::MINIMUM_VERSION;
    app.warn_old_git(None);
    app.warn_old_git(Some(minimum));
    assert_eq!(app.notice.text, None);
    app.warn_old_git(Some((minimum.0, minimum.1 - 1)));
    let text = app.notice.text.take().expect("旧版本应有提示");
    assert!(text.contains("2.23") && text.contains("2.24"), "{text}");
    app.warn_old_git(Some((1, 9)));
    assert_eq!(app.notice.text, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 开启定时获取后第一个周期立即获取：远程的新提交取回本地，分支行上的落后数随之更新；
/// 下一次在一个周期之后。关闭时不安排获取。
#[test]
fn auto_fetch_brings_in_new_upstream_commits() {
    let dir = sample_folder("git-auto-fetch");
    let (bare, work, other) = (dir.join("bare.git"), dir.join("work"), dir.join("other"));
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
    if !made {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let _ = bare;
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.git_refresh();
    let behind = |app: &App| app.git.status.as_ref().map_or(0, |s| s.behind);
    assert_eq!(behind(&app), 0);

    let now = std::time::Instant::now();
    assert_eq!(app.auto_fetch_tick(now), None);
    app.config.git_auto_fetch = Some(true);
    assert_eq!(
        app.auto_fetch_tick(now),
        Some(now + crate::app::git::fetch::FETCH_PERIOD)
    );
    assert_eq!(behind(&app), 1);
    // 还没到下一个周期：不再获取。
    assert_eq!(
        app.auto_fetch_tick(now),
        Some(now + crate::app::git::fetch::FETCH_PERIOD)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 设置的 Git 程序不存在时提交视图写明是哪个程序；清空设置后恢复正常。
#[test]
fn a_missing_git_program_is_named_and_clearing_it_recovers() {
    let dir = search_folder("git-custom-program");
    if !make_repo(&dir) {
        return;
    }
    let Some(mut app) = git_app(&dir) else {
        return;
    };
    app.git_refresh();
    assert!(app.git.status.is_some() && app.git.problem.is_none());

    app.set_git_path(Some("winc-no-such-git-program".to_owned()));
    assert_eq!(
        app.config.git_path.as_deref(),
        Some("winc-no-such-git-program")
    );
    assert!(app.git.status.is_none());
    let problem = app.git.problem.clone().unwrap_or_default();
    assert!(problem.contains("winc-no-such-git-program"), "{problem}");
    assert!(app.git.missing);

    app.set_git_path(None);
    assert!(app.git.status.is_some() && app.git.problem.is_none() && !app.git.missing);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拉取方式设为变基：平时的拉取变基，按住 ⌥ 换成拉取并合并；拉取的说明写明方式，拉取对话框
/// 默认勾选变基。
#[test]
fn the_pull_mode_setting_decides_how_pull_brings_in_commits() {
    use crate::app::input::context_menu::MenuEntry;
    use winit::keyboard::ModifiersState;
    let dir = sample_folder("git-pull-mode");
    let Some(work) = repo_with_remote(&dir) else {
        return;
    };
    let git = |at: &Path, args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "git {args:?}");
    };
    // 另一个克隆推上去一个提交，本地也有一个：两边分叉。
    git(&dir, &["clone", "-q", "bare.git", "other"]);
    let other = dir.join("other");
    std::fs::write(other.join("theirs.txt"), "theirs\n").unwrap();
    git(&other, &["add", "theirs.txt"]);
    git(&other, &["commit", "-q", "-m", "theirs"]);
    git(&other, &["push", "-q", "origin", "main"]);
    std::fs::write(work.join("ours.txt"), "ours\n").unwrap();
    git(&work, &["add", "ours.txt"]);
    git(&work, &["commit", "-q", "-m", "ours"]);
    git(&work, &["fetch", "-q"]);
    // 变基要写提交：仓库自己配好身份，不依赖本机的全局配置。
    git(&work, &["config", "user.name", "t"]);
    git(&work, &["config", "user.email", "t@t"]);
    let Some(mut app) = git_app(&work) else {
        return;
    };
    app.set_pull_mode(winc_config::PullMode::Rebase);
    app.git_refresh();

    let label = app.menu_label(MenuCommand::GitPull);
    assert!(label.subtitle.is_some_and(|s| s.ends_with("变基")));
    app.pointer.modifiers = ModifiersState::ALT;
    let alternate = app.picker_action_command(crate::app::PickerPart::Pull);
    assert_eq!(alternate, MenuCommand::GitPullMerge);
    // 「⋯」菜单里拉取的 ⌥ 备选项相同。
    let more = app.commit_more_menu();
    let at = more
        .iter()
        .position(|e| *e == MenuEntry::Item(MenuCommand::GitPull, true));
    assert_eq!(
        at.and_then(|i| more.get(i + 1)),
        Some(&MenuEntry::Alternate(MenuCommand::GitPullMerge, true))
    );
    app.pointer.modifiers = ModifiersState::empty();
    app.run_action(Action::GitPullFrom);
    assert!(app.overlays.form_dialog.as_ref().unwrap().copy);
    app.close_form_dialog();

    app.run_branch_command(MenuCommand::GitPull);
    app.poll_branch_ops();
    assert_eq!(
        output_of(&work, &["rev-list", "--merges", "--count", "HEAD"]).trim(),
        "0"
    );
    assert_eq!(
        output_of(&work, &["log", "-1", "--format=%s"]).trim(),
        "ours"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
