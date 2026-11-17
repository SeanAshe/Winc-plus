//! 查找 `git` 程序的测试：设置的程序、找不到时的原因、本机没有 `git` 时的仓库。

use super::*;

/// 设置的程序可以是完整路径或 `PATH` 里的程序名；空白视为未设置。
#[test]
fn a_configured_program_is_used_by_path_or_by_name() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Some(real) = shell.find("git") else {
        return;
    };
    for program in [Some(real.to_str().unwrap()), Some("git"), Some("  "), None] {
        let git = Git::find_with(&shell, program).unwrap();
        assert!(git.local, "{program:?}");
    }
    assert_eq!(Git::find_with(&shell, real.to_str()).unwrap().program, real);
}

/// 设置的程序不存在时报告设置的写法，不退回 `PATH` 里的 `git`。
#[test]
fn a_missing_configured_program_is_reported_by_name() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let missing = crate::test_util::scratch("no-git").join("bin").join("git");
    for program in [missing.to_str().unwrap(), "winc-no-such-git-program"] {
        let error = Git::find_with(&shell, Some(program)).unwrap_err();
        assert_eq!(error, Unavailable::NotFound(program.to_owned()));
        assert!(error.to_string().contains(program));
    }
}

/// 本机没有 `git` 时本机的仓库按未安装报告，不运行不存在的程序。
#[test]
fn a_local_repository_without_local_git_reports_not_installed() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(mut git) = Git::find(&shell) else {
        return;
    };
    git.local = false;
    let dir = crate::test_util::scratch("no-local-git");
    let error = git.status(&dir, &Cancel::default()).unwrap_err();
    assert!(
        error.contains(&Unavailable::NotInstalled.to_string()),
        "{error}"
    );
}

/// 凭据助手的取值交给 shell 执行：路径加单引号，其中的单引号转义，反斜杠换成斜杠。
#[test]
fn the_credential_helper_value_quotes_the_program_for_the_shell() {
    use crate::git::locate::credential_helper_value;
    assert_eq!(
        credential_helper_value("/Applications/Winc.app/Contents/MacOS/winc"),
        "!'/Applications/Winc.app/Contents/MacOS/winc' git-credential"
    );
    assert_eq!(
        credential_helper_value(r"C:\Program Files\Winc\winc.exe"),
        "!'C:/Program Files/Winc/winc.exe' git-credential"
    );
    assert_eq!(
        credential_helper_value("/home/o'neil/winc"),
        r"!'/home/o'\''neil/winc' git-credential"
    );
}

/// 凭据助手排在环境中已有的配置条目之后，不覆盖它们。
#[test]
fn the_credential_helper_is_appended_to_the_command_environment() {
    use crate::git::locate::add_credential_helper;
    let mut command = std::process::Command::new("git");
    add_credential_helper(&mut command, Path::new("/opt/winc"));
    let envs: Vec<(String, String)> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned())))
        .collect();
    let count: usize = (envs.iter())
        .find(|(key, _)| key == "GIT_CONFIG_COUNT")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap();
    let key = format!("GIT_CONFIG_KEY_{}", count - 1);
    let value = format!("GIT_CONFIG_VALUE_{}", count - 1);
    assert!(envs.contains(&(key, "credential.helper".to_owned())));
    assert!(envs.contains(&(value, "!'/opt/winc' git-credential".to_owned())));
}
