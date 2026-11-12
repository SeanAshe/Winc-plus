//! 窗口布局：各区域的几何，绘制和点击共用同一份。
//!
//! 坐标和尺寸的单位均为逻辑像素，原点在窗口左上角。[`Layout::compute`] 根据窗口大小和界面
//! 状态一次算出主要区域；查找条、快速打开面板、设置页等的内部部件由各自的 `*Parts` 类型计算。

use std::sync::atomic::{AtomicU32, Ordering};

use winc_render::{Color, Frame, Rect};

use winc_config::ThemeChoice;

use crate::app::{PICKER_PAD, PICKER_ROW, PLACES_FOOTER};
use crate::icons;
use crate::keymap::{self, Action, KeyCode, Mods, Os, Stroke};
use crate::theme::*;

mod dialog;
mod overlays;
mod settings;
#[cfg(test)]
mod tests;
mod workspace;
mod wrap;

pub use dialog::*;
pub use overlays::*;
pub use settings::*;
pub use workspace::panel::*;
pub use workspace::sidebar::*;
pub use wrap::wrap_text;

/// 窗口各主要区域的矩形，由 [`Layout::compute`] 生成。
#[derive(Debug, Clone, Copy, Default)]
pub struct Layout {
    /// 整个窗口。
    pub window: Rect,
    /// 侧栏卡片，从窗口顶部的留白之下到状态栏。
    ///
    /// 宽度不随展开比例变化：未完全展开时左侧伸到窗口外，
    /// 隐藏时整个位于窗口外（`right()` 不大于 0）。
    pub sidebar: Rect,
    /// 侧栏第二行：左边项目和分支，右端当前视图的工具。
    pub sidebar_header: Rect,
    /// 侧栏中的文件树区域，位于项目名所在行之下；文件视图里不含底部的大纲。
    pub tree: Rect,
    /// 文件视图底部的大纲：标题行连同展开时的符号列表；不在文件视图时为空。
    pub outline: Rect,
    /// 侧栏和编辑区之间可拖动的分隔条。
    pub splitter: Rect,
    /// 侧栏开关按钮：固定在红绿灯右侧，侧栏展开、收起时都不动。
    pub sidebar_button: Rect,
    /// 编辑区卡片顶部的顶栏。
    pub top_bar: Rect,
    /// 顶栏右端的窗口按钮（最小化、最大化、关闭），由应用绘制标题栏时才有，否则为空矩形；由
    /// `App::relayout` 放置。
    pub controls: Rect,
    /// 顶栏中放置标签和新建按钮的区域。
    pub tabs: Rect,
    /// 标签栏右端切换「源代码 / 分栏 / 预览」的三个按钮（仅 SVG 等可预览的文件有，见
    /// [`preview_segments`]）；没有时为空矩形，由 `App::relayout` 放置。
    pub preview: Rect,
    /// 标签栏右端「在浏览器中打开」按钮（仅当前文件是网页时有）；没有时为空矩形，由
    /// `App::relayout` 放置。
    pub browser: Rect,
    /// 分栏预览时编辑区右半部分的预览区域（编辑区、文本区不含该区域）；不分栏时为空矩形。
    pub split: Rect,
    /// 分栏预览时左右两半之间可拖动的范围，比分隔线两侧各宽出数个逻辑像素；不分栏时为空矩形。
    pub split_divider: Rect,
    /// 编辑区，包含行号栏和文本区。
    pub editor: Rect,
    /// 编辑区左侧的行号栏。
    pub gutter: Rect,
    /// 编辑区中行号栏右侧的文本区。
    pub text: Rect,
    /// 编辑区右侧的缩略图；没有显示时宽度为 0。
    pub minimap: Rect,
    /// 悬浮在编辑区右上角的查找条，展开替换时为两行高；关闭时尺寸为 0。
    pub search: Rect,
    /// 快速打开面板（⌘P）：输入框加结果列表；关闭时尺寸为 0。
    pub picker: Rect,
    /// 窗口底部的状态栏。
    pub status: Rect,
    /// 编辑区旁边的终端面板：位于下方时从编辑区卡片下部划出，位于右侧时从右部划出；
    /// 关闭时尺寸为 0。
    ///
    /// 由 `App::relayout` 计算。
    pub terminal: Rect,
    /// 编辑区和终端面板之间的间距：面板在下方时上下拖动调整高度，在右侧时左右拖动调整宽度。
    pub terminal_divider: Rect,
}

