//! 合并冲突的测试：标记行上的按钮、「采用对方」与「两者都保留」、标记为已解决并完成合并。

use winc_editor::git::merge::{Pick, SideAlign};
use winc_i18n::t;

use crate::app::git::merge::MergeTarget;

use super::*;

/// 在 `dir` 中做一次会冲突的合并：`main` 与 `other` 修改了 `src/main.rs` 的同一行。
fn conflicted_repo(dir: &Path) -> bool {
    let text: String = (0..30).map(|i| format!("let value_{i} = {i};\n")).collect();
    std::fs::write(dir.join("src/main.rs"), &text).unwrap();
    if !make_repo(dir) {
        return false;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    // `make_repo` 修改或新增的文件先储藏，工作区干净后才能合并。
    assert!(git(&["stash", "-u", "-q"]));
    assert!(git(&["switch", "-q", "-c", "other"]));
    std::fs::write(
        dir.join("src/main.rs"),
        text.replace("let value_10 = 10;", "let value_10 = 1000; // other"),
    )
    .unwrap();
    assert!(git(&["commit", "-q", "-am", "other"]));
    assert!(git(&["switch", "-q", "main"]));
    std::fs::write(
        dir.join("src/main.rs"),
        text.replace("let value_10 = 10;", "let value_10 = 100; // main"),
    )
    .unwrap();
    assert!(git(&["commit", "-q", "-am", "main"]));
    assert!(!git(&["merge", "other"]));
    true
}

#[test]
fn conflicts_resolve_in_the_file_and_finish_the_merge() {
    let dir = search_folder("git-merge");
    if !conflicted_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, Some(winc_editor::git::Operation::Merge));
    let index = app.editor.active.unwrap();
    assert_eq!(app.conflicts(index).len(), 1);
    // 合并的提交信息草稿已经填入。
    assert!(app.commit.message.text().contains("other"));

    // 标记行上有按钮；右上角的卡片显示还剩一处。
    paint(&mut app);
    use crate::app::git::merge::MergeTarget;
    use winc_editor::git::merge::Pick;
    assert!(
        app.hits
            .merge
            .iter()
            .any(|(_, t)| *t == MergeTarget::Pick(0, Pick::Both))
    );
    assert!(app.hits.merge_card.is_some());
    // 下一处冲突：光标跳转到冲突所在的行。
    assert!(app.run_action(Action::NextDifference));

    // 采用对方：整段替换为对方的那一行，可以撤销。
    app.press_merge(MergeTarget::Pick(0, Pick::Theirs));
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.contains("let value_10 = 1000; // other\n"));
    assert!(!text.contains("<<<<<<<") && !text.contains("// main"));
    assert!(app.run_action(Action::Undo));
    assert_eq!(app.conflicts(index).len(), 1);
    app.press_merge(MergeTarget::Pick(0, Pick::Both));
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.contains("// main\nlet value_10 = 1000; // other\n"));

    // 冲突全部解决后：卡片变成「标记为已解决」；点击后保存并 `git add`。
    paint(&mut app);
    assert!(
        app.hits
            .merge
            .iter()
            .any(|(_, t)| *t == MergeTarget::MarkResolved)
    );
    app.press_merge(MergeTarget::MarkResolved);
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert!(
        status
            .entries
            .iter()
            .all(|e| e.shown() != winc_editor::git::Change::Conflicted)
    );

    app.git_commit();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_with_conflict_markers_are_not_committed() {
    let dir = search_folder("git-merge-guard");
    if !conflicted_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.unresolved.len(), 1);
    // 冲突的文件单独成组并排在最上方；提交信息上方显示说明条和「中止」。
    let first = app.commit_rows().first().cloned();
    assert!(matches!(
        first,
        Some(CommitRow::Group(CommitGroup::Conflicts))
    ));
    assert!(app.commit_parts().banner.is_some());
    assert!(
        app.hits
            .commit
            .iter()
            .any(|(_, t)| *t == CommitTarget::Abort)
    );
    // 仍有冲突标记时按钮不可用，按下也不会提交。
    let blocker = app.commit.blocker(&status);
    assert_eq!(blocker.as_deref(), Some("1 个文件仍有未解决的冲突"));
    app.git_commit();
    app.git_refresh();
    assert_eq!(
        app.git.status.as_ref().unwrap().operation,
        Some(winc_editor::git::Operation::Merge)
    );

    // 在编辑器中选定一侧但尚未保存时，按下按钮先保存；标记已移除，因此正常提交。
    use crate::app::git::merge::MergeTarget;
    use winc_editor::git::merge::Pick;
    paint(&mut app);
    app.press_merge(MergeTarget::Pick(0, Pick::Theirs));
    app.git_commit();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, None);
    assert!(status.unresolved.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：在文件里解决合并冲突，深浅两套配色。
#[test]
#[ignore]
fn shoot_merge_conflict() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "git-merge-dark"
        } else {
            "git-merge-light"
        });
        if !conflicted_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::NextDifference);
        shoot(&mut app, if dark { "merge-dark" } else { "merge-light" });
        app.run_action(Action::ShowCommit);
        shoot(
            &mut app,
            if dark {
                "merge-commit-dark"
            } else {
                "merge-commit-light"
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 打开有一段冲突的文件：两侧都改了第 2 行，对方还改了第 5 行，后者由 `git` 自动合入。
fn three_way_app(name: &str) -> Option<(App, PathBuf)> {
    three_way_app_with(
        name,
        "one\ntwo\nthree\nfour\nfive\n",
        "one\nTWO side\nthree\nfour\nFIVE\n",
        "one\nTWO main\nthree\nfour\nfive\n",
    )
}

/// 与 [`three_way_app`] 相同，文件的共同祖先、`side` 和 `main` 分支上的内容由调用方给出。
fn three_way_app_with(name: &str, base: &str, side: &str, main: &str) -> Option<(App, PathBuf)> {
    let dir = search_folder(name);
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let file = dir.join("merged.txt");
    let mut ready = run(&["init", "-q", "-b", "main"])
        && run(&["config", "user.name", "t"])
        && run(&["config", "user.email", "t@t"])
        && run(&["config", "commit.gpgsign", "false"]);
    std::fs::write(&file, base).unwrap();
    ready = ready && run(&["add", "--all"]) && run(&["commit", "-q", "-m", "base"]);
    ready = ready && run(&["switch", "-q", "-c", "side"]);
    std::fs::write(&file, side).unwrap();
    ready = ready && run(&["commit", "-qam", "side"]) && run(&["switch", "-q", "main"]);
    std::fs::write(&file, main).unwrap();
    ready = ready && run(&["commit", "-qam", "main"]);
    if !ready {
        skip_without_git("无法用 git 建测试仓库");
        return None;
    }
    // 合并必然产生冲突，退出码不为 0。
    let _ = run(&["merge", "--no-edit", "side"]);
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return None;
    }
    app.open_path(&file);
    paint(&mut app);
    Some((app, file))
}

#[test]
fn the_three_way_view_shows_both_sides_next_to_the_file() {
    let Some((mut app, file)) = three_way_app("merge-three-way") else {
        return;
    };
    let editor_w = app.layout.editor.w;
    assert!(app.merge.pane_rects[0].w == 0.0);
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(app.merge_panes_shown());
    let [left, right] = app.merge.pane_rects;
    assert!(left.w > 0.0 && right.w > 0.0);
    // 中栏位于两侧之间，文字区随文档移动。
    assert!(app.layout.editor.x >= app.merge.ribbons[0].right());
    assert!(app.layout.editor.right() <= app.merge.ribbons[1].x);
    assert!(app.merge.footer.y >= left.bottom());
    // 冲突折叠成一行，文件的内容不变。
    let conflict = app.conflicts(app.editor.active.unwrap())[0].clone();
    let doc = app.editor.active().unwrap();
    assert!(doc.folds().is_collapsed(conflict.start));
    assert!(doc.text().rope().unwrap().to_string().contains("<<<<<<<"));
    assert!(app.layout.text.x >= app.layout.editor.x);
    let panes = app.merge.panes.as_ref().unwrap();
    assert_eq!(
        panes.sides[0].text.to_string(),
        "one\nTWO main\nthree\nfour\nfive\n"
    );
    assert_eq!(
        panes.sides[1].text.to_string(),
        "one\nTWO side\nthree\nfour\nFIVE\n"
    );

    // 右栏的按钮采用对方这一段；中间是普通文档，修改可以撤销。
    let target = MergeTarget::Pick(0, Pick::Theirs);
    let ribbon = app.merge.ribbons[1];
    assert!(
        (app.hits.merge.iter()).any(|(rect, t)| *t == target && ribbon.contains(rect.x, rect.y)),
        "连接带上应当有采用按钮"
    );
    app.press_merge(target);
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(
        text.contains("TWO side") && !text.contains("<<<<<<<"),
        "{text}"
    );
    paint(&mut app);
    // 解决后不留下折叠：冲突之后的行都正常显示。
    let doc = app.editor.active().unwrap();
    assert!(doc.folds().is_empty());
    let three = text.lines().position(|l| l == "three").unwrap();
    assert!(!doc.folds().is_hidden(three));
    // 没有冲突标记但文件尚未标记为已解决时，两侧仍然显示。
    assert!(app.merge_panes_shown());

    // 标记为已解决后两侧收起。
    app.press_merge(MergeTarget::MarkResolved);
    app.git_refresh();
    paint(&mut app);
    assert!(!app.merge_panes_shown());
    assert_eq!(app.merge.pane_rects[0].w, 0.0);
    assert_eq!(app.layout.editor.w, editor_w);
    assert!(std::fs::read_to_string(&file).unwrap().contains("FIVE"));
}

#[test]
fn the_three_way_view_can_be_closed() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-close") else {
        return;
    };
    let editor_w = app.layout.editor.w;
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(app.merge.pane_rects[0].w > 0.0);
    // 关闭后文档恢复原来的宽度。
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(!app.merge_panes_shown());
    assert_eq!(app.layout.editor.w, editor_w);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_three_way_view() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-shot") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    shoot(&mut app, "git-three-way");
}

