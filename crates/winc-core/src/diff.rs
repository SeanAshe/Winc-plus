//! 两段文本之间的逐行差异，以及修改前的位置在修改后落在哪里。
//!
//! [`line_changes`] 把整段文本的替换（例如格式化的输出）转成按行划分、从后往前排列的
//! [`Change`] 列表，由 [`crate::editing::edit::Transaction`] 依次应用；[`map_positions`] 把修改前的
//! 位置映射到修改后。[`line_hunks`]、[`inline_changes`] 供并排的差异视图使用，[`hunk_key`]、
//! [`pick_changes`] 供按块提交使用。

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::ops::Range;

use crate::editing::edit::Change;
use crate::hash::fnv;

/// 逐行差分允许的最大差异行数（行）。
///
/// 中间不同的行数（删除的与新增的之和）超过该值时不再细分，首尾相同的行之间整段替换。
/// 回溯需要记录每一步的状态，内存占用与该值的平方成正比：取 1000 时约占 4 MB。
const MAX_EDITS: usize = 1000;
/// 差分累计工作量的上限（步）。
///
/// 每处理一条对角线计 1 步，每次相等行的比较再计 1 步。行数很多、差异又很大时
/// （例如整篇重排的大文件），超过该上限就整段替换，避免格式化的时间耗在差分上。该值为经验值。
const MAX_WORK: usize = 20_000_000;

/// 两侧不同的一段：`a` 的行区间被替换为 `b` 的行区间（行号从 0 起，左闭右开）。
type Hunk = (Range<usize>, Range<usize>);

/// 一段改动：旧文本里 `old` 这几行换成了新文本里 `new` 这几行（行号从 0 起，左闭右开）。
///
/// 只有一侧为空时是纯粹的新增（`old` 为空）或删除（`new` 为空）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineHunk {
    /// 旧文本中的行区间（行号从 0 起，左闭右开）。
    pub old: Range<usize>,
    /// 新文本中的行区间（行号从 0 起，左闭右开）。
    pub new: Range<usize>,
}

/// 返回从 `old` 到 `new` 的各段改动，按位置从前往后排列。
///
/// 两段文本相同时结果为空。行按内容连同换行符比较，只差换行符的行也算改动。差异太大、超出
/// 逐行差分的上限时，首尾相同的行之间整段算作一处改动。
pub fn line_hunks(old: &str, new: &str) -> Vec<LineHunk> {
    let (a, b) = (lines(old), lines(new));
    hunks(&a, &b)
        .into_iter()
        .map(|(old, new)| LineHunk { old, new })
        .collect()
}

/// 返回一段改动的旧内容 `old` 和新内容 `new` 之间按词比较的结果：两侧各自改动了的字符区间
/// （按字符计，从 0 起，左闭右开，按位置排列，相邻的区间已合并）。
///
/// 词是连续的字母、数字和下划线（含汉字）、连续的空白，或者单个其他字符。只差空白的地方
/// 也算改动；差异太大、超出差分上限时，整段算作改动。
pub fn inline_changes(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (a, b) = (words(old), words(new));
    let (ia, ib) = intern(
        &a.iter().map(|w| w.0).collect::<Vec<_>>(),
        &b.iter().map(|w| w.0).collect::<Vec<_>>(),
    );
    let Some(hunks) = diff(&ia, &ib) else {
        let all = |text: &str| {
            let n = text.chars().count();
            (n > 0).then_some(0..n).into_iter().collect()
        };
        return (all(old), all(new));
    };
    let span = |words: &[(&str, usize, usize)], range: Range<usize>| {
        if range.is_empty() {
            return None;
        }
        let start = words.get(range.start).map(|w| w.1);
        let end = range
            .end
            .checked_sub(1)
            .and_then(|i| words.get(i))
            .map(|w| w.2);
        start.zip(end).map(|(s, e)| s..e)
    };
    let mut left: Vec<Range<usize>> = Vec::new();
    let mut right: Vec<Range<usize>> = Vec::new();
    for (ra, rb) in hunks {
        for (out, range) in [(&mut left, span(&a, ra)), (&mut right, span(&b, rb))] {
            let Some(range) = range else { continue };
            match out.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => out.push(range),
            }
        }
    }
    (left, right)
}

