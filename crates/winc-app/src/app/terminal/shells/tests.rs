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
    // 路径含空白时加上引号。
    let shells = unix_shells("", Some("/my dir/zsh"), |_: &str| true);
    assert_eq!(names(&shells)[0], "zsh");
    assert_eq!(shells[0].command, "\"/my dir/zsh\"");
}

#[test]
fn windows_lists_programs_on_the_path_git_bash_and_wsl_distros() {
    let has = |name: &str| name != "pwsh.exe";
    let bash = r"C:\Program Files\Git\bin\bash.exe";
    let shells = windows_shells(has, Some(bash), &["Ubuntu".to_owned()]);
    assert_eq!(
        names(&shells),
        ["powershell", "cmd", "Git Bash", "Ubuntu (WSL)"]
    );
    assert_eq!(shells[0].command, "powershell.exe");
    assert_eq!(
        shells[2].command,
        r#""C:\Program Files\Git\bin\bash.exe" --login -i"#
    );
    assert_eq!(shells[3].command, "wsl.exe -d Ubuntu");
    assert_eq!(
        names(&windows_shells(has, None, &[])),
        ["powershell", "cmd"]
    );
}

#[test]
fn git_bash_is_found_from_git_on_the_path_or_the_default_folders() {
    let vars = |key: &str| match key {
        "ProgramFiles" => Some(r"C:\Program Files".to_owned()),
        "LocalAppData" => Some(r"C:\Users\me\AppData\Local".to_owned()),
        _ => None,
    };
    let path = [r"C:\Windows".to_owned(), r"D:\Tools\Git\cmd\".to_owned()];
    // `PATH` 中 `cmd` 目录里的 git.exe 指向安装目录，优先于默认目录。
    let files = [
        r"D:\Tools\Git\cmd\git.exe",
        r"D:\Tools\Git\bin\bash.exe",
        r"C:\Program Files\Git\bin\bash.exe",
    ];
    let exists = |path: &str| files.contains(&path);
    let found = git_bash(&path, vars, exists);
    assert_eq!(found.as_deref(), Some(r"D:\Tools\Git\bin\bash.exe"));
    // `PATH` 中没有 Git 时查找默认目录：全局安装、当前用户安装。
    let files = [r"C:\Users\me\AppData\Local\Programs\Git\bin\bash.exe"];
    let exists = |path: &str| files.contains(&path);
    let found = git_bash(&path, vars, exists);
    assert_eq!(found.as_deref(), Some(files[0]));
    assert_eq!(git_bash(&path, vars, |_: &str| false), None);
}

#[test]
fn the_wsl_list_is_utf16_without_docker() {
    let text = "\u{feff}Ubuntu-22.04\r\ndocker-desktop\r\ndocker-desktop-data\r\nMy Distro\r\n\r\n";
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-22.04", "My Distro"]);
    assert!(parse_wsl_list(b"").is_empty());
    let shells = windows_shells(|_: &str| false, None, &["My Distro".to_owned()]);
    assert_eq!(shells[0].command, "wsl.exe -d \"My Distro\"");
}

#[test]
fn commands_split_on_whitespace_outside_double_quotes() {
    let words = command_words(r#""C:\Program Files\Git\bin\bash.exe" --login  -i"#);
    assert_eq!(
        words,
        [r"C:\Program Files\Git\bin\bash.exe", "--login", "-i"]
    );
    assert_eq!(command_words(r#"sh -c "" x"#), ["sh", "-c", "", "x"]);
    assert_eq!(command_words(r#"a"b c"d"#), ["ab cd"]);
    assert!(command_words("   ").is_empty());
}

#[test]
fn the_program_name_drops_the_directory_and_extension() {
    assert_eq!(program_name("/bin/zsh"), "zsh");
    assert_eq!(program_name("nu --login"), "nu");
    assert_eq!(program_name(r"C:\Tools\pwsh.exe -NoLogo"), "pwsh");
    assert_eq!(
        program_name(r#""C:\Program Files\Git\bin\bash.exe" -i"#),
        "bash"
    );
    assert_eq!(program_name(""), "");
}
