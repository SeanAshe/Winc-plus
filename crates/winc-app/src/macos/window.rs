//! 窗口外观与系统偏好的读取。
//!
//! - 红绿灯按钮在顶栏中垂直居中；顶栏中的标签可以拖动；双击顶栏的行为跟随系统偏好。
//! - 读取「减弱动态效果」和首选语言；查询鼠标此刻在窗口中的位置（拖文件进来时用）。
//! - 测量时窗口静默（透明、点击穿透、不抢占焦点）。

use super::files::{ns_string, rust_string};
use super::*;
use objc2::Encode;
use objc2::runtime::Bool;

/// `NSWindowButton` 的前三个值：关闭、最小化、缩放。
pub(super) const BUTTONS: [usize; 3] = [0, 1, 2];

/// 返回鼠标此刻在窗口内容区中的位置（`AppKit` 的点，原点在左上角）；取不到时返回 `None`。
///
/// 从系统文件管理器拖文件进来时，系统不发鼠标移动事件，松手后的 `DroppedFile` 也不带位置；
/// 该事件在松手后立即处理，此时鼠标的位置就是落点。
pub fn mouse_location(window: &Window) -> Option<(f32, f32)> {
    let ns_window = ns_window(window)?;

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用。两个都是 NSWindow /
    // NSView 的公开只读方法，返回值是值类型的点和矩形；内容视图先判空再使用。
    unsafe {
        let point: CGPoint = msg_send![ns_window, mouseLocationOutsideOfEventStream];
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        let bounds: CGRect = msg_send![content.as_ref()?, bounds];
        // 窗口坐标的原点在左下角；内容视图铺满整个窗口（标题栏透明），高度相同。
        Some((point.x as f32, (bounds.size.height - point.y) as f32))
    }
}

/// 是否为光标所在位置最上层的窗口；按住鼠标按键期间、取不到窗口时为是。
///
/// 鼠标移动事件不保证只发给光标下最上层的窗口：窗口在被其他窗口遮挡的位置仍可能收到。
/// 按住按键拖动时光标可以移到其他窗口上方，移动事件仍属于按下时的窗口。
pub fn under_cursor(window: &Window) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return true;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用。`pressedMouseButtons`、
    // `mouseLocation` 和 `windowNumberAtPoint:belowWindowWithWindowNumber:` 是公开的类方法，
    // `windowNumber` 是公开的只读属性；参数和返回值都是整数或值类型的点，不涉及指针所有权。
    unsafe {
        let pressed: usize = msg_send![class!(NSEvent), pressedMouseButtons];
        if pressed != 0 {
            return true;
        }
        let point: CGPoint = msg_send![class!(NSEvent), mouseLocation];
        // 第二个参数为 0 时从最上层的窗口开始查找，范围包括其他应用的窗口。
        let top: isize = msg_send![
            class!(NSWindow),
            windowNumberAtPoint: point,
            belowWindowWithWindowNumber: 0isize
        ];
        let own: isize = msg_send![ns_window, windowNumber];
        top == own
    }
}

/// 把窗口设为全透明、不画阴影、鼠标点击穿透到下方的窗口，用于测量。
///
/// 窗口仍然上屏并呈现（隐藏的窗口收不到重画事件，无法测量首帧），只是不可见、不可点击。
pub fn make_invisible(window: &Window) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用；三个调用都是
    // NSWindow 的公开 setter，参数是 `f64` 或 `bool` 值类型，不涉及指针所有权。
    unsafe {
        let () = msg_send![ns_window, setAlphaValue: 0.0f64];
        let () = msg_send![ns_window, setHasShadow: false];
        let () = msg_send![ns_window, setIgnoresMouseEvents: true];
    }
}

/// 是否有提示框或面板（sheet）挂在窗口上。
///
/// 弹出 sheet 时窗口会失去键盘焦点，但用户并没有离开窗口。
pub fn has_sheet(window: &Window) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return false;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用；`attachedSheet` 是
    // NSWindow 的公开 getter，返回可能为空的指针，只判空不解引用。
    unsafe {
        let sheet: *mut AnyObject = msg_send![ns_window, attachedSheet];
        !sheet.is_null()
    }
}

/// 把窗口显示到屏幕上但不使其成为键盘焦点窗口，用于测量。
///
/// `winit` 的 `set_visible(true)` 调用 `makeKeyAndOrderFront:`，会从当前前台应用处抢走键盘焦点。
pub fn show_without_focus(window: &Window) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，指向有效的 NSWindow；本函数只在主线程
    // 调用，满足 AppKit 的线程要求。`orderFrontRegardless` 只调整窗口层级，不激活应用，不
    // 抢占键盘焦点，也不转移窗口的所有权。
    unsafe {
        let () = msg_send![ns_window, orderFrontRegardless];
    }
}

