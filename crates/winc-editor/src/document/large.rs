//! 大文件文本 [`LargeText`]：原文留在磁盘上按需读取，内存里只保存行索引和改动（piece table）。
//!
//! 打开时扫描一遍文件，每隔一定行数或字节数记录一个检查点；按位置或行号查找时，只需从最近的
//! 检查点向后读取少量字节。编辑记录为对原文件和新增缓冲的片段引用，原文件本身不被修改。
//! 存在超长行时只读打开。

mod lines;
mod original;
mod utf8;

use original::*;
pub(crate) use utf8::*;

use std::fs::File;
use std::io::{self, Write};
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use winc_core::text::{EditBuffer, Line, Lines};
use winc_i18n::t;

use crate::document::encoding::UTF8_BOM;

/// 一行最多能访问的字节数（字节，16 MiB）。
///
/// 原文件中有更长的行时只读打开，行内超出的部分不可访问。
pub(crate) const LINE_CAP: usize = 16 << 20;
/// 相邻检查点之间的最大行数（行）。
///
/// 与 [`CHECKPOINT_BYTES`] 任一条件满足即设置新的检查点。
const CHECKPOINT_LINES: u64 = 64;
/// 相邻检查点之间的最大字节数（字节，64 KiB）。
///
/// 与 [`CHECKPOINT_LINES`] 任一条件满足即设置新的检查点。
const CHECKPOINT_BYTES: u64 = 64 << 10;
/// 顺序扫描时每次读取的字节数（字节，128 KiB）。
///
/// 建索引只有解码与统计的开销，扫描的耗时因此趋近磁盘的顺序读取带宽，再增大也不会明显缩短建索引
/// 的时间，反而使释放后的缓冲在分配器中多占用内存。该值取自调参，作用范围限于顺序扫描。
const CHUNK: usize = 128 << 10;
/// 建索引时每段至少包含的字节数（字节，32 MiB）。
///
/// 线程数为 `size / SEGMENT_MIN`，限制在 1 到 [`MAX_WORKERS`]（且不超过 CPU 核数）之间，
/// 因此小文件只用一个线程。
const SEGMENT_MIN: u64 = 32 << 20;
/// 建索引最多使用的线程数（个）。
///
/// 扫描是顺序读取，耗时趋近磁盘的读取带宽，再增加线程也不会缩短建索引的时间，反而加大分段结果
/// 合并的开销。该值取自调参，实际线程数同时受 CPU 核数与 `SEGMENT_MIN` 限制。
const MAX_WORKERS: usize = 8;
/// 从检查点向后查找时每次读取的字节数（字节，16 KiB）。
///
/// 相邻检查点通常相距不超过 [`CHECKPOINT_BYTES`]，一般读取数次即可到达目标位置。
const PROBE: usize = 16 << 10;
/// 按顺序读取时第一次读取的字节数（字节，4 KiB），之后每次翻倍，直到 [`CHUNK`]。
///
/// 读取短行时只需一次小规模读取。
const FIRST_READ: usize = 4 << 10;

/// 片段内容的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Original,
    /// 内存中的新增缓冲（`LargeText::added`）。
    Added,
}

/// 一个片段：来源里的一段字节，以及其中的字符数和换行数。
#[derive(Debug, Clone, Copy)]
struct Piece {
    source: Source,
    /// 片段在来源中的起始字节偏移。
    start: u64,
    bytes: u64,
    chars: u64,
    newlines: u64,
    /// 片段起点在原文件中的字符偏移，用来查行索引，仅对原文件的片段有意义。
    orig_char: u64,
    /// 片段起点在原文件中的行号（从 0 起），用途同 `orig_char`。
    orig_line: u64,
}

/// 文本中的一个位置：第 `piece` 个片段（从 0 起）内的第 `offset` 个字节。
///
/// `piece` 等于片段数时表示文本末尾。
#[derive(Debug, Clone, Copy)]
struct Loc {
    piece: usize,
    offset: u64,
}

