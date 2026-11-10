//! 全屏查看 mermaid 图的状态与交互。
//!
//! 图覆盖整个编辑区：拖动或滚动平移；捏合，或按住 `⌘`（其他系统为 `Ctrl`）滚动，以鼠标处为
//! 中心缩放；`Esc` 关闭。绘制见 `view::preview::markdown` 的 `draw_diagram_view`。

use std::sync::Arc;

use winc_editor::DocumentId;
use winc_render::{Color, Rect};
use winc_typeset::Item;
use winit::keyboard::{Key, NamedKey};

use crate::app::App;

/// 缩放倍数的下限（相对预览正文中图的默认字号）。
///
/// 取自调参：倍数为 0.25 时图中的字号约 3.5 逻辑像素，文字已无法辨认，更小的倍数只能看到
/// 连线的走向。
pub(crate) const MIN_ZOOM: f32 = 0.25;
/// 缩放倍数的上限（相对预览正文中图的默认字号）。
///
/// 取自调参：倍数为 4 时图中的字号约 56 逻辑像素，单个节点已接近查看区域的宽度。
pub(crate) const MAX_ZOOM: f32 = 4.0;
/// 放大、缩小按钮（以及 `+`、`-` 键）每次缩放的倍数：连按 3 次约为 2 倍。
const ZOOM_STEP: f32 = 1.25;
/// 滚动缩放时，每逻辑像素滚动量对应的缩放倍数的自然对数。
///
/// 取自调参：滚动 100 逻辑像素缩放约 1.6 倍，触控板一次滑动的缩放幅度与按钮缩放两级相近。
const WHEEL_ZOOM: f32 = 0.005;

/// 全屏查看时右上角工具栏的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagramTool {
    /// 缩小一级。
    ZoomOut,
    /// 显示当前倍数；点击后恢复适应查看区域的倍数，图居中。
    Fit,
    /// 放大一级。
    ZoomIn,
    /// 关闭全屏查看。
    Close,
}

/// 全屏查看中的 mermaid 图。
pub(crate) struct DiagramView {
    /// 图所在的文档；切换到其他标签或不再显示预览时关闭全屏查看。
    pub doc: DocumentId,
    /// 图的源码。
    pub source: Arc<str>,
    /// 缩放倍数（相对预览正文中图的默认字号）。
    ///
    /// 为 `None` 时取适应查看区域的倍数 `fit`，窗口大小改变后随之更新。
    pub zoom: Option<f32>,
    /// 适应查看区域的倍数，绘制时由图的大小和查看区域的大小计算。
    pub fit: f32,
    /// 图的中心相对查看区域中心的偏移（逻辑像素）。
    pub pan: (f32, f32),
    /// 上一帧的查看区域（窗口坐标，逻辑像素）。
    pub area: Rect,
    /// 上一帧工具栏各按钮的位置。
    pub buttons: Vec<(Rect, DiagramTool)>,
    /// 鼠标所在的按钮。
    pub hover: Option<DiagramTool>,
    /// 拖动平移的起点：按下时鼠标的位置（窗口坐标）和当时的 `pan`；未在拖动时为 `None`。
    pub grab: Option<((f32, f32), (f32, f32))>,
    /// 按当前倍数排版的结果；倍数、字号、配色或查看区域改变时重新排版。
    pub laid: Option<DiagramLaid>,
}

impl DiagramView {
    /// 当前的缩放倍数。
    pub(crate) fn scale(&self) -> f32 {
        self.zoom.unwrap_or(self.fit)
    }

    /// 返回 `(x, y)`（窗口坐标）处的工具栏按钮。
    fn tool_at(&self, x: f32, y: f32) -> Option<DiagramTool> {
        self.buttons
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, tool)| tool)
    }
}

impl App {
    /// 全屏查看上一帧绘制了全屏按钮的图（`MarkdownPreview::expand`）。
    pub(crate) fn open_diagram_view(&mut self) {
        let (Some(source), Some(doc)) = (
            self.hits.markdown_expand.clone(),
            self.editor.active().map(|d| d.id),
        ) else {
            return;
        };
        self.markdown.diagram = Some(DiagramView {
            doc,
            source,
            zoom: None,
            fit: 1.0,
            pan: (0.0, 0.0),
            area: Rect::default(),
            buttons: Vec::new(),
            hover: None,
            grab: None,
            laid: None,
        });
        self.request_redraw();
    }

    /// 关闭全屏查看。
    pub(crate) fn close_diagram_view(&mut self) {
        if self.markdown.diagram.take().is_some() {
            self.request_redraw();
            self.update_cursor_icon();
        }
    }

