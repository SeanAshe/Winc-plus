//! 服务进程一侧：从输入读取请求，在本机执行，把结果写到输出。
//!
//! 运行在远端主机上，由 `winc-server` 程序调用。只使用标准输入输出，不监听端口，不写任何文件；
//! 输入结束（连接断开）时结束仍在运行的命令并返回，进程随之退出。请求中的路径是该主机上的
//! 绝对路径。
//!
//! 文件操作在读取请求的线程上逐个执行；运行命令各用一个线程，其间其他请求照常应答。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::wire::{self, Frame, Reader, Writer, kind};

/// 查询运行中的命令是否结束、是否被取消的间隔（毫秒）。
///
/// 取值由两方面约束：命令结束后最多再等这么久才应答，过大时每条命令都变慢；过小时等待期间
/// 被唤醒的次数增多。2 ms 远小于一次网络往返，每条运行中的命令每秒唤醒 500 次。
const POLL: Duration = Duration::from_millis(2);
/// 一段输出的最大长度（字节，64 KiB）：与管道缓冲区的常见大小相同，一次读取即可取完。
const CHUNK: usize = 64 << 10;
/// 连接断开后等待运行中的命令结束的最长时间（秒）。命令已被强制结束，正常情况下立即退出；
/// 超过该时间仍未退出时不再等待，进程照常结束。
const DRAIN: Duration = Duration::from_secs(2);
/// 命令退出后等待其输出读完的时间（毫秒），与本机运行命令时相同。
///
/// 输出的管道可能仍被命令遗留在后台的子进程占用：超过这段时间仍未关闭就结束进程组，再等同样
/// 长的时间，之后不再等待读取输出的线程。
const GRACE: Duration = Duration::from_millis(200);

/// 写应答的一端，由读取请求的线程和各个运行命令的线程共用。
type Output = Arc<Replies>;

/// 写应答的一端，以及正在等着写的线程数。
struct Replies {
    writer: Mutex<Box<dyn Write + Send>>,
    waiting: AtomicUsize,
}

impl Replies {
    /// 取得写的一端。
    fn lock(&self) -> MutexGuard<'_, Box<dyn Write + Send>> {
        self.waiting.fetch_add(1, Ordering::Relaxed);
        let writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.waiting.fetch_sub(1, Ordering::Relaxed);
        writer
    }

    /// 等别的线程都写完之后再取得写的一端：用于成批的数据，让它排在其他应答之后。
    ///
    /// 锁本身不保证先来后到：刚放开锁的线程可以立刻再次取得，等着的线程一直轮不到。
    fn lock_last(&self) -> MutexGuard<'_, Box<dyn Write + Send>> {
        while self.waiting.load(Ordering::Relaxed) > 0 {
            thread::yield_now();
        }
        self.lock()
    }
}
/// 运行中的命令：请求号对应其状态。
type Running = Arc<Mutex<HashMap<u32, Arc<Job>>>>;

/// 一条运行中的命令。
#[derive(Default)]
struct Job {
    /// 是否已要求强制结束。
    cancelled: AtomicBool,
    /// 命令提出、尚未得到答复的输入请求：编号对应等待答复的一方。
    asks: Mutex<HashMap<u32, mpsc::Sender<Option<Vec<u8>>>>>,
}

