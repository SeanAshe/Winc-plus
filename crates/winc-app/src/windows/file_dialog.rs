//! 系统文件对话框的拥有者窗口。
//!
//! 对话框在后台线程上运行，拥有者必须是同一线程的窗口：拥有者属于其他线程时，系统挂接两个线程
//! 的输入队列，界面线程的事件循环随之空转。

// Win32 调用；句柄只在创建它们的线程上使用，主窗口的句柄只用于跨线程安全的调用。
#![allow(unsafe_code)]

use std::num::NonZeroIsize;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EnumThreadWindows, GetPropW, GetWindowRect, IsWindowVisible,
    RemovePropW, SetForegroundWindow, SetPropW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use winit::window::Window;

/// 主窗口上记录对话框所在线程编号的窗口属性名。
const DIALOG_THREAD: &str = "WincFileDialogThread";

/// 以 NUL 结尾的 UTF-16 字符串。
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// 主窗口的句柄，可以交给运行对话框的后台线程。
#[derive(Debug, Clone, Copy)]
pub struct MainWindow(isize);

impl MainWindow {
    /// 返回 `window` 的句柄；不是 Win32 窗口时返回 `None`。
    pub fn of(window: &Window) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        Some(MainWindow(handle.hwnd.get()))
    }

    fn hwnd(self) -> HWND {
        self.0 as HWND
    }
}

/// 对话框的拥有者：当前线程上与主窗口重合的隐藏窗口。存在期间主窗口被禁用。
///
/// 释放时恢复并激活主窗口，再销毁隐藏窗口；必须在创建它的线程上释放。
#[derive(Debug)]
pub struct DialogOwner {
    main: MainWindow,
    owner: NonZeroIsize,
}

impl DialogOwner {
    /// 在当前线程上为主窗口 `main` 创建拥有者并禁用主窗口；创建失败时返回 `None`。
    pub fn new(main: MainWindow) -> Option<Self> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let class = wide("STATIC");
        // SAFETY: `rect` 是可写的栈变量，`class` 以 NUL 结尾且在调用期间有效；主窗口已销毁时
        // `GetWindowRect` 失败，隐藏窗口放在原点。
        let owner = unsafe {
            GetWindowRect(main.hwnd(), &mut rect);
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                null(),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        let owner = NonZeroIsize::new(owner as isize)?;
        let name = wide(DIALOG_THREAD);
        // SAFETY: `name` 以 NUL 结尾且在调用期间有效；属性值是线程编号，不是指针。
        unsafe {
            SetPropW(
                main.hwnd(),
                name.as_ptr(),
                GetCurrentThreadId() as usize as _,
            );
            EnableWindow(main.hwnd(), 0);
        }
        Some(DialogOwner { main, owner })
    }
}

impl Drop for DialogOwner {
    fn drop(&mut self) {
        let name = wide(DIALOG_THREAD);
        // SAFETY: `owner` 由本线程创建且只销毁一次；`name` 以 NUL 结尾且在调用期间有效。
        unsafe {
            RemovePropW(self.main.hwnd(), name.as_ptr());
            EnableWindow(self.main.hwnd(), 1);
            SetForegroundWindow(self.main.hwnd());
            DestroyWindow(self.owner.get() as HWND);
        }
    }
}

impl HasWindowHandle for DialogOwner {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.owner));
        // SAFETY: 隐藏窗口在 `self` 释放之前一直存在。
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for DialogOwner {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

/// 窗口 `window` 有打开的文件对话框时，把对话框切到前台。
pub fn raise_file_dialog(window: &Window) {
    let Some(main) = MainWindow::of(window) else {
        return;
    };
    let name = wide(DIALOG_THREAD);
    // SAFETY: `name` 以 NUL 结尾且在调用期间有效；属性值是线程编号。
    let thread = unsafe { GetPropW(main.hwnd(), name.as_ptr()) } as usize as u32;
    if thread == 0 {
        return;
    }
    // SAFETY: 回调只读取窗口句柄，不保存。
    unsafe { EnumThreadWindows(thread, Some(raise_visible), 0) };
}

/// 把第一个可见的窗口（对话框）切到前台并停止枚举。
unsafe extern "system" fn raise_visible(hwnd: HWND, _: LPARAM) -> BOOL {
    // SAFETY: `hwnd` 由系统在枚举时传入，枚举期间有效。
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        SetForegroundWindow(hwnd);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

    #[test]
    fn the_owner_disables_the_main_window_until_it_is_dropped() {
        let class = wide("STATIC");
        // SAFETY: `class` 以 NUL 结尾且在调用期间有效；窗口在测试结束前销毁。
        let main = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                100,
                100,
                null_mut(),
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!main.is_null());
        let main_window = MainWindow(main as isize);
        let name = wide(DIALOG_THREAD);

        let owner = DialogOwner::new(main_window).unwrap();
        let hidden = owner.owner.get() as HWND;
        let RawWindowHandle::Win32(handle) = owner.window_handle().unwrap().as_raw() else {
            panic!("不是 Win32 窗口");
        };
        assert_eq!(handle.hwnd, owner.owner);
        // SAFETY: 两个窗口都由本线程创建且尚未销毁；`name` 以 NUL 结尾。
        unsafe {
            assert_eq!(IsWindowEnabled(main), 0);
            assert!(!GetPropW(main, name.as_ptr()).is_null());
        }

        drop(owner);
        // SAFETY: 同上；隐藏窗口已销毁，`IsWindow` 只检查句柄。
        unsafe {
            assert_ne!(IsWindowEnabled(main), 0);
            assert!(GetPropW(main, name.as_ptr()).is_null());
            assert_eq!(IsWindow(hidden), 0);
            DestroyWindow(main);
        }
    }
}
