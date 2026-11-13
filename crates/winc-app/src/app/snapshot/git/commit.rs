//! 提交视图的测试：改动列表、勾选、提交、菜单、提交信息、滚动与固定行、折叠、长文件名。

use super::*;
use std::time::{Duration, Instant};

use crate::app::SyncState;
use crate::app::animation::{Clock, TREE_ANIMATION};
use crate::app::git::commit::COMMITTED_SHOWN;

mod checks;
mod names;
mod stash;
mod sync;

/// 创建含大量深层新文件（已暂存）的仓库：`assets/pets/a`、`assets/pets/b` 下各 12 张图，
/// 另加 3 个散文件。
fn deep_changes_repo(name: &str) -> Option<PathBuf> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "all"]);
    for sub in ["a", "b"] {
        let folder = dir.join("assets/pets").join(sub);
        std::fs::create_dir_all(&folder).unwrap();
        for i in 0..12 {
            std::fs::write(folder.join(format!("{i}.png")), "x").unwrap();
        }
    }
    for f in ["arm_L.png", "arm_R.png", "bg.png"] {
        std::fs::write(dir.join("assets/pets").join(f), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
    Some(dir)
}

/// 在 [`deep_changes_repo`] 的基础上新增一个名字很长的已暂存文件，返回仓库目录与文件名。
fn long_name_repo(name: &str) -> Option<(PathBuf, String)> {
    let dir = deep_changes_repo(name)?;
    let long = "a-very-long-file-name-that-never-fits-in-the-sidebar-at-all.png".to_owned();
    std::fs::write(dir.join("assets/pets/a").join(&long), "x").unwrap();
    git_in(&dir, &["add", "-A"]);
    Some((dir, long))
}

#[test]
fn right_clicking_a_changed_file_offers_diff_open_and_rollback() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some((mut app, root)) = git_app("git-commit-menu", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // `make_repo` 修改过 `src/main.rs`：右键该文件所在的行。
    let main = root.join("src/main.rs");
    let index = app
        .git
        .status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|e| e.path == main)
        .unwrap();
    let row = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::File(index))
        .unwrap()
        .0;
    right_click(&mut app, (row.x + 120.0, row.y + row.h / 2.0));
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::CommitFile(main.clone()))
    );
    assert_eq!(app.git.commit.selected.as_deref(), Some(main.as_path()));
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::GitShowDiff,
            MenuCommand::GitOpenFile,
            MenuCommand::GitRollbackFile,
            MenuCommand::RevealInTree,
            MenuCommand::Reveal,
            MenuCommand::CopyPath,
            MenuCommand::CopyRelativePath,
        ]
    );

    // 「回滚」只作用于该文件（没有窗口时不弹确认框）。
    let before = std::fs::read_to_string(&main).unwrap();
    app.run_menu_command(MenuCommand::GitRollbackFile);
    for _ in 0..200 {
        app.poll_commit();
        if std::fs::read_to_string(&main).unwrap() != before {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(std::fs::read_to_string(&main).unwrap(), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hovering_a_change_offers_open_and_revert() {
    let Some((mut app, root)) = git_app("git-commit-row-actions", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let main = root.join("src/main.rs");
    let entries = |app: &App| app.git.status.as_ref().unwrap().entries.clone();
    let index = entries(&app).iter().position(|e| e.path == main).unwrap();
    let find =
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).map(|h| h.0);
    // 未悬停时不显示按钮；悬停该行时右端显示「打开文件」「回滚」，两者均有悬停提示。
    assert!(find(&app, CommitTarget::OpenFile(index)).is_none());
    let row = find(&app, CommitTarget::File(index)).unwrap();
    app.on_mouse_move(row.x + 120.0, row.y + row.h / 2.0);
    paint(&mut app);
    let open = find(&app, CommitTarget::OpenFile(index)).unwrap();
    let revert = find(&app, CommitTarget::RevertFile(index)).unwrap();
    assert!(open.right() <= revert.x && revert.right() <= row.right());
    app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
    assert_eq!(
        app.pointer.hover,
        Hover::Commit(CommitTarget::RevertFile(index))
    );
    assert!(app.pointer.hover.has_tooltip());

    app.press_commit(CommitTarget::OpenFile(index));
    let canonical = winc_editor::canonical_path(&main);
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(canonical.as_path())
    );
    assert_eq!(app.focus, Focus::Editor);
    // 没有窗口时不弹出确认框，直接回滚。
    app.press_commit(CommitTarget::RevertFile(index));
    app.git_refresh();
    assert!(entries(&app).iter().all(|e| e.path != main));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：悬停改动行时右端的「打开文件」「回滚」，鼠标位于「回滚」上（`commit-row-actions`）。
#[test]
#[ignore]
fn shoot_commit_row_actions() {
    let Some((mut app, root)) = git_app("git-commit-row-actions-shot", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let main = root.join("src/main.rs");
    let entries = app.git.status.as_ref().unwrap().entries.clone();
    let index = entries.iter().position(|e| e.path == main).unwrap();
    let find =
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let row = find(&app, CommitTarget::File(index));
    app.on_mouse_move(row.x + 120.0, row.y + row.h / 2.0);
    paint(&mut app);
    let revert = find(&app, CommitTarget::RevertFile(index));
    app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
    app.overlays.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
    shoot(&mut app, "commit-row-actions");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：提交视图，深浅两套配色。
#[test]
#[ignore]
fn shoot_commit_view() {
    for dark in [false, true] {
        let dir = search_folder(if dark { "commit-dark" } else { "commit-light" });
        if !long_repo(&dir) {
            return;
        }
        std::fs::write(dir.join("src/config.rs"), "pub struct Config;\n").unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/notes.md"), "# notes\n").unwrap();
        std::fs::create_dir_all(dir.join("src/core/model")).unwrap();
        std::fs::write(dir.join("src/core/model/user.rs"), "pub struct User;\n").unwrap();
        std::fs::write(dir.join("src/core/model/order.rs"), "pub struct Order;\n").unwrap();
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        app.edit_field(|f| f.insert("修复配置读取\n\n启动时配置文件不存在不再错误，改用默认值。"));
        // 选中 `src/main.rs`：列表中高亮，右侧预览该文件的差异。
        let main = app.editor.root().unwrap().join("src/main.rs");
        let index = app
            .git
            .status
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|e| e.path == main);
        paint(&mut app);
        app.press_commit(CommitTarget::File(index.unwrap()));
        app.focus = Focus::CommitMessage;
        // 鼠标停在标题行的「Git 日志」按钮上，提示已显示。
        let log = app.commit_parts().tools[0].0;
        app.on_mouse_move(log.x + 4.0, log.y + 4.0);
        app.overlays.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, if dark { "commit-dark" } else { "commit-light" });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：侧栏最窄（230）和默认宽度（300）下的提交视图，分支名很长（`commit-width-*`），检查第二、
/// 三行和提交卡片底行没有互相压住：最窄时「修改上次提交」收进按钮的菜单。另有领先上游、没有更改时
/// 的空状态和下一步按钮（`commit-width-clean-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_widths() {
    for width in [
        crate::theme::MIN_PANEL_WIDTH,
        crate::theme::DEFAULT_PANEL_WIDTH,
    ] {
        let name = format!("commit-width-{width}");
        let Some((mut app, root)) = git_app(&name, false) else {
            return;
        };
        git_in(
            &root,
            &[
                "switch",
                "-q",
                "-c",
                "feature/login-lockout-after-three-tries",
            ],
        );
        app.panels.sidebar.width = width;
        app.git_refresh();
        app.run_action(Action::ShowCommit);
        app.edit_field(|f| f.insert("修复日志刷新没有反馈"));
        shoot(&mut app, &name);

        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "修复日志刷新没有反馈"]);
        git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
        git_in(
            &root,
            &["remote", "add", "origin", "https://example.invalid/r.git"],
        );
        let upstream = "refs/remotes/origin/feature/login-lockout-after-three-tries";
        git_in(&root, &["update-ref", upstream, "HEAD~1"]);
        let set = "--set-upstream-to=origin/feature/login-lockout-after-three-tries";
        git_in(&root, &["branch", "-q", set]);
        app.git_refresh();
        shoot(&mut app, &format!("commit-width-clean-{width}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 截图：动画途中的三帧：储藏一组拉开到三分之一，提交完以后提交卡片滑出到一半，推送以后
/// 「上一次提交」收起到三分之一。
#[test]
#[ignore]
fn shoot_commit_motion() {
    let Some((mut app, root)) = git_app("commit-motion", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    for note in ["改登录", "试一下新写法", "调日志刷新"] {
        std::fs::write(
            root.join("src/main.rs"),
            format!("fn main() {{ {note:?}; }}\n"),
        )
        .unwrap();
        git_in(&root, &["stash", "push", "-q", "-m", note]);
    }
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let closed = app.commit_parts().stashes.unwrap().h;
    app.press_commit(CommitTarget::Stashes);
    app.git.commit.stash_motion = Some((closed, Clock::frozen(TREE_ANIMATION / 3, TREE_ANIMATION)));
    shoot(&mut app, "commit-stash-opening");

    app.git.commit.stash_motion = None;
    git_in(&root, &["commit", "-q", "-am", "now"]);
    app.git_refresh();
    app.git.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    shoot(&mut app, "commit-card-leaving");

    // 推送了：「上一次提交」收起到一半。
    app.git.commit.card.motion = None;
    git_in(
        &root,
        &["remote", "add", "origin", "https://example.invalid/r.git"],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.track_commit_bars();
    app.git_refresh();
    app.git.commit.undo.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 3, TREE_ANIMATION)));
    shoot(&mut app, "commit-undo-folding");
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：没有改动的提交视图（居中的「没有更改」，没有提交卡片），深浅两套配色。
#[test]
#[ignore]
fn shoot_commit_empty() {
    for dark in [false, true] {
        let name = if dark {
            "commit-empty-dark"
        } else {
            "commit-empty-light"
        };
        let dir = search_folder(name);
        if !make_repo(&dir) {
            return;
        }
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "all"]);
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_the_module_off_takes_focus_out_of_the_commit_view() {
    let Some((mut app, root)) = git_app("git-focus", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run_action(Action::ShowCommit);
    assert_eq!(app.focus, Focus::CommitMessage);
    app.settings_toggle(Setting::GitModule);
    assert_eq!(app.panels.sidebar.view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Editor);
    // 键入的字符不会进入已隐藏的提交信息。
    app.edit_field(|f| f.insert("x"));
    assert_eq!(app.git.commit.message.text(), "");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn clicks_on_rows_drawn_before_a_refresh_are_ignored() {
    let Some((mut app, root)) = git_app("git-stale-click", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let before = app
        .git
        .commit
        .checked(&app.git.status.as_ref().unwrap().entries[0]);
    // 状态在绘制以后刷新过：按上一帧记下的行号点，不处理。
    git_in(&root, &["add", "src/new.rs"]);
    app.git_refresh();
    app.press_commit(CommitTarget::Check(0));
    assert_eq!(
        app.git
            .commit
            .checked(&app.git.status.as_ref().unwrap().entries[0]),
        before
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_more_menu_greys_out_what_does_not_apply() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-commit-more", false) else {
        return;
    };
    std::fs::write(root.join("new.txt"), "x").unwrap();
    git_in(&root, &["add", "-A"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let enabled = |app: &App, command| {
        app.commit_more_menu()
            .contains(&MenuEntry::Item(command, true))
    };
    // 先清空勾选：列表初始全部展开，且没有任何文件被勾选。
    for group in CommitGroup::ALL {
        for _ in 0..3 {
            let status = app.git.status.clone().unwrap();
            if app.git.commit.group_state(&status, group).is_none() {
                break;
            }
            app.press_commit(CommitTarget::GroupCheck(group));
        }
    }
    assert!(enabled(&app, MenuCommand::CommitTree));
    assert!(!enabled(&app, MenuCommand::CommitExpandAll));
    assert!(enabled(&app, MenuCommand::CommitCollapseAll));
    // 没勾文件时悬停组标题，右端仍是文件数，没有「回滚」。
    let reverts = |app: &mut App| {
        let (band, _) = *(app.hits.commit.iter())
            .find(|(_, t)| *t == CommitTarget::Group(CommitGroup::Changes))
            .unwrap();
        app.on_mouse_move(band.x + band.w / 2.0, band.y + band.h / 2.0);
        paint(app);
        (app.hits.commit.iter()).any(|(_, t)| matches!(t, CommitTarget::GroupRevert(_)))
    };
    assert!(!reverts(&mut app));

    app.run_commit_more_command(MenuCommand::CommitCollapseAll);
    let groups = [CommitGroup::Changes];
    assert!(groups.iter().all(|g| app.git.commit.group_collapsed(*g)));
    assert!(enabled(&app, MenuCommand::CommitExpandAll));
    assert!(!enabled(&app, MenuCommand::CommitCollapseAll));
    app.run_commit_more_command(MenuCommand::CommitExpandAll);
    assert!(groups.iter().all(|g| !app.git.commit.group_collapsed(*g)));

    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    paint(&mut app);
    assert!(reverts(&mut app));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_message_grows_with_its_text_and_the_list_follows() {
    let Some((mut app, root)) = git_app("git-message-grow", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 自上而下依次是改动列表和底部的提交卡片；卡片里提交信息在上，底行左边是「修改上次提交」，
    // 右边是「提交 ⌄」。空信息时高度为两行。
    let one = app.commit_parts();
    let card = one.card.unwrap();
    assert_eq!(one.message.h, 2.0 * 20.0 + 4.0);
    assert_eq!(card.bottom(), app.layout.sidebar.bottom() - 8.0);
    assert!(one.list.bottom() <= card.y);
    assert!(one.commit.y >= one.message.bottom());
    assert_eq!(one.commit_menu.x, one.commit.right());
    assert!(one.commit_menu.right() <= card.right());
    let amend = one.amend.unwrap();
    assert!(amend.right() < one.commit.x);

    // 写入三行时框高增加一行，卡片往上长，列表随之变短。
    app.edit_field(|f| f.insert("修复配置读取\n\n启动时不再报错"));
    paint(&mut app);
    let three = app.commit_parts();
    assert_eq!(three.message.h, 3.0 * 20.0 + 4.0);
    assert_eq!(three.card.unwrap().bottom(), card.bottom());
    assert_eq!(one.list.bottom() - three.list.bottom(), 20.0);

    // 行数超过八行后不再长高，改为在框内滚动。
    app.edit_field(|f| f.insert(&"\n一行".repeat(20)));
    paint(&mut app);
    assert_eq!(app.commit_parts().message.h, 8.0 * 20.0 + 4.0);

    // 清空后框高从八行过渡回两行：途中介于两者之间，文字仍按两行的高度排版。
    use crate::app::animation::{Clock, TREE_ANIMATION};
    app.git.commit.message.set_text("");
    paint(&mut app);
    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.git.commit.message_motion = Some((8.0 * 20.0 + 4.0, half));
    let parts = app.commit_parts();
    assert!(parts.message.h > 44.0 && parts.message.h < 8.0 * 20.0 + 4.0);
    assert_eq!(parts.message_settled.h, 44.0);
    assert!(app.animating());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn clicking_a_visible_change_under_the_pinned_rows_does_not_scroll() {
    let Some(dir) = deep_changes_repo("git-sticky-click") else {
        return;
    };
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rows = app.commit_rows();
    let list = app.commit_parts().list;
    let b = rows
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "b"))
        .unwrap();
    app.git.commit.scroll = (b + 4) as f32 * COMMIT_ROW;
    paint(&mut app);
    let covered = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.rect.bottom())
        .fold(list.y, f32::max);
    // 固定行下方露出的第一个文件：点击该行，列表不动，固定的行也不变。
    let before = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.row)
        .collect::<Vec<_>>();
    let (i, _) = rows
        .iter()
        .enumerate()
        .find(|(i, r)| {
            matches!(r, CommitRow::File { .. })
                && list.y + *i as f32 * COMMIT_ROW - app.git.commit.scroll >= covered
        })
        .unwrap();
    let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
    let scroll = app.git.commit.scroll;
    click(&mut app, (list.x + 160.0, y));
    paint(&mut app);
    assert_eq!(app.git.commit.scroll, scroll);
    let after = app
        .commit_sticky(&rows)
        .iter()
        .map(|s| s.row)
        .collect::<Vec<_>>();
    assert_eq!(after, before);

    // 被向上推的固定行只占自己那一格：可点击区域不伸入上一层的固定行。
    for step in 0..(rows.len() * 4) {
        app.git.commit.scroll = step as f32 * COMMIT_ROW / 4.0;
        paint(&mut app);
        for s in app.commit_sticky(&rows) {
            let slot = list.y + s.level as f32 * COMMIT_ROW;
            for (rect, target) in &app.hits.commit {
                if *target == CommitTarget::Sticky(s.row, s.level) {
                    assert!(rect.y >= slot - 0.01, "{step}: {rect:?} above {slot}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：大量深层文件的改动列表：滚到中间时固定在顶部的组、目录和缩进参考线，以及平铺列表。
#[test]
#[ignore]
fn shoot_deep_changes() {
    for dark in [false, true] {
        let name = if dark { "deep-dark" } else { "deep-light" };
        let Some(dir) = deep_changes_repo(name) else {
            return;
        };
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        app.git.commit.scroll = 12.0 * COMMIT_ROW;
        shoot(&mut app, &format!("{name}-tree"));
        // 固定的行被下一个目录向上推：只在自己那一格内移动。
        let rows = app.commit_rows();
        let list = app.commit_parts().list;
        let pushed = (0..rows.len() * 4)
            .map(|q| q as f32 * COMMIT_ROW / 4.0)
            .find(|&scroll| {
                app.git.commit.scroll = scroll;
                app.commit_sticky(&rows)
                    .iter()
                    .any(|s| s.level > 0 && s.rect.y < list.y + s.level as f32 * COMMIT_ROW - 8.0)
            });
        if let Some(scroll) = pushed {
            app.git.commit.scroll = scroll;
            shoot(&mut app, &format!("{name}-pushed"));
        }
        app.toggle_commit_layout();
        app.git.commit.scroll = 0.0;
        shoot(&mut app, &format!("{name}-flat"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn the_changes_list_scrollbar_can_be_dragged() {
    use crate::layout::Scrollbar;

    let Some(dir) = deep_changes_repo("git-commit-scrollbar") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let max = app.commit_max_scroll();
    assert!(max > 0.0);
    let selected = app.git.commit.selected.clone();

    // 行的右端（状态字母所在处）就是滚动条：悬停认作滚动条，按下拖到底再拖回顶部，不选中文件。
    let list = app.commit_parts().list;
    let x = list.right() - 3.0;
    app.on_mouse_move(x, list.y + 10.0);
    assert_eq!(app.pointer.hover, Hover::Scrollbar(Scrollbar::Commit));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, list.bottom() + 40.0);
    assert_eq!(app.git.commit.scroll, max);
    app.on_mouse_move(x, list.y - 40.0);
    assert_eq!(app.git.commit.scroll, 0.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.git.commit.selected, selected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn change_list_keys_jump_to_the_ends_and_page() {
    let Some(dir) = deep_changes_repo("git-commit-paging") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.focus = Focus::CommitList;
    let key = |app: &mut App, key: NamedKey| app.commit_list_key(&Key::Named(key));
    let files = app.commit_files();
    let rows = app.commit_rows();
    let row_of = |app: &App| {
        let selected = app.git.commit.selected.clone();
        rows.iter()
            .position(|r| match r {
                CommitRow::File { index, .. } => {
                    let status = app.git.status.as_ref().unwrap();
                    Some(&status.entries[*index].path) == selected.as_ref()
                }
                _ => false,
            })
            .unwrap()
    };

    // `Home`、`End` 移到第一个、最后一个文件（跳过组、目录行），列表随之滚动到可见位置。
    key(&mut app, NamedKey::End);
    assert_eq!(app.git.commit.selected.as_ref(), files.last());
    assert!(app.git.commit.scroll > 0.0);
    key(&mut app, NamedKey::Home);
    assert_eq!(app.git.commit.selected.as_ref(), files.first());
    assert_eq!(app.git.commit.scroll, 0.0);

    // 翻页距离至少为可见整行数减一，且落在文件上；到达末尾时停在最后一个文件。
    let page = (app.commit_parts().list.h / COMMIT_ROW).floor() as usize - 1;
    let start = row_of(&app);
    key(&mut app, NamedKey::PageDown);
    let next = row_of(&app);
    assert!(next >= start + page, "{start} → {next}，一页 {page}");
    key(&mut app, NamedKey::PageUp);
    assert!(row_of(&app) <= next - page);
    for _ in 0..rows.len() {
        key(&mut app, NamedKey::PageDown);
    }
    assert_eq!(app.git.commit.selected.as_ref(), files.last());

    // 按住 `⇧` 翻页时扩展选区。
    key(&mut app, NamedKey::Home);
    app.pointer.modifiers = ModifiersState::SHIFT;
    key(&mut app, NamedKey::PageDown);
    assert!(app.commit_multi_selected());
    app.pointer.modifiers = ModifiersState::empty();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 所有视图共用一个侧栏宽度：切换不变，用真实的鼠标事件拖动分隔条，新窗口、下次启动也沿用。
#[test]
fn the_sidebar_keeps_one_width_across_views() {
    use crate::theme::DEFAULT_PANEL_WIDTH;

    let Some((mut app, root)) = git_app("git-commit-width", false) else {
        return;
    };
    paint(&mut app);
    assert_eq!(app.panels.sidebar.width, DEFAULT_PANEL_WIDTH);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.panels.sidebar.width, DEFAULT_PANEL_WIDTH);

    // 提交视图里按住分隔条拖动：列表、提交信息框旁边都能拖。
    let drag = |app: &mut App, y: f32, by: f32| {
        let x = app.layout.splitter.x + 1.0;
        app.on_mouse_move(x, y);
        assert_eq!(app.pointer.hover, Hover::Splitter);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x + by, y);
        app.on_mouse_up(MouseButton::Left);
        paint(app);
    };
    let message = app.commit_parts().message;
    drag(&mut app, message.y + 4.0, 40.0);
    let dragged = DEFAULT_PANEL_WIDTH + 40.0;
    assert_eq!(app.panels.sidebar.width, dragged);
    assert_eq!(app.state.panel_width, Some(dragged));

    // 切到文件树、搜索还是这个宽度。
    app.show_files();
    assert_eq!(app.panels.sidebar.width, dragged);
    app.set_sidebar_view(SidebarView::Search);
    assert_eq!(app.panels.sidebar.width, dragged);

    // 文件树里拖动，切回提交视图也是拖过的宽度。
    app.show_files();
    paint(&mut app);
    drag(&mut app, 400.0, -20.0);
    let dragged = dragged - 20.0;
    assert_eq!(app.state.panel_width, Some(dragged));
    app.run_action(Action::ShowCommit);
    assert_eq!(app.panels.sidebar.width, dragged);

    // 记下的宽度比这个窗口放得下的大：从看到的宽度开始拖，一动就有反应。
    app.panels.sidebar.width = 5000.0;
    paint(&mut app);
    let shown = app.layout.window.w - 320.0;
    drag(&mut app, 400.0, -20.0);
    assert_eq!(app.panels.sidebar.width, shown - 20.0);

    // 新窗口、下次启动从状态恢复，太窄的值抬到最小宽度。
    app.state.panel_width = Some(10.0);
    app.restore_panel_width();
    assert_eq!(app.panels.sidebar.width, crate::theme::MIN_PANEL_WIDTH);

    let _ = std::fs::remove_dir_all(&root);
}

/// 只提交部分行的改动：截图用于检查行勾选框、整段勾选框的「部分」状态和不提交行的底色。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_partial_lines() {
    let dir = search_folder("git-lines-shot");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_5 = 500;\n",
            "let value_5 = 500;\nlet added_a = 1;\nlet added_b = 2;\nlet added_c = 3;\n",
        ),
    )
    .unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    shoot(&mut app, "diff-partial-lines");
    app.press_diff(DiffTarget::Layout);
    shoot(&mut app, "diff-partial-lines-unified");
}

/// 回归：滚动到长列表下方后提交，剩下的改动比滚动位置少，列表不能一片空白。
#[test]
fn a_shorter_list_is_visible_after_scrolling_far_down() {
    let Some(dir) = deep_changes_repo("git-commit-shrink") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.show_commit();
    paint(&mut app);
    let max = app.commit_max_scroll();
    assert!(max > 0.0, "样本需要超出列表高度");
    app.git.commit.scroll = max;
    paint(&mut app);

    // 只留下一个改动：其余的全部提交到仓库。
    let keep = dir.join("kept-change.txt");
    git_in(&dir, &["add", "--all"]);
    git_in(
        &dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "all",
        ],
    );
    std::fs::write(&keep, "x").unwrap();
    app.git_refresh();
    paint(&mut app);

    assert!(app.git.commit.scroll <= app.commit_max_scroll());
    assert_eq!(app.git.commit.scroll, 0.0);
    assert!(
        (app.hits.commit.iter()).any(|(_, target)| matches!(target, CommitTarget::File(_))),
        "列表应当画出剩下的改动"
    );
}

/// 「⋯」的分组：储藏，远程仓库，视图，输出；同步、新建分支和工作树只在分支选择器里。
#[test]
fn the_commit_more_menu_keeps_each_action_in_one_group() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-commit-more-groups", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let menu = app.commit_more_menu();
    let groups: Vec<Vec<Option<MenuCommand>>> = (menu.split(|e| *e == MenuEntry::Separator))
        .map(|group| {
            (group.iter())
                .map(|entry| match entry {
                    MenuEntry::Item(command, _) => Some(*command),
                    _ => None,
                })
                .collect()
        })
        .collect();
    let at = |command| (groups.iter()).position(|group| group.contains(&Some(command)));
    assert_eq!(at(MenuCommand::GitStash), Some(0));
    assert_eq!(at(MenuCommand::CommitTree), Some(groups.len() - 2));
    assert_eq!(at(MenuCommand::CommitExpandAll), Some(groups.len() - 2));
    assert_eq!(at(MenuCommand::GitShowOutput), Some(groups.len() - 1));
    // 刷新是标题行上的按钮，拉取、推送、获取和新建工作树在分支选择器里，这里都不重复。
    for command in [
        MenuCommand::GitFetch,
        MenuCommand::GitPush,
        MenuCommand::NewWorktree,
    ] {
        assert_eq!(at(command), None, "{command:?}");
    }
    // 没有上游时两种拉取都写明原因。
    for command in [MenuCommand::GitPull, MenuCommand::GitPullRebase] {
        assert!(app.menu_label(command).subtitle.is_some(), "{command:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 「刷新」点下去以后至少转满一圈：读取几十毫秒就结束，只按读取的时长转看不出点过。
#[test]
fn the_refresh_button_spins_for_at_least_one_turn() {
    let Some((mut app, root)) = git_app("git-commit-refresh-spin", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(!app.commit_refreshing());
    app.press_commit(CommitTarget::Refresh);
    // 测试里读取就地完成，这时转的只是至少一圈的时长。
    assert!(app.commit_refreshing());
    assert!(app.animating());
    let turn = Duration::from_secs_f32(1.0 / crate::theme::SPIN_SPEED);
    app.git.commit.refreshed = Instant::now().checked_sub(turn);
    assert!(!app.commit_refreshing());
    let _ = std::fs::remove_dir_all(&root);
}

/// 改动都提交以后提交卡片滑出视图底部：途中卡片大小不变、整体往下移，上面的改动列表跟着变高，
/// 卡片上的按钮不再响应点击；停稳以后卡片不再占地方。
#[test]
fn the_commit_card_slides_away_once_nothing_is_left_to_commit() {
    let Some((mut app, root)) = git_app("git-commit-card-slide", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rest = app.commit_parts();
    let card = rest.card.unwrap();
    // 测试里没有窗口，记下的只是「上一帧显示着卡片」，不会开始过渡。
    app.track_commit_bars();
    assert!(app.git.commit.card.motion.is_none());

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    app.git_refresh();
    app.track_commit_bars();
    assert!(app.git.commit.card.motion.is_none());
    let gone = app.commit_parts();
    assert!(gone.card.is_none());

    app.git.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let half = parts.card.unwrap();
    assert_eq!((half.x, half.w, half.h), (card.x, card.w, card.h));
    assert!(
        card.y < half.y && half.y < app.layout.sidebar.bottom(),
        "{half:?}"
    );
    assert!(rest.list.bottom() < parts.list.bottom());
    assert!(parts.list.bottom() < gone.list.bottom());
    paint(&mut app);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Commit));

    app.git.commit.card.motion = Some((1.0, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.git.commit.card.motion.is_none());
    assert_eq!(app.commit_parts().list, gone.list);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：侧栏宽度刚好放得下「修改上次提交」和按钮的一种写法、放不下另一种时按下或松开 ⌥，
/// 按钮变宽：过渡前、过渡的四分之一处、过渡结束。
#[test]
#[ignore]
fn shoot_commit_button_motion() {
    use crate::app::animation::{Clock, TREE_ANIMATION};
    use winit::keyboard::ModifiersState;

    let Some((mut app, root)) = git_app("git-commit-button-motion-shot", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("修复"));
    let fits = |app: &mut App, modifiers| {
        app.pointer.modifiers = modifiers;
        paint(app);
        app.commit_parts().amend.is_some()
    };
    let [plain, alt] = [ModifiersState::empty(), ModifiersState::ALT];
    let (from_keys, to_keys) = loop {
        match (fits(&mut app, plain), fits(&mut app, alt)) {
            (true, false) => break (plain, alt),
            (false, true) => break (alt, plain),
            _ => {}
        }
        assert!(app.panels.sidebar.width > crate::theme::MIN_PANEL_WIDTH);
        app.panels.sidebar.width -= 2.0;
    };
    app.pointer.modifiers = from_keys;
    shoot(&mut app, "commit-button-before");
    let before = app.commit_parts();
    let from = before.commit.w + before.commit_menu.w;
    app.pointer.modifiers = to_keys;
    paint(&mut app);
    let quarter = Clock::frozen(TREE_ANIMATION / 4, TREE_ANIMATION);
    app.git.commit.button_motion = Some((from, quarter));
    app.git.commit.amend_motion = Some((false, quarter));
    shoot(&mut app, "commit-button-quarter");
    app.git.commit.button_motion = None;
    app.git.commit.amend_motion = None;
    shoot(&mut app, "commit-button-after");
    app.pointer.modifiers = plain;
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交以后改动列表不是一下子换掉：过渡途中离开的行按刷新前的状态还画着、各行不响应点击，空状态
/// 同时淡入；只是保存了文件（改动的文件和状态都没变）时不做过渡。
#[test]
fn committed_rows_fade_out_instead_of_vanishing() {
    let Some((mut app, root)) = git_app("git-commit-shift", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let old = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    assert!(!rows.is_empty());
    let has_rows = |app: &App| {
        (app.hits.commit.iter())
            .any(|(_, t)| matches!(t, CommitTarget::File(_) | CommitTarget::Group(_)))
    };
    assert!(has_rows(&app));

    // 保存（内容变了，文件和状态都没变）：不算变化。
    assert!(!crate::app::git::commit::entries_changed(
        &old,
        &old.clone()
    ));

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    app.git_refresh();
    let new = app.git.status.clone().unwrap();
    assert!(new.entries.is_empty());
    assert!(crate::app::git::commit::entries_changed(&old, &new));

    app.git.commit.shift = Some(crate::app::git::commit::CommitShift {
        status: old,
        rows,
        scroll: 0.0,
        clock: Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION),
    });
    assert!(app.animating());
    paint(&mut app);
    assert!(!has_rows(&app));
    assert!(app.git.commit.overflow.is_none());

    app.git.commit.shift.as_mut().unwrap().clock = Clock::frozen(TREE_ANIMATION, TREE_ANIMATION);
    app.settle_animations();
    assert!(app.git.commit.shift.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_message_wraps_between_words() {
    let dir = search_folder("commit-message-wrap");
    if !long_repo(&dir) {
        return;
    }
    std::fs::write(dir.join("src/config.rs"), "pub struct Config;\n").unwrap();
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    app.run_action(Action::ShowCommit);
    // 返回各显示行的文字。
    let rows = |app: &mut App, text: &str| {
        app.git.commit.message.set_text("");
        app.edit_field(|f| f.insert(text));
        paint(app);
        let view = &app.git.commit.message.view;
        let last = view.rows.last().copied().unwrap_or(0);
        (0..=last)
            .map(|row| {
                let mut stops = (view.stops.iter().zip(&view.rows)).filter(|(_, r)| **r == row);
                let start = stops.next().map_or(0, |(s, _)| s.0);
                let end = (view.ends.iter().find(|(r, ..)| *r == row))
                    .map(|&(_, pos, _)| pos)
                    .or(stops.next_back().map(|(s, _)| s.0))
                    .unwrap_or(start);
                text[start..end].to_owned()
            })
            .collect::<Vec<_>>()
    };

    // 英文在空格处折行：每一行都以完整的词结尾，行与行之间不丢字。
    let text = "refactor: split four long functions and narrow the visibility of App state";
    let lines = rows(&mut app, text);
    assert!(lines.len() > 1, "{lines:?}");
    assert_eq!(lines.concat(), text);
    for line in &lines[..lines.len() - 1] {
        assert!(line.ends_with(' '), "{lines:?}");
    }
    assert_eq!(app.git.commit.message.view.ends.len(), lines.len() - 1);

    // 一个词比一行还宽：从放不下的字符处折行。
    let long = "x".repeat(200);
    let lines = rows(&mut app, &long);
    assert!(lines.len() > 1 && lines.concat() == long, "{lines:?}");

    // 中文可以在任意两个字之间折行，不需要空格。
    let cjk = "修复配置读取时的错误".repeat(8);
    let lines = rows(&mut app, &cjk);
    assert!(lines.len() > 1 && lines.concat() == cjk, "{lines:?}");

    // 中英文混排：英文单词不从中间断开。
    let mixed = format!("{}configuration", "配置".repeat(20));
    let lines = rows(&mut app, &mixed);
    assert!(
        lines.iter().any(|l| l.ends_with("configuration")),
        "{lines:?}"
    );
    assert!(lines.iter().all(|l| !l.ends_with("config")), "{lines:?}");
}