/// 大文件的文本：原文留在磁盘上按需读取，内存里只保存新增的文字和片段表。
///
/// 位置使用 `char` 偏移，遵循 [`Lines`] 的约定；读取时按片段换算成文件偏移，因此改动少的
/// 文件不必整篇读入内存。
pub struct LargeText {
    original: Original,
    /// 新增缓冲：插入的文字都追加在这里，只增不减。
    added: String,
    pieces: Vec<Piece>,
    /// `chars_before[i]`、`lines_before[i]` 是前 `i` 个片段的字符数和换行数，比片段多一项。
    chars_before: Vec<u64>,
    lines_before: Vec<u64>,
    /// 单行可访问的字节数上限（字节），默认为 `LINE_CAP`。
    line_cap: usize,
    /// 最近读出的一行：（行号，文字，字符数）。
    ///
    /// 一次按键需要多次读取光标所在的行（换算行列、查找字素边界、上下移动），每次都要从磁盘读出
    /// 并解码整行；缓存最近读出的一行后，再次读取同一行时不访问磁盘。片段表变化时清空缓存
    /// （`LargeText::rebuild_from`）。
    last_line: Mutex<Option<(usize, String, usize)>>,
}

impl LargeText {
    /// 打开大文件：扫描整个文件建立行索引，同时检测 UTF-8 合法性、换行风格和超长行。
    ///
    /// # Errors
    ///
    /// 读取文件失败时返回对应的 `std::io::Error`。
    pub fn open(file: File) -> io::Result<Self> {
        Self::open_with(file, LINE_CAP)
    }

    fn open_with(file: File, line_cap: usize) -> io::Result<Self> {
        let size = file.metadata()?.len();
        let cores = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_WORKERS);
        let original = Original::open(file, (size / SEGMENT_MIN).clamp(1, cores as u64))?;

        let pieces = if original.len > 0 {
            vec![Piece {
                source: Source::Original,
                start: 0,
                bytes: original.len,
                chars: original.chars,
                newlines: original.newlines,
                orig_char: 0,
                orig_line: 0,
            }]
        } else {
            Vec::new()
        };

