//! 远程模块的测试：模块开关、经连接打开目录和文件、失败的提示，以及最近列表中的远端目录。
//!
//! 服务端在本进程内运行，不经过 `ssh`；各项检查放在同一个测试中，因为关闭模块会断开进程内
//! 的全部连接。

use std::os::unix::ffi::OsStrExt;

use winc_backend::remote;

use super::*;
use crate::app::git::form::FormKind;
use crate::keymap::Action;
use winc_pty::protocol::Modes;

/// 在本进程内启动服务端并登记为到 `name` 的连接：该目标的虚拟路径指向本机的同名路径。
fn loopback(name: &str) -> winc_backend::Target {
    let (requests, to_server) = std::io::pipe().unwrap();
    let (replies, from_server) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = remote::serve(requests, from_server);
    });
    let target = winc_backend::Target::new(name).unwrap();
    let connection = remote::Connection::over(replies, to_server).unwrap();
    remote::register(target.clone(), connection);
    target
}

/// 在面板的输入框中键入 `text`。
fn typed(app: &mut App, text: &str) {
    app.edit_field(|field| field.insert(text));
}

/// 连接远端主机的面板中各行的文字。
fn titles(app: &App) -> Vec<String> {
    let list = app.overlays.picker.remote.as_ref().unwrap();
    (app.overlays.picker.hits.iter())
        .map(|&row| list.rows[row].title.clone())
        .collect()
}

