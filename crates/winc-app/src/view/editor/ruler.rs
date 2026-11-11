//! 滚动条上的标注，以及选中文字在文件中和屏幕上的其他出现。

use super::*;
use crate::layout::hscrollbar_thumb;

/// 绘制编辑区的滚动条，同时是概览标尺。
///
/// 左侧一列（[`GIT_LANE`]）标出与 `HEAD` 比较得到的改动，右侧一列（[`MARK_LANE`]）标出查找结果
/// 或选中文字的其他出现，以及书签；半透明的滑块绘制在标记之上，可滚动时还标出光标所在的行。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_editor_scrollbar(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let show_matches = app.layout.search.h > 0.0 || app.vim_highlights();
    let occurrences = if show_matches {
        None
    } else {
        occurrence_lines(app, index)
    };

    let (p, layout) = (app.palette, app.layout);
    let doc = &app.editor.documents[index];
    let scroll = (doc.scroll_line * f64::from(app.fonts.line_height)) as f32;
    let owner = doc.id.0;
    let total = doc.row_count();
    let content = editor_content_height(total, layout.editor.h, app.fonts.line_height);
    let track = layout.editor;
    let ruler = Ruler::new(track, total);

    // Git 改动：标记高度与改动所占的行数成比例，鼠标所指的一处向左加宽 1 逻辑像素。
    // 三栏合并中改动由连接带表示，不再标出与 `HEAD` 的差别。
    let hunks = if app.merge_panes_placed() {
        Default::default()
    } else {
        app.changes.hunks(doc.id)
    };
    for (i, h) in hunks.iter().enumerate() {
        let color = crate::view::diff::hunk_shade(p, h).mark;
        let lane = if app.hover == Hover::RulerChange(i) {
            (GIT_LANE.0 + 1.0, GIT_LANE.1 + 1.0)
        } else {
            GIT_LANE
        };
        ruler.mark(frame, lane, ruler_rows(doc, h), color);
    }

    // 按行号换算为显示行并直接绘制，不另行收集：查找结果的数量可能很大。
    let mut marks = |lines: &[usize], color: Color| {
        let rows = lines.iter().map(|&line| doc.row_of(line));
        ruler.runs(frame, MARK_LANE, rows, color);
    };
    if show_matches {
        marks(app.search.lines(), p.search_current);
    } else if let Some(lines) = occurrences {
        marks(&lines, p.ruler_occurrence);
    }
    marks(doc.bookmark_lines(), p.accent);

    // 滑块半透明，绘制在标记之上：标记密集时仍可辨认滑块的位置。
    let caret = doc.row_of(doc.text().line_of(doc.selection.head));
    let look = draw_scrollbar(
        app,
        frame,
        ScrollArea::Editor,
        owner,
        track,
        scroll,
        content,
    );

    // 光标所在行的标记只是滑块位置的参照，随滑块一起隐藏；其余标记一直显示。
    if look.opacity > 0.0 {
        let color = p.ruler_caret.alpha(p.ruler_caret.a * look.opacity);
        ruler.line(frame, caret, color);
    }

    let max_x = app.text_max_scroll_x();
    let track = app.text_hscroll_track();
    app.text_content_w = if max_x > 0.0 { track.w + max_x } else { 0.0 };
    let scroll_x = app.editor.documents[index].scroll_x;
    let thumb = hscrollbar_thumb(track, scroll_x, app.text_content_w);
    let look = app.scrollbar_look(
        ScrollArea::EditorHorizontal,
        owner,
        scroll_x,
        thumb.map(|_| track),
    );
    if let Some(thumb) = thumb {
        draw_thumb(frame, p, thumb, look);
    }
}

/// 滚动条标注的缓存：键为（文档、版本号、查找的词、是否整词），值为该词出现的行号（从 0 起）；
/// 没有可标注的结果时为 `None`。
pub(crate) type RulerCache = (
    (winc_editor::DocumentId, u64, Vec<char>, bool),
    Option<Arc<[usize]>>,
);

/// 返回选中的文本在整个文件中出现的行号（从 0 起）；没有选区、只出现一次时返回 `None`。
///
/// 只依据选区，不依据光标所在的词：按光标所在的词标记时，整列标记随光标移动频繁变化，
/// 光标停在 `self`、`return` 等常见词上时整列布满标记。
///
/// 结果按（文档、版本号、查找内容）缓存，选区不变时滚动不重新计算。每次修改都需要重新搜索
/// 整个文件，因此文件超过 [`RULER_SCAN_MAX`] 时不查找。
fn occurrence_lines(app: &mut App, index: usize) -> Option<Arc<[usize]>> {
    let doc = &app.editor.documents[index];
    if doc
        .text()
        .rope()
        .is_none_or(|rope| rope.len_bytes() > RULER_SCAN_MAX)
    {
        return None;
    }
    if doc.selection.is_empty() {
        return None;
    }
    let occurrences = Occurrences::of(doc)?;
    let key = (
        doc.id,
        doc.revision,
        occurrences.needle.clone(),
        occurrences.whole_word,
    );
    if let Some((cached, lines)) = &app.ruler_cache
        && *cached == key
    {
        return lines.clone();
    }

    let mut lines = Vec::new();
    let mut count = 0;
    let mut chars = Vec::new();
    doc.text()
        .visit_lines(0, MAX_LINE_BYTES, |line, _, content| {
            chars.clear();
            chars.extend(content.chars().take(MAX_LINE_CHARS));
            let found = occurrences.find(&chars).count();
            if found > 0 {
                count += found;
                lines.push(line);
            }
            ControlFlow::Continue(())
        });
    let lines: Option<Arc<[usize]>> = (count >= 2).then(|| lines.into());
    app.ruler_cache = Some((key, lines.clone()));
    lines
}

