//! EPUB 的阅读位置：离开一本书（切换、关闭标签）时记下，随目录的现场（`Session::book_places`）
//! 保存，再次打开时排版完成后回到原处。
//!
//! 位置按顶层块记录（[`Place`]），窗口宽度、字号改变后仍对应同一处内容。书中的图片在排版之后
//! 才读完，读完后上方的块变高：恢复之后、用户滚动之前，每次重排都按记下的位置重新对齐。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::app::App;
use crate::app::preview::markdown::laid::Laid;

/// 阅读位置：视口顶边所在的顶层块（从 0 起），以及视口顶边到该块上边缘的距离与块高之比。
pub(crate) type Place = (usize, f32);

/// 各本书的阅读位置。
#[derive(Default)]
pub(crate) struct BookPlaces {
    /// 正在预览的 EPUB 文件；预览的不是 EPUB 时为 `None`。
    path: Option<PathBuf>,
    /// 本次运行中离开过的书的阅读位置。
    places: HashMap<PathBuf, Place>,
    /// 待对齐的位置和上次对齐后的滚动位置（逻辑像素）：滚动位置变了说明用户已经滚动，不再对齐。
    hold: Option<(Place, Option<f32>)>,
}

/// 排版结果 `laid` 中滚动位置 `scroll` 对应的阅读位置；没有块时为 `None`。
fn place_at(laid: &Laid, scroll: f32) -> Option<Place> {
    let block = laid
        .tops
        .partition_point(|&top| top <= scroll)
        .checked_sub(1)?;
    let top = laid.tops[block];
    let bottom = laid.tops.get(block + 1).copied().unwrap_or(laid.height);
    Some((block, (scroll - top) / (bottom - top).max(1.0)))
}

/// 阅读位置 `place` 在排版结果 `laid` 中的滚动位置；块号超出块数（文件已被替换）时为 `None`。
fn scroll_of(laid: &Laid, (block, at): Place) -> Option<f32> {
    let top = *laid.tops.get(block)?;
    let bottom = laid.tops.get(block + 1).copied().unwrap_or(laid.height);
    Some(top + at * (bottom - top))
}

/// 阅读位置 `place` 是否位于书的开头：与未记录位置相同，不必保存。
fn at_start((block, at): Place) -> bool {
    block == 0 && at <= 0.0
}

impl App {
    /// 转换好 EPUB 文件 `path` 时调用；`fresh` 为真（本次打开后尚未滚动过）时，排版完成后回到
    /// 记下的阅读位置。
    pub(in crate::app) fn begin_book(&mut self, path: PathBuf, fresh: bool) {
        let saved = || self.saved_book_place(&path);
        let place = fresh
            .then(|| {
                self.preview
                    .markdown
                    .book
                    .places
                    .get(&path)
                    .copied()
                    .or_else(saved)
            })
            .flatten();
        let book = &mut self.preview.markdown.book;
        book.hold = place.map(|place| (place, None));
        book.path = Some(path);
    }

    /// 排版完成后按待对齐的位置滚动（见 [`BookPlaces::hold`]）。绘制预览时调用。
    pub(crate) fn hold_book_place(&mut self) {
        let md = &mut self.preview.markdown;
        let (Some(laid), Some((place, set))) = (md.layout.as_ref(), md.book.hold) else {
            return;
        };
        if set.is_some_and(|y| y != md.scroll()) {
            md.book.hold = None;
            return;
        }
        let Some(y) = scroll_of(laid, place) else {
            md.book.hold = None;
            return;
        };
        md.scroll_to(y);
        md.book.hold = Some((place, Some(md.scroll())));
    }

    /// 返回目录的现场中记录的、文件 `path` 的阅读位置。
    fn saved_book_place(&self, path: &Path) -> Option<Place> {
        let session = self.state.session_for(self.editor.root()?)?;
        let (_, place) = session.book_places.iter().find(|(file, _)| file == path)?;
        Some(*place)
    }

    /// 返回目录 `root` 中各 EPUB 文件的阅读位置，按路径排序，用于保存现场。
    ///
    /// 以现场中已有的记录为基础，本次运行中读过的书改用当前的位置；位于书的开头的不记录。
    pub(in crate::app) fn book_session(&self, root: &Path) -> Vec<(PathBuf, Place)> {
        let mut files: BTreeMap<PathBuf, Place> = (self.state.session_for(root))
            .map(|s| s.book_places.iter().cloned().collect())
            .unwrap_or_default();
        let md = &self.preview.markdown;
        let current = md.book.path.clone().zip(md.book_place());
        for (path, place) in md
            .book
            .places
            .iter()
            .map(|(p, at)| (p.clone(), *at))
            .chain(current)
        {
            if !path.starts_with(root) {
                continue;
            }
            if at_start(place) {
                files.remove(&path);
            } else {
                files.insert(path, place);
            }
        }
        files.into_iter().collect()
    }
}

impl crate::app::preview::markdown::MarkdownPreview {
    /// 正在预览的书当前的阅读位置；还在对齐时为待对齐的位置，尚未排版时为 `None`。
    fn book_place(&self) -> Option<Place> {
        if let Some((place, _)) = self.book.hold {
            return Some(place);
        }
        place_at(self.layout.as_ref()?, self.scroll())
    }

    /// 离开正在预览的书（切换文档、不再显示预览）：记下阅读位置。
    pub(super) fn leave_book(&mut self) {
        if let Some(path) = self.book.path.take()
            && let Some(place) = self.book_place()
        {
            self.book.places.insert(path, place);
        }
        self.book.hold = None;
    }
}
