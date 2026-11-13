//! 端到端测试共用的主程序一端：启动真正的 `winc-pty`，收发消息，按帧还原屏幕。

#![allow(dead_code)] // 各个测试文件只用到这里的一部分。

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use winc_pty::protocol::{
    Block, Decoder, Frame, FromHelper, Line, MAGIC, Marks, Message, Size, ToHelper, VERSION,
};
/// 等待一条消息或一次退出的最长时间（10 s）。
pub const WAIT: Duration = Duration::from_secs(10);

/// 超时报告中列出的最近消息的条数：足以看出辅助进程停止发送之前在发什么。
const RECENT: usize = 8;

/// 测试用的终端尺寸：40 列 × 6 行，网格区域 320 × 96 像素（每格 8 × 16 像素）。
pub const SIZE: Size = Size {
    cols: 40,
    rows: 6,
    width: 320,
    height: 96,
};

/// 主程序一端：发送消息，在后台线程里接收消息。
pub struct Client {
    /// 辅助进程。
    pub child: Child,
    /// 写给辅助进程的一端；关掉它相当于主程序退出。
    pub stdin: Option<ChildStdin>,
    /// 读线程送来的消息。
    rx: Receiver<FromHelper>,
    /// 按帧还原出来的屏幕。
    pub screen: Vec<Line>,
    /// 当前等待的内容，超时报告中列出；没有说明时为空。
    pub waiting: String,
    /// 最近一次收到的命令块。
    blocks: Vec<Block>,
    /// 最近收到的消息的种类与时刻，最多 [`RECENT`] 条。
    recent: VecDeque<(Instant, String)>,
}

