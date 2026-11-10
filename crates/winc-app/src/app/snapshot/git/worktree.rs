//! 工作树的测试：项目菜单里的一组、分支被其他工作树签出、在当前窗口打开，以及附属工作树的徽章。

use super::*;

/// 在仓库 `root` 旁边用新分支 `feat` 签出一个工作树，返回该目录（解析过符号链接）。
fn add_worktree(root: &Path) -> PathBuf {
    let name = format!("{}-feat", root.file_name().unwrap().to_string_lossy());
    let linked = root.with_file_name(name);
    let _ = std::fs::remove_dir_all(&linked);
    git_in(
        root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            linked.to_str().unwrap(),
        ],
    );
    winc_config::files::canonicalize(linked).unwrap()
}

/// 关闭已打开的对话框，重新打开新建工作树对话框并输入分支名 `name`。
fn fill_dialog(app: &mut App, name: &str) {
    app.close_form_dialog();
    app.open_worktree_dialog();
    // 留在当前目录，接着检查仓库里的变化。
    app.form_dialog.as_mut().unwrap().new_window = true;
    app.edit_field(|f| f.insert(name));
}

/// 在对话框里输入 `name`，按「创建」。
fn create(app: &mut App, name: &str) {
    fill_dialog(app, name);
    app.submit_form_dialog();
}

/// 返回对话框输入 `name` 后按「创建」会生成的计划。
fn plan(app: &mut App, name: &str) -> crate::app::git::form::FormPlan {
    fill_dialog(app, name);
    let plan = app.form_plan(app.form_dialog.as_ref().unwrap());
    app.close_form_dialog();
    plan
}

