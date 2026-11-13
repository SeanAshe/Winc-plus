//! 文件树各行的测试：合并的目录、过长的名称和悬停时的展开。

use super::*;

#[test]
fn long_tree_names_unfold_under_the_mouse() {
    let dir = sample_folder("unfold");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.panels.sidebar.width = 230.0;
    let long = row_of(&app, "img_WNoi");
    let short = row_of(&app, "short");

    // 名称放得下的行不展开。
    hover_tree_row(&mut app, short, 120.0);
    paint(&mut app);
    assert_eq!(app.panels.tree.overflow, None);

    // 名称放不下的行，从侧栏右边缘起继续绘制到名称末尾。
    hover_tree_row(&mut app, long, 120.0);
    paint(&mut app);
    let (rect, index) = app.panels.tree.overflow.unwrap();
    assert_eq!((rect.x, index), (app.layout.tree.right(), long));

    // 鼠标移到展开的那一段上仍算悬停在该行，点击则打开该文件。
    let y = rect.y + rect.h / 2.0;
    app.on_mouse_move(rect.right() - 4.0, y);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::TreeRow(long));
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
        app.overlays.context_menu.as_ref().unwrap().target,
        MenuTarget::Tree(Some(path))
    );

    // 关掉菜单，鼠标移出到编辑区后收起展开的部分。
    app.dismiss_popup();
    app.on_mouse_move(rect.right() + 4.0, y);
    paint(&mut app);
    assert!(
        !matches!(app.pointer.hover, Hover::TreeRow(_)),
        "{:?}",
        app.pointer.hover
    );
    assert_eq!(app.panels.tree.overflow, None);

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
    app.panels.sidebar.width = MIN_PANEL_WIDTH;
    app.relayout();
    paint(&mut app);
    let tree = app.layout.tree;
    let thumb = hscrollbar_thumb(tree, 0.0, app.panels.tree.content_w).unwrap();
    let x = thumb.x + thumb.w / 2.0;
    // 紧挨在滚动条那一条上面的行。
    let last_row = app
        .tree_row_at(tree.bottom() - SCROLLBAR_HIT - 1.0)
        .unwrap();

    // 从树底向上 `SCROLLBAR_HIT` 高的一条属于滚动条：其下的行不显示悬停高亮，长名称也不展开。
    app.on_mouse_move(x, tree.bottom() - SCROLLBAR_HIT);
    paint(&mut app);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::TreeHorizontal)
    );
    assert_eq!(app.panels.tree.overflow, None);
    assert!(app.scrollbar_active(Scrollbar::TreeHorizontal));
    app.on_mouse_move(x, tree.bottom() - SCROLLBAR_HIT - 1.0);
    assert_eq!(app.pointer.hover, Hover::TreeRow(last_row));
    assert!(!app.scrollbar_active(Scrollbar::TreeHorizontal));

    // 点击滑块右侧的轨道：滑块立即跳到点击处，无需移动鼠标。
    let track_x = thumb.right() + 30.0;
    app.on_mouse_move(track_x, thumb.y + 3.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.panels.tree.scroll_x > 0.0);

    // 拖动滑块时鼠标划过文件树的行，悬停仍保持在滚动条上；松开后才轮到下方的行，
    // 且没有打开其他文件。
    app.on_mouse_move(track_x, tree.y + 10.0);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::TreeHorizontal)
    );
    app.on_mouse_up(MouseButton::Left);
    assert!(matches!(app.pointer.hover, Hover::TreeRow(_)));
    assert_eq!(app.editor.active().unwrap().name(), "a.txt");

    // 编辑区右侧的滚动条同样如此：该区域属于滚动条，按下时不会把光标放进文本里。
    let code: String = (0..300).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.rs"), code).unwrap();
    app.open_path(&dir.join("long.rs"));
    paint(&mut app);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.right() - 4.0, editor.bottom() - 40.0);
    assert_eq!(app.pointer.hover, Hover::Scrollbar(Scrollbar::Editor));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    assert_eq!(app.editor.active().unwrap().selection.head, 0);

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
    assert!(app.bars.goto_input.is_none());

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
    assert!(app.panels.name_input.is_none());
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
    let entry = app.panels.name_input.as_ref().unwrap();
    assert_eq!((entry.index, entry.depth), (src + 1, 1));
    assert_eq!(
        app.name_input_rect().unwrap().y,
        app.layout.tree.y + (src as f32 + 1.0) * TREE_ROW
    );

    // 名称输入框的起点与普通行的名称相同：缩进 + 箭头 + 图标 + 间隔，并随横向滚动偏移。
    let name_x = |app: &App, depth: usize| {
        let tree = app.layout.tree;
        tree.x + 48.0 + depth as f32 * TREE_INDENT - app.panels.tree.scroll_x
    };
    paint(&mut app);
    assert_eq!(
        app.panels.name_input.as_ref().unwrap().field.view.area.x,
        name_x(&app, 1)
    );
    app.panels.tree.scroll_x = 12.0;
    paint(&mut app);
    assert_eq!(
        app.panels.name_input.as_ref().unwrap().field.view.area.x,
        name_x(&app, 1)
    );
    app.panels.tree.scroll_x = 0.0;

    // 收起的目录：先展开，输入行落在子行的第一行；展开走动画流程（离屏测试中直接到位）。
    app.close_name_input();
    let target = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().selected = Some(target);
    app.editor.workspace.as_mut().unwrap().toggle(target);
    assert!(!app.editor.workspace.as_ref().unwrap().rows()[target].expanded);
    app.run_tree_tool(TreeTool::NewFolder);
    assert!(app.editor.workspace.as_ref().unwrap().rows()[target].expanded);
    assert_eq!(app.panels.name_input.as_ref().unwrap().index, target + 1);

    // 名称已存在：输入行保留，原因显示在右侧，磁盘上不做修改。
    app.edit_field(|f| f.insert("main.rs"));
    app.submit_name_input();
    assert_eq!(
        app.panels.name_input.as_ref().unwrap().error.as_deref(),
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
    assert!(app.panels.name_input.is_none());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn narrow_sidebar_folds_tools_before_the_name() {
    let dir = sample_folder("a-rather-long-project-name");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.panels.sidebar.width = 400.0;
    paint(&mut app);
    assert_eq!(app.hits.tree_tools.len(), 5);
    let full = app.hits.project.w;

    // 从宽到窄一点点收：工具栏还留着按钮时项目名完整显示，按钮一个个收进「⋯」；
    // 只剩「⋯」以后项目名才变短，最后停在最小宽度。
    let mut width = app.panels.sidebar.width;
    let mut shortened = false;
    while width >= MIN_PANEL_WIDTH {
        app.panels.sidebar.width = width;
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
    app.panels.sidebar.width = MIN_PANEL_WIDTH;
    paint(&mut app);
    let more = app.hits.tree_more.unwrap();
    assert!(app.hits.tree_tools.len() < 5);
    let hit = app.hits.project;
    assert!(hit.w >= TREE_NAME_MIN);
    assert!(app.hits.tree_tools.iter().all(|(r, _)| r.x >= hit.right()));
    app.on_mouse_move(hit.x + hit.w / 2.0, hit.y + hit.h / 2.0);
    assert_eq!(app.pointer.hover, Hover::ProjectName);

    // 点击「⋯」弹出系统菜单，列出被收起的按钮，顺序与工具栏中的顺序相同；
    // 悬停提示为「更多操作」。
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};
    app.on_mouse_move(more.x + 4.0, more.y + 4.0);
    assert_eq!(app.pointer.hover, Hover::TreeMore);
    app.on_mouse_down(MouseButton::Left);
    let menu = app.overlays.context_menu.clone().unwrap();
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
    app.panels.tree.motion = Some(crate::app::TreeMotion {
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
    app.panels.sidebar.width = 230.0;
    app.editor.workspace.as_mut().unwrap().selected = None;
    app.start_new_entry(false);
    paint(&mut app);
    let long = row_of(&app, "img_WNoi");

    // 输入行插在最上面，长名称所在的行下移一行；展开的部分画在该行自己的位置上。
    let tree = app.layout.tree;
    let y = tree.y + (long + 1) as f32 * TREE_ROW + TREE_ROW / 2.0;
    app.on_mouse_move(tree.x + 120.0, y);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::TreeRow(long));
    let (rect, index) = app.panels.tree.overflow.unwrap();
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
    assert!(app.panels.name_input.is_none());
    app.edit_field(|f| f.insert("2"));
    assert_eq!(app.bars.goto_input.as_ref().unwrap().text(), "2");
    app.close_prompt();

    // 没有输入任何内容就回车：输入行收起，焦点回到文件树，而不是停在已经消失的输入框上。
    app.start_new_entry(false);
    app.submit_name_input();
    assert!(app.panels.name_input.is_none());
    assert_eq!(app.focus, Focus::Tree);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rows_scrolled_into_view_are_not_hidden_under_sticky_folders() {
    let dir = nested_folder("sticky-reveal");
    let mut app = app_with(&dir, false);
    let (_, core) = expand_nested(&mut app);
    let tree = app.layout.tree;

    // 向上选择一个已滚出视野的文件：滚动到固定的两层目录下方，保证可见。
    app.panels.tree.scroll = (core + 20) as f32 * TREE_ROW;
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
    let entry = app.panels.name_input.as_mut().unwrap();
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
    app.panels.sidebar.width = 230.0;
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
    let (rect, _) = app.panels.tree.overflow.unwrap();
    let x = segment_x(&app, row, &golf);
    assert!(x > rect.x && x < rect.right(), "{x} {rect:?}");
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::TreeRow(row));

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
    app.panels.sidebar.width = 230.0;
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
    let (rect, index) = app.panels.tree.overflow.unwrap();
    assert_eq!(index, row);

    // 继续移到那一段上的某一级：放进那一级。
    let at = segment_x(&app, row, &golf);
    assert!(at > rect.x && at < rect.right(), "{at} {rect:?}");
    app.on_mouse_move(at, top);
    assert_eq!(app.tree_dragging().unwrap().target, Some(golf.clone()));
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());

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
    app.panels.sidebar.width = 230.0;
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
    assert_eq!(app.pointer.hover, Hover::Splitter);

    // 从左边移到放不下名称的一行：经过分隔条时仍算在这一行上。
    app.on_mouse_move(x - 20.0, top);
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());
    app.on_mouse_move(x, top);
    assert_eq!(app.pointer.hover, Hover::TreeRow(row));
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
    app.panels.sidebar.width = 230.0;
    let row = row_of(&app, "alpha");
    app.editor.workspace.as_mut().unwrap().toggle(row);
    paint(&mut app);
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.right() - 4.0, y);
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());

    // 这一行正在展开、收起：完整名称不消失。
    let motion = |parent| crate::app::TreeMotion {
        parent,
        count: 2,
        expanding: false,
        removed: Vec::new(),
        clock: crate::app::animation::Clock::new(std::time::Duration::from_secs(60)),
    };
    app.panels.tree.motion = Some(motion(row));
    assert_eq!(app.overflow_candidate(), Some(row));

    // 其他行在动画中：行的位置还在变，不显示。
    app.panels.tree.motion = Some(motion(row + 1));
    assert_eq!(app.overflow_candidate(), None);
    app.panels.tree.motion = None;
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
    assert_eq!(app.panels.tree.hover_segment.as_ref(), Some(&main));
    app.frame.dirty = false;
    app.on_mouse_move(segment_x(&app, row, &java), y);
    assert_eq!(app.panels.tree.hover_segment.as_ref(), Some(&java));
    assert!(app.frame.dirty);

    // 移到图标上、其他行上都不再悬停某一级。
    app.on_mouse_move(app.layout.tree.x + 12.0, y);
    assert_eq!(app.panels.tree.hover_segment, None);
    app.on_mouse_move(segment_x(&app, row, &main), y);
    let below = app.tree_row_top(row + 1) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.x + 60.0, below);
    assert_eq!(app.panels.tree.hover_segment, None);
}