/// 逐个处理 `input` 中的请求并把应答写到 `output`，直到输入结束。
///
/// 单个请求失败时应答错误并继续；请求不合协议（帧长度越界、帧不完整）时返回错误，调用方应
/// 结束进程。返回之前强制结束仍在运行的命令。
///
/// # Errors
///
/// 读取、写入失败，或收到不合协议的帧时返回对应的 `io::Error`。
pub fn serve(mut input: impl Read, output: impl Write + Send + 'static) -> io::Result<()> {
    let output: Output = Arc::new(Replies {
        writer: Mutex::new(Box::new(output)),
        waiting: AtomicUsize::new(0),
    });
    let running = Running::default();
    let mut uploads: HashMap<u32, Upload> = HashMap::new();
    // 载荷超过一帧的请求：先到的各段按请求号存放；超过上限的内容丢弃，请求到达时应答错误。
    let mut parts: HashMap<u32, Option<Vec<u8>>> = HashMap::new();
    let result = loop {
        let mut frame = match wire::read_frame(&mut input) {
            Ok(Some(frame)) => frame,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        };
        if frame.kind == kind::PART {
            let body = parts.entry(frame.id).or_insert_with(|| Some(Vec::new()));
            match body {
                Some(sofar) if sofar.len() + frame.payload.len() <= UPLOAD_MAX => {
                    sofar.extend_from_slice(&frame.payload);
                }
                _ => *body = None,
            }
            continue;
        }
        match parts.remove(&frame.id) {
            None => {}
            Some(Some(mut body)) => {
                body.extend_from_slice(&frame.payload);
                frame.payload = body;
            }
            Some(None) => {
                let too_large = Err(io::Error::other("request too large"));
                if let Err(error) = reply(&output, frame.id, too_large) {
                    break Err(error);
                }
                continue;
            }
        }
        let sent = match frame.kind {
            kind::CANCEL => {
                let running = running.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(job) = running.get(&frame.id) {
                    job.cancelled.store(true, Ordering::Relaxed);
                }
                Ok(())
            }
            kind::ANSWER => {
                let job = (running.lock().unwrap_or_else(PoisonError::into_inner))
                    .get(&frame.id)
                    .cloned();
                // 命令已经结束，或答复不合格式：没有等待它的一方，丢弃。
                if let (Some(job), Ok((ticket, reply))) = (job, read_answer(&frame.payload)) {
                    let waiting = (job.asks.lock())
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&ticket);
                    if let Some(waiting) = waiting {
                        let _ = waiting.send(reply);
                    }
                }
                Ok(())
            }
            kind::EXEC => {
                start(frame, &output, &running);
                Ok(())
            }
            kind::READ if reads_apart(&frame) => {
                read_apart(frame, &output);
                Ok(())
            }
            // 保存的内容分多帧到达，先按请求号存起来，收到结束的一帧才写入并应答。
            kind::SAVE_BEGIN => {
                if let Ok(upload) = Upload::begin(&frame.payload) {
                    uploads.insert(frame.id, upload);
                }
                Ok(())
            }
            kind::SAVE_DATA => {
                if let Some(upload) = uploads.get_mut(&frame.id) {
                    upload.push(&frame.payload, frame.id);
                }
                Ok(())
            }
            kind::SAVE_END => {
                let upload = uploads.remove(&frame.id);
                reply(&output, frame.id, finish_upload(upload, &frame.payload))
            }
            // 目录项分批写出，目录再大也不受一帧的限制。
            kind::READ_DIR => match list(&frame, &output) {
                Ok(()) => reply(&output, frame.id, Ok(Vec::new())),
                Err(Failure::Request(error)) => reply(&output, frame.id, Err(error)),
                Err(Failure::Output(error)) => Err(error),
            },
            kind::BATCH => reply(&output, frame.id, batch(&frame.payload)),
            _ => reply(&output, frame.id, handle(&frame)),
        };
        if let Err(error) = sent {
            break Err(error);
        }
    };
    drain(&running);
    result
}

/// 写出请求 `id` 的最终应答。
fn reply(output: &Output, id: u32, result: io::Result<Vec<u8>>) -> io::Result<()> {
    let (kind, payload) = match result {
        Ok(payload) => (kind::OK, payload),
        Err(error) => (kind::ERROR, wire::encode_error(&error)),
    };
    let mut output = output.lock();
    wire::write_frame(&mut *output, kind, id, &payload)?;
    output.flush()
}

/// 在新线程上运行 `frame` 请求的命令，结束时写出应答。
fn start(frame: Frame, output: &Output, running: &Running) {
    let job = Arc::new(Job::default());
    running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(frame.id, Arc::clone(&job));
    let (output, running) = (Arc::clone(output), Arc::clone(running));
    thread::spawn(move || {
        let result = exec(&frame, &job, &output);
        // 还在等答复的输入请求不会再有答复：丢弃发送端，等待的一方按没有答复处理。
        (job.asks.lock())
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&frame.id);
        // 写不出去说明连接已断开，读取请求的线程会随之结束。
        let _ = reply(&output, frame.id, result);
    });
}

