//! 全屏查看 mermaid 图的绘制。
//!
//! 图覆盖编辑区（分栏时包括右侧的预览）并居中显示。每个缩放倍数都重新排版而不是缩放位图，
//! 文字和线条在任何倍数下都保持清晰。右上角是缩放和关闭的工具栏；交互见
//! `app::preview::markdown`。

use std::collections::{HashMap, HashSet};

use winc_render::{Color, FontKind, Frame, Rect, Stroke};

use super::BASE_RATIO;
use super::layout::{self, Colors, Cx, Env, Item, Memo};
use crate::app::App;
use crate::app::preview::markdown::{DiagramTool, DiagramView, MAX_ZOOM, MIN_ZOOM};
use crate::icons;
use crate::theme::{Palette, RADIUS_POPUP};

/// 工具栏按钮的边长（逻辑像素），与预览中代码块的复制按钮相同。
const BUTTON: f32 = 28.0;
/// 工具栏中显示倍数的按钮的宽度（逻辑像素）：界面小字号下可容纳最宽的「400%」。
const PERCENT: f32 = 52.0;
/// 工具栏与查看区域右边、上边的距离（逻辑像素）。
const INSET: f32 = 12.0;
/// 工具栏按钮与工具栏边缘、按钮与分隔线之间的间距（逻辑像素）。
const PAD: f32 = 4.0;
/// 适应查看区域时图四周至少留出的空白（逻辑像素）：等于工具栏底边到查看区域上边的距离，
/// 图按适应倍数居中显示时不被工具栏遮住。
const MARGIN: f32 = INSET + BUTTON + PAD * 2.0;
/// 适应查看区域的倍数上限。
///
/// 取自调参：只有几个节点的小图放大到 2 倍时字号约 28 逻辑像素，继续放大只增加视线移动的
/// 距离，不再提高可读性。
const FIT_MAX: f32 = 2.0;

/// 按某个缩放倍数排版的图。
pub(crate) struct DiagramLaid {
    /// 排版依据：缩放倍数、正文字号、正文颜色、底色；任何一项改变时重新排版。
    key: (f32, f32, Color, Color),
    /// 节点上的文字是否折窄：倍数为 1 时两种折行各排版一次，取适应倍数较大的一种。
    narrow: bool,
    /// 排版得到的各项，图的左上角位于原点。
    items: Vec<Item>,
    /// 图的宽高（逻辑像素）。
    size: (f32, f32),
    /// 倍数为 1 时图的宽高（逻辑像素），用于计算适应查看区域的倍数。
    natural: (f32, f32),
    /// 计算 `natural` 时可用的查看区域大小（逻辑像素）；区域改变后重新选择是否折窄。
    room: (f32, f32),
}

/// 返回全屏查看的查看区域：编辑区，分栏时包括右侧的预览。
fn view_area(app: &App) -> Rect {
    let l = app.layout;
    if l.split.w > 0.0 {
        Rect::new(
            l.editor.x,
            l.editor.y,
            l.split.right() - l.editor.x,
            l.editor.h,
        )
    } else {
        l.editor
    }
}

/// 绘制全屏查看的图；不再显示该文档的预览（切换标签、切回源代码）时关闭全屏查看。
pub(in crate::view) fn draw_diagram_view(app: &mut App, frame: &mut Frame<'_>) {
    let Some(view) = app.markdown.diagram.as_ref() else {
        return;
    };
    let shown = app.markdown.area.w > 0.0 && app.editor.active().is_some_and(|d| d.id == view.doc);
    if !shown {
        app.close_diagram_view();
        return;
    }
    let area = view_area(app);
    let p = app.palette;
    frame.rect(area, p.editor_bg);

    let base = (app.fonts.code * BASE_RATIO).round();
    let Some(view) = app.markdown.diagram.take() else {
        return;
    };
    let mut view = lay_out(app, frame, view, base, area);
    view.area = area;
    if let Some(laid) = &view.laid {
        let (w, h) = laid.size;
        let left = (area.x + (area.w - w) / 2.0 + view.pan.0).round();
        let top = (area.y + (area.h - h) / 2.0 + view.pan.1).round();
        draw_items(frame, &laid.items, (left, top), area);
    }
    draw_toolbar(app, frame, p, &mut view, area);
    app.markdown.diagram = Some(view);
}

