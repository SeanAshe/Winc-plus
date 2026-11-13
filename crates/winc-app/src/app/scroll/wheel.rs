//! 滚轮与触控板的滚动：换算滚动量、按手势锁定方向、鼠标滚轮的过渡，以及按屏幕刷新安排绘制。
//!
//! 按住主修饰键时的缩放与调整字号在 `input/mouse.rs` 中处理，其余的滚轮事件交给
//! [`App::on_wheel`] 或带手势信息的 [`App::scroll_by_wheel`]。

use super::*;

use winit::event::{MouseScrollDelta, TouchPhase};

/// 鼠标滚轮滚动一格的距离（逻辑像素）；macOS 上不使用。
///
/// 依据滚轮一格的增量：Windows 的 `WHEEL_DELTA` 和 Wayland 的 `axis_value120` 都将一格记为 120，
/// 每单位合 1 逻辑像素。
#[cfg(not(target_os = "macos"))]
const WHEEL_NOTCH: f32 = 120.0;
/// Windows 上滚轮一格默认滚动的行数（行），依据 `SPI_GETWHEELSCROLLLINES` 的默认值。
#[cfg(windows)]
const WHEEL_LINES: f32 = 3.0;

/// 两次触控板滚动事件的间隔超过该值时视为新的手势（毫秒）。
///
/// 取自调参：用于没有手势阶段信息的事件，远长于手势进行中相邻事件的间隔。
pub(in crate::app) const WHEEL_GESTURE_GAP: Duration = Duration::from_millis(150);

/// 最近一次触控板或触摸滚动之后保持逐帧绘制的时长（毫秒）。
///
/// 长于 60 Hz 屏幕的一帧（16.7 毫秒）：事件按刷新的节奏送达时，手势进行中显示链接不在两次事件
/// 之间暂停；手势结束后在 120 Hz 屏幕上至多多绘制 3 帧。
const SCROLL_INPUT_HOLD: Duration = Duration::from_millis(20);

/// 鼠标滚轮过渡的最短时长（毫秒）。
///
/// 等于 60 Hz 屏幕上的 3 帧：更短时过渡只有一至两帧，与直接滚动没有区别。
const WHEEL_GLIDE_MIN: Duration = Duration::from_millis(50);
/// 鼠标滚轮过渡时长与两格间隔之比。
///
/// 由缓出曲线决定：经过一半时长时已完成 87.5%。匀速转动滚轮时，每格到来之前尚未完成的滚动
/// 稳定在一格的七分之一左右，画面不会越来越落后于滚轮。
const WHEEL_GLIDE_RATIO: u32 = 2;

/// 返回鼠标滚轮一格的过渡时长：与上一格的间隔 `interval` 的 [`WHEEL_GLIDE_RATIO`] 倍，限制在
/// [`WHEEL_GLIDE_MIN`] 与 `SCROLL_GLIDE` 之间；没有上一格时为 `SCROLL_GLIDE`。
///
/// 快速转动滚轮时各格间隔短，过渡随之缩短，画面紧随滚轮；单独的一格按完整时长过渡。
pub(in crate::app) fn wheel_glide_length(interval: Option<Duration>) -> Duration {
    let full = crate::app::animation::SCROLL_GLIDE;
    interval.map_or(full, |interval| {
        (interval * WHEEL_GLIDE_RATIO).clamp(WHEEL_GLIDE_MIN, full)
    })
}

/// 触控板手势锁定的滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    /// 只横向滚动。
    Horizontal,
    /// 只竖直滚动。
    Vertical,
}

impl Axis {
    /// 返回位移 (`dx`, `dy`) 的主方向；两者相等时取竖直方向。
    fn of(dx: f32, dy: f32) -> Axis {
        if dy.abs() >= dx.abs() {
            Axis::Vertical
        } else {
            Axis::Horizontal
        }
    }

    /// 只保留位移 (`dx`, `dy`) 中本方向的分量。
    fn keep(self, dx: f32, dy: f32) -> (f32, f32) {
        match self {
            Axis::Vertical => (0.0, dy),
            Axis::Horizontal => (dx, 0.0),
        }
    }
}