/// 手动编辑时展开冲突并将光标放入其中；退出三栏后所有冲突恢复原状。
#[test]
fn a_conflict_can_be_opened_for_manual_editing_and_folds_are_undone_on_exit() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-edit") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let start = app.conflicts(app.editor.active.unwrap())[0].start;
    app.press_merge(MergeTarget::EditConflict(0));
    let doc = app.editor.active().unwrap();
    assert!(!doc.folds().is_collapsed(start));
    let caret = doc.text().pos_to_line_col(doc.selection.head).0;
    assert_eq!(caret, start + 1);

    // 在展开的冲突里键入，下一帧不会重新折叠该冲突。
    let pos = app.editor.active().unwrap().selection.head;
    app.select(pos, pos);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().folds().is_collapsed(start));

    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(!app.merge_panes_shown());
    assert!(app.editor.active().unwrap().folds().is_empty());
}

/// 窗口宽度不足以显示三栏时不折叠冲突，否则内容不可见且没有两侧可用。
#[test]
fn conflicts_are_unfolded_when_the_panes_no_longer_fit() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-narrow") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().folds().is_empty());
    app.merge.pane_rects = [Rect::default(); 2];
    app.sync_merge_panes();
    assert!(app.editor.active().unwrap().folds().is_empty());
}