impl Layout {
    /// 根据窗口大小和界面状态计算各区域的位置。
    ///
    /// - `width`、`height`：窗口的逻辑大小。
    /// - `panel_width`：侧栏完全展开时的宽度。
    /// - `shown`：侧栏露出的比例，0 为隐藏，1 为完全展开，动画中介于两者之间。
    /// - `search_rows`：查找条的行数，0 为关闭，1 为只有查找，2 为查找加替换。
    /// - `picker_rows`：快速打开面板的结果行数，0 为关闭。
    /// - `gutter_width`：行号栏的宽度。
    /// - `status_height`：窗口底部留给状态栏的高度。
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        width: f32,
        height: f32,
        panel_width: f32,
        shown: f32,
        search_rows: usize,
        picker_rows: usize,
        gutter_width: f32,
        status_height: f32,
    ) -> Layout {
        let window = Rect::new(0.0, 0.0, width, height);
        let status = Rect::new(0.0, height - status_height, width, status_height);
        let body_h = status.y.max(0.0);

        // 取整：拖动分隔条时宽度来自鼠标位置，可能是小数。`visible_w` 已取整；若 `full_w` 不取整，
        // 两者的小数差会计入侧栏起点，拖动时整个侧栏随鼠标左右抖动。
        let full_w = panel_width
            .min((width - 320.0).max(MIN_PANEL_WIDTH))
            .round();
        let visible_w = (full_w * shown.clamp(0.0, 1.0)).round();
        let side_x = visible_w - full_w;
        // 侧栏占据的一列宽 `full_w`：左侧和上方留出 `ISLAND_MARGIN`，右侧留出卡片间距的一半，
        // 其余是侧栏卡片。
        let m = ISLAND_MARGIN;
        let sidebar = Rect::new(
            side_x + m,
            m,
            full_w - m - ISLAND_GAP / 2.0,
            (body_h - m).max(0.0),
        );
        let sidebar_header = Rect::new(sidebar.x, TOP_BAR_END, sidebar.w, SIDEBAR_HEADER);
        let tree = Rect::new(
            sidebar.x,
            sidebar_header.bottom(),
            sidebar.w,
            (body_h - sidebar_header.bottom()).max(0.0),
        );
        // 侧栏动画进行中（`shown` 小于 1）不显示分隔条，因此不能拖动；显示时宽 6 像素
        // （`ISLAND_GAP`）。
        let splitter_w = if shown >= 1.0 { 6.0 } else { 0.0 };
        let splitter = Rect::new(visible_w - 3.0, 0.0, splitter_w, body_h);

        // 编辑区卡片从侧栏卡片右侧的间距之后开始；侧栏隐藏时与窗口左边缘相隔 `ISLAND_MARGIN`。
        let main_x = (visible_w + ISLAND_GAP / 2.0).max(m);
        let main_w = (width - m - main_x).max(0.0);
        let top_bar = Rect::new(main_x, m, main_w, TOP_BAR);

        // 侧栏开关固定在红绿灯右侧，侧栏展开、收起和动画期间位置不变，连续点击时无需重新定位。
        // 侧栏收起时标签从开关右侧开始，与开关相隔 `TAB_GAP`，和标签之间的间距相同；
        // 侧栏展开时开关右边是视图切换图标（`view_tabs`）。
        let button_y = m + (TOP_BAR - BUTTON) / 2.0;
        let sidebar_button = Rect::new(
            first_button_x(icons::SIDEBAR_WIDTH),
            button_y,
            BUTTON,
            BUTTON,
        );
        let tabs_x = (main_x + TAB_INSET).max(sidebar_button.right() + TAB_GAP);
        // 标签栏延伸到顶栏右端：查找由快捷键打开，设置的入口位于菜单栏和命令面板中。
        let tabs = Rect::new(
            tabs_x,
            m,
            (top_bar.right() - 8.0 - tabs_x).max(0.0),
            TOP_BAR,
        );

        let editor = Rect::new(main_x, TOP_BAR_END, main_w, (body_h - TOP_BAR_END).max(0.0));
        let gutter = Rect::new(editor.x, editor.y, gutter_width, editor.h);
        let text = Rect::new(
            gutter.right(),
            editor.y,
            (editor.w - gutter_width).max(0.0),
            editor.h,
        );

        let search = if search_rows > 0 {
            let w = SEARCH_WIDTH.min(editor.w - 48.0).max(0.0);
            Rect::new(
                editor.right() - SCROLLBAR_HIT - 8.0 - w,
                editor.y + 8.0,
                w,
                search_rows as f32 * BAR_HEIGHT,
            )
        } else {
            Rect::default()
        };

        let picker = if picker_rows > 0 {
            // 面板宽度以编辑区为基准，左右合计留出 48 像素（与查找条相同）；窄窗口下随编辑区收窄。
            let w = PICKER_WIDTH.min(editor.w - 48.0).max(0.0);
            let h = BAR_HEIGHT + picker_rows as f32 * PICKER_ROW + 2.0 * PICKER_PAD;
            let x = (editor.x + (editor.w - w) / 2.0).round();
            Rect::new(x, editor.y + 8.0, w, h)
        } else {
            Rect::default()
        };

        Layout {
            window,
            sidebar,
            sidebar_header,
            tree,
            outline: Rect::default(),
            splitter,
            sidebar_button,
            top_bar,
            controls: Rect::default(),
            tabs,
            preview: Rect::default(),
            browser: Rect::default(),
            split: Rect::default(),
            split_divider: Rect::default(),
            editor,
            gutter,
            text,
            minimap: Rect::default(),
            search,
            picker,
            status,
            terminal: Rect::default(),
            terminal_divider: Rect::default(),
        }
    }
}

