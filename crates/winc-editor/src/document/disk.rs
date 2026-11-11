//! 文档与磁盘文件的同步：保存、另存为、磁盘文件被其他程序修改后的冲突与同步、按指定编码
//! 重新打开、恢复现场时尚未读取的文档，以及读文件和原子写入。

use super::*;

use winc_config::files::{create_temporary, sync_dir, sync_file};
use winc_i18n::t;

mod append;
mod save_job;
pub use save_job::{SaveJob, Saved};

/// [`Document::check_disk`] 的结果。
#[derive(Debug)]
pub enum DiskSync {
    /// 已经就地处理完。
    Done(DiskChange),
    /// 要读文件（只读末尾追加的部分，或者整个重读）：在后台 [`Reread::read`]，读好后交给
    /// [`Document::apply_reread`]。
    Reread(Reread),
}

/// 一次要在后台执行的读取：文件路径、编码、发出时文档的版本号，以及试读追加部分时使用的内容
/// 副本。
#[derive(Debug)]
pub struct Reread {
    path: PathBuf,
    allow_binary: bool,
    encoding: Encoding,
    revision: u64,
    append: Option<append::Append>,
}

impl Reread {
    /// 读取文件，在后台线程上调用；文件只在末尾追加了内容时只读取追加的部分，否则整个重读。
    ///
    /// 耗时与文件大小成正比，发出读取时已备好追加部分的内容副本。
    pub fn read(self) -> DiskRead {
        let Reread {
            path,
            allow_binary,
            encoding,
            revision,
            append,
        } = self;
        let outcome = match append.and_then(|a| a.read(&path)) {
            Some(appended) => Outcome::Appended(appended),
            None => Outcome::Loaded(load_with(&path, allow_binary, encoding)),
        };
        DiskRead {
            path,
            revision,
            outcome,
        }
    }
}

/// 后台读好的文件内容，交给 [`Document::apply_reread`]。
pub struct DiskRead {
    path: PathBuf,
    revision: u64,
    outcome: Outcome,
}

/// 按用户选择的编码重新读取文件的任务：在后台 [`EncodingReread::read`]，读好后交给
/// [`Document::finish_reopen_with_encoding`]。
#[derive(Debug)]
pub struct EncodingReread {
    path: PathBuf,
    encoding: Encoding,
    revision: u64,
}

impl EncodingReread {
    /// 读取文件并按指定编码解码（耗时与文件大小成正比，在后台线程上调用）。
    pub fn read(self) -> EncodingRead {
        EncodingRead {
            loaded: load_as(&self.path, self.encoding),
            path: self.path,
            encoding: self.encoding,
            revision: self.revision,
        }
    }
}

/// 按用户选择的编码读好的文件内容，见 [`EncodingReread`]。
pub struct EncodingRead {
    path: PathBuf,
    encoding: Encoding,
    revision: u64,
    loaded: Result<Loaded, OpenError>,
}

impl EncodingRead {
    /// 用户选择的编码。
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }
}

/// 「使用磁盘版本」要在后台执行的读取和比较，见 [`Document::begin_take_disk_version`]。
pub struct DiskTake {
    path: PathBuf,
    /// 发起时文档的编码：非 UTF-8 的文件按该编码重新解码。
    encoding: Encoding,
    /// 发起时文档的内容；可以逐行比较、按一次编辑换上新内容时为 `Some`。
    current: Option<Rope>,
    revision: u64,
}

impl DiskTake {
    /// 读取文件，可以逐行比较时算出改动（耗时与文件大小成正比，在后台线程上调用）。
    pub fn read(self) -> DiskTaken {
        let loaded = load_with(&self.path, self.encoding == Encoding::Binary, self.encoding);
        let changes = match (&loaded, &self.current) {
            (Ok(fresh), Some(current))
                if !matches!(fresh.encoding, Encoding::Lossy | Encoding::Binary) =>
            {
                match &fresh.text {
                    Content::Rope(rope) => {
                        let (old, new) = (current.to_string(), rope.to_string());
                        Some(if old == new {
                            Vec::new()
                        } else {
                            diff::line_changes(&old, &new)
                        })
                    }
                    Content::Large(_) => None,
                }
            }
            _ => None,
        };
        DiskTaken {
            path: self.path,
            revision: self.revision,
            loaded,
            changes,
        }
    }
}

/// 后台读好的磁盘版本，交给 [`Document::finish_take_disk_version`]。
pub struct DiskTaken {
    path: PathBuf,
    revision: u64,
    loaded: Result<Loaded, OpenError>,
    /// 从发起时的内容到磁盘内容的逐行改动；无法逐行比较时为 `None`，整个重新读取。
    changes: Option<Vec<winc_core::Change>>,
}

/// 后台读好的一个文件，见 [`Document::read_any`]。
pub struct Opened {
    config: FileConfig,
    content: OpenedContent,
}

/// 打开时读到了什么。
enum OpenedContent {
    /// 图片：只有修改时间，解码另外做。
    Image(Option<SystemTime>),
    /// xlsx 工作簿：只有修改时间，读取另外做。
    Workbook(Option<SystemTime>),
    /// docx 文字文档：只有修改时间，读取和转换另外做。
    Docx(Option<SystemTime>),
    /// PDF：只有修改时间，读取和渲染另外做。
    Pdf(Option<SystemTime>),
    /// 文本，或者打不开的原因。
    Text(Result<Loaded, OpenError>),
}