/// 回归：折叠行的文字被提示条遮挡，光标移到该行时必须展开，否则键入的字符会修改不可见的标记。
#[test]
fn moving_the_caret_onto_a_folded_conflict_expands_it() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-caret") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let start = app.conflicts(app.editor.active.unwrap())[0].start;
    assert!(app.editor.active().unwrap().folds().is_collapsed(start));
    let doc = app.editor.active().unwrap();
    let pos = doc.text().line_start(start);
    app.select(pos, pos);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().folds().is_collapsed(start));
    // 光标离开后恢复折叠。
    app.select(0, 0);
    paint(&mut app);
    assert!(app.editor.active().unwrap().folds().is_collapsed(start));
}

/// 选区一端位于冲突中时不折叠，避免选区在用户不知情时被改动。
#[test]
fn folding_never_moves_a_selection_endpoint() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-selection") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let start = app.conflicts(app.editor.active.unwrap())[0].start;
    let inside = app.editor.active().unwrap().text().line_start(start + 1);
    app.select(inside, 0);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.selection.anchor, inside);
    assert_eq!(doc.selection.head, 0);
}

/// 上一处、下一处只滚动，不把光标放到折叠行上。
#[test]
fn jumping_between_conflicts_keeps_them_folded() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-jump") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let start = app.conflicts(app.editor.active.unwrap())[0].start;
    let caret = app.editor.active().unwrap().selection;
    app.press_merge(MergeTarget::Next);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.selection, caret);
    assert!(doc.folds().is_collapsed(start));
}