#[test]
fn a_folder_on_a_connected_host_opens_as_the_workspace() {
    let dir = sample_folder("remote-open").canonicalize().unwrap();
    std::os::unix::fs::symlink(&dir, dir.with_extension("link")).unwrap();
    let repository = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .is_ok_and(|status| status.success());
    let mut app = blank_app(false);
    app.config.git_enabled = Some(true);
    // 测试线程相当于界面线程：不允许等待远端的应答，后台任务就地执行时除外。
    remote::forbid_blocking();
    folders_on_a_remote_host_are_not_taken_for_files();
    features_work_in_a_remote_workspace();
    clipboard_images_pasted_into_a_remote_terminal_are_sent_there_first();
    ssh_arguments_open_the_project_and_file();

    // 模块默认关闭：状态栏上没有连接的按钮，命令不可用，按下也不打开面板。
    paint(&mut app);
    let listed = |app: &App| {
        (app.hits.status.iter()).any(|(_, item)| *item == crate::app::StatusItem::Remote)
    };
    assert!(!listed(&app));
    assert!(!app.action_available(Action::RemoteConnect));
    assert!(!app.run_action(Action::RemoteConnect));
    assert!(!app.overlays.picker.open);
    app.config.remote_enabled = Some(true);
    assert!(app.run_action(Action::RemoteConnect));
    assert!(app.overlays.picker.open && app.overlays.picker.remote.is_some());
    app.close_picker();

    // 模块开着时，状态栏上终端开关的左边有连接的按钮，点了打开面板。
    paint(&mut app);
    let remote_button = crate::app::StatusItem::Remote;
    assert!(
        app.hits
            .status
            .iter()
            .any(|(_, item)| *item == remote_button)
    );
    app.open_status_menu(remote_button);
    assert!(app.overlays.picker.open && app.overlays.picker.remote.is_some());
    app.close_picker();

    // 开始页上也有连接的入口，打开的是同一个面板。
    paint(&mut app);
    assert!((app.hits.start.iter()).any(|(_, action)| *action == StartAction::Remote));
    app.run_start_action(StartAction::Remote);
    assert!(app.overlays.picker.open && app.overlays.picker.remote.is_some());
    // 不能作为主机名的输入没有可选的行，回车什么也不做。（列出的主机取自用户的 `ssh` 配置，
    // 这里不对它们作断言。）
    typed(&mut app, "-oProxyCommand=touch /tmp/x:/");
    assert!(titles(&app).is_empty() && app.remote_picker_message().is_some());
    app.picker_accept();
    assert!(app.overlays.picker.open && app.notice.text.is_none());
    app.close_picker();
    {
        use crate::app::input::context_menu::{MenuCommand, MenuEntry, project_menu};
        let commands = |hosts| -> Vec<MenuCommand> {
            let flat = |entry: &MenuEntry| match entry {
                MenuEntry::Item(command, _) => vec![*command],
                MenuEntry::Submenu(_, items) => (items.iter())
                    .filter_map(|item| match item {
                        MenuEntry::Item(command, _) => Some(*command),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let menu = project_menu(0, None, false, 0, hosts, false);
            menu.iter().flat_map(flat).collect()
        };
        let base = [MenuCommand::NewWindow, MenuCommand::OpenFolder];
        assert_eq!(commands(None), [base[0], base[1], MenuCommand::CloseFolder]);
        assert_eq!(
            commands(Some(0)),
            [
                base[0],
                base[1],
                MenuCommand::RemoteConnect,
                MenuCommand::CloseFolder
            ]
        );
        assert_eq!(
            commands(Some(2)),
            [
                base[0],
                base[1],
                MenuCommand::RemoteHost(0),
                MenuCommand::RemoteHost(1),
                MenuCommand::RemoteOther,
                MenuCommand::CloseFolder
            ]
        );
    }

    // 已连接的主机列在面板中；选中后从主目录开始浏览。
    let target = loopback("app-remote-open");
    let link = dir.with_extension("link");
    app.open_remote_picker();
    assert_eq!(titles(&app)[..2], ["新主机…", "app-remote-open"]);
    // 新主机：在对话框中填写主机和端口。填写的内容无效时不能连接，原因写在输入框里。
    app.picker_accept();
    let dialog = |app: &App| {
        let dialog = app.overlays.form_dialog.as_ref().unwrap();
        assert_eq!(dialog.kind, FormKind::Connect);
        let plan = app.form_plan(dialog);
        (plan.enabled, plan.problem, dialog.error.clone())
    };
    assert_eq!(dialog(&app), (false, None, None));
    app.edit_field(|field| field.insert("bad host"));
    assert_eq!(dialog(&app), (false, Some("主机名无效".to_owned()), None));
    // 连接失败：对话框留着，原因写在里面，改了可以再连。（测试环境中没有可上传的服务程序。）
    app.open_connect_dialog("winc-test.invalid", "22022", true);
    let (enabled, problem, error) = dialog(&app);
    assert!(enabled && problem.is_none());
    assert!(error.is_some_and(|text| text.contains("缺少远端服务程序")));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    // 取消是上一步：回到主机面板。
    app.close_form_dialog();
    assert!(app.overlays.form_dialog.is_none() && app.overlays.picker.open);
    assert!(app.remote_picker_host().is_none());

    // 选中已连接的主机：经对话框连接，成功后进入选择目录的面板，从主目录开始。
    app.picker_move(true);
    app.picker_accept();
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(app.remote_picker_host(), Some("app-remote-open"));
    // `Esc` 回到上一步（主机面板），再选一次回来。
    assert!(app.remote_picker_back());
    assert!(app.remote_picker_host().is_none() && !app.remote_picker_back());
    app.picker_move(true);
    app.picker_accept();
    assert_eq!(app.remote_picker_host(), Some("app-remote-open"));
    assert_eq!(titles(&app)[0], "打开此目录");
    // 输入路径：到 `/` 为止的部分是要进入的目录，之后的用来筛选。
    typed(&mut app, &format!("{}/", link.display()));
    assert_eq!(app.overlays.picker.input.text(), "");
    assert_eq!(titles(&app), ["打开此目录", "..", "src"]);
    typed(&mut app, "sr");
    assert_eq!(titles(&app), ["src"]);
    typed(&mut app, "x");
    assert!(titles(&app).is_empty());
    assert!(
        app.remote_picker_message()
            .is_some_and(|text| text.contains("无匹配"))
    );
    app.edit_field(|field| field.delete(crate::field::Unit::Char, false));
    // `⇥` 进入选中的目录；输入为空时 `⌫` 回到上级。以 `.` 开头的目录输入 `.` 才列出。
    use winit::keyboard::{Key, NamedKey};
    assert!(app.remote_picker_key(&Key::Named(NamedKey::Tab)));
    assert_eq!(titles(&app), ["打开此目录", ".."]);
    assert!(app.remote_picker_key(&Key::Named(NamedKey::Backspace)));
    assert_eq!(titles(&app), ["打开此目录", "..", "src"]);
    if repository {
        typed(&mut app, ".");
        assert_eq!(titles(&app), [".git"]);
        app.edit_field(|field| field.delete(crate::field::Unit::Char, false));
    }
    // 回车执行选中的一行：第一行是打开此目录。经符号链接走到的目录按远端的真实路径打开。
    app.picker_accept();
    assert!(!app.overlays.picker.open);
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    let names: Vec<_> = (app.editor.workspace.as_ref().unwrap().rows().iter())
        .map(|entry| entry.name.clone())
        .collect();
    assert!(names.iter().any(|name| name == "src"), "{names:?}");
    assert!(names.iter().any(|name| name == "a.txt"), "{names:?}");

    // 仓库由后台查找，状态经连接上的 `git` 读取：尚未提交的文件都是未跟踪的。
    if repository {
        assert_eq!(app.git.root.as_deref(), Some(root.as_path()));
        assert!(app.git.status.is_some(), "没有读到仓库的状态");
    }

    // 其中的文件经连接读取。
    app.open_path(&root.join("src/main.rs"));
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.path.as_deref(),
        Some(root.join("src/main.rs").as_path())
    );
    assert!(doc.text().rope().unwrap().to_string().contains("println!"));
    // 格式化在那台主机上进行：工具按名字在那里查找，内容经标准输入送去；本机的环境不带过去。
    let shown = |app: &App| app.editor.active().unwrap().text_range(0, usize::MAX);
    let before = shown(&app).unwrap();
    let command = "tr a-z A-Z".to_owned();
    app.config
        .format_commands
        .insert("rust".to_owned(), command);
    app.format_active();
    app.wait_formatting();
    assert_eq!(shown(&app).unwrap(), before.to_uppercase());
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    // 那台主机上没有的工具：说明找不到。
    let missing = "winc-no-such-formatter".to_owned();
    app.config
        .format_commands
        .insert("rust".to_owned(), missing);
    app.format_active();
    app.wait_formatting();
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("winc-no-such-formatter"), "{notice}");
    app.config.format_commands.clear();
    assert!(app.run_action(Action::Undo));
    assert_eq!(shown(&app).unwrap(), before);

    // 目录名之后标出主机；打开远端目录时才有「断开连接」。
    let name = &app.editor.workspace.as_ref().unwrap().name;
    assert_eq!(name, "remote-open [SSH: app-remote-open]");
    assert!(app.action_available(Action::RemoteDisconnect));

    // 展开子目录：内容由后台读取后换入。
    let src = row_of(&app, "src");
    app.set_dir_expanded(src, Some(true));
    assert_eq!(row_of(&app, "main.rs"), src + 1);

    // 编辑后保存：内容经连接写到远端。
    let doc = app.editor.active_mut().unwrap();
    doc.apply(winc_core::edit::replace_selection(
        doc.text().rope().unwrap(),
        winc_core::Selection::point(0),
        "// remote\n",
        winc_core::EditKind::Other,
    ));
    assert!(app.run_action(Action::Save));
    let saved = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(saved.starts_with("// remote\n"), "{saved}");
    assert!(!app.editor.active().unwrap().is_dirty());

    // 其他程序在远端新建了文件，窗口重新获得焦点：文件树重读后出现，已展开的目录保持展开。
    std::fs::write(dir.join("src/new.rs"), "").unwrap();
    app.check_disk();
    assert_eq!(row_of(&app, "new.rs"), src + 2);
    assert_eq!(row_of(&app, "main.rs"), src + 1);

    // 文件树中的操作在后台执行，完成后文件树随之更新。新建：
    let new_input = |dir: &Path, name: &str| crate::app::workspace::tree::NameInput {
        dir: dir.to_path_buf(),
        kind: winc_editor::EntryKind::File,
        index: 0,
        depth: 0,
        field: {
            let mut field = crate::app::TextField::default();
            field.set_text(name);
            field
        },
        error: None,
        rename: None,
        contents: None,
    };
    app.panels.name_input = Some(new_input(&root.join("src"), "made.rs"));
    app.submit_name_input();
    assert!(dir.join("src/made.rs").is_file());
    assert!(app.panels.name_input.is_none());
    let made = row_of(&app, "made.rs");
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(made));
    // 同名的再建一次：原因显示在输入行上。
    app.panels.name_input = Some(new_input(&root.join("src"), "made.rs"));
    app.submit_name_input();
    assert!(
        app.panels
            .name_input
            .as_ref()
            .is_some_and(|entry| entry.error.is_some())
    );
    app.close_name_input();

    // 改名：已打开的文档换成新路径。
    let mut rename = new_input(&root.join("src"), "renamed.rs");
    rename.rename = Some((root.join("src/made.rs"), false));
    app.submit_rename(rename);
    assert!(dir.join("src/renamed.rs").is_file() && !dir.join("src/made.rs").exists());
    row_of(&app, "renamed.rs");
    assert!((app.editor.documents.iter()).any(|d| d.path == Some(root.join("src/renamed.rs"))));

    // 移动到根目录，再删除：远端没有废纸篓，直接删除。
    app.move_entries(&[root.join("src/renamed.rs")], &root);
    assert!(dir.join("renamed.rs").is_file() && !dir.join("src/renamed.rs").exists());
    row_of(&app, "renamed.rs");
    std::fs::create_dir_all(dir.join("old/deep")).unwrap();
    std::fs::write(dir.join("old/deep/x"), "").unwrap();
    app.delete_entries(&[root.join("renamed.rs"), root.join("old")], true);
    assert!(!dir.join("renamed.rs").exists() && !dir.join("old").exists());
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);

    // 本机的文件和目录拖进远端的文件树：内容传到那台主机上；远端的文件也能复制回本机。
    let near = sample_folder("remote-near").canonicalize().unwrap();
    std::fs::create_dir_all(near.join("kit/inner")).unwrap();
    std::fs::write(near.join("kit/inner/tool.txt"), "from here").unwrap();
    app.copy_entries_into(vec![near.join("kit")], root.clone());
    assert_eq!(
        std::fs::read(dir.join("kit/inner/tool.txt")).unwrap(),
        b"from here"
    );
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    row_of(&app, "kit");
    let from = root.join("kit/inner/tool.txt");
    let back = remote::allow_blocking(|| {
        winc_editor::copy_entry(&near, &from, &near, &Default::default())
    })
    .unwrap();
    assert_eq!(std::fs::read(back).unwrap(), b"from here");

    // 目录不存在：连接保留，提示找不到，从仍然存在的最近一级上级目录开始选择别的目录；已打开的
    // 目录不变。
    let browsing = |app: &App| match app.overlays.picker.remote.as_ref().map(|list| &list.stage) {
        Some(crate::app::remote::picker::Stage::Browse { dir, .. }) => Some(dir.clone()),
        _ => None,
    };
    let local = dir.display().to_string();
    app.open_remote_picker();
    typed(&mut app, &format!("app-remote-open:{local}/gone/inner"));
    app.picker_accept();
    let notice = app.notice.text.take().unwrap();
    assert!(
        notice.starts_with("未找到「") && notice.contains("/gone/inner"),
        "{notice}"
    );
    assert!(!notice.contains("最近列表"), "{notice}");
    assert_eq!(browsing(&app), Some(local.clone()));
    assert!(remote::is_connected(&target));
    assert_eq!(app.editor.root(), Some(root.as_path()));
    app.close_picker();
    // 相对于主目录的路径同样给出最近一级上级目录。
    app.open_remote_picker();
    typed(&mut app, "app-remote-open:~/winc-no-such-folder/inner");
    app.picker_accept();
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap();
    let home = home.canonicalize().unwrap().display().to_string();
    let notice = app.notice.text.take().unwrap();
    assert!(
        notice.contains(&format!("{home}/winc-no-such-folder/inner")),
        "{notice}"
    );
    assert_eq!(browsing(&app), Some(home));
    app.close_picker();

    // 最近列表中远端的目录经同一流程打开，不在界面线程上判断它是否存在；本机的目录不经此处。
    assert!(app.state.recent.contains(&root));
    assert!(app.reopen_remote(&root));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert!(!app.reopen_remote(&dir));

    // 最近列表中的远端目录已被删除：移出最近列表，连接保留，从它的上级目录开始选择。
    let old = dir.join("old");
    std::fs::create_dir(&old).unwrap();
    let old = winc_backend::virtual_path(&target, old.as_os_str().as_bytes());
    assert!(app.reopen_remote(&old));
    assert_eq!(app.editor.root(), Some(old.as_path()));
    assert!(app.reopen_remote(&root));
    assert!(app.state.recent.contains(&old));
    std::fs::remove_dir(dir.join("old")).unwrap();
    assert!(app.reopen_remote(&old));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("已从最近列表中移除"), "{notice}");
    assert!(!app.state.recent.contains(&old));
    assert_eq!(browsing(&app), Some(local.clone()));
    assert!(remote::is_connected(&target));
    assert_eq!(app.editor.root(), Some(root.as_path()));
    app.close_picker();

    // 连接中断（这里由本程序断开来模拟）：窗口重新获得焦点时提示一次，按钮重新连接。
    remote::disconnect(&target);
    app.check_disk();
    let notice = app.notice.text.clone().unwrap();
    assert!(notice.contains("连接已断开"), "{notice}");
    assert_eq!(
        app.notice.action,
        Some(crate::app::NoticeAction::Reconnect(root.clone()))
    );
    // 目录仍然打开，文件树保持原样。
    assert_eq!(app.editor.root(), Some(root.as_path()));
    row_of(&app, "src");
    app.notice.text = None;
    app.check_disk();
    assert!(app.notice.text.is_none());
    let target = loopback("app-remote-open");
    app.run_notice_action();
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    std::fs::write(dir.join("after.txt"), "").unwrap();
    app.check_disk();
    row_of(&app, "after.txt");

    // 断开连接：关闭目录并断开，不弹出连接中断的提示；之后经最近列表重新连接。
    assert!(app.run_action(Action::RemoteDisconnect));
    assert_eq!(app.editor.root(), None);
    assert!(!remote::is_connected(&target));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    assert!(!app.action_available(Action::RemoteDisconnect));
    let target = loopback("app-remote-open");
    assert!(app.reopen_remote(&root));
    assert_eq!(app.editor.root(), Some(root.as_path()));

    // 关闭模块：连接断开。此后最近列表中的该目录不会被当作已删除的目录移除，而是提示原因。
    app.config.remote_enabled = Some(false);
    app.stop_remote();
    assert!(!remote::is_connected(&target));
    assert!(app.reopen_remote(&root));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("远程模块未启用"), "{notice}");

    // 重新启用后需要重新连接；测试环境中没有可上传的服务程序，在启动 `ssh` 之前失败。
    app.config.remote_enabled = Some(true);
    assert!(app.reopen_remote(&root));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("缺少远端服务程序"), "{notice}");
    assert!(app.state.recent.contains(&root));

    // 主机面板的最后一行通向 `ssh` 配置文件。`⇥` 在连接对话框中修改选中的主机，不开始连接；
    // 取消回到面板。
    app.open_remote_picker();
    assert_eq!(titles(&app).last().unwrap(), "编辑 SSH 配置文件…");
    let host = (titles(&app).iter())
        .position(|title| title == "app-remote-open")
        .unwrap();
    app.overlays.picker.selected = host;
    assert!(app.remote_picker_key(&Key::Named(NamedKey::Tab)));
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.kind, FormKind::Connect);
    assert_eq!(
        (dialog.name.text(), dialog.running),
        ("app-remote-open", None)
    );
    assert!(dialog.error.is_none() && !app.overlays.picker.open);
    // 密钥文件：不存在时不能连接并说明；存在时按主机记下，`ssh` 的命令带上它，再打开对话框
    // 时已经填好。（连接本身因为测试环境中没有服务程序而失败。）
    let key = dir.join("id_test");
    let fill = |app: &mut App, text: &str| {
        let dialog = app.overlays.form_dialog.as_mut().unwrap();
        dialog.third.set_text("");
        dialog.third.insert(text);
    };
    fill(&mut app, "/winc/no/such/key");
    let plan = app.form_plan(app.overlays.form_dialog.as_ref().unwrap());
    assert!(!plan.enabled);
    assert_eq!(plan.problem.as_deref(), Some("未找到密钥文件"));
    // 密钥一行的右端是选文件的按钮；在系统面板中选好的文件填进这一行。
    let layout = app.dialog_layout();
    let choose = layout.choose.unwrap();
    assert_eq!(
        layout.part_at(choose.x + 1.0, choose.y + 1.0),
        Some(crate::layout::DialogPart::Change)
    );
    assert!(layout.third.contains(choose.x, choose.y));
    std::fs::write(&key, "").unwrap();
    app.key_file_picked(&key);
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.third.text(), key.to_str().unwrap());
    assert_eq!(dialog.focus, crate::layout::DialogPart::Third);
    assert!(
        app.form_plan(app.overlays.form_dialog.as_ref().unwrap())
            .enabled
    );
    app.submit_form_dialog();
    assert_eq!(app.state.remote_key("app-remote-open"), key.to_str());
    assert_eq!(remote::identity(&target).as_deref(), key.to_str());
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert!(dialog.error.is_some());
    assert_eq!(dialog.third.text(), key.to_str().unwrap());
    app.close_form_dialog();
    assert!(app.overlays.picker.open);
    // 最近的目录和只来自最近目录的主机在行尾有「✕」，「新主机」和配置文件两行没有。移除主机
    // 时连同其上的目录。
    app.overlays.picker.selected = host;
    paint(&mut app);
    let forget = crate::app::PickerPart::ForgetRemote(host);
    let closes: Vec<_> = app
        .hits
        .picker_parts
        .iter()
        .map(|(_, part)| *part)
        .collect();
    assert_eq!(closes, [forget]);
    app.press_picker_part(forget);
    assert!(!app.state.recent.contains(&root));
    assert!(!titles(&app).iter().any(|title| title == "app-remote-open"));
    assert_eq!(app.state.remote_key("app-remote-open"), None);
    assert_eq!(remote::identity(&target), None);
    app.close_picker();
    local_only_actions_are_not_offered_on_a_remote_host();
    files_copied_in_a_remote_tree_are_pasted_there();
    remote_items_download_to_this_computer();
}

