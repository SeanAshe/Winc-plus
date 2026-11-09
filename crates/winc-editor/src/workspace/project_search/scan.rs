//! 读磁盘上的文件：按块读、解码并逐行匹配，不把整个文件读进内存。

use super::*;

/// 读磁盘文件并搜索，最多收 `limit` 处，`cancel` 置位时停止；二进制或无法读取的文件返回 `None`，
/// 返回值的第二项表示是否有字节按 U+FFFD 替换字符解码。
///
/// 判断规则与打开文件时相同，因此搜出的行号和列与打开后一致：开头 `BINARY_SNIFF` 字节含 NUL
/// 的文件是二进制；不是 UTF-8 时整个读入内存并识别编码；带 BOM 的 UTF-16 在 `UTF16_INLINE`
/// 以内同样整篇解码，超过的不搜索。其余大文件按需从磁盘读取，只按 `\n` 分行，非 UTF-8 的字节
/// 替换为 U+FFFD。
pub(super) fn search_disk(
    matcher: &Matcher,
    buffer: &mut Vec<u8>,
    path: &Path,
    limit: usize,
    cancel: &AtomicBool,
) -> Option<(Vec<LineMatch>, bool)> {
    let mut file = fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size > OPEN_LIMIT {
        return None;
    }
    let large = size > LARGE_FILE;

    // 先读第一块，长度至少为 `BINARY_SNIFF`，用于判断是否为二进制。
    buffer.clear();
    let mut left = size;
    read_chunk(&mut file, buffer, &mut left, CHUNK.max(BINARY_SNIFF)).ok()?;
    // 带 BOM 的 UTF-16 在 `UTF16_INLINE` 以内整篇解码，与打开文件相同。
    let inline_utf16 = utf16_bom(buffer) && size <= UTF16_INLINE;
    if looks_binary(buffer) || (large && utf16_bom(buffer) && !inline_utf16) {
        return None;
    }

    // 跨行查找需要整个文件的内容；按需读取的大文件不支持跨行查找，与文件内查找相同。
    // 整篇解码的 UTF-16 除外，它的行号与打开后的文本一致。
    if matcher.is_multiline() {
        if large && !inline_utf16 {
            return None;
        }
        let mut bytes = std::mem::take(buffer);
        bytes.try_reserve_exact(size as usize + 1).ok()?;
        file.read_to_end(&mut bytes).ok()?;
        let decoded = decode(&bytes);
        let (text, starts) = joined_text(&decoded.text);
        let result = (
            joined_matches(matcher, &text, &starts, limit),
            decoded.lossy(),
        );
        drop(decoded);
        *buffer = bytes;
        buffer.clear();
        if buffer.capacity() > BUFFER_MAX {
            *buffer = Vec::new();
        }
        return Some(result);
    }

    let mut search = Stream {
        matcher,
        limit,
        matches: Vec::new(),
        lossy: false,
        index: 0,
    };
    let streamed = if utf16_bom(buffer) {
        None
    } else {
        search.run(buffer, &mut file, left, large, cancel)
    };
    let result = match streamed {
        Some(()) => Some((search.matches, search.lossy)),
        // 不是 UTF-8：整个读入内存并识别编码，从头重新搜索。超过 `LARGE_FILE` 的只有
        // `UTF16_INLINE` 以内的 UTF-16 走这里，更大的文件已在上面跳过。
        None if !large || inline_utf16 => {
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size as usize + 1).ok()?;
            file.rewind().ok()?;
            file.read_to_end(&mut bytes).ok()?;
            let decoded = decode(&bytes);
            let mut matches = Vec::new();
            let _ = for_each_line(&decoded.text, 0, true, |index, line| {
                collect(matcher, index, line, &mut matches, limit)
            });
            Some((matches, decoded.lossy()))
        }
        None => None,
    };

    // 超长行使读缓冲的容量超过 `BUFFER_MAX`。搜索完当前文件后释放缓冲，避免每个工作线程长期
    // 持有大块内存。
    if buffer.capacity() > BUFFER_MAX {
        *buffer = Vec::new();
    }
    result
}

/// 按块读取并搜索一个文件的状态。
struct Stream<'a> {
    matcher: &'a Matcher,
    limit: usize,
    matches: Vec<LineMatch>,
    /// 是否有非 UTF-8 的字节被替换为 U+FFFD（只有大文件会这样）。
    lossy: bool,
    /// 下一行的行号（从 0 起）。
    index: usize,
}

