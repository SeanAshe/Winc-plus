//! 各文件上次的位置：关闭标签、保存现场时记下文本的光标和视口、PDF 和 EPUB 的阅读位置，再次
//! 打开时回到原处。记在状态文件里（`State::places`），与文件是否在打开的目录里无关。

use std::path::PathBuf;

use winc_config::{FilePlace, State};
use winc_core::{Lines, Selection};
use winc_editor::{Document, DocumentId};

use crate::app::App;
use crate::app::scroll::set_scroll_line;

/// 一个文件此刻的位置中要改记的一部分；`None` 表示在开头，去掉原有的记录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) enum PlaceNow {
    /// 文本的光标和视口。
    Text(Option<(usize, usize, usize)>),
    /// PDF、EPUB 的阅读位置。
    View(Option<(usize, f32)>),
}

/// 把 `places` 记入 `state`，排在后面的记为更近用到的。
pub(in crate::app) fn record(state: &mut State, places: &[(PathBuf, PlaceNow)]) {
    for (path, now) in places {
        state.set_place(path, |place: &mut FilePlace| match *now {
            PlaceNow::Text(text) => place.text = text,
            PlaceNow::View(view) => place.view = view,
        });
    }
}

/// 返回文档 `doc` 的光标（行、列）和视口顶边所在的行；不显示可编辑的文本、没有对应的文件或
/// 尚未读取时为 `None`，光标和视口都在开头时为 `Some(None)`。
fn text_place(doc: &Document) -> Option<(PathBuf, PlaceNow)> {
    let path = doc
        .path
        .clone()
        .filter(|_| doc.edits_text() && !doc.is_pending())?;
    let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
    let top = doc.line_of_row(doc.scroll_line.max(0.0) as usize);
    let place = Some((line, col, top)).filter(|&place| place != (0, 0, 0));
    Some((path, PlaceNow::Text(place)))
}

impl App {
    /// 返回各文件此刻的位置：本次运行中看过的 PDF 和 EPUB，以及打开着的各文本，当前标签排在
    /// 最后。与已经记下的相同的不列出。
    pub(in crate::app) fn places_now(&self) -> Vec<(PathBuf, PlaceNow)> {
        let views = self
            .pdf_places_now()
            .into_iter()
            .chain(self.book_places_now());
        let views = views.map(|(path, view)| (path, PlaceNow::View(view)));
        let active = self.editor.active;
        let docs = self.editor.documents.iter().enumerate();
        let (front, rest): (Vec<_>, Vec<_>) = docs.partition(|(i, _)| Some(*i) == active);
        let texts = rest
            .into_iter()
            .chain(front)
            .filter_map(|(_, doc)| text_place(doc));
        views
            .chain(texts)
            .filter(|place| !self.place_known(place))
            .collect()
    }

    /// 状态里记的是否已经是 `place`。
    fn place_known(&self, (path, now): &(PathBuf, PlaceNow)) -> bool {
        let known = self.state.place_of(path).unwrap_or_default();
        match *now {
            PlaceNow::Text(text) => known.text == text,
            PlaceNow::View(view) => known.view == view,
        }
    }

    /// 关闭 `ids` 对应的标签之前记下其中各文本的位置。
    pub(in crate::app) fn remember_text_places(&mut self, ids: &[DocumentId]) {
        let docs = ids.iter().filter_map(|id| self.editor.document(*id));
        let places: Vec<_> = (docs.filter_map(text_place))
            .filter(|place| !self.place_known(place))
            .collect();
        if !places.is_empty() {
            self.update_state(move |state| record(state, &places));
        }
    }

    /// 第 `index` 个文档刚读好或刚打开：光标和视口都还在开头时，回到上次记下的位置。行号超出
    /// 文件末尾（文件在别处改过）时停在最后一行。
    pub(in crate::app) fn restore_text_place(&mut self, index: usize) {
        let Some(doc) = self.editor.documents.get(index) else {
            return;
        };
        let untouched = doc.selection == Selection::point(0) && doc.scroll_line == 0.0;
        if !untouched || !doc.edits_text() || doc.is_pending() {
            return;
        }
        let place = doc
            .path
            .as_deref()
            .and_then(|path| self.state.place_of(path));
        let Some((line, col, top)) = place.and_then(|place| place.text) else {
            return;
        };
        let text = doc.text();
        let last = text.line_count().saturating_sub(1);
        let pos = text.line_col_to_pos(line.min(last), col);
        let row = doc.row_at(text.line_start(top.min(last)));
        let doc = &mut self.editor.documents[index];
        doc.selection = Selection::point(pos);
        set_scroll_line(doc, row as f64);
    }

    /// 启动时调用：命令行上给出的文件已经读好，各自回到上次的位置；指定了行号的不动。
    pub(crate) fn restore_text_places(&mut self) {
        for index in 0..self.editor.documents.len() {
            self.restore_text_place(index);
        }
    }
}
