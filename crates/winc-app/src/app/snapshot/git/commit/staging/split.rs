//! 暂存区视图中分开显示的差异：「更改」中的文件比较暂存区与工作区，「暂存的更改」中的文件比较
//! 上次提交与暂存区；在其中勾上、取消一段即暂存、取消暂存，这一段随即移到另一种差异中。

use super::*;

/// 点提交视图中文件 `index` 的第 `nth` 行（同一个文件在两组各有一行时，0 是靠上的一组）。
fn click_file_row(app: &mut App, index: usize, nth: usize) {
    paint(app);
    let rows: Vec<Rect> = (app.hits.commit.iter())
        .filter(|(_, t)| *t == CommitTarget::File(index))
        .map(|(rect, _)| *rect)
        .collect();
    let row = rows[nth];
    app.pointer.last_click = None;
    click(app, (row.x + 40.0, row.y + row.h / 2.0));
    paint(app);
}

/// 当前差异标签的标题和左右两侧的标题。
fn titles(app: &App) -> (String, String, String) {
    let doc = app.editor.active().unwrap();
    let view = doc.diff.as_deref().unwrap();
    let title = doc.title.clone().unwrap_or_default();
    (title, view.old.title.clone(), view.new.title.clone())
}

#[test]
fn each_group_shows_its_own_diff_and_moves_hunks_between_them() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-split-diffs");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.git.commit.set_staging(true);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    let index = (app.git.status.as_ref().unwrap().entries.iter())
        .position(|e| e.path == file)
        .unwrap();

    // 「更改」：暂存区 ↔ 工作区，两处改动都未暂存。
    click_file_row(&mut app, index, 0);
    let (title, old, new) = titles(&app);
    assert_eq!((title.as_str(), old.as_str()), ("main.rs · 更改", "暂存区"));
    assert!(new.starts_with("工作区"), "{new}");
    assert_eq!(picks(&app), [Pick::Skip, Pick::Skip]);

    // 勾上第一处即暂存：它立即从「更改」中移走，暂存区里只有它。
    app.press_diff(DiffTarget::Include(0));
    assert_eq!(picks(&app), [Pick::Skip]);
    let staged = staged_diff(&root);
    assert!(staged.contains("+let value_5 = 500;"), "{staged}");
    assert!(!staged.contains("-let value_50 = 50;"), "{staged}");

    // 文件现在两组都有：点「暂存的更改」中的一行，比较上次提交与暂存区，右侧只读。
    click_file_row(&mut app, index, 0);
    let (title, old, new) = titles(&app);
    assert_eq!(title, "main.rs · 暂存的更改");
    assert!(old.starts_with("HEAD"), "{old}");
    assert_eq!(new, "暂存区");
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    assert!(view.new.path.is_none(), "暂存区一侧不能编辑");
    assert_eq!(picks(&app), [Pick::Apply]);

    // 取消勾选即取消暂存：暂存区回到上次提交的内容。
    app.press_diff(DiffTarget::Include(0));
    assert!(picks(&app).is_empty());
    assert_eq!(staged_diff(&root), "");

    // 回到「更改」：两处改动又都在这里。
    click_file_row(&mut app, index, 0);
    assert_eq!(titles(&app).0, "main.rs · 更改");
    assert_eq!(picks(&app), [Pick::Skip, Pick::Skip]);
    let _ = std::fs::remove_dir_all(&dir);
}
