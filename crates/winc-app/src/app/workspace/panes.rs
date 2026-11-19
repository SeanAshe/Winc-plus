//! 编辑区的窗格：各窗格的位置、窗格之间的分隔线，前台窗格的切换，以及拆分、合并的命令。
//!
//! 前台窗格就是平常的编辑区：标签栏、编辑区的各个矩形都是它的。绘制、滚动其他窗格时临时把
//! 它换到前台（[`App::with_pane`]），完成后换回。

use winc_editor::panes::{Axis, Node, PaneId};

use super::*;
use crate::app::Scrollbars;
use crate::app::TabHit;
use crate::app::editor::scrollbar::PANE_AREAS;
use crate::app::preview::PanePreview;

/// 分隔线的粗细（逻辑像素）。
pub(crate) const DIVIDER: f32 = 1.0;
/// 分隔线两侧各放宽的可拖动范围（逻辑像素，经验值）。
const GRIP: f32 = 3.0;
/// 拆分后每一侧的最小宽度或高度（逻辑像素，经验值）。
const PANE_MIN: f32 = 160.0;
/// 拖动标签时，鼠标离窗格边缘不足窗格宽或高的这一比例就拆分，否则放进窗格。
const DROP_EDGE: f32 = 1.0 / 3.0;

/// 一个窗格的位置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PaneRects {
    /// 窗格编号。
    pub id: PaneId,
    /// 整个窗格（含标签栏）。
    pub whole: Rect,
    /// 标签栏：贴着编辑区卡片上沿的窗格用顶栏的一段，其余的在窗格顶部另占一行。
    pub tabs: Rect,
    /// 编辑区。
    pub editor: Rect,
}

/// 窗格之间的一道分隔线。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Divider {
    /// 对应第几个拆分（按窗格树的前序遍历）。
    pub index: usize,
    /// 拆分的方向。
    pub axis: Axis,
    /// 画出的线。
    pub line: Rect,
    /// 被拆分的整块区域：拖动时按它换算比例。
    pub span: Rect,
}

/// 拖动的标签松开后放到哪里。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TabDrop {
    /// 放进窗格；`at` 是放在它的第几个标签的位置，为 `None` 时排在最后。
    Into {
        /// 放进的窗格。
        pane: PaneId,
        /// 放在第几个标签的位置。
        at: Option<usize>,
    },
    /// 把窗格按 `axis` 拆开，标签放进新窗格；`before` 为真时新窗格在前（左边或上边）。
    Split {
        /// 被拆的窗格。
        pane: PaneId,
        /// 拆分的方向。
        axis: Axis,
        /// 新窗格是否在前。
        before: bool,
    },
}

/// 窗格在界面上的状态。
#[derive(Debug, Default)]
pub(crate) struct PaneUi {
    /// 各窗格的位置，按从左到右、从上到下的顺序；没有拆分时为空。
    pub rects: Vec<PaneRects>,
    /// 窗格之间的分隔线。
    pub dividers: Vec<Divider>,
    /// 拆分前的整个编辑区；没有拆分时为 `None`。
    pub card: Option<Rect>,
    /// 整张编辑区卡片（含顶栏那一行）。
    area: Rect,
    /// 拖动标签时松开会放到的位置，以及标出它的区域。
    pub drop: Option<(TabDrop, Rect)>,
    /// 鼠标所在的前台以外的窗格：`App::pointer` 的悬停按这个窗格判断。
    pub hover: Option<PaneId>,
    /// 正在画前台以外的窗格：当前标签画得淡一些。
    pub drawing_background: bool,
    /// 这一次从系统拖进来的文件拆出的窗格：同一次拖进来的其余文件也开在这里。
    pub dropped_into: Option<PaneId>,
    /// 前台以外各窗格的标签栏和滚动条。
    parked: Vec<ParkedPane>,
}

/// 不在前台的窗格在界面上的状态。
#[derive(Debug)]
struct ParkedPane {
    id: PaneId,
    bar: TabBar,
    hits: Vec<TabHit>,
    /// 只用其中 [`PANE_AREAS`] 的部分。
    scrollbars: Scrollbars,
    preview: PanePreview,
}

