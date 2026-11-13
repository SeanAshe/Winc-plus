//! 打开文件用于读取：文件信息、开头的一段和内容。

#[cfg(test)]
mod tests;

use std::io::{self, Read, Seek};
use std::path::Path;

use super::Metadata;
use crate::remote;
use crate::{Target, locate};

/// 打开的文件：文件信息、开头的一段，以及读取全部内容或换成 [`Handle`] 的入口。
///
/// 本机的文件三者取自同一次打开，其间文件被其他程序改名替换时读到的仍是打开时的那个文件。
/// 远端的文件按路径逐次读取，没有这一保证。
pub struct Opened {
    /// 打开时的文件信息。
    pub metadata: Metadata,
    /// 文件开头的若干字节，长度不超过 [`open`] 的 `sniff`；文件更短时是全部内容。
    pub head: Vec<u8>,
    source: Source,
    /// 远端文件已随打开一并取回的开头部分，以及它是否已是全部内容；本机的文件没有。
    ///
    /// 顺序读取时先用这一部分：不超过它的文件，打开和读取合计只有一次往返。
    prefix: (Vec<u8>, bool),
}

/// 打开远端文件时一并取回的字节数：一次读取的上限减去 4 KiB，为同一个应答中的文件信息和
/// 各字段的长度留出位置。
const PREFIX: u32 = remote::MAX_READ - (4 << 10);

/// 文件内容的来源。
#[derive(Debug)]
enum Source {
    /// 本机上打开的文件。
    Local(std::fs::File),
    /// 远端主机上的文件：连接目标和远端的路径。每次读取时按目标取用连接，重新连接之后
    /// 仍然可用。
    Remote(Target, Vec<u8>),
}

impl Opened {
    /// 返回从文件开头顺序读取内容的读取器；每次调用都从开头开始。
    ///
    /// # Errors
    ///
    /// 无法回到文件开头时返回对应的 `io::Error`。
    pub fn reader(&mut self) -> io::Result<Box<dyn Read + '_>> {
        match &mut self.source {
            Source::Local(file) => {
                file.rewind()?;
                Ok(Box::new(file))
            }
            Source::Remote(target, path) => Ok(Box::new(Sequential {
                target,
                path,
                prefix: &self.prefix.0,
                offset: 0,
                // 文件是空的、或开头的部分已是全部内容时，读完它就结束。
                ended: self.prefix.1 && self.prefix.0.is_empty(),
                complete: self.prefix.1,
            })),
        }
    }

    /// 换成可按偏移读取的句柄。
    pub fn into_handle(self) -> Handle {
        Handle(self.source)
    }
}

/// 从头顺序读取远端文件，每次请求一段。
struct Sequential<'a> {
    target: &'a Target,
    path: &'a [u8],
    /// 打开时已取回的开头部分，以及它是否已是全部内容。
    prefix: &'a [u8],
    complete: bool,
    offset: u64,
    ended: bool,
}

impl Read for Sequential<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ended || buf.is_empty() {
            return Ok(0);
        }
        // 已取回的开头部分直接给出，不发请求。
        let taken = usize::try_from(self.offset).unwrap_or(usize::MAX);
        if let Some(rest) = self.prefix.get(taken..).filter(|rest| !rest.is_empty()) {
            let count = rest.len().min(buf.len());
            buf[..count].copy_from_slice(&rest[..count]);
            self.offset += count as u64;
            self.ended = self.complete && count == rest.len();
            return Ok(count);
        }
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let connection = remote::connection(self.target)?;
        let (data, end) = connection.read_range(self.path, self.offset, len)?;
        buf[..data.len()].copy_from_slice(&data);
        self.offset += data.len() as u64;
        self.ended = end;
        Ok(data.len())
    }
}

