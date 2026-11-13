//! 提交视图中贮藏列表的测试。

use super::*;

/// 截图：改动列表下面展开的储藏一组（`commit-stashes`），鼠标停在第一条上（「应用」和「⋯」），
/// 储藏超过五条时最后一行是「还有 N 条」。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_stashes() {
    let Some((mut app, root)) = git_app("commit-stashes", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    for (i, note) in [
        "改登录",
        "试一下新写法",
        "调日志刷新",
        "备份配置",
        "临时打点",
        "旧方案",
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(root.join("src/main.rs"), format!("fn main() {{ {i}; }}\n")).unwrap();
        git_in(&root, &["stash", "push", "-q", "-m", note]);
    }
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    let (row, _) = *(app.hits.commit.iter())
        .find(|(_, t)| *t == CommitTarget::Stash(0))
        .unwrap();
    app.on_mouse_move(row.x + 20.0, row.y + row.h / 2.0);
    shoot(&mut app, "commit-stashes");
    // 打开第一条储藏：它的文件列在下面，正在查看的文件处于选中状态。
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-u", "-m", "带一个新文件"]);
    app.git_refresh();
    paint(&mut app);
    app.press_commit(CommitTarget::Stash(0));
    shoot(&mut app, "commit-stash-files");
    let _ = std::fs::remove_dir_all(&root);
}

/// 改动列表下面的储藏一组：默认收起；展开后每条一行。点一下在储藏的预览标签里打开它的第一个文件，
/// 差异里的下一个文件在这条储藏的文件之间切换，一起收进去的未跟踪文件排在后面；悬停时的「应用」
/// 把改动放回工作区。
#[test]
fn stashes_are_listed_below_the_changes_and_open_their_files() {
    let Some((mut app, root)) = git_app("git-commit-stashes", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { stashed(); }\n").unwrap();
    std::fs::write(root.join("notes.txt"), "draft\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-u", "-m", "试一下新写法"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let stash = app.git.stashes[0].clone();
    assert_eq!(stash.branch(), Some("main"));
    assert_eq!(stash.parents.len(), 3);

    // 收着时只有标题一行，在列表下面。
    let parts = app.commit_parts();
    let section = parts.stashes.unwrap();
    assert_eq!(
        section.h,
        COMMIT_ROW + 2.0 * crate::app::git::commit::STASH_GAP
    );
    assert!(parts.list.bottom() <= section.y);
    assert!(!(app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));
    app.press_commit(CommitTarget::Stashes);
    paint(&mut app);
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::Stash(0)));

    // 点一下：两个文件，已跟踪的在前；预览标签里是第一个，标题写着储藏的名字。
    app.press_commit(CommitTarget::Stash(0));
    let view = app.git.commit.stash_view.clone().unwrap();
    let names: Vec<String> = (view.files.iter())
        .map(|(f, ..)| winc_editor::display_name(&f.path).into_owned())
        .collect();
    assert_eq!(names, ["main.rs", "notes.txt"]);
    assert!(view.files[1].1.is_none());
    let doc = app.editor.active().unwrap();
    assert!(doc.diff.is_some());
    assert_eq!(app.git.commit.stash_preview, Some(doc.id));
    assert!(doc.name().contains("stash@{0}"));
    assert!(app.diff_steps_files());
    // 下一个文件是一起收进去的未跟踪文件。
    app.step_diff_file(true);
    assert_eq!(app.git.commit.stash_view.as_ref().unwrap().at, 1);
    assert!(app.editor.active().unwrap().name().contains("notes.txt"));
    assert_eq!(app.next_stash_file(true), None);

    // 储藏那一行下面列出它的两个文件，点击第一个时预览标签切回该文件。
    use crate::app::StashLine;
    let lines = [StashLine::Stash(0), StashLine::File(0), StashLine::File(1)];
    assert_eq!(app.stash_lines(), lines);
    paint(&mut app);
    let section = app.commit_parts().stashes.unwrap();
    assert_eq!(
        section.h,
        4.0 * COMMIT_ROW + 2.0 * crate::app::git::commit::STASH_GAP
    );
    assert!((app.hits.commit.iter()).any(|(_, t)| *t == CommitTarget::StashFile(1)));
    app.press_commit(CommitTarget::StashFile(0));
    assert_eq!(app.git.commit.stash_view.as_ref().unwrap().at, 0);
    assert!(app.editor.active().unwrap().name().contains("main.rs"));

    // 再次点击这条储藏：文件收起，预览标签保留；再次点击时重新展开。
    app.press_commit(CommitTarget::Stash(0));
    assert_eq!(app.stash_lines(), [StashLine::Stash(0)]);
    assert!(app.editor.active().unwrap().name().contains("main.rs"));
    app.press_commit(CommitTarget::Stash(0));
    assert_eq!(app.stash_lines(), lines);
    // 关闭预览标签：文件不再列出。
    app.run_action(Action::CloseTab);
    paint(&mut app);
    assert_eq!(app.stash_lines(), [StashLine::Stash(0)]);

    // 「应用」：改动回到工作区，储藏还在。
    app.press_commit(CommitTarget::StashApply(0));
    app.git_refresh();
    assert!(root.join("notes.txt").exists());
    assert_eq!(app.git.stashes.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// 储藏一组像抽屉一样拉开：途中一组的高度和箭头的角度都在两端之间，上面的改动列表跟着变矮，
/// 还在移动的行不响应点击；停稳以后各行才能点。
#[test]
fn the_stash_group_opens_like_a_drawer() {
    let Some((mut app, root)) = git_app("git-commit-stash-drawer", false) else {
        return;
    };
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { stashed(); }\n").unwrap();
    git_in(&root, &["stash", "push", "-q", "-m", "试一下新写法"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { now(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let closed = app.commit_parts().stashes.unwrap();
    app.press_commit(CommitTarget::Stashes);
    let open = app.commit_parts().stashes.unwrap();
    assert_eq!(open.h, closed.h + COMMIT_ROW);

    app.git.commit.stash_motion =
        Some((closed.h, Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION)));
    let parts = app.commit_parts();
    let half = parts.stashes.unwrap();
    assert!(closed.h < half.h && half.h < open.h, "{half:?}");
    assert_eq!(half.bottom(), open.bottom());
    assert_eq!(parts.list.bottom(), half.y);
    assert!(0.0 < app.stash_chevron() && app.stash_chevron() < 1.0);
    assert!(app.animating());
    paint(&mut app);
    let has = |app: &App, target| (app.hits.commit.iter()).any(|(_, t)| *t == target);
    assert!(has(&app, CommitTarget::Stashes));
    assert!(!has(&app, CommitTarget::Stash(0)));

    app.git.commit.stash_motion = Some((closed.h, Clock::frozen(TREE_ANIMATION, TREE_ANIMATION)));
    app.settle_animations();
    assert!(app.git.commit.stash_motion.is_none());
    paint(&mut app);
    assert!(has(&app, CommitTarget::Stash(0)));
    assert_eq!(app.stash_chevron(), 1.0);
    let _ = std::fs::remove_dir_all(&root);
}
