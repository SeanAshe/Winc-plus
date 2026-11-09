//! 磁盘上的原文：分段并行扫描以建立检查点索引，按行号或字符位置定位并读取字节。

use super::*;

/// 原文件中某一行行首的位置记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Checkpoint {
    /// 行号（从 0 起）。
    pub(super) line: u64,
    /// 行首的字节偏移，相对于内容起点（不含 BOM）。
    pub(super) byte: u64,
    /// 行首的字符偏移（从 0 起）。
    pub(super) char: u64,
}

/// 磁盘上的原文件及其行索引。
pub(super) struct Original {
    pub(super) file: File,
    /// 文件开头是否有 UTF-8 BOM。
    pub(super) bom: bool,
    /// 内容在文件中的起点（字节偏移）：有 BOM 时跳过它。
    pub(super) base: u64,
    /// 建索引时文件的大小（字节），与 `mtime` 一起用于发现外部修改。
    pub(super) size: u64,
    /// 建索引时文件的修改时间。
    pub(super) mtime: Option<SystemTime>,
    /// 扫描到的内容字节数（不含 BOM）。
    pub(super) len: u64,
    /// 扫描到的内容字符数。
    pub(super) chars: u64,
    /// 扫描到的内容中的换行数（`\n` 的个数）。
    pub(super) newlines: u64,
    /// 检查点，行号和字符偏移均升序。
    pub(super) checkpoints: Vec<Checkpoint>,
    /// 超长行内部的位置记录，字符偏移升序；`line` 是所在行的行号。
    ///
    /// 检查点只设在行首，按字符位置查找时最多要从行首读到目标。超长行可达数 MB，
    /// 因此在行内每隔一个读取块（[`CHUNK`]）另记一处，查找时最多读一个读取块。
    pub(super) marks: Vec<Checkpoint>,
    /// 内容是否全部是合法的 UTF-8。
    pub(super) utf8: bool,
    /// 内容中第一个换行是否为 `\r\n`。
    pub(super) crlf: bool,
    /// 最长一行的字节数（不含 `\n`）。
    pub(super) longest_line: u64,
    /// 建索引时内容末尾的最多 4 KiB。
    ///
    /// 发现外部修改时文件已经改变，`Original` 里只剩当时的行索引。`grow` 用这份记录确认
    /// 原有的末尾没有变动；重新读取文件得到的已是修改后的内容，无法用于比较。
    pub(super) tail: Vec<u8>,
}

/// 一段内容的扫描结果。
///
/// 检查点的行号和字符偏移相对于该段开头，字节偏移相对于内容起点（不含 BOM）。
pub(super) struct Segment {
    pub(super) checkpoints: Vec<Checkpoint>,
    /// 超长行内部的位置记录，坐标的含义同 `checkpoints`。
    pub(super) marks: Vec<Checkpoint>,
    /// 段内换行数（个，`\n` 的个数）。
    pub(super) newlines: u64,
    /// 段内字符数（个，非法字节按一个字符计）。
    pub(super) chars: u64,
    /// 实际扫描到的位置（字节偏移）。
    ///
    /// 文件在扫描中被截短时小于计划的终点。
    pub(super) end: u64,
    /// 段内内容是否全部是合法 UTF-8。
    pub(super) utf8: bool,
    /// 段内第一个换行是否为 `\r\n`；段内没有换行时为 `None`。
    pub(super) first_crlf: Option<bool>,
    /// 段内最长一行的字节数（字节，不含 `\n`）。
    pub(super) longest_line: u64,
}

