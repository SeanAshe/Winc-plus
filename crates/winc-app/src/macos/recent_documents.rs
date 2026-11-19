//! 系统的最近文档列表：程序坞菜单顶部和「最近使用的项目」中列出的最近打开的目录和文件。
//!
//! 列表由 `NSDocumentController` 保存，winc 未运行时同样可用；选中其中一项时系统经
//! `application:openURLs:` 把目录交回（见 `on_open_files`）。列表不能单独移除某一项，
//! 最近列表变化时清空后按顺序重新记入。

use super::*;

/// 最近一次记入系统的目录和文件；与这次要记入的相同时不重写。
static NOTED: Mutex<Option<(Vec<PathBuf>, Vec<PathBuf>)>> = Mutex::new(None);

/// 把最近打开的目录 `folders` 和文件 `documents`（都是最近的在前）记入系统的最近文档列表。
///
/// 先记文件、后记目录：系统设置的最近项目数较少时，留下的是目录。与上一次记入的相同时不做
/// 任何事。只能在主线程调用（`AppKit` 的线程要求）。
pub fn sync_recent_documents(folders: &[PathBuf], documents: &[PathBuf]) {
    let Ok(mut noted) = NOTED.lock() else {
        return;
    };
    let wanted = (folders.to_vec(), documents.to_vec());
    if noted.as_ref() == Some(&wanted) {
        return;
    }
    *noted = Some(wanted);
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
        // 后记入的排在前面：从最早的开始记入，最近的最后记入。
        let entries = (documents.iter().rev().map(|f| (f, false)))
            .chain(folders.iter().rev().map(|f| (f, true)));
        for (path, folder) in entries {
            if let Some(url) = files::file_url(path, folder) {
                let () = msg_send![controller, noteNewRecentDocumentURL: &*url];
            }
        }
    }
}

#[cfg(test)]
mod tests;