impl ParkedPane {
    /// 刚拆出来、还没有画过的窗格。
    fn new(id: PaneId) -> Self {
        ParkedPane {
            id,
            bar: TabBar::default(),
            hits: Vec::new(),
            scrollbars: Scrollbars::default(),
            preview: PanePreview::default(),
        }
    }
}

impl App {
    /// 按窗格树划分编辑区卡片，并把前台窗格的位置设为编辑区；没有拆分时什么也不改。
    pub(in crate::app) fn place_panes(&mut self) {
        self.panes.rects.clear();
        self.panes.dividers.clear();
        self.panes.card = None;
        let l = self.layout;
        let card = Rect::new(
            l.editor.x,
            l.top_bar.y,
            l.editor.w,
            l.editor.bottom() - l.top_bar.y,
        );
        self.panes.area = card;
        if !self.editor.panes.is_split() {
            return;
        }
        self.panes.card = Some(l.editor);
        let mut index = 0;
        let tree = self.editor.panes.tree.clone();
        self.place_node(&tree, card, &mut index);
        let focused = self.editor.panes.focused;
        if let Some(rects) = self.panes.rects.iter().find(|r| r.id == focused).copied() {
            self.use_pane_rects(rects);
        }
        // 其他窗格的当前标签同样显示着，尚未读取时在后台读取。
        let shown: Vec<DocumentId> = (self.editor.panes.others.iter())
            .filter_map(|p| p.active)
            .collect();
        for doc in shown {
            if let Some(index) = self.editor.index_of(doc) {
                self.load_in_background(index);
            }
        }
    }

    fn place_node(&mut self, node: &Node, area: Rect, index: &mut usize) {
        match node {
            Node::Pane(id) => {
                let rects = self.pane_rects(*id, area);
                self.panes.rects.push(rects);
            }
            Node::Split {
                axis,
                share,
                first,
                second,
            } => {
                let at = *index;
                *index += 1;
                let (a, line, b) = split_area(area, *axis, *share);
                self.panes.dividers.push(Divider {
                    index: at,
                    axis: *axis,
                    line,
                    span: area,
                });
                self.place_node(first, a, index);
                self.place_node(second, b, index);
            }
        }
    }

    /// 窗格 `id` 占 `area` 时的标签栏和编辑区。
    fn pane_rects(&self, id: PaneId, area: Rect) -> PaneRects {
        let l = &self.layout;
        if area.y <= l.top_bar.y {
            // 贴着卡片上沿：标签栏用顶栏的一段，避开左边的侧栏按钮和右边的窗口按钮。
            let x = (area.x + TAB_INSET).max(l.tabs.x);
            let right = (area.right() - 8.0).min(l.tabs.right());
            let tabs = Rect::new(x, l.tabs.y, (right - x).max(0.0), l.tabs.h);
            let editor = Rect::new(area.x, l.editor.y, area.w, area.bottom() - l.editor.y);
            return PaneRects {
                id,
                whole: area,
                tabs,
                editor,
            };
        }
        let tabs = Rect::new(
            area.x + TAB_INSET,
            area.y,
            (area.w - TAB_INSET - 8.0).max(0.0),
            TOP_BAR,
        );
        let editor = Rect::new(
            area.x,
            area.y + TOP_BAR,
            area.w,
            (area.h - TOP_BAR).max(0.0),
        );
        PaneRects {
            id,
            whole: area,
            tabs,
            editor,
        }
    }

    /// 把编辑区的各个矩形设为窗格 `rects` 的。
    fn use_pane_rects(&mut self, rects: PaneRects) {
        let gutter = self.gutter_width();
        let l = &mut self.layout;
        l.tabs = rects.tabs;
        l.editor = rects.editor;
        l.gutter = Rect::new(rects.editor.x, rects.editor.y, gutter, rects.editor.h);
        l.text = Rect::new(
            l.gutter.right(),
            rects.editor.y,
            (rects.editor.w - gutter).max(0.0),
            rects.editor.h,
        );
        if l.search.h > 0.0 {
            l.search.w = l.search.w.min(rects.editor.w - 48.0).max(0.0);
            l.search.x = rects.editor.right() - SCROLLBAR_HIT - 8.0 - l.search.w;
            l.search.y = rects.editor.y + 8.0;
        }
    }

