//! 粘贴文件、文本和图片，以及在文件树中复制文件；剪贴板中的内容依次取文件、文本、图片。
//! 同时有文本和图片时取文本：从文档、表格中复制的内容常常同时带着一张图片。
//!
//! 编辑区：粘贴到 Markdown 文档时文件写成链接、图片写成 `![…](…)`，剪贴板中的图片先存到
//! 文档所在的目录；其他文档中文件写成路径。文件树：文件复制到选中的目录，文本和图片粘贴为
//! 新文件，先在文件树中给出建议的名字，按 `↩` 确认。

use winc_editor::paste::{self, CopyError, Snippet};
use winc_editor::{Workspace, display_name};
use winc_i18n::t;

use super::*;

impl App {
    /// 返回剪贴板中的文件（从系统文件管理器或文件树中复制的）；没有时为空。
    pub(super) fn clipboard_files(&mut self) -> Vec<PathBuf> {
        #[cfg(test)]
        {
            self.copied_files.clone()
        }
        #[cfg(not(test))]
        {
            self.system_clipboard()
                .and_then(|clipboard| clipboard.get().file_list().ok())
                .unwrap_or_default()
        }
    }

    /// 把剪贴板中的图片存到临时目录 `winc-paste`，返回新文件的路径。没有图片时返回 `None`。
    ///
    /// 终端里的程序读不到剪贴板图片，粘贴时把这个路径发给它。文件名为 `image.png`，重名时
    /// 依次加 `-1`、`-2`，不覆盖已有文件。
    pub(in crate::app) fn clipboard_image_file(&mut self) -> Option<std::io::Result<PathBuf>> {
        let png = self.clipboard_png()?;
        let dir = std::env::temp_dir().join("winc-paste");
        if let Err(error) = winc_backend::fs::create_dir_all(&dir) {
            return Some(Err(error));
        }
        Some(paste::save_image(&dir, &png))
    }