/// 后台读到了什么。
enum Outcome {
    /// 整个重读的内容。
    Loaded(Result<Loaded, OpenError>),
    /// 只读了末尾追加的部分。
    Appended(append::Appended),
}

impl DiskRead {
    /// 读的时候文档还和 `doc` 一样：版本、路径都没变，也没有未保存的修改。
    pub fn applies_to(&self, doc: &Document) -> bool {
        doc.revision == self.revision
            && doc.path.as_deref() == Some(self.path.as_path())
            && !doc.is_dirty()
    }
}

impl Document {
    /// 把文档写回磁盘文件，成功后标记为已保存并清除冲突标记。
    ///
    /// 先写同目录下的临时文件，成功后再改名替换目标文件，不会留下写了一半的文件。
    ///
    /// # Errors
    ///
    /// 文档没有路径或只读、文本含当前编码无法表示的字符、大文件的原文件在打开后被其他程序
    /// 修改，或写入失败时返回 `std::io::Error`。
    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                t!("editor-file-no-path"),
            ));
        };

        if let Some(reason) = self.readonly_reason() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
        }

        // 符号链接写入其指向的文件，不替换链接本身。
        let target = winc_config::files::canonicalize(path).unwrap_or_else(|_| path.clone());
        let bom = self.encoding == Encoding::Utf8Bom;

        match &self.text {
            // 非 UTF-8 编码（GBK、UTF-16 等）按原编码写回；遇到该编码无法表示的字符时不写入，
            // 并在错误信息中指出该字符。
            Content::Rope(rope) if !matches!(self.encoding, Encoding::Utf8 | Encoding::Utf8Bom) => {
                let bytes = charset::encode(&rope.to_string(), self.encoding).map_err(|c| {
                    let (char, encoding) = (c.to_string(), self.encoding.label());
                    let message = t!(
                        "editor-encoding-unencodable",
                        char = char,
                        encoding = encoding
                    );
                    io::Error::other(message)
                })?;
                write_atomic(&target, false, true, |out| out.write_all(&bytes))?;
            }
            Content::Rope(rope) => write_atomic(&target, bom, true, |out| {
                rope.chunks()
                    .try_for_each(|chunk| out.write_all(chunk.as_bytes()))
            })?,
            Content::Large(large) => {
                // 改动表中的片段引用原文件的字节范围；原文件被其他程序原地修改后，按这些范围
                // 写出的内容是错误的。
                if large.original_changed() {
                    return Err(io::Error::other(t!("editor-original-changed")));
                }

                // 片段引用原文件的字节范围，原地写入会在读取这些片段之前截断原文件。
                write_atomic(&target, bom, false, |out| large.write_to(out))?;
                // 继续引用被替换掉的原文件（打开着的句柄仍指向原来的内容），与新文件内容相同：
                // 重新打开要按文件大小重建行索引，同步保存在界面线程上执行（另存为、关闭前保存），
                // 不在这里做。原文件占的磁盘空间在文档关闭或重新载入时释放。
            }
        }

        self.history.mark_saved();
        let metadata = fs::metadata(&target).ok();
        self.disk_mtime = metadata.as_ref().and_then(|m| m.modified().ok());
        self.disk_len = metadata.map_or(0, |m| m.len());
        self.conflict = false;
        Ok(())
    }

    /// 另存为：换成新路径（语言按新文件名重新识别）并保存。
    ///
    /// 保存失败时文档仍对应原来的路径（语言、`.editorconfig` 也恢复），不会指向一个没有写成的文件。
    ///
    /// # Errors
    ///
    /// 与 [`Document::save`] 相同。
    pub fn save_as(&mut self, path: PathBuf) -> io::Result<()> {
        let before = (
            self.path.clone(),
            self.highlighter.clone(),
            self.editorconfig.clone(),
        );
        self.set_path(path);
        let result = self.save();
        if result.is_err() {
            (self.path, self.highlighter, self.editorconfig) = before;
            self.revision += 1;
            self.columns = None;
        }
        result
    }

    /// 是否能不询问用户直接保存（自动保存）：磁盘文件自上次读写后没有被其他程序修改或删除，
    /// 且文件不是只读的。
    ///
    /// 磁盘文件已被修改即为冲突，需要用户决定保留哪一边；已被删除的文件不会自动写回
    /// （[`Document::deleted_on_disk`]），用户手动保存时由 [`Document::save`] 提示。
    pub fn can_save_quietly(&self) -> bool {
        if self.conflict || self.deleted_on_disk() {
            return false;
        }
        let Some(path) = &self.path else {
            return false;
        };
        fs::metadata(path).map_or_else(
            |_| crate::wsl::reachable(path),
            |metadata| !metadata.permissions().readonly() && !self.disk_differs(&metadata),
        )
    }

    /// 文件在最近一次读写时还在，现在磁盘上没有了（被其他程序删除、移动、重命名，或切换分支）。
    ///
    /// 这时自动保存不写（[`Document::can_save_quietly`]）：自动保存写回后，删掉的文件重新出现在
    /// 磁盘上，用户无从得知原因。用户自己按保存时仍然新建。从来没在磁盘上的（新建还没保存的、
    /// 另存为的目标）不算。
    pub fn deleted_on_disk(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        self.disk_mtime.is_some()
            && fs::metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
            && crate::wsl::reachable(path)
    }

    /// 磁盘文件是否与最近一次读写时不同：修改时间或大小变了。
    ///
    /// 仅比较修改时间不足：有的文件系统时间只精确到 1～2 秒（FAT、exFAT、HFS+、一些网络文件
    /// 系统），保存后同一秒内别的程序再写一次，修改时间可能不变，大小通常会变。最近一次读写时
    /// 文件还不存在（`disk_mtime` 为 `None`）而现在有了，也算不同。
    fn disk_differs(&self, metadata: &fs::Metadata) -> bool {
        metadata.modified().ok() != self.disk_mtime || metadata.len() != self.disk_len
    }

    /// 磁盘文件是否已被其他程序修改，而文档也有未保存的修改，用户还没有决定保留哪一边。
    pub fn in_conflict(&self) -> bool {
        self.conflict
    }

    /// 是否在保存时会覆盖其他程序对磁盘文件所做的修改：已经标记为冲突，或者磁盘文件自上次读写后
    /// 又被修改了（文件监视还没来得及通知）。
    ///
    /// 后一种情况同时标记为冲突。
    ///
    /// 磁盘上已没有该文件、没有路径时返回 `false`：保存只会新建文件。
    pub fn check_overwrite(&mut self) -> bool {
        if self.conflict {
            return true;
        }
        let Some(path) = &self.path else {
            return false;
        };
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        if self.disk_differs(&metadata) {
            self.disk_mtime = metadata.modified().ok();
            self.disk_len = metadata.len();
            self.conflict = true;
        }
        self.conflict
    }

    /// 读出磁盘上的当前内容（按文档的编码解码，换行统一为 `\n`），用于和未保存的内容比较。
    ///
    /// # Errors
    ///
    /// 没有路径、读取失败、文件变成二进制、超过大小上限，或是按需读取的大文件时返回 [`OpenError`]。
    pub fn disk_text(&self) -> Result<String, OpenError> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, t!("editor-file-no-path")))?;
        let too_large = || io::Error::other(t!("editor-large-cannot-compare")).into();
        // 大文件读入时要先把整个文件扫一遍建索引，无法逐行比较，因此先按大小排除。
        if fs::metadata(path)?.len() > LARGE_FILE {
            return Err(too_large());
        }
        match load_with(path, false, self.encoding)?.text {
            Content::Rope(rope) => Ok(rope.to_string()),
            Content::Large(_) => Err(too_large()),
        }
    }

    /// 解决冲突：换成磁盘上的内容，放弃未保存的修改。
    ///
    /// 在调用的线程上读取和比较；界面线程上用 [`Document::begin_take_disk_version`]，交给后台。
    /// 返回值的含义见 [`Document::finish_take_disk_version`]。
    ///
    /// # Errors
    ///
    /// 读取失败时返回 [`OpenError`]，文档不变。
    pub fn take_disk_version(&mut self) -> Result<bool, OpenError> {
        let Some(take) = self.begin_take_disk_version() else {
            return Ok(false);
        };
        Ok(self.finish_take_disk_version(take.read())?.unwrap_or(false))
    }

    /// 准备「使用磁盘版本」：返回要在后台执行的读取和比较，自身不访问磁盘。
    ///
    /// 文档没有路径时返回 `None`。
    pub fn begin_take_disk_version(&self) -> Option<DiskTake> {
        let path = self.path.clone()?;
        let current = match &self.text {
            Content::Rope(rope) if self.text_editable() => Some(rope.clone()),
            _ => None,
        };
        Some(DiskTake {
            path,
            encoding: self.encoding,
            current,
            revision: self.revision,
        })
    }

    /// 换上后台读好的磁盘版本 `taken`，放弃未保存的修改。
    ///
    /// 换内容算一次修改，⌘Z 可以回到放弃之前的内容，返回 `Some(true)`；大文件、只读文档或内容有
    /// 无法解码的字节时整个重新读取，撤销记录清空并返回 `Some(false)`；读取期间文档被编辑过或换了
    /// 路径时不换，返回 `None`。
    ///
    /// # Errors
    ///
    /// 读取失败时返回 [`OpenError`]，文档不变。
    pub fn finish_take_disk_version(
        &mut self,
        taken: DiskTaken,
    ) -> Result<Option<bool>, OpenError> {
        if self.revision != taken.revision || self.path.as_deref() != Some(taken.path.as_path()) {
            return Ok(None);
        }
        let loaded = taken.loaded?;
        let Some(changes) = taken.changes.filter(|_| self.text_editable()) else {
            self.replace_content(loaded);
            return Ok(Some(false));
        };

        if !changes.is_empty() {
            let before = self.selection;
            let mapped = diff::map_positions(&changes, &[before.anchor, before.head]);
            let after = Selection {
                anchor: mapped[0],
                head: mapped[1],
            };
            // 预览中的 SVG 暂时只读，`apply` 会忽略修改：换内容时临时放开。
            let readonly = std::mem::replace(&mut self.readonly, false);
            self.apply(Transaction::new(changes, before, after, EditKind::Other));
            self.readonly = readonly;
            self.selection = after.clamped(self.text.end_pos());
        }
        self.encoding = loaded.encoding;
        self.line_ending = loaded.line_ending;
        self.disk_mtime = loaded.mtime;
        self.disk_len = loaded.len;
        self.history.mark_saved();
        self.conflict = false;
        Ok(Some(true))
    }

    /// 解决冲突：保留未保存的修改，下次保存时覆盖磁盘上的内容（之后自动保存也仍然进行）。
    pub fn keep_mine(&mut self) {
        if let Some(path) = &self.path
            && let Ok(metadata) = fs::metadata(path)
        {
            self.disk_mtime = metadata.modified().ok();
            self.disk_len = metadata.len();
        }
        self.conflict = false;
    }

    /// 检查磁盘文件是否被其他程序修改；文档没有未保存的修改时直接重新加载。
    ///
    /// 整个重读就在调用的线程上读；界面线程上用 [`Document::check_disk`]，把重读交给后台。
    ///
    /// # Errors
    ///
    /// 重新加载时读取失败、内容变成二进制或超过大小上限时返回 [`OpenError`]。
    pub fn sync_with_disk(&mut self) -> Result<DiskChange, OpenError> {
        match self.check_disk() {
            DiskSync::Done(change) => Ok(change),
            DiskSync::Reread(reread) => self.apply_reread(reread.read()),
        }
    }

    /// 和 [`Document::sync_with_disk`] 一样检查磁盘文件，但不读文件内容：要读时返回
    /// [`DiskSync::Reread`]，调用方在后台 [`Reread::read`]，读好后交给
    /// [`Document::apply_reread`]。
    ///
    /// 其余情况（没变、冲突、图片、xlsx 工作簿、docx、PDF）只看文件的元数据，就地处理完。元数据在
    /// 调用的线程上读；界面线程上在后台读好元数据，再调用 [`Document::check_disk_with`]。
    pub fn check_disk(&mut self) -> DiskSync {
        let metadata = self.path.as_ref().and_then(|p| fs::metadata(p).ok());
        self.check_disk_with(metadata.as_ref())
    }

    /// 和 [`Document::check_disk`] 一样，但用调用方读好的文件元数据 `metadata`（文件不存在或
    /// 无法访问时为 `None`），自身不访问磁盘。
    pub fn check_disk_with(&mut self, metadata: Option<&fs::Metadata>) -> DiskSync {
        let Some(path) = self.path.clone() else {
            return DiskSync::Done(DiskChange::Unchanged);
        };

        // 图片、工作簿、docx、PDF 被其他程序修改：重新读取。递增 `revision` 使正在进行的那次
        // 读取结果被丢弃（见 `set_picture`、`set_table`），工作簿随后按新版本重新读
        // （`table_to_load`），docx、PDF 由界面按新版本重新转换、渲染。
        if matches!(
            self.unsupported,
            Some(Unsupported::Image | Unsupported::Workbook | Unsupported::Docx | Unsupported::Pdf)
        ) {
            let mtime = metadata.and_then(|m| m.modified().ok());
            if mtime.is_none() || mtime == self.disk_mtime {
                return DiskSync::Done(DiskChange::Unchanged);
            }
            self.disk_mtime = mtime;
            self.revision += 1;
            if self.unsupported == Some(Unsupported::Image) {
                self.picture = Some(PictureState::Loading);
            }
            return DiskSync::Done(DiskChange::Reloaded);
        }
        // 无法打开的文件在用户点「重试」后才读取；尚未读取的文档在切换到对应标签时才读取。
        if self.unsupported.is_some() || self.pending {
            return DiskSync::Done(DiskChange::Unchanged);
        }

        // 文件被删除或暂时无法访问时保留文档，不当作修改。
        let Some(metadata) = metadata else {
            return DiskSync::Done(DiskChange::Unchanged);
        };

        if !self.disk_differs(metadata) {
            return DiskSync::Done(DiskChange::Unchanged);
        }

        if self.is_dirty() {
            // 记录新的修改时间和大小，使同一次外部修改只提示一次。
            self.disk_mtime = metadata.modified().ok();
            self.disk_len = metadata.len();
            self.conflict = true;
            return DiskSync::Done(DiskChange::Conflict);
        }

        DiskSync::Reread(Reread {
            path,
            allow_binary: self.encoding == Encoding::Binary,
            encoding: self.encoding,
            revision: self.revision,
            append: self.append_source(),
        })
    }

    /// 换上后台读好的内容 `reread`（见 [`Document::check_disk`]）：整个重读的返回
    /// [`DiskChange::Reloaded`]，只读末尾追加部分的接上后返回 [`DiskChange::Appended`]。
    ///
    /// 读取期间文档变了（编辑过、换了路径）时读好的内容不能用，返回 [`DiskChange::Unchanged`]。
    ///
    /// # Errors
    ///
    /// 重读失败（读取出错、内容变成二进制或超过大小上限）时返回 [`OpenError`]，文档不变。
    pub fn apply_reread(&mut self, reread: DiskRead) -> Result<DiskChange, OpenError> {
        if !reread.applies_to(self) {
            return Ok(DiskChange::Unchanged);
        }
        match reread.outcome {
            Outcome::Loaded(loaded) => {
                self.replace_content(loaded?);
                Ok(DiskChange::Reloaded)
            }
            Outcome::Appended(appended) => {
                self.apply_append(appended);
                Ok(DiskChange::Appended)
            }
        }
    }

    /// 换成新读入的内容 `loaded`：清空撤销历史，光标尽量保持在原来的行和列。
    fn replace_content(&mut self, loaded: Loaded) {
        let (line, col) = self.text.pos_to_line_col(self.selection.head);
        self.set_content(loaded);
        self.history.clear();
        self.selection = Selection::point(self.text.line_col_to_pos(line, col));
        self.goal_cell = None;
        self.after_change(0);
    }

    /// 按 `encoding` 重新读取磁盘文件（丢弃未保存的修改），光标尽量保持在原来的行和列。
    ///
    /// 存在无法解码的字节时文档转为只读：坏字节显示为 U+FFFD，保存会改写原文件。在调用的线程上
    /// 读取；界面线程上用 [`Document::begin_reopen_with_encoding`]，把读取交给后台。
    ///
    /// # Errors
    ///
    /// 文档没有路径、是大文件或二进制文件，或读取失败时返回原因（显示给用户的文字）。
    pub fn reopen_with_encoding(&mut self, encoding: Encoding) -> Result<(), String> {
        let read = self.begin_reopen_with_encoding(encoding)?.read();
        self.finish_reopen_with_encoding(read).map(|_| ())
    }

    /// 准备按 `encoding` 重新读取磁盘文件，返回要在后台执行的读取任务，本方法自身不访问磁盘。
    ///
    /// # Errors
    ///
    /// 文档没有路径、是大文件或二进制文件时返回原因（显示给用户的文字）。
    pub fn begin_reopen_with_encoding(&self, encoding: Encoding) -> Result<EncodingReread, String> {
        let Some(path) = &self.path else {
            return Err(t!("editor-file-no-path").to_owned());
        };
        if self.is_large() {
            return Err(t!("editor-large-utf8-only").to_owned());
        }
        if self.encoding == Encoding::Binary {
            return Err(t!("editor-file-is-binary").to_owned());
        }
        Ok(EncodingReread {
            path: path.clone(),
            encoding,
            revision: self.revision,
        })
    }

    /// 换上按用户选择的编码读好的内容 `read`（丢弃未保存的修改），返回是否换上。
    ///
    /// 读取期间文档被编辑过或换了路径时不换，返回 `Ok(false)`。
    ///
    /// # Errors
    ///
    /// 读取失败时返回原因（显示给用户的文字），文档不变。
    pub fn finish_reopen_with_encoding(&mut self, read: EncodingRead) -> Result<bool, String> {
        if self.revision != read.revision || self.path.as_deref() != Some(read.path.as_path()) {
            return Ok(false);
        }
        let loaded = read.loaded.map_err(|e| e.to_string())?;
        self.replace_content(loaded);
        Ok(true)
    }

    /// 设置之后保存使用的编码 `encoding`，返回是否设置成功；保存由调用方随后执行。
    ///
    /// 不能按该编码保存（[`Document::can_save_with`]）时不设置，返回 `false`。
    pub fn set_save_encoding(&mut self, encoding: Encoding) -> bool {
        if !self.can_save_with(encoding) {
            return false;
        }
        self.encoding = encoding;
        true
    }

    /// 能否改用 `encoding` 保存。
    ///
    /// 文档只读或该编码不支持写入时不能。大文件按片段原样写出原文件的字节，
    /// 只能用 UTF-8（带或不带 BOM）。
    pub fn can_save_with(&self, encoding: Encoding) -> bool {
        let utf8 = matches!(encoding, Encoding::Utf8 | Encoding::Utf8Bom);
        !self.readonly && encoding.writable() && (utf8 || !self.is_large())
    }

    /// 文件在磁盘上被重命名或移动（例如在文件树中重命名）后更新路径。
    ///
    /// 按新文件名重新识别语言，内容、撤销记录和修改状态保持不变。
    pub fn moved_to(&mut self, path: PathBuf) {
        self.set_path(path);
    }

    /// 创建尚未读取的文档：只记录路径，文本为空，文档只读，不检查磁盘。
    ///
    /// 恢复现场时当前标签以外的文档先按本方法占位，切换到该标签时再读取（[`crate::Editor::load`]），
    /// 几十个标签的启动不必逐个读文件。占位的文档不会被保存、不随磁盘同步，也不备份。
    pub fn pending(id: DocumentId, path: PathBuf) -> Self {
        let mut document = Document::empty(id);
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            document.set_language(name);
        }
        document.path = Some(path);
        document.readonly = true;
        document.pending = true;
        document
    }

    /// 是否是尚未读取的文档（见 [`Document::pending`]）。
    ///
    /// 这样的文档文本为空，不代表文件的内容：要文件内容时读磁盘，或先 [`crate::Editor::load`]。
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// 创建尚未读取的文档（见 [`Document::pending`]），读取时把二进制内容也按文本读入，只读。
    pub(crate) fn pending_as_text(id: DocumentId, path: PathBuf) -> Self {
        let mut document = Self::pending(id, path);
        document.pending_as_text = true;
        document
    }

    /// 尚未读取的文档在读取时是否把二进制内容也按文本读入（见 [`Document::pending_as_text`]）。
    pub(crate) fn reads_as_text(&self) -> bool {
        self.pending_as_text
    }

    /// 设置文档对应的路径 `path`，按新文件名重新识别语言，并从磁盘读取适用的 `.editorconfig`。
    pub(super) fn set_path(&mut self, path: PathBuf) {
        let config = editorconfig::lookup(&path);
        self.set_path_with(path, config);
    }

    /// 和 [`Document::set_path`] 一样，但用已经读好的 `.editorconfig` 设置 `config`，不访问磁盘。
    fn set_path_with(&mut self, path: PathBuf, config: FileConfig) {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.set_language(name);
        }
        self.editorconfig = config;
        self.path = Some(path);
    }

    /// 打开文件：图片作为预览打开，其余文件作为文本打开。
    ///
    /// 图片的解码由界面交给后台线程，见 [`PictureState::Loading`]。无法打开的文件也返回文档，
    /// 由 `unsupported` 字段说明原因。就在调用的线程上读；界面线程上在后台
    /// [`Document::read_any`]，再 [`Document::from_opened`]。
    pub fn open_any(id: DocumentId, path: PathBuf) -> Self {
        let opened = Document::read_any(&path);
        Document::from_opened(id, path, opened)
    }

    /// 读文件 `path`，供 [`Document::from_opened`] 建文档（耗时和文件大小成正比，在后台线程上
    /// 调用）：图片、xlsx 工作簿、docx 和 PDF 只读修改时间（解码、读取另由界面交给后台），其余按
    /// 文本读；同时读适用的 `.editorconfig`。
    pub fn read_any(path: &Path) -> Opened {
        Self::read(path, false)
    }

    /// 与 [`Document::read_any`] 相同；`as_text` 为 `true` 时图片和二进制文件也按文本读入
    /// （「以文本方式打开」）。
    pub(crate) fn read(path: &Path, as_text: bool) -> Opened {
        let mtime = || fs::metadata(path).and_then(|m| m.modified()).ok();
        let content = if !as_text && winc_preview::image::is_image(path) {
            OpenedContent::Image(mtime())
        } else if !as_text && winc_preview::table::is_workbook(path) {
            OpenedContent::Workbook(mtime())
        } else if !as_text && winc_preview::docx::is_docx(path) {
            OpenedContent::Docx(mtime())
        } else if !as_text && winc_preview::pdf::supported() && winc_preview::pdf::is_pdf(path) {
            OpenedContent::Pdf(mtime())
        } else {
            OpenedContent::Text(load(path, as_text))
        };
        Opened {
            config: editorconfig::lookup(path),
            content,
        }
    }

    /// 用后台读好的 `opened`（[`Document::read_any`]）建文档，自身不访问磁盘；结果和
    /// [`Document::open_any`] 相同。
    pub fn from_opened(id: DocumentId, path: PathBuf, opened: Opened) -> Self {
        let mut document = Document::empty(id);
        document.set_path_with(path, opened.config);
        match opened.content {
            OpenedContent::Image(mtime) => {
                document.readonly = true;
                document.unsupported = Some(Unsupported::Image);
                document.picture = Some(PictureState::Loading);
                document.disk_mtime = mtime;
            }
            OpenedContent::Workbook(mtime) => {
                document.readonly = true;
                document.unsupported = Some(Unsupported::Workbook);
                document.table = Some(TableView::loading(document.revision));
                document.disk_mtime = mtime;
            }
            OpenedContent::Docx(mtime) => {
                document.readonly = true;
                document.unsupported = Some(Unsupported::Docx);
                document.disk_mtime = mtime;
            }
            OpenedContent::Pdf(mtime) => {
                document.readonly = true;
                document.unsupported = Some(Unsupported::Pdf);
                document.disk_mtime = mtime;
            }
            OpenedContent::Text(Ok(loaded)) => document.set_content(loaded),
            OpenedContent::Text(Err(e)) => {
                document.readonly = true;
                document.unsupported = Some(e.into());
            }
        }
        document
    }

    /// 换上读好的内容 `loaded`：按编码决定是否只读，更新编码、换行风格、修改时间和缩进检测。
    pub(super) fn set_content(&mut self, loaded: Loaded) {
        let overlong = matches!(&loaded.text, Content::Large(large) if large.has_overlong_lines());
        self.readonly = matches!(loaded.encoding, Encoding::Lossy | Encoding::Binary) || overlong;
        // 正在预览时仍保持只读；切回源代码时恢复为新读取内容对应的 `readonly`。
        if self.previewing() {
            self.readonly_before_preview = self.readonly;
            self.readonly = true;
        }
        self.text = loaded.text;
        self.whole.forget();
        self.clear_folds();
        self.encoding = loaded.encoding;
        self.line_ending = loaded.line_ending;
        self.disk_mtime = loaded.mtime;
        self.disk_len = loaded.len;
        self.conflict = false;
        self.detected_indent = indent::detect(&self.text);
        // 尚不含换行符的文件（例如新建的空文件）使用 `.editorconfig` 指定的换行风格。
        if let Some(eol) = self.editorconfig.end_of_line
            && self.text.line_count() <= 1
        {
            self.line_ending = eol;
        }
    }
}