/// `NSWindowStyleMaskFullScreen`：窗口处于全屏（或正在进入全屏）时 `styleMask` 含这一位。
const STYLE_FULL_SCREEN: usize = 1 << 14;

/// 窗口的红绿灯：窗口模式下放进顶栏并垂直居中，全屏时交还给 `AppKit`。
///
/// 全屏时 `AppKit` 在屏幕顶部显示的标题栏中绘制红绿灯，并按其默认位置排列。因此进入全屏的过渡
/// 开始时，按钮恢复到 `AppKit` 原来的位置。退出全屏的过渡期间，`AppKit` 重新排布标题栏，按钮回到
/// 默认位置，此时摆放的位置不会保留：过渡开始时隐藏标题栏容器，过渡结束后按窗口模式摆放并重新
/// 显示，过渡期间按钮的位置不会变化。
///
/// 上述处理在通知发出时同步完成：监听不指定队列，通知中心在发出通知的线程（主线程）上直接调用
/// 回调。若经事件循环处理，过渡的前几帧仍使用原来的位置。释放时注销监听。
pub struct TrafficLights {
    shared: std::rc::Rc<Lights>,
    tokens: Vec<Retained<AnyObject>>,
}

/// [`TrafficLights`] 与通知回调共用的状态。
struct Lights {
    /// 窗口的 `NSWindow`，由本结构持有一份引用。
    window: Retained<AnyObject>,
    /// 顶栏的高度（逻辑像素），红绿灯在其中垂直居中。
    bar_height: f64,
    /// 第一个按钮距窗口左边缘的距离（逻辑像素）。
    left: f64,
    /// 界面缩放倍数：逻辑像素乘以该值得到 `AppKit` 的点，按钮本身的大小不随之变化。
    zoom: std::cell::Cell<f64>,
    /// 第一次摆放之前 `AppKit` 给出的位置：标题栏容器的矩形和三个按钮的原点；全屏时恢复。
    native: std::cell::Cell<Option<(CGRect, [CGPoint; 3])>>,
    /// 窗口是否处于全屏或正在进入全屏；在全屏过渡开始时更新，早于 winit 报告的状态。
    fullscreen: std::cell::Cell<bool>,
    /// 按钮位置改变后调用，由应用重新计算布局并重画。
    moved: Box<dyn Fn()>,
}

impl TrafficLights {
    /// 管理 `window` 的红绿灯：按钮放进高 `bar_height` 的顶栏并垂直居中，第一个按钮距窗口左
    /// 边缘 `left`（逻辑像素）；全屏过渡中按钮的位置改变后调用 `moved`。取不到 `NSWindow` 时
    /// 返回 `None`。
    pub fn new(
        window: &Window,
        bar_height: f64,
        left: f64,
        moved: impl Fn() + 'static,
    ) -> Option<TrafficLights> {
        let ns_window = ns_window(window)?;
        // SAFETY: `ns_window` 是主线程上仍然存活的 NSWindow；`retain` 增加一次引用计数，由
        // `Retained` 在释放时归还。`styleMask` 是 NSWindow 的公开属性。
        let (window, style) = unsafe {
            let retained = Retained::retain(std::ptr::from_ref(ns_window).cast_mut())?;
            let style: usize = msg_send![ns_window, styleMask];
            (retained, style)
        };
        let shared = std::rc::Rc::new(Lights {
            window,
            bar_height,
            left,
            zoom: std::cell::Cell::new(1.0),
            native: std::cell::Cell::new(None),
            fullscreen: std::cell::Cell::new(style & STYLE_FULL_SCREEN != 0),
            moved: Box::new(moved),
        });
        let mut tokens = Vec::new();
        for (name, step) in [
            ("NSWindowWillEnterFullScreenNotification", Step::WillEnter),
            ("NSWindowWillExitFullScreenNotification", Step::WillExit),
            ("NSWindowDidExitFullScreenNotification", Step::DidExit),
        ] {
            let name = ns_string(name)?;
            let lights = std::rc::Rc::clone(&shared);
            let block = RcBlock::new(move |_note: *mut AnyObject| lights.transition(step));
            // SAFETY: `shared.window` 是仍然存活的 NSWindow，`name` 是有效的 NSString。监听只接收
            // 该窗口的通知；`queue` 为空，`block` 在发出通知的线程（主线程）上同步执行，由通知
            // 中心复制并持有。返回的监听对象由本结构持有，释放时注销。
            let token: Option<Retained<AnyObject>> = unsafe {
                let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
                let queue: *mut AnyObject = std::ptr::null_mut();
                msg_send![
                    center,
                    addObserverForName: &*name,
                    object: &*shared.window,
                    queue: queue,
                    usingBlock: &*block
                ]
            };
            tokens.extend(token);
        }
        Some(TrafficLights { shared, tokens })
    }