/// 截图：开始页、主机面板、目录面板（浏览与筛选）和打开远端目录后的窗口。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_remote_picker() {
    let dir = sample_folder("remote-shots").canonicalize().unwrap();
    for name in ["docs", "scripts", "tests", ".config"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    // 状态栏：浅色主题下终端开关左边的连接按钮。
    let mut light = blank_app(false);
    light.config.remote_enabled = Some(true);
    light.config.terminal_enabled = Some(true);
    shoot(&mut light, "remote-status-light");

    let mut app = blank_app(true);
    app.config.remote_enabled = Some(true);
    app.config.terminal_enabled = Some(true);
    remote::forbid_blocking();
    shoot(&mut app, "remote-start");

    let target = loopback("devbox");
    app.open_remote_picker();
    shoot(&mut app, "remote-hosts");
    // 新主机的对话框：空的、填写有误的、连接失败的。
    app.picker_accept();
    shoot(&mut app, "remote-dialog");
    app.open_connect_dialog("root@152.53.50.196", "22022", true);
    shoot(&mut app, "remote-dialog-failed");
    app.close_form_dialog();
    app.picker_move(true);
    app.picker_accept();
    typed(&mut app, &format!("{}/", dir.display()));
    shoot(&mut app, "remote-browse");
    typed(&mut app, "s");
    shoot(&mut app, "remote-browse-filter");
    typed(&mut app, "zzz");
    shoot(&mut app, "remote-browse-none");
    for _ in 0..4 {
        app.edit_field(|field| field.delete(crate::field::Unit::Char, false));
    }
    app.picker_accept();
    shoot(&mut app, "remote-workspace");
    // 打开过的远端目录列在主机面板的最前面。
    app.open_remote_picker();
    shoot(&mut app, "remote-hosts-recent");
    // 选中最近的目录（行尾有「✕」）和主机（提示可以修改）。
    app.picker_move(true);
    shoot(&mut app, "remote-hosts-forget");
    app.picker_move(true);
    shoot(&mut app, "remote-hosts-edit");
    app.overlays.picker.selected = 0;
    typed(&mut app, "me@staging:~/app");
    shoot(&mut app, "remote-hosts-typed");
    app.close_picker();
    // 不经对话框的连接：右上角的提示带转动的图标和「取消」。
    app.notify_pinned(
        "正在向 devbox 安装服务程序…",
        crate::app::NoticeAction::CancelConnect,
    );
    shoot(&mut app, "remote-notice");
    remote::disconnect(&target);
}

