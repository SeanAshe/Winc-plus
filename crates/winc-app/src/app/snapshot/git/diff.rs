//! 并排差异的测试：打开与逐处跳转、编辑右侧、从左侧取回改动、滚动、点击，以及跟随文件更新。

use super::*;

mod edit;
mod side;

#[test]
fn expanded_diff_pins_the_enclosing_scopes_for_each_version() {
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT};

    let dir = sample_folder("diff-sticky");
    let mut app = app_with(&dir.join("short.rs"), false);
    let language = winc_syntax::detect("Example.java");
    let old = format!(
        "class Example {{\n    void before() {{\n{}    }}\n}}\n",
        "        beforeStep();\n".repeat(55),
    );
    let new = format!(
        "class Example {{\n    // new method\n    void after() {{\n{}    }}\n}}\n",
        "        afterStep();\n".repeat(55),
    );
    let view = DiffView::new(
        DiffSide::new("old", &old, None, language),
        DiffSide::new("new", &new, None, language),
    );
    app.editor.show_diff("Example.java", view);
    let sticky = |app: &App| {
        app.hits
            .diff
            .iter()
            .filter_map(|&(rect, target)| match target {
                DiffTarget::Sticky(side, line) => Some((side, line, rect)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let lines = |app: &App| {
        sticky(app)
            .iter()
            .map(|&(side, line, _)| (side, line))
            .collect::<Vec<_>>()
    };

    app.editor.active_mut().unwrap().scroll_line = 16.0;
    paint(&mut app);
    assert!(sticky(&app).is_empty(), "折叠模式不占用正文空间");
    app.press_diff(DiffTarget::Collapse);
    paint(&mut app);
    assert_eq!(lines(&app), [(LEFT, 0), (LEFT, 1), (RIGHT, 0), (RIGHT, 2)]);

    // 查找命中恰好位于被固定行覆盖的位置时，滚动使它露出来。
    let doc = app.editor.active().unwrap();
    let before = doc.scroll_line;
    let view = doc.diff.as_deref().unwrap();
    let parts = diff_parts(app.layout.editor, view, app.metrics.cell);
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    let top = view.side_scroll(RIGHT, before, visible).floor() as usize;
    let line = view.line_at(RIGHT, top).unwrap();
    let start = view.new.text.line_to_char(line) + 8;
    app.reveal_diff_match(RIGHT, start, start + 9);
    assert!(app.editor.active().unwrap().scroll_line < before);
    app.editor.active_mut().unwrap().scroll_line = before;
    paint(&mut app);

    let right_method = sticky(&app)[3].2;
    click(
        &mut app,
        (
            right_method.x + right_method.w / 2.0,
            right_method.y + right_method.h / 2.0,
        ),
    );
    assert!(app.editor.active().unwrap().scroll_line < 3.0);

    // 从一个折成多段的正文行中间开始滚动，仍显示同一个函数头。
    let doc = app.editor.active_mut().unwrap();
    let view = doc.diff.as_deref_mut().unwrap();
    view.set_wrap(Some((14, 4)));
    let row = view.row_of(RIGHT, 12);
    assert!(view.row_of(RIGHT, 13) > row + 1);
    doc.scroll_line = view.pos_of_top(RIGHT, row as f64 + 1.25, visible);
    paint(&mut app);
    assert_eq!(lines(&app), [(LEFT, 0), (LEFT, 1), (RIGHT, 0), (RIGHT, 2)]);

    // 单栏以顶行的版本为准，删除区和新增区显示不同的函数头。
    for (side, signature) in [(LEFT, 1), (RIGHT, 2)] {
        let doc = app.editor.active_mut().unwrap();
        let view = doc.diff.as_deref_mut().unwrap();
        view.set_wrap(None);
        view.set_unified(true);
        doc.scroll_line = view.pos_of(side, view.row_of(side, 12) as f64);
        paint(&mut app);
        assert_eq!(lines(&app), [(side, 0), (side, signature)]);
    }

    app.config.sticky_scroll = Some(false);
    paint(&mut app);
    assert!(sticky(&app).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 打开 `Example.java` 的工作区差异（右侧可编辑），展开未改动的内容，返回应用与临时目录。
///
/// 两侧都是一个类里的一个长方法：类在第 0 行，右侧的方法在第 2 行。
fn sticky_worktree_diff(name: &str) -> (App, std::path::PathBuf) {
    use winc_editor::diff_view::{DiffSide, DiffView};

    let dir = sample_folder(name);
    let file = dir.join("Example.java");
    let old = format!(
        "class Example {{\n    void before() {{\n{}    }}\n}}\n",
        "        beforeStep();\n".repeat(55),
    );
    let new = format!(
        "class Example {{\n    // new method\n    void after() {{\n{}    }}\n}}\n",
        "        afterStep();\n".repeat(55),
    );
    std::fs::write(&file, &new).unwrap();
    let mut app = app_with(&file, false);
    let path = app.editor.active().unwrap().path.clone();
    let language = winc_syntax::detect("Example.java");
    let mut view = DiffView::new(
        DiffSide::new("old", &old, None, language),
        DiffSide::new("worktree", &new, path, language),
    );
    view.editing = true;
    app.editor
        .show_diff_in(None, "Example.java (worktree)", view);
    app.focus = Focus::Editor;
    app.press_diff(DiffTarget::Collapse);
    paint(&mut app);
    (app, dir)
}

/// 返回右侧第 `line` 行位于右侧顶部之下的显示行数（在顶部时为 0）。
fn right_row_below_top(app: &App, line: usize) -> f64 {
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let parts = diff_parts(app.layout.editor, view, app.metrics.cell);
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    view.row_of(RIGHT, line) as f64 - view.side_scroll(RIGHT, doc.scroll_line, visible)
}

/// 返回右侧工作区文件对应的文档。
fn worktree_source(app: &mut App) -> &mut winc_editor::Document {
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let path = view.new.path.clone().unwrap();
    (app.editor.documents.iter_mut())
        .find(|d| d.diff.is_none() && d.path.as_deref() == Some(path.as_path()))
        .unwrap()
}

/// 把差异滚动到右侧第 `line` 行位于顶部。
fn scroll_right_line_to_top(app: &mut App, line: usize) {
    let parts = {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        diff_parts(app.layout.editor, view, app.metrics.cell)
    };
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    let doc = app.editor.active_mut().unwrap();
    let view = doc.diff.as_deref().unwrap();
    doc.scroll_line = view.pos_of_top(RIGHT, view.row_of(RIGHT, line) as f64, visible);
    paint(app);
}

/// 回归：单栏中顶部是删除的行、其下是新增的行时，固定行按屏幕上实际显示的行判断，不会把
/// 下方仍可见的函数头也固定在顶部（同一行显示两次）。
#[test]
fn unified_diffs_do_not_pin_headers_that_are_still_visible() {
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT};

    let dir = sample_folder("diff-sticky-unified");
    let mut app = app_with(&dir.join("short.rs"), false);
    let body = "            work();\n".repeat(40);
    let old = format!(
        "impl App {{\n    fn a() {{\n        one();\n    }}\n\n    /// old\n    \
         fn b() {{\n        if ready {{\n{body}        }}\n    }}\n}}\n"
    );
    let new = format!(
        "impl App {{\n    fn a() {{\n        one();\n    }}\n\n    /// new\n    /// more\n    \
         fn b() {{\n        if ready {{\n{body}        }}\n    }}\n}}\n"
    );
    let language = winc_syntax::detect("short.rs");
    let view = DiffView::new(
        DiffSide::new("old", &old, None, language),
        DiffSide::new("new", &new, None, language),
    );
    app.editor.show_diff("short.rs", view);
    app.press_diff(DiffTarget::Collapse);
    paint(&mut app);
    let doc = app.editor.active_mut().unwrap();
    let view = doc.diff.as_deref_mut().unwrap();
    view.set_unified(true);
    // 顶部停在删除的 `/// old` 一行的中间，其下依次是两行新增和未改动的 `fn b`。
    doc.scroll_line = view.pos_of(LEFT, view.row_of(LEFT, 5) as f64) + 0.25;
    paint(&mut app);
    let pinned: Vec<_> = (app.hits.diff.iter())
        .filter_map(|&(_, target)| match target {
            DiffTarget::Sticky(side, line) => Some((side, line)),
            _ => None,
        })
        .collect();
    assert_eq!(pinned, [(LEFT, 0)]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截图：「»」替换一段改动后，该段颜色淡出到一半时的画面。
#[test]
#[ignore]
fn shoot_diff_replace_fade() {
    let dir = search_folder("git-diff-fade");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    let file = app.git.root.clone().unwrap().join("src/main.rs");
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    app.press_diff(DiffTarget::Revert(0));
    let doc = app.editor.active().unwrap().id;
    app.diff.fade = Some(crate::app::diff::DiffFade {
        doc,
        left: 5..6,
        right: 5..6,
        kind: crate::app::HunkKind::Modified,
        clock: crate::app::animation::Clock::frozen(
            std::time::Duration::from_millis(120),
            std::time::Duration::from_millis(420),
        ),
    });
    shoot(&mut app, "diff-fade");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：在差异中查找，右侧标出匹配。
#[test]
#[ignore]
fn shoot_diff_search() {
    let dir = search_folder("git-diff-search");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    app.run_action(Action::GitShowChanges);
    paint(&mut app);
    app.run_action(Action::Find);
    app.bars.find.input.set_text("value_4");
    app.bars.search.query = "value_4".into();
    app.search_changed();
    shoot(&mut app, "diff-search");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：当前文件改动的并排差异，深浅两套配色。
#[test]
#[ignore]
fn shoot_git_diff() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "git-diff-dark"
        } else {
            "git-diff-light"
        });
        if !long_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        // 在开头再添加两行且不保存：三种改动（新增、修改、删除）都有。
        app.run(winc_editor::Command::Insert(
            "use std::fs;\nuse std::io;\n".into(),
        ));
        app.run_action(Action::GitShowChanges);
        // 光标放在右侧第一处改动上。
        paint(&mut app);
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        let lh = app.fonts.line_height;
        click(
            &mut app,
            (parts.panes[1].text.x + 120.0, parts.body.y + lh * 3.5),
        );
        shoot(&mut app, if dark { "diff-dark" } else { "diff-light" });
        // 鼠标停在第二处改动的「»」上，提示已显示。
        let revert = app
            .hits
            .diff
            .iter()
            .find(|(_, t)| *t == DiffTarget::Revert(1))
            .unwrap()
            .0;
        app.on_mouse_move(revert.x + 4.0, revert.y + 4.0);
        app.overlays.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(
            &mut app,
            if dark {
                "diff-revert-dark"
            } else {
                "diff-revert-light"
            },
        );
        // 鼠标停在第二处改动的勾选框上。
        let check = app
            .hits
            .diff
            .iter()
            .find(|(_, t)| *t == DiffTarget::Include(1))
            .unwrap()
            .0;
        app.on_mouse_move(check.x + 4.0, check.y + 4.0);
        app.overlays.tooltip.since = None;
        shoot(
            &mut app,
            if dark {
                "diff-check-dark"
            } else {
                "diff-check-light"
            },
        );
        // 新文件（不在 `HEAD` 中）：左侧为空。
        let text: String = (0..80)
            .map(|i| format!("pub const C{i}: u32 = {i};\n"))
            .collect();
        std::fs::write(dir.join("src/new.rs"), text).unwrap();
        app.open_path(&dir.join("src/new.rs"));
        app.run_action(Action::GitShowChanges);
        paint(&mut app);
        // 滚动一次，滚动条的滑块显示。
        app.editor.active_mut().unwrap().scroll_line = 20.0;
        shoot(
            &mut app,
            if dark {
                "diff-new-dark"
            } else {
                "diff-new-light"
            },
        );
        // 滚动到底：滑块停在卡片的圆角上方，不超出卡片。
        app.editor.active_mut().unwrap().scroll_line = 78.0;
        shoot(
            &mut app,
            if dark {
                "diff-new-bottom-dark"
            } else {
                "diff-new-bottom-light"
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn diff_scrollbar_stays_below_the_header_and_new_files_have_no_replace() {
    let Some((mut app, root)) = git_app("git-diff-track", true) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    paint(&mut app);
    // 滚动条的轨道不覆盖标题栏：标题栏右端（折叠开关旁边）不是滚动条。
    let editor = app.layout.editor;
    let track = app.scroll_track();
    assert_eq!(track.y, editor.y + crate::app::diff::DIFF_HEADER);
    assert_ne!(
        app.hover_at(editor.right() - 4.0, editor.y + 10.0),
        Hover::Scrollbar(crate::layout::Scrollbar::Editor)
    );
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| matches!(t, DiffTarget::Revert(_)))
    );

    // 新文件：左侧为空，没有「»」（用左侧替换等于清空整个文件）。
    let text: String = (0..80)
        .map(|i| format!("pub const C{i}: u32 = {i};\n"))
        .collect();
    std::fs::write(root.join("src/fresh.rs"), text).unwrap();
    app.git_refresh();
    app.open_path(&root.join("src/fresh.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.one_side_empty());
    // 左侧表示 `HEAD` 中没有该文件，不是空文件：不写「空文件」。
    assert!(view.old.missing && !view.new.missing);
    assert!(
        !app.hits
            .diff
            .iter()
            .any(|(_, t)| matches!(t, DiffTarget::Revert(_)))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn status_refreshes_rediff_a_git_diff_tab_only_when_head_moves() {
    let Some((mut app, root)) = git_app("git-diff-refresh", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    assert!(app.run_action(Action::GitShowChanges));
    let view = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        std::ptr::from_ref(view) as usize
    };
    let before = view(&app);

    // 文件已在编辑器中打开且 `HEAD` 未变：右侧随文档更新，刷新状态时不再调用 git 重新比较。
    app.git_refresh();
    assert_eq!(view(&app), before);

    // `HEAD` 改变：重新读取左侧内容。
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "second"]);
    app.git_refresh();
    assert_ne!(view(&app), before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn find_in_a_commit_diff_searches_the_selected_snapshot() {
    use crate::keymap::{KeyCode, Mods, Os, Stroke};
    use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT};

    let dir = sample_folder("diff-commit-find");
    let mut app = app_with(&dir.join("short.rs"), false);
    let language = winc_syntax::detect("main.rs");
    let side = |text: &str| DiffSide::new("abc1234", text, None, language);
    let common = format!(
        "{}命中一\n{}",
        "context\n".repeat(70),
        "context\n".repeat(70)
    );
    let old = format!("左侧\n{common}old\n");
    let new = format!("右侧\n{common}命中二\n");
    let view = DiffView::new(side(&old), side(&new));
    app.editor.show_diff_in(None, "main.rs", view);
    paint(&mut app);

    let find = Stroke {
        key: KeyCode::Char('f'),
        mods: if Os::CURRENT == Os::Mac {
            Mods::META
        } else {
            Mods::CTRL
        },
    };
    let set_query = |app: &mut App, query: &str| {
        app.bars.find.input.set_text(query);
        app.bars.search.query = query.to_owned();
        app.search_changed();
    };
    let selection = |app: &App| {
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        assert!(!view.editing);
        view.read_selection.unwrap()
    };
    assert!(app.on_shortcut(find));
    assert!(app.bars.find.open);
    assert_eq!(app.focus, crate::app::Focus::Search);
    set_query(&mut app, "命中");
    assert_eq!(app.bars.search.matches().len(), 2);
    let first = selection(&app);
    assert_eq!(first.side, RIGHT);
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let row = view.row_of_new(71);
    assert_eq!(view.line_at(RIGHT, row), Some(71));
    assert!(doc.scroll_line > 0.0);
    app.advance_search();
    paint(&mut app);
    assert_eq!(app.bars.search.current_match(), Some(first.range));

    app.find_next(false);
    assert!(selection(&app).range.0 > first.range.0);
    app.find_next(true);
    assert_eq!(selection(&app), first);
    app.press_search_button(crate::layout::SearchButton::Expand);
    assert!(!app.bars.find.replace_open);
    app.close_search();

    let doc = app.editor.active_mut().unwrap();
    doc.scroll_line = 0.0;
    let view = doc.diff.as_deref().unwrap();
    let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
    let point = (
        parts.panes[LEFT].text.x + crate::app::diff::DIFF_TEXT_PAD + app.metrics.cell,
        parts.body.y + 4.0,
    );
    click(&mut app, point);
    assert!(app.on_shortcut(find));
    assert_eq!(app.bars.search.query, "左侧");
    assert_eq!(selection(&app).side, LEFT);
    set_query(&mut app, "命中");
    assert_eq!(app.bars.search.matches().len(), 1);

    // 同一标签换成另一份差异后，同一侧的旧结果作废。
    let id = app.editor.active().unwrap().id;
    let deleted = DiffView::new(side("命中\n命中\n命中\n"), side("").with_missing(true));
    app.editor.show_diff_in(Some(id), "deleted.rs", deleted);
    app.advance_search();
    assert_eq!(app.bars.search.matches().len(), 3);
    app.find_next(false);
    assert_eq!(selection(&app).side, LEFT);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stepping_through_changes_marks_each_one_and_walks_the_files() {
    let dir = search_folder("git-diff-step");
    std::fs::write(dir.join("src/zeta.rs"), "a\nb\nc\n").unwrap();
    if !long_repo(&dir) {
        return;
    }
    std::fs::write(dir.join("src/zeta.rs"), "a\nB\nc\nd\n").unwrap();
    // 只保留两个有改动的文件。
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&main, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let tabs = app.editor.documents.len();
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };
    let caret_line = |app: &App| {
        let source = app
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(&main));
        let doc = source.unwrap();
        doc.text().pos_to_line_col(doc.selection.head).0
    };
    assert!(title(&app).starts_with("main.rs"));
    assert!(app.diff_steps_files());

    // 打开时不标记。第一次跳转标记第一处；文件已经打开，光标移到它的文档里，键入的字符写入右侧。
    assert_eq!(focus(&app), None);
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(caret_line(&app), 5);
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert_eq!(caret_line(&app), 50);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.editing);
    paint(&mut app);

    // 越过最后一处：第一次提示，文件不变。
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(
        app.status.message.as_deref(),
        Some("已是最后一处更改。再次执行可切换到下一个文件：zeta.rs")
    );
    // 滚动后需要重新确认：此次按视口的位置查找改动，不切换文件；遍历各处改动后才再次提示。
    app.editor.active_mut().unwrap().scroll_line += 1.0;
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(app.diff.cross, None);
    for _ in 0..3 {
        if app.diff.cross.is_none() {
            assert!(app.run_action(Action::NextDifference));
        }
    }
    assert!(title(&app).starts_with("main.rs"));
    // 提示消失后同样需要重新确认。
    let (at, cross) = app.diff.cross.unwrap();
    app.diff.cross = Some((at - std::time::Duration::from_secs(60), cross));
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("main.rs"));

    // 再按一次进入下一个文件，仍在同一个标签，停在第一处；回到上一个文件时位于其最后一处。
    assert!(app.run_action(Action::NextDifference));
    assert!(title(&app).starts_with("zeta.rs"), "{}", title(&app));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(focus(&app), Some(0));
    // 该文件未打开：不为它新开标签，光标是右侧的只读光标。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(!view.editing);
    assert_eq!(view.read_caret(), Some((winc_editor::diff_view::RIGHT, 2)));
    paint(&mut app);
    // 没有更多文件时不提示再按一次。
    assert!(app.run_action(Action::NextDifference));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(app.status.message.as_deref(), Some("已是最后一处更改"));
    // 该文件有两处改动：先回到第一处，再提示，第三次回到上一个文件。
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(focus(&app), Some(0));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(title(&app).starts_with("zeta.rs"));
    assert!(app.run_action(Action::PreviousDifference));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(focus(&app), Some(1));

    // `F8` 切换文件时总是从第一处开始；到达末端时提示，位置不变。
    assert!(app.run_action(Action::PreviousDiffFile));
    assert_eq!(app.status.message.as_deref(), Some("已是第一个文件"));
    assert!(app.run_action(Action::NextDiffFile));
    assert!(title(&app).starts_with("zeta.rs"));
    assert!(app.run_action(Action::PreviousDiffFile));
    assert!(title(&app).starts_with("main.rs"));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.editor.documents.len(), tabs);

    // 点击差异后不再标记。
    paint(&mut app);
    let parts = crate::app::diff_parts(
        app.layout.editor,
        app.editor.active().unwrap().diff.as_deref().unwrap(),
        app.metrics.cell,
    );
    click(
        &mut app,
        (parts.panes[0].text.x + 20.0, parts.body.y + 30.0),
    );
    assert_eq!(focus(&app), None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn moving_to_a_change_clears_the_message_about_having_none() {
    let dir = search_folder("git-diff-messages");
    if !long_repo(&dir) {
        return;
    }
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };

    // 刚打开时向回没有改动；向下跳转到第一处后，该提示不再成立。
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.status.message.as_deref(), Some("已是第一处更改"));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.status.message, None);

    // 到达末端时提示方向对应的消息；反方向跳转后提示清除，不留下相反的消息。
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(app.status.message.as_deref(), Some("已是最后一处更改"));
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(focus(&app), Some(0));
    assert_eq!(app.status.message, None);
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.status.message.as_deref(), Some("已是第一处更改"));

    // 隐藏状态栏时消息在提示条上，同样清除。
    app.settings_toggle(Setting::StatusBar);
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.notice.text.as_deref(), Some("已是第一处更改"));
    assert!(app.run_action(Action::NextDifference));
    assert_eq!(focus(&app), Some(1));
    assert_eq!(app.notice.text, None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn diff_navigation_keys_work_with_focus_in_the_commit_list() {
    use crate::keymap::{KeyCode, Mods, Stroke};

    let dir = search_folder("git-diff-list-keys");
    if !long_repo(&dir) {
        return;
    }
    std::fs::remove_file(dir.join("src/new.rs")).unwrap();
    std::fs::write(dir.join("src/zeta.rs"), "a\n").unwrap();
    git_in(&dir, &["add", "src/zeta.rs"]);
    let main = winc_editor::canonical_path(&dir.join("src/main.rs"));
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git_refresh();
    app.fetch_changes(&main, Target::Open);
    paint(&mut app);
    let f = |n: u8, mods: Mods| Stroke {
        key: KeyCode::F(n),
        mods,
    };
    let focus = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        view.focus.map(|f| f.0)
    };
    let title = |app: &App| app.editor.active().unwrap().title.clone().unwrap();

    // 点击列表中的文件后焦点保留在列表：跳到改动、切换文件都生效，焦点不移动。
    app.focus = Focus::CommitList;
    assert!(app.on_shortcut(f(7, Mods::NONE)));
    assert_eq!(focus(&app), Some(0));
    assert!(app.on_shortcut(f(5, Mods::ALT)));
    assert_eq!(focus(&app), Some(1));
    assert!(app.on_shortcut(f(7, Mods::SHIFT)));
    assert_eq!(focus(&app), Some(0));
    assert!(app.on_shortcut(f(5, Mods::ALT | Mods::SHIFT)));
    assert_eq!(app.focus, Focus::CommitList);
    assert!(app.on_shortcut(f(8, Mods::NONE)));
    assert!(title(&app).starts_with("zeta.rs"), "{}", title(&app));
    assert!(app.on_shortcut(f(8, Mods::SHIFT)));
    assert!(title(&app).starts_with("main.rs"), "{}", title(&app));
    assert_eq!(app.focus, Focus::CommitList);

    // 其他键仍然由列表处理；焦点在文件树时不操作差异。
    assert!(!app.on_shortcut(Stroke {
        key: KeyCode::Down,
        mods: Mods::NONE,
    }));
    app.focus = Focus::Tree;
    assert!(!app.on_shortcut(f(7, Mods::NONE)));

    // 当前标签不是差异时，提交列表不处理这些键。
    app.focus = Focus::CommitList;
    let tab = app.editor.active.unwrap();
    app.close_tab(tab);
    assert!(!app.editor.active().is_some_and(|d| d.diff.is_some()));
    assert!(!app.on_shortcut(f(7, Mods::NONE)));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clicks_and_keys_in_a_diff_scroll_it_and_touch_nothing_else() {
    let Some((mut app, root)) = git_app("git-diff-keys", true) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run_action(Action::GitShowChanges);
    // 展开未改动的内容：超出一屏，可以滚动。
    app.press_diff(DiffTarget::Collapse);
    paint(&mut app);
    let scroll = app.editor.active().unwrap().scroll_line;

    // 点击正文：只获取焦点，不在不可见的文本中放置光标，视图不动；⌘ 点击也不转到定义。
    let body = app.layout.editor;
    click(&mut app, (body.x + body.w * 0.75, body.bottom() - 40.0));
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    assert_eq!(app.editor.active().unwrap().selection.head, 0);

    // 键盘滚动。
    app.diff_key(&Key::Named(NamedKey::ArrowDown), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll + 1.0);
    app.diff_key(&Key::Named(NamedKey::Home), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    app.diff_key(&Key::Named(NamedKey::End), false);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    app.diff_key(&Key::Named(NamedKey::PageUp), false);
    // 字符键不执行任何操作（差异只读）。
    let before = app.editor.active().unwrap().scroll_line;
    app.diff_key(&Key::Character("x".into()), false);
    assert_eq!(app.editor.active().unwrap().scroll_line, before);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diffs_follow_the_file_and_skip_binaries_and_stale_previews() {
    let Some((mut app, root)) = git_app("git-diff-live", false) else {
        return;
    };
    let main = root.join("src/main.rs");
    app.git_preview_changes(&main);
    let id = app.editor.active().unwrap().id;
    let new_text = |app: &App| {
        let doc = app.editor.document(id).unwrap();
        doc.diff.as_deref().unwrap().new.text.to_string()
    };
    assert_eq!(new_text(&app), "fn main() {}\n");

    // 文件再次变化：状态刷新以后，已打开的差异原地更新，不切换标签。
    std::fs::write(&main, "fn main() { run() }\n").unwrap();
    app.open_path(&root.join("src/config.rs"));
    let active = app.editor.active().unwrap().id;
    app.apply_changes(winc_editor::Changes {
        paths: vec![main.clone()],
        rescan: false,
        git: false,
    });
    assert_eq!(new_text(&app), "fn main() { run() }\n");
    assert_eq!(app.editor.active().unwrap().id, active);

    // 被忽略的路径里的变化不触发刷新。
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("target/debug/app"), "x").unwrap();
    std::fs::write(root.join(".gitignore"), "/target\n").unwrap();
    app.apply_changes(winc_editor::Changes {
        paths: vec![root.join(".gitignore")],
        rescan: false,
        git: false,
    });
    let generation = app.git.generation;
    app.apply_changes(winc_editor::Changes {
        paths: vec![root.join("target/debug/app")],
        rescan: false,
        git: false,
    });
    assert_eq!(app.git.generation, generation);

    // 过期的预览请求返回时不打开。
    let tabs = app.editor.documents.len();
    let seq = app.git.preview_seq;
    app.git.preview_seq += 1;
    app.fetch_changes(&root.join("src/new.rs"), Target::Preview(seq));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(app.editor.active().unwrap().id, active);

    // 二进制文件不显示差异，在状态栏说明。
    std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 0, 3]).unwrap();
    app.git_preview_changes(&root.join("blob.bin"));
    assert!(
        app.status
            .message
            .as_deref()
            .is_some_and(|m| m.contains("二进制")),
        "{:?}",
        app.status.message
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 测量：滚动差异标签与滚动普通文件时每帧的绘制耗时。
///
/// 运行方式：`cargo test -p winc-app measure_diff_scroll -- --ignored --nocapture`。
#[test]
#[ignore]
fn measure_diff_scroll() {
    use std::time::Instant;
    let dir = search_folder("git-diff-measure");
    let text: String = (0..3000)
        .map(|i| format!("    let value_{i} = compute(value_{i}, \"text {i}\");\n"))
        .collect();
    std::fs::write(dir.join("src/main.rs"), &text).unwrap();
    if !make_repo(&dir) {
        return;
    }
    let mut changed = String::new();
    for (i, line) in text.lines().enumerate() {
        match i % 25 {
            0 => changed.push_str(&line.replace("compute", "recompute")),
            7 => continue,
            13 => {
                changed.push_str(line);
                changed.push_str("\n    extra();");
            }
            _ => changed.push_str(line),
        }
        changed.push('\n');
    }
    std::fs::write(dir.join("src/main.rs"), changed).unwrap();
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    let frames = |app: &mut App, label: &str| {
        paint(app);
        let start = Instant::now();
        for _ in 0..60 {
            app.editor.active_mut().unwrap().scroll_line += 1.3;
            app.clamp_scroll();
            paint(app);
        }
        eprintln!("{label}: {:?} / 帧", start.elapsed() / 120);
    };
    frames(&mut app, "普通文件");
    app.run_action(Action::GitShowChanges);
    frames(&mut app, "差异");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：工作区删除了 `HEAD` 中间的一大段，滚动到该段中部：
/// 左侧逐行显示被删除的内容，右侧停在删除处。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_diff_long_deletion() {
    for dark in [false, true] {
        let name = if dark {
            "deletion-dark"
        } else {
            "deletion-light"
        };
        let dir = search_folder(name);
        if !make_repo(&dir) {
            return;
        }
        let lines: Vec<String> = (0..120).map(|i| format!("let value_{i} = {i};")).collect();
        std::fs::write(dir.join("long.rs"), lines.join("\n") + "\n").unwrap();
        git_in(&dir, &["add", "-A"]);
        git_in(&dir, &["commit", "-q", "-m", "long"]);
        let kept: Vec<&String> = lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !(30..90).contains(i))
            .map(|(_, l)| l)
            .collect();
        let kept: String = kept.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join("long.rs"), kept).unwrap();
        let mut app = app_with(&dir.join("long.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::GitShowChanges);
        paint(&mut app);
        let doc = app.editor.active_mut().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let span = view.hunk_span(0);
        doc.scroll_line = span.start + 20.0;
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 创建一个仓库：`src/wrap.rs` 中有数行很长的代码，工作区修改了其中一行。
fn long_lines_repo(name: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    let long = |tag: &str| {
        format!(
            "    let {tag} = {};\n",
            "value + other * third, ".repeat(16)
        )
    };
    let text = format!("fn main() {{\n{}{}}}\n", long("first"), long("second"));
    let file = dir.join("src/wrap.rs");
    std::fs::write(&file, &text).unwrap();
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "long"]);
    let changed = text.replace("let second = value", "let second = changed");
    std::fs::write(&file, changed).unwrap();
    Some((dir, winc_editor::canonical_path(&file)))
}

#[test]
fn word_wrap_applies_to_the_diff_view() {
    use crate::app::diff::DIFF_TEXT_PAD;

    let Some((dir, file)) = long_lines_repo("git-diff-wrap") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    fn view(app: &App) -> &winc_editor::diff_view::DiffView {
        app.editor.active().unwrap().diff.as_deref().unwrap()
    }
    assert!(!view(&app).is_wrapping());
    let plain = view(&app).rows[RIGHT].len();
    app.editor.active_mut().unwrap().scroll_x = 80.0;

    // 开启：长行分为多段，横向不再滚动。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    assert!(view(&app).is_wrapping());
    assert!(view(&app).rows[RIGHT].len() > plain);
    assert_eq!(app.editor.active().unwrap().scroll_x, 0.0);

    // 改动的字符只在其所在的段中标记：改动行后续各段的开头没有标记。
    {
        let view = view(&app);
        let row = view.rows[RIGHT]
            .iter()
            .position(|r| {
                matches!(
                    r,
                    SideRow::Line {
                        hunk: Some(_),
                        part: 1,
                        ..
                    }
                )
            })
            .unwrap();
        let SideRow::Line { line, .. } = view.rows[RIGHT][row] else {
            unreachable!();
        };
        let indent = view.breaks(RIGHT, line).indent as f32;
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        let lh = app.fonts.line_height;
        let scroll = view.side_scroll(RIGHT, 0.0, f64::from(parts.body.h / lh));
        let x = parts.panes[RIGHT].text.x + DIFF_TEXT_PAD + indent * app.metrics.cell;
        let y = parts.body.y + (row as f64 - scroll) as f32 * lh + 3.0;
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
        let pixel = |x: f32, y: f32| {
            let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
            image.pixels[y * image.width as usize + x]
        };
        assert_eq!(pixel(x + 0.5, y), pixel(x - 6.0, y));
    }

    // 点击一行的第二段：光标落在该段中对应的列。
    let (row, line) = view(&app).rows[RIGHT]
        .iter()
        .enumerate()
        .find_map(|(i, r)| match r {
            SideRow::Line { line, part: 1, .. } => Some((i, *line)),
            _ => None,
        })
        .unwrap();
    let breaks = view(&app).breaks(RIGHT, line);
    let parts = crate::app::diff_parts(app.layout.editor, view(&app), app.metrics.cell);
    let visible = f64::from(parts.body.h / app.fonts.line_height);
    let scroll = view(&app).side_scroll(RIGHT, 0.0, visible);
    let y = parts.body.y + ((row as f64 - scroll) as f32 + 0.5) * app.fonts.line_height;
    let cells = breaks.indent + 2;
    let x = parts.panes[RIGHT].text.x + DIFF_TEXT_PAD + (cells as f32 + 0.3) * app.metrics.cell;
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let source = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    let (at, col) = source.text().pos_to_line_col(source.selection.head);
    assert_eq!((at, col), (line, breaks.start_of(1) + 2));
    paint(&mut app);

    // 关闭：每行恢复为一个显示行。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    assert!(!view(&app).is_wrapping());
    assert_eq!(view(&app).rows[RIGHT].len(), plain);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn toggling_word_wrap_keeps_the_top_line_of_the_diff_in_place() {
    let Some((dir, file)) = uneven_lines_repo("git-diff-wrap-top") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.fetch_changes(&file, Target::Open);
    paint(&mut app);
    // 屏幕上右侧最上面的（行，段，行内偏移）。
    fn top(app: &App) -> (usize, usize, f64) {
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
        let visible = f64::from(parts.body.h / app.fonts.line_height);
        let row = view.side_scroll(RIGHT, doc.scroll_line, visible);
        let SideRow::Line { line, part, .. } = view.rows[RIGHT][row as usize] else {
            panic!("{row}");
        };
        (line, part, row.fract())
    }
    let start = {
        let doc = app.editor.active_mut().unwrap();
        let view = doc.diff.as_deref().unwrap();
        doc.scroll_line = view.pos_of(RIGHT, view.row_of_new(90) as f64 + 0.25);
        top(&app)
    };

    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let wrapped = top(&app);
    assert_eq!((wrapped.0, wrapped.1), (start.0, 0));
    assert!((wrapped.2 - start.2).abs() < 0.01, "{wrapped:?} {start:?}");

    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let plain = top(&app);
    assert_eq!(plain.0, start.0);
    assert!((plain.2 - start.2).abs() < 0.01, "{plain:?} {start:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 长短行相间、每隔数行删除一段的改动：顶边和中线之间总有两侧不等长的改动。
fn uneven_lines_repo(name: &str) -> Option<(PathBuf, PathBuf)> {
    let dir = search_folder(name);
    if !make_repo(&dir) {
        return None;
    }
    let line = |i: usize| match i % 3 {
        0 => format!("{i} {}\n", "value + other * third, ".repeat(16)),
        _ => format!("{i}\n"),
    };
    let file = dir.join("src/wrap.rs");
    std::fs::write(&file, (0..400).map(line).collect::<String>()).unwrap();
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "uneven"]);
    let kept = (0..400)
        .filter(|i| i % 12 >= 6)
        .map(line)
        .collect::<String>();
    std::fs::write(&file, kept).unwrap();
    Some((dir, winc_editor::canonical_path(&file)))
}

/// 截图：显示全部空白符的差异，深浅两套配色。
#[test]
#[ignore]
fn shoot_diff_whitespace() {
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        let dir = search_folder(&format!("whitespace-diff-shot-{theme}"));
        if !long_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run(winc_editor::Command::Insert("use std::fs;  \n".into()));
        app.run_action(Action::GitShowChanges);
        app.config.render_whitespace = Some(winc_config::RenderWhitespace::All);
        shoot(&mut app, &format!("whitespace-diff-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 回归：差异标签自身没有路径，文件树按它对应的工作区文件定位。切回文件视图、切换到该标签、
/// 点击「定位当前文件」都展开并选中该文件。
#[test]
fn a_diff_tab_is_located_in_the_file_tree_by_its_worktree_file() {
    let dir = search_folder("git-diff-locate");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run(winc_editor::Command::Insert("// 新的一行\n".into()));
    assert!(app.run_action(Action::GitShowChanges));
    let diff = app.editor.active.unwrap();
    assert!(app.editor.documents[diff].path.is_none());
    assert!(app.editor.documents[diff].diff.is_some());

    // 每一步之前收起全部目录并清除选中，文件所在的目录需要重新展开。
    let forget = |app: &mut App| {
        let ws = app.editor.workspace.as_mut().unwrap();
        ws.collapse_all();
        ws.selected = None;
    };
    let located = |app: &App| {
        let ws = app.editor.workspace.as_ref().unwrap();
        ws.selected.map(|index| ws.rows()[index].name.to_string())
    };

    forget(&mut app);
    app.set_sidebar_view(SidebarView::Search);
    app.set_sidebar_view(SidebarView::Files);
    assert_eq!(located(&app).as_deref(), Some("main.rs"));

    forget(&mut app);
    app.activate_tab(0);
    app.activate_tab(diff);
    assert_eq!(located(&app).as_deref(), Some("main.rs"));

    forget(&mut app);
    app.run_tree_tool(crate::app::TreeTool::Locate);
    assert_eq!(located(&app).as_deref(), Some("main.rs"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 差异标签中折叠命令作用于未改动的长段：全部展开、全部折叠，展开的段可以用行号栏中的收起标记
/// 单独折叠回去。
#[test]
fn fold_commands_and_the_gutter_mark_refold_unchanged_runs_in_a_diff() {
    let dir = search_folder("git-diff-refold");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run(winc_editor::Command::Insert("// 新的一行\n".into()));
    assert!(app.run_action(Action::GitShowChanges));
    let folds = |app: &App| {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let folded = view.rows[RIGHT]
            .iter()
            .filter(|r| matches!(r, SideRow::Fold { .. }));
        (folded.count(), view.open_folds.len())
    };
    let (folded, open) = folds(&app);
    assert!(folded > 0 && open == 0);

    // 全部展开：没有折叠行，每一段都带收起标记。
    assert!(app.run_action(Action::UnfoldAll));
    assert_eq!(folds(&app), (0, folded));

    // 点击一个收起标记：只有该段折叠回去。
    paint(&mut app);
    let mark = app.hits.diff.iter().find_map(|(rect, target)| {
        matches!(target, DiffTarget::Refold(_)).then_some((*rect, *target))
    });
    let (rect, target) = mark.expect("展开的段有收起标记");
    assert!(rect.w > 0.0 && rect.h > 0.0);
    app.press_diff(target);
    assert_eq!(folds(&app), (1, folded - 1));

    // 全部折叠：回到打开时的状态。
    assert!(app.run_action(Action::FoldAll));
    assert_eq!(folds(&app), (folded, 0));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 未改动内容展开、折叠的过渡：过渡期间差异保持展开的排版，折叠在过渡结束时写入；其间修改折叠
/// 状态或点击时先结束过渡。
#[test]
fn folding_an_unchanged_run_in_a_diff_is_applied_when_its_transition_ends() {
    use crate::app::animation::{Clock, FOLD_ANIMATION};

    let dir = search_folder("git-diff-fold-motion");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run(winc_editor::Command::Insert("// 新的一行\n".into()));
    assert!(app.run_action(Action::GitShowChanges));
    let open = |app: &App, gap| {
        app.editor
            .active()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .is_open(gap)
    };
    // 取编辑区顶部之下的第一段折叠。
    let gap = {
        let doc = app.editor.active().unwrap();
        let view = doc.diff.as_deref().unwrap();
        let top = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
        let below = view.rows[RIGHT].iter().skip(top + 1);
        let folds = below.filter_map(|row| match row {
            SideRow::Fold { gap, .. } => Some(*gap),
            SideRow::Line { .. } => None,
        });
        folds.into_iter().next().expect("顶部之下有折叠的段")
    };
    let (half, done) = (
        Clock::stopped(FOLD_ANIMATION, FOLD_ANIMATION / 2),
        Clock::stopped(FOLD_ANIMATION, FOLD_ANIMATION),
    );

    // 测试中没有窗口的时钟：展开立即生效，不留下过渡。
    app.press_diff(DiffTarget::Fold(gap));
    assert!(open(&app, gap) && app.motion.diff_fold.is_none());

    // 展开的过渡：差异已是展开状态，过渡期间可以正常绘制，结束后仍然展开。
    assert!(app.begin_diff_fold(gap, true, half));
    paint(&mut app);
    assert!(app.motion.diff_fold.is_some());
    assert!(app.begin_diff_fold(gap, true, done));
    app.settle_animations();
    assert!(app.motion.diff_fold.is_none() && open(&app, gap));

    // 折叠的过渡：进行中仍为展开状态，时钟走完后才折叠。
    assert!(app.begin_diff_fold(gap, false, half));
    paint(&mut app);
    app.settle_animations();
    assert!(app.motion.diff_fold.is_some() && open(&app, gap));
    assert!(app.begin_diff_fold(gap, false, done));
    app.settle_animations();
    assert!(app.motion.diff_fold.is_none() && !open(&app, gap));
    // 已折叠的段不能开始过渡。
    assert!(!app.begin_diff_fold(gap, false, half));

    // 过渡期间执行其他折叠命令：先写入进行中的折叠，再执行该命令。
    app.press_diff(DiffTarget::Fold(gap));
    assert!(app.begin_diff_fold(gap, false, half));
    assert!(app.run_action(Action::UnfoldAll));
    assert!(app.motion.diff_fold.is_none() && open(&app, gap));

    // 过渡期间点击：先结束过渡。
    assert!(app.begin_diff_fold(gap, false, half));
    let (x, y) = (
        app.layout.editor.right() - 40.0,
        app.layout.editor.bottom() - 10.0,
    );
    app.click_diff(x, y);
    assert!(app.motion.diff_fold.is_none() && !open(&app, gap));
    std::fs::remove_dir_all(&dir).unwrap();
}
