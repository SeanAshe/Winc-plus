//! 背景图层的磁盘缓存：缩放后的图层（未按不透明度预乘）和它对应的原图。
//!
//! 只保留最近一张图片的图层。文件开头是固定长度的头部（见 [`HEADER`]），其后是 RGBA 像素；
//! 原图的路径、大小或修改时间与头部不符时缓存作废。

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// 文件开头的标记，格式改变时更换。
const MAGIC: &[u8; 8] = b"WINCWP01";

/// 头部的长度（字节）：标记，原图的路径散列、大小、修改时间（秒、纳秒），原图解码后的宽高，
/// 图层的宽高。
const HEADER: usize = 8 + 8 + 8 + 8 + 4 + 4 * 4;

/// 原图的标识：路径的散列、文件大小和修改时间。
#[derive(Debug, PartialEq)]
pub(super) struct Stamp {
    path: u64,
    len: u64,
    modified: (u64, u32),
}

impl Stamp {
    /// 返回路径为 `path`、元数据为 `metadata` 的图片的标识。
    pub(super) fn of(path: &Path, metadata: &std::fs::Metadata) -> Stamp {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let modified = metadata.modified().ok();
        let since = modified.and_then(|time| time.duration_since(UNIX_EPOCH).ok());
        Stamp {
            path: hasher.finish(),
            len: metadata.len(),
            modified: since.map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos())),
        }
    }

    fn bytes(&self) -> [u8; 28] {
        let mut out = [0; 28];
        out[..8].copy_from_slice(&self.path.to_le_bytes());
        out[8..16].copy_from_slice(&self.len.to_le_bytes());
        out[16..24].copy_from_slice(&self.modified.0.to_le_bytes());
        out[24..].copy_from_slice(&self.modified.1.to_le_bytes());
        out
    }
}

/// 缓存中的图层：大小、原图解码后的大小和非预乘的 RGBA 像素。
pub(super) struct Cached {
    pub size: (u32, u32),
    pub full: (u32, u32),
    pub rgba: Vec<u8>,
}

/// 读取缓存 `file`；文件不存在、格式不符，或不是 `stamp` 这张图片的图层时返回 `None`。
pub(super) fn read(file: &Path, stamp: &Stamp) -> Option<Cached> {
    let mut bytes = std::fs::read(file).ok()?;
    let header = bytes.get(..HEADER)?;
    if &header[..8] != MAGIC || header[8..36] != stamp.bytes() {
        return None;
    }
    let number = |at: usize| {
        let field = header[at..at + 4].try_into().ok()?;
        Some(u32::from_le_bytes(field))
    };
    let full = (number(36)?, number(40)?);
    let size = (number(44)?, number(48)?);
    let pixels = (size.0 as usize)
        .checked_mul(size.1 as usize)?
        .checked_mul(4)?;
    if size.0 == 0 || size.1 == 0 || bytes.len() != HEADER + pixels {
        return None;
    }
    bytes.drain(..HEADER);
    Some(Cached {
        size,
        full,
        rgba: bytes,
    })
}

/// 把图层 `layer` 写入缓存 `file`：先写到同目录的临时文件再改名，读取方不会读到写了一半的文件。
/// 写入失败时不保留缓存，下次重新解码。
pub(super) fn write(file: &Path, stamp: &Stamp, layer: &Cached) {
    let mut bytes = Vec::with_capacity(HEADER + layer.rgba.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&stamp.bytes());
    for n in [layer.full.0, layer.full.1, layer.size.0, layer.size.1] {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    bytes.extend_from_slice(&layer.rgba);
    let partial = file.with_extension("cache.tmp");
    let written = std::fs::write(&partial, &bytes).and_then(|()| std::fs::rename(&partial, file));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}