impl Client {
    /// 启动 `winc-pty` 并完成握手。
    ///
    /// # Panics
    ///
    /// 辅助进程启动失败、或者第一条消息不是握手时 panic。
    pub fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_winc-pty"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("启动 winc-pty");
        let mut stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut decoder = Decoder::default();
            let mut buf = [0; 65536];
            loop {
                let n = match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                decoder.push(&buf[..n]);
                while let Some(message) = decoder.next_message::<FromHelper>().unwrap() {
                    if tx.send(message).is_err() {
                        return;
                    }
                }
            }
        });
        let stdin = child.stdin.take();
        let mut client = Client {
            child,
            stdin,
            rx,
            screen: Vec::new(),
            waiting: String::new(),
            blocks: Vec::new(),
            recent: VecDeque::new(),
        };
        match client.recv() {
            FromHelper::Hello { magic, version, .. } => {
                assert_eq!(magic, MAGIC);
                assert_eq!(version, VERSION);
            }
            other => panic!("第一条消息应该是握手：{other:?}"),
        }
        client
    }

    /// 编成一条消息写给辅助进程。
    ///
    /// # Panics
    ///
    /// stdin 已经关掉、或者写入失败时 panic。
    pub fn send(&mut self, message: ToHelper) {
        let mut bytes = Vec::new();
        message.encode(&mut bytes);
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(&bytes).unwrap();
        stdin.flush().unwrap();
    }

    /// 等待下一条消息。
    ///
    /// # Panics
    ///
    /// 超过 [`WAIT`] 还没收到、或者读线程已经退出时 panic。
    pub fn recv(&mut self) -> FromHelper {
        let message = match self.rx.recv_timeout(WAIT) {
            Ok(message) => message,
            Err(_) => panic!("等辅助进程的消息超时。\n{}", self.report()),
        };
        // 种类取调试输出中变体的名字。
        let kind = format!("{message:?}");
        let kind = kind.split([' ', '{', '(']).next().unwrap_or_default();
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back((Instant::now(), kind.to_owned()));
        if let FromHelper::Blocks { blocks, .. } = &message {
            self.blocks.clone_from(blocks);
        }
        message
    }

    /// 返回超时报告：当前等待的内容、最近收到的消息、屏幕各行的标记和文字、最近一次收到的
    /// 命令块。
    ///
    /// 用于判断超时的原因：最近的消息都在很久以前，说明辅助进程不再发送；屏幕和命令块已是
    /// 期望的样子，说明等待的条件有误。
    pub fn report(&self) -> String {
        let mut report = String::new();
        let waiting = if self.waiting.is_empty() {
            "（未说明）"
        } else {
            &self.waiting
        };
        let _ = writeln!(report, "正在等待：{waiting}");
        let _ = write!(report, "最近的消息（种类，距现在的毫秒数）：");
        for (at, kind) in &self.recent {
            let _ = write!(report, " {kind} {}", at.elapsed().as_millis());
        }
        let _ = writeln!(
            report,
            "\n屏幕（行号，标记 P 提示符、B 命令、C 输出、D 结束，文字）："
        );
        let flag = |marks: Marks, bit: u8, letter: char| {
            if marks.flags & bit == 0 { '-' } else { letter }
        };
        let last = self.screen.iter().rposition(|l| !l.trimmed().is_empty());
        for (row, line) in self.screen.iter().enumerate() {
            if last.is_none_or(|last| row > last) {
                break;
            }
            let m = line.marks;
            let _ = writeln!(
                report,
                "{row:4} {}{}{}{} {}",
                flag(m, Marks::PROMPT, 'P'),
                flag(m, Marks::COMMAND, 'B'),
                flag(m, Marks::OUTPUT, 'C'),
                flag(m, Marks::DONE, 'D'),
                line.trimmed()
            );
        }
        let _ = writeln!(report, "命令块：");
        for b in &self.blocks {
            let _ = writeln!(
                report,
                "  #{} 提示符 {} 命令 {} 输出 {:?} 结束 {:?} 退出码 {:?} 命令行 {:?}",
                b.id, b.prompt, b.command, b.output, b.end, b.exit, b.text
            );
        }
        report
    }

    /// 在标签 `tab` 里启动 `program`，环境里带一项 `WINC_SECRET` 用来检查它不会被带进 shell。
    pub fn spawn(&mut self, tab: u16, program: &str, args: &[&str]) {
        self.send(ToHelper::Spawn {
            tab,
            program: Some(program.into()),
            args: args.iter().map(|a| a.as_bytes().to_vec()).collect(),
            login: false,
            cwd: b"/".to_vec(),
            env: vec![
                (b"PATH".to_vec(), b"/usr/bin:/bin".to_vec()),
                (b"WINC_SECRET".to_vec(), b"1".to_vec()),
                (b"TERM".to_vec(), b"dumb".to_vec()),
            ],
            scrollback: 100,
            size: SIZE,
            integration: None,
            banner: Vec::new(),
        });
    }

    /// 像主程序那样启动一个交互式 shell：给定环境，带 shell 集成的目录。
    #[cfg(unix)]
    pub fn spawn_shell(
        &mut self,
        tab: u16,
        program: &str,
        login: bool,
        env: Vec<(Vec<u8>, Vec<u8>)>,
        integration: &std::path::Path,
    ) {
        use std::os::unix::ffi::OsStrExt;
        self.send(ToHelper::Spawn {
            tab,
            program: Some(program.into()),
            args: vec![],
            login,
            cwd: b"/".to_vec(),
            env,
            scrollback: 1000,
            // 高到测试里的命令都不会滚出屏幕：屏幕上的行号就是绝对行号。
            size: Size {
                cols: 80,
                rows: 100,
                width: 640,
                height: 1600,
            },
            integration: Some(integration.as_os_str().as_bytes().to_vec()),
            banner: Vec::new(),
        });
    }

    /// 把帧应用到还原的屏幕上，每收到一帧就再给额度。
    pub fn apply(&mut self, tab: u16, frame: &Frame) {
        let rows = usize::from(frame.rows);
        if frame.full || self.screen.len() != rows {
            self.screen = vec![Line::default(); rows];
        }
        self.screen.rotate_left(usize::from(frame.scroll).min(rows));
        for (row, line) in &frame.lines {
            self.screen[usize::from(*row)] = line.clone();
        }
        self.send(ToHelper::Credit { tab });
    }

    /// 还原出来的屏幕文字，每行去掉行尾空格、末尾的空行去掉。
    pub fn text(&self) -> String {
        let lines: Vec<&str> = self.screen.iter().map(Line::trimmed).collect();
        lines.join("\n").trim_end().to_string()
    }

    /// 一直接收消息，把每一帧应用到屏幕上，直到 `done` 返回真。
    ///
    /// # Panics
    ///
    /// 超过 [`WAIT`] 还没满足 `done` 时 panic。
    pub fn until(&mut self, mut done: impl FnMut(&Client, &FromHelper) -> bool) -> FromHelper {
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(
                Instant::now() < deadline,
                "消息不断到达，但等待的条件一直没有满足。\n{}",
                self.report()
            );
            let message = self.recv();
            if let FromHelper::Failed { message, .. } = &message {
                panic!("辅助进程报启动失败：{message}");
            }
            if let FromHelper::Frame { tab, frame } = &message {
                self.apply(*tab, frame);
            }
            if done(self, &message) {
                return message;
            }
        }
    }

    /// 等到辅助进程报退出，返回退出码。
    ///
    /// # Panics
    ///
    /// 超时或者收到的不是退出消息时 panic。
    pub fn until_exit(&mut self) -> i32 {
        match self.until(|_, m| matches!(m, FromHelper::Exited { .. })) {
            FromHelper::Exited { code, .. } => code,
            _ => unreachable!(),
        }
    }

    /// 等到辅助进程报退出且屏幕文字满足 `done`，返回退出码。
    ///
    /// 退出前读到的输出要等额度回来才成帧，可能晚于退出消息到达，因此报退出后继续等待屏幕。
    ///
    /// # Panics
    ///
    /// 超时时 panic。
    pub fn until_exit_showing(&mut self, done: impl Fn(&str) -> bool) -> i32 {
        let code = self.until_exit();
        if !done(&self.text()) {
            self.until(|c, _| done(&c.text()));
        }
        code
    }

    /// 等到屏幕文字包含 `text`。
    ///
    /// # Panics
    ///
    /// 超时还没出现时 panic。
    pub fn until_text(&mut self, text: &str) {
        self.waiting = format!("屏幕上出现「{text}」");
        self.until(|c, _| c.text().contains(text));
        self.waiting.clear();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
