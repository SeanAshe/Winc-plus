//! 在远端主机上搜索和列出文件：本机把条件交给远端的服务进程，由它遍历目录、读文件，只把结果
//! 传回来。
//!
//! 服务进程以 [`SEARCH`]、[`FILES`] 或 [`DIRS`] 为参数再次运行自身（见 `search`、`files`、
//! `dirs`，只在 Unix 主机上有）：条件从标准输入读入，结果逐条写到标准输出，本机随到随读。结果
//! 按需发送，本机读得慢时远端随之放慢；本机放弃读取时远端的进程被结束，不会留下还在遍历的进程。
//!
//! 条件和结果是紧凑的二进制：整数是小端的 `u32`、`u64`，字节串前面是 `u32` 的长度；每条结果前面
//! 是 `u32` 的长度。路径是远端主机上的绝对路径（原始字节）。

use std::io::{self, Read, Write};
use std::ops::Range;

use crate::lines::LineMatch;
use crate::query::Query;
use crate::walk::Probe;

/// 服务进程搜索时的参数：`winc-server search`。
pub const SEARCH: &str = "search";
/// 服务进程列出文件时的参数：`winc-server files`。
pub const FILES: &str = "files";
/// 服务进程找出递归展开的目录时的参数：`winc-server dirs`。
pub const DIRS: &str = "dirs";

/// 服务进程自身的一条命令：读入标准输入中的条件，把结果写到第二个参数。
pub type Command = fn(&[u8], &mut (dyn Write + Send)) -> io::Result<()>;

/// 服务进程以自身运行的各条命令：参数和实现。
#[cfg(unix)]
pub const COMMANDS: [(&str, Command); 3] = [(SEARCH, search), (FILES, files), (DIRS, dirs)];
/// 一条结果的长度上限（字节，256 MiB）：读到更长的长度说明数据已损坏，不再读下去。
///
/// 单个文件的结果最多 `MAX_RESULTS` 处，每处的预览有长度上限，远小于此。
const RECORD_MAX: usize = 256 << 20;

/// 搜索哪些文件，对应 [`Scope`](crate::Scope)；目录和文件在 [`Request::paths`] 中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 遍历目录下的全部文件；`ignored` 为是否也搜索被忽略的文件。
    Tree {
        /// 是否也搜索被 `.gitignore` 等规则忽略的文件。
        ignored: bool,
    },
    /// 只搜索目录下的依赖目录。
    Dependencies,
    /// 只搜索给出的文件。
    Files,
}

/// 一次远端搜索的条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// 查询文本、匹配选项和「包含」「排除」。
    pub query: Query,
    /// 本机的地区（见 [`crate::text::region`]），远端按它猜测编码，两边解码的结果相同。
    pub region: Option<String>,
    /// 最多收多少处匹配。
    pub limit: usize,
    /// 搜索哪些文件。
    pub mode: Mode,
    /// 「包含」「排除」的模式相对的目录。
    pub root: Vec<u8>,
    /// 要遍历的目录，或要搜索的文件（[`Mode::Files`]）。
    pub paths: Vec<Vec<u8>>,
    /// 本机有未保存修改的文件：远端不读它们，遍历到时报告 [`Record::Skipped`]，由本机搜索内存中
    /// 的内容。
    pub skip: Vec<Vec<u8>>,
}

/// 远端写回的一条结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// 一个文件的匹配，按位置排序；`lossy` 表示有字节按 U+FFFD 替换字符解码。
    File {
        /// 文件在远端主机上的路径。
        path: Vec<u8>,
        /// 文件中的匹配。
        matches: Vec<LineMatch>,
        /// 是否有无法解码的字节。
        lossy: bool,
    },
    /// 遍历到了 [`Request::skip`] 中的第几个文件。
    Skipped(usize),
    /// 搜索结束；`truncated` 表示达到了上限，`gave_up` 表示有行因正则回溯过多而放弃。
    End {
        /// 是否因达到上限而停止。
        truncated: bool,
        /// 是否有行因正则回溯过多而放弃查找。
        gave_up: bool,
    },
}

