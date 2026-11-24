//! 远端文件差异的测试：右侧的文档在后台读入，读入失败的文档不作为右侧内容。

use std::os::unix::ffi::OsStrExt;

use winc_editor::diff_view::{DiffSide, DiffView};

use super::*;
use crate::app::DiffTarget;

/// 打开远端文件 `file` 的差异：左侧是 `head`，右侧是磁盘上的内容 `work`，右侧进入编辑。
fn remote_diff(app: &mut App, file: &Path, head: &str, work: &str) {
    let language = winc_syntax::detect("a.c");
    let view = DiffView::new(
        DiffSide::new("HEAD", head, None, language),
        DiffSide::new("Working Tree", work, Some(file.to_path_buf()), language),
    );
    app.editor.show_diff("a.c", view);
    app.editor
        .active_mut()
        .unwrap()
        .diff
        .as_deref_mut()
        .unwrap()
        .editing = true;
}

/// 差异右侧的文字。
fn right_text(app: &App) -> String {
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    view.new.text.to_string()
}

/// 回归：远端文件的差异右侧进入编辑时，界面线程不读取该文件；读入的文档可以编辑，右侧不会
/// 变成空白。未打开的远端文件替换一段改动时，先在后台读入文档，再经文档替换。
///
/// 由 [`super::a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`super::folders_on_a_remote_host_are_not_taken_for_files`]。
pub(super) fn a_remote_diff_keeps_its_working_tree_side() {
    let dir = sample_folder("remote-diff").canonicalize().unwrap();
    let work = "one\ntwo\nthree\n";
    std::fs::write(dir.join("a.c"), work).unwrap();
    let target = loopback("app-remote-diff");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let file = root.join("a.c");
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    remote::forbid_blocking();
    app.open_folder(&root);

    remote_diff(&mut app, &file, "one\n2\nthree\n", work);
    assert!(app.diff_edit_text("x"));
    let source = (app.editor.documents.iter())
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    assert!(source.unsupported.is_none());
    assert_eq!(
        source.text_range(0, usize::MAX).unwrap(),
        "xone\ntwo\nthree\n"
    );
    assert_eq!(right_text(&app), "xone\ntwo\nthree\n");

    // 文件没有打开：替换先读入文档，替换后的内容在文档中，可以撤销，磁盘上的文件不变。
    let source = app
        .editor
        .documents
        .iter()
        .position(|d| d.path.as_deref() == Some(&*file));
    app.editor
        .close_everywhere(app.editor.documents[source.unwrap()].id);
    remote_diff(&mut app, &file, "one\n2\nthree\n", work);
    app.press_diff(DiffTarget::Revert(0));
    let source = (app.editor.documents.iter())
        .find(|d| d.path.as_deref() == Some(file.as_path()))
        .unwrap();
    assert_eq!(source.text_range(0, usize::MAX).unwrap(), "one\n2\nthree\n");
    assert!(source.is_dirty());
    assert_eq!(right_text(&app), "one\n2\nthree\n");
    assert_eq!(std::fs::read_to_string(dir.join("a.c")).unwrap(), work);

    remote::disconnect(&target);
    let _ = std::fs::remove_dir_all(&dir);
}
