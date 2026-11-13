//! 文件操作：函数名和语义与 `std::fs` 中的同名函数相同，路径可以是本机路径或虚拟路径。
//!
//! 虚拟路径经到该主机的连接访问（见 [`remote`]），此时还可能返回连接的错误：
//! 没有连接时为 [`io::ErrorKind::NotConnected`]，当前线程不允许等待时为
//! [`io::ErrorKind::WouldBlock`]，连接中断时为 [`io::ErrorKind::BrokenPipe`]。文件信息用本 crate
//! 的 [`Metadata`] 表示：
//! `std::fs::Metadata` 无法由远端的数据构造。

mod change;
mod dir;
mod open;
mod save;
#[cfg(test)]
mod tests;

pub use change::{
    copy, copy_symlink, copy_watched, create_dir, create_dir_all, create_new, remove_all,
    remove_file, rename, set_mode, write,
};
pub use dir::{Entry, ReadDir, read_dir};
pub use open::{Handle, Opened, open};
pub use save::{BeforeReplace, SaveError, SaveOptions, save};

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{locate, remote};

/// 文件的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 普通文件。
    File,
    /// 目录。
    Dir,
    /// 符号链接；只出现在 [`symlink_metadata`] 的结果中。
    Symlink,
    /// 其他：设备、管道、套接字等。
    Other,
}

impl Kind {
    /// 是否为普通文件。
    pub fn is_file(self) -> bool {
        self == Self::File
    }

    /// 是否为目录。
    pub fn is_dir(self) -> bool {
        self == Self::Dir
    }

    /// 是否为符号链接。
    pub fn is_symlink(self) -> bool {
        self == Self::Symlink
    }
}

impl From<std::fs::FileType> for Kind {
    fn from(file_type: std::fs::FileType) -> Self {
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_dir() {
            Self::Dir
        } else if file_type.is_file() {
            Self::File
        } else {
            Self::Other
        }
    }
}

/// 修改时间：自 Unix 纪元起的秒数和秒内的纳秒数（0 到 999 999 999），纪元之前秒数为负。
///
/// 只用于判断文件是否发生变化，按原样保存和比较，不换算为本机的时间类型：各系统时间类型的
/// 精度不同，换算会丢失远端文件系统的纳秒部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp {
    /// 自 Unix 纪元起的秒数。
    pub secs: i64,
    /// 秒内的纳秒数。
    pub nanos: u32,
}

impl From<SystemTime> for Stamp {
    fn from(time: SystemTime) -> Self {
        match time.duration_since(UNIX_EPOCH) {
            Ok(after) => Self {
                secs: after.as_secs().try_into().unwrap_or(i64::MAX),
                nanos: after.subsec_nanos(),
            },
            // 纪元之前：秒数向下取整，纳秒部分保持非负。
            Err(error) => {
                let before = error.duration();
                let secs: i64 = before.as_secs().try_into().unwrap_or(i64::MAX);
                if before.subsec_nanos() == 0 {
                    Self {
                        secs: -secs,
                        nanos: 0,
                    }
                } else {
                    Self {
                        secs: -secs - 1,
                        nanos: 1_000_000_000 - before.subsec_nanos(),
                    }
                }
            }
        }
    }
}

/// Unix 系统上的文件属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unix {
    /// 文件类型与权限位（`st_mode`）。
    pub mode: u32,
    /// 属主的用户编号。
    pub uid: u32,
    /// 硬链接数。
    pub nlink: u64,
    /// 所在设备的编号；与 `ino` 一起标识同一个文件。
    pub dev: u64,
    /// inode 编号。
    pub ino: u64,
}

/// 文件信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// 种类。
    pub kind: Kind,
    /// 大小（字节）。
    pub len: u64,
    /// 修改时间；文件系统不提供时为 `None`。
    pub modified: Option<Stamp>,
    /// 是否只读：Unix 上所有写权限位均未设置，Windows 上设置了只读属性。
    pub readonly: bool,
    /// Unix 系统上的属性；文件位于 Windows 时为 `None`。
    pub unix: Option<Unix>,
}

impl Metadata {
    /// 是否为普通文件。
    pub fn is_file(&self) -> bool {
        self.kind.is_file()
    }

    /// 是否为目录。
    pub fn is_dir(&self) -> bool {
        self.kind.is_dir()
    }

    /// 是否为符号链接。
    pub fn is_symlink(&self) -> bool {
        self.kind.is_symlink()
    }
}

impl From<&std::fs::Metadata> for Metadata {
    fn from(metadata: &std::fs::Metadata) -> Self {
        let kind = Kind::from(metadata.file_type());
        #[cfg(unix)]
        let unix = {
            use std::os::unix::fs::MetadataExt;
            Some(Unix {
                mode: metadata.mode(),
                uid: metadata.uid(),
                nlink: metadata.nlink(),
                dev: metadata.dev(),
                ino: metadata.ino(),
            })
        };
        #[cfg(not(unix))]
        let unix = None;
        Self {
            kind,
            len: metadata.len(),
            modified: metadata.modified().ok().map(Stamp::from),
            readonly: metadata.permissions().readonly(),
            unix,
        }
    }
}