/// 光标所在的词（或选中的一小段文字）在屏幕上其他出现位置的查找条件；这些位置铺一层淡底色，
/// 便于辨认同一标识符的使用位置。
///
/// 没有选区时取光标两侧的标识符，按整词查找（`id` 不会命中 `width`）；有选区时（不跨行、
/// 不超过 `OCCURRENCE_MAX`、不全是空白）查找完全相同的文字，选区本身不再标注。纯数字不标注，
/// 避免大量 `0`、`1` 的标记造成干扰。
pub(super) struct Occurrences {
    /// 要查找的字符序列。
    needle: Vec<char>,
    /// 是否按整词匹配（匹配处前后不能紧邻词内字符）。
    whole_word: bool,
    /// 选区本身的范围（字符偏移，起止）；该处已有选区底色，不再标注。
    pub(super) skip: Option<(usize, usize)>,
}

/// 查找其他出现时选中文字或光标所在词的最大长度（字符）。
///
/// 超过该长度时不再查找，限定每个可见行逐处比较的开销。
const OCCURRENCE_MAX: usize = 200;

/// 在滚动条上标注选中文字的其他出现时的文件大小上限（字节，512 KiB）。
///
/// 每次修改都需要搜索整个文件（release 构建下 1 MB 约 5 ms，512 KiB 约 2.5 ms，远小于输入时
/// 一帧的预算）。超过上限时只在屏幕上标注，不在滚动条上标注。
const RULER_SCAN_MAX: usize = 512 << 10;

impl Occurrences {
    /// 按当前选区或光标位置确定要标注的内容；无需标注时返回 `None`。
    pub(super) fn of(doc: &winc_editor::Document) -> Option<Occurrences> {
        let text = doc.text();
        let selection = doc.selection;

        if !selection.is_empty() {
            let (from, to) = (selection.from(), selection.to());
            if to - from > OCCURRENCE_MAX || text.line_of(from) != text.line_of(to) {
                return None;
            }
            let needle: Vec<char> = text.substring(from, to).chars().collect();
            // 单个字符的出现过于普遍，标注不提供有效信息。
            if needle.len() < 2 || needle.iter().all(|c| c.is_whitespace()) {
                return None;
            }
            return Some(Occurrences {
                needle,
                whole_word: false,
                skip: Some((from, to)),
            });
        }

        // 只读取光标两侧的标识符，不读取整行（行可能很长）：该逻辑每帧执行。
        let (line, col) = text.pos_to_line_col(selection.head);
        let content = text.line_content(line);
        let slice = content.slice();
        let col = col.min(slice.len_chars()).min(MAX_LINE_CHARS);
        // 与选中文字相同，最多取 `OCCURRENCE_MAX` 个字符：光标位于数 MB 长、无分隔的字符串中时，
        // 每个可见行都需要逐处比较；计数超过上限即停止，不遍历整串。
        let word = |c: &char| is_word_char(*c);
        let limit = OCCURRENCE_MAX + 1;
        let before = (slice.chars_at(col).reversed())
            .take(limit)
            .take_while(word)
            .count();
        let after = slice
            .chars_at(col)
            .take((MAX_LINE_CHARS - col).min(limit))
            .take_while(word)
            .count();
        if before + after > OCCURRENCE_MAX {
            return None;
        }
        let needle: Vec<char> = slice.slice(col - before..col + after).chars().collect();

        if needle.is_empty() || needle.iter().all(char::is_ascii_digit) {
            return None;
        }
        Some(Occurrences {
            needle,
            whole_word: true,
            skip: None,
        })
    }

    /// 返回 `chars`（一行）中每一处出现的列范围（字符偏移，从 0 起，左闭右开）。
    pub(super) fn find<'a>(
        &'a self,
        chars: &'a [char],
    ) -> impl Iterator<Item = (usize, usize)> + 'a {
        line_occurrences(chars, &self.needle, self.whole_word)
    }

    /// 返回 `segments` 所指的屏幕行范围内的出现次数；计数到 2 即停止，因此结果最大为 2。
    pub(super) fn count(&self, text: &impl Lines, segments: &[Range<usize>]) -> usize {
        let mut count = 0;
        let mut chars = Vec::new();
        for range in segments {
            text.visit_lines(range.start, MAX_LINE_BYTES, |line, _, content| {
                if line >= range.end {
                    return ControlFlow::Break(());
                }
                chars.clear();
                chars.extend(content.chars().take(MAX_LINE_CHARS));
                count += self.find(&chars).take(2).count();
                if count >= 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
        }
        count
    }
}
