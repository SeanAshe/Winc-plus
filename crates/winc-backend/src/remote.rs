//! 远端主机：到服务进程的连接，以及经连接执行的文件操作。
//!
//! 每个连接目标至多一个连接，登记在进程级的表中，[`fs`](crate::fs) 中的函数遇到虚拟路径时按
//! 目标取用。连接是一个子进程（通常是 `ssh`）的标准输入输出，对端是服务进程（Unix 上的 `serve`）。
//!
//! 请求是同步的：发出后等待应答。同一连接上可以同时有多个请求，各自等待自己的应答。会等待
//! 网络的调用不能发生在界面线程上，见 [`forbid_blocking`]。除文件操作外，还可以在远端运行
//! 命令，见 [`exec`]。

mod deploy;
mod files;
#[cfg(unix)]
mod server;
#[cfg(test)]
mod tests;
mod wire;

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

pub use deploy::{Abort, ConnectError, Deploy, Program, Stage};
#[cfg(unix)]
pub use server::{answer_from_server, serve};
pub(crate) use wire::MAX_READ;
use wire::{Frame, Reader, Writer, kind};

use crate::Target;
use crate::fs::{Kind, Metadata, SaveError};

/// 到服务进程的一个连接。
///
/// 多个线程可以同时发出请求，各自等待自己的应答：一个运行很久的命令不妨碍其他线程读取文件。
/// 丢弃时关闭子进程的标准输入并结束子进程；服务进程在输入结束时结束它启动的命令并退出。
pub struct Connection {
    link: Arc<Link>,
    reader: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

/// 连接中由发出请求的线程和读取应答的线程共用的部分。
struct Link {
    /// 写请求的一端；连接中断后为 `None`。
    output: Mutex<Option<BufWriter<Box<dyn Write + Send>>>>,
    waiting: Mutex<Waiting>,
    /// 承载连接的子进程；连接建立在其他字节流上时（见 [`Connection::over`]）没有。
    child: Mutex<Option<Child>>,
    /// 本程序是否已开始断开（见 [`Connection::shutdown`]）：此后连接的结束不算意外中断。
    shutting: AtomicBool,
    /// 已发出的请求数（见 [`requests`]）。
    sent: AtomicU64,
    /// 连接意外中断时调用一次的函数（见 [`Connection::on_close`]）。
    on_close: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// 最近一次收到对端的帧，或从没有请求变为有请求在等待的时刻：对端没有动静的时间由此算起。
    heard: Mutex<Instant>,
    /// 是否有请求在等待而对端迟迟没有动静（见 [`is_stalled`]）。
    stalled: AtomicBool,
    /// 上一项变化时调用的函数（见 [`Connection::on_stall`]）。
    on_stall: Mutex<Option<Box<dyn Fn() + Send>>>,
}

/// 等待应答的请求。
#[derive(Default)]
struct Waiting {
    /// 下一个请求号。
    next: u32,
    /// 请求号对应的接收端：应答的各帧按到达的顺序送入。
    replies: HashMap<u32, mpsc::Sender<Frame>>,
    /// 连接是否已中断：此后的请求立即失败。
    closed: bool,
}

impl Link {
    /// 登记一个新请求并写出，返回请求号和接收其应答的一端。
    fn send(&self, kind: u8, payload: &[u8]) -> io::Result<(u32, mpsc::Receiver<Frame>)> {
        let (sender, receiver) = mpsc::channel();
        let id = {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            if waiting.closed {
                return Err(unavailable(Unavailable::Lost));
            }
            let id = waiting.next;
            waiting.next = waiting.next.wrapping_add(1);
            // 此前没有请求在等待：对端此前的安静不算数，从现在算起。
            if waiting.replies.is_empty() {
                *self.heard.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
            }
            waiting.replies.insert(id, sender);
            id
        };
        self.sent.fetch_add(1, Ordering::Relaxed);
        // 放不进一帧的载荷：前面的部分先作为若干 `PART` 发出，对端接在一起。各段之间可以
        // 穿插其他请求的帧。
        let mut rest = payload;
        let sent = loop {
            if rest.len() <= wire::MAX_READ as usize {
                break self.write(kind, id, rest);
            }
            let (part, later) = rest.split_at(wire::MAX_READ as usize);
            if let Err(error) = self.write(kind::PART, id, part) {
                break Err(error);
            }
            rest = later;
        };
        if let Err(error) = sent {
            self.forget(id);
            return Err(error);
        }
        Ok((id, receiver))
    }

    /// 写出一帧；连接已中断时返回 [`io::ErrorKind::BrokenPipe`]。
    fn write(&self, kind: u8, id: u32, payload: &[u8]) -> io::Result<()> {
        if self.closed() {
            return Err(unavailable(Unavailable::Lost));
        }
        let mut output = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(output) = output.as_mut() else {
            return Err(unavailable(Unavailable::Lost));
        };
        wire::write_frame(output, kind, id, payload)?;
        output.flush()
    }

    /// 不再等待请求 `id` 的应答。
    fn forget(&self, id: u32) {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.replies.remove(&id);
    }

