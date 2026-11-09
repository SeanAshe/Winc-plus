//! 大文件文本按行读取和编辑：`Lines`、`EditBuffer` 的实现。

use super::*;

impl Lines for LargeText {
    fn line_count(&self) -> usize {
        self.total_newlines() as usize + 1
    }

    fn line_start(&self, line: usize) -> usize {
        self.locate_line(line).1 as usize
    }

    fn line_of(&self, pos: usize) -> usize {
        let pos = pos as u64;

        if pos >= self.total_chars() {
            return self.total_newlines() as usize;
        }

        let i = self.piece_at(pos);
        let (_, newlines) = self.split_point(i, pos - self.chars_before[i]);
        (self.lines_before[i] + newlines) as usize
    }

    fn line_content(&self, line: usize) -> Line<'_> {
        Line::Owned(self.with_line(line, |text, _| text.to_owned()))
    }

    fn line_prefix(&self, line: usize, max_bytes: usize) -> Line<'_> {
        let cap = self.cap().min(max_bytes);
        let (mut bytes, ended) = self.read_line(line, cap);
        bytes.truncate(visible_len(&bytes, ended, cap));
        Line::Owned(
            String::from_utf8(bytes)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        )
    }

    fn line_len(&self, line: usize) -> usize {
        self.with_line(line, |_, chars| chars)
    }

    fn substring(&self, from: usize, to: usize) -> String {
        let (a, b) = (
            self.locate_char(from.min(to) as u64),
            self.locate_char(to as u64),
        );
        let mut bytes = Vec::new();

        self.for_each_range(a, b, |p, start, end| match p.source {
            Source::Added => {
                bytes.extend_from_slice(&self.added_bytes(p)[start as usize..end as usize])
            }
            Source::Original => {
                let old = bytes.len();
                bytes.resize(old + (end - start) as usize, 0);
                let n = self.original.read(p.start + start, &mut bytes[old..]);
                bytes.truncate(old + n);
            }
        });

        String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    fn end_pos(&self) -> usize {
        self.total_chars() as usize
    }

    fn visit_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        let cap = self.cap().min(max_bytes);
        self.visit_capped(from, cap, false, |line, start, content| {
            f(line, start, content.unwrap_or_default())
        })
    }

    fn visit_short_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    ) -> Option<usize> {
        // 多取 4 字节（UTF-8 字符的最大长度）：超过 `max_bytes` 字节的行要么取满上限、不交出
        // 内容，要么交出的内容长于 `max_bytes`。
        let cap = self.cap().min(max_bytes.saturating_add(4));
        self.visit_capped(from, cap, true, |line, start, content| {
            f(line, start, content.filter(|text| text.len() <= max_bytes))
        })
    }
}

/// [`LargeText::visit_run`] 结束的原因。
enum Run {
    /// `f` 在这一行返回了 `Break`。
    Stopped(usize),
    /// 已交出最后一行。
    Ended,
    /// 这一行已取满上限并交出，其余部分没有读取。
    Skipped(usize),
}

impl LargeText {
    /// 从第 `from` 行起依次把每一行交给 `f`，每行最多取 `cap` 字节，含义同
    /// [`Lines::visit_lines`]；`skip` 为 `true` 时取满 `cap` 字节的行不解码，以 `None` 交出。
    fn visit_capped(
        &self,
        from: usize,
        cap: usize,
        skip: bool,
        mut f: impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    ) -> Option<usize> {
        if from >= self.line_count() {
            return None;
        }

        let mut line = from;
        let last = self.total_newlines() as usize;
        // 每一轮从第 `line` 行的行首顺序读取；某一行已取满 `cap` 字节时交出该行，下一轮按索引
        // 定位到下一行，不再读取该行的剩余部分。页缓存命中时实测：每行约 1 MB 的 SQL 文件中，
        // 一屏 45 行整行读取约 27 ms，读到上限即停约 2 ms。
        loop {
            // 不交出内容的超长行无需读取：其字节数由相邻两行的行首位置得出。超长行通常连续出现，
            // 因此在首行和每个超长行之后检查；检查普通行的开销不超过一次行首定位。
            while skip && line < last {
                let (here, start) = self.locate_line(line);
                let (next, _) = self.locate_line(line + 1);
                if self.byte_distance(here, next) <= cap as u64 {
                    break;
                }
                if f(line, start as usize, None).is_break() {
                    return Some(line);
                }
                line += 1;
            }
            match self.visit_run(line, cap, skip, &mut f) {
                Run::Stopped(at) => return Some(at),
                Run::Ended => return None,
                Run::Skipped(at) if at >= last => return None,
                Run::Skipped(at) => line = at + 1,
            }
        }
    }

    /// 返回位置 `a` 到其后的位置 `b` 之间的字节数。
    fn byte_distance(&self, a: Loc, b: Loc) -> u64 {
        let whole: u64 = self.pieces[a.piece..b.piece].iter().map(|p| p.bytes).sum();
        (whole + b.offset).saturating_sub(a.offset)
    }

