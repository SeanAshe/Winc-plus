//! shell 集成的注入：不改用户的配置文件，只换几个环境变量或参数，让 shell 先读 winc 的脚本，
//! 脚本里再读取用户自己的配置。
//!
//! zsh 换 `ZDOTDIR`，bash 用 `--rcfile`，fish 把 winc 的目录加到 `XDG_DATA_DIRS`。脚本编进辅助
//! 进程，启动前内容不一致时（旧版本留下的、被改过的）重写；写不进去或不认识的 shell 不注入，只是
//! 没有命令块。
//!
//! 每个 shell 另外获得一个随机数（`WINC_NONCE`，脚本读取后即删除），报告命令行时附带该随机数。
//!
//! Windows 上仅注入 WSL 发行版的默认 shell（`inject_wsl`）。脚本保存在 Windows 的配置目录，发行版
//! 通过 `wslpath` 换算路径后读取，因此不在发行版内写入文件。

use std::fs;
use std::io;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use winc_pty::scripts::{BASH, FISH, ZSH};

/// 在 WSL 发行版内启动 shell 并注入集成脚本的脚本。
#[cfg(any(windows, test))]
const WSL: &[(&str, &str)] = winc_pty::scripts::LAUNCH;

/// 在发行版内运行 `WSL` 脚本的命令，`$0` 是脚本目录的 Windows 路径。
///
/// 命令写成单行，避免换行符经 `wsl.exe` 的命令行传递时发生变化。脚本不可读时（如发行版未挂载
/// Windows 磁盘）启动不带集成的 shell，终端仍可使用。
#[cfg(any(windows, test))]
const WSL_RUN: &str = r#"d=$(wslpath -u "$0" 2>/dev/null) && [ -r "$d/wsl.sh" ] && exec sh "$d/wsl.sh" "$d"; unset WINC_NONCE; exec "${SHELL:-/bin/sh}" -l"#;

/// 通过 `WSLENV` 从 Windows 传给发行版的环境变量：集成脚本用到的变量，以及程序据此判断终端能力的
/// 变量。
#[cfg(any(windows, test))]
const WSL_ENV: &[u8] = b"WINC_NONCE:WINC_PROMPT:COLORTERM:TERM_PROGRAM:TERM_PROGRAM_VERSION";

/// 环境变量表，每项是（键，值）。
type Env = Vec<(Vec<u8>, Vec<u8>)>;

/// 按 shell 的名字（`program` 的最后一段）准备注入：写好脚本，改 `env`、`args`、`login`。
#[cfg(unix)]
///
/// 返回脚本报告命令行时附带的随机数，没有注入时返回 `None`。只给不带参数的 shell 注入；带了
/// 参数（`-c`、脚本文件）或脚本目录 `dir` 为空时不注入。
pub fn inject(
    dir: &[u8],
    program: &[u8],
    args: &mut Vec<Vec<u8>>,
    login: &mut bool,
    env: &mut Env,
) -> Option<String> {
    if !args.is_empty() || dir.is_empty() {
        return None;
    }
    let dir = Path::new(std::ffi::OsStr::from_bytes(dir));
    let name = program.rsplit(|&b| b == b'/').next().unwrap_or(program);
    let scripts = match name {
        b"zsh" => ZSH,
        b"bash" => BASH,
        b"fish" => FISH,
        _ => return None,
    };
    if let Err(error) = write_scripts(dir, scripts) {
        eprintln!(
            "winc-pty: 无法写入 shell 集成脚本（{}）：{error}",
            dir.display()
        );
        return None;
    }
    let nonce = match nonce() {
        Ok(nonce) => nonce,
        Err(error) => {
            eprintln!("winc-pty: 无法生成 shell 集成的随机数：{error}");
            return None;
        }
    };
    set(env, b"WINC_NONCE", nonce.clone().into_bytes());
    let path = |relative: &str| dir.join(relative).as_os_str().as_bytes().to_vec();

    match name {
        b"zsh" => {
            if let Some(old) = take(env, b"ZDOTDIR") {
                set(env, b"WINC_ZDOTDIR", old);
            }
            set(env, b"ZDOTDIR", path("zsh"));
        }
        b"bash" => {
            // 登录 shell 不读 --rcfile，改成非登录启动，脚本里替它读登录 shell 的文件。
            if *login {
                *login = false;
                set(env, b"WINC_BASH_LOGIN", b"1".to_vec());
            }
            args.extend([b"--rcfile".to_vec(), path("bash/winc.bash")]);
        }
        _ => {
            let mut dirs = path("fish");
            dirs.push(b':');
            match take(env, b"XDG_DATA_DIRS") {
                Some(old) => {
                    dirs.extend_from_slice(&old);
                    set(env, b"WINC_XDG_DATA_DIRS", old);
                }
                // 没设时用 fish 的默认值；winc 的目录加在前面，默认值仍然保留。
                None => dirs.extend_from_slice(b"/usr/local/share:/usr/share"),
            }
            set(env, b"XDG_DATA_DIRS", dirs);
        }
    }
    Some(nonce)
}

