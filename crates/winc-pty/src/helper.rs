//! 辅助进程的事件循环：等 stdin（主程序的消息）、每个终端的伪终端和子进程退出，处理消息后发帧。
//!
//! Unix 上一个 `poll` 等全部描述符，不开线程；Windows 上由后台线程读 stdin 和输出、等进程退出，
//! 再有事情时唤醒循环（`conpty.rs`）。没有待处理的事情时一直阻塞，不占用 CPU。
//!
//! 发帧按额度：伪终端的输出随到随解析，只有主程序给了额度、屏幕又变了的终端才发一帧，中间状态
//! 直接跳过。

mod integration;
#[cfg(unix)]
mod pty;
#[cfg(windows)]
#[path = "helper/conpty.rs"]
mod pty;

use std::collections::BTreeMap;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use pty::{Command, Pty};
use winc_pty::Terminal;
use winc_pty::protocol::{
    Decoder, Event, FromHelper, Line, MAGIC, Message, Size, Theme, ToHelper, VERSION,
};

/// 单次读取伪终端输出的缓冲大小（字节）。
///
/// 64 KiB 是经验值：读得越大系统调用越少，每轮最多读 [`READ_BUDGET`] 的十六分之一。
const READ_CHUNK: usize = 64 << 10;
/// 每轮循环里一个终端最多读的字节数（字节）。
///
/// 1 MiB 是经验值：刷屏的终端每轮最多占 16 次读，其他终端和主程序的消息仍能及时处理。
const READ_BUDGET: usize = 1 << 20;
/// 待写进伪终端的字节上限（字节），16 MiB。
///
/// shell 不读输入时，再来的输入（粘贴、按键、对查询的回答）一律丢弃，内存占用不会持续增长。
const MAX_PENDING_INPUT: usize = 16 << 20;
/// 一条历史回答里行的总字节上限（字节），超过就分几条发。
///
/// 取 8 MiB，为协议的消息上限 64 MiB 留出余量。
const HISTORY_CHUNK: usize = 8 << 20;
/// 同步输出（模式 2026）最多等待的时长。
///
/// 150 ms 是经验值：期限一到就按已收到的内容发一帧，程序一直不关闭同步输出时屏幕也能更新。
const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// 关掉终端后等进程组自己退出的时间，过了就发 SIGKILL。
///
/// 500 ms 是经验值：响应 SIGHUP 的程序一般在一个轮询周期内退出，这段余量既够用，又不会让关闭
/// 终端时界面长时间等待。
const HANGUP_GRACE: Duration = Duration::from_millis(500);
/// 关掉的终端里 shell 已经回收、前台作业还在时，检查前台作业是否退出的间隔。
///
/// 作业不是辅助进程的子进程，退出时没有 SIGCHLD，只能按 50 ms 的间隔查看。
const JOB_POLL: Duration = Duration::from_millis(50);
/// 主程序退出后等待终端全部退出的最长时间（毫秒）。
///
/// 由 [`HANGUP_GRACE`] 决定：挂断后到期发 SIGKILL，再留同样长的时间让进程组退出。
const FINISH_WAIT: Duration = Duration::from_millis(2 * HANGUP_GRACE.as_millis() as u64);
/// 前台进程名的最短查询间隔：发帧后最多按该间隔查询一次。
///
/// 300 ms 是经验值：足以让标签名跟上命令切换，两次查询之间不再读进程信息，刷屏时不会每帧
/// 都查。
const PROCESS_INTERVAL: Duration = Duration::from_millis(300);

struct Tab {
    /// 终端仿真状态：屏幕、回滚缓冲、命令块和待发事件。
    term: Terminal,
    /// shell 退出并回收以后为 `None`。
    pty: Option<Pty>,
    /// 还没写进伪终端的字节，上限见 [`MAX_PENDING_INPUT`]。
    input: PendingInput,
    /// 主程序给的额度：可以再发一帧。
    credit: bool,
    /// 分几次发来的输入还没收完时，是否收下余下的部分；没有进行中的输入时为 `None`。
    ///
    /// 收到第一段时就按整次输入的大小定下，整次输入一起收下或者一起丢弃。
    pasting: Option<bool>,
    /// 是否读到了结尾（从端都关了）。
    eof: bool,
    /// 开始等待同步输出结束的时刻；不在同步中时为 `None`。
    sync_since: Option<Instant>,
    /// 上次查到的前台进程（进程号、名字），用于发现前台程序的变化；名字用于发现同一进程的 `exec`。
    process: Option<(pty::Pid, String)>,
    /// 上次查询前台进程的时刻。
    process_checked: Option<Instant>,
    /// 发过帧、还没再看前台进程；离上次查询不到 [`PROCESS_INTERVAL`] 时等到时间再查。
    process_due: bool,
    /// 这次查询之后是否还要再查一次：发帧时命令可能还没启动，没有输出的命令不会再发帧。
    process_recheck: bool,
    /// 收到 [`ToHelper::Stop`] 以后：挂断时的前台作业，以及到期仍未退出时强制结束的时刻。
    stopping: Option<(Option<pty::Pid>, Instant)>,
}