/// 把整个文件按打开时的规则解码成 rope。用于超过 `LARGE_FILE`、大文件读取器无法表示的编码
/// （带 BOM 的 UTF-16）。
fn load_decoded(file: &mut fs::File, mtime: Option<SystemTime>) -> Result<Loaded, OpenError> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let decoded = charset::decode(&bytes);
    let rope = Rope::from_str(&decoded.text);
    Ok(Loaded {
        line_ending: LineEnding::detect(&rope),
        text: Content::Rope(rope),
        encoding: decoded.encoding,
        mtime,
        len: bytes.len() as u64,
    })
}

/// 读入文件并按 `encoding` 解码（「通过编码重新打开」）。
///
/// 无法解码的字节替换为 U+FFFD，编码记为 `Lossy`；文件超过 `UTF16_INLINE` 时返回错误。修改时间
/// 和大小取自同一个已打开的文件、在读取内容之前获取：读取期间文件被其他程序修改时，记录的
/// 修改时间早于实际内容，之后的磁盘同步仍能发现这次修改。
fn load_as(path: &Path, encoding: Encoding) -> Result<Loaded, OpenError> {
    let file = fs::File::open(path)?;
    let mtime = file.metadata()?.modified().ok();
    // 文档打开时可能不超过 `LARGE_FILE`，之后文件变大：超过 `UTF16_INLINE` 时不再整篇解码。
    // 读的过程中还在变大的，读到上限为止再判断。
    let mut bytes = Vec::new();
    file.take(UTF16_INLINE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > UTF16_INLINE {
        return Err(OpenError::Io(io::Error::other(t!(
            "editor-large-utf8-only"
        ))));
    }
    let (text, damaged) = charset::decode_as(&bytes, encoding);
    let rope = Rope::from_str(&text);
    Ok(Loaded {
        line_ending: LineEnding::detect(&rope),
        text: Content::Rope(rope),
        encoding: if damaged { Encoding::Lossy } else { encoding },
        mtime,
        len: bytes.len() as u64,
    })
}

/// 读入文件；开头 `BINARY_SNIFF` 字节内含 NUL 时视为二进制文件，`allow_binary` 为 `true` 时
/// 仍然读入，编码记为 `Binary`。
pub(super) fn load(path: &Path, allow_binary: bool) -> Result<Loaded, OpenError> {
    load_with(path, allow_binary, Encoding::Utf8)
}

/// 与 [`load`] 相同，但非 UTF-8 的文件优先按 `prefer`（用户通过「通过编码重新打开」选择的编码）
/// 解码。
///
/// 按文档当前的编码重新读取时传入它，避免自动猜测的编码一旦错误，每次重读都得到乱码。
fn load_with(path: &Path, allow_binary: bool, prefer: Encoding) -> Result<Loaded, OpenError> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    let mtime = metadata.modified().ok();

    if size > OPEN_LIMIT {
        return Err(OpenError::TooLarge(size));
    }

    let mut head = Vec::with_capacity(BINARY_SNIFF);
    (&mut file)
        .take(BINARY_SNIFF as u64)
        .read_to_end(&mut head)?;

    // 带 BOM 的 UTF-16 含 NUL，但不是二进制。大文件读取器只认 UTF-8，因此在
    // `UTF16_INLINE` 以内整篇解码；再大的无法按 UTF-16 打开。
    let utf16 = utf16_bom(&head);
    if utf16 && size > UTF16_INLINE {
        return Err(OpenError::Io(io::Error::other(t!(
            "editor-large-utf8-only"
        ))));
    }
    let binary = looks_binary(&head);

    if binary && !allow_binary {
        return Err(OpenError::Binary(size));
    }

    if size > LARGE_FILE && utf16 {
        file.rewind()?;
        return load_decoded(&mut file, mtime);
    }

    if size > LARGE_FILE {
        let large = LargeText::open(file)?;
        let encoding = match (large.is_utf8(), large.has_bom()) {
            _ if binary => Encoding::Binary,
            (false, _) => Encoding::Lossy,
            (true, true) => Encoding::Utf8Bom,
            (true, false) => Encoding::Utf8,
        };
        let line_ending = if large.is_crlf() {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        return Ok(Loaded {
            text: Content::Large(Box::new(large)),
            encoding,
            line_ending,
            mtime,
            len: size,
        });
    }

    file.rewind()?;

    // 合法的 UTF-8 边读边建 rope，内存中不会同时存在整个文件的另一份副本；BOM 的处理与
    // `decode` 相同。只有非 UTF-8（GBK、UTF-16 等）才把整个文件读入内存，交给 `decode` 识别编码。
    let (rope, encoding) = match Rope::from_reader(BufReader::with_capacity(64 << 10, &file)) {
        Ok(mut rope) => {
            let bom = head.starts_with(&UTF8_BOM);
            if bom {
                rope.remove(..1);
            }
            let encoding = if bom {
                Encoding::Utf8Bom
            } else {
                Encoding::Utf8
            };
            (rope, encoding)
        }
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            file.rewind()?;
            let mut bytes = Vec::with_capacity(size as usize);
            file.read_to_end(&mut bytes)?;
            // 二进制文件以文本方式打开：不检测编码，非法字节显示为 U+FFFD。
            if binary {
                (
                    Rope::from_str(&String::from_utf8_lossy(&bytes)),
                    Encoding::Binary,
                )
            } else if let Encoding::Legacy(_) = prefer {
                // 用户指定了编码：按该编码解码，无法解码的字节替换为 U+FFFD（文档只读）。
                let (text, damaged) = charset::decode_as(&bytes, prefer);
                (
                    Rope::from_str(&text),
                    if damaged { Encoding::Lossy } else { prefer },
                )
            } else {
                let decoded = charset::decode(&bytes);
                (Rope::from_str(&decoded.text), decoded.encoding)
            }
        }
        Err(e) => return Err(e.into()),
    };
    let encoding = if binary { Encoding::Binary } else { encoding };

    let line_ending = LineEnding::detect(&rope);
    Ok(Loaded {
        text: Content::Rope(rope),
        encoding,
        line_ending,
        mtime,
        len: size,
    })
}