#[test]
fn worktrees_show_in_the_location_picker_and_the_branch_menu() {
    use crate::app::git::branch::BranchOp;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-worktrees", false) else {
        return;
    };
    // 只有一个工作树：没有位置行。
    assert!(!app.git_has_locations());

    let linked = add_worktree(&root);
    app.git_refresh();
    assert_eq!(app.git.worktrees.len(), 2);
    assert_eq!(app.git.current_worktree(), Some(0));

    // 位置选择器：两个工作树排在仓库下面，当前的选中，另一个写着分支。
    assert!(app.git_has_locations());
    app.run_action(Action::ShowCommit);
    app.open_location_picker();
    let rows = app.picker.location_rows.clone().unwrap();
    let places: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            crate::app::git::location_picker::LocationRow::Place(place) => Some(place.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(places.len(), 2);
    assert!(places[0].current);
    assert_eq!(places[1].label, winc_editor::display_name(&linked));
    assert_eq!(places[1].worktree, Some(1));
    // 操作菜单：主工作树和当前的工作树不能移除，附属的可以。
    app.open_context_menu(
        MenuTarget::Location(places[1].path.clone(), Some(1)),
        (0.0, 0.0),
    );
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::LocationNewWindow, true)));
    assert!(commands.contains(&(MenuCommand::LocationRemove, true)));
    app.open_context_menu(
        MenuTarget::Location(places[0].path.clone(), Some(0)),
        (0.0, 0.0),
    );
    assert!(
        !menu_commands(&app)
            .iter()
            .any(|(c, _)| *c == MenuCommand::LocationRemove)
    );
    app.context_menu = None;
    app.close_picker();

    // 分支菜单：`feat` 已被其他工作树签出，「签出」表示切换过去，且不能删除。
    let feat = app
        .git
        .branches
        .iter()
        .position(|b| b.name == "feat")
        .unwrap();
    app.open_context_menu(MenuTarget::BranchOps(feat), (0.0, 0.0));
    let commands = menu_commands(&app);
    let checkout = MenuCommand::GitBranch(BranchOp::Checkout, feat);
    assert!(commands.contains(&(checkout, true)));
    assert!(commands.contains(&(MenuCommand::GitBranch(BranchOp::Delete, feat), false)));
    assert!(app.menu_label(checkout).title.starts_with("切换到工作树「"));
    // 切换在当前窗口完成，和位置选择器、分支选择器一样。
    app.context_menu = None;
    choose_in_branch_menu(&mut app, checkout);
    assert_eq!(app.editor.root(), Some(linked.as_path()));
    app.switch_folder(&root);
    assert_eq!(app.editor.root(), Some(root.as_path()));
    app.git_refresh();

    // 新建工作树默认也在当前窗口打开，要新窗口时再勾选。
    app.open_worktree_dialog();
    assert!(!app.form_dialog.as_ref().unwrap().new_window);
    app.close_form_dialog();

    // 在当前窗口打开：工作区切换到该目录；在该工作树中查看，主工作树是另一项，`main` 被占用。
    app.open_worktree(1, false);
    assert_eq!(app.editor.root(), Some(linked.as_path()));
    app.git_refresh();
    assert_eq!(app.git.current_worktree(), Some(1));
    assert_eq!(
        app.git.main_worktree().map(|w| w.path.clone()),
        Some(root.clone())
    );
    assert_eq!(app.git.held_branches(), ["main"]);
    paint(&mut app);

    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn worktrees_are_created_removed_and_pruned() {
    use crate::app::git::branch::BranchOp;
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-worktree-life", false) else {
        return;
    };
    app.trash = delete_for_test;
    let trees = root.with_file_name(format!(
        "{}.worktrees",
        root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&trees);
    // 被忽略的 `.env` 按配置复制到新工作树。
    std::fs::write(root.join(".git").join("info").join("exclude"), ".env\n").unwrap();
    std::fs::write(root.join(".env"), "KEY=1").unwrap();
    app.config.git_worktree_copy = vec![".env".to_owned()];

    // 只有一个工作树：分支菜单中显示创建位置。
    assert_eq!(
        app.worktree_dir_hint("feat/x"),
        format!("{}/feat-x", winc_editor::display_name(&trees))
    );
    create(&mut app, "feat/x");
    assert!(app.form_dialog.is_none());
    let feat = trees.join("feat-x");
    assert_eq!(std::fs::read_to_string(feat.join(".env")).unwrap(), "KEY=1");
    assert_eq!(app.git.worktrees.len(), 2);
    assert_eq!(app.git.worktrees[1].branch(), Some("feat/x"));

    // 分支菜单：已有的分支签出到新工作树。
    git_in(&root, &["branch", "other"]);
    app.git_refresh();
    let other = app
        .git
        .branches
        .iter()
        .position(|b| b.name == "other")
        .unwrap();
    choose_in_branch_menu(
        &mut app,
        MenuCommand::GitBranch(BranchOp::NewWorktree, other),
    );
    assert_eq!(app.git.worktrees.len(), 3);
    assert!(trees.join("other").join(".git").is_file());

    // 对话框里输入已有的名字：已有分支直接签出，被其他工作树占用时切换过去，当前分支不可用。
    git_in(&root, &["branch", "spare"]);
    app.git_refresh();
    let spare = plan(&mut app, "spare");
    assert!(spare.enabled && !spare.base_enabled);
    assert_eq!((spare.confirm, spare.base.as_str()), ("创建", "现有分支"));
    let held = plan(&mut app, "feat/x");
    assert!(held.enabled && held.confirm == "打开");
    assert_eq!(
        held.dir
            .map(|d| winc_config::files::canonicalize(d).unwrap()),
        Some(winc_config::files::canonicalize(trees.join("feat-x")).unwrap())
    );
    let main = plan(&mut app, "main");
    assert!(!main.enabled);
    assert_eq!(
        main.problem.as_deref(),
        Some("当前分支无法在其他工作树中签出")
    );
    let fresh = plan(&mut app, "brand-new");
    assert!(fresh.enabled && fresh.base_enabled && fresh.problem.is_none());
    assert_eq!(fresh.base, "main（当前分支）");
    let empty = plan(&mut app, "");
    assert!(!empty.enabled && empty.problem.is_none());
    // 「-」开头的名字会被 `git branch` 当成选项：不建。
    let dash = plan(&mut app, "--unset-upstream");
    assert!(!dash.enabled);
    assert_eq!(dash.problem.as_deref(), Some("分支名称不能以「-」开头"));
    create(&mut app, "--unset-upstream");
    assert_eq!(app.git.worktrees.len(), 3);
    // 建不了时对话框保留，原因写在输入框里。
    assert!(app.form_dialog.is_some());
    create(&mut app, "main");
    assert_eq!(app.git.worktrees.len(), 3);
    create(&mut app, "spare");
    let at = |app: &App, dir: &Path| {
        let dir = winc_config::files::canonicalize(dir).unwrap();
        app.git
            .worktrees
            .iter()
            .position(|w| w.path == dir)
            .unwrap()
    };
    let spare = trees.join("spare");
    assert_eq!(app.git.worktrees[at(&app, &spare)].branch(), Some("spare"));
    // 日志里的提交：游离地签出，目录用短提交号。
    let oid = app.git.status.as_ref().unwrap().oid.clone().unwrap();
    app.new_worktree_at(oid.clone(), None);
    let short = &oid[..7];
    assert!(trees.join(short).join(".git").is_file());
    assert_eq!(app.git.worktrees.len(), 5);
    for dir in [trees.join(short), spare] {
        let i = at(&app, &dir);
        app.remove_worktree(i);
        assert!(!dir.exists());
    }
    assert_eq!(app.git.worktrees.len(), 3);

    // 移除：有改动也能移除（确认框写明），目录被删除，已合并的分支一起删掉。
    std::fs::write(feat.join("scratch.txt"), "x").unwrap();
    let i = at(&app, &feat);
    let target = MenuTarget::Location(feat.clone(), Some(i));
    app.open_context_menu(target, (0.0, 0.0));
    assert!(menu_commands(&app).contains(&(MenuCommand::LocationRemove, true)));
    app.context_menu = None;
    app.remove_worktree(i);
    assert!(!feat.exists());
    assert_eq!(app.git.worktrees.len(), 2);
    assert!(app.git.branches.iter().all(|b| b.name != "feat/x"));

    // 目录在外部被删除：标记为可清理，清理后只剩一个工作树。
    std::fs::remove_dir_all(trees.join("other")).unwrap();
    app.git_refresh();
    assert!(app.git.worktrees.iter().any(|w| w.prunable));
    // 该分支所在的工作树已被删除，git 仍不允许再次签出：签出项灰显，对话框提示先清理。
    assert_eq!(app.git.stale_branches(), ["other"]);
    assert!(app.git.held_branches().is_empty());
    let stale = plan(&mut app, "other").problem.unwrap_or_default();
    assert!(stale.contains("请先清理"), "{stale}");
    create(&mut app, "other");
    assert_eq!(app.git.worktrees.len(), 2);
    app.prune_worktrees();
    assert!(app.git.worktrees.len() < 2);

    let _ = std::fs::remove_dir_all(&trees);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：附属工作树的徽章，浅色与深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_worktree_badge() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("worktree-shot-{theme}"), false, dark)
        else {
            return;
        };
        let linked = add_worktree(&root);
        app.git_refresh();
        app.run_action(Action::ShowCommit);
        shoot(&mut app, &format!("worktree-main-{theme}"));
        app.open_worktree(1, false);
        app.git_refresh();
        shoot(&mut app, &format!("worktree-badge-{theme}"));
        let _ = std::fs::remove_dir_all(&linked);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 同一个仓库的工作树之间切换时，后台读完之前徽章的依据就是对的；换到别的仓库时不留。
