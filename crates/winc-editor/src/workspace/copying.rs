//! 复制文件和目录的过程：逐项复制，以及供界面读取的进度和取消的入口。

#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use winc_i18n::t;

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

/// 下载目录时跳过的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// 相对下载项所在目录的路径，以 `/` 分隔，如 `project/data/app.sock`。
    pub name: String,
    /// 跳过的原因，用于提示。
    pub reason: String,
}

/// 下载一项时跳过的子项及出错的位置。
#[derive(Debug)]
pub(crate) struct Skips {
    /// 下载项的路径。
    root: PathBuf,
    /// 跳过的子项，按遍历顺序。
    pub list: Vec<Skipped>,
    /// 出错的子项；错误发生在下载项本身时为 `None`。
    failed: Option<PathBuf>,
}

impl Skips {
    pub(crate) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            list: Vec::new(),
            failed: None,
        }
    }

    /// 出错位置相对下载项所在目录的路径，写法同 [`Skipped::name`]。
    pub(crate) fn failed_name(&self) -> String {
        self.name(self.failed.as_deref().unwrap_or(&self.root))
    }

    fn skip(&mut self, path: &Path, reason: String) {
        let name = self.name(path);
        self.list.push(Skipped { name, reason });
    }

    /// `path` 相对下载项所在目录的路径，写法同 [`Skipped::name`]。
    fn name(&self, path: &Path) -> String {
        let root = crate::display_name(&self.root);
        match winc_backend::relative_lossy(path, &self.root) {
            Some(relative) if !relative.is_empty() => {
                format!("{root}/{}", relative.replace('\\', "/"))
            }
            _ => root.into_owned(),
        }
    }
}

/// 把 `from` 复制为尚不存在的 `to`：文件复制内容和权限，目录连同其中的内容一起复制，符号链接
/// 按 `links` 复制；进度记入 `copying`。
///
/// # Errors
///
/// 被 `copying` 取消时删除已复制出的 `to`，返回 [`io::ErrorKind::Interrupted`]；其余返回对应的
/// `io::Error`，已复制出的部分保留。
pub(crate) fn copy_item(from: &Path, to: &Path, links: Links, copying: &Copying) -> io::Result<()> {
    copy_top(from, to, links, copying, None)
}

/// 同 [`copy_item`]，但跳过目录中无法跨主机传输的子项并记入 `skips`，规则见 [`copy_tree`]。
///
/// # Errors
///
/// 同 [`copy_item`]；子项出错时把该子项记入 `skips`。
pub(crate) fn download_item(
    from: &Path,
    to: &Path,
    links: Links,
    copying: &Copying,
    skips: &mut Skips,
) -> io::Result<()> {
    copy_top(from, to, links, copying, Some(skips))
}

fn copy_top(
    from: &Path,
    to: &Path,
    links: Links,
    copying: &Copying,
    skips: Option<&mut Skips>,
) -> io::Result<()> {
    let meta = winc_backend::fs::symlink_metadata(from)?;
    copying.begin(
        crate::display_name(from).into_owned(),
        meta.is_file().then_some(meta.len),
    );
    let copied = copy_tree(from, to, meta.kind, links, copying, skips);
    if copied.is_err() && copying.is_cancelled() {
        // `to` 是本次复制新建的：删除它不会碰到原有的内容。还没有建出来时删除失败，不必处理。
        let _ = winc_backend::fs::remove_all(to);
        return Err(io::ErrorKind::Interrupted.into());
    }
    copied
}

/// 复制种类为 `kind` 的 `from`；目录逐项递归，每一项开始之前检查是否已取消。
///
/// 有 `skips` 时跳过目录中的特殊文件（设备、命名管道、套接字）和按 [`link_skip_reason`] 无法
/// 复制的符号链接，并记入其中；`from` 本身不跳过。
fn copy_tree(
    from: &Path,
    to: &Path,
    kind: Kind,
    links: Links,
    copying: &Copying,
    mut skips: Option<&mut Skips>,
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
        let path = entry.path();
        let target = to.join(winc_backend::carry_name(&entry.name, from, to));
        let Some(skips) = skips.as_deref_mut() else {
            copy_tree(&path, &target, entry.kind, links, copying, None)?;
            continue;
        };
        if entry.kind.is_other() {
            skips.skip(&path, t!("editor-download-skipped-special").to_owned());
            continue;
        }
        if let Err(error) = copy_tree(&path, &target, entry.kind, links, copying, Some(skips)) {
            if entry.kind.is_symlink()
                && let Some(reason) = link_skip_reason(&error)
            {
                skips.skip(&path, reason);
                continue;
            }
            skips.failed.get_or_insert(path);
            return Err(error);
        }
    }
    Ok(())
}

/// 符号链接复制失败的错误 `error` 表明该链接无法跨主机传输时，返回用于提示的原因；其他错误
/// （如连接断开、权限不足）返回 `None`。
///
/// 以复制目标内容的方式复制链接（Windows）时，目标不存在、为目录、为特殊文件或存在循环都会失败。
/// 远端只传回错误的种类和描述，`ELOOP` 没有稳定的 [`io::ErrorKind`]，按描述识别。
fn link_skip_reason(error: &io::Error) -> Option<String> {
    Some(match error.kind() {
        io::ErrorKind::NotFound => t!("editor-download-skipped-link-missing").to_owned(),
        io::ErrorKind::IsADirectory => t!("editor-download-skipped-link-folder").to_owned(),
        io::ErrorKind::InvalidInput => t!("editor-download-skipped-special").to_owned(),
        _ => {
            let text = error.to_string().to_ascii_lowercase();
            let looped = ["too many levels of symbolic links", "symbolic link loop"];
            if !looped.iter().any(|loop_text| text.contains(loop_text)) {
                return None;
            }
            t!("editor-download-skipped-link-loop").to_owned()
        }
    })
}