/// 先写同目录下的临时文件，成功后再改名替换目标文件；失败时删除临时文件。
///
/// `bom` 为 `true` 时先写入 UTF-8 BOM，`write` 负责写出其后的内容；`in_memory` 为 `true` 时
/// 写出的内容全部在内存中，改名替换会改变文件属性的情况改为原地写入（[`write_in_place`]）。
///
/// # Errors
///
/// 目标文件只读、改名替换会改变文件身份，或写入失败时返回 `std::io::Error`。
pub(super) fn write_atomic(
    path: &Path,
    bom: bool,
    in_memory: bool,
    write: impl FnOnce(&mut io::BufWriter<fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or(Cow::Borrowed("file"), |n| n.to_string_lossy());

    // 文件所在的目录可能已被删除（例如在文件树中删除了整个目录，而该文件的标签仍保留）：
    // 重新创建后再保存。
    fs::create_dir_all(dir)?;

    let existing = fs::metadata(path).ok();
    // `rename` 只检查目录的写权限，不检查目标文件的权限；目标文件只读时必须在此拒绝，
    // 否则该文件会被替换。
    if existing
        .as_ref()
        .is_some_and(|m| m.permissions().readonly())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            t!("editor-file-read-only"),
        ));
    }
    // 通过 Windows 改名替换发行版中的文件会丢失 Linux 权限和属主。
    let in_distro = existing.is_some() && crate::wsl::locate_here(path).is_some();
    if in_memory && in_distro {
        return write_in_place(path, bom, write);
    }
    if existing.as_ref().is_some_and(rename_changes_identity) {
        if in_memory {
            return write_in_place(path, bom, write);
        }
        return Err(io::Error::other(t!("editor-large-identity")));
    }
    // 临时文件名带随机后缀且只新建、不跟随符号链接（`create_new`）：目录其他用户也能写时，
    // 对方预先放一个同名的符号链接就能让保存写进链接指向的文件。
    let (tmp, file) = match create_temporary(dir, &name, existing.is_some()) {
        Ok(created) => created,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(io::Error::other(t!("editor-temp-file-failed")));
        }
        Err(e)
            if in_memory && existing.is_some() && e.kind() == io::ErrorKind::PermissionDenied =>
        {
            // 目录不可写而文件可写：无法创建临时文件，改为原地写入。
            return write_in_place(path, bom, write);
        }
        Err(e) => return Err(e),
    };

    let result = (|| {
        let mut out = io::BufWriter::with_capacity(1 << 20, file);

        if bom {
            out.write_all(&UTF8_BOM)?;
        }

        write(&mut out)?;
        let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
        sync_file(&file)?;

        if let Some(metadata) = &existing {
            // 临时文件新建时只给自己读写，写完再换成原文件的权限，写的过程中其他用户读不到内容。
            file.set_permissions(metadata.permissions())?;
            // 新文件的属组是当前用户的默认组（macOS 上是目录的属组），改回原文件的属组；当前用户
            // 不在该组里时系统拒绝，沿用新文件的属组。
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let _ = std::os::unix::fs::fchown(&file, None, Some(metadata.gid()));
            }
            #[cfg(target_vendor = "apple")]
            copy_extended_attributes(path, &file);
        }

        // 大文件的片段引用原文件，无法原地写入，由发行版复制权限。
        if in_distro {
            crate::wsl::keep_identity(path, &tmp)?;
        }
        winc_config::files::replace_file(&tmp, path)?;
        sync_dir(dir);
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }

    result
}

