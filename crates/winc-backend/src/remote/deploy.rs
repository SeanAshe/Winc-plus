//! 部署：在一条连接上确认远端主机有与本机一致的服务进程，没有时上传，然后启动它。
//!
//! 连接的子进程先在远端运行一段引导脚本：脚本报告主机的系统和架构，在上传目录中查找对应的
//! 服务进程，缺少时从标准输入接收并保存，最后以服务进程替换自身。此后同一对标准输入输出上
//! 就是请求和应答，不需要第二次登录。
//!
//! 服务进程保存在 `<上传目录>/<内容摘要>/winc-server`：不同版本、不同架构的程序摘要不同，
//! 互不覆盖。每次连接只保留用上的这一个版本，其余的随即删除，上传目录不会越积越多；多台
//! 架构不同的主机共用一个主目录时，各自连接时重新上传自己的版本。

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::{Connection, Ssh, quote, ssh};
use crate::Target;

/// 引导脚本，交给远端的 `sh -c` 运行。参数依次是上传目录，以及每个服务进程的系统、摘要和字节数。
///
/// 脚本经用户的登录 shell 传给 `sh`，因此写成一行，不含单引号和反斜杠：各种登录 shell 对
/// 单引号内的换行和反斜杠处理不一致。`!` 之后只能是空格或 `=`：有的登录 shell 在单引号内仍把
/// `!` 加其他字符当作历史命令展开。输出的每一行以 `winc:` 开头，与登录脚本的输出区分。
/// 上传的内容先写入临时文件，字节数相符后才改名，中断的上传不会留下不完整的程序。
/// 已有的程序字节数与本机的不符（文件损坏）时同样重新上传，不必由用户去删除。
///
/// 启动之前清理上传目录中其他版本的程序：只保留即将运行的这一个，其他版本再次需要时重新
/// 上传。只删除名字是摘要（小写十六进制）、其中只有 `winc-server` 一个文件的子目录，先删文件
/// 再删空目录，不递归删除；目录中有其他内容时不动。正在运行的旧版本不受影响：删除的只是
/// 目录项。
const SCRIPT: &str = concat!(
    "dir=$1; shift; sys=$(uname -s)-$(uname -m); digest=; ",
    "while [ $# -ge 3 ]; do ",
    "if [ \"$1\" = \"$sys\" ]; then digest=$2; size=$3; fi; shift 3; ",
    "done; ",
    "if [ -z \"$digest\" ]; then echo \"winc:unsupported $sys\"; exit 0; fi; ",
    "case $dir in ",
    "\"\") dir=$HOME/.winc-server ;; ",
    "\"~/\"*) dir=$HOME/${dir#\"~/\"} ;; ",
    "esac; ",
    "bin=$dir/$digest/winc-server; ",
    "if [ ! -x \"$bin\" ] || [ $(wc -c < \"$bin\") -ne \"$size\" ]; then ",
    "echo \"winc:need $sys\"; umask 077; ",
    "mkdir -p \"$dir/$digest\" && tmp=$bin.$$ && head -c \"$size\" > \"$tmp\" && ",
    "[ $(wc -c < \"$tmp\") -eq \"$size\" ] && chmod 700 \"$tmp\" && mv -f \"$tmp\" \"$bin\" || ",
    "{ rm -f \"$tmp\"; echo \"winc:failed\"; exit 1; }; ",
    "fi; ",
    "for old in \"$dir\"/*; do name=${old##*/}; ",
    "rest=$(printf %s \"$name\" | tr -d 0-9a-f); ",
    "if [ -z \"$rest\" ] && [ \"$name\" != \"$digest\" ] && ",
    "[ \"$(ls -A \"$old\")\" = winc-server ]; then ",
    "rm -f \"$old/winc-server\" && rmdir \"$old\"; fi; ",
    "done 2>/dev/null; ",
    "echo \"winc:ready\"; exec \"$bin\"",
);

/// 脚本输出行的前缀。
const MARKER: &[u8] = b"winc:";

/// 等到脚本的一行输出之前，至多跳过的字节数（64 KiB）。
///
/// 登录脚本可能先输出若干行问候语；超过这个数量仍没有脚本的输出，判定对端没有运行脚本，
/// 不再继续读取。
const PREAMBLE_MAX: usize = 64 << 10;

/// 保留的错误输出的字节数（4 KiB）：`ssh` 和脚本的错误是若干行文字，只保留末尾的部分。
const MESSAGE_MAX: usize = 4 << 10;

