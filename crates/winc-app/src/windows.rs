//! Windows 上交由系统的操作，读取的系统设置，任务栏的跳转列表，文件类型图标，以及按屏幕刷新绘制动画的
//! 显示链接。

mod assoc;
mod display_link;
mod jump_list;
mod touch;

pub use assoc::{register_file_icons, register_file_types, unregister_file_types};
pub use display_link::DisplayLink;
pub use jump_list::sync_jump_list;
pub use touch::{WindowHitTest, WindowInput};

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    SHFILEOPSTRUCTW, SHFileOperationW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETWHEELSCROLLCHARS, SPI_GETWHEELSCROLLLINES, SystemParametersInfoW,
};

/// 系统设置中鼠标滚轮一格滚动的行数（竖直）和字符数（横向）。
///
/// 行数设置为一次滚动一屏时为 `None`；读取失败时取系统默认值 3。
#[allow(unsafe_code)] // SystemParametersInfoW
pub fn wheel_scroll_amounts() -> (Option<u32>, u32) {
    /// `WHEEL_PAGESCROLL`（`<winuser.h>`）：一次滚动一屏。
    const PAGE: u32 = u32::MAX;
    let read = |action| {
        let mut value: u32 = 3;
        // SAFETY: 这两项设置向 `pvparam` 指向的位置写入一个 `u32`，`value` 在调用期间有效。
        let ok = unsafe { SystemParametersInfoW(action, 0, (&raw mut value).cast(), 0) };
        if ok != 0 { value } else { 3 }
    };
    let lines = read(SPI_GETWHEELSCROLLLINES);
    (
        (lines != PAGE).then_some(lines),
        read(SPI_GETWHEELSCROLLCHARS),
    )
}

/// 把文件或目录 `path` 移到回收站。
///
/// 放不进回收站的项（太大、在网络位置上）由系统先询问是否彻底删除，不会直接删除；用户在询问时
/// 取消也算失败。
///
/// # Errors
///
/// 系统没能移动（文件正被占用、没有权限等）或用户取消时返回 `io::Error`。
#[allow(unsafe_code)] // SHFileOperationW
pub fn move_to_recycle_bin(path: &Path) -> io::Result<()> {
    let path = std::path::absolute(path)?;
    // 可以一次给多个路径：各以 NUL 结尾，最后再多一个 NUL。
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: (FOF_ALLOWUNDO
            | FOF_NOCONFIRMATION
            | FOF_NOERRORUI
            | FOF_SILENT
            | FOF_WANTNUKEWARNING) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    // SAFETY: `operation` 中的指针指向本函数中有效的缓冲区（`from` 以两个 NUL 结尾），其余为空；
    // 调用同步完成，返回后不再使用这些指针。
    let code = unsafe { SHFileOperationW(&mut operation) };
    if operation.fAnyOperationsAborted != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            winc_i18n::t!("common-trash-cancelled"),
        ));
    }
    if code != 0 {
        // 返回的是外壳自己的错误码（`DE_*`），与系统错误码部分重合，只用于说明。
        return Err(io::Error::other(winc_i18n::t!(
            "common-trash-failed-code",
            code = format!("0x{code:X}")
        )));
    }
    Ok(())
}

/// 返回鼠标此刻在窗口客户区中的位置（物理像素，原点在左上角）；取不到时返回 `None`。
///
/// 从资源管理器拖文件进来时，系统不发鼠标移动事件，松手后的 `DroppedFile` 也不带位置。
/// 该事件在松手后立即处理，此时鼠标的位置就是落点。
#[allow(unsafe_code)] // GetCursorPos、ScreenToClient
pub fn mouse_location(window: &Window) -> Option<(f32, f32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    // `HWND` 是 `*mut c_void`；winit 给出的是非零的 `isize`。
    let hwnd = win32.hwnd.get() as *mut _;
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` 在调用期间有效。`hwnd` 来自仍然存活的 winit 窗口，只用于把屏幕坐标
    // 换成该窗口的客户区坐标，不保存这个指针。
    unsafe {
        if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
            return None;
        }
    }
    Some((point.x as f32, point.y as f32))
}

/// 离开当前接上的控制台；没有接上控制台时没有效果。
#[allow(unsafe_code)] // FreeConsole
pub fn detach_console() {
    use windows_sys::Win32::System::Console::FreeConsole;
    // SAFETY: 没有参数；未接上控制台时调用失败，没有其他影响。
    unsafe { FreeConsole() };
}

/// 允许进程 `pid` 把自己的窗口切到前台；本进程没有这项权限时没有效果。
#[allow(unsafe_code)] // AllowSetForegroundWindow
pub fn allow_foreground(pid: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    // SAFETY: 只传入进程编号；调用失败时没有其他影响。
    unsafe { AllowSetForegroundWindow(pid) };
}

/// 从命令行启动时接上该命令行的控制台，之后的 `println!`、`eprintln!` 输出到该控制台；从图形
/// 界面启动（没有父控制台）时什么也不做。
///
/// winc 是图形界面程序（`windows_subsystem = "windows"`），系统不会为它准备控制台。
#[allow(unsafe_code)] // AttachConsole
pub fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: 只接上父进程的控制台，失败（没有父控制台）时没有效果。
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// 返回显示器 `monitor`（`HMONITOR`）可用区域的宽度和高度（物理像素）；读取失败时返回 `None`。
///
/// 可用区域是显示器减去任务栏等系统保留区域后的部分，最大化的窗口占满可用区域。
#[allow(unsafe_code)] // GetMonitorInfoW
pub fn work_size(monitor: isize) -> Option<(f64, f64)> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    // SAFETY: `MONITORINFO` 只含整数字段，全零是有效值。
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    // SAFETY: `info` 是可写的栈变量，`cbSize` 已按结构体大小填写；句柄无效时调用失败并返回 0。
    if unsafe { GetMonitorInfoW(monitor as _, &mut info) } == 0 {
        return None;
    }
    let work = info.rcWork;
    Some((
        f64::from(work.right - work.left),
        f64::from(work.bottom - work.top),
    ))
}
