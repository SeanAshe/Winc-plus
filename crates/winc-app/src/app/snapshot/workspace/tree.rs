//! 文件树的测试：放不下的名称、横向滚动、新建条目的输入行、工具栏、固定在顶部的目录，
//! 紧凑文件夹，以及在终端中打开。

use super::*;

#[test]
fn long_tree_names_unfold_under_the_mouse() {
    let dir = sample_folder("unfold");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.sidebar.width = 230.0;
    let long = row_of(&app, "img_WNoi");
    let short = row_of(&app, "short");

    // 名称放得下的行不展开。
    hover_tree_row(&mut app, short, 120.0);
    paint(&mut app);
    assert_eq!(app.tree.overflow, None);

    // 名称放不下的行，从侧栏右边缘起继续绘制到名称末尾。
    hover_tree_row(&mut app, long, 120.0);
    paint(&mut app);
    let (rect, index) = app.tree.overflow.unwrap();
    assert_eq!((rect.x, index), (app.layout.tree.right(), long));

    // 鼠标移到展开的那一段上仍算悬停在该行，点击则打开该文件。
    let y = rect.y + rect.h / 2.0;
    app.on_mouse_move(rect.right() - 4.0, y);
    paint(&mut app);
    assert_eq!(app.hover, Hover::TreeRow(long));
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(long));
    app.on_mouse_up(MouseButton::Left);
    assert!(app.editor.active().unwrap().name().starts_with("img_WNoi"));

    // 在展开的那一段上右键，弹出的仍是该行的菜单，而不是编辑区的菜单。
    right_click(&mut app, (rect.right() - 4.0, y));
    let path = app.editor.workspace.as_ref().unwrap().rows()[long]
        .path
        .clone();
    assert_eq!(
        app.context_menu.as_ref().unwrap().target,
        MenuTarget::Tree(Some(path))
    );

    // 关掉菜单，鼠标移出到编辑区后收起展开的部分。
    app.dismiss_popup();
    app.on_mouse_move(rect.right() + 4.0, y);
    paint(&mut app);
    assert!(!matches!(app.hover, Hover::TreeRow(_)), "{:?}", app.hover);
    assert_eq!(app.tree.overflow, None);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tree_scrolls_sideways_and_drags() {
    let dir = sample_folder("hscroll");
    let mut app = app_with(&dir.join("a.txt"), true);
    app.sidebar.width = 200.0;
    app.relayout();
    paint(&mut app);

    // 横向滚动条：仅当内容比文件树宽时才出现。
    let tree = app.layout.tree;
    assert!(
        app.tree.content_w > tree.w,
        "{} {}",
        app.tree.content_w,
        tree.w
    );
    let thumb = hscrollbar_thumb(tree, 0.0, app.tree.content_w).unwrap();
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
    let dragged = app.tree.scroll_x;
    assert!(dragged > 0.0);

    // 触控板的横向滚动同样有效，且不会超出滚动范围。
    for _ in 0..40 {
        app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            -40.0, 0.0,
        )));
    }

    assert!(app.tree.scroll_x <= app.tree_max_scroll_x());
    paint(&mut app);

    // 横向滚动到底时，滑块停在离文件树右边缘 `SCROLLBAR_END` 处。
    let last = hscrollbar_thumb(tree, app.tree_max_scroll_x(), app.tree.content_w).unwrap();
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
fn the_scrollbar_strip_belongs_to_the_scrollbar_not_the_row_under_it() {
    // 文件树的内容比树本身高，横向滚动条压在最后一行上。
    let dir = sample_folder("scrollbar-strip");
    for i in 0..40 {
        std::fs::write(dir.join(format!("zz_{i:02}.txt")), "x").unwrap();
    }
    let mut app = app_with(&dir.join("a.txt"), false);
    app.sidebar.width = MIN_PANEL_WIDTH;
    app.relayout();
    paint(&mut app);
    let tree = app.layout.tree;
    let thumb = hscrollbar_thumb(tree, 0.0, app.tree.content_w).unwrap();
    let x = thumb.x + thumb.w / 2.0;
    // 紧挨在滚动条那一条上面的行。
    let last_row = app
        .tree_row_at(tree.bottom() - SCROLLBAR_HIT - 1.0)
        .unwrap();

    // 从树底向上 `SCROLLBAR_HIT` 高的一条属于滚动条：其下的行不显示悬停高亮，长名称也不展开。
    app.on_mouse_move(x, tree.bottom() - SCROLLBAR_HIT);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::TreeHorizontal));
    assert_eq!(app.tree.overflow, None);
    assert!(app.scrollbar_active(Scrollbar::TreeHorizontal));
    app.on_mouse_move(x, tree.bottom() - SCROLLBAR_HIT - 1.0);
    assert_eq!(app.hover, Hover::TreeRow(last_row));
    assert!(!app.scrollbar_active(Scrollbar::TreeHorizontal));

    // 点击滑块右侧的轨道：滑块立即跳到点击处，无需移动鼠标。
    let track_x = thumb.right() + 30.0;
    app.on_mouse_move(track_x, thumb.y + 3.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.tree.scroll_x > 0.0);

    // 拖动滑块时鼠标划过文件树的行，悬停仍保持在滚动条上；松开后才轮到下方的行，
    // 且没有打开其他文件。
    app.on_mouse_move(track_x, tree.y + 10.0);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::TreeHorizontal));
    app.on_mouse_up(MouseButton::Left);
    assert!(matches!(app.hover, Hover::TreeRow(_)));
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");

    // 编辑区右侧的滚动条同样如此：该区域属于滚动条，按下时不会把光标放进文本里。
    let code: String = (0..300).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.rs"), code).unwrap();
    app.open_path(&dir.join("long.rs"));
    paint(&mut app);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.right() - 4.0, editor.bottom() - 40.0);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::Editor));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    assert_eq!(app.editor.active().unwrap().selection.head, 0);

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
    let scroll = app.tree.scroll;

    // 在编辑区第 3 行按下、拖到第 6 行：选中文本，文件树不滚动。
    let text = app.layout.text;
    let line_height = app.fonts.line_height;
    let line_y = |line: f32| text.y + line * line_height + line_height / 2.0;
    app.on_mouse_move(text.x + 40.0, line_y(2.0));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(text.x + 40.0, line_y(5.0));
    app.on_mouse_up(MouseButton::Left);

    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.tree.scroll, scroll);
    let doc = app.editor.active().unwrap();
    assert!(doc.selection.to() > doc.selection.from());

    // 状态栏同样如此：从状态栏按下并拖到文件树区域，文件树不滚动。
    app.on_mouse_move(text.x + 40.0, app.layout.status.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(text.x + 40.0, tree.y + 4.0);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.tree.scroll, scroll);

    // 文件树自己的滚动条仍可正常拖动。
    let thumb = scrollbar_thumb(tree, scroll, (rows as f32 + 1.0) * TREE_ROW).unwrap();
    app.on_mouse_move(thumb.x + 3.0, thumb.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(thumb.x + 3.0, thumb.y + 200.0);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.tree.scroll > scroll);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn new_entry_row_is_drawn_where_it_is_clicked() {
    let dir = sample_folder("new-row");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.run_tree_tool(TreeTool::NewFile);

    // 输入行插入在文件树中，位于第一个条目的位置，而不是浮在上方的对话框。
    let row = app.name_input_rect().unwrap();
    assert!(app.layout.tree.contains(row.x, row.y));
    assert_eq!(row.y, app.layout.tree.y);
    assert!(app.goto_input.is_none());

    // 绘制位置与点击判断使用同一个矩形：点击该行时，焦点留在输入框中。
    paint(&mut app);
    app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.focus, Focus::Prompt);

    // 点击其他位置：尚未输入名称，输入行收起，没有创建任何文件。
    app.on_mouse_move(900.0, 500.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.name_input.is_none());
    assert!(!dir.join("名字").exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn new_entry_row_follows_the_selection_and_reports_errors() {
    let dir = sample_folder("new-row-dir");
    let mut app = app_with(&dir.join("a.txt"), false);
    paint(&mut app);

    // 选中目录 `src`：输入行插在 `src` 的子树第一行，缩进比 `src` 深一级。
    let src = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().selected = Some(src);
    app.run_tree_tool(TreeTool::NewFile);
    let entry = app.name_input.as_ref().unwrap();
    assert_eq!((entry.index, entry.depth), (src + 1, 1));
    assert_eq!(
        app.name_input_rect().unwrap().y,
        app.layout.tree.y + (src as f32 + 1.0) * TREE_ROW
    );

    // 名称输入框的起点与普通行的名称相同：缩进 + 箭头 + 图标 + 间隔，并随横向滚动偏移。
    let name_x = |app: &App, depth: usize| {
        let tree = app.layout.tree;
        tree.x + 48.0 + depth as f32 * TREE_INDENT - app.tree.scroll_x
    };
    paint(&mut app);
    assert_eq!(
        app.name_input.as_ref().unwrap().field.view.area.x,
        name_x(&app, 1)
    );
    app.tree.scroll_x = 12.0;
    paint(&mut app);
    assert_eq!(
        app.name_input.as_ref().unwrap().field.view.area.x,
        name_x(&app, 1)
    );
    app.tree.scroll_x = 0.0;

    // 收起的目录：先展开，输入行落在子行的第一行；展开走动画流程（离屏测试中直接到位）。
    app.close_name_input();
    let target = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().selected = Some(target);
    app.editor.workspace.as_mut().unwrap().toggle(target);
    assert!(!app.editor.workspace.as_ref().unwrap().rows()[target].expanded);
    app.run_tree_tool(TreeTool::NewFolder);
    assert!(app.editor.workspace.as_ref().unwrap().rows()[target].expanded);
    assert_eq!(app.name_input.as_ref().unwrap().index, target + 1);

    // 名称已存在：输入行保留，原因显示在右侧，磁盘上不做修改。
    app.edit_field(|f| f.insert("main.rs"));
    app.submit_name_input();
    assert_eq!(
        app.name_input.as_ref().unwrap().error.as_deref(),
        Some("已存在同名项")
    );
    assert_eq!(app.focus, Focus::Prompt);

    // 换用新名称再提交，即可创建成功。
    app.edit_field(|f| {
        f.select_all();
        f.insert("helper.rs")
    });
    app.submit_name_input();
    assert!(dir.join("src/helper.rs").exists());
    assert!(app.name_input.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn narrow_sidebar_folds_tools_before_the_name() {
    let dir = sample_folder("a-rather-long-project-name");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.sidebar.width = 400.0;
    paint(&mut app);
    assert_eq!(app.hits.tree_tools.len(), 5);
    let full = app.hits.project.w;

    // 从宽到窄一点点收：工具栏还留着按钮时项目名完整显示，按钮一个个收进「⋯」；
    // 只剩「⋯」以后项目名才变短，最后停在最小宽度。
    let mut width = app.sidebar.width;
    let mut shortened = false;
    while width >= MIN_PANEL_WIDTH {
        app.sidebar.width = width;
        paint(&mut app);
        if app.hits.tree_tools.is_empty() {
            assert!(app.hits.tree_more.is_some());
            shortened |= app.hits.project.w < full;
        } else {
            assert_eq!(app.hits.project.w, full, "宽 {width} 时项目名被截短了");
        }
        assert!(app.hits.project.w >= TREE_NAME_MIN);
        if let Some(more) = app.hits.tree_more {
            assert!(more.x >= app.hits.project.right());
        }
        width -= 2.0;
    }
    assert!(shortened);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn narrow_sidebar_folds_tools_into_more() {
    let dir = sample_folder("more");
    let mut app = app_with(&dir.join("a.txt"), false);

    // 侧栏足够宽时，五个工具按钮都显示，没有「⋯」。
    paint(&mut app);
    assert_eq!(app.hits.tree_tools.len(), 5);
    assert!(app.hits.tree_more.is_none());

    // 缩到最窄：项目名仍然保留且可点击（只检查悬停，不实际点击，因为点击会弹出选择目录的
    // 系统对话框）。
    app.sidebar.width = MIN_PANEL_WIDTH;
    paint(&mut app);
    let more = app.hits.tree_more.unwrap();
    assert!(app.hits.tree_tools.len() < 5);
    let hit = app.hits.project;
    assert!(hit.w >= TREE_NAME_MIN);
    assert!(app.hits.tree_tools.iter().all(|(r, _)| r.x >= hit.right()));
    app.on_mouse_move(hit.x + hit.w / 2.0, hit.y + hit.h / 2.0);
    assert_eq!(app.hover, Hover::ProjectName);

    // 点击「⋯」弹出系统菜单，列出被收起的按钮，顺序与工具栏中的顺序相同；
    // 悬停提示为「更多操作」。
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};
    app.on_mouse_move(more.x + 4.0, more.y + 4.0);
    assert_eq!(app.hover, Hover::TreeMore);
    app.on_mouse_down(MouseButton::Left);
    let menu = app.context_menu.clone().unwrap();
    assert_eq!(menu.target, MenuTarget::TreeTools);
    let kept = app.hits.tree_tools.len();
    assert_eq!(menu.entries.len(), TreeTool::ALL.len() - kept);
    assert_eq!(
        menu.entries.last(),
        Some(&MenuEntry::Item(MenuCommand::TreeLocate, true))
    );

    // 选择「折叠所有目录」的效果与点击工具栏上的对应按钮相同。
    let src = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().toggle(src);
    app.run_menu_command(MenuCommand::TreeCollapseAll);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert!(ws.rows().iter().all(|e| !e.expanded));

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
        app.name_input.as_ref().map(|e| e.kind),
        Some(EntryKind::File)
    ));
    app.edit_field(|f| f.insert("deep/hello.rs"));
    app.submit_prompt();
    assert!(dir.join("deep/hello.rs").exists());
    assert_eq!(app.editor.active().unwrap().name(), "hello.rs");
    assert!(app.name_input.is_none());

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

#[test]
fn clicks_during_a_folder_animation_land_on_the_row_you_see() {
    let dir = search_folder("tree-motion-hit");
    let mut app = app_with(&dir, false);
    let src = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().toggle(src);
    paint(&mut app);
    let removed: Vec<_> = app.editor.workspace.as_ref().unwrap().rows()[src + 1..src + 3].to_vec();

    // 收起 `src` 的动画刚开始：两个子行仍画在原位，`src` 后面的那一项（数据中是第 `src + 1` 行）
    // 画在这两个子行下方。
    app.editor.workspace.as_mut().unwrap().toggle(src);
    app.tree.motion = Some(crate::app::TreeMotion {
        parent: src,
        count: 2,
        expanding: false,
        removed,
        clock: Clock::new(TREE_ANIMATION),
    });
    let slot_y = |v: usize| app.layout.tree.y + v as f32 * TREE_ROW + TREE_ROW / 2.0;
    assert_eq!(app.tree_row_at(slot_y(src + 3)), Some(src + 1));
    // 正在收起的子行已经不在文件树数据中，不会被点中。
    assert_eq!(app.tree_row_at(slot_y(src + 1)), None);
    assert_eq!(app.tree_row_at(slot_y(src)), Some(src));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_unfolded_name_lines_up_with_its_row_below_the_new_entry_row() {
    let dir = sample_folder("unfold-below-input");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.sidebar.width = 230.0;
    app.editor.workspace.as_mut().unwrap().selected = None;
    app.start_new_entry(false);
    paint(&mut app);
    let long = row_of(&app, "img_WNoi");

    // 输入行插在最上面，长名称所在的行下移一行；展开的部分画在该行自己的位置上。
    let tree = app.layout.tree;
    let y = tree.y + (long + 1) as f32 * TREE_ROW + TREE_ROW / 2.0;
    app.on_mouse_move(tree.x + 120.0, y);
    paint(&mut app);
    assert_eq!(app.hover, Hover::TreeRow(long));
    let (rect, index) = app.tree.overflow.unwrap();
    assert_eq!(index, long);
    assert_eq!(rect.y, tree.y + (long + 1) as f32 * TREE_ROW + 1.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn another_input_taking_focus_finishes_the_name_being_typed() {
    let dir = search_folder("name-input-focus");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    app.open_path(&root.join("src/main.rs"));

    // 输入名称后再按 ⌃G：名称先提交，跳转条使用自己的输入框，输入不会进入名称。
    app.editor.workspace.as_mut().unwrap().selected = None;
    app.start_new_entry(false);
    app.edit_field(|f| f.insert("notes.md"));
    app.open_goto();
    assert!(root.join("notes.md").is_file());
    assert!(app.name_input.is_none());
    app.edit_field(|f| f.insert("2"));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "2");
    app.close_prompt();

    // 没有输入任何内容就回车：输入行收起，焦点回到文件树，而不是停在已经消失的输入框上。
    app.start_new_entry(false);
    app.submit_name_input();
    assert!(app.name_input.is_none());
    assert_eq!(app.focus, Focus::Tree);

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
    app.tree.scroll = (core + 5) as f32 * TREE_ROW + 3.0;
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
    assert_eq!(app.hover, Hover::TreeRow(core));

    // `core` 的最后几行滚到顶部时，`core` 被向上推，部分被 `src` 遮挡。
    let last = core + 30;
    app.tree.scroll = (last - 1) as f32 * TREE_ROW + 6.0;
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
    app.tree.scroll = (core + 5) as f32 * TREE_ROW + 3.0;
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
fn rows_scrolled_into_view_are_not_hidden_under_sticky_folders() {
    let dir = nested_folder("sticky-reveal");
    let mut app = app_with(&dir, false);
    let (_, core) = expand_nested(&mut app);
    let tree = app.layout.tree;

    // 向上选择一个已滚出视野的文件：滚动到固定的两层目录下方，保证可见。
    app.tree.scroll = (core + 20) as f32 * TREE_ROW;
    paint(&mut app);
    let target = core + 10;
    app.scroll_tree_to(target, false);
    paint(&mut app);
    let header = app
        .tree_sticky()
        .iter()
        .map(|s| s.rect.bottom())
        .fold(tree.y, f32::max);
    assert_eq!(app.tree_row_top(target), header);
    assert_eq!(app.tree_row_at(header + 5.0), Some(target));

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
    app.tree.scroll = 1e6;
    paint(&mut app);
    // 点在空白行的上半部分，避开底边横向滚动条的点击范围（`SCROLLBAR_HIT`）。
    let blank = (tree.x + 80.0, tree.bottom() - TREE_ROW + 4.0);
    assert!(app.tree_blank_at(blank.1));
    assert!(!app.tree_blank_at(tree.y + 5.0));

    // 单击空白处只是取消选中，不选中任何一项。
    click(&mut app, blank);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, None);
    assert!(app.name_input.is_none());

    // 双击：在根目录插入一行输入名称的输入行，并滚动到该行。
    app.last_click = None;
    click(&mut app, blank);
    click(&mut app, blank);
    let input = app.name_input.as_ref().unwrap();
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    assert_eq!((&input.dir, input.index, input.depth), (&root, 0, 0));
    assert_eq!(app.focus, Focus::Prompt);
    paint(&mut app);
    assert!(app.name_input_rect().is_some());

    app.name_input
        .as_mut()
        .unwrap()
        .field
        .set_text("新文件.txt");
    app.submit_name_input();
    assert!(dir.join("新文件.txt").is_file());
    assert!(!dir.join("src/core/新文件.txt").exists());

    // 在行上双击时行为不变：目录先展开再收起，不新建文件。
    app.last_click = None;
    let row = (tree.x + 80.0, app.tree_row_top(src) + 5.0);
    app.tree.scroll = 0.0;
    paint(&mut app);
    let row = (row.0, app.tree_row_top(src) + 5.0);
    click(&mut app, row);
    click(&mut app, row);
    assert!(app.name_input.is_none());

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
    app.sidebar.width = 420.0;
    paint(&mut app);
    let y = 300.0;
    let grab = |app: &App| app.layout.splitter.x + 1.0;

    // 双击分隔条恢复默认宽度。
    let x = grab(&app);
    click(&mut app, (x, y));
    click(&mut app, (x, y));
    assert_eq!(app.sidebar.width, DEFAULT_PANEL_WIDTH);
    paint(&mut app);

    // 向左拖动时宽度停在最小宽度；继续拖过最小宽度的一半后收起侧栏，焦点离开文件树，
    // 宽度恢复为拖动前的值。
    app.focus = Focus::Tree;
    let x = grab(&app);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - (DEFAULT_PANEL_WIDTH - 200.0), y);
    assert!(app.sidebar_open());
    assert_eq!(app.sidebar.width, MIN_PANEL_WIDTH);
    app.on_mouse_move(x - (DEFAULT_PANEL_WIDTH - MIN_PANEL_WIDTH / 2.0) - 1.0, y);
    assert!(!app.sidebar_open());
    assert_eq!(app.focus, Focus::Editor);
    assert_eq!(app.sidebar.width, DEFAULT_PANEL_WIDTH);

    // 不松开鼠标向右拖回时，侧栏重新打开，宽度随鼠标位置变化。
    app.on_mouse_move(x - 20.0, y);
    assert!(app.sidebar_open());
    assert_eq!(app.sidebar.width, DEFAULT_PANEL_WIDTH - 20.0);
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
    assert!(app.name_input.is_some());
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
fn clicking_a_merged_folder_selects_that_level() {
    let (mut app, root, row) = compact_app("compact-click");
    let main = root.join("src/main");
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    let x = segment_x(&app, row, &main);
    let count = app.editor.workspace.as_ref().unwrap().rows().len();

    // 单击选中点中的那一级，同时收起这一行；新建的内容放进这一级。
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.segment(), Some(main.as_path()));
    assert_eq!(ws.target_dir(), main);
    assert!(ws.rows().len() < count);
    assert_eq!(app.shortcut_target(), Some(main.clone()));
    paint(&mut app);

    // 点在图标上选中这一行本身。
    app.on_mouse_move(app.layout.tree.x + 12.0, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.segment(), None);
    assert_eq!(ws.target_dir(), root.join("src/main/java"));
}

#[test]
fn renaming_a_merged_folder_changes_only_that_level() {
    let (mut app, root, row) = compact_app("compact-rename");
    let main = root.join("src/main");

    app.start_rename(&main);
    let entry = app.name_input.as_mut().unwrap();
    assert_eq!(entry.field.text(), "main");
    entry.field.set_text("core");
    app.submit_name_input();
    assert!(root.join("src/core/java/a").is_dir());
    assert_eq!(
        app.editor.workspace.as_ref().unwrap().rows()[row].label(),
        "src/core/java"
    );
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
fn clicking_a_merged_folder_past_the_sidebar_selects_it() {
    let dir = sample_folder("compact-overflow");
    let root = dir.join("project");
    let names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let deep: PathBuf = names.iter().collect();
    std::fs::create_dir_all(root.join(&deep).join("x")).unwrap();
    std::fs::create_dir_all(root.join(&deep).join("y")).unwrap();
    let mut app = app_with(&root, true);
    app.sidebar.width = 230.0;
    let root = app.editor.root().unwrap().to_path_buf();
    let row = row_of(&app, "alpha");
    let ws = app.editor.workspace.as_mut().unwrap();
    ws.toggle(row);
    ws.select(None);
    paint(&mut app);

    // 鼠标移到行上，放不下的部分继续绘制在编辑区上方；再移到那一段上的某一级目录。
    let golf = root.join("alpha/bravo/charlie/delta/echo/foxtrot/golf");
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.right() - 4.0, y);
    paint(&mut app);
    let (rect, _) = app.tree.overflow.unwrap();
    let x = segment_x(&app, row, &golf);
    assert!(x > rect.x && x < rect.right(), "{x} {rect:?}");
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(app.hover, Hover::TreeRow(row));

    app.on_mouse_down(MouseButton::Left);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.segment(), Some(golf.as_path()));
    app.on_mouse_up(MouseButton::Left);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.segment(), Some(golf.as_path()));
}

#[test]
fn dragging_onto_a_merged_row_drops_into_the_folder_under_the_pointer() {
    let (mut app, root, _) = compact_app("compact-drop");
    std::fs::write(root.join("note.txt"), "").unwrap();
    app.editor.workspace.as_mut().unwrap().refresh();
    paint(&mut app);
    let row = row_of(&app, "java");
    let main = root.join("src/main");
    let java = root.join("src/main/java");
    let file = row_of(&app, "note.txt");
    let (x, y) = tree_row_center(&app, file);
    let top = app.tree_row_top(row) + TREE_ROW / 2.0;

    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    // 鼠标在中间的一级上：放进那一级；在最后一级或名称以外：放进这一行本身。
    app.on_mouse_move(segment_x(&app, row, &main), top);
    assert_eq!(app.tree_dragging().unwrap().target, Some(main.clone()));
    paint(&mut app);
    app.on_mouse_move(segment_x(&app, row, &java), top);
    assert_eq!(app.tree_dragging().unwrap().target, Some(java.clone()));
    app.on_mouse_move(app.layout.tree.right() - 8.0, top);
    assert_eq!(app.tree_dragging().unwrap().target, Some(java));

    // 移到中间的一级松开，确认后文件进入那一级。
    app.on_mouse_move(segment_x(&app, row, &main), top);
    app.on_mouse_up(MouseButton::Left);
    app.confirm_move(&[root.join("note.txt")], &main, false);
    assert!(main.join("note.txt").is_file());
}

#[test]
fn dragging_past_the_sidebar_reaches_merged_folders_that_do_not_fit() {
    let dir = sample_folder("compact-drop-overflow");
    let root = dir.join("project");
    let names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let deep: PathBuf = names.iter().collect();
    std::fs::create_dir_all(root.join(&deep).join("x")).unwrap();
    std::fs::create_dir_all(root.join(&deep).join("y")).unwrap();
    std::fs::write(root.join("note.txt"), "").unwrap();
    let mut app = app_with(&root, true);
    app.sidebar.width = 230.0;
    let root = app.editor.root().unwrap().to_path_buf();
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let golf = root.join("alpha/bravo/charlie/delta/echo/foxtrot/golf");
    let file = row_of(&app, "note.txt");
    let (x, y) = tree_row_center(&app, file);
    let top = app.tree_row_top(row) + TREE_ROW / 2.0;

    // 拖到放不下名称的一行上：编辑区上方继续绘制完整名称。
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(app.layout.tree.right() - 4.0, top);
    paint(&mut app);
    let (rect, index) = app.tree.overflow.unwrap();
    assert_eq!(index, row);

    // 继续移到那一段上的某一级：放进那一级。
    let at = segment_x(&app, row, &golf);
    assert!(at > rect.x && at < rect.right(), "{at} {rect:?}");
    app.on_mouse_move(at, top);
    assert_eq!(app.tree_dragging().unwrap().target, Some(golf.clone()));
    paint(&mut app);
    assert!(app.tree.overflow.is_some());

    // 移开到编辑区：不再有放置目标。
    app.on_mouse_move(rect.right() + 40.0, top);
    paint(&mut app);
    app.on_mouse_move(rect.right() + 41.0, top);
    assert_eq!(app.tree_dragging().unwrap().target, None);
    app.on_mouse_up(MouseButton::Left);
}

#[test]
fn a_row_that_does_not_fit_covers_the_sidebar_splitter() {
    let dir = sample_folder("compact-splitter");
    let root = dir.join("project");
    let names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let deep: PathBuf = names.iter().collect();
    std::fs::create_dir_all(root.join(&deep).join("x")).unwrap();
    std::fs::create_dir_all(root.join("zzz")).unwrap();
    let mut app = app_with(&root, true);
    app.sidebar.width = 230.0;
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let x = app.layout.tree.right();
    let top = app.tree_row_top(row) + TREE_ROW / 2.0;
    assert!(app.layout.splitter.contains(x, top));

    // 名称放得下的行：分隔条照常可以拖动。
    let other = row_of(&app, "zzz");
    app.on_mouse_move(x, app.tree_row_top(other) + TREE_ROW / 2.0);
    paint(&mut app);
    assert_eq!(app.hover, Hover::Splitter);

    // 从左边移到放不下名称的一行：经过分隔条时仍算在这一行上。
    app.on_mouse_move(x - 20.0, top);
    paint(&mut app);
    assert!(app.tree.overflow.is_some());
    app.on_mouse_move(x, top);
    assert_eq!(app.hover, Hover::TreeRow(row));
}

#[test]
fn a_row_keeps_its_full_name_while_it_expands() {
    let dir = sample_folder("compact-motion");
    let root = dir.join("project");
    let names = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
    ];
    let deep: PathBuf = names.iter().collect();
    std::fs::create_dir_all(root.join(&deep).join("x")).unwrap();
    std::fs::create_dir_all(root.join(&deep).join("y")).unwrap();
    std::fs::create_dir_all(root.join("zzz")).unwrap();
    let mut app = app_with(&root, true);
    app.sidebar.width = 230.0;
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.right() - 4.0, y);
    paint(&mut app);
    assert!(app.tree.overflow.is_some());

    // 这一行正在展开、收起：完整名称不消失。
    let motion = |parent| crate::app::TreeMotion {
        parent,
        count: 2,
        expanding: false,
        removed: Vec::new(),
        clock: crate::app::animation::Clock::new(std::time::Duration::from_secs(60)),
    };
    app.tree.motion = Some(motion(row));
    assert_eq!(app.overflow_candidate(), Some(row));

    // 其他行在动画中：行的位置还在变，不显示。
    app.tree.motion = Some(motion(row + 1));
    assert_eq!(app.overflow_candidate(), None);
    app.tree.motion = None;
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
fn typing_finds_any_level_of_a_merged_row() {
    let (mut app, root, row) = compact_app("compact-typing");
    std::fs::create_dir_all(root.join("aaa")).unwrap();
    app.editor.workspace.as_mut().unwrap().refresh();
    paint(&mut app);
    let row = row + 1;
    app.focus = Focus::Tree;
    app.editor.workspace.as_mut().unwrap().select(Some(0));
    assert_eq!(
        app.editor.workspace.as_ref().unwrap().rows()[row].label(),
        "src/main/java"
    );
    app.on_key_tree(&winit::keyboard::Key::Character("m".into()));
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(row));
}

