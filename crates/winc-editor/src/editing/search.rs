//! 文件内查找与替换：纯文本或正则，可选区分大小写、全字匹配。
//!
//! 搜索按时间片分段进行，每次 `update` 最多耗时 `SLICE`，长文件不会阻塞界面；结果按位置排序，
//! 匹配数达到 [`MAX_MATCHES`] 后停止搜索并标记结果不完整。

mod matcher;

pub(crate) use matcher::*;

use std::borrow::Cow;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use memchr::memmem::Finder;
use regex::{Regex, RegexBuilder};

use winc_core::{Change, EditKind, Lines, Selection, Transaction};
use winc_i18n::t;

use crate::document::{Document, DocumentId};

/// 最多记录的匹配数（处）。
///
/// 达到该数量时计数显示为「10000+」，跳转只在这些匹配之间循环。
pub const MAX_MATCHES: usize = 10_000;
/// 单次 `update` 的搜索时间上限（毫秒）。
///
/// 经验值：限制一次调用阻塞界面的时间。
const SLICE: Duration = Duration::from_millis(8);
/// 检查一次时钟之间处理的行数（行）。
///
/// 避免每行都读取时钟。
const CLOCK_INTERVAL: usize = 256;
/// 检查一次时钟之间处理的字节数（字节，256 KiB），与 `CLOCK_INTERVAL` 先到者为准。
///
/// 只按行数计时，大文件中每行几 MB 时 256 行就是数百 MB，一次调用会阻塞界面数秒。经验值。
const CLOCK_BYTES: usize = 256 << 10;

/// 不依赖文档的查找：在给定的文字里按和 [`Search`] 相同的条件找，语义完全相同。
#[derive(Debug)]
pub struct TextSearch(Matcher);

impl TextSearch {
    /// 编译查询，各条件含义同 [`Search`] 的同名字段。
    ///
    /// # Errors
    ///
    /// 正则表达式有误时返回错误说明。
    pub fn new(
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
        regex: bool,
    ) -> Result<Self, String> {
        let options = Options {
            case_sensitive,
            whole_word,
            regex,
        };
        Matcher::new(query, options).map(TextSearch)
    }

    /// 返回 `text` 里所有不重叠匹配的字节范围，从前到后。
    pub fn find_all(&self, text: &str) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        let _ = self.0.find_all(text, |start, end| {
            found.push((start, end));
            std::ops::ControlFlow::Continue(())
        });
        found
    }
}

/// 一个文档上的查找状态，包括条件、匹配结果和替换串。
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// 查找的内容；为空时没有匹配。
    pub query: String,
    /// 替换成的文字。
    ///
    /// 正则模式下 `$1`、`${name}` 引用分组，`\n` 换成文档的换行符，`\t` 换成制表符。
    pub replacement: String,
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 是否要求全字匹配：匹配的两端不能紧挨着字母、数字或下划线。
    pub whole_word: bool,
    /// 是否按正则匹配。
    pub regex: bool,
    /// 当前匹配在 `matches` 中的下标（从 0 起）。
    pub current: Option<usize>,
    /// 匹配的位置范围（`char` 偏移，左闭右开），按位置排序、互不重叠。
    matches: Vec<(usize, usize)>,
    /// 有匹配的行号，升序、不重复。
    lines: Vec<usize>,
    /// 结果所属的文档及其版本；与本次相同时不重新搜索。
    key: Option<Key>,
    /// 编译 `matcher` 时的查询和选项；与当前条件相同时不必重新编译。
    compiled: Option<(String, Options)>,
    /// 按 `compiled` 的条件编译好的匹配器。
    ///
    /// 查询为空或无法编译时为 `None`。
    matcher: Option<Matcher>,
    /// 查询无法编译或不被支持时的错误说明。
    error: Option<String>,
    /// 搜索未完成时的断点，下次 `update` 从这里继续；已完成时为 `None`。
    resume: Option<Resume>,
    /// 已搜索的行里，最早一处因正则回溯过多而放弃查找的行号；没有时为 `None`。
    gave_up: Option<usize>,
}

/// 搜索结果所属的文档、版本，以及差异快照的一侧。
///
/// 查询和选项另记在 `Search::compiled` 中，比较时不必复制查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    doc: DocumentId,
    revision: u64,
    /// 在差异标签的哪一侧快照中搜索（`LEFT` 或 `RIGHT`）；搜索文档自身的文本时为 `None`。
    snapshot_side: Option<usize>,
}

