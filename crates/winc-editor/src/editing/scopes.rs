//! 代码块识别：粘滞滚动据此把已滚出屏幕顶部的函数、类、`if` 固定在顶部，折叠据此把块折叠为一行。
//!
//! 不依赖语言服务，按缩进判断哪些行属于同一个块，Markdown 按标题分节。粘滞滚动只在需要的位置
//! 向上、向下各查看有限的行，不为整个文件建立索引，因此大文件中的输入不会变慢；折叠命令一次扫描
//! 整个文件（[`fold_ranges`]）。多行字符串、块注释中的行不参与缩进判断：调用方用 `quoted` 说明
//! 某一行的行首是否位于字符串或注释内。

use ropey::RopeSlice;
use winc_core::Lines;
use winc_core::text;
use winc_syntax::Language;

/// 查找所在块时向上最多查看的行数（行）。
///
/// 取自调参：覆盖常见长度的函数，同时限制大文件中向上扫描的行数。
const SCAN_UP: usize = 10_000;
/// 查找块的末尾时向下最多查看的行数（行）。
///
/// 取自调参：只有块的末尾接近屏幕顶部时才需要查找末尾，取较小的值以减少向下扫描的行数。
pub(crate) const SCAN_DOWN: usize = 64;

/// 一个块，从 `start` 行开始，到 `end` 行结束（`end` 行含在块内，块末尾的空行不计）。
///
/// 块的开头可以不止一行：`void f()` 下一行单独的 `{`，以及 `fn f(` 到 `) -> T {` 之间
/// 多行书写的参数列表，都从第一行算起。固定显示的是这些开头行中的第一行，该行滚到
/// 屏幕顶部后保持不动，直到整个块滚出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// 块的第一行（行号从 0 起），粘滞滚动固定显示该行。
    pub start: usize,
    /// 块的最后一行（行号从 0 起）；向下 `SCAN_DOWN`（64）行内未找到块的末尾时为 `None`。
    pub end: Option<usize>,
    /// 屏幕行序列（[`sticky`] 的 `rows`）中，属于该块的最后一行的下标（从 0 起）；
    /// `rows` 的每一行都在块内时为 `None`。
    ///
    /// 固定显示的行会被块最后一行之后的屏幕行推出顶部。存在折叠时，即使块的最后一行距离较远，
    /// 屏幕上紧随其后的行也可能已在块外。
    pub last_row: Option<usize>,
}

/// 某一行外层的块，以及推算另一行的块时复用的中间状态。
///
/// 粘滞滚动每帧要查看屏幕顶部的数行，逐行向上查看上万行的代价过大：下方的行基于上方行的
/// 结果继续计算，向下滚动时还复用上一帧的结果，只有向上滚动或跳转到其他位置时才从头查看。
#[derive(Debug, Clone)]
pub struct Ancestry {
    /// 被查看的行号（从 0 起）。
    line: usize,
    /// 按缩进划分：包含该行的各块的开头行及其缩进（列），从外到内；开头行可能重复，尚未去重。
    levels: Vec<(Scope, usize)>,
    /// 从该行向上查找时使用的缩进（列）：缩进比 `threshold` 浅的行才是外层块的开头。
    threshold: usize,
    /// Markdown：该行之前仍未结束的标题，每项为（标题级别，行号）。
    open: Vec<(usize, usize)>,
    /// Markdown：该行之前是否处于代码块围栏内。
    fenced: bool,
}