/// 把文本切成词，每个词带着自己的字符区间（起点、终点）。
fn words(text: &str) -> Vec<(&str, usize, usize)> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut chars = text.char_indices().enumerate().peekable();
    while let Some((n, (byte, c))) = chars.next() {
        let kind = class(c);
        let (mut end_n, mut end_byte) = (n + 1, byte + c.len_utf8());
        if kind != Class::Other {
            while let Some(&(m, (b, d))) = chars.peek() {
                if class(d) != kind {
                    break;
                }
                (end_n, end_byte) = (m + 1, b + d.len_utf8());
                chars.next();
            }
        }
        out.push((&text[byte..end_byte], n, end_n));
    }
    out
}

/// 逐行比较 `a` 和 `b`，返回不同的行区间（行号在 `a`、`b` 中的绝对位置），按位置从前往后排列。
///
/// 先去掉首尾相同的行再差分；差异太大时中间整段算作一处。
fn hunks(a: &[&str], b: &[&str]) -> Vec<Hunk> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    if a_mid.is_empty() && b_mid.is_empty() {
        return Vec::new();
    }
    let (ia, ib) = intern(a_mid, b_mid);
    diff(&ia, &ib)
        .unwrap_or_else(|| vec![(0..a_mid.len(), 0..b_mid.len())])
        .into_iter()
        .map(|(ra, rb)| {
            (
                ra.start + prefix..ra.end + prefix,
                rb.start + prefix..rb.end + prefix,
            )
        })
        .collect()
}

/// 返回把 `old` 改成 `new` 的逐行修改，按位置从后往前排列。
///
/// 两段文本相同时结果为空。从后往前排列使 [`crate::editing::edit::Transaction`] 依次应用时，
/// 靠前的位置不受靠后的修改影响。
pub fn line_changes(old: &str, new: &str) -> Vec<Change> {
    let (a, b) = (lines(old), lines(new));
    let mut changes = Vec::new();
    let mut line = 0;
    let mut start = 0;
    for (ra, rb) in hunks(&a, &b) {
        start += a[line..ra.start]
            .iter()
            .map(|l| l.chars().count())
            .sum::<usize>();
        let removed = a[ra.clone()].concat();
        let removed_chars = removed.chars().count();
        changes.push(Change {
            start,
            removed,
            inserted: b[rb].concat(),
        });
        start += removed_chars;
        line = ra.end;
    }

    changes.reverse();
    changes
}

/// 返回 `positions` 中每个位置在应用 `changes`（[`line_changes`] 的结果）之后的位置。
///
/// 返回值与 `positions` 一一对应。位于未修改行的位置按之前增减的字符数平移；位于被修改行的
/// 位置先按去掉空白后相同的行对应，再按之前的非空白字符个数定位：格式化几乎只改动空白，光标
/// 因此仍停在同一个词的同一处。落在同一段改动内的多个位置（选区两端、窗口首行）共用一次计算。
///
/// # Panics
///
/// `changes` 中某一处的 `start` 加上 `removed` 的字符数超出 `usize` 的范围时 panic。
pub fn map_positions(changes: &[Change], positions: &[usize]) -> Vec<usize> {
    let mut mapped: Vec<Option<usize>> = vec![None; positions.len()];
    let mut delta = 0isize;

    for change in changes.iter().rev() {
        let removed = change.removed.chars().count();
        let end = change.start + removed;
        let mut alignment = None;
        for (pos, out) in positions.iter().zip(&mut mapped) {
            if out.is_some() || *pos >= end {
                continue;
            }
            *out = Some(if *pos < change.start {
                pos.saturating_add_signed(delta)
            } else {
                let alignment = alignment
                    .get_or_insert_with(|| Alignment::new(&change.removed, &change.inserted));
                change.start.saturating_add_signed(delta) + alignment.map(pos - change.start)
            });
        }
        delta += change.inserted.chars().count() as isize - removed as isize;
    }

    positions
        .iter()
        .zip(mapped)
        .map(|(pos, out)| out.unwrap_or_else(|| pos.saturating_add_signed(delta)))
        .collect()
}

/// 一段改动内新旧两侧的对应关系：去掉空白后相同的行一一对应，不同的行合成小段整体对应。
struct Alignment<'a> {
    /// 旧文本的各行，含换行符。
    old: Vec<&'a str>,
    /// 新文本的各行，含换行符。
    new: Vec<&'a str>,
    /// 新旧两侧不同的行区间。
    ///
    /// 末尾补一个空区间，使最后一个区间之后的相同行与前面的相同行走同一套处理逻辑。
    hunks: Vec<Hunk>,
}

