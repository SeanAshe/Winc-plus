//! `git`、`ssh` 的输入请求通道：winc 自身作为 askpass 程序（`GIT_ASKPASS`、`SSH_ASKPASS`）运行。
//!
//! askpass 程序以提示文字为参数启动，把答复写到标准输出。窗口进程在一次 Git 操作期间监听一个本地
//! 端点（[`Listener`]），askpass 程序（[`helper`]）连接环境变量指定的端点，转发提示并等待答复。
//! winc 不写入脚本。用户配置的凭据助手优先；用户在对话框中勾选「记住密码」时，winc 作为排在最后
//! 的凭据助手（`git-credential` 协议）把验证通过的 HTTP(S) 凭据写入系统的凭据存储
//! （[`crate::credentials`]）。
//!
//! 请求依次为一个字节（0 为普通询问，1 为 `SSH_ASKPASS_PROMPT` 是 `confirm` 的询问，2 为凭据助手
//! 的请求）、`u32` 小端长度和文字。答复以一个字节开头（1 表示有内容，0 表示取消或没有结果），
//! 有内容时再跟 `u32` 小端长度和文字。Unix 套接字位于仅
//! 当前用户可访问的目录，命名管道使用默认安全描述符并拒绝来自其他主机的连接。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：凭据询问程序的脚本和通道在本机"
)]

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, PoisonError};

use winc_editor::git::{ASKPASS_ENDPOINT, Askpass, CREDENTIAL_HELPER_ARG};

use crate::credentials::{self, Credential};

/// 提示文字的长度上限（字节）。
///
/// 长度最大的已知提示是 `ssh` 首次连接时的主机密钥说明，通常为数百字节；上限用于拒绝格式错误的
/// 消息。
const MESSAGE_LIMIT: u32 = 64 << 10;
/// 答复的长度上限（字节）：访问令牌的长度远小于 1 MiB，超过上限的消息按格式错误处理。
const ANSWER_LIMIT: u32 = 1 << 20;

/// 请求的首字节：凭据助手的请求。
const CREDENTIAL: u8 = 2;

/// 同一进程内下一个端点的序号。
static NEXT: AtomicU64 = AtomicU64::new(0);

/// 无法识别的提示所需的输入方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// 明文输入。
    Visible,
    /// 遮盖输入内容；无法判断时使用此方式。
    Secret,
    /// 无需输入，只需确认。
    None,
}

/// 解析后的输入请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// HTTP(S) 用户名；`server` 是不含用户名的远程地址。
    Username { server: String },
    /// HTTP(S) 密码；`account` 是地址中的用户名。
    Password {
        server: String,
        account: Option<String>,
    },
    /// 私钥口令；`key` 是私钥文件的路径。
    Passphrase { key: String },
    /// SSH 登录密码。
    SshPassword { account: String, host: String },
    /// 首次连接时确认主机密钥。
    HostKey {
        host: String,
        key_type: String,
        fingerprint: String,
    },
    /// 无法识别的提示，原样显示。
    Other { text: String, input: InputKind },
}

impl Prompt {
    /// 解析提示文字；`confirm` 表示 `SSH_ASKPASS_PROMPT` 为 `confirm`。
    ///
    /// 依据 `git` 和 OpenSSH 的固定英文提示。文字与已知格式不完全相同时按 [`Prompt::Other`] 原样
    /// 显示，避免遗漏提示中的信息。
    fn parse(text: &str, confirm: bool) -> Prompt {
        let parsed = if confirm {
            None
        } else {
            parse_credential(text).or_else(|| parse_host_key(text))
        };
        parsed.unwrap_or_else(|| {
            let input = if confirm || text.contains("(yes/no") {
                InputKind::None
            } else if text.starts_with("Username") {
                InputKind::Visible
            } else {
                InputKind::Secret
            };
            Prompt::Other {
                text: text.trim_end().to_owned(),
                input,
            }
        })
    }

    /// 返回可以记住密码的登录请求的 `协议://主机`；其他请求返回 `None`。
    ///
    /// 只有 HTTP(S) 的凭据经凭据助手协议验证和保存。
    pub fn origin(&self) -> Option<String> {
        let (Prompt::Username { server } | Prompt::Password { server, .. }) = self else {
            return None;
        };
        let (scheme, _) = server.split_once("://")?;
        matches!(scheme, "http" | "https").then(|| format!("{scheme}://{}", host_of(server)))
    }

    /// 是否无需输入，只需确认。
    pub fn is_confirmation(&self) -> bool {
        matches!(
            self,
            Prompt::HostKey { .. }
                | Prompt::Other {
                    input: InputKind::None,
                    ..
                }
        )
    }
}