    /// 读取应答并分发给各请求，直到连接中断；此时所有等待中的请求都以连接中断结束。
    fn receive(&self, mut input: impl io::Read) {
        // 读到不合协议的帧、输入结束、读取出错：都无法继续使用该连接。
        while let Ok(Some(frame)) = wire::read_frame(&mut input) {
            *self.heard.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
            self.set_stalled(false);
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            // 输出的一段、输入请求之后还有后续的帧，保留接收端；其余类型是最后一帧。
            let sender = if matches!(frame.kind, kind::CHUNK | kind::ASK) {
                waiting.replies.get(&frame.id).cloned()
            } else {
                waiting.replies.remove(&frame.id)
            };
            drop(waiting);
            // 没有对应的请求：该请求已被放弃，丢弃其应答。
            if let Some(sender) = sender {
                let _ = sender.send(frame);
            }
        }
        let unexpected = !self.shutting.load(Ordering::Relaxed);
        self.close();
        if !unexpected {
            return;
        }
        // 对端或 `ssh` 自己结束了：连接已不可用，结束并回收子进程，不留下僵尸进程。
        if let Some(child) = (self.child.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        let hook = (self.on_close.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 记下对端是否迟迟没有动静；有变化时调用 [`Connection::on_stall`] 设置的函数。
    fn set_stalled(&self, stalled: bool) {
        if self.stalled.swap(stalled, Ordering::Relaxed) == stalled {
            return;
        }
        let hook = self.on_stall.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(hook) = hook.as_ref() {
            hook();
        }
    }

    /// 每隔 [`PROBE`] 检查一次，直到连接中断：有请求在等待而对端没有动静时发出探测，探测
    /// 之后仍没有动静、达到 [`STALL`] 时记为没有响应（见 [`check`]）。收到任何一帧（包括探测
    /// 的应答）即恢复。
    ///
    /// 只持有连接的弱引用：连接被丢弃后下一次检查时退出。
    fn watch(link: std::sync::Weak<Self>) {
        let mut probed = None;
        loop {
            thread::sleep(PROBE);
            let Some(link) = link.upgrade() else {
                return;
            };
            let waiting = {
                let waiting = link.waiting.lock().unwrap_or_else(PoisonError::into_inner);
                if waiting.closed {
                    return;
                }
                !waiting.replies.is_empty()
            };
            if !waiting {
                probed = None;
                continue;
            }
            // 先取当前时刻，再读 `heard`：两次读取之间到达的帧使 `heard` 不早于 `now`，不会
            // 被当作探测之前收到的帧。
            let now = Instant::now();
            let heard = *link.heard.lock().unwrap_or_else(PoisonError::into_inner);
            let (stalled, probe) = check(heard, &mut probed, now);
            if stalled {
                link.set_stalled(true);
            }
            // 运行很久而没有输出的命令也让对端安静：探测的应答把它与连接停滞区分开。应答
            // 不必等待，接收端随即丢弃。
            if probe {
                let _ = link.send(kind::PING, &[]);
            }
        }
    }

    /// 由本程序立即断开，不等对端收尾：等待中的请求随即失败，连接的结束不算意外中断。
    ///
    /// 子进程是进程组的组长时（见 [`Connection::deploy`]）连同它启动的进程一起结束：它们持有
    /// 输出的管道，留着的话读取应答的线程等不到结束。
    fn kill(&self) {
        self.shutting.store(true, Ordering::Relaxed);
        self.close();
        if let Some(child) = (self.child.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            #[cfg(unix)]
            deploy::kill_group(child);
            let _ = child.kill();
        }
    }

    /// 连接是否已中断。
    fn closed(&self) -> bool {
        (self.waiting.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .closed
    }

    /// 让所有等待中的请求立即以连接中断结束，并关闭写请求的一端；此后的请求立即失败。
    ///
    /// 不等待正在进行的写入：网络停滞时写入可能长时间不返回，它占着写的一端，这里就不关闭，
    /// 由随后结束子进程使其失败返回。
    fn close(&self) {
        {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            waiting.closed = true;
            // 丢弃发送端，等待中的线程从接收端得到「已断开」。
            waiting.replies.clear();
        }
        if let Ok(mut output) = self.output.try_lock() {
            *output = None;
        }
    }
}

/// 读到的一段数据，以及是否已到文件末尾。
pub(crate) type Chunk = (Vec<u8>, bool);

/// 把最后一帧换成结果：成功时是载荷，失败时是对端报告的错误。
fn finish(frame: Frame) -> io::Result<Vec<u8>> {
    match frame.kind {
        kind::OK => Ok(frame.payload),
        kind::ERROR => Err(wire::decode_error(&frame.payload)),
        _ => Err(wire::invalid("unknown reply")),
    }
}

impl Connection {
    /// 启动 `command` 作为连接，并与对端交换协议版本。
    ///
    /// `command` 的标准输入输出接到本进程，标准错误丢弃。
    ///
    /// # Errors
    ///
    /// 无法启动命令、对端不是服务进程，或协议版本不一致时返回对应的 `io::Error`。
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let (input, output) = (child.stdout.take(), child.stdin.take());
        let (Some(input), Some(output)) = (input, output) else {
            return Err(io::Error::other("connection has no standard streams"));
        };
        Self::establish(input, output, Some(child))
    }

    /// 在一对字节流上建立连接，并与对端交换协议版本：对端从 `output` 的另一端读取请求，把应答
    /// 写到 `input` 的另一端。
    ///
    /// 用于对端不是子进程的情形，例如测试中在同一进程内运行的服务端。
    ///
    /// # Errors
    ///
    /// 对端不是服务进程，或协议版本不一致时返回对应的 `io::Error`。
    pub fn over(
        input: impl io::Read + Send + 'static,
        output: impl Write + Send + 'static,
    ) -> io::Result<Self> {
        Self::establish(input, output, None)
    }

    fn establish(
        input: impl io::Read + Send + 'static,
        output: impl Write + Send + 'static,
        child: Option<Child>,
    ) -> io::Result<Self> {
        let connection = Self::attach(input, output, child);
        connection.hello()?;
        Ok(connection)
    }

    /// 在一对字节流上开始收发，尚未与对端交换协议版本（见 [`Connection::hello`]）。
    fn attach(
        input: impl io::Read + Send + 'static,
        output: impl Write + Send + 'static,
        child: Option<Child>,
    ) -> Self {
        let link = Arc::new(Link {
            output: Mutex::new(Some(BufWriter::new(Box::new(output)))),
            waiting: Mutex::default(),
            child: Mutex::new(child),
            shutting: AtomicBool::new(false),
            sent: AtomicU64::new(0),
            on_close: Mutex::default(),
            heard: Mutex::new(Instant::now()),
            stalled: AtomicBool::new(false),
            on_stall: Mutex::default(),
        });
        let reader = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.receive(BufReader::new(input)))
        };
        let watched = Arc::downgrade(&link);
        thread::spawn(move || Link::watch(watched));
        Self {
            link,
            reader: Some(reader),
        }
    }

    /// 与对端交换协议版本。
    fn hello(&self) -> io::Result<()> {
        let hello = Writer::default().u32(wire::VERSION).finish();
        let reply = self.request(kind::HELLO, &hello)?;
        let mut reply = Reader::new(&reply);
        if reply.u32()? != wire::VERSION {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "protocol version mismatch",
            ));
        }
        reply.finish()
    }

    /// 设置连接意外中断（对端退出、`ssh` 因网络中断结束）时调用的函数，在读取应答的线程上
    /// 调用一次；本程序主动断开时不调用。已有的函数被替换。
    pub fn on_close(&self, hook: impl FnOnce() + Send + 'static) {
        *(self.link.on_close.lock()).unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
    }