        let mut text = LargeText {
            original,
            added: String::new(),
            pieces,
            chars_before: Vec::new(),
            lines_before: Vec::new(),
            line_cap,
            last_line: Mutex::new(None),
        };
        text.rebuild();
        Ok(text)
    }

    /// 是否原文件的全部内容都是合法的 UTF-8。
    pub fn is_utf8(&self) -> bool {
        self.original.utf8
    }

    /// 是否原文件开头有 UTF-8 BOM。
    pub fn has_bom(&self) -> bool {
        self.original.bom
    }

    /// 是否原文件的第一个换行是 CRLF（`\r\n`）。
    pub fn is_crlf(&self) -> bool {
        self.original.crlf
    }

    /// 是否存在长于 `LINE_CAP` 的行。
    ///
    /// 存在这样的行时只读打开，超出的部分不可访问。
    pub fn has_overlong_lines(&self) -> bool {
        self.original.longest_line > self.line_cap as u64
    }

    /// 原文件在打开之后是否被其他程序原地修改过，且不只是在末尾追加内容。
    ///
    /// 片段引用原文件中的字节，原文件变化后再保存会写出错误的内容。只在末尾追加（例如日志）
    /// 时，片段引用的部分没有变化，仍然可以写出；追加的内容不在文档中，保存后被文档的内容覆盖。
    pub fn original_changed(&self) -> bool {
        let Ok(metadata) = self.original.file.metadata() else {
            return true;
        };
        let size = metadata.len();
        if size == self.original.size && metadata.modified().ok() == self.original.mtime {
            return false;
        }
        size <= self.original.size || !self.original.tail_intact()
    }

    /// 原文件只在末尾增长（如日志持续写入）时，只扫描新增的部分并接到索引之后。
    ///
    /// 返回新增的字符数。
    ///
    /// 以下情况返回 `None`，由调用方重新打开整个文件：文档已被修改（片段表不止一个原文件
    /// 片段，或新增缓冲非空）；文件没有变长；原有末尾的内容变了；新增部分不是合法 UTF-8。
    /// 末尾尚未写完的多字节字符留到下一次再接入。
    pub fn grow(&mut self) -> Option<u64> {
        let only_original = match self.pieces.as_slice() {
            // 没有片段：只有原文件本身为空时才视为未修改，否则说明内容已被全部删除。
            [] => self.original.len == 0,
            [p] => p.source == Source::Original && p.start == 0 && p.bytes == self.original.len,
            _ => false,
        };
        if !only_original || !self.added.is_empty() {
            return None;
        }
        let metadata = self.original.file.metadata().ok()?;
        let size = metadata.len();
        if size <= self.original.size {
            return None;
        }
        // 原有的末尾与建索引时记录的一致，才能认定只是在末尾追加：文件也可能被整个重写，
        // 此时行索引已不再对应，只能重新打开。
        let o = &self.original;
        if !o.tail_intact() {
            return None;
        }

        // 末尾尚未写完的多字节字符暂不接入，留到下一次。
        let mut tail = [0u8; 4];
        let tail_len = (size - o.base - o.len).min(4) as usize;
        let n = read_full(&o.file, size - tail_len as u64, &mut tail[..tail_len]).ok()?;
        let new_len = size - o.base - (n - complete_prefix(&tail[..n])) as u64;
        if new_len <= o.len {
            return None;
        }

        // 从最后一行的行首开始扫描，使最后一行的长度和检查点都能与原索引衔接。
        let (line_byte, line_char) = o.locate_line(o.newlines);
        let start = Checkpoint {
            line: o.newlines,
            byte: line_byte,
            char: line_char,
        };
        let mut segment = scan_segment(&o.file, o.base, line_byte, new_len).ok()?;
        if !segment.utf8 || segment.end < new_len {
            return None;
        }

        let added_chars = line_char + segment.chars - o.chars;
        let original = &mut self.original;
        // 接入新内容后内容末尾已经变化，重新记录末尾的 4 KiB。
        let check = new_len.min(4 << 10) as usize;
        let mut tail = vec![0; check];
        if read_full(
            &original.file,
            original.base + new_len - check as u64,
            &mut tail,
        )
        .is_ok()
        {
            original.tail = tail;
        }
        original.checkpoints.retain(|c| c.byte < line_byte);
        original.marks.retain(|m| m.byte < line_byte);
        segment.checkpoints[0] = Checkpoint {
            line: 0,
            byte: line_byte,
            char: 0,
        };
        let shift = |c: &Checkpoint| Checkpoint {
            line: start.line + c.line,
            byte: c.byte,
            char: start.char + c.char,
        };
        original
            .checkpoints
            .extend(segment.checkpoints.iter().map(shift));
        original.marks.extend(segment.marks.iter().map(shift));
        original.len = segment.end;
        original.chars = start.char + segment.chars;
        original.newlines = start.line + segment.newlines;
        original.longest_line = original.longest_line.max(segment.longest_line);
        // 原内容没有任何换行时，换行风格取决于新增的部分。
        if start.line == 0 {
            original.crlf = segment.first_crlf.unwrap_or(false);
        }
        // 记录的大小只到已接入的位置：尚未写完的字符留到下一次接入。
        original.size = original.base + segment.end;
        original.mtime = metadata.modified().ok();

        self.pieces = vec![Piece {
            source: Source::Original,
            start: 0,
            bytes: original.len,
            chars: original.chars,
            newlines: original.newlines,
            orig_char: 0,
            orig_line: 0,
        }];
        self.rebuild();
        Some(added_chars)
    }

    /// 返回 `from` 到 `to`（字符偏移）之间的字节数，用于复制之前判断是否超过上限。
    pub fn byte_len(&self, from: usize, to: usize) -> u64 {
        let (a, b) = (
            self.locate_char(from.min(to) as u64),
            self.locate_char(to as u64),
        );
        let mut total = 0;
        self.for_each_range(a, b, |_, start, end| total += end - start);
        total
    }

    /// 创建副本：与原文件共用文件句柄，按位置读取，互不影响；片段表和行索引各自独立。
    ///
    /// 后台保存时使用：界面线程上继续编辑原来的内容，后台写出副本。
    ///
    /// # Errors
    ///
    /// 复制文件句柄失败（打开的文件太多）时返回对应的 `std::io::Error`。
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            original: self.original.try_clone()?,
            added: self.added.clone(),
            pieces: self.pieces.clone(),
            chars_before: self.chars_before.clone(),
            lines_before: self.lines_before.clone(),
            line_cap: self.line_cap,
            last_line: Mutex::new(None),
        })
    }

    /// 把整个文本按原样（不含 BOM）写出。
    ///
    /// # Errors
    ///
    /// 写入失败（磁盘已满、管道关闭等）时返回对应的 `std::io::Error`。无法读取原文件（读取出错、
    /// 写出过程中被其他程序截短）时返回 [`io::ErrorKind::UnexpectedEof`]：写出的内容不完整，
    /// 调用方不能用它替换原文件。写完时原文件已被其他程序修改（`LargeText::original_changed`）
    /// 同样返回错误：写出的内容可能混有修改前后的字节，替换原文件还会丢失该程序的修改。
    pub fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        let mut result = Ok(());
        let mut written = 0u64;

        self.stream(
            Loc {
                piece: 0,
                offset: 0,
            },
            |chunk| match out.write_all(chunk) {
                Ok(()) => {
                    written += chunk.len() as u64;
                    ControlFlow::Continue(())
                }
                Err(e) => {
                    result = Err(e);
                    ControlFlow::Break(())
                }
            },
        );

        let expected: u64 = self.pieces.iter().map(|p| p.bytes).sum();
        if result.is_ok() && written != expected {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                t!("editor-large-original-truncated"),
            ));
        }
        if result.is_ok() && self.original_changed() {
            return Err(io::Error::other(t!("editor-original-changed")));
        }
        result
    }

    fn total_chars(&self) -> u64 {
        *self.chars_before.last().expect("至少有一项")
    }

    fn total_newlines(&self) -> u64 {
        *self.lines_before.last().expect("至少有一项")
    }

    /// 按片段表重建 `chars_before` 和 `lines_before`。
    fn rebuild(&mut self) {
        self.chars_before.clear();
        self.lines_before.clear();
        self.chars_before.push(0);
        self.lines_before.push(0);
        self.rebuild_from(0);
    }

    /// 从第 `i` 个片段起重算 `chars_before` 和 `lines_before`：前 `i` 个片段没有变，
    /// `chars_before[..i]` 等前缀和仍然有效，只重算之后的部分。
    fn rebuild_from(&mut self, i: usize) {
        *self
            .last_line
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner) = None;
        self.chars_before.truncate(i + 1);
        self.lines_before.truncate(i + 1);

        for p in &self.pieces[i..] {
            self.chars_before
                .push(self.chars_before.last().expect("至少有一项") + p.chars);
            self.lines_before
                .push(self.lines_before.last().expect("至少有一项") + p.newlines);
        }
    }

    fn added_bytes(&self, p: &Piece) -> &[u8] {
        &self.added.as_bytes()[p.start as usize..(p.start + p.bytes) as usize]
    }

    /// 返回包含第 `char` 个字符的片段的序号。
    ///
    /// `char` 必须小于总字符数。
    fn piece_at(&self, char: u64) -> usize {
        self.chars_before.partition_point(|&c| c <= char) - 1
    }

    /// 返回片段 `i` 中第 `c` 个字符的（字节偏移，该字符之前的换行数），偏移相对于片段起点。
    fn split_point(&self, i: usize, c: u64) -> (u64, u64) {
        self.cut_point(&self.pieces[i], c)
    }

    /// 返回片段 `p` 中第 `c` 个字符的（片段内字节偏移，之前的换行数）。
    fn cut_point(&self, p: &Piece, c: u64) -> (u64, u64) {
        match p.source {
            Source::Original => {
                let (byte, line) = self.original.locate_char(p.orig_char + c);
                (
                    byte.saturating_sub(p.start).min(p.bytes),
                    line.saturating_sub(p.orig_line),
                )
            }
            Source::Added => {
                let bytes = self.added_bytes(p);
                let b = byte_of_char(bytes, c as usize);
                (
                    b as u64,
                    memchr::memchr_iter(b'\n', &bytes[..b]).count() as u64,
                )
            }
        }
    }

    /// 返回第 `char` 个字符所在的位置；`char` 不小于总字符数时返回文本末尾。
    fn locate_char(&self, char: u64) -> Loc {
        if char >= self.total_chars() {
            return Loc {
                piece: self.pieces.len(),
                offset: 0,
            };
        }

        let i = self.piece_at(char);
        let (offset, _) = self.split_point(i, char - self.chars_before[i]);
        Loc { piece: i, offset }
    }

    /// 返回第 `line` 行（从 0 起）行首的（片段内位置，字符偏移）；`line` 超出时取最后一行。
    fn locate_line(&self, line: usize) -> (Loc, u64) {
        let line = (line as u64).min(self.total_newlines());

        if line == 0 {
            return (
                Loc {
                    piece: 0,
                    offset: 0,
                },
                0,
            );
        }

        // `i` 是包含第 `line` 个换行的片段，`k` 是该换行在片段中的序号（从 1 起）。
        let i = self.lines_before[1..].partition_point(|&l| l < line);
        let k = line - self.lines_before[i];
        let p = &self.pieces[i];

        let (offset, chars) = match p.source {
            Source::Original => {
                let (byte, char) = self.original.locate_line(p.orig_line + k);
                (
                    byte.saturating_sub(p.start),
                    char.saturating_sub(p.orig_char),
                )
            }
            Source::Added => {
                let bytes = self.added_bytes(p);
                let at = memchr::memchr_iter(b'\n', bytes)
                    .nth(k as usize - 1)
                    .expect("片段里有这么多换行");
                (at as u64 + 1, char_count(&bytes[..=at]))
            }
        };

        let loc = if offset >= p.bytes {
            Loc {
                piece: i + 1,
                offset: 0,
            }
        } else {
            Loc { piece: i, offset }
        };
        (loc, self.chars_before[i] + chars)
    }

    /// 从 `from` 开始按顺序把文本的字节交给 `f`，`f` 返回 `Break` 时停止。
    ///
    /// 每一块都在完整字符处结束。
    fn stream(&self, from: Loc, mut f: impl FnMut(&[u8]) -> ControlFlow<()>) {
        let mut buf = Vec::new();
        let mut size = FIRST_READ;

        for (i, p) in self.pieces.iter().enumerate().skip(from.piece) {
            let mut offset = if i == from.piece { from.offset } else { 0 };

            match p.source {
                Source::Added => {
                    let bytes = self.added_bytes(p);

                    if f(&bytes[offset as usize..]).is_break() {
                        return;
                    }
                }
                Source::Original => {
                    while offset < p.bytes {
                        let want = ((p.bytes - offset) as usize).min(size);

                        if buf.len() < want {
                            buf.resize(want, 0);
                        }

                        let chunk = self
                            .original
                            .read_aligned(p.start + offset, &mut buf[..want]);
                        let n = chunk.len();

                        // 原文件被截短，该片段剩下的部分无法读取。
                        if n == 0 {
                            break;
                        }

                        if f(chunk).is_break() {
                            return;
                        }

                        offset += n as u64;
                        size = (size * 2).min(CHUNK);
                    }
                }
            }
        }
    }

    /// 对 `a` 到 `b` 覆盖的每个片段调用 `f(片段, 起始字节, 结束字节)`，偏移相对于片段起点。
    fn for_each_range(&self, a: Loc, b: Loc, mut f: impl FnMut(&Piece, u64, u64)) {
        for i in a.piece..(b.piece + 1).min(self.pieces.len()) {
            let p = &self.pieces[i];
            let start = if i == a.piece { a.offset } else { 0 };
            let end = if i == b.piece { b.offset } else { p.bytes };

            if end > start {
                f(p, start, end);
            }
        }
    }

    /// 返回一行的字节（不含 `\n`，最多 `cap` 字节），以及是否读到了换行符。
    fn read_line(&self, line: usize, cap: usize) -> (Vec<u8>, bool) {
        let (loc, _) = self.locate_line(line);
        let mut bytes = Vec::new();
        let mut ended = false;

        self.stream(loc, |chunk| match memchr::memchr(b'\n', chunk) {
            Some(i) => {
                push_capped(&mut bytes, &chunk[..i], cap);
                ended = true;
                ControlFlow::Break(())
            }
            None => {
                push_capped(&mut bytes, chunk, cap);
                if bytes.len() >= cap {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }
        });

        (bytes, ended)
    }

    /// 单行可访问的字节数上限：存在超长行时为 `line_cap`，否则不限制。
    fn cap(&self) -> usize {
        if self.has_overlong_lines() {
            self.line_cap
        } else {
            usize::MAX
        }
    }

    /// 在第 `start` 个字符处切开其所在的片段，返回以该字符开头的片段的序号。
    ///
    /// `start` 不小于总字符数时不切开，返回片段数。
    fn split(&mut self, start: u64) -> usize {
        if start >= self.total_chars() {
            return self.pieces.len();
        }

        let i = self.piece_at(start);
        let c = start - self.chars_before[i];

        if c == 0 {
            return i;
        }

        let (left, right) = self.cut(self.pieces[i], c);
        self.pieces[i] = left;
        self.pieces.insert(i + 1, right);
        // 切开不改变总数：只在两半之间插入一项前缀和，之后的各项不变。
        self.chars_before
            .insert(i + 1, self.chars_before[i] + left.chars);
        self.lines_before
            .insert(i + 1, self.lines_before[i] + left.newlines);
        i + 1
    }

    /// 把片段 `p` 在第 `c` 个字符处切成两段。
    fn cut(&self, p: Piece, c: u64) -> (Piece, Piece) {
        let (offset, newlines) = self.cut_point(&p, c);
        let left = Piece {
            bytes: offset,
            chars: c,
            newlines,
            ..p
        };
        let right = Piece {
            start: p.start + offset,
            bytes: p.bytes - offset,
            chars: p.chars - c,
            newlines: p.newlines - newlines,
            orig_char: p.orig_char + c,
            orig_line: p.orig_line + newlines,
            ..p
        };
        (left, right)
    }

    /// 一次做完多处修改：`ranges` 是改动之前的（起点、终点、插入的文字），按位置排好、互不重叠。
    ///
    /// 一次遍历旧的片段表拼出新表，最后重算一次前缀和。
    fn replace_sorted(&mut self, ranges: &[(u64, u64, &str)]) {
        let added: Vec<Option<Piece>> = ranges
            .iter()
            .map(|&(_, _, text)| (!text.is_empty()).then(|| self.add(text)))
            .collect();
        let old = std::mem::take(&mut self.pieces);
        let mut out = Vec::with_capacity(old.len() + 2 * ranges.len());
        let mut rest = old.into_iter();
        // 旧表中尚未处理的部分：当前片段的剩余部分，及其在旧文字中的起点。
        let (mut cur, mut pos) = (rest.next(), 0u64);
        // 把旧文字中到 `to` 为止的部分移入新表（`keep` 为 `true`）或舍弃。
        let mut walk = |to: u64, keep: bool, out: &mut Vec<Piece>| {
            while let Some(p) = cur {
                if pos + p.chars <= to {
                    if keep {
                        out.push(p);
                    }
                    pos += p.chars;
                    cur = rest.next();
                } else {
                    if pos < to {
                        let (left, right) = self.cut(p, to - pos);
                        if keep {
                            out.push(left);
                        }
                        pos = to;
                        cur = Some(right);
                    }
                    break;
                }
            }
        };
        for (&(from, to, _), piece) in ranges.iter().zip(added) {
            walk(from, true, &mut out);
            walk(to, false, &mut out);
            out.extend(piece);
        }
        walk(u64::MAX, true, &mut out);
        self.pieces = out;
        self.rebuild();
    }

    /// 把 `text` 写入新增缓冲，返回指向这段文字的片段。
    fn add(&mut self, text: &str) -> Piece {
        let piece = Piece {
            source: Source::Added,
            start: self.added.len() as u64,
            bytes: text.len() as u64,
            chars: text.chars().count() as u64,
            newlines: memchr::memchr_iter(b'\n', text.as_bytes()).count() as u64,
            orig_char: 0,
            orig_line: 0,
        };
        self.added.push_str(text);
        piece
    }

    fn delete(&mut self, from: u64, to: u64) {
        let a = self.split(from);
        let b = self.split(to);
        self.pieces.drain(a..b);
        self.rebuild_from(a);
    }

    fn insert(&mut self, at: u64, text: &str) {
        let piece = self.add(text);

        // 连续输入：前一个片段正好位于新增缓冲末尾时，直接延长该片段，片段数不增加。
        if at > 0 {
            let i = self.piece_at(at - 1);
            let p = &mut self.pieces[i];

            if self.chars_before[i] + p.chars == at
                && p.source == Source::Added
                && p.start + p.bytes == piece.start
            {
                p.bytes += piece.bytes;
                p.chars += piece.chars;
                p.newlines += piece.newlines;
                self.rebuild_from(i);
                return;
            }
        }

        let i = self.split(at);
        self.pieces.insert(i, piece);
        self.rebuild_from(i);
    }
}

#[cfg(test)]
mod tests;