/// 回归：解决之后再次冲突时，两侧不能显示上一次合并的版本。
#[test]
fn a_finished_merge_forgets_its_side_panes() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-forget") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(app.merge.panes.is_some());
    app.press_merge(MergeTarget::Pick(0, Pick::Ours));
    paint(&mut app);
    app.press_merge(MergeTarget::MarkResolved);
    app.git_refresh();
    paint(&mut app);
    assert!(app.merge.panes.is_none());
}

/// 创建一个文件，第 100 到 159 行在两个分支上都改过：合并后这一段是 120 多行的冲突。
fn long_conflict_app(name: &str) -> Option<(App, PathBuf)> {
    let text = |tag: &str| -> String {
        (0..300)
            .map(|i| {
                if (100..160).contains(&i) {
                    format!("{tag} {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect()
    };
    three_way_app_with(name, &text("base"), &text("side"), &text("main"))
}

/// 返回第 `line` 行与滚动位置之间相隔的显示行数。
fn row_on_screen(app: &App, line: usize) -> f64 {
    let doc = app.editor.active().unwrap();
    doc.row_of(line) as f64 - doc.scroll_line
}

/// 回归：文档已修改而对齐尚未计算完成时，连接带上的冲突序号已经过期，点击会解决另一段冲突。
#[test]
fn ribbon_buttons_ignore_clicks_until_the_alignment_is_current() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-stale") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let ribbon = |app: &App| {
        (app.hits.merge.iter()).any(|(_, target)| {
            matches!(
                target,
                MergeTarget::Pick(_, Pick::Ours) | MergeTarget::Pick(_, Pick::Theirs)
            )
        })
    };
    assert!(ribbon(&app));
    let doc = app.editor.active_mut().unwrap();
    crate::app::git::changes::replace_lines(doc, 0..0, "new\n".into()).unwrap();
    let revision = doc.revision;
    // 测试里后台任务就地完成：标记为正在计算，模拟结果尚未送达。
    app.merge.panes.as_mut().unwrap().aligning = Some(revision);
    paint(&mut app);
    assert!(!ribbon(&app));
}

/// 回归：冲突收起时光标所在的行不能在屏幕上移动，否则双击的第二下会落到别的行。
#[test]
fn collapsing_a_conflict_keeps_the_caret_line_in_place() {
    let Some((mut app, _file)) = long_conflict_app("merge-three-way-anchor") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let conflict = app.conflicts(app.editor.active.unwrap())[0].clone();
    app.press_merge(MergeTarget::EditConflict(0));
    paint(&mut app);
    let below = conflict.end + 1;
    let doc = app.editor.active_mut().unwrap();
    doc.scroll_line = doc.row_of(below) as f64 - 5.0;
    let pos = doc.text().line_start(below);
    assert_eq!(row_on_screen(&app, below), 5.0);
    app.select(pos, pos);
    paint(&mut app);
    assert!(
        (app.editor.active().unwrap().folds()).is_collapsed(conflict.start),
        "光标离开后冲突应当收起"
    );
    assert_eq!(row_on_screen(&app, below), 5.0);
}

/// 回归：退出三栏时展开冲突，下方的内容不能因此滚动离开。
#[test]
fn leaving_the_three_way_view_keeps_the_scrolled_line_in_place() {
    let Some((mut app, _file)) = long_conflict_app("merge-three-way-exit-view") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let conflict = app.conflicts(app.editor.active.unwrap())[0].clone();
    let below = conflict.end + 20;
    let doc = app.editor.active_mut().unwrap();
    doc.scroll_line = doc.row_of(below) as f64;
    paint(&mut app);
    assert_eq!(row_on_screen(&app, below), 0.0);
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(!app.merge_panes_placed());
    assert_eq!(row_on_screen(&app, below), 0.0);
}

/// 回归：无法继续滚动时需要提示没有更多冲突，不能没有任何反应。
#[test]
fn jumping_reports_when_the_view_cannot_move() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-limits") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    app.status.message = None;
    app.press_merge(MergeTarget::Previous);
    assert_eq!(
        app.status.message.as_deref(),
        Some(t!("git-merge-no-previous"))
    );
    app.status.message = None;
    app.press_merge(MergeTarget::Next);
    assert_eq!(app.status.message.as_deref(), Some(t!("git-merge-no-next")));
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
}

/// 回归：`HEAD` 的改动标记不再绘制时，这些位置不能再响应鼠标。
#[test]
fn hidden_change_marks_do_not_respond_in_the_three_way_view() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-marks") else {
        return;
    };
    let responds = |app: &App| {
        let l = app.layout;
        (0..l.editor.h as i32).any(|dy| {
            let y = l.editor.y + dy as f32;
            let mark = (l.gutter.x as i32..l.gutter.right() as i32)
                .any(|x| app.change_mark_at(x as f32, y).is_some());
            let ruler = (l.editor.right() as i32 - 16..l.editor.right() as i32)
                .any(|x| app.ruler_change_at(x as f32, y).is_some());
            mark || ruler
        })
    };
    assert!(responds(&app));
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    assert!(app.merge_panes_placed());
    assert!(!responds(&app));
}