    /// 从第 `from` 行起顺序读取并依次交给 `f`，直到 `f` 返回 `Break`、读到末尾，或某一行取满
    /// `cap` 字节；各行的含义同 [`LargeText::visit_capped`]。
    fn visit_run(
        &self,
        from: usize,
        cap: usize,
        skip: bool,
        f: &mut impl FnMut(usize, usize, Option<&str>) -> ControlFlow<()>,
    ) -> Run {
        let (loc, start) = self.locate_line(from);
        let (mut line, mut start) = (from, start as usize);
        // 跨越读取块的行先拼接到 `carry`；整行落在同一块内时直接借用，不复制。
        let (mut carry, mut scratch) = (Vec::new(), String::new());
        let mut result = None;

        self.stream(loc, |chunk| {
            // 每块都在完整字符处结束。整块是合法 UTF-8 时，按换行切出的每一段也合法，只需校验一次。
            // 块内没有换行时整块属于同一个超长行，交出的内容另行校验，不必校验整块。
            let valid = memchr::memchr(b'\n', chunk).and_then(|_| std::str::from_utf8(chunk).ok());
            let count = |bytes: &[u8]| {
                if valid.is_some() {
                    count_valid(bytes)
                } else {
                    char_count(bytes)
                }
            };
            let mut at = 0;

            while let Some(i) = memchr::memchr(b'\n', &chunk[at..]).map(|i| at + i) {
                // 字符数按整行统计才能得到准确的行首位置；交给 `f` 的内容只保留前 `cap` 字节。
                // `carry` 不足 `cap` 字节，含有这一行此前的全部字节，在行尾一并统计；
                // 取满 `cap` 字节的行由索引给出下一行的行首，不必统计。
                let chars = count(&chunk[at..i]) as usize
                    + if carry.is_empty() {
                        0
                    } else {
                        char_count(&carry) as usize
                    };

                let flow = if carry.is_empty() {
                    emit_line(
                        f,
                        (line, start),
                        &chunk[at..i],
                        valid.map(|t| &t[at..i]),
                        true,
                        cap,
                        &mut scratch,
                    )
                } else {
                    push_capped(&mut carry, &chunk[at..i], cap);
                    let flow = emit_line(f, (line, start), &carry, None, true, cap, &mut scratch);
                    carry.clear();
                    flow
                };

                if flow.is_break() {
                    result = Some(Run::Stopped(line));
                    return ControlFlow::Break(());
                }

                (line, start) = (line + 1, start + chars + 1);
                at = i + 1;
            }

            push_capped(&mut carry, &chunk[at..], cap);
            if carry.len() >= cap {
                // 后面的字节不会交给 `f`，下一行的行首改由索引给出。
                let flow = if skip {
                    f(line, start, None)
                } else {
                    emit_line(f, (line, start), &carry, None, false, cap, &mut scratch)
                };
                result = Some(if flow.is_break() {
                    Run::Stopped(line)
                } else {
                    Run::Skipped(line)
                });
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });

        if let Some(result) = result {
            return result;
        }

        // 最后一行后面没有换行符，单独交给 `f`。
        if emit_line(f, (line, start), &carry, None, false, cap, &mut scratch).is_break() {
            Run::Stopped(line)
        } else {
            Run::Ended
        }
    }

    /// 用第 `line` 行的文字和字符数调用 `f`；该行不是最近读出的行时，先从磁盘读出、解码并缓存。
    fn with_line<R>(&self, line: usize, f: impl FnOnce(&str, usize) -> R) -> R {
        let mut last = self
            .last_line
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_, text, chars) = match &mut *last {
            Some(cached) if cached.0 == line => cached,
            slot => {
                let cap = self.cap();
                let (mut bytes, ended) = self.read_line(line, cap);
                bytes.truncate(visible_len(&bytes, ended, cap));
                // 合法的 UTF-8（绝大多数行）直接把读到的字节用作文字，无需再复制。
                let text = String::from_utf8(bytes)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
                let chars = text.chars().count();
                slot.insert((line, text, chars))
            }
        };
        f(text, *chars)
    }
}

impl EditBuffer for LargeText {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        if removed > 0 {
            self.delete(start as u64, (start + removed) as u64);
        }

        if !inserted.is_empty() {
            self.insert(start as u64, inserted);
        }
    }

    /// 一次完成多处修改：逐处修改时每处都要在片段表中间插入并重算其后的前缀和，数千处的
    /// 「全部替换」因此耗时为平方级。
    ///
    /// 各处按位置排列（全部替换、多光标输入从前往后，撤销从后往前）时，先换算为改动之前的
    /// 位置，再遍历一次片段表拼出新表；顺序不规则时逐处修改。
    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        match sorted_ranges(changes) {
            Some(ranges) if ranges.len() > 1 => self.replace_sorted(&ranges),
            _ => {
                for &(start, removed, inserted) in changes {
                    self.replace(start, removed, inserted);
                }
            }
        }
    }
}

/// 把依次修改的 `changes` 换算成改动之前的位置（起点、终点、插入的文字），按位置从前往后排列。
///
/// 各处既不是从前往后、也不是从后往前互不重叠地排列时返回 `None`。
fn sorted_ranges<'a>(changes: &[(usize, usize, &'a str)]) -> Option<Vec<(u64, u64, &'a str)>> {
    let len = |s: &str| s.chars().count();
    let ascending = changes.windows(2).all(|w| w[1].0 >= w[0].0 + len(w[0].2));
    if ascending {
        // 修改后的位置减去前面各处修改使文字增加的长度，即为改动之前的位置。
        let mut delta = 0i64;
        let ranges = changes
            .iter()
            .map(|&(start, removed, inserted)| {
                let from = (start as i64 - delta) as u64;
                delta += len(inserted) as i64 - removed as i64;
                (from, from + removed as u64, inserted)
            })
            .collect();
        return Some(ranges);
    }
    // 从后往前排列时，每处修改都位于之前各处的前面，位置不受其影响。
    let descending = changes.windows(2).all(|w| w[1].0 + w[1].1 <= w[0].0);
    descending.then(|| {
        (changes.iter().rev())
            .map(|&(start, removed, inserted)| (start as u64, (start + removed) as u64, inserted))
            .collect()
    })
}