/// 子进程结束后等待其错误输出读完的最长时间。
///
/// 错误输出通常随子进程结束而关闭；`ssh` 留在后台的共用连接进程可能继续持有它，不能无限等待。
const MESSAGE_WAIT: Duration = Duration::from_millis(500);

/// 一个系统上的服务进程。
#[derive(Clone, Debug)]
pub struct Program {
    /// 系统和架构，写法是远端 `uname -s` 与 `uname -m` 的输出以 `-` 相连，例如 `Linux-x86_64`。
    pub system: String,
    /// 内容的摘要，小写十六进制；用作远端的目录名。
    pub digest: String,
    /// 程序的内容。
    pub content: Vec<u8>,
}

/// 部署所需的全部内容。
#[derive(Clone, Debug, Default)]
pub struct Deploy {
    /// 远端的上传目录；空表示 `~/.winc-server`，以 `~/` 开头的相对于远端的主目录。
    pub dir: String,
    /// 各个系统上的服务进程。
    pub programs: Vec<Program>,
}

impl Deploy {
    /// 返回在远端主机上运行的引导命令，每个元素是一个参数。
    ///
    /// # Errors
    ///
    /// 上传目录含反斜杠或控制字符、系统名含空白，或摘要不是小写十六进制时返回
    /// [`io::ErrorKind::InvalidInput`]：这些内容会成为远端命令的参数和目录名。
    pub fn command(&self) -> io::Result<Vec<String>> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
        if self.dir.chars().any(|c| c == '\\' || c.is_control()) {
            return Err(invalid("server directory has unsupported characters"));
        }
        let mut words = vec!["sh".to_owned(), "-c".to_owned(), SCRIPT.to_owned()];
        // `sh -c` 之后的第一个参数是 `$0`。
        words.extend(["winc".to_owned(), self.dir.clone()]);
        for program in &self.programs {
            let system = &program.system;
            if system.is_empty() || !system.chars().all(|c| c.is_ascii_graphic() && c != '\\') {
                return Err(invalid("system name has unsupported characters"));
            }
            let digest = &program.digest;
            if digest.is_empty()
                || !digest
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(invalid("digest is not lowercase hexadecimal"));
            }
            words.extend([
                system.clone(),
                digest.clone(),
                program.content.len().to_string(),
            ]);
        }
        Ok(words)
    }

    /// 返回经 `ssh` 连接 `target` 并运行引导命令的命令。
    ///
    /// `ssh` 的参数与 [`ssh_command`](super::ssh_command) 相同；引导命令的每个参数各自作为
    /// 一个词交给远端的 shell。
    ///
    /// # Errors
    ///
    /// 同 [`Deploy::command`]。
    pub fn ssh_command(&self, target: &Target, options: Ssh) -> io::Result<Command> {
        let mut command = ssh(target, options);
        command.args(self.command()?.iter().map(|word| quote(word)));
        Ok(command)
    }
}

/// 建立连接失败的原因。
#[derive(Debug)]
pub enum ConnectError {
    /// 远端主机的系统或架构没有对应的服务进程；内容是远端报告的系统名。
    Unsupported(String),
    /// 子进程没有启动服务进程就结束了，例如登录失败或无法写入上传目录；内容是它的错误输出，
    /// 可能为空。
    Failed(String),
    /// 无法启动子进程，或启动服务进程后交换协议版本失败。
    Io(io::Error),
    /// 调用方中止了连接（见 [`Abort`]）。
    Aborted,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(system) => write!(f, "unsupported remote system: {system}"),
            Self::Failed(message) if message.is_empty() => f.write_str("connection failed"),
            Self::Failed(message) => f.write_str(message),
            Self::Io(error) => error.fmt(f),
            Self::Aborted => f.write_str("connection aborted"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Unsupported(_) | Self::Failed(_) | Self::Aborted => None,
        }
    }
}

impl From<io::Error> for ConnectError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// 中止正在进行的连接：结束承载它的子进程，[`Connection::deploy`] 随即返回
/// [`ConnectError::Aborted`]。可以在连接开始之前、进行当中或结束之后调用，可以调用多次。
///
/// 连接建立之后仍然有效：调用方在连接上还有收尾的请求（例如取得目录的路径）时，中止使连接
/// 断开，这些请求随即失败。调用方用完之后丢弃它，连接此后不再受它影响。
#[derive(Clone, Default)]
pub struct Abort(Arc<Mutex<Aborting>>);