/// 强制结束所有运行中的命令，并等待它们的线程退出，最长 [`DRAIN`]。
fn drain(running: &Running) {
    let deadline = Instant::now() + DRAIN;
    loop {
        let left = {
            let running = running.lock().unwrap_or_else(PoisonError::into_inner);
            for job in running.values() {
                job.cancelled.store(true, Ordering::Relaxed);
            }
            running.len()
        };
        if left == 0 || Instant::now() >= deadline {
            return;
        }
        thread::sleep(POLL);
    }
}

/// 执行一个文件操作的请求，返回应答的载荷。
fn handle(frame: &Frame) -> io::Result<Vec<u8>> {
    let mut input = Reader::new(&frame.payload);
    match frame.kind {
        kind::HELLO => {
            let version = input.u32()?;
            input.finish()?;
            if version != wire::VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "protocol version mismatch",
                ));
            }
            Ok(Writer::default().u32(wire::VERSION).finish())
        }
        kind::PING => {
            input.finish()?;
            Ok(Vec::new())
        }
        kind::METADATA | kind::SYMLINK_METADATA => {
            let path = path(&mut input)?;
            input.finish()?;
            let metadata = if frame.kind == kind::METADATA {
                crate::fs::metadata(path)?
            } else {
                crate::fs::symlink_metadata(path)?
            };
            Ok(wire::write_metadata(Writer::default(), &metadata).finish())
        }
        kind::READ => {
            let (data, end) = read(&frame.payload)?;
            Ok(Writer::default().bytes(&data).u8(u8::from(end)).finish())
        }
        kind::READ_LINK => {
            let path = path(&mut input)?;
            input.finish()?;
            let target = crate::fs::read_link(path)?;
            Ok(Writer::default()
                .bytes(target.as_os_str().as_bytes())
                .finish())
        }
        kind::CREATE_DIR => {
            let path = path(&mut input)?;
            let all = input.u8()? != 0;
            input.finish()?;
            if all {
                crate::fs::create_dir_all(path)?;
            } else {
                crate::fs::create_dir(path)?;
            }
            Ok(Vec::new())
        }
        kind::CREATE_NEW | kind::WRITE => {
            let path = path(&mut input)?;
            let contents = input.bytes()?;
            input.finish()?;
            if frame.kind == kind::CREATE_NEW {
                crate::fs::create_new(path, contents)?;
            } else {
                crate::fs::write(path, contents)?;
            }
            Ok(Vec::new())
        }
        kind::RENAME | kind::COPY | kind::COPY_SYMLINK => {
            let (from, to) = (path(&mut input)?, path(&mut input)?);
            input.finish()?;
            match frame.kind {
                kind::RENAME => crate::fs::rename(from, to)?,
                kind::COPY_SYMLINK => crate::fs::copy_symlink(from, to)?,
                _ => return Ok(Writer::default().u64(crate::fs::copy(from, to)?).finish()),
            }
            Ok(Vec::new())
        }
        kind::SET_MODE => {
            let (path, mode) = (path(&mut input)?, input.u32()?);
            input.finish()?;
            crate::fs::set_mode(path, mode)?;
            Ok(Vec::new())
        }
        kind::SYMLINK => {
            let target = input.bytes()?.to_vec();
            let link = path(&mut input)?;
            input.finish()?;
            std::os::unix::fs::symlink(OsStr::from_bytes(&target), link)?;
            Ok(Vec::new())
        }
        kind::REMOVE_FILE => {
            let path = path(&mut input)?;
            input.finish()?;
            crate::fs::remove_file(path)?;
            Ok(Vec::new())
        }
        kind::REMOVE_ALL => {
            let path = path(&mut input)?;
            input.finish()?;
            crate::fs::remove_all(path)?;
            Ok(Vec::new())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unknown request",
        )),
    }
}

/// 执行读取请求 `payload`（路径、偏移、长度），返回读到的数据和是否已到文件末尾。
fn read(payload: &[u8]) -> io::Result<(Vec<u8>, bool)> {
    let mut input = Reader::new(payload);
    let path = path(&mut input)?;
    let (offset, len) = (input.u64()?, input.u32()?);
    input.finish()?;
    if len > wire::MAX_READ {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "read too large",
        ));
    }
    // 只读取普通文件：打开命名管道会一直等到另一端出现，占住读取请求的线程。
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut data = vec![0; len as usize];
    let handle = crate::fs::open(path, 0)?.into_handle();
    let read = handle.read_full(offset, &mut data)?;
    data.truncate(read);
    Ok((data, read < len as usize))
}