/// 查找选项，各字段含义同 [`Search`] 的同名字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Options {
    /// 是否区分大小写。
    pub case_sensitive: bool,
    /// 是否全字匹配。
    pub whole_word: bool,
    /// 是否按正则匹配。
    pub regex: bool,
}

/// 搜索断点：下次从第 `line` 行（从 0 起）继续，`pos` 是该行行首的位置（字符偏移）。
#[derive(Debug, Clone, Copy)]
struct Resume {
    line: usize,
    pos: usize,
}

impl Search {
    /// 返回结果所属的文档标识；还没搜过时为 `None`。
    pub fn document(&self) -> Option<DocumentId> {
        self.key.as_ref().map(|k| k.doc)
    }

    /// 返回当前结果所属的差异快照一侧；结果不属于 `doc` 的当前版本，或不是快照的结果时
    /// 返回 `None`。
    pub fn snapshot_side(&self, doc: &Document) -> Option<usize> {
        self.key
            .filter(|k| k.doc == doc.id && k.revision == doc.revision)?
            .snapshot_side
    }

    /// 返回全部匹配的位置范围（`char` 偏移），按起点排序、互不重叠。
    pub fn matches(&self) -> &[(usize, usize)] {
        &self.matches
    }

    /// 返回有匹配的行号（从 0 起），升序且不重复。
    pub fn lines(&self) -> &[usize] {
        &self.lines
    }

    /// 返回与位置范围 `from` 到 `to`（字符偏移，左闭右开）有交集的匹配。
    pub fn matches_in(&self, from: usize, to: usize) -> &[(usize, usize)] {
        let start = self.matches.partition_point(|&(_, end)| end <= from);
        let end = start + self.matches[start..].partition_point(|&(begin, _)| begin < to);
        &self.matches[start..end]
    }

    /// 返回当前匹配的位置范围；没有当前匹配时返回 `None`。
    pub fn current_match(&self) -> Option<(usize, usize)> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    /// 是否因为达到 [`MAX_MATCHES`] 而截断，此时结果不完整。
    pub fn is_truncated(&self) -> bool {
        self.matches.len() >= MAX_MATCHES
    }

    /// 是否有行因正则回溯过多而放弃查找，此时结果可能不全：计数是下限，不能全部替换。
    pub fn gave_up(&self) -> bool {
        self.gave_up.is_some()
    }

    /// 是否已搜索到文档末尾，即没有因时间片用尽而暂停。
    pub fn is_complete(&self) -> bool {
        self.resume.is_none()
    }

    /// 返回已搜索范围的行数上界：行号（从 0 起）小于该值的行都已搜索过。
    ///
    /// 搜索完成时为 `usize::MAX`；因达到 [`MAX_MATCHES`] 而停止时，为最后一个匹配所在行的下一行，
    /// 之后的行不算已搜索。
    pub fn searched_lines(&self) -> usize {
        match self.resume {
            Some(resume) => resume.line,
            None if self.is_truncated() => self.lines.last().map_or(0, |&l| l + 1),
            None => usize::MAX,
        }
    }

    /// 是否由 `doc` 的当前内容搜索得到：文档标识与 `revision` 都相同，且不是差异快照的结果。
    ///
    /// 文档内容修改后、下一次 `update` 之前为 `false`。
    pub fn is_for(&self, doc: &Document) -> bool {
        self.key.as_ref().is_some_and(|k| {
            k.doc == doc.id && k.revision == doc.revision && k.snapshot_side.is_none()
        })
    }

    /// 返回查询的错误说明（正则语法错误，或大文件上的跨行查找）；没有错误时返回 `None`。
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn options(&self) -> Options {
        Options {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
        }
    }

    /// 查询或选项变化时重新编译匹配器、作废已有结果，下次 [`Self::update`] 从头搜索。
    ///
    /// 条件未变化时直接返回。替换只需要匹配器、不需要搜索结果，因此只调用这一步。
    fn ensure_matcher(&mut self) {
        let options = self.options();
        if self
            .compiled
            .as_ref()
            .is_some_and(|(query, compiled)| *query == self.query && *compiled == options)
        {
            return;
        }

        self.matcher = None;
        self.error = None;
        if !self.query.is_empty() {
            match Matcher::new(&self.query, options) {
                Ok(matcher) => self.matcher = Some(matcher),
                Err(error) => self.error = Some(error),
            }
        }
        self.compiled = Some((self.query.clone(), options));
        self.restart();
    }