    /// 设置「对端是否迟迟没有动静」（见 [`is_stalled`]）变化时调用的函数，在连接自己的线程上
    /// 调用；已有的函数被替换。
    pub fn on_stall(&self, hook: impl Fn() + Send + 'static) {
        *(self.link.on_stall.lock()).unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
    }

    /// 发出一个请求并等待应答，返回应答的载荷。
    ///
    /// # Errors
    ///
    /// 对端应答的错误原样返回；连接中断时返回 [`io::ErrorKind::BrokenPipe`]，此后该连接上的
    /// 请求都会失败。
    fn request(&self, kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
        let (_, replies) = self.link.send(kind, payload)?;
        match replies.recv() {
            Ok(frame) => finish(frame),
            Err(_) => Err(unavailable(Unavailable::Lost)),
        }
    }

    /// 在一次往返中发出若干个请求（类型和载荷），按顺序返回各自的应答或错误。
    ///
    /// 只用于在对端直接应答的请求（见 [`kind::BATCH`]）。各应答合计不能超过一帧：调用方保证
    /// 这一点，超出的部分以错误返回。
    fn batch(&self, requests: &[(u8, Vec<u8>)]) -> io::Result<Vec<io::Result<Vec<u8>>>> {
        let count = u32::try_from(requests.len()).map_err(|_| wire::invalid("too many"))?;
        let mut payload = Writer::default().u32(count);
        for (kind, body) in requests {
            payload = payload.u8(*kind).bytes(body);
        }
        let reply = self.request(kind::BATCH, &payload.finish())?;
        let mut reply = Reader::new(&reply);
        let mut results = Vec::with_capacity(requests.len());
        for _ in requests {
            let ok = reply.u8()? != 0;
            let body = reply.bytes()?;
            results.push(if ok {
                Ok(body.to_vec())
            } else {
                Err(wire::decode_error(body))
            });
        }
        reply.finish()?;
        Ok(results)
    }

    /// 发出一个应答为空的请求。
    fn command(&self, kind: u8, payload: Vec<u8>) -> io::Result<()> {
        let reply = self.request(kind, &payload)?;
        Reader::new(&reply).finish()
    }

    /// 在远端运行 `exec` 描述的命令并等待它结束，返回退出码和全部输出。
    ///
    /// 等待期间每隔 [`CANCEL_POLL`] 调用 `cancel`：返回真时通知对端强制结束命令（连同它启动的
    /// 子进程），命令结束后返回 [`io::ErrorKind::Interrupted`]。
    fn exec(
        &self,
        exec: &Exec,
        cancel: &dyn Fn() -> bool,
        answer: &dyn Fn(&Ask) -> Option<Vec<u8>>,
    ) -> io::Result<Output> {
        let (id, replies) = self.link.send(kind::EXEC, &exec.encode())?;
        let mut output = Output::default();
        let mut asked = false;
        loop {
            let frame = match replies.recv_timeout(CANCEL_POLL) {
                Ok(frame) => frame,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(unavailable(Unavailable::Lost));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !asked && cancel() {
                        asked = true;
                        self.link.write(kind::CANCEL, id, &[])?;
                    }
                    continue;
                }
            };
            // 命令在等用户输入：答复之前它不会继续，也不会有别的输出。
            if frame.kind == kind::ASK {
                let mut request = Reader::new(&frame.payload);
                let ticket = request.u32()?;
                let ask = Ask {
                    mode: request.bytes()?.to_vec(),
                    prompt: request.bytes()?.to_vec(),
                };
                request.finish()?;
                let reply = answer(&ask);
                let payload = Writer::default()
                    .u32(ticket)
                    .flag(reply.is_some())
                    .bytes(&reply.unwrap_or_default())
                    .finish();
                self.link.write(kind::ANSWER, id, &payload)?;
                continue;
            }
            if frame.kind != kind::CHUNK {
                let reply = finish(frame)?;
                let mut reply = Reader::new(&reply);
                let has_code = reply.u8()? != 0;
                let code = reply.u32()?.cast_signed();
                reply.finish()?;
                output.code = has_code.then_some(code);
                return Ok(output);
            }
            let mut chunk = Reader::new(&frame.payload);
            let stream = chunk.u8()?;
            let data = chunk.bytes()?;
            chunk.finish()?;
            match stream {
                kind::STDOUT => output.stdout.extend_from_slice(data),
                kind::STDERR => output.stderr.extend_from_slice(data),
                _ => return Err(wire::invalid("unknown output stream")),
            }
            // 输出超过上限：不再接收，通知对端结束命令。
            if exec
                .limit
                .is_some_and(|limit| output.stdout.len() + output.stderr.len() > limit)
            {
                let _ = self.link.write(kind::CANCEL, id, &[]);
                self.link.forget(id);
                return Err(io::ErrorKind::FileTooLarge.into());
            }
        }
    }
}

/// 关闭输入之后等待对端自行退出的最长时间（秒），超过后强制结束子进程。
///
/// 对端在输入结束时先结束它启动的命令再退出，自身最多等待 2 秒；这里多留 1 秒，正常情况下
/// 不会抢在它之前结束子进程，否则远端的命令无人结束。
const SHUTDOWN: Duration = Duration::from_secs(3);

/// 请子进程结束之后等待它退出的最长时间（秒），超过后强制结束。`ssh` 收到信号后只需关闭
/// 连接并删除控制套接字，不等待网络。
const TERMINATE: Duration = Duration::from_secs(1);

/// 请子进程自行结束（`SIGTERM`），返回是否已发出；没有这种信号的系统上返回假。
// 本函数只有对 `libc::kill` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[cfg(unix)]
#[allow(unsafe_code)]
fn terminate(child: &Child) -> bool {
    let Ok(id) = i32::try_from(child.id()) else {
        return false;
    };
    // SAFETY: `child` 尚未被回收，其进程号不会被其他进程占用。`kill` 只发送信号，不访问内存，
    // 失败时返回 -1。
    unsafe { libc::kill(id, libc::SIGTERM) == 0 }
}

#[cfg(not(unix))]
fn terminate(_: &Child) -> bool {
    false
}