/// 已经关掉、等进程组退出的终端。
struct Closing {
    /// 关掉的伪终端，等进程组退出后释放。
    pty: Pty,
    /// 关掉时的前台作业的进程组（见 `Pty::foreground_group`）。
    ///
    /// 忽略 SIGHUP 的程序不会随 shell 退出，到期一并 SIGKILL，不留下孤儿进程。
    job: Option<pty::Pid>,
    /// shell 是否已经回收。
    ///
    /// 回收以后再查 `try_wait` 拿不到状态，因此单独记下：`kill` 只发给进程组。
    reaped: bool,
    /// 到这个时刻仍未退出就发 SIGKILL；已经发过时为 `None`。
    kill_at: Option<Instant>,
}

struct Helper {
    /// 各终端标签，键是主程序分配的标签号（从 0 起）。
    tabs: BTreeMap<u16, Tab>,
    /// 已经关掉、等进程组退出的终端。
    closing: Vec<Closing>,
    /// 解析主程序发来的字节。
    decoder: Decoder,
    /// 待写给主程序的字节。
    out: Vec<u8>,
    /// 回答颜色查询用的主题。
    theme: Option<Theme>,
    /// 是否新建过终端；没有开过终端时 stdin 结束才退出。
    spawned: bool,
    /// SIGCHLD 的自写管道。
    #[cfg(unix)]
    children: OwnedFd,
    /// 读 stdin 和伪终端共用的缓冲，长度见 [`READ_CHUNK`]。
    buf: Vec<u8>,
}