/// 返回绘制时记录的可点击区域中包含 `(x, y)` 的一项的目标；区域重叠时取排在前面的一项，
/// 没有时返回 `None`。
pub fn hit<T: Copy>(regions: &[(Rect, T)], x: f32, y: f32) -> Option<T> {
    regions
        .iter()
        .find(|(rect, _)| rect.contains(x, y))
        .map(|&(_, target)| target)
}

/// 返回使 `top..bottom`（内容坐标）落入高度为 `view` 的可见范围所需的滚动位置。
///
/// 已经可见时保持 `scroll` 不变；在可见范围上方时，使其顶边与可见范围的顶边对齐；在下方时，
/// 使其底边与可见范围的底边对齐。
pub fn scroll_to_show(scroll: f32, top: f32, bottom: f32, view: f32) -> f32 {
    if top < scroll {
        top
    } else if bottom > scroll + view {
        bottom - view
    } else {
        scroll
    }
}

/// 三段切换的总宽度（逻辑像素）：三个按钮之间相隔 `TAB_GAP`，
/// 与左侧新建按钮到三段切换的间距相同。
pub const PREVIEW_GROUP_W: f32 = BUTTON * 3.0 + TAB_GAP * 2.0;

/// 返回三段切换 `group` 中的三个按钮，依次为源代码、分栏、预览，彼此相隔 `TAB_GAP`。
///
/// 出现、消失的动画中 `group` 比 [`PREVIEW_GROUP_W`] 窄，按钮宽度和间隔按同一比例缩小。
pub fn preview_segments(group: Rect) -> [Rect; 3] {
    let k = group.w / PREVIEW_GROUP_W;
    let (w, gap) = (BUTTON * k, TAB_GAP * k);
    [0.0, 1.0, 2.0].map(|i| Rect::new(group.x + (w + gap) * i, group.y, w, group.h))
}

