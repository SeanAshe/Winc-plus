//! 文件操作：移到废纸篓，以及 Rust 字符串、路径和 NSString、NSURL 之间的转换。

use super::*;

/// 把文件或目录移到废纸篓；移入后用户可在废纸篓中通过「放回原处」恢复。
///
/// # Errors
///
/// `path` 不是有效的 UTF-8、`NSFileManager` 不可用，或系统拒绝移动时返回 `io::Error`。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：废纸篓只接收本机的文件"
)]
pub fn move_to_trash(path: &Path) -> std::io::Result<()> {
    // 按链接本身判断是否为目录：指向目录的符号链接不当作目录，URL 末尾不加 `/`，移走的是链接
    // 而不是它指向的目录。
    let is_dir = path.symlink_metadata().is_ok_and(|m| m.is_dir());
    let url = file_url(path, is_dir)
        .ok_or_else(|| std::io::Error::other(winc_i18n::t!("common-trash-invalid-path")))?;

    // SAFETY: `NSFileManager` 的 `defaultManager` 和 `trashItemAtURL:resultingItemURL:error:`
    // 是公开 API，可在任意线程调用。`url` 由 `Retained` 持有，在本块内有效；`resulting` 传空
    // 指针表示不需要新位置；`error` 指向局部变量，调用失败时被写入自动释放的 `NSError`，在
    // 当前自动释放池排空前有效，本块内只读取其描述。
    unsafe {
        let manager: *mut AnyObject = msg_send![class!(NSFileManager), defaultManager];
        let manager = manager
            .as_ref()
            .ok_or_else(|| std::io::Error::other(winc_i18n::t!("common-trash-no-file-manager")))?;
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let resulting = std::ptr::null_mut::<*mut AnyObject>();
        let ok: bool = msg_send![
            manager,
            trashItemAtURL: &*url,
            resultingItemURL: resulting,
            error: &mut error
        ];
        if ok {
            return Ok(());
        }

        let reason = error
            .as_ref()
            .and_then(|error| {
                let text: *mut AnyObject = msg_send![error, localizedDescription];
                rust_string(text.as_ref()?)
            })
            .unwrap_or_else(|| winc_i18n::t!("common-trash-failed").to_owned());
        Err(std::io::Error::other(reason))
    }
}

/// 默认浏览器（打开 `https` 链接的应用）的路径；取不到时返回 `None`。
pub fn default_browser() -> Option<PathBuf> {
    let link = ns_string("https://example.com")?;
    // SAFETY: `URLWithString:`、`sharedWorkspace` 和 `URLForApplicationToOpenURL:` 是公开 API，
    // 返回自动释放的对象（或空指针），判空后在本块内使用；`link` 由 `Retained` 持有，存活至
    // 本块结束；`url_path` 要求的有效 NSURL 由判空保证。
    unsafe {
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: &*link];
        let url = url.as_ref()?;
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut AnyObject = msg_send![workspace.as_ref()?, URLForApplicationToOpenURL: url];
        url_path(app.as_ref()?)
    }
}

/// 把 Rust 字符串转成 NSString（不要求以 NUL 结尾）；创建失败时返回 `None`。
pub(super) fn ns_string(text: &str) -> Option<Retained<AnyObject>> {
    // NSUTF8StringEncoding = 4。
    // SAFETY: `alloc` 加 `initWithBytes:length:encoding:` 返回 +1 的对象，交给 `Retained` 管理；
    // `text` 的指针和长度描述有效的 UTF-8 字节，在调用期间有效，NSString 会复制一次。
    unsafe {
        let string: *mut AnyObject = msg_send![class!(NSString), alloc];
        let string: *mut AnyObject = msg_send![
            string,
            initWithBytes: text.as_ptr().cast::<std::ffi::c_void>(),
            length: text.len(),
            encoding: 4usize
        ];
        Retained::from_raw(string)
    }
}

/// 把 `NSString` 拷贝成 Rust 字符串，无效的 UTF-8 换成替换字符；取不到内容时返回 `None`。
///
/// # Safety
///
/// `ns` 必须指向有效的 NSString，并在调用期间保持存活。
pub(super) unsafe fn rust_string(ns: &AnyObject) -> Option<String> {
    // SAFETY: 由调用方保证 `ns` 是有效的 NSString；`UTF8String` 指向的内存在 `ns` 释放前有效，
    // 判空后立即拷贝。
    unsafe {
        let utf8: *const c_char = msg_send![ns, UTF8String];
        (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
    }
}

/// 把路径转成 `file://` 的 NSURL；`is_dir` 指明路径是否为目录。
///
/// 路径不是有效的 UTF-8 时返回 `None`。
pub(super) fn file_url(path: &Path, is_dir: bool) -> Option<Retained<AnyObject>> {
    let path = ns_string(path.to_str()?)?;
    // SAFETY: `path` 是有效的 NSString，存活至本块结束；`fileURLWithPath:isDirectory:` 是
    // 公开的类方法，返回自动释放的对象，`Retained::retain` 持有时即取得所有权，再返回。
    unsafe {
        let url: *mut AnyObject =
            msg_send![class!(NSURL), fileURLWithPath: &*path, isDirectory: is_dir];
        Retained::retain(url)
    }
}

/// 把 `file://` 的 NSURL 转回路径，使用文件系统中的原始字节，不经过字符串；不是文件 URL 时
/// 返回 `None`。
///
/// # Safety
///
/// `url` 必须指向有效的 NSURL 对象，并在调用期间保持存活。
pub(super) unsafe fn url_path(url: &AnyObject) -> Option<PathBuf> {
    // SAFETY: 由调用方保证 `url` 是有效的 NSURL；`fileSystemRepresentation` 返回的 C 字符串
    // 在 `url` 释放前有效，此处立即拷贝。不是文件 URL 时（`isFileURL` 为假）它返回空字符串
    // 而不是空指针，因此先问 `isFileURL`；空指针也在拷贝前检查。
    unsafe {
        let is_file: bool = msg_send![url, isFileURL];
        if !is_file {
            return None;
        }
        let path: *const c_char = msg_send![url, fileSystemRepresentation];
        (!path.is_null()).then(|| PathBuf::from(OsStr::from_bytes(CStr::from_ptr(path).to_bytes())))
    }
}

#[cfg(test)]
mod tests;
