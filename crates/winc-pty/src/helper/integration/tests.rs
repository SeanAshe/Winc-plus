//! `integration` 的测试：写入脚本时统一换行符、识别进入默认 shell 的 `wsl.exe`、`WSLENV` 的合并，
//! 以及发行版内的启动脚本（以同名脚本代替 `wslpath` 和 shell）。

use super::*;

fn args(list: &[&str]) -> Vec<Vec<u8>> {
    list.iter().map(|arg| arg.as_bytes().to_vec()).collect()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "winc-pty-integration-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn scripts_are_written_with_lf_line_endings() {
    let dir = scratch("lf");
    write_scripts(&dir, &[("a/b.sh", "one\r\ntwo\n")]).unwrap();
    assert_eq!(fs::read(dir.join("a/b.sh")).unwrap(), b"one\ntwo\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn only_plain_entry_into_the_default_shell_is_injected() {
    let wsl = br"C:\Windows\System32\wsl.exe";
    assert!(opens_default_wsl_shell(wsl, &[]));
    assert!(opens_default_wsl_shell(
        wsl,
        &args(&["-d", "Ubuntu", "--cd", "/home/me"])
    ));
    assert!(opens_default_wsl_shell(
        b"WSL.EXE",
        &args(&["--user", "me"])
    ));
    assert!(!opens_default_wsl_shell(wsl, &args(&["--exec", "htop"])));
    assert!(!opens_default_wsl_shell(wsl, &args(&["-d"])));
    assert!(!opens_default_wsl_shell(wsl, &args(&["--", "ls"])));
    assert!(!opens_default_wsl_shell(b"cmd.exe", &[]));
}

#[test]
fn environment_for_wsl_keeps_what_the_user_forwards() {
    let mut env = vec![(b"WSLENV".to_vec(), b"GOPATH/l".to_vec())];
    forward_to_wsl(&mut env);
    let forwarded = b"GOPATH/l:WINC_NONCE:WINC_PROMPT:COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION";
    assert_eq!(env[0].1, forwarded);

    let mut env = Vec::new();
    forward_to_wsl(&mut env);
    assert_eq!(env[0].1, WSL_ENV);
}

#[cfg(unix)]
mod boot {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    fn executable(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// 在 `dir` 中准备脚本和代替 `wslpath` 的程序，以名为 `shell` 的程序运行启动命令，返回该程序
    /// 收到的参数和环境变量。`wslpath_works` 为假时 `wslpath` 以失败退出。
    fn run(
        dir: &Path,
        shell: &str,
        wslpath_works: bool,
        env: &[(&str, &str)],
    ) -> (Vec<String>, HashMap<String, String>) {
        let scripts = dir.join("scripts");
        write_scripts(&scripts, WSL).unwrap();
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wslpath = if wslpath_works {
            format!("#!/bin/sh\nprintf '%s' '{}'\n", scripts.display())
        } else {
            "#!/bin/sh\nexit 1\n".to_owned()
        };
        executable(&bin.join("wslpath"), &wslpath);
        // 代替 shell 的程序依次输出各参数和环境变量。
        let fake = bin.join(shell);
        executable(
            &fake,
            "#!/bin/sh\nfor a; do printf 'arg=%s\\n' \"$a\"; done\nenv\n",
        );
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let mut command = Command::new("sh");
        command
            .args(["-c", WSL_RUN, r"C:\Users\me\winc\shell"])
            .env_clear()
            .env("PATH", path)
            .env("SHELL", &fake)
            .env("WINC_NONCE", "secret");
        command.envs(env.iter().copied());
        let output = command.output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut args = Vec::new();
        let mut vars = HashMap::new();
        for line in text.lines() {
            if let Some(arg) = line.strip_prefix("arg=") {
                args.push(arg.to_owned());
            } else if let Some((key, value)) = line.split_once('=') {
                vars.insert(key.to_owned(), value.to_owned());
            }
        }
        (args, vars)
    }

    #[test]
    fn bash_reads_the_integration_as_its_rcfile() {
        let dir = scratch("boot-bash");
        let (args, vars) = run(&dir, "bash", true, &[]);
        let rcfile = dir.join("scripts/bash/winc.bash").display().to_string();
        assert_eq!(args, ["--rcfile", rcfile.as_str()]);
        assert_eq!(vars["WINC_BASH_LOGIN"], "1");
        assert_eq!(vars["WINC_NONCE"], "secret");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn zsh_uses_the_integration_directory_and_remembers_the_old_one() {
        let dir = scratch("boot-zsh");
        let (args, vars) = run(&dir, "zsh", true, &[("ZDOTDIR", "/home/me/z")]);
        assert_eq!(args, ["-l"]);
        assert_eq!(
            vars["ZDOTDIR"],
            dir.join("scripts/zsh").display().to_string()
        );
        assert_eq!(vars["WINC_ZDOTDIR"], "/home/me/z");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fish_finds_the_integration_through_its_data_directories() {
        let dir = scratch("boot-fish");
        let scripts = dir.join("scripts/fish").display().to_string();
        let (args, vars) = run(&dir, "fish", true, &[]);
        assert_eq!(args, ["-l"]);
        assert_eq!(
            vars["XDG_DATA_DIRS"],
            format!("{scripts}:/usr/local/share:/usr/share")
        );
        assert!(!vars.contains_key("WINC_XDG_DATA_DIRS"));

        let (_, vars) = run(&dir, "fish", true, &[("XDG_DATA_DIRS", "/opt/share")]);
        assert_eq!(vars["XDG_DATA_DIRS"], format!("{scripts}:/opt/share"));
        assert_eq!(vars["WINC_XDG_DATA_DIRS"], "/opt/share");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_shells_start_plainly_without_the_nonce() {
        let dir = scratch("boot-other");
        let (args, vars) = run(&dir, "nu", true, &[]);
        assert_eq!(args, ["-l"]);
        assert!(!vars.contains_key("WINC_NONCE"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_terminal_still_opens_when_the_scripts_cannot_be_read() {
        let dir = scratch("boot-no-path");
        let (args, vars) = run(&dir, "bash", false, &[]);
        assert_eq!(args, ["-l"]);
        assert!(!vars.contains_key("WINC_NONCE"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_remote_shell_gets_a_fresh_nonce_in_its_command() {
    let command = |text: &str| vec![b"-t".to_vec(), b"host".to_vec(), text.as_bytes().to_vec()];
    // 只换最后一处：目录名里恰好有这个记号时不动它。
    let mut args = command("cd '/a @WINC_NONCE@' && exec sh -c 'x' winc @WINC_NONCE@ 1");
    let nonce = inject_remote(b"/usr/bin/ssh", &mut args).unwrap();
    assert_eq!(nonce.len(), 32);
    let expected = format!("cd '/a @WINC_NONCE@' && exec sh -c 'x' winc {nonce} 1");
    assert_eq!(args[2], expected.as_bytes());
    assert_eq!(args[..2], command("")[..2]);
    let mut again = command("exec sh -c 'x' winc @WINC_NONCE@ 1");
    assert_ne!(inject_remote(b"ssh", &mut again).unwrap(), nonce);

    // 没有记号的命令、别的程序、没有参数：不注入，参数不变。
    let mut plain = command("cd '/a' && exec $SHELL -l");
    assert_eq!(inject_remote(b"ssh", &mut plain), None);
    assert_eq!(plain, command("cd '/a' && exec $SHELL -l"));
    let mut other = command("@WINC_NONCE@");
    assert_eq!(inject_remote(b"/bin/zsh", &mut other), None);
    assert_eq!(other, command("@WINC_NONCE@"));
    assert_eq!(inject_remote(b"ssh", &mut []), None);
    assert!(
        inject_remote(
            br"C:\Windows\System32\OpenSSH\ssh.exe",
            &mut command("@WINC_NONCE@")
        )
        .is_some()
    );
}