/// 返回编辑区自上而下显示 `rows` 中的行（行号从 0 起）时固定在顶部的块，从外到内，最多 `max` 个。
///
/// 第 `d` 个固定的块包含行 `rows[d]`，开头位于该行上方。没有折叠时 `rows` 是连续的行，
/// 需要比 `max` 多一行才能判断最内层的块是否即将结束；`tab_width` 是 Tab 的显示宽度
/// （显示列）。`partial` 表示已滚动到行中间，该行本身是块的开头时也固定显示；`memo`
/// 复用上一次在同一文档上记录的结果。
#[allow(clippy::too_many_arguments)]
pub fn sticky(
    text: &impl Lines,
    language: &Language,
    rows: &[usize],
    partial: bool,
    max: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
    memo: &mut Vec<Ancestry>,
) -> Vec<Scope> {
    let rows: Vec<usize> = rows
        .iter()
        .copied()
        .take_while(|&l| l < text.line_count())
        .collect();
    // 之后判断哪些行属于同一个块时需要逐行的结果，因此先记录每一行的 `Ancestry`；
    // 每行的计算从上方最近一个已计算的行继续。
    let mut known = std::mem::take(memo);
    let mut around: Vec<Vec<Scope>> = Vec::with_capacity(rows.len());
    for &line in &rows {
        let hint = known
            .iter()
            .filter(|a| a.line <= line)
            .max_by_key(|a| a.line);
        let ancestry = ancestry(text, language, line, tab_width, quoted, hint);
        around.push(enclosing(text, language, &ancestry, tab_width, quoted));
        known.push(ancestry);
    }
    known.sort_by_key(|a| a.line);
    known.dedup_by_key(|a| a.line);
    let keep = known.len().saturating_sub(MEMO);
    *memo = known.split_off(keep);

    let inside = |i: usize, scope: &Scope| {
        rows[i] == scope.start || around[i].iter().any(|a| a.start == scope.start)
    };

    let mut stack: Vec<Scope> = Vec::new();
    while stack.len() < max {
        let d = stack.len();
        let Some(&probe) = rows.get(d) else {
            break;
        };
        let mut here = around[d].clone();
        if partial
            && here.len() == d
            && let Some(own) = own_scope(text, language, probe, tab_width, quoted)
            && own.start == probe
            && here.last().is_none_or(|outer| outer.start != own.start)
        {
            here.push(own);
        }
        let same_outer = here.iter().zip(&stack).all(|(a, b)| a.start == b.start);
        let Some(&scope) = here.get(d).filter(|_| same_outer) else {
            break;
        };
        let end = scope_end(text, language, scope.start, probe, tab_width, quoted);
        let last = (d..rows.len())
            .take_while(|&i| inside(i, &scope))
            .last()
            .unwrap_or(d);
        let last_row = (last + 1 < rows.len()).then_some(last);
        stack.push(Scope {
            end,
            last_row,
            ..scope
        });
    }
    stack
}

/// 复用的上一次计算结果最多保留的行数（行）。
///
/// 需容纳屏幕顶部的数行，以及向下滚动时新滚入顶部的数行。取自调参：16 行覆盖常见窗口高度。
const MEMO: usize = 16;

