//! 复制文件和目录的过程：逐项复制，以及供界面读取的进度和取消的入口。

#[cfg(test)]
mod tests;

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use winc_backend::fs::Kind;

/// 一次复制的进度和取消的入口：执行复制的线程写入，界面线程读取和取消。
#[derive(Debug, Default)]
pub struct Copying {
    cancelled: AtomicBool,
    /// 正在复制的一项已复制的字节数。
    copied: AtomicU64,
    /// 正在复制的一项的名字；它是单个文件时还有文件的大小（字节）。
    item: Mutex<(String, Option<u64>)>,
}

impl Copying {
    /// 取消：正在复制的一项停止并删除已复制出的部分，此前复制完的各项保留。
    ///
    /// 文件在同一台机器上复制时由那台机器一次复制完，复制完这个文件之后才停止。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// 是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// 正在复制的一项已复制的字节数。
    pub fn copied(&self) -> u64 {
        self.copied.load(Ordering::Relaxed)
    }

    /// 正在复制的一项的名字，以及它是单个文件时的大小（字节）；目录的大小事先不知道。
    pub fn item(&self) -> (String, Option<u64>) {
        self.item
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 开始复制名为 `name` 的一项；`size` 是它作为单个文件的大小。
    fn begin(&self, name: String, size: Option<u64>) {
        *self.item.lock().unwrap_or_else(PoisonError::into_inner) = (name, size);
        self.copied.store(0, Ordering::Relaxed);
    }

    /// 记下新复制的 `bytes` 字节，返回是否继续复制。
    fn advance(&self, bytes: u64) -> bool {
        self.copied.fetch_add(bytes, Ordering::Relaxed);
        !self.is_cancelled()
    }
}

/// 复制符号链接的方式：把链接 `from` 复制为 `to`。
pub(crate) type Links = fn(&Path, &Path) -> io::Result<()>;

/// 把 `from` 复制为尚不存在的 `to`：文件复制内容和权限，目录连同其中的内容一起复制，符号链接
/// 按 `links` 复制；进度记入 `copying`。
///
/// # Errors
///
/// 被 `copying` 取消时删除已复制出的 `to`，返回 [`io::ErrorKind::Interrupted`]；其余返回对应的
/// `io::Error`，已复制出的部分保留。
pub(crate) fn copy_item(from: &Path, to: &Path, links: Links, copying: &Copying) -> io::Result<()> {
    let meta = winc_backend::fs::symlink_metadata(from)?;
    copying.begin(
        crate::display_name(from).into_owned(),
        meta.is_file().then_some(meta.len),
    );
    let copied = copy_tree(from, to, meta.kind, links, copying);
    if copied.is_err() && copying.is_cancelled() {
        // `to` 是本次复制新建的：删除它不会碰到原有的内容。还没有建出来时删除失败，不必处理。
        let _ = winc_backend::fs::remove_all(to);
        return Err(io::ErrorKind::Interrupted.into());
    }
    copied
}

/// 复制种类为 `kind` 的 `from`；目录逐项递归，每一项开始之前检查是否已取消。
fn copy_tree(
    from: &Path,
    to: &Path,
    kind: Kind,
    links: Links,
    copying: &Copying,
) -> io::Result<()> {
    if copying.is_cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if kind.is_symlink() {
        return links(from, to);
    }
    if !kind.is_dir() {
        let watch = |bytes| copying.advance(bytes);
        return winc_backend::fs::copy_watched(from, to, &watch).map(drop);
    }
    winc_backend::fs::create_dir(to)?;
    for entry in winc_backend::fs::read_dir(from)? {
        let entry = entry?;
        copy_tree(
            &entry.path(),
            &to.join(&entry.name),
            entry.kind,
            links,
            copying,
        )?;
    }
    Ok(())
}