impl std::fmt::Debug for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Abort").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Aborting {
    aborted: bool,
    /// 引导阶段的子进程；服务进程启动后交给连接。
    child: Option<Child>,
    /// 服务进程启动之后的连接。
    link: Option<Arc<super::Link>>,
}

impl Abort {
    /// 中止连接：引导阶段结束子进程，之后断开连接。
    pub fn abort(&self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.aborted = true;
        if let Some(mut child) = state.child.take() {
            abandon(&mut child);
        }
        if let Some(link) = state.link.take() {
            link.kill();
        }
    }

    /// 是否已中止。
    pub fn is_aborted(&self) -> bool {
        (self.0.lock().unwrap_or_else(PoisonError::into_inner)).aborted
    }

    /// 交出子进程；已被中止（子进程已结束）时返回 `None`。
    fn take(&self) -> Option<Child> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.child.take().filter(|_| !state.aborted)
    }

    /// 结束并回收子进程。
    fn end(&self) {
        let child = (self.0.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .child
            .take();
        if let Some(mut child) = child {
            abandon(&mut child);
        }
    }
}

impl Connection {
    /// 启动 `command` 作为连接，由它运行 `deploy` 的引导命令（见 [`Deploy::ssh_command`]），
    /// 按需上传服务进程，并与启动后的服务进程交换协议版本。
    ///
    /// `command` 的标准输入输出接到本进程；标准错误只用于在失败时说明原因。引导阶段没有时限
    /// （登录可能在等待用户输入），调用方用 `abort` 中止。
    ///
    /// # Errors
    ///
    /// 见 [`ConnectError`]。失败时子进程已经结束。
    pub fn deploy(command: Command, deploy: &Deploy, abort: &Abort) -> Result<Self, ConnectError> {
        Self::deploy_reporting(command, deploy, abort, &|_| {})
    }

    /// 同 [`Connection::deploy`]，并在进入登录之后的各个阶段时调用 `report`（在调用本函数的
    /// 线程上）。
    ///
    /// # Errors
    ///
    /// 见 [`ConnectError`]。失败时子进程已经结束。
    pub fn deploy_reporting(
        mut command: Command,
        deploy: &Deploy,
        abort: &Abort,
        report: &dyn Fn(Stage),
    ) -> Result<Self, ConnectError> {
        // 自成一个进程组：中止时连同它启动的进程一起结束（见 `abandon`）。
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let streams = (child.stdout.take(), child.stdin.take(), child.stderr.take());
        {
            let mut state = abort.0.lock().unwrap_or_else(PoisonError::into_inner);
            if state.aborted {
                abandon(&mut child);
                return Err(ConnectError::Aborted);
            }
            state.child = Some(child);
        }
        let (Some(input), Some(mut output), Some(errors)) = streams else {
            abort.end();
            return Err(io::Error::other("connection has no standard streams").into());
        };
        let errors = Messages::collect(errors);
        let mut input = BufReader::new(input);

        let step = bootstrap(&mut input, &mut output, deploy, report);
        let Some(child) = abort.take() else {
            return Err(ConnectError::Aborted);
        };
        let mut child = child;
        match step {
            Ok(Step::Ready) => {}
            Ok(Step::Unsupported(system)) => {
                abandon(&mut child);
                return Err(ConnectError::Unsupported(system));
            }
            // 读写失败也是因为子进程结束了：原因在它的错误输出中。
            Ok(Step::Ended) | Err(_) => {
                drop(output);
                abandon(&mut child);
                return Err(ConnectError::Failed(errors.wait()));
            }
        }
        // 引导阶段已读入缓冲区的内容属于服务进程的应答，连同缓冲区一起交出。
        let connection = Self::attach(input, output, Some(child));
        {
            let mut state = abort.0.lock().unwrap_or_else(PoisonError::into_inner);
            if state.aborted {
                return Err(ConnectError::Aborted);
            }
            state.link = Some(Arc::clone(&connection.link));
        }
        match connection.hello() {
            Ok(()) => Ok(connection),
            Err(_) if abort.is_aborted() => Err(ConnectError::Aborted),
            // 服务进程没能启动（例如目录不允许执行）：原因在错误输出中，握手的错误只是结果。
            Err(error) => Err(match errors.wait() {
                reason if reason.is_empty() => ConnectError::Io(error),
                reason => ConnectError::Failed(reason),
            }),
        }
    }
}