#[test]
fn arrow_keys_walk_through_merged_folders() {
    let (mut app, root, row) = compact_app("compact-keys");
    let (src, main, java) = (
        root.join("src"),
        root.join("src/main"),
        root.join("src/main/java"),
    );
    app.focus = Focus::Tree;
    let segment = |app: &App| {
        let ws = app.editor.workspace.as_ref().unwrap();
        ws.current_path().map(Path::to_path_buf)
    };
    app.editor.workspace.as_mut().unwrap().select(Some(row));
    assert_eq!(segment(&app), Some(java.clone()));

    // 从最后一级往左走到第一级，这一行不收起。
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowLeft);
    assert_eq!(segment(&app), Some(main.clone()));
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowLeft);
    assert_eq!(segment(&app), Some(src.clone()));
    let open = |app: &App| app.editor.workspace.as_ref().unwrap().rows()[row].expanded;
    assert!(open(&app));

    // 第一级再按左键才收起；往右走回最后一级，再按右键展开。
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowLeft);
    assert!(!open(&app));
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowRight);
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowRight);
    assert_eq!(segment(&app), Some(java.clone()));
    assert!(!open(&app));
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowRight);
    assert!(open(&app));

    // 上下键换行，回到这一行时选中的是最后一级。
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowDown);
    press_tree_key(&mut app, winit::keyboard::NamedKey::ArrowUp);
    assert_eq!(segment(&app), Some(java));
}