impl Connection {
    /// 断开：关闭输入，等待对端结束它启动的命令并退出，最长 [`SHUTDOWN`]，之后强制结束子进程。
    ///
    /// 仍在等待应答的请求立即以 [`io::ErrorKind::BrokenPipe`] 结束，不等对端退出。
    fn shutdown(&self) {
        self.link.shutting.store(true, Ordering::Relaxed);
        self.link.close();
        let mut child = (self.link.child.lock()).unwrap_or_else(PoisonError::into_inner);
        let Some(child) = child.as_mut() else {
            return;
        };
        // 已退出，或无法查询：不再等待。
        let exited = |child: &mut Child, wait: Duration| {
            let deadline = Instant::now() + wait;
            while Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            false
        };
        if exited(child, SHUTDOWN) {
            return;
        }
        // 被复用的 `ssh` 在别的会话（终端）结束前不退出：先请它结束，让它删除控制套接字。
        if terminate(child) && exited(child, TERMINATE) {
            return;
        }
        let _ = child.kill();
    }

    /// 在后台线程上断开，调用方不等待。
    fn shutdown_in_background(self: Arc<Self>) {
        thread::spawn(move || self.shutdown());
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shutdown();
        // 回收子进程，不留下僵尸进程。
        if let Some(child) = (self.link.child.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            let _ = child.wait();
        }
        // 子进程结束后其输出关闭，读取应答的线程随之退出。
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// 保存时一段内容的大小（字节，256 KiB）。
///
/// 每段是一帧，发送期间占用连接的写入一端；取值使其他请求在两段之间最多等待一段的发送时间，
/// 同时不让 16 MiB 的文档拆成过多的帧（64 帧）。
const UPLOAD_CHUNK: usize = 256 << 10;

/// 保存的内容的写入端：攒够 [`UPLOAD_CHUNK`] 就作为一帧发出。
struct Upload<'a> {
    link: &'a Link,
    id: u32,
    pending: Vec<u8>,
}

impl Write for Upload<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(data);
        while self.pending.len() >= UPLOAD_CHUNK {
            let rest = self.pending.split_off(UPLOAD_CHUNK);
            self.link.write(kind::SAVE_DATA, self.id, &self.pending)?;
            self.pending = rest;
        }
        Ok(data.len())
    }

    /// 发出尚未攒够一段的内容。
    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            self.link.write(kind::SAVE_DATA, self.id, &self.pending)?;
            self.pending.clear();
        }
        Ok(())
    }
}

/// 同一个连接上的两个远端路径：连接、第一个路径、第二个路径。
pub(crate) type Pair = (Arc<Connection>, Vec<u8>, Vec<u8>);

/// 把两个路径解析到同一个连接上：都是本机路径时返回 `None`，由调用方在本机执行。
///
/// # Errors
///
/// 一个在本机、一个在远端，或分属两个连接时返回 [`io::ErrorKind::Unsupported`]：改名不能跨
/// 主机，复制由调用方先分出跨主机的情况。其余同 [`connection`]。
pub(crate) fn pair(from: &Path, to: &Path) -> io::Result<Option<Pair>> {
    match (crate::locate(from), crate::locate(to)) {
        (None, None) => Ok(None),
        (Some(from), Some(to)) if from.target == to.target => {
            Ok(Some((connection(&from.target)?, from.path, to.path)))
        }
        _ => Err(io::ErrorKind::Unsupported.into()),
    }
}

/// 有请求在等待而对端没有动静时，每隔多久探测一次（5 秒）。
///
/// 取值与常见编辑器的远程连接保持活动的间隔一致。没有请求在等待时不探测。本 crate 的测试中
/// 缩短为 50 ms。
const PROBE: Duration = Duration::from_millis(if cfg!(test) { 50 } else { 5000 });

/// 有请求在等待而对端安静多久之后记为没有响应（20 秒）。
///
/// 取值与常见编辑器判定远程连接超时的时间一致。这里只用于提示，不断开连接：何时放弃由
/// `ssh` 按用户的设置决定。本 crate 的测试中缩短为 200 ms。
const STALL: Duration = Duration::from_millis(if cfg!(test) { 200 } else { 20_000 });

/// 在时刻 `now` 检查有请求在等待的连接，返回（是否记为没有响应，是否发出探测）。
///
/// `heard` 是对端没有动静的时间的起点（见 [`Link::heard`]）；`probed` 是此后最早一次探测的
/// 时刻，由本函数维护：`heard` 不早于该时刻时清除，决定探测而尚无记录时记为 `now`。
///
/// 没有动静的时间达到 [`STALL`]、并且已有探测在其后没有收到任何帧时，才记为没有响应。检查
/// 被推迟时（进程被暂停、系统从睡眠中恢复）该时间同样达到 [`STALL`]，但尚未探测，只发出探测。
fn check(heard: Instant, probed: &mut Option<Instant>, now: Instant) -> (bool, bool) {
    if probed.is_some_and(|sent| heard >= sent) {
        *probed = None;
    }
    let quiet = now.saturating_duration_since(heard);
    let stalled = quiet >= STALL && probed.is_some();
    let probe = quiet >= PROBE;
    if probe {
        probed.get_or_insert(now);
    }
    (stalled, probe)
}

/// 等待命令结束时检查是否被取消的间隔（毫秒）。取消由用户的操作或超时触发，50 ms 以内的
/// 延迟不可察觉；间隔内没有新的输出时线程才被唤醒一次。
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// 要在远端运行的命令。
///
/// 各部分是远端主机上的原始字节。命令继承服务进程的环境变量，[`Exec::env`] 设置的在其上覆盖。
#[derive(Debug, Clone, Default)]
pub struct Exec {
    program: Vec<u8>,
    cwd: Vec<u8>,
    args: Vec<Vec<u8>>,
    envs: Vec<(Vec<u8>, Vec<u8>)>,
    stdin: Vec<u8>,
    /// 命令需要用户输入时是否转给本机答复，见 [`Exec::asking`]。
    ask: bool,
    /// 输出（标准输出与标准错误合计）的字节数上限；只在本机一侧使用，不发给对端。
    limit: Option<usize>,
}

/// 远端运行的命令提出的一次输入请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// 提问的方式：`ssh` 设置的 `SSH_ASKPASS_PROMPT`（`confirm`、`none`），没有时为空。
    pub mode: Vec<u8>,
    /// 提示的文字。
    pub prompt: Vec<u8>,
}

