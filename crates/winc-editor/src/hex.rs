//! 十六进制视图：按需从磁盘读文件的字节，每行 [`ROW`] 个；选中一段字节，复制成十六进制或文字。
//!
//! 只读，文件不读进内存：绘制时只取看得见的几行，按块读取并缓存最近用过的几块，文件多大都能
//! 滚动。文件被其他程序修改后按新的文件信息作废缓存（[`HexView::refresh`]），下次读取时重新
//! 打开文件。

use std::cell::RefCell;
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use winc_backend::fs::{Handle, Metadata, Stamp};

/// 每行的字节数。
pub const ROW: u64 = 16;
/// 一次从磁盘读的字节数（64 KiB）。
const BLOCK: u64 = 64 << 10;
/// 缓存的块数，合计 4 MiB。
const KEPT: usize = 64;
/// 一次最多复制的字节数（16 MiB），与按文本打开的大文件的界限相同。
pub const COPY_LIMIT: u64 = crate::LARGE_FILE;

/// 一个文件的十六进制视图：文件的大小、读过的块和选中的字节。
pub struct HexView {
    path: PathBuf,
    /// 打开的文件；文件变化后清掉，下次读取时重新打开。
    file: RefCell<Option<Handle>>,
    size: u64,
    mtime: Option<Stamp>,
    /// 读过的块：（块号，内容），最近用过的在后。
    blocks: RefCell<Vec<(u64, Vec<u8>)>>,
    /// 选区开始的字节；与 `head` 相同时只选中这一个字节。
    pub anchor: u64,
    /// 光标所在的字节。
    pub head: u64,
    /// 打开文件失败的原因。
    pub error: Option<String>,
}

impl std::fmt::Debug for HexView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HexView")
            .field("size", &self.size)
            .field("anchor", &self.anchor)
            .field("head", &self.head)
            .finish_non_exhaustive()
    }
}

impl HexView {
    /// 打开文件 `path`（会访问磁盘，在后台线程上调用）；失败时视图为空，原因记在 `error` 里。
    pub fn open(path: &Path) -> HexView {
        let mut view = HexView {
            path: path.to_path_buf(),
            file: RefCell::default(),
            size: 0,
            mtime: None,
            blocks: RefCell::default(),
            anchor: 0,
            head: 0,
            error: None,
        };
        match winc_backend::fs::open(path, 0) {
            Ok(opened) => {
                (view.size, view.mtime) = (opened.metadata.len, opened.metadata.modified);
                *view.file.get_mut() = Some(opened.into_handle());
            }
            Err(e) => view.error = Some(e.to_string()),
        }
        view
    }

    /// 文件变成了 `metadata` 所示的样子：清空读过的块，下次读取时重新打开文件；选区收到新的
    /// 大小以内。不访问磁盘。
    pub fn refresh(&mut self, metadata: &Metadata) {
        self.blocks.get_mut().clear();
        *self.file.get_mut() = None;
        (self.size, self.mtime) = (metadata.len, metadata.modified);
        self.error = None;
        let last = self.size.saturating_sub(1);
        (self.anchor, self.head) = (self.anchor.min(last), self.head.min(last));
    }

    /// 文件信息 `metadata` 是否与打开时的不同：大小或修改时间变了。
    pub fn outdated(&self, metadata: &Metadata) -> bool {
        metadata.len != self.size || metadata.modified != self.mtime
    }

    /// 文件的大小（字节）。
    pub fn size(&self) -> u64 {
        self.size
    }

    /// 行数：每行 [`ROW`] 个字节，空文件也有一行。
    pub fn rows(&self) -> usize {
        usize::try_from(self.size.div_ceil(ROW))
            .unwrap_or(usize::MAX)
            .max(1)
    }

    /// 从 `start` 起最多 `len` 个字节，到文件末尾为止；读取失败时只有已读到的部分。
    pub fn bytes(&self, start: u64, len: u64) -> Vec<u8> {
        let end = start.saturating_add(len).min(self.size);
        let mut out = Vec::with_capacity(end.saturating_sub(start) as usize);
        let mut at = start;
        while at < end {
            let index = at / BLOCK;
            let took = self.with_block(index, |block| {
                let from = (at - index * BLOCK) as usize;
                let to = ((end - index * BLOCK) as usize).min(block.len());
                out.extend_from_slice(block.get(from..to).unwrap_or_default());
                to.saturating_sub(from)
            });
            match took {
                Some(n) if n > 0 => at += n as u64,
                _ => break,
            }
        }
        out
    }

    /// 用第 `index` 块的内容运行 `f`：缓存里没有时从磁盘读入；读不到时返回 `None`。
    fn with_block<T>(&self, index: u64, f: impl FnOnce(&[u8]) -> T) -> Option<T> {
        let mut blocks = self.blocks.borrow_mut();
        if let Some(at) = blocks.iter().position(|(i, _)| *i == index) {
            let entry = blocks.remove(at);
            blocks.push(entry);
        } else {
            let data = self.read_block(index)?;
            if blocks.len() >= KEPT {
                blocks.remove(0);
            }
            blocks.push((index, data));
        }
        blocks.last().map(|(_, data)| f(data))
    }

    /// 从磁盘读第 `index` 块；文件还没有打开时先打开。
    fn read_block(&self, index: u64) -> Option<Vec<u8>> {
        let mut file = self.file.borrow_mut();
        if file.is_none() {
            *file = Some(winc_backend::fs::open(&self.path, 0).ok()?.into_handle());
        }
        let file = file.as_ref()?;
        let start = index * BLOCK;
        let mut data = vec![0; BLOCK.min(self.size.saturating_sub(start)) as usize];
        let mut filled = 0;
        while filled < data.len() {
            match file.read_at(start + filled as u64, &mut data[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
        data.truncate(filled);
        Some(data)
    }

    /// 选中的字节：从选区较小的一端到较大的一端（含）；空文件为空。
    pub fn selection(&self) -> Range<u64> {
        if self.size == 0 {
            return 0..0;
        }
        let (start, end) = (self.anchor.min(self.head), self.anchor.max(self.head));
        start..(end + 1).min(self.size)
    }

    /// 把光标移到字节 `at`（限制在文件以内）；`extend` 时保留选区的另一端，否则只选中它。
    pub fn move_to(&mut self, at: u64, extend: bool) {
        self.head = at.min(self.size.saturating_sub(1));
        if !extend {
            self.anchor = self.head;
        }
    }

    /// 选中整个文件。
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.size.saturating_sub(1);
    }
}

/// 把 `bytes` 写成十六进制：每个字节两位大写，用空格隔开（`4D 5A 90`）。
pub fn hex_text(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{b:02X}");
    }
    out
}

/// 字节 `b` 在 ASCII 一栏里的样子：可显示的 ASCII 字符照原样，其余为 `None`。
pub fn ascii(b: u8) -> Option<char> {
    (b.is_ascii_graphic() || b == b' ').then_some(char::from(b))
}

#[cfg(test)]
mod tests;
