//! 文件树的测试：放不下的名称、横向滚动、新建条目的输入行、工具栏、固定在顶部的目录，
//! 紧凑目录，以及在终端中打开。

use super::*;

mod rows;

#[test]
fn tree_scrolls_sideways_and_drags() {
    let dir = sample_folder("hscroll");
    let mut app = app_with(&dir.join("a.txt"), true);
    app.panels.sidebar.width = 200.0;
    app.relayout();
    paint(&mut app);

    // 横向滚动条：仅当内容比文件树宽时才出现。
    let tree = app.layout.tree;
    assert!(
        app.panels.tree.content_w > tree.w,
        "{} {}",
        app.panels.tree.content_w,
        tree.w
    );
    let thumb = hscrollbar_thumb(tree, 0.0, app.panels.tree.content_w).unwrap();
    assert_eq!(
        (thumb.x, thumb.bottom()),
        (tree.x + SCROLLBAR_END, tree.bottom() - SCROLLBAR_INSET)
    );
    assert!(app.tree_max_scroll_x() > 0.0);

    // 按住滑块向右拖动，横向滚动位置随之变化。
    app.on_mouse_move(thumb.x + 4.0, thumb.y + 3.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(thumb.x + 60.0, thumb.y + 3.0);
    app.on_mouse_up(MouseButton::Left);
    let dragged = app.panels.tree.scroll_x;
    assert!(dragged > 0.0);

    // 触控板的横向滚动同样有效，且不会超出滚动范围。
    for _ in 0..40 {
        app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            -40.0, 0.0,
        )));
    }

    assert!(app.panels.tree.scroll_x <= app.tree_max_scroll_x());
    paint(&mut app);

    // 横向滚动到底时，滑块停在离文件树右边缘 `SCROLLBAR_END` 处。
    let last = hscrollbar_thumb(tree, app.tree_max_scroll_x(), app.panels.tree.content_w).unwrap();
    assert_eq!(last.right(), tree.right() - SCROLLBAR_END);

    // 行数少于文件树高度时没有竖直滑块，因此也无法拖动滑块。
    let rows = app.editor.workspace.as_ref().unwrap().rows().len();
    assert!((rows as f32 + 1.0) * TREE_ROW < tree.h);
    assert_eq!(
        scrollbar_thumb(tree, 0.0, (rows as f32 + 1.0) * TREE_ROW),
        None
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn big_tree_does_not_steal_clicks_from_the_editor() {
    // 文件多到文件树出现竖直滑块时，在编辑区点击或拖选，都不能被当作拖动文件树的滚动条。
    let dir = sample_folder("big-tree");
    let code: String = (0..50).map(|i| format!("line {i}\n")).collect();

    for i in 0..80 {
        std::fs::write(dir.join(format!("file{i:02}.txt")), &code).unwrap();
    }

    let mut app = app_with(&dir.join("file00.txt"), false);
    paint(&mut app);
    let tree = app.layout.tree;
    let rows = app.editor.workspace.as_ref().unwrap().rows().len();
    assert!(scrollbar_thumb(tree, 0.0, (rows as f32 + 1.0) * TREE_ROW).is_some());
    let scroll = app.panels.tree.scroll;

    // 在编辑区第 3 行按下、拖到第 6 行：选中文本，文件树不滚动。
    let text = app.layout.text;
    let line_height = app.fonts.line_height;
    let line_y = |line: f32| text.y + line * line_height + line_height / 2.0;
    app.on_mouse_move(text.x + 40.0, line_y(2.0));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(text.x + 40.0, line_y(5.0));
    app.on_mouse_up(MouseButton::Left);

    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.panels.tree.scroll, scroll);
    let doc = app.editor.active().unwrap();
    assert!(doc.selection.to() > doc.selection.from());

    // 状态栏同样如此：从状态栏按下并拖到文件树区域，文件树不滚动。
    app.on_mouse_move(text.x + 40.0, app.layout.status.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(text.x + 40.0, tree.y + 4.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.panels.tree.scroll, scroll);

    // 文件树自己的滚动条仍可正常拖动。
    let thumb = scrollbar_thumb(tree, scroll, (rows as f32 + 1.0) * TREE_ROW).unwrap();
    app.on_mouse_move(thumb.x + 3.0, thumb.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(thumb.x + 3.0, thumb.y + 200.0);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.panels.tree.scroll > scroll);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tree_tools_create_a_file_and_collapse_folders() {
    let dir = sample_folder("tools");
    let mut app = app_with(&dir.join("a.txt"), false);
    paint(&mut app);

    let button = |app: &App, tool: TreeTool| {
        app.hits
            .tree_tools
            .iter()
            .find(|(_, t)| *t == tool)
            .map(|(r, _)| *r)
            .unwrap()
    };

    // 新建文件：输入名称并回车，文件被创建并打开。
    let new_file = button(&app, TreeTool::NewFile);
    app.on_mouse_move(new_file.x + 4.0, new_file.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(matches!(
        app.panels.name_input.as_ref().map(|e| e.kind),
        Some(EntryKind::File)
    ));
    app.edit_field(|f| f.insert("deep/hello.rs"));
    app.submit_prompt();
    assert!(dir.join("deep/hello.rs").exists());
    assert_eq!(app.editor.active().unwrap().name(), "hello.rs");
    assert!(app.panels.name_input.is_none());

    // 新建目录：创建在选中文件所在的目录中（`hello.rs` 在 `deep` 里），焦点留在文件树。
    paint(&mut app);
    let new_folder = button(&app, TreeTool::NewFolder);
    app.on_mouse_move(new_folder.x + 4.0, new_folder.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.edit_field(|f| f.insert("docs"));
    app.submit_prompt();
    assert!(dir.join("deep/docs").is_dir());
    assert_eq!(app.focus, Focus::Tree);

    // 点击「刷新」后文件树仍然存在，没有报错。
    let refresh = button(&app, TreeTool::Refresh);
    app.on_mouse_move(refresh.x + 4.0, refresh.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.editor.workspace.is_some());

    // 全部折叠：只剩根目录下的行；选中的文件被收起时，改为选中包含它的最外层目录。
    let collapse = button(&app, TreeTool::CollapseAll);
    app.on_mouse_move(collapse.x + 4.0, collapse.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert!(ws.rows().iter().all(|e| e.depth == 0));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 创建嵌套目录测试用的目录并返回其路径。
///
/// `src` 与 `src/core` 两层目录各有 30 个文件，展开后比文件树高很多；
/// `src` 中 `core` 之前还有一个收起的目录 `api`。
pub(in crate::app::snapshot) fn nested_folder(name: &str) -> PathBuf {
    let dir = sample_folder(name);
    for i in 0..30 {
        std::fs::write(dir.join(format!("src/aa_{i:02}.rs")), "x").unwrap();
    }
    std::fs::create_dir_all(dir.join("src/api")).unwrap();
    std::fs::write(dir.join("src/api/a.rs"), "x").unwrap();
    std::fs::create_dir_all(dir.join("src/core")).unwrap();
    for i in 0..30 {
        std::fs::write(dir.join(format!("src/core/m_{i:02}.rs")), "x").unwrap();
    }
    dir
}

/// 展开 `src` 和 `src/core`，返回这两个目录在文件树中的行号（从 0 起）。
///
/// # Panics
///
/// 文件树中没有这两个目录时 panic，调用方须先用 [`nested_folder`] 建目录。
pub(in crate::app::snapshot) fn expand_nested(app: &mut App) -> (usize, usize) {
    let src = row_of(app, "src");
    app.set_dir_expanded(src, Some(true));
    let core = row_of(app, "core");
    app.set_dir_expanded(core, Some(true));
    paint(app);
    (src, core)
}

#[test]
fn folders_stick_to_the_top_while_their_files_scroll() {
    let dir = nested_folder("sticky");
    let mut app = app_with(&dir, false);
    let (src, core) = expand_nested(&mut app);
    let tree = app.layout.tree;

    // 没有滚动时没有固定的目录行。
    assert!(app.tree_sticky().is_empty());

    // 滚动到 `core` 内部：`src` 和 `core` 固定在顶部两行，点击被覆盖的位置命中的是这两行。
    app.panels.tree.scroll = (core + 5) as f32 * TREE_ROW + 3.0;
    paint(&mut app);
    let sticky = app.tree_sticky();
    let pinned: Vec<_> = sticky
        .iter()
        .map(|s| (s.index, s.level, s.rect.y))
        .collect();
    assert_eq!(
        pinned,
        [(src, 0, tree.y), (core, 1, tree.y + TREE_ROW)],
        "{pinned:?}"
    );
    assert_eq!(app.tree_row_at(tree.y + 5.0), Some(src));
    assert_eq!(app.tree_row_at(tree.y + TREE_ROW + 5.0), Some(core));
    let below = app.tree_row_at(tree.y + 2.0 * TREE_ROW + 5.0).unwrap();
    assert!(below > core + 5, "{below}");
    app.on_mouse_move(tree.x + 80.0, tree.y + TREE_ROW + 5.0);
    assert_eq!(app.pointer.hover, Hover::TreeRow(core));

    // `core` 的最后几行滚到顶部时，`core` 被向上推，部分被 `src` 遮挡。
    let last = core + 30;
    app.panels.tree.scroll = (last - 1) as f32 * TREE_ROW + 6.0;
    paint(&mut app);
    let sticky = app.tree_sticky();
    assert_eq!(sticky.len(), 2);
    assert_eq!(sticky[1].index, core);
    assert!(sticky[1].rect.y < tree.y + TREE_ROW, "{:?}", sticky[1].rect);
    assert_eq!(sticky[1].rect.bottom(), app.tree_row_top(last) + TREE_ROW);
    // 两行重叠的位置，点中的是绘制在上层的 `src`。
    assert_eq!(app.tree_row_at(tree.y + TREE_ROW - 2.0), Some(src));

    // 点击固定的 `core`：`core` 收起，真正的 `core` 行停在原先固定的第二行位置，
    // `src` 仍固定在顶部。
    app.panels.tree.scroll = (core + 5) as f32 * TREE_ROW + 3.0;
    paint(&mut app);
    click(&mut app, (tree.x + 80.0, tree.y + TREE_ROW + 5.0));
    paint(&mut app);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert!(!ws.rows()[core].expanded);
    assert_eq!(ws.selected, Some(core));
    assert_eq!(app.tree_row_top(core), tree.y + TREE_ROW);
    let pinned: Vec<_> = app.tree_sticky().iter().map(|s| s.index).collect();
    assert_eq!(pinned, [src]);

    // 再点击固定的 `src`：`src` 也收起，回到自己所在的那一行。
    click(&mut app, (tree.x + 80.0, tree.y + 5.0));
    paint(&mut app);
    assert!(!app.editor.workspace.as_ref().unwrap().rows()[src].expanded);
    assert!(app.tree_sticky().is_empty());
    assert!((tree.y..tree.bottom()).contains(&app.tree_row_top(src)));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn double_clicking_below_the_tree_creates_a_file_in_the_root() {
    let dir = nested_folder("blank-double-click");
    let mut app = app_with(&dir, false);
    let (src, core) = expand_nested(&mut app);
    let tree = app.layout.tree;

    // 选中 `core` 中的一个文件，再滚到底：最后一行下方还留有一行空白。
    app.editor.workspace.as_mut().unwrap().selected = Some(core + 3);
    app.panels.tree.scroll = 1e6;
    paint(&mut app);
    // 点在空白行的上半部分，避开底边横向滚动条的点击范围（`SCROLLBAR_HIT`）。
    let blank = (tree.x + 80.0, tree.bottom() - TREE_ROW + 4.0);
    assert!(app.tree_blank_at(blank.1));
    assert!(!app.tree_blank_at(tree.y + 5.0));

    // 单击空白处只是取消选中，不选中任何一项。
    click(&mut app, blank);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, None);
    assert!(app.panels.name_input.is_none());

    // 双击：在根目录插入一行输入名称的输入行，并滚动到该行。
    app.pointer.last_click = None;
    click(&mut app, blank);
    click(&mut app, blank);
    let input = app.panels.name_input.as_ref().unwrap();
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    assert_eq!((&input.dir, input.index, input.depth), (&root, 0, 0));
    assert_eq!(app.focus, Focus::Prompt);
    paint(&mut app);
    assert!(app.name_input_rect().is_some());

    app.panels
        .name_input
        .as_mut()
        .unwrap()
        .field
        .set_text("新文件.txt");
    app.submit_name_input();
    assert!(dir.join("新文件.txt").is_file());
    assert!(!dir.join("src/core/新文件.txt").exists());

    // 在行上双击时行为不变：目录先展开再收起，不新建文件。
    app.pointer.last_click = None;
    let row = (tree.x + 80.0, app.tree_row_top(src) + 5.0);
    app.panels.tree.scroll = 0.0;
    paint(&mut app);
    let row = (row.0, app.tree_row_top(src) + 5.0);
    click(&mut app, row);
    click(&mut app, row);
    assert!(app.panels.name_input.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_in_terminal_goes_to_the_folder_of_what_is_selected() {
    let dir = search_folder("terminal");
    let mut app = app_with(&dir, false);
    let root = app.editor.root().unwrap().to_path_buf();

    // 没有打开的文件：使用打开的目录。
    app.run_action(crate::keymap::Action::OpenInTerminal);
    assert_eq!(app.opened_terminal.as_deref(), Some(root.as_path()));

    // 已有打开的文件：使用该文件所在的目录。
    app.open_path(&root.join("src/main.rs"));
    app.run_action(crate::keymap::Action::OpenInTerminal);
    assert_eq!(
        app.opened_terminal.as_deref(),
        Some(root.join("src").as_path())
    );

    // 在文件树中右键一个目录：使用该目录本身。
    app.run_tree_command(MenuCommand::OpenInTerminal, Some(root.join("src")));
    assert_eq!(
        app.opened_terminal.as_deref(),
        Some(root.join("src").as_path())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_splitter_resets_on_double_click_and_snaps_the_sidebar_closed() {
    let dir = sample_folder("splitter");
    let mut app = app_with(&dir, false);
    app.panels.sidebar.width = 420.0;
    paint(&mut app);
    let y = 300.0;
    let grab = |app: &App| app.layout.splitter.x + 1.0;

    // 双击分隔条恢复默认宽度。
    let x = grab(&app);
    click(&mut app, (x, y));
    click(&mut app, (x, y));
    assert_eq!(app.panels.sidebar.width, DEFAULT_PANEL_WIDTH);
    paint(&mut app);

    // 向左拖动时宽度停在最小宽度；继续拖过最小宽度的一半后收起侧栏，焦点离开文件树，
    // 宽度恢复为拖动前的值。
    app.focus = Focus::Tree;
    let x = grab(&app);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - (DEFAULT_PANEL_WIDTH - 200.0), y);
    assert!(app.sidebar_open());
    assert_eq!(app.panels.sidebar.width, MIN_PANEL_WIDTH);
    app.on_mouse_move(x - (DEFAULT_PANEL_WIDTH - MIN_PANEL_WIDTH / 2.0) - 1.0, y);
    assert!(!app.sidebar_open());
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.panels.sidebar.width, DEFAULT_PANEL_WIDTH);

    // 不松开鼠标向右拖回时，侧栏重新打开，宽度随鼠标位置变化。
    app.on_mouse_move(x - 20.0, y);
    assert!(app.sidebar_open());
    assert_eq!(app.panels.sidebar.width, DEFAULT_PANEL_WIDTH - 20.0);
    app.on_mouse_up(MouseButton::Left);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_empty_folder_says_so() {
    let dir = std::env::temp_dir().join(format!("winc-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app_with(&dir, false);
    paint(&mut app);
    assert!(app.editor.workspace.as_ref().unwrap().rows().is_empty());
    shoot(&mut app, "tree-empty");

    // 双击空白处在根目录新建文件，并显示输入名称的一行。
    let tree = app.layout.tree;
    let at = (tree.x + tree.w / 2.0, tree.y + 100.0);
    click(&mut app, at);
    click(&mut app, at);
    assert!(app.panels.name_input.is_some());
    paint(&mut app);
    shoot(&mut app, "tree-empty-creating");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 打开含 `src/main/java/{a,b}` 的目录，展开 `src`，并绘制一帧。
fn compact_app(name: &str) -> (App, PathBuf, usize) {
    let dir = sample_folder(name);
    let root = dir.join("project");
    std::fs::create_dir_all(root.join("src/main/java/a")).unwrap();
    std::fs::create_dir_all(root.join("src/main/java/b")).unwrap();
    let mut app = app_with(&root, true);
    let root = app.editor.root().unwrap().to_path_buf();
    let row = row_of(&app, "src");
    let ws = app.editor.workspace.as_mut().unwrap();
    ws.toggle(row);
    ws.select(None);
    paint(&mut app);
    (app, root, row)
}

/// 返回第 `row` 行里合并显示的目录 `path` 的中点横坐标。
fn segment_x(app: &App, row: usize, path: &Path) -> f32 {
    app.hits
        .tree_segments
        .iter()
        .find(|(index, .., p)| *index == row && p == path)
        .map(|(_, left, right, _)| (left + right) / 2.0)
        .unwrap()
}

#[test]
fn the_compact_folders_setting_applies_at_once() {
    let (mut app, _, row) = compact_app("compact-setting");
    let label = |app: &App| {
        app.editor.workspace.as_ref().unwrap().rows()[row]
            .label()
            .into_owned()
    };
    assert_eq!(label(&app), "src/main/java");

    app.settings_toggle(Setting::CompactFolders);
    assert_eq!(label(&app), "src");
    paint(&mut app);
    assert!(app.hits.tree_segments.is_empty());

    app.settings_toggle(Setting::CompactFolders);
    assert_eq!(label(&app), "src/main/java");
}

#[test]
fn the_compact_setting_also_applies_to_the_folder_opened_at_startup() {
    let dir = sample_folder("compact-startup");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join("src/main/java/a")).unwrap();
    std::fs::create_dir_all(root.join("src/main/java/b")).unwrap();
    let mut editor = Editor::default();
    editor.open_folder(&root).unwrap();
    let config = Config {
        compact_folders: Some(false),
        ..Config::default()
    };
    let mut app = App::new(editor, None, Bench::from_env(), config);
    let ws = app.editor.workspace.as_mut().unwrap();
    ws.toggle(0);
    assert_eq!(ws.rows()[0].label(), "src");
    assert_eq!(ws.rows()[1].label(), "main");
}

/// 在文件树里按下方向键等命名键。
fn press_tree_key(app: &mut App, key: winit::keyboard::NamedKey) {
    app.on_key_tree(&winit::keyboard::Key::Named(key));
}

#[test]
fn option_clicking_a_folder_expands_and_collapses_it_all_the_way() {
    let dir = sample_folder("recursive-click");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join("a/b/c")).unwrap();
    std::fs::create_dir_all(root.join("a/d/e")).unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();
    let mut app = app_with(&root, true);
    paint(&mut app);
    let row = row_of(&app, "a");
    let click = |app: &mut App, modifiers| {
        let (x, y) = tree_row_center(app, row);
        app.pointer.modifiers = modifiers;
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
        paint(app);
    };
    let labels = |app: &App| -> Vec<String> {
        let ws = app.editor.workspace.as_ref().unwrap();
        ws.rows().iter().map(|e| e.label().into_owned()).collect()
    };
    let alt = winit::keyboard::ModifiersState::ALT;

    // 一次展开到底，单子目录照常合并。
    click(&mut app, alt);
    assert_eq!(labels(&app), ["a", "b/c", "d/e", "other"]);

    // 再按 `⌥` 单击全部收起，之后普通单击只展开一层。
    click(&mut app, alt);
    assert_eq!(labels(&app), ["a", "other"]);
    click(&mut app, winit::keyboard::ModifiersState::empty());
    assert_eq!(labels(&app), ["a", "b", "d", "other"]);
}

#[test]
fn expanding_a_folder_lengthens_its_label_with_a_transition() {
    let dir = sample_folder("label-expand");
    let root = dir.join("project");
    std::fs::create_dir_all(root.join("src/main/java/a")).unwrap();
    std::fs::create_dir_all(root.join("src/main/java/b")).unwrap();
    let mut app = app_with(&root, true);
    app.panels.tree.labels.animate_without_window = true;
    let root = app.editor.root().unwrap().to_path_buf();
    paint(&mut app);
    let row = row_of(&app, "src");
    assert!(!app.panels.tree.labels.moving());

    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let view = app.panels.tree.labels.peek(&root.join("src")).unwrap();
    assert!(app.panels.tree.labels.moving());
    assert_eq!(view.old, ["src"]);
    assert_eq!(view.common, 1);
}

#[test]
fn resizing_the_sidebar_and_scrolling_sideways_do_not_animate_labels() {
    let dir = sample_folder("label-resize");
    let root = dir.join("project");
    let names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let deep: PathBuf = names.iter().collect();
    std::fs::create_dir_all(root.join(&deep).join("x")).unwrap();
    std::fs::create_dir_all(root.join(&deep).join("y")).unwrap();
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(root.join(long), "").unwrap();
    let mut app = app_with(&root, true);
    app.panels.sidebar.width = 230.0;
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    app.panels.tree.labels.animate_without_window = true;
    paint(&mut app);
    assert!(!app.panels.tree.labels.moving());

    for width in [250.0, 300.0, 210.0] {
        app.panels.sidebar.width = width;
        paint(&mut app);
        assert!(!app.panels.tree.labels.moving(), "{width}");
    }
    app.panels.tree.scroll_x = 30.0;
    paint(&mut app);
    assert!(!app.panels.tree.labels.moving());
}

/// 在文件树中单击（`clicks` 为 2 时双击）名为 `name` 的文件，之后画一帧。
fn click_tree_file(app: &mut App, root: &Path, name: &str, clicks: usize) {
    let ws = app.editor.workspace.as_ref().unwrap();
    let row = ws
        .rows()
        .iter()
        .position(|e| e.path == root.join(name))
        .unwrap();
    let (x, y) = (
        app.layout.tree.x + 60.0,
        app.tree_row_top(row) + TREE_ROW / 2.0,
    );
    // 与上一次单击无关：快速连续的测试步骤不被当成双击。
    app.pointer.last_click = None;
    for _ in 0..clicks {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
    }
    paint(app);
}

#[test]
fn single_clicks_preview_files_in_one_tab() {
    let dir = sample_folder("preview-click");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.join(name), format!("{name}\n")).unwrap();
    }
    let mut app = app_with(&dir, false);
    app.config.preview_on_click = Some(true);
    paint(&mut app);
    // 文件树中是规范化后的路径（macOS 上临时目录经过符号链接）。
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let tabs = app.editor.documents.len();
    let active_name = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.path.as_ref().unwrap().file_name().unwrap().to_owned()
    };

    // 单击：在预览标签中打开，焦点留在文件树；单击下一个文件时就地替换，标签数不变。
    click_tree_file(&mut app, &dir, "a.txt", 1);
    assert_eq!(app.editor.documents.len(), tabs + 1);
    let preview = app.editor.active;
    assert_eq!(app.preview_index(), preview);
    assert_eq!(app.focus, Focus::Tree);
    click_tree_file(&mut app, &dir, "b.txt", 1);
    assert_eq!(app.editor.documents.len(), tabs + 1);
    assert_eq!(app.editor.active, preview);
    assert_eq!(active_name(&app), "b.txt");
    assert_eq!(app.preview_index(), preview);

    // 双击使其成为普通标签；之后单击的文件另开预览标签。
    click_tree_file(&mut app, &dir, "b.txt", 2);
    assert_eq!(app.preview_index(), None);
    click_tree_file(&mut app, &dir, "c.txt", 1);
    assert_eq!(app.editor.documents.len(), tabs + 2);
    assert_eq!(app.preview_index(), app.editor.active);

    // 编辑过的预览标签成为普通标签：下一次单击另开标签。
    app.focus = Focus::Editor;
    app.run(Command::Insert("x".into()));
    assert_eq!(app.preview_index(), None);
    click_tree_file(&mut app, &dir, "a.txt", 1);
    assert_eq!(app.editor.documents.len(), tabs + 3);

    // 以其他方式打开预览标签中的文件、双击其标签，都使其成为普通标签。
    app.open_path(&dir.join("a.txt"));
    assert_eq!(app.preview_index(), None);
    let a = app.editor.active().unwrap().id;
    app.remove_tabs(&[a]);
    click_tree_file(&mut app, &dir, "a.txt", 1);
    let index = app.preview_index().unwrap();
    let hit = app.hits.tabs.iter().find(|h| h.index == index).unwrap().tab;
    let (x, y) = (hit.x + hit.w / 2.0, hit.y + hit.h / 2.0);
    app.pointer.last_click = None;
    for _ in 0..2 {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
    }
    assert_eq!(app.preview_index(), None);

    // 关闭设置后，单击在普通标签中打开。
    app.config.preview_on_click = Some(false);
    let before = app.editor.documents.len();
    click_tree_file(&mut app, &dir, "c.txt", 1);
    click_tree_file(&mut app, &dir, "b.txt", 1);
    assert_eq!(app.editor.documents.len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：预览标签（名称倾斜）和设置中的开关。
#[test]
#[ignore]
fn shoot_preview_tab() {
    let dir = sample_folder("preview-shot");
    let mut app = app_with(&dir, false);
    app.config.preview_on_click = Some(true);
    paint(&mut app);
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    click_tree_file(&mut app, &dir, "Cargo.toml", 2);
    click_tree_file(&mut app, &dir, "short.rs", 1);
    shoot(&mut app, "preview-tab-light");
    app.open_settings();
    app.settings_show_page(crate::layout::SettingsPage::Editor);
    app.settings.scroll = app.settings_layout().max_scroll();
    shoot(&mut app, "preview-tab-settings-light");
    let _ = std::fs::remove_dir_all(&dir);
}
