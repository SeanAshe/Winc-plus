//! 预览中收起、展开标题下面的一节：点标题左边的箭头切换，收起的标题按文档记下。
//!
//! 收起的块不排版，见 `laid::hidden_blocks`；光标所在的一节始终显示。

use super::*;

impl App {
    /// 收起或展开键为 `key` 的标题（见 `Markdown::sections`）。收起时光标在这一节里的移到
    /// 标题末尾，免得这一节因为光标在里面又显示出来。
    pub(in crate::app) fn toggle_heading_fold(&mut self, key: &str) {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return;
        };
        let folds = self.preview.markdown.folds.entry(id).or_default();
        let folding = !folds.remove(key);
        if folding {
            folds.insert(key.to_owned());
            self.leave_folded_section(key);
        } else if folds.is_empty() {
            self.preview.markdown.folds.remove(&id);
        }
        // 各块从原来的位置滑到新位置：收起的一节合上，展开的一节从标题下面展开。
        self.preview.markdown.refold = true;
        self.preview.markdown.slide = true;
        self.request_redraw();
    }

    /// 锚点 `anchor`（未解码）位于收起的一节中时展开包含它的各节，排版后跳转；返回是否展开了。
    pub(in crate::app) fn unfold_to_anchor(&mut self, anchor: &str) -> bool {
        let key = crate::app::preview::markdown::percent_decode(anchor).to_lowercase();
        let (Some(id), Some(parsed)) = (
            self.editor.active().map(|d| d.id),
            self.preview.markdown.parsed.as_ref(),
        ) else {
            return false;
        };
        let Some(block) = parsed
            .heading_keys()
            .iter()
            .position(|k| k.as_ref() == Some(&key))
        else {
            return false;
        };
        let Some(folds) = self.preview.markdown.folds.get_mut(&id) else {
            return false;
        };
        let around: Vec<String> = (parsed.sections().into_iter().enumerate())
            .filter_map(|(i, s)| s.filter(|s| i < block && block < s.end))
            .map(|s| s.key)
            .filter(|k| folds.contains(k))
            .collect();
        if around.is_empty() {
            return false;
        }
        for k in &around {
            folds.remove(k);
        }
        if folds.is_empty() {
            self.preview.markdown.folds.remove(&id);
        }
        self.preview.markdown.refold = true;
        self.preview.markdown.pending_anchor = Some((id, anchor.to_owned()));
        self.request_redraw();
        true
    }

    /// 光标在键为 `key` 的一节里时移到标题末尾。
    fn leave_folded_section(&mut self, key: &str) {
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let Some((i, section)) = (laid.sections.iter().enumerate())
            .find_map(|(i, s)| s.as_ref().filter(|s| s.key == key).map(|s| (i, s)))
        else {
            return;
        };
        let (Some(heading), Some(last)) = (laid.ranges.get(i), laid.ranges.get(section.end - 1))
        else {
            return;
        };
        let (inside, heading_end) = (heading.end..last.end, heading.end);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let caret = rope.char_to_byte(doc.selection.head.min(rope.len_chars()));
        if inside.contains(&caret) {
            // 标题的原文范围带着行尾的换行：光标放在换行之前。
            let mut at = rope.byte_to_char(heading_end.min(rope.len_bytes()));
            while at > 0 && matches!(rope.char(at - 1), '\n' | '\r') {
                at -= 1;
            }
            doc.selection = winc_core::Selection::point(at);
        }
    }
}