/// 返回 `path` 的文件信息，跟随符号链接。
///
/// # Errors
///
/// 与 `std::fs::metadata` 相同；`path` 是虚拟路径时还有连接的错误，见本模块的说明。
pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::metadata(path).map(|m| Metadata::from(&m)),
        Some(at) => remote::connection(&at.target)?.metadata(&at.path, true),
    }
}

/// 返回 `path` 的文件信息，不跟随符号链接。
///
/// # Errors
///
/// 与 `std::fs::symlink_metadata` 相同；`path` 是虚拟路径时还有连接的错误，见本模块的说明。
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::symlink_metadata(path).map(|m| Metadata::from(&m)),
        Some(at) => remote::connection(&at.target)?.metadata(&at.path, false),
    }
}

/// 一个路径在调用方列表中的位置，以及它在远端的路径。
type Indexed = (usize, Vec<u8>);

/// 对若干路径逐个取文件信息（跟随符号链接），结果与 `paths` 的顺序一一对应。
///
/// 远端路径由一次请求完成，不逐个往返。
pub fn metadata_many<P: AsRef<Path>>(paths: &[P]) -> Vec<io::Result<Metadata>> {
    let mut results: Vec<Option<io::Result<Metadata>>> = Vec::new();
    // 远端的路径按主机分组，每个主机一次往返；本机的路径逐个取。
    let mut groups: Vec<(crate::Target, Vec<Indexed>)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let path = path.as_ref();
        match locate(path) {
            None => {
                let found = std::fs::metadata(path).map(|m| Metadata::from(&m));
                results.push(Some(found));
            }
            Some(at) => {
                results.push(None);
                match groups.iter_mut().find(|(target, _)| *target == at.target) {
                    Some((_, group)) => group.push((index, at.path)),
                    None => groups.push((at.target, vec![(index, at.path)])),
                }
            }
        }
    }
    for (target, group) in groups {
        let remote: Vec<&[u8]> = group.iter().map(|(_, path)| path.as_slice()).collect();
        let found = remote::connection(&target).and_then(|c| c.metadata_many(&remote));
        match found {
            Ok(found) => {
                for ((index, _), metadata) in group.iter().zip(found) {
                    results[*index] = Some(metadata);
                }
            }
            // 整组失败（没有连接、连接中断）：每个路径都是同一种错误。
            Err(error) => {
                for (index, _) in &group {
                    results[*index] = Some(Err(io::Error::new(error.kind(), error.to_string())));
                }
            }
        }
    }
    (results.into_iter())
        .map(|result| {
            result.unwrap_or_else(|| Err(remote::unavailable(remote::Unavailable::NotConnected)))
        })
        .collect()
}

/// 是否表示文件不存在：路径上没有该项，或路径中间的一段不是目录。
///
/// 其他错误（权限不足、连接中断）不说明文件是否存在，不能据此认为文件已被删除。
pub fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

/// 是否存在并且是普通文件（跟随符号链接）；无法确定时为否。
pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok_and(|m| m.is_file())
}

/// 是否存在并且是目录（跟随符号链接）；无法确定时为否。
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok_and(|m| m.is_dir())
}

/// 是否为符号链接（不跟随链接，指向不存在目标的链接同样为是）；无法确定时为否。
pub fn is_symlink(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    symlink_metadata(path).is_ok_and(|m| m.is_symlink())
}

/// 是否存在（跟随符号链接，指向不存在目标的符号链接不算存在）；无法确定时为否。
pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok()
}

/// 读取 `path` 的全部内容。
///
/// # Errors
///
/// 与 `std::fs::read` 相同；`path` 是虚拟路径时还有连接的错误，见本模块的说明。
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::read(path),
        Some(at) => remote::connection(&at.target)?.read(&at.path),
    }
}

/// 读取 `path` 的全部内容并按 UTF-8 解码。
///
/// # Errors
///
/// 与 `std::fs::read_to_string` 相同；`path` 是虚拟路径时还有连接的错误，见本模块的说明。
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::read_to_string(path),
        Some(at) => {
            let bytes = remote::connection(&at.target)?.read(&at.path)?;
            String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidData.into())
        }
    }
}

/// 返回符号链接 `path` 指向的目标。
///
/// # Errors
///
/// 与 `std::fs::read_link` 相同。`path` 是虚拟路径时，结果是远端的目标原样的写法，不是虚拟路径。
pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::read_link(path),
        Some(at) => {
            let target = remote::connection(&at.target)?.read_link(&at.path)?;
            #[cfg(unix)]
            let target: std::ffi::OsString = std::os::unix::ffi::OsStringExt::from_vec(target);
            // Windows 的字符串无法表示不是有效 UTF-8 的目标，换成替换字符。
            #[cfg(not(unix))]
            let target = String::from_utf8_lossy(&target).into_owned();
            Ok(PathBuf::from(target))
        }
    }
}
