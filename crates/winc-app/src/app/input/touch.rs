//! 触摸输入：识别点按、长按与单指滚动，以及顶栏、窗口边缘、滚动条和分隔条的拖动。
//!
//! 点按在抬手时才发出，滑动列表时不会先打开文件或触发按钮。出现第二个接触点时取消当前手势，
//! 全部接触点结束后才识别下一次操作；手势取消或窗口失焦时同样不补发点击。

use std::time::{Duration, Instant};

use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, MouseButton, MouseScrollDelta, Touch, TouchPhase};
use winit::window::ResizeDirection;

use super::*;

#[cfg(test)]
mod tests;

type Point = (f32, f32);

/// 在顶栏或窗口边缘落指时交由系统执行的窗口操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum WindowTouch {
    /// 拖动顶栏移动窗口。
    Move,
    /// 拖动边缘或角落改变窗口大小。
    Resize(ResizeDirection),
}

/// 点按与滚动的分界距离（逻辑像素）。
///
/// 接触点偏离落指位置超过该距离后开始滚动；距离以内的移动视为点按和长按时的手指抖动。
const SLOP: f32 = 8.0;
/// 长按的判定时长：落指后在 [`SLOP`] 范围内保持达到该时长即打开右键菜单。
const LONG_PRESS: Duration = Duration::from_millis(500);
/// 两次点按判定为双击时允许的落指位置偏差（逻辑像素）。
///
/// 两次落指的位置偏差通常大于鼠标连击的偏差，因此取点按与滚动分界距离 [`SLOP`] 的两倍。
pub(in crate::app) const TAP_SLOP: f32 = 2.0 * SLOP;
/// 接触点的失效时长：超过该时长未收到新的触摸事件时，未结束的接触点视为已失效。
///
/// 由触摸提升的鼠标事件紧随对应的触摸事件到达，间隔远小于该时长；系统接管窗口拖动时
/// 可能不再送达抬手事件，失效的接触点不再拦截鼠标事件（见 [`App::touch_blocks_mouse`]）。
const STALE: Duration = Duration::from_secs(1);

/// 单指手势的识别阶段。
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// 尚未越过 [`SLOP`]：抬手时按持续时长判定为点按或长按。
    Pending,
    /// 已越过 [`SLOP`]：移动转换为滚动。
    Scroll,
    /// 落指于滚动条或分隔条：移动和抬手直接转换为左键拖动。
    Drag,
    /// 已打开长按菜单或已交由系统执行窗口操作：后续事件不再产生操作。
    Held,
}

/// 单指手势；坐标均为窗口内的逻辑像素。
struct Gesture {
    /// 落指位置。
    origin: Point,
    /// 上一个事件的位置，用于计算滚动增量。
    last: Point,
    /// 落指时刻，用于判定长按。
    since: Instant,
    mode: Mode,
}

/// 单个窗口的触摸状态：未结束的接触点与正在识别的手势。
#[derive(Default)]
pub(in crate::app) struct TouchState {
    /// 未结束的接触点，由设备与编号共同标识。
    contacts: Vec<(DeviceId, u64)>,
    gesture: Option<Gesture>,
    /// 最近一次收到触摸事件的时刻。
    pub(in crate::app) last_event: Option<Instant>,
    /// 是否正在把一次点按作为鼠标点击发出；为真时连击判定按 [`TAP_SLOP`] 放宽位置偏差。
    pub(in crate::app) tapping: bool,
    /// 向系统命中测试提供可交互区域的句柄；窗口创建前或安装失败时为 `None`。
    #[cfg(windows)]
    pub(in crate::app) window_input: Option<crate::windows::WindowInput>,
}

#[derive(Debug, PartialEq)]
enum Action {
    Tap(Point),
    Context(Point),
    Scroll { origin: Point, delta: Point },
    Press(Point),
    Move(Point),
    Release(Point),
    CancelDrag,
}

impl TouchState {
    /// 是否仍有未结束的接触点，包括手势已因多指操作取消的接触点。
    pub(in crate::app) fn active(&self) -> bool {
        !self.contacts.is_empty()
    }

