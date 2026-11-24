//! Markdown 中粘贴时改写为对应的写法：网址、图片网址、表格单元格和 HTML 内容。

use super::*;

/// 粘贴处：光标所在行中光标之前、之后的文字，以及上一行是否为空（或不存在）。
pub(super) struct Spot {
    before: String,
    after: String,
    blank_above: bool,
}

impl Spot {
    /// 返回放在粘贴处的多块内容 `text`，前后与其他文字各隔一个空行。
    fn block(&self, text: &str) -> String {
        let lead = if !self.before.trim().is_empty() {
            "\n\n"
        } else if !self.blank_above {
            "\n"
        } else {
            ""
        };
        let text = text.trim_end_matches('\n');
        let tail = if self.after.trim().is_empty() {
            ""
        } else {
            "\n\n"
        };
        format!("{lead}{text}{tail}")
    }
}

impl App {
    /// 返回 Markdown 中粘贴 `content` 时改写成的文字；按普通方式粘贴时为 `None`。
    ///
    /// - 选中一行之内的文字后粘贴网址：`[文字](网址)`；选中的文字本身是网址时不改写。
    /// - 没有选区时粘贴图片网址：`![](网址)`。
    /// - 粘贴表格单元格（见 `cells::from_tsv`）：单独成块的 GFM 表格。
    pub(in crate::app) fn markdown_paste(&self, content: &str) -> Option<String> {
        let doc = self.editor.active()?;
        let is_url = super::writing::is_url;
        let url = content.trim();
        let rope = doc.text().rope()?;
        let chars = rope.len_chars();
        let (from, to) = (
            doc.selection.from().min(chars),
            doc.selection.to().min(chars),
        );
        let selected = rope.slice(from..to).to_string();
        let table = winc_preview::markdown::cells::from_tsv(content).filter(|_| from == to);
        let link = is_url(url)
            && !selected.is_empty()
            && !selected.contains(['\n', '`'])
            && !is_url(selected.trim());
        let image = is_url(url) && selected.is_empty() && super::writing::is_image_url(url);
        if table.is_none() && !link && !image {
            return None;
        }
        let spot = self.prose_spot()?;
        if let Some(table) = table {
            return Some(spot.block(&table));
        }
        let text = if image { "" } else { selected.as_str() };
        Some(winc_preview::markdown::paragraph::link(text, Some(url), image).0)
    }

    /// 返回 Markdown 中粘贴剪贴板 HTML 时写成的 Markdown；HTML 没有结构时为 `None`。
    ///
    /// 只有一段文字时就地插入，有多块时单独成块。
    pub(in crate::app) fn markdown_html_paste(&mut self) -> Option<String> {
        let spot = self.prose_spot()?;
        let html = self.clipboard.html()?;
        let converted = winc_preview::markdown::from_html::from_html(&html);
        if !converted.rich {
            return None;
        }
        let body = converted.text.trim_end();
        let starts_block = ["#", "- ", "> ", "|", "```", "---", "!["]
            .iter()
            .any(|p| body.starts_with(p))
            || body
                .split_once(". ")
                .is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()));
        match !body.contains('\n') && !starts_block {
            true => Some(body.to_owned()),
            false => Some(spot.block(&converted.text)),
        }
    }

    /// 粘贴为纯文本：原样插入剪贴板中的文字，不把网址、表格和网页内容改写为 Markdown 的写法，
    /// 也不把文件、图片写成链接；其余同普通的粘贴（见 `App::paste_text_as`）。
    pub(in crate::app) fn paste_plain(&mut self) {
        if let Some(text) = self.clipboard_text() {
            self.paste_text_as(text, false);
        }
    }

    /// 返回光标处的粘贴处；不是 Markdown 文档、有多个选区，或光标在代码块、公式、HTML 块、
    /// 行内代码中时为 `None`。
    pub(super) fn prose_spot(&self) -> Option<Spot> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(PreviewKind::Markdown) || !doc.extra_selections().is_empty() {
            return None;
        }
        let rope = doc.text().rope()?;
        let from = doc.selection.from().min(rope.len_chars());
        let (from, line) = (rope.char_to_byte(from), rope.char_to_line(from));
        let start = rope.line_to_byte(line);
        let before = rope.byte_slice(start..from).to_string();
        // 该行光标之前的反引号数量为奇数时位于行内代码中。
        if before.matches('`').count() % 2 == 1 {
            return None;
        }
        // 有当前版本的解析结果时按其判断所在的块，否则只按之前的围栏行判断代码块。
        if self.preview.markdown.checked == Some(doc.revision)
            && let Some(parsed) = self.preview.markdown.parsed.as_ref()
        {
            let i = parsed.leaves.partition_point(|l| l.range.start <= from);
            if let Some(leaf) = i.checked_sub(1).map(|i| &parsed.leaves[i])
                && leaf.range.contains(&from)
                && !leaf.prose
            {
                return None;
            }
        } else if in_fence(rope.lines_at(0).take(line)) {
            return None;
        }
        let to = rope.char_to_byte(doc.selection.to().min(rope.len_chars()));
        let end = rope.line_to_byte((line + 1).min(rope.len_lines()));
        let after = rope.byte_slice(to.max(start).min(end)..end).to_string();
        let blank_above = line == 0 || rope.line(line - 1).to_string().trim().is_empty();
        Some(Spot {
            before,
            after,
            blank_above,
        })
    }
}