/// 远端的命令向用户提问时，服务进程交给它的 askpass 程序（服务进程自身）用来找到服务进程的
/// 环境变量：值是一个只有用户本人可以访问的套接字的路径。
pub const ASKPASS_SOCKET: &str = "WINC_ASKPASS_SOCKET";

impl Exec {
    /// 创建运行程序 `program` 的命令；`program` 不含 `/` 时在远端的 `PATH` 中查找。
    pub fn new(program: impl AsRef<[u8]>) -> Self {
        Self {
            program: program.as_ref().to_vec(),
            ..Self::default()
        }
    }

    /// 创建再次运行服务进程自身的命令，功能由随后的参数决定（例如 `watch <目录>`）。
    pub fn this_server() -> Self {
        Self::new(wire::SELF)
    }

    /// 追加一个参数。
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<[u8]>) -> Self {
        self.args.push(arg.as_ref().to_vec());
        self
    }

    /// 设置输出的字节数上限：超过时结束命令，[`exec`] 返回 [`io::ErrorKind::FileTooLarge`]。
    #[must_use]
    pub fn limit(mut self, bytes: usize) -> Self {
        self.limit = Some(bytes);
        self
    }

    /// 设置一个环境变量。
    #[must_use]
    pub fn env(mut self, name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        self.envs
            .push((name.as_ref().to_vec(), value.as_ref().to_vec()));
        self
    }

    /// 让命令（`git`，以及它启动的 `ssh`）需要用户输入时把请求转回本机，由 [`exec_asking`] 的
    /// 调用方答复。对端为此把 `GIT_ASKPASS`、`SSH_ASKPASS` 设为服务进程自身。
    #[must_use]
    pub fn asking(mut self) -> Self {
        self.ask = true;
        self
    }

    /// 设置写入命令标准输入的内容；命令、参数和输入合计不能超过一帧（约 1 MiB）。
    #[must_use]
    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = stdin.into();
        self
    }

    /// 编码为 [`kind::EXEC`] 的载荷。
    fn encode(&self) -> Vec<u8> {
        let mut out = Writer::default()
            .bytes(&self.program)
            .bytes(&self.cwd)
            .u32(self.args.len() as u32);
        for arg in &self.args {
            out = out.bytes(arg);
        }
        out = out.u32(self.envs.len() as u32);
        for (name, value) in &self.envs {
            out = out.bytes(name).bytes(value);
        }
        out.bytes(&self.stdin).flag(self.ask).finish()
    }
}

/// 命令运行的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// 退出码；命令被信号结束时为 `None`。
    pub code: Option<i32>,
    /// 标准输出的全部内容。
    pub stdout: Vec<u8>,
    /// 标准错误的全部内容。
    pub stderr: Vec<u8>,
}

/// 在目录 `dir` 所在的主机上以 `dir` 为工作目录运行 `exec`，等待它结束。
///
/// 等待期间反复调用 `cancel`，返回真时强制结束命令；调用方用它实现取消和超时。
///
/// # Errors
///
/// `dir` 不是虚拟路径时返回 [`io::ErrorKind::InvalidInput`]：本机的命令由调用方直接启动。
/// 没有连接、当前线程不允许等待、连接中断时返回连接的错误（见 [`fs`](crate::fs) 的说明）；
/// 远端无法启动命令时返回对端报告的错误；被取消时返回 [`io::ErrorKind::Interrupted`]。
/// 命令以非零的退出码结束不是错误，见 [`Output::code`]。
pub fn exec(dir: &Path, exec: &Exec, cancel: &dyn Fn() -> bool) -> io::Result<Output> {
    exec_asking(dir, exec, cancel, &|_| None)
}

/// 同 [`exec`]；命令提出输入请求时（见 [`Exec::asking`]）调用 `answer`，返回 `None` 表示
/// 不答复，命令按用户取消处理。`answer` 在调用本函数的线程上执行，可以等待用户。
///
/// # Errors
///
/// 同 [`exec`]。
pub fn exec_asking(
    dir: &Path,
    exec: &Exec,
    cancel: &dyn Fn() -> bool,
    answer: &dyn Fn(&Ask) -> Option<Vec<u8>>,
) -> io::Result<Output> {
    let Some(at) = crate::locate(dir) else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    let mut exec = exec.clone();
    exec.cwd = at.path;
    connection(&at.target)?.exec(&exec, cancel, answer)
}

/// 在远端运行、输出随到随读的命令，由 [`spawn`] 启动。
///
/// 实现了 [`io::Read`]：读到的是标准输出，命令结束且输出读完时返回 0。标准错误另行累积，
/// 见 [`Running::stderr`]。丢弃时命令若仍在运行，通知对端强制结束它。
///
/// 对端不等读取就持续发送输出：尚未读取的部分留在本机的内存中，合计不超过
/// [`Exec::limit`] 设置的上限。
pub struct Running {
    connection: Arc<Connection>,
    id: u32,
    replies: mpsc::Receiver<Frame>,
    /// 已收到、尚未读走的标准输出，以及其中已读走的字节数。
    stdout: (Vec<u8>, usize),
    stderr: Vec<u8>,
    received: usize,
    limit: Option<usize>,
    /// 命令已结束时为其退出码（被信号结束时是 `None`）。
    code: Option<Option<i32>>,
    /// 读取等待的最后期限，见 [`Running::read_until`]。
    deadline: Option<Instant>,
}