/// 返回包含 `line` 的块：`line` 是块的开头，或是块开头后半部分所在的行时返回该块；
/// 否则返回 `None`。
///
/// 开头的后半部分指 `void f()` 下一行单独的 `{`，以及多行参数列表收尾的 `) -> T {`。
fn own_scope(
    text: &impl Lines,
    language: &Language,
    line: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> Option<Scope> {
    let header = |at: usize| {
        let start = header_start(text, at, tab_width);
        Scope {
            start,
            end: None,
            last_row: None,
        }
    };
    if starts_block(text, language, line, tab_width, quoted) {
        return Some(header(line));
    }
    let next = line + 1;
    let brace_below = next < text.line_count()
        && head(text, next).slice().to_string().trim() == "{"
        && header_start(text, next, tab_width) == line
        && starts_block(text, language, next, tab_width, quoted);
    brace_below.then(|| header(next))
}

/// 一份文本上记录的 [`Ancestry`]，按文本的版本号和 Tab 宽度失效。
///
/// 编辑区的文档与差异的两侧各持有一份，粘滞滚动和光标避让都从这里复用已有的结果。
#[derive(Debug, Clone, Default)]
pub(crate) struct ScopeMemo {
    /// 记录时文本的版本号和 Tab 宽度（显示列）。
    key: (u64, usize),
    /// 记录的结果，按行号排序。
    found: Vec<Ancestry>,
}

impl ScopeMemo {
    /// 返回版本 `revision`、Tab 宽度 `tab_width` 下可以复用并继续记录的结果；条件不同时先清空。
    pub(crate) fn records(&mut self, revision: u64, tab_width: usize) -> &mut Vec<Ancestry> {
        if self.key != (revision, tab_width) {
            self.key = (revision, tab_width);
            self.found.clear();
        }
        &mut self.found
    }

    /// 返回版本 `revision`、Tab 宽度 `tab_width` 下可以复用的结果；条件不同时为空。
    pub(crate) fn get(&self, revision: u64, tab_width: usize) -> &[Ancestry] {
        if self.key == (revision, tab_width) {
            &self.found
        } else {
            &[]
        }
    }

    /// 是否没有记录任何结果。
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    /// 丢弃本次记录的结果：高亮尚未推算到位时，字符串中的行可能被当作代码。
    pub(crate) fn discard(&mut self) {
        self.found.clear();
    }

    /// 文本从第 `line` 行起改动、版本号由 `revision` 加 1 时，保留改动上方仍然适用的结果。
    ///
    /// 在屏幕下方输入时，粘滞滚动无需从头向上查找。
    pub(crate) fn edited(&mut self, revision: u64, line: usize) {
        if self.key.0 == revision {
            self.found.retain(|a| a.survives_edit_at(line));
            self.key.0 += 1;
        }
    }
}

impl Ancestry {
    /// 是否第 `line` 行起的改动不影响该结果。
    ///
    /// 结果只取决于该行向下 [`SCAN_DOWN`] 行以内的文字和上方的文字，行号也不变，因此改动
    /// 位于该范围之下时结果仍然有效。
    pub(crate) fn survives_edit_at(&self, line: usize) -> bool {
        self.line + SCAN_DOWN < line
    }
}

/// 返回 `line` 外层的块数，即外层块固定显示在顶部时占用的行数。
///
/// 光标移到被固定行遮住的位置时，需要多滚动相应行数。`memo` 是 [`sticky`] 记录的结果，
/// 从上方最近一行已计算的结果继续计算，无需向上查看 `SCAN_UP` 行。
pub fn depth(
    text: &impl Lines,
    language: &Language,
    line: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
    memo: &[Ancestry],
) -> usize {
    let hint = memo
        .iter()
        .filter(|a| a.line <= line)
        .max_by_key(|a| a.line);
    let ancestry = ancestry(text, language, line, tab_width, quoted, hint);
    enclosing(text, language, &ancestry, tab_width, quoted).len()
}

/// 返回包含 `ancestry` 所对应行的块，从外到内。
///
/// 包括缩进比该行浅的各层（Markdown 为该行所在的各节），以及该行位于块开头的后半部分
/// （`{`、`) -> T {` 等行）时该行所属的块。
fn enclosing(
    text: &impl Lines,
    language: &Language,
    ancestry: &Ancestry,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> Vec<Scope> {
    let line = ancestry.line;
    let mut found = if language.id == "markdown" {
        headings_around(text, ancestry)
    } else {
        let mut found: Vec<Scope> = ancestry.levels.iter().map(|&(scope, _)| scope).collect();
        // 内层块的开头行可能与外层块相同（`append(` 下一行是 `{`）：同一行不重复固定，
        // 保留外层的块，其范围包含内层。
        found.dedup_by(|inner, outer| inner.start == outer.start);
        found
    };
    if let Some(own) = own_scope(text, language, line, tab_width, quoted)
        && own.start < line
        && found.last().is_none_or(|outer| outer.start != own.start)
    {
        found.push(own);
    }
    found
}

/// 向上查找 `line` 外层的块。
///
/// `hint` 是上方某一行已计算的结果：从 `line` 向上查找到该行即停止，更上层的块直接取自 `hint`。
/// 没有 `hint` 时最多向上查看 `SCAN_UP` 行。
fn ancestry(
    text: &impl Lines,
    language: &Language,
    line: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
    hint: Option<&Ancestry>,
) -> Ancestry {
    if language.id == "markdown" {
        return sections(text, line, hint);
    }

    // 空行归入其下方第一个非空行所在的块：块中间的空行属于该块，块末尾的空行不属于。
    let below = (line..text.line_count().min(line + SCAN_DOWN))
        .find(|&l| !text::is_blank(head(text, l).slice()) && !quoted(l));
    let threshold = below.map_or(0, |b| indent_of(text, b, tab_width));
    let mut indent = threshold;
    let mut found = Vec::new();
    let mut at = line;
    let stop = line.saturating_sub(SCAN_UP);
    let mut inherited = Vec::new();
    while indent > 0 && at > stop {
        // 到达已计算过的行：该行向上查找时的缩进阈值不低于当前阈值，因此该行记录的缩进更浅的
        // 各层即为所需的结果。
        if let Some(h) = hint
            && h.line == at
            && indent <= h.threshold
        {
            inherited = h
                .levels
                .iter()
                .copied()
                .filter(|&(_, i)| i < indent)
                .collect();
            break;
        }
        at -= 1;
        let content = head(text, at);
        let slice = content.slice();
        if text::is_blank(slice) {
            continue;
        }
        let own = text::indent_cells(slice, tab_width);
        if own >= indent {
            continue;
        }
        let trimmed = slice.to_string();
        let trimmed = trimmed.trim();
        if is_comment(trimmed, language) || quoted(at) {
            continue;
        }
        let start = header_start(text, at, tab_width);
        found.push((
            Scope {
                start,
                end: None,
                last_row: None,
            },
            own,
        ));
        indent = own;
    }
    found.reverse();
    inherited.extend(found);
    Ancestry {
        line,
        levels: inherited,
        threshold,
        open: Vec::new(),
        fenced: false,
    }
}

/// 是否为注释或预处理指令；这类行不是块的开头。
///
/// 语言的行注释符，以及以 `#`、`*` 或块注释起始符开头的行都按注释处理。Python 的 `#` 注释、
/// C 的 `#ifdef` 常顶格写在缩进的代码中间。
fn is_comment(trimmed: &str, language: &Language) -> bool {
    let line_comment = language
        .line_comments
        .iter()
        .any(|c| trimmed.starts_with(c));
    line_comment || trimmed.starts_with(['#', '*']) || trimmed.starts_with("/*")
}

/// 是否为紧接在块开头之后的续行：单独的 `{`，或以 `)`、`]` 开头（参数列表的收尾）。
fn continues_header(trimmed: &str) -> bool {
    trimmed == "{" || trimmed.starts_with([')', ']'])
}

/// 返回块的开头所在的行号（从 0 起）；`line` 是缩进开始变深的行。
///
/// 开头各行的缩进相同，其间可以有缩进更深的续行（如 `for pos in brace` 下一行的 `.into_iter()`）。
fn header_start(text: &impl Lines, line: usize, tab_width: usize) -> usize {
    let content = head(text, line).slice().to_string();
    let trimmed = content.trim();
    let indent = indent_of(text, line, tab_width);
    // `{` 单独一行：向上越过缩进更深的续行，找到缩进与该 `{` 相同的行（`void f()`、
    // `for x in xs`）。
    // 紧邻的上一行是已结束的语句、中间隔着空行，或向上先遇到缩进更浅且不紧邻的行时，
    // 该 `{` 是独立的块（如 Rust 的 `{ … }`），从 `{` 算起。
    if trimmed == "{" {
        let mut at = line;
        while at > 0 && line - at < SCAN_DOWN {
            at -= 1;
            let above = head(text, at).slice().to_string();
            let prev = above.trim();
            if prev.is_empty() || (at + 1 == line && prev.ends_with([';', '{', '}'])) {
                return line;
            }
            let own = indent_of(text, at, tab_width);
            if own == indent {
                return at;
            }
            // 紧邻的上一行缩进更浅且尚未结束（`out.append(` 下一行是 `{`）：该行即为外层块的
            // 开头，`{` 是其后半部分，不单独固定。
            if own < indent {
                return if at + 1 == line { at } else { line };
            }
        }
        return line;
    }
    // `) -> T {`、`):`：参数列表跨多行时，开头是左括号所在的行（`fn f(`），其缩进与当前行相同。
    if trimmed.starts_with([')', ']']) {
        let col = text::leading_whitespace(head(text, line).slice());
        let at = text.line_start(line) + col + 1;
        if let Some((open, _)) = winc_core::movement::matching_bracket(text, at, 100_000) {
            let open = text.line_of(open);
            if indent_of(text, open, tab_width) == indent {
                return open;
            }
        }
    }
    line
}

fn indent_of(text: &impl Lines, line: usize, tab_width: usize) -> usize {
    text::indent_cells(head(text, line).slice(), tab_width)
}

/// 返回以 `start` 为开头的块的最后一行（行号从 0 起）。
///
/// `inside` 是已知位于块内的一行，从该行向下查找；向下 [`SCAN_DOWN`] 行内没有结束，
/// 且未到文档末尾时返回 `None`。
fn scope_end(
    text: &impl Lines,
    language: &Language,
    start: usize,
    inside: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> Option<usize> {
    if language.id == "markdown" {
        return section_end(text, start, inside);
    }
    let indent = indent_of(text, start, tab_width);
    let mut last = inside;
    let limit = text.line_count().min(inside + SCAN_DOWN);
    for line in inside + 1..limit {
        let content = head(text, line);
        let slice = content.slice();
        if text::is_blank(slice) || quoted(line) {
            continue;
        }
        let own = indent_of(text, line, tab_width);
        // 开头的后半部分（`{`、`) -> T {`，其下是缩进更深的块体）与开头缩进相同，仍属于该块；
        // 仅收尾的 `);` 不算，块到该行的上一行为止。
        let header = own == indent
            && continues_header(slice.to_string().trim())
            && starts_block(text, language, line, tab_width, quoted);
        if own <= indent && !header {
            return Some(last);
        }
        last = line;
    }
    (limit == text.line_count()).then_some(last)
}

/// 返回 Markdown 标题的级别（1 到 6，即 `#` 的个数）；`content` 不是标题时返回 `None`。
///
/// 标题是 1 到 6 个 `#` 之后跟空格、制表符或行尾。直接在 `content` 上检查，不把整行复制为
/// `String`。
fn heading_level(content: RopeSlice<'_>) -> Option<usize> {
    let mut hashes = 0;
    for c in content.chars() {
        if c != '#' {
            if c != ' ' && c != '\t' {
                return None;
            }
            break;
        }
        hashes += 1;
        if hashes > 6 {
            return None;
        }
    }
    (hashes > 0).then_some(hashes)
}

/// 是否为代码块的围栏行（忽略行首空白后以三个反引号或三个波浪号开头）；围栏内的 `#` 不是标题。
///
/// 直接在 `slice` 上检查，不把整行复制为 `String`。
fn is_fence(slice: RopeSlice<'_>) -> bool {
    let mut chars = slice.chars().skip_while(|c| c.is_whitespace());
    match chars.next() {
        Some(c @ ('`' | '~')) => chars.next() == Some(c) && chars.next() == Some(c),
        _ => false,
    }
}

/// 是否整行只有空白字符（含空行）；Markdown 节末尾的这类行不计入节。
fn is_empty_line(slice: RopeSlice<'_>) -> bool {
    slice.chars().all(char::is_whitespace)
}

/// 返回 `line` 之前仍未结束的标题（Markdown）。
///
/// `hint` 是上方某一行已计算的结果，从该行继续向下统计；没有 `hint` 时从向上 `SCAN_UP`
/// 行处开始统计，并假定起点不在代码块内。
fn sections(text: &impl Lines, line: usize, hint: Option<&Ancestry>) -> Ancestry {
    let (from, mut open, mut fenced) = match hint {
        Some(h) if h.line <= line => (h.line, h.open.clone(), h.fenced),
        _ => (line.saturating_sub(SCAN_UP), Vec::new(), false),
    };
    for at in from..line {
        let content = head(text, at);
        if is_fence(content.slice()) {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(level) = heading_level(content.slice()) {
            while open.last().is_some_and(|&(l, _)| l >= level) {
                open.pop();
            }
            open.push((level, at));
        }
    }
    Ancestry {
        line,
        levels: Vec::new(),
        threshold: 0,
        open,
        fenced,
    }
}

/// 返回包含该行的各级标题，从外到内（标题级别的数值递增）。
///
/// 该行本身是标题时，同级及更深的各节在该行结束，不包含该行。
fn headings_around(text: &impl Lines, ancestry: &Ancestry) -> Vec<Scope> {
    let mut open = ancestry.open.clone();
    if !ancestry.fenced
        && let Some(level) = heading_level(head(text, ancestry.line).slice())
    {
        while open.last().is_some_and(|&(l, _)| l >= level) {
            open.pop();
        }
    }
    open.into_iter()
        .map(|(_, start)| Scope {
            start,
            end: None,
            last_row: None,
        })
        .collect()
}

/// 返回 `start` 行标题所在的节的最后一行（行号从 0 起）。
///
/// 节到下一个同级或更高级标题之前结束，末尾的空行不计。向下查看到 `inside` 之后
/// [`SCAN_DOWN`] 行为止，范围内未结束且未到文档末尾时返回 `None`。
fn section_end(text: &impl Lines, start: usize, inside: usize) -> Option<usize> {
    let level = heading_level(head(text, start).slice())?;
    // 从标题开始逐行统计代码块围栏，才能判断 `inside` 之后的 `#` 是否位于代码块内。
    let mut fenced = false;
    let mut last = start;
    let limit = text.line_count().min(inside + SCAN_DOWN);
    for line in start + 1..limit {
        let content = head(text, line);
        let slice = content.slice();
        if is_fence(slice) {
            fenced = !fenced;
        } else if !fenced && heading_level(slice).is_some_and(|l| l <= level) {
            return Some(last);
        }
        if !is_empty_line(slice) {
            last = line;
        }
    }
    (limit == text.line_count()).then_some(last)
}

/// 一个可折叠的块：折叠后保留 `start` 行，隐藏其后到 `end` 行为止的各行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    /// 折叠块的第一行（从 0 起），折叠后仍然显示。
    pub start: usize,
    /// 折叠块的最后一行（从 0 起），折叠后不显示。
    pub end: usize,
    /// 嵌套层级，最外层为 1。
    pub level: usize,
}

/// 一行去掉缩进后的前 4 个字符，用于判断该行是否为注释（见 [`is_comment`]）。
///
/// 存放在栈上：整个文件的折叠扫描对每一行都读取一次，不为每行分配内存。
struct Head {
    bytes: [u8; 16],
    len: usize,
}

impl Head {
    fn as_str(&self) -> &str {
        // `bytes[..len]` 由完整的 UTF-8 字符编码而成。
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

/// 返回去掉缩进后的前 4 个字符。
fn line_head(slice: RopeSlice<'_>) -> Head {
    let mut head = Head {
        bytes: [0; 16],
        len: 0,
    };
    for c in slice
        .chars()
        .skip_while(|c| *c == ' ' || *c == '\t')
        .take(4)
    {
        head.len += c.encode_utf8(&mut head.bytes[head.len..]).len();
    }
    head
}

/// 返回整个文件中可折叠的块，按开头所在行排序。
///
/// 一行的下一行缩进更深时，该行是块的开头；块到下一个缩进不比该行深的行之前结束，末尾的
/// 空行不计。注释、预处理指令、字符串中的行（`quoted` 为 `true`）既不开启新块也不结束块，
/// 只把块向下延伸。
///
/// # Panics
///
/// 不会发生：`pop` 之前由 `while` 条件确认了栈非空。
pub fn fold_ranges(
    text: &impl Lines,
    language: &Language,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> Vec<FoldRange> {
    struct Open {
        start: usize,
        indent: usize,
        end: Option<usize>,
    }

    if language.id == "markdown" {
        return heading_ranges(text);
    }

    let mut open: Vec<Open> = Vec::new();
    let mut found = Vec::new();
    let close = |o: Open, level: usize, found: &mut Vec<FoldRange>| {
        if let Some(end) = o.end {
            found.push(FoldRange {
                start: o.start,
                end,
                level,
            });
        }
    };
    for line in 0..text.line_count() {
        let content = head(text, line);
        let slice = content.slice();
        if text::is_blank(slice) {
            continue;
        }
        let indent = text::indent_cells(slice, tab_width);
        if is_comment(line_head(slice).as_str(), language) || quoted(line) {
            for o in open
                .iter_mut()
                .filter(|o| o.indent < indent && o.end.is_some())
            {
                o.end = Some(line);
            }
            continue;
        }
        while open.last().is_some_and(|o| o.indent >= indent) {
            let level = open.len();
            // 循环条件已确认栈非空，`pop` 不会返回 `None`。
            close(open.pop().unwrap(), level, &mut found);
        }
        for o in &mut open {
            o.end = Some(line);
        }
        open.push(Open {
            start: line,
            indent,
            end: None,
        });
    }
    while let Some(o) = open.pop() {
        let level = open.len() + 1;
        close(o, level, &mut found);
    }
    found.sort_by_key(|r| r.start);
    found
}

/// 是否为块的开头：`line` 之下第一个有内容的行缩进更深。
///
/// 只向下查看 `SCAN_DOWN`（64）行，用于绘制行号栏中的折叠箭头，无需找到块的末尾。
pub fn starts_block(
    text: &impl Lines,
    language: &Language,
    line: usize,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> bool {
    if language.id == "markdown" {
        return heading_starts_section(text, line, &mut Fences::default());
    }
    let content = head(text, line);
    let slice = content.slice();
    if text::is_blank(slice) || is_comment(line_head(slice).as_str(), language) || quoted(line) {
        return false;
    }
    let indent = text::indent_cells(slice, tab_width);
    let limit = text.line_count().min(line + 1 + SCAN_DOWN);
    for below in line + 1..limit {
        let content = head(text, below);
        let slice = content.slice();
        if text::is_blank(slice) || is_comment(line_head(slice).as_str(), language) || quoted(below)
        {
            continue;
        }
        return text::indent_cells(slice, tab_width) > indent;
    }
    false
}

/// 返回 `lines`（升序）中是块的开头的行，结果与逐行调用 [`starts_block`] 相同。
///
/// 绘制行号栏的折叠箭头时每帧要查询屏幕上的每一行。Markdown 的标题要先判断是否位于代码块内：
/// 逐行查询时每个标题都要向上统计 `SCAN_UP` 行内的围栏，本函数只在第一个标题处统计一次，
/// 之后从上一个标题继续向下统计（见 `Fences`），被折叠隐藏的行也计入统计。
pub fn block_starts(
    text: &impl Lines,
    language: &Language,
    lines: impl IntoIterator<Item = usize>,
    tab_width: usize,
    quoted: &mut dyn FnMut(usize) -> bool,
) -> Vec<usize> {
    let lines = lines.into_iter();
    if language.id == "markdown" {
        let mut fences = Fences::default();
        return lines
            .filter(|&line| heading_starts_section(text, line, &mut fences))
            .collect();
    }
    lines
        .filter(|&line| starts_block(text, language, line, tab_width, quoted))
        .collect()
}

/// 自上而下依次查询各行时，增量统计行首之前的代码块围栏，判断该行是否位于代码块内。
#[derive(Default)]
struct Fences {
    /// 已统计到的行（不含）及该行之前是否位于代码块内；尚未统计时为 `None`。
    at: Option<(usize, bool)>,
}

impl Fences {
    /// 返回第 `line` 行是否位于代码块内。
    ///
    /// 第一次查询、`line` 早于上次查询的行或与之相隔超过 `SCAN_UP` 行时，与 [`sections`] 相同，
    /// 从向上 `SCAN_UP` 行处开始统计，并假定起点不在代码块内；否则从上次查询的行继续统计。
    fn fenced(&mut self, text: &impl Lines, line: usize) -> bool {
        let (from, mut fenced) = match self.at {
            Some((at, fenced)) if at <= line && line - at <= SCAN_UP => (at, fenced),
            _ => (line.saturating_sub(SCAN_UP), false),
        };
        for at in from..line {
            fenced ^= is_fence(head(text, at).slice());
        }
        self.at = Some((line, fenced));
        fenced
    }
}

/// 是否 `line` 为 Markdown 中可折叠的节的开头（即 [`heading_ranges`] 的结果中含该行）。
///
/// 条件：`line` 是标题、不在代码块内，且其下到同级或更高级标题之前还有内容。
///
/// 不为此计算整个文件的节：绘制行号栏和粘滞滚动每帧都要查询多行。代码块围栏由 `fences` 统计，
/// 依次查询多行时共用同一个 `fences`。
fn heading_starts_section(text: &impl Lines, line: usize, fences: &mut Fences) -> bool {
    let Some(level) = heading_level(head(text, line).slice()) else {
        return false;
    };
    if fences.fenced(text, line) {
        return false;
    }
    // `line` 之下第一个有内容的行是同级或更高级标题时，该节为空，无法折叠。
    for at in line + 1..text.line_count() {
        let content = head(text, at);
        let slice = content.slice();
        if !is_empty_line(slice) {
            return heading_level(slice).is_none_or(|l| l > level);
        }
    }
    false
}

/// 返回 Markdown 中每个标题所在的节对应的折叠块：到下一个同级或更高级标题之前，末尾的空行不计；
/// 代码块内的 `#` 不是标题。
fn heading_ranges(text: &impl Lines) -> Vec<FoldRange> {
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut found = Vec::new();
    let mut fenced = false;
    let mut last = 0;
    let close = |(_, start): (usize, usize), depth: usize, last: usize, found: &mut Vec<_>| {
        if last > start {
            found.push(FoldRange {
                start,
                end: last,
                level: depth,
            });
        }
    };
    for line in 0..text.line_count() {
        let content = head(text, line);
        let slice = content.slice();
        if is_fence(slice) {
            fenced = !fenced;
        } else if !fenced && let Some(level) = heading_level(slice) {
            while open.last().is_some_and(|&(l, _)| l >= level) {
                let depth = open.len();
                close(open.pop().unwrap(), depth, last, &mut found);
            }
            open.push((level, line));
        }
        if !is_empty_line(slice) {
            last = line;
        }
    }
    while let Some(o) = open.pop() {
        close(o, open.len() + 1, last, &mut found);
    }
    found.sort_by_key(|r| r.start);
    found
}

/// 返回第 `line` 行开头最多 [`text::MAX_LINE_BYTES`] 字节的内容。
///
/// 块和标题的判断只检查行首的缩进、标记和开头的若干字符，与绘制处理的范围相同；
/// 按需读取的大文件因此无需读出整行。
fn head<T: Lines>(text: &T, line: usize) -> text::Line<'_> {
    text.line_prefix(line, text::MAX_LINE_BYTES)
}

#[cfg(test)]
mod tests;
