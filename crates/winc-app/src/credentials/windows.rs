//! Windows 凭据管理器中的普通凭据。

#![allow(unsafe_code, reason = "凭据管理器只有 C 接口")]

use std::mem::MaybeUninit;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredEnumerateW,
    CredFree, CredReadW, CredWriteW,
};

use super::SERVICE;

/// 返回条目 `name` 在凭据管理器中的目标名，以 NUL 结尾。
fn target(name: &str) -> Vec<u16> {
    format!("{SERVICE}:{name}")
        .encode_utf16()
        .chain([0])
        .collect()
}

pub(super) fn available() -> bool {
    true
}

pub(super) fn get(name: &str) -> Option<String> {
    let target = target(name);
    let mut found = MaybeUninit::<*mut CREDENTIALW>::uninit();
    // SAFETY: `target` 以 NUL 结尾，`found` 是有效的输出指针。
    let read = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, found.as_mut_ptr()) };
    if read == 0 {
        return None;
    }
    // SAFETY: 调用成功时系统已写入 `found`。
    let found = unsafe { found.assume_init() };
    if found.is_null() {
        return None;
    }
    // SAFETY: `found` 非空，指向系统分配的凭据，其中的指针和长度描述同一块内存。
    let secret = unsafe {
        let credential = &*found;
        let size = credential.CredentialBlobSize as usize;
        if credential.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(credential.CredentialBlob, size).to_vec()
        }
    };
    // SAFETY: `found` 由 `CredReadW` 分配，此后不再使用。
    unsafe { CredFree(found.cast()) };
    String::from_utf8(secret).ok()
}

pub(super) fn set(name: &str, secret: &str) -> bool {
    let mut target = target(name);
    let mut secret = secret.as_bytes().to_vec();
    let Ok(size) = u32::try_from(secret.len()) else {
        return false;
    };
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Comment: null_mut(),
        LastWritten: FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
        CredentialBlobSize: size,
        CredentialBlob: secret.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: null_mut(),
        TargetAlias: null_mut(),
        UserName: null_mut(),
    };
    // SAFETY: `credential` 中的指针在调用期间有效，系统复制其内容。
    unsafe { CredWriteW(&credential, 0) != 0 }
}

pub(super) fn delete(name: &str) {
    let target = target(name);
    // SAFETY: `target` 以 NUL 结尾。
    unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
}

pub(super) fn names() -> Vec<String> {
    let filter = target("*");
    let mut count = 0;
    let mut found = MaybeUninit::<*mut *mut CREDENTIALW>::uninit();
    // SAFETY: `filter` 以 NUL 结尾，`count`、`found` 是有效的输出指针。
    let listed = unsafe { CredEnumerateW(filter.as_ptr(), 0, &mut count, found.as_mut_ptr()) };
    if listed == 0 {
        return Vec::new();
    }
    // SAFETY: 调用成功时系统已写入 `found`。
    let found = unsafe { found.assume_init() };
    if found.is_null() {
        return Vec::new();
    }
    let prefix = format!("{SERVICE}:");
    // SAFETY: `found` 非空，指向 `count` 个凭据指针；非空的目标名以 NUL 结尾。
    let names = unsafe {
        std::slice::from_raw_parts(found, count as usize)
            .iter()
            .filter(|credential| !credential.is_null())
            .filter_map(|&credential| {
                let name = (*credential).TargetName;
                if name.is_null() {
                    return None;
                }
                let length = (0..).take_while(|&i| *name.add(i) != 0).count();
                let name = String::from_utf16(std::slice::from_raw_parts(name, length)).ok()?;
                name.strip_prefix(&prefix).map(str::to_owned)
            })
            .collect()
    };
    // SAFETY: `found` 由 `CredEnumerateW` 分配，此后不再使用。
    unsafe { CredFree(found.cast()) };
    names
}
