//! 粘贴到本机终端的图片：存为本机的 PNG 文件，再把文件的路径作为一次粘贴发给程序。
//!
//! 只在 Windows 上这样处理：Ctrl+V 是终端的粘贴键，不发给程序，程序读取剪贴板图片所用的按键
//! 各不相同，读取也可能失败。终端读取图片并存为文件，粘贴的内容与把该文件拖进终端相同，远端
//! 标签的做法见 `upload`。其他平台上 ⌃V 发给程序，剪贴板中的图片由程序自己读取。

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use winc_backend::fs;
use winc_i18n::t;

use super::editor::path_text;
use super::upload::write_new;
use super::*;
use crate::app::NoticeKind;
use crate::app::editor::clipboard::ClipImage;

/// 是否把剪贴板中只有的图片存为文件后粘贴路径，见模块文档。
const IMAGE_AS_FILE: bool = cfg!(windows);

/// 存放的图片保留的时间（7 天）：程序可能在发送消息时才读取文件；与远端存放目录
/// （`upload::PREPARE`）的保留时间相同。
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// 测试中代替存放目录（[`paste_dir`]）的目录，避免写进系统的临时目录。
#[cfg(test)]
pub(crate) static DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

impl App {
    /// Windows 上本机的标签 `id`（当前目录为 `cwd`）中粘贴、剪贴板中没有文字（`text_empty`
    /// 为真）时，把剪贴板中的图片存为文件后粘贴其路径（[`App::save_clipboard_image`]）。
    ///
    /// 返回是否这样处理了；其他平台上、标签在远端或 WSL 中、剪贴板里没有图片时返回 `false`，
    /// 按原来的方式粘贴。
    pub(super) fn paste_image_file(&mut self, id: u16, cwd: &[u8], text_empty: bool) -> bool {
        if !IMAGE_AS_FILE || !text_empty {
            return false;
        }
        // 当前目录是 Unix 路径时（在标签里 `ssh` 到了其他主机），程序打不开本机的文件；WSL 中的
        // 程序同样不认 Windows 的路径。
        let path = bytes_path(cwd);
        if cwd.starts_with(b"/")
            || winc_backend::is_remote(&path)
            || winc_editor::wsl::locate(&path).is_some()
        {
            return false;
        }
        self.save_clipboard_image(id)
    }

    /// 在后台把剪贴板中的图片存为 PNG 文件（[`save`]），存好后把路径粘贴到标签 `id`
    /// （[`App::terminal_image_saved`]）；剪贴板中没有图片时返回 `false`。
    ///
    /// 测试中没有事件循环，就地完成。
    pub(in crate::app) fn save_clipboard_image(&mut self, id: u16) -> bool {
        let Some(image) = self.clipboard.image() else {
            return false;
        };
        let task = move || save(&paste_dir(), image).map_err(|e| e.to_string());
        match self.link.proxy.clone() {
            Some(proxy) => {
                winc_editor::background::spawn_blocking(move || {
                    let _ = proxy.send_event(UserEvent::TerminalImageSaved(id, task()));
                });
            }
            None => {
                let result = task();
                self.terminal_image_saved(id, result);
            }
        }
        true
    }

    /// 剪贴板中的图片已存为文件（[`App::save_clipboard_image`]）：把文件的路径粘贴到标签 `id`；
    /// 该标签已不是正在看的标签时不粘贴，提示存到了哪里。失败时说明原因。
    pub(crate) fn terminal_image_saved(&mut self, id: u16, result: Result<PathBuf, String>) {
        let path = match result {
            Ok(path) => path_text(&path),
            Err(error) => {
                self.notify(
                    NoticeKind::Error,
                    t!("terminal-image-paste-failed", error = error),
                );
                return;
            }
        };
        if self.terminal.tab().is_some_and(|tab| tab.id == id) {
            self.confirmed_terminal_paste(id, path);
        } else {
            self.notify(
                NoticeKind::Done,
                t!("terminal-image-saved-elsewhere", path = path),
            );
        }
    }
}

/// 返回存放图片的目录：系统临时目录下的 `winc/paste`。Windows 的临时目录位于用户目录中，只有
/// 本人可以访问。
fn paste_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = DIR.lock().unwrap().clone() {
        return dir;
    }
    std::env::temp_dir().join("winc").join("paste")
}

/// 把 `image` 编码为 PNG，存入 `dir`，返回文件的路径；名字是 `<毫秒时间戳>-paste.png`，重名时
/// 在时间戳后加编号。顺便删去存放超过 [`KEEP`] 的图片。
///
/// 编码与图片的大小成正比，删除旧图片与目录中的文件数成正比：应在后台线程调用。
///
/// # Errors
///
/// 剪贴板中的像素数据无法编码、目录无法创建或写入失败时返回对应的错误。
pub(super) fn save(dir: &Path, image: ClipImage) -> io::Result<PathBuf> {
    let png = image
        .into_png()
        .ok_or_else(|| io::Error::other(t!("terminal-upload-no-image")))?;
    fs::create_dir_all(dir)?;
    let now = SystemTime::now();
    remove_old(dir, now.checked_sub(KEEP).unwrap_or(UNIX_EPOCH));
    let stamp = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    write_new(dir, stamp, "paste.png", &png)
}

/// 删去 `dir` 中修改时间早于 `before` 的 PNG 文件；读不到的目录和删不掉的文件跳过。
fn remove_old(dir: &Path, before: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let before = fs::Stamp::from(before);
    for entry in entries.flatten().filter(|entry| entry.kind.is_file()) {
        let path = entry.path();
        let old = (fs::metadata(&path).ok())
            .and_then(|meta| meta.modified)
            .is_some_and(|stamp| stamp < before);
        if old && path.extension().is_some_and(|ext| ext == "png") {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests;