/// 运行到主程序关闭 stdin（或者最后一个终端关掉）为止。
///
/// # Errors
///
/// 读 stdin、写 stdout、等描述符失败时返回对应的 `std::io::Error`。
pub fn run() -> io::Result<()> {
    #[cfg(unix)]
    pty::close_inherited_on_exec();
    let mut helper = Helper {
        tabs: BTreeMap::new(),
        closing: Vec::new(),
        decoder: Decoder::default(),
        out: Vec::new(),
        theme: None,
        spawned: false,
        #[cfg(unix)]
        children: pty::child_signals()?,
        buf: vec![0; READ_CHUNK],
    };
    #[cfg(windows)]
    pty::read_stdin_in_background();
    helper.send(&FromHelper::Hello {
        magic: MAGIC,
        version: VERSION,
        build: env!("CARGO_PKG_VERSION").into(),
    });
    helper.flush()?;

    let result = loop {
        match helper.turn() {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    helper.finish();
    result
}

impl Helper {
    /// 挂断终端和它的前台作业，到期仍未退出时由 [`Helper::kill_overdue`] 结束。
    fn close(&mut self, pty: pty::Pty) {
        let job = pty.foreground_group();
        pty.hang_up();
        if let Some(job) = job {
            pty::hang_up_job(job);
        }
        self.closing.push(Closing {
            pty,
            job,
            reaped: false,
            kill_at: Some(Instant::now() + HANGUP_GRACE),
        });
    }

    /// 主程序已退出或连接已断开：挂断所有终端，并等待它们退出，不留下孤儿进程。
    ///
    /// 最多等待 [`FINISH_WAIT`]：进程组在 SIGKILL 之后仍未退出时（如处于不可中断的睡眠）不再等待。
    fn finish(&mut self) {
        let tabs = std::mem::take(&mut self.tabs);
        for pty in tabs.into_values().filter_map(|tab| tab.pty) {
            self.close(pty);
        }
        let deadline = Instant::now() + FINISH_WAIT;
        loop {
            self.settle_closing();
            self.kill_overdue();
            if self.closing.is_empty() || Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(JOB_POLL);
        }
    }

    fn send(&mut self, message: &FromHelper) {
        message.encode(&mut self.out);
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.out.is_empty() {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&self.out)?;
            stdout.flush()?;
            self.out.clear();
            self.out.shrink_to(winc_pty::protocol::IDLE_BUFFER);
        }
        Ok(())
    }

    /// 循环一轮：等到有事可做，处理消息、输出和退出的子进程，再发帧。
    ///
    /// 返回 `false` 时退出。
    #[cfg(windows)]
    fn turn(&mut self) -> io::Result<bool> {
        let timeout = self.timeout();
        pty::wait((timeout >= 0).then(|| Duration::from_millis(timeout as u64)));
        if pty::stdin_pending() && !self.read_messages()? {
            return Ok(false);
        }
        let mut ended = Vec::new();
        for (&id, tab) in &mut self.tabs {
            if !tab.eof {
                read_output(tab, &mut self.buf);
                if tab.eof {
                    ended.push(id);
                }
            }
            if !tab.input.is_empty() {
                write_input(tab);
            }
        }
        // 退出不另外通知：每一轮都看一遍（终端只有几个，查询不阻塞）。
        self.reap();
        self.report_disconnected(ended);
        self.settle_closing();
        self.kill_overdue();
        self.send_frames();
        self.flush()?;

        Ok(!(self.spawned && self.tabs.is_empty() && self.closing.is_empty()))
    }

    /// 循环一轮：`poll` 等到有事可做，处理消息、输出和退出的子进程，再发帧。
    ///
    /// 返回 `false` 时退出。
    #[cfg(unix)]
    fn turn(&mut self) -> io::Result<bool> {
        let mut fds = vec![
            pollfd(0, libc::POLLIN),
            pollfd(self.children.as_raw_fd(), libc::POLLIN),
        ];
        let mut ids = Vec::new();
        for (&id, tab) in &self.tabs {
            let Some(pty) = &tab.pty else { continue };
            let mut events = 0;
            if !tab.eof {
                events |= libc::POLLIN;
            }
            if !tab.input.is_empty() {
                events |= libc::POLLOUT;
            }
            if events != 0 {
                fds.push(pollfd(pty.fd(), events));
                ids.push(id);
            }
        }
        pty::poll(&mut fds, self.timeout())?;

        if fds[0].revents != 0 && !self.read_messages()? {
            return Ok(false);
        }
        // 子进程退出时 SIGCHLD 写入自管道：只在这时回收，不在每一轮对每个进程调用 `waitpid`。
        let children_exited = fds[1].revents != 0;
        if children_exited {
            pty::drain(fds[1].fd);
        }
        for (fd, id) in fds[2..].iter().zip(ids) {
            if let Some(tab) = self.tabs.get_mut(&id) {
                if fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                    read_output(tab, &mut self.buf);
                }
                if fd.revents & libc::POLLOUT != 0 {
                    write_input(tab);
                }
            }
        }
        if children_exited {
            self.reap();
        }
        self.settle_closing();
        self.kill_overdue();
        self.send_frames();
        self.flush()?;

        Ok(!(self.spawned && self.tabs.is_empty() && self.closing.is_empty()))
    }

    /// 返回 `poll` 最多等待的毫秒数。
    ///
    /// 取同步输出、SIGKILL、前台作业（[`JOB_POLL`]）和前台进程查询各期限中最近的一个，上限 1000 加
    /// 1 毫秒（加 1 避免 0 被当作一直等待）；没有期限时返回 -1，表示一直等待。
    fn timeout(&self) -> i32 {
        let now = Instant::now();
        let sync = self
            .tabs
            .values()
            .filter(|t| t.credit)
            .filter_map(|t| t.sync_since.map(|s| s + SYNC_TIMEOUT));
        let kills = self.closing.iter().filter_map(|c| c.kill_at);
        let stops = (self.tabs.values()).filter_map(|t| t.stopping.map(|(_, at)| at));
        let jobs = (self.closing.iter())
            .filter(|c| c.reaped && c.job.is_some())
            .map(|_| now + JOB_POLL);
        let processes = (self.tabs.values())
            .filter(|t| t.process_due)
            .map(|t| t.process_checked.map_or(now, |at| at + PROCESS_INTERVAL));
        match sync
            .chain(kills)
            .chain(stops)
            .chain(jobs)
            .chain(processes)
            .min()
        {
            Some(at) => at.saturating_duration_since(now).as_millis().min(1000) as i32 + 1,
            None => -1,
        }
    }

    /// 读主程序发来的消息并逐条处理。
    ///
    /// stdin 结束（主程序退出了）或者数据不对时返回 `false`；读取失败时返回
    /// `std::io::Error`。
    fn read_messages(&mut self) -> io::Result<bool> {
        let n = pty::read_stdin(&mut self.buf)?;
        if n == 0 {
            return Ok(false);
        }
        self.decoder.push(&self.buf[..n]);
        loop {
            match self.decoder.next_message::<ToHelper>() {
                Ok(Some(message)) => self.handle(message),
                Ok(None) => return Ok(true),
                Err(error) => {
                    eprintln!("winc-pty: {error}");
                    return Ok(false);
                }
            }
        }
    }

    fn handle(&mut self, message: ToHelper) {
        match message {
            ToHelper::Spawn {
                tab,
                program,
                args,
                login,
                cwd,
                env,
                scrollback,
                size,
                integration,
                banner,
            } => self.spawn(
                tab,
                program,
                args,
                login,
                &cwd,
                env,
                scrollback,
                size,
                integration,
                &banner,
            ),
            ToHelper::Input { tab, bytes, rest } => {
                let Some(tab) = self.tabs.get_mut(&tab) else {
                    return;
                };
                let keep = admit(&mut tab.pasting, tab.input.len(), bytes.len(), rest);
                if keep && tab.pty.is_some() && !tab.eof {
                    tab.input.extend(&bytes);
                    // 先直接写一次：多数时候写得进去，不必等下一轮 poll，按键早一轮到 shell。
                    write_input(tab);
                }
            }
            ToHelper::Resize { tab, size } => {
                if let Some(tab) = self.tabs.get_mut(&tab) {
                    tab.term.resize(size);
                    if let Some(pty) = &tab.pty {
                        pty.resize(size);
                    }
                }
            }
            ToHelper::Credit { tab } => {
                if let Some(tab) = self.tabs.get_mut(&tab) {
                    tab.credit = true;
                }
            }
            ToHelper::History {
                tab: id,
                id: request,
                start,
                end,
            } => {
                if let Some(tab) = self.tabs.get(&id) {
                    let (start, lines) = tab.term.history(start, end);
                    for (start, lines, more) in history_chunks(start, lines) {
                        self.send(&FromHelper::History {
                            tab: id,
                            id: request,
                            start,
                            lines,
                            more,
                        });
                    }
                }
            }
            ToHelper::Clear { tab } => {
                if let Some(tab) = self.tabs.get_mut(&tab) {
                    tab.term.clear();
                }
            }
            ToHelper::Close { tab } => {
                if let Some(tab) = self.tabs.remove(&tab)
                    && let Some(pty) = tab.pty
                {
                    self.close(pty);
                }
            }
            ToHelper::Stop { tab } => {
                if let Some(tab) = self.tabs.get_mut(&tab)
                    && let Some(pty) = &tab.pty
                    && tab.stopping.is_none()
                {
                    let job = pty.foreground_group();
                    pty.hang_up();
                    if let Some(job) = job {
                        pty::hang_up_job(job);
                    }
                    tab.stopping = Some((job, Instant::now() + HANGUP_GRACE));
                }
            }
            ToHelper::Theme(theme) => {
                self.theme = Some(theme);
                for tab in self.tabs.values_mut() {
                    tab.term.set_theme(Some(theme));
                }
            }
        }
    }

    /// 新建终端并登记标签：解析程序路径、注入 shell 集成、启动伪终端；失败时发 `Failed`。
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        id: u16,
        program: Option<Vec<u8>>,
        mut args: Vec<Vec<u8>>,
        #[cfg_attr(windows, allow(unused_mut))] mut login: bool,
        cwd: &[u8],
        env: Vec<(Vec<u8>, Vec<u8>)>,
        scrollback: u32,
        size: Size,
        integration: Option<Vec<u8>>,
        banner: &[u8],
    ) {
        if self.tabs.contains_key(&id) {
            self.send(&FromHelper::Failed {
                tab: id,
                message: format!("终端 {id} 已存在"),
            });
            return;
        }
        let mut env = environment(env);
        if let Some(theme) = self.theme {
            set_colorfgbg(&mut env, theme.bg);
        }
        let program = program
            .filter(|p| !p.is_empty())
            .or_else(|| {
                lookup(&env, b"SHELL")
                    .filter(|s| !s.is_empty() && cfg!(unix))
                    .map(<[u8]>::to_vec)
            })
            .or_else(|| pty::user_shell(&env))
            .unwrap_or_else(|| pty::FALLBACK_SHELL.to_vec());
        let Some(path) = resolve(&program, &env) else {
            self.send(&FromHelper::Failed {
                tab: id,
                message: format!("未找到程序「{}」", String::from_utf8_lossy(&program)),
            });
            return;
        };
        // 集成脚本只适用于 Unix shell。Windows 上仅注入 WSL 发行版内的 shell，其他 shell 不提供命令块。
        #[cfg(unix)]
        let nonce = integration
            .and_then(|dir| integration::inject(&dir, &path, &mut args, &mut login, &mut env));
        #[cfg(windows)]
        let nonce =
            integration.and_then(|dir| integration::inject_wsl(&dir, &path, &mut args, &mut env));
        #[cfg(not(any(unix, windows)))]
        let nonce: Option<_> = {
            let _ = (integration, &mut login, &mut args, &mut env);
            None
        };
        #[cfg(windows)]
        if integration::is_wsl(&path) {
            integration::forward_to_wsl(&mut env);
        }
        let name = program.rsplit(|&b| b == b'/').next().unwrap_or(&program);
        let argv0 = if login {
            [b"-", name].concat()
        } else {
            program.clone()
        };

        // 没给目录时从家目录开始。
        let cwd = if cwd.is_empty() {
            lookup(&env, pty::HOME).unwrap_or(b"/")
        } else {
            cwd
        };
        let command = Command {
            program: &path,
            argv0: &argv0,
            args: &args,
            cwd,
            env: &env,
            size,
        };
        match Pty::spawn(&command) {
            Ok(pty) => {
                let mut term = Terminal::new(
                    usize::from(size.cols),
                    usize::from(size.rows),
                    scrollback as usize,
                );
                term.resize(size);
                term.set_theme(self.theme);
                term.feed(banner);
                if let Some(nonce) = nonce {
                    term.set_nonce(nonce);
                }
                self.tabs.insert(
                    id,
                    Tab {
                        term,
                        pty: Some(pty),
                        input: PendingInput::default(),
                        credit: true,
                        pasting: None,
                        eof: false,
                        sync_since: None,
                        process: None,
                        process_checked: None,
                        process_due: false,
                        process_recheck: false,
                        stopping: None,
                    },
                );
                self.spawned = true;
            }
            Err(error) => self.send(&FromHelper::Failed {
                tab: id,
                message: format!("无法启动「{}」：{error}", String::from_utf8_lossy(&program)),
            }),
        }
    }

    /// 回收退出的子进程：先读完剩下的输出，再挂断它留下的进程组并报退出。
    ///
    /// 收到 SIGCHLD 后调用。多个子进程先后退出时信号可能合并为一次，因此检查所有终端。
    fn reap(&mut self) {
        let mut exited = Vec::new();
        let buf = &mut self.buf;
        for (&id, tab) in &mut self.tabs {
            let Some(pty) = &tab.pty else { continue };
            if let Some(code) = pty.try_wait() {
                // 先把剩下的输出读完再报退出。
                read_output(tab, buf);
                // shell 退出后，留在后台、还占着这个终端的进程一起挂断。
                if let Some(pty) = tab.pty.take() {
                    pty.hang_up_group();
                }
                tab.input.clear();
                exited.push((id, code));
            }
        }
        for (tab, code) in exited {
            self.send(&FromHelper::Exited { tab, code });
        }
    }

    /// 报告这一轮读到结尾、[`Helper::reap`] 之后 shell 仍未退出的终端：伪终端的进程不在了。
    ///
    /// shell 随后退出时还会发 [`FromHelper::Exited`]，主程序以后者为准。程序可能还在做事，这里
    /// 不结束它们，由用户决定何时关闭或重新启动。
    #[cfg(windows)]
    fn report_disconnected(&mut self, ended: Vec<u16>) {
        for tab in ended {
            if self.tabs.get(&tab).is_some_and(|t| t.pty.is_some()) {
                self.send(&FromHelper::Disconnected { tab });
            }
        }
    }

    /// 释放 shell 和前台作业都已退出的关掉的终端。
    ///
    /// 每一轮都看：只等 shell 时，忽略 SIGHUP 的作业还没等到 SIGKILL 就会漏掉；作业退出时没有
    /// SIGCHLD，不能只在 [`Helper::reap`] 里看。关闭的终端只有几个，`waitpid` 不阻塞。
    fn settle_closing(&mut self) {
        self.closing.retain_mut(|closing| {
            closing.reaped = closing.reaped || closing.pty.try_wait().is_some();
            !closing.reaped || closing.job.is_some_and(pty::job_alive)
        });
    }

    /// 向挂断后到期仍未退出的进程组发送 SIGKILL，每个只发一次。
    ///
    /// 进程退出后由 [`Helper::reap`] 回收。
    fn kill_overdue(&mut self) {
        let now = Instant::now();
        for closing in &mut self.closing {
            if closing.kill_at.is_some_and(|at| now >= at) {
                closing.pty.kill(closing.reaped);
                if let Some(job) = closing.job {
                    pty::kill_job(job);
                }
                closing.kill_at = None;
            }
        }
        for tab in self.tabs.values_mut() {
            if let Some((job, at)) = tab.stopping
                && now >= at
            {
                if let Some(pty) = &tab.pty {
                    pty.kill(false);
                }
                if let Some(job) = job {
                    pty::kill_job(job);
                }
                tab.stopping = None;
            }
        }
    }

    /// 把事件、命令块、回答和帧发给主程序；帧受额度和同步输出限制。
    fn send_frames(&mut self) {
        let now = Instant::now();
        let mut messages = Vec::new();

        for (&id, tab) in &mut self.tabs {
            for event in coalesce(tab.term.take_events()) {
                messages.push(FromHelper::Event { tab: id, event });
            }
            // 命令块不等额度，变了就发：后台标签没有额度，它的命令结束时也要知道。重排、清回滚以后
            // 行号变了、那一帧还没发出去时标上 `early`，主程序等那一帧到了再换上。
            if let Some(blocks) = tab.term.take_blocks() {
                let early = tab.term.renumbered();
                messages.push(FromHelper::Blocks {
                    tab: id,
                    blocks,
                    early,
                });
            }
            let responses = tab.term.take_responses();
            // 程序一直查询又不读回答（`yes $'\e[6n'`）时，回答和输入一样有上限。
            if !responses.is_empty()
                && tab.pty.is_some()
                && tab.input.len() + responses.len() <= MAX_PENDING_INPUT
            {
                tab.input.extend(&responses);
                write_input(tab);
            }
            check_process(id, tab, now, &mut messages);

            if tab.term.syncing() {
                let since = *tab.sync_since.get_or_insert(now);
                if now < since + SYNC_TIMEOUT {
                    continue;
                }
                // 过了期限：仍然发帧，下一段重新计时。程序一直不关闭同步输出（中途被杀掉的
                // 全屏程序）时，期限会停在过去，`timeout` 每次都算出 1 ms，辅助进程每秒醒来
                // 上千次。
                tab.sync_since = Some(now);
            } else {
                tab.sync_since = None;
            }
            if !tab.credit {
                continue;
            }
            if let Some(frame) = tab.term.take_frame() {
                tab.credit = false;
                messages.push(FromHelper::Frame { tab: id, frame });
                tab.process_due = true;
                tab.process_recheck = true;
                check_process(id, tab, now, &mut messages);
            }
        }
        for message in &messages {
            self.send(message);
        }
    }
}