#[test]
fn hovering_tracks_the_merged_folder_under_the_pointer() {
    let (mut app, root, row) = compact_app("compact-hover");
    let (main, java) = (root.join("src/main"), root.join("src/main/java"));
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;

    // 同一行内换一级：悬停目标跟着换，并且请求重画。
    app.on_mouse_move(segment_x(&app, row, &main), y);
    paint(&mut app);
    assert_eq!(app.tree.hover_segment.as_ref(), Some(&main));
    app.dirty = false;
    app.on_mouse_move(segment_x(&app, row, &java), y);
    assert_eq!(app.tree.hover_segment.as_ref(), Some(&java));
    assert!(app.dirty);

    // 移到图标上、其他行上都不再悬停某一级。
    app.on_mouse_move(app.layout.tree.x + 12.0, y);
    assert_eq!(app.tree.hover_segment, None);
    app.on_mouse_move(segment_x(&app, row, &main), y);
    let below = app.tree_row_top(row + 1) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.x + 60.0, below);
    assert_eq!(app.tree.hover_segment, None);
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
        app.modifiers = modifiers;
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        app.modifiers = winit::keyboard::ModifiersState::empty();
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
    app.tree.labels.animate_without_window = true;
    let root = app.editor.root().unwrap().to_path_buf();
    paint(&mut app);
    let row = row_of(&app, "src");
    assert!(!app.tree.labels.moving());

    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let view = app.tree.labels.peek(&root.join("src")).unwrap();
    assert!(app.tree.labels.moving());
    assert_eq!(view.old, ["src"]);
    assert_eq!(view.common, 1);
}

