//! 运行外部程序（格式化工具、`git`）所用的环境变量：从图形界面启动的应用只继承系统提供的最小
//! 环境，因此要向用户的登录 shell 查询。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：读取本机 shell 的环境"
)]

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::process;

/// 环境变量列表，元素为（名字，值）。
pub(super) type Vars = Vec<(OsString, OsString)>;

/// 等待登录 shell 输出环境的最长时间（5 秒，经验值）。
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
/// 环境输出前后的标记。
///
/// `.zshrc` 等启动脚本可能打印其他内容，只取两个标记之间的部分。
const MARKER: &str = "__WINC_ENVIRONMENT__";
/// 需要排除的变量：`PWD`、`OLDPWD`、`SHLVL`、`_` 是 shell 自身的状态，
/// `WINC_RESOLVING_ENVIRONMENT` 是本程序设置的标志，都不属于用户的环境配置。
const SKIP: [&str; 5] = ["PWD", "OLDPWD", "SHLVL", "_", "WINC_RESOLVING_ENVIRONMENT"];

/// 返回运行外部程序时使用的环境变量。
///
/// 进程设置了 `TERM`（从命令行启动）时，直接使用继承来的环境；否则运行一次交互式登录 shell，
/// 取它打印出的环境：nvm、pyenv 这类版本的初始化通常写在 `.zshrc` 里，只有交互式 shell 才会读取。
/// Windows 上图形程序继承的环境是完整的，同样直接使用。运行 shell 最多等待 [`SHELL_TIMEOUT`]，
/// 取不到时回退到继承来的环境，并在 `PATH` 之后补上存在的常见安装目录。
pub(super) fn resolve() -> Vars {
    // Windows 上图形程序继承的就是完整环境，没有登录 shell 的概念。
    if cfg!(not(unix)) || std::env::var_os("TERM").is_some() {
        return std::env::vars_os().collect();
    }

    let shell = std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .or_else(account_shell)
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    match login_shell(&shell, SHELL_TIMEOUT) {
        Ok(vars) => vars,
        Err(e) => {
            eprintln!("winc: 无法取得登录 shell 的环境（{e}），外部程序按默认的 PATH 找");
            fallback()
        }
    }
}

/// 运行交互式登录 shell `shell` 打印环境，最多等待 `timeout`，返回解析后的环境变量。
///
/// # Errors
///
/// shell 无法运行、超时，或输出中没有可用的环境（见 `parse`）时，返回错误说明。
fn login_shell(shell: &OsStr, timeout: Duration) -> Result<Vars, String> {
    let script = print_script();
    let mut command = Command::new(shell);
    command
        .args(["-i", "-l", "-c", &script])
        .env("WINC_RESOLVING_ENVIRONMENT", "1")
        .stderr(Stdio::null());

    // 没有输入，`process::run` 把标准输入接到空设备：`.zshrc` 中等待输入的提示会直接读到文件
    // 结束，不会阻塞。
    let output = process::run(&mut command, Vec::new(), timeout, &Default::default())
        .map_err(|e| e.to_string())?;
    parse(&output.stdout).ok_or_else(|| "shell 未输出环境变量".to_owned())
}

/// 返回让 shell 打印环境的脚本，输出由 [`path_in`] 或 [`parse`] 解析。
pub(crate) fn print_script() -> String {
    format!("printf '{MARKER}'; command env -0; printf '{MARKER}'")
}

/// 返回 [`print_script`] 的输出 `output` 中 `PATH` 的值；没有可用的环境时返回 `None`。
pub(crate) fn path_in(output: &[u8]) -> Option<String> {
    let vars = parse(output)?;
    let (_, path) = vars.into_iter().find(|(name, _)| name == "PATH")?;
    path.into_string().ok()
}

/// 返回 `env -0` 输出中两个标记之间的环境变量；缺少标记，或环境中没有 `PATH` 时返回 `None`。
///
/// 条目以 NUL 分隔，因此值中可以含换行；名字为空或在 [`SKIP`] 中的条目被丢弃。
fn parse(output: &[u8]) -> Option<Vars> {
    let marker = MARKER.as_bytes();
    let start = find(output, marker)? + marker.len();
    let end = start + find(&output[start..], marker)?;

    let vars = output[start..end]
        .split(|&b| b == 0)
        .filter_map(|entry| {
            let eq = entry.iter().position(|&b| b == b'=')?;
            let (name, value) = (&entry[..eq], &entry[eq + 1..]);
            let name = std::str::from_utf8(name).ok()?;
            (!name.is_empty() && !SKIP.contains(&name)).then(|| {
                (
                    OsString::from(name),
                    OsString::from(String::from_utf8_lossy(value).into_owned()),
                )
            })
        })
        .collect::<Vars>();
    vars.iter().any(|(name, _)| name == "PATH").then_some(vars)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// 返回用户数据库中当前用户的登录 shell；查不到或为空时返回 `None`。
///
/// 没有 `SHELL` 变量（由不设置它的程序启动）时使用。
#[cfg(unix)]
#[allow(unsafe_code)] // 读取用户数据库只能用 libc 的 `getpwuid_r`
fn account_shell() -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt;
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut found = std::ptr::null_mut();
    // SAFETY: 结构体和缓冲区都来自本地变量，指针有效且按 `libc::c_char` 对齐，长度如实给出；
    // `getpwuid_r` 只写入它们和 `found`，可重入，不涉及共享的静态区。
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    // SAFETY: 调用成功时结构体已填好，`pw_shell` 为空指针或指向 `buf` 中以 NUL 结尾的字符串。
    let shell = unsafe {
        let entry = entry.assume_init();
        if entry.pw_shell.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(entry.pw_shell).to_bytes().to_vec()
    };
    (!shell.is_empty()).then(|| OsStr::from_bytes(&shell).to_os_string())
}

#[cfg(not(unix))]
fn account_shell() -> Option<OsString> {
    None
}

/// 返回回退用的环境：继承来的环境，并在 `PATH` 之后补上存在且尚未包含的常见安装目录。
///
/// 补充的目录为 `/opt/homebrew/bin`、`/usr/local/bin`、`$HOME/.cargo/bin` 和 `$HOME/go/bin`。
fn fallback() -> Vars {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let extra = [
        Some(PathBuf::from("/opt/homebrew/bin")),
        Some(PathBuf::from("/usr/local/bin")),
        home.as_ref().map(|h| h.join(".cargo/bin")),
        home.as_ref().map(|h| h.join("go/bin")),
    ];

    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for dir in extra.into_iter().flatten() {
        if dir.is_dir() && !paths.contains(&dir) {
            paths.push(dir);
        }
    }

    let mut vars: Vars = std::env::vars_os()
        .filter(|(name, _)| name != "PATH")
        .collect();
    if let Ok(path) = std::env::join_paths(paths) {
        vars.push((OsString::from("PATH"), path));
    }
    vars
}

#[cfg(test)]
mod tests;