impl Stream<'_> {
    /// 搜索整个文件；调用前 `buffer` 已含文件开头的一段，文件还有 `left` 字节没读。
    ///
    /// 按 rope 分行的文件遇到非 UTF-8 的内容时返回 `None`，由调用方整个读入后重新搜索。
    fn run(
        &mut self,
        buffer: &mut Vec<u8>,
        file: &mut fs::File,
        mut left: u64,
        large: bool,
        cancel: &AtomicBool,
    ) -> Option<()> {
        // 待搜索内容在 `buffer` 中的起点（字节偏移）；BOM 不算内容。
        let mut start = if buffer.starts_with(&UTF8_BOM) {
            UTF8_BOM.len()
        } else {
            0
        };
        // 大文件中超过 `LINE_CAP` 的行只搜前面一段（打开后也只能看到这一段），其余部分跳到
        // 换行符之后。
        let mut skipping = false;
        // `buffer[start..scanned]` 内不含 `\n`。
        let mut scanned = start;

        loop {
            let eof = left == 0;
            let data = &buffer[start..];

            // 只搜完整的行：到最后一个 `\n` 为止，其后的半行留到下一块；文件读完时搜全部。
            // 留下的半行不含 `\n`，下一块只在新读入的部分里找 `\n`，因此很长的行不会每块
            // 都从头扫描。
            let end = if eof {
                data.len()
            } else {
                match memchr::memrchr(b'\n', &buffer[scanned..]) {
                    Some(at) => scanned - start + at + 1,
                    None if large && data.len() > LINE_CAP => {
                        if !skipping {
                            let cut = visible_len(data, false, LINE_CAP);
                            if self.line_lossy(&data[..cut]).is_break() {
                                return Some(());
                            }
                            skipping = true;
                        }
                        buffer.clear();
                        (start, scanned) = (0, 0);
                        read_chunk(file, buffer, &mut left, CHUNK).ok()?;
                        continue;
                    }
                    None => {
                        scanned = buffer.len();
                        read_chunk(file, buffer, &mut left, CHUNK).ok()?;
                        continue;
                    }
                }
            };
            let mut lines = &data[..end];
            if skipping {
                // 跳过超长行的剩余部分：文件没读完时这里一定有换行符；没有换行符说明该行一直
                // 延伸到文件末尾。
                match memchr::memchr(b'\n', lines) {
                    Some(at) => lines = &lines[at + 1..],
                    None => return Some(()),
                }
                skipping = false;
            }

            // 先在整块中查找候选位置（`Matcher::block_find`），只对含候选的行逐行匹配，其余行只
            // 统计行数：大多数块里没有任何匹配。整块查找把 `\n`、`\r\n`、`\r` 都当作换行，而大
            // 文件只按 `\n` 分行、行内单独的 `\r` 是内容，含这种 `\r` 的块不能用整块查找。
            let flow = if self.matcher.has_block()
                && let Ok(text) = std::str::from_utf8(lines)
                && !(large && has_lone_cr(lines))
            {
                self.candidates(text, large, eof)
            } else if large {
                self.large_lines(lines, eof)
            } else {
                let text = std::str::from_utf8(lines).ok()?;
                let (matcher, limit, matches) = (self.matcher, self.limit, &mut self.matches);
                for_each_line(text, self.index, eof, |index, line| {
                    collect(matcher, index, line, matches, limit)
                })
            };
            match flow {
                ControlFlow::Continue(next) => self.index = next,
                ControlFlow::Break(()) => return Some(()),
            }
            if eof || cancel.load(Relaxed) {
                return Some(());
            }

            // 丢弃已搜索的部分，把剩下的半行移到缓冲开头，再读下一块。
            buffer.drain(..start + end);
            (start, scanned) = (0, buffer.len());
            read_chunk(file, buffer, &mut left, CHUNK).ok()?;
        }
    }

    /// 搜索大文件的若干行：只按 `\n` 分行，与打开后看到的文本一致（去掉行尾的 `\r`，截去
    /// 超长部分）。
    ///
    /// `last` 为 `true` 时，最后一个换行符之后的内容也算一行。
    fn large_lines(&mut self, lines: &[u8], last: bool) -> ControlFlow<(), usize> {
        let mut from = 0;
        for end in memchr_iter(b'\n', lines) {
            let line = &lines[from..end];
            self.line_lossy(&line[..visible_len(line, true, LINE_CAP)])?;
            from = end + 1;
        }
        if last {
            let line = &lines[from..];
            self.line_lossy(&line[..visible_len(line, false, LINE_CAP)])?;
        }
        ControlFlow::Continue(self.index)
    }

    /// 在整块文本中查找候选位置，只对候选所在的行逐行匹配，中间的行只统计行数。
    ///
    /// `text` 以换行符结尾（`last` 为 `true` 时不一定），换行规则同 [`Self::run`]。一行内的
    /// 匹配在整块文本中同样会成为候选，因此从 `pos` 起第一个候选之前的行内一定没有匹配。
    fn candidates(&mut self, text: &str, large: bool, last: bool) -> ControlFlow<(), usize> {
        let bytes = text.as_bytes();
        // `pos` 之后的文本尚未检查，且总是位于一行的开头。
        let mut pos = 0;

        while let Some(mut at) = self.matcher.block_find(text, pos) {
            if at == bytes.len() && !last {
                break;
            }
            // 候选所在行的三个位置：行首、内容结尾、下一行行首。
            let (start, end, next) = if large {
                let start = memchr::memrchr(b'\n', &bytes[pos..at]).map_or(pos, |i| pos + i + 1);
                let end = memchr::memchr(b'\n', &bytes[start..]).map_or(bytes.len(), |i| start + i);
                let raw = &bytes[start..end];
                (
                    start,
                    start + visible_len(raw, end < bytes.len(), LINE_CAP),
                    end + 1,
                )
            } else {
                // 候选从 `\r\n` 的 `\n` 开始时，归入以 `\r` 结尾的那一行。
                if at > pos && bytes.get(at) == Some(&b'\n') && bytes[at - 1] == b'\r' {
                    at -= 1;
                }
                let start =
                    memchr::memrchr2(b'\n', b'\r', &bytes[pos..at]).map_or(pos, |i| pos + i + 1);
                let end = memchr::memchr2(b'\n', b'\r', &bytes[start..])
                    .map_or(bytes.len(), |i| start + i);
                let crlf = bytes[end..].starts_with(b"\r\n");
                (start, end, end + 1 + usize::from(crlf))
            };
            self.index += count_breaks(&bytes[pos..start]);
            let index = self.index;
            self.index += 1;
            collect(
                self.matcher,
                index,
                &text[start..end],
                &mut self.matches,
                self.limit,
            )?;
            if next > bytes.len() {
                // 这是最后一行（`last`），没有下一行。
                return ControlFlow::Continue(self.index);
            }
            pos = next;
        }

        self.index += count_breaks(&bytes[pos..]) + usize::from(last);
        ControlFlow::Continue(self.index)
    }

    /// 搜索大文件的一行：非 UTF-8 的字节替换为 U+FFFD。
    fn line_lossy(&mut self, line: &[u8]) -> ControlFlow<(), ()> {
        let text = String::from_utf8_lossy(line);
        self.lossy |= matches!(text, std::borrow::Cow::Owned(_));
        let index = self.index;
        self.index += 1;
        collect(self.matcher, index, &text, &mut self.matches, self.limit)
    }
}