/// 读取请求的长度超过多少字节时分段应答（字节，256 KiB，与保存时一段的大小相同）。
///
/// 一帧在写完之前占着应答的一端：整段数据作为一帧时，带宽有限的连接上别的请求要等它写完。
/// 分成几段、在另一个线程上写出，别的应答可以排在两段之间。
const READ_PART: usize = 256 << 10;

/// 是否读取请求 `frame` 的长度超过 [`READ_PART`]；载荷不合格式时为假，由通常的流程应答错误。
fn reads_apart(frame: &Frame) -> bool {
    let mut input = Reader::new(&frame.payload);
    let len = (|| {
        input.bytes()?;
        input.u64()?;
        input.u32()
    })();
    len.is_ok_and(|len| len as usize > READ_PART)
}

/// 在新线程上执行读取请求 `frame`：数据分成不超过 [`READ_PART`] 的几段，前面各段作为
/// [`kind::CHUNK`] 写出，最后一段连同是否已到末尾作为应答。
fn read_apart(frame: Frame, output: &Output) {
    let output = Arc::clone(output);
    thread::spawn(move || {
        let result = read(&frame.payload).and_then(|(data, end)| {
            let mut rest = data.as_slice();
            while rest.len() > READ_PART {
                let (part, later) = rest.split_at(READ_PART);
                let mut writer = output.lock_last();
                wire::write_frame(&mut *writer, kind::CHUNK, frame.id, part)?;
                writer.flush()?;
                drop(writer);
                rest = later;
            }
            Ok(Writer::default().bytes(rest).u8(u8::from(end)).finish())
        });
        let (kind, payload) = match result {
            Ok(payload) => (kind::OK, payload),
            Err(error) => (kind::ERROR, wire::encode_error(&error)),
        };
        // 写不出去说明连接已断开，读取请求的线程会随之结束。
        let mut writer = output.lock_last();
        let _ = wire::write_frame(&mut *writer, kind, frame.id, &payload);
        let _ = writer.flush();
    });
}

/// 一次保存中留在内存里的内容上限（字节，64 MiB），超过后转存到磁盘上。
///
/// 全部在内存中的文档不超过 `LARGE_FILE`（16 MiB），按其他编码写出时可能更大（UTF-16 至多为
/// UTF-8 的两倍）；取其四倍，文档的保存都在内存中收齐。更大的内容（传到这台主机上的文件）
/// 写进目标目录里的临时文件，对方无法让本进程无限占用内存。
const UPLOAD_MAX: usize = 64 << 20;

/// 正在接收的一次保存。
struct Upload {
    path: Vec<u8>,
    bom: bool,
    in_memory: bool,
    content: Vec<u8>,
    /// 超过 [`UPLOAD_MAX`] 之后存放内容的临时文件：已从目录中删除，只由这个句柄引用，关闭后
    /// 占用的空间随即释放。
    spilled: Option<std::fs::File>,
    /// 转存或写入临时文件时的错误：之后的内容不再保留，结束时报告它。
    failed: Option<io::Error>,
}

impl Upload {
    /// 解析 [`kind::SAVE_BEGIN`] 的载荷。
    fn begin(payload: &[u8]) -> io::Result<Self> {
        let mut input = Reader::new(payload);
        let path = input.bytes()?.to_vec();
        let (bom, in_memory) = (input.u8()? != 0, input.u8()? != 0);
        input.finish()?;
        Ok(Self {
            path,
            bom,
            in_memory,
            content: Vec::new(),
            spilled: None,
            failed: None,
        })
    }