    /// 返回剪贴板中的图片，编码为 PNG；没有时返回 `None`。
    ///
    /// macOS 上直接取出剪贴板中的 PNG（只有 TIFF 时由系统转换）；其他平台上 arboard 给出 RGBA
    /// 像素，再编码为 PNG。
    fn clipboard_png(&mut self) -> Option<Vec<u8>> {
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
            let image = self.system_clipboard()?.get_image().ok()?;
            paste::encode_png(image.width, image.height, &image.bytes)
        }
    }

    /// 剪贴板中是否有图片，尽量不取出数据（只有 macOS 能做到，其他平台上要解码一次）。
    fn clipboard_has_image(&mut self) -> bool {
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
            self.system_clipboard()
                .is_some_and(|clipboard| clipboard.get_image().is_ok())
        }
    }

    /// 剪贴板中是否有能粘贴到文件树中的内容（文件、文本或图片）。
    pub(in crate::app) fn can_paste_into_tree(&mut self) -> bool {
        !self.clipboard_files().is_empty()
            || self.clipboard_text().is_some_and(|text| !text.is_empty())
            || self.clipboard_has_image()
    }

    /// 把文件或目录 `path` 放到剪贴板中，可以粘贴到系统文件管理器、文件树或 Markdown 文档中。
    pub(in crate::app) fn copy_entry(&mut self, path: &Path) {
        self.copy_entries(&[path.to_path_buf()]);
    }

    /// 同 [`Self::copy_entry`]，一次放入 `paths` 里的各项（文件树里多选时）。
    pub(in crate::app) fn copy_entries(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        self.shared.cut_files.borrow_mut().clear();
        self.line_clipboard = None;
        #[cfg(test)]
        {
            self.copied = None;
            self.copied_image = None;
            self.copied_files = paths.to_vec();
        }
        #[cfg(not(test))]
        if let Some(clipboard) = self.system_clipboard() {
            let _ = clipboard.set().file_list(paths);
        }
        self.flash(match paths {
            [path] => t!("editor-paste-copied-item", name = display_name(path)),
            many => t!("editor-paste-copied-items", count = many.len()),
        });
    }

    /// 剪切文件树里的 `paths`：和复制一样放到剪贴板中，再记下这些路径；之后粘贴到文件树时把
    /// 它们移过去，而不是复制（粘贴到系统文件管理器、编辑区时仍是复制和插入）。
    pub(in crate::app) fn cut_entries(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        self.copy_entries(paths);
        *self.shared.cut_files.borrow_mut() = paths.to_vec();
        self.flash(match paths {
            [path] => t!("editor-paste-cut-item", name = display_name(path)),
            many => t!("editor-paste-cut-items", count = many.len()),
        });
    }

    /// 粘贴到编辑区：剪贴板中有文件时粘贴文件，其次是文本，最后是图片。
    pub(super) fn paste_into_editor(&mut self) {
        let files = self.clipboard_files();
        if !files.is_empty() {
            self.paste_files(&files);
        } else if let Some(text) = self.clipboard_text() {
            self.paste_text(text);
        } else if let Some(png) = self.clipboard_png() {
            self.paste_image(&png);
        }
    }

    /// 当前文档能否粘贴：不显示文本或只读时返回 `false`，只读时提示原因。
    ///
    /// 粘贴文件和图片前先检查，避免文档改不了却已在磁盘上复制了文件。
    fn can_paste_into_document(&mut self) -> bool {
        let Some(doc) = self.editor.active() else {
            return false;
        };
        if !doc.edits_text() {
            return false;
        }
        if let Some(reason) = doc.edit_blocked(&Command::Paste(String::new())) {
            self.flash(reason);
            return false;
        }
        true
    }

    /// 当前文档是否为 Markdown；是时返回其所在的目录（文档尚未保存时为 `None`）。
    fn markdown_dir(&self) -> Option<Option<PathBuf>> {
        let doc = self.editor.active()?;
        (doc.highlighter.language().id == "markdown").then(|| {
            doc.path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
    }

    /// 把文件 `files` 粘贴到当前文档中。
    ///
    /// Markdown 文档中写成链接（图片写成 `![…](…)`），项目之外的图片先复制到文档旁边；其他
    /// 文档中写成文件的绝对路径，每行一个。
    fn paste_files(&mut self, files: &[PathBuf]) {
        if !self.can_paste_into_document() {
            return;
        }
        let Some(doc_dir) = self.markdown_dir() else {
            let paths: Vec<_> = files.iter().map(|p| p.to_string_lossy()).collect();
            self.run(Command::Paste(paths.join("\n")));
            return;
        };

        let root = self.editor.root().map(Path::to_path_buf);
        let targets = match &doc_dir {
            Some(dir) => match paste::bring_images(dir, root.as_deref(), files) {
                Ok(targets) => targets,
                Err(CopyError { path, error }) => {
                    self.notify(t!(
                        "editor-paste-copy-failed",
                        name = display_name(&path),
                        error = error.to_string()
                    ));
                    return;
                }
            },
            None => files.to_vec(),
        };
        let copied: Vec<PathBuf> = targets
            .iter()
            .filter(|t| !files.contains(t))
            .cloned()
            .collect();
        let snippet = paste::markdown_links(doc_dir.as_deref(), root.as_deref(), &targets);
        self.insert_snippet(snippet);
        self.files_created(copied);
    }

    /// 把剪贴板中的图片 `png` 粘贴到当前文档：存到 Markdown 文档所在的目录中，插入
    /// `![image](image.png)` 并选中替代文字。
    ///
    /// 其他文档、尚未保存的 Markdown 文档中不能粘贴图片，提示原因。
    fn paste_image(&mut self, png: &[u8]) {
        let dir = match self.markdown_dir() {
            Some(Some(dir)) => dir,
            Some(None) => {
                self.flash(t!("editor-paste-image-save-first"));
                return;
            }
            None => {
                self.flash(t!("editor-paste-image-markdown-only"));
                return;
            }
        };
        if !self.can_paste_into_document() {
            return;
        }

        match paste::save_image(&dir, png) {
            Ok(path) => {
                let root = self.editor.root().map(Path::to_path_buf);
                let snippet =
                    paste::markdown_links(Some(&dir), root.as_deref(), std::slice::from_ref(&path));
                self.insert_snippet(snippet);
                self.files_created(vec![path]);
            }
            Err(e) => self.notify(t!("editor-paste-image-save-failed", error = e.to_string())),
        }
    }

    /// 在光标处插入 `snippet`，并选中其中的占位文字（有时）。
    fn insert_snippet(&mut self, snippet: Snippet) {
        let length = snippet.text.chars().count();
        self.run(Command::Paste(snippet.text));
        if let (Some(range), Some(doc)) = (snippet.placeholder, self.editor.active()) {
            let start = doc.selection.head.saturating_sub(length);
            self.select(start + range.start, start + range.end);
        }
    }

    /// 粘贴时在磁盘上新建了文件 `paths`：更新文件树和快速打开的文件表。
    fn files_created(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if let Some(ws) = self.editor.workspace.as_mut() {
            ws.refresh_paths(&paths);
        }
        self.picker_touched(paths);
        self.relayout();
    }

    /// 粘贴到文件树中选中的目录（选中文件时为其所在的目录，没有选中时为根目录）。
    ///
    /// 剪贴板中的文件复制过去，复制在后台进行，复制大目录时界面不会停住，完成后选中最后粘贴
    /// 的一项；是在文件树里剪切的（见 [`Self::cut_entries`]）时移过去，已在该目录中的不动。
    /// 文本、图片粘贴为新文件：在该目录中打开名字输入行，填好建议的名字（文本按内容猜扩展名，
    /// 图片为 `image.png`）并选中扩展名之前的部分，按 ↩ 新建并打开。
    pub(in crate::app) fn paste_into_tree(&mut self) {
        let Some(dir) = self.editor.workspace.as_ref().map(Workspace::target_dir) else {
            return;
        };

        let files = self.clipboard_files();
        // 剪贴板里还是剪切的那些文件：移过去，只移一次。
        if !files.is_empty() && files == *self.shared.cut_files.borrow() {
            self.shared.cut_files.borrow_mut().clear();
            let moving: Vec<PathBuf> = files
                .into_iter()
                .filter(|p| p.parent() != Some(dir.as_path()))
                .collect();
            self.move_entries(&moving, &dir);
            return;
        }
        if !files.is_empty() {
            let remote = winc_backend::is_remote(&dir)
                || files.iter().any(|path| winc_backend::is_remote(path));
            let copy = move |copying: &winc_editor::Copying| {
                paste::copy_into(&files, &dir, copying).map_err(|CopyError { path, error }| {
                    t!(
                        "editor-paste-failed",
                        name = display_name(&path),
                        error = error.to_string()
                    )
                })
            };
            if let Some(result) = self.spawn_copy(remote, copy, UserEvent::Pasted) {
                self.pasted_into_tree(result);
            }
            return;
        }

        if let Some(text) = self.clipboard_text().filter(|text| !text.is_empty()) {
            let name = paste::text_file_name(&dir, &text);
            self.start_pasted_entry(name, text.into_bytes());
        } else if let Some(png) = self.clipboard_png() {
            let name = paste::image_file_name(&dir);
            self.start_pasted_entry(name, png);
        } else {
            self.flash(t!("editor-paste-clipboard-empty"));
        }
    }

    /// 粘贴到文件树完成：刷新文件树，选中最后粘贴的一项；失败时提示原因（失败之前粘贴好的项
    /// 保留，同样显示出来）。
    pub(in crate::app) fn pasted_into_tree(&mut self, result: Result<Vec<PathBuf>, String>) {
        let pasted = result.unwrap_or_else(|message| {
            self.notify(message);
            Vec::new()
        });
        let Some(ws) = self.editor.workspace.as_mut() else {
            return;
        };
        ws.refresh();
        if let Some(last) = pasted.last() {
            ws.reveal(last);
        }
        if let Some(index) = ws.selected {
            self.scroll_tree_to(index, false);
        }
        self.picker_touched(pasted);
        self.relayout();
        self.request_redraw();
    }
}