impl<'a> Alignment<'a> {
    fn new(old: &'a str, new: &'a str) -> Self {
        let (old, new) = (lines(old), lines(new));
        // 只比较去掉空白后的内容，并用其 64 位 FNV-1a 哈希代替字符串：整篇重排时这一段可能是
        // 整个文件，用哈希可避免为每行分配字符串。哈希碰撞的后果只是光标位置略有偏差。
        let squash = |lines: &[&str]| -> Vec<u64> {
            lines
                .iter()
                .map(|l| fnv(l.bytes().filter(|b| !b.is_ascii_whitespace())))
                .collect()
        };
        let mut hunks = diff(&squash(&old), &squash(&new))
            .unwrap_or_else(|| vec![(0..old.len(), 0..new.len())]);
        hunks.push((old.len()..old.len(), new.len()..new.len()));
        Alignment { old, new, hunks }
    }

    /// 返回旧侧第 `offset` 个字符在新侧对应的位置（字符偏移，相对本段开头）。
    fn map(&self, offset: usize) -> usize {
        let (a, b) = (&self.old, &self.new);
        let chars = |lines: &[&str]| lines.iter().map(|l| l.chars().count()).sum::<usize>();
        let (mut ai, mut bi) = (0, 0);
        let (mut a_off, mut b_off) = (0, 0);
        for (ra, rb) in &self.hunks {
            for (old_line, new_line) in a[ai..ra.start].iter().zip(&b[bi..rb.start]) {
                let len = old_line.chars().count();
                if offset < a_off + len {
                    return b_off + by_content(old_line.chars(), new_line.chars(), offset - a_off);
                }
                a_off += len;
                b_off += new_line.chars().count();
            }

            let (old_part, new_part) = (&a[ra.clone()], &b[rb.clone()]);
            let len = chars(old_part);
            if offset < a_off + len {
                let (old, new) = (old_part.iter(), new_part.iter());
                let (old, new) = (old.flat_map(|l| l.chars()), new.flat_map(|l| l.chars()));
                return b_off + by_content(old, new, offset - a_off);
            }
            a_off += len;
            b_off += chars(new_part);
            (ai, bi) = (ra.end, rb.end);
        }

        b_off
    }
}

/// 返回 `old` 中第 `offset` 个字符在 `new` 中的对应位置，按该位置之前的非空白字符个数对应。
///
/// 紧邻字符的位置跟随该字符：位于词尾、行尾时停在对应字符之后，位于字符之前时停在对应字符
/// 之前。两侧都是空白时，同一行内的位置停在前一个字符之后；空行上的位置从上一个字符之后数过
/// 同样多个换行，目标行有字符时停在该字符之前，是空行时停在行首。
///
/// 两侧都按字符逐个读取，不收集成数组，因为一段改动可能是整个文件。
fn by_content(
    old: impl IntoIterator<Item = char>,
    new: impl IntoIterator<Item = char>,
    offset: usize,
) -> usize {
    let mut old = old.into_iter();
    // `offset` 之前的非空白字符数，最后一个非空白字符之后的换行数（`\r\n` 计为一个）。
    let (mut count, mut breaks) = (0, 0);
    let (mut any, mut after_word, mut cr) = (false, false, false);
    for c in old.by_ref().take(offset) {
        after_word = !c.is_whitespace();
        match c {
            '\n' => {
                breaks += 1;
                cr = false;
            }
            c if c.is_whitespace() => {
                breaks += usize::from(cr);
                cr = c == '\r';
            }
            _ => {
                (count, breaks, any, cr) = (count + 1, 0, true, false);
            }
        }
    }
    breaks += usize::from(cr);
    let before_word = old.next().is_some_and(|c| !c.is_whitespace());

    let mut new = new.into_iter().peekable();
    let (mut i, mut seen) = (0, 0);
    // 读过的最后两个字符：`new` 的非空白字符不够时，停在末尾的换行符之前。
    let mut tail = ['\0'; 2];
    while seen < count {
        let Some(c) = new.next() else {
            return match tail {
                ['\r', '\n'] => i - 2,
                [_, '\n' | '\r'] => i - 1,
                _ => i,
            };
        };
        seen += usize::from(!c.is_whitespace());
        i += 1;
        tail = [tail[1], c];
    }
    if after_word || (!before_word && any && breaks == 0) {
        return i;
    }
    if before_word {
        while new.peek().is_some_and(|c| c.is_whitespace()) {
            new.next();
            i += 1;
        }
        return i;
    }

    let mut remaining = breaks;
    while remaining > 0 {
        match new.peek() {
            None => break,
            Some('\r') => {
                new.next();
                i += 1;
                if new.next_if_eq(&'\n').is_some() {
                    i += 1;
                }
                remaining -= 1;
            }
            Some('\n') => {
                new.next();
                i += 1;
                remaining -= 1;
            }
            Some(c) if c.is_whitespace() => {
                new.next();
                i += 1;
            }
            // 新文本中的空行比旧文本少：停在下一个非空白字符之前。
            Some(_) => return i,
        }
    }
    let line_start = i;
    while new
        .peek()
        .is_some_and(|c| c.is_whitespace() && !matches!(c, '\n' | '\r'))
    {
        new.next();
        i += 1;
    }
    if new.peek().is_some_and(|c| !c.is_whitespace()) {
        i
    } else {
        line_start
    }
}

