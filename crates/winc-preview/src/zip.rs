//! 读 zip 压缩包中的文件：支持「存储」（不压缩）和 deflate 两种压缩方法，不支持 zip64 和加密。
//!
//! 按包末尾的中央目录找各文件的记录，再按记录的偏移找到本地文件头和数据；字段顺序与签名见
//! ZIP 文件格式的 APPNOTE.TXT。

use miniz_oxide::inflate::decompress_to_vec_with_limit;

/// 中央目录结束记录的签名（`0x06054b50`）。
const END_SIGNATURE: u32 = 0x0605_4b50;
/// 中央目录中每个文件记录的签名（`0x02014b50`）。
const ENTRY_SIGNATURE: u32 = 0x0201_4b50;
/// 本地文件头的签名（`0x04034b50`）。
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
/// 中央目录结束记录的长度（字节，不含注释）。
///
/// 固定为 22 字节：签名 4 字节、两段计数各 2 字节、中央目录大小 4 字节、偏移 4 字节、
/// 注释长度 2 字节。
const END_LEN: usize = 22;
/// 解压后一个文件的大小上限（字节，1 GiB，经验值）。
///
/// 防止压缩比极高的包在解压时耗尽内存：解压输出的上限取本值与该文件解压后大小中较小的一个。
const MAX_ENTRY: usize = 1024 * 1024 * 1024;

/// 压缩包中的一个文件。
#[derive(Debug)]
struct Entry {
    /// 包中的路径，用 `/` 分隔。
    name: String,
    /// 压缩方法：0 不压缩，8 deflate。
    method: u16,
    /// 压缩后的大小（字节，即包中的存储长度）。
    compressed: usize,
    /// 解压后的大小（字节）。
    size: usize,
    /// 本地文件头的偏移（字节，从包的起点算起）。
    offset: usize,
}

/// 打开的 zip 压缩包。
pub(crate) struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

/// 读 zip 时的错误。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    /// 不是 zip 压缩包，或者结构损坏。
    Invalid,
    /// zip64、加密的包，或者不支持的压缩方法。
    Unsupported,
}

impl<'a> Archive<'a> {
    /// 读 `bytes` 的中央目录，创建压缩包。
    ///
    /// # Errors
    ///
    /// 找不到中央目录或结构损坏时返回 [`Error::Invalid`]；zip64 或加密的包返回
    /// [`Error::Unsupported`]。
    pub(crate) fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let end = find_end(bytes).ok_or(Error::Invalid)?;
        let count = usize::from(u16_at(bytes, end + 10).ok_or(Error::Invalid)?);
        let start = u32_at(bytes, end + 16).ok_or(Error::Invalid)?;
        if count == 0xffff || start == u32::MAX {
            return Err(Error::Unsupported);
        }
        let mut at = start as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(bytes, at) != Some(ENTRY_SIGNATURE) {
                return Err(Error::Invalid);
            }
            let field = |offset: usize| u32_at(bytes, at + offset).ok_or(Error::Invalid);
            let short = |offset: usize| {
                u16_at(bytes, at + offset)
                    .map(usize::from)
                    .ok_or(Error::Invalid)
            };
            let flags = short(8)?;
            let method = short(10)? as u16;
            let (compressed, size) = (field(20)?, field(24)?);
            let (name_len, extra_len, comment_len) = (short(28)?, short(30)?, short(32)?);
            let offset = field(42)?;
            if compressed == u32::MAX || size == u32::MAX || offset == u32::MAX {
                return Err(Error::Unsupported);
            }
            let name = bytes
                .get(at + 46..at + 46 + name_len)
                .ok_or(Error::Invalid)?;
            // 只有加密标志位（最低位）妨碍读取；其余标志位（如 UTF-8 文件名）不影响内容。
            if flags & 1 != 0 {
                return Err(Error::Unsupported);
            }
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method,
                compressed: compressed as usize,
                size: size as usize,
                offset: offset as usize,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    /// 解压包中路径为 `name` 的文件（不区分大小写）；没有该文件时返回 `None`。
    ///
    /// # Errors
    ///
    /// 文件的压缩方法不支持时返回 [`Error::Unsupported`]；数据损坏或长度超出包的范围时返回
    /// [`Error::Invalid`]。
    pub(crate) fn read(&self, name: &str) -> Option<Result<Vec<u8>, Error>> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))?;
        Some(self.extract(entry))
    }

    /// 解压 `entry` 的数据；`entry` 必须来自本包的中央目录。
    fn extract(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        let bytes = self.bytes;
        let at = entry.offset;
        if u32_at(bytes, at) != Some(LOCAL_SIGNATURE) {
            return Err(Error::Invalid);
        }
        let name_len = usize::from(u16_at(bytes, at + 26).ok_or(Error::Invalid)?);
        let extra_len = usize::from(u16_at(bytes, at + 28).ok_or(Error::Invalid)?);
        let start = at + 30 + name_len + extra_len;
        let data = bytes
            .get(start..start + entry.compressed)
            .ok_or(Error::Invalid)?;
        match entry.method {
            0 => Ok(data.to_vec()),
            8 => {
                let limit = entry.size.min(MAX_ENTRY);
                decompress_to_vec_with_limit(data, limit).map_err(|_| Error::Invalid)
            }
            _ => Err(Error::Unsupported),
        }
    }
}

/// 中央目录结束记录的位置：从末尾往前找签名（末尾可能带最长 65535 字节的注释）。
fn find_end(bytes: &[u8]) -> Option<usize> {
    let last = bytes.len().checked_sub(END_LEN)?;
    let first = last.saturating_sub(usize::from(u16::MAX));
    (first..=last)
        .rev()
        .find(|&at| u32_at(bytes, at) == Some(END_SIGNATURE))
}

/// `bytes` 中 `at` 处的小端 `u16`。
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// `bytes` 中 `at` 处的小端 `u32`。
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
/// 测试用：把 `files`（包中路径、内容）写成 zip：`deflate` 时用 deflate 压缩，否则原样存放。
///
/// `Archive` 读取时不核对 CRC，校验字段固定写 0。
pub(crate) fn build(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    // 本地文件头和中央目录的记录按 ZIP 文件格式（APPNOTE.TXT）逐字段写出；压缩级别 6 是
    // `miniz_oxide` 默认的级别。
    for (name, content) in files {
        let data = if deflate {
            miniz_oxide::deflate::compress_to_vec(content, 6)
        } else {
            content.to_vec()
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0]);
        out.extend(method.to_le_bytes());
        out.extend([0; 8]);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((content.len() as u32).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(&data);

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0]);
        central.extend(method.to_le_bytes());
        central.extend([0; 8]);
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((content.len() as u32).to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend([0, 0]);
    out
}