impl Running {
    /// 设置此后读取的最后期限：到时仍没有输出时读取返回 [`io::ErrorKind::TimedOut`]。
    pub fn read_until(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    /// 命令已结束时返回其退出码（被信号结束时是 `None`）；仍在运行时返回外层的 `None`。
    pub fn code(&self) -> Option<Option<i32>> {
        self.code
    }

    /// 返回到目前为止收到的标准错误。
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// 返回可以在其他线程上结束这条命令的句柄：读取的线程正等待输出时，由它来结束。
    pub fn stopper(&self) -> Stopper {
        Stopper {
            connection: Arc::clone(&self.connection),
            id: self.id,
        }
    }

    /// 收下一帧：输出累积起来，最后一帧记下退出码。
    fn accept(&mut self, frame: Frame) -> io::Result<()> {
        if frame.kind != kind::CHUNK {
            let reply = match finish(frame) {
                Ok(reply) => reply,
                // 命令被结束（见 `Stopper`）：没有退出码，输出到此为止，不是读取的错误。
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    self.code = Some(None);
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let mut reply = Reader::new(&reply);
            let has_code = reply.u8()? != 0;
            let code = reply.u32()?.cast_signed();
            reply.finish()?;
            self.code = Some(has_code.then_some(code));
            return Ok(());
        }
        let mut chunk = Reader::new(&frame.payload);
        let stream = chunk.u8()?;
        let data = chunk.bytes()?;
        chunk.finish()?;
        self.received += data.len();
        if self.limit.is_some_and(|limit| self.received > limit) {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        match stream {
            kind::STDOUT => self.stdout.0.extend_from_slice(data),
            kind::STDERR => self.stderr.extend_from_slice(data),
            _ => return Err(wire::invalid("unknown output stream")),
        }
        Ok(())
    }
}

/// 结束一条由 [`spawn`] 启动的命令，见 [`Running::stopper`]。
pub struct Stopper {
    connection: Arc<Connection>,
    id: u32,
}

impl Stopper {
    /// 通知对端强制结束命令（连同它启动的子进程）；命令已经结束或连接已断开时没有效果。
    /// 读取的一方随后读到输出结束。
    pub fn stop(&self) {
        let _ = self.connection.link.write(kind::CANCEL, self.id, &[]);
    }
}

impl io::Read for Running {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let (data, taken) = &mut self.stdout;
            if *taken < data.len() {
                let count = buf.len().min(data.len() - *taken);
                buf[..count].copy_from_slice(&data[*taken..*taken + count]);
                *taken += count;
                if *taken == data.len() {
                    data.clear();
                    *taken = 0;
                }
                return Ok(count);
            }
            if self.code.is_some() || buf.is_empty() {
                return Ok(0);
            }
            let frame = match self.deadline {
                None => self
                    .replies
                    .recv()
                    .map_err(|_| unavailable(Unavailable::Lost))?,
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match self.replies.recv_timeout(left) {
                        Ok(frame) => frame,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            return Err(io::ErrorKind::TimedOut.into());
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(unavailable(Unavailable::Lost));
                        }
                    }
                }
            };
            self.accept(frame)?;
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.code.is_none() {
            let _ = self.connection.link.write(kind::CANCEL, self.id, &[]);
            self.connection.link.forget(self.id);
        }
    }
}

/// 在目录 `dir` 所在的主机上以 `dir` 为工作目录启动 `exec`，返回按需读取其输出的 [`Running`]。
///
/// # Errors
///
/// 同 [`exec`]；命令启动之后的错误在读取时返回。
pub fn spawn(dir: &Path, exec: &Exec) -> io::Result<Running> {
    let Some(at) = crate::locate(dir) else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    let mut exec = exec.clone();
    exec.cwd = at.path;
    // 输出随到随读的命令没有答复输入请求的一方。
    exec.ask = false;
    let connection = connection(&at.target)?;
    let (id, replies) = connection.link.send(kind::EXEC, &exec.encode())?;
    Ok(Running {
        connection,
        id,
        replies,
        stdout: (Vec::new(), 0),
        stderr: Vec::new(),
        received: 0,
        limit: exec.limit,
        code: None,
        deadline: None,
    })
}

/// 连接不可用的情形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// 连接已中断：正在等待和此后的请求都失败。
    Lost,
    /// 没有到该目标的连接。
    NotConnected,
}

/// 把连接不可用的情形写成给用户看的说明的函数，由 [`describe_unavailable`] 设置。
static DESCRIBE: Mutex<Option<Describe>> = Mutex::new(None);

/// 把连接不可用的情形写成说明文字的函数。
type Describe = fn(Unavailable) -> String;

/// 设置连接不可用时错误的说明文字：此后这类错误的 `Display` 是 `describe` 的返回值，在错误
/// 产生时调用。没有设置时是英文的简短说明。错误的种类（[`io::ErrorKind`]）不受影响。
///
/// 这类错误会原样出现在「无法保存：……」一类的提示里，由持有界面语言的一层给出说明。
pub fn describe_unavailable(describe: Describe) {
    *DESCRIBE.lock().unwrap_or_else(PoisonError::into_inner) = Some(describe);
}

/// 返回连接不可用的错误：中断时种类是 [`io::ErrorKind::BrokenPipe`]，没有连接时是
/// [`io::ErrorKind::NotConnected`]。
pub(crate) fn unavailable(what: Unavailable) -> io::Error {
    let describe = *DESCRIBE.lock().unwrap_or_else(PoisonError::into_inner);
    let (kind, fallback) = match what {
        Unavailable::Lost => (io::ErrorKind::BrokenPipe, "the connection was lost"),
        Unavailable::NotConnected => (io::ErrorKind::NotConnected, "not connected to the host"),
    };
    let message = describe.map_or_else(|| fallback.to_owned(), |describe| describe(what));
    io::Error::new(kind, message)
}

/// 进程中已建立的连接，按连接目标登记。
static CONNECTIONS: Mutex<Option<HashMap<Target, Arc<Connection>>>> = Mutex::new(None);

/// 登记到 `target` 的连接，此后该目标的虚拟路径经它访问；已有的连接被替换并断开。
pub fn register(target: Target, connection: Connection) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let replaced = connections
        .get_or_insert_with(HashMap::new)
        .insert(target, Arc::new(connection));
    drop(connections);
    if let Some(replaced) = replaced {
        replaced.shutdown_in_background();
    }
}

/// 断开到 `target` 的连接；没有连接时没有效果。
///
/// 不等待断开完成。正在进行的请求随后以 [`io::ErrorKind::BrokenPipe`] 结束，远端仍在运行的
/// 命令被强制结束。
pub fn disconnect(target: &Target) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = connections.as_mut().and_then(|all| all.remove(target));
    drop(connections);
    if let Some(removed) = removed {
        removed.shutdown_in_background();
    }
}

/// 断开全部连接；其余同 [`disconnect`]。
pub fn disconnect_all() {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = connections.take();
    drop(connections);
    for connection in removed.into_iter().flat_map(HashMap::into_values) {
        connection.shutdown_in_background();
    }
}

/// 是否有到 `target` 的可用连接：已登记，且没有中断。
pub fn is_connected(target: &Target) -> bool {
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    (connections.as_ref())
        .and_then(|all| all.get(target))
        .is_some_and(|connection| !connection.link.closed())
}