#[test]
fn a_long_copy_shows_its_progress_and_can_be_cancelled() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::app::NoticeAction;

    let dir = sample_folder("copy-progress").canonicalize().unwrap();
    std::fs::write(dir.join("big.bin"), vec![0u8; 3 << 20]).unwrap();
    let mut app = blank_app(false);
    let copying = Arc::new(winc_editor::Copying::default());
    winc_editor::copy_entry(&dir, &dir.join("big.bin"), &dir, &copying).unwrap();
    app.panels.tree.copies.track(Arc::clone(&copying), false);

    // 右上角正显示着别的提示：不替换它，等它消失之后再显示进度。
    app.notify(crate::app::NoticeKind::Info, "别的提示");
    let now = Instant::now() + Duration::from_secs(1);
    assert!(app.copy_tick(now).is_some_and(|next| next > now));
    assert_eq!(app.notice.text.as_deref(), Some("别的提示"));
    app.dismiss_notice();
    let now = now + Duration::from_secs(2);
    app.copy_tick(now);
    assert_eq!(app.notice.action, Some(NoticeAction::CancelCopy));
    assert_eq!(
        app.notice.text.as_deref(),
        Some("正在复制「big.bin」… 3.0 MB / 3.0 MB")
    );
    shoot(&mut app, "copy-progress");

    // 「取消」：复制收到取消，提示收起，不再显示进度；复制结束时说明已取消。
    app.run_notice_action();
    assert!(copying.is_cancelled() && app.notice.text.is_none());
    assert!(app.copy_tick(now + Duration::from_secs(5)).is_none());
    assert!(app.copy_finished(&copying));
    assert_eq!(app.status.message.as_deref(), Some("已取消复制"));

    // 没有被取消的复制结束时收起它的进度。
    let copying = Arc::new(winc_editor::Copying::default());
    app.panels.tree.copies.track(Arc::clone(&copying), false);
    app.clear_flash();
    app.copy_tick(now + Duration::from_secs(9));
    assert_eq!(app.notice.action, Some(NoticeAction::CancelCopy));
    assert!(!app.copy_finished(&copying));
    assert!(app.notice.text.is_none() && app.status.message.is_none());

    // 下载的进度和取消说下载。
    let copying = Arc::new(winc_editor::Copying::default());
    winc_editor::copy_entry(&dir, &dir.join("big.bin"), &dir, &copying).unwrap();
    app.panels.tree.copies.track(Arc::clone(&copying), true);
    app.copy_tick(now + Duration::from_secs(20));
    assert_eq!(
        app.notice.text.as_deref(),
        Some("正在下载「big.bin」… 3.0 MB / 3.0 MB")
    );
    app.run_notice_action();
    assert!(app.copy_finished(&copying));
    assert_eq!(app.status.message.as_deref(), Some("已取消下载"));
}

/// 写入的内容延迟 `delay` 之后才到达 `inner`，各次写入互不等待：模拟网络的延迟。
struct Delayed(std::sync::mpsc::Sender<(std::time::Instant, Vec<u8>)>);

impl Delayed {
    fn new(mut inner: impl std::io::Write + Send + 'static, delay: std::time::Duration) -> Self {
        let (sender, queue) = std::sync::mpsc::channel::<(std::time::Instant, Vec<u8>)>();
        std::thread::spawn(move || {
            for (sent, data) in queue {
                let due = sent + delay;
                std::thread::sleep(due.saturating_duration_since(std::time::Instant::now()));
                if inner.write_all(&data).and_then(|()| inner.flush()).is_err() {
                    return;
                }
            }
        });
        Self(sender)
    }
}

