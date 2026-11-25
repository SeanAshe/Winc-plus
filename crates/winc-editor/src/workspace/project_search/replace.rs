//! 跨文件替换：替换文档或磁盘上的文件，预览替换后的行。

use std::ops::Range;

use winc_core::Lines;

use super::*;
use crate::editing::search::{Pattern, unescape};

/// 替换文档中 `matches` 列出的匹配（结果列表中仍保留的），作为一次可撤销的修改，返回替换的处数。
///
/// 规则与文件内「替换」相同：支持正则分组引用、`\n` 和 `\t` 转义，只读文档不修改。替换前按文档当前
/// 内容逐处核对，已不再是匹配的（文件在搜索之后被修改）不替换。
///
/// # Errors
///
/// 文档只读时返回原因（显示给用户的文字）。
pub fn replace_in_document(
    doc: &mut Document,
    query: &Query,
    replacement: &str,
    matches: &[LineMatch],
) -> Result<usize, &'static str> {
    let text = doc.text();
    // 跨行匹配的结尾可以超出起始行，只需不超出文档范围；替换前还会核对该范围是否仍是匹配。
    let mut ranges: Vec<(usize, usize)> = matches
        .iter()
        .filter(|m| {
            m.line < text.line_count() && text.line_start(m.line) + m.cols.end <= text.end_pos()
        })
        .map(|m| {
            let start = text.line_start(m.line);
            (start + m.cols.start, start + m.cols.end)
        })
        .collect();
    ranges.sort_unstable();
    ranges.dedup();

    in_file_search(query, replacement).replace_ranges(doc, &ranges)
}

/// 替换未打开文件中 `matches` 列出的匹配：读入内容、替换，再原子写回（先写临时文件，再改名），
/// 返回替换的处数。
///
/// 一处都没有替换时不写盘。编码、BOM 和换行符保持原样；含无法解码的字节（文档只读）的文件不修改。
///
/// # Errors
///
/// 无法打开（不存在、没有权限、是二进制文件或超过大小上限）、含无法解码的字节或写入失败时返回原因。
pub fn replace_in_file(
    path: &Path,
    query: &Query,
    replacement: &str,
    matches: &[LineMatch],
) -> Result<usize, String> {
    let mut doc = Document::open(DocumentId(u64::MAX), path.to_path_buf())
        .map_err(|e| Into::<crate::Unsupported>::into(e).to_string())?;
    let count =
        replace_in_document(&mut doc, query, replacement, matches).map_err(str::to_owned)?;

    if count > 0 {
        doc.save().map_err(|e| e.to_string())?;
    }

    Ok(count)
}

/// 结果列表中显示的「替换后」文本：正则模式下展开分组引用。
///
/// 仅用于显示：在预览文本内原地展开（环视可以看到前后的字符）；展开失败时只按匹配到的那一段展开。
/// 结果与真正替换时（按整行匹配）几乎总是相同。
pub struct ReplacePreview {
    regex: Option<Pattern>,
    template: String,
}

impl ReplacePreview {
    /// 按查询条件准备替换预览：正则查询会先编译表达式并展开替换串里的转义。
    pub fn new(query: &Query, replacement: &str) -> Self {
        let regex = query
            .regex
            .then(|| Pattern::new(&query.text, !query.case_sensitive, false).ok())
            .flatten();
        let template = if regex.is_some() {
            unescape(replacement, "\n")
        } else {
            replacement.to_owned()
        };

        Self { regex, template }
    }

    /// 返回 `line` 中字节范围 `range` 处的匹配替换之后的文本。
    pub fn apply(&self, line: &str, range: Range<usize>) -> String {
        let Some(regex) = &self.regex else {
            return self.template.clone();
        };
        regex
            .expand_at(line, (range.start, range.end), &self.template)
            .or_else(|| regex.expand_first(line.get(range)?, &self.template))
            .unwrap_or_else(|| self.template.clone())
    }
}