    fn push(&mut self, data: &[u8], id: u32) {
        if self.failed.is_some() {
            return;
        }
        if self.spilled.is_none() && self.content.len() + data.len() <= UPLOAD_MAX {
            self.content.extend_from_slice(data);
            return;
        }
        let written = match &mut self.spilled {
            Some(file) => file.write_all(data),
            None => self.spill(id).and_then(|mut file| {
                file.write_all(&std::mem::take(&mut self.content))?;
                file.write_all(data)?;
                self.spilled = Some(file);
                Ok(())
            }),
        };
        if let Err(error) = written {
            (self.content, self.spilled, self.failed) = (Vec::new(), None, Some(error));
        }
    }

    /// 在目标所在的目录里建一个只有本进程能访问的临时文件并立即从目录中删除；`id` 是这次
    /// 保存的请求号，用来区分同时进行的几次保存。与目标在同一个文件系统上：那里本来就要有
    /// 放下这些内容的空间。
    fn spill(&self, id: u32) -> io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt;

        let dir = absolute(&self.path)?.parent().unwrap_or(Path::new("/"));
        let name = format!(".winc-upload-{}-{id}", std::process::id());
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true).mode(0o600);
        let file = options.open(&path)?;
        std::fs::remove_file(&path)?;
        Ok(file)
    }
}

/// 结束一次保存：`payload` 是 [`kind::SAVE_END`] 的载荷；`upload` 为 `None` 说明没有收到合法的
/// 开始一帧。
fn finish_upload(upload: Option<Upload>, payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut input = Reader::new(payload);
    let commit = input.u8()? != 0;
    input.finish()?;
    let upload = upload.ok_or_else(|| wire::invalid("save was not started"))?;
    if !commit {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "save abandoned"));
    }
    let mut upload = upload;
    if let Some(error) = upload.failed.take() {
        return Err(error);
    }
    let options = crate::fs::SaveOptions {
        bom: upload.bom,
        in_memory: upload.in_memory,
        ..Default::default()
    };
    // 符号链接写入其指向的文件，不替换链接本身；文件尚不存在时按原路径新建。
    let path = absolute(&upload.path)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "服务进程只在 Unix 上运行，路径没有需要去掉的前缀"
    )]
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let saved = crate::fs::save(&target, &options, |out| match &mut upload.spilled {
        Some(file) => {
            use std::io::Seek;
            file.rewind()?;
            io::copy(file, out).map(drop)
        }
        None => out.write_all(&upload.content),
    });
    let result = match saved {
        Ok(metadata) => {
            return Ok(wire::write_metadata(Writer::default().u8(kind::SAVED), &metadata).finish());
        }
        Err(crate::fs::SaveError::ReadOnly) => kind::SAVE_READ_ONLY,
        Err(crate::fs::SaveError::IdentityWouldChange) => kind::SAVE_IDENTITY,
        Err(crate::fs::SaveError::TemporaryFile) => kind::SAVE_TEMPORARY,
        Err(crate::fs::SaveError::Io(error)) => return Err(error),
    };
    Ok(Writer::default().u8(result).finish())
}

/// 读出请求中的路径，见 [`absolute`]。
fn path<'a>(input: &mut Reader<'a>) -> io::Result<&'a Path> {
    absolute(input.bytes()?)
}

/// 把请求中的字节当作路径：必须是绝对路径且不含 NUL。
fn absolute(bytes: &[u8]) -> io::Result<&Path> {
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be absolute",
        ));
    }
    Ok(Path::new(OsStr::from_bytes(bytes)))
}

/// 依次执行 [`kind::BATCH`] 中的各个请求，把各自的结果编码进一个应答。
fn batch(payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut input = Reader::new(payload);
    let mut out = Writer::default();
    let mut size = 0;
    for _ in 0..input.u32()? {
        let (kind, payload) = (input.u8()?, input.bytes()?.to_vec());
        let result = match kind {
            // 这些类型不在读取请求的线程上直接应答，或有多帧的应答。
            kind::EXEC
            | kind::CANCEL
            | kind::READ_DIR
            | kind::BATCH
            | kind::PART
            | kind::SAVE_BEGIN
            | kind::SAVE_DATA
            | kind::SAVE_END => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "request cannot be batched",
            )),
            _ if size > wire::MAX_READ as usize => Err(io::Error::other("batch too large")),
            _ => handle(&Frame {
                kind,
                id: 0,
                payload,
            }),
        };
        let (ok, body) = match result {
            Ok(body) => (true, body),
            Err(error) => (false, wire::encode_error(&error)),
        };
        size += body.len();
        out = out.u8(u8::from(ok)).bytes(&body);
    }
    input.finish()?;
    Ok(out.finish())
}