impl std::io::Write for Delayed {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let sent = self.0.send((std::time::Instant::now(), data.to_vec()));
        sent.map(|()| data.len())
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 往返延迟为 `delay` 的进程内连接，其余同 [`loopback`]。
fn slow_loopback(name: &str, delay: std::time::Duration) -> winc_backend::Target {
    let (requests, to_server) = std::io::pipe().unwrap();
    let (replies, from_server) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = remote::serve(requests, from_server);
    });
    let target = winc_backend::Target::new(name).unwrap();
    let delayed = Delayed::new(to_server, delay);
    let connection = remote::Connection::over(replies, delayed).unwrap();
    remote::register(target.clone(), connection);
    target
}

/// 测量：各项常用操作各发出多少个请求，以及在 250 毫秒的往返延迟下各用多久。
#[test]
#[ignore = "测量，手动运行"]
fn measure_remote_round_trips() {
    let dir = sample_folder("remote-trips").canonicalize().unwrap();
    std::fs::create_dir_all(dir.join("src/deep/er")).unwrap();
    std::fs::write(dir.join("src/deep/er/x.rs"), "fn x() {}\n").unwrap();
    let git = |args: &[&str]| {
        let run = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output();
        run.is_ok_and(|output| output.status.success())
    };
    let _ = git(&["init", "-q"])
        && git(&["add", "."])
        && git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ]);
    let target = slow_loopback("trips", std::time::Duration::from_millis(250));
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.config.git_enabled = Some(true);
    remote::forbid_blocking();
    let mut last = remote::requests(&target);
    let mut clock = std::time::Instant::now();
    let mut step = |what: &str| {
        let now = remote::requests(&target);
        let spent = clock.elapsed().as_secs_f32();
        eprintln!("TRIPS {:>3} 个请求 {spent:>5.2} 秒  {what}", now - last);
        (last, clock) = (now, std::time::Instant::now());
    };
    app.open_folder(&root);
    step("打开目录（含仓库查找与状态）");
    let src = row_of(&app, "src");
    app.set_dir_expanded(src, Some(true));
    step("展开一个目录");
    app.open_path(&root.join("src/main.rs"));
    step("打开一个文件（两层目录）");
    app.open_path(&root.join("src/deep/er/x.rs"));
    step("打开一个文件（四层目录）");
    let doc = app.editor.active_mut().unwrap();
    doc.apply(winc_core::edit::replace_selection(
        doc.text().rope().unwrap(),
        winc_core::Selection::point(0),
        "// x\n",
        winc_core::EditKind::Other,
    ));
    assert!(app.run_action(Action::Save));
    step("保存");
    app.check_disk();
    step("窗口重新获得焦点");
    remote::disconnect(&target);
}

/// 命令行上的 `ssh://` 参数：模块关闭时提示原因；开启时连接、打开项目目录，再打开文件并跳到
/// 指定位置。目录已经开着时不再连接，直接打开文件。
///
/// 由 [`a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`folders_on_a_remote_host_are_not_taken_for_files`]。
fn ssh_arguments_open_the_project_and_file() {
    let dir = search_folder("remote-cli").canonicalize().unwrap();
    let target = loopback("app-remote-cli");
    let url = |path: &Path, suffix: &str| {
        let path = path.to_str().unwrap().replace(' ', "%20");
        std::ffi::OsString::from(format!("ssh://app-remote-cli{path}{suffix}"))
    };
    let main = dir.join("src/main.rs");
    let parsed = crate::cli::parse([url(&dir, ""), url(&main, ":2:5")]).unwrap();
    let open = crate::cli::remote_open(&parsed.targets).unwrap();
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());

    let mut app = blank_app(false);
    app.open_remote(open.clone());
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("远程模块未启用"), "{notice}");
    assert!(app.editor.root().is_none());

    app.config.remote_enabled = Some(true);
    remote::forbid_blocking();
    app.open_remote(open);
    assert_eq!(app.editor.root(), Some(root.as_path()));
    let caret = |app: &App| {
        let doc = app.editor.active().unwrap();
        let head = doc.selection.head;
        (doc.path.clone().unwrap(), doc.text().pos_to_line_col(head))
    };
    assert_eq!(caret(&app), (root.join("src/main.rs"), (1, 4)));

    let config = dir.join("src/config.rs");
    let parsed = crate::cli::parse([url(&dir, ""), url(&config, ":3")]).unwrap();
    app.open_remote(crate::cli::remote_open(&parsed.targets).unwrap());
    assert_eq!(caret(&app), (root.join("src/config.rs"), (2, 0)));
    remote::disconnect(&target);
}