/// 解析用户名、密码和私钥口令的提示；不符合这些格式时返回 `None`。
fn parse_credential(text: &str) -> Option<Prompt> {
    let quoted = |prefix: &str| {
        text.strip_prefix(prefix)?
            .strip_suffix("': ")
            .map(str::to_owned)
    };
    if let Some(server) = quoted("Username for '") {
        return Some(Prompt::Username { server });
    }
    if let Some(url) = quoted("Password for '") {
        let (server, account) = split_account(&url);
        return Some(Prompt::Password { server, account });
    }
    if let Some(key) = quoted("Enter passphrase for key '") {
        return Some(Prompt::Passphrase { key });
    }
    let (account, host) = text.strip_suffix("'s password: ")?.split_once('@')?;
    Some(Prompt::SshPassword {
        account: account.to_owned(),
        host: host.to_owned(),
    })
}

/// 把 `scheme://用户@主机/路径` 拆成不含用户名的地址和用户名。
fn split_account(url: &str) -> (String, Option<String>) {
    let Some((scheme, rest)) = url.split_once("://") else {
        return (url.to_owned(), None);
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once('@') {
        Some((account, _)) => (
            format!("{scheme}://{}", &rest[account.len() + 1..]),
            Some(account.to_owned()),
        ),
        None => (url.to_owned(), None),
    }
}

/// 返回远程地址 `server` 的主机部分，用于对话框标题；不是 URL 时返回 `server` 本身。
pub fn host_of(server: &str) -> &str {
    let rest = server.split_once("://").map_or(server, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

/// 解析 OpenSSH 首次连接的主机密钥说明；存在其他内容（如主机的其他名称）时返回 `None`。
fn parse_host_key(text: &str) -> Option<Prompt> {
    let mut lines = text.trim_end().lines();
    let host = lines
        .next()?
        .strip_prefix("The authenticity of host '")?
        .strip_suffix("' can't be established.")?;
    // OpenSSH 的版本不同，这一行可能写作 `is SHA256:….` 或 `is: SHA256:…`。
    let (key_type, fingerprint) = lines.next()?.split_once(" key fingerprint is")?;
    let fingerprint = fingerprint.trim_start_matches(':').trim_start();
    let mut last = lines.next()?;
    if last == "This key is not known by any other names." {
        last = lines.next()?;
    }
    let complete = last.starts_with("Are you sure you want to continue connecting (yes/no");
    (complete && lines.next().is_none()).then(|| Prompt::HostKey {
        host: host.to_owned(),
        key_type: key_type.to_owned(),
        fingerprint: fingerprint.trim_end_matches('.').to_owned(),
    })
}

/// 一次输入请求。
#[derive(Debug)]
pub struct Question {
    /// 请求的内容。
    pub prompt: Prompt,
    /// 接收答复的通道；发送端被丢弃时视为取消。
    pub reply: Sender<String>,
    /// 用户选择记住密码时在答复前置位；`None` 表示该请求不提供此选项。
    pub remember: Option<Arc<AtomicBool>>,
}

/// 确认主机密钥时写给 `ssh` 的答复。
pub const CONFIRMED: &str = "yes";

/// 进程作为 askpass 程序或凭据助手启动时转发请求并返回退出码；普通启动时返回 `None`。
///
/// 环境变量只在 Git 操作的命令上设置，但钩子会继承该变量，因此同时要求参数的形式相符：恰好一个
/// 参数（提示文字），或 [`CREDENTIAL_HELPER_ARG`] 加一个动作。钩子中以其他参数运行 `winc` 仍按
/// 普通启动处理。
pub fn helper() -> Option<i32> {
    let endpoint = std::env::var_os(ASKPASS_ENDPOINT)?;
    let mut args = std::env::args_os().skip(1);
    let prompt = match (args.next()?, args.next(), args.next()) {
        (prompt, None, _) => prompt,
        (mark, Some(action), None) if mark == CREDENTIAL_HELPER_ARG => {
            return Some(credential_helper(&endpoint, &action.to_string_lossy()));
        }
        _ => return None,
    };
    let mode = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
    // `none` 表示通知（如请求触摸安全密钥），`ssh` 不读取答复，并在不再需要时结束该进程。
    if mode == "none" {
        return Some(0);
    }
    let prompt = prompt.to_string_lossy();
    let answer = imp::connect(&endpoint).and_then(|mut stream| {
        write_question(&mut stream, &prompt, mode == "confirm")?;
        read_answer(&mut stream)
    });
    let Ok(Some(answer)) = answer else {
        return Some(1);
    };
    let mut stdout = io::stdout();
    let written = writeln!(stdout, "{answer}").and_then(|()| stdout.flush());
    Some(i32::from(written.is_err()))
}

/// 作为凭据助手处理 `git` 的一次请求：把标准输入中的属性转发给窗口进程，输出其答复。
///
/// 只处理 `get`、`store`、`erase`。助手失败时 `git` 继续使用其他方式，因此退出码恒为 0。
fn credential_helper(endpoint: &OsStr, action: &str) -> i32 {
    if !matches!(action, "get" | "store" | "erase") {
        return 0;
    }
    let mut request = format!("{action}\n");
    let limit = u64::from(ANSWER_LIMIT);
    if io::stdin()
        .take(limit)
        .read_to_string(&mut request)
        .is_err()
    {
        return 0;
    }
    let answer = imp::connect(endpoint).and_then(|mut stream| {
        stream.write_all(&[CREDENTIAL])?;
        write_text(&mut stream, &request)?;
        stream.flush()?;
        read_answer(&mut stream)
    });
    if let Ok(Some(answer)) = answer {
        let mut stdout = io::stdout();
        let _ = stdout
            .write_all(answer.as_bytes())
            .and_then(|()| stdout.flush());
    }
    0
}

/// 处理凭据助手的请求 `request`（动作一行，其后是 `git-credential` 的属性），返回写给 `git` 的
/// 属性；没有结果时返回 `None`。
///
/// `kept` 是本次操作中用户选择记住的 `协议://主机`：`git` 把验证通过的凭据交给每一个助手保存，
/// 包括其他助手提供的凭据，这里只保存用户选择记住的。
fn credential(request: &str, kept: &Mutex<Vec<String>>) -> Option<String> {
    let mut lines = request.lines();
    let action = lines.next()?;
    let attribute = |name: &str| {
        (request.lines().skip(1))
            .filter_map(|line| line.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    };
    let protocol = attribute("protocol")?;
    if !matches!(protocol, "http" | "https") {
        return None;
    }
    let origin = format!("{protocol}://{}", attribute("host")?);
    let username = attribute("username");
    match action {
        "get" => {
            let found = credentials::find(&origin, username)?;
            Some(format!(
                "username={}\npassword={}\n",
                found.username, found.password
            ))
        }
        "store" => {
            let chosen = kept
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&origin);
            if chosen {
                credentials::save(
                    &origin,
                    &Credential {
                        username: username?.to_owned(),
                        password: attribute("password")?.to_owned(),
                    },
                );
            }
            None
        }
        "erase" => {
            credentials::forget(&origin, username);
            None
        }
        _ => None,
    }
}

/// 写一段带长度前缀的文字。
fn write_text(stream: &mut impl Write, text: &str) -> io::Result<()> {
    let length = u32::try_from(text.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    stream.write_all(&length.to_le_bytes())?;
    stream.write_all(text.as_bytes())
}

/// 读一段带长度前缀的文字，超过 `limit` 字节时报错。
fn read_text(stream: &mut impl Read, limit: u32) -> io::Result<String> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length);
    if length > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; length as usize];
    stream.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}

fn write_question(stream: &mut impl Write, prompt: &str, confirm: bool) -> io::Result<()> {
    stream.write_all(&[u8::from(confirm)])?;
    write_text(stream, prompt)?;
    stream.flush()
}

fn read_answer(stream: &mut impl Read) -> io::Result<Option<String>> {
    let mut answered = [0];
    stream.read_exact(&mut answered)?;
    if answered[0] == 0 {
        return Ok(None);
    }
    read_text(stream, ANSWER_LIMIT).map(Some)
}

/// 处理一个连接：读取请求并交给 `ask`，收到答复或 [`Question::reply`] 被丢弃后写回结果。
///
/// `kept` 不为 `None` 时处理凭据助手的请求，并为 HTTP(S) 的登录请求提供记住密码的选项。
fn serve(
    stream: &mut (impl Read + Write),
    ask: &impl Fn(Question),
    kept: Option<&Mutex<Vec<String>>>,
) -> io::Result<()> {
    let mut kind = [0];
    stream.read_exact(&mut kind)?;
    if kind[0] == CREDENTIAL {
        let request = read_text(stream, ANSWER_LIMIT)?;
        match kept.and_then(|kept| credential(&request, kept)) {
            Some(answer) => {
                stream.write_all(&[1])?;
                write_text(stream, &answer)?;
            }
            None => stream.write_all(&[0])?,
        }
        return stream.flush();
    }
    let prompt = Prompt::parse(&read_text(stream, MESSAGE_LIMIT)?, kind[0] != 0);
    let origin = kept.and(prompt.origin());
    let remember = origin.as_ref().map(|_| Arc::new(AtomicBool::new(false)));
    let (reply, answered) = channel();
    ask(Question {
        prompt,
        reply,
        remember: remember.clone(),
    });
    match answered.recv() {
        Ok(answer) => {
            if let (Some(kept), Some(origin)) = (kept, origin)
                && remember.is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                kept.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(origin);
            }
            stream.write_all(&[1])?;
            write_text(stream, &answer)?;
        }
        Err(_) => stream.write_all(&[0])?,
    }
    stream.flush()
}