    /// 按当前状态摆放红绿灯，返回最后一个按钮右边缘距窗口左边缘的距离（逻辑像素），布局据此在
    /// 红绿灯右侧留出空间；见 [`Lights::place`]。
    pub fn place(&self) -> Option<f64> {
        self.shared.place()
    }

    /// 设置界面缩放倍数 `zoom`，之后的 [`Self::place`] 按缩放后的顶栏摆放。
    pub fn set_zoom(&self, zoom: f64) {
        self.shared.zoom.set(zoom);
    }
}

impl Drop for TrafficLights {
    fn drop(&mut self) {
        // SAFETY: `defaultCenter` 始终返回有效的通知中心；每个 `token` 都是
        // `addObserverForName:object:queue:usingBlock:` 返回并由本结构持有的监听对象，注销已失效
        // 的监听同样安全。本结构随窗口在主线程上释放。
        unsafe {
            let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
            for token in &self.tokens {
                let () = msg_send![center, removeObserver: &**token];
            }
        }
    }
}

/// 全屏过渡中需要处理的时刻。
#[derive(Clone, Copy)]
enum Step {
    /// 开始进入全屏。
    WillEnter,
    /// 开始退出全屏。
    WillExit,
    /// 已退出全屏。
    DidExit,
}

impl Lights {
    /// 处理全屏过渡中的一个时刻（见 [`TrafficLights`]），之后通知应用重新布局。
    fn transition(&self, step: Step) {
        let end = match step {
            Step::WillEnter => {
                self.fullscreen.set(true);
                self.set_hidden(false);
                self.restore();
                Some(0.0)
            }
            Step::WillExit => {
                self.fullscreen.set(false);
                self.set_hidden(true);
                self.place()
            }
            Step::DidExit => {
                self.fullscreen.set(false);
                let end = self.place();
                self.set_hidden(false);
                end
            }
        };
        if let Some(end) = end {
            crate::layout::set_traffic_lights_end(end as f32);
        }
        (self.moved)();
    }

    /// 窗口模式下把红绿灯移入顶栏并垂直居中，返回最后一个按钮右边缘距窗口左边缘的距离（逻辑
    /// 像素）；按钮间距由系统决定（macOS 26 为 23，更早的版本为 20）。
    ///
    /// 全屏时不移动按钮，返回 0：系统不在窗口内绘制红绿灯，布局按无红绿灯处理，顶栏左侧不保留
    /// 其位置。取不到按钮时返回 `None`，调用方保留现有布局。
    fn place(&self) -> Option<f64> {
        if self.fullscreen.get() {
            return Some(0.0);
        }
        let ns_window = &*self.window;
        // SAFETY: `ns_window` 是本结构持有的 NSWindow，只在主线程上使用。用到的都是 NSWindow /
        // NSView 的公开方法，返回值先判空再使用。
        unsafe {
            let buttons = BUTTONS.map(|kind| {
                let button: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
                button.as_ref()
            });
            let [Some(close), Some(minimize), Some(maximize)] = buttons else {
                return None;
            };
            let container = titlebar_container(close)?;
            if self.native.get().is_none() {
                let origin = |button: &AnyObject| {
                    let frame: CGRect = msg_send![button, frame];
                    frame.origin
                };
                let frame: CGRect = msg_send![container, frame];
                let origins = [origin(close), origin(minimize), origin(maximize)];
                self.native.set(Some((frame, origins)));
            }

            // NSView 默认以左下角为原点：容器的底边放在窗口顶部向下 `bar_height` 处，使其占满
            // 顶栏。`AppKit` 可能忽略设置的高度，但按钮相对容器底边摆放，结果相同。
            let zoom = self.zoom.get();
            let (bar_height, left) = (self.bar_height * zoom, self.left * zoom);
            let window_frame: CGRect = msg_send![ns_window, frame];
            let mut frame: CGRect = msg_send![container, frame];
            frame.size.height = bar_height;
            frame.origin.y = window_frame.size.height - bar_height;
            let () = msg_send![container, setFrame: frame];

            let close_frame: CGRect = msg_send![close, frame];
            let minimize_frame: CGRect = msg_send![minimize, frame];
            let spacing = minimize_frame.origin.x - close_frame.origin.x;

            let mut end = left;
            for (i, button) in [close, minimize, maximize].into_iter().enumerate() {
                let size: CGRect = msg_send![button, frame];
                let origin = CGPoint::new(
                    left + i as f64 * spacing,
                    ((bar_height - size.size.height) / 2.0).round(),
                );
                let () = msg_send![button, setFrameOrigin: origin];
                end = origin.x + size.size.width;
            }
            Some(end / zoom)
        }
    }