    /// 把缩放倍数乘以 `factor`，结果限制在 [`MIN_ZOOM`] 与 [`MAX_ZOOM`] 之间。
    ///
    /// 缩放中心为 `around`（窗口坐标），为 `None` 时取查看区域的中心；缩放中心处的内容位置不变。
    pub(crate) fn zoom_diagram(&mut self, factor: f32, around: Option<(f32, f32)>) {
        let Some(view) = self.markdown.diagram.as_mut() else {
            return;
        };
        let from = view.scale();
        let to = (from * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let center = (
            view.area.x + view.area.w / 2.0,
            view.area.y + view.area.h / 2.0,
        );
        // 图按倍数等比缩放，各点相对缩放中心的偏移同比例变化，因此缩放中心处的内容位置不变。
        let (cx, cy) = around.map_or((0.0, 0.0), |(x, y)| (x - center.0, y - center.1));
        let k = to / from;
        view.pan = (cx + (view.pan.0 - cx) * k, cy + (view.pan.1 - cy) * k);
        view.zoom = Some(to);
        self.request_redraw();
    }

    /// 恢复适应查看区域的倍数，图居中。
    fn fit_diagram(&mut self) {
        if let Some(view) = self.markdown.diagram.as_mut() {
            view.zoom = None;
            view.pan = (0.0, 0.0);
            self.request_redraw();
        }
    }

    /// 处理全屏查看时在 `(x, y)`（窗口坐标）按下左键：按在工具栏按钮上时执行该按钮，其余位置
    /// 开始拖动平移。
    ///
    /// 返回是否已处理；按在查看区域以外（标签栏、侧栏）时返回 `false`，由调用方按常规处理。
    pub(crate) fn diagram_view_mouse_down(&mut self, x: f32, y: f32) -> bool {
        let Some(view) = self.markdown.diagram.as_mut() else {
            return false;
        };
        if !view.area.contains(x, y) {
            return false;
        }
        match view.tool_at(x, y) {
            Some(DiagramTool::ZoomOut) => self.zoom_diagram(1.0 / ZOOM_STEP, None),
            Some(DiagramTool::ZoomIn) => self.zoom_diagram(ZOOM_STEP, None),
            Some(DiagramTool::Fit) => self.fit_diagram(),
            Some(DiagramTool::Close) => self.close_diagram_view(),
            None => {
                view.grab = Some(((x, y), view.pan));
                self.update_cursor_icon();
            }
        }
        true
    }

    /// 处理全屏查看时鼠标移到 `(x, y)`（窗口坐标）：拖动中平移图，否则更新悬停的按钮。
    ///
    /// 鼠标位于查看区域内或正在拖动时返回 `true`。
    pub(crate) fn diagram_view_mouse_move(&mut self, x: f32, y: f32) -> bool {
        let Some(view) = self.markdown.diagram.as_mut() else {
            return false;
        };
        if let Some(((x0, y0), (px, py))) = view.grab {
            view.pan = (px + x - x0, py + y - y0);
            self.request_redraw();
            return true;
        }
        let hover = view.tool_at(x, y);
        let inside = view.area.contains(x, y);
        if hover != view.hover {
            view.hover = hover;
            self.request_redraw();
        }
        if inside {
            self.update_cursor_icon();
        }
        inside
    }

    /// 松开左键时结束拖动平移；返回松开前是否在拖动。
    pub(crate) fn diagram_view_mouse_up(&mut self) -> bool {
        let grabbed = self
            .markdown
            .diagram
            .as_mut()
            .is_some_and(|v| v.grab.take().is_some());
        if grabbed {
            self.update_cursor_icon();
        }
        grabbed
    }

    /// 处理全屏查看时的滚动 `(dx, dy)`（逻辑像素）：按住 `⌘`（其他系统为 `Ctrl`）时以鼠标处为
    /// 中心缩放，否则平移。
    ///
    /// 鼠标位于查看区域内时返回 `true`。
    pub(crate) fn diagram_view_wheel(&mut self, dx: f32, dy: f32) -> bool {
        let (x, y) = self.mouse;
        let Some(view) = self.markdown.diagram.as_mut() else {
            return false;
        };
        if !view.area.contains(x, y) {
            return false;
        }
        let zooming = if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        };
        if zooming {
            self.zoom_diagram((dy * WHEEL_ZOOM).exp(), Some((x, y)));
        } else {
            view.pan = (view.pan.0 + dx, view.pan.1 + dy);
            self.request_redraw();
        }
        true
    }

    /// 处理触控板捏合 `delta`（放大为正）：鼠标位于查看区域内时以鼠标处为中心缩放。
    pub(crate) fn diagram_view_pinch(&mut self, delta: f32) {
        let (x, y) = self.mouse;
        if self
            .markdown
            .diagram
            .as_ref()
            .is_some_and(|v| v.area.contains(x, y))
        {
            self.zoom_diagram(1.0 + delta, Some((x, y)));
        }
    }

    /// 处理全屏查看时的按键：`Esc` 关闭，`+`、`-` 缩放，`0` 恢复适应查看区域的倍数；是否同时
    /// 按下 `cmd` 不影响结果。
    ///
    /// 返回是否已处理。焦点在编辑区时，其余不带 `cmd` 的按键同样视为已处理：图遮住了文字，
    /// 输入不应写入不可见的文档。带 `cmd` 的快捷键返回 `false`，按常规处理。
    pub(crate) fn diagram_view_key(&mut self, key: &Key, base: &str, cmd: bool) -> bool {
        if self.markdown.diagram.is_none() {
            return false;
        }
        match (key, base) {
            (Key::Named(NamedKey::Escape), _) => self.close_diagram_view(),
            (_, "=" | "+") => self.zoom_diagram(ZOOM_STEP, None),
            (_, "-") => self.zoom_diagram(1.0 / ZOOM_STEP, None),
            (_, "0") => self.fit_diagram(),
            _ => return !cmd && self.focus == crate::app::Focus::Editor,
        }
        true
    }
}

/// 按某个缩放倍数排版的图。
pub(crate) struct DiagramLaid {
    /// 排版依据：缩放倍数、正文字号、正文颜色、底色；任何一项改变时重新排版。
    pub(crate) key: (f32, f32, Color, Color),
    /// 节点上的文字是否折窄：倍数为 1 时两种折行各排版一次，取适应倍数较大的一种。
    pub(crate) narrow: bool,
    /// 排版得到的各项，图的左上角位于原点。
    pub(crate) items: Vec<Item>,
    /// 图的宽高（逻辑像素）。
    pub(crate) size: (f32, f32),
    /// 倍数为 1 时图的宽高（逻辑像素），用于计算适应查看区域的倍数。
    pub(crate) natural: (f32, f32),
    /// 计算 `natural` 时可用的查看区域大小（逻辑像素）；区域改变后重新选择是否折窄。
    pub(crate) room: (f32, f32),
}