    /// 文档改动后能否保留改动之上的结果：跨行匹配可能从上方伸进改动的行，不能保留。
    fn resume_kept(&self) -> bool {
        self.matcher.as_ref().is_some_and(|m| !m.is_multiline())
    }

    /// 前 `kept` 行没变：保留这些行里的匹配，下次 [`Self::update`] 从第 `kept` 行（或上次尚未
    /// 搜到的更早位置）继续搜索。
    ///
    /// `kept` 为 0 时同 [`Self::restart`]。
    fn restart_from(&mut self, kept: usize, doc: &Document) {
        let line = kept.min(self.searched_lines());
        let text = doc.text();
        if line == 0 || line >= text.line_count() {
            self.restart();
            return;
        }
        let pos = text.line_start(line);
        let matches = self.matches.partition_point(|&(start, _)| start < pos);
        self.matches.truncate(matches);
        let lines = self.lines.partition_point(|&l| l < line);
        self.lines.truncate(lines);
        self.gave_up = self.gave_up.filter(|&l| l < line);
        self.current = None;
        // 留下的匹配已达到上限：与搜到上限时相同，不再继续搜索。
        self.resume = (matches < MAX_MATCHES).then_some(Resume { line, pos });
    }

    /// 清空结果，下次 [`Self::update`] 从第 0 行开始搜索。
    fn restart(&mut self) {
        self.matches.clear();
        self.lines.clear();
        self.gave_up = None;
        self.current = None;
        self.resume = self.matcher.is_some().then_some(Resume { line: 0, pos: 0 });
    }

    /// 按当前条件更新匹配结果，单次调用最多耗时 `SLICE`。
    ///
    /// 查询或选项变化时从头搜索；文档改动只丢弃改动所在行起的结果，从那一行继续搜索（见
    /// [`Document::unchanged_lines_since`]）；上次未搜完时从断点继续一个时间片。跨行查找（正则
    /// 含 `\n` 等）不分片，一次搜完，文档改动后从头搜索。
    pub fn update(&mut self, doc: &Document) {
        // 仅文档内容变化时，已编译的匹配器仍然有效，不必重新编译。
        self.ensure_matcher();
        let key = Key {
            doc: doc.id,
            revision: doc.revision,
            snapshot_side: None,
        };
        if self.key != Some(key) {
            let kept = match self.key {
                Some(old)
                    if old.doc == doc.id && old.snapshot_side.is_none() && self.resume_kept() =>
                {
                    doc.unchanged_lines_since(old.revision)
                }
                _ => 0,
            };
            self.key = Some(key);
            self.restart_from(kept, doc);
        }
        self.scan(doc.text(), !doc.text().is_large());
    }

    /// 在差异标签 `doc` 第 `side` 侧的快照文本 `text` 中推进查找，单次调用最多耗时 `SLICE`。
    ///
    /// 快照不可修改，结果按（标签，版本，侧）区分，任一项变化时从头搜索。
    pub fn update_snapshot(&mut self, doc: &Document, side: usize, text: &ropey::Rope) {
        self.ensure_matcher();
        let key = Key {
            doc: doc.id,
            revision: doc.revision,
            snapshot_side: Some(side),
        };
        if self.key != Some(key) {
            self.key = Some(key);
            self.restart();
        }
        self.scan(text, true);
    }

