//! 工作区文件里的合并冲突：认出 `git` 合并、变基、拣选冲突时写入的 `<<<<<<<`、`|||||||`
//! （`merge.conflictStyle=diff3` 时才有）、`=======`、`>>>>>>>` 标记。
//!
//! 按用户选的当前、对方、两者都要算出替换整段（连同标记行）的文本；替换是文档里的普通修改。

use std::ops::Range;
use std::path::Path;

use ropey::{Rope, RopeSlice};
use winc_core::diff::{LineHunk, line_hunks};

/// 冲突两侧名字最多保留的长度（`char` 数）。
///
/// 名字一般是分支名或提交号，长度有限；截断过长的名字，避免把整行复制进内存。
const LABEL_LIMIT: usize = 192;

/// 文件里的一段冲突，行号从 0 起。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// `<<<<<<<` 那一行。
    pub start: usize,
    /// `|||||||` 那一行（共同祖先的版本从下一行开始）；没有时为 `None`。
    pub base: Option<usize>,
    /// `=======` 那一行。
    pub middle: usize,
    /// `>>>>>>>` 那一行。
    pub end: usize,
    /// 当前这一侧的名字（`<<<<<<< HEAD` 里的 `HEAD`）。
    pub ours_label: String,
    /// 对方的名字（`>>>>>>> feature` 里的 `feature`）。
    pub theirs_label: String,
}

impl Conflict {
    /// 整段，连同四条（或三条）标记行。
    pub fn lines(&self) -> Range<usize> {
        self.start..self.end + 1
    }

    /// 当前这一侧的行。
    pub fn ours(&self) -> Range<usize> {
        self.start + 1..self.base.unwrap_or(self.middle)
    }

    /// 对方的行。
    pub fn theirs(&self) -> Range<usize> {
        self.middle + 1..self.end
    }

    /// `line` 是不是这一段的标记行。
    pub fn is_marker(&self, line: usize) -> bool {
        line == self.start || line == self.middle || line == self.end || Some(line) == self.base
    }
}

/// 解决一段冲突时选哪边。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    /// 只要当前这一侧。
    Ours,
    /// 只要对方这一侧。
    Theirs,
    /// 两者都要，当前的在前。
    Both,
}

impl Pick {
    /// 按钮上的文字。
    pub fn label(self) -> &'static str {
        match self {
            Pick::Ours => winc_i18n::t!("git-conflict-accept-current"),
            Pick::Theirs => winc_i18n::t!("git-conflict-accept-incoming"),
            Pick::Both => winc_i18n::t!("git-conflict-accept-both"),
        }
    }
}

/// 文件 `path` 中是否仍有成对的冲突标记；无法读取（已删除、是目录）时视为没有。
///
/// 先按块读取查找 `<<<<<<<`，找到时才读成 rope 逐段确认，合法的 UTF-8 不保留整个文件的
/// 字节副本。
pub fn file_has_conflicts(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut opened) = winc_backend::fs::open(path, 0) else {
        return false;
    };
    let Ok(mut file) = opened.reader() else {
        return false;
    };
    let finder = memchr::memmem::Finder::new(OPEN);
    let mut block = vec![0; 64 << 10];
    // 块开头留着上一块末尾的 `OPEN.len() - 1` 个字节，跨块的标记也能找到。
    let mut kept = 0;
    let found = loop {
        let n = match file.read(&mut block[kept..]) {
            Ok(0) => break false,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        let filled = kept + n;
        if finder.find(&block[..filled]).is_some() {
            break true;
        }
        kept = filled.min(OPEN.len() - 1);
        block.copy_within(filled - kept..filled, 0);
    };
    drop(file);
    if !found {
        return false;
    }
    // 重新取读取器，从文件开头再读一遍。
    let Ok(file) = opened.reader() else {
        return false;
    };
    let text = match Rope::from_reader(std::io::BufReader::new(file)) {
        Ok(text) => text,
        Err(_) => {
            let Ok(bytes) = winc_backend::fs::read(path) else {
                return false;
            };
            Rope::from_str(&String::from_utf8_lossy(&bytes))
        }
    };
    !find_conflicts(&text).is_empty()
}