/// 发过帧以后查一次前台进程，变了就通知主程序。
///
/// 离上次查询不到 [`PROCESS_INTERVAL`] 时先记下，到时间再查（`timeout` 按时醒来）：命令结束
/// 以后没有新的输出，标签上的名字也要换回来。
fn check_process(id: u16, tab: &mut Tab, now: Instant, messages: &mut Vec<FromHelper>) {
    if !tab.process_due
        || tab
            .process_checked
            .is_some_and(|at| now < at + PROCESS_INTERVAL)
    {
        return;
    }
    tab.process_due = std::mem::take(&mut tab.process_recheck);
    tab.process_checked = Some(now);
    if let Some(pty) = &tab.pty
        && let Some(found) = pty.foreground()
        && tab.process.as_ref() != Some(&found)
    {
        let name = found.1.clone();
        tab.process = Some(found);
        messages.push(FromHelper::Process { tab: id, name });
    }
}

/// 返回是否收下一条输入；`rest` 是同一次输入后面还剩的字节数。
///
/// 整次输入一起收下或者一起丢弃：只丢弃中间几段时粘贴的内容会出现缺口，缺少结尾的
/// `ESC[201~` 时程序会一直停在括号粘贴模式里。
fn admit(pasting: &mut Option<bool>, pending: usize, len: usize, rest: u64) -> bool {
    let keep =
        pasting.unwrap_or_else(|| pending as u64 + len as u64 + rest <= MAX_PENDING_INPUT as u64);
    *pasting = (rest > 0).then_some(keep);
    keep
}