#[test]
fn a_long_name_slides_out_on_hover_and_back_when_the_pointer_leaves() {
    let dir = sample_folder("label-hover");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    let mut app = app_with(&dir, true);
    app.tree.labels.animate_without_window = true;
    app.sidebar.width = 230.0;
    let dir = app.editor.root().unwrap().to_path_buf();
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let key = dir.join(long);
    let at_rest = app.tree.labels.peek(&key).unwrap().width;
    assert!(!app.tree.labels.moving());

    // 移上去：完整的命中范围立即生效，绘制的宽度从侧栏边缘开始缓动。
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.x + 60.0, y);
    paint(&mut app);
    let (rect, _) = app.tree.overflow.unwrap();
    let view = app.tree.labels.peek(&key).unwrap();
    assert!(app.tree.labels.moving());
    assert!(view.old.is_empty());
    assert!(
        view.width < rect.right() - app.layout.tree.x,
        "{}",
        view.width
    );
    assert!(
        (view.width - at_rest).abs() < 40.0,
        "{} {at_rest}",
        view.width
    );

    // 移开：不再响应，宽度缓动回去。
    app.on_mouse_move(
        app.layout.window.right() - 20.0,
        app.layout.window.bottom() - 40.0,
    );
    paint(&mut app);
    assert_eq!(app.tree.overflow, None);
    assert!(app.tree.labels.moving());
}

