//! EPUB 的阅读位置：离开一本书（切换、关闭标签）时记下，记入各文件上次的位置
//! （`State::places`），再次打开时排版完成后回到原处。
//!
//! 位置按顶层块记录（[`Place`]），窗口宽度、字号改变后仍对应同一处内容。书中的图片在排版之后
//! 才读完，读完后上方的块变高：回到书中或转到一处之后、用户滚动之前，每次重排都重新对齐。

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
    /// 转换好 EPUB 文件 `path` 时调用：排版完成后回到本次运行中离开时的阅读位置，本次没有离开过
    /// 且 `fresh`（打开后尚未滚动过）时回到上次记下的位置。
    pub(in crate::app) fn begin_book(&mut self, path: PathBuf, fresh: bool) {
        let left = self.preview.markdown.book.places.get(&path).copied();
        let place = left.or_else(|| fresh.then(|| self.saved_book_place(&path)).flatten());
        let book = &mut self.preview.markdown.book;
        book.hold = place.map(|place| (place, None));
        book.path = Some(path);
    }

    /// 正在预览一本书时，保持对齐到滚动位置 `y` 处的内容，直到用户滚动；返回是否在预览书。
    pub(in crate::app) fn hold_book_at(&mut self, y: f32) -> bool {
        let md = &mut self.preview.markdown;
        let Some(place) = md.layout.as_ref().and_then(|laid| place_at(laid, y)) else {
            return false;
        };
        if md.book.path.is_none() {
            return false;
        }
        md.book.hold = Some((place, None));
        true
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

    /// 返回上次记下的、文件 `path` 的阅读位置。
    fn saved_book_place(&self, path: &Path) -> Option<Place> {
        self.state.place_of(path)?.view
    }

    /// 返回本次运行中读过的各本书此刻的阅读位置；位于书的开头的为 `None`，不必记。
    pub(in crate::app) fn book_places_now(&self) -> Vec<(PathBuf, Option<Place>)> {
        let md = &self.preview.markdown;
        let current = md.book.path.clone().zip(md.book_place());
        let left = md.book.places.iter().map(|(path, at)| (path.clone(), *at));
        let places: BTreeMap<PathBuf, Place> = left.chain(current).collect();
        (places.into_iter())
            .map(|(path, at)| (path, Some(at).filter(|at| !at_start(*at))))
            .collect()
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