impl Original {
    /// 复制结构体：另建一个指向同一文件的文件句柄，其余字段原样复制。
    pub(super) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            checkpoints: self.checkpoints.clone(),
            marks: self.marks.clone(),
            tail: self.tail.clone(),
            ..*self
        })
    }

    /// 原有内容的末尾（最多 4 KiB）是否与建索引时记录的 `tail` 相同。
    ///
    /// 相同时认定原有内容没有变化，文件变长只是因为在末尾追加了内容。
    pub(super) fn tail_intact(&self) -> bool {
        let check = self.len.min(4 << 10);
        let mut now = vec![0; check as usize];
        read_full(&self.file, self.base + self.len - check, &mut now)
            .is_ok_and(|n| n == now.len() && now == self.tail)
    }

    /// 打开 `file` 并按 `workers` 个线程分段扫描并建立索引（实际线程数见 [`LargeText::open`]）。
    pub(super) fn open(file: File, workers: u64) -> io::Result<Self> {
        let metadata = file.metadata()?;
        let mut head = [0u8; 3];
        let bom = read_full(&file, 0, &mut head)? == 3 && head == UTF8_BOM;
        let base = if bom { 3 } else { 0 };

        let mut original = Original {
            file,
            bom,
            base,
            size: metadata.len(),
            mtime: metadata.modified().ok(),
            len: metadata.len().saturating_sub(base),
            chars: 0,
            newlines: 0,
            checkpoints: Vec::new(),
            marks: Vec::new(),
            utf8: true,
            crlf: false,
            longest_line: 0,
            tail: Vec::new(),
        };
        original.index(workers)?;
        Ok(original)
    }

    /// 把内容切成约 `count` 段并行扫描，再按顺序合并各段的检查点。
    fn index(&mut self, count: u64) -> io::Result<()> {
        let bounds = segment_bounds(&self.file, self.base, self.len, count)?;

        // 每段使用一个后台线程；线程不能借用 `self.file`，因此克隆文件句柄后以 `Arc` 共享。
        let file = Arc::new(self.file.try_clone()?);
        let (base, parts) = (self.base, bounds.len() - 1);
        let bounds: Arc<[u64]> = bounds.into();
        let segments = crate::background::split(parts, parts, move |i| {
            scan_segment(&file, base, bounds[i], bounds[i + 1])
        })
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;

        let total: usize = segments.iter().map(|s| s.checkpoints.len()).sum();
        self.checkpoints = Vec::with_capacity(total);
        let (mut line, mut char) = (0, 0);

        for segment in &segments {
            let shift = |c: &Checkpoint| Checkpoint {
                line: line + c.line,
                byte: c.byte,
                char: char + c.char,
            };
            self.checkpoints
                .extend(segment.checkpoints.iter().map(shift));
            self.marks.extend(segment.marks.iter().map(shift));
            line += segment.newlines;
            char += segment.chars;
            self.utf8 &= segment.utf8;
            self.longest_line = self.longest_line.max(segment.longest_line);
        }

        self.crlf = segments.iter().find_map(|s| s.first_crlf).unwrap_or(false);
        self.len = segments.last().map_or(0, |s| s.end);
        // 记录内容末尾的最多 4 KiB，供 `grow` 与文件当前的末尾比较。
        let check = self.len.min(4 << 10) as usize;
        let mut tail = vec![0; check];
        if read_full(&self.file, self.base + self.len - check as u64, &mut tail).is_ok() {
            self.tail = tail;
        }
        self.newlines = line;
        self.chars = char;
        Ok(())
    }

    /// 读取内容中从 `offset`（字节偏移）开始的字节，不超过建索引时的长度；返回读到的字节数。
    ///
    /// 读取出错时按已到达末尾处理，返回 0。
    pub(super) fn read(&self, offset: u64, buf: &mut [u8]) -> usize {
        let available = self.len.saturating_sub(offset).min(buf.len() as u64) as usize;
        read_full(&self.file, self.base + offset, &mut buf[..available]).unwrap_or(0)
    }

    /// 从 `offset` 读取一块，末尾被截断的多字节字符不计入返回的切片；到达末尾时返回空切片。
    pub(super) fn read_aligned<'b>(&self, offset: u64, buf: &'b mut [u8]) -> &'b [u8] {
        let n = self.read(offset, buf);
        let take = if n < buf.len() {
            n
        } else {
            complete_prefix(&buf[..n]).max(1)
        };
        &buf[..take]
    }

    /// 返回满足 `before` 的最后一个检查点；没有满足条件的检查点时返回第一个检查点。
    fn checkpoint_by(&self, before: impl Fn(&Checkpoint) -> bool) -> Checkpoint {
        self.checkpoints[self.checkpoints.partition_point(before).saturating_sub(1)]
    }

    /// 返回第 `line` 行（从 0 起）行首的（字节偏移，字符偏移）；`line` 超出时取最后一行。
    pub(super) fn locate_line(&self, line: u64) -> (u64, u64) {
        let line = line.min(self.newlines);
        let mut at = self.checkpoint_by(|c| c.line <= line);
        // 超长行的下一行行首恰好有检查点，无需读取；逐行跳过超长行时每行都会执行此分支。
        if at.line == line {
            return (at.byte, at.char);
        }
        let mut buf = [0u8; PROBE];

        while at.line < line {
            let chunk = self.read_aligned(at.byte, &mut buf);

            // 文件被截短，读不到足够的行。
            if chunk.is_empty() {
                break;
            }

            let need = (line - at.line) as usize;

            match memchr::memchr_iter(b'\n', chunk).nth(need - 1) {
                Some(i) => {
                    at = Checkpoint {
                        line,
                        byte: at.byte + i as u64 + 1,
                        char: at.char + char_count(&chunk[..=i]),
                    };
                }
                None => {
                    at.line += memchr::memchr_iter(b'\n', chunk).count() as u64;
                    at.char += char_count(chunk);
                    at.byte += chunk.len() as u64;
                }
            }
        }

        (at.byte, at.char)
    }

    /// 返回第 `char` 个字符的（字节偏移，所在行号）；`char` 超出时取末尾。
    pub(super) fn locate_char(&self, char: u64) -> (u64, u64) {
        let char = char.min(self.chars);
        let mut at = self.checkpoint_by(|c| c.char <= char);
        // 目标在超长行内部时，从行内最近的位置记录开始读。
        let i = self.marks.partition_point(|m| m.char <= char);
        if let Some(mark) = i.checked_sub(1).map(|i| self.marks[i])
            && mark.byte > at.byte
        {
            at = mark;
        }
        let mut buf = [0u8; PROBE];

        while at.char < char {
            let chunk = self.read_aligned(at.byte, &mut buf);

            if chunk.is_empty() {
                break;
            }

            let need = (char - at.char) as usize;
            let chars = char_count(chunk) as usize;
            let take = if chars > need {
                byte_of_char(chunk, need)
            } else {
                chunk.len()
            };

            at.line += memchr::memchr_iter(b'\n', &chunk[..take]).count() as u64;
            at.char += need.min(chars) as u64;
            at.byte += take as u64;
        }

        (at.byte, at.line)
    }
}