/// 按内容返回一段改动的键：包含同样几行（都带换行符）的旧侧和新侧得到相同的键。
///
/// 重新比较以后行号会变，内容不变的改动仍然可以认出。键不写入磁盘，只在同一个进程内比较。
pub fn hunk_key<'a>(
    old: impl IntoIterator<Item = &'a str>,
    new: impl IntoIterator<Item = &'a str>,
) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    for line in old {
        hasher.write(line.as_bytes());
    }
    // 分开两侧：「a」换成「」和「」换成「a」不能视为同一处改动。
    hasher.write_u8(0xff);
    for line in new {
        hasher.write(line.as_bytes());
    }
    hasher.finish()
}

/// 一段改动中不应用的行：行号相对这段改动的开头（从 0 起）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineSet {
    /// 保留的旧行，即不删除的行。
    pub old: BTreeSet<usize>,
    /// 不加入的新行。
    pub new: BTreeSet<usize>,
}

/// [`pick_changes`] 对一段改动的处理方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// 应用整段改动。
    Apply,
    /// 不应用这段改动。
    Skip,
    /// 除 [`LineSet`] 中的行以外都应用。
    Lines(LineSet),
}

/// 返回从 `old` 出发、按 `pick` 应用 `new` 相对它的各段改动之后得到的文本。
///
/// `pick` 的参数是这段改动的 [`hunk_key`] 和它在 `old` 里的行区间。全部应用时结果是 `new`，全部
/// 不应用时结果是 `old`。按行应用时，保留的旧行排在加入的新行之前。
pub fn pick_changes(old: &str, new: &str, pick: impl Fn(u64, Range<usize>) -> Pick) -> String {
    let (a, b) = (lines(old), lines(new));
    let mut out = String::with_capacity(new.len().max(old.len()));
    let mut at = 0;
    for (o, n) in hunks(&a, &b) {
        for line in &a[at..o.start] {
            out.push_str(line);
        }
        let key = hunk_key(a[o.clone()].iter().copied(), b[n.clone()].iter().copied());
        match pick(key, o.clone()) {
            Pick::Apply => b[n].iter().for_each(|line| out.push_str(line)),
            Pick::Skip => a[o.clone()].iter().for_each(|line| out.push_str(line)),
            Pick::Lines(skipped) => {
                let kept = (a[o.clone()].iter().enumerate())
                    .filter(|(i, _)| skipped.old.contains(i))
                    .map(|(_, line)| *line);
                let added = (b[n].iter().enumerate())
                    .filter(|(i, _)| !skipped.new.contains(i))
                    .map(|(_, line)| *line);
                for line in kept.chain(added) {
                    // 文件末尾没有换行符的旧行后面接上新行时，两行不能连在一起。
                    if !out.is_empty() && !out.ends_with(['\n', '\r']) {
                        out.push_str(if line.ends_with("\r\n") { "\r\n" } else { "\n" });
                    }
                    out.push_str(line);
                }
            }
        }
        at = o.end;
    }
    for line in &a[at..] {
        out.push_str(line);
    }
    out
}

/// 返回从 `old` 到 `new` 的各段改动里已经在 `staged` 里的部分：每段改动的 [`hunk_key`] 和按
/// [`pick_changes`] 的写法表示的处理方式。
///
/// 用于只暂存了一部分的文件：`old` 是上一次提交里的版本，`new` 是工作区，`staged` 是暂存区。
/// 旧行在 `staged` 中仍然存在时视为未删除，新行在 `staged` 中存在时视为已加入。按这些处理方式
/// 应用到 `old` 上，结果通常等于 `staged`；暂存之后又修改过同一处时无法精确还原，结果是与
/// `staged` 最接近的组合。
pub fn staged_picks(old: &str, new: &str, staged: &str) -> Vec<(u64, Pick)> {
    let (a, b, s) = (lines(old), lines(new), lines(staged));
    let removed: BTreeSet<usize> = hunks(&a, &s).into_iter().flat_map(|(o, _)| o).collect();
    let unstaged: BTreeSet<usize> = hunks(&b, &s).into_iter().flat_map(|(n, _)| n).collect();
    hunks(&a, &b)
        .into_iter()
        .map(|(o, n)| {
            let key = hunk_key(a[o.clone()].iter().copied(), b[n.clone()].iter().copied());
            let set = LineSet {
                old: (o.clone().filter(|i| !removed.contains(i)))
                    .map(|i| i - o.start)
                    .collect(),
                new: (n.clone().filter(|j| unstaged.contains(j)))
                    .map(|j| j - n.start)
                    .collect(),
            };
            let pick = if set.old.is_empty() && set.new.is_empty() {
                Pick::Apply
            } else if set.old.len() == o.len() && set.new.len() == n.len() {
                Pick::Skip
            } else {
                Pick::Lines(set)
            };
            (key, pick)
        })
        .collect()
}