/// 列目录失败的两种情况：请求本身失败（应答错误后继续），或应答写不出去（连接已断开）。
enum Failure {
    Request(io::Error),
    Output(io::Error),
}

/// 列出 `frame` 请求的目录，各项的种类与名字分批写成 [`kind::CHUNK`]，每批不超过 [`CHUNK`]
/// 字节；最终的应答由调用方写出。无法取得种类的项不列出。
fn list(frame: &Frame, output: &Output) -> Result<(), Failure> {
    let entries = (|| {
        let mut input = Reader::new(&frame.payload);
        let path = path(&mut input)?;
        input.finish()?;
        crate::fs::read_dir(path)
    })()
    .map_err(Failure::Request)?;
    let send = |count: u32, items: Writer| {
        let mut payload = Writer::default().u32(count).finish();
        payload.extend_from_slice(&items.finish());
        let mut output = output.lock();
        wire::write_frame(&mut *output, kind::CHUNK, frame.id, &payload).map_err(Failure::Output)
    };
    let (mut count, mut size, mut items) = (0u32, 0usize, Writer::default());
    for entry in entries.flatten() {
        let name = entry.name.as_bytes();
        items = items.u8(wire::kind_byte(entry.kind)).bytes(name);
        count += 1;
        size += name.len() + 5;
        if size >= CHUNK {
            send(count, std::mem::take(&mut items))?;
            (count, size) = (0, 0);
        }
    }
    if count > 0 {
        send(count, items)?;
    }
    Ok(())
}

/// 解析 [`kind::ANSWER`] 的载荷：请求的编号和答复（没有答复时为 `None`）。
fn read_answer(payload: &[u8]) -> io::Result<(u32, Option<Vec<u8>>)> {
    let mut input = Reader::new(payload);
    let (ticket, answered) = (input.u32()?, input.u8()? != 0);
    let reply = input.bytes()?.to_vec();
    input.finish()?;
    Ok((ticket, answered.then_some(reply)))
}

/// 把运行中的命令提出的输入请求转给对端的通道：一个只有用户本人可以访问的套接字，命令的
/// askpass 程序（服务进程自身，见 [`answer_from_server`]）连上它提问。
///
/// 套接字放在一个新建的、权限为 0700 的临时目录里；丢弃时停止接受连接并删除目录。
struct Relay {
    dir: PathBuf,
    done: Arc<AtomicBool>,
}

impl Relay {
    /// 套接字的路径。
    fn socket(&self) -> PathBuf {
        self.dir.join("ask")
    }

    /// 建立通道并开始接受连接；`id` 是命令的请求号。
    fn open(id: u32, job: &Arc<Job>, output: &Output) -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;

        let name = format!("winc-ask-{}-{id}", std::process::id());
        let dir = std::env::temp_dir().join(name);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let relay = Self {
            dir,
            done: Arc::default(),
        };
        let listener = UnixListener::bind(relay.socket())?;
        listener.set_nonblocking(true)?;
        let (done, job, output) = (Arc::clone(&relay.done), Arc::clone(job), Arc::clone(output));
        thread::spawn(move || {
            let mut ticket = 0;
            while !done.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(POLL);
                    continue;
                };
                ticket += 1;
                let (job, output) = (Arc::clone(&job), Arc::clone(&output));
                // 每次提问各用一个线程：等用户答复期间不耽误别的提问。
                thread::spawn(move || {
                    let _ = relay_one(stream, id, ticket, &job, &output);
                });
            }
        });
        Ok(relay)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(self.socket());
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// 处理 askpass 程序的一次提问：读入它发来的提问方式和提示，转给对端，等到答复后写回。
///
/// 发来的内容是提问的方式、一个换行、提示的文字；写回的第一个字节是有没有答复，其后是答复。
fn relay_one(
    mut stream: UnixStream,
    id: u32,
    ticket: u32,
    job: &Job,
    output: &Output,
) -> io::Result<()> {
    // 有的系统上接受的连接沿用监听套接字的非阻塞设置。
    stream.set_nonblocking(false)?;
    let mut request = Vec::new();
    (&mut stream)
        .take(ASK_MAX as u64)
        .read_to_end(&mut request)?;
    let split = request.iter().position(|&b| b == b'\n');
    let (mode, prompt) = request.split_at(split.unwrap_or(0));
    let prompt = prompt.strip_prefix(b"\n").unwrap_or(prompt);
    let (sender, answer) = mpsc::channel();
    (job.asks.lock())
        .unwrap_or_else(PoisonError::into_inner)
        .insert(ticket, sender);
    let payload = Writer::default().u32(ticket).bytes(mode).bytes(prompt);
    write_frame(output, kind::ASK, id, &payload.finish())?;
    // 命令结束、连接断开时发送端被丢弃：按没有答复处理。
    match answer.recv().ok().flatten() {
        Some(reply) => {
            stream.write_all(&[1])?;
            stream.write_all(&reply)
        }
        None => stream.write_all(&[0]),
    }
}