    /// 把窗格 `pane` 换到前台：编辑器的标签和视图状态，以及标签栏的状态一起换。
    pub(in crate::app) fn switch_pane(&mut self, pane: PaneId) -> bool {
        let old = self.editor.panes.focused;
        if pane == old {
            return false;
        }
        self.settle_editor_glide();
        if !self.editor.focus_pane(pane) {
            return false;
        }
        // 前台的标签栏、滚动条和预览与窗格 `pane` 收着的直接交换，交换下来的收给 `old`。
        let at = self.panes.parked.iter().position(|p| p.id == pane);
        let mut parked = match at {
            Some(at) => self.panes.parked.remove(at),
            None => ParkedPane::new(pane),
        };
        self.exchange_pane_ui(&mut parked);
        parked.id = old;
        self.panes.parked.push(parked);
        self.forget_closed_panes();
        true
    }

    /// 前台的标签栏、滚动条和预览与 `parked` 交换。
    fn exchange_pane_ui(&mut self, parked: &mut ParkedPane) {
        std::mem::swap(&mut self.tabs, &mut parked.bar);
        std::mem::swap(&mut self.hits.tabs, &mut parked.hits);
        (self.scroll.bars).exchange(&mut parked.scrollbars, &PANE_AREAS);
        self.preview.exchange(&mut parked.preview);
    }

    /// 把前台窗格的标签栏、滚动条和预览收给窗格 `id`，前台换上空白的。
    fn park_pane_ui(&mut self, id: PaneId) {
        let mut parked = ParkedPane::new(id);
        self.exchange_pane_ui(&mut parked);
        self.panes.parked.retain(|p| p.id != id);
        self.panes.parked.push(parked);
    }

    /// 前台换上窗格 `id` 收着的标签栏、滚动条和预览；没有收着时换上空白的。
    fn unpark_pane_ui(&mut self, id: PaneId) {
        let at = self.panes.parked.iter().position(|p| p.id == id);
        let mut parked = match at {
            Some(at) => self.panes.parked.remove(at),
            None => ParkedPane::new(id),
        };
        self.exchange_pane_ui(&mut parked);
    }

    /// 前台以外的各窗格收着的预览状态。
    pub(crate) fn parked_previews(&self) -> impl Iterator<Item = &PanePreview> {
        self.panes.parked.iter().map(|p| &p.preview)
    }

    /// 丢掉已经合并掉的窗格收着的标签栏、滚动条和预览。
    pub(in crate::app) fn forget_closed_panes(&mut self) {
        let alive = self.editor.panes.tree.panes();
        self.panes.parked.retain(|p| alive.contains(&p.id));
    }

    /// 把前台换成窗格 `pane`（点击、快捷键）：换完重新排版，焦点回到编辑区。
    pub(in crate::app) fn focus_pane(&mut self, pane: PaneId) {
        if self.switch_pane(pane) {
            self.focus = Focus::Editor;
            self.relayout();
            self.request_redraw();
        }
    }

    /// 临时把窗格 `pane` 换到前台、编辑区换成它的位置，运行 `f`，再换回原来的前台窗格。
    pub(crate) fn with_pane<T>(&mut self, pane: PaneId, f: impl FnOnce(&mut App) -> T) -> T {
        let back = self.editor.panes.focused;
        let rects = self.panes.rects.iter().find(|r| r.id == pane).copied();
        // 前台窗格的平滑滚动留到换回来以后接着走。
        let glide = self.scroll.glide.take();
        let switched = rects.is_some() && self.switch_pane(pane);
        let Some(rects) = rects.filter(|_| switched) else {
            self.scroll.glide = glide;
            return f(self);
        };
        let saved = (self.layout, self.git.merge.geometry());
        self.use_pane_rects(rects);
        self.place_editor_parts();
        self.sync_wrap();
        let result = f(self);
        self.switch_pane(back);
        self.layout = saved.0;
        self.git.merge.set_geometry(saved.1);
        self.sync_wrap();
        self.scroll.glide = glide;
        result
    }

