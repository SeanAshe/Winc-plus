//! `touch` 的原生测试：向临时窗口注入触摸，验证顶栏触摸经由系统完成移动、缩放与吸附。
//!
//! 运行时需要交互式桌面，并开启系统的窗口吸附；触摸只注入到测试窗口内。

#![allow(unsafe_code)] // 测试窗口的 Win32 查询、子类回调与触摸注入。

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_TOUCH_INFO,
    TOUCH_FEEDBACK_NONE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, PT_TOUCH, SPI_GETWINARRANGING, SystemParametersInfoW, TOUCH_MASK_CONTACTAREA,
    TOUCH_MASK_ORIENTATION, TOUCH_MASK_PRESSURE, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
    WindowFromPoint,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::{EventLoopBuilderExtWindows, WindowAttributesExtWindows};
use winit::window::{WindowId, WindowLevel};

use super::*;

/// 测试窗口上观察到的事件计数。
#[derive(Default)]
struct Observed {
    /// 收到 `WM_ENTERSIZEMOVE` 的次数。
    entered: AtomicUsize,
    /// 收到 `WM_EXITSIZEMOVE` 的次数。
    exited: AtomicUsize,
    /// winit 交付的 `WindowEvent::Touch` 数量。
    touches: AtomicUsize,
    /// winit 交付的鼠标按下数量。
    mouse_presses: AtomicUsize,
}

#[test]
#[ignore = "injects touch into a temporary visible window; requires an interactive Windows desktop with Snap enabled"]
fn native_touch_uses_system_move_resize_and_snap() {
    struct TestApp {
        window: Option<Window>,
        deadline: Instant,
        proxy: EventLoopProxy<()>,
        injection: Arc<Mutex<Option<io::Result<()>>>>,
        observed: Arc<Observed>,
    }
    impl ApplicationHandler<()> for TestApp {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window = event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Winc native touch test")
                        .with_decorations(false)
                        .with_undecorated_shadow(true)
                        .with_active(false)
                        .with_window_level(WindowLevel::AlwaysOnTop)
                        .with_position(PhysicalPosition::new(200, 200))
                        .with_inner_size(PhysicalSize::new(600, 400))
                        .with_min_inner_size(PhysicalSize::new(200, 150)),
                )
                .unwrap();
            WindowInput::new(&window).unwrap().update(WindowHitTest {
                caption_height: 48.0,
                resize_border: 4.0,
                control_gap: 0.0,
                client: vec![Rect::new(10.0, 8.0, 50.0, 32.0)],
            });
            let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() else {
                unreachable!()
            };
            let hwnd = handle.hwnd.get();
            let data = Arc::into_raw(self.observed.clone());
            // SAFETY: `hwnd` 是本线程创建且仍存活的测试窗口；`data` 转交给 `observe`，
            // 在 `WM_NCDESTROY` 中释放。
            let installed =
                unsafe { SetWindowSubclass(hwnd as _, Some(observe), 2, data as usize) };
            assert_ne!(installed, 0);
            let scale = window.scale_factor();
            self.window = Some(window);
            let proxy = self.proxy.clone();
            let result = self.injection.clone();
            let observed = self.observed.clone();
            std::thread::spawn(move || {
                *result.lock().unwrap() = Some(exercise(hwnd, scale, &observed));
                let _ = proxy.send_event(());
            });
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if matches!(event, WindowEvent::Touch(_)) {
                self.observed.touches.fetch_add(1, Ordering::Relaxed);
            } else if matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                }
            ) {
                self.observed.mouse_presses.fetch_add(1, Ordering::Relaxed);
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            event_loop.exit();
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if Instant::now() >= self.deadline {
                event_loop.exit();
            }
        }
    }
    let event_loop = EventLoop::<()>::with_user_event()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut app = TestApp {
        window: None,
        deadline: Instant::now() + Duration::from_secs(20),
        proxy: event_loop.create_proxy(),
        injection: Arc::default(),
        observed: Arc::default(),
    };
    event_loop.run_app(&mut app).unwrap();
    app.injection
        .lock()
        .unwrap()
        .take()
        .expect("injection completed")
        .unwrap();
}

unsafe extern "system" fn observe(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        // SAFETY: `data` 是安装时转交的唯一 `Arc` 引用。先移除子类回调，此后不再访问 `data`；
        // `WM_NCDESTROY` 只到达一次，因此引用只释放一次。
        unsafe {
            RemoveWindowSubclass(hwnd, Some(observe), id);
            drop(Arc::from_raw(data as *const Observed));
        }
    } else {
        // SAFETY: `WM_NCDESTROY` 之前子类回调始终持有 `data` 对应的 `Arc` 引用，指针有效。
        let observed = unsafe { &*(data as *const Observed) };
        if msg == WM_ENTERSIZEMOVE {
            observed.entered.fetch_add(1, Ordering::Relaxed);
        } else if msg == WM_EXITSIZEMOVE {
            observed.exited.fetch_add(1, Ordering::Relaxed);
        }
    }
    // SAFETY: 参数原样来自系统传给当前回调的消息；`observe` 只计数，不改动消息。
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

