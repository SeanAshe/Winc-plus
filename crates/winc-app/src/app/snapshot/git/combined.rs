//! 「未提交的更改」标签：各段的顺序和比较的内容、跨文件跳转、折叠、与差异标签对等的操作、
//! 合并差异视图的开关、滚动中的排版，以及跟随 Git 状态和编辑器更新。

use super::*;
use crate::app::git::DiffKind;
use crate::app::git::combined::{Body, CombinedHit};
use winc_editor::diff_view::{LEFT, RIGHT};

/// 第 `i` 段上一帧画出的差异部件。
fn section_hits(app: &App, i: usize) -> Vec<(Rect, DiffTarget)> {
    let tab = app.combined_tab().unwrap();
    (tab.section_hits.iter())
        .find(|(at, _)| *at == i)
        .map(|(_, hits)| hits.clone())
        .unwrap_or_default()
}

/// 各段：（相对仓库的路径，比较的内容，状态字母）。
fn sections(app: &App) -> Vec<(String, DiffKind, char)> {
    let root = app.git.root.clone().unwrap();
    let tab = app.combined_tab().expect("开着「未提交的更改」");
    (tab.sections.iter())
        .map(|s| {
            let path = s.path.strip_prefix(&root).unwrap_or(&s.path);
            (path.display().to_string(), s.kind, s.change.letter())
        })
        .collect()
}

/// 第 `i` 段的差异。
fn view(app: &App, i: usize) -> &winc_editor::diff_view::DiffView {
    match &app.combined_tab().unwrap().sections[i].body {
        Body::Ready(view) => view,
        _ => panic!("第 {i} 段还没有差异"),
    }
}

/// 当前的滚动位置。
fn scroll(app: &App) -> f64 {
    app.editor.active().unwrap().scroll_line
}

/// 在仓库里加三个 40 行的新文件 `src/big_0.rs` … `src/big_2.rs`，标签比一屏长。
fn add_big_files(root: &Path) {
    let text: String = (0..40).map(|i| format!("let line_{i} = {i};\n")).collect();
    for i in 0..3 {
        std::fs::write(root.join(format!("src/big_{i}.rs")), &text).unwrap();
    }
}

/// 路径以 `name` 结尾的那一段在第几段。
fn section_of(app: &App, name: &str) -> usize {
    let sections = &app.combined_tab().unwrap().sections;
    sections
        .iter()
        .position(|s| s.path.ends_with(name))
        .unwrap()
}

/// 勾选式下按提交视图的顺序列出各段（已跟踪的更改在前，未跟踪的文件在后），每段比较上次提交
/// 与工作区；F7 跨文件跳到下一处更改，F8 跳到下一个文件的段头，段头可以折叠。
#[test]
fn all_changes_line_up_file_by_file() {
    let Some((mut app, _root)) = git_app("git-combined", true) else {
        return;
    };
    assert!(app.run_action(Action::GitShowAllChanges));
    assert!(app.combined_active());
    // 标题跟着视口顶边的那个文件。
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.title.as_deref(), Some("main.rs · 差异"));
    assert_eq!(
        sections(&app),
        [
            ("src/main.rs".to_owned(), DiffKind::Whole, 'M'),
            ("src/new.rs".to_owned(), DiffKind::Whole, 'U'),
        ]
    );
    // 每段与单独打开它的差异相同：右侧可以编辑，对应提交视图中的文件。
    let main = view(&app, 0);
    assert_eq!(main.is_unified(), app.editor.diff_unified);
    assert!(main.new.path.is_some() && main.repo_path.is_some());
    assert_eq!(main.hunks.len(), 2);

    paint(&mut app);
    let hits: Vec<CombinedHit> = app.hits.combined.iter().map(|(_, h)| *h).collect();
    for hit in [
        CombinedHit::Header(0),
        CombinedHit::Header(1),
        CombinedHit::Previous,
        CombinedHit::Next,
        CombinedHit::Collapse,
        CombinedHit::Layout,
        CombinedHit::View,
    ] {
        assert!(hits.contains(&hit), "{hit:?} {hits:?}");
    }
    // 正文里与差异标签相同：折起的未改动内容可以点开，有勾选框和「»」。
    let diff: Vec<DiffTarget> = section_hits(&app, 0).into_iter().map(|(_, t)| t).collect();
    for want in [
        DiffTarget::Fold(1),
        DiffTarget::Include(0),
        DiffTarget::Revert(0),
    ] {
        assert!(diff.contains(&want), "{want:?} {diff:?}");
    }

    // 加几个长文件，标签比一屏长。F7 依次走过第一个文件的两处更改，再进到第二个文件。
    add_big_files(&app.git.root.clone().unwrap());
    app.git_refresh();
    let starts = app.combined_starts();
    let header = app.combined_header_rows();
    let hunk = |app: &App, i: usize, h: usize| starts[i] + header + view(app, i).hunk_pos(h);
    let mut seen = Vec::new();
    for _ in 0..3 {
        assert!(app.run_action(Action::NextDifference));
        seen.push(app.combined_focus().unwrap());
    }
    let expected = [hunk(&app, 0, 0), hunk(&app, 0, 1), hunk(&app, 1, 0)];
    assert_eq!(seen, expected);
    // ⇧F7 回到上一处；一直往下停在最后一个文件的最后一处，之后不再动。
    assert!(app.run_action(Action::PreviousDifference));
    assert_eq!(app.combined_focus(), Some(expected[1]));
    let count = app.combined_tab().unwrap().sections.len();
    let last = hunk(&app, count - 1, view(&app, count - 1).hunks.len() - 1);
    for _ in 0..10 {
        assert!(app.run_action(Action::NextDifference));
    }
    assert_eq!(app.combined_focus(), Some(last));

    // F8 跳到下一个、上一个文件的段头（段头那一行在顶边，上面的间隔滚出去）。
    let heads = app.combined_heads();
    app.set_active_scroll(0.0);
    assert!(app.run_action(Action::NextDiffFile));
    assert_eq!(scroll(&app), heads[1]);
    assert!(app.run_action(Action::NextDiffFile));
    assert_eq!(scroll(&app), heads[2]);
    assert!(app.run_action(Action::PreviousDiffFile));
    assert_eq!(scroll(&app), heads[1]);

    // 点段头折叠第一段：它只剩段头，第二段跟着上来。按住 ⌥ 点段头：跟着这一段全部展开或折叠。
    let folded = |app: &App| -> Vec<bool> {
        let sections = &app.combined_tab().unwrap().sections;
        sections.iter().map(|s| s.folded).collect()
    };
    app.press_combined(CombinedHit::Header(0));
    assert!(folded(&app)[0]);
    let starts = app.combined_starts();
    assert!((starts[1] - starts[0] - header).abs() < 1e-9);
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    app.press_combined(CombinedHit::Header(0));
    assert!(folded(&app).iter().all(|f| !f));
    app.press_combined(CombinedHit::Header(1));
    assert!(folded(&app).iter().all(|f| *f));
}