#[test]
fn a_row_that_does_not_fit_covers_the_tree_scrollbar() {
    let dir = sample_folder("overflow-scrollbar");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    for i in 0..80 {
        std::fs::write(dir.join(format!("z{i:02}.txt")), "").unwrap();
    }
    let mut app = app_with(&dir, true);
    app.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row) + TREE_ROW / 2.0;
    let other = app.tree_row_top(row + 2) + TREE_ROW / 2.0;
    let x = app.layout.tree.right() - 3.0;

    // 名称放得下的行：滚动条照常响应。
    app.on_mouse_move(x, other);
    assert!(matches!(app.hover, Hover::Scrollbar(_)), "{:?}", app.hover);

    // 从左边移到放不下名称的一行：经过滚动条时仍算在这一行上，名称不收起。
    app.on_mouse_move(x - 40.0, top);
    paint(&mut app);
    assert!(app.tree.overflow.is_some());
    app.on_mouse_move(x, top);
    paint(&mut app);
    assert_eq!(app.hover, Hover::TreeRow(row));
    assert!(app.tree.overflow.is_some());
}

#[test]
fn a_row_that_does_not_fit_stays_above_the_tree_scrollbar() {
    let dir = sample_folder("overflow-over-thumb");
    for i in 0..80 {
        let name = format!("z{i:02}_a_very_long_file_name_that_does_not_fit_in_the_sidebar.txt");
        std::fs::write(dir.join(name), "").unwrap();
    }
    let mut app = app_with(&dir, true);
    app.sidebar.width = 230.0;
    paint(&mut app);
    let tree = app.layout.tree;
    let gutter = tree.right() - 4.0;

    // 从行内移到右边缘：这一行优先，也不唤出滚动条。
    let row = row_of(&app, "z05");
    let top = app.tree_row_top(row);
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert!(app.tree.overflow.is_some());
    app.on_mouse_move(gutter, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert_eq!(app.hover, Hover::TreeRow(row));
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 0.0);

    // 沿边缘移到下一行：那一行还没展开，滚动条接管，拿得到。
    app.on_mouse_move(gutter, top + TREE_ROW * 1.5);
    assert!(matches!(app.hover, Hover::Scrollbar(_)), "{:?}", app.hover);
    paint(&mut app);
    assert_eq!(app.tree.overflow, None);
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 1.0);

    // 滚动时滚动条处于显示状态：写全名的一行覆盖在滑块之上，下一行不覆盖。先滚过开头几个名字
    // 较短的文件，使滑块旁边是名字放不下的行。
    app.on_mouse_move(tree.x + 60.0, tree.y + 30.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -3.0));
    paint(&mut app);
    let total = app.editor.workspace.as_ref().unwrap().rows().len() as f32 * TREE_ROW;
    let thumb = crate::layout::scrollbar_thumb(tree, app.tree.scroll, total).unwrap();
    let y = thumb.y + thumb.h / 2.0;
    let row = app.tree_row_at(y).unwrap();
    let rows = app.editor.workspace.as_ref().unwrap().rows();
    let name = rows[row].path.to_string_lossy();
    assert!(name.contains("a_very_long"), "{name}");
    let top = app.tree_row_top(row);
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert!(app.tree.overflow.is_some());
    assert!(app.scrollbar_shown(ScrollArea::Tree) > 0.0);
    let pixel = |app: &App, x: f32, y: f32| {
        let image = app.renderer.as_ref().unwrap().image().unwrap();
        let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let x = thumb.x + thumb.w / 2.0;
    assert_eq!(pixel(&app, x, top + 4.0), pixel(&app, x - 20.0, top + 4.0));
    let below = top + TREE_ROW + 4.0;
    assert_ne!(pixel(&app, x, below), pixel(&app, x - 20.0, below));
}