/// 一次提问的内容上限（字节，64 KiB）：提示是一两行文字，远小于此；超出的部分不读。
const ASK_MAX: usize = 64 << 10;

/// 作为命令的 askpass 程序运行时调用：把提示 `prompt` 交给启动该命令的服务进程（经环境变量
/// [`ASKPASS_SOCKET`](super::ASKPASS_SOCKET) 给出的套接字），返回用户的答复；用户没有答复时
/// 返回 `None`。
///
/// # Errors
///
/// 没有该环境变量，或无法连接、读写套接字时返回对应的 `io::Error`。
pub fn answer_from_server(prompt: &[u8]) -> io::Result<Option<Vec<u8>>> {
    let socket = std::env::var_os(super::ASKPASS_SOCKET)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no askpass socket"))?;
    let mut stream = UnixStream::connect(socket)?;
    let mode = std::env::var_os("SSH_ASKPASS_PROMPT").unwrap_or_default();
    stream.write_all(mode.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.write_all(prompt)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply)?;
    Ok(match reply.split_first() {
        Some((1, answer)) => Some(answer.to_vec()),
        _ => None,
    })
}

/// 运行 `frame` 请求的命令：输出分段写到 `output`，结束时返回退出码；`job` 被要求结束时
/// 强制结束命令并返回 [`io::ErrorKind::Interrupted`]。
fn exec(frame: &Frame, job: &Arc<Job>, output: &Output) -> io::Result<Vec<u8>> {
    let cancelled = &job.cancelled;
    let mut input = Reader::new(&frame.payload);
    let text = |bytes: &[u8]| -> io::Result<()> {
        if bytes.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in command",
            ));
        }
        Ok(())
    };
    let program = input.bytes()?;
    text(program)?;
    let mut command = if program == wire::SELF {
        Command::new(std::env::current_exe()?)
    } else {
        Command::new(OsStr::from_bytes(program))
    };
    let cwd = input.bytes()?;
    if !cwd.is_empty() {
        command.current_dir(absolute(cwd)?);
    }
    for _ in 0..input.u32()? {
        let arg = input.bytes()?;
        text(arg)?;
        command.arg(OsStr::from_bytes(arg));
    }
    for _ in 0..input.u32()? {
        let (name, value) = (input.bytes()?, input.bytes()?);
        text(name)?;
        text(value)?;
        if name.is_empty() || name.contains(&b'=') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid variable name",
            ));
        }
        command.env(OsStr::from_bytes(name), OsStr::from_bytes(value));
    }
    let stdin = input.bytes()?.to_vec();
    let ask = input.u8()? != 0;
    input.finish()?;
    // 命令需要用户输入时运行服务进程自身来提问（`git` 和它启动的 `ssh` 各认一个变量）；
    // `SSH_ASKPASS_REQUIRE` 使 `ssh` 不依赖 `DISPLAY` 和控制终端。通道随命令结束而关闭。
    // 通道建不起来（临时目录不可写）时命令照常运行，只是需要输入时会失败。
    let relay = ask
        .then(|| Relay::open(frame.id, job, output).ok())
        .flatten();
    if let Some(relay) = &relay {
        let this = std::env::current_exe()?;
        command
            .env("GIT_ASKPASS", &this)
            .env("SSH_ASKPASS", &this)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(super::ASKPASS_SOCKET, relay.socket());
    }

    // 命令自成一个进程组：取消时连同它启动的子进程一起结束，否则子进程继续占着输出的管道。
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    // 输入在单独的线程上写入并关闭：命令在读完输入之前就写满输出时，两边不会互相等待。
    let writer = child.stdin.take().map(|mut pipe| {
        thread::spawn(move || {
            let _ = pipe.write_all(&stdin);
        })
    });
    let pumps = [
        child
            .stdout
            .take()
            .map(|pipe| pump(pipe, kind::STDOUT, frame.id, output)),
        child
            .stderr
            .take()
            .map(|pipe| pump(pipe, kind::STDERR, frame.id, output)),
    ];
    let status = wait(&mut child, cancelled);
    // 命令已退出，输出的管道可能仍被它遗留在后台的子进程占用：等 `GRACE`，仍未关闭就结束
    // 进程组，再等 `GRACE`。此后不再等待：脱离了进程组的子进程不能让这个请求一直不结束。
    let group = i32::try_from(child.id()).ok();
    let finished = |threads: &[Option<thread::JoinHandle<()>>]| {
        threads
            .iter()
            .flatten()
            .all(thread::JoinHandle::is_finished)
    };
    let mut killed = false;
    let mut deadline = Instant::now() + GRACE;
    while !finished(&pumps) {
        if Instant::now() >= deadline || cancelled.load(Ordering::Relaxed) {
            if killed {
                break;
            }
            killed = true;
            if let Some(group) = group {
                kill_group(group);
            }
            deadline = Instant::now() + GRACE;
        }
        thread::sleep(POLL);
    }
    // 已结束的线程立即返回；仍在读取的线程和写入输入的线程不再等待，随管道关闭自行结束。
    for thread in pumps.into_iter().flatten().chain(writer) {
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
    match status? {
        None => Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
        Some(status) => Ok(Writer::default()
            .u8(u8::from(status.code().is_some()))
            .u32(status.code().unwrap_or(0).cast_unsigned())
            .finish()),
    }
}