impl Request {
    /// 编码为写进标准输入的字节。
    pub fn encode(&self) -> Vec<u8> {
        let q = &self.query;
        let mut out = Encoder::default();
        out.text(&q.text)
            .flag(q.case_sensitive)
            .flag(q.whole_word)
            .flag(q.regex)
            .text(&q.include)
            .flag(q.include_ignored)
            .text(&q.exclude)
            .text(&q.required)
            .text(self.region.as_deref().unwrap_or_default())
            .u64(self.limit as u64);
        match self.mode {
            Mode::Tree { ignored } => out.u8(0).flag(ignored),
            Mode::Dependencies => out.u8(1),
            Mode::Files => out.u8(2),
        };
        out.bytes(&self.root).list(&self.paths).list(&self.skip);
        out.0
    }

    /// 从 [`Request::encode`] 写出的字节还原。
    ///
    /// # Errors
    ///
    /// 字节不合格式时返回 [`io::ErrorKind::InvalidData`]。
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut input = Decoder(bytes);
        let query = Query {
            text: input.text()?,
            case_sensitive: input.flag()?,
            whole_word: input.flag()?,
            regex: input.flag()?,
            include: input.text()?,
            include_ignored: input.flag()?,
            exclude: input.text()?,
            required: input.text()?,
        };
        let region = Some(input.text()?).filter(|region| !region.is_empty());
        let limit = usize::try_from(input.u64()?).map_err(|_| invalid())?;
        let mode = match input.u8()? {
            0 => Mode::Tree {
                ignored: input.flag()?,
            },
            1 => Mode::Dependencies,
            2 => Mode::Files,
            _ => return Err(invalid()),
        };
        let request = Self {
            query,
            region,
            limit,
            mode,
            root: input.bytes()?.to_vec(),
            paths: input.list()?,
            skip: input.list()?,
        };
        input.finish()?;
        Ok(request)
    }
}

/// 写出一条结果。
///
/// # Errors
///
/// 写入失败时返回对应的 `io::Error`。
pub fn write_record(output: &mut impl Write, record: &Record) -> io::Result<()> {
    let mut out = Encoder::default();
    match record {
        Record::File {
            path,
            matches,
            lossy,
        } => {
            out.u8(0).bytes(path).flag(*lossy).u64(matches.len() as u64);
            for m in matches {
                out.u64(m.line as u64)
                    .range(&m.cols)
                    .text(&m.preview)
                    .range(&m.highlight);
            }
        }
        Record::Skipped(index) => {
            out.u8(1).u64(*index as u64);
        }
        Record::End { truncated, gave_up } => {
            out.u8(2).flag(*truncated).flag(*gave_up);
        }
    }
    frame(output, &out.0)
}

/// 写出一条结果：前面是它的长度。
fn frame(output: &mut (impl Write + ?Sized), body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| invalid())?;
    output.write_all(&len.to_le_bytes())?;
    output.write_all(body)
}

/// 读出一条结果的内容；数据在两条结果之间结束时返回 `None`。
fn unframe(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > RECORD_MAX {
        return Err(invalid());
    }
    let mut body = vec![0; len];
    input
        .read_exact(&mut body)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => invalid(),
            _ => error,
        })?;
    Ok(Some(body))
}

/// 读出下一条结果；数据在两条结果之间结束时返回 `None`。
///
/// # Errors
///
/// 读取失败时返回对应的 `io::Error`；数据不合格式或在一条结果中间结束时返回
/// [`io::ErrorKind::InvalidData`]。
pub fn read_record(input: &mut impl Read) -> io::Result<Option<Record>> {
    let Some(body) = unframe(input)? else {
        return Ok(None);
    };
    let mut input = Decoder(&body);
    let record = match input.u8()? {
        0 => {
            let path = input.bytes()?.to_vec();
            let lossy = input.flag()?;
            let count = input.u64()?;
            // 每处至少占 44 字节，数量不会超过剩下的字节数。
            let mut matches = Vec::with_capacity((count as usize).min(input.0.len() / 44));
            for _ in 0..count {
                matches.push(LineMatch {
                    line: input.usize()?,
                    cols: input.range()?,
                    preview: input.text()?,
                    highlight: input.range()?,
                });
            }
            Record::File {
                path,
                matches,
                lossy,
            }
        }
        1 => Record::Skipped(input.usize()?),
        2 => Record::End {
            truncated: input.flag()?,
            gave_up: input.flag()?,
        },
        _ => return Err(invalid()),
    };
    input.finish()?;
    Ok(Some(record))
}