/// 按行切分文本，每行保留自己的换行符（`\n`、`\r\n` 或单独的 `\r`）。
///
/// 最后一行没有换行符时也算一行，以换行符结尾时不多出一个空行；行号与 [`line_hunks`] 一致。
/// 返回的每行内容包含自己的换行符，需要不含换行符的行时由调用方去掉。
pub fn lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let end = match bytes[i] {
            b'\n' => i + 1,
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => i + 2,
            b'\r' => i + 1,
            _ => {
                i += 1;
                continue;
            }
        };
        lines.push(&text[start..end]);
        start = end;
        i = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// 把两侧的条目替换为编号，内容相同的条目编号相同，后续比较只需比较整数。
fn intern<T: Hash + Eq>(a: &[T], b: &[T]) -> (Vec<u32>, Vec<u32>) {
    let mut ids: HashMap<&T, u32> = HashMap::new();
    let mut id = |line| {
        let next = ids.len() as u32;
        *ids.entry(line).or_insert(next)
    };
    let ia = a.iter().map(&mut id).collect();
    let ib = b.iter().map(&mut id).collect();
    (ia, ib)
}

/// 用 Myers 差分算法求 `a` 与 `b` 不同的区间，按位置从前往后排列。
///
/// 差异行数超过 [`MAX_EDITS`] 或工作量超过 [`MAX_WORK`] 时返回 `None`，由调用方整段替换。
fn diff<T: PartialEq>(a: &[T], b: &[T]) -> Option<Vec<Hunk>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let limit = (a.len() + b.len()).min(MAX_EDITS) as isize;
    let offset = limit + 1;
    let mut v = vec![0isize; 2 * limit as usize + 3];
    // 依次保存第 `d` 步之后各对角线 `k`（-d 至 d）的终点 `x`，第 `d` 步的数据从下标 `d²` 开始。
    let mut trace: Vec<i32> = Vec::new();
    let mut work = 0;
    let mut found = None;

    for d in 0..=limit {
        for k in (-d..=d).step_by(2) {
            let index = (k + offset) as usize;
            let down = k == -d || (k != d && v[index - 1] < v[index + 1]);
            let start = if down { v[index + 1] } else { v[index - 1] + 1 };
            let (mut x, mut y) = (start, start - k);
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            work += (x - start) as usize + 1;
            v[index] = x;
            if x >= n && y >= m {
                found = Some(d);
            }
        }
        trace.extend((-d..=d).map(|k| v[(k + offset) as usize] as i32));
        if found.is_some() {
            break;
        }
        if work > MAX_WORK {
            return None;
        }
    }
    let edits = found?;

    // 从终点回溯，每一步对应删除 `a` 的一行或插入 `b` 的一行。
    let mut steps = Vec::with_capacity(edits as usize);
    let (mut x, mut y) = (n, m);
    for d in (1..=edits).rev() {
        let previous = |k: isize| trace[((d - 1) * (d - 1) + k + d - 1) as usize] as isize;
        let k = x - y;
        let down = k == -d || (k != d && previous(k - 1) < previous(k + 1));
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = previous(prev_k);
        let prev_y = prev_x - prev_k;
        steps.push((prev_x as usize, prev_y as usize, down));
        (x, y) = (prev_x, prev_y);
    }

    let mut hunks: Vec<Hunk> = Vec::new();
    for (x, y, insert) in steps.into_iter().rev() {
        let (end_a, end_b) = if insert { (x, y + 1) } else { (x + 1, y) };
        match hunks.last_mut() {
            Some((ra, rb)) if ra.end == x && rb.end == y => {
                ra.end = end_a;
                rb.end = end_b;
            }
            _ => hunks.push((x..end_a, y..end_b)),
        }
    }
    Some(hunks)
}

#[cfg(test)]
mod tests;