fn check(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

fn frame(hwnd: HWND) -> RECT {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `hwnd` 是存活的测试窗口；输出参数指向可写的栈变量。
    unsafe {
        GetWindowRect(hwnd, &mut rect);
    }
    rect
}

fn maximized(hwnd: HWND) -> bool {
    // SAFETY: `IsZoomed` 只读取测试窗口的最大化状态，没有指针参数。
    unsafe { IsZoomed(hwnd) != 0 }
}

fn exercise(raw: isize, scale: f64, observed: &Observed) -> io::Result<()> {
    let hwnd = raw as HWND;
    std::thread::sleep(Duration::from_millis(200));
    let mut snap: i32 = 0;
    // SAFETY: `SPI_GETWINARRANGING` 只读取系统设置，输出指向可写的 `i32`。
    unsafe {
        SystemParametersInfoW(SPI_GETWINARRANGING, 0, (&mut snap as *mut i32).cast(), 0);
    }
    check(snap != 0, "this test requires Windows Snap to be enabled")?;
    // SAFETY: `MONITORINFO` 是纯数据结构，全零是合法值；`cbSize` 在下一行设置。
    let mut monitor: MONITORINFO = unsafe { std::mem::zeroed() };
    monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    // SAFETY: `hwnd` 是存活的测试窗口；输出指向已设置 `cbSize` 的 `MONITORINFO`。
    unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
    }
    // SAFETY: 为当前进程初始化单个接触点的触摸注入，没有指针参数。
    if unsafe { InitializeTouchInjection(1, TOUCH_FEEDBACK_NONE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let title = |r: &RECT| POINT {
        x: r.left + (150.0 * scale) as i32,
        y: r.top + (24.0 * scale) as i32,
    };
    let before = frame(hwnd);
    let button = POINT {
        x: before.left + (30.0 * scale) as i32,
        y: before.top + (24.0 * scale) as i32,
    };
    inject_drag(raw, button, button)?;
    check(
        observed.touches.load(Ordering::Relaxed) >= 2,
        "caption controls must receive client touch events",
    )?;
    check(
        observed.entered.load(Ordering::Relaxed) == 0,
        "a caption control must not start a window drag",
    )?;
    let touches = observed.touches.load(Ordering::Relaxed);

    let start = title(&before);
    inject_drag(
        raw,
        start,
        POINT {
            x: start.x + 100,
            y: start.y + 60,
        },
    )?;
    let moved = frame(hwnd);
    check(
        (moved.left - before.left - 100).abs() <= 3 && (moved.top - before.top - 60).abs() <= 3,
        "native touch must move the window without a mouse button",
    )?;

    let edge = POINT {
        x: moved.right - 2,
        y: moved.bottom - 2,
    };
    inject_drag(
        raw,
        edge,
        POINT {
            x: edge.x + 80,
            y: edge.y + 60,
        },
    )?;
    let resized = frame(hwnd);
    check(
        resized.right - moved.right >= 70 && resized.bottom - moved.bottom >= 50,
        "native touch must resize the window",
    )?;

    let work = monitor.rcWork;
    let screen = monitor.rcMonitor;
    let tap = title(&resized);
    inject_contact(raw, tap, tap, 60)?;
    inject_contact(raw, tap, tap, 600)?;
    check(
        maximized(hwnd),
        "double-tapping the title bar must maximize",
    )?;
    let tap = POINT {
        x: (work.left + work.right) / 2,
        y: work.top + (40.0 * scale) as i32,
    };
    inject_contact(raw, tap, tap, 60)?;
    inject_contact(raw, tap, tap, 600)?;
    check(
        !maximized(hwnd),
        "double-tapping the maximized title bar must restore",
    )?;
    inject_drag(
        raw,
        title(&resized),
        POINT {
            x: (work.left + work.right) / 2,
            y: screen.top,
        },
    )?;
    check(
        maximized(hwnd),
        "dragging to the top must use Windows maximize snapping",
    )?;

    // 等待系统的最大化动画结束后，再从最大化窗口的顶栏拖出。
    std::thread::sleep(Duration::from_millis(750));
    inject_drag(
        raw,
        POINT {
            x: (work.left + work.right) / 2,
            y: work.top + (40.0 * scale) as i32,
        },
        POINT {
            x: work.left + 350,
            y: work.top + 220,
        },
    )?;
    let restored = frame(hwnd);
    check(
        !maximized(hwnd),
        "dragging a maximized title bar must restore the window",
    )?;
    check(
        (restored.right - restored.left - (resized.right - resized.left)).abs() <= 16,
        "Windows must restore the pre-snap width",
    )?;

    inject_drag(
        raw,
        title(&restored),
        POINT {
            x: screen.left,
            y: (work.top + work.bottom) / 2,
        },
    )?;
    let snapped = frame(hwnd);
    let expected = (work.right - work.left) / 2;
    check(
        (snapped.left - work.left).abs() <= 16
            && (snapped.right - snapped.left - expected).abs() <= 20,
        "dragging to the side must use Windows half-screen snapping",
    )?;
    check(
        observed.entered.load(Ordering::Relaxed) >= 5,
        "gestures must enter the system move/size loop",
    )?;
    check(
        observed.entered.load(Ordering::Relaxed) == observed.exited.load(Ordering::Relaxed),
        "every gesture must exit the system move/size loop",
    )?;
    check(
        observed.touches.load(Ordering::Relaxed) == touches,
        "native window gestures must not leak into client gestures",
    )?;
    // 窗口类启用 `CS_DBLCLKS` 后，客户区的第二次按下仍须送达 winit；应用的双击选词依赖该事件。
    for message in [
        WM_LBUTTONDOWN,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
        WM_LBUTTONDBLCLK,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
    ] {
        // SAFETY: `SendMessageW` 同步发送到存活的测试窗口，`lparam` 为客户区坐标 (100, 100)，
        // 不含指针；不注入全局鼠标输入。
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                message,
                0,
                (100 << 16) | 100,
            );
        }
    }
    check(
        observed.mouse_presses.load(Ordering::Relaxed) == 2,
        "client double clicks must deliver both presses",
    )?;
    Ok(())
}

