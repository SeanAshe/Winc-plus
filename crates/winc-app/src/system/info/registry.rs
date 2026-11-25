//! 读取 `HKEY_LOCAL_MACHINE` 下的字符串和 32 位整数值。

#![allow(unsafe_code)] // 读取注册表只能调用系统接口

use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};

/// 字符串值的缓冲区长度（UTF-16 码元，含结尾的 NUL）：版本号这类值远短于此，更长的值读取失败后
/// 返回 `None`。
const TEXT_LEN: usize = 256;

/// 返回以 NUL 结尾的 UTF-16 文字。
fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain([0])
        .collect()
}

/// 读取 `HKLM\key` 中的字符串值 `name`；不存在、类型不对或过长时返回 `None`。
pub(super) fn string(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let mut text = [0u16; TEXT_LEN];
    let mut bytes = (TEXT_LEN * 2) as u32;
    // SAFETY: `key`、`name` 以 NUL 结尾；`text` 和 `bytes` 描述同一个缓冲区；类型输出为空。
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            text.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    // `bytes` 含结尾的 NUL。
    let len = (bytes as usize / 2).saturating_sub(1);
    let value = String::from_utf16(&text[..len]).ok()?;
    (!value.is_empty()).then_some(value)
}

/// 读取 `HKLM\key` 中的 32 位整数值 `name`；不存在或类型不对时返回 `None`。
pub(super) fn dword(key: &str, name: &str) -> Option<u32> {
    let (key, name) = (wide(key), wide(name));
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: `key`、`name` 以 NUL 结尾；`value` 和 `bytes` 描述同一个 4 字节的缓冲区。
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut value).cast(),
            &mut bytes,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}