    /// 依次画各窗格：先画前台以外的，最后画前台的。
    ///
    /// 悬停和按住拖动的效果只给鼠标所在的窗格：画其他窗格时按它自己的位置重新判断悬停，不画
    /// 光标；鼠标在其他窗格上时前台窗格不显示悬停。标签栏右端按钮的出现、消失属于前台窗格，
    /// 其他窗格的直接显示或不显示。
    pub(crate) fn draw_panes(&mut self, mut draw: impl FnMut(&mut App, bool)) {
        let (focus, hover) = (self.focus, self.pointer.hover);
        let drag = std::mem::replace(&mut self.pointer.drag, Drag::None);
        let (x, y) = self.pointer.mouse;
        let mut elsewhere = false;
        let dragging = self.tab_dragging();
        let fx = &mut self.motion.module_fx;
        let buttons = (
            std::mem::take(&mut fx.preview),
            std::mem::take(&mut fx.browser),
            fx.preview_mode.take(),
        );
        let under = self.background_pane_at(x, y);
        for pane in self.background_panes() {
            self.focus = Focus::Tree;
            let inside = !dragging && under == Some(pane);
            self.with_pane(pane, |app| {
                elsewhere |= inside;
                app.pointer.hover = if inside {
                    app.hover_at(x, y)
                } else {
                    Hover::None
                };
                app.advance_previews();
                app.panes.drawing_background = true;
                draw(app, false);
                app.panes.drawing_background = false;
            });
        }
        let fx = &mut self.motion.module_fx;
        (fx.preview, fx.browser, fx.preview_mode) = buttons;
        self.focus = focus;
        self.pointer.drag = drag;
        self.pointer.hover = if elsewhere { Hover::None } else { hover };
        draw(self, true);
        self.pointer.hover = hover;
    }

    /// 鼠标在 `(x, y)` 时悬停的东西。在其他窗格上时按那个窗格判断，并记下是哪个窗格
    /// （[`PaneUi::hover`]）；Markdown 预览里代码块的悬停也记在那个窗格。
    pub(in crate::app) fn hover_here(&mut self, x: f32, y: f32) -> Hover {
        let pane = self
            .background_pane_at(x, y)
            .filter(|_| !self.tab_dragging());
        self.panes.hover = pane;
        let here = move |app: &mut App| {
            app.track_code_hover(x, y);
            app.hover_at(x, y)
        };
        match pane {
            Some(pane) => self.with_pane(pane, here),
            None => here(self),
        }
    }

    /// 按鼠标当前的位置重新判断悬停（见 [`Self::hover_here`]）。
    pub(in crate::app) fn refresh_pointer_hover(&mut self) {
        let (x, y) = self.pointer.mouse;
        let hover = self.hover_here(x, y);
        self.set_hover(hover);
    }

    /// 在鼠标所在的窗格里运行 `f`：鼠标在其他窗格上时临时把它换到前台。
    pub(crate) fn in_hovered_pane<T>(&mut self, f: impl FnOnce(&mut App) -> T) -> T {
        let (x, y) = self.pointer.mouse;
        match self
            .panes
            .hover
            .filter(|p| self.background_pane_at(x, y) == Some(*p))
        {
            Some(pane) => self.with_pane(pane, f),
            None => f(self),
        }
    }

    /// 前台以外的窗格，按从左到右、从上到下的顺序。
    pub(crate) fn background_panes(&self) -> Vec<PaneId> {
        let focused = self.editor.panes.focused;
        (self.panes.rects.iter())
            .map(|r| r.id)
            .filter(|id| *id != focused)
            .collect()
    }

    /// `(x, y)` 是否被浮在窗格上面的提示框、菜单、对话框、设置或 `⌘P` 面板盖住：盖住时点击、
    /// 滚动和悬停都交给它们。
    pub(in crate::app) fn overlay_covers(&self, x: f32, y: f32) -> bool {
        self.overlays.alert.is_some()
            || self.popup().is_some()
            || self.overlays.form_dialog.is_some()
            || self.settings.open
            || (self.overlays.picker.open && self.layout.picker.contains(x, y))
    }

