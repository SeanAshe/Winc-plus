//! 滚动条的自动隐藏：平时不显示，滚动后显示 [`SCROLLBAR_HOLD`] 后淡出；鼠标移到滚动条所在的
//! 位置、或正在拖动滚动条时一直显示，颜色也加深。
//!
//! 每个带滚动条的区域对应一个 [`ScrollArea`]。[`App::scrollbar_look`] 取得滑块此刻的样子，并与
//! 上一帧的滚动位置比较，位置变了就重新计时，滚轮、键盘、查找跳转都会让滚动条出现；保持阶段结束
//! 时的唤醒由 [`App::scrollbar_tick`] 安排，过渡期间由 [`App::animating`] 逐帧重绘。

use std::time::{Duration, Instant};

use winc_render::Color;

use super::*;

/// 滚动停止（或鼠标离开滚动条）后，滚动条保持完全显示的时长（经验值）。
const SCROLLBAR_HOLD: Duration = Duration::from_millis(800);
/// 滚动条淡入的时长（经验值）：比淡出短，滚动时马上有反馈。
const SCROLLBAR_SHOW: Duration = Duration::from_millis(150);
/// 滚动条淡出的时长（经验值）。
const SCROLLBAR_HIDE: Duration = Duration::from_millis(300);
/// 鼠标移上、移开滚动条时，滑块颜色加深、变回的时长（经验值）。
const SCROLLBAR_HEAT: Duration = Duration::from_millis(150);

/// 带滚动条的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollArea {
    /// 编辑区的竖直滚动条。
    Editor,
    /// 编辑区的水平滚动条。
    EditorHorizontal,
    /// 文件树的竖直滚动条。
    Tree,
    /// 文件树的水平滚动条。
    TreeHorizontal,
    /// 全局搜索的结果列表。
    Panel,
    /// 提交视图的改动列表。
    Commit,
    /// 文件视图底部大纲的符号列表。
    Outline,
    /// 设置页的内容，包括快捷键列表。
    Settings,
    /// 终端面板的回滚缓冲。
    Terminal,
    /// Markdown 预览。
    Markdown,
    /// Markdown 预览中比正文宽的表格的水平滚动条。
    MarkdownTable,
}

impl ScrollArea {
    /// 全部区域，遍历时用。
    const ALL: [ScrollArea; 11] = [
        ScrollArea::Editor,
        ScrollArea::EditorHorizontal,
        ScrollArea::Tree,
        ScrollArea::TreeHorizontal,
        ScrollArea::Panel,
        ScrollArea::Commit,
        ScrollArea::Outline,
        ScrollArea::Settings,
        ScrollArea::Terminal,
        ScrollArea::Markdown,
        ScrollArea::MarkdownTable,
    ];

    /// 该区域可以拖动的滚动条；不能拖动时为 `None`。
    pub(crate) fn bar(self) -> Option<Scrollbar> {
        match self {
            ScrollArea::Editor => Some(Scrollbar::Editor),
            ScrollArea::EditorHorizontal => Some(Scrollbar::EditorHorizontal),
            ScrollArea::Tree => Some(Scrollbar::Tree),
            ScrollArea::TreeHorizontal => Some(Scrollbar::TreeHorizontal),
            ScrollArea::Panel => Some(Scrollbar::Panel),
            ScrollArea::Commit => Some(Scrollbar::Commit),
            ScrollArea::Outline => Some(Scrollbar::Outline),
            ScrollArea::Settings => Some(Scrollbar::Settings),
            ScrollArea::Terminal => Some(Scrollbar::Terminal),
            ScrollArea::Markdown => Some(Scrollbar::Markdown),
            ScrollArea::MarkdownTable => Some(Scrollbar::MarkdownTable),
        }
    }

    fn horizontal(self) -> bool {
        matches!(
            self,
            ScrollArea::EditorHorizontal | ScrollArea::TreeHorizontal | ScrollArea::MarkdownTable
        )
    }
}

