//! 文件视图底部大纲的测试：展开、收起与调整高度，符号树与层级、光标所在符号的高亮、收起与
//! 展开符号、收起状态的记忆、点击与键盘跳转、编辑后的延迟刷新，以及没有符号时的说明。

use std::time::Instant;

use winit::event::MouseButton;
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::app::workspace::outline::{OUTLINE_DELAY, OUTLINE_HEADER};
use crate::app::{OutlineEmpty, OutlineTarget};
use crate::keymap::Action;
use crate::layout::SidebarView;

/// 一个类带两个方法，后面是两个顶层函数；方法的行号分别是 1、4，函数是 8、12。
const PYTHON: &str = "class Path:
    def resolve(self):
        pass

    def parent(self):
        pass


def load_config():
    pass


def main():
    pass
";

/// 打开写有 [`PYTHON`] 的 `tool.py`，按 `⌘7` 展开大纲。
fn outline_app(name: &str, dark: bool) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let file = dir.join("tool.py");
    std::fs::write(&file, PYTHON).unwrap();
    let mut app = app_with(&file, dark);
    app.run_action(Action::ShowOutline);
    paint(&mut app);
    (app, dir)
}

/// 各行的符号名，按列出的顺序。
fn row_names(app: &App) -> Vec<String> {
    let rows = app.outline_rows();
    rows.iter()
        .map(|&i| app.outline.items[i].name.clone())
        .collect()
}

/// 大纲在文件视图底部：平时只有标题行；`⌘7` 切到文件视图并展开，文件树让出这一段；点标题行
/// 收起；拖动标题行的上沿调整高度，文件树至少保留三行。
#[test]
fn the_outline_opens_below_the_file_tree_and_resizes() {
    let dir = sample_folder("outline-section");
    let file = dir.join("tool.py");
    std::fs::write(&file, PYTHON).unwrap();
    let mut app = app_with(&file, false);
    paint(&mut app);
    let full = app.layout.tree.h;
    assert_eq!(app.layout.outline.h, OUTLINE_HEADER);
    assert_eq!(app.layout.tree.bottom(), app.layout.outline.y);
    assert!(!app.outline_shown());

    app.run_action(Action::ShowOutline);
    paint(&mut app);
    assert_eq!(app.sidebar.view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Outline);
    assert_eq!(app.outline_list().h, 10.0 * TREE_ROW);
    assert_eq!(
        app.layout.tree.h + app.layout.outline.h,
        full + OUTLINE_HEADER
    );

    // 拖动上沿：往上拖 48，列表高两行；往下拖过头时停在最小高度（三行）。
    let divider = app.outline_divider();
    let (x, y) = (divider.x + 40.0, divider.y + divider.h / 2.0);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y - 48.0);
    assert_eq!(app.outline_list().h, 12.0 * TREE_ROW);
    app.on_mouse_move(x, y + 1000.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.outline_list().h, 3.0 * TREE_ROW);

    // 点标题行收起：只剩标题行，焦点回到文件树。
    paint(&mut app);
    let header = app
        .hits
        .outline
        .iter()
        .find(|(_, t)| *t == OutlineTarget::Header)
        .unwrap()
        .0;
    click(&mut app, (header.x + 40.0, header.y + header.h / 2.0));
    paint(&mut app);
    assert_eq!(app.layout.outline.h, OUTLINE_HEADER);
    assert_eq!(app.focus, Focus::Tree);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 设置里关掉大纲：文件视图底部不再有标题行，文件树占满；`⌘7` 只在状态栏说明，不切换焦点。
#[test]
fn the_outline_can_be_turned_off_in_settings() {
    let (mut app, dir) = outline_app("outline-off", false);
    let full = app.layout.tree.h + app.layout.outline.h;
    app.settings_toggle(crate::layout::Setting::Outline);
    assert_eq!(app.config.outline, Some(false));
    assert_eq!(app.focus, Focus::Tree);
    paint(&mut app);
    assert_eq!(app.layout.outline.h, 0.0);
    assert_eq!(app.layout.tree.h, full);
    assert!(app.hits.outline.is_empty());

    app.focus = Focus::Editor;
    app.run_action(Action::ShowOutline);
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.layout.outline.h, 0.0);

    app.settings_toggle(crate::layout::Setting::Outline);
    paint(&mut app);
    assert!(app.layout.outline.h > OUTLINE_HEADER);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 符号按层级排成树，方法在类下面；光标所在的符号高亮，移动光标后高亮跟着变。