fn inject_drag(hwnd: isize, start: POINT, end: POINT) -> io::Result<()> {
    inject_contact(hwnd, start, end, 500)
}

fn inject_contact(hwnd: isize, start: POINT, end: POINT, settle_ms: u64) -> io::Result<()> {
    // 确认落指位置命中测试窗口，避免触摸注入到其他窗口。
    // SAFETY: `WindowFromPoint` 按值接收坐标，没有指针参数。
    if unsafe { WindowFromPoint(start) } as isize != hwnd {
        return Err(io::Error::other("test window is covered"));
    }
    // SAFETY: `POINTER_TOUCH_INFO` 是纯数据结构，全零是合法值，表示可选字段均未设置。
    let mut contact: POINTER_TOUCH_INFO = unsafe { std::mem::zeroed() };
    contact.pointerInfo.pointerType = PT_TOUCH;
    contact.pointerInfo.pointerId = 0;
    contact.pointerInfo.ptPixelLocation = start;
    contact.touchMask = TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE;
    contact.orientation = 90;
    contact.pressure = 320;
    let send = |contact: &mut POINTER_TOUCH_INFO, flags| -> io::Result<()> {
        contact.pointerInfo.pointerFlags = flags;
        let p = contact.pointerInfo.ptPixelLocation;
        contact.rcContact = RECT {
            left: p.x - 2,
            top: p.y - 2,
            right: p.x + 2,
            bottom: p.y + 2,
        };
        // SAFETY: `contact` 指向一个字段完整的 `POINTER_TOUCH_INFO`，与数量参数 1 一致。
        if unsafe { InjectTouchInput(1, contact) } == 0 {
            Err(io::Error::other(format!(
                "InjectTouchInput ({flags:#x}, {}, {}): {}",
                p.x,
                p.y,
                io::Error::last_os_error()
            )))
        } else {
            Ok(())
        }
    };
    let active = POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
    send(&mut contact, active | POINTER_FLAG_DOWN)?;
    let steps = if start.x == end.x && start.y == end.y {
        1
    } else {
        20
    };
    let moved = (1..=steps).try_for_each(|step| {
        std::thread::sleep(Duration::from_millis(20));
        contact.pointerInfo.ptPixelLocation = POINT {
            x: start.x + (end.x - start.x) * step / steps,
            y: start.y + (end.y - start.y) * step / steps,
        };
        send(&mut contact, active | POINTER_FLAG_UPDATE)
    });
    std::thread::sleep(Duration::from_millis(if steps == 1 { 40 } else { 250 }));
    // 移动失败时仍然抬手，避免系统中留下处于接触状态的指针。
    let released = send(&mut contact, POINTER_FLAG_UP);
    std::thread::sleep(Duration::from_millis(settle_ms));
    moved.and(released)
}
