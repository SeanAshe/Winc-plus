//! 远程模块的测试：模块开关、经连接打开目录和文件、失败的提示，以及最近列表中的远端目录。
//!
//! 服务端在本进程内运行，不经过 `ssh`；各项检查放在同一个测试中，因为关闭模块会断开进程内
//! 的全部连接。

use std::os::unix::ffi::OsStrExt;

use winc_backend::remote;

use super::*;
use crate::app::git::form::FormKind;
use crate::keymap::Action;

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
    app.delete_entries(&[root.join("renamed.rs"), root.join("old")]);
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

    // 目录不存在：说明原因，已打开的目录不变。
    app.open_remote_picker();
    typed(&mut app, "app-remote-open:/winc/no/such/folder");
    assert_eq!(
        titles(&app)[0],
        "连接到 app-remote-open:/winc/no/such/folder"
    );
    app.picker_accept();
    let notice = app.notice.text.take().unwrap();
    assert!(notice.starts_with("无法连接 app-remote-open："), "{notice}");
    assert_eq!(app.editor.root(), Some(root.as_path()));

    // 最近列表中远端的目录经同一流程打开，不在界面线程上判断它是否存在；本机的目录不经此处。
    assert!(app.state.recent.contains(&root));
    assert!(app.reopen_remote(&root));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert!(!app.reopen_remote(&dir));

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
    app.panels.tree.copies.track(Arc::clone(&copying));

    // 右上角正显示着别的提示：不替换它，等它消失之后再显示进度。
    app.notify("别的提示");
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
    app.panels.tree.copies.track(Arc::clone(&copying));
    app.clear_flash();
    app.copy_tick(now + Duration::from_secs(9));
    assert_eq!(app.notice.action, Some(NoticeAction::CancelCopy));
    assert!(!app.copy_finished(&copying));
    assert!(app.notice.text.is_none() && app.status.message.is_none());
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