/// 计算适应查看区域的倍数，并按当前倍数排版图；倍数、字号、配色和查看区域都未改变时沿用
/// 上次的结果。
fn lay_out(
    app: &mut App,
    frame: &mut Frame<'_>,
    mut view: DiagramView,
    base: f32,
    area: Rect,
) -> DiagramView {
    let p = app.palette;
    let room = (
        (area.w - MARGIN * 2.0).max(1.0),
        (area.h - MARGIN * 2.0).max(1.0),
    );
    let fit_of = |(w, h): (f32, f32)| (room.0 / w.max(1.0)).min(room.1 / h.max(1.0));
    let (natural, narrow) = match &view.laid {
        Some(laid)
            if laid.key.1 == base
                && laid.key.2 == p.text
                && laid.key.3 == p.editor_bg
                && laid.room == room =>
        {
            (laid.natural, laid.narrow)
        }
        _ => {
            // 两种折行各排版一次，取适应倍数较大的一种：较宽的图折窄后可以按更大的字号显示。
            let mut size = |narrow| {
                lay_diagram(app, frame, &view.source, 1.0, narrow, base).map(|(_, w, h)| (w, h))
            };
            let Some(wide) = size(false) else {
                // 源码无法解析时不绘制图，仍绘制工具栏，以便关闭全屏查看。
                view.laid = None;
                return view;
            };
            match size(true) {
                Some(narrowed) if fit_of(narrowed) > fit_of(wide) => (narrowed, true),
                _ => (wide, false),
            }
        }
    };
    view.fit = fit_of(natural).clamp(MIN_ZOOM, FIT_MAX.min(MAX_ZOOM));
    let key = (view.scale(), base, p.text, p.editor_bg);
    if view
        .laid
        .as_ref()
        .is_none_or(|laid| laid.key != key || laid.room != room)
    {
        view.laid =
            lay_diagram(app, frame, &view.source, key.0, narrow, base).map(|(items, w, h)| {
                DiagramLaid {
                    key,
                    narrow,
                    items,
                    size: (w, h),
                    natural,
                    room,
                }
            });
    }
    view
}

/// 按正文字号 `base` 的 `zoom` 倍排版一张图（`narrow` 见 `Cx::diagram_alone`），返回各项和
/// 宽高；无法识别或不支持绘制时返回 `None`。
///
/// 使用独立的排版缓存：缩放时每个倍数都要重新排版，这些结果对正文的排版没有复用价值。
fn lay_diagram(
    app: &App,
    frame: &mut Frame<'_>,
    source: &str,
    zoom: f32,
    narrow: bool,
    base: f32,
) -> Option<(Vec<Item>, f32, f32)> {
    let p = app.palette;
    let images = HashMap::new();
    let toggled = HashSet::new();
    let mut memo = Memo::default();
    let mut cx = Cx {
        frame,
        base,
        palette: p,
        colors: Colors::new(p),
        dark: p.dark,
        images: &images,
        doc: None,
        wanted: Vec::new(),
        remote: false,
        memo: &mut memo,
        toggled: &toggled,
        text: String::new(),
        chars: 0,
        raw: None,
        cell: None,
        preedit: false,
    };
    let env = Env {
        color: cx.colors.text,
        depth: 0,
    };
    cx.diagram_alone(source, zoom, narrow, env)
}