/// 回归：界面线程上查询远端路径的类别一律得到「不是目录」，据此取上一级时，「在终端中打开」
/// 和「在目录中查找」会作用到远端目录的上一级。
///
/// 由 [`a_folder_on_a_connected_host_opens_as_the_workspace`] 调用：已连接的主机是进程内共有的
/// 状态，另起一个测试会与它列出主机的检查相互干扰。
fn folders_on_a_remote_host_are_not_taken_for_files() {
    use crate::app::input::context_menu::MenuCommand;
    let dir = search_folder("remote-folders").canonicalize().unwrap();
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    std::fs::write(dir.join("src/deep/a.rs"), "fn a() {}\n").unwrap();
    std::fs::create_dir_all(dir.join(".venv")).unwrap();
    std::fs::write(dir.join(".venv/pyvenv.cfg"), "home = /usr/bin\n").unwrap();
    let target = loopback("app-remote-folders");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    remote::forbid_blocking();
    app.open_folder(&root);
    let src = row_of(&app, "src");
    app.set_dir_expanded(src, Some(true));
    assert!(
        !winc_backend::fs::is_dir(root.join("src")),
        "界面线程上查不到"
    );
    assert_eq!(app.known_folder(&root), Some(true));
    assert_eq!(app.known_folder(&root.join("src/deep")), Some(true));
    assert_eq!(app.known_folder(&root.join("src/main.rs")), Some(false));

    let opened = |app: &mut App, path: Option<PathBuf>| {
        app.opened_terminal = None;
        app.run_tree_command(MenuCommand::OpenInTerminal, path);
        app.opened_terminal.clone().expect("记下了目录")
    };
    // 空白处是打开的目录，不是远端的根目录；目录是它自己，文件是所在的目录。
    assert_eq!(opened(&mut app, None), root);
    assert_eq!(opened(&mut app, Some(root.join("src"))), root.join("src"));
    assert_eq!(
        opened(&mut app, Some(root.join("src/deep"))),
        root.join("src/deep")
    );
    assert_eq!(
        opened(&mut app, Some(root.join("src/main.rs"))),
        root.join("src")
    );

    // 快捷键：焦点在文件树上时取选中的目录，在编辑区时取打开的文件所在的目录。
    app.focus = Focus::Tree;
    app.editor.workspace.as_mut().unwrap().select(Some(src));
    app.opened_terminal = None;
    app.run_action(Action::OpenInTerminal);
    assert_eq!(
        app.opened_terminal.as_deref(),
        Some(root.join("src").as_path())
    );
    app.open_path(&root.join("src/deep/a.rs"));
    app.focus = Focus::Editor;
    app.run_action(Action::OpenInTerminal);
    assert_eq!(
        app.opened_terminal.as_deref(),
        Some(root.join("src/deep").as_path())
    );

    // 在目录中查找：范围是这个目录，不是它的上一级。
    app.find_in_folder(&root.join("src"));
    assert_eq!(app.panels.global.include.text(), "./src");
    app.find_in_folder(&root.join("src/deep"));
    assert_eq!(app.panels.global.include.text(), "./src/deep");
    app.find_in_folder(&root.join("src/main.rs"));
    assert_eq!(app.panels.global.include.text(), "./src");

    // 终端模块开着时在那个目录新建终端标签：目录是完整的远端路径。
    app.config.terminal_enabled = Some(true);
    app.terminal = Default::default();
    app.run_tree_command(MenuCommand::OpenInTerminal, Some(root.join("src")));
    let want = root.join("src");
    assert_eq!(app.terminal.want, [want.as_os_str().as_bytes().to_vec()]);

    // 打开之前的检查：远端的文件无法查询时仍然打开，由读取报告结果；目录不当作文件。
    assert!(app.may_be_file(&root.join("src/main.rs")));
    assert!(app.may_be_file(&root.join("src/deep/unlisted.rs")));
    assert!(!app.may_be_file(&root.join("src")));
    assert!(app.may_exist(&root.join("Cargo.toml")));
    // 重新打开关闭的标签：远端的文件不被当作已经不存在而跳过。
    app.open_path(&root.join("src/main.rs"));
    let count = app.editor.documents.len();
    app.close_tab(app.editor.active.unwrap());
    assert_eq!(app.editor.documents.len(), count - 1);
    app.reopen_closed();
    assert_eq!(app.editor.documents.len(), count);
    let reopened = app.editor.active().unwrap();
    assert_eq!(
        reopened.path.as_deref(),
        Some(root.join("src/main.rs").as_path())
    );
    // 现场里记下远端的标签，下次打开时恢复。
    assert!(app.session_tab(reopened).is_some());
    // 终端输出中的位置：文件树知道的、打开的目录之内带行号的可以跳转；不带行号、看不到的
    // 路径和目录之外的不认。
    assert!(app.terminal_target(&root.join("src/main.rs"), false));
    assert!(app.terminal_target(&root.join("src/deep/unlisted.rs"), true));
    assert!(!app.terminal_target(&root.join("src/deep/unlisted.rs"), false));
    let outside = winc_backend::virtual_path(&target, b"/no/such/place.rs");
    assert!(!app.terminal_target(&outside, true));

    // 现场中远端终端的目录可以只有那台主机上的路径：恢复时换算到打开的目录所在的主机，
    // 不在本机启动 shell；已经换算过的不再换算。
    app.terminal = Default::default();
    let host_path = dir.join("src");
    let session = winc_config::TerminalSession {
        tabs: vec![host_path, root.join("src/deep"), PathBuf::new()],
        open: true,
        ..Default::default()
    };
    app.restore_terminal(&session);
    let wanted: Vec<PathBuf> = (app.terminal.want.iter())
        .map(|dir| PathBuf::from(std::ffi::OsStr::from_bytes(dir)))
        .collect();
    assert_eq!(
        wanted,
        [root.join("src"), root.join("src/deep"), PathBuf::new()]
    );
    app.terminal = Default::default();

    // 远端的 shell 上报当前目录：标签记下的、现场里保存的都是那台主机上的路径，重启标签和
    // 重新打开目录时仍回到那里。
    super::terminal::show_terminal(&mut app, "$ ");
    let id = app.terminal.tab().unwrap().id;
    let reported = dir.join("src").as_os_str().as_bytes().to_vec();
    app.terminal_message(winc_pty::protocol::FromHelper::Event {
        tab: id,
        event: winc_pty::protocol::Event::Cwd(reported),
    });
    let saved = app.terminal_session().expect("有终端标签");
    assert_eq!(saved.tabs, [root.join("src")]);
    app.terminal = Default::default();

    // Python 环境：远端的目录按文件树上看得到的判断。看得到环境的目录同样经过询问（测试中没有
    // 窗口，按同意处理）；看不到的不加载，不会不经询问就加载远端的环境。
    app.terminal.want = vec![Vec::new(), want.as_os_str().as_bytes().to_vec()];
    assert_eq!(app.python_choices(), Some(vec![true, false]));

    remote::disconnect(&target);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端工作区中不经文件树的各项功能：快速打开、全局搜索、跳转定义、查找引用、复制路径、
/// 外部修改后的重读和打开不存在的文件。由 [`a_folder_on_a_connected_host_opens_as_the_workspace`]
/// 调用，原因同 [`folders_on_a_remote_host_are_not_taken_for_files`]。
fn features_work_in_a_remote_workspace() {
    let dir = search_folder("remote-features").canonicalize().unwrap();
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

/// 远端主机上的终端里粘贴剪贴板中的图片或本机文件：先传到那台主机上的缓存目录，再粘贴那里的
/// 路径（那里的程序读不到本机的剪贴板）。本机的终端不变。由
/// [`a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`folders_on_a_remote_host_are_not_taken_for_files`]。
fn clipboard_images_pasted_into_a_remote_terminal_are_sent_there_first() {
    let dir = sample_folder("remote-terminal-paste")
        .canonicalize()
        .unwrap();
    let cache = dir.join("cache");
    *crate::app::terminal::upload::CACHE.lock().unwrap() = Some(cache.clone());
    let target = loopback("app-remote-terminal-paste");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);
    super::terminal::show_terminal(&mut app, "\x1b[?2004h");
    let id = app.terminal.tab().unwrap().id;
    app.terminal.tab_mut().unwrap().cwd = root.as_os_str().as_bytes().to_vec();
    let log = app.attach_recording_helper(&dir);
    let pasted = |app: &App, from: usize| -> String {
        let sent = app.input_sent_to(&log, id);
        let text = String::from_utf8(sent[from..].to_vec()).unwrap();
        let inner = text
            .strip_prefix("\x1b[200~")
            .and_then(|t| t.strip_suffix("\x1b[201~"));
        inner.unwrap_or_else(|| panic!("{text:?}")).to_owned()
    };
    let folder = cache.join("winc/paste");

    // 剪贴板里只有图片：存为缓存目录中的 PNG，粘贴它在那台主机上的路径。
    let png = winc_editor::paste::encode_png(1, 1, &[255, 0, 0, 255]).unwrap();
    app.clipboard.copied = None;
    app.clipboard.copied_image = Some(png.clone());
    app.terminal_paste();
    let path = pasted(&app, 0);
    assert!(path.starts_with(folder.to_str().unwrap()), "{path}");
    assert!(path.ends_with(".png"), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), png);
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);

    // 从文件管理器复制的本机文件：传过去，名字里的空白换掉，粘贴的路径不需要引号。剪贴板里
    // 同时有文件名的文字（访达复制文件时如此），仍然传文件。
    let shot = dir.join("my shot.png");
    std::fs::write(&shot, &png).unwrap();
    app.clipboard.copied_image = None;
    app.clipboard.copied = Some("my shot.png".into());
    app.clipboard.copied_files = vec![shot];
    let before = app.input_sent_to(&log, id).len();
    app.terminal_paste();
    let path = pasted(&app, before);
    assert!(
        path.starts_with(folder.to_str().unwrap()) && path.ends_with("-my_shot.png"),
        "{path}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), png);

    // ⌃V：有程序运行、在主屏上、开启了括号粘贴时同样先传再粘贴路径。shell 的提示符下（⌃V 是
    // 逐字输入）、备用屏上的程序（vim 的 ⌃V 是列选择）和剪贴板里只有文字时照常发给程序。
    app.clipboard.copied = None;
    app.clipboard.copied_files.clear();
    app.clipboard.copied_image = Some(png.clone());
    app.terminal.tab_mut().unwrap().process = "bash".into();
    assert!(!app.ctrl_v_uploads());
    app.terminal.tab_mut().unwrap().process = "claude".into();
    let before = app.input_sent_to(&log, id).len();
    assert!(app.ctrl_v_uploads());
    assert!(pasted(&app, before).starts_with(folder.to_str().unwrap()));
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::ALT_SCREEN, true);
    assert!(!app.ctrl_v_uploads());
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::ALT_SCREEN, false);
    app.clipboard.copied = Some("text".into());
    assert!(!app.ctrl_v_uploads());
    app.clipboard.copied = None;
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::BRACKETED_PASTE, false);
    assert!(!app.ctrl_v_uploads());
    app.terminal
        .tab_mut()
        .unwrap()
        .modes
        .set(Modes::BRACKETED_PASTE, true);

    // 目录和超过上限的文件不传：说明原因，什么也不粘贴。
    app.clipboard.copied_files = vec![dir.join("cache")];
    let before = app.input_sent_to(&log, id).len();
    app.terminal_paste();
    assert_eq!(app.input_sent_to(&log, id).len(), before);
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("不能把文件夹粘贴到远端终端"), "{notice}");
    let large = dir.join("large.bin");
    let file = std::fs::File::create(&large).unwrap();
    file.set_len(crate::app::terminal::upload::MAX_UPLOAD + 1)
        .unwrap();
    app.clipboard.copied_files = vec![large];
    app.terminal_paste();
    assert_eq!(app.input_sent_to(&log, id).len(), before);
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("太大"), "{notice}");

    // 本机的终端不变：仍是一次空的括号粘贴，由程序自己读剪贴板；⌃V 也照常发给程序。
    app.clipboard.copied_files.clear();
    app.clipboard.copied_image = Some(png);
    app.terminal.tab_mut().unwrap().cwd = dir.as_os_str().as_bytes().to_vec();
    let before = app.input_sent_to(&log, id).len();
    app.terminal_paste();
    assert_eq!(pasted(&app, before), "");
    assert!(!app.ctrl_v_uploads());

    *crate::app::terminal::upload::CACHE.lock().unwrap() = None;
    remote::disconnect(&target);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端工作区不提供只能在本机执行的动作：菜单中不列出，浏览器按钮不出现；从快捷键、命令面板
