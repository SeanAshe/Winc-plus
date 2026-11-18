//! 把文件拖出窗口，交给系统的拖放：放进其他程序时复制，拖回 Winc 自己的窗口时不放下。

// COM 接口和 Win32 调用；每个 `unsafe` 块都有 `// SAFETY:` 说明。
#![allow(unsafe_code)]

use std::path::{Component, PathBuf};

use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HWND, S_OK,
};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, IDropSource, IDropSource_Impl};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHParseDisplayName,
};
use windows::core::{BOOL, HRESULT, HSTRING, implement};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetWindowThreadProcessId, IDC_NO, LoadCursorW, SetCursor,
    WindowFromPoint,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// 把 `paths` 中的各项交给系统的拖放，在拖放结束后返回；某一项无法交给系统时返回 `false`。
///
/// 必须在按住鼠标左键时调用。拖放期间系统接管鼠标，松开鼠标的事件不再送达窗口，窗口的其他事件
/// 在返回后才处理。
pub fn drag_files(window: &Window, paths: &[PathBuf]) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(handle.hwnd.get() as _);
    let Some(data) = data_object(paths) else {
        return false;
    };
    let source: IDropSource = Source.into();
    // SAFETY: `hwnd` 属于仍然存活的 `window`，`data` 和 `source` 在调用期间有效。winit 在创建
    // 窗口时已为本线程初始化 OLE；拖放期间到达的窗口事件由 winit 缓存，返回后再派发。
    // 失败时也可能已经接管过鼠标，不再回到窗口内的拖动。
    let _ = unsafe { SHDoDragDrop(Some(hwnd), &data, &source, DROPEFFECT_COPY) };
    true
}

/// 返回描述 `paths` 中各项的外壳数据对象；某一项无法解析时返回 `None`。
///
/// 外壳不解析带 `\\?\` 前缀的路径（过长或以点结尾），这样的项不解析，直接返回 `None`。
fn data_object(paths: &[PathBuf]) -> Option<IDataObject> {
    let verbatim = |path: &PathBuf| match path.components().next() {
        Some(Component::Prefix(prefix)) => prefix.kind().is_verbatim(),
        _ => false,
    };
    if paths.iter().any(verbatim) {
        return None;
    }
    let mut items = Items(Vec::with_capacity(paths.len()));
    for path in paths {
        let name = HSTRING::from(std::path::absolute(path).ok()?.as_os_str());
        let mut item = std::ptr::null_mut();
        // SAFETY: `name` 在调用期间有效；成功时 `item` 指向新分配的项目标识，由 `Items` 释放。
        unsafe { SHParseDisplayName(&name, None, &mut item, 0, None) }.ok()?;
        items.0.push(item.cast_const());
    }
    // SAFETY: `items` 中都是有效的项目标识，函数会复制一份，不保留这些指针。
    let array = unsafe { SHCreateShellItemArrayFromIDLists(&items.0) }.ok()?;
    // SAFETY: `array` 是有效的接口；`BHID_DataObject` 是静态常量，调用期间有效。
    unsafe { array.BindToHandler(None, &BHID_DataObject) }.ok()
}

/// `SHParseDisplayName` 分配的项目标识，离开作用域时释放。
struct Items(Vec<*const ITEMIDLIST>);

impl Drop for Items {
    fn drop(&mut self) {
        for &item in &self.0 {
            // SAFETY: 每一项都由 `SHParseDisplayName` 分配，只释放一次。
            unsafe { CoTaskMemFree(Some(item.cast())) };
        }
    }
}

/// 拖放的来源：松开时鼠标在本程序的窗口上则取消，经过时显示禁止的指针。
#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.contains(MK_LBUTTON) {
            S_OK
        } else if over_own_window() {
            DRAGDROP_S_CANCEL
        } else {
            DRAGDROP_S_DROP
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        if !over_own_window() {
            return DRAGDROP_S_USEDEFAULTCURSORS;
        }
        // SAFETY: `IDC_NO` 是系统预置的指针，不需要模块句柄，也不需要释放。
        unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_NO)) };
        S_OK
    }
}

/// 鼠标是否在本程序的某个窗口上；按顶层窗口判断，嵌在窗口中的其他进程的子窗口也算。
fn over_own_window() -> bool {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` 是可写的栈变量；`WindowFromPoint` 找不到窗口时返回空句柄，`GetAncestor`
    // 和 `GetWindowThreadProcessId` 对空句柄分别返回空句柄和 0，`pid` 保持 0。
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(GetAncestor(WindowFromPoint(point), GA_ROOT), &mut pid);
        pid == std::process::id()
    }
}
