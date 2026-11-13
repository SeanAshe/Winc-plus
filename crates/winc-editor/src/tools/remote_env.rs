//! 远端主机上的程序：用户登录 shell 的 `PATH`，以及按名字查找程序。
//!
//! 服务进程由 `ssh` 直接启动，它的环境不经过用户的登录脚本，`PATH` 里通常没有用户自己安装的
//! 工具（`~/.cargo/bin`、版本管理器的目录）。在远端运行用户的工具之前，先向登录 shell 问出
//! 它的 `PATH`；每台主机只问一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use winc_backend::Target;
use winc_backend::remote::{self, Exec};

/// 向远端查询环境或查找程序时最长等待的时间（5 秒，经验值，同本机查询工具链时的时限）：
/// 登录脚本正常在一秒内结束，超时按查不到处理。
const TIMEOUT: Duration = Duration::from_secs(5);

/// 输出中标出 `PATH` 的一行的前缀：登录脚本可能先输出别的内容。
const MARK: &str = "winc-path:";

/// 让用户的登录 shell 输出它的 `PATH`。由 `sh` 运行；内层的命令交给用户的 shell 解释，写法
/// 只用各种 shell 共有的部分。不接受 `-l` 与 `-c` 同用的 shell 改为不带 `-l` 再问一次。
const ASK: &str = r#"s=${SHELL:-sh}; c='printf "\nwinc-path:%s\n" "$PATH"'
"$s" -l -c "$c" 2>/dev/null || "$s" -c "$c""#;

/// 各台主机登录 shell 的 `PATH`；问不出来的记为 `None`，不反复再问。
static PATHS: Mutex<Option<HashMap<Target, Option<String>>>> = Mutex::new(None);

/// 在 `dir` 所在的主机上运行 `exec`，返回它的标准输出；失败、超时或以非零状态退出时返回
/// `None`。
fn ask(dir: &Path, exec: &Exec) -> Option<String> {
    let deadline = Instant::now() + TIMEOUT;
    let output = remote::exec(dir, exec, &|| Instant::now() >= deadline).ok()?;
    (output.code == Some(0)).then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 返回 `dir` 所在的远端主机上用户登录 shell 的 `PATH`；`dir` 不在远端或问不出来时返回
/// `None`，这时命令沿用服务进程的环境。第一次调用要等远端应答，在后台线程上调用。
pub fn login_path(dir: &Path) -> Option<String> {
    let at = winc_backend::locate(dir)?;
    let known = |paths: &Option<HashMap<Target, Option<String>>>| {
        paths.as_ref().and_then(|all| all.get(&at.target).cloned())
    };
    if let Some(path) = known(&PATHS.lock().unwrap_or_else(PoisonError::into_inner)) {
        return path;
    }
    let root = winc_backend::virtual_path(&at.target, b"/");
    let output = ask(&root, &Exec::new("sh").arg("-c").arg(ASK));
    let path = output.and_then(|output| {
        let line = output.lines().rev().find_map(|l| l.strip_prefix(MARK))?;
        (!line.is_empty()).then(|| line.to_owned())
    });
    let mut paths = PATHS.lock().unwrap_or_else(PoisonError::into_inner);
    paths
        .get_or_insert_default()
        .insert(at.target, path.clone());
    path
}

/// 在 `dir` 所在的远端主机上按登录 shell 的 `PATH` 查找程序 `name`，返回它在那台主机上的
/// 路径；找不到时返回 `None`。每次调用等一次远端应答，在后台线程上调用。
pub fn find_program(dir: &Path, name: &str) -> Option<PathBuf> {
    let at = winc_backend::locate(dir)?;
    let root = winc_backend::virtual_path(&at.target, b"/");
    let mut exec = Exec::new("sh")
        .arg("-c")
        .arg(r#"command -v -- "$1""#)
        .arg("sh")
        .arg(name);
    if let Some(path) = login_path(dir) {
        exec = exec.env("PATH", path);
    }
    let found = ask(&root, &exec)?;
    // 别名、函数和内建命令输出的不是路径。
    let path = found.lines().next()?.trim();
    path.starts_with('/').then(|| PathBuf::from(path))
}