/// 从 `from` 过渡到 `to` 的一段动画；没有时钟（不需要过渡，或系统开启了减弱动态效果）时直接
/// 取终点。
#[derive(Debug, Clone, Copy, Default)]
struct Tween {
    from: f32,
    to: f32,
    clock: Option<Clock>,
}

impl Tween {
    fn value(&self) -> f32 {
        self.clock
            .map_or(self.to, |clock| clock.between(self.from, self.to))
    }

    /// 过渡是否还没有结束。
    fn running(&self) -> bool {
        self.clock.is_some_and(|clock| !clock.done())
    }
}

/// 一个区域的滚动条显示状态。
#[derive(Debug, Clone, Copy, Default)]
struct Reveal {
    /// 上一帧绘制时区域中显示的对象和滚动位置，用来发现滚动。
    seen: Option<(u64, f32)>,
    /// 开始计时的时刻：最近一次滚动，或鼠标离开、拖动结束的时刻。
    ///
    /// 此后滚动条保持显示 [`SCROLLBAR_HOLD`]，到时由 [`App::scrollbar_tick`] 清除。
    since: Option<Instant>,
    /// 上一帧是否因为悬停或拖动而一直显示。
    held: bool,
    /// 本帧绘制的可滚动区域；内容放得下、没有滑块，或本帧没有绘制时为 `None`。
    area: Option<Rect>,
    opacity: Tween,
    /// 颜色加深程度的过渡。
    heat: Tween,
}

/// 各区域滚动条的显示状态，以及鼠标所在的滚动条。
#[derive(Debug, Default)]
pub(crate) struct Scrollbars {
    reveals: [Reveal; ScrollArea::ALL.len()],
    /// 鼠标所在的滚动条：位于某个区域靠右边缘（水平滚动条为下边缘）[`SCROLLBAR_HIT`] 的范围内。
    hovered: Option<ScrollArea>,
}

/// 滚动条滑块此刻的样子。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ThumbLook {
    /// 不透明度：0 为完全隐藏，1 为完全显示。
    pub opacity: f32,
    /// 颜色加深的程度：0 为平时的颜色，1 为鼠标在滚动条上或正在拖动时的颜色。
    pub heat: f32,
}

impl ThumbLook {
    /// 返回滑块的颜色：在平时的 `normal` 和加深的 `active` 之间按加深程度插值，再乘以不透明度。
    pub(crate) fn color(self, normal: Color, active: Color) -> Color {
        let mix = |a: f32, b: f32| a + (b - a) * self.heat;
        Color {
            r: mix(normal.r, active.r),
            g: mix(normal.g, active.g),
            b: mix(normal.b, active.b),
            a: mix(normal.a, active.a) * self.opacity,
        }
    }
}

impl App {
    /// 开始绘制新的一帧：清除上一帧记下的可滚动区域，由本帧绘制的滚动条重新登记。
    pub(crate) fn begin_scrollbars(&mut self) {
        self.text_content_w = 0.0;
        for reveal in &mut self.scrollbars.reveals {
            reveal.area = None;
        }
    }