/// 返回 `text` 中按位置排列的各段冲突；标记不成对（例如只有 `<<<<<<<`）的不计入。
///
/// 每次修改都要调用：先在 rope 的各块上找 `<<<<<<<`，没有就没有冲突。有标记时同样用 memmem
/// 找出四种标记的位置，只确认落在行首的行；逐行检查每行要查一次 rope 的树，标记行却只占极少数。
pub fn find_conflicts(text: &Rope) -> Vec<Conflict> {
    let mut found = Vec::new();
    let Some(first) = first_open_marker(text) else {
        return found;
    };
    let first_line = text.byte_to_line(first);
    // 正在读的一段：`<<<<<<<` 的行号和名字，读到的 `|||||||`、`=======`。
    let mut open: Option<(usize, String, Option<usize>, Option<usize>)> = None;
    for i in marker_lines(text, first) {
        if i < first_line {
            continue;
        }
        // 只有真的标记行才复制名字。
        match marker(text.line(i)) {
            Some(('<', label)) => open = Some((i, label_text(label), None, None)),
            Some(('|', _)) => {
                if let Some((_, _, base @ None, None)) = open.as_mut() {
                    *base = Some(i);
                }
            }
            Some(('=', _)) => {
                if let Some((_, _, _, middle @ None)) = open.as_mut() {
                    *middle = Some(i);
                }
            }
            Some(('>', label)) => {
                if let Some((start, ours, base, Some(middle))) = open.take() {
                    found.push(Conflict {
                        start,
                        base,
                        middle,
                        end: i,
                        ours_label: ours,
                        theirs_label: label_text(label),
                    });
                }
            }
            _ => {}
        }
    }
    found
}

/// 冲突开头的标记。
const OPEN: &[u8] = b"<<<<<<<";