/// 返回文件树第 `v` 行（从 0 起）的位置和裁剪范围；动画中行序按展开后的状态计算。
///
/// `block` 是正在展开或收起的子行范围（行序，不含末尾），`hidden` 是该范围尚未露出的高度（像素）。
/// 子行的位置不动，只显示已露出的一段，其余被裁剪；子行之后的行向上移动 `hidden`。
/// 没有动画时 `block` 为 `None`。
pub fn tree_row_slot(
    tree: Rect,
    scroll: f32,
    v: usize,
    block: Option<(usize, usize)>,
    hidden: f32,
) -> (Rect, Rect) {
    let y_of = |v: usize| tree.y + v as f32 * TREE_ROW - scroll;
    let mut row = Rect::new(tree.x, y_of(v), tree.w, TREE_ROW);
    let mut clip = tree;

    match block {
        Some((start, end)) if (start..end).contains(&v) => {
            let top = y_of(start).max(tree.y);
            let bottom = (y_of(end) - hidden).min(tree.bottom());
            clip = Rect::new(tree.x, top, tree.w, (bottom - top).max(0.0));
        }
        Some((_, end)) if v >= end => row.y -= hidden,
        _ => {}
    }

    (row, clip)
}

/// 编辑区可滚动超出最后一行的距离（可见高度的比例）。
///
/// 滚到底时最后一行位于可见高度的 60% 处，下方留出 40% 的空白；不允许滚得更远，
/// 避免可见范围内只剩空白。
pub const OVERSCROLL: f32 = 0.4;

/// 返回编辑区（以及差异、日志）的最大滚动位置（显示行，可以是小数）。
///
/// 内容放得下时为 0，不滚动；放不下时滚到底，最后一行下面留 [`OVERSCROLL`] 的空白。
pub fn editor_max_scroll(lines: usize, view_height: f32, line_height: f32) -> f64 {
    let (view_height, line_height) = (f64::from(view_height), f64::from(line_height));
    let content = lines as f64 * line_height;
    if content <= view_height || line_height <= 0.0 {
        return 0.0;
    }
    (content - view_height * (1.0 - f64::from(OVERSCROLL))) / line_height
}

/// 返回编辑区内容的总高度（逻辑像素），供滚动条使用：最大滚动距离加一屏的高度。
pub fn editor_content_height(lines: usize, view_height: f32, line_height: f32) -> f32 {
    (editor_max_scroll(lines, view_height, line_height) * f64::from(line_height)) as f32
        + view_height
}

/// 返回竖直滚动条的滑块位置；内容不超过可视高度时没有滑块，返回 `None`。
///
/// 滑块的活动范围比 `track` 两端各短 [`SCROLLBAR_END`]。
pub fn scrollbar_thumb(track: Rect, scroll: f32, content: f32) -> Option<Rect> {
    let (y, h) = thumb_span(track.y, track.h, scroll, content)?;
    let x = track.right() - SCROLLBAR_INSET - SCROLLBAR_THUMB;
    Some(Rect::new(x, y, SCROLLBAR_THUMB, h))
}

/// 概览标尺中 Git 改动一列标记的参数（离轨道右边的距离，宽度），逻辑像素。
///
/// 该列位于滑块左侧、与滑块相隔 1 逻辑像素，既不与滑块重叠，
/// 也不超出滚动条的可点击区域（[`SCROLLBAR_HIT`]）。
pub const GIT_LANE: (f32, f32) = (11.0, 3.0);

/// 概览标尺中查找结果、选中文字出现位置与书签一列标记的参数（离轨道右边的距离，宽度），
/// 逻辑像素。
///
/// 宽度与 [`GIT_LANE`] 相同，居中于滑块所在的一列；该列绘制为圆头，
/// [`GIT_LANE`] 绘制为直角细条（与行号栏的标记同形），二者形状可区分。
pub const MARK_LANE: (f32, f32) = (5.5, 3.0);

/// 概览标尺上单个标记的最小高度（逻辑像素）。
///
/// 高度不足该值时标记在轨道上太短，难以辨认，也难以点中。
pub const RULER_MIN: f32 = 4.0;

/// 编辑区滚动条上的概览标尺：把显示行换算成轨道上的位置，绘制和点击共用。
///
/// 两端与滑块相同，各留出 [`SCROLLBAR_END`]。
pub struct Ruler {
    top: f32,
    height: f32,
    right: f32,
    clip: Rect,
    total: usize,
}

