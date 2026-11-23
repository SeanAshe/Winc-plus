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
/// 解压后一个文件的大小上限（字节，1 GiB）。
///
/// 防止压缩比极高的包在解压时耗尽内存：解压输出的上限取本值与该文件解压后大小中较小的一个。
/// 表格的文字总量上限也是 1 GiB，再大的部件读出来也显示不了。
const MAX_ENTRY: usize = 1024 * 1024 * 1024;

/// 一个包累计解压出的字节数与包本身大小之比的上限（倍）。
///
/// 处理办公文档的常见实现按 1% 的压缩后与解压后之比判定压缩炸弹，即 100 倍；正常文档在
/// 5 到 20 倍。这里按整个包累计，不按单个文件：内容规整的表格单个部件可以超过 100 倍。
const MAX_RATIO: usize = 100;

/// 包累计解压量的上限不低于的值（字节）：等于 [`MAX_ENTRY`]，很小的包也可以解出一个部件
/// 允许的量。
const MIN_BUDGET: usize = MAX_ENTRY;

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
    /// 各次读取累计可以解压出的字节数，见 [`budget`]。
    budget: usize,
    /// 已经解压出的字节数，各次读取累计。
    spent: std::sync::atomic::AtomicUsize,
    /// 是否有读取因累计解压量超过上限而被拒绝（见 [`Archive::exceeded`]）。
    refused: std::sync::atomic::AtomicBool,
}

/// 读 zip 时的错误。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    /// 不是 zip 压缩包，或者结构损坏。
    Invalid,
    /// zip64、加密的包，或者不支持的压缩方法。
    Unsupported,
    /// 累计解压量超过了这个包的上限（见 [`budget`]）：压缩比异常，不再解压。
    TooLarge,
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
        Ok(Self {
            bytes,
            entries,
            budget: budget(bytes.len()),
            spent: std::sync::atomic::AtomicUsize::new(0),
            refused: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// 解压包中路径为 `name` 的文件（不区分大小写）；没有该文件时返回 `None`。
    ///
    /// # Errors
    ///
    /// 文件的压缩方法不支持时返回 [`Error::Unsupported`]；数据损坏或长度超出包的范围时返回
    /// [`Error::Invalid`]；这个包累计解压量超过上限时返回 [`Error::TooLarge`]。
    pub(crate) fn read(&self, name: &str) -> Option<Result<Vec<u8>, Error>> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))?;
        Some(self.extract(entry))
    }

    /// 测试用：把累计解压量的上限改为 `budget` 字节。
    #[cfg(test)]
    pub(crate) fn with_budget(mut self, budget: usize) -> Self {
        self.budget = budget;
        self
    }

    /// 是否有读取因累计解压量超过上限而被拒绝。
    ///
    /// 各格式把读不出的部件当作没有，文档仍然显示；超过上限不是缺一个部件，整份文档按过大
    /// 处理，调用方转换完后用它判断。
    pub(crate) fn exceeded(&self) -> bool {
        self.refused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 解压 `entry` 的数据；`entry` 必须来自本包的中央目录。
    fn extract(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        use std::sync::atomic::Ordering::Relaxed;
        let left = self.budget.saturating_sub(self.spent.load(Relaxed));
        if entry.size.min(MAX_ENTRY) > left {
            self.refused.store(true, Relaxed);
            return Err(Error::TooLarge);
        }
        let data = self.extract_unmetered(entry)?;
        self.spent.fetch_add(data.len(), Relaxed);
        Ok(data)
    }

    /// 解压 `entry` 的数据，不计入累计解压量。
    fn extract_unmetered(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
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

/// 大小为 `archive` 字节的包各次读取累计可以解压出的字节数：包本身大小的 [`MAX_RATIO`] 倍，
/// 不低于 [`MIN_BUDGET`]。
///
/// 单个文件有 [`MAX_ENTRY`] 的上限，但包里可以有很多个：逐个读下去，解压量没有尽头。
fn budget(archive: usize) -> usize {
    archive.saturating_mul(MAX_RATIO).max(MIN_BUDGET)
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