/// 暂存区视图中同一个文件在「暂存的更改」和「更改」两组里各成一段，分别比较上次提交与暂存区、
/// 暂存区与工作区，顺序同提交视图；未跟踪的文件在「更改」一组，同样比较暂存区与工作区。
#[test]
fn the_staging_area_splits_a_file_into_staged_and_unstaged_parts() {
    let Some((mut app, root)) = git_app("git-combined-staging", true) else {
        return;
    };
    git_in(&root, &["add", "src/main.rs"]);
    let text = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        text.replace("value_9 = 9", "value_9 = 99"),
    )
    .unwrap();
    app.settings_toggle(Setting::GitStagingArea);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    assert_eq!(
        sections(&app),
        [
            ("src/main.rs".to_owned(), DiffKind::Staged, 'M'),
            ("src/main.rs".to_owned(), DiffKind::Unstaged, 'M'),
            ("src/new.rs".to_owned(), DiffKind::Unstaged, 'U'),
        ]
    );
    // 暂存的两处和之后在工作区里又改的一处分在两段里。
    assert_eq!(view(&app, 0).hunks.len(), 2);
    assert_eq!(view(&app, 1).hunks.len(), 1);
}

/// 「合并差异视图」开关在两种差异的标题栏最右端，与设置是同一项：从提交视图打开的差异上打开，
/// 换成「未提交的更改」并停在这个文件，预览标签换掉；之后在提交视图中点文件滚到它那一段，焦点
/// 留在列表里。在「未提交的更改」上关掉，换回视口顶边那个文件的差异；在设置里切换同样立即生效。
#[test]
fn the_combined_view_toggle_switches_between_the_two_diffs() {
    let Some((mut app, root)) = git_app("git-combined-toggle", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    app.show_commit();
    app.git_preview_changes(&root.join("src/big_1.rs"));
    paint(&mut app);
    let preview = app.editor.active().unwrap().id;
    let tabs = app.editor.documents.len();
    assert!(app.editor.active().unwrap().diff.is_some());
    let button = (app.hits.diff.iter())
        .find(|(_, t)| *t == DiffTarget::Combined)
        .expect("差异标题栏上有开关")
        .0;
    click(&mut app, (button.x + 4.0, button.y + 4.0));
    assert_eq!(app.config.git_combined_diff, Some(true));
    // 同一个标签换了显示，标题还是这个文件。
    assert!(app.combined_active());
    assert_eq!(app.editor.active().unwrap().id, preview);
    assert_eq!(app.editor.documents.len(), tabs);
    let i = section_of(&app, "src/big_1.rs");
    assert_eq!(scroll(&app), app.combined_heads()[i]);
    paint(&mut app);
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("big_1.rs · 差异")
    );

    let focus = app.focus;
    app.git_preview_changes(&root.join("src/big_2.rs"));
    assert!(app.combined_active());
    assert_eq!(app.focus, focus);
    let i = section_of(&app, "src/big_2.rs");
    assert_eq!(scroll(&app), app.combined_heads()[i]);

    // 工具栏最右端的开关：关掉，换回视口顶边那个文件自己的差异。
    paint(&mut app);
    let button = (app.hits.combined.iter())
        .find(|(_, h)| *h == CombinedHit::View)
        .unwrap()
        .0;
    click(&mut app, (button.x + 4.0, button.y + 4.0));
    assert_eq!(app.config.git_combined_diff, Some(false));
    assert!(app.combined_tab().is_none());
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.id, preview);
    assert!(
        doc.diff
            .as_deref()
            .is_some_and(|v| v.file_path().unwrap().ends_with("big_2.rs"))
    );
    assert_eq!(doc.title.as_deref(), Some("big_2.rs · 差异"));

    // 设置里打开：同样立即换过去。
    app.settings_toggle(Setting::GitCombinedDiff);
    assert!(app.combined_active());
}