/// 改名替换是否会改变已有文件 `metadata` 的身份：有多个硬链接（改名只替换一个名字，其他名字
/// 仍指向旧内容），或属主不是当前用户（改名后属主变为当前用户）。
#[cfg(unix)]
// 本函数只有对 `libc::geteuid` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn rename_changes_identity(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: `geteuid` 没有参数，不访问内存，总是成功。
    let user = unsafe { libc::geteuid() };
    metadata.nlink() > 1 || metadata.uid() != user
}

/// 改名替换是否会改变已有文件 `metadata` 的身份；非 Unix 平台不检查。
#[cfg(not(unix))]
fn rename_changes_identity(_metadata: &fs::Metadata) -> bool {
    false
}

/// 原地写入 `path`：截断后写入新内容，再同步到磁盘。
///
/// 不经过临时文件：写入中途失败（磁盘已满、进程中止）时文件只剩部分内容，因此只在改名替换不可行
/// 或会改变文件属性时使用，见 [`write_atomic`]。
fn write_in_place(
    path: &Path,
    bom: bool,
    write: impl FnOnce(&mut io::BufWriter<fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    let mut out = io::BufWriter::with_capacity(1 << 20, file);
    if bom {
        out.write_all(&UTF8_BOM)?;
    }
    write(&mut out)?;
    let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    sync_file(&file)
}

/// 把 `source` 的扩展属性（包括彩色标签）和 ACL 复制到 `target`。
///
/// 尽力而为：读不到原文件或文件系统不支持时不复制，保存本身不受影响。
#[cfg(target_vendor = "apple")]
// 本函数只有对 `libc::fcopyfile` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn copy_extended_attributes(source: &Path, target: &fs::File) {
    use std::os::fd::AsRawFd;
    let Ok(source) = fs::File::open(source) else {
        return;
    };
    // SAFETY: 两个参数都是调用期间被借用、保持打开的有效文件描述符；状态参数传空指针，
    // `fcopyfile` 允许这样做且不会写入；失败时只返回 -1。
    unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            target.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        );
    }
}