/// 是否到 `target` 的连接上有请求在等待，而对端已有 20 秒没有任何动静：网络多半已经
/// 停滞，`ssh` 还没有判定连接中断。连接仍然保留，对端恢复应答后随即为假。
pub fn is_stalled(target: &Target) -> bool {
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    (connections.as_ref())
        .and_then(|all| all.get(target))
        .is_some_and(|connection| {
            !connection.link.closed() && connection.link.stalled.load(Ordering::Relaxed)
        })
}

/// 返回到 `target` 的连接至今发出的请求数；没有连接时为 0。
///
/// 每个请求要等一次网络往返，往返次数决定了延迟较高的连接上各项操作的快慢；用于测量和测试。
pub fn requests(target: &Target) -> u64 {
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    (connections.as_ref())
        .and_then(|all| all.get(target))
        .map_or(0, |connection| connection.link.sent.load(Ordering::Relaxed))
}

/// 返回有可用连接的各个目标，按名字排序。
pub fn connected() -> Vec<Target> {
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut targets: Vec<Target> = (connections.iter().flatten())
        .filter(|(_, connection)| !connection.link.closed())
        .map(|(target, _)| target.clone())
        .collect();
    targets.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    targets
}

/// 返回到 `target` 的连接。
///
/// # Errors
///
/// 当前线程不允许等待（见 [`forbid_blocking`]）时返回 [`io::ErrorKind::WouldBlock`]；没有到
/// 该目标的连接时返回 [`io::ErrorKind::NotConnected`]。
pub(crate) fn connection(target: &Target) -> io::Result<Arc<Connection>> {
    if BLOCKING_FORBIDDEN.get() {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    (connections.as_ref())
        .and_then(|all| all.get(target))
        .cloned()
        .ok_or_else(|| unavailable(Unavailable::NotConnected))
}

thread_local! {
    /// 是否不允许当前线程等待远端的应答。
    static BLOCKING_FORBIDDEN: Cell<bool> = const { Cell::new(false) };
}

/// 不允许当前线程等待远端的应答：此后该线程上对虚拟路径的操作立即返回
/// [`io::ErrorKind::WouldBlock`]，不发出请求。
///
/// 界面线程在启动时调用。远端的应答可能在数秒之后才到达，界面线程等待期间无法处理输入和
/// 重绘；访问远端文件的操作必须放到后台线程。本机路径不受影响。
pub fn forbid_blocking() {
    BLOCKING_FORBIDDEN.set(true);
}

/// 是否允许当前线程等待远端的应答（见 [`forbid_blocking`]）。
///
/// 把远端的操作分到别的线程上再等它们完成，同样是等待：这样做之前先问这里。
pub fn may_block() -> bool {
    !BLOCKING_FORBIDDEN.get()
}

/// 在 `work` 期间允许当前线程等待远端的应答，结束后恢复原来的限制。
///
/// 用于没有后台线程的环境（测试）中就地执行本应在后台进行的任务。
pub fn allow_blocking<R>(work: impl FnOnce() -> R) -> R {
    let forbidden = BLOCKING_FORBIDDEN.replace(false);
    let result = work();
    BLOCKING_FORBIDDEN.set(forbidden);
    result
}

/// 各个目标登录时使用的密钥文件，由 [`set_identity`] 登记。
static IDENTITIES: Mutex<Option<HashMap<Target, String>>> = Mutex::new(None);

/// 登记连接 `target` 时使用的密钥文件 `key`（`ssh -i`）；`None` 或空的路径去掉登记，由用户的
/// `ssh` 配置和 agent 决定。此后为该目标构造的 `ssh` 命令都带上它。
///
/// 路径原样交给 `ssh`，其中开头的 `~` 由 `ssh` 展开；这里不读取文件。
pub fn set_identity(target: &Target, key: Option<&str>) {
    let mut keys = IDENTITIES.lock().unwrap_or_else(PoisonError::into_inner);
    let keys = keys.get_or_insert_default();
    match key.filter(|key| !key.is_empty()) {
        Some(key) => keys.insert(target.clone(), key.to_owned()),
        None => keys.remove(target),
    };
}

/// 返回为 `target` 登记的密钥文件（见 [`set_identity`]）。
pub fn identity(target: &Target) -> Option<String> {
    let keys = IDENTITIES.lock().unwrap_or_else(PoisonError::into_inner);
    keys.as_ref()?.get(target).cloned()
}

/// 套接字地址中路径的最大长度（字节）：取所支持的系统中最小的一个（macOS 的 `sun_path` 为
/// 104 字节，Linux 为 108）。
const SOCKET_PATH_MAX: usize = 104;

/// 控制套接字文件名的长度（字节）：前缀 `winc-`、`%C` 展开成的 40 个十六进制字符，以及
/// `ssh` 创建套接字时临时加上的 17 个字符的后缀。
const CONTROL_NAME: usize = 5 + 40 + 17;

/// 已经以可复用的方式建立过连接的目标（见 [`Ssh::share`]）。
static SHARED: Mutex<Vec<Target>> = Mutex::new(Vec::new());

/// 返回供复用的连接使用的控制套接字（`ControlPath` 的值）：用户的 `~/.ssh` 目录中，按目标
/// 区分（`%C` 是 `ssh` 对本机名、主机、端口和用户算出的摘要）。
///
/// `~/.ssh` 只有用户本人可以进入，套接字由 `ssh` 以只有本人可以访问的权限创建。不支持连接
/// 复用的系统、目录不存在或路径放不进套接字地址时返回 `None`，这时不复用。
fn control_path() -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    let dir = std::path::PathBuf::from(std::env::var_os("HOME")?).join(".ssh");
    let text = dir.to_str()?;
    let fits = text.len() + 1 + CONTROL_NAME < SOCKET_PATH_MAX;
    // `%` 在 `ControlPath` 中引出替换，目录本身的 `%` 写成 `%%`。
    (fits && dir.is_dir()).then(|| format!("{}/winc-%C", text.replace('%', "%%")))
}