/// 鼠标滚轮单次滚动的过渡状态：一格的距离分多帧完成。
///
/// 开始时记下滚动的区域，各帧都滚动该区域；鼠标移到其他区域后不改变。
#[derive(Debug, Clone)]
pub struct WheelGlide {
    /// 本次过渡的总距离（横，纵），单位为逻辑像素。
    pub total: (f32, f32),
    /// 已完成的比例，取值 0 到 1。
    pub done: f32,
    /// 本次过渡的时钟。
    pub clock: Clock,
    /// 滚动的区域。
    pub(crate) target: ScrollTarget,
    /// 开始时鼠标的位置（逻辑像素），见 [`App::scroll_target_by`]。
    pub(crate) origin: (f32, f32),
    /// 是否按住 `Shift`：竖直滚动按横向处理。
    pub(crate) shift: bool,
}

impl WheelGlide {
    /// 返回尚未完成的距离（横，纵）（逻辑像素）。
    fn rest(&self) -> (f32, f32) {
        let left = 1.0 - self.done;
        (self.total.0 * left, self.total.1 * left)
    }
}

/// 滚轮与触控板滚动的状态。
#[derive(Default)]
pub struct Wheel {
    /// 鼠标滚轮滚动的过渡状态；没有进行中的过渡时为 `None`。
    pub(crate) glide: Option<WheelGlide>,
    /// 上一次鼠标滚轮整格滚动的时刻，用于按两格的间隔确定过渡时长。
    pub(in crate::app) at: Option<Instant>,
    /// 调整字号时累计的滚动量（档），满一档调整一次。
    pub(in crate::app) zoom: f32,
    /// 当前的触控板手势是否用于调整字号。
    pub(in crate::app) zooming: bool,
    /// 上一次触控板滚动事件的时刻，用于区分两次手势。
    pub(in crate::app) pixel_at: Option<Instant>,
    /// 当前触控板手势锁定的方向；手势尚未产生位移时为 `None`。
    pub(crate) axis: Option<Axis>,
    /// 最近一次触控板或触摸滚动的时刻，见 [`SCROLL_INPUT_HOLD`]。
    pub(in crate::app) input_at: Option<Instant>,
    /// 编辑区横向滚动中尚未体现在滚动位置上的距离（逻辑像素）。
    pub(in crate::app) carry_x: f64,
}

impl App {
    /// 记录一次滚轮事件 `delta` 的手势阶段 `phase`，返回是否开始了新的触控板手势。
    ///
    /// 触控板的事件带 `TouchPhase::Started`，或与上一事件的间隔超过 [`WHEEL_GESTURE_GAP`] 时为新
    /// 手势，此时解除方向的锁定。鼠标滚轮的事件返回 `false`。
    pub(in crate::app) fn note_wheel_gesture(
        &mut self,
        delta: MouseScrollDelta,
        phase: TouchPhase,
    ) -> bool {
        let MouseScrollDelta::PixelDelta(_) = delta else {
            return false;
        };
        let now = Instant::now();
        let last = self.scroll.wheel.pixel_at.replace(now);
        let fresh = phase == TouchPhase::Started
            || last.is_none_or(|at| now.duration_since(at) > WHEEL_GESTURE_GAP);
        if fresh {
            self.scroll.wheel.axis = None;
        }
        fresh
    }

    /// 处理滚轮事件 `delta`：滚动鼠标所在区域对应的内容，每个事件只按位移较大的方向滚动。
    ///
    /// 用于没有手势信息的来源（触摸滚动）；窗口的滚轮事件经 [`Self::scroll_by_wheel`] 处理。
    pub(in crate::app) fn on_wheel(&mut self, delta: MouseScrollDelta) {
        self.scroll_by_wheel(delta, false);
    }