/// 再读最多 `max` 字节，追加到 `buffer` 末尾。
///
/// `left` 是文件还剩多少字节没读（从打开时的大小算起，因此不必多读一次才知道读完）；文件在
/// 此期间变短、读不到内容时也视为读完。
///
/// # Errors
///
/// 读取失败（被信号中断的情况除外，会重试）时返回 `std::io::Error`，`buffer` 恢复到调用前的长度。
fn read_chunk(
    file: &mut fs::File,
    buffer: &mut Vec<u8>,
    left: &mut u64,
    max: usize,
) -> std::io::Result<()> {
    let want = (*left).min(max as u64) as usize;
    let len = buffer.len();
    buffer.resize(len + want, 0);
    let read = loop {
        match file.read(&mut buffer[len..]) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => break other,
        }
    };
    let read = read.inspect_err(|_| buffer.truncate(len))?;
    buffer.truncate(len + read);
    *left = if read == 0 { 0 } else { *left - read as u64 };
    Ok(())
}

/// 是否含有后面不跟 `\n` 的 `\r`。
fn has_lone_cr(bytes: &[u8]) -> bool {
    memchr_iter(b'\r', bytes).any(|i| bytes.get(i + 1) != Some(&b'\n'))
}

/// 返回按 rope 规则分行时的换行数：`\n`、`\r\n`、单独的 `\r`（与 [`for_each_line`] 相同）。
fn count_breaks(bytes: &[u8]) -> usize {
    if memchr::memchr(b'\r', bytes).is_none() {
        return memchr_iter(b'\n', bytes).count();
    }
    memchr2_iter(b'\n', b'\r', bytes)
        .filter(|&i| !(bytes[i] == b'\n' && i > 0 && bytes[i - 1] == b'\r'))
        .count()
}
