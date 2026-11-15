//! 查询鼠标此刻在窗口中的位置：从资源管理器拖文件进来时，系统不发鼠标移动事件。

// Win32 调用；只读鼠标位置和窗口坐标，不保存句柄。
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// 返回鼠标此刻在窗口客户区中的位置（系统逻辑像素，原点在左上角）；取不到时返回 `None`。
///
/// 拖文件进来的过程中和松手后的 `DroppedFile` 都不带位置；该事件在松手后立即处理，此时鼠标的
/// 位置就是落点。物理像素按窗口当前的 DPI 换算（96 DPI 对应缩放系数 1）。
pub fn mouse_location(window: &Window) -> Option<(f32, f32)> {
    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let hwnd = handle.hwnd.get() as _;
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` 是可写的栈变量；`hwnd` 属于仍然存活的 `window`。
    if unsafe { GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 } {
        return None;
    }
    // SAFETY: `hwnd` 属于仍然存活的 `window`，`GetDpiForWindow` 没有指针参数。
    let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    Some((point.x as f32 / scale, point.y as f32 / scale))
}
