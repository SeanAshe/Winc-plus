//! 截图的测试：文件树（钉住的目录、工具栏、刷新、拖动）和拖动标签。

use super::*;

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_sticky_folders() {
    let dir = crate::app::snapshot::workspace::tree::nested_folder("sticky-shots");

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut app = app_with(&dir, dark);
        let (_, core) = crate::app::snapshot::workspace::tree::expand_nested(&mut app);

        // 滚动到 `core` 内部，鼠标停在固定在顶部的 `core` 上。
        app.tree.scroll = (core + 5) as f32 * TREE_ROW + 9.0;
        paint(&mut app);
        let tree = app.layout.tree;
        app.on_mouse_move(tree.x + 80.0, tree.y + TREE_ROW + 5.0);
        shoot(&mut app, &format!("tree-sticky-{theme}"));

        // `core` 的最后一行即将滚出，`core` 被向上推出一半。
        app.on_mouse_move(900.0, 500.0);
        app.tree.scroll = (core + 29) as f32 * TREE_ROW + 13.0;
        shoot(&mut app, &format!("tree-sticky-pushed-{theme}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_tree_tools_folding() {
    use crate::app::ToolFold;
    // 与 `snapshot_tree_tools` 使用不同的目录：两个测试并行运行，先结束的会删除该目录。
    let dir = sample_folder("tree-tools-fold");

    // 收起动画的几个中间状态：定位和全部折叠滑进「⋯」，左边的按钮跟着右移。
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut app = app_with(&dir.join("a.txt"), dark);
        app.on_mouse_move(900.0, 500.0);
        for t in [1.0, 0.75, 0.5, 0.25, 0.0] {
            app.tool_fold = ToolFold::frozen([1.0, 1.0, 1.0, t, t], 1.0 - t);
            shoot(
                &mut app,
                &format!("tree-tools-fold-{}-{theme}", (t * 100.0) as u32),
            );
        }
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_tree_tools() {
    let dir = sample_folder("tree-tools");

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut app = app_with(&dir.join("a.txt"), dark);
        app.on_mouse_move(900.0, 500.0);
        shoot(&mut app, &format!("tree-tools-{theme}"));

        // 鼠标停在「刷新」上，提示已显示。`TreeState::tools` 在绘制时填充，因此先绘制一帧。
        let refresh = app
            .hits
            .tree_tools
            .iter()
            .find(|(_, tool)| *tool == TreeTool::Refresh)
            .map(|(r, _)| *r)
            .unwrap();
        let (cx, cy) = (refresh.x + refresh.w / 2.0, refresh.y + refresh.h / 2.0);
        app.on_mouse_move(cx, cy);
        // 将悬停起始时间前移 `TOOLTIP_DELAY`（500 毫秒），模拟悬停已满该时长。
        app.tooltip.since = Some(Instant::now() - TOOLTIP_DELAY);
        shoot(&mut app, &format!("tree-tools-tip-{theme}"));

        // 新建文件的输入条。
        app.run_tree_tool(TreeTool::NewFile);
        app.edit_field(|f| f.insert("sample/hello.rs"));
        shoot(&mut app, &format!("tree-tools-new-{theme}"));

        // 侧栏较窄、名称较长时：名称截短，放不下的工具栏按钮从右向左收进「⋯」菜单。
        app.close_prompt();
        app.focus = Focus::Editor;
        app.sidebar.width = 200.0;
        app.relayout();
        app.on_mouse_move(900.0, 500.0);
        shoot(&mut app, &format!("tree-tools-narrow-{theme}"));

        // 最窄时：放不下的按钮收进「⋯」（点了弹系统菜单，截不到图）。
        app.tree.scroll_x = 0.0;
        app.sidebar.width = MIN_PANEL_WIDTH;
        paint(&mut app);
        shoot(&mut app, &format!("tree-more-{theme}"));
        app.sidebar.width = 200.0;

        // 横向滚动到底：缩进、图标、名称一起左移，底色不动，底部显示横向滑块。
        app.tree.scroll_x = app.tree_max_scroll_x();
        shoot(&mut app, &format!("tree-hscroll-{theme}"));

        // 鼠标停在横向滚动条所在区域：滑块颜色加深，宽度不变。
        let tree = app.layout.tree;
        let thumb = hscrollbar_thumb(tree, app.tree.scroll_x, app.tree.content_w).unwrap();
        app.on_mouse_move(thumb.x + thumb.w / 2.0, thumb.y + 3.0);
        shoot(&mut app, &format!("tree-hscroll-hover-{theme}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_refresh() {
    // 以本仓库作为样本：目录层级足够深，可以看出刷新前后展开状态是否保持。
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&repo, dark);
        app.sidebar.width = 230.0;
        // 展开 crates，再点刷新。
        let i = row_of(&app, "crates");
        app.editor.workspace.as_mut().unwrap().toggle(i);
        paint(&mut app);
        app.run_tree_tool(TreeTool::Refresh);
        paint(&mut app);
        shoot(&mut app, &format!("refresh-{tag}"));
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_tree_drag() {
    let dir = search_folder("drag-shot");
    std::fs::create_dir_all(dir.join("lib/deep")).unwrap();
    std::fs::write(dir.join("lib/deep/x.rs"), "").unwrap();
    std::fs::write(dir.join("lib/mod.rs"), "").unwrap();
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        let lib = row_of(&app, "lib");
        app.editor.workspace.as_mut().unwrap().toggle(lib);
        paint(&mut app);

        // 按住 `Cargo.toml` 拖到 `lib` 上：`lib` 连同其展开的子行铺一层底色，鼠标旁跟随显示
        // 被拖动的名称。
        let cargo = row_of(&app, "Cargo.toml");
        let (x, y) = tree_row_center(&app, cargo);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        let (_, lib_y) = tree_row_center(&app, lib + 2);
        app.on_mouse_move(x + 20.0, lib_y);
        shoot(&mut app, &format!("tree-drag-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_tab_drag() {
    let dir = search_folder("tab-drag-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/main.rs"));
        app.open_path(&dir.join("src/config.rs"));
        app.open_path(&dir.join("Cargo.toml"));
        paint(&mut app);

        // 按住 `main.rs` 向右拖到 `config.rs` 和 `Cargo.toml` 之间：`config.rs` 已让位到左侧，
        // `main.rs` 跟随鼠标，部分压在 `Cargo.toml` 上。让位的标签直接画在最终位置
        // （清空 `slides` 以跳过滑动动画）。
        let tab = app.hits.tabs.iter().find(|h| h.index == 0).unwrap().tab;
        let (x, y) = (tab.x + 20.0, tab.y + tab.h / 2.0);
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_move(x + 140.0, y);
        app.tabs.slides.clear();
        shoot(&mut app, &format!("tab-drag-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
