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