/// 连接在登录之后经过的阶段；此前是建立网络连接和登录，由 `ssh` 进行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 远端没有这一版本的服务进程，正在上传。
    Uploading,
    /// 服务进程即将启动，随后交换协议版本。
    Starting,
}

/// 引导阶段的结果。
enum Step {
    /// 服务进程即将启动。
    Ready,
    /// 远端的系统没有对应的服务进程。
    Unsupported(String),
    /// 脚本报告失败，或没有输出就结束了。
    Ended,
}

/// 读取脚本的输出，在它要求时上传与远端系统对应的服务进程。
fn bootstrap(
    input: &mut impl BufRead,
    output: &mut impl Write,
    deploy: &Deploy,
    report: &dyn Fn(Stage),
) -> io::Result<Step> {
    let mut line = read_line(input)?;
    if let Some(system) = line.as_deref().and_then(|line| line.strip_prefix("need ")) {
        // 脚本只会要求它在参数中找到的系统。
        let Some(program) = deploy.programs.iter().find(|p| p.system == system) else {
            return Ok(Step::Ended);
        };
        report(Stage::Uploading);
        // 脚本中途失败时不再读取，写入随之失败；原因由接下来的一行或错误输出说明。
        let _ = output
            .write_all(&program.content)
            .and_then(|()| output.flush());
        line = read_line(input)?;
    }
    Ok(match line.as_deref() {
        Some("ready") => {
            report(Stage::Starting);
            Step::Ready
        }
        Some(line) => match line.strip_prefix("unsupported ") {
            Some(system) => Step::Unsupported(system.to_owned()),
            None => Step::Ended,
        },
        None => Step::Ended,
    })
}

/// 读到脚本输出的下一行，返回去掉前缀后的内容；输出结束，或跳过的内容超过 [`PREAMBLE_MAX`]
/// 时返回 `None`。
fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut budget = PREAMBLE_MAX;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = input
            .by_ref()
            .take(budget as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(None);
        }
        budget -= read;
        if let Some(text) = line.strip_prefix(MARKER) {
            return Ok(Some(String::from_utf8_lossy(text).trim_end().to_owned()));
        }
    }
}