impl Ruler {
    /// 轨道 `track` 上显示 `total` 个显示行的标尺。
    pub fn new(track: Rect, total: usize) -> Self {
        Ruler {
            top: track.y + SCROLLBAR_END,
            height: track.h - 2.0 * SCROLLBAR_END,
            right: track.right(),
            clip: track,
            total: total.max(1),
        }
    }

    /// 第 `row` 个显示行的上沿在轨道上的位置。
    fn y(&self, row: usize) -> f32 {
        self.top + row as f32 / self.total as f32 * self.height
    }

    /// 返回 `lane` 上从 `rows.start` 行到 `rows.end` 行的一段，至少 [`RULER_MIN`] 高；
    /// `rows` 为空时标记以该行的上沿为中心。
    pub fn rect(&self, lane: (f32, f32), rows: std::ops::Range<usize>) -> Rect {
        let (y0, y1) = (self.y(rows.start), self.y(rows.end));
        let h = (y1 - y0).max(RULER_MIN);
        let y = if rows.is_empty() { y0 - h / 2.0 } else { y0 };
        Rect::new(self.right - lane.0, y.round(), lane.1, h.round())
    }

    /// 在 `lane` 上绘制一段直角标记，形状与行号栏的标记相同（几何同 [`Ruler::rect`]）。
    pub fn mark(
        &self,
        frame: &mut Frame<'_>,
        lane: (f32, f32),
        rows: std::ops::Range<usize>,
        color: Color,
    ) {
        frame.rect_clipped(self.rect(lane, rows), color, 0.0, self.clip);
    }

    /// 在第 `row` 个显示行的中间绘制横跨两列标记、高 2 逻辑像素的细线，标出光标所在行。
    pub fn line(&self, frame: &mut Frame<'_>, row: usize, color: Color) {
        let y = (self.y(row) + self.y(row + 1)) / 2.0;
        let (left, right) = (
            self.right - GIT_LANE.0,
            self.right - MARK_LANE.0 + MARK_LANE.1,
        );
        let rect = Rect::new(left, (y - 1.0).round(), right - left, 2.0);
        frame.rect_clipped(rect, color, 1.0, self.clip);
    }

    /// 在 `lane` 上为每个显示行绘制 [`RULER_MIN`] 高的标记；间隔不到 1 逻辑像素的相邻标记
    /// 合并为一段，密集处显示为一道长条，而非多条重叠的短横。
    ///
    /// `rows` 必须按升序排列。
    pub fn runs(
        &self,
        frame: &mut Frame<'_>,
        lane: (f32, f32),
        rows: impl IntoIterator<Item = usize>,
        color: Color,
    ) {
        let mut run: Option<(f32, f32)> = None;
        let flush = |frame: &mut Frame<'_>, (y0, y1): (f32, f32)| {
            let rect = Rect::new(self.right - lane.0, y0.round(), lane.1, (y1 - y0).round());
            frame.rect_clipped(rect, color, lane.1 / 2.0, self.clip);
        };
        for row in rows {
            let y = self.y(row);
            let (y0, y1) = (y, y + (self.y(row + 1) - y).max(RULER_MIN));
            run = match run {
                Some((a, b)) if y0 <= b + 1.0 => Some((a, b.max(y1))),
                Some(done) => {
                    flush(frame, done);
                    Some((y0, y1))
                }
                None => Some((y0, y1)),
            };
        }
        if let Some(done) = run {
            flush(frame, done);
        }
    }
}

/// 是否位于某个控件所在的列：横坐标 `x` 落在 `controls` 中某个控件向两侧各扩 `gap` 后的
/// 横向范围内。
///
/// 顶栏空白处的双击只在控件所在的列之外切换窗口的最大化。控件正上方、正下方以及相邻控件之间的
/// 空白紧邻控件，落在这些位置的连续点击多为偏离控件的点击，不改变窗口状态。
pub fn in_control_column(controls: &[Rect], gap: f32, x: f32) -> bool {
    controls
        .iter()
        .any(|r| x >= r.x - gap && x < r.right() + gap)
}