/// 可按偏移读取的文件句柄。
///
/// 本机的句柄存在期间，文件被改名替换后读到的仍是打开时的内容；文件被原地修改时读到修改后的
/// 内容，用 [`Handle::metadata`] 发现。远端的句柄按路径逐次读取，文件被替换后读到新的内容。
/// 可以在多个线程上同时读取。
#[derive(Debug)]
pub struct Handle(Source);

impl Handle {
    /// 返回句柄指向的文件此刻的信息。本机的句柄不经过路径，文件改名后仍是同一个文件。
    ///
    /// # Errors
    ///
    /// 与 `std::fs::File::metadata` 相同；远端的句柄还有连接的错误。
    pub fn metadata(&self) -> io::Result<Metadata> {
        match &self.0 {
            Source::Local(file) => file.metadata().map(|m| Metadata::from(&m)),
            Source::Remote(target, path) => remote::connection(target)?.metadata(path, true),
        }
    }

    /// 从字节偏移 `offset` 处读取，至多填满 `buf`，返回读到的字节数；到达文件末尾时返回 0。
    ///
    /// # Errors
    ///
    /// 与系统的按偏移读取（Unix 的 `pread`，Windows 的 `seek_read`）相同，包括被信号中断；
    /// 远端的句柄还有连接的错误。
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        match &self.0 {
            Source::Local(file) => {
                #[cfg(unix)]
                return std::os::unix::fs::FileExt::read_at(file, buf, offset);
                #[cfg(windows)]
                return std::os::windows::fs::FileExt::seek_read(file, buf, offset);
            }
            Source::Remote(target, path) => {
                let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
                let (data, _) = remote::connection(target)?.read_range(path, offset, len)?;
                buf[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
        }
    }

    /// 从字节偏移 `offset` 处读满 `buf` 或读到文件末尾，返回读到的字节数。
    ///
    /// # Errors
    ///
    /// 与 [`Self::read_at`] 相同；被信号中断时重试。
    pub fn read_full(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let mut filled = 0;
        while filled < buf.len() {
            match self.read_at(offset + filled as u64, &mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(filled)
    }

    /// 复制句柄，两个句柄指向同一个文件。
    ///
    /// # Errors
    ///
    /// 与 `std::fs::File::try_clone` 相同。
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self(match &self.0 {
            Source::Local(file) => Source::Local(file.try_clone()?),
            Source::Remote(target, path) => Source::Remote(target.clone(), path.clone()),
        }))
    }
}

impl From<std::fs::File> for Handle {
    fn from(file: std::fs::File) -> Self {
        Self(Source::Local(file))
    }
}

/// 打开 `path` 用于读取，并读出开头至多 `sniff` 字节。
///
/// # Errors
///
/// 与 `std::fs::File::open` 和读取相同；`path` 是虚拟路径时还有连接的错误，见
/// [`fs`](crate::fs) 的说明。
pub fn open(path: impl AsRef<Path>, sniff: usize) -> io::Result<Opened> {
    let path = path.as_ref();
    if let Some(at) = locate(path) {
        // 文件信息和开头的一段在一次往返中取回；多数文件不超过这一段，随后的读取不再发请求。
        let connection = remote::connection(&at.target)?;
        let (metadata, data) = connection.open(&at.path, PREFIX)?;
        // 与本机一致：目录不能当作文件打开读取。
        if metadata.is_dir() {
            return Err(io::ErrorKind::IsADirectory.into());
        }
        let (prefix, complete) = data?;
        return Ok(Opened {
            metadata,
            head: prefix[..prefix.len().min(sniff)].to_vec(),
            source: Source::Remote(at.target, at.path),
            prefix: (prefix, complete),
        });
    }
    let mut file = std::fs::File::open(path)?;
    let metadata = Metadata::from(&file.metadata()?);
    let mut head = Vec::with_capacity(sniff);
    (&mut file).take(sniff as u64).read_to_end(&mut head)?;
    Ok(Opened {
        metadata,
        head,
        source: Source::Local(file),
        prefix: (Vec::new(), false),
    })
}