/// 返回各段的分界字节偏移（含起点 0 和终点 `len`）。
///
/// 分界点从等分点后移到下一个换行之后，使每段都从行首开始；无法后移的分界点被舍弃。
fn segment_bounds(file: &File, base: u64, len: u64, count: u64) -> io::Result<Vec<u64>> {
    let mut bounds = vec![0];
    let mut buf = [0u8; PROBE];

    for k in 1..count {
        let last = *bounds.last().expect("至少有起点");
        let mut at = (len * k / count).max(last);

        while at < len {
            let n = read_full(
                file,
                base + at,
                &mut buf[..(len - at).min(PROBE as u64) as usize],
            )?;

            if n == 0 {
                at = len;
                break;
            }

            if let Some(i) = memchr::memchr(b'\n', &buf[..n]) {
                at += i as u64 + 1;
                break;
            }

            at += n as u64;
        }

        if at < len && at > last {
            bounds.push(at);
        }
    }

    bounds.push(len);
    Ok(bounds)
}

/// 扫描内容中 `from..to`（字节偏移）这一段：设置检查点、统计换行和字符、找出最长的行、校验 UTF-8。
///
/// `from` 必须是行首。
pub(super) fn scan_segment(file: &File, base: u64, from: u64, to: u64) -> io::Result<Segment> {
    // 按平均每行 64 字节预估检查点个数，一次分配到位，避免反复扩容留下的内存碎片；估计值的
    // 偏差只影响扩容次数，不影响结果。
    let mut checkpoints = Vec::with_capacity(((to - from) / (CHECKPOINT_LINES * 64)) as usize + 1);
    let mut last = Checkpoint {
        line: 0,
        byte: from,
        char: 0,
    };
    checkpoints.push(last);

    let mut segment = Segment {
        checkpoints,
        marks: Vec::new(),
        newlines: 0,
        chars: 0,
        end: from,
        utf8: true,
        first_crlf: None,
        longest_line: 0,
    };
    let mut buf = vec![0u8; CHUNK];
    let mut line_start = from;

    while segment.end < to {
        let want = ((to - segment.end) as usize).min(CHUNK);
        let n = read_full(file, base + segment.end, &mut buf[..want])?;

        if n == 0 {
            break;
        }

        // 块尾被截断的多字节字符留给下一块，使每块都能单独校验和统计字符。
        let take = if n < CHUNK {
            n
        } else {
            complete_prefix(&buf[..n]).max(1)
        };
        let chunk = &buf[..take];
        let valid = std::str::from_utf8(chunk).is_ok();
        let count = |bytes: &[u8]| {
            if valid {
                count_valid(bytes)
            } else {
                char_count(bytes)
            }
        };
        let mut counted = 0;
        segment.utf8 &= valid;

        for i in memchr::memchr_iter(b'\n', chunk) {
            let at = segment.end + i as u64;
            segment
                .first_crlf
                .get_or_insert(i > 0 && chunk[i - 1] == b'\r');
            segment.longest_line = segment.longest_line.max(at - line_start);
            segment.newlines += 1;
            line_start = at + 1;

            if segment.newlines - last.line >= CHECKPOINT_LINES
                || line_start - last.byte >= CHECKPOINT_BYTES
            {
                segment.chars += count(&chunk[counted..=i]);
                counted = i + 1;
                last = Checkpoint {
                    line: segment.newlines,
                    byte: line_start,
                    char: segment.chars,
                };
                segment.checkpoints.push(last);
            }
        }

        segment.chars += count(&chunk[counted..]);
        segment.end += take as u64;

        // 块尾位于一行中间，且距这一行最近的记录已超过 `CHECKPOINT_BYTES`：在块尾记录位置。
        let recorded = segment
            .marks
            .last()
            .map_or(last.byte, |m| m.byte.max(last.byte));
        if segment.end > line_start && segment.end - recorded >= CHECKPOINT_BYTES {
            segment.marks.push(Checkpoint {
                line: segment.newlines,
                byte: segment.end,
                char: segment.chars,
            });
        }
    }

    segment.longest_line = segment.longest_line.max(segment.end - line_start);
    Ok(segment)
}

/// 从文件的 `offset`（字节偏移）处读满 `buf` 或读到文件末尾，返回读到的字节数。
pub(super) fn read_full(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;

    while filled < buf.len() {
        match read_at(file, &mut buf[filled..], offset + filled as u64) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }

    Ok(filled)
}

/// 从 `offset` 读取 `buf`，不改变文件当前的读取位置（Unix 用 `pread`）。
#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

/// 从 `offset` 读取 `buf`，不改变文件当前的读取位置（Windows 用 `seek_read`）。
#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}
