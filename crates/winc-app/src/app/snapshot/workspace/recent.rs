//! 最近打开的目录（`⌃R`）的测试：列出与筛选、再按一次往下选、打开、移除、已不存在的目录，
//! 以及按住 `⌥` 在新窗口打开。

use super::*;
use crate::keymap::Action;
use crate::winc::{Request, Winc};

/// 返回 `path` 规范化后的路径，与最近列表中记录的形式相同。
fn canonical(path: &Path) -> PathBuf {
    winc_config::files::canonicalize(path).unwrap()
}

/// 返回列出 `rows` 行时面板卡片的高度（逻辑像素）。
fn card_height(rows: usize) -> f32 {
    crate::theme::BAR_HEIGHT + rows as f32 * PICKER_ROW + 2.0 * crate::app::PICKER_PAD
}

#[test]
fn recent_folders_are_listed_filtered_and_opened_from_the_keyboard() {
    let alpha = search_folder("recent-alpha");
    let bravo = sample_folder("recent-bravo");
    let charlie = sample_folder("nest-recent/charlie");
    let mut app = app_with(&alpha, false);
    app.state.recent = vec![canonical(&alpha), canonical(&charlie), canonical(&bravo)];

    // 列出当前目录以外的目录，按最近列表的顺序；第一条是上一次打开的目录，回车即可切回。
    app.run_action(Action::OpenRecent);
    assert!(app.picker.open && app.picker.folders);
    assert_eq!(app.focus, Focus::Picker);
    assert_eq!(app.picker.hits, [1, 2]);
    assert_eq!(app.picker.selected, 0);

    // 列表打开时再按一次：选中下一条。
    app.run_action(Action::OpenRecent);
    assert_eq!(app.picker.selected, 1);

    // 按目录名筛选，命中的字标在目录名上。
    app.picker.input.set_text("bravo");
    app.picker_refresh();
    assert_eq!(app.picker.hits.first(), Some(&2));
    let (in_name, in_parent) = app.folder_matched(0);
    assert_eq!(in_name, [7, 8, 9, 10, 11]);
    assert!(in_parent.is_empty());

    // 也按上级目录筛选；目录名命中的排在上级目录命中的前面，与最近列表中的先后无关。
    app.picker.input.set_text("recent");
    app.picker_refresh();
    assert_eq!(app.picker.hits, [2, 1]);
    let (in_name, in_parent) = app.folder_matched(1);
    assert!(in_name.is_empty() && !in_parent.is_empty());

    // 输入 `>`、`@` 不会换成命令或符号列表。
    app.picker.input.set_text(">");
    app.picker_refresh();
    assert!(app.picker.folders && !app.picker_lists_commands());
    app.picker.input.set_text("@");
    app.picker_refresh();
    assert!(app.picker.folders && !app.picker_lists_symbols());

    // `⌘P` 换成文件列表，`⌃R` 再换回来。
    app.run_action(Action::QuickOpen);
    assert!(app.picker.open && !app.picker.folders);
    app.run_action(Action::OpenRecent);
    assert!(app.picker.folders);
    assert!(app.picker.input.text().is_empty());

    // 回车在此窗口打开选中的目录，面板关闭。
    app.picker_accept();
    assert!(!app.picker.open && !app.picker.folders);
    assert_eq!(app.editor.root(), Some(canonical(&charlie).as_path()));

    let _ = std::fs::remove_dir_all(&alpha);
    let _ = std::fs::remove_dir_all(&bravo);
    let _ = std::fs::remove_dir_all(charlie.parent().unwrap());
}

#[test]
fn recent_folders_can_be_removed_and_opened_in_a_new_window() {
    let bravo = sample_folder("recent-window-bravo");
    let charlie = sample_folder("recent-window-charlie");
    let missing = std::env::temp_dir().join("winc-recent-picker-missing-zzz");
    let mut winc = Winc::new(None, None);
    let key = winc.add_detached(blank_app(false));
    let app = winc.app(key);
    app.state.recent = vec![canonical(&bravo), missing.clone(), canonical(&charlie)];

    // 没有打开目录时同样可用，列出全部。
    app.run_action(Action::OpenRecent);
    assert_eq!(app.picker.hits, [0, 1, 2]);
    assert_eq!(app.layout.picker.h, card_height(3));

    // 悬停在一行上时右端出现「✕」；点击「✕」只从列表中移除该目录，面板保持打开。
    paint(app);
    let row = crate::app::workspace::picker::picker_parts(app, app.layout.picker)
        .row(0, app.picker.scroll);
    app.on_mouse_move(row.x + 60.0, row.y + row.h / 2.0);
    assert_eq!(app.hover, Hover::PickerRow(0));
    paint(app);
    let forget = crate::app::PickerPart::ForgetFolder(0);
    let close = app
        .hits
        .picker_parts
        .iter()
        .find(|(_, p)| *p == forget)
        .unwrap()
        .0;
    app.on_mouse_move(close.x + close.w / 2.0, close.y + close.h / 2.0);
    assert_eq!(app.hover, Hover::PickerPart(forget));
    app.on_mouse_down(MouseButton::Left);
    assert!(app.picker.open && app.picker.folders);
    assert_eq!(app.state.recent, [missing.clone(), canonical(&charlie)]);
    assert_eq!(app.picker.hits, [0, 1]);

    // 打开已不存在的目录：从列表中移除并提示，不打开任何目录。
    app.picker_accept();
    assert!(!app.picker.open);
    assert!(app.editor.root().is_none());
    assert_eq!(app.state.recent, [canonical(&charlie)]);
    assert!(
        app.notice
            .text
            .as_deref()
            .is_some_and(|n| n.contains("已从最近列表中移除"))
    );

    // 按住 `⌥` 回车：请应用层在新窗口打开，此窗口不变。
    app.run_action(Action::OpenRecent);
    app.modifiers = winit::keyboard::ModifiersState::ALT;
    app.picker_accept();
    app.modifiers = winit::keyboard::ModifiersState::empty();
    assert!(app.editor.root().is_none());
    let (requests, _) = winc.settle_for_test();
    assert_eq!(requests, [Request::NewWindow(Some(canonical(&charlie)))]);

    // 列表空了：面板照常打开，留一行显示提示。
    let app = winc.app(key);
    app.state.recent.clear();
    app.run_action(Action::OpenRecent);
    assert!(app.picker.open && app.picker.hits.is_empty());
    assert_eq!(app.layout.picker.h, card_height(1));

    let _ = std::fs::remove_dir_all(&bravo);
    let _ = std::fs::remove_dir_all(&charlie);
}

/// 截图：开着一个目录时按 `⌃R`，列出另外三个最近的目录，第二个目录名命中查询。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_recent_folders() {
    let alpha = search_folder("recent-shot-alpha");
    let others = ["winc-site", "notes", "nest-recent/winc"].map(sample_folder);
    let mut app = app_with(&alpha, false);
    app.state.recent = std::iter::once(&alpha)
        .chain(&others)
        .map(|p| canonical(p.as_path()))
        .collect();
    app.run_action(Action::OpenRecent);
    shoot(&mut app, "recent-folders");
    app.picker.input.set_text("winc");
    app.picker_refresh();
    shoot(&mut app, "recent-folders-filtered");

    let _ = std::fs::remove_dir_all(&alpha);
    let _ = std::fs::remove_dir_all(&others[0]);
    let _ = std::fs::remove_dir_all(&others[1]);
    let _ = std::fs::remove_dir_all(others[2].parent().unwrap());
}
