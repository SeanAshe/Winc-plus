//! 查找本机的 `git` 程序：在 `PATH` 里找，或使用设置的程序（`git.path`）。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use super::{CREDENTIAL_HELPER_ARG, Git, PROBE_TIMEOUT, Unavailable};
use crate::tools::shell::Shell;
use crate::tools::shell::process::{self, Cancel};

impl Git {
    /// 在用户 shell 的 `PATH` 里找 `git`，确认它能用。
    ///
    /// 第一次调用要等取到环境。
    ///
    /// # Errors
    ///
    /// 找不到，或者 macOS 上找到的是没装开发者工具时的占位程序，返回 [`Unavailable`]。
    pub fn find(shell: &Shell) -> Result<Git, Unavailable> {
        Git::find_with(shell, None)
    }

    /// 同 [`Git::find`]，`program` 是设置的 `git` 程序（`git.path`）：完整路径，或在 `PATH` 里
    /// 查找的程序名；为 `None` 或空白时在 `PATH` 里找 `git`。
    ///
    /// 设置的程序只用于本机的仓库：发行版、远端主机上的仓库使用各自系统中的 `git`。
    ///
    /// # Errors
    ///
    /// 同 [`Git::find`]；设置的程序不存在或不能运行时返回 [`Unavailable::NotFound`]。
    pub fn find_with(shell: &Shell, program: Option<&str>) -> Result<Git, Unavailable> {
        let custom = program.map(str::trim).filter(|text| !text.is_empty());
        let found = match custom {
            Some(text) => {
                let found = locate_program(shell, text);
                Some(found.ok_or_else(|| Unavailable::NotFound(text.to_owned()))?)
            }
            None => shell.find("git"),
        };
        let local = found.is_some();
        // 仅在 WSL 中安装了 `git` 时仍然可用：发行版中的仓库使用发行版内的 `git`（`Git::at`）。
        let program = found
            .or_else(|| crate::wsl::available().then(|| PathBuf::from("git.exe")))
            .ok_or(Unavailable::NotInstalled)?;
        if cfg!(target_os = "macos")
            && program == Path::new("/usr/bin/git")
            && !developer_tools_installed()
        {
            return Err(Unavailable::NoDeveloperTools);
        }
        Ok(Git {
            program,
            local,
            shell: shell.clone(),
            trust: Arc::default(),
            askpass: None,
        })
    }
}

/// 查找设置的 `git` 程序 `text`：含路径分隔符时是文件的路径（开头的 `~/` 指用户的主目录），
/// 否则是在 `PATH` 里查找的程序名。文件不存在时返回 `None`。
fn locate_program(shell: &Shell, text: &str) -> Option<PathBuf> {
    if !text.contains(['/', '\\']) {
        return shell.find(text);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let path = match (text.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(text),
    };
    winc_backend::fs::is_file(&path).then_some(path)
}

/// macOS 上命令行开发者工具是否已安装（`xcode-select -p` 成功）。
fn developer_tools_installed() -> bool {
    let mut command = Command::new("/usr/bin/xcode-select");
    command.arg("-p").stderr(Stdio::null());
    matches!(
        process::run(&mut command, Vec::new(), PROBE_TIMEOUT, &Cancel::default()),
        Ok(output) if output.status.success()
    )
}

/// 仓库 `root` 的 `git` 是否在本机运行：远端主机和 WSL 发行版中的仓库使用各自系统中的 `git`。
pub fn runs_here(root: &Path) -> bool {
    !winc_backend::is_remote(root) && crate::wsl::locate_here(root).is_none()
}

/// 把 `program` 追加为 `command` 的凭据助手，排在用户配置的助手之后。
///
/// 经环境变量 `GIT_CONFIG_COUNT`、`GIT_CONFIG_KEY_<n>`、`GIT_CONFIG_VALUE_<n>` 追加配置（Git
/// 2.31 起支持，更早的版本忽略这些变量），保留环境中已有的条目。只用于本机的仓库：远端主机
/// 和发行版中的 `git` 无法访问本机的凭据存储。
pub(super) fn add_credential_helper(command: &mut Command, program: &Path) {
    let count = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|count| count.parse::<usize>().ok())
        .unwrap_or(0);
    command
        .env("GIT_CONFIG_COUNT", (count + 1).to_string())
        .env(format!("GIT_CONFIG_KEY_{count}"), "credential.helper")
        .env(
            format!("GIT_CONFIG_VALUE_{count}"),
            credential_helper_value(&program.to_string_lossy()),
        );
}

/// 返回 `credential.helper` 的取值：以 `!` 开头，`git` 交给 shell 执行并在末尾追加动作。
///
/// 路径放在单引号中，其中的单引号写作 `'\''`；反斜杠换成斜杠，Git for Windows 的 shell 两种
/// 写法都接受。
pub(super) fn credential_helper_value(program: &str) -> String {
    let program = program.replace('\\', "/").replace('\'', r"'\''");
    format!("!'{program}' {CREDENTIAL_HELPER_ARG}")
}
