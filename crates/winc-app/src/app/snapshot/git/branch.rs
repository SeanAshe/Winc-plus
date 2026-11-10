//! 分支菜单与新建分支对话框的测试：新建、切换、推送、删除，以及远程分支与提交并推送。

use super::*;
use crate::app::GitAnchor;

#[test]
fn branch_menu_creates_switches_pushes_and_deletes() {
    use crate::app::git::branch::{BranchOp, branch_ops};
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-branches", false) else {
        return;
    };
    // 先把工作区的改动提交，保证切换分支时没有未提交内容。
    git_in(&root, &["add", "."]);
    git_in(&root, &["commit", "-q", "-m", "second"]);
    app.git_refresh();
    let index_of = |app: &App, name: &str| {
        app.git
            .branches
            .iter()
            .position(|b| b.name == name)
            .unwrap()
    };
    let head = |app: &App| match app.git.status.as_ref().and_then(|s| s.head.clone()) {
        Some(Head::Branch(name)) => name,
        other => panic!("{other:?}"),
    };

    // 新建分支：对话框里输入名称，回车创建并签出。空格这类字符无法输入。
    app.open_branch_dialog(None, None);
    assert_eq!(app.focus, Focus::Dialog);
    paint(&mut app);
    app.edit_field(|f| f.insert("feature x"));
    let dialog = app.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.name.text(), "featurex");
    assert_eq!(app.form_plan(dialog).base, "main（当前分支）");
    app.submit_form_dialog();
    assert!(app.form_dialog.is_none());
    assert!(app.branch.running.is_none());
    assert_eq!(head(&app), "featurex");

    // 分支重名时不能创建：原因显示在输入框里，对话框保留；取消「创建后签出」时只创建、不切换。
    app.open_branch_dialog(None, None);
    app.edit_field(|f| f.insert("main"));
    let plan = app.form_plan(app.form_dialog.as_ref().unwrap());
    assert!(!plan.enabled);
    assert_eq!(plan.problem.as_deref(), Some("已存在同名的分支"));
    app.submit_form_dialog();
    assert!(app.form_dialog.is_some());
    app.edit_field(|f| {
        f.set_text("side");
        true
    });
    app.form_dialog.as_mut().unwrap().checkout = false;
    app.submit_form_dialog();
    assert_eq!(head(&app), "featurex");
    assert!(app.git.branches.iter().any(|b| b.name == "side"));

    // 菜单里：当前分支只能重命名，别的分支能检出、新建、比较、合并、变基、删除。
    // 没有上游：分支选择器里的拉取灰显。
    assert!(!app.picker_action_enabled(crate::app::PickerPart::Pull));
    let ops = |app: &App, name: &str| {
        branch_ops(index_of(app, name), &app.git.branches, &[], &[], false)
            .iter()
            .filter(|entry| matches!(entry, MenuEntry::Item(..)))
            .count()
    };
    assert_eq!(ops(&app, "featurex"), 1);
    assert_eq!(ops(&app, "main"), 8);

    // 检出 main，再删除 featurex。
    let main = index_of(&app, "main");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Checkout, main));
    assert_eq!(head(&app), "main");
    let feature = index_of(&app, "featurex");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Delete, feature));
    // 先确认：取消后分支还在，确认后才删除。
    assert!(app.alert.as_ref().unwrap().buttons[0].destructive);
    press_alert(&mut app, 1);
    assert!(app.git.branches.iter().any(|b| b.name == "featurex"));
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Delete, feature));
    press_alert(&mut app, 0);
    assert!(app.git.branches.iter().all(|b| b.name != "featurex"));

    // 菜单打开期间分支列表刷新、下标错位：按弹出菜单时的列表执行，删除的仍是菜单上的分支。
    git_in(&root, &["branch", "aaa"]);
    git_in(&root, &["branch", "doomed"]);
    app.git_refresh();
    let doomed = index_of(&app, "doomed");
    app.open_context_menu(
        crate::app::input::context_menu::MenuTarget::BranchOps(doomed),
        (0.0, 0.0),
    );
    git_in(&root, &["branch", "--delete", "aaa"]);
    app.git_refresh();
    assert_ne!(index_of(&app, "doomed"), doomed);
    app.run_menu_command(MenuCommand::GitBranch(BranchOp::Delete, doomed));
    press_alert(&mut app, 0);
    assert!(app.git.branches.iter().all(|b| b.name != "doomed"));
    assert!(app.git.branches.iter().any(|b| b.name == "side"));

    // 没有远程：推送说明原因。
    app.run_branch_command(MenuCommand::GitPush);
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("未配置远程仓库")),
        "{:?}",
        app.notice.text
    );

    // 加一个远程：第一次推送设好上游，之后获取、拉取都能用，远程分支出现在列表里。
    let remote = root.with_extension("remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(
        root.parent().unwrap(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    app.git_refresh();
    // 推送前确认目标远程：取消时不推送，确认后才推送。
    app.run_branch_command(MenuCommand::GitPush);
    press_alert(&mut app, 1);
    app.git_refresh();
    assert!(
        app.git
            .branches
            .iter()
            .all(|b| !b.current || b.upstream.is_none())
    );
    app.run_branch_command(MenuCommand::GitPush);
    press_alert(&mut app, 0);
    app.git_refresh();
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.current && b.upstream.is_some())
    );
    app.run_branch_command(MenuCommand::GitFetch);
    app.run_branch_command(MenuCommand::GitPull);
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.remote && b.name == "origin/main")
    );

    // 远程分支检出成跟踪该远程分支的本地分支。
    git_in(&root, &["push", "-q", "origin", "main:topic"]);
    app.run_branch_command(MenuCommand::GitFetch);
    let topic = index_of(&app, "origin/topic");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Checkout, topic));
    assert_eq!(head(&app), "topic");
    paint(&mut app);

    // 提交并推送：提交成功以后继续推送，本地不再领先远程。
    std::fs::write(root.join("src/main.rs"), "fn main() { pushed() }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("提交并推送"));
    app.run_commit_button_command(crate::app::input::context_menu::MenuCommand::CommitPush);
    press_alert(&mut app, 0);
    app.git_refresh();
    let status = app.git.status.as_ref().unwrap();
    assert!(status.entries.is_empty(), "{:?}", status.entries);
    assert_eq!(status.ahead, 0);
    paint(&mut app);

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：新建分支对话框的空、已填写、重名（错误色）、基于日志里的提交（「基于」不可选）四种状态，
/// 浅色与深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_branch_dialog() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("branch-dialog-{theme}"), false, dark)
        else {
            return;
        };
        app.open_branch_dialog(None, None);
        shoot(&mut app, &format!("branch-dialog-empty-{theme}"));
        app.edit_field(|f| f.insert("feat/login-page"));
        shoot(&mut app, &format!("branch-dialog-{theme}"));
        app.edit_field(|f| {
            f.set_text("main");
            true
        });
        shoot(&mut app, &format!("branch-dialog-error-{theme}"));
        app.close_form_dialog();
        let oid = app.git.status.as_ref().unwrap().oid.clone().unwrap();
        app.open_branch_dialog(None, Some(oid));
        app.edit_field(|f| f.insert("hotfix"));
        shoot(&mut app, &format!("branch-dialog-commit-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_running_branch_operation_slides_into_the_status_bar() {
    let Some((mut app, root)) = git_app("git-status-running", false) else {
        return;
    };
    app.status.motion.animate_without_window = true;
    paint(&mut app);
    // 打开时各项出现的过渡直接走完。
    let slide = crate::app::status::motion::STATUS_SLIDE;
    app.status.motion.freeze(slide);
    app.status.motion.settle();
    paint(&mut app);
    assert!(!app.status.motion.moving());
    app.branch.running = Some(winc_i18n::t!("git-op-sync").to_owned());
    paint(&mut app);
    assert!(app.status.motion.moving());
    let _ = std::fs::remove_dir_all(&root);
}

/// 分支选择器测试用的仓库：`feature/login`、`feature/search`、`fix/one` 三个分支，当前在 `main`。
fn picker_repo(name: &str, dark: bool) -> Option<(App, PathBuf)> {
    let (mut app, root) = git_app_in(name, false, dark)?;
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    for branch in ["feature/login", "feature/search", "fix/one"] {
        git_in(&root, &["branch", branch]);
    }
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    Some((app, root))
}

fn picker_labels(app: &App) -> Vec<String> {
    use crate::app::git::branch::picker::BranchRow;
    app.picker
        .branch_rows
        .as_deref()
        .unwrap()
        .iter()
        .filter_map(|row| match row {
            BranchRow::Folder { label, .. } | BranchRow::Branch { label, .. } => {
                Some(label.clone())
            }
            _ => None,
        })
        .collect()
}

/// 点提交视图里的 `target`（按这一帧画出来的位置）。
fn click_commit(app: &mut App, target: CommitTarget) {
    let rect = (app.hits.commit.iter())
        .find(|(_, t)| *t == target)
        .unwrap()
        .0;
    click(app, (rect.x + 4.0, rect.y + 4.0));
}

fn head_name(app: &App) -> Option<String> {
    let branch = app.git.branches.iter().find(|b| b.current)?;
    Some(branch.name.clone())
}

#[test]
fn the_branch_row_opens_the_picker_below_it_and_enter_switches() {
    use crate::app::git::branch::picker::{BranchRow, BranchSection};

    let Some((mut app, root)) = picker_repo("pk", false) else {
        return;
    };
    click_commit(&mut app, CommitTarget::Branch);
    assert!(app.picker_lists_branches());
    assert_eq!(app.focus, Focus::Picker);
    paint(&mut app);
    // 面板贴在分支行下方，不跑到屏幕中间。
    let row = app.commit_parts().branch;
    assert_eq!(app.layout.picker.x, row.x);
    assert!(app.layout.picker.y >= row.bottom());
    // 当前分支在最前；其余目录默认收起，当前分支不在目录里时目录都收着。
    assert_eq!(picker_labels(&app), ["main", "feature", "fix"]);

    // 搜索展开目录，回车切换并关闭。
    app.picker.input.set_text("search");
    app.picker_refresh();
    assert_eq!(picker_labels(&app), ["feature", "search"]);
    app.picker_accept();
    assert!(!app.picker.open);
    assert_eq!(head_name(&app).as_deref(), Some("feature/search"));

    // 分支不多：不单列最近使用，一开始选中当前分支；它所在的目录默认展开。
    app.open_branch_picker(GitAnchor::Branch);
    let rows = app.picker.branch_rows.clone().unwrap();
    assert_eq!(rows[0], BranchRow::Heading(BranchSection::Local));
    assert!(matches!(
        &rows[app.picker.selected],
        BranchRow::Branch { label, .. } if label == "search"
    ));
    paint(&mut app);

    // 再点分支行（面板之外）关闭。
    let row = app.commit_parts().branch;
    click(&mut app, (row.x + 4.0, row.y + 4.0));
    assert!(!app.picker.open);
    let _ = std::fs::remove_dir_all(&root);
}

/// 有未提交的更改时切换分支：面板关掉，用提示框询问带过去还是储藏，取消时什么都不变。
#[test]
fn switching_with_uncommitted_changes_asks_to_bring_or_stash_them() {
    let Some((mut app, root)) = picker_repo("git-picker-dirty", false) else {
        return;
    };
    std::fs::write(root.join("src/main.rs"), "fn changed() {}\n").unwrap();
    app.git_refresh();
    let ask_for = |app: &mut App, query: &str| {
        app.open_branch_picker(GitAnchor::Branch);
        app.picker.input.set_text(query);
        app.picker_refresh();
        app.picker_accept();
    };

    // 问的时候还没切换，面板已经关了；取消以后还在原来的分支。
    ask_for(&mut app, "feature/login");
    assert!(!app.picker.open);
    assert_eq!(head_name(&app).as_deref(), Some("main"));
    assert_eq!(app.alert.as_ref().unwrap().buttons.len(), 3);
    press_alert(&mut app, 2);
    assert!(app.alert.is_none());
    assert_eq!(head_name(&app).as_deref(), Some("main"));

    // 带过去：更改还在工作区，分支换了。
    ask_for(&mut app, "feature/login");
    press_alert(&mut app, 0);
    assert_eq!(head_name(&app).as_deref(), Some("feature/login"));
    assert!(!app.git.status.as_ref().unwrap().entries.is_empty());

    // 储藏后切换：工作区干净，储藏多一条。
    let stashes = app.git.stashes.len();
    ask_for(&mut app, "feature/search");
    press_alert(&mut app, 1);
    assert_eq!(head_name(&app).as_deref(), Some("feature/search"));
    let tracked = app
        .git
        .status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .filter(|e| !e.untracked())
        .count();
    assert_eq!(tracked, 0);
    assert_eq!(app.git.stashes.len(), stashes + 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_search_box_can_create_a_branch() {
    let Some((mut app, root)) = picker_repo("git-picker-create", false) else {
        return;
    };
    app.open_branch_picker(GitAnchor::Branch);
    // 搜不到时回车就新建。
    app.picker.input.set_text("brand new");
    app.picker_refresh();
    app.picker_accept();
    assert!(!app.picker.open);
    assert_eq!(head_name(&app).as_deref(), Some("brand-new"));

    // 底栏的「新建分支」：关闭选择器，打开新建分支的对话框（基于当前分支）。
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    app.press_picker_part(PickerPart::NewBranch);
    assert!(!app.picker.open);
    let kind = app.form_dialog.as_ref().map(|dialog| dialog.kind.clone());
    assert_eq!(
        kind,
        Some(crate::app::git::form::FormKind::Branch { at: None })
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_branch_picker_folds_folders_and_opens_branch_actions() {
    use crate::app::git::branch::BranchOp;
    use crate::app::input::context_menu::MenuCommand;

    let Some((mut app, root)) = picker_repo("git-picker-fold", false) else {
        return;
    };
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    // 选中目录，右键展开，再按进入，左键回到目录，再按收起。
    let at = |app: &App, name: &str| {
        use crate::app::git::branch::picker::BranchRow;
        (app.picker.branch_rows.as_deref().unwrap().iter()).position(|row| match row {
            BranchRow::Folder { label, .. } | BranchRow::Branch { label, .. } => label == name,
            _ => false,
        })
    };
    app.picker.selected = at(&app, "feature").unwrap();
    assert!(app.branch_picker_side(true));
    assert!(picker_labels(&app).contains(&"login".to_owned()));
    assert!(app.branch_picker_side(true));
    assert_eq!(app.picker.selected, at(&app, "login").unwrap());
    assert!(app.branch_picker_side(false));
    assert_eq!(app.picker.selected, at(&app, "feature").unwrap());
    assert!(app.branch_picker_side(false));
    assert!(!picker_labels(&app).contains(&"login".to_owned()));
    // 输入查询时左右键交给输入框。
    app.picker.input.set_text("f");
    app.picker_refresh();
    assert!(!app.branch_picker_side(true));
    app.picker.input.set_text("");
    app.picker_refresh();

    // 点「⋯」打开分支的操作菜单，选择器还在；选了一项才关闭。
    app.picker.selected = at(&app, "main").unwrap();
    let main_row = app.picker.selected;
    app.picker.selected = at(&app, "fix").unwrap();
    assert!(app.branch_picker_side(true));
    app.picker.selected = at(&app, "one").unwrap();
    let _ = main_row;
    paint(&mut app);
    let more = PickerPart::More(app.picker.selected);
    assert!(app.hits.picker_parts.iter().any(|(_, p)| *p == more));
    app.press_picker_part(more);
    assert!(app.picker_lists_branches());
    let branches = &app.git.branches;
    let one = branches.iter().position(|b| b.name == "fix/one").unwrap();
    let delete = MenuCommand::GitBranch(BranchOp::Delete, one);
    assert!(menu_commands(&app).contains(&(delete, true)));
    app.run_menu_command(delete);
    assert!(!app.picker.open);
    press_alert(&mut app, 0);
    assert!(app.git.branches.iter().all(|b| b.name != "fix/one"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 未完全合并的分支：确认删除后再问一次，再次确认才强制删除。
#[test]
fn an_unmerged_branch_is_deleted_only_after_a_second_confirmation() {
    use crate::app::git::branch::BranchOp;
    use crate::app::input::context_menu::MenuCommand;

    let Some((mut app, root)) = git_app("git-delete-unmerged", false) else {
        return;
    };
    git_in(&root, &["add", "."]);
    git_in(&root, &["commit", "-q", "-m", "second"]);
    git_in(&root, &["switch", "-q", "-c", "side"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "only here"]);
    git_in(&root, &["switch", "-q", "main"]);
    app.git_refresh();
    let exists = |app: &App| app.git.branches.iter().any(|b| b.name == "side");
    let delete = |app: &mut App| {
        let side = app.git.branches.iter().position(|b| b.name == "side");
        let command = MenuCommand::GitBranch(BranchOp::Delete, side.unwrap());
        choose_in_branch_menu(app, command);
        // 第一次确认：git 拒绝，换成强制删除的询问，分支还在。
        press_alert(app, 0);
        assert!(exists(app));
        assert!(app.alert.as_ref().unwrap().buttons[0].destructive);
    };

    delete(&mut app);
    press_alert(&mut app, 1);
    assert!(app.alert.is_none());
    assert!(exists(&app));

    delete(&mut app);
    press_alert(&mut app, 0);
    assert!(app.alert.is_none());
    assert!(!exists(&app));
    let _ = std::fs::remove_dir_all(&root);
}

/// 默认宽度的提交视图里面板和分支行左右对齐，离侧栏左右的距离相同。
#[test]
fn the_picker_lines_up_with_the_context_rows() {
    let Some((mut app, root)) = picker_repo("git-picker-fits", false) else {
        return;
    };
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    let (card, row, sidebar) = (
        app.layout.picker,
        app.commit_parts().branch,
        app.layout.sidebar,
    );
    assert_eq!((card.x, card.right()), (row.x, row.right()));
    assert_eq!(card.x - sidebar.x, sidebar.right() - card.right());
    assert!(sidebar.bottom() - card.bottom() >= card.x - sidebar.x);

    let _ = std::fs::remove_dir_all(&root);
}

/// 迁移更改：文件在两个分支上内容不同时 git 拒绝切换，储藏、切换后把更改应用到新分支。
#[test]
fn carrying_changes_works_when_git_refuses_the_switch() {
    let Some((mut app, root)) = picker_repo("git-carry", false) else {
        return;
    };
    let file = root.join("a.txt");
    // 目标分支改了文件的开头，当前分支未提交的更改在结尾，两者不冲突。
    let lines: String = (1..=9).map(|i| format!("{i}\n")).collect();
    std::fs::write(&file, &lines).unwrap();
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    git_in(&root, &["switch", "-q", "-c", "other"]);
    std::fs::write(&file, lines.replacen("1\n", "one\n", 1)).unwrap();
    git_in(&root, &["commit", "-q", "-am", "other"]);
    git_in(&root, &["switch", "-q", "main"]);
    std::fs::write(&file, format!("{lines}tail\n")).unwrap();
    app.git_refresh();
    let other = (app.git.branches.iter())
        .find(|b| b.name == "other")
        .unwrap()
        .clone();

    app.switch_branch(other.clone(), false);
    app.git_refresh();
    let current = app.git.branches.iter().find(|b| b.current).unwrap();
    assert_eq!(current.name, "other");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("one\n") && text.ends_with("tail\n"),
        "{text}"
    );
    assert!(app.git.stashes.is_empty(), "应用后不留储藏");

    // 更改与目标分支冲突：已切换，冲突标在工作区，储藏保留，有提示。
    git_in(&root, &["checkout", "-q", "--", "a.txt"]);
    git_in(&root, &["switch", "-q", "main"]);
    std::fs::write(&file, lines.replacen("1\n", "mine\n", 1)).unwrap();
    app.git_refresh();
    app.notice.text = None;
    app.switch_branch(other, false);
    app.git_refresh();
    let current = app.git.branches.iter().find(|b| b.current).unwrap();
    assert_eq!(current.name, "other");
    assert!(std::fs::read_to_string(&file).unwrap().contains("<<<<<<<"));
    assert_eq!(app.git.stashes.len(), 1);
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("other"))
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 只有未跟踪的文件阻止切换：选分支时不询问，切换后文件内容在储藏里，并提示无法应用。
#[test]
fn an_untracked_file_in_the_way_is_stashed_and_reported() {
    let Some((mut app, root)) = picker_repo("git-carry-untracked", false) else {
        return;
    };
    let file = root.join("new.txt");
    git_in(&root, &["switch", "-q", "-c", "other"]);
    std::fs::write(&file, "theirs\n").unwrap();
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "other"]);
    git_in(&root, &["switch", "-q", "main"]);
    std::fs::write(&file, "mine\n").unwrap();
    app.git_refresh();
    let other = (app.git.branches.iter())
        .find(|b| b.name == "other")
        .unwrap()
        .clone();

    let other = (app.git.branches.iter()).position(|b| b.name == other.name);
    app.pick_branch(other.unwrap());
    app.git_refresh();
    let current = app.git.branches.iter().find(|b| b.current).unwrap();
    assert_eq!(current.name, "other");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");
    assert_eq!(app.git.stashes.len(), 1, "无法恢复的内容保留在储藏里");
    // 工作区没有冲突标记，提示不能说有。
    let conflict = winc_i18n::t!("git-switch-carry-conflict", branch = "other");
    assert_ne!(app.notice.text.as_deref(), Some(conflict.as_str()));
    assert!(app.notice.text.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// 储藏本身失败时（合并未完成）报告的是切换失败的原因。
#[test]
fn a_failed_stash_reports_why_the_switch_failed() {
    let Some((mut app, root)) = picker_repo("git-carry-merging", false) else {
        return;
    };
    let file = root.join("a.txt");
    std::fs::write(&file, "base\n").unwrap();
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    git_in(&root, &["switch", "-q", "-c", "other"]);
    std::fs::write(&file, "theirs\n").unwrap();
    git_in(&root, &["commit", "-q", "-am", "theirs"]);
    git_in(&root, &["switch", "-q", "main"]);
    std::fs::write(&file, "mine\n").unwrap();
    git_in(&root, &["commit", "-q", "-am", "mine"]);
    // 合并产生冲突，停在未完成的状态。
    let _ = std::process::Command::new("git")
        .args(["merge", "-q", "other"])
        .current_dir(&root)
        .output();
    app.git_refresh();
    let other = (app.git.branches.iter())
        .find(|b| b.name == "other")
        .unwrap()
        .clone();
    app.notice.text = None;
    app.carry_then_checkout(other);
    let notice = app.notice.text.as_deref().unwrap();
    assert!(!notice.to_lowercase().contains("stash"), "{notice}");
    assert!(app.git.stashes.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// 迁移更改时没有东西可储藏（例如分支不存在且工作区干净）：只报告原来的错误，不动已有的储藏。
#[test]
fn a_refused_switch_with_nothing_to_stash_keeps_old_stashes() {
    let Some((mut app, root)) = picker_repo("git-carry-clean", false) else {
        return;
    };
    let file = root.join("a.txt");
    std::fs::write(&file, "kept\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "--include-untracked"]);
    app.git_refresh();
    let mut missing = (app.git.branches.iter())
        .find(|b| b.name == "fix/one")
        .unwrap()
        .clone();
    missing.name = "no-such-branch".to_owned();
    app.carry_then_checkout(missing);
    app.git_refresh();
    assert_eq!(app.git.stashes.len(), 1);
    assert!(app.notice.text.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// 储藏并签出失败时把更改恢复回来，错误里写明。
#[test]
fn a_failed_switch_restores_the_stashed_changes() {
    let Some((mut app, root)) = picker_repo("git-stash-restore", false) else {
        return;
    };
    let file = root.join("a.txt");
    std::fs::write(&file, "changed\n").unwrap();
    app.git_refresh();
    let changed = app.git.status.as_ref().unwrap().entries.len();
    assert!(changed > 0);
    let mut missing = (app.git.branches.iter())
        .find(|b| b.name == "fix/one")
        .unwrap()
        .clone();
    missing.name = "no-such-branch".to_owned();
    app.stash_then_checkout(missing);
    app.git_refresh();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "changed\n");
    assert!(app.git.stashes.is_empty());
    assert_eq!(app.git.status.as_ref().unwrap().entries.len(), changed);
    let _ = std::fs::remove_dir_all(&root);
}

/// 拉取能处理分叉的分支，和同步一样；不在分支上时分支选择器里的推送灰显。
#[test]
fn the_status_menu_pulls_diverged_branches_and_needs_a_branch_to_push() {
    use crate::app::input::context_menu::MenuCommand;
    use crate::app::{PickerPart, StatusItem};

    let Some((mut app, root)) = picker_repo("git-pull-diverged", false) else {
        return;
    };
    let remote = root.with_extension("pull-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "main"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    git_in(&root, &["push", "-q", "origin", "main"]);
    git_in(&root, &["reset", "-q", "--hard", "HEAD~1"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ours"]);
    app.git_refresh();
    assert!(matches!(
        app.git_sync_state(),
        crate::app::SyncState::Both { .. }
    ));
    app.run_branch_command(MenuCommand::GitPull);
    app.git_refresh();
    assert_eq!(app.git.status.as_ref().unwrap().behind, 0);

    git_in(&root, &["checkout", "-q", "--detach"]);
    app.git_refresh();
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    app.open_status_menu(StatusItem::Branch);
    assert!(app.picker.open);
    assert!(!app.picker_action_enabled(PickerPart::Push));
    assert!(app.picker_action_enabled(PickerPart::Fetch));
    app.close_picker();

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&remote);
}

/// 工作区有未提交的更改时仍然拉取：更改先储藏、拉取后恢复；恢复时冲突则提示冲突已在工作区中
/// 标记、原更改仍在储藏中。
#[test]
fn pulling_keeps_uncommitted_changes_and_reports_a_conflicting_restore() {
    use crate::app::input::context_menu::MenuCommand;

    let Some((mut app, root)) = picker_repo("git-pull-carry", false) else {
        return;
    };
    let remote = root.with_extension("carry-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "main"]);
    // 上游改了 `src/main.rs`，本地回到改动之前，再在同一行改出自己的、未提交的版本。
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() { theirs() }\n").unwrap();
    git_in(&root, &["commit", "-q", "-am", "theirs"]);
    git_in(&root, &["push", "-q", "origin", "main"]);
    git_in(&root, &["reset", "-q", "--hard", "HEAD~1"]);
    std::fs::write(&file, "fn main() { mine() }\n").unwrap();
    app.git_refresh();

    app.run_branch_command(MenuCommand::GitPull);
    app.git_refresh();
    assert_eq!(app.git.status.as_ref().unwrap().behind, 0);
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("拉取完成") && n.contains("保留在储藏中")),
        "{:?}",
        app.notice.text
    );
    assert!(std::fs::read_to_string(&file).unwrap().contains("mine"));
    assert_eq!(app.git.stashes.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&remote);
}

/// 侧栏收着时也能从状态栏打开分支选择器：面板在状态栏上方、窗口之内，搜索框下面是拉取、推送、
/// 获取一行，列表从它下面开始；按住 ⌥ 时拉取换成拉取并变基。
#[test]
fn the_status_bar_opens_the_branch_picker_with_the_sync_actions() {
    use crate::app::input::context_menu::MenuCommand;
    use crate::app::{PickerPart, StatusItem};

    let Some((mut app, root)) = picker_repo("git-picker-status", false) else {
        return;
    };
    app.set_sidebar(false);
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    app.open_status_menu(StatusItem::Branch);
    paint(&mut app);
    assert!(app.picker.open && app.picker_has_actions());
    let (item, _) = *(app.hits.status.iter())
        .find(|(_, i)| *i == StatusItem::Branch)
        .unwrap();
    let card = app.layout.picker;
    assert!(card.bottom() <= item.y && card.y >= 0.0);
    assert!(card.x >= 0.0 && card.right() <= app.layout.window.right());
    let parts = crate::app::workspace::picker::picker_parts(&app, card);
    assert!(parts.actions.h > 0.0);
    assert!(parts.actions.y >= parts.bar.bottom() && parts.list.y >= parts.actions.bottom());

    assert_eq!(
        app.picker_action_command(PickerPart::Pull),
        MenuCommand::GitPull
    );
    app.modifiers = winit::keyboard::ModifiersState::ALT;
    assert_eq!(
        app.picker_action_command(PickerPart::Pull),
        MenuCommand::GitPullRebase
    );
    app.modifiers = winit::keyboard::ModifiersState::empty();
    let _ = std::fs::remove_dir_all(&root);
}

/// 命令面板里的「Git：切换分支…」：提交视图开着时从分支行展开，看着文件视图时从状态栏展开；
/// 不在仓库里时同步一类的命令只说明一声。
#[test]
fn the_switch_branch_command_opens_the_picker_where_it_can_be_seen() {
    use crate::app::GitAnchor;

    let Some((mut app, root)) = picker_repo("git-picker-command", false) else {
        return;
    };
    app.run_action(Action::GitSwitchBranch);
    assert!(app.picker.open && app.picker_lists_branches());
    assert_eq!(app.picker.anchor, GitAnchor::Branch);
    app.close_picker();
    app.run_action(Action::ShowFiles);
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    app.run_action(Action::GitSwitchBranch);
    assert_eq!(app.picker.anchor, GitAnchor::Status);
    app.close_picker();
    for action in [
        Action::GitFetch,
        Action::GitPull,
        Action::GitPush,
        Action::GitStash,
    ] {
        assert_eq!(
            action.module(),
            Some(crate::keymap::Module::Git),
            "{action:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：从状态栏打开的分支选择器（侧栏收着），搜索框下面是拉取、推送、获取
/// （`branch-picker-status`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_status_branch_picker() {
    use crate::app::StatusItem;

    let Some((mut app, root)) = picker_repo("git-picker-status-shot", false) else {
        return;
    };
    git_in(
        &root,
        &["remote", "add", "origin", "https://example.invalid/r.git"],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    app.git_refresh();
    app.set_sidebar(false);
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    app.open_status_menu(StatusItem::Branch);
    shoot(&mut app, "branch-picker-status");
    let _ = std::fs::remove_dir_all(&root);
}

/// 远程分支和本地分支同一层；最外层的远程分支按左键回到「远程分支」一组。
#[test]
fn the_left_key_moves_from_a_remote_branch_to_its_group() {
    use crate::app::git::branch::picker::BranchRow;

    let Some((mut app, root)) = picker_repo("git-picker-remote", false) else {
        return;
    };
    git_in(&root, &["update-ref", "refs/remotes/origin/topic", "HEAD"]);
    app.git_refresh();
    app.open_branch_picker(GitAnchor::Branch);
    let group = |app: &App| {
        (app.picker.branch_rows.as_deref().unwrap().iter())
            .position(|row| matches!(row, BranchRow::Remotes { .. }))
            .unwrap()
    };
    app.picker.selected = group(&app);
    assert!(app.branch_picker_side(true));

    app.picker.selected = group(&app) + 1;
    let row = &app.picker.branch_rows.as_deref().unwrap()[app.picker.selected];
    assert!(matches!(row, BranchRow::Branch { depth: 0, .. }));
    assert!(app.branch_picker_side(false));
    assert_eq!(app.picker.selected, group(&app));
    let _ = std::fs::remove_dir_all(&root);
}

/// 展开时滚到能看见新露出的行，展开的那一行不滚出去；收起以后列表不留空白。
#[test]
fn folding_keeps_the_new_rows_in_view() {
    use crate::app::git::branch::picker::BranchRow;

    let Some((mut app, root)) = picker_repo("git-picker-reveal", false) else {
        return;
    };
    for i in 0..24 {
        let name = format!("refs/remotes/origin/r{i:02}");
        git_in(&root, &["update-ref", &name, "HEAD"]);
    }
    app.git_refresh();
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    let group = (app.picker.branch_rows.as_deref().unwrap().iter())
        .position(|row| matches!(row, BranchRow::Remotes { .. }))
        .unwrap();
    assert_eq!(app.picker.scroll, 0.0);

    // 展开：放不下时滚到组在最上面，下面的行露出来。
    app.picker.selected = group;
    app.picker_accept();
    paint(&mut app);
    let (parts, row) = (
        crate::app::workspace::picker::picker_parts(&app, app.layout.picker),
        crate::app::PICKER_ROW,
    );
    assert_eq!(app.picker.scroll, group as f32 * row);
    assert!(parts.row(group, app.picker.scroll).y >= parts.list.y);
    assert!(parts.row(group + 1, app.picker.scroll).bottom() <= parts.list.bottom());

    // 收起：一屏放得下了，滚动量回到 0，列表下面没有空白。
    app.picker_accept();
    paint(&mut app);
    assert_eq!(app.picker.scroll, 0.0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 展开、收起的动画：下面的行往上收，子行从父行下面露出来，面板高度跟着过渡，结束后清除。
#[test]
fn folding_animates_the_rows_and_the_height() {
    use crate::app::animation::Clock;
    use crate::app::git::branch::picker::{BranchRow, PickerFold};

    let Some((mut app, root)) = picker_repo("git-picker-motion", false) else {
        return;
    };
    for name in ["r1", "r2", "r3"] {
        let name = format!("refs/remotes/origin/{name}");
        git_in(&root, &["update-ref", &name, "HEAD"]);
    }
    app.git_refresh();
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    let closed = app.layout.picker.h;
    let group = (app.picker.branch_rows.as_deref().unwrap().iter())
        .position(|row| matches!(row, BranchRow::Remotes { .. }))
        .unwrap();
    app.picker.selected = group;
    let before = app.picker_fold_before();
    app.picker_accept();
    paint(&mut app);
    let opened = app.layout.picker.h;
    assert_eq!(opened, closed + 3.0 * crate::app::PICKER_ROW);

    let half = Clock::frozen(
        std::time::Duration::from_millis(1000),
        std::time::Duration::from_millis(2000),
    );
    app.picker.fold = Some(PickerFold {
        before,
        count: 3,
        expanding: true,
        clock: half,
    });
    let fold = app.picker.fold.as_ref().unwrap();
    let hidden = fold.hidden();
    assert!(hidden > 0.0 && hidden < 3.0 * crate::app::PICKER_ROW);
    assert!(fold.turned() > 0.0 && fold.turned() < 1.0);
    assert!(app.animating());
    paint(&mut app);
    assert!(app.layout.picker.h > closed && app.layout.picker.h < opened);
    shoot(&mut app, "branch-picker-fold");

    // 再按一下鼠标：动画取消，直接到位。
    let inside = app.layout.picker;
    app.on_mouse_move(inside.x + 40.0, inside.y + 12.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.picker.fold.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：提交视图和分支选择器，深浅两套配色。
#[test]
#[ignore]
fn shoot_branch_picker() {
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        let Some((mut app, root)) = picker_repo(&format!("git-picker-shot-{theme}"), dark) else {
            return;
        };
        for branch in ["fix/deep/two", "release/1.0", "release/2.0"] {
            git_in(&root, &["branch", branch]);
        }
        git_in(&root, &["checkout", "-q", "fix/one"]);
        git_in(&root, &["checkout", "-q", "main"]);
        git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(root.join("src/main.rs"), "fn changed() {}\n").unwrap();
        app.git_refresh();
        paint(&mut app);
        shoot(&mut app, &format!("commit-context-{theme}"));
        app.confirm_force_push(1);
        shoot(&mut app, &format!("commit-ask-{theme}"));
        app.alert = None;
        app.open_branch_picker(GitAnchor::Branch);
        shoot(&mut app, &format!("branch-picker-{theme}"));
        app.picker.input.set_text("rel");
        app.picker_refresh();
        shoot(&mut app, &format!("branch-picker-search-{theme}"));
        app.close_picker();
        app.open_branch_picker(GitAnchor::Branch);
        app.picker.input.set_text("fix/one");
        app.picker_refresh();
        app.picker_accept();
        shoot(&mut app, &format!("branch-picker-dirty-{theme}"));
        app.alert = None;
        app.open_branch_picker(GitAnchor::Branch);
        app.press_picker_part(PickerPart::NewBranch);
        shoot(&mut app, &format!("branch-dialog-{theme}"));
        app.close_form_dialog();
        for branch in ["dev", "feature/login"] {
            git_in(
                &root,
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    "HEAD",
                ],
            );
        }
        app.git_refresh();
        app.open_branch_picker(GitAnchor::Branch);
        app.picker.selected = (app.picker.branch_rows.as_deref().unwrap().iter())
            .position(|row| {
                matches!(
                    row,
                    crate::app::git::branch::picker::BranchRow::Remotes { .. }
                )
            })
            .unwrap();
        app.branch_picker_side(true);
        shoot(&mut app, &format!("branch-picker-remote-{theme}"));
        app.close_picker();
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 分支选择器中鼠标所在的行与键盘选中的行相互独立：鼠标移动不改变选中项（打开时是当前分支），
/// 鼠标所在的行显示自己的「⋯」；鼠标移出后选中项保持不变。
#[test]
fn hovering_a_picker_row_leaves_the_selection_alone() {
    let Some((mut app, root)) = picker_repo("git-picker-hover", false) else {
        return;
    };
    git_in(&root, &["branch", "topic"]);
    app.git_refresh();
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    let current = app.picker.selected;
    let rows = app.picker.branch_rows.clone().unwrap();
    let other = (0..rows.len())
        .find(|&r| {
            r != current
                && matches!(
                    rows[r],
                    crate::app::git::branch::picker::BranchRow::Branch { .. }
                )
        })
        .unwrap();
    let more = |app: &App, row| {
        (app.hits.picker_parts.iter()).any(|(_, p)| *p == crate::app::PickerPart::More(row))
    };
    assert!(more(&app, current));
    assert!(!more(&app, other));

    let parts = crate::app::workspace::picker::picker_parts(&app, app.layout.picker);
    let row = parts.row(other, app.picker.scroll);
    app.on_mouse_move(row.x + 40.0, row.y + row.h / 2.0);
    assert_eq!(app.hover, Hover::PickerRow(other));
    paint(&mut app);
    assert_eq!(app.picker.selected, current);
    assert!(more(&app, other) && more(&app, current));

    let card = app.layout.picker;
    app.on_mouse_move(card.right() + 40.0, card.y + 10.0);
    paint(&mut app);
    assert_eq!(app.picker.selected, current);
    assert!(!more(&app, other));
    let _ = std::fs::remove_dir_all(&root);
}

/// 本地分支多、单列最近使用时，打开面板自动选中上一个用过的分支：以浅色底显示，不与对勾标出的
/// 当前分支混淆；回车仍切回它。移动选中项或输入查询后改为普通的选中样式，清空查询后恢复。
#[test]
fn the_default_branch_row_is_highlighted_softly() {
    use crate::app::git::branch::picker::{BranchRow, BranchSection};

    let Some((mut app, root)) = picker_repo("git-picker-default", false) else {
        return;
    };
    for k in 0..20 {
        git_in(&root, &["branch", &format!("topic-{k:02}")]);
    }
    git_in(&root, &["checkout", "-q", "topic-07"]);
    git_in(&root, &["checkout", "-q", "main"]);
    app.git_refresh();
    app.open_branch_picker(GitAnchor::Branch);
    paint(&mut app);
    let rows = app.picker.branch_rows.clone().unwrap();
    assert_eq!(rows[0], BranchRow::Heading(BranchSection::Recent));
    assert!(matches!(
        &rows[app.picker.selected],
        BranchRow::Branch { label, .. } if label == "topic-07"
    ));
    assert_eq!(app.picker.default, Some(app.picker.selected));

    app.picker_move(true);
    assert_eq!(app.picker.default, None);
    app.picker_move(false);
    assert_eq!(app.picker.default, None);

    app.picker.input.set_text("topic");
    app.picker_refresh();
    assert_eq!(app.picker.default, None);
    app.picker.input.set_text("");
    app.picker_refresh();
    assert_eq!(app.picker.default, Some(app.picker.selected));
    paint(&mut app);
    app.picker_accept();
    assert_eq!(head_name(&app).as_deref(), Some("topic-07"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 发布前的提示框写明目标远程和地址（按名字排在前面的远程不会被选中）；勾选「不再询问」后写入
/// 配置，设置中的开关随之关闭，之后推送不再询问。
#[test]
fn publishing_asks_with_the_target_remote_until_told_not_to() {
    use crate::layout::Setting;

    let Some((mut app, root)) = git_app("git-push-confirm", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "one"]);
    let fork = root.with_extension("fork.git");
    let origin = root.with_extension("origin.git");
    for bare in [&fork, &origin] {
        let _ = std::fs::remove_dir_all(bare);
        git_in(
            root.parent().unwrap(),
            &["init", "-q", "--bare", bare.to_str().unwrap()],
        );
    }
    git_in(
        &root,
        &[
            "remote",
            "add",
            "github-desktop-alice",
            fork.to_str().unwrap(),
        ],
    );
    git_in(
        &root,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    app.git_refresh();
    assert_eq!(app.git.publish_remote.as_deref(), Some("origin"));
    assert!(app.switch_on(Setting::GitConfirmPush));

    app.run_branch_command(MenuCommand::GitPush);
    let alert = app.alert.as_ref().unwrap();
    assert!(alert.message.contains("origin"), "{}", alert.message);
    assert!(
        alert.detail.contains(origin.to_str().unwrap()),
        "{}",
        alert.detail
    );
    assert!(alert.suppression.is_some() && !alert.checked);

    // 勾选「不再询问」后确认：推到 `origin`，配置随之写入。测试中没有配置文件路径，不写入用户的
    // 配置。
    app.alert.as_mut().unwrap().checked = true;
    press_alert(&mut app, 0);
    app.git_refresh();
    let has = |bare: &Path| {
        Command::new("git")
            .arg("-C")
            .arg(bare)
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/main"])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    assert!(has(&origin) && !has(&fork));
    assert_eq!(app.config.git_confirm_push, Some(false));
    assert!(!app.switch_on(Setting::GitConfirmPush));
    assert!(app.config_path.is_none());

    // 之后推送不再询问。
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "two"]);
    app.git_refresh();
    app.run_branch_command(MenuCommand::GitPush);
    assert!(app.alert.is_none());
    app.git_refresh();
    assert_eq!(app.git.status.as_ref().unwrap().ahead, 0);

    let _ = std::fs::remove_dir_all(&fork);
    let _ = std::fs::remove_dir_all(&origin);
    let _ = std::fs::remove_dir_all(&root);
}