/// 返回字节偏移 `from` 起、行首是七个同样的标记符号（`<`、`|`、`=`、`>`）的行号，升序、
/// 不重复。
///
/// 在 rope 的各块上用 memmem 找四种标记，跨块的标记靠上一块末尾留下的几个字节接上；标记行
/// 后面的内容是否合乎格式由 [`marker`] 判断。
fn marker_lines(text: &Rope, from: usize) -> Vec<usize> {
    const KEEP: usize = OPEN.len() - 1;
    let finders = [b"<<<<<<<", b"|||||||", b"=======", b">>>>>>>"].map(memchr::memmem::Finder::new);
    let mut lines = Vec::new();
    let mut check = |at: usize| {
        let line = text.byte_to_line(at);
        if text.line_to_byte(line) == at {
            lines.push(line);
        }
    };
    let (chunks, start, _, _) = text.chunks_at_byte(from);
    let mut start = start;
    // `window` 是上一块末尾的 `KEEP` 个字节接上这一块：其中的标记要么跨块，要么就在这一块里。
    let (mut tail, mut window): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
    for chunk in chunks {
        let bytes = chunk.as_bytes();
        window.clear();
        window.extend_from_slice(&tail);
        window.extend_from_slice(bytes);
        let base = start - tail.len();
        for finder in &finders {
            for i in finder.find_iter(&window) {
                check(base + i);
            }
        }
        start += bytes.len();
        tail.extend_from_slice(bytes);
        let excess = tail.len().saturating_sub(KEEP);
        tail.drain(..excess);
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

/// 返回 `text` 里第一个 `<<<<<<<` 的字节偏移（不管在不在行首）；没有时返回 `None`。
///
/// 标记跨块时，靠上一块末尾留下的 `OPEN.len() - 1` 个字节接上。
fn first_open_marker(text: &Rope) -> Option<usize> {
    const KEEP: usize = OPEN.len() - 1;
    let finder = memchr::memmem::Finder::new(OPEN);
    // `window` 是此前读过的最后 `KEEP` 个字节加上这一块的开头 `KEEP` 个字节，跨块的标记在这里找。
    let (mut tail, mut window) = (Vec::with_capacity(KEEP), Vec::with_capacity(2 * KEEP));
    let mut start = 0;
    for chunk in text.chunks() {
        let bytes = chunk.as_bytes();
        window.clear();
        window.extend_from_slice(&tail);
        window.extend_from_slice(&bytes[..bytes.len().min(KEEP)]);
        if let Some(i) = finder.find(&window) {
            return Some(start - tail.len() + i);
        }
        if let Some(i) = finder.find(bytes) {
            return Some(start + i);
        }
        start += bytes.len();
        // 留下到目前为止最后 `KEEP` 个字节（这一块短于 `KEEP` 时还带着上一块的）。
        tail.extend_from_slice(bytes);
        let excess = tail.len().saturating_sub(KEEP);
        tail.drain(..excess);
    }
    None
}

/// `line` 是标记行时返回（符号，后面的名字，不含换行符）。
///
/// 标记是行首七个同样的符号（`<`、`|`、`=`、`>`），后面要么没有内容，要么是一个空格加名字；
/// `=======` 后面不能有内容。名字末尾的换行符不计入。
fn marker(line: RopeSlice<'_>) -> Option<(char, RopeSlice<'_>)> {
    let first = line.get_char(0)?;
    if !matches!(first, '<' | '|' | '=' | '>')
        || line.len_chars() < 7
        || !line.chars().take(7).all(|c| c == first)
    {
        return None;
    }
    let mut end = line.len_chars();
    while end > 7 && matches!(line.char(end - 1), '\n' | '\r') {
        end -= 1;
    }
    let rest = line.slice(7..end);
    match rest.get_char(0) {
        None => Some((first, rest)),
        Some(' ') if first != '=' => Some((first, rest.slice(1..))),
        _ => None,
    }
}

/// 标记行上的名字：去掉首尾空白，最多 [`LABEL_LIMIT`] 个字符。
fn label_text(label: RopeSlice<'_>) -> String {
    let label: String = label.chars().take(LABEL_LIMIT).collect();
    label.trim().to_owned()
}

/// 按 `pick` 解决 `conflict` 时换上去的文本，各行保留原来的换行符。
pub fn resolution(text: &Rope, conflict: &Conflict, pick: Pick) -> String {
    let take = |range: Range<usize>, out: &mut String| {
        for line in range {
            out.extend(text.line(line).chunks());
        }
    };
    let mut out = String::new();
    match pick {
        Pick::Ours => take(conflict.ours(), &mut out),
        Pick::Theirs => take(conflict.theirs(), &mut out),
        Pick::Both => {
            take(conflict.ours(), &mut out);
            // 当前那一段的最后一行没有换行符（位于文件末尾）时补一个，避免两段连成一行。
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            take(conflict.theirs(), &mut out);
        }
    }
    out
}

/// 文件和一侧版本之间对应的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignSegment {
    /// 文件中的行区间；这一侧有、文件中没有时是插入位置上的空区间。
    pub file: Range<usize>,
    /// 这一侧的行区间；文件中有、这一侧没有时是插入位置上的空区间。
    pub side: Range<usize>,
    /// 第几段冲突；`git` 已自动合并的改动为 `None`。
    pub conflict: Option<usize>,
}

/// 一段冲突在 [`SideAlign`] 中的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AlignedConflict {
    /// 在文件中的行区间，连同标记行。
    lines: Range<usize>,
    /// 在解决后的文本中的开头行号。
    start: usize,
    /// 在解决后的文本中的行数。
    length: usize,
}

/// 冲突文件和其中一侧的完整版本（暂存阶段 `:2:`、`:3:`）之间的行对应关系。
///
/// 把各段冲突按这一侧解决得到「解决后的文本」，再与这一侧的版本逐行比较；两者不同的行是 `git`
/// 从另一侧自动合入的改动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideAlign {
    conflicts: Vec<AlignedConflict>,
    /// 解决后的文本相对这一侧的改动：`old` 是解决后的文本的行，`new` 是这一侧的行。
    hunks: Vec<LineHunk>,
    /// 各段冲突在这一侧的行区间，下标同传入的冲突。
    pub blocks: Vec<Range<usize>>,
    /// 要连接的各段，创建时算好：绘制每帧都要用。
    segments: Vec<AlignSegment>,
}

