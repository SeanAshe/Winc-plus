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
use crate::app::workspace::copying::copy_outcome;
use crate::winc::RemoteCopy;

impl App {
    /// 返回剪贴板中的文件（从系统文件管理器或文件树中复制的）；没有时为空。
    ///
    /// 文件树里复制的远端文件不在系统剪贴板的文件列表中（见 [`Self::copy_entries`]）：剪贴板里
    /// 还是复制时写入的那段文字时取这些文件，先于系统剪贴板；文字已被替换时不再取，读系统
    /// 剪贴板。顺序不能反过来：复制远端文件时系统剪贴板里可能正好留着更早复制的本机文件。
    pub(in crate::app) fn clipboard_files(&mut self) -> Vec<PathBuf> {
        let copied = self.link.shared.remote_copy.borrow().clone();
        if let Some(copied) = copied {
            if self
                .clipboard
                .text()
                .is_some_and(|text| copied.holds(&text))
            {
                return copied.paths;
            }
            *self.link.shared.remote_copy.borrow_mut() = None;
        }
        self.clipboard.files()
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
        Some(paste::save_image(&dir, "image.png", &png))
    }

    /// 忘掉文件树里复制的远端文件：剪贴板里的文字是 Winc 刚写入的别的内容（即使碰巧与复制时
    /// 写入的路径相同，例如「复制路径」），不再当作那些文件。
    pub(in crate::app) fn forget_remote_copy(&mut self) {
        *self.link.shared.remote_copy.borrow_mut() = None;
    }

    /// 返回剪贴板中的图片，编码为 PNG；没有时返回 `None`。
    fn clipboard_png(&mut self) -> Option<Vec<u8>> {
        self.clipboard.png()
    }

    /// 剪贴板中是否有图片。
    fn clipboard_has_image(&mut self) -> bool {
        self.clipboard.has_image()
    }

    /// 剪贴板中是否有能粘贴到文件树中的内容（文件、文本或图片）。
    ///
    /// 文件树里复制的远端文件在剪贴板中是文字，按文字计。
    pub(in crate::app) fn can_paste_into_tree(&mut self) -> bool {
        self.clipboard_text().is_some_and(|text| !text.is_empty())
            || !self.clipboard.files().is_empty()
            || self.clipboard_has_image()
    }

    /// 把文件或目录 `path` 放到剪贴板中，可以粘贴到系统文件管理器、文件树或 Markdown 文档中。
    pub(in crate::app) fn copy_entry(&mut self, path: &Path) {
        self.copy_entries(&[path.to_path_buf()]);
    }

    /// 同 [`Self::copy_entry`]，一次放入 `paths` 里的各项（文件树里多选时）。
    ///
    /// 本机的文件放进系统剪贴板的文件列表。其中有远端的文件时，系统剪贴板放不下（见
    /// [`Clipboard::set_files`](super::clipboard::Clipboard::set_files)）：改为写入它们在远端主机上
    /// 的路径，文件本身记在各窗口共用的 [`RemoteCopy`] 中（见 [`Self::clipboard_files`]），粘贴到终
    /// 端时得到路径。写不进剪贴板时说明原因；返回是否放进了剪贴板。
    pub(in crate::app) fn copy_entries(&mut self, paths: &[PathBuf]) -> bool {
        if paths.is_empty() {
            return false;
        }
        let (written, copy) = if paths.iter().any(|path| winc_backend::is_remote(path)) {
            let lines: Vec<_> = paths
                .iter()
                .map(|p| winc_backend::native_lossy(p))
                .collect();
            let text = lines.join("\n");
            let written = self.clipboard.set_text(text.clone(), false);
            let copy = RemoteCopy {
                paths: paths.to_vec(),
                text,
            };
            (written, Some(copy))
        } else {
            (self.clipboard.set_files(paths), None)
        };
        if !written {
            self.notify(NoticeKind::Error, t!("editor-paste-clipboard-unavailable"));
            return false;
        }
        // 写入成功后才替换：写不进时剪贴板里还是上一次的内容，上一次的剪切和远端文件仍然有效。
        *self.link.shared.remote_copy.borrow_mut() = copy;
        self.link.shared.cut_files.borrow_mut().clear();
        self.flash(match paths {
            [path] => t!("editor-paste-copied-item", name = display_name(path)),
            many => t!("editor-paste-copied-items", count = many.len()),
        });
        true
    }