    /// 从上次的断点起在 `text` 中继续搜索一个时间片；`multiline` 表示能否取得整篇文本，
    /// 不能时不支持跨行查找。
    fn scan(&mut self, text: &impl Lines, multiline: bool) {
        let (Some(from), Some(matcher)) = (self.resume.take(), &self.matcher) else {
            return;
        };
        let (matches, lines, gave_up) = (&mut self.matches, &mut self.lines, &mut self.gave_up);

        // 跨行查找需要整篇文本，一次搜完。大文件按需从磁盘读取，无法取得整篇文本，因此不支持。
        if matcher.is_multiline() {
            if !multiline {
                self.error = Some(t!("editor-find-large-multiline").to_owned());
                return;
            }
            let (joined, starts) = joined(text);
            matcher.scan_joined(&joined, &starts, |m| {
                matches.push(m.range);
                for line in m.line..=m.last {
                    if lines.last().is_none_or(|&l| l < line) {
                        lines.push(line);
                    }
                }
                if matches.len() >= MAX_MATCHES {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if matcher.take_gave_up() {
                *gave_up = Some(0);
            }
            return;
        }

        let started = Instant::now();
        let (mut count, mut bytes, mut full, mut paused_at) = (0, 0, false, 0);

        let stopped = text.visit_lines(from.line, usize::MAX, |index, start, line| {
            count += 1;

            if count % CLOCK_INTERVAL == 0 || bytes >= CLOCK_BYTES {
                bytes = 0;
                if started.elapsed() >= SLICE {
                    paused_at = start;
                    return ControlFlow::Break(());
                }
            }
            bytes += line.len();

            let flow = matcher.scan(index, start, line, matches, lines, MAX_MATCHES);
            if matcher.take_gave_up() {
                gave_up.get_or_insert(index);
            }
            full = flow.is_break();
            flow
        });

        self.resume = match stopped {
            Some(line) if !full => Some(Resume {
                line,
                pos: paused_at,
            }),
            _ => None,
        };
    }

    /// 返回起点不早于 `pos` 的第一个匹配，并设为当前匹配；`backwards` 时返回起点早于 `pos` 的
    /// 最后一个匹配。
    ///
    /// 到达一端后绕回另一端；没有任何匹配时返回 `None`。搜索尚未完成时只返回确定是「下一个」的
    /// 匹配，不会提前绕回另一端。
    pub fn next_from(&mut self, pos: usize, backwards: bool) -> Option<(usize, usize)> {
        // `matches` 按起点排序：`i` 是第一个起点不早于 `pos` 的匹配。
        let i = self.matches.partition_point(|&(start, _)| start < pos);
        let found = if backwards {
            // 断点早于 `pos` 时，`pos` 之前可能还有尚未找到的匹配，无法确定「上一个」。
            let scanned = self.resume.is_none_or(|r| r.pos >= pos);
            scanned.then(|| i.checked_sub(1)).flatten()
        } else {
            (i < self.matches.len()).then_some(i)
        };

        let index = match found {
            Some(index) => index,
            None if self.is_complete() && !self.matches.is_empty() => {
                if backwards {
                    self.matches.len() - 1
                } else {
                    0
                }
            }
            None => return None,
        };

        self.current = Some(index);
        Some(self.matches[index])
    }

    /// 返回 `ranges`（按位置排序、互不重叠）中仍是匹配的位置及各自的替换文字，正则模式下已
    /// 展开分组引用和转义。
    ///
    /// 在对应的行上重新匹配，不依赖已记录的 `matches`，因此超过 [`MAX_MATCHES`] 而尚未搜到的
    /// 位置也能判断，搜索之后文档被修改也不会替换到错误位置。
    fn edits_for(
        &self,
        doc: &Document,
        ranges: &[(usize, usize)],
    ) -> Vec<((usize, usize), String)> {
        let Some(matcher) = &self.matcher else {
            return Vec::new();
        };
        let regex = matcher.template_regex();
        let template = regex.map(|_| unescape(&self.replacement, doc.line_ending.as_str()));
        let text = doc.text();
        let mut edits = Vec::new();

        // 跨行查找：在整篇文本上重新匹配，只替换 `ranges` 中仍恰好是匹配的位置；
        // 大文件不支持跨行查找，返回空结果。
        if matcher.is_multiline() {
            if text.rope().is_none() {
                return edits;
            }
            let (joined, starts) = joined(text);
            let mut found = Vec::new();
            matcher.scan_joined(&joined, &starts, |m| {
                found.push((m.range, m.bytes));
                ControlFlow::Continue(())
            });
            for &range in ranges {
                let Ok(i) = found.binary_search_by(|(r, _)| r.cmp(&range)) else {
                    continue;
                };
                let (s, e) = found[i].1;
                let inserted = match (regex, &template) {
                    (Some(regex), Some(template)) => {
                        let Some(replaced) = regex.expand_at(&joined, (s, e), template) else {
                            continue;
                        };
                        replaced
                    }
                    _ => self.replacement.clone(),
                };
                edits.push((range, inserted));
            }
            return edits;
        }
        // 一行内的匹配：（在文档中的位置范围，在该行内的字节范围）。
        let mut found: Vec<((usize, usize), (usize, usize))> = Vec::new();
        let mut rest = ranges;

        while let Some(&(first, _)) = rest.first() {
            let line = text.line_of(first);
            let line_start = text.line_start(line);
            let line_end = line_start + text.line_len(line);
            let count = rest
                .iter()
                .take_while(|r| r.0 < line_end.max(first + 1))
                .count();
            let (here, next) = rest.split_at(count);
            rest = next;

            let content = text.line_content(line).slice().to_string();
            let bytes = content.as_bytes();
            let (mut byte, mut pos) = (0, line_start);
            found.clear();
            let _ = matcher.find_all(&content, |s, e| {
                let start = pos + count_chars(&bytes[byte..s]);
                (byte, pos) = (e, start + count_chars(&bytes[s..e]));
                found.push(((start, pos), (s, e)));
                ControlFlow::Continue(())
            });

            for &range in here {
                let Ok(i) = found.binary_search_by(|(r, _)| r.cmp(&range)) else {
                    continue;
                };
                let inserted = match (regex, &template) {
                    (Some(regex), Some(template)) => {
                        let Some(replaced) = regex.expand_at(&content, found[i].1, template) else {
                            continue;
                        };
                        replaced
                    }
                    _ => self.replacement.clone(),
                };
                edits.push((range, inserted));
            }
        }

        edits
    }

    /// 替换与选区完全重合的匹配，返回是否发生了替换；替换后光标位于新文字之后。
    /// 选区不与任何匹配完全重合时不替换并返回 `Ok(false)`，调用方可以转而选中下一个匹配。
    ///
    /// # Errors
    ///
    /// 文档只读时返回只读原因；被替换的文字超过复制上限而无法取出时返回错误说明。
    pub fn replace_current(&mut self, doc: &mut Document) -> Result<bool, &'static str> {
        if let Some(reason) = doc.readonly_reason() {
            return Err(reason);
        }

        self.ensure_matcher();
        let range = (doc.selection.from(), doc.selection.to());
        let edits = self.edits_for(doc, &[range]);
        let Some((_, inserted)) = edits.first() else {
            return Ok(false);
        };

        let caret = range.0 + inserted.chars().count();
        let transaction = replace_transaction(doc, &edits, Selection::point(caret))?;
        doc.apply(transaction);
        Ok(true)
    }

    /// 替换 `ranges`（按位置排序、互不重叠）中仍与匹配完全一致的位置，作为一次可撤销的修改；
    /// 已不是匹配的位置会被跳过。
    ///
    /// 返回替换的处数。
    ///
    /// # Errors
    ///
    /// 文档只读时返回只读原因；被替换的文字超过复制上限而无法取出时返回错误说明。
    pub fn replace_ranges(
        &mut self,
        doc: &mut Document,
        ranges: &[(usize, usize)],
    ) -> Result<usize, &'static str> {
        if let Some(reason) = doc.readonly_reason() {
            return Err(reason);
        }

        self.ensure_matcher();
        let edits = self.edits_for(doc, ranges);
        let Some(&((first, _), _)) = edits.first() else {
            return Ok(0);
        };

        let transaction = replace_transaction(doc, &edits, Selection::point(first))?;
        doc.apply(transaction);
        Ok(edits.len())
    }

    /// 替换全部匹配，作为一次可撤销的修改。
    ///
    /// 返回替换的处数。
    ///
    /// # Errors
    ///
    /// 文档只读、搜索尚未完成、匹配数达到 [`MAX_MATCHES`]，或被替换的文字超过复制上限而无法取出时，
    /// 返回错误说明。
    pub fn replace_all(&mut self, doc: &mut Document) -> Result<usize, &'static str> {
        if let Some(reason) = doc.readonly_reason() {
            return Err(reason);
        }

        self.update(doc);

        if !self.is_complete() {
            return Err(t!("editor-replace-all-pending"));
        }

        if self.is_truncated() {
            return Err(t!("editor-replace-all-too-many"));
        }

        // 放弃查找的行里可能还有匹配，只替换找到的会留下一部分没换。
        if self.gave_up() {
            return Err(t!("editor-replace-all-gave-up"));
        }

        let ranges = self.matches.clone();
        self.replace_ranges(doc, &ranges)
    }
}

/// 构造替换 `edits`（按位置排序、互不重叠）的事务；变更从后往前排列，使前面的位置不受影响。
fn replace_transaction(
    doc: &Document,
    edits: &[((usize, usize), String)],
    after: Selection,
) -> Result<Transaction, &'static str> {
    let changes = edits
        .iter()
        .rev()
        .map(|((start, end), inserted)| {
            Some(Change {
                start: *start,
                removed: doc.text_range(*start, *end)?,
                inserted: inserted.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(t!("editor-replace-too-long"))?;

    Ok(Transaction::new(
        changes,
        doc.selection,
        after,
        EditKind::Other,
    ))
}

#[cfg(test)]
mod tests;