    fn update(
        &mut self,
        touch: &Touch,
        point: Point,
        now: Instant,
        direct: bool,
    ) -> Option<Action> {
        let key = (touch.device_id, touch.id);
        self.last_event = Some(now);
        if touch.phase == TouchPhase::Started {
            if self.contacts.contains(&key) {
                return None;
            }
            self.contacts.push(key);
            if self.contacts.len() > 1 {
                return self.cancel_gesture();
            }
            self.gesture = Some(Gesture {
                origin: point,
                last: point,
                since: now,
                mode: if direct { Mode::Drag } else { Mode::Pending },
            });
            return direct.then_some(Action::Press(point));
        }

        if !self.contacts.contains(&key) {
            return None;
        }
        if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.contacts.retain(|&contact| contact != key);
        }
        if touch.phase == TouchPhase::Cancelled {
            return self.cancel_gesture();
        }

        let gesture = self.gesture.as_mut()?;
        let ended = touch.phase == TouchPhase::Ended;
        let mut delta = (point.0 - gesture.last.0, point.1 - gesture.last.1);
        if gesture.mode == Mode::Pending {
            let from_start = (point.0 - gesture.origin.0, point.1 - gesture.origin.1);
            if from_start.0.hypot(from_start.1) > SLOP {
                gesture.mode = Mode::Scroll;
                // 越过阈值时包括最初几像素，内容与手指的位移保持一致。
                delta = from_start;
            }
        }
        gesture.last = point;
        let action = match gesture.mode {
            Mode::Pending if ended => Some(if now.duration_since(gesture.since) >= LONG_PRESS {
                Action::Context(gesture.origin)
            } else {
                Action::Tap(gesture.origin)
            }),
            Mode::Scroll if delta != (0.0, 0.0) => Some(Action::Scroll {
                origin: gesture.origin,
                delta,
            }),
            Mode::Drag => Some(if ended {
                Action::Release(point)
            } else {
                Action::Move(point)
            }),
            _ => None,
        };
        if ended {
            self.cancel_gesture();
        }
        action
    }

    fn cancel_gesture(&mut self) -> Option<Action> {
        self.gesture
            .take()
            .and_then(|g| (g.mode == Mode::Drag).then_some(Action::CancelDrag))
    }

    fn deadline(&self) -> Option<Instant> {
        self.gesture
            .as_ref()
            .filter(|g| g.mode == Mode::Pending)
            .map(|g| g.since + LONG_PRESS)
    }

    fn hold(&mut self, now: Instant) -> Option<Action> {
        if now < self.deadline()? {
            return None;
        }
        let gesture = self.gesture.as_mut()?;
        gesture.mode = Mode::Held;
        Some(Action::Context(gesture.origin))
    }
}

impl App {
    /// 处理一个触摸事件：坐标换算为逻辑像素后识别手势，转换为鼠标点击、滚动或拖动。
    pub(in crate::app) fn on_touch(&mut self, touch: Touch) {
        let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
        let point = (
            touch.location.x as f32 / scale,
            touch.location.y as f32 / scale,
        );
        if touch.phase == TouchPhase::Started && !self.touch.active() {
            self.drop_drags();
            self.scroll_glide = None;
            self.wheel.glide = None;
            self.dismiss_hint();
            self.cursor_inside = true;
            self.on_mouse_move(point.0, point.1);
            if let Some(operation) = self.touch_window_at(point.0, point.1) {
                // 登记接触点并置为 `Held`：此后的移动和抬手只结束本次接触，不产生点击或滚动。
                self.touch.update(&touch, point, Instant::now(), false);
                if let Some(gesture) = self.touch.gesture.as_mut() {
                    gesture.mode = Mode::Held;
                }
                self.start_touch_window(point, operation);
                #[cfg(windows)]
                self.sync_window_input();
                return;
            }
        }
        if touch.phase == TouchPhase::Cancelled
            || (touch.phase == TouchPhase::Started && self.touch.active())
        {
            self.last_click = None;
        }
        let direct = touch.phase == TouchPhase::Started
            && !self.touch.active()
            && self.touch_drag_at(point.0, point.1);
        let action = self.touch.update(&touch, point, Instant::now(), direct);
        if let Some(action) = action {
            self.apply_touch(action);
        }
        if !self.touch.active() {
            self.cursor_inside = false;
            self.set_hover(Hover::None);
        }
        // 命中测试的结果取决于是否有接触点（见 `App::window_hit_test`），落指和抬手后需要同步。
        #[cfg(windows)]
        self.sync_window_input();
    }

