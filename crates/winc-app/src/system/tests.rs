//! `system` 的测试：交给系统打开的链接只放行网页和邮件；远端主机上的路径不交给本机的程序。

use super::*;

#[test]
fn only_web_and_mail_links_go_to_the_system() {
    for url in ["https://docs.rs", "HTTP://x", "mailto:a@b.c"] {
        assert!(openable_url(url), "{url}");
        assert!(open_url(url).is_ok(), "{url}");
    }
    for url in [
        "file:///Applications/Calculator.app",
        "smb://host/share",
        "-a/Applications/Calculator.app",
        "/tmp/x.command",
        "x-man-page://ls",
        "",
    ] {
        assert!(!openable_url(url), "{url}");
        let error = open_url(url).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{url}");
    }
}

#[test]
fn files_on_a_remote_host_are_not_handed_to_local_programs() {
    let target = winc_backend::Target::new("devbox").unwrap();
    let file = winc_backend::virtual_path(&target, b"/home/alice/notes.md");
    let refused = |result: io::Result<()>| {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
    };
    refused(open_with_default_app(&file));
    refused(open_in_browser(&file));
    refused(reveal_in_file_manager(&file));
    refused(move_to_trash(&file));
    refused(open_terminal(file.parent().unwrap(), None));
}

#[test]
fn a_folder_in_a_wsl_distro_opens_a_terminal_inside_the_distro() {
    // 没有指定终端程序：进入发行版和目录，不以网络路径作为 `cmd.exe` 的当前目录。
    let dir = Path::new(r"\\wsl.localhost\Ubuntu-26.04\home\me\project");
    let (args, cwd) = windows_terminal(dir, None);
    assert_eq!(
        args,
        ["wsl.exe", "-d", "Ubuntu-26.04", "--cd", "/home/me/project"]
    );
    assert_eq!(cwd, None);

    // 指定了终端程序，或目录不在发行版中：仍然在该目录下启动。
    let (args, cwd) = windows_terminal(dir, Some("wt.exe"));
    assert_eq!((args, cwd), (vec!["wt.exe".to_owned()], Some(dir)));
    let local = Path::new(r"C:\code\project");
    let (args, cwd) = windows_terminal(local, None);
    assert_eq!((args, cwd), (vec!["cmd.exe".to_owned()], Some(local)));
}

#[test]
fn folders_without_a_trash_are_deleted_for_good() {
    // WSL 发行版中的路径在 Windows 上是网络路径，没有回收站；远端主机上的路径同样没有。
    let distro = Path::new(r"\\wsl.localhost\Ubuntu\home\me\a.txt");
    assert!(has_no_trash(distro, true));
    assert!(!has_no_trash(distro, false));
    assert!(!has_no_trash(Path::new(r"C:\code\a.txt"), true));
    let target = winc_backend::Target::new("me@no-trash").unwrap();
    assert!(has_no_trash(
        &winc_backend::virtual_path(&target, b"/srv/a"),
        false
    ));
}