/// 执行时说明原因，不显示系统的错误码。由 [`a_folder_on_a_connected_host_opens_as_the_workspace`]
/// 调用，原因同 [`folders_on_a_remote_host_are_not_taken_for_files`]。
fn local_only_actions_are_not_offered_on_a_remote_host() {
    use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

    let dir = search_folder("remote-local-only").canonicalize().unwrap();
    std::fs::write(dir.join("page.html"), "<p>hi</p>\n").unwrap();
    let target = loopback("app-remote-local-only");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);
    let file = root.join("src/main.rs");

    let listed = |app: &mut App, target: MenuTarget| -> Vec<MenuCommand> {
        app.open_context_menu(target, (10.0, 10.0));
        let menu = app.overlays.context_menu.take().unwrap();
        (menu.entries.into_iter())
            .filter_map(|entry| match entry {
                MenuEntry::Item(command, _) => Some(command),
                _ => None,
            })
            .collect()
    };
    // 终端模块关闭：两项都不列出；开启后「在终端中打开」进入内置终端，照常列出。
    for terminal in [false, true] {
        app.config.terminal_enabled = Some(terminal);
        let tree = listed(&mut app, MenuTarget::Tree(Some(file.clone())));
        assert!(!tree.contains(&MenuCommand::Reveal), "{tree:?}");
        assert_eq!(tree.contains(&MenuCommand::OpenInTerminal), terminal);
        assert!(tree.contains(&MenuCommand::CopyPath) && tree.contains(&MenuCommand::NewFile));
        let blank = listed(&mut app, MenuTarget::Tree(None));
        assert!(!blank.contains(&MenuCommand::Reveal), "{blank:?}");
        assert_eq!(blank.contains(&MenuCommand::OpenInTerminal), terminal);
    }
    app.config.terminal_enabled = Some(false);
    let changed = listed(&mut app, MenuTarget::CommitFile(file.clone()));
    assert!(!changed.contains(&MenuCommand::Reveal), "{changed:?}");
    assert!(changed.contains(&MenuCommand::GitOpenFile));
    let places = app.location_row_menu(&root, None);
    assert!(!places.contains(&MenuEntry::Item(MenuCommand::LocationReveal, true)));
    // 本机的文件不受影响。
    let local = listed(&mut app, MenuTarget::Tree(Some(dir.join("src/main.rs"))));
    assert!(local.contains(&MenuCommand::Reveal) && local.contains(&MenuCommand::OpenInTerminal));
    let places = app.location_row_menu(&dir, None);
    assert!(places.contains(&MenuEntry::Item(MenuCommand::LocationReveal, true)));

    // 快捷键和命令面板仍能到达这些动作：说明原因（以及可以怎样把挂载在本机的目录对应起来）。
    app.reveal_in_file_manager(&file);
    let notice = app.notice.text.take();
    assert_eq!(
        notice.as_deref(),
        Some(winc_i18n::t!("workspace-remote-reveal-unmapped"))
    );

    // 远端的目录挂载在本机（`remote.localFolders`，这里用同一个目录代替挂载点）：「在文件管理器
    // 中显示」重新列出，作用于本机上的那一份，不再提示；挂载点之外的文件仍不列出。
    let mount = format!("app-remote-local-only:{}/src", dir.display());
    let local = dir.join("src").display().to_string();
    app.config.remote_local_folders.insert(mount, local);
    assert_eq!(app.local_folder(&file), Some(dir.join("src/main.rs")));
    let tree = listed(&mut app, MenuTarget::Tree(Some(file.clone())));
    assert!(tree.contains(&MenuCommand::Reveal), "{tree:?}");
    let outside = listed(&mut app, MenuTarget::Tree(Some(root.join("page.html"))));
    assert!(!outside.contains(&MenuCommand::Reveal), "{outside:?}");
    app.reveal_in_file_manager(&file);
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    app.config.remote_local_folders.clear();
    app.open_in_terminal(Some(&file));
    let notice = app.notice.text.take();
    assert_eq!(
        notice.as_deref(),
        Some(winc_i18n::t!("workspace-remote-terminal"))
    );

    // 远端的目录没有运行配置：按钮不显示，命令说明原因，不在本机新建配置文件。
    app.config.terminal_enabled = Some(true);
    for action in [Action::RunConfiguration, Action::EditRunConfigurations] {
        app.run_action(action);
        let notice = app.notice.text.take();
        assert_eq!(
            notice.as_deref(),
            Some(winc_i18n::t!("terminal-run-remote"))
        );
    }
    assert!(app.run.list.is_empty());
    assert!(!dir.join(".winc").exists());
    app.config.terminal_enabled = Some(false);

    // 远端的网页文件没有「在浏览器中打开」；本机的有。
    app.open_path(&root.join("page.html"));
    assert!(app.editor.active().is_some_and(|d| d.path.is_some()));
    assert!(app.browser_page().is_none());
    app.open_path(&dir.join("page.html"));
    assert!(app.browser_page().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端工作区的文件树里复制、剪切文件，再粘贴到同一工作区。由
/// [`a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`folders_on_a_remote_host_are_not_taken_for_files`]。
///
/// 远端文件树的「下载…」：文件和目录复制到本机选定的目录，重名时不覆盖，完成后可以在文件管理器
/// 中显示。由 [`a_folder_on_a_connected_host_opens_as_the_workspace`] 调用，原因同
/// [`folders_on_a_remote_host_are_not_taken_for_files`]。
fn remote_items_download_to_this_computer() {
    let dir = search_folder("remote-download").canonicalize().unwrap();
    std::fs::write(dir.join("notes.txt"), "note\n").unwrap();
    std::fs::write(dir.join("50%.txt"), "half\n").unwrap();
    std::fs::create_dir_all(dir.join("kit/inner")).unwrap();
    std::fs::write(dir.join("kit/inner/tool.txt"), "tool\n").unwrap();
    let here = sample_folder("remote-download-here")
        .canonicalize()
        .unwrap();
    std::fs::write(here.join("notes.txt"), "mine\n").unwrap();
    let target = loopback("app-remote-download");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);

    let items = ["notes.txt", "kit", "50%.txt"].map(|name| root.join(name));
    app.download(items.to_vec(), here.clone());
    let read = |name: &str| std::fs::read_to_string(here.join(name)).unwrap();
    assert_eq!(read("notes.txt"), "mine\n");
    assert_eq!(read("notes 副本.txt"), "note\n");
    assert_eq!(read("kit/inner/tool.txt"), "tool\n");
    assert_eq!(read("50%.txt"), "half\n");
    assert_eq!(app.notice.text.as_deref(), Some("已下载 3 项"));
    assert_eq!(
        app.notice.action,
        Some(NoticeAction::Reveal(here.join("50%.txt")))
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&here);
}