/// 结束并回收没有成为连接的子进程，连同它启动的进程。
///
/// 子进程自成一个进程组（见 [`Connection::deploy`]）：只结束它本身时，它启动的进程（例如代理
/// 命令）可能继续持有输出的管道，读取的一方就等不到结束。
fn abandon(child: &mut Child) {
    #[cfg(unix)]
    kill_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

/// 强制结束以 `child` 为组长的进程组。
// 本函数只有对 `libc::kill` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[cfg(unix)]
#[allow(unsafe_code)]
pub(super) fn kill_group(child: &Child) {
    if let Ok(group) = i32::try_from(child.id()) {
        // SAFETY: `child` 尚未被回收，其进程号不会被其他进程占用；它是自己进程组的组长，负的
        // 进程号表示该进程组。`kill` 只发送信号，不访问内存，失败时返回 -1。
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
}

/// 子进程的错误输出：由一个线程持续读取，只保留末尾的 [`MESSAGE_MAX`] 字节。
///
/// 连接建立后线程继续读取并丢弃，避免子进程因管道写满而停住；子进程结束时线程退出。
struct Messages {
    text: Arc<Mutex<Vec<u8>>>,
    reader: thread::JoinHandle<()>,
}

impl Messages {
    fn collect(mut errors: impl Read + Send + 'static) -> Self {
        let text = Arc::new(Mutex::new(Vec::new()));
        let reader = {
            let text = Arc::clone(&text);
            thread::spawn(move || {
                let mut chunk = [0; 1024];
                while let Ok(read @ 1..) = errors.read(&mut chunk) {
                    let mut text = text.lock().unwrap_or_else(PoisonError::into_inner);
                    text.extend_from_slice(&chunk[..read]);
                    let excess = text.len().saturating_sub(MESSAGE_MAX);
                    text.drain(..excess);
                }
            })
        };
        Self { text, reader }
    }

    /// 等错误输出读完（最长 [`MESSAGE_WAIT`]），返回其中的文字。
    fn wait(self) -> String {
        let deadline = Instant::now() + MESSAGE_WAIT;
        while !self.reader.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let text = self.text.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&text).trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy(dir: &str, digest: &str) -> Deploy {
        Deploy {
            dir: dir.to_owned(),
            programs: vec![Program {
                system: "Linux-x86_64".to_owned(),
                digest: digest.to_owned(),
                content: vec![0; 7],
            }],
        }
    }

    #[test]
    fn the_bootstrap_command_is_safe_to_pass_through_a_login_shell() {
        // 单引号、反斜杠和换行在不同的登录 shell 中含义不同，脚本中不能出现。
        assert!(!SCRIPT.contains(['\'', '\\', '\n']));
        // `!` 之后是其他字符时，有的登录 shell 在单引号内也会当作历史命令展开。
        let after_bang = SCRIPT.split('!').skip(1).map(|rest| rest.chars().next());
        assert!(
            after_bang
                .into_iter()
                .all(|next| matches!(next, Some(' ' | '=')))
        );

        let words = deploy("~/it's here", "0af3").command().unwrap();
        assert_eq!(words[..2], ["sh", "-c"]);
        assert_eq!(
            words[3..],
            ["winc", "~/it's here", "Linux-x86_64", "0af3", "7"]
        );

        let target = Target::new("alice@devbox").unwrap();
        let command = deploy("~/it's here", "0af3").ssh_command(&target, Ssh::default());
        let command = command.unwrap();
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args[..5],
            ["-T", "-o", "RemoteCommand=none", "alice@devbox", "--"]
        );
        assert_eq!(args[5..7], ["'sh'", "'-c'"]);
        assert_eq!(args[9], r"'~/it'\''s here'");
    }

    #[test]
    fn values_that_would_change_the_remote_command_are_rejected() {
        for dir in ["a\\b", "a\nb"] {
            let error = deploy(dir, "0af3").command().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{dir:?}");
        }
        for digest in ["", "../x", "0AF3", "0af3 x"] {
            let error = deploy("", digest).command().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{digest:?}");
        }
        let mut odd = deploy("", "0af3");
        odd.programs[0].system = "Linux x86".to_owned();
        assert!(odd.command().is_err());
    }

    #[test]
    fn output_before_the_script_is_skipped_up_to_a_limit() {
        let mut input: &[u8] = b"Welcome!\nwinc is here\nwinc:ready\nrest";
        assert_eq!(read_line(&mut input).unwrap().as_deref(), Some("ready"));
        assert_eq!(input, b"rest");

        let mut noise = vec![b'x'; PREAMBLE_MAX];
        noise.extend_from_slice(b"\nwinc:ready\n");
        assert_eq!(read_line(&mut noise.as_slice()).unwrap(), None);
        assert_eq!(read_line(&mut b"".as_slice()).unwrap(), None);
    }

    #[test]
    fn the_program_is_sent_only_when_the_script_asks_for_it() {
        let deploy = deploy("", "0af3");
        let run = |script: &[u8]| {
            let mut sent = Vec::new();
            let step = bootstrap(&mut &*script, &mut sent, &deploy, &|_| {}).unwrap();
            (step, sent)
        };
        assert!(matches!(run(b"winc:ready\n"), (Step::Ready, sent) if sent.is_empty()));
        assert!(matches!(
            run(b"winc:need Linux-x86_64\nwinc:ready\n"),
            (Step::Ready, sent) if sent == [0; 7]
        ));
        assert!(matches!(
            run(b"winc:need Linux-x86_64\nwinc:failed\n"),
            (Step::Ended, _)
        ));
        assert!(matches!(
            run(b"winc:unsupported SunOS-sun4v\n"),
            (Step::Unsupported(system), sent) if system == "SunOS-sun4v" && sent.is_empty()
        ));
        assert!(matches!(run(b"winc:need Plan9-mips\n"), (Step::Ended, sent) if sent.is_empty()));
        assert!(matches!(run(b"motd only\n"), (Step::Ended, _)));
    }

    #[test]
    fn the_stages_after_login_are_reported_in_order() {
        let deploy = deploy("", "0af3");
        let stages = |script: &[u8]| {
            let seen = std::cell::RefCell::new(Vec::new());
            let report = |stage| seen.borrow_mut().push(stage);
            bootstrap(&mut &*script, &mut Vec::new(), &deploy, &report).unwrap();
            seen.into_inner()
        };
        assert_eq!(stages(b"winc:ready\n"), [Stage::Starting]);
        assert_eq!(
            stages(b"winc:need Linux-x86_64\nwinc:ready\n"),
            [Stage::Uploading, Stage::Starting]
        );
        assert_eq!(
            stages(b"winc:need Linux-x86_64\nwinc:failed\n"),
            [Stage::Uploading]
        );
        assert!(stages(b"winc:unsupported SunOS-sun4v\n").is_empty());
    }
}
