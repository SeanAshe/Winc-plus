//! 远端目录与项目文件的测试：在同一台主机上添加目录，另存为项目，打开远端主机上的项目文件。

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

/// 远端主机上的项目文件从面板、`ssh://` 参数、「打开项目」提示和新窗口打开，沿用已有的连接；
/// 不写主机的目录在同一台主机上，相对路径按项目文件所在的目录展开。由
/// [`super::a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`super::folders_on_a_remote_host_are_not_taken_for_files`]。
pub(super) fn remote_project_files_open_over_the_connection() {
    let dir = sample_folder("remote-workspace").canonicalize().unwrap();
    let other = sample_folder("remote-workspace-other")
        .canonicalize()
        .unwrap();
    std::fs::create_dir_all(dir.join("project1")).unwrap();
    let text = format!(
        "{{ \"folders\": [{{ \"path\": \"./project1\" }}, {{ \"path\": \"{}\" }}] }}",
        other.display()
    );
    std::fs::write(dir.join("dev.code-workspace"), text).unwrap();
    std::fs::write(dir.join("app.ipr"), "<project />").unwrap();
    let target = loopback("app-remote-workspace");
    let on_host = |path: &Path| winc_backend::virtual_path(&target, path.as_os_str().as_bytes());
    let (root, extra) = (on_host(&dir.join("project1")), on_host(&other));
    let file = on_host(&dir.join("dev.code-workspace"));
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    remote::forbid_blocking();

    // 面板列出项目文件，不列出其他文件和远端不打开的 IDEA 项目；选中后打开项目。
    let home = dir.to_string_lossy().into_owned();
    app.browse_remote_folder(target.clone(), home.clone(), home);
    let shown = titles(&app);
    assert!(
        shown.contains(&"dev.code-workspace".to_owned()),
        "{shown:?}"
    );
    assert!(
        !shown
            .iter()
            .any(|title| title == "app.ipr" || title == "a.txt")
    );
    typed(&mut app, "dev");
    assert_eq!(titles(&app), ["dev.code-workspace"]);
    app.overlays.picker.selected = 0;
    app.remote_picker_accept();
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert_eq!(app.extra_folders(), std::slice::from_ref(&extra));

    // 向项目添加目录时不列出项目文件。
    app.ask_add_folder();
    app.overlays.picker.input.set_text("../");
    app.picker_refresh();
    assert!(
        !titles(&app)
            .iter()
            .any(|title| title.ends_with(".code-workspace"))
    );
    app.close_picker();

    // 「打开项目」提示同样在后台读取；不是项目文件的路径说明原因，连接保留。
    app.open_project_file(&file);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&extra));
    let url = |path: &Path| {
        let path = path.to_str().unwrap().replace(' ', "%20");
        std::ffi::OsString::from(format!("ssh://app-remote-workspace{path}"))
    };
    let parsed = crate::cli::parse([url(&dir.join("a.txt"))]).unwrap();
    app.open_remote(crate::cli::remote_open(&parsed.targets).unwrap());
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("「a.txt」不是目录或项目文件"), "{notice}");
    assert!(remote::is_connected(&target));
    assert_eq!(app.editor.root(), Some(root.as_path()));

    // `ssh://` 参数给出项目文件时打开项目。
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    let parsed = crate::cli::parse([url(&dir.join("dev.code-workspace"))]).unwrap();
    app.open_remote(crate::cli::remote_open(&parsed.targets).unwrap());
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert_eq!(app.extra_folders(), std::slice::from_ref(&extra));

    // 在新窗口中打开：新窗口取各窗口共用的、已读好的项目。
    let mut winc = crate::winc::Winc::new(None, None);
    let (first, second) = (winc.add_detached(app), winc.add_detached(blank_app(false)));
    winc.app(first).open_project_file(&file);
    winc.app(second).open_folder_now(&file);
    assert_eq!(winc.app(second).editor.root(), Some(root.as_path()));
    assert_eq!(winc.app(second).extra_folders(), [extra]);
    remote::disconnect(&target);
    for path in [&dir, &other] {
        let _ = std::fs::remove_dir_all(path);
    }
}