/// 上一处、下一处滚动到冲突；到达末端后提示没有更多冲突。
#[test]
fn jumping_stops_at_the_conflict_and_then_reports() {
    let Some((mut app, _file)) = long_conflict_app("merge-three-way-jump-far") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let start = app.conflicts(app.editor.active.unwrap())[0].start;
    app.status.message = None;
    app.press_merge(MergeTarget::Next);
    assert_eq!(row_on_screen(&app, start), 3.0);
    assert_eq!(app.status.message, None);
    app.press_merge(MergeTarget::Next);
    assert_eq!(row_on_screen(&app, start), 3.0);
    assert_eq!(app.status.message.as_deref(), Some(t!("git-merge-no-next")));
}

/// 后台任务不保证按提交的顺序完成：旧版本的结果不能覆盖新版本的。
#[test]
fn an_older_alignment_never_replaces_a_newer_one() {
    let Some((mut app, _file)) = three_way_app("merge-three-way-order") else {
        return;
    };
    app.press_merge(MergeTarget::ThreePane);
    paint(&mut app);
    let align = || {
        let rope = ropey::Rope::from_str("x\n");
        let side = |pick| SideAlign::new(&rope, &[], "x\n", pick);
        [side(Pick::Ours), side(Pick::Theirs)]
    };
    let panes = app.merge.panes.as_mut().unwrap();
    let at = panes.aligned.as_ref().unwrap().0;
    panes.aligning = Some(at + 1);
    panes.accept_alignment(at + 1, align());
    assert_eq!(panes.aligned.as_ref().unwrap().0, at + 1);
    assert_eq!(panes.aligning, None);
    panes.aligning = Some(at + 2);
    panes.accept_alignment(at, align());
    assert_eq!(panes.aligned.as_ref().unwrap().0, at + 1);
    assert_eq!(panes.aligning, Some(at + 2));
}