#[test]
fn a_long_name_slides_out_on_hover_and_back_when_the_pointer_leaves() {
    let dir = sample_folder("label-hover");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    let mut app = app_with(&dir, true);
    app.panels.tree.labels.animate_without_window = true;
    app.panels.sidebar.width = 230.0;
    let dir = app.editor.root().unwrap().to_path_buf();
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let key = dir.join(long);
    let at_rest = app.panels.tree.labels.peek(&key).unwrap().width;
    assert!(!app.panels.tree.labels.moving());

    // 移上去：完整的命中范围立即生效，绘制的宽度从侧栏边缘开始缓动。
    let y = app.tree_row_top(row) + TREE_ROW / 2.0;
    app.on_mouse_move(app.layout.tree.x + 60.0, y);
    paint(&mut app);
    let (rect, _) = app.panels.tree.overflow.unwrap();
    let view = app.panels.tree.labels.peek(&key).unwrap();
    assert!(app.panels.tree.labels.moving());
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
    assert_eq!(app.panels.tree.overflow, None);
    assert!(app.panels.tree.labels.moving());
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
    app.panels.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row) + TREE_ROW / 2.0;
    let other = app.tree_row_top(row + 2) + TREE_ROW / 2.0;
    let x = app.layout.tree.right() - 3.0;

    // 名称放得下的行：滚动条照常响应。
    app.on_mouse_move(x, other);
    assert!(
        matches!(app.pointer.hover, Hover::Scrollbar(_)),
        "{:?}",
        app.pointer.hover
    );

    // 从左边移到放不下名称的一行：经过滚动条时仍算在这一行上，名称不收起。
    app.on_mouse_move(x - 40.0, top);
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());
    app.on_mouse_move(x, top);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::TreeRow(row));
    assert!(app.panels.tree.overflow.is_some());
}