/// 强制结束进程组 `group`（组长的进程号）。组内已没有进程时没有效果。
// 本函数只有对 `libc::kill` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn kill_group(group: i32) {
    // SAFETY: 负的进程号表示进程组。`kill` 只发送信号，不访问内存，失败时返回 -1。组长已被
    // 回收后，其进程号在组内仍有进程时不会被重新分配，信号不会发给无关的进程。
    unsafe { libc::kill(-group, libc::SIGKILL) };
}

/// 等待 `child` 结束并返回退出状态；`cancelled` 被设置时强制结束其进程组并返回 `None`。
fn wait(child: &mut Child, cancelled: &AtomicBool) -> io::Result<Option<std::process::ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if cancelled.load(Ordering::Relaxed) {
            if let Ok(group) = i32::try_from(child.id()) {
                kill_group(group);
            }
            child.wait()?;
            return Ok(None);
        }
        thread::sleep(POLL);
    }
}

/// 向对端写出一帧并刷新。
fn write_frame(output: &Output, kind: u8, id: u32, payload: &[u8]) -> io::Result<()> {
    let mut output = output.lock();
    wire::write_frame(&mut *output, kind, id, payload)?;
    output.flush()
}

/// 在新线程上把 `pipe` 的内容分段写成来源为 `stream` 的 [`kind::CHUNK`]，直到管道关闭。
fn pump(
    mut pipe: impl Read + Send + 'static,
    stream: u8,
    id: u32,
    output: &Output,
) -> thread::JoinHandle<()> {
    let output = Arc::clone(output);
    thread::spawn(move || {
        let mut buffer = vec![0; CHUNK];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) => return,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            let payload = Writer::default().u8(stream).bytes(&buffer[..read]).finish();
            let mut output = output.lock();
            let sent = wire::write_frame(&mut *output, kind::CHUNK, id, &payload);
            // 连接已断开：不再读取，命令随后被强制结束。
            if sent.and_then(|()| output.flush()).is_err() {
                return;
            }
        }
    })
}