    /// 剪切文件树里的 `paths`：和复制一样放到剪贴板中，再记下这些路径；之后粘贴到文件树时把
    /// 它们移过去，而不是复制（粘贴到系统文件管理器、编辑区时仍是复制和插入）。没能放进剪贴板
    /// 时不记下。
    pub(in crate::app) fn cut_entries(&mut self, paths: &[PathBuf]) {
        if !self.copy_entries(paths) {
            return;
        }
        *self.link.shared.cut_files.borrow_mut() = paths.to_vec();
        self.flash(match paths {
            [path] => t!("editor-paste-cut-item", name = display_name(path)),
            many => t!("editor-paste-cut-items", count = many.len()),
        });
    }

    /// 粘贴到编辑区：依次尝试剪贴板中的文件、文本和图片。
    pub(super) fn paste_into_editor(&mut self) {
        let files = self.clipboard_files();
        if !files.is_empty() {
            self.paste_files(&files);
        } else if let Some(text) = self.clipboard_text() {
            // 网址和表格单元格的改写（`markdown_paste`）优先于可编辑的预览中 HTML 的转换。
            let markdown = match self.markdown_paste(&text) {
                Some(markdown) => Some(markdown),
                None => self.markdown_html_paste(&text),
            };
            match markdown {
                Some(markdown) => self.run(Command::Insert(markdown)),
                None => self.paste_text(text),
            }
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
    /// Markdown 文档中写成链接（图片写成 `![…](…)`），项目之外的图片先复制到图片目录；其他文档
    /// 中写成文件的绝对路径，每行一个。
    fn paste_files(&mut self, files: &[PathBuf]) {
        if !self.can_paste_into_document() {
            return;
        }
        let Some(doc_dir) = self.markdown_dir() else {
            // 远端的文件写成那台主机上的路径，不写虚拟路径。
            let paths: Vec<_> = files
                .iter()
                .map(|p| winc_backend::native_lossy(p))
                .collect();
            self.run(Command::Paste(paths.join("\n")));
            return;
        };

        let root = self.editor.root().map(Path::to_path_buf);
        let folder = self.markdown_image_folder();
        let targets = match (&doc_dir, &folder) {
            (Some(dir), Some(folder)) => {
                match paste::bring_images(dir, folder, root.as_deref(), files) {
                    Ok(targets) => targets,
                    Err(CopyError { path, error, done }) => {
                        self.notify(
                            NoticeKind::Error,
                            t!(
                                "editor-paste-copy-failed",
                                name = display_name(&path),
                                error = error.to_string()
                            ),
                        );
                        // 失败之前已复制的图片保留在磁盘上，同样刷新显示。
                        self.files_created(
                            done.into_iter().filter(|t| !files.contains(t)).collect(),
                        );
                        return;
                    }
                }
            }
            _ => files.to_vec(),
        };
        let mut copied: Vec<PathBuf> = targets
            .iter()
            .filter(|t| !files.contains(t))
            .cloned()
            .collect();
        // 图片目录可能是新建的，一并刷新。
        copied.extend(folder.filter(|f| !copied.is_empty() && doc_dir.as_ref() != Some(f)));
        let snippet = paste::markdown_links(doc_dir.as_deref(), root.as_deref(), &targets);
        self.insert_snippet(snippet);
        self.files_created(copied);
    }

    /// 把剪贴板中的图片 `png` 存入图片目录，在当前文档中插入 `![主名](…)` 并选中替代文字。
    ///
    /// 其他文档、尚未保存的 Markdown 文档中不能粘贴图片，提示原因。
    pub(in crate::app) fn paste_image(&mut self, png: &[u8]) {
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

        let Some(doc) = self.editor.active().and_then(|d| d.path.clone()) else {
            return;
        };
        let folder = paste::image_folder(&doc, self.config.markdown_image_folder.as_deref());
        let setting = self.config.markdown_image_name.as_deref();
        let name = paste::image_name(&doc, setting, local_stamp);
        match paste::save_image(&folder, &name, png) {
            Ok(path) => {
                let root = self.editor.root().map(Path::to_path_buf);
                let snippet =
                    paste::markdown_links(Some(&dir), root.as_deref(), std::slice::from_ref(&path));
                self.insert_snippet(snippet);
                let mut created = vec![path];
                created.extend((folder != dir).then_some(folder));
                self.files_created(created);
            }
            Err(e) => self.notify(
                NoticeKind::Error,
                t!("editor-paste-image-save-failed", error = e.to_string()),
            ),
        }
    }

    /// 返回当前文档的图片目录（`markdown.imageFolder`）；文档尚未保存时为 `None`。
    fn markdown_image_folder(&self) -> Option<PathBuf> {
        let doc = self.editor.active()?.path.as_deref()?;
        let setting = self.config.markdown_image_folder.as_deref();
        Some(paste::image_folder(doc, setting))
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
        // 剪贴板里还是剪切的那些文件：移过去，只移一次。不同主机之间不能移动（只能在同一个连接
        // 上改名），改为复制，原处的保留。
        if !files.is_empty() && files == *self.link.shared.cut_files.borrow() {
            self.link.shared.cut_files.borrow_mut().clear();
            let host = |path: &Path| winc_backend::locate(path).map(|at| at.target);
            if files.iter().all(|file| host(file) == host(&dir)) {
                let moving: Vec<PathBuf> = files
                    .into_iter()
                    .filter(|p| p.parent() != Some(dir.as_path()))
                    .collect();
                self.move_entries(&moving, &dir);
                return;
            }
            self.notify(NoticeKind::Info, t!("editor-paste-cut-copied-across-hosts"));
        }
        if !files.is_empty() {
            let remote = winc_backend::is_remote(&dir)
                || files.iter().any(|path| winc_backend::is_remote(path));
            let copy = move |copying: &winc_editor::Copying| {
                let copied = paste::copy_into(&files, &dir, copying);
                copy_outcome(copied, copying, |path, error| {
                    t!(
                        "editor-paste-failed",
                        name = display_name(path),
                        error = error.to_string()
                    )
                })
            };
            let done = |(done, failure), copying| UserEvent::Pasted(done, failure, copying);
            if let Some((done, failure)) = self.spawn_copy(remote, false, copy, done) {
                self.pasted_into_tree(done, failure);
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

    /// 粘贴到文件树完成：刷新文件树，选中最后粘贴的一项 `pasted`；失败时提示原因 `failure`
    /// （失败之前粘贴好的项保留，同样显示出来）。
    pub(in crate::app) fn pasted_into_tree(
        &mut self,
        pasted: Vec<PathBuf>,
        failure: Option<String>,
    ) {
        if let Some(message) = failure {
            self.notify(NoticeKind::Error, message);
        }
        if self.editor.workspace.is_none() {
            return;
        }
        let last = pasted.last().cloned();
        self.rescan_tree(move |app| {
            let Some(ws) = app.editor.workspace.as_mut() else {
                return;
            };
            if let Some(last) = &last {
                ws.reveal(last);
            }
            if let Some(index) = ws.selected {
                app.scroll_tree_to(index, false);
            }
            app.relayout();
            app.request_redraw();
        });
        self.picker_touched(pasted);
        self.relayout();
        self.request_redraw();
    }
}

/// 返回当前本地时间，格式为 `20261119-153012`，用于图片名中的 `${date}`。
fn local_stamp() -> String {
    let (y, mo, d, h, mi, sec) = crate::system::local_time();
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{sec:02}")
}