#[test]
fn a_row_that_does_not_fit_stays_above_the_tree_scrollbar() {
    let dir = sample_folder("overflow-over-thumb");
    for i in 0..80 {
        let name = format!("z{i:02}_a_very_long_file_name_that_does_not_fit_in_the_sidebar.txt");
        std::fs::write(dir.join(name), "").unwrap();
    }
    let mut app = app_with(&dir, true);
    app.panels.sidebar.width = 230.0;
    paint(&mut app);
    let tree = app.layout.tree;
    let gutter = tree.right() - 4.0;

    // 从行内移到右边缘：这一行优先，也不唤出滚动条。
    let row = row_of(&app, "z05");
    let top = app.tree_row_top(row);
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());
    app.on_mouse_move(gutter, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::TreeRow(row));
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 0.0);

    // 沿边缘移到下一行：那一行还没展开，滚动条接管，拿得到。
    app.on_mouse_move(gutter, top + TREE_ROW * 1.5);
    assert!(
        matches!(app.pointer.hover, Hover::Scrollbar(_)),
        "{:?}",
        app.pointer.hover
    );
    paint(&mut app);
    assert_eq!(app.panels.tree.overflow, None);
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 1.0);

    // 滚动时滚动条处于显示状态：写全名的一行覆盖在滑块之上，下一行不覆盖。先滚过开头几个名字
    // 较短的文件，使滑块旁边是名字放不下的行。
    app.on_mouse_move(tree.x + 60.0, tree.y + 30.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -3.0));
    paint(&mut app);
    let total = app.editor.workspace.as_ref().unwrap().rows().len() as f32 * TREE_ROW;
    let thumb = crate::layout::scrollbar_thumb(tree, app.panels.tree.scroll, total).unwrap();
    let y = thumb.y + thumb.h / 2.0;
    let row = app.tree_row_at(y).unwrap();
    let rows = app.editor.workspace.as_ref().unwrap().rows();
    let name = rows[row].path.to_string_lossy();
    assert!(name.contains("a_very_long"), "{name}");
    let top = app.tree_row_top(row);
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    assert!(app.panels.tree.overflow.is_some());
    assert!(app.scrollbar_shown(ScrollArea::Tree) > 0.0);
    let pixel = |app: &App, x: f32, y: f32| {
        let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
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
    app.panels.tree.labels.animate_without_window = true;
    app.panels.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row);
    let tree = app.layout.tree;
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    app.panels.tree.labels.finish();
    paint(&mut app);

    // 移开后刚开始收回：侧栏内和溢出的那一段底色相同。
    app.on_mouse_move(
        app.layout.window.right() - 20.0,
        app.layout.window.bottom() - 40.0,
    );
    paint(&mut app);
    assert!(app.panels.tree.labels.moving());
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
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
fn a_retracting_name_is_not_drawn_once_the_tree_is_hidden() {
    let dir = sample_folder("label-hidden");
    let long = "a_very_long_file_name_that_does_not_fit_in_the_sidebar_at_all.txt";
    std::fs::write(dir.join(long), "").unwrap();
    let mut app = app_with(&dir, true);
    app.panels.tree.labels.animate_without_window = true;
    app.panels.sidebar.width = 230.0;
    paint(&mut app);
    let row = row_of(&app, "a_very_long");
    let top = app.tree_row_top(row);
    let tree = app.layout.tree;
    app.on_mouse_move(tree.x + 60.0, top + TREE_ROW / 2.0);
    paint(&mut app);
    app.panels.tree.labels.finish();
    paint(&mut app);
    app.on_mouse_move(
        app.layout.window.right() - 20.0,
        app.layout.window.bottom() - 40.0,
    );
    paint(&mut app);
    assert!(app.panels.tree.labels.moving());

    // 切到搜索视图：编辑区上方恢复背景色，不留下收回中的名字。
    app.panels.sidebar.view = SidebarView::Search;
    paint(&mut app);
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let pixel = |x: f32, y: f32| {
        let (x, y) = ((x * SCALE) as usize, (y * SCALE) as usize);
        image.pixels[y * image.width as usize + x]
    };
    let band = pixel(app.layout.tree.right() + 40.0, top + 4.0);
    let below = pixel(app.layout.tree.right() + 40.0, top + 4.0 + TREE_ROW * 3.0);
    assert_eq!(band, below);
}
