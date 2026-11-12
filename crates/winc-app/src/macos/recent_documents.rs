//! 系统的最近文档列表：程序坞菜单顶部和「最近使用的项目」中列出的最近打开的目录。
//!
//! 列表由 `NSDocumentController` 保存，winc 未运行时同样可用；选中其中一项时系统经
//! `application:openURLs:` 把目录交回（见 `on_open_files`）。列表不能单独移除某一项，
//! 最近列表变化时清空后按顺序重新记入。

use super::*;

/// 最近一次记入系统的目录；与当前的最近列表相同时不重写。
static NOTED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

/// 把最近打开的目录 `recent`（最近的在前）记入系统的最近文档列表。
///
/// 与上一次记入的目录相同时不做任何事。只能在主线程调用（`AppKit` 的线程要求）。
pub fn sync_recent_documents(recent: &[PathBuf]) {
    let Ok(mut noted) = NOTED.lock() else {
        return;
    };
    if noted.as_deref() == Some(recent) {
        return;
    }
    *noted = Some(recent.to_vec());
    drop(noted);

    // SAFETY: `NSDocumentController` 的 `sharedDocumentController`、`clearRecentDocuments:`、
    // `noteNewRecentDocumentURL:` 是公开 API，调用方保证在主线程调用；控制器判空后使用，传入的
    // `NSURL` 在调用期间由 `Retained` 持有。
    unsafe {
        let controller: *mut AnyObject =
            msg_send![class!(NSDocumentController), sharedDocumentController];
        let Some(controller) = controller.as_ref() else {
            return;
        };
        let () = msg_send![controller, clearRecentDocuments: std::ptr::null_mut::<AnyObject>()];
        // 后记入的排在前面：从最早的开始记入，最近的目录最后记入。
        for folder in recent.iter().rev() {
            if let Some(url) = files::file_url(folder, true) {
                let () = msg_send![controller, noteNewRecentDocumentURL: &*url];
            }
        }
    }
}

#[cfg(test)]
mod tests;