/// 开着缩略图时它不占「未提交的更改」：工具栏右端的按钮点得到。
#[test]
fn the_minimap_leaves_the_combined_toolbar_alone() {
    let Some((mut app, root)) = git_app("git-combined-minimap", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    app.settings_toggle(Setting::Minimap);
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    assert_eq!(app.layout.minimap.w, 0.0);
    let next = (app.hits.combined.iter())
        .find(|(_, h)| *h == CombinedHit::Next)
        .unwrap()
        .0;
    click(&mut app, (next.x + next.w / 2.0, next.y + next.h / 2.0));
    assert!(app.combined_focus().is_some());
}

/// 并排时两侧按改动对齐：每处更改占两侧中较多的行数，较短的一侧在下面留空，更改之后两侧
/// 逐行对应；切换单栏、并排时各段跟着换。自动换行按设置给各段断行。
#[test]
fn side_by_side_sections_align_both_sides() {
    let Some((mut app, _root)) = git_app("git-combined-sides", true) else {
        return;
    };
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    // 默认并排（同差异标签），两侧按改动对齐。
    let main = section_of(&app, "src/main.rs");
    let view = view(&app, main);
    assert!(!view.is_unified() && view.aligned);
    let map = view.unified_map();
    // 删除一行的那处更改之后，两侧同一行的位置相同。
    let (left, right) = (view.row_of(LEFT, 52), view.row_of(RIGHT, 51));
    assert_eq!(
        map.aligned_pos(LEFT, left as f64),
        map.aligned_pos(RIGHT, right as f64)
    );
    app.press_combined(CombinedHit::Layout);
    assert!(view_of(&app, main).is_unified());

    // 自动换行作用于所有文件。
    app.settings_toggle(Setting::WordWrap);
    paint(&mut app);
    let sections = &app.combined_tab().unwrap().sections;
    for section in sections {
        if let Body::Ready(view) = &section.body {
            assert!(view.wrap().is_some(), "{}", section.path.display());
        }
    }
}

/// 第 `i` 段的差异（借用 `app` 的另一份写法，方便在可变借用之后再取）。
fn view_of(app: &App, i: usize) -> &winc_editor::diff_view::DiffView {
    view(app, i)
}

/// ⌘F 在各段中查找：选中的匹配所在的段展开、滚到看得见，作为那一段的选区；F3 到下一个文件。
#[test]
fn find_walks_matches_across_files() {
    let Some((mut app, root)) = git_app("git-combined-find", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    assert!(app.run_action(Action::Find));
    assert!(app.bars.find.open);
    app.bars.find.input.set_text("line_30 ");
    app.bars.search.query = "line_30 ".into();
    app.search_changed();
    paint(&mut app);
    let selected = |app: &App| -> Vec<usize> {
        let sections = &app.combined_tab().unwrap().sections;
        (sections.iter().enumerate())
            .filter(|(_, s)| matches!(&s.body, Body::Ready(v) if v.read_selection.is_some()))
            .map(|(i, _)| i)
            .collect()
    };
    let first = section_of(&app, "src/big_0.rs");
    assert_eq!(selected(&app), [first]);
    assert!(scroll(&app) > app.combined_starts()[first]);
    app.find_next(false);
    assert_eq!(selected(&app), [section_of(&app, "src/big_1.rs")]);
    // Git 状态刷新、各段没有变化：查找不重新开始，接着往下找。
    app.git_refresh();
    paint(&mut app);
    app.find_next(false);
    assert_eq!(selected(&app), [section_of(&app, "src/big_2.rs")]);
}

/// 命令「Git：逐行追溯」在各段中打开能追溯的一侧，按段分别追溯；再执行一次关闭。
#[test]
fn blame_runs_for_each_section() {
    let Some((mut app, _root)) = git_app("git-combined-blame", true) else {
        return;
    };
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    let main = section_of(&app, "src/main.rs");
    assert!(view(&app, main).blamed[LEFT]);
    let key = |app: &App, side| {
        let tab = app.combined_tab().unwrap();
        crate::app::git::blame::BlameKey::Section(tab.doc, tab.sections[main].id, side)
    };
    assert!(app.blame_of_key(key(&app, LEFT)).is_some());
    // 正文左边多出追溯一栏，可以点。
    assert!(
        section_hits(&app, main)
            .iter()
            .any(|(_, t)| matches!(t, DiffTarget::Blame(..)))
    );
    assert!(app.run_action(Action::GitBlame));
    assert!(!view(&app, main).blamed.contains(&true));
}

/// 跟着更新：新出现的文件加一段，编辑器里还没保存的修改重新比较那一段；段头先列出来，看得见的
/// 位置不动。
#[test]
fn the_combined_tab_follows_the_status_and_the_editor() {
    let Some((mut app, root)) = git_app("git-combined-follow", true) else {
        return;
    };
    assert!(app.run_action(Action::GitShowAllChanges));
    std::fs::write(root.join("a.txt"), "changed\n").unwrap();
    app.git_refresh();
    let paths: Vec<String> = sections(&app).into_iter().map(|s| s.0).collect();
    assert!(paths.contains(&"a.txt".to_owned()), "{paths:?}");

    // 在编辑器里改 `src/main.rs`（不保存），回到标签：那一段多了一处更改。
    let before = view(&app, section_of(&app, "src/main.rs")).hunks.len();
    app.open_path(&root.join("src/main.rs"));
    app.run(winc_editor::Command::Insert("// 新的一行\n".into()));
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let main = section_of(&app, "src/main.rs");
    assert_eq!(view(&app, main).hunks.len(), before + 1);

    // 关掉标签：状态一起丢掉。
    let index = app.editor.active.unwrap();
    app.close_tab(index);
    assert!(app.combined_tab().is_none());
}

/// 「未提交的更改」标签的样子：浅色和深色，开头和滚到中间（段头吸顶）。
///
/// 运行：`cargo test -p winc-app shoot_combined -- --ignored`，图片写到 `WINC_SNAPSHOT_DIR`
/// （默认为临时目录下的 `winc-snapshots`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_combined() {
    for dark in [false, true] {
        let Some((mut app, root)) = git_app_in("git-combined-shot", true, dark) else {
            return;
        };
        add_big_files(&root);
        app.git_refresh();
        app.show_commit();
        assert!(app.run_action(Action::GitShowAllChanges));
        paint(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("combined-{theme}"));
        let starts = app.combined_starts();
        app.set_active_scroll(starts[1] + 6.0);
        // 鼠标停在吸顶的段头上，露出它的按钮。
        let body = app.combined_parts().body;
        app.on_mouse_move(body.x + body.w / 2.0, body.y + 20.0);
        shoot(&mut app, &format!("combined-scrolled-{theme}"));
    }
}

/// 在某一段里与差异标签一样操作：点勾选框把这一处移出提交、文件成为部分提交；点进右侧的文字
/// 放下光标，键入的文字写进这个文件。
#[test]
fn sections_stage_and_edit_like_the_diff_tab() {
    let Some((mut app, root)) = git_app("git-combined-edit", true) else {
        return;
    };
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let main = section_of(&app, "src/main.rs");
    let check = (section_hits(&app, main).into_iter())
        .find(|(_, t)| *t == DiffTarget::Include(0))
        .unwrap()
        .0;
    click(&mut app, (check.x + check.w / 2.0, check.y + check.h / 2.0));
    let status = app.git.status.as_ref().unwrap();
    let entry = status
        .entries
        .iter()
        .find(|e| e.path.ends_with("src/main.rs"))
        .unwrap();
    assert_eq!(app.git.commit.file_state(entry), Some(false));

    // 点在第一行的文字上，键入一个字：写进编辑器里的 `src/main.rs`。
    paint(&mut app);
    let (area, _) = app.combined_section_area(main).unwrap();
    let view = view(&app, main);
    let parts = crate::app::diff_parts(area, view, app.metrics.cell);
    let text = parts.panes[RIGHT].text;
    let at = (text.x + 8.0, text.y + app.fonts.line_height / 2.0);
    click(&mut app, at);
    assert!(view_of(&app, main).editing);
    assert!(app.diff_edit_text("x"));
    let path = winc_editor::canonical_path(&root.join("src/main.rs"));
    let doc = app
        .editor
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(path.as_path()));
    let first = doc.and_then(|d| d.text_range(0, 1));
    assert_eq!(first.as_deref(), Some("x"), "没有写进文件");
}