/// 登录 `target` 的另一条 `ssh`（终端）复用已建立的连接所需的选项；该目标没有以可复用的
/// 方式连接过时返回 `None`。
///
/// `ControlMaster=no` 使它只使用已有的连接，自己不成为被复用的一方；已有的连接不在了，或
/// 服务器不允许再开会话时，`ssh` 自行改为单独登录。
pub fn shared(target: &Target) -> Option<[String; 4]> {
    let known = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
    let path = control_path().filter(|_| known.contains(target))?;
    Some([
        "-o".to_owned(),
        "ControlMaster=no".to_owned(),
        "-o".to_owned(),
        format!("ControlPath={path}"),
    ])
}

/// `ssh` 的可选行为。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ssh {
    /// 不向用户提问（`BatchMode=yes`）：需要输入口令或确认主机密钥时直接失败。
    pub batch: bool,
    /// 让 `ssh` 定期探测对端（`ServerAliveInterval=60`），网络中断后自行结束，连接上等待中的
    /// 请求随之失败。命令行上的选项优先于用户的配置，只应在用户没有设置时使用（见
    /// [`Ssh::filling`]）。
    pub keep_alive: bool,
    /// 限制建立网络连接的时间（`ConnectTimeout=15`）：主机没有应答时不必等到系统的时限。
    /// 不限制登录，等待用户输入口令的时间不计入。同样只应在用户没有设置时使用。
    pub connect_timeout: bool,
    /// 让这条连接可以被复用（`ControlMaster=auto`）：此后登录同一目标的 `ssh`（终端）经它
    /// 开会话，不再握手和认证。控制套接字在用户的 `~/.ssh` 目录中；连接结束时复用随之结束
    /// （`ControlPersist=no`），不留下后台的 `ssh`。同样只应在用户没有设置时使用。
    pub share: bool,
}

impl Ssh {
    /// 返回为 `target` 补上用户没有设置的各项之后的行为，不向用户提问与否由调用方决定。
    ///
    /// 运行 `ssh -G` 读取对该主机生效的配置，不建立连接。无法运行时什么都不补：不能确定用户
    /// 没有设置时，不替用户添加。
    pub fn filling(target: &Target) -> Self {
        let mut command = Command::new("ssh");
        hide_console(&mut command);
        if let Some(port) = target.port() {
            command.arg("-p").arg(port.to_string());
        }
        command.arg("-G").arg(target.host()).stdin(Stdio::null());
        match command.output() {
            Ok(output) => Self::missing(&String::from_utf8_lossy(&output.stdout)),
            Err(_) => Self::default(),
        }
    }

    /// 按 `ssh -G` 输出的生效配置 `config`，返回其中没有设置、需要补上的各项。
    ///
    /// 没有设置的探测间隔输出为 `0`，没有设置的连接时限输出为 `none`；读不到这两项时不补。
    /// 输出为空（无法读取配置）时什么都不补。
    fn missing(config: &str) -> Self {
        let value = |name: &str| {
            config.lines().find_map(|line| {
                let (key, value) = line.split_once(' ')?;
                key.eq_ignore_ascii_case(name).then_some(value.trim())
            })
        };
        if config.trim().is_empty() {
            return Self::default();
        }
        Self {
            batch: false,
            keep_alive: value("serveraliveinterval") == Some("0"),
            connect_timeout: matches!(value("connecttimeout"), Some("none" | "0" | "-1")),
            // 没有设置时 `controlmaster` 输出为 `false`，`controlpath` 一行不出现。
            share: value("controlmaster") == Some("false")
                && matches!(value("controlpath"), None | Some("none")),
        }
    }
}

/// 用户没有设置时使用的探测间隔：`ssh` 每 60 秒探测一次，连续 3 次（OpenSSH 的默认次数）
/// 没有应答后结束连接。
const KEEP_ALIVE: &str = "ServerAliveInterval=60";

/// 用户没有设置时使用的连接时限（15 秒）：超过后 `ssh` 报告超时并结束。系统的时限由 TCP
/// 重传决定，通常在一分钟以上；15 秒足以覆盖高延迟的网络，又不让用户对着无应答的主机久等。
const CONNECT_TIMEOUT: &str = "ConnectTimeout=15";

/// Windows 上图形程序启动命令行程序时默认弹出控制台窗口：`CREATE_NO_WINDOW`（`0x0800_0000`）
/// 使其不创建窗口。其他系统上没有效果。
fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(command, 0x0800_0000);
    #[cfg(not(windows))]
    let _ = command;
}

/// 返回经 `ssh` 连接 `target` 并在远端运行程序 `server` 的命令。
///
/// 目标和 `ssh` 的其余设置取自用户的 `~/.ssh/config`。`-T` 不分配终端，`RemoteCommand=none`
/// 使配置中为该主机设置的登录命令不替换这里的程序；`server` 作为一个整体交给远端的 shell，
/// 其中的字符不被解释。
pub fn ssh_command(target: &Target, server: &str, options: Ssh) -> Command {
    let mut command = ssh(target, options);
    command.arg(quote(server));
    command
}

/// 返回连接 `target` 的 `ssh` 命令，其后追加的参数是在远端运行的命令；参数的含义见
/// [`ssh_command`]。选项都写在主机之前：主机之后的参数属于远端的命令。
fn ssh(target: &Target, options: Ssh) -> Command {
    let mut command = Command::new("ssh");
    hide_console(&mut command);
    command.args(["-T", "-o", "RemoteCommand=none"]);
    if options.batch {
        command.args(["-o", "BatchMode=yes"]);
    }
    if options.keep_alive {
        command.args(["-o", KEEP_ALIVE]);
    }
    if options.connect_timeout {
        command.args(["-o", CONNECT_TIMEOUT]);
    }
    if let Some(path) = control_path().filter(|_| options.share) {
        command.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=no", "-o"]);
        command.arg(format!("ControlPath={path}"));
        let mut known = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if !known.contains(target) {
            known.push(target.clone());
        }
    }
    // 密钥文件是 `-i` 的值，单独一个参数：以 `-` 开头也不会被当作选项。
    if let Some(key) = identity(target) {
        command.arg("-i").arg(key);
    }
    // 指定了端口时由 `-p` 给出；没有指定时不加，由用户的配置决定。
    if let Some(port) = target.port() {
        command.arg("-p").arg(port.to_string());
    }
    command.arg(target.host()).arg("--");
    command
}

/// 把 `text` 写成远端 shell 中的一个词：包在单引号内，其中的单引号写成 `'\''`。
fn quote(text: &str) -> OsString {
    format!("'{}'", text.replace('\'', r"'\''")).into()
}