    /// 处理滚轮事件 `delta`：滚动鼠标所在区域对应的内容。
    ///
    /// `gesture` 为 `true` 时触控板的滚动在整个手势中只沿手势开始时的主方向（见
    /// [`Self::note_wheel_gesture`]）；鼠标滚轮按每个事件的主方向。鼠标滚轮的整格滚动分多帧完成
    /// （[`WheelGlide`]），触控板的滚动量立即生效。
    pub(in crate::app) fn scroll_by_wheel(&mut self, delta: MouseScrollDelta, gesture: bool) {
        self.dismiss_hint();
        let scale = self
            .frame
            .renderer
            .as_ref()
            .map_or(1.0, Renderer::scale_factor);
        let (dx, dy) = match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                let (step_x, step_y) = self.wheel_step();
                (x * step_x, y * step_y)
            }
            MouseScrollDelta::PixelDelta(p) => (p.x as f32 / scale, p.y as f32 / scale),
        };
        // 鼠标位于全屏查看的图上时，滚动用于平移或缩放图，不再传给被遮住的预览。
        if self.diagram_view_wheel(dx, dy) {
            self.pace_wheel_frame();
            return;
        }
        let line = matches!(delta, MouseScrollDelta::LineDelta(..));
        if !line {
            self.scroll.wheel.input_at = Some(Instant::now());
        }
        // 触控板竖直滚动时常伴有少量横向位移；只按位移较大的方向滚动，避免长行在竖直滚动时
        // 左右抖动。同一手势中的方向保持不变，斜向滑动不在两个方向之间交替。
        let axis = match self.scroll.wheel.axis {
            Some(axis) if gesture && !line => axis,
            _ => Axis::of(dx, dy),
        };
        if gesture && !line && (dx, dy) != (0.0, 0.0) {
            self.scroll.wheel.axis = Some(axis);
        }
        let (dx, dy) = axis.keep(dx, dy);

        let origin = self.pointer.mouse;
        let target = self.scroll_target_at(origin.0, origin.1);
        // 滚轮和触控板由用户直接控制：该区域的跳转过渡停在屏幕上显示的位置，从该位置继续。
        self.settle_glide_of(target);
        self.scroll.fling = None;
        let glide = self
            .wheel_glides(delta)
            .then(|| self.next_wheel_glide())
            .flatten()
            .and_then(|length| self.clock(length));
        match glide {
            Some(clock) => self.start_wheel_glide((dx, dy), clock),
            None => {
                let shift = self.wheel_shift(dx, line);
                self.scroll_target_by(target, origin, shift, (dx, dy));
            }
        }
        self.pace_wheel_frame();
    }

    /// 是否把竖直滚动按横向处理：按住 `Shift` 转动鼠标滚轮。
    ///
    /// 只用于鼠标滚轮（`line`）。触控板本身支持横向滚动，手指离开后仍持续发出惯性滚动事件；
    /// 此时按 `⇧Tab` 等组合键，惯性滚动会被视为横向滚动，使编辑区意外右移数列。
    fn wheel_shift(&self, dx: f32, line: bool) -> bool {
        self.pointer.modifiers.shift_key() && dx == 0.0 && line
    }

    /// 是否对滚动量 `delta` 使用过渡：仅限鼠标滚轮的整格滚动。
    ///
    /// 不足一格的滚动来自高精度设备，例如 Windows 的高精度触控板，事件已足够密集。鼠标位于终端时
    /// 不使用过渡：终端把每格滚动转换为发给程序的按键，不可拆分。
    pub(in crate::app) fn wheel_glides(&self, delta: MouseScrollDelta) -> bool {
        let MouseScrollDelta::LineDelta(x, y) = delta else {
            return false;
        };
        let (mouse_x, mouse_y) = self.pointer.mouse;
        (x.abs() >= 1.0 || y.abs() >= 1.0) && !self.layout.terminal.contains(mouse_x, mouse_y)
    }

    /// 鼠标滚轮滚动一格的距离（逻辑像素），依次为横向和竖直。
    ///
    /// - 终端中为 3 行：每行作为一次滚轮事件发给终端中的程序。
    /// - macOS 上为 3 行：系统按滚动的速度增加每格的行数。
    /// - Windows 上为 `WHEEL_NOTCH`，按系统设置的行数相对默认的 `WHEEL_LINES` 缩放；设置为
    ///   一次滚动一屏时为编辑区的一屏。横向按系统设置的字符数，每个字符一个字符宽度。
    /// - 其他系统上为 `WHEEL_NOTCH`。
    pub(in crate::app) fn wheel_step(&self) -> (f32, f32) {
        let (x, y) = self.pointer.mouse;
        if self.layout.terminal.contains(x, y) {
            return (
                self.metrics.terminal_cell * 3.0,
                self.fonts.terminal_line_height * 3.0,
            );
        }
        #[cfg(target_os = "macos")]
        {
            (self.metrics.cell * 3.0, self.fonts.line_height * 3.0)
        }
        #[cfg(windows)]
        {
            let (rows, chars) = crate::windows::wheel_scroll_amounts();
            let line = self.fonts.line_height;
            let page = (self.layout.text.h - line).max(line);
            let y = rows.map_or(page, |rows| rows as f32 / WHEEL_LINES * WHEEL_NOTCH);
            (chars as f32 * self.metrics.cell, y)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            (WHEEL_NOTCH, WHEEL_NOTCH)
        }
    }

    /// 返回本格鼠标滚轮滚动的过渡时长（见 [`wheel_glide_length`]），并记录本格的时刻；不使用
    /// 过渡时返回 `None`，直接滚动。
    ///
    /// 只有 macOS 上存在显示链接时使用过渡。Windows 上每格立即滚动；其他系统上按计时器绘制，
    /// 帧与屏幕刷新不同步，过渡只增加绘制量而不会更平滑。
    fn next_wheel_glide(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let interval = self
            .scroll
            .wheel
            .at
            .replace(now)
            .map(|at| now.duration_since(at));
        #[cfg(target_os = "macos")]
        let follows_refresh = self.frame.display_link.is_some();
        #[cfg(not(target_os = "macos"))]
        let follows_refresh = false;
        follows_refresh.then(|| wheel_glide_length(interval))
    }

    /// 在鼠标所在的区域追加滚动距离 `delta`（逻辑像素），按 `clock` 重新计时。
    ///
    /// 进行中的过渡滚动同一区域时，未完成的距离计入新的总距离；滚动其他区域时，未完成的距离
    /// 立即作用于原来的区域。
    pub(in crate::app) fn start_wheel_glide(&mut self, delta: (f32, f32), clock: Clock) {
        let origin = self.pointer.mouse;
        let target = self.scroll_target_at(origin.0, origin.1);
        let shift = self.wheel_shift(delta.0, true);
        let mut rest = (0.0, 0.0);
        if let Some(glide) = self.scroll.wheel.glide.take() {
            if glide.target == target && glide.shift == shift {
                rest = glide.rest();
            } else {
                self.scroll_target_by(glide.target, glide.origin, glide.shift, glide.rest());
            }
        }
        self.scroll.wheel.glide = Some(WheelGlide {
            total: (rest.0 + delta.0, rest.1 + delta.1),
            done: 0.0,
            clock,
            target,
            origin,
            shift,
        });
        self.request_redraw();
    }

    /// 应用本帧对应的滚动距离，绘制前调用；时钟结束时清除过渡状态。
    ///
    /// 记下的区域已不可滚动时（切换了标签页、关闭了面板）丢弃余下的距离。
    pub(in crate::app) fn advance_wheel_glide(&mut self) {
        let Some(glide) = &mut self.scroll.wheel.glide else {
            return;
        };
        let (target, origin, shift) = (glide.target, glide.origin, glide.shift);
        let progress = glide.clock.progress();
        let step = progress - glide.done;
        glide.done = progress;
        let delta = (glide.total.0 * step, glide.total.1 * step);
        let done = glide.clock.done();
        let gone = self.scroll_target_at(origin.0, origin.1) != target;
        if done || gone {
            self.scroll.wheel.glide = None;
        }
        if !gone {
            self.scroll_target_by(target, origin, shift, delta);
        }
    }

    /// 是否在触控板或触摸滚动之后的 [`SCROLL_INPUT_HOLD`] 之内，需要继续逐帧绘制。
    pub(in crate::app) fn scroll_input_live(&self) -> bool {
        (self.scroll.wheel.input_at).is_some_and(|at| at.elapsed() < SCROLL_INPUT_HOLD)
    }

    /// 把滚轮事件引起的绘制推迟到下一次屏幕刷新，每次刷新最多绘制一帧。
    ///
    /// winit 收到重绘请求后在处理完当前事件时绘制，不等待屏幕刷新。滚轮事件与刷新不同步时，
    /// 部分刷新周期绘制两帧，部分周期没有新帧，滚动出现抖动。没有显示链接（macOS 14 以下，或
    /// Windows 上无法启动其线程）或窗口不可见时不推迟。
    pub(in crate::app) fn pace_wheel_frame(&mut self) {
        #[cfg(any(target_os = "macos", windows))]
        if self.frame.display_link.is_some() && !self.unseen() {
            self.frame.dirty = false;
            self.retry_frame(Instant::now());
        }
    }
}