impl SideAlign {
    /// 计算文件 `text`（其中的冲突为 `conflicts`）与 `pick` 这一侧的版本 `side` 的对应关系。
    pub fn new(text: &Rope, conflicts: &[Conflict], side: &str, pick: Pick) -> SideAlign {
        let mut resolved = String::new();
        let mut aligned = Vec::with_capacity(conflicts.len());
        let (mut at, mut count) = (0, 0);
        let copy = |range: Range<usize>, out: &mut String| {
            for line in range {
                out.extend(text.line(line).chunks());
            }
        };
        for conflict in conflicts {
            copy(at..conflict.start, &mut resolved);
            count += conflict.start - at;
            let length = match pick {
                Pick::Ours => conflict.ours().len(),
                Pick::Theirs => conflict.theirs().len(),
                Pick::Both => conflict.ours().len() + conflict.theirs().len(),
            };
            resolved.push_str(&resolution(text, conflict, pick));
            aligned.push(AlignedConflict {
                lines: conflict.lines(),
                start: count,
                length,
            });
            count += length;
            at = conflict.end + 1;
        }
        copy(at..text.len_lines(), &mut resolved);
        let mut align = SideAlign {
            conflicts: aligned,
            hunks: line_hunks(&resolved, side),
            blocks: Vec::new(),
            segments: Vec::new(),
        };
        align.blocks = (align.conflicts.iter())
            .map(|c| {
                let begin = align.resolved_to_side(c.start);
                begin..align.resolved_to_side(c.start + c.length).max(begin)
            })
            .collect();
        align.segments = align.build_segments();
        align
    }

    /// 返回这一侧中对应文件第 `line` 行的行号；冲突内的行对应冲突在这一侧的开头。
    pub fn side_line(&self, line: usize) -> usize {
        let mut resolved = line;
        for (conflict, block) in self.conflicts.iter().zip(&self.blocks) {
            if line < conflict.lines.start {
                break;
            }
            if line < conflict.lines.end {
                return block.start;
            }
            resolved = conflict.start + conflict.length + (line - conflict.lines.end);
        }
        self.resolved_to_side(resolved)
    }

    /// 返回这一侧中对应解决后的文本第 `line` 行的行号。
    fn resolved_to_side(&self, line: usize) -> usize {
        let mut shift = 0isize;
        for hunk in &self.hunks {
            if line < hunk.old.start {
                break;
            }
            if line < hunk.old.end {
                return hunk.new.start + (line - hunk.old.start).min(hunk.new.len());
            }
            shift = hunk.new.end as isize - hunk.old.end as isize;
        }
        line.saturating_add_signed(shift)
    }

    /// 返回这一侧与文件之间要连接的各段，按位置排列：各段冲突和 `git` 已自动合并的改动。
    pub fn segments(&self) -> &[AlignSegment] {
        &self.segments
    }

    /// 计算 [`SideAlign::segments`] 的结果。
    fn build_segments(&self) -> Vec<AlignSegment> {
        let mut segments: Vec<AlignSegment> = (self.conflicts.iter().zip(&self.blocks).enumerate())
            .map(|(i, (conflict, block))| AlignSegment {
                file: conflict.lines.clone(),
                side: block.clone(),
                conflict: Some(i),
            })
            .collect();
        for hunk in &self.hunks {
            // 冲突本身已经单独列出，其中的差别不再算作自动合并的改动。
            let overlaps = (self.conflicts.iter())
                .any(|c| hunk.old.start < c.start + c.length && c.start < hunk.old.end);
            if overlaps {
                continue;
            }
            segments.push(AlignSegment {
                file: self.resolved_to_file(hunk.old.start)..self.resolved_to_file(hunk.old.end),
                side: hunk.new.clone(),
                conflict: None,
            });
        }
        segments.sort_by_key(|segment| segment.file.start);
        segments
    }

    /// 返回文件中对应解决后的文本第 `line` 行的行号。
    fn resolved_to_file(&self, line: usize) -> usize {
        let mut shift = 0;
        for conflict in &self.conflicts {
            if line < conflict.start + conflict.length {
                break;
            }
            shift = conflict.lines.end - (conflict.start + conflict.length);
        }
        line + shift
    }
}

#[cfg(test)]
mod tests;