    /// `(x, y)` 所在的前台以外的窗格；被浮层盖住时为 `None`。
    pub(in crate::app) fn background_pane_at(&self, x: f32, y: f32) -> Option<PaneId> {
        if self.overlay_covers(x, y) {
            return None;
        }
        let focused = self.editor.panes.focused;
        (self.panes.rects.iter())
            .find(|r| r.id != focused && (r.tabs.contains(x, y) || r.editor.contains(x, y)))
            .map(|r| r.id)
    }

    /// 各窗格的位置；没有拆分时是占满编辑区的唯一一个。
    fn pane_areas(&self) -> Vec<PaneRects> {
        if !self.panes.rects.is_empty() {
            return self.panes.rects.clone();
        }
        vec![self.pane_rects(self.editor.panes.focused, self.panes.area)]
    }

    /// `(x, y)` 所在的窗格（含它的标签栏）；没有拆分时为 `None`。
    pub(in crate::app) fn pane_at(&self, x: f32, y: f32) -> Option<PaneRects> {
        (self.panes.rects.iter())
            .find(|r| r.tabs.contains(x, y) || r.editor.contains(x, y))
            .copied()
    }

    /// 在 `(x, y)` 处放下标签或文件时落到哪个窗格，以及贴近它的哪条边（拆分的方向，`before`
    /// 为真时新窗格在前）；落在标签栏那一行或窗格中部时没有边。
    pub(in crate::app) fn pane_drop_at(
        &self,
        (x, y): (f32, f32),
    ) -> Option<(PaneRects, Option<(Axis, bool)>)> {
        let pane = (self.pane_areas().into_iter()).find(|r| r.whole.contains(x, y))?;
        let body = pane.editor;
        if y < body.y {
            return Some((pane, None));
        }
        let rx = (x - body.x) / body.w.max(1.0);
        let ry = (y - body.y) / body.h.max(1.0);
        let edges = [
            (rx, Axis::Row, true),
            (1.0 - rx, Axis::Row, false),
            (ry, Axis::Column, true),
            (1.0 - ry, Axis::Column, false),
        ];
        let (near, axis, before) = (edges.into_iter())
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or((1.0, Axis::Row, false));
        Some((pane, (near < DROP_EDGE).then_some((axis, before))))
    }

    /// 放下时标出的区域：窗格 `pane` 整个（含标签栏那一行），拆分时是新窗格那一半。
    pub(crate) fn drop_area(&self, pane: PaneId, edge: Option<(Axis, bool)>) -> Option<Rect> {
        let area = (self.pane_areas().into_iter())
            .find(|r| r.id == pane)?
            .whole;
        let half = |n: f32| (n / 2.0).round();
        Some(match edge {
            None => area,
            Some((Axis::Row, true)) => Rect::new(area.x, area.y, half(area.w), area.h),
            Some((Axis::Row, false)) => {
                Rect::new(area.right() - half(area.w), area.y, half(area.w), area.h)
            }
            Some((Axis::Column, true)) => Rect::new(area.x, area.y, area.w, half(area.h)),
            Some((Axis::Column, false)) => {
                Rect::new(area.x, area.bottom() - half(area.h), area.w, half(area.h))
            }
        })
    }

    /// 拖动前台窗格的标签 `docs` 到 `(x, y)` 松开时放到哪里，以及标出它的区域；在前台窗格的
    /// 标签栏上（调整顺序）或放下后什么也不变时为 `None`。
    pub(in crate::app) fn tab_drop_at(
        &self,
        docs: &[DocumentId],
        (x, y): (f32, f32),
        copy: bool,
    ) -> Option<(TabDrop, Rect)> {
        let focused = self.editor.panes.focused;
        let (pane, edge) = self.pane_drop_at((x, y))?;
        if y < pane.editor.y {
            // 标签栏那一行：放进另一个窗格，插在鼠标所在的位置，标出一道竖线。
            if pane.id == focused {
                return None;
            }
            let (at, marker) = self.tab_slot_at(pane, x);
            let into = TabDrop::Into {
                pane: pane.id,
                at: Some(at),
            };
            return Some((into, marker));
        }
        let Some((axis, before)) = edge else {
            let into = TabDrop::Into {
                pane: pane.id,
                at: None,
            };
            return (pane.id != focused).then_some((into, pane.whole));
        };
        // 把窗格仅有的标签都拖到它自己的边上：拆出来以后原窗格就空了，什么也不变。
        if pane.id == focused && !copy && docs.len() >= self.editor.tab_count() {
            return None;
        }
        let drop = TabDrop::Split {
            pane: pane.id,
            axis,
            before,
        };
        Some((drop, self.drop_area(pane.id, edge)?))
    }

