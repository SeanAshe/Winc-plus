//! 文件树拖放的测试：拖动阈值、拖入目录移动文件、放回原处、悬停展开与边缘滚动，
//! 以及从窗口外拖入文件。

use std::time::{Duration, Instant};

use super::*;

/// 创建拖放测试用的目录并返回其路径。
///
/// 有 `src`、`lib` 两个目录，`lib` 中有子目录 `deep`（默认收起），以及若干文件。
fn drag_folder(name: &str) -> PathBuf {
    let dir = search_folder(name);
    std::fs::create_dir_all(dir.join("lib/deep")).unwrap();
    std::fs::write(dir.join("lib/deep/x.rs"), "").unwrap();
    std::fs::write(dir.join("notes.md"), "notes\n").unwrap();
    dir
}

/// 按住第 `from` 行（从 0 起），移到第 `to` 行上，不松开鼠标。
fn drag_row(app: &mut App, from: usize, to: usize) {
    let (x, y) = tree_row_center(app, from);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    let (_, to_y) = tree_row_center(app, to);
    app.on_mouse_move(x + 10.0, to_y);
}

fn path_of(app: &App, index: usize) -> PathBuf {
    app.editor.workspace.as_ref().unwrap().rows()[index]
        .path
        .clone()
}

#[test]
fn a_click_still_opens_and_a_small_wobble_is_still_a_click() {
    let dir = drag_folder("drag-click");
    let mut app = app_with(&dir, false);
    paint(&mut app);

    // 按下时只选中，松开才打开；中间移动 2 像素（小于拖动阈值）仍算单击。
    let notes = row_of(&app, "notes.md");
    let (x, y) = tree_row_center(&app, notes);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(notes));
    assert!(app.editor.active().is_none());
    assert!(app.tree_open_pending());
    app.on_mouse_move(x + 2.0, y + 2.0);
    assert!(app.tree_dragging().is_none());
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.active().unwrap().name(), "notes.md");
    assert_eq!(app.focus, Focus::Tree);
    assert!(!app.tree_open_pending());

    // 单击目录是切换其展开状态。按住目录时松开不会打开文件，不算待打开。
    let lib = row_of(&app, "lib");
    let at = tree_row_center(&app, lib);
    app.on_mouse_move(at.0, at.1);
    app.on_mouse_down(MouseButton::Left);
    assert!(!app.tree_open_pending());
    app.on_mouse_up(MouseButton::Left);
    assert!(app.editor.workspace.as_ref().unwrap().rows()[lib].expanded);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dragging_a_file_onto_a_folder_moves_it_and_its_tab_follows() {
    let dir = drag_folder("drag-move");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    app.open_path(&root.join("notes.md"));
    app.run(Command::Insert("edited ".into()));
    paint(&mut app);

    // 拖到 `lib` 上：`lib` 是放置目标，鼠标下的行不显示悬停高亮，鼠标旁跟随显示被拖动的名称。
    let notes = row_of(&app, "notes.md");
    let lib = row_of(&app, "lib");
    drag_row(&mut app, notes, lib);
    // 形成拖动后松开不会打开文件，不算待打开。
    assert!(!app.tree_open_pending());
    let drag = app.tree_dragging().unwrap();
    assert_eq!(drag.target.as_deref(), Some(root.join("lib").as_path()));
    assert_eq!(app.hover, Hover::None);
    paint(&mut app);

    // 拖到文件上等于放入该文件所在的目录；拖回被拖文件原本所在的目录没有意义，因此没有放置目标。
    let main = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().toggle(main);
    paint(&mut app);
    let config = row_of(&app, "config.rs");
    let (_, y) = tree_row_center(&app, config);
    app.on_mouse_move(60.0, y);
    let target = app.tree_dragging().unwrap().target.clone();
    assert_eq!(target.as_deref(), Some(root.join("src").as_path()));
    let cargo = row_of(&app, "Cargo.toml");
    let (_, y) = tree_row_center(&app, cargo);
    app.on_mouse_move(60.0, y);
    assert_eq!(app.tree_dragging().unwrap().target, None);

    // 放在 `lib` 上：文件移入，已打开标签的路径更新为新路径，未保存的修改仍然保留。
    let lib = row_of(&app, "lib");
    let (_, y) = tree_row_center(&app, lib);
    app.on_mouse_move(60.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert!(root.join("lib/notes.md").is_file());
    assert!(!root.join("notes.md").exists());
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.path.as_deref(),
        Some(root.join("lib/notes.md").as_path())
    );
    assert!(doc.is_dirty());
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.selected_entry().unwrap().path, root.join("lib/notes.md"));
    assert!(app.tree.drag.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn never_asking_again_is_remembered_in_the_config() {
    let dir = drag_folder("drag-never-ask");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    assert_eq!(app.config.confirm_drag_and_drop, None);

    // 在提示框中点击「移动」且没有勾选「不再询问」：文件移入，下次仍会询问。
    app.confirm_move(&[root.join("notes.md")], &root.join("lib"), false);
    assert!(root.join("lib/notes.md").is_file());
    assert_eq!(app.config.confirm_drag_and_drop, None);

    // 勾选后：写入配置，以后拖动直接移动。测试中没有配置文件路径，不会写入用户的配置。
    app.confirm_move(&[root.join("lib/notes.md")], &root, true);
    assert!(root.join("notes.md").is_file());
    assert_eq!(app.config.confirm_drag_and_drop, Some(false));
    assert!(app.config_path.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_cannot_go_into_itself_and_escape_cancels() {
    let dir = drag_folder("drag-refuse");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    let lib = row_of(&app, "lib");
    app.editor.workspace.as_mut().unwrap().toggle(lib);
    paint(&mut app);

    // 将 `lib` 拖到 `lib` 自己的子目录 `deep` 上：不能放置，不移动任何文件。
    let deep = row_of(&app, "deep");
    drag_row(&mut app, lib, deep);
    assert_eq!(app.tree_dragging().unwrap().target, None);
    app.on_mouse_up(MouseButton::Left);
    assert!(root.join("lib/deep/x.rs").is_file());

    // 拖到 `src` 上再按 Esc（按键处理中调用的就是 `cancel_tree_drag`）：取消拖动，
    // 松开鼠标也不移动。
    let src = row_of(&app, "src");
    drag_row(&mut app, lib, src);
    assert!(app.tree_dragging().unwrap().target.is_some());
    app.cancel_tree_drag();
    assert!(app.tree_dragging().is_none());
    app.on_mouse_up(MouseButton::Left);
    assert!(root.join("lib").is_dir());

    // 不覆盖同名文件：`src` 里已有 `main.rs`，把根目录下的同名文件拖进去会提示错误。
    std::fs::write(root.join("main.rs"), "top\n").unwrap();
    app.editor.workspace.as_mut().unwrap().refresh();
    paint(&mut app);
    let top = row_of(&app, "main.rs");
    let src = row_of(&app, "src");
    drag_row(&mut app, top, src);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(
        std::fs::read_to_string(root.join("main.rs")).unwrap(),
        "top\n"
    );
    assert!(
        app.notice
            .text
            .as_deref()
            .unwrap()
            .starts_with("无法移动「main.rs」")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resting_on_a_closed_folder_opens_it_and_the_edges_scroll() {
    let dir = drag_folder("drag-rest");
    for i in 0..40 {
        std::fs::write(dir.join(format!("zz_{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, false);
    app.editor.workspace.as_mut().unwrap().set_compact(false);
    paint(&mut app);

    // 停在收起的 `lib` 上，停留足够久后自动展开，可以继续放入 `deep`。
    let notes = row_of(&app, "notes.md");
    let lib = row_of(&app, "lib");
    drag_row(&mut app, notes, lib);
    let start = Instant::now();
    assert!(app.tree_drag_tick(start).is_some_and(|wake| wake > start));
    assert!(!app.editor.workspace.as_ref().unwrap().rows()[lib].expanded);
    app.tree_drag_tick(start + Duration::from_secs(1));
    assert!(app.editor.workspace.as_ref().unwrap().rows()[lib].expanded);
    assert_eq!(path_of(&app, lib + 1).file_name().unwrap(), "deep");

    // 鼠标贴着文件树下边缘：文件树自动向下滚动，放置目标随之重新计算。
    let tree = app.layout.tree;
    app.on_mouse_move(60.0, tree.bottom() - 2.0);
    assert_eq!(app.tree.scroll, 0.0);
    let wake = app.tree_drag_tick(Instant::now());
    assert!(app.tree.scroll > 0.0 && wake.is_some());
    // 离开边缘后停止滚动，也不再需要唤醒事件循环。
    app.on_mouse_move(60.0, tree.y + tree.h / 2.0);
    let scroll = app.tree.scroll;
    assert_eq!(app.tree_drag_tick(Instant::now()), None);
    assert_eq!(app.tree.scroll, scroll);

    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_dropped_on_the_tree_are_copied_and_elsewhere_opened() {
    let dir = sample_folder("drop-from-finder");
    let outside = dir.with_file_name("drop-from-finder-outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(outside.join("assets")).unwrap();
    std::fs::write(outside.join("notes.md"), "# notes").unwrap();
    std::fs::write(outside.join("assets/logo.svg"), "<svg/>").unwrap();
    let outside = winc_config::files::canonicalize(&outside).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);

    // 落在目录行上：复制进该目录，原文件还在，树展开到副本并选中它。
    let src = row_of(&app, "src");
    let (x, y) = tree_row_center(&app, src);
    app.drop_file_at(&outside.join("notes.md"), x, y);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    assert!(root.join("src/notes.md").is_file());
    assert!(outside.join("notes.md").is_file());
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src/notes.md"));
    assert_eq!(app.focus, Focus::Tree);

    // 落在文件树下方的空白处：目录连同内容复制进根目录。同名时不覆盖，副本改名。
    let blank = (app.layout.tree.x + 80.0, app.layout.tree.bottom() - 40.0);
    app.drop_file_at(&outside.join("assets"), blank.0, blank.1);
    assert!(root.join("assets/logo.svg").is_file());
    app.drop_file_at(&outside.join("assets"), blank.0, blank.1);
    assert!(root.join("assets 2/logo.svg").is_file());

    // 落在编辑区：打开，不复制。
    let editor = app.layout.editor;
    app.drop_file_at(
        &outside.join("notes.md"),
        editor.x + 200.0,
        editor.y + 200.0,
    );
    let active = app.editor.active().unwrap();
    assert_eq!(
        active.path.as_deref(),
        Some(outside.join("notes.md").as_path())
    );
    assert!(!root.join("notes.md").exists());

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn files_dragged_over_the_window_mark_where_they_will_land() {
    let dir = drag_folder("drop-hover");
    let mut app = app_with(&dir, false);
    paint(&mut app);
    assert_eq!(
        app.file_hover_tick(Instant::now()),
        None,
        "没有在拖时不用醒"
    );

    // 拖进来时在目录行上：标出这个目录，之后每帧查一次鼠标的位置。
    let lib = row_of(&app, "lib");
    let (x, y) = tree_row_center(&app, lib);
    app.on_mouse_move(x, y);
    app.hover_file();
    assert_eq!(app.file_hover(), Some(&DropZone::Tree(path_of(&app, lib))));
    shoot(&mut app, "drop-hover-tree");

    // 拖到编辑区：松手是打开，整个编辑区连同标签栏铺底色。
    let editor = app.layout.editor;
    app.on_mouse_move(editor.x + 200.0, editor.y + 200.0);
    assert!(app.file_hover_tick(Instant::now()).is_some());
    assert_eq!(app.file_hover(), Some(&DropZone::Open));
    shoot(&mut app, "drop-hover-editor");

    // 拖出窗口，或者松了手：不再标出。
    app.end_file_hover();
    assert_eq!(app.file_hover(), None);
    assert_eq!(app.file_hover_tick(Instant::now()), None);
    app.hover_file();
    app.drop_file(&dir.join("notes.md"));
    assert_eq!(app.file_hover(), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