/// 可以拖动的滚动条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrollbar {
    /// 编辑区的竖直滚动条。
    Editor,
    /// 编辑区的水平滚动条。
    EditorHorizontal,
    /// 差异标签左侧的水平滚动条。
    DiffLeftHorizontal,
    /// 差异标签右侧的水平滚动条；单栏时只有这一条。
    DiffRightHorizontal,
    /// 文件树的竖直滚动条。
    Tree,
    /// 文件树的水平滚动条。
    TreeHorizontal,
    /// 全局搜索结果列表的竖直滚动条。
    Panel,
    /// 提交视图改动列表的竖直滚动条。
    Commit,
    /// 文件视图底部大纲符号列表的竖直滚动条。
    Outline,
    /// 终端面板的竖直滚动条（回滚缓冲）。
    Terminal,
    /// Markdown 预览的竖直滚动条。
    Markdown,
    /// Markdown 预览中比正文宽的表格的水平滚动条。
    MarkdownTable,
    /// PDF 预览的竖直滚动条。
    Pdf,
    /// PDF 预览缩略图栏的竖直滚动条。
    PdfStrip,
    /// 设置页（含快捷键列表）的竖直滚动条。
    Settings,
}

/// 返回缩略图最上方显示的行：`total` 行超出 `shown` 行时，随滚动位置 `scroll`（最大为 `max`）
/// 按比例移动。
pub fn minimap_first_row(total: usize, shown: usize, scroll: f64, max: f64) -> usize {
    if total <= shown || max <= 0.0 {
        return 0;
    }
    ((scroll / max).clamp(0.0, 1.0) * (total - shown) as f64) as usize
}

/// 返回水平滚动条的滑块位置，紧贴 `track` 的下边；内容不超过可视宽度时没有滑块，返回 `None`。
///
/// 滑块的活动范围比 `track` 两端各短 [`SCROLLBAR_END`]。
pub fn hscrollbar_thumb(track: Rect, scroll: f32, content: f32) -> Option<Rect> {
    let (x, w) = thumb_span(track.x, track.w, scroll, content)?;
    let y = track.bottom() - SCROLLBAR_INSET - SCROLLBAR_THUMB;
    Some(Rect::new(x, y, w, SCROLLBAR_THUMB))
}

/// 返回滑块沿轨道方向的起点和长度（逻辑像素）。
///
/// `start`、`len` 是可视区域沿轨道方向的起点和长度。
///
/// 长度按「可见 / 内容」的比例计算，至少 24 逻辑像素，使长文档的滑块仍便于点中；活动范围
/// 不足 24 逻辑像素时取活动范围的长度。
fn thumb_span(start: f32, len: f32, scroll: f32, content: f32) -> Option<(f32, f32)> {
    if content <= len || len <= 0.0 {
        return None;
    }

    let (start, room) = thumb_room(start, len);
    let size = (room * len / content).clamp(24.0_f32.min(room), room);
    let offset = (scroll / (content - len)).clamp(0.0, 1.0) * (room - size);
    Some((start + offset, size))
}

/// 返回滑块的活动范围（起点和长度）：轨道两端各让出 [`SCROLLBAR_END`]。
fn thumb_room(start: f32, len: f32) -> (f32, f32) {
    (start + SCROLLBAR_END, (len - 2.0 * SCROLLBAR_END).max(0.0))
}

/// 返回拖动滑块时的新滚动位置；拖动过程中按下点在滑块内的偏移 `grab` 保持不变。
///
/// `start`、`len` 是轨道的起点和长度，`thumb` 是滑块长度，`pointer` 是指针坐标，`max` 是
/// 最大滚动量。滑块的活动范围与 [`scrollbar_thumb`] 相同。
pub fn scroll_for_thumb(
    start: f32,
    len: f32,
    thumb: f32,
    pointer: f32,
    grab: f32,
    max: f32,
) -> f32 {
    let (start, room) = thumb_room(start, len);
    ((pointer - grab - start) / (room - thumb).max(1.0)).clamp(0.0, 1.0) * max
}