/// 一次 Git 操作期间接收输入请求的本地端点，丢弃时关闭。
pub struct Listener {
    endpoint: OsString,
    stop: Arc<AtomicBool>,
    /// 是否作为凭据助手保存和提供凭据。
    remember: bool,
}

impl Listener {
    /// 创建端点，每收到一个请求就在后台线程中调用 `ask`。
    ///
    /// `remember` 表示是否作为凭据助手保存和提供凭据。无法创建端点时返回 `None`；操作仍然执行，
    /// 需要输入时由 `git` 报错。
    pub fn bind(remember: bool, ask: impl Fn(Question) + Send + 'static) -> Option<Listener> {
        let stop = Arc::new(AtomicBool::new(false));
        let kept = remember.then(Mutex::default);
        let endpoint = imp::listen(stop.clone(), ask, kept)?;
        Some(Listener {
            endpoint,
            stop,
            remember,
        })
    }

    /// 返回传给 `git` 的 askpass 配置；无法取得当前程序的路径时返回 `None`。
    pub fn askpass(&self) -> Option<Askpass> {
        Some(Askpass {
            program: std::env::current_exe().ok()?,
            endpoint: self.endpoint.clone(),
            credential_helper: self.remember,
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        imp::close(&self.endpoint);
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    use super::*;

    pub(super) fn connect(endpoint: &OsStr) -> io::Result<UnixStream> {
        UnixStream::connect(endpoint)
    }

    pub(super) fn listen(
        stop: Arc<AtomicBool>,
        ask: impl Fn(Question) + Send + 'static,
        kept: Option<Mutex<Vec<String>>>,
    ) -> Option<OsString> {
        let name = format!(
            "askpass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = crate::instance::private_file(&name, "sock")?;
        let listener = UnixListener::bind(&path).ok()?;
        // 权限设为 0600，只允许当前用户连接。
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        std::thread::Builder::new()
            .name("winc-askpass".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(mut stream) = stream {
                        let _ = serve(&mut stream, &ask, kept.as_ref());
                    }
                }
            })
            .ok()?;
        Some(path.into_os_string())
    }

    /// 连接一次以唤醒阻塞在 `accept` 上的监听线程，使其读取停止标志，之后移除套接字文件。
    pub(super) fn close(endpoint: &OsStr) {
        let _ = UnixStream::connect(endpoint);
        let _ = std::fs::remove_file(endpoint);
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;

    use super::*;
    use crate::instance::{connect_pipe, create_pipe};

    pub(super) fn connect(endpoint: &OsStr) -> io::Result<File> {
        File::options().read(true).write(true).open(endpoint)
    }

    pub(super) fn listen(
        stop: Arc<AtomicBool>,
        ask: impl Fn(Question) + Send + 'static,
        kept: Option<Mutex<Vec<String>>>,
    ) -> Option<OsString> {
        let user = std::env::var("USERNAME").unwrap_or_default();
        let endpoint = OsString::from(format!(
            r"\\.\pipe\winc-{user}-askpass-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let name: Vec<u16> = endpoint.encode_wide().chain([0]).collect();
        let mut next = create_pipe(&name, true)?;
        std::thread::Builder::new()
            .name("winc-askpass".into())
            .spawn(move || {
                loop {
                    let connected = connect_pipe(&next);
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(waiting) = create_pipe(&name, false) else {
                        break;
                    };
                    let mut stream = std::mem::replace(&mut next, waiting);
                    if connected {
                        let _ = serve(&mut stream, &ask, kept.as_ref());
                    }
                }
            })
            .ok()?;
        Some(endpoint)
    }

    /// 连接一次以唤醒阻塞在 `ConnectNamedPipe` 上的监听线程，使其读取停止标志。
    pub(super) fn close(endpoint: &OsStr) {
        let _ = File::options().read(true).write(true).open(endpoint);
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::*;

    pub(super) fn connect(_endpoint: &OsStr) -> io::Result<io::Empty> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub(super) fn listen(
        _stop: Arc<AtomicBool>,
        _ask: impl Fn(Question) + Send + 'static,
        _kept: Option<Mutex<Vec<String>>>,
    ) -> Option<OsString> {
        None
    }

    pub(super) fn close(_endpoint: &OsStr) {}
}

#[cfg(test)]
mod tests;