/// 服务进程的 [`SEARCH`]：按标准输入 `input` 中的条件搜索，结果写到 `output`。
///
/// # Errors
///
/// 条件不合格式时返回 [`io::ErrorKind::InvalidData`]，其余同 [`serve`]。
#[cfg(unix)]
pub fn search(input: &[u8], output: &mut (dyn Write + Send)) -> io::Result<()> {
    serve(&Request::decode(input)?, output)
}

/// 列出文件的条件：根目录下变化了的各个路径，见 [`crate::probe`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// 根目录在远端主机上的路径。
    pub root: Vec<u8>,
    /// 相对于根目录的路径（以 `/` 分隔，按 [`std::path::Path`] 的顺序排好）；空串是根目录本身。
    pub paths: Vec<String>,
    /// 收录的层数；为 `None` 时不限。
    pub layers: Option<usize>,
}

impl Listing {
    /// 编码为写进标准输入的字节。
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Encoder::default();
        out.bytes(&self.root)
            .flag(self.layers.is_some())
            .u64(self.layers.unwrap_or(0) as u64)
            .texts(&self.paths);
        out.0
    }

    /// 从 [`Listing::encode`] 写出的字节还原。
    ///
    /// # Errors
    ///
    /// 字节不合格式时返回 [`io::ErrorKind::InvalidData`]。
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut input = Decoder(bytes);
        let root = input.bytes()?.to_vec();
        let limited = input.flag()?;
        let layers = input.usize()?;
        let paths = input.texts()?;
        input.finish()?;
        Ok(Self {
            root,
            paths,
            layers: limited.then_some(layers),
        })
    }
}

/// 服务进程的 [`FILES`]：按标准输入 `input` 中的 [`Listing`] 查看各个路径，每个路径一条结果，按
/// 顺序写到 `output`（见 [`read_probe`]）。
///
/// # Errors
///
/// 条件不合格式时返回 [`io::ErrorKind::InvalidData`]；写入失败时返回对应的 `io::Error`。
#[cfg(unix)]
pub fn files(input: &[u8], output: &mut (dyn Write + Send)) -> io::Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::AtomicBool;

    let listing = Listing::decode(input)?;
    let root = std::path::Path::new(OsStr::from_bytes(&listing.root));
    let found = crate::walk::probe(
        root,
        &listing.paths,
        listing.layers,
        &AtomicBool::new(false),
    );
    let mut output = io::BufWriter::new(output);
    for probe in &found {
        let mut out = Encoder::default();
        match probe {
            Probe::Missing => out.u8(0),
            Probe::Unlisted => out.u8(1),
            Probe::File => out.u8(2),
            Probe::Covered => out.u8(3),
            Probe::Dir { files, dirs } => out.u8(4).texts(files).texts(dirs),
        };
        frame(&mut output, &out.0)?;
    }
    output.flush()
}

/// 读出服务进程的 [`FILES`] 写出的下一条结果；数据在两条结果之间结束时返回 `None`。
///
/// # Errors
///
/// 读取失败时返回对应的 `io::Error`；数据不合格式或在一条结果中间结束时返回
/// [`io::ErrorKind::InvalidData`]。
pub fn read_probe(input: &mut impl Read) -> io::Result<Option<Probe>> {
    let Some(body) = unframe(input)? else {
        return Ok(None);
    };
    let mut input = Decoder(&body);
    let probe = match input.u8()? {
        0 => Probe::Missing,
        1 => Probe::Unlisted,
        2 => Probe::File,
        3 => Probe::Covered,
        4 => Probe::Dir {
            files: input.texts()?,
            dirs: input.texts()?,
        },
        _ => return Err(invalid()),
    };
    input.finish()?;
    Ok(Some(probe))
}

