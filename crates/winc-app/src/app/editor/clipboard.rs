//! 剪贴板：读写系统剪贴板中的文本、文件和图片，并记录最近一次整行复制的文本。
//!
//! 测试不访问系统剪贴板：写入的内容保存在 [`Clipboard`] 的字段中，读取时从这些字段取，
//! 测试直接检查或预置这些字段。

use std::path::PathBuf;

/// 一个窗口使用的剪贴板。
#[derive(Default)]
pub(crate) struct Clipboard {
    /// 系统剪贴板，首次使用时才初始化。
    #[cfg(not(test))]
    system: Option<arboard::Clipboard>,
    /// 测试中代替系统剪贴板的文本。
    #[cfg(test)]
    pub copied: Option<String>,
    /// 测试中代替系统剪贴板的文件列表。
    #[cfg(test)]
    pub copied_files: Vec<PathBuf>,
    /// 测试中代替系统剪贴板的图片（PNG）。
    #[cfg(test)]
    pub copied_image: Option<Vec<u8>>,
    /// 最近一次整行复制的文本；写入的不是整行或写入文件时为 `None`。
    whole_line: Option<String>,
}

impl Clipboard {
    /// 返回剪贴板中的文本；没有文本或无法访问系统剪贴板时返回 `None`。
    pub(in crate::app) fn text(&mut self) -> Option<String> {
        #[cfg(test)]
        {
            self.copied.clone()
        }
        #[cfg(not(test))]
        {
            self.system()?.get_text().ok()
        }
    }

    /// 把 `text` 写入剪贴板，替换其中的文件和图片。
    ///
    /// `whole_line` 为 `true` 表示 `text` 是整行，见 [`Self::holds_line`]。
    pub(in crate::app) fn set_text(&mut self, text: String, whole_line: bool) {
        self.whole_line = whole_line.then(|| text.clone());
        #[cfg(test)]
        {
            self.copied = Some(text);
            self.copied_files.clear();
            self.copied_image = None;
        }
        #[cfg(not(test))]
        if let Some(system) = self.system() {
            let _ = system.set_text(text);
        }
    }

    /// 最近一次写入的是否是整行，并且内容与 `text` 相同。
    ///
    /// 剪贴板的内容被其他程序替换后，`text`（当前读到的内容）与记录的文本不同，返回 `false`。
    pub(in crate::app) fn holds_line(&self, text: &str) -> bool {
        self.whole_line.as_deref() == Some(text)
    }

    /// 返回剪贴板中的文件（从系统文件管理器或文件树中复制的）；没有时为空。
    pub(in crate::app) fn files(&mut self) -> Vec<PathBuf> {
        #[cfg(test)]
        {
            self.copied_files.clone()
        }
        #[cfg(not(test))]
        {
            self.system()
                .and_then(|system| system.get().file_list().ok())
                .unwrap_or_default()
        }
    }

    /// 把文件或目录 `paths` 写入剪贴板，替换其中的文本和图片。
    pub(in crate::app) fn set_files(&mut self, paths: &[PathBuf]) {
        self.whole_line = None;
        #[cfg(test)]
        {
            self.copied = None;
            self.copied_image = None;
            self.copied_files = paths.to_vec();
        }
        #[cfg(not(test))]
        if let Some(system) = self.system() {
            let _ = system.set().file_list(paths);
        }
    }

    /// 返回剪贴板中的图片，编码为 PNG；没有时返回 `None`。
    ///
    /// macOS 上直接取出剪贴板中的 PNG（只有 TIFF 时由系统转换）；其他平台上 arboard 给出 RGBA
    /// 像素，再编码为 PNG。
    pub(in crate::app) fn png(&mut self) -> Option<Vec<u8>> {
        #[cfg(test)]
        {
            self.copied_image.clone()
        }
        #[cfg(all(not(test), target_os = "macos"))]
        {
            crate::macos::pasteboard_png()
        }
        #[cfg(all(not(test), not(target_os = "macos")))]
        {
            let image = self.system()?.get_image().ok()?;
            winc_editor::paste::encode_png(image.width, image.height, &image.bytes)
        }
    }

    /// 剪贴板中是否有图片。
    ///
    /// macOS 上不取出数据；其他平台上需要解码一次。
    pub(in crate::app) fn has_image(&mut self) -> bool {
        #[cfg(test)]
        {
            self.copied_image.is_some()
        }
        #[cfg(all(not(test), target_os = "macos"))]
        {
            crate::macos::pasteboard_has_image()
        }
        #[cfg(all(not(test), not(target_os = "macos")))]
        {
            self.system()
                .is_some_and(|system| system.get_image().is_ok())
        }
    }

    /// 返回系统剪贴板，首次调用时创建；创建失败时返回 `None`。
    #[cfg(not(test))]
    fn system(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.system.is_none() {
            self.system = arboard::Clipboard::new().ok();
        }
        self.system.as_mut()
    }
}
