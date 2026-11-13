//! 列目录。

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::Kind;
use crate::{locate, remote};

/// 目录中的一项。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 名字，不含目录。
    pub name: OsString,
    /// 种类，不跟随符号链接：符号链接本身为 [`Kind::Symlink`]。
    pub kind: Kind,
    dir: Arc<Path>,
}

impl Entry {
    /// 返回该项的完整路径：所列的目录加上名字。
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }
}

/// [`read_dir`] 返回的迭代器，逐项给出目录的直接子项，顺序不确定，不含 `.` 和 `..`。
///
/// 读取某一项或取其种类失败时该项为错误，其余各项不受影响。
pub struct ReadDir {
    dir: Arc<Path>,
    entries: Entries,
}

/// 目录项的来源。
enum Entries {
    /// 本机的目录，逐项读取。部分系统上该类型有数百字节，装箱后两个变体大小相近。
    Local(Box<std::fs::ReadDir>),
    /// 远端的目录，已随一次应答全部取回：名字（远端的原始字节）和种类。
    Remote(std::vec::IntoIter<(Vec<u8>, Kind)>),
}

impl Iterator for ReadDir {
    type Item = io::Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        let dir = Arc::clone(&self.dir);
        match &mut self.entries {
            Entries::Local(entries) => Some(entries.next()?.and_then(|entry| {
                Ok(Entry {
                    name: entry.file_name(),
                    kind: entry.file_type()?.into(),
                    dir,
                })
            })),
            Entries::Remote(entries) => {
                let (name, kind) = entries.next()?;
                let name = crate::path::segment_name(&name);
                Some(Ok(Entry { name, kind, dir }))
            }
        }
    }
}

/// 列出目录 `path` 的直接子项。
///
/// # Errors
///
/// 与 `std::fs::read_dir` 相同；`path` 是虚拟路径时还有连接的错误，见 [`fs`](crate::fs) 的说明。
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    let path = path.as_ref();
    let entries = match locate(path) {
        None => Entries::Local(Box::new(std::fs::read_dir(path)?)),
        Some(at) => {
            let entries = remote::connection(&at.target)?.read_dir(&at.path)?;
            Entries::Remote(entries.into_iter())
        }
    };
    Ok(ReadDir {
        dir: Arc::from(path),
        entries,
    })
}
