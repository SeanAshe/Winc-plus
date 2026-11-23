//! 状态栏上 Markdown 文档的字数：整篇的字数，有选区时另有选中部分的字数。
//!
//! 在后台数（`winc_preview::markdown::words`），按文档、版本和选区缓存：打字时不在界面线程上
//! 读整篇，结果出来之前显示上一次的数。

use std::sync::mpsc;

use super::*;

/// 数字数的依据：文档、版本和选区（字符位置，没有选区时为 0..0）。
type Key = (DocumentId, u64, usize, usize);
/// 数完的结果：依据、整篇的字数、选中部分的字数。
type Counted = (Key, usize, usize);

/// 字数的缓存和正在进行的计数。
pub(crate) struct Words {
    /// 最近一次数完的结果。
    pub(in crate::app) done: Option<Counted>,
    /// 正在数的依据。
    running: Option<Key>,
    results: (mpsc::Sender<Counted>, mpsc::Receiver<Counted>),
}

impl Default for Words {
    fn default() -> Self {
        Self {
            done: None,
            running: None,
            results: mpsc::channel(),
        }
    }
}

impl App {
    /// 当前文档要数字数时的依据：Markdown 文本文档，不是按需读取的大文件。
    fn words_key(&self) -> Option<Key> {
        let doc = self.editor.active()?;
        if doc.preview_kind() != Some(winc_editor::PreviewKind::Markdown)
            || !doc.holds_file_text()
            || doc.is_large()
        {
            return None;
        }
        let (from, to) = match doc.selection.is_empty() {
            true => (0, 0),
            false => (doc.selection.from(), doc.selection.to()),
        };
        Some((doc.id, doc.revision, from, to))
    }

    /// 每帧绘制之前调用：当前文档的字数过期、又没有在数时交给后台数。
    pub(in crate::app) fn track_words(&mut self) {
        let Some(key) = self.words_key() else {
            return;
        };
        let words = &self.status.words;
        if words.done.is_some_and(|(k, ..)| k == key) || words.running.is_some() {
            return;
        }
        let Some(rope) = self.editor.active().and_then(|d| d.text().rope()).cloned() else {
            return;
        };
        // 同一版本只换了选区时沿用整篇的字数。
        let known = (words.done)
            .filter(|((id, revision, ..), ..)| (*id, *revision) == (key.0, key.1))
            .map(|(_, total, _)| total);
        self.status.words.running = Some(key);
        let sender = self.status.words.results.0.clone();
        self.spawn_then(UserEvent::Counted, move || {
            let (_, _, from, to) = key;
            let len = rope.len_chars();
            let total =
                known.unwrap_or_else(|| winc_preview::markdown::words::count(rope.chunks()));
            let selected = match (from.min(len), to.min(len)) {
                (a, b) if a < b => winc_preview::markdown::words::count(rope.slice(a..b).chunks()),
                _ => 0,
            };
            let _ = sender.send((key, total, selected));
        });
    }

    /// 收下后台数好的字数；当前文档又变了时继续数。
    pub(in crate::app) fn poll_words(&mut self) {
        while let Ok(result) = self.status.words.results.1.try_recv() {
            if self.status.words.running == Some(result.0) {
                self.status.words.running = None;
            }
            self.status.words.done = Some(result);
        }
        self.track_words();
        self.request_redraw();
    }

    /// 状态栏上的字数：当前 Markdown 文档数过时为 `Some`，内容是（整篇，选中部分）。
    ///
    /// 新的结果出来之前显示这个文档上一次的结果。
    pub(crate) fn word_count(&self) -> Option<(usize, usize)> {
        let (id, ..) = self.words_key()?;
        let ((done, ..), total, selected) = self.status.words.done?;
        (done == id).then_some((total, selected))
    }
}
