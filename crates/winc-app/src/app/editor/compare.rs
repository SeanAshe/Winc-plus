//! 当前文档与其他内容比较：与剪贴板、与已保存的版本。
//!
//! 在差异标签中打开：左侧是参照的内容，只读；右侧是当前文档。比较整篇有路径的文档时右侧连着
//! 文档，可以直接编辑，随编辑重新比较（见 [`App::sync_diff_source`]）；比较选中部分或未命名的
//! 文档时右侧是发起比较时的快照，只读。读取和比较在后台进行。

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use ropey::Rope;
use winc_editor::diff_view::{DiffSide, DiffView, RIGHT};
use winc_editor::{Document, Encoding, LARGE_FILE, PreviewKind};
use winc_i18n::t;

use super::*;

use crate::app::scroll::set_scroll_line;

/// 比较时放在左侧、作为参照的内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reference {
    /// 剪贴板中的文字。
    Clipboard,
    /// 磁盘上已保存的版本。
    Saved,
}

/// 参照内容的来源：剪贴板中的文字在界面线程上取出，已保存的版本在后台读取。
enum Left {
    /// 剪贴板中的文字。
    Text(String),
    /// 按编码读取的文件。
    File(PathBuf, Encoding),
}

/// 右侧的内容：发起比较时文档的文字，以及连着文档时的路径和版本号。
struct Right {
    /// 右侧的标题。
    title: String,
    /// 比较的文字。
    text: Rope,
    /// 连着的文档（路径，版本号）；右侧是快照时为 `None`。
    source: Option<(PathBuf, u64)>,
}

/// 后台比较好的结果。
struct Compared {
    reference: Reference,
    /// 比较的是否为选中部分。
    selection: bool,
    /// 文档的名字，用于标签标题和提示。
    name: String,
    /// 比较的结果；无法比较时为原因。
    view: Result<DiffView, String>,
}

/// 与其他内容比较的后台任务的结果通道：发送端交给工作线程，接收端由界面线程轮询。
pub(crate) struct Comparing {
    results: (Sender<Compared>, Receiver<Compared>),
}

impl Default for Comparing {
    fn default() -> Self {
        Comparing { results: channel() }
    }
}

/// 文档 `doc` 是否有可以比较的文字：已读取的文本，不是图片、表格工作簿、差异、日志、阅读或
/// 终端标签。
fn comparable(doc: &Document) -> bool {
    let workbook = doc.table.is_some() && doc.preview_kind() != Some(PreviewKind::Table);
    doc.holds_file_text()
        && doc.picture.is_none()
        && !workbook
        && doc.diff.is_none()
        && doc.log.is_none()
        && doc.reading.is_none()
        && !doc.terminal
}

impl App {
    /// 把当前文档与剪贴板中的文字比较；有选区时比较选中部分（多个光标时取主光标的选区）。
    pub(in crate::app) fn compare_with_clipboard(&mut self) {
        if !self.active_comparable() {
            return;
        }
        let Some(text) = self.clipboard.text().filter(|text| !text.is_empty()) else {
            self.flash(t!("editor-compare-clipboard-empty"));
            return;
        };
        if text.len() as u64 > LARGE_FILE {
            self.flash(t!("editor-compare-clipboard-too-large"));
            return;
        }
        self.compare_active(Reference::Clipboard, Left::Text(text));
    }

    /// 把当前文档与磁盘上已保存的版本比较，右侧是含未保存修改的文档。
    pub(in crate::app) fn compare_with_saved(&mut self) {
        if !self.active_comparable() {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            let name = doc.name().into_owned();
            self.flash(t!("editor-compare-saved-untitled", name = name));
            return;
        };
        let encoding = doc.encoding;
        self.compare_active(Reference::Saved, Left::File(path, encoding));
    }

    /// 当前标签是否有可以比较的文字；没有时说明原因。
    fn active_comparable(&mut self) -> bool {
        let doc = self.editor.active().filter(|doc| comparable(doc));
        match doc.map(|doc| doc.text().rope().is_some()) {
            Some(true) => true,
            Some(false) => {
                self.flash(t!("editor-large-cannot-compare"));
                false
            }
            None => {
                self.flash(t!("editor-compare-no-text"));
                false
            }
        }
    }