#[test]
fn switching_worktrees_keeps_what_they_share_until_the_refresh() {
    let Some((mut app, root)) = git_app("git-worktree-keep", false) else {
        return;
    };
    let linked = add_worktree(&root);
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    app.git_refresh();
    assert!(app.git.main_worktree().is_none());

    app.git.clear_for(Some(linked.as_path()));
    app.git.root = Some(linked.clone());
    assert_eq!(
        app.git.main_worktree().map(|w| w.path.as_path()),
        Some(root.as_path())
    );
    assert_eq!(app.git.remotes, ["origin"]);
    // 分支列表还没读到，不提示发布。
    app.git.status = Some(winc_editor::git::Status {
        head: Some(Head::Branch("feat".to_owned())),
        ..Default::default()
    });
    assert_eq!(app.git_sync_state(), crate::app::SyncState::Hidden);

    app.git.clear_for(Some(Path::new("/elsewhere")));
    assert!(app.git.worktrees.is_empty());
    assert!(app.git.remotes.is_empty());

    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目录已不在的工作树和失效的分支一样：行内没有按钮，点击弹出菜单，菜单里只有清理。
#[test]
fn a_missing_worktree_is_cleaned_up_from_its_menu() {
    use crate::app::git::location_picker::LocationRow;
    use crate::app::input::context_menu::MenuCommand;

    let Some((mut app, root)) = git_app("git-location-missing", false) else {
        return;
    };
    let linked = add_worktree(&root);
    std::fs::remove_dir_all(&linked).unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.open_location_picker();
    let rows = app.picker.location_rows.clone().unwrap();
    app.picker.selected = rows
        .iter()
        .position(|row| matches!(row, LocationRow::Place(place) if place.missing))
        .unwrap();
    paint(&mut app);
    let more = PickerPart::More(app.picker.selected);
    assert!(app.hits.picker_parts.iter().any(|(_, part)| *part == more));

    app.location_picker_accept();
    assert_eq!(menu_commands(&app), [(MenuCommand::LocationPrune, true)]);
    assert!(app.picker_lists_locations());
    app.context_menu = None;
    app.close_picker();
    app.prune_worktrees();
    assert!(app.git.worktrees.iter().all(|w| !w.prunable));
    let _ = std::fs::remove_dir_all(&root);
}

/// 打开的目录里有多个仓库时项目就是这个目录，选中其中的附属工作树也不变。
#[test]
fn the_project_name_stays_the_folder_with_several_repositories() {
    let dir = search_folder("git-project-folder");
    let a = dir.join("a");
    let b = dir.join("b");
    for repo in [&a, &b] {
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        if !make_repo(repo) {
            return;
        }
    }
    let linked = dir.join("c");
    git_in(
        &a,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat",
            linked.to_str().unwrap(),
        ],
    );
    let mut app = app_with(&dir, false);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let folder = app.editor.workspace.as_ref().unwrap().name.clone();
    let index = (app.git.repositories.iter())
        .position(|repo| repo.ends_with("c"))
        .unwrap();
    app.git_switch_repository(index);
    app.git_refresh();
    assert!(app.git.main_worktree().is_some());
    assert_eq!(app.project_name(), Some(folder));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 同一个仓库的工作树之间切换时读过的状态留着，换到别的仓库时不留。
#[test]
fn only_worktrees_of_the_repository_share_its_state() {
    let Some((mut app, root)) = git_app("git-worktree-share", false) else {
        return;
    };
    let linked = add_worktree(&root);
    app.git_refresh();
    assert!(app.git.shares_repository(&linked));
    assert!(app.git.shares_repository(&root));
    assert!(!app.git.shares_repository(Path::new("/elsewhere")));
    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 只有一个仓库时工作树直接列出，没有分组标题和缩进。
#[test]
fn a_single_repository_lists_its_worktrees_flat() {
    use crate::app::git::location_picker::LocationRow;

    let Some((mut app, root)) = git_app("git-location-flat", false) else {
        return;
    };
    let linked = add_worktree(&root);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.open_location_picker();
    let rows = app.picker.location_rows.clone().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| match row {
        LocationRow::Place(place) => place.depth == 0,
        LocationRow::Repository { .. } => false,
    }));
    // 位置行的写法不受影响。
    app.close_picker();
    let main = winc_i18n::t!("git-location-main-worktree").to_owned();
    assert_eq!(app.git_location_label(), (main, None));
    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 状态栏的分支名弹出的是提交视图里同一个分支选择器，从状态栏往上展开：在里面切换、拉取、推送；
/// 工作树弹系统菜单，只有一个位置时没有这一项。
#[test]
fn the_status_bar_switches_worktrees_and_branches_from_menus() {
    use crate::app::input::context_menu::MenuCommand;
    use crate::app::{GitAnchor, PickerPart, StatusItem};

    let Some((mut app, root)) = git_app("git-status-menus", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    for branch in ["feature/a", "feature/b", "fix/one"] {
        git_in(&root, &["branch", branch]);
    }
    app.git_refresh();
    app.open_path(&root.join("src/main.rs"));
    paint(&mut app);
    let hit = |app: &App, item| app.hits.status.iter().any(|(_, i)| *i == item);
    assert!(hit(&app, StatusItem::Branch));
    assert!(!hit(&app, StatusItem::Worktree));

    // 文件视图里也能从状态栏打开：面板的底边在分支名上面，右边和它对齐。
    app.open_status_menu(StatusItem::Branch);
    assert!(app.popup().is_none());
    assert!(app.picker.open && app.picker_lists_branches());
    assert_eq!(app.picker.anchor, GitAnchor::Status);
    paint(&mut app);
    let (item, _) = *(app.hits.status.iter())
        .find(|(_, i)| *i == StatusItem::Branch)
        .unwrap();
    let card = app.layout.picker;
    assert!(card.bottom() <= item.y);
    assert!((card.right() - item.right()).abs() < 0.5 || card.right() <= item.right());
    // 没有远程：拉取、推送、获取都在，都灰显。
    for part in [PickerPart::Pull, PickerPart::Push, PickerPart::Fetch] {
        assert!(!app.picker_action_enabled(part), "{part:?}");
        assert!(
            (app.hits.picker_parts.iter()).any(|(_, p)| *p == part),
            "{part:?}"
        );
    }
    let index = |app: &App, name: &str| {
        let rows = app.picker.branch_rows.as_deref().unwrap();
        rows.iter().position(|row| {
            matches!(row, crate::app::git::branch::picker::BranchRow::Branch { index, .. }
                if app.git.branches[*index].name == name)
        })
    };
    // 「feature」目录收着：搜一下，选中切过去。
    app.picker.input.set_text("feature/a");
    app.picker_refresh();
    app.picker.selected = index(&app, "feature/a").unwrap();
    app.branch_picker_accept();
    assert_eq!(
        app.git.status.as_ref().unwrap().head,
        Some(Head::Branch("feature/a".to_owned()))
    );

    // 领先上游：三项都能用，悬停提示写明作用；推送真的推上去。
    let remote = root.with_extension("status-remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(&root, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_in(&root, &["push", "-q", "-u", "origin", "feature/a"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    app.git_refresh();
    paint(&mut app);
    app.open_status_menu(StatusItem::Branch);
    paint(&mut app);
    for part in [PickerPart::Pull, PickerPart::Push, PickerPart::Fetch] {
        assert!(app.picker_action_enabled(part), "{part:?}");
    }
    assert_eq!(
        app.menu_label(MenuCommand::GitFetch).subtitle.as_deref(),
        Some("下载远程更新，不改动本地分支")
    );
    // 当前分支那一行写的是上游。
    let current = (app.git.branches.iter()).find(|b| b.current).unwrap();
    assert_eq!(current.upstream.as_deref(), Some("origin/feature/a"));
    app.press_picker_part(PickerPart::Push);
    press_alert(&mut app, 0);
    assert!(!app.picker.open);
    assert_eq!(app.git_sync_state(), crate::app::SyncState::InSync);
    let _ = std::fs::remove_dir_all(&remote);

    let linked = add_worktree(&root);
    app.git_refresh();
    paint(&mut app);
    assert!(hit(&app, StatusItem::Worktree));
    app.open_status_menu(StatusItem::Worktree);
    let places = menu_commands(&app);
    assert!(places.contains(&(MenuCommand::StatusPlace(1), true)));
    assert!(places.contains(&(MenuCommand::NewWorktree, true)));
    app.run_menu_command(MenuCommand::StatusPlace(1));
    assert_eq!(app.editor.root(), Some(linked.as_path()));

    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 位置按钮和项目名在同一行，分支行上移，放不下时项目名和位置都不压到工具按钮。
#[test]
fn the_location_shares_the_row_with_the_project() {
    let Some((mut app, root)) = git_app("git-location-header", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let single = app.commit_parts();
    let linked = add_worktree(&root);
    app.git_refresh();
    paint(&mut app);
    let parts = app.commit_parts();
    assert_eq!(parts.branch, single.branch);
    assert_eq!(parts.message, single.message);

    let location = (app.hits.commit.iter())
        .find(|(_, t)| *t == CommitTarget::Location)
        .unwrap()
        .0;
    let tools = parts.tools[0].0;
    assert_eq!(location.y, tools.y);
    assert!(app.hits.project.right() <= location.x);
    assert!(location.right() <= tools.x);

    // 放得下时项目名不截短：按钮紧贴位置按钮，中间只有固定的间距。
    for name in ["w", "wi", "winc", "test", "project", "项目"] {
        app.editor.workspace.as_mut().unwrap().name = name.to_owned();
        paint(&mut app);
        let location = (app.hits.commit.iter())
            .find(|(_, t)| *t == CommitTarget::Location)
            .unwrap()
            .0;
        let gap = location.x - app.hits.project.right();
        assert!((gap - 4.0).abs() < 1.0, "{name}: {gap}");
    }

    app.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    let location = (app.hits.commit.iter())
        .find(|(_, t)| *t == CommitTarget::Location)
        .unwrap()
        .0;
    assert!(app.hits.project.right() <= location.x);
    assert!(location.right() <= app.commit_parts().tools[0].0.x);

    app.open_location_picker();
    paint(&mut app);
    assert!(app.layout.picker.y >= app.commit_parts().header.bottom());

    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 其他工作树有更改时位置行有悬停提示；没有时不提示。
#[test]
fn the_location_row_tells_when_other_worktrees_have_changes() {
    let Some((mut app, root)) = git_app("git-location-others", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    let linked = add_worktree(&root);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let tip = |app: &mut App| {
        let rect = (app.hits.commit.iter())
            .find(|(_, t)| *t == CommitTarget::Location)
            .unwrap()
            .0;
        app.on_mouse_move(900.0, 500.0);
        app.on_mouse_move(rect.x + 4.0, rect.y + 4.0);
        assert!(app.tooltip.since.is_some());
        crate::view::tests::tooltip(app).map(|(_, text)| text)
    };
    assert_eq!(tip(&mut app), None);

    // 刷新时其他位置的状态也重读，不用打开选择器。
    std::fs::write(linked.join("new.txt"), "x").unwrap();
    app.git_refresh();
    paint(&mut app);
    let text = tip(&mut app);
    assert_eq!(text.as_deref(), Some("其他 1 个工作树有未提交的更改"));

    // 选择器开着时每一行都写着。
    app.open_location_picker();
    paint(&mut app);
    assert_eq!(crate::view::tests::tooltip(&app), None);

    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 项目名始终是仓库，位置行写工作树：主工作树写「主工作树」，附属工作树写它自己的名字。
#[test]
fn the_location_row_does_not_repeat_the_project_name() {
    let Some((mut app, root)) = git_app("git-location-label", false) else {
        return;
    };
    let linked = add_worktree(&root);
    app.git_refresh();
    let name = |path: &Path| path.file_name().unwrap().to_string_lossy().into_owned();
    let main = winc_i18n::t!("git-location-main-worktree").to_owned();
    assert_eq!(app.git_location_label(), (main, None));
    assert_eq!(app.project_name(), Some(name(&root)));

    app.open_worktree(1, false);
    app.git_refresh();
    assert_eq!(app.git_location_label(), (name(&linked), None));
    assert_eq!(app.project_name(), Some(name(&root)));
    let _ = std::fs::remove_dir_all(&linked);
    let _ = std::fs::remove_dir_all(&root);
}

/// 对话框中的选择全部生效：从选定的起点新建、建在选定的目录下、不复制被忽略的文件、在当前窗口打开。
///
/// 键盘：`Esc` 关闭，回车创建；点击卡片外面也会关闭。
#[test]
fn the_new_worktree_dialog_builds_what_it_shows() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-worktree-dialog", false) else {
        return;
    };
    std::fs::write(root.join(".git").join("info").join("exclude"), ".env\n").unwrap();
    std::fs::write(root.join(".env"), "KEY=1").unwrap();
    app.config.git_worktree_copy = vec![".env".to_owned()];
    git_in(&root, &["branch", "base"]);
    git_in(
        &root,
        &["commit", "-q", "--allow-empty", "-m", "after base"],
    );
    app.git_refresh();
    let folder = root.with_file_name(format!(
        "{}-picked",
        root.file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let folder = winc_config::files::canonicalize(folder).unwrap();

    // 打开时焦点在分支名上；关闭后焦点回到原来的位置。
    let before = app.focus;
    app.open_worktree_dialog();
    assert_eq!(app.focus, Focus::Dialog);
    app.close_form_dialog();
    assert_eq!(app.focus, before);
    assert!(app.form_dialog.is_none());

    // 点击卡片外面关闭对话框。
    fill_dialog(&mut app, "x");
    let card = app.dialog_layout().card;
    app.mouse = (card.x - 10.0, card.y - 10.0);
    app.click_form_dialog(card.x - 10.0, card.y - 10.0);
    assert!(app.form_dialog.is_none());

    // 「基于」菜单：当前分支打勾，其余分支都在；选了「base」。
    fill_dialog(&mut app, "from-base");
    app.open_context_menu(MenuTarget::FormBase, (0.0, 0.0));
    let base = app
        .git
        .branches
        .iter()
        .position(|b| b.name == "base")
        .unwrap();
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::FormBase(0), true)));
    assert!(commands.contains(&(MenuCommand::FormBase(base + 1), true)));
    assert!(app.menu_checked(MenuCommand::FormBase(0)));
    assert_eq!(
        app.menu_label(MenuCommand::FormBase(0)).title,
        "main（当前分支）"
    );
    app.run_menu_command(MenuCommand::FormBase(base + 1));
    assert!(app.menu_checked(MenuCommand::FormBase(base + 1)));
    app.worktree_folder_picked(folder.clone());
    let dialog = app.form_dialog.as_mut().unwrap();
    dialog.copy = false;
    dialog.new_window = false;
    let shown = app.form_plan(app.form_dialog.as_ref().unwrap());
    assert_eq!(shown.base, "base");
    assert_eq!(shown.dir, Some(folder.join("from-base")));
    app.submit_form_dialog();

    // 建在选定的目录下，从 `base` 开始，不复制 `.env`，当前窗口切换过去。
    let made = folder.join("from-base");
    assert!(made.join(".git").is_file());
    assert!(!made.join(".env").exists());
    assert_eq!(app.editor.root(), Some(made.as_path()));
    let head = |dir: &Path| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let base_oid = {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "base"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(head(&made), base_oid);

    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn form_dialog_moves_focus_with_tab_and_keeps_failures_in_the_card() {
    use crate::layout::DialogPart;

    let Some((mut app, root)) = git_app("git-form-keys", false) else {
        return;
    };
    let key = |app: &mut App, key: NamedKey| {
        assert!(app.dialog_key(&Key::Named(key), "", false));
    };
    app.open_worktree_dialog();
    app.edit_field(|f| f.insert("feat/keys"));
    let focus = |app: &App| app.form_dialog.as_ref().unwrap().focus;
    // `Tab` 的顺序：名称 → 基于 → 选择目录 → 在新窗口中打开 → 取消 → 创建 → 名称；`⇧Tab` 逆序。
    let mut order = Vec::new();
    for _ in 0..6 {
        key(&mut app, NamedKey::Tab);
        order.push(focus(&app));
    }
    assert_eq!(
        order,
        [
            DialogPart::Base,
            DialogPart::Change,
            DialogPart::NewWindow,
            DialogPart::Cancel,
            DialogPart::Confirm,
            DialogPart::Branch,
        ]
    );
    // 空格切换焦点所在的勾选项；焦点不在名称上时，键入的文字不进入输入框。
    app.modifiers = ModifiersState::SHIFT;
    key(&mut app, NamedKey::Tab);
    key(&mut app, NamedKey::Tab);
    key(&mut app, NamedKey::Tab);
    app.modifiers = ModifiersState::empty();
    assert_eq!(focus(&app), DialogPart::NewWindow);
    key(&mut app, NamedKey::Space);
    assert!(app.form_dialog.as_ref().unwrap().new_window);
    let name = |app: &App| app.form_dialog.as_ref().unwrap().name.text().to_owned();
    assert!(app.dialog_key(&Key::Character("x".into()), "x", false));
    assert_eq!(name(&app), "feat/keys");
    paint(&mut app);

    // git 拒绝名称时保留对话框并在其中显示原因，不另行通知；修改名称后清空原因。
    app.close_form_dialog();
    app.open_branch_dialog(None, None);
    app.edit_field(|f| f.insert("bad..name"));
    app.submit_form_dialog();
    let dialog = app.form_dialog.as_ref().expect("stays open");
    assert!(dialog.running.is_none());
    assert!(dialog.error.is_some());
    assert_eq!(dialog.focus, DialogPart::Branch);
    paint(&mut app);
    assert!(!app.dialog_key(&Key::Named(NamedKey::Backspace), "", false));
    assert!(app.form_dialog.as_ref().unwrap().error.is_none());

    // 操作成功后才关闭对话框。
    app.edit_field(|f| {
        f.set_text("good-name");
        true
    });
    app.submit_form_dialog();
    assert!(app.form_dialog.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：新建工作树对话框的多种状态及新建分支失败时的对话框，浅色与深色各一张。
///
/// 状态包括空白、已填写、选中当前分支（错误色且「基于」不可选）和键盘焦点位于选项上。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_worktree_dialog() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("worktree-dialog-{theme}"), false, dark)
        else {
            return;
        };
        app.config.git_worktree_copy = vec![".env*".to_owned(), "config/local.toml".to_owned()];
        app.open_worktree_dialog();
        shoot(&mut app, &format!("worktree-dialog-empty-{theme}"));
        app.edit_field(|f| f.insert("feat/login-page"));
        shoot(&mut app, &format!("worktree-dialog-{theme}"));
        // 当前分支不能再签出：状态行是错误色，「创建」灰显，「基于」不可选。
        app.edit_field(|f| {
            f.set_text("main");
            true
        });
        app.form_dialog.as_mut().unwrap().new_window = false;
        shoot(&mut app, &format!("worktree-dialog-error-{theme}"));
        // 键盘焦点位于「在新窗口中打开」时，勾选框外绘制一圈强调色细线。
        app.edit_field(|f| {
            f.set_text("feat/login-page");
            true
        });
        app.form_dialog.as_mut().unwrap().focus = crate::layout::DialogPart::NewWindow;
        shoot(&mut app, &format!("worktree-dialog-focus-{theme}"));
        // git 拒绝名称时保留对话框，并在按钮左侧显示原因。
        app.close_form_dialog();
        app.open_branch_dialog(None, None);
        app.edit_field(|f| f.insert("bad..name"));
        app.submit_form_dialog();
        shoot(&mut app, &format!("branch-dialog-failed-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
