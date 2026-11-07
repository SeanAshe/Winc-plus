//! `shells` 的测试：Unix 和 Windows 上列出哪些 shell、顺序，WSL 列表的解析，程序名的取法。

use super::*;

fn names(shells: &[ShellChoice]) -> Vec<&str> {
    shells.iter().map(|shell| shell.name.as_str()).collect()
}

#[test]
fn unix_lists_what_exists_with_the_default_first() {
    let list = "# 注释\n/bin/bash\n/bin/csh\n/bin/sh\n/bin/zsh\n/usr/bin/bash\n";
    let exists = |path: &str| ["/bin/bash", "/bin/zsh", "/usr/bin/bash"].contains(&path);

    let shells = unix_shells(list, Some("/bin/bash"), exists);
    assert_eq!(names(&shells), ["bash", "zsh"]);
    assert_eq!(shells[0].command, "/bin/bash");

    // 没有默认 shell 时按固定顺序；同名只留第一个。
    let shells = unix_shells(list, None, exists);
    assert_eq!(names(&shells), ["zsh", "bash"]);
    assert_eq!(shells[1].command, "/bin/bash");
}

#[test]
fn unix_finds_unregistered_shells_and_keeps_an_unusual_default() {
    let exists = |path: &str| matches!(path, "/opt/homebrew/bin/fish" | "/usr/local/bin/xonsh");
    // 没有登记的 fish 也能找到；默认 shell 不在固定列表里也列出来。
    let shells = unix_shells("", Some("/usr/local/bin/xonsh"), exists);
    assert_eq!(names(&shells), ["xonsh", "fish"]);
    assert_eq!(shells[1].command, "/opt/homebrew/bin/fish");
}

#[test]
fn unix_skips_what_cannot_be_started() {
    let exists = |_: &str| false;
    assert!(unix_shells("/bin/zsh\n", Some("/bin/zsh"), exists).is_empty());
    // 路径含空白：启动命令按空白分词，放不进去。
    let shells = unix_shells("/my dir/zsh\n", Some("/my dir/zsh"), |_: &str| true);
    assert_eq!(shells[0].command, "/opt/homebrew/bin/zsh");
}

#[test]
fn windows_lists_programs_on_the_path_and_wsl_distros() {
    let has = |name: &str| name != "pwsh.exe";
    let shells = windows_shells(has, &["Ubuntu".to_owned()]);
    assert_eq!(names(&shells), ["powershell", "cmd", "Ubuntu (WSL)"]);
    assert_eq!(shells[0].command, "powershell.exe");
    assert_eq!(shells[2].command, "wsl.exe -d Ubuntu");
}

#[test]
fn the_wsl_list_is_utf16_without_docker_or_spaces() {
    let text = "\u{feff}Ubuntu-22.04\r\ndocker-desktop\r\ndocker-desktop-data\r\nMy Distro\r\n\r\n";
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-22.04"]);
    assert!(parse_wsl_list(b"").is_empty());
}

#[test]
fn the_program_name_drops_the_directory_and_extension() {
    assert_eq!(program_name("/bin/zsh"), "zsh");
    assert_eq!(program_name("nu --login"), "nu");
    assert_eq!(program_name(r"C:\Tools\pwsh.exe -NoLogo"), "pwsh");
    assert_eq!(program_name(""), "");
}