#[test]
fn the_outline_lists_symbols_as_a_tree_and_follows_the_caret() {
    let (mut app, dir) = outline_app("outline-tree", false);
    assert_eq!(app.focus, Focus::Outline);
    let tree: Vec<(&str, usize)> = (app.outline.items.iter())
        .map(|s| (s.name.as_str(), s.depth))
        .collect();
    assert_eq!(
        tree,
        [
            ("Path", 0),
            ("resolve", 1),
            ("parent", 1),
            ("load_config", 0),
            ("main", 0)
        ]
    );
    assert_eq!(row_names(&app).len(), 5);

    // 光标在 `parent` 的方法体里：高亮 `parent`。
    app.go_to(5, 4, 0);
    paint(&mut app);
    let rows = app.outline_rows();
    assert_eq!(app.outline_highlight(&rows), Some(2));
    // 收起 `Path` 后，光标所在的方法被挡住，高亮落在 `Path` 上。
    app.press_outline(OutlineTarget::Arrow(0));
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);
    app.outline.selected = None;
    let rows = app.outline_rows();
    assert_eq!(app.outline_highlight(&rows), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点击一行跳转到该符号并选中名字，焦点交给编辑区；点击箭头只收起、展开；标题行右端的按钮
/// 全部折叠或全部展开。
#[test]
fn clicking_the_outline_jumps_and_folds() {
    let (mut app, dir) = outline_app("outline-click", false);
    let find = |app: &App, target| {
        app.hits
            .outline
            .iter()
            .find(|(_, t)| *t == target)
            .unwrap()
            .0
    };

    let row = find(&app, OutlineTarget::Row(3));
    click(&mut app, (row.x + row.w / 2.0, row.y + row.h / 2.0));
    paint(&mut app);
    assert_eq!(app.focus, Focus::Editor);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head).0, 8);
    let (from, to) = (doc.selection.anchor, doc.selection.head);
    assert_eq!(
        doc.text_range(from.min(to), from.max(to)).as_deref(),
        Some("load_config")
    );

    let arrow = find(&app, OutlineTarget::Arrow(0));
    click(&mut app, (arrow.x + arrow.w / 2.0, arrow.y + arrow.h / 2.0));
    paint(&mut app);
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);
    assert_eq!(app.focus, Focus::Outline);

    // 唯一有下级的符号已收起，按钮是「全部展开」。
    assert!(!app.outline_any_expanded());
    let fold = find(&app, OutlineTarget::FoldAll);
    click(&mut app, (fold.x + fold.w / 2.0, fold.y + fold.h / 2.0));
    paint(&mut app);
    assert_eq!(row_names(&app).len(), 5);
    app.press_outline(OutlineTarget::FoldAll);
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 收起状态按文件记录：关闭标签后重新打开仍保持收起；状态随目录的现场保存，重新打开目录后
/// 恢复；全部展开后现场中不再记录该文件。
#[test]
fn folded_symbols_are_remembered_across_tabs_and_sessions() {
    let (mut app, dir) = outline_app("outline-remember", false);
    let file = app.editor.active().unwrap().path.clone().unwrap();
    app.press_outline(OutlineTarget::Arrow(0));
    paint(&mut app);
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);

    app.run_action(Action::CloseTab);
    paint(&mut app);
    app.open_path(&file);
    paint(&mut app);
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);

    app.remember_session();
    let root = app.editor.root().unwrap().to_path_buf();
    let state = app.state.clone();
    let session = state.session_for(&root).unwrap();
    assert_eq!(session.outline, [(file.clone(), vec!["Path".to_owned()])]);

    // 模拟重新启动：新的界面从保存的现场读入收起状态。
    let mut again = blank_app(false);
    again.state = state;
    again.open_path(&dir);
    again.open_path(&file);
    again.run_action(Action::ShowOutline);
    paint(&mut again);
    assert_eq!(row_names(&again), ["Path", "load_config", "main"]);

    // 展开后，现场中不再记录该文件。
    again.press_outline(OutlineTarget::Arrow(0));
    again.remember_session();
    assert!(again.state.session_for(&root).unwrap().outline.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 后台读取的文件：读完之前不提取（此时内容为空），读完后列出符号。读完后文档的版本号不变，
/// 不能据此判定内容没有变化。
#[test]
fn a_file_read_in_the_background_lists_its_symbols_once_read() {
    let (mut app, dir) = outline_app("outline-loading", false);
    let notes = dir.join("notes.md");
    std::fs::write(&notes, "# Title\n\n## Usage\n").unwrap();
    let index = app.editor.open_tab(&notes);
    let job = app.editor.begin_load(index).unwrap();
    let id = job.id();
    let ticket = app.loads.tickets.issue(id);
    paint(&mut app);
    assert!(app.editor.active().unwrap().is_pending());
    assert_ne!(app.outline.empty, Some(OutlineEmpty::NoSymbols));

    app.loads.results.0.send((id, ticket, job.run())).unwrap();
    app.poll_loads();
    paint(&mut app);
    assert!(!app.editor.active().unwrap().is_pending());
    let names: Vec<&str> = app.outline.items.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Title", "Usage"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 收起、展开一个符号与文件树的目录相同：下级的行逐渐收回（露出），不会立即消失，下面的行
/// 随之移动，箭头同时转向。
#[test]
fn folding_a_symbol_animates_like_a_folder() {
    use crate::app::animation::{Clock, TREE_ANIMATION};
    use crate::app::workspace::outline::OutlineFold;

    let (mut app, dir) = outline_app("outline-fold", false);
    app.press_outline(OutlineTarget::Arrow(0));
    assert_eq!(row_names(&app), ["Path", "load_config", "main"]);
    // 收起到一半：`Path` 的两个方法仍在绘制的行里，只露出一半高度，箭头转到一半。
    let half = Clock::frozen(TREE_ANIMATION / 2, TREE_ANIMATION);
    app.outline.fold = Some(OutlineFold {
        symbol: 0,
        expanding: false,
        clock: half,
    });
    let layout = app.outline_layout();
    assert_eq!(layout.rows, [0, 1, 2, 3, 4]);
    assert_eq!(layout.block, Some((1, 3)));
    let reveal = app.outline.fold.unwrap().reveal();
    assert!(reveal > 0.0 && reveal < 1.0);
    assert_eq!(layout.hidden, (1.0 - reveal) * 2.0 * TREE_ROW);
    assert_eq!(layout.turning, Some((0, reveal)));
    assert!(app.animating());
    paint(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 焦点在大纲上时：上下键移动，左键先收起、再移到上级，右键展开，回车跳转，`Esc` 回到编辑区。
#[test]
fn the_keyboard_moves_folds_and_jumps_in_the_outline() {
    let (mut app, dir) = outline_app("outline-keys", false);
    let key = |app: &mut App, key: NamedKey| app.outline_key(&Key::Named(key));
    let highlight = |app: &App| app.outline_highlight(&app.outline_rows());

    // 光标在第一行：高亮 `Path`；向下两次到 `parent`，再向上回到 `resolve`。
    assert_eq!(highlight(&app), Some(0));
    key(&mut app, NamedKey::ArrowDown);
    key(&mut app, NamedKey::ArrowDown);
    key(&mut app, NamedKey::ArrowUp);
    assert_eq!(highlight(&app), Some(1));
    // 方法没有下级，左键移到上级 `Path`；再按左键收起它，右键展开。
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(highlight(&app), Some(0));
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(row_names(&app).len(), 3);
    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(row_names(&app).len(), 5);
    // 展开着时右键移到第一个下级；`End` 到最后一行，回车跳转。
    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(highlight(&app), Some(1));
    key(&mut app, NamedKey::End);
    key(&mut app, NamedKey::Enter);
    assert_eq!(app.focus, Focus::Editor);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head).0, 12);

    app.focus = Focus::Outline;
    key(&mut app, NamedKey::Escape);
    assert_eq!(app.focus, Focus::Editor);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 编辑后不立即重新提取：等 `OUTLINE_DELAY` 内没有新的修改才提取，连续输入时不逐键重算。
#[test]
fn edits_refresh_the_outline_after_a_pause() {
    let (mut app, dir) = outline_app("outline-edit", false);
    app.go_to(0, 0, 0);
    app.run(winc_editor::Command::Insert(
        "def first():\n    pass\n\n".into(),
    ));
    let now = Instant::now();
    let due = app.sync_outline(now);
    assert!(due.is_some_and(|due| due > now));
    assert_eq!(app.outline.items.len(), 5);
    assert_eq!(app.sync_outline(now + OUTLINE_DELAY), None);
    assert_eq!(app.outline.items.len(), 6);
    assert_eq!(app.outline.items[0].name, "first");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 没有打开的文件、文件里没有符号、标签不是文本时写明原因；Markdown 的标题按级别排成树，预览
/// 时同样列出。
#[test]
fn the_outline_explains_why_it_is_empty() {
    let dir = sample_folder("outline-empty");
    let mut app = app_with(&dir, false);
    app.run_action(Action::ShowOutline);
    paint(&mut app);
    assert_eq!(app.outline.empty, Some(OutlineEmpty::NoFile));

    let text = dir.join("a.txt");
    std::fs::write(&text, "plain words\n").unwrap();
    app.open_path(&text);
    paint(&mut app);
    assert_eq!(app.outline.empty, Some(OutlineEmpty::NoSymbols));

    let notes = dir.join("notes.md");
    std::fs::write(&notes, "# Title\n\n## Usage\n\ntext\n\n## Build\n").unwrap();
    app.open_path(&notes);
    paint(&mut app);
    assert_eq!(app.outline.empty, None);
    let tree: Vec<(&str, usize)> = (app.outline.items.iter())
        .map(|s| (s.name.as_str(), s.depth))
        .collect();
    assert_eq!(tree, [("Title", 0), ("Usage", 1), ("Build", 1)]);

    // 切到 Markdown 的预览，再切到别的文件、切回来：在预览状态下重新提取，标题同样列出。
    app.run_action(Action::TogglePreview);
    assert!(app.editor.active().unwrap().edits_text());
    assert!(!app.editor.active().unwrap().shows_text());
    app.open_path(&text);
    paint(&mut app);
    app.open_path(&notes);
    paint(&mut app);
    assert!(!app.editor.active().unwrap().shows_text());
    assert_eq!(app.outline.empty, None);
    assert_eq!(app.outline.items.len(), 3);

    // 二进制文件没有大纲。
    app.open_path(&dir.join("data.bin"));
    paint(&mut app);
    assert_eq!(app.outline.empty, Some(OutlineEmpty::NotText));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：大纲展开，光标在 `parent` 里、鼠标停在 `load_config` 上；大纲收起，鼠标停在标题行上；没有
/// 大纲时的说明。深浅两套配色。
#[test]
#[ignore]
fn shoot_outline() {
    for dark in [false, true] {
        let (mut app, dir) = outline_app("outline-shot", dark);
        app.go_to(5, 4, 0);
        paint(&mut app);
        let row = app
            .hits
            .outline
            .iter()
            .find(|(_, t)| *t == OutlineTarget::Row(3))
            .unwrap()
            .0;
        app.on_mouse_move(row.x + 40.0, row.y + row.h / 2.0);
        shoot(
            &mut app,
            if dark {
                "outline-dark"
            } else {
                "outline-light"
            },
        );
        // 收起后鼠标停在标题行上：悬停的底色与上面的分隔线、下面的卡片边缘都留有空隙。
        app.press_outline(OutlineTarget::Header);
        paint(&mut app);
        let header = app.outline_header();
        app.on_mouse_move(header.x + 60.0, header.y + header.h / 2.0);
        let name = if dark {
            "outline-closed-dark"
        } else {
            "outline-closed-light"
        };
        shoot(&mut app, name);
        // 二进制文件没有大纲：说明在列表里居中。
        app.press_outline(OutlineTarget::Header);
        app.open_path(&dir.join("data.bin"));
        let name = if dark {
            "outline-empty-dark"
        } else {
            "outline-empty-light"
        };
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