/// 以 `(left, top)`（窗口坐标）为图的左上角绘制各项，跳过与 `clip` 不相交的项。
fn draw_items(frame: &mut Frame<'_>, items: &[Item], (left, top): (f32, f32), clip: Rect) {
    for item in items {
        let (y0, y1) = item.span_y();
        if top + y1 < clip.y || top + y0 > clip.bottom() {
            continue;
        }
        match item {
            Item::Text {
                x,
                baseline,
                text,
                font,
                faux,
                size,
                color,
                ..
            } => {
                frame.text_faux(
                    *font,
                    *faux,
                    *size,
                    left + x,
                    top + baseline,
                    text,
                    *color,
                    clip,
                );
            }
            Item::Fill {
                rect,
                color,
                radius,
            } => {
                let r = Rect::new(left + rect.x, top + rect.y, rect.w, rect.h);
                frame.rect_clipped(r, *color, *radius, clip);
            }
            Item::Corners { rect, color, radii } => {
                let r = Rect::new(left + rect.x, top + rect.y, rect.w, rect.h);
                frame.rect_corners(r, *color, *radii, clip);
            }
            Item::Path {
                strokes,
                width,
                color,
                ..
            } => {
                let moved: Vec<Stroke> = strokes
                    .iter()
                    .map(|&s| layout::shift(s, left, top))
                    .collect();
                frame.stroke(&moved, *width, *color, clip);
            }
            Item::Polygon { points, color, .. } => {
                let moved: Vec<(f32, f32)> =
                    points.iter().map(|&(x, y)| (x + left, y + top)).collect();
                frame.polygon(&moved, *color, clip);
            }
            _ => {}
        }
    }
}

/// 绘制右上角的工具栏：缩小、当前倍数（点击后恢复适应倍数）、放大、关闭；按钮位置记入
/// `view.buttons`。
fn draw_toolbar(app: &App, frame: &mut Frame<'_>, p: &Palette, view: &mut DiagramView, area: Rect) {
    let tools = [
        (DiagramTool::ZoomOut, BUTTON),
        (DiagramTool::Fit, PERCENT),
        (DiagramTool::ZoomIn, BUTTON),
        (DiagramTool::Close, BUTTON),
    ];
    // 关闭按钮与缩放按钮之间以竖线分隔。
    let gap = PAD * 2.0 + 1.0;
    let inner: f32 = tools.iter().map(|&(_, w)| w).sum::<f32>() + gap;
    let bar = Rect::new(
        area.right() - INSET - inner - PAD * 2.0,
        area.y + INSET,
        inner + PAD * 2.0,
        BUTTON + PAD * 2.0,
    );
    let covered = frame.covered(bar, RADIUS_POPUP);
    frame.shadow(bar, RADIUS_POPUP, 12.0, 4.0, p.shadow, Some(covered));
    frame.rect_clipped(bar, p.popup_border, RADIUS_POPUP, area);
    frame.rect_clipped(bar.inset(1.0, 1.0), p.popup_bg, RADIUS_POPUP - 1.0, area);

    view.buttons.clear();
    let mut x = bar.x + PAD;
    for (tool, w) in tools {
        if tool == DiagramTool::Close {
            let line = Rect::new(x + PAD, bar.y + PAD * 2.0, 1.0, bar.h - PAD * 4.0);
            frame.rect_clipped(line, p.separator, 0.0, area);
            x += gap;
        }
        let r = Rect::new(x, bar.y + PAD, w, BUTTON);
        if view.hover == Some(tool) {
            frame.rect_clipped(r, p.hover, RADIUS_POPUP - PAD, area);
        }
        let center = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let color = p.text_muted;
        match tool {
            DiagramTool::ZoomOut => icons::minus(frame, center, 10.0, color, area),
            DiagramTool::ZoomIn => icons::plus(frame, center, 10.0, color, area),
            DiagramTool::Close => icons::close(frame, center, 9.0, color, area),
            DiagramTool::Fit => {
                let size = app.fonts.ui_small;
                let label = format!("{:.0}%", view.scale() * 100.0);
                let tw = frame.measure(FontKind::Ui, size, &label);
                let baseline = crate::view::ui_baseline(app, r, size);
                frame.text(
                    FontKind::Ui,
                    size,
                    (center.0 - tw / 2.0).round(),
                    baseline,
                    &label,
                    p.text,
                    area,
                );
            }
        }
        view.buttons.push((r, tool));
        x += w;
    }
}