/// 把一次历史回答按 [`HISTORY_CHUNK`] 分成几段，每段是第一行行号、行和后面还有没有行。
///
/// 至少一段，没有行时也回答一次，主程序才知道请求结束了。
fn history_chunks(start: u64, lines: Vec<Line>) -> Vec<(u64, Vec<Line>, bool)> {
    split_lines(start, lines, HISTORY_CHUNK)
}

fn split_lines(mut start: u64, lines: Vec<Line>, limit: usize) -> Vec<(u64, Vec<Line>, bool)> {
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut size = 0;
    for line in lines {
        let len = line.encoded_len();
        if !chunk.is_empty() && size + len > limit {
            let n = chunk.len() as u64;
            chunks.push((start, std::mem::take(&mut chunk), true));
            start += n;
            size = 0;
        }
        size += len;
        chunk.push(line);
    }
    chunks.push((start, chunk, false));
    chunks
}

/// 一轮里每种事件只保留最后一次发生的，链接事件全部保留。
///
/// 标题、当前目录、剪贴板、通知、响铃各留一个：程序在循环里改标题（或者用 `yes` 刷 OSC）时，
/// 向主程序发送的事件不会多到成千上万条。
fn coalesce(mut events: Vec<Event>) -> Vec<Event> {
    // 命令边界由辅助进程切成块另发（`Blocks`），主程序不用。
    events.retain(|e| !matches!(e, Event::Command(_)));
    if events.len() <= 1 {
        return events;
    }
    let keep_last = |e: &Event| {
        matches!(
            e,
            Event::Title(_)
                | Event::Cwd(_)
                | Event::Clipboard(_)
                | Event::Notify { .. }
                | Event::Bell
        )
    };
    // 从后往前走，每种只留第一次见到的（也就是最后一次发生的），一遍走完。
    let mut seen = Vec::new();
    let mut out: Vec<Event> = Vec::with_capacity(events.len().min(64));
    for event in events.into_iter().rev() {
        if keep_last(&event) {
            let kind = std::mem::discriminant(&event);
            if seen.contains(&kind) {
                continue;
            }
            seen.push(kind);
        }
        out.push(event);
    }
    out.reverse();
    out
}