#[test]
fn a_retracting_name_shares_one_background_across_the_sidebar_edge() {
    let dir = sample_folder("retract-background");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    let mut app = app_with(&dir, true);
    app.tree.labels.animate_without_window = true;
    app.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row);
    let tree = app.layout.tree;
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    app.tree.labels.finish();
    paint(&mut app);

    // 移开后刚开始收回：侧栏内和溢出的那一段底色相同。
    app.on_mouse_move(
        app.layout.window.right() - 20.0,
        app.layout.window.bottom() - 40.0,
    );
    paint(&mut app);
    assert!(app.tree.labels.moving());
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let y = top + 4.0;
    let (inside, outside) = (pixel(tree.right() - 30.0, y), pixel(tree.right() + 6.0, y));
    for shift in [16, 8, 0] {
        let (a, b) = ((inside >> shift) & 0xff, (outside >> shift) & 0xff);
        assert!(a.abs_diff(b) <= 3, "{inside:x} {outside:x}");
    }
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
    app.sidebar.width = 230.0;
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    app.tree.labels.animate_without_window = true;
    paint(&mut app);
    assert!(!app.tree.labels.moving());

    for width in [250.0, 300.0, 210.0] {
        app.sidebar.width = width;
        paint(&mut app);
        assert!(!app.tree.labels.moving(), "{width}");
    }
    app.tree.scroll_x = 30.0;
    paint(&mut app);
    assert!(!app.tree.labels.moving());
}

#[test]
fn a_retracting_name_is_not_drawn_once_the_tree_is_hidden() {
    let dir = sample_folder("label-hidden");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    let mut app = app_with(&dir, true);
    app.tree.labels.animate_without_window = true;
    app.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row);
    let tree = app.layout.tree;
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    app.tree.labels.finish();
    paint(&mut app);
    app.on_mouse_move(
        app.layout.window.right() - 20.0,
        app.layout.window.bottom() - 40.0,
    );
    paint(&mut app);
    assert!(app.tree.labels.moving());

    // 切到搜索视图：编辑区上方恢复背景色，不留下收回中的名字。
    app.sidebar.view = SidebarView::Search;
    paint(&mut app);
    let image = app.renderer.as_ref().unwrap().image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let band = pixel(app.layout.tree.right() + 40.0, top + 4.0);
    let below = pixel(app.layout.tree.right() + 40.0, top + 4.0 + TREE_ROW * 3.0);
    assert_eq!(band, below);
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
    app.last_click = None;
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
    app.last_click = None;
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
