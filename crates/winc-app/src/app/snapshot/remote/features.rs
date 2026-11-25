//! 远端工作区中不经文件树的各项功能的测试：快速打开、全局搜索、跳转定义、查找引用、复制路径。

use std::os::unix::ffi::OsStrExt;

use super::*;

/// 远端工作区中不经文件树的各项功能：快速打开、全局搜索、跳转定义、查找引用、复制路径、
/// 外部修改后的重读和打开不存在的文件。由
/// [`super::a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`super::folders_on_a_remote_host_are_not_taken_for_files`]。
pub(super) fn features_work_in_a_remote_workspace() {
    let dir = search_folder("remote-features").canonicalize().unwrap();
    for file in ["nest/b/c/file.rs", "nest/d/e/other.rs"] {
        std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
        std::fs::write(dir.join(file), "").unwrap();
    }
    let target = loopback("app-remote-features");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);

    // 快速打开：文件表在后台线程上经连接列出。测试中没有事件循环，这里显式在后台建立。
    app.overlays.picker.index = winc_editor::FileIndex::spawn(&root, || {});
    app.open_picker();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.overlays.picker.index.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.picker_indexed();
    }
    app.overlays.picker.input.set_text("cfg");
    app.picker_refresh();
    let picker = &app.overlays.picker;
    let first = picker.index.file(picker.hits[0]).unwrap().path.to_owned();
    assert!(first.ends_with("config.rs"), "{first}");
    app.close_picker();

    // 外部新建的文件不必打开就能找到；已删除的文件从最近打开的文件中去掉，不再列出。
    std::fs::write(dir.join("src/fresh.rs"), "").unwrap();
    let (fresh, stale) = (root.join("src/fresh.rs"), root.join("src/stale.rs"));
    app.history.files.insert(0, stale.clone());
    app.picker_touched(vec![fresh, stale.clone()]);
    while !app.overlays.picker.index.is_ready() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.picker_indexed();
    assert!(!app.history.files.contains(&stale));
    app.open_picker();
    app.overlays.picker.input.set_text("fresh");
    app.picker_refresh();
    let picker = &app.overlays.picker;
    assert_eq!(
        picker.index.file(picker.hits[0]).unwrap().path,
        "src/fresh.rs"
    );
    app.close_picker();

    // 按住 Alt 单击目录：各级目录由那台主机一次找出，一并展开；再按一次全部收起。
    let names = |app: &App| -> Vec<String> {
        let rows = app.editor.workspace.as_ref().unwrap().rows();
        rows.iter().map(|row| row.name.clone()).collect()
    };
    app.toggle_dir_recursively(row_of(&app, "nest"));
    let shown = names(&app);
    assert!(shown.iter().any(|name| name == "file.rs"), "{shown:?}");
    assert!(shown.iter().any(|name| name == "other.rs"), "{shown:?}");
    app.toggle_dir_recursively(row_of(&app, "nest"));
    assert!(!names(&app).iter().any(|name| name == "file.rs"));

    // 全局搜索在那台主机上的文件中进行。
    app.set_sidebar_view(SidebarView::Search);
    app.panels.global.query.insert("load_config");
    app.run_global_search();
    finish_search(&mut app);
    assert_eq!(app.panels.global.engine.error(), None);
    assert_eq!(app.panels.global.engine.total(), 2);

    // 跳转定义：定义在另一个文件中时打开它；查找引用是一次全字的全局搜索。
    app.open_path(&root.join("src/main.rs"));
    let text = app
        .editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap();
    let at = text[..text.find("load_config").unwrap()].chars().count() + 1;
    app.select(at, at);
    app.go_to_definition();
    assert_eq!(app.editor.active().unwrap().name(), "config.rs");
    app.find_references();
    finish_search(&mut app);
    assert_eq!(app.panels.global.engine.total(), 2);

    // 复制路径给出那台主机上的写法，可以直接用在那里的终端中。
    app.copy_path(&root.join("src/main.rs"), false);
    let copied = dir.join("src/main.rs").to_string_lossy().into_owned();
    assert_eq!(app.clipboard_text(), Some(copied));
    app.copy_path(&root.join("src/main.rs"), true);
    assert_eq!(app.clipboard_text().as_deref(), Some("src/main.rs"));

    // 代码引用同样给出那台主机上的路径：绝对路径不带连接目标，相对路径不变，多行带行号范围。
    let active = app.editor.active().unwrap().path.clone().unwrap();
    assert_eq!(active, root.join("src/config.rs"));
    let remote = dir.join("src/config.rs").to_string_lossy().into_owned();
    app.select(0, 0);
    app.copy_reference(true);
    assert_eq!(app.clipboard_text(), Some(format!("{remote}:1")));
    app.copy_reference(false);
    assert_eq!(app.clipboard_text().as_deref(), Some("src/config.rs:1"));
    let second = app.editor.active().unwrap().text().line_start(1);
    app.select(0, second + 1);
    app.copy_reference(true);
    assert_eq!(app.clipboard_text(), Some(format!("{remote}:1-2")));
    // 文件不在打开的目录内时，相对的引用退回完整路径，同样是那台主机上的写法。
    let inside = app.editor.workspace.take();
    app.copy_reference(false);
    assert_eq!(app.clipboard_text(), Some(format!("{remote}:1-2")));
    app.editor.workspace = inside;

    // 其他程序修改了已打开的文件：窗口重新获得焦点时重读。
    app.open_path(&root.join("src/config.rs"));
    std::fs::write(dir.join("src/config.rs"), "pub struct Changed;\n").unwrap();
    app.check_disk();
    let now = app
        .editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap();
    assert!(now.contains("Changed"), "{now}");

    // 不存在的文件：标签中说明原因，不是一个可以编辑的空文档。
    app.open_path(&root.join("src/missing.rs"));
    let missing = app.editor.active().unwrap();
    assert!(
        matches!(
            missing.unsupported,
            Some(winc_editor::Unsupported::Unreadable(_))
        ),
        "{:?}",
        missing.unsupported
    );

    remote::disconnect(&target);
    let _ = std::fs::remove_dir_all(&dir);
}
