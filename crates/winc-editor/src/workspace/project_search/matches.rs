//! 在文本里找匹配的行，截出带上下文的预览。

use super::*;

/// 搜索已打开且有未保存修改的文档：使用内存中的内容，最多收 `limit` 处。
pub(super) fn search_lines(matcher: &Matcher, rope: &Rope, limit: usize) -> Vec<LineMatch> {
    if matcher.is_multiline() {
        let (text, starts) = joined(rope);
        return joined_matches(matcher, &text, &starts, limit);
    }
    let mut matches = Vec::new();
    rope.visit_lines(0, usize::MAX, |index, _, line| {
        collect(matcher, index, line, &mut matches, limit)
    });
    matches
}

/// 跨行查找：`text` 是各行按 `\n` 连接的文本，`starts` 是每行行首在文件中的位置（字符偏移）。
///
/// 匹配记在起始行上，列范围的结尾可以超出该行（按文件中的位置计算，原换行符为 `\r\n` 的
/// 算两个字符）；预览取起始行。
pub(super) fn joined_matches(
    matcher: &Matcher,
    text: &str,
    starts: &[usize],
    limit: usize,
) -> Vec<LineMatch> {
    let mut out = Vec::new();
    if limit == 0 {
        return out;
    }
    matcher.scan_joined(text, starts, |m| {
        let line_end = memchr::memchr(b'\n', &text.as_bytes()[m.line_byte..])
            .map_or(text.len(), |at| m.line_byte + at);
        let line = &text[m.line_byte..line_end];
        let (s, e) = (
            m.bytes.0 - m.line_byte,
            m.bytes.1.min(line_end) - m.line_byte,
        );
        let (preview, highlight) = preview(line, s, e);
        let start = starts[m.line];
        out.push(LineMatch {
            line: m.line,
            cols: m.range.0 - start..m.range.1 - start,
            preview,
            highlight,
        });
        if out.len() >= limit {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    out
}

/// 把解码后的整个文件按 `\n` 连接，并返回每行行首在文件中的位置（字符偏移）。
///
/// 与 [`joined`] 相同，行按 rope 的规则分。
pub(super) fn joined_text(text: &str) -> (String, Vec<usize>) {
    let (mut joined, mut starts) = (String::with_capacity(text.len()), Vec::new());
    // 上一行起点的字节偏移（`byte`）和字符偏移（`pos`）。
    let (mut byte, mut pos) = (0, 0);
    let _ = for_each_line(text, 0, true, |_, line| {
        let at = line.as_ptr() as usize - text.as_ptr() as usize;
        pos += text[byte..at].chars().count();
        byte = at;
        if !starts.is_empty() {
            joined.push('\n');
        }
        joined.push_str(line);
        starts.push(pos);
        ControlFlow::Continue(())
    });
    (joined, starts)
}

/// 构造一个文件的结果；没有匹配时返回 `None`。
pub(super) fn file_result(
    (root, extra): (&Path, &[(String, PathBuf)]),
    path: &Path,
    matches: Vec<LineMatch>,
    lossy: bool,
) -> Option<FileResult> {
    if matches.is_empty() {
        return None;
    }

    // 附加的目录中的文件：相对于所在的目录，前面加上该目录的名字。
    let folder = extra.iter().find(|(_, folder)| path.starts_with(folder));
    let relative = path.strip_prefix(folder.map_or(root, |(_, folder)| folder.as_path()));
    let label = folder.map(|(label, _)| std::borrow::Cow::Borrowed(label.as_str()));
    let relative = (label.into_iter())
        .chain((relative.unwrap_or(path).components()).map(|c| c.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/");

    Some(FileResult {
        path: path.to_path_buf(),
        relative,
        matches,
        lossy,
    })
}

/// 把第 `index` 行（从 0 起）中的匹配追加到 `out`，`out` 收满 `limit` 处时返回 `Break`。
pub(super) fn collect(
    matcher: &Matcher,
    index: usize,
    line: &str,
    out: &mut Vec<LineMatch>,
    limit: usize,
) -> ControlFlow<()> {
    if out.len() >= limit {
        return ControlFlow::Break(());
    }

    // 把字节偏移换算为字符列。匹配按位置排序且互不重叠，从左到右增量计数即可，超长的行也
    // 不会反复计数：`byte` 处的字符列是 `col`。
    let bytes = line.as_bytes();
    let (mut byte, mut col) = (0, 0);

    matcher.find_all(line, |s, e| {
        let start = col + count_chars(&bytes[byte..s]);
        (byte, col) = (e, start + count_chars(&bytes[s..e]));
        let (preview, highlight) = preview(line, s, e);
        out.push(LineMatch {
            line: index,
            cols: start..col,
            preview,
            highlight,
        });

        if out.len() >= limit {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
}

/// 返回匹配所在的预览文本，以及匹配在预览中的字节范围。
///
/// `start`、`end` 是匹配在 `line` 中的字节范围。预览去掉行首空白；匹配之前超过
/// `PREVIEW_BEFORE` 个字符时截去前面的部分并以「…」代替；总长有上限（`PREVIEW_CHARS`）。
///
/// # Panics
///
/// `start` 或 `end` 不在 `line` 的字符边界上时 panic。
fn preview(line: &str, start: usize, end: usize) -> (String, Range<usize>) {
    let indent = line.len() - line.trim_start().len();
    let start = start.max(indent);
    let end = end.max(start);

    // 匹配前面最多留 `PREVIEW_BEFORE` 个字符。
    let from = line[indent..start]
        .char_indices()
        .rev()
        .nth(PREVIEW_BEFORE - 1)
        .map_or(indent, |(b, _)| indent + b);
    let ellipsis = from > indent;

    // 按这一行剩下的长度预留（制表符和控制字符换成的空格不比原字符长），过长的行按上限预留；
    // 预览会一直留在结果里，建好后再去掉截断留下的余量。
    let rest = line.len() - from + if ellipsis { '…'.len_utf8() } else { 0 };
    let mut text = String::with_capacity(rest.min(PREVIEW_CHARS * 4));

    if ellipsis {
        text.push('…');
    }

    let (mut from_at, mut to_at) = (None, None);

    for (count, (b, c)) in line[from..].char_indices().enumerate() {
        let b = from + b;

        if b == start {
            from_at = Some(text.len());
        }

        if b == end {
            to_at = Some(text.len());
        }

        // 达到字符数上限且匹配已完整时停止；匹配本身超过上限的 4 倍（例如正则 `.*`）时也截断。
        if count >= PREVIEW_CHARS && (b >= end || count >= 4 * PREVIEW_CHARS) {
            break;
        }

        text.push(if c == '\t' || c.is_control() { ' ' } else { c });
    }

    let highlight = from_at.unwrap_or(text.len())..to_at.unwrap_or(text.len());
    text.shrink_to_fit();

    (text, highlight)
}

/// 按 rope 的规则分行（`\n`、`\r\n`、单独的 `\r`）并对每一行调用 `f`，使搜出的行号与打开后
/// 文档中的行号一致。
///
/// 行号从 `first` 起计。`last` 为 `false` 时，`text` 以 `\n` 结尾且后面还有内容，最后一个
/// `\n` 之后的空串不算一行。返回下一行的行号；`f` 返回 `Break` 时停止并返回 `Break`。
pub(super) fn for_each_line(
    text: &str,
    first: usize,
    last: bool,
    mut f: impl FnMut(usize, &str) -> ControlFlow<()>,
) -> ControlFlow<(), usize> {
    let bytes = text.as_bytes();
    let (mut start, mut index) = (0, first);

    if memchr::memchr(b'\r', bytes).is_none() {
        // 只含 `\n` 的文件（常见情形）直接用 `memchr_iter` 查找。
        for end in memchr_iter(b'\n', bytes) {
            f(index, &text[start..end])?;
            start = end + 1;
            index += 1;
        }
    } else {
        for end in memchr2_iter(b'\n', b'\r', bytes) {
            // `\r\n` 已在 `\r` 处计为一个换行，跳过其中的 `\n`。
            if bytes[end] == b'\n' && end > 0 && bytes[end - 1] == b'\r' {
                start = end + 1;
                continue;
            }
            f(index, &text[start..end])?;
            start = end + 1;
            index += 1;
        }
    }

    if last {
        f(index, &text[start..])?;
        index += 1;
    }
    ControlFlow::Continue(index)
}