/// 按 `request` 在本机（远端主机）搜索，结果逐条写到 `output`，最后写出 [`Record::End`]。
///
/// 写不出去（本机已放弃读取）时停止搜索并返回错误。
///
/// # Errors
///
/// 查询或「包含」「排除」有误（本机发出之前已经检查过）、写入失败时返回对应的 `io::Error`。
#[cfg(unix)]
pub fn serve(request: &Request, output: impl Write + Send) -> io::Result<()> {
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::ops::ControlFlow;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, PoisonError};

    use crate::engine::{Budget, Scope, search_parallel};
    use crate::matcher::Matcher;
    use crate::query::Filter;

    let path = |bytes: &[u8]| PathBuf::from(OsStr::from_bytes(bytes));
    let paths = || request.paths.iter().map(|p| path(p)).collect();
    if let Some(region) = &request.region {
        crate::text::set_region(region);
    }
    let query = &request.query;
    let matcher = Matcher::requiring(&query.text, query.options(), &query.required)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    let scope = match request.mode {
        Mode::Tree { ignored } => Scope::Tree {
            dirs: paths(),
            filter: Filter::new(&path(&request.root), &query.include, &query.exclude)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
            ignored,
        },
        Mode::Dependencies => Scope::Dependencies(paths()),
        Mode::Files => Scope::Files(paths()),
    };
    let skip: HashMap<PathBuf, usize> = (request.skip.iter().enumerate())
        .map(|(index, p)| (path(p), index))
        .collect();

    let output = Mutex::new(io::BufWriter::new(output));
    let failed = Mutex::new(None);
    let send = |record: &Record| {
        let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
        // 每条立即送出：本机随到随显示。
        let sent = write_record(&mut *output, record).and_then(|()| output.flush());
        sent.map_err(|error| {
            *failed.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
        })
    };
    let budget = Budget::new(request.limit);
    search_parallel(scope.paths(), &matcher, &budget, |searcher, file: &Path| {
        if let Some(&index) = skip.get(file) {
            return match send(&Record::Skipped(index)) {
                Ok(()) => ControlFlow::Continue(()),
                Err(()) => ControlFlow::Break(()),
            };
        }
        let Some((mut matches, lossy)) = searcher.file(file) else {
            return ControlFlow::Continue(());
        };
        let flow = budget.take(&mut matches);
        if !matches.is_empty() {
            let record = Record::File {
                path: file.as_os_str().as_bytes().to_vec(),
                matches,
                lossy,
            };
            if send(&record).is_err() {
                return ControlFlow::Break(());
            }
        }
        flow
    });
    if let Some(error) = (failed.lock().unwrap_or_else(PoisonError::into_inner)).take() {
        return Err(error);
    }
    let end = Record::End {
        truncated: budget.truncated(),
        gave_up: matcher.gave_up(),
    };
    send(&end).map_err(|()| io::Error::from(io::ErrorKind::BrokenPipe))
}

/// 递归展开一个目录的条件，见 [`crate::descend`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descent {
    /// 目录在远端主机上的路径。
    pub path: Vec<u8>,
    /// 最多找的目录数。
    pub dirs: usize,
    /// 最多看的目录项数。
    pub entries: usize,
}

impl Descent {
    /// 编码为写进标准输入的字节。
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Encoder::default();
        out.bytes(&self.path)
            .u64(self.dirs as u64)
            .u64(self.entries as u64);
        out.0
    }

    /// 从 [`Descent::encode`] 写出的字节还原。
    ///
    /// # Errors
    ///
    /// 字节不合格式时返回 [`io::ErrorKind::InvalidData`]。
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut input = Decoder(bytes);
        let descent = Self {
            path: input.bytes()?.to_vec(),
            dirs: input.usize()?,
            entries: input.usize()?,
        };
        input.finish()?;
        Ok(descent)
    }
}

