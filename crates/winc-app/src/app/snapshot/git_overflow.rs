//! Git 文件名悬停展开与侧栏宽度拖拽的跨平台回归测试。

use super::*;
use crate::app::{CommitName, CommitTarget, Drag};
use winc_editor::git::{Change, Entry, Head, Status};

fn long_change_app(name: &str, flat: bool, dark: bool) -> (App, PathBuf, CommitName, Rect) {
    let dir = sample_folder(name);
    let mut app = app_with(&dir, dark);
    let root = app.editor.root().unwrap().to_path_buf();
    let path =
        root.join("src/a-very-long-file-name-that-extends-over-the-editor-or-terminal.zip.sha256");
    app.git.root = Some(root);
    app.git.status = Some(Status {
        head: Some(Head::Branch("dev".into())),
        entries: vec![Entry {
            path: path.clone(),
            from: None,
            staged: None,
            unstaged: Some(Change::Modified),
            is_dir: false,
        }],
        ..Status::default()
    });
    app.git.commit.set_flat(flat);
    app.git.commit.names.animate_without_window = true;
    app.panels.sidebar.view = SidebarView::Commit;
    paint(&mut app);
    let row = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::File(0))
        .unwrap()
        .0;
    (app, dir, CommitName::File(path), row)
}

fn pixels_beside_row(app: &App, row: Rect) -> Vec<u32> {
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let left = ((app.layout.sidebar.right() + 12.0) * SCALE) as usize;
    let right = (left + 600).min(image.width as usize - 12);
    let top = (row.y * SCALE) as usize;
    let bottom = ((row.bottom() - 1.0) * SCALE) as usize;
    (top..bottom)
        .flat_map(|y| {
            let start = y * image.width as usize;
            image.pixels[start + left..start + right].to_vec()
        })
        .collect()
}

/// 写全名的改动行画在分隔条上：在这一行的高度内按下选中这一行，不拖动分隔条；从编辑区一侧
/// 移来时这一行没有展开，分隔条可以拖动。
#[test]
fn git_overflow_row_covers_the_splitter_on_a_hovered_file_row() {
    for flat in [false, true] {
        let (mut app, dir, _, row) =
            long_change_app(&format!("git-overflow-splitter-{flat}"), flat, false);
        let y = row.y + row.h / 2.0;
        app.on_mouse_move(row.x + 120.0, y);
        paint(&mut app);
        app.git.commit.names.finish();
        app.git.commit.actions_slide = None;
        paint(&mut app);
        let spill = app.git.commit.overflow.and_then(|o| o.spill).unwrap();
        let splitter = app.layout.splitter;
        let x = splitter.x + splitter.w / 2.0;
        app.on_mouse_move(x, y);
        paint(&mut app);
        assert!(matches!(app.pointer.hover, Hover::Commit(_)));
        assert!(app.git.commit.overflow.is_some());
        app.on_mouse_move(spill.x + 40.0, y);
        paint(&mut app);
        assert!(matches!(app.pointer.hover, Hover::Commit(_)));
        app.on_mouse_move(x, y);
        paint(&mut app);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        assert!(!matches!(app.pointer.drag, Drag::Splitter { .. }));
        assert!(app.git.commit.selected.is_some());

        // 从编辑区一侧移来：分隔条可以拖动。
        app.on_mouse_move(spill.right() + 40.0, y);
        paint(&mut app);
        app.on_mouse_move(x, y);
        paint(&mut app);
        assert_eq!(app.pointer.hover, Hover::Splitter);
        // 与上一次按下隔得近会算作双击分隔条（恢复默认宽度）：当作一次新的按下。
        app.pointer.last_click = None;
        app.on_mouse_down(MouseButton::Left);
        assert!(matches!(app.pointer.drag, Drag::Splitter { .. }));
        let before = app.layout.sidebar.w;
        app.on_mouse_move(x + 64.0, y);
        assert!(app.layout.sidebar.w > before + 40.0);
        app.on_mouse_up(MouseButton::Left);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn git_overflow_retraction_leaves_no_filename_over_the_editor_during_resize() {
    for dark in [false, true] {
        for flat in [false, true] {
            let (mut app, dir, key, row) =
                long_change_app(&format!("git-overflow-retract-{dark}-{flat}"), flat, dark);
            let baseline = pixels_beside_row(&app, row);
            let y = row.y + row.h / 2.0;
            app.on_mouse_move(row.x + 120.0, y);
            paint(&mut app);
            app.git.commit.names.finish();
            app.git.commit.actions_slide = None;
            paint(&mut app);
            assert!(
                pixels_beside_row(&app, row) != baseline,
                "必须先显示伸出侧栏的全名"
            );
            // 在另一行开始拖动，让原来那一行进入收回动画，复现偶发的残留。
            let x = app.layout.splitter.x + app.layout.splitter.w / 2.0;
            let away = y + 3.0 * row.h;
            // 切换视图后文件树可能保留全名区域；隐藏的区域不应抢占分隔条。
            app.panels.tree.overflow =
                Some((Rect::new(x - 10.0, away - row.h / 2.0, 80.0, row.h), 0));
            app.on_mouse_move(x + 40.0, away);
            assert!(!matches!(app.pointer.hover, Hover::TreeRow(_)));
            app.on_mouse_move(x, away);
            paint(&mut app);
            assert!(app.git.commit.names.moving());
            assert_eq!(app.pointer.hover, Hover::Splitter);
            app.on_mouse_down(MouseButton::Left);
            assert!(matches!(app.pointer.drag, Drag::Splitter { .. }));
            paint(&mut app);
            assert!(app.git.commit.overflow.is_none());
            assert!(
                pixels_beside_row(&app, row) == baseline,
                "拖拽期间不应在编辑区上残留文件名"
            );
            assert!(!app.git.commit.names.moving());
            let clipped = app.git.commit.names.peek(&key).unwrap().width;
            app.on_mouse_move(x + 48.0, away);
            paint(&mut app);
            assert!(app.git.commit.names.peek(&key).unwrap().width > clipped);
            app.on_mouse_up(MouseButton::Left);
            paint(&mut app);
            assert!(app.git.commit.overflow.is_none());
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
