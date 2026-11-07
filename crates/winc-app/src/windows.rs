//! Windows 上交由系统的操作，读取的系统设置，按屏幕刷新绘制动画的显示链接，以及文件类型图标。

mod assoc;
mod display_link;

pub use assoc::{register_file_icons, register_file_types, unregister_file_types};
pub use display_link::DisplayLink;

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

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