/// 为经 `ssh` 在远端主机上启动的 shell 准备注入：`args` 的最后一项（在远端运行的命令）里有
/// [`winc_pty::scripts::REMOTE_NONCE`] 时，把最后一处换成新生成的随机数并返回它。
///
/// 脚本已由 winc 经它自己的连接写到远端；命令里没有该记号（没有启用集成、脚本没有写好）时
/// 不注入，返回 `None`。
pub fn inject_remote(program: &[u8], args: &mut [Vec<u8>]) -> Option<String> {
    let name = program
        .rsplit(|&b| b == b'/' || b == b'\\')
        .next()
        .unwrap_or(program);
    if !name.eq_ignore_ascii_case(b"ssh") && !name.eq_ignore_ascii_case(b"ssh.exe") {
        return None;
    }
    let mark = winc_pty::scripts::REMOTE_NONCE.as_bytes();
    let command = args.last_mut()?;
    let at = command.windows(mark.len()).rposition(|part| part == mark)?;
    let nonce = match nonce() {
        Ok(nonce) => nonce,
        Err(error) => {
            eprintln!("winc-pty: 无法生成 shell 集成的随机数：{error}");
            return None;
        }
    };
    command.splice(at..at + mark.len(), nonce.bytes());
    Some(nonce)
}

/// 为 WSL 发行版的默认 shell 准备注入：写入脚本，并修改 `args`、`env`。
///
/// 返回脚本报告命令行时附带的随机数。`program` 和 `args` 不是进入默认 shell 的 `wsl.exe`，或脚本
/// 写入失败时不注入，返回 `None`。
#[cfg(windows)]
pub fn inject_wsl(
    dir: &[u8],
    program: &[u8],
    args: &mut Vec<Vec<u8>>,
    env: &mut Env,
) -> Option<String> {
    if dir.is_empty() || !opens_default_wsl_shell(program, args) {
        return None;
    }
    let text = String::from_utf8_lossy(dir).into_owned();
    let path = Path::new(&text);
    for scripts in [ZSH, BASH, FISH, WSL] {
        if let Err(error) = write_scripts(path, scripts) {
            eprintln!("winc-pty: 无法写入 shell 集成脚本（{text}）：{error}");
            return None;
        }
    }
    let nonce = match nonce() {
        Ok(nonce) => nonce,
        Err(error) => {
            eprintln!("winc-pty: 无法生成 shell 集成的随机数：{error}");
            return None;
        }
    };
    set(env, b"WINC_NONCE", nonce.clone().into_bytes());
    args.extend(
        ["--exec", "sh", "-c", WSL_RUN, &text]
            .iter()
            .map(|arg| arg.as_bytes().to_vec()),
    );
    Some(nonce)
}

/// `program` 和 `args` 是否仅表示进入发行版的默认 shell；带其他选项或要运行的命令时返回 `false`。
#[cfg(any(windows, test))]
fn opens_default_wsl_shell(program: &[u8], args: &[Vec<u8>]) -> bool {
    is_wsl(program)
        && args.chunks(2).all(|pair| {
            pair.len() == 2
                && matches!(
                    pair[0].as_slice(),
                    b"-d" | b"--distribution" | b"--cd" | b"-u" | b"--user"
                )
        })
}

/// `program` 是否是 `wsl.exe`。
#[cfg(any(windows, test))]
pub fn is_wsl(program: &[u8]) -> bool {
    let name = program
        .rsplit(|&b| b == b'/' || b == b'\\')
        .next()
        .unwrap_or(program);
    name.eq_ignore_ascii_case(b"wsl.exe") || name.eq_ignore_ascii_case(b"wsl")
}

/// 把 `WSL_ENV` 追加到 `WSLENV`，并保留已有的值：`wsl.exe` 只向发行版传递其中列出的变量。
#[cfg(any(windows, test))]
pub fn forward_to_wsl(env: &mut Env) {
    let mut list = take(env, b"WSLENV").unwrap_or_default();
    if !list.is_empty() {
        list.push(b':');
    }
    list.extend_from_slice(WSL_ENV);
    set(env, b"WSLENV", list);
}

/// 生成 128 位的随机数，写成 32 个十六进制数字的字符串。
fn nonce() -> io::Result<String> {
    let bytes = random_bytes()?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
fn random_bytes() -> io::Result<[u8; 16]> {
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(windows)]
#[allow(unsafe_code)] // 只能通过系统接口取得系统随机数
fn random_bytes() -> io::Result<[u8; 16]> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut bytes = [0; 16];
    // SAFETY: 缓冲区是本地数组，长度与实际一致；算法句柄为空时要求传入此标志，由系统选择算法。
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(io::Error::other(format!("BCryptGenRandom: {status:#x}")));
    }
    Ok(bytes)
}

/// 内容不同的脚本重写一遍。
///
/// 先写入同目录的临时文件再改名，同时启动的 shell 不会读到未写完的脚本。换行符统一为 LF：源码在
/// Windows 上可能以 CRLF 检出，而这些脚本由 Linux 上的 shell 读取。
fn write_scripts(dir: &Path, scripts: &[(&str, &str)]) -> io::Result<()> {
    for (relative, content) in scripts {
        let path = dir.join(relative);
        let content = content.replace("\r\n", "\n");
        if fs::read(&path).is_ok_and(|old| old == content.as_bytes()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&temp, content)?;
        fs::rename(&temp, &path)?;
    }
    Ok(())
}

fn take(env: &mut Env, key: &[u8]) -> Option<Vec<u8>> {
    let i = env.iter().position(|(k, _)| k == key)?;
    Some(env.remove(i).1)
}

fn set(env: &mut Env, key: &[u8], value: Vec<u8>) {
    take(env, key);
    env.push((key.to_vec(), value));
}

#[cfg(test)]
mod tests;