/// 回归：点在滚到下面的一段上，交给的是鼠标下的那一段，不是第一段；点击和绘制都不改变滚动
/// 位置（平滑滚动中绘制用的是显示中的位置，不能写回去）。
#[test]
fn clicks_go_to_the_section_under_the_pointer() {
    let Some((mut app, root)) = git_app("git-combined-click", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let i = section_of(&app, "src/big_1.rs");
    app.set_active_scroll(app.combined_starts()[i] + 10.0);
    paint(&mut app);
    let before = scroll(&app);
    paint(&mut app);
    assert_eq!(scroll(&app), before);
    let body = app.combined_parts().body;
    click(&mut app, (body.x + body.w * 0.75, body.y + body.h / 2.0));
    paint(&mut app);
    assert_eq!(scroll(&app), before);
    assert_eq!(app.combined_tab().unwrap().current, Some(i));
}

/// 回归：平滑滚动中逐帧绘制（绘制时用显示中的滚动位置），开着逐行追溯和自动换行，编辑器里的
/// 修改让一段变高：每一帧的段头都在按这一帧的位置算出的地方，正文紧接在段头下面，不重叠。
#[test]
fn every_frame_keeps_headers_and_bodies_in_place() {
    let Some((mut app, root)) = git_app("git-combined-probe-scroll", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    app.settings_toggle(Setting::WordWrap);
    assert!(app.run_action(Action::GitBlame));
    let combined = app.editor.active().unwrap().id;
    paint(&mut app);
    let mut renderer = app.frame.renderer.take().unwrap();
    for step in 0..30 {
        if step % 7 == 3 {
            app.open_path(&root.join("src/big_0.rs"));
            app.run(winc_editor::Command::Insert(
                "let inserted_line_that_is_rather_long_so_it_wraps_around_the_view_width = 1;\n"
                    .into(),
            ));
            let index = app
                .editor
                .documents
                .iter()
                .position(|d| d.id == combined)
                .unwrap();
            app.activate_tab(index);
        }
        if step % 10 == 0 {
            let before = scroll(&app);
            app.run_action(Action::NextDiffFile);
            let doc = app.editor.active().unwrap().id;
            app.scroll.glide = Some(crate::app::scroll::ScrollGlide {
                doc,
                from: before,
                clock: crate::app::animation::Clock::new(std::time::Duration::from_millis(300)),
            });
        }
        app.relayout();
        let frame = app.begin_scroll_frame();
        renderer.draw(|f| crate::view::draw(&mut app, f));
        let shown = scroll(&app);
        let starts = app.combined_starts();
        let body = app.combined_parts().body;
        let lh = f64::from(app.fonts.line_height);
        let mut wrong = Vec::new();
        for (rect, hit) in &app.hits.combined {
            if let CombinedHit::Header(i) = hit {
                let want = body.y + ((starts[*i] - shown) * lh) as f32 + app.combined_gap();
                if rect.y > body.y + 1.0 && (rect.y - want).abs() > 0.5 {
                    wrong.push((step, *i, rect.y, want));
                }
                if let Some((area, _)) = app.combined_section_area(*i)
                    && rect.y > body.y + 1.0
                    && (area.y + crate::app::diff::DIFF_HEADER - rect.bottom()).abs() > 0.5
                {
                    let top = area.y + crate::app::diff::DIFF_HEADER;
                    wrong.push((step, *i, top, rect.bottom()));
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:?}");
        app.end_scroll_frame(frame);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    app.frame.renderer = Some(renderer);
}

/// 回归：鼠标停在某一段的改动行上，这一行的勾选框显示出来（悬停按这一段的部件算），绘制后
/// 重新判断悬停时不被标签下面的空文档盖掉；滚动后鼠标下换了一段也一样。
#[test]
fn hovering_a_changed_line_shows_its_checkbox() {
    let Some((mut app, root)) = git_app("git-combined-hover", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    for name in ["src/main.rs", "src/big_1.rs"] {
        let i = section_of(&app, name);
        app.set_active_scroll(app.combined_starts()[i]);
        paint(&mut app);
        let (rect, target) = (section_hits(&app, i).into_iter())
            .find(|(r, t)| matches!(t, DiffTarget::IncludeLine(..)) && r.h > 0.0)
            .expect("有改动行的勾选格");
        let at = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let cells = |app: &App| -> Vec<u32> {
            let xs = (0..rect.w as usize).map(|dx| rect.x + dx as f32 + 0.5);
            xs.map(|x| pixel(app, (x, at.1))).collect()
        };
        let before = cells(&app);
        app.on_mouse_move(at.0, at.1);
        paint(&mut app);
        assert_eq!(app.pointer.hover, Hover::Diff(target), "{name}");
        assert_ne!(cells(&app), before, "{name} 的勾选框没有画出来");
        app.on_mouse_move(0.0, 0.0);
    }
}

/// 回归：视口顶边落在两段之间的间隔里时，下面那一段的段头还没滚过顶边，不再贴一份在顶边
/// （贴上的一份会和它本身叠在一起）；上一段贴着的段头已被顶出去。
#[test]
fn a_header_is_never_pinned_over_itself() {
    let Some((mut app, root)) = git_app("git-combined-pin", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let gap = f64::from(app.combined_gap() / app.fonts.line_height);
    let count = app.combined_tab().unwrap().sections.len();
    for i in 1..count {
        for part in [0.25, 0.5, 0.95] {
            app.set_active_scroll(app.combined_starts()[i] + gap * part);
            paint(&mut app);
            let body = app.combined_parts().body;
            let mut shown: Vec<usize> = (app.hits.combined.iter())
                .filter(|(r, _)| r.intersect(body).h > 0.0)
                .filter_map(|(_, hit)| match hit {
                    CombinedHit::Header(j) => Some(*j),
                    _ => None,
                })
                .collect();
            let all = shown.len();
            shown.dedup();
            shown.sort_unstable();
            shown.dedup();
            assert_eq!(shown.len(), all, "第 {i} 段，间隔的 {part}：{shown:?}");
        }
    }
}

/// 回归：一段正在展开未更改的内容时滚动：这一段的正文按过渡中的高度画，后面的段头紧接在它
/// 下面；滚动一行，改动整体上移一行，不在过渡和不过渡之间来回跳。
#[test]
fn scrolling_during_an_unfold_keeps_the_layout() {
    let Some((mut app, root)) = git_app("git-combined-unfold", true) else {
        return;
    };
    let text: String = (0..40).map(|i| format!("let far_{i} = {i};\n")).collect();
    std::fs::write(root.join("src/far.rs"), &text).unwrap();
    git_in(&root, &["add", "."]);
    git_in(&root, &["commit", "-qm", "far"]);
    let changed = text
        .replace("let far_3 = 3;", "let far_3 = 30;")
        .replace("let far_30 = 30;", "let far_30 = 300;");
    std::fs::write(root.join("src/far.rs"), changed).unwrap();
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let i = section_of(&app, "src/far.rs");
    let gap = (view(&app, i).rows[RIGHT].iter())
        .find_map(|row| match row {
            winc_editor::diff_view::SideRow::Fold { gap, .. } => Some(*gap),
            winc_editor::diff_view::SideRow::Line { .. } => None,
        })
        .expect("两处改动之间有折叠的内容");
    let start = app.combined_starts()[i] + app.combined_header_rows() + 1.0;
    app.set_active_scroll(start);
    paint(&mut app);
    let half = crate::app::animation::Clock::stopped(
        crate::app::animation::FOLD_ANIMATION,
        crate::app::animation::FOLD_ANIMATION / 2,
    );
    let begun = app.with_combined_section(i, |app| {
        app.press_diff(DiffTarget::Fold(gap));
        app.begin_diff_fold(gap, true, half)
    });
    assert_eq!(begun, Some(true));
    let lh = app.fonts.line_height;
    let mut tops = Vec::new();
    for step in 0..2 {
        app.set_active_scroll(start + f64::from(step));
        paint(&mut app);
        assert!(app.motion.diff_fold.is_some());
        let last = view(&app, i).hunks.len() - 1;
        let hunk = (section_hits(&app, i).into_iter())
            .find(|(_, t)| *t == DiffTarget::Include(last))
            .expect("第二处改动在屏幕上")
            .0;
        tops.push(hunk.y);
        // 正文的底边就是下一段的间隔顶边。
        let (area, _) = app.combined_section_area(i).unwrap();
        let next = app.combined_starts()[i + 1];
        let body = app.combined_parts().body;
        let want = body.y + ((next - scroll(&app)) * f64::from(lh)) as f32;
        assert!(
            (area.bottom() - want).abs() < 0.5,
            "{} != {want}",
            area.bottom()
        );
    }
    assert!((tops[0] - tops[1] - lh).abs() < 0.5, "{tops:?}");
    // 过渡中第二处改动比展开完时高出还没露出的那一截。
    let hidden = app.motion.diff_fold.as_ref().unwrap().hidden(lh);
    app.motion.diff_fold = None;
    paint(&mut app);
    let last = view(&app, i).hunks.len() - 1;
    let settled = (section_hits(&app, i).into_iter())
        .find(|(_, t)| *t == DiffTarget::Include(last))
        .unwrap()
        .0;
    assert!(hidden > 0.0);
    assert!(
        (settled.y - tops[1] - hidden).abs() < 0.5,
        "{} {tops:?} {hidden}",
        settled.y
    );
}

/// 刷新 Git 状态时，磁盘上没改过的文件不重新比较：差异原样留着（高亮、展开的内容、追溯都不
/// 重来，滚动中不闪）；文件改了的那一段重新比较。
#[test]
fn refreshing_keeps_sections_whose_files_did_not_change() {
    let Some((mut app, root)) = git_app("git-combined-keep", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let address =
        |app: &App, name: &str| std::ptr::from_ref(view(app, section_of(app, name))) as usize;
    let (kept, changed) = (address(&app, "src/big_0.rs"), address(&app, "src/big_1.rs"));
    let stats = view(&app, section_of(&app, "src/big_1.rs")).stats();

    let text: String = (0..41).map(|i| format!("let line_{i} = {i};\n")).collect();
    std::fs::write(root.join("src/big_1.rs"), text).unwrap();
    app.git_refresh();
    paint(&mut app);
    assert_eq!(
        address(&app, "src/big_0.rs"),
        kept,
        "没改过的文件重新比较了"
    );
    assert_ne!(address(&app, "src/big_1.rs"), changed);
    let big_1 = view(&app, section_of(&app, "src/big_1.rs"));
    assert_eq!(big_1.stats(), (stats.0 + 1, stats.1));
}

/// 回归：从头到尾逐半行滚过整个标签，每一帧每一段的差异都只画在它自己露出来的那一块里。
/// 段头刚从底边上来、正文还在视口外时，这一段曾按整个正文区从开头画，盖在上一段上。
#[test]
fn sections_only_draw_inside_their_own_area() {
    let Some((mut app, root)) = git_app("git-combined-sweep", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let end = app.combined_rows() as f64;
    let mut at = 0.0;
    while at < end {
        app.set_active_scroll(at);
        paint(&mut app);
        let tab = app.combined_tab().unwrap();
        for (i, hits) in &tab.section_hits {
            let hits: Vec<_> = hits.iter().filter(|(r, _)| r.h > 0.0).collect();
            if hits.is_empty() {
                continue;
            }
            let Some((area, _)) = app.combined_section_area(*i) else {
                panic!("滚到 {at}：第 {i} 段看不见，却画了 {hits:?}");
            };
            let top = area.y + crate::app::diff::DIFF_HEADER;
            for (rect, target) in hits {
                assert!(
                    rect.y >= top - 0.5 && rect.bottom() <= area.bottom() + 0.5,
                    "滚到 {at}：第 {i} 段的 {target:?} 在 {rect:?}，露出的是 {top}..{}",
                    area.bottom()
                );
            }
        }
        at += 0.5;
    }
}

/// 第一段上面没有间隔：段头贴着工具栏下面；第二段起段头上面有一行高的间隔。
#[test]
fn only_sections_after_the_first_have_a_gap() {
    let Some((mut app, root)) = git_app("git-combined-first-gap", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let body = app.combined_parts().body;
    let header = |app: &App, i: usize| {
        (app.hits.combined.iter())
            .find(|(_, hit)| *hit == CombinedHit::Header(i))
            .map(|(r, _)| *r)
            .unwrap()
    };
    assert_eq!(header(&app, 0).y, body.y);
    let first = app.combined_section_area(0).unwrap().0;
    let gap = header(&app, 1).y - first.bottom();
    assert!((gap - app.combined_gap()).abs() < 0.5, "{gap}");
}

/// 点段头折叠、展开一段带过渡：过渡中正文按比例变矮（变高），后面的段跟着移动；时钟走完后
/// 折叠写入这一段。
#[test]
fn folding_a_section_animates_its_height() {
    let Some((mut app, root)) = git_app("git-combined-fold-motion", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let i = section_of(&app, "src/big_0.rs");
    let height = |app: &App| app.combined_starts()[i + 1] - app.combined_starts()[i];
    let open = height(&app);
    let head = app.combined_header_rows();
    let rows = open - head;
    let (half, done) = (
        crate::app::animation::Clock::stopped(
            crate::app::animation::FOLD_ANIMATION,
            crate::app::animation::FOLD_ANIMATION / 2,
        ),
        crate::app::animation::Clock::stopped(
            crate::app::animation::FOLD_ANIMATION,
            crate::app::animation::FOLD_ANIMATION,
        ),
    );

    // 折叠：过渡中还没折叠，正文露出的比例与箭头的转角一致。
    app.fold_combined_section(i, Some(half));
    paint(&mut app);
    let shown = app.combined_section_open(i);
    assert!(shown > 0.0 && shown < 1.0, "{shown}");
    assert!((height(&app) - head - rows * f64::from(shown)).abs() < 1e-6);
    assert!(!app.combined_tab().unwrap().sections[i].folded);
    // 正文只画在露出的那一截里。
    let (area, _) = app.combined_section_area(i).unwrap();
    for (rect, _) in section_hits(&app, i).iter().filter(|(r, _)| r.h > 0.0) {
        assert!(rect.bottom() <= area.bottom() + 0.5, "{rect:?} {area:?}");
    }
    app.combined_tab_mut_for_test().fold.as_mut().unwrap().clock = done;
    app.settle_animations();
    assert!(app.combined_tab().unwrap().sections[i].folded);
    assert!((height(&app) - head).abs() < 1e-6);

    // 展开：一开始就按展开算，正文从 0 长到全高。
    app.fold_combined_section(i, Some(half));
    assert!(!app.combined_tab().unwrap().sections[i].folded);
    let grown = height(&app) - head;
    assert!(grown > 0.0 && grown < rows, "{grown}");
    app.combined_tab_mut_for_test().fold.as_mut().unwrap().clock = done;
    app.settle_animations();
    assert!((height(&app) - open).abs() < 1e-6);
    assert!(!app.combined_tab().unwrap().folding());
}

/// 回归：在某一段里放下光标后 Git 状态刷新、前面多了一个文件：按键仍然交给原来那个文件的段，
/// 不交给现在排在原位置上的另一个文件。
#[test]
fn the_current_section_follows_its_file_across_refreshes() {
    let Some((mut app, root)) = git_app("git-combined-current", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    let i = section_of(&app, "src/big_1.rs");
    app.set_active_scroll(app.combined_heads()[i]);
    paint(&mut app);
    let (area, _) = app.combined_section_area(i).unwrap();
    let parts = crate::app::diff_parts(area, view(&app, i), app.metrics.cell);
    let text = parts.panes[RIGHT].text;
    let at = (text.x + 8.0, text.y + app.fonts.line_height / 2.0);
    click(&mut app, at);
    assert_eq!(app.combined_tab().unwrap().current, Some(i));

    // 排在它前面的新文件。
    std::fs::write(root.join("src/big_00.rs"), "let a = 1;\n").unwrap();
    app.git_refresh();
    let current = app.combined_tab().unwrap().current.unwrap();
    assert_ne!(current, i, "新文件应排在前面");
    assert_eq!(current, section_of(&app, "src/big_1.rs"));
}

/// 回归：「未提交的更改」不是当前标签时 Git 状态刷新，它的滚动位置按它自己的长度限制，不按
/// 当前标签（一个很短的文件）的长度归零；前面多了一个文件时，视口顶边仍是原来那个文件。
#[test]
fn a_background_refresh_keeps_the_scroll_position() {
    let Some((mut app, root)) = git_app("git-combined-background", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    let i = section_of(&app, "src/big_2.rs");
    app.set_active_scroll(app.combined_heads()[i]);
    paint(&mut app);
    let before = scroll(&app);
    let combined = app.editor.active.unwrap();
    app.open_path(&root.join("src/new.rs"));
    // 改前面一个文件的内容，行数不变。
    let text: String = (0..40).map(|i| format!("let other_{i} = {i};\n")).collect();
    std::fs::write(root.join("src/big_0.rs"), text).unwrap();
    app.git_refresh();
    app.activate_tab(combined);
    assert!(
        (scroll(&app) - before).abs() < 1e-6,
        "{} != {before}",
        scroll(&app)
    );

    // 前面多了一个文件：仍停在 `big_2.rs` 的段头上。
    std::fs::write(root.join("a.txt"), "changed again\n").unwrap();
    app.git_refresh();
    let head = app.combined_heads()[section_of(&app, "src/big_2.rs")];
    assert!(
        (scroll(&app) - head).abs() < 1e-6,
        "{} != {head}",
        scroll(&app)
    );
}

/// 回归：开着合并差异视图时从段头「在差异标签中打开」的单独差异里再点标题栏的开关（先关再开）：
/// 换回原来的「未提交的更改」标签，不另起一个、把原来的留成空标签。
#[test]
fn turning_the_view_on_from_a_separate_diff_reuses_the_tab() {
    let Some((mut app, root)) = git_app("git-combined-reuse", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    app.settings_toggle(Setting::GitCombinedDiff);
    assert!(app.run_action(Action::GitShowAllChanges));
    let combined = app.combined_tab().unwrap().doc;
    let i = section_of(&app, "src/big_0.rs");
    app.press_combined(CombinedHit::OpenDiff(i));
    assert!(app.editor.active().unwrap().diff.is_some());
    let diff = app.editor.active().unwrap().id;
    let tabs = app.editor.documents.len();

    app.press_diff(DiffTarget::Combined);
    assert_eq!(app.config.git_combined_diff, Some(false));
    app.press_diff(DiffTarget::Combined);
    assert_eq!(app.config.git_combined_diff, Some(true));
    assert!(app.combined_active());
    assert_eq!(app.combined_tab().unwrap().doc, combined);
    assert_eq!(app.editor.documents.len(), tabs);
    let separate = app.editor.document(diff).unwrap();
    assert!(separate.diff.is_some(), "单独的差异标签留着");
    let _ = root;
}

/// 鼠标在正文上时交给段；在滚动条、命令面板这些盖在正文上的部件上时不交给段。
#[test]
fn only_the_bare_body_hands_the_pointer_to_a_section() {
    let Some((mut app, root)) = git_app("git-combined-pointer", true) else {
        return;
    };
    add_big_files(&root);
    app.git_refresh();
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let i = section_of(&app, "src/main.rs");
    let (area, _) = app.combined_section_area(i).unwrap();
    let body = app.combined_parts().body;
    let inside = (
        body.x + body.w * 0.6,
        area.y + crate::app::diff::DIFF_HEADER + 4.0,
    );
    assert!(app.combined_takes_pointer(inside.0, inside.1));
    assert!(
        !app.combined_takes_pointer(body.right() - 3.0, inside.1),
        "滚动条"
    );

    assert!(app.run_action(Action::ShowCommands));
    paint(&mut app);
    let picker = app.layout.picker;
    let over = (picker.x + picker.w / 2.0, picker.y + picker.h / 2.0);
    assert!(body.contains(over.0, over.1));
    assert!(!app.combined_takes_pointer(over.0, over.1), "命令面板");
}

/// 「折叠未更改的内容」是一项设置（`editor.diffCollapseUnchanged`，默认开）：关掉后差异打开时
/// 就全部展开；差异标题栏和「未提交的更改」工具栏上的开关改的就是这一项，打开着的差异一起跟着变。
#[test]
fn unchanged_regions_follow_the_collapse_setting() {
    use winc_editor::diff_view::SideRow;

    let Some((mut app, root)) = git_app("git-diff-collapse-setting", true) else {
        return;
    };
    let folded = |view: &winc_editor::diff_view::DiffView| {
        view.collapses()
            && view.rows[RIGHT]
                .iter()
                .any(|r| matches!(r, SideRow::Fold { .. }))
    };
    let tab_view = |app: &App| -> (bool, bool) {
        let view = app.editor.active().unwrap().diff.as_deref().unwrap();
        let rows = view.rows[RIGHT].iter();
        (
            view.collapses(),
            rows.clone().any(|r| matches!(r, SideRow::Fold { .. })),
        )
    };

    // 关掉：新打开的差异全部展开。
    app.settings_toggle(Setting::DiffCollapse);
    assert_eq!(app.config.diff_collapse_unchanged, Some(false));
    app.fetch_changes(&root.join("src/main.rs"), Target::Open);
    assert_eq!(tab_view(&app), (false, false));
    let diff_tab = app.editor.active().unwrap().id;

    // 标题栏上的开关：设置打开，这个差异折叠起来。
    app.press_diff(DiffTarget::Collapse);
    assert_eq!(app.config.diff_collapse_unchanged, Some(true));
    assert_eq!(tab_view(&app), (true, true));

    // 「未提交的更改」按设置折叠；工具栏上的开关关掉设置，另一个标签里的差异也一起展开。
    assert!(app.run_action(Action::GitShowAllChanges));
    paint(&mut app);
    let main = section_of(&app, "src/main.rs");
    assert!(folded(view(&app, main)));
    app.press_combined(CombinedHit::Collapse);
    assert_eq!(app.config.diff_collapse_unchanged, Some(false));
    assert!(!view(&app, main).collapses());
    let other = app
        .editor
        .document(diff_tab)
        .unwrap()
        .diff
        .as_deref()
        .unwrap();
    assert!(!other.collapses());
}