/// 服务进程的 [`DIRS`]：按标准输入 `input` 中的 [`Descent`] 找出要展开的各级目录，作为一条结果
/// 写到 `output`（见 [`read_dirs`]）。
///
/// # Errors
///
/// 条件不合格式时返回 [`io::ErrorKind::InvalidData`]；写入失败时返回对应的 `io::Error`。
#[cfg(unix)]
pub fn dirs(input: &[u8], output: &mut (dyn Write + Send)) -> io::Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let descent = Descent::decode(input)?;
    let path = std::path::Path::new(OsStr::from_bytes(&descent.path));
    let (found, limited) = crate::walk::descend(path, descent.dirs, descent.entries);
    let found: Vec<Vec<u8>> = (found.iter())
        .map(|dir| dir.as_os_str().as_bytes().to_vec())
        .collect();
    let mut out = Encoder::default();
    out.flag(limited).list(&found);
    frame(output, &out.0)?;
    output.flush()
}

/// 读出服务进程的 [`DIRS`] 写出的结果：各级目录在远端主机上的路径，以及是否只找了一部分。
///
/// # Errors
///
/// 读取失败时返回对应的 `io::Error`；数据不合格式或不完整时返回 [`io::ErrorKind::InvalidData`]。
pub fn read_dirs(input: &mut impl Read) -> io::Result<(Vec<Vec<u8>>, bool)> {
    let body = unframe(input)?.ok_or_else(invalid)?;
    let mut input = Decoder(&body);
    let limited = input.flag()?;
    let found = input.list()?;
    input.finish()?;
    Ok((found, limited))
}

/// 数据不合格式。
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "malformed search data")
}

/// 编码条件和结果。
#[derive(Default)]
struct Encoder(Vec<u8>);

impl Encoder {
    fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }

    fn flag(&mut self, value: bool) -> &mut Self {
        self.u8(u8::from(value))
    }

    fn u64(&mut self, value: u64) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn bytes(&mut self, value: &[u8]) -> &mut Self {
        // 一次搜索的条件和单条结果都远小于 4 GiB。
        self.0
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.0.extend_from_slice(value);
        self
    }

    fn text(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }

    fn range(&mut self, value: &Range<usize>) -> &mut Self {
        self.u64(value.start as u64).u64(value.end as u64)
    }

    fn texts(&mut self, values: &[String]) -> &mut Self {
        self.u64(values.len() as u64);
        for value in values {
            self.text(value);
        }
        self
    }

    fn list(&mut self, values: &[Vec<u8>]) -> &mut Self {
        self.u64(values.len() as u64);
        for value in values {
            self.bytes(value);
        }
        self
    }
}

/// 解码条件和结果：每读一项，从前面去掉相应的字节。
struct Decoder<'a>(&'a [u8]);

impl<'a> Decoder<'a> {
    fn take(&mut self, len: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < len {
            return Err(invalid());
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn flag(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }

    fn u64(&mut self) -> io::Result<u64> {
        let bytes = self.take(8)?.try_into().map_err(|_| invalid())?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn usize(&mut self) -> io::Result<usize> {
        usize::try_from(self.u64()?).map_err(|_| invalid())
    }

    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = self.take(4)?.try_into().map_err(|_| invalid())?;
        self.take(u32::from_le_bytes(len) as usize)
    }

    fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| invalid())
    }

    fn range(&mut self) -> io::Result<Range<usize>> {
        Ok(self.usize()?..self.usize()?)
    }

    fn texts(&mut self) -> io::Result<Vec<String>> {
        let count = self.u64()?;
        // 每项至少占 4 字节，数量不会超过剩下的字节数。
        let mut values = Vec::with_capacity((count as usize).min(self.0.len() / 4));
        for _ in 0..count {
            values.push(self.text()?);
        }
        Ok(values)
    }

    fn list(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let count = self.u64()?;
        // 每项至少占 4 字节，数量不会超过剩下的字节数。
        let mut values = Vec::with_capacity((count as usize).min(self.0.len() / 4));
        for _ in 0..count {
            values.push(self.bytes()?.to_vec());
        }
        Ok(values)
    }

    fn finish(&self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}

#[cfg(test)]
mod tests;
