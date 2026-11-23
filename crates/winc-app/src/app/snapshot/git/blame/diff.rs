//! 差异中的逐行追溯：一个开关同时开关两侧，各自追溯自己的版本，追溯栏在行号与中间的连接带之间。

use super::*;
use crate::app::DiffTarget;
use crate::app::diff::DIFF_BLAME_W;
use crate::app::git::blame::BlameKey;
use crate::app::input::context_menu::MenuCommand;
use winc_editor::diff_view::{DiffSide, DiffView, LEFT, RIGHT};

/// 当前差异标签的几何。
fn parts(app: &App) -> crate::app::DiffParts {
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    crate::app::diff_parts(app.layout.editor, view, app.metrics.cell)
}

/// 当前差异第 `side` 侧第 `line` 行的提交说明。
fn summary(app: &App, side: usize, line: usize) -> Option<String> {
    let id = app.editor.active().unwrap().id;
    let blame = app.blame_of_key(BlameKey::Side(id, side))?;
    let commit = &blame.commits[*blame.lines.get(line)? as usize];
    Some(if commit.uncommitted {
        "未提交".to_owned()
    } else {
        commit.summary.clone()
    })
}

#[test]
fn each_side_of_a_diff_blames_its_own_version() {
    let Some((mut app, root)) = git_app("git-diff-blame", false) else {
        return;
    };
    blame_repo(&root);
    let file = root.join("src/server.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        text.replace("fn main() {", "fn main() -> Result<()> {"),
    )
    .unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);
    let before = parts(&app);
    assert_eq!(before.panes[LEFT].blame.w, 0.0);

    // 在行号栏上右键：菜单里是「逐行追溯」，打开以后两侧都多一栏，在行号与中间的连接带之间。
    let gutter = before.panes[LEFT].gutter;
    right_click(&mut app, (gutter.x + 4.0, gutter.y + 4.0));
    assert_eq!(menu_commands(&app), [(MenuCommand::ToggleBlame, true)]);
    assert!(!app.menu_checked(MenuCommand::ToggleBlame));
    app.run_menu_command(MenuCommand::ToggleBlame);
    paint(&mut app);
    let after = parts(&app);
    let (left, right) = (after.panes[LEFT], after.panes[RIGHT]);
    assert_eq!((left.blame.w, right.blame.w), (DIFF_BLAME_W, DIFF_BLAME_W));
    assert_eq!(left.blame.x, left.gutter.right());
    assert_eq!(after.ribbon.x, left.blame.right());
    assert_eq!(right.blame.x, after.ribbon.right());
    assert_eq!(right.gutter.x, right.blame.right());
    // 连接带的宽度不变，只是两侧的文字区窄了。
    assert_eq!(after.ribbon.w, before.ribbon.w);
    assert!(after.panes[LEFT].text.w < before.panes[LEFT].text.w);
    // 两侧的文字区一样宽（右侧取剩下的宽度，取整后最多差 1）。
    assert!((after.panes[LEFT].text.w - after.panes[RIGHT].text.w).abs() <= 1.0);
    // 左侧是 `HEAD`：改过的那一行仍是原来的提交；右侧改过的行未提交。
    assert_eq!(
        summary(&app, LEFT, 2).as_deref(),
        Some("服务器：监听 8080 端口")
    );
    assert_eq!(
        summary(&app, LEFT, 3).as_deref(),
        Some("服务器：对外网开放")
    );
    assert_eq!(summary(&app, RIGHT, 2).as_deref(), Some("未提交"));
    assert_eq!(
        summary(&app, RIGHT, 3).as_deref(),
        Some("服务器：对外网开放")
    );

    // 右键菜单打勾跟着开关。
    let gutter = parts(&app).panes[RIGHT].gutter;
    right_click(&mut app, (gutter.x + 4.0, gutter.y + 4.0));
    assert!(app.menu_checked(MenuCommand::ToggleBlame));
    app.overlays.context_menu = None;

    // 悬停在左侧一行上提示提交，点击在日志中选中它。
    let column = parts(&app).panes[LEFT].blame;
    let y = column.y + 3.5 * app.fonts.line_height;
    let hit = (app.hits.diff.iter()).find(|(r, _)| r.contains(column.x + 20.0, y));
    let target = hit.map(|&(_, t)| t);
    assert!(
        matches!(target, Some(DiffTarget::Blame(LEFT, _))),
        "{target:?}"
    );
    let Some(DiffTarget::Blame(side, line)) = target else {
        unreachable!()
    };
    let tip = app.diff_blame_tip(side, line).unwrap();
    assert!(tip.contains("服务器："), "{tip}");
    click(&mut app, (column.x + 20.0, y));
    assert!(app.log_active());

    // 回到差异，文字的右键菜单里同样有「逐行追溯」，点击两侧一起关掉。
    app.git_preview_changes(&file);
    paint(&mut app);
    let text = parts(&app).panes[LEFT].text;
    right_click(&mut app, (text.x + 40.0, text.y + 4.0));
    let commands = menu_commands(&app);
    assert!(
        commands.contains(&(MenuCommand::ToggleBlame, true)),
        "{commands:?}"
    );
    app.run_menu_command(MenuCommand::ToggleBlame);
    paint(&mut app);
    let off = parts(&app);
    assert_eq!(
        (off.panes[LEFT].blame.w, off.panes[RIGHT].blame.w),
        (0.0, 0.0)
    );
    assert_eq!(off.panes[LEFT].text, before.panes[LEFT].text);
    // 命令「Git：逐行追溯」再按一次：两侧一起打开。
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    let on = parts(&app);
    assert_eq!(
        (on.panes[LEFT].blame.w, on.panes[RIGHT].blame.w),
        (DIFF_BLAME_W, DIFF_BLAME_W)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 单栏：两侧共用行号之后的一栏，每行取所在一侧的结果；右侧没开时未改动的行取左侧的结果，
/// 按左侧的行号。行号栏上同样可以右键开关。
#[test]
fn single_column_blame_follows_each_rows_side() {
    let Some((mut app, root)) = git_app("git-diff-blame-unified", false) else {
        return;
    };
    blame_repo(&root);
    let file = root.join("src/server.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("// 新加的第一行\n{text}")).unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    let unified = |app: &mut App| {
        let view = app
            .editor
            .active_mut()
            .unwrap()
            .diff
            .as_deref_mut()
            .unwrap();
        view.set_unified(true);
    };
    unified(&mut app);
    paint(&mut app);
    let gutter = parts(&app).panes[LEFT].gutter;
    right_click(&mut app, (gutter.x + 4.0, gutter.y + 4.0));
    assert_eq!(menu_commands(&app), [(MenuCommand::ToggleBlame, true)]);
    app.run_menu_command(MenuCommand::ToggleBlame);
    paint(&mut app);
    let p = parts(&app);
    assert_eq!(p.panes[LEFT].blame, p.panes[RIGHT].blame);
    assert_eq!(p.panes[LEFT].blame.w, DIFF_BLAME_W);
    assert_eq!(p.panes[LEFT].blame.x, p.panes[RIGHT].gutter.right());
    // 每行的格子按所在的一侧：新加的行在右侧，未提交。
    let blamed = |app: &App| {
        let mut cells: Vec<_> = (app.hits.diff.iter())
            .filter_map(|&(rect, t)| match t {
                DiffTarget::Blame(side, line) => Some((rect.y, side, line)),
                _ => None,
            })
            .collect();
        cells.sort_by(|a, b| a.0.total_cmp(&b.0));
        cells
    };
    let first = blamed(&app)[0];
    assert_eq!((first.1, first.2), (RIGHT, 0));
    assert_eq!(summary(&app, RIGHT, 0).as_deref(), Some("未提交"));

    // 右侧没开：新加的行没有格子，下面未改动的行按左侧的行号取左侧的结果。
    let view = app
        .editor
        .active_mut()
        .unwrap()
        .diff
        .as_deref_mut()
        .unwrap();
    view.blamed = [true, false];
    paint(&mut app);
    let cells = blamed(&app);
    assert!(cells.iter().all(|&(_, side, _)| side == LEFT), "{cells:?}");
    assert_eq!(cells[0].2, 0);
    assert!(cells[0].0 > first.0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 不是 Git 中的版本的差异（比较两段文字）不能逐行追溯：说明原因，不多出一栏。
#[test]
fn a_diff_of_plain_texts_cannot_be_blamed() {
    let Some((mut app, root)) = git_app("git-diff-blame-plain", false) else {
        return;
    };
    let language = winc_syntax::detect("a.txt");
    let side = |title: &str, text: &str| DiffSide::new(title, text, None, language);
    let view = DiffView::new(side("a", "one\n"), side("b", "two\n"));
    app.editor.show_diff("texts", view);
    paint(&mut app);
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    assert_eq!(parts(&app).panes[RIGHT].blame.w, 0.0);
    let message = app.status.message.as_deref().or(app.notice.text.as_deref());
    assert_eq!(message, Some(winc_i18n::t!("git-blame-diff-unavailable")));
    let _ = std::fs::remove_dir_all(&root);
}

/// 从日志打开的历史差异：两侧各追溯父提交和这次提交；在文件栏里换文件，开着的追溯保留。
#[test]
fn a_history_diff_keeps_blame_across_its_files() {
    use crate::app::git::log::LogTarget;
    let Some((mut app, root)) = git_app("git-diff-blame-history", false) else {
        return;
    };
    blame_repo(&root);
    let text = std::fs::read_to_string(root.join("src/server.rs")).unwrap();
    std::fs::write(root.join("src/server.rs"), text.replace("8080", "9090")).unwrap();
    std::fs::write(root.join("notes.txt"), "端口改成 9090\n").unwrap();
    git_in(&root, &["add", "."]);
    git_in(&root, &["commit", "-q", "-m", "换端口"]);
    app.git_refresh();
    app.run_action(Action::GitShowLog);
    paint(&mut app);
    app.select_log_row(0);
    for _ in 0..5 {
        paint(&mut app);
        app.poll_log();
    }
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    let files = view.details.as_ref().unwrap().files.clone().unwrap();
    let server = files
        .iter()
        .position(|f| f.path.ends_with("server.rs"))
        .unwrap();
    app.press_log(LogTarget::File(server));
    app.poll_log();
    paint(&mut app);
    app.toggle_diff_blame();
    paint(&mut app);
    assert_eq!(
        summary(&app, LEFT, 3).as_deref(),
        Some("服务器：对外网开放")
    );
    assert_eq!(summary(&app, RIGHT, 3).as_deref(), Some("换端口"));

    // 换到另一个文件：两侧仍开着，追溯的是新文件。
    let notes = files
        .iter()
        .position(|f| f.path.ends_with("notes.txt"))
        .unwrap();
    app.press_diff(DiffTarget::File(notes));
    app.poll_log();
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.new.text.to_string().contains("9090"));
    // 新文件在父提交中不存在：左侧不能追溯，开关随之关掉；右侧仍开着。
    assert_eq!(view.blamed, [false, true]);
    assert_eq!(summary(&app, RIGHT, 0).as_deref(), Some("换端口"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：差异两侧都开着逐行追溯（`diff-blame`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_diff_blame() {
    let Some((mut app, root)) = git_app("git-diff-blame-shot", false) else {
        return;
    };
    blame_repo(&root);
    let file = root.join("src/server.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    let text = text.replace("        handle(stream);\n", "        handle(stream)?;\n");
    std::fs::write(&file, text).unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    app.toggle_diff_blame();
    paint(&mut app);
    let column = parts(&app).panes[LEFT].blame;
    app.on_mouse_move(column.x + 40.0, column.y + 2.5 * app.fonts.line_height);
    shoot(&mut app, "diff-blame");
    let _ = std::fs::remove_dir_all(&root);
}