    /// 从系统拖进来的文件 `path` 在窗格 `pane` 旁边拆出的新窗格里打开（新窗格按 `axis` 和
    /// `before` 放）。同一次拖进来的几个文件开在同一个新窗格里。
    pub(in crate::app) fn open_dropped_in_split(
        &mut self,
        path: &std::path::Path,
        pane: PaneId,
        (axis, before): (Axis, bool),
    ) {
        if let Some(into) = self
            .panes
            .dropped_into
            .filter(|p| self.panes.rects.iter().any(|r| r.id == *p))
        {
            self.focus_pane(into);
            self.open_path(path);
            return;
        }
        self.focus_pane(pane);
        let canonical = winc_config::files::canonicalize(path).unwrap_or_else(|_| path.into());
        let had = (self.editor.tabs().iter()).any(|d| d.path.as_deref() == Some(&*canonical));
        self.open_path(path);
        // 没打开（读不了、被拒绝）时当前标签还是原来的，不拆。
        let Some(doc) = (self.editor.active())
            .filter(|d| d.path.as_deref() == Some(&*canonical))
            .map(|d| d.id)
        else {
            return;
        };
        self.settle_editor_glide();
        if self.editor.split_with(axis, before, doc, pane).is_none() {
            return;
        }
        self.park_pane_ui(pane);
        if !had {
            self.editor.remove_from_pane(pane, doc);
        }
        self.forget_closed_panes();
        self.panes.dropped_into = Some(self.editor.panes.focused);
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 在窗格 `pane` 的标签栏上横坐标 `x` 处插入标签：插在第几个标签的位置，以及标出插入处的
    /// 竖线。
    fn tab_slot_at(&self, pane: PaneRects, x: f32) -> (usize, Rect) {
        let hits = match self.panes.parked.iter().find(|p| p.id == pane.id) {
            Some(parked) => &parked.hits[..],
            None => &self.hits.tabs[..],
        };
        let at = hits.iter().filter(|h| h.tab.x + h.tab.w / 2.0 < x).count();
        let line_x = match (hits.get(at), hits.last()) {
            (Some(next), _) => next.tab.x - TAB_GAP / 2.0,
            (None, Some(last)) => last.tab.right() + TAB_GAP / 2.0,
            (None, None) => pane.tabs.x,
        };
        let bar = pane.tabs;
        (at, Rect::new(line_x - 1.0, bar.y + 8.0, 2.0, bar.h - 16.0))
    }

    /// 把前台窗格的标签 `docs` 放到 `drop`：放进的窗格或拆出的新窗格成为前台，`shown` 成为
    /// 它的当前标签；`copy` 为假时从原窗格去掉，原窗格因此空了就合并掉。
    pub(in crate::app) fn drop_tabs(
        &mut self,
        docs: &[DocumentId],
        shown: DocumentId,
        drop: TabDrop,
        copy: bool,
    ) {
        let Some(&first) = docs.first() else {
            return;
        };
        let source = self.editor.panes.focused;
        self.settle_editor_glide();
        self.tabs.marked.clear();
        let mut slot = None;
        match drop {
            TabDrop::Into { pane, at } => {
                if !self.switch_pane(pane) {
                    return;
                }
                slot = at;
            }
            TabDrop::Split { pane, axis, before } => {
                if pane != source && !self.switch_pane(pane) {
                    return;
                }
                if self
                    .editor
                    .split_with(axis, before, first, source)
                    .is_none()
                {
                    return;
                }
                self.park_pane_ui(pane);
            }
        }
        for (k, doc) in docs.iter().enumerate() {
            let index = self.editor.adopt_tab(*doc, source);
            // 放在标签栏上松手的位置；已经是这里的标签的，挪过去。
            if let (Some(index), Some(at)) = (index, slot) {
                let before = at + k;
                let to = if index < before { before - 1 } else { before };
                self.editor
                    .move_document(index, to.min(self.editor.tab_count() - 1));
            }
        }
        if !copy {
            for doc in docs {
                self.editor.remove_from_pane(source, *doc);
            }
        }
        self.forget_closed_panes();
        if let Some(index) = self.editor.index_of(shown) {
            self.activate_tab(index);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// `(x, y)` 所在的窗格分隔线（可拖动的范围比线宽一些）。
    pub(in crate::app) fn pane_divider_at(&self, x: f32, y: f32) -> Option<usize> {
        (self.panes.dividers.iter())
            .find(|d| grip(d).contains(x, y))
            .map(|d| d.index)
    }

    /// 正被悬停或拖动的窗格分隔线。
    pub(crate) fn active_pane_divider(&self) -> Option<usize> {
        match (self.pointer.hover, &self.pointer.drag) {
            (_, Drag::PaneDivider { index }) => Some(*index),
            (Hover::PaneDivider(index), _) => Some(index),
            _ => None,
        }
    }

    /// 正被悬停或拖动的窗格分隔线的方向。
    pub(in crate::app) fn pane_divider_axis(&self) -> Option<Axis> {
        let index = self.active_pane_divider()?;
        (self.panes.dividers.iter())
            .find(|d| d.index == index)
            .map(|d| d.axis)
    }

    /// 拖动第 `index` 个拆分的分隔线到 `(x, y)`：两侧都至少留 [`PANE_MIN`]。
    pub(in crate::app) fn drag_pane_divider(&mut self, index: usize, (x, y): (f32, f32)) {
        let Some(divider) = self
            .panes
            .dividers
            .iter()
            .find(|d| d.index == index)
            .copied()
        else {
            return;
        };
        let span = divider.span;
        let (start, size, at) = match divider.axis {
            Axis::Row => (span.x, span.w, x),
            Axis::Column => (span.y, span.h, y),
        };
        if size < PANE_MIN * 2.0 {
            return;
        }
        let share = ((at - start).clamp(PANE_MIN, size - PANE_MIN)) / size;
        if let Some(slot) = self.editor.panes.tree.share_mut(index) {
            *slot = share;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 双击分隔线：两侧恢复各占一半。
    pub(in crate::app) fn reset_pane_divider(&mut self, index: usize) {
        if let Some(slot) = self.editor.panes.tree.share_mut(index) {
            *slot = 0.5;
        }
        self.relayout();
        self.request_redraw();
    }

    /// 向右（`Axis::Row`）或向下拆分前台窗格，新窗格打开当前标签。
    pub(in crate::app) fn split_editor(&mut self, axis: Axis) {
        let old = self.editor.panes.focused;
        self.settle_editor_glide();
        if self.editor.split(axis).is_none() {
            return;
        }
        // 编辑器已经把新窗格换到前台；标签栏和滚动条跟着换。
        self.park_pane_ui(old);
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 把当前标签移到新拆出的窗格，原窗格里不再有它；窗格里只有这一个标签时不动。
    pub(in crate::app) fn move_to_split(&mut self, axis: Axis) {
        let old = self.editor.panes.focused;
        let Some(doc) = self.editor.active().map(|d| d.id) else {
            return;
        };
        if self.editor.tab_count() < 2 {
            return;
        }
        self.split_editor(axis);
        self.editor.remove_from_pane(old, doc);
        self.relayout();
    }

    /// 合并全部窗格，标签都回到前台窗格。
    pub(in crate::app) fn unsplit_editor(&mut self) {
        if !self.editor.panes.is_split() {
            return;
        }
        self.settle_editor_glide();
        self.editor.unsplit();
        self.panes.parked.clear();
        self.relayout();
        self.request_redraw();
    }

    /// 前台换成下一个（`forward`）或上一个窗格，到头时回到另一端。
    pub(in crate::app) fn cycle_pane(&mut self, forward: bool) {
        let order = self.editor.panes.tree.panes();
        let Some(at) = order.iter().position(|p| *p == self.editor.panes.focused) else {
            return;
        };
        let n = order.len();
        if n < 2 {
            return;
        }
        let next = if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        };
        self.focus_pane(order[next]);
    }

    /// 前台窗格的标签都关掉以后：不是唯一的窗格时去掉它，前台换成相邻的窗格。
    pub(in crate::app) fn drop_empty_pane(&mut self) {
        let gone = self.editor.panes.focused;
        if !self.editor.drop_empty_pane() {
            return;
        }
        self.unpark_pane_ui(self.editor.panes.focused);
        self.panes.parked.retain(|p| p.id != gone);
        self.relayout();
    }
}

impl App {
    /// 现场中记的窗格；`slots` 是各文档在现场标签中的序号。没有拆分时为 `None`。
    pub(in crate::app) fn session_panes(
        &self,
        slots: &[(winc_editor::DocumentId, usize)],
    ) -> Option<winc_config::SessionPanes> {
        if !self.editor.panes.is_split() {
            return None;
        }
        let order = self.editor.panes.tree.panes();
        let slot = |id| slots.iter().find(|(doc, _)| *doc == id).map(|&(_, at)| at);
        let panes = (order.iter())
            .map(|&pane| {
                let (tabs, active) = self.editor.pane_tabs(pane);
                winc_config::SessionPane {
                    tabs: tabs.into_iter().filter_map(slot).collect(),
                    active: active.and_then(slot),
                }
            })
            .collect();
        let focused = order.iter().position(|p| *p == self.editor.panes.focused)?;
        Some(winc_config::SessionPanes {
            tree: session_node(&self.editor.panes.tree, &order),
            panes,
            focused,
        })
    }
}

/// 把窗格树换成现场中的写法，叶子是窗格在 `order` 中的序号。
fn session_node(node: &Node, order: &[PaneId]) -> winc_config::PaneNode {
    match node {
        Node::Pane(id) => {
            winc_config::PaneNode::Pane(order.iter().position(|p| p == id).unwrap_or(0))
        }
        Node::Split {
            axis,
            share,
            first,
            second,
        } => winc_config::PaneNode::Split {
            column: *axis == Axis::Column,
            share: *share,
            first: Box::new(session_node(first, order)),
            second: Box::new(session_node(second, order)),
        },
    }
}

/// 把现场中的窗格排列换成窗格树，叶子 `i` 成为窗格 `PaneId(i)`。
pub(crate) fn pane_tree(node: &winc_config::PaneNode) -> Node {
    match node {
        winc_config::PaneNode::Pane(at) => Node::Pane(PaneId(*at as u32)),
        winc_config::PaneNode::Split {
            column,
            share,
            first,
            second,
        } => Node::Split {
            axis: if *column { Axis::Column } else { Axis::Row },
            share: *share,
            first: Box::new(pane_tree(first)),
            second: Box::new(pane_tree(second)),
        },
    }
}

/// 把 `area` 按 `axis` 和 `share` 分成两半，返回前一半、分隔线和后一半。
fn split_area(area: Rect, axis: Axis, share: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::Row => {
            let first = (area.w * share).round().clamp(0.0, area.w);
            let a = Rect::new(area.x, area.y, first, area.h);
            let line = Rect::new(area.x + first, area.y, DIVIDER, area.h);
            let rest = (area.w - first - DIVIDER).max(0.0);
            let b = Rect::new(line.right(), area.y, rest, area.h);
            (a, line, b)
        }
        Axis::Column => {
            let first = (area.h * share).round().clamp(0.0, area.h);
            let a = Rect::new(area.x, area.y, area.w, first);
            let line = Rect::new(area.x, area.y + first, area.w, DIVIDER);
            let rest = (area.h - first - DIVIDER).max(0.0);
            let b = Rect::new(area.x, line.bottom(), area.w, rest);
            (a, line, b)
        }
    }
}

/// 分隔线可以拖动的范围。
fn grip(divider: &Divider) -> Rect {
    let line = divider.line;
    match divider.axis {
        Axis::Row => Rect::new(line.x - GRIP, line.y, line.w + 2.0 * GRIP, line.h),
        Axis::Column => Rect::new(line.x, line.y - GRIP, line.w, line.h + 2.0 * GRIP),
    }
}