    /// 返回区域 `area` 的滚动条滑块此刻的样子，并记下本帧的滚动位置。
    ///
    /// `owner` 标识区域中显示的对象（编辑区的文档、设置的分类页）：对象变了不当作滚动，滚动条
    /// 立即隐藏。`rect` 是可滚动的区域，鼠标位于其靠边的 [`SCROLLBAR_HIT`] 范围内时滚动条一直显示；
    /// 内容放得下、没有滑块时传入 `None`。
    pub(crate) fn scrollbar_look(
        &mut self,
        area: ScrollArea,
        owner: u64,
        scroll: f32,
        rect: Option<Rect>,
    ) -> ThumbLook {
        let now = Instant::now();
        let active = area.bar().is_some_and(|bar| self.scrollbar_active(bar));
        let held = self.scrollbars.hovered == Some(area) || active;
        let reveal = &mut self.scrollbars.reveals[area as usize];

        let changed = reveal.seen.is_some_and(|(o, _)| o != owner);
        let scrolled = matches!(reveal.seen, Some((o, s)) if o == owner && s != scroll);
        // 换了显示的对象（例如切换文档）时，上一个对象的滚动条立即隐藏，不做过渡。鼠标离开或
        // 拖动结束后，滚动条先保持显示 [`SCROLLBAR_HOLD`] 再淡出，与滚动停止后相同。
        if changed {
            *reveal = Reveal::default();
        }
        if scrolled || (reveal.held && !held) {
            reveal.since = Some(now);
        }
        reveal.seen = Some((owner, scroll));
        reveal.held = held;
        reveal.area = rect;

        let visible =
            rect.is_some() && (held || reveal.since.is_some_and(|s| now < s + SCROLLBAR_HOLD));
        let (opacity, heat) = (reveal.opacity, reveal.heat);
        let duration = if visible {
            SCROLLBAR_SHOW
        } else {
            SCROLLBAR_HIDE
        };
        let opacity = self.retarget(opacity, if visible { 1.0 } else { 0.0 }, duration);
        let heat = self.retarget(heat, if active { 1.0 } else { 0.0 }, SCROLLBAR_HEAT);

        let reveal = &mut self.scrollbars.reveals[area as usize];
        reveal.opacity = opacity;
        reveal.heat = heat;
        ThumbLook {
            opacity: opacity.value(),
            heat: heat.value(),
        }
    }

    /// 让 `tween` 从此刻的值过渡到 `to`：走完整个 0 到 1 用时 `full`，只走一部分时按比例缩短。
    ///
    /// 终点没变时原样返回。
    fn retarget(&mut self, tween: Tween, to: f32, full: Duration) -> Tween {
        if tween.to == to {
            return tween;
        }
        let from = tween.value();
        let distance = (to - from).abs();
        let clock = if distance > 0.01 {
            self.clock(full.mul_f32(distance))
        } else {
            None
        };
        Tween { from, to, clock }
    }

    /// 根据鼠标位置 (`x`, `y`)（逻辑像素）更新鼠标所在的滚动条；变化时重绘。
    ///
    /// 设置浮层打开时只认设置页的滚动条，浮层下方的区域不响应。
    pub(in crate::app) fn update_scrollbar_hover(&mut self, x: f32, y: f32) {
        let settings = self.settings.open;
        let dialog = self.form_dialog.is_some();
        let hovered = ScrollArea::ALL.into_iter().find(|&area| {
            if dialog || settings != (area == ScrollArea::Settings) {
                return false;
            }
            // 鼠标在展开了完整名称的一行上：不唤出文件树的滚动条。
            if area == ScrollArea::Tree && self.over_tree_overflow_row(y) {
                return false;
            }
            if area == ScrollArea::Commit && self.over_commit_overflow_row(y) {
                return false;
            }
            self.scrollbars.reveals[area as usize]
                .area
                .is_some_and(|r| {
                    r.contains(x, y)
                        && if area.horizontal() {
                            y >= r.bottom() - SCROLLBAR_HIT
                        } else {
                            x >= r.right() - SCROLLBAR_HIT
                        }
                })
        });

        if hovered != self.scrollbars.hovered {
            self.scrollbars.hovered = hovered;
            self.request_redraw();
        }
    }

    /// 保持阶段结束时清除计时并重绘，开始淡出；返回下一次需要检查的时刻。
    pub(in crate::app) fn scrollbar_tick(&mut self, now: Instant) -> Option<Instant> {
        let mut wake: Option<Instant> = None;
        let mut redraw = false;

        for reveal in &mut self.scrollbars.reveals {
            let Some(since) = reveal.since else {
                continue;
            };
            let end = since + SCROLLBAR_HOLD;

            if now < end {
                wake = Some(wake.map_or(end, |w| w.min(end)));
            } else {
                reveal.since = None;
                redraw = true;
            }
        }

        if redraw {
            self.request_redraw();
        }
        wake
    }

    /// 是否有滚动条正在淡入、淡出或变色，需要逐帧重绘。
    pub(in crate::app) fn scrollbars_animating(&self) -> bool {
        self.scrollbars
            .reveals
            .iter()
            .any(|reveal| reveal.opacity.running() || reveal.heat.running())
    }
}

#[cfg(test)]
mod tests;
