//! 远端目录与项目文件的测试：在同一台主机上添加目录，另存为项目。

use std::os::unix::ffi::OsStrExt;

use super::*;

/// 远端目录的窗口添加同一台主机上的目录：从打开的目录的上级开始浏览，第一行是「添加此目录」。
/// 另存为项目后，项目文件按主机记下这些目录。由
/// [`super::a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`super::folders_on_a_remote_host_are_not_taken_for_files`]。
pub(super) fn remote_folders_are_added_and_saved_as_a_project() {
    let dir = sample_folder("remote-project-main").canonicalize().unwrap();
    let other = sample_folder("remote-project-other")
        .canonicalize()
        .unwrap();
    let target = loopback("app-remote-project");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let sibling = winc_backend::virtual_path(&target, other.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);
    assert!(app.can_add_folder());

    app.ask_add_folder();
    let parent = dir.parent().unwrap().to_string_lossy().into_owned();
    let first = &app.overlays.picker.remote.as_ref().unwrap().rows[0];
    assert_eq!(
        (first.title.as_str(), first.detail.as_str()),
        ("添加此目录", parent.as_str())
    );
    app.overlays.picker.input.set_text("remote-project-other/");
    app.picker_refresh();
    app.overlays.picker.selected = 0;
    app.remote_picker_accept();
    assert_eq!(app.extra_folders(), std::slice::from_ref(&sibling));

    let local = sample_folder("remote-project-file").canonicalize().unwrap();
    app.save_project_as(local.join("ops"));
    let file = local.join("ops.winc-project");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("{ \"host\": \"app-remote-project\", \"path\": "),
        "{text}"
    );
    let saved: Vec<_> = (winc_config::read_project(&file).unwrap().into_iter())
        .map(|folder| folder.path)
        .collect();
    assert_eq!(saved, [root, sibling]);
    for path in [&dir, &other, &local] {
        let _ = std::fs::remove_dir_all(path);
    }
}