#[cfg(unix)]
fn pollfd(fd: i32, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

/// 把伪终端里现有的输出交给终端仿真，一次最多读 [`READ_BUDGET`]。
fn read_output(tab: &mut Tab, buf: &mut [u8]) {
    let Some(pty) = &tab.pty else { return };
    let mut total = 0;
    while total < READ_BUDGET {
        match pty.read(buf) {
            Ok(0) => {
                // 没人读了，没写进去的输入也不用再写；不再放进 poll，避免反复报告挂断。
                tab.eof = true;
                tab.input.clear();
                break;
            }
            Ok(n) => {
                tab.term.feed(&buf[..n]);
                total += n;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// 把待写的输入尽量写进伪终端，写不进去的部分留在缓冲里。
fn write_input(tab: &mut Tab) {
    let Some(pty) = &tab.pty else { return };
    while !tab.input.is_empty() {
        match pty.write(tab.input.bytes()) {
            Ok(0) => break,
            Ok(n) => tab.input.advance(n),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// 等待写入伪终端的字节。
///
/// 程序读取得慢时每次只能写入一部分：已写入的部分只记偏移，积累到一半以上才移动剩余的字节，因此总的
/// 移动量与字节数成正比。写完后释放多余的容量，一次大段粘贴不会一直占用内存。
#[derive(Default)]
struct PendingInput {
    buf: Vec<u8>,
    /// 已写入的字节数。
    start: usize,
}

impl PendingInput {
    fn len(&self) -> usize {
        self.buf.len() - self.start
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    fn extend(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 记下又写入了 `n` 个字节。
    fn advance(&mut self, n: usize) {
        self.start += n;
        if self.is_empty() {
            self.clear();
        } else if self.start * 2 >= self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.buf.shrink_to(winc_pty::protocol::IDLE_BUFFER);
        self.start = 0;
    }
}

/// 返回终端里的环境变量：主程序给的用户环境去掉 winc 自己用的变量，再设上终端相关的变量。
///
/// 设上 `TERM`、`COLORTERM`、`TERM_PROGRAM`、`TERM_PROGRAM_VERSION`；没有任何 `LANG`、
/// `LC_ALL`、`LC_CTYPE` 时补 `LC_CTYPE=UTF-8`。`COLORFGBG` 由 [`set_colorfgbg`] 按当前底色另写。
fn environment(env: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<(Vec<u8>, Vec<u8>)> {
    const REPLACED: &[&[u8]] = &[
        b"TERM",
        b"COLORTERM",
        b"TERM_PROGRAM",
        b"TERM_PROGRAM_VERSION",
    ];
    let mut env: Vec<_> = env
        .into_iter()
        // winc 自己用的变量不给 shell；WINC_PROMPT 留给集成脚本，脚本读完即删除。
        .filter(|(key, _)| {
            !REPLACED.contains(&&key[..]) && (!key.starts_with(b"WINC_") || key == b"WINC_PROMPT")
        })
        .collect();
    // 从图形界面启动的应用没有 `LANG`，shell 会把中文当成乱码、算错宽度。Windows 的程序不看这些
    // 变量。
    if cfg!(unix)
        && !env
            .iter()
            .any(|(k, _)| matches!(&k[..], b"LANG" | b"LC_ALL" | b"LC_CTYPE"))
    {
        env.push((b"LC_CTYPE".to_vec(), b"UTF-8".to_vec()));
    }
    env.extend([
        (b"TERM".to_vec(), b"xterm-256color".to_vec()),
        (b"COLORTERM".to_vec(), b"truecolor".to_vec()),
        (b"TERM_PROGRAM".to_vec(), b"winc".to_vec()),
        (
            b"TERM_PROGRAM_VERSION".to_vec(),
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
        ),
    ]);
    env
}

/// 按终端底色写 `COLORFGBG`：浅色底 `0;15`，深色底 `15;0`。
///
/// agent 一类程序靠它决定用浅色还是深色界面。沿用外面终端留下的深色值时，浅色窗口里的输入框和
/// 高亮会被画成黑的。
fn set_colorfgbg(env: &mut Vec<(Vec<u8>, Vec<u8>)>, bg: [u8; 3]) {
    env.retain(|(key, _)| key != b"COLORFGBG");
    let y = u32::from(bg[0]) * 54 + u32::from(bg[1]) * 183 + u32::from(bg[2]) * 18;
    let value: &[u8] = if y > 32_512 { b"0;15" } else { b"15;0" };
    env.push((b"COLORFGBG".to_vec(), value.to_vec()));
}

fn lookup<'a>(env: &'a [(Vec<u8>, Vec<u8>)], key: &[u8]) -> Option<&'a [u8]> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| &v[..])
}

/// 返回程序在 Unix 上的路径：带 `/` 的原样用，否则取 `PATH` 中第一个可执行的文件。
///
/// 没有 `PATH` 时按 `/usr/bin:/bin:/usr/sbin:/sbin` 查找。
#[cfg(unix)]
fn resolve(program: &[u8], env: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    if program.contains(&b'/') {
        return Some(program.to_vec());
    }
    let path = lookup(env, b"PATH").unwrap_or(b"/usr/bin:/bin:/usr/sbin:/sbin");
    path.split(|&b| b == b':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| [dir, b"/", program].concat())
        .find(|candidate| {
            std::fs::metadata(std::ffi::OsStr::from_bytes(candidate))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// 返回程序在 Windows 上的路径：带目录（`\\` 或 `/`）的原样用，否则在 `PATH` 中查找。
///
/// `PATH` 以 `;` 分隔；程序没写扩展名时按 `PATHEXT` 逐个补上，取第一个存在的文件。
#[cfg(windows)]
fn resolve(program: &[u8], env: &[(Vec<u8>, Vec<u8>)]) -> Option<Vec<u8>> {
    let program = String::from_utf8_lossy(program).into_owned();
    if program.contains(['\\', '/']) {
        return Some(program.into_bytes());
    }
    // Windows 的环境变量名不分大小写。
    let var = |name: &str| {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
    };
    let names: Vec<String> = if std::path::Path::new(&program).extension().is_some() {
        vec![program.clone()]
    } else {
        let pathext = var("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        (pathext.split(';'))
            .filter(|ext| ext.starts_with('.'))
            .map(|ext| format!("{program}{}", ext.to_ascii_lowercase()))
            .collect()
    };
    let path = var("PATH").unwrap_or_default();
    (path.split(';'))
        .filter(|dir| !dir.is_empty())
        .flat_map(|dir| {
            names
                .iter()
                .map(move |name| std::path::Path::new(dir).join(name))
        })
        .find(|candidate| candidate.is_file())
        .map(|found| found.to_string_lossy().into_owned().into_bytes())
}

#[cfg(test)]
mod tests;