    /// 隐藏或显示标题栏容器（连同其中的红绿灯）；取不到容器时不做处理。
    fn set_hidden(&self, hidden: bool) {
        let ns_window = &*self.window;
        // SAFETY: 同 [`Lights::place`]；`setHidden:` 是 NSView 的公开方法。
        unsafe {
            let close: *mut AnyObject = msg_send![ns_window, standardWindowButton: BUTTONS[0]];
            if let Some(container) = close.as_ref().and_then(|close| titlebar_container(close)) {
                let () = msg_send![container, setHidden: Bool::new(hidden)];
            }
        }
    }

    /// 将标题栏容器和三个按钮恢复到首次摆放之前 `AppKit` 给出的位置；尚未摆放时不做处理。
    fn restore(&self) {
        let Some((frame, origins)) = self.native.get() else {
            return;
        };
        let ns_window = &*self.window;
        // SAFETY: 同 [`Lights::place`]。
        unsafe {
            let buttons = BUTTONS.map(|kind| {
                let button: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
                button.as_ref()
            });
            let [Some(close), Some(_), Some(_)] = buttons else {
                return;
            };
            if let Some(container) = titlebar_container(close) {
                let () = msg_send![container, setFrame: frame];
            }
            for (button, origin) in buttons.into_iter().flatten().zip(origins) {
                let () = msg_send![button, setFrameOrigin: origin];
            }
        }
    }
}

/// 让顶栏中的标签可以拖动。
///
/// 标题栏透明、内容铺到标题栏下方时，`AppKit` 仍把标题栏那一条当作拖动窗口的区域：在那里
/// 按下并拖动，系统直接移动窗口，应用收不到拖动事件。本函数让内容视图对
/// `mouseDownCanMoveWindow` 返回 `NO`，使按下时不移动窗口；顶栏空白处由应用自己调用
/// `drag_window` 拖动窗口。必须在窗口显示之前调用。
pub fn keep_title_bar_drags(window: &Window) {
    if let Some(view) = ns_view(window) {
        add_cannot_move_window(view.class());
    }
}

type CanMoveWindow = extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool;

/// 实现 `- (BOOL)mouseDownCanMoveWindow`：固定返回 `NO`，窗口不随内容视图中的按下而移动。
extern "C-unwind" fn cannot_move_window(_this: *mut AnyObject, _cmd: Sel) -> Bool {
    Bool::NO
}

/// 返回值和参数的类型编码：BOOL、self、_cmd。
///
/// BOOL 在 Apple 芯片上是 `B`，在 Intel 上是 `c`。
fn can_move_window_types() -> CString {
    CString::new(format!("{}@:", Bool::ENCODING)).expect("类型编码里没有 NUL")
}

/// 给 `class` 添加返回 `NO` 的 `mouseDownCanMoveWindow` 方法，返回是否添加成功；类自身已有该
/// 方法时不做任何事并返回 `false`。
fn add_cannot_move_window(class: &AnyClass) -> bool {
    let types = can_move_window_types();
    // SAFETY: `CanMoveWindow` 的签名与 `can_move_window_types` 给出的编码一致：返回 BOOL，
    // 参数依次为 self（@）、_cmd（:）。两者同为 `extern "C-unwind"` 函数指针，大小相同。
    // 父类 `NSView` 的实现被该类覆盖，只影响该类的实例。
    unsafe {
        let imp = std::mem::transmute::<CanMoveWindow, Imp>(cannot_move_window);
        add_raw_method(class, sel!(mouseDownCanMoveWindow), imp, &types)
    }
}

