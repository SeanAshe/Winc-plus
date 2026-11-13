//! 日志中提交的右键菜单及其命令的测试。

use super::*;

#[test]
fn log_commit_menu_checks_out_branches_picks_reverts_and_resets() {
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let Some((mut app, root)) = git_app("git-log-ops", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let row_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits
            .iter()
            .position(|c| c.subject == subject)
            .unwrap()
    };
    let oid_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits[row_of(app, subject)].oid.clone()
    };
    let head = |root: &Path| {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["log", "-1", "--format=%H %s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let right_click_row = |app: &mut App, subject: &str| {
        let row = row_of(app, subject);
        let rect = app
            .hits
            .log
            .iter()
            .find(|(_, t)| *t == LogTarget::Row(row))
            .unwrap()
            .0;
        right_click(app, (rect.x + 200.0, rect.y + rect.h / 2.0));
    };
    let refresh = |app: &mut App| {
        app.git_refresh();
        paint(app);
    };

    // 右键一个提交：菜单中的命令作用于该提交，
    // 包括签出、新建分支、拣选、还原、重置（子菜单）与复制。
    right_click_row(&mut app, "补充使用说明");
    let readme = oid_of(&app, "补充使用说明");
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::LogCommit(readme.clone()))
    );
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(readme.as_str()));
    let menu = &app.overlays.context_menu.as_ref().unwrap().entries;
    assert!(
        menu.iter().any(
            |e| matches!(e, MenuEntry::Submenu(title, _) if title == "将「main」重置到此提交")
        )
    );
    assert!(menu_commands(&app).contains(&(MenuCommand::LogResetHard, true)));

    // 还原：新建一次逆向改动的提交，`README.md` 被删除；日志随之刷新。
    app.run_menu_command(MenuCommand::LogRevert);
    assert!(head(&root).contains("Revert"), "{}", head(&root));
    assert!(!root.join("README.md").exists());
    refresh(&mut app);
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert!(view.commits[0].subject.starts_with("Revert"));

    // 拣选 `dev` 上的提交到 `main`。
    right_click_row(&mut app, "配置：加入调试开关");
    app.run_menu_command(MenuCommand::LogCherryPick);
    assert!(
        head(&root).ends_with("配置：加入调试开关"),
        "{}",
        head(&root)
    );
    refresh(&mut app);

    // 硬重置回「启动日志写到文件里」（没有窗口时不弹确认框）。
    let tip = oid_of(&app, "启动日志写到文件里");
    right_click_row(&mut app, "启动日志写到文件里");
    app.run_menu_command(MenuCommand::LogResetHard);
    assert!(head(&root).starts_with(&tip));
    assert!(root.join("README.md").exists());
    refresh(&mut app);

    // 有本地分支指向的提交：签出该分支，不进入分离的 HEAD。
    right_click_row(&mut app, "配置：加入调试开关");
    app.run_menu_command(MenuCommand::LogCheckout);
    refresh(&mut app);
    let current = app
        .git
        .branches
        .iter()
        .find(|b| b.current)
        .map(|b| b.name.clone());
    assert_eq!(current.as_deref(), Some("dev"));

    // 从提交新建分支：对话框里「基于」固定为该提交且不能修改，输入名称后基于该提交创建并签出。
    right_click_row(&mut app, "启动时先加载配置");
    let base = oid_of(&app, "启动时先加载配置");
    app.run_menu_command(MenuCommand::LogNewBranch);
    assert_eq!(app.focus, Focus::Dialog);
    app.edit_field(|f| f.insert("hotfix"));
    let plan = app.form_plan(app.overlays.form_dialog.as_ref().unwrap());
    assert_eq!(plan.base, format!("提交 {}", &base[..7]));
    assert!(!plan.base_enabled);
    app.submit_form_dialog();
    refresh(&mut app);
    let current = app
        .git
        .branches
        .iter()
        .find(|b| b.current)
        .map(|b| b.name.clone());
    assert_eq!(current.as_deref(), Some("hotfix"));
    assert!(head(&root).starts_with(&base));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn log_compares_two_commits_or_a_branch_and_the_branch_menu_stashes() {
    use crate::app::git::branch::StashOp;
    use crate::app::git::log::LogTarget;
    use crate::app::input::context_menu::MenuCommand;
    use winc_editor::git::Change;

    let Some((mut app, root)) = git_app("git-compare", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    let row_of = |app: &App, subject: &str| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.commits
            .iter()
            .position(|c| c.subject == subject)
            .unwrap()
    };
    let compare = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .compare
            .clone()
    };

    // 选中「补充使用说明」，⌘ 点击「登录：账号密码表单」：较旧的提交（登录）在左，较新的在右。
    let readme = row_of(&app, "补充使用说明");
    let login = row_of(&app, "登录：账号密码表单");
    app.press_log(LogTarget::Row(readme));
    app.pointer.modifiers = command_modifiers();
    app.press_log(LogTarget::Row(login));
    app.pointer.modifiers = ModifiersState::empty();
    let c = compare(&app).expect("在比较");
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(c.from, view.commits[login].oid);
    assert_eq!(c.to, view.commits[readme].oid);
    let files = c.files.unwrap();
    let change = |name: &str| {
        files
            .iter()
            .find(|f| f.path.ends_with(name))
            .map(|f| f.change)
    };
    assert_eq!(change("README.md"), Some(Change::Added));
    assert_eq!(change("src/login.rs"), Some(Change::Deleted));

    // 点击文件打开两次提交之间的差异；回到日志按 Esc 退出比较。
    paint(&mut app);
    let i = files
        .iter()
        .position(|f| f.path.ends_with("README.md"))
        .unwrap();
    app.press_log(LogTarget::File(i));
    let doc = app.editor.active().unwrap();
    assert!(doc.title.as_deref().unwrap().contains('…'));
    assert!(doc.diff.is_some());
    app.run_action(Action::GitShowLog);
    app.log_key(&Key::Named(NamedKey::Escape));
    assert!(compare(&app).is_none());

    // 分支菜单「与当前分支比较」：从 `main` 到 `feature/login`。
    let feature = app
        .git
        .branches
        .iter()
        .position(|b| b.name == "feature/login")
        .unwrap();
    let compare_feature =
        MenuCommand::GitBranch(crate::app::git::branch::BranchOp::Compare, feature);
    choose_in_branch_menu(&mut app, compare_feature);
    let c = compare(&app).expect("在比较分支");
    assert_eq!((c.from.as_str(), c.to.as_str()), ("main", "feature/login"));
    assert!(!c.files.unwrap().is_empty());

    // 储藏：修改一个文件、新增一个文件后储藏（附说明），工作区恢复干净；菜单中列出该储藏。
    std::fs::write(root.join("src/main.rs"), "fn main() { changed(); }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    app.git_refresh();
    app.run_branch_command(MenuCommand::GitStash);
    assert_eq!(app.focus, Focus::Prompt);
    app.edit_field(|f| f.insert("试一下新写法"));
    app.submit_prompt();
    app.git_refresh();
    assert_eq!(app.git.stashes.len(), 1);
    assert!(app.git.stashes[0].message.ends_with("试一下新写法"));
    assert!(!root.join("notes.txt").exists());
    assert_ne!(
        std::fs::read_to_string(root.join("src/main.rs")).unwrap(),
        "fn main() { changed(); }\n"
    );
    // 改动列表下方新增储藏分组，展开后列出该储藏。
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.commit_parts().stashes.is_some());
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));

    // 应用并删除：改动恢复，储藏被移除。
    choose_in_branch_menu(&mut app, MenuCommand::GitStashOp(StashOp::Pop, 0));
    app.git_refresh();
    assert!(app.git.stashes.is_empty());
    assert!(root.join("notes.txt").exists());

    // 再储藏一次（不附说明），之后删除（没有窗口时不弹确认框）。
    app.run_branch_command(MenuCommand::GitStash);
    app.submit_prompt();
    app.git_refresh();
    assert_eq!(app.git.stashes.len(), 1);
    choose_in_branch_menu(&mut app, MenuCommand::GitStashOp(StashOp::Drop, 0));
    app.git_refresh();
    assert!(app.git.stashes.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_log_opens_from_the_commit_view_button_and_the_branch_menu_starts_with_fetch() {
    use crate::app::git::branch::branch_menu;
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-log-entry", false) else {
        return;
    };
    // 提交视图标题行最左边的工具按钮是「Git 日志」，悬停提示带 ⌘9。
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let (rect, target) = app.commit_parts().tools[0];
    assert_eq!(target, CommitTarget::Log);
    click(&mut app, (rect.x + 4.0, rect.y + 4.0));
    assert!(app.log_active());

    // 「⋯」不重复列出日志入口和同步，最上方是储藏；获取在分支选择器里，悬停提示写明作用。
    let menu = branch_menu(&[], None, false);
    assert_eq!(
        menu.first(),
        Some(&MenuEntry::Item(MenuCommand::GitStash, true))
    );
    let label = app.menu_label(MenuCommand::GitFetch);
    assert_eq!(
        label.subtitle.as_deref(),
        Some("下载远程更新，不修改本地分支")
    );
    let _ = std::fs::remove_dir_all(&root);
}
