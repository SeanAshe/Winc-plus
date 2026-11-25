//! 右键菜单与关闭标签的测试：文件树和标签栏菜单的目标与菜单项、重命名同步已打开文档、
//! 删除后标签的取舍、有未保存修改时的保存提示。

use super::*;

#[test]
fn closing_unsaved_tabs_asks_save_discard_or_cancel() {
    let dir = search_folder("closing");
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    let main = root.join("src/main.rs");
    let original = std::fs::read_to_string(&main).unwrap();
    app.open_path(&main);
    app.run(Command::Insert("// x\n".into()));

    // 取消：标签和修改都保留。
    app.close_tab(0);
    assert_eq!(app.editor.documents.len(), 1);
    assert_eq!(app.closing.prompt.as_ref().unwrap().dirty.len(), 1);
    app.answer_save_prompt(SaveChoice::Cancel);
    assert!(app.closing.prompt.is_none());
    assert!(app.editor.active().unwrap().is_dirty());

    // 不保存：关闭标签，磁盘上的文件保持原样。
    app.close_tab(0);
    app.answer_save_prompt(SaveChoice::Discard);
    assert!(app.editor.documents.is_empty());
    assert_eq!(std::fs::read_to_string(&main).unwrap(), original);

    // ⇧⌘T 重新打开，光标回到关闭时的位置。
    app.reopen_closed();
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(main.as_path())
    );
    assert_eq!(app.editor.active().unwrap().selection.head, 5);
    assert!(app.history.closed.is_empty());

    // 保存：先保存，再关闭。
    app.run(Command::Insert("// y\n".into()));
    app.close_tab(0);
    app.answer_save_prompt(SaveChoice::Save);
    assert!(app.editor.documents.is_empty());
    assert!(std::fs::read_to_string(&main).unwrap().contains("// y"));

    // 未命名的文档：先弹出「另存为」，在面板上取消则不关闭任何标签。
    app.new_file();
    app.run(Command::Insert("draft".into()));
    let id = app.editor.active().unwrap().id;
    app.close_tab(0);
    app.answer_save_prompt(SaveChoice::Save);
    assert!(app.closing.continuation.is_some());
    app.on_picked(Pick::SaveAs(id), None);
    assert!(app.closing.continuation.is_none());
    assert_eq!(app.editor.documents.len(), 1);

    // 再次关闭并这次选定保存位置：文件写入磁盘，标签关闭。
    app.close_tab(0);
    app.answer_save_prompt(SaveChoice::Save);
    let draft = root.join("draft.txt");
    app.on_picked(Pick::SaveAs(id), Some(draft.clone()));
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "draft");
    assert!(app.editor.documents.is_empty());

    // 一次关闭多个标签：没有修改的直接关闭，有修改的合并成一次询问。
    app.open_path(&root.join("Cargo.toml"));
    app.open_path(&root.join("a.txt"));
    app.run(Command::Insert("1".into()));
    app.open_path(&main);
    app.run(Command::Insert("2".into()));
    let ids: Vec<_> = app.editor.documents.iter().map(|d| d.id).collect();
    app.close_tabs(ids);
    assert_eq!(app.editor.documents.len(), 2);
    assert_eq!(app.closing.prompt.as_ref().unwrap().dirty.len(), 2);
    // 提示框已显示时再请求关闭，不会再次询问。
    app.request_quit();
    assert_eq!(app.closing.prompt.as_ref().unwrap().dirty.len(), 2);
    app.answer_save_prompt(SaveChoice::Cancel);

    // 退出：有修改时先询问，选择不保存则直接退出；没有修改时不询问，直接退出。
    app.request_quit();
    assert!(!app.closing.quit_requested);
    app.answer_save_prompt(SaveChoice::Discard);
    assert!(app.closing.quit_requested);
    let mut clean = app_with(&dir, false);
    clean.request_quit();
    assert!(clean.closing.quit_requested && clean.closing.prompt.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tree_context_menu_targets_the_clicked_row() {
    let dir = search_folder("ctx-tree");
    let mut app = app_with(&dir.join("Cargo.toml"), false);
    app.trash = delete_for_test;
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    // 在文件上右键：先选中该文件，菜单中没有「在目录中查找」。
    let cargo = row_of(&app, "Cargo.toml");
    let at = tree_row_center(&app, cargo);
    right_click(&mut app, at);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(cargo));
    assert_eq!(app.focus, Focus::Tree);
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::Rename, true)));
    assert!(
        !commands
            .iter()
            .any(|(c, _)| *c == MenuCommand::FindInFolder)
    );

    app.run_menu_command(MenuCommand::CopyRelativePath);
    assert_eq!(app.clipboard.copied.as_deref(), Some("Cargo.toml"));
    assert!(app.overlays.context_menu.is_none());

    // 在空白处右键：目标是根目录，不能重命名，也不能删除。
    let below = app.layout.tree.bottom() - 10.0;
    let at = (app.layout.tree.x + 80.0, below);
    right_click(&mut app, at);
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, None);
    let commands = menu_commands(&app);
    assert!(!commands.iter().any(|(c, _)| *c == MenuCommand::Delete));
    app.run_menu_command(MenuCommand::CopyPath);
    assert_eq!(
        app.clipboard.copied.as_deref(),
        Some(root.to_str().unwrap())
    );
    // 菜单已经使用过，再次执行菜单命令不产生任何效果。
    app.run_menu_command(MenuCommand::Delete);
    assert!(root.is_dir());

    // 在 `src` 上「新建文件」：输入行插入到 `src` 的子树中。
    let src = row_of(&app, "src");
    let at = tree_row_center(&app, src);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::NewFile);
    let entry = app.panels.name_input.as_ref().unwrap();
    assert_eq!(entry.dir, root.join("src"));
    app.close_name_input();

    // 在 `src` 上「在目录中查找」：全局搜索的范围限定为 `./src`。
    let src = row_of(&app, "src");
    let at = tree_row_center(&app, src);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::FindInFolder);
    assert_eq!(app.panels.sidebar.view, SidebarView::Search);
    assert_eq!(app.panels.global.include.text(), "./src");
    assert!(app.panels.global.details_open);
    assert_eq!(app.focus, Focus::GlobalQuery);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rename_in_the_tree_moves_open_documents() {
    let dir = search_folder("ctx-rename");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    // 先修改内容，用于验证重命名之后修改仍然保留。
    app.run(Command::Insert("// x\n".into()));
    assert!(app.editor.active().unwrap().is_dirty());

    // 右键文件 → 重命名：该行原地变成输入框，选中扩展名之前的部分，不插入新行。
    let main = row_of(&app, "main.rs");
    let rows_before = app.editor.workspace.as_ref().unwrap().rows().len();
    let at = tree_row_center(&app, main);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::Rename);
    let entry = app.panels.name_input.as_ref().unwrap();
    assert_eq!(entry.index, main);
    assert_eq!(entry.field.selected_text(), "main");
    assert_eq!(app.focus, Focus::Prompt);
    assert_eq!(
        app.name_input_rect().unwrap().y,
        tree_row_center(&app, main).1 - TREE_ROW / 2.0
    );
    paint(&mut app);
    // 重命名期间下方的行位置不变：点击下一行的位置，命中的仍是下一行。
    assert_eq!(
        app.tree_row_at(tree_row_center(&app, main + 1).1),
        Some(main + 1)
    );
    assert_eq!(
        app.editor.workspace.as_ref().unwrap().rows().len(),
        rows_before
    );

    // 名称已存在：输入框保持打开，并显示原因。
    app.edit_field(|f| f.insert("config"));
    app.submit_prompt();
    assert_eq!(
        app.panels.name_input.as_ref().unwrap().error.as_deref(),
        Some("已存在同名项")
    );

    // 换用新名称：磁盘文件改名，标签的路径同步更新，未保存的修改仍然保留。
    app.edit_field(|f| {
        f.select_all();
        f.insert("app.rs")
    });
    app.submit_prompt();
    assert!(app.panels.name_input.is_none());
    assert!(root.join("src/app.rs").is_file() && !root.join("src/main.rs").exists());
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.path.as_deref(), Some(root.join("src/app.rs").as_path()));
    assert!(doc.is_dirty());
    assert_eq!(app.focus, Focus::Tree);

    // 目录改名：其中已打开文件的路径同步更新；用快捷键触发的重命名走同一流程。
    let src = row_of(&app, "src");
    app.editor.workspace.as_mut().unwrap().selected = Some(src);
    app.focus = Focus::Tree;
    app.start_rename(&root.join("src"));
    app.edit_field(|f| {
        f.select_all();
        f.insert("lib")
    });
    app.submit_prompt();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.path.as_deref(), Some(root.join("lib/app.rs").as_path()));

    // Esc 放弃：不做任何修改。
    app.start_rename(&root.join("Cargo.toml"));
    app.close_name_input();
    assert!(root.join("Cargo.toml").is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_from_the_tree_closes_clean_tabs_and_keeps_dirty_ones() {
    let dir = search_folder("ctx-delete");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    app.trash = delete_for_test;
    app.open_path(&dir.join("src/config.rs"));
    app.run(Command::Insert("// x\n".into()));
    app.open_path(&dir.join("Cargo.toml"));
    paint(&mut app);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();

    // 删除目录 `src`：没有修改的 `main.rs` 关闭，有修改的 `config.rs` 保留。
    let src = row_of(&app, "src");
    let at = tree_row_center(&app, src);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::Delete);
    assert!(!root.join("src").exists());
    let names: Vec<String> = app
        .editor
        .documents
        .iter()
        .map(|d| d.name().into_owned())
        .collect();
    assert_eq!(names, ["config.rs", "Cargo.toml"]);
    assert!(
        app.status
            .message
            .as_deref()
            .is_some_and(|m| m.contains(&format!("已移到{}", super::trash_name())))
    );
    // 选中原位置上的行（即原先排在 `src` 之后的那一行）。
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.selected, Some(src));
    assert_eq!(ws.selected_entry().unwrap().name, ws.rows()[src].name);

    // 保留的 `config.rs` 仍可保存：所在目录已被删除时，保存过程会重新创建该目录。
    let config = app.editor.documents[0].id;
    assert!(app.save_document(config, None));
    assert!(root.join("src/config.rs").is_file());

    // 焦点在文件树时按 ⌘⌫ 走同一流程；根目录本身不能删除。
    app.ask_delete(&root);
    assert!(root.is_dir());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tab_context_menu_closes_others_right_and_saved() {
    let dir = search_folder("ctx-tabs");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    app.open_path(&dir.join("src/config.rs"));
    app.open_path(&dir.join("Cargo.toml"));
    app.new_file();
    paint(&mut app);
    let names = |app: &App| -> Vec<String> {
        app.editor
            .documents
            .iter()
            .map(|d| d.name().into_owned())
            .collect()
    };
    let tab = |app: &App, i: usize| {
        let rect = app.hits.tabs.iter().find(|h| h.index == i).unwrap().tab;
        (rect.x + 20.0, rect.y + rect.h / 2.0)
    };

    // 未命名的标签：与路径相关的菜单项都置灰。
    let at = tab(&app, 3);
    right_click(&mut app, at);
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::CopyPath, false)));
    assert!(commands.contains(&(MenuCommand::CloseRight, false)));
    app.run_menu_command(MenuCommand::CopyPath);
    assert!(app.clipboard.copied.is_none());

    // 在 `config.rs` 上「关闭右侧」：右侧的两个标签关闭。
    let at = tab(&app, 1);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseRight);
    assert_eq!(names(&app), ["main.rs", "config.rs"]);

    // 有未保存修改的标签不会被「关闭其他」直接关闭，而是弹出保存提示。
    app.activate_tab(0);
    app.run(Command::Insert("// x\n".into()));
    paint(&mut app);
    let at = tab(&app, 1);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseOthers);
    assert_eq!(names(&app), ["main.rs", "config.rs"]);
    assert_eq!(app.closing.prompt.as_ref().unwrap().dirty.len(), 1);
    app.answer_save_prompt(SaveChoice::Cancel);

    // 「关闭已保存」只保留有未保存修改的标签。
    paint(&mut app);
    let at = tab(&app, 0);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::CloseSaved);
    assert_eq!(names(&app), ["main.rs"]);

    // 「在文件树中显示」：侧栏切回文件树，并选中该文件。
    app.set_sidebar_view(SidebarView::Search);
    paint(&mut app);
    let at = tab(&app, 0);
    right_click(&mut app, at);
    app.run_menu_command(MenuCommand::RevealInTree);
    assert_eq!(app.panels.sidebar.view, SidebarView::Files);
    let ws = app.editor.workspace.as_ref().unwrap();
    assert_eq!(ws.selected_entry().unwrap().name, "main.rs");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bookmark_commands_are_available_in_the_editor_menu() {
    let dir = sample_folder("bookmark-menu");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    let id = app.editor.active().unwrap().id;
    app.editor.active_mut().unwrap().selection = winc_core::Selection::point(0);

    let menu = |bookmarked| crate::app::ContextMenu {
        target: crate::app::input::context_menu::MenuTarget::Editor(id),
        entries: crate::app::input::context_menu::editor_menu(true, false, bookmarked, false, true),
        branches: Vec::new(),
        stashes: Vec::new(),
        popup: None,
    };

    app.overlays.context_menu = Some(menu(false));
    app.run_menu_command(crate::app::MenuCommand::AddBookmark);
    assert_eq!(app.editor.active().unwrap().bookmark_lines(), [0]);

    app.overlays.context_menu = Some(menu(true));
    app.run_menu_command(crate::app::MenuCommand::ListBookmarks);
    assert!(app.overlays.picker.open && app.overlays.picker.places.is_some());
    app.close_picker();

    app.overlays.context_menu = Some(menu(true));
    app.run_menu_command(crate::app::MenuCommand::RemoveBookmark);
    assert!(app.editor.active().unwrap().bookmark_lines().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn copy_reference_copies_the_lines() {
    use crate::app::snapshot::editor::navigation::press;
    use crate::keymap::KeyCode::Char;
    use winc_core::Selection;

    let dir = sample_folder("copy-reference");
    let mut app = app_with(&dir.join("short.rs"), false);
    let copy = |app: &mut App, anchor, head| {
        app.editor.active_mut().unwrap().selection = Selection { anchor, head };
        assert!(press(app, &["cmd", "shift", "alt"], Char('c')));
        app.clipboard.copied.clone().unwrap()
    };
    // `short.rs` 的第 0 行是 `fn main() {`，第 1 行从字符偏移 12 起，第 2 行从字符偏移 35 起。
    assert_eq!(copy(&mut app, 3, 3), "short.rs:1");
    assert_eq!(copy(&mut app, 20, 3), "short.rs:1-2");
    // 选区结束在行首时不含该行：位置 35 是第 3 行行首，位置 36 才进入该行。
    assert_eq!(copy(&mut app, 0, 35), "short.rs:1-2");
    assert_eq!(copy(&mut app, 0, 36), "short.rs:1-3");

    // 完整路径。
    assert!(app.run_action(crate::keymap::Action::CopyAbsoluteReference));
    let file = app.editor.active().unwrap().path.clone().unwrap();
    let expected = format!("{}:1-3", file.display());
    assert_eq!(app.clipboard.copied.as_deref(), Some(expected.as_str()));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn menus_without_a_system_menu_are_drawn_in_the_window() {
    use crate::app::PopupRow;
    use crate::app::status::status_bar::StatusItem;
    use winit::keyboard::{Key, NamedKey};

    let dir = sample_folder("popup-menu");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    // 没有窗口（等同没有系统菜单）：状态栏的编码菜单画在窗口里，两项都是子菜单。
    app.open_status_menu(StatusItem::Encoding);
    paint(&mut app);
    let popup = app.popup().expect("窗口内的菜单").clone();
    let top = &popup.levels[0];
    assert_eq!(top.rows.len(), 2);
    assert!(
        top.rows
            .iter()
            .all(|r| matches!(r, PopupRow::Submenu { .. }))
    );
    let window = app.layout.window;
    assert!(top.rect.x >= window.x && top.rect.right() <= window.right());
    assert!(top.rect.y >= window.y && top.rect.bottom() <= window.bottom());

    // 停在第一项上展开子菜单；编码菜单在窗口右下角，右边放不下，子菜单放在左边。移到第二项
    // 换成它的子菜单。
    let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (x, y) = center(top.row_rects[0]);
    app.on_mouse_move(x, y);
    paint(&mut app);
    let popup = app.popup().unwrap();
    assert_eq!(popup.levels.len(), 2);
    assert!(popup.levels[1].rect.right() <= top.rect.x + 4.0);
    let (x, y) = center(top.row_rects[1]);
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(app.popup().unwrap().levels[1].parent, Some(1));

    // 点子菜单里的一项：执行这项命令（以 UTF-8 保存，文件原本就是 UTF-8，没有可见的变化），
    // 菜单关掉，状态栏的按钮放开。
    let sub = app.popup().unwrap().levels[1].clone();
    let utf8 = sub
        .rows
        .iter()
        .position(|r| matches!(r, PopupRow::Item { label, .. } if label.title.contains("UTF-8")))
        .unwrap();
    let (x, y) = center(sub.row_rects[utf8]);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(app.overlays.menu_button, None);

    // 键盘：↓ 停在第一项，→ 展开子菜单并停在第一项，← 收起，Esc 关掉。
    app.open_status_menu(StatusItem::Encoding);
    paint(&mut app);
    let key = |app: &mut App, named: NamedKey| {
        assert!(app.popup_logical_key(&Key::Named(named)));
    };
    key(&mut app, NamedKey::ArrowDown);
    assert_eq!(app.popup().unwrap().levels[0].hover, Some(0));
    key(&mut app, NamedKey::ArrowRight);
    let popup = app.popup().unwrap();
    assert_eq!(popup.levels.len(), 2);
    assert!(popup.levels[1].hover.is_some());
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(app.popup().unwrap().levels.len(), 1);
    key(&mut app, NamedKey::Escape);
    assert!(app.overlays.context_menu.is_none());

    // 左键点在菜单外面：只关掉菜单，这次点击不传给下层，光标不动。
    app.open_status_menu(StatusItem::Encoding);
    paint(&mut app);
    let caret = app.editor.active().unwrap().selection;
    let text = app.layout.editor;
    app.on_mouse_move(text.x + 200.0, text.y + 60.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(app.editor.active().unwrap().selection, caret);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：窗口内的菜单和提示框为模态，打开后即显示箭头，即使鼠标下方是文字；提示框打开期间，
/// 下方的标签不显示悬停，按住主修饰键滚动也不缩放编辑区。关掉以后鼠标不动，指针也回到文字上的
/// 文本光标。
#[test]
fn menus_and_alerts_drawn_in_the_window_cover_what_is_below() {
    use crate::app::AlertButton;
    use winit::keyboard::{Key, NamedKey};
    use winit::window::CursorIcon;

    let dir = sample_folder("modal-pointer");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let l = app.layout.text;
    let text = (l.x + 8.0, l.y + app.fonts.line_height / 2.0);
    right_click(&mut app, text);
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Default);
    app.popup_logical_key(&Key::Named(NamedKey::Escape));
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Text);

    app.on_mouse_move(text.0, text.1);
    let buttons = vec![AlertButton::new("好")];
    assert!(app.show_alert("提示", "", buttons, None, |_, _| None));
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Default);
    app.pointer.modifiers = command_modifiers();
    let notch = MouseScrollDelta::LineDelta(0.0, 1.0);
    app.on_wheel_event(notch, winit::event::TouchPhase::Moved);
    assert_eq!(app.config.editor_size, None);
    let tab = app.hits.tabs[0].tab;
    app.on_mouse_move(tab.x + tab.w / 2.0, tab.y + tab.h / 2.0);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::None);
    app.on_mouse_move(text.0, text.1);
    assert!(app.alert_key(&Key::Named(NamedKey::Enter), false));
    assert!(app.overlays.alert.is_none());
    assert_eq!(app.pointer.cursor_icon, CursorIcon::Text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 窗口内绘制的语言菜单可以键入文字筛选：第一行写出键入的文字，名称以它开头的排在前面，停在
/// 第一项，↩ 选中；`⌫` 删去最后一个字，没有匹配时写一行说明，`Esc` 先清空筛选、再关掉菜单。
/// 按住 ⌘/Ctrl 的按键不筛选；有子菜单的菜单（编码）不筛选。
#[test]
fn typing_filters_the_language_menu() {
    use crate::app::PopupRow;
    use crate::app::status::status_bar::StatusItem;
    use winit::keyboard::{Key, NamedKey};
    let dir = sample_folder("popup-filter");
    let mut app = app_with(&dir.join("a.txt"), false);
    paint(&mut app);
    let typed = |app: &mut App, text: &str| {
        for c in text.chars() {
            let key = match c {
                ' ' => Key::Named(NamedKey::Space),
                c => Key::Character(c.to_string().into()),
            };
            assert!(app.popup_logical_key(&key));
        }
    };
    let named = |app: &mut App, key: NamedKey| assert!(app.popup_logical_key(&Key::Named(key)));
    let titles = |app: &App| -> Vec<String> {
        (app.popup().unwrap().levels[0].rows.iter())
            .filter_map(|r| match r {
                PopupRow::Item { label, .. } => Some(label.title.clone()),
                PopupRow::Query(q) => Some(format!("[{q}]")),
                _ => None,
            })
            .collect()
    };

    app.open_status_menu(StatusItem::Language);
    paint(&mut app);
    let all = app.popup().unwrap().levels[0].rows.len();
    typed(&mut app, "Ru");
    let found = titles(&app);
    assert_eq!(found[0], "[Ru]");
    assert_eq!(found[1], "Ruby");
    assert!(found.contains(&"Rust".to_owned()));
    // 名称以「ru」开头的都排在只是含有「ru」的之前。
    let prefix: Vec<bool> = (found[1..].iter())
        .map(|t| t.to_lowercase().starts_with("ru"))
        .collect();
    assert!(prefix.windows(2).all(|w| w[0] || !w[1]), "{found:?}");
    assert_eq!(app.popup().unwrap().levels[0].hover, Some(1));
    typed(&mut app, "s");
    assert_eq!(&titles(&app)[1..], ["Rust"]);
    named(&mut app, NamedKey::Enter);
    assert!(app.overlays.context_menu.is_none());
    assert_eq!(
        app.editor.active().unwrap().highlighter.language().name,
        "Rust"
    );

    // 没有匹配：一行说明，不能选中；⌫ 删字后恢复匹配。
    app.open_status_menu(StatusItem::Language);
    paint(&mut app);
    typed(&mut app, "zzq");
    assert_eq!(titles(&app), ["[zzq]", "无匹配项"]);
    assert_eq!(app.popup().unwrap().levels[0].hover, None);
    named(&mut app, NamedKey::Backspace);
    assert_eq!(app.popup().unwrap().query, "zz");
    // 按住 ⌘/Ctrl 的按键是快捷键，不写进筛选文字。
    app.pointer.modifiers = if cfg!(target_os = "macos") {
        winit::keyboard::ModifiersState::SUPER
    } else {
        winit::keyboard::ModifiersState::CONTROL
    };
    typed(&mut app, "c");
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(app.popup().unwrap().query, "zz");
    // Esc：先清空筛选，列出全部；再按一次关掉菜单。
    // 刚才选过的 Rust 移到最近选择的语言中：多出一条分隔线。
    named(&mut app, NamedKey::Escape);
    assert_eq!(app.popup().unwrap().levels[0].rows.len(), all + 1);
    named(&mut app, NamedKey::Escape);
    assert!(app.overlays.context_menu.is_none());

    // 筛选时空格是文字：「Visual Basic」这类名字可以继续键入。
    app.open_status_menu(StatusItem::Language);
    paint(&mut app);
    typed(&mut app, "a b");
    assert_eq!(app.popup().unwrap().query, "a b");
    named(&mut app, NamedKey::Escape);
    named(&mut app, NamedKey::Escape);

    // 编码菜单有子菜单：键入的文字不筛选。
    app.open_status_menu(StatusItem::Encoding);
    paint(&mut app);
    typed(&mut app, "u");
    assert!(app.popup().unwrap().query.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：键入「ru」筛选语言菜单（`popup-filter-light`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_popup_filter() {
    use crate::app::status::status_bar::StatusItem;
    use winit::keyboard::Key;
    let dir = sample_folder("popup-filter-shot");
    let mut app = app_with(&dir.join("a.txt"), false);
    paint(&mut app);
    app.open_status_menu(StatusItem::Language);
    paint(&mut app);
    for c in ["r", "u"] {
        app.popup_logical_key(&Key::Character(c.into()));
    }
    shoot(&mut app, "popup-filter-light");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 命令「更改文件的语言」在面板中列出语言，键入文字筛选：查询恰好是扩展名的语言排在最前，名字
/// 中分散出现的也能找到；没有匹配时列表为空。
#[test]
fn the_language_picker_filters_by_name_and_extension() {
    use crate::app::LanguageRow;
    let dir = sample_folder("language-picker");
    let mut app = app_with(&dir.join("a.txt"), false);
    paint(&mut app);
    app.run_action(crate::keymap::Action::ChangeLanguageMode);
    let first = |app: &mut App, query: &str| {
        app.overlays.picker.input.set_text(query);
        app.picker_refresh();
        (app.overlays.picker.hits.first())
            .and_then(|&h| LanguageRow::of(h))
            .map(LanguageRow::title)
    };
    assert_eq!(first(&mut app, "ts"), Some("TypeScript"));
    assert_eq!(first(&mut app, "md"), Some("Markdown"));
    assert_eq!(first(&mut app, "rust"), Some("Rust"));
    assert!(first(&mut app, "rs").is_some());
    assert_eq!(first(&mut app, "zzqq"), None);
    // 命中的字标出：「rust」标出 Rust 的四个字。
    first(&mut app, "rust");
    assert_eq!(app.language_matched(0), [0, 1, 2, 3]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：面板中的语言列表，键入「ru」筛选（`language-picker-light`）；状态栏的语言菜单
/// （`language-menu-light`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_language_picker() {
    let dir = sample_folder("language-picker-shot");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    app.run_action(crate::keymap::Action::ChangeLanguageMode);
    paint(&mut app);
    shoot(&mut app, "language-picker-open");
    app.overlays.picker.input.set_text("ru");
    app.picker_refresh();
    shoot(&mut app, "language-picker-light");
    app.close_picker();
    app.open_status_menu(StatusItem::Language);
    paint(&mut app);
    shoot(&mut app, "language-menu-light");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：窗口内绘制的右键菜单（文件树的一行上），鼠标停在第四项（`popup-menu-light`、
/// `popup-menu-dark`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_popup_menu() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let dir = sample_folder(&format!("popup-menu-shot-{theme}"));
        let mut app = app_with(&dir.join("short.rs"), dark);
        paint(&mut app);
        let at = tree_row_center(&app, 1);
        right_click(&mut app, at);
        paint(&mut app);
        let row = app.popup().unwrap().levels[0].row_rects[3];
        app.on_mouse_move(row.x + 40.0, row.y + row.h / 2.0);
        shoot(&mut app, &format!("popup-menu-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn alerts_without_a_system_dialog_are_drawn_in_the_window() {
    use crate::app::{AlertButton, AlertKey, AlertPart, UserEvent};
    use winit::keyboard::{Key, NamedKey};

    let dir = sample_folder("alert");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let ask = |app: &mut App| {
        let buttons = vec![
            AlertButton::new("打开设置"),
            AlertButton::cancel("取消"),
            AlertButton {
                key: Some(AlertKey::Command('d')),
                ..AlertButton::new("不打开")
            },
        ];
        let asked = app.show_alert(
            "要打开设置吗？",
            "说明",
            buttons,
            Some(("不再询问", false)),
            |i, _| (i == 0).then_some(UserEvent::OpenSettings),
        );
        assert!(asked);
        paint(app);
    };

    // 没有系统提示框：画在窗口里；第一个按钮在最右边，其余依次向左。
    ask(&mut app);
    let parts = app.overlays.alert.as_ref().unwrap().parts.clone();
    assert_eq!(parts.buttons.len(), 3);
    assert!(parts.buttons[0].x > parts.buttons[1].x && parts.buttons[1].x > parts.buttons[2].x);
    // 已经有提示框时不再弹第二个。
    assert!(!app.show_alert(
        "另一个",
        "",
        vec![AlertButton::new("好")],
        None,
        |_, _| None
    ));

    // 打开期间别的按键不处理；勾选框可以点，点按钮关掉提示框并处理它的事件。
    let center = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (x, y) = center(parts.checkbox.unwrap());
    app.on_mouse_move(x, y);
    assert_eq!(
        app.overlays.alert.as_ref().unwrap().hover,
        Some(AlertPart::Checkbox)
    );
    app.on_mouse_down(MouseButton::Left);
    assert!(app.overlays.alert.as_ref().unwrap().checked);
    let (x, y) = center(parts.buttons[0]);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.overlays.alert.is_none());
    assert!(app.settings.open);
    app.close_settings();

    // ↩ 选默认按钮，Esc 选「取消」，⌘D（其他平台 Ctrl+D）选绑了 D 的按钮。
    ask(&mut app);
    assert!(app.alert_key(&Key::Named(NamedKey::Escape), false));
    assert!(app.overlays.alert.is_none() && !app.settings.open);
    ask(&mut app);
    assert!(app.alert_key(&Key::Character("d".into()), false));
    assert!(app.overlays.alert.is_some(), "没按着 ⌘ 时 D 不算");
    assert!(app.alert_key(&Key::Character("d".into()), true));
    assert!(app.overlays.alert.is_none() && !app.settings.open);
    ask(&mut app);
    assert!(app.alert_key(&Key::Named(NamedKey::Enter), false));
    assert!(app.settings.open);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：窗口内绘制的提示框（关闭有未保存修改的标签时那样的三个按钮和「不再询问」）
/// （`alert-light`、`alert-dark`），以及只有一句的关闭前确认（`alert-short-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_alert() {
    use crate::app::{AlertButton, AlertKey};
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let dir = sample_folder(&format!("alert-shot-{theme}"));
        let mut app = app_with(&dir.join("short.rs"), dark);
        let buttons = vec![
            AlertButton::new("保存"),
            AlertButton::cancel("取消"),
            AlertButton {
                key: Some(AlertKey::Command('d')),
                ..AlertButton::destructive("不保存")
            },
        ];
        app.show_alert(
            "要保存对「short.rs」的修改吗？",
            "如果不保存，你的修改将丢失。",
            buttons,
            Some(("不再询问", false)),
            |_, _| None,
        );
        paint(&mut app);
        let button = app.overlays.alert.as_ref().unwrap().parts.buttons[1];
        app.on_mouse_move(button.x + 10.0, button.y + 10.0);
        shoot(&mut app, &format!("alert-{theme}"));

        // 只有一句的提示框按内容收窄。
        let mut app = app_with(&dir.join("short.rs"), dark);
        app.config.confirm_close = Some(true);
        app.ask_quit();
        paint(&mut app);
        shoot(&mut app, &format!("alert-short-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 二进制文件「以文本打开」以后，编码菜单里「通过编码重新打开」的各项可用，选了 UTF-16 LE 就按它
/// 显示，不再只读。
#[test]
fn a_binary_file_can_be_reopened_with_an_encoding() {
    use crate::app::PlaceholderAction;
    use crate::app::status::status_bar::StatusItem;
    use winc_editor::Encoding;
    use winc_editor::encoding::CHOICES;
    let dir = sample_folder("reopen-binary-menu");
    let path = dir.join("wide.txt");
    let wide: Vec<u8> = "中文 ok\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(&path, wide).unwrap();
    let mut app = app_with(&path, false);
    app.run_placeholder_action(PlaceholderAction::OpenAsText);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.encoding, Encoding::Binary);
    let id = doc.id;

    paint(&mut app);
    app.open_status_menu(StatusItem::Encoding);
    let reopen = (menu_commands(&app).into_iter())
        .filter(|(command, _)| matches!(command, MenuCommand::ReopenEncoding(_)));
    assert!(reopen.into_iter().all(|(_, enabled)| enabled));
    app.overlays.context_menu = None;

    let le = (CHOICES.iter())
        .position(|(e, _)| *e == Encoding::Utf16Le)
        .unwrap();
    app.run_status_command(MenuCommand::ReopenEncoding(le), id);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.encoding, Encoding::Utf16Le);
    assert_eq!(doc.text().rope().unwrap().to_string(), "中文 ok\n");
    assert!(doc.readonly_reason().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `.properties` 打开后按 Unicode 转义显示字符，编码菜单中列出「Unicode 转义」并勾选，小写的转义
/// 同样处理；其他文件不列出。
#[test]
fn the_encoding_menu_offers_unicode_escapes_for_properties() {
    use crate::app::status::status_bar::StatusItem;
    use winc_editor::Encoding;
    use winc_editor::encoding::CHOICES;
    let dir = sample_folder("unicode-escapes-menu");
    let escapes = (CHOICES.iter())
        .position(|(e, _)| matches!(e, Encoding::UnicodeEscapes { .. }))
        .unwrap();
    let listed = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let mut app = app_with(&path, false);
        paint(&mut app);
        let encoding = app.editor.active().unwrap().encoding;
        app.open_status_menu(StatusItem::Encoding);
        let command = MenuCommand::ReopenEncoding(escapes);
        let listed = (menu_commands(&app).into_iter()).any(|(c, _)| c == command);
        (encoding, listed, listed && app.menu_checked(command))
    };
    for (lowercase, text) in [(false, "hi=\\u4F60\n"), (true, "hi=\\u4f60\n")] {
        let latin1 = false;
        assert_eq!(
            listed("messages.properties", text),
            (Encoding::UnicodeEscapes { lowercase, latin1 }, true, true)
        );
    }
    assert_eq!(
        listed("notes.txt", "hi=\\u4F60\n"),
        (Encoding::Utf8, false, false)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
