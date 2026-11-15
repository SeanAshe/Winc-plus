//! X11 上查询鼠标此刻在窗口中的位置：从文件管理器拖文件进来时，拖动源抓住了鼠标，窗口收不到
//! 鼠标移动事件。

// Xlib 调用；只读鼠标位置，不保存连接和窗口。
#![allow(unsafe_code)]

use std::os::raw::{c_int, c_uint, c_ulong};

use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;
use x11_dl::xlib::Xlib;

/// 返回鼠标此刻在窗口中的位置（逻辑像素，原点在左上角）；不是 X11 窗口或取不到时返回 `None`。
///
/// 拖文件进来的过程中和松手后的 `DroppedFile` 都不带位置；该事件在松手后立即处理，此时鼠标的
/// 位置就是落点。`XQueryPointer` 在鼠标被其他程序抓住时也能查询。物理像素按窗口的缩放系数换算。
pub fn mouse_location(window: &Window) -> Option<(f32, f32)> {
    let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) else {
        return None;
    };
    let (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(handle)) =
        (display.as_raw(), handle.as_raw())
    else {
        return None;
    };
    let display = display.display?.as_ptr().cast();
    // 与 winit 共用同一份已加载的 libX11（`open` 缓存加载结果）。
    let xlib = Xlib::open().ok()?;
    let (mut root, mut child): (c_ulong, c_ulong) = (0, 0);
    let (mut root_x, mut root_y, mut x, mut y): (c_int, c_int, c_int, c_int) = (0, 0, 0, 0);
    let mut mask: c_uint = 0;
    // SAFETY: `display` 和 `handle.window` 属于仍然存活的 `window`，在主线程调用；输出参数都是
    // 可写的栈变量。
    let same_screen = unsafe {
        (xlib.XQueryPointer)(
            display,
            handle.window,
            &mut root,
            &mut child,
            &mut root_x,
            &mut root_y,
            &mut x,
            &mut y,
            &mut mask,
        )
    };
    if same_screen == 0 {
        return None;
    }
    let scale = window.scale_factor() as f32;
    Some((x as f32 / scale, y as f32 / scale))
}