    /// 在后台把当前文档与 `left` 比较，结果由 [`App::poll_compares`] 打开。
    ///
    /// 当前标签须有可以比较的文字（见 [`App::active_comparable`]）。
    fn compare_active(&mut self, reference: Reference, left: Left) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let name = doc.name().into_owned();
        let language = doc.highlighter.language();
        let (from, to) = (doc.selection.from(), doc.selection.to());
        // 与剪贴板比较时有选区就比较选区；与已保存的版本比较总是整篇。
        let selection = reference == Reference::Clipboard && from < to;
        let right = if selection {
            let first = rope.char_to_line(from) + 1;
            let last = rope.char_to_line(to.saturating_sub(1).max(from)) + 1;
            let title = match first == last {
                true => t!(
                    "editor-compare-side-line",
                    name = name.as_str(),
                    line = first
                ),
                false => t!(
                    "editor-compare-side-lines",
                    name = name.as_str(),
                    first = first,
                    last = last
                ),
            };
            Right {
                title,
                text: rope.slice(from..to).into(),
                source: None,
            }
        } else {
            let path = doc.path.clone();
            let title = (path.as_deref().and_then(|p| self.relative_path(p)))
                .unwrap_or_else(|| name.clone());
            Right {
                title,
                text: rope.clone(),
                source: path.map(|path| (path, doc.revision)),
            }
        };
        let sender = self.caches.compares.results.0.clone();
        self.spawn_then(UserEvent::Compared, move || {
            let view = compare(left, right, language, &name);
            let compared = Compared {
                reference,
                selection,
                name,
                view,
            };
            let _ = sender.send(compared);
        });
    }

    /// 收下后台比较好的结果：内容相同时提示，否则在差异标签中打开并定位到第一处改动。
    pub(in crate::app) fn poll_compares(&mut self) {
        while let Ok(compared) = self.caches.compares.results.1.try_recv() {
            let name = compared.name.as_str();
            let view = match compared.view {
                Ok(view) => view,
                Err(e) => {
                    self.flash(e);
                    continue;
                }
            };
            if view.is_identical() {
                self.flash(match (compared.reference, compared.selection) {
                    (Reference::Clipboard, true) => {
                        t!("editor-compare-clipboard-identical-selection").to_owned()
                    }
                    (Reference::Clipboard, false) => {
                        t!("editor-compare-clipboard-identical", name = name)
                    }
                    (Reference::Saved, _) => t!("editor-compare-saved-identical", name = name),
                });
                continue;
            }
            let title = match compared.reference {
                Reference::Clipboard => t!("editor-compare-clipboard-title", name = name),
                Reference::Saved => t!("editor-compare-saved-title", name = name),
            };
            let index = self.show_compare(&title, view);
            // 比较期间文档又改了：右侧换成文档现在的内容。
            self.sync_diff_source(index);
        }
    }

    /// 在标题为 `title` 的差异标签中打开 `view`（已有同名的标签时替换它），定位到第一处改动，
    /// 返回该标签的序号。
    pub(in crate::app) fn show_compare(&mut self, title: &str, view: DiffView) -> usize {
        let index = self.editor.show_diff(title, view);
        let doc = &mut self.editor.documents[index];
        let first = doc.diff.as_deref().and_then(|v| v.hunk_rows[RIGHT].first());
        if let Some(row) = first.map(|r| r.start) {
            set_scroll_line(doc, row.saturating_sub(3) as f64);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
        index
    }
}

/// 读取参照内容并与 `right` 比较；参照内容无法读取时返回原因。在后台线程调用。
fn compare(
    left: Left,
    right: Right,
    language: &'static winc_syntax::Language,
    name: &str,
) -> Result<DiffView, String> {
    let (title, text) = match left {
        Left::Text(text) => (t!("editor-compare-side-clipboard"), text),
        Left::File(path, encoding) => {
            let text = winc_editor::saved_text(&path, encoding).map_err(|e| {
                let error = e.to_string();
                t!("editor-compare-saved-failed", name = name, error = error)
            })?;
            (t!("editor-compare-side-saved"), text)
        }
    };
    let old = DiffSide::new(title, &text, None, language);
    let path = right.source.as_ref().map(|(path, _)| path.clone());
    let new = DiffSide::new(right.title, &right.text.to_string(), path, language);
    let mut view = DiffView::new(old, new);
    view.source = right.source.map(|(_, revision)| revision);
    Ok(view)
}
