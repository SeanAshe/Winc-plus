//! macOS 上 `winit` 没有提供、需要直接调用 `AppKit` 的能力。
//!
//! 包括窗口背后的 `NSWindow`、菜单（含「窗口」菜单和程序坞菜单）、文件对话框、目录菜单和帧同步
//! （`CADisplayLink`）。所有 Objective-C 消息发送都集中在本模块；除特别说明外，函数只能在主线程
//! 调用，各处的线程前提由紧贴 `unsafe` 块的 `// SAFETY:` 注释说明。

// 范围：本模块及其全部子模块；每个 `unsafe` 块都有 `// SAFETY:` 说明。
#![allow(unsafe_code)] // Objective-C 消息发送和运行时

mod app_menu;
mod dialogs;
mod display_link;
mod files;
mod folder_menu;
pub mod install;
mod pasteboard;
mod recent_documents;
mod text_input;
mod updates;
mod window;
mod window_menu;

pub use app_menu::{
    add_install_command_menu, add_settings_menu, intercept_quit, on_menu_command, on_open_files,
    on_reopen, suspend_menu_shortcuts,
};
pub use dialogs::{
    AlertButton, MOD_COMMAND, MenuIcon, MenuItem, MenuPlace, Panel, alert, key_equivalent,
    popup_menu, show_panel,
};
pub use display_link::DisplayLink;
pub use files::{default_browser, move_to_trash};
pub use folder_menu::{FolderItem, on_folder_menu, popup_folder_menu};
// 测试中由 `App` 上的字段代替剪贴板，不使用这两个函数。
#[cfg_attr(test, allow(unused_imports))]
pub use pasteboard::{pasteboard_has_image, pasteboard_png};
pub use recent_documents::sync_recent_documents;
pub use text_input::{capture_inserted_text, with_inserted_text};
pub use updates::{check as check_for_updates, start as start_updates};
pub use window::{
    TrafficLights, has_sheet, keep_title_bar_drags, make_invisible, mouse_location,
    preferred_language, reduce_motion, show_without_focus, title_bar_double_click, under_cursor,
};
pub use window_menu::add_window_menus;

use std::ffi::{CStr, CString, OsStr, c_char};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use objc2::ffi;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{class, msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use winc_render::Image;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// 返回 `winit` 窗口的内容视图（`NSView`），不是 `AppKit` 窗口时返回 `None`。
fn ns_view(window: &Window) -> Option<&AnyObject> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };

    // SAFETY: 句柄来自仍然存活的 winit 窗口，`ns_view` 指向有效的 NSView；调用方在主线程上调用，
    // 满足 AppKit 的线程要求。返回引用的生命周期与 `window` 相同，winit 窗口存活期间其 NSView
    // 保持有效。
    Some(unsafe { handle.ns_view.cast().as_ref() })
}

/// 返回 `winit` 窗口背后的 `NSWindow`，不是 `AppKit` 窗口时返回 `None`。
fn ns_window(window: &Window) -> Option<&AnyObject> {
    let view = ns_view(window)?;
    // SAFETY: `view` 是主线程上有效的 NSView，`window` 是其公开属性，可能为空，先判空再使用。
    // winit 窗口存活期间其 NSWindow 保持有效，返回引用的生命周期与 `window` 相同。
    unsafe {
        let ns_window: *mut AnyObject = msg_send![view, window];
        ns_window.as_ref()
    }
}

/// 为 `class` 添加实例方法 `sel`，实现为 `imp`、类型编码为 `types`，返回是否添加成功。
///
/// `class` 自身已有该方法时不修改类并返回 `false`，不替换已有的实现。
///
/// # Safety
///
/// `imp` 必须是由 `extern "C-unwind" fn` 转换来的函数指针，其签名与 `types` 描述的返回值和参数
/// 类型一致；`Imp` 是不带参数类型的函数指针，运行时按该编码传参。
unsafe fn add_raw_method(class: &AnyClass, sel: Sel, imp: Imp, types: &CStr) -> bool {
    let class = (class as *const AnyClass).cast_mut();
    // SAFETY: `class` 是有效的类对象，`types` 是以 NUL 结尾的 C 字符串，借用保证它在调用期间有效；
    // `imp` 的签名由调用方保证与编码一致。
    unsafe { ffi::class_addMethod(class, sel, imp, types.as_ptr()).as_bool() }
}

#[cfg(test)]
mod tests;