/// 处理双击顶栏：按系统的「连按窗口标题栏」偏好（`AppleActionOnDoubleClick`）缩放或最小化窗口，
/// 偏好为「无操作」时不处理。
///
/// 缩放、最小化在当前事件处理完毕后开始（`performSelector:withObject:afterDelay:`，延迟为 0）。
/// 双击由应用的鼠标处理识别；若在 winit 的事件回调中同步调用 `zoom:`，`AppKit` 在这次调用内执行
/// 完整个缩放动画，winit 将其间所有窗口大小变化推迟到回调返回后派发，动画期间界面不重画，结束时
/// 直接显示新的大小。由运行循环开始动画时，每一帧的大小变化都同步派发，界面随窗口逐帧重新布局
/// （见 `WindowEvent::Resized` 的处理）。
pub fn title_bar_double_click(window: &Window) {
    let action = match action_on_double_click().as_deref() {
        Some("Minimize") => sel!(miniaturize:),
        Some("None") => return,
        // 「缩放」「填充」和没有设置过都按缩放处理；`zoom:` 在缩放和还原之间切换。
        _ => sel!(zoom:),
    };
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    // SAFETY: `ns_window` 是主线程上仍然存活的 NSWindow；`zoom:`、`miniaturize:` 是它的公开动作
    // 方法，参数 `sender` 可以为空。`performSelector:withObject:afterDelay:` 在当前线程的运行循环
    // 上安排调用，并在调用前持有 `ns_window`。
    unsafe {
        let sender: *mut AnyObject = std::ptr::null_mut();
        let () = msg_send![
            ns_window,
            performSelector: action,
            withObject: sender,
            afterDelay: 0.0f64
        ];
    }
}

/// 用户是否在系统的辅助功能设置中打开了「减弱动态效果」。
pub fn reduce_motion() -> bool {
    // SAFETY: `NSWorkspace` 的 `sharedWorkspace` 和该只读属性都是公开 API，返回值先判空后使用。
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        workspace
            .as_ref()
            .is_some_and(|w| msg_send![w, accessibilityDisplayShouldReduceMotion])
    }
}

/// 返回用户在系统中排第一的首选语言（如 `zh-Hans-CN`、`en-US`）；无法取得时返回 `None`。
///
/// 从图形界面启动的应用没有 `LANG` 环境变量，因此需要向系统查询。
pub fn preferred_language() -> Option<String> {
    // SAFETY: NSLocale 的 `preferredLanguages` 是公开的类方法，返回 NSArray<NSString>；返回的
    // 对象先判空再使用，非空时是有效的 NSString，交给 `rust_string` 立即拷贝。
    unsafe {
        let languages: *mut AnyObject = msg_send![class!(NSLocale), preferredLanguages];
        let first: *mut AnyObject = msg_send![languages.as_ref()?, firstObject];
        rust_string(first.as_ref()?)
    }
}

/// 读取全局偏好 `AppleActionOnDoubleClick`，没有设置过时返回 `None`。
fn action_on_double_click() -> Option<String> {
    // SAFETY: NSUserDefaults 和 NSString 的类方法是线程安全的公开 API；传入的 C 字符串以 NUL 结尾；
    // 返回的对象先判空再使用；`value` 是有效的 NSString，交给 `rust_string` 立即拷贝。
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let defaults = defaults.as_ref()?;
        let name = c"AppleActionOnDoubleClick".as_ptr();
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: name];
        let key = key.as_ref()?;
        let value: *mut AnyObject = msg_send![defaults, stringForKey: key];
        rust_string(value.as_ref()?)
    }
}

/// 从按钮向上查找标题栏容器（`NSTitlebarContainerView`），找不到时返回 `None`。
///
/// 不同系统版本中间相隔的层数不同，因此最多向上查找 4 层。
///
/// # Safety
///
/// `view` 必须是有效的 NSView，并且在主线程上调用。
unsafe fn titlebar_container(view: &AnyObject) -> Option<&AnyObject> {
    let mut current = view;

    for _ in 0..4 {
        // SAFETY: 由调用方保证 `current` 是主线程上有效的 NSView，`superview` 可能为空。
        let parent: *mut AnyObject = unsafe { msg_send![current, superview] };
        // SAFETY: 父视图指针非空时指向有效的 NSView；本函数运行期间视图层级不会被修改，因此
        // 返回的引用与 `view` 的借用期同样有效。
        current = unsafe { parent.as_ref()? };

        if current
            .class()
            .name()
            .to_bytes()
            .ends_with(b"TitlebarContainerView")
        {
            return Some(current);
        }
    }

    None
}

#[cfg(test)]
mod tests;
