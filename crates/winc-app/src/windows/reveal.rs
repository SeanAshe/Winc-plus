//! 经 shell 在默认的文件管理器中显示并选中文件或目录。

// COM 与 shell 调用；每个 `unsafe` 块都有 `// SAFETY:` 说明。
#![allow(unsafe_code)]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
use windows::core::PCWSTR;

/// 在后台线程中经 `SHOpenFolderAndSelectItems` 显示并选中 `path`；失败时只打开所在的目录。
///
/// # Errors
///
/// 无法创建后台线程时返回 `io::Error`。
pub fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let path = std::path::absolute(path)?;
    std::thread::Builder::new()
        .name("winc-reveal".to_owned())
        .spawn(move || {
            if select(&path).is_err() {
                let folder = path.parent().unwrap_or(&path);
                if let Err(e) = crate::system::open_with_default_app(folder) {
                    eprintln!("winc: 无法打开文件管理器：{e}");
                }
            }
        })
        .map(|_| ())
}

/// 请 shell 打开 `path` 的父目录并选中 `path`。
fn select(path: &Path) -> windows::core::Result<()> {
    // SAFETY: 本线程尚未初始化 COM；成功后在返回前调用一次 `CoUninitialize` 配对。
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` 以 NUL 结尾，在调用期间有效；返回的 PIDL 由下面的 `ILFree` 释放。
    let pidl = unsafe { ILCreateFromPathW(PCWSTR(wide.as_ptr())) };
    let result = if pidl.is_null() {
        Err(windows::core::Error::empty())
    } else {
        // SAFETY: `pidl` 是上面创建的完整 PIDL；`cidl` 为 0 时打开其父目录并选中该项。
        let result = unsafe { SHOpenFolderAndSelectItems(pidl, None, 0) };
        // SAFETY: `pidl` 由 `ILCreateFromPathW` 创建，只释放一次，此后不再使用。
        unsafe { ILFree(Some(pidl.cast_const())) };
        result
    };
    // SAFETY: 与上面成功的 `CoInitializeEx` 配对。
    unsafe { CoUninitialize() };
    result
}