/// 回归（#272）：系统剪贴板的文件列表放不下远端的文件，写入失败被忽略，粘贴时用的是剪贴板里
/// 原有的内容：旧文字写成 `untitled.txt`，旧的本机文件被复制到远端，剪切永远不移动。
fn files_copied_in_a_remote_tree_are_pasted_there() {
    let dir = search_folder("remote-tree-copy").canonicalize().unwrap();
    std::fs::write(dir.join("notes.txt"), "note\n").unwrap();
    std::fs::create_dir(dir.join("into-copy")).unwrap();
    std::fs::create_dir(dir.join("into-cut")).unwrap();
    let stale = sample_folder("remote-tree-stale").join("a.txt");
    let target = loopback("app-remote-tree-copy");
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    let mut app = blank_app(false);
    app.config.remote_enabled = Some(true);
    app.open_folder(&root);
    let select = |app: &mut App, name: &str| {
        let row = row_of(app, name);
        app.editor.workspace.as_mut().unwrap().selected = Some(row);
    };
    let notes = root.join("notes.txt");

    // 剪贴板里原有本机的文件：复制远端的文件以后，粘贴的是远端的文件。系统剪贴板里写的是它在
    // 那台主机上的路径，粘贴到终端、聊天窗口里可以直接用。
    app.clipboard.copied_files = vec![stale.clone()];
    app.copy_entry(&notes);
    let native = dir.join("notes.txt").to_string_lossy().into_owned();
    assert_eq!(app.clipboard_text(), Some(native.clone()));
    assert_eq!(app.clipboard_files(), std::slice::from_ref(&notes));
    select(&mut app, "into-copy");
    app.paste_into_tree();
    assert_eq!(
        std::fs::read_to_string(dir.join("into-copy/notes.txt")).unwrap(),
        "note\n"
    );
    assert!(!dir.join("into-copy/a.txt").exists() && dir.join("notes.txt").exists());
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);

    // 剪切：粘贴时移过去，只移一次。
    app.cut_entries(std::slice::from_ref(&notes));
    assert_eq!(
        *app.link.shared.cut_files.borrow(),
        std::slice::from_ref(&notes)
    );
    select(&mut app, "into-cut");
    app.paste_into_tree();
    assert!(dir.join("into-cut/notes.txt").exists() && !dir.join("notes.txt").exists());
    assert!(app.link.shared.cut_files.borrow().is_empty());

    // 粘贴到编辑区：非 Markdown 文档中写那台主机上的路径，不写本机的虚拟路径。
    let moved = root.join("into-cut/notes.txt");
    app.copy_entry(&moved);
    app.open_path(&root.join("src/main.rs"));
    app.focus = Focus::Editor;
    app.select(0, 0);
    app.paste();
    let text = app
        .editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap();
    let native = dir
        .join("into-cut/notes.txt")
        .to_string_lossy()
        .into_owned();
    assert!(text.starts_with(&native), "{text}");

    // 复制以后剪贴板被别的内容替换（这里由本程序写入一段文字）：不再当作复制的文件。
    app.copy_entry(&moved);
    app.set_clipboard("other".into(), false);
    assert!(app.clipboard_files().is_empty());
    assert!(app.link.shared.remote_copy.borrow().is_none());

    // 本程序写入的文字与复制时写入的路径相同（例如「复制路径」）：同样不再当作复制的文件。
    app.copy_entry(&moved);
    let path = app.clipboard_text().unwrap();
    app.set_clipboard(path, false);
    assert!(app.clipboard_files().is_empty());

    // 写不进剪贴板：说明原因，上一次的剪切仍然有效。
    app.cut_entries(std::slice::from_ref(&moved));
    app.clipboard.unavailable = true;
    app.copy_entry(&root.join("src/main.rs"));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("无法写入系统剪贴板"), "{notice}");
    app.clipboard.unavailable = false;
    assert_eq!(app.clipboard_files(), std::slice::from_ref(&moved));
    assert_eq!(
        *app.link.shared.cut_files.borrow(),
        std::slice::from_ref(&moved)
    );

    // 剪切的远端文件粘贴到另一个窗口里本机的目录：不同主机之间不能移动，改为复制，原处的保留。
    // 两个窗口共用各窗口间的状态和系统剪贴板。
    let local = sample_folder("remote-tree-local").canonicalize().unwrap();
    let mut other = app_with(&local, false);
    other.link.shared = std::rc::Rc::clone(&app.link.shared);
    other.clipboard.copied = app.clipboard.copied.clone();
    other.paste_into_tree();
    let notice = other.notice.text.take().unwrap();
    assert!(notice.contains("不同主机之间不能移动"), "{notice}");
    assert!(local.join("notes.txt").exists() && dir.join("into-cut/notes.txt").exists());
    assert!(app.link.shared.cut_files.borrow().is_empty());
    let _ = std::fs::remove_dir_all(&local);

    remote::disconnect(&target);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(stale.parent().unwrap());
}