    /// 是否忽略当前的鼠标事件。
    ///
    /// 仍有接触点时，由触摸提升的鼠标事件不得再次触发点击。接触点超过 [`STALE`] 未收到新的
    /// 触摸事件时视为已失效（例如系统接管窗口拖动后未送达抬手事件），清除后按常规处理鼠标事件，
    /// 避免鼠标输入持续无响应。
    pub(in crate::app) fn touch_blocks_mouse(&mut self) -> bool {
        if !self.touch.active() {
            return false;
        }
        let stale = (self.touch.last_event).is_none_or(|at| at.elapsed() >= STALE);
        if stale {
            self.cancel_touch();
            return false;
        }
        true
    }

    fn start_touch_window(&mut self, (x, y): Point, operation: WindowTouch) {
        if operation == WindowTouch::Move
            && self.title_bar_pressed(x, y)
            && self.title_bar_zooms_at(x)
        {
            self.title_bar_double_click();
            return;
        }
        let Some(window) = &self.window.handle else {
            return;
        };
        let _ = match operation {
            WindowTouch::Move => window.drag_window(),
            WindowTouch::Resize(direction) => window.drag_resize_window(direction),
        };
    }

    /// 到达长按时限时打开右键菜单，并返回下一次需要唤醒事件循环的时刻。
    ///
    /// 静止的长按不产生移动事件，因此由事件循环的定时唤醒触发。
    pub(in crate::app) fn touch_tick(&mut self, now: Instant) -> Option<Instant> {
        if let Some(action) = self.touch.hold(now) {
            self.apply_touch(action);
        }
        self.touch.deadline()
    }

    /// 中断当前触摸：结束进行中的拖动并清除全部接触点。
    ///
    /// 用于窗口失焦、移动、缩放或被遮挡时；此后迟到的移动和抬手事件不产生操作。
    pub(in crate::app) fn cancel_touch(&mut self) {
        let active = self.touch.active();
        if let Some(action) = self.touch.cancel_gesture() {
            self.apply_touch(action);
        }
        self.touch.contacts.clear();
        if active {
            self.last_click = None;
            self.cursor_inside = false;
            self.set_hover(Hover::None);
            #[cfg(windows)]
            self.sync_window_input();
        }
    }

    fn apply_touch(&mut self, action: Action) {
        match action {
            Action::Tap((x, y)) => {
                self.on_mouse_move(x, y);
                self.touch.tapping = true;
                self.on_mouse_down(MouseButton::Left);
                self.on_mouse_up(MouseButton::Left);
                self.touch.tapping = false;
            }
            Action::Context((x, y)) => {
                self.last_click = None;
                self.on_mouse_move(x, y);
                self.on_mouse_down(MouseButton::Right);
                self.on_mouse_up(MouseButton::Right);
            }
            Action::Scroll { origin, delta } => {
                self.last_click = None;
                // 始终滚动落指处的区域：手指越过侧栏、终端等区域的边界时不切换滚动对象。
                let mouse = self.mouse;
                self.mouse = origin;
                let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
                self.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    f64::from(delta.0 * scale),
                    f64::from(delta.1 * scale),
                )));
                self.mouse = mouse;
            }
            Action::Press((x, y)) => {
                self.last_click = None;
                self.on_mouse_move(x, y);
                self.on_mouse_down(MouseButton::Left);
            }
            Action::Move((x, y)) => self.on_mouse_move(x, y),
            Action::Release((x, y)) => {
                self.on_mouse_move(x, y);
                self.on_mouse_up(MouseButton::Left);
            }
            // 只有滚动条和分隔条在落指时立即按下；此处的左键抬起保存尺寸并结束拖动，
            // 不触发按钮，也不放置文件。
            Action::CancelDrag => self.on_mouse_up(MouseButton::Left),
        }
    }
}
