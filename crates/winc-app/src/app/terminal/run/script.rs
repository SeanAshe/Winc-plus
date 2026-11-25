//! 在终端中运行文件树里的脚本：按文件的种类拼出一个运行配置，交给运行配置的启动流程，输出留在
//! 以文件名命名的终端标签里。运行是用户对这个文件明确的操作，不再询问。

use std::path::Path;

use winc_config::RunConfig;

use super::{ShellKind, launch_kind};
use crate::app::App;
use crate::app::terminal::helper::{Integration, default_shell_applies, launch, os_bytes};

/// 返回运行脚本 `path` 的命令，脚本的路径作为参数跟在后面；不是能运行的脚本时为 `None`。
///
/// Windows（`windows`）上按扩展名：`.cmd`、`.bat` 交给命令提示符，`.ps1` 交给 PowerShell 并忽略
/// 执行策略（默认的策略不允许运行脚本）；`kind` 是执行这一行的 shell，决定命令提示符怎样调用。
/// 其他系统上，开头是 `#!` 且有执行权限的文件（`direct`）直接执行，否则按扩展名交给对应的 shell。
fn script_command(
    path: &Path,
    kind: ShellKind,
    (windows, direct): (bool, bool),
) -> Option<&'static str> {
    let extension = path.extension().and_then(|e| e.to_str());
    let extension = extension.map(str::to_ascii_lowercase);
    match (windows, extension.as_deref()) {
        (_, _) if kind == ShellKind::Wsl => None,
        (true, Some("cmd" | "bat")) => Some(match kind {
            ShellKind::Cmd => "call",
            // MSYS 的 shell 把 `/C` 当作路径改写，写成 `//C` 才原样传给 `cmd`。
            ShellKind::Posix => "cmd //C",
            _ => "cmd /C",
        }),
        (true, Some("ps1")) => Some("powershell -NoLogo -ExecutionPolicy Bypass -File"),
        (true, _) => None,
        (false, _) if direct => Some("exec"),
        (false, Some("sh")) => Some("sh"),
        (false, Some("bash")) => Some("bash"),
        (false, Some("zsh")) => Some("zsh"),
        (false, Some("fish")) => Some("fish"),
        (false, _) => None,
    }
}

/// 返回运行脚本 `path` 的配置：名字是文件名，工作目录是脚本所在的目录（在打开的目录 `root` 中
/// 时写成相对路径）；其余参数和不能运行的情形见 [`script_command`]。
pub(super) fn script_config(
    path: &Path,
    root: &Path,
    kind: ShellKind,
    system: (bool, bool),
) -> Option<RunConfig> {
    let command = script_command(path, kind, system)?;
    let dir = path.parent()?;
    let cwd = match dir.strip_prefix(root) {
        Ok(inside) if inside.as_os_str().is_empty() => None,
        Ok(inside) => Some(inside.to_string_lossy().into_owned()),
        Err(_) => Some(dir.to_string_lossy().into_owned()),
    };
    Some(RunConfig {
        name: path.file_name()?.to_string_lossy().into_owned(),
        command: command.to_owned(),
        args: vec![path.to_string_lossy().into_owned()],
        cwd,
        env: Vec::new(),
    })
}

/// 文件 `path` 是否开头是 `#!` 且有执行权限；读不了时为否。
fn runs_directly(path: &Path) -> bool {
    let Ok(opened) = winc_backend::fs::open(path, 2) else {
        return false;
    };
    let unix = opened.metadata.unix;
    unix.is_some_and(|unix| unix.mode & 0o111 != 0) && opened.head == b"#!"
}

impl App {
    /// 返回在终端中运行文件 `path` 的配置（见 [`script_config`]）；终端模块关着、文件或打开的
    /// 目录在远端主机上、不是能运行的脚本时为 `None`。
    fn script_run(&self, path: &Path) -> Option<RunConfig> {
        let root = self.editor.root()?;
        let local = !winc_backend::is_remote(path) && !winc_backend::is_remote(root);
        if !self.terminal_enabled() || !local {
            return None;
        }
        let windows = cfg!(windows);
        let dir = os_bytes(path.parent()?.as_os_str());
        let shell = self.config.terminal_shell.as_deref();
        let shell = shell.filter(|_| default_shell_applies(&dir, windows));
        let kind = launch_kind(&launch(shell, dir, windows, Integration::Off), windows);
        let direct = !windows && runs_directly(path);
        script_config(path, root, kind, (windows, direct))
    }

    /// 文件树中的文件 `path` 是否列出「在终端中运行」。
    pub(in crate::app) fn can_run_script(&self, path: &Path) -> bool {
        self.script_run(path).is_some()
    }

    /// 在终端中运行脚本 `path`：它开着且有未保存的修改时先保存，保存不了就不运行。
    pub(in crate::app) fn run_script(&mut self, path: &Path) {
        let Some(config) = self.script_run(path) else {
            return;
        };
        let file = self.editor.document_path(path);
        let mut docs = self.editor.documents.iter();
        let open = docs.find(|doc| doc.path.as_deref() == Some(&*file));
        if let Some(id) = open.filter(|doc| doc.is_dirty()).map(|doc| doc.id)
            && !self.write_document(id, None, true)
        {
            return;
        }
        self.start_run(config);
    }
}
