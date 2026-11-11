//! 无界面编辑器：管理打开的文档与工作区，并提供编辑命令、查找、格式化、Git 操作和跳转。
//!
//! 核心类型是 [`Editor`]（打开的 [`Document`] 与可选的 [`Workspace`]），界面层通过它的方法读写
//! 状态，本 crate 不做绘制。所有位置都是 `char` 偏移（从 0 起），行号、列号从 0 起。
//!
//! 模块分工见各模块的 `//!`：`document`、`editing`、`workspace`、`navigation`、`git`、`tools`。

pub mod background;
pub mod backup;
pub mod diff_view;
mod document;
mod editing;
pub mod git;
mod navigation;
mod tools;
mod workspace;

#[cfg(test)]
mod test_util;

pub use document::content::Content;
pub use document::disk::{
    DiskRead, DiskSync, DiskTake, DiskTaken, EncodingRead, EncodingReread, Opened, Reread, SaveJob,
    Saved,
};
pub use document::encoding::Encoding;
pub use document::large::LargeText;
pub use document::{
    COPY_LIMIT, DiskChange, Document, DocumentId, EDIT_LIMIT, LARGE_FILE, OpenError, PictureState,
    PreviewKind, PreviewMode, TableState, TableView, Unsupported, looks_binary,
};
pub use document::{editorconfig, encoding};
pub use editing::command::Command;
pub use editing::cursors::MAX_CURSORS;
pub use editing::folding::{Fold, FoldCommand, Folds, LineWindow, WrapJob, WrapResult};
pub use editing::long_lines::LineSpan;
pub use editing::search::{MAX_MATCHES, Search, TextSearch};
pub use editing::{paste, scopes};
pub use navigation::bookmarks::{BookmarkMap, Bookmarks};
pub use navigation::{bookmarks, definition, keypath, symbols, usages};
pub use tools::{format, shell, wsl};
pub use workspace::picker::{FileIndex, IndexedFile, MAX_RESULTS as PICKER_MAX_RESULTS};
pub use workspace::project_search::{
    FileResult, LineMatch, MAX_RESULTS, MAX_THREADS as MAX_SEARCH_THREADS, ProjectSearch, Query,
    ReplacePreview, replace_in_document, replace_in_file,
};
pub use workspace::tree::{
    DirEntry, DirListing, DirScan, EXPAND_DIRS, Entry, EntryKind, Workspace, copy_entry, list_dir,
    natural_cmp, segment_name,
};
pub use workspace::watch::{Changes, Watcher};

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// 编辑器状态：打开的文档（每个对应一个标签）和工作区。
pub struct Editor {
    /// 已打开的文档，按标签栏中的顺序排列。
    pub documents: Vec<Document>,
    /// 活动标签在 `documents` 中的下标（从 0 起）。
    pub active: Option<usize>,
    /// 打开的工作区（目录）；没有打开目录时为 `None`。
    pub workspace: Option<Workspace>,
    /// 默认的制表符显示宽度（列）。
    ///
    /// 只作用于未设置制表符宽度的文档；文档可经状态栏或 `.editorconfig` 覆盖，见
    /// [`Self::active_tab_width`]。
    pub tab_width: usize,
    /// 新打开的差异是否单栏显示。
    pub diff_unified: bool,
    /// 未打开文件的书签，键为文件的规范路径。
    ///
    /// 打开文件时移入对应文档，关闭标签时从文档取回。无法打开的文件（占位标签）没有文本，
    /// 其书签保留在此处，「重试」打开成功后再移入。
    pub bookmarks: BookmarkMap,
    /// 未打开文件中折叠的块，键为文件的规范路径；打开文件时移入文档，关闭标签时取回。
    pub folds: FoldMap,
    /// 可预览的文件读取完成时采用的显示方式（[`Editor::finish_load`]）；默认为源代码。
    pub preview_default: PreviewMode,
    next_id: u64,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            documents: Vec::new(),
            active: None,
            workspace: None,
            tab_width: 4,
            diff_unified: false,
            bookmarks: BookmarkMap::new(),
            folds: FoldMap::new(),
            preview_default: PreviewMode::Source,
            next_id: 0,
        }
    }
}

impl Editor {
    /// 当前标签对应的文档；没有打开任何标签时返回 `None`。
    pub fn active(&self) -> Option<&Document> {
        self.active.and_then(|i| self.documents.get(i))
    }

    /// 返回当前文档中一个 Tab 字符占用的列数。
    ///
    /// 优先使用文档自身的设置（状态栏选择或 `.editorconfig`），否则使用 [`Self::tab_width`]。
    pub fn active_tab_width(&self) -> usize {
        self.active()
            .map_or(self.tab_width, |doc| doc.tab_width(self.tab_width))
    }

    /// 返回当前标签对应文档的可变借用；没有打开任何标签时返回 `None`。
    pub fn active_mut(&mut self) -> Option<&mut Document> {
        self.active.and_then(|i| self.documents.get_mut(i))
    }

    /// 返回标识为 `id` 的文档所在的标签序号（从 0 起）；没有该文档时返回 `None`。
    pub fn index_of(&self, id: DocumentId) -> Option<usize> {
        self.documents.iter().position(|d| d.id == id)
    }

    /// 按标识查找文档，文档不必是活动标签；没有时返回 `None`。
    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.iter().find(|d| d.id == id)
    }

    /// 按标识查找文档的可变借用，文档不必是活动标签；没有时返回 `None`。
    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|d| d.id == id)
    }

    /// 返回工作区根目录；没有打开目录时返回 `None`。
    pub fn root(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|w| w.root.as_path())
    }

    /// 打开文件并切换到其标签，返回标签序号（从 0 起）。
    ///
    /// 路径规范化后与已打开文档相同时直接切换到已有标签，不重复读取。无法打开的文件（二进制、
    /// 超过大小上限、读取失败）同样占用一个占位标签，编辑区显示原因。读取在调用线程上进行；
    /// 界面线程上改用 [`Editor::open_tab`]，在后台读取。
    pub fn open(&mut self, path: &Path) -> usize {
        let index = self.open_tab(path);
        self.load(index);
        index
    }

    /// 为 `path` 创建标签并切换过去，返回标签序号，不读取文件。
    ///
    /// 新开的标签尚未读取（[`Document::pending`]），由调用方在后台读取（[`Editor::begin_load`]）。
    /// 路径规范化后与已打开文档相同时切换到已有标签。
    pub fn open_tab(&mut self, path: &Path) -> usize {
        let index = self.open_pending(path);
        self.active = Some(index);
        index
    }

    /// 为 `path` 创建一个尚未读取的标签（[`Document::pending`]），不切换过去，返回标签序号。
    ///
    /// 用于恢复会话：切换到该标签时再读取文件。路径规范化后与已打开文档相同时返回已有标签。
    pub fn open_pending(&mut self, path: &Path) -> usize {
        let canonical = canonical_path(path);
        if let Some(index) = self
            .documents
            .iter()
            .position(|d| d.path.as_deref() == Some(canonical.as_path()))
        {
            return index;
        }
        let id = self.allocate_id();
        self.documents.push(Document::pending(id, canonical));
        self.documents.len() - 1
    }

    /// 第 `index` 个标签尚未读取（[`Document::pending`]）时在当前线程读入文件，返回是否已读取。
    ///
    /// 耗时与文件大小成正比；界面线程上改用 [`Editor::begin_load`] 在后台读取。标签已读取或
    /// `index` 越界时返回 `false`。
    pub fn load(&mut self, index: usize) -> bool {
        self.begin_load(index)
            .is_some_and(|job| self.finish_load(job.run()).is_some())
    }

    /// 第 `index` 个标签尚未读取时返回读取该标签的任务：在后台调用 [`LoadJob::run`]，读取结果
    /// 交给 [`Editor::finish_load`]。
    ///
    /// 标签已读取或 `index` 越界时返回 `None`。
    pub fn begin_load(&self, index: usize) -> Option<LoadJob> {
        let doc = self.documents.get(index).filter(|d| d.is_pending())?;
        Some(LoadJob {
            id: doc.id,
            path: doc.path.clone()?,
            as_text: doc.reads_as_text(),
        })
    }

    /// 用后台读取的文件 `loaded` 替换对应标签的内容，返回该标签的序号；无法替换时返回 `None`。
    ///
    /// 读取失败的文件替换为说明原因的占位标签。文档标识不变，`revision` 在原有值上继续递增，
    /// 按（标识、版本号）缓存的结果随之作废。读取期间标签已关闭、已经读取（经 [`Editor::load`]
    /// 就地读取）或路径已改变时不做改动，返回 `None`。
    pub fn finish_load(&mut self, loaded: LoadedFile) -> Option<usize> {
        let index = self.index_of(loaded.id)?;
        let doc = &mut self.documents[index];
        if !doc.is_pending() || doc.path.as_deref() != Some(loaded.path.as_path()) {
            return None;
        }
        let mut fresh = Document::from_opened(loaded.id, loaded.path, loaded.opened);
        // 可预览的文件按最近一次选择的方式显示；不可预览的文件不改变显示方式。
        if self.preview_default != PreviewMode::Source {
            fresh.set_preview_mode(self.preview_default);
        }
        adopt_bookmarks(&mut self.bookmarks, &mut fresh);
        adopt_folds(&mut self.folds, &mut fresh, self.tab_width);
        replace_document(doc, fresh);
        Some(index)
    }

    /// 当前标签尚未读取时读入文件，返回是否已读取；见 [`Editor::load`]。
    pub fn load_active(&mut self) -> bool {
        self.active.is_some_and(|index| self.load(index))
    }

    /// 把第 `index` 个无法打开的文件的占位标签改回尚未读取，返回是否改动。
    ///
    /// `as_text` 为 `false` 时对应「重试」，为 `true` 时对应「以文本方式打开」图片或二进制文件。
    /// 读取交给 [`Editor::begin_load`]（界面线程上在后台读取）或 [`Editor::load`]；重试仍然失败时，
    /// 读取结果仍是说明原因的占位标签。`index` 越界或标签不是占位标签时返回 `false`。
    pub fn reopen(&mut self, index: usize, as_text: bool) -> bool {
        let Some(doc) = self.documents.get_mut(index) else {
            return false;
        };
        let (Some(path), Some(_)) = (doc.path.clone(), &doc.unsupported) else {
            return false;
        };
        let fresh = if as_text {
            Document::pending_as_text(doc.id, path)
        } else {
            Document::pending(doc.id, path)
        };
        replace_document(doc, fresh);
        true
    }

    /// 在只读标签中显示一段文本（格式化工具的输出），标签标题为 `title`，并切换到该标签。
    ///
    /// 已存在标题相同且没有对应文件的标签时，替换其内容而不新开标签。返回标签序号（从 0 起）。
    pub fn show_output(&mut self, title: &str, text: &str) -> usize {
        self.show_readonly(title, |id| Document::output(id, title, text))
    }

    /// 在标签 `slot` 中显示并排差异（替换原有内容）并切换到该标签，返回标签序号（从 0 起）。
    ///
    /// `slot` 已经关闭或不是差异标签时按 [`Editor::show_diff`] 查找或新建。逐个预览多个文件的
    /// 改动时只占一个标签。
    pub fn show_diff_in(
        &mut self,
        slot: Option<DocumentId>,
        title: &str,
        mut view: diff_view::DiffView,
    ) -> usize {
        view.set_unified(self.diff_unified);
        let index = slot.and_then(|id| {
            self.documents
                .iter()
                .position(|d| d.id == id && d.diff.is_some())
        });
        let Some(index) = index else {
            return self.show_diff(title, view);
        };
        let doc = &mut self.documents[index];
        let fresh = Document::diff(doc.id, title, view);
        replace_document(doc, fresh);
        self.active = Some(index);
        index
    }

    /// 把差异标签 `id` 的内容替换为差异 `view`（标题改为 `title`），返回其序号，不切换标签。
    ///
    /// 滚动位置保持不变，折叠状态与光标是否在右侧沿用原差异。
    /// 标签已经关闭或不是差异标签时返回 `None`。
    pub fn replace_diff(
        &mut self,
        id: DocumentId,
        title: &str,
        view: diff_view::DiffView,
    ) -> Option<usize> {
        let index = self
            .documents
            .iter()
            .position(|d| d.id == id && d.diff.is_some())?;
        let doc = &mut self.documents[index];
        let (scroll, scroll_x) = (doc.scroll_line, doc.scroll_x);
        let mut view = view;
        if let Some(old) = doc.diff.as_deref() {
            view.inherit(old);
        }
        let fresh = Document::diff(id, title, view);
        replace_document(doc, fresh);
        doc.scroll_line = scroll;
        doc.scroll_x = scroll_x;
        Some(index)
    }

    /// 在只读标签中显示并排差异，标签标题为 `title`，并切换到该标签。
    ///
    /// 已存在标题相同、没有对应文件的标签时，替换为新的差异而不新开标签。返回标签序号（从 0 起）。
    pub fn show_diff(&mut self, title: &str, mut view: diff_view::DiffView) -> usize {
        view.set_unified(self.diff_unified);
        self.show_readonly(title, |id| Document::diff(id, title, view))
    }

    /// 在没有对应文件、标题为 `title` 的标签中显示 `make` 创建的文档并切换到该标签。
    ///
    /// 已有这样的标签时替换内容、保留文档标识，没有时新建。返回标签序号（从 0 起）。
    fn show_readonly(&mut self, title: &str, make: impl FnOnce(DocumentId) -> Document) -> usize {
        let existing = self
            .documents
            .iter()
            .position(|d| d.path.is_none() && d.title.as_deref() == Some(title));
        let index = match existing {
            Some(index) => {
                let slot = &mut self.documents[index];
                let fresh = make(slot.id);
                replace_document(slot, fresh);
                index
            }
            None => {
                let id = self.allocate_id();
                self.documents.push(make(id));
                self.documents.len() - 1
            }
        };
        self.active = Some(index);
        index
    }

    /// 切换到标题为 `title` 的日志标签，没有时新建，返回标签序号与是否新建。
    ///
    /// 新建的日志标签内容为空（`LogView::new()`），提交记录由调用方读取。
    pub fn show_log(&mut self, title: &str) -> (usize, bool) {
        if let Some(index) = self
            .documents
            .iter()
            .position(|d| d.log.is_some() && d.title.as_deref() == Some(title))
        {
            self.active = Some(index);
            return (index, false);
        }
        let id = self.allocate_id();
        self.documents
            .push(Document::log(id, title, git::log::LogView::new()));
        let index = self.documents.len() - 1;
        self.active = Some(index);
        (index, true)
    }

    /// 新建一个未命名的空文档并切换到对应标签，返回标签序号（从 0 起）。
    pub fn new_untitled(&mut self) -> usize {
        let id = self.allocate_id();
        self.documents.push(Document::empty(id));
        let index = self.documents.len() - 1;
        self.active = Some(index);
        index
    }

    /// 关闭第 `index` 个标签并返回其文档；`index` 越界时返回 `None`。
    ///
    /// 关闭活动标签后，活动标签切换到右侧相邻的标签；右侧没有标签时切换到左侧。文档的书签
    /// 存入 [`Self::bookmarks`]，折叠的块存入 [`Self::folds`]。
    pub fn close(&mut self, index: usize) -> Option<Document> {
        if index >= self.documents.len() {
            return None;
        }

        let document = self.documents.remove(index);
        if let Some(path) = &document.path
            && !document.bookmarks().is_empty()
        {
            self.bookmarks
                .insert(path.clone(), document.bookmarks().clone());
        }
        if let Some(path) = &document.path
            && !document.is_pending()
        {
            keep_folds(&mut self.folds, path, document.folded());
        }

        self.active = match self.active {
            _ if self.documents.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.min(self.documents.len() - 1)),
            other => other,
        };

        Some(document)
    }

    /// 把第 `from` 个标签移动到第 `to` 个的位置，活动标签仍然指向同一个文档。
    ///
    /// `from` 或 `to` 越界，或二者相等时不做任何改动。
    pub fn move_document(&mut self, from: usize, to: usize) {
        if from >= self.documents.len() || to >= self.documents.len() || from == to {
            return;
        }

        let document = self.documents.remove(from);
        self.documents.insert(to, document);

        self.active = self.active.map(|active| match active {
            a if a == from => to,
            a if from < a && a <= to => a - 1,
            a if to <= a && a < from => a + 1,
            a => a,
        });
    }

    /// 同步磁盘上 `from` 到 `to` 的改名（文件或目录）。
    ///
    /// 路径位于 `from` 之下的已打开文档，以及未打开文件的书签，都改用 `to` 之下的新路径。
    pub fn moved(&mut self, from: &Path, to: &Path) {
        // 文件树中的路径不解析符号链接，已打开文档的路径是解析后的规范路径（例如 `docs` 是
        // 符号链接时，文档路径位于链接目标目录内）。因此把 `from`、`to` 都规范化之后再做前缀比较。
        // `from` 在磁盘上已不存在，`canonical_path` 会改为规范化 `from` 所在的目录。
        let (from, to) = (canonical_path(from), canonical_path(to));
        for doc in &mut self.documents {
            if let Some(path) = doc.path.as_deref().and_then(|p| rebase(p, &from, &to)) {
                doc.moved_to(path);
            }
        }
        let moved: Vec<(PathBuf, PathBuf)> = self
            .bookmarks
            .keys()
            .filter_map(|old| Some((old.clone(), rebase(old, &from, &to)?)))
            .collect();
        for (old, new) in moved {
            if let Some(marks) = self.bookmarks.remove(&old) {
                self.bookmarks.insert(new, marks);
            }
        }
        let moved: Vec<PathBuf> = self.folds.keys().cloned().collect();
        for old in moved {
            if let (Some(new), Some(folds)) = (rebase(&old, &from, &to), self.folds.remove(&old)) {
                self.folds.insert(new, folds);
            }
        }
    }

    /// 返回所有文件的书签（键为路径）：未打开文件的书签加上已打开文档的书签。
    ///
    /// 用于把书签写入会话状态。占位标签（无法打开的文件、图片）和尚未读取的标签的书签仍保存在
    /// [`Self::bookmarks`] 中。
    pub fn all_bookmarks(&self) -> BookmarkMap {
        let mut all = self.bookmarks.clone();
        for doc in &self.documents {
            if let Some(path) = &doc.path
                && doc.unsupported.is_none()
                && !doc.is_pending()
            {
                if doc.bookmarks().is_empty() {
                    all.remove(path);
                } else {
                    all.insert(path.clone(), doc.bookmarks().clone());
                }
            }
        }
        all
    }

    /// 返回所有文件中折叠的块（键为路径）：未打开的文件加上已打开的文档，用于写入会话状态。
    pub fn all_folds(&self) -> FoldMap {
        let mut all = self.folds.clone();
        for doc in &self.documents {
            if let Some(path) = &doc.path
                && doc.unsupported.is_none()
                && !doc.is_pending()
            {
                keep_folds(&mut all, path, doc.folded());
            }
        }
        all
    }

    /// 打开目录作为工作区，只读取顶层目录，并清空未打开文件的书签和折叠的块。
    ///
    /// 两者按工作区目录保存，切换目录后不再适用；新目录的由调用方写入。
    ///
    /// # Errors
    ///
    /// 目录不存在或没有读权限时返回 `std::io::Error`。
    pub fn open_folder(&mut self, path: &Path) -> std::io::Result<()> {
        self.workspace = Some(Workspace::open(path)?);
        self.bookmarks.clear();
        self.folds.clear();
        Ok(())
    }

    fn allocate_id(&mut self) -> DocumentId {
        self.next_id += 1;
        DocumentId(self.next_id)
    }
}

/// 文档 `document` 能显示文本时，把 `bookmarks` 中该文件的书签移入文档。
///
/// 占位标签（无法打开的文件、图片）的文本为空，移入的书签会被限制到第 0 行，原有行号随之丢失，
/// 因此书签保留在 `bookmarks` 中。
fn adopt_bookmarks(bookmarks: &mut BookmarkMap, document: &mut Document) {
    if document.unsupported.is_some() {
        return;
    }
    if let Some(marks) = document.path.as_ref().and_then(|p| bookmarks.remove(p)) {
        document.set_bookmarks(marks);
    }
}

/// 各文件中折叠的块（开头行，末尾行），键为路径。
pub type FoldMap = std::collections::BTreeMap<PathBuf, Vec<(usize, usize)>>;

/// 文档 `document` 能显示文本时，把 `folds` 中该文件折叠的块移入文档（见
/// [`Document::restore_folds`]）。
fn adopt_folds(folds: &mut FoldMap, document: &mut Document, tab_width: usize) {
    if document.unsupported.is_some() {
        return;
    }
    if let Some(saved) = document.path.as_ref().and_then(|p| folds.remove(p)) {
        let tab_width = document.tab_width(tab_width);
        document.restore_folds(&saved, tab_width);
    }
}

/// 在 `folds` 中记下 `path` 折叠的块；没有折叠的块时去掉该文件。
fn keep_folds(folds: &mut FoldMap, path: &Path, folded: Vec<(usize, usize)>) {
    if folded.is_empty() {
        folds.remove(path);
    } else {
        folds.insert(path.to_path_buf(), folded);
    }
}

/// 返回 `from` 改名为 `to` 后路径 `path` 的新路径。
///
/// `path` 位于 `from` 之下（或等于 `from`）时返回 `to` 之下的对应路径；否则返回 `None`。
fn rebase(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(from).ok()?;
    Some(if rest.as_os_str().is_empty() {
        to.to_path_buf()
    } else {
        to.join(rest)
    })
}

/// 读取一个尚未读取的标签对应的文件，由 [`Editor::begin_load`] 创建。
#[derive(Debug)]
pub struct LoadJob {
    id: DocumentId,
    path: PathBuf,
    /// 是否把图片和二进制文件也按文本读入（「以文本方式打开」）。
    as_text: bool,
}

impl LoadJob {
    /// 待读取的标签。
    pub fn id(&self) -> DocumentId {
        self.id
    }

    /// 读取文件；耗时与文件大小成正比，在后台线程上调用。
    pub fn run(self) -> LoadedFile {
        LoadedFile {
            opened: Document::read(&self.path, self.as_text),
            id: self.id,
            path: self.path,
        }
    }
}

/// 后台读取完成的文件，交给 [`Editor::finish_load`]。
pub struct LoadedFile {
    id: DocumentId,
    path: PathBuf,
    opened: Opened,
}

/// 用新建的文档 `fresh` 替换 `slot`，并让 `revision` 在原有值上继续递增。
///
/// `fresh` 与 `slot` 使用相同的 `DocumentId`。界面层按（`DocumentId`，`revision`）缓存查找
/// 结果、粘滞滚动、括号配对和后台解码的图片，因此新文档的 `revision` 必须大于旧文档的，
/// 否则新内容会命中旧内容的缓存。
fn replace_document(slot: &mut Document, mut fresh: Document) {
    fresh.revision += slot.revision + 1;
    *slot = fresh;
}

/// 返回用于显示的名称：路径的最后一段；没有最后一段（如根目录）时返回整个路径。
pub fn display_name(path: &Path) -> Cow<'_, str> {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
}

/// 返回用于显示的路径。
///
/// 位于 `root` 之下时返回相对路径；位于用户主目录（环境变量 `HOME`）之下时返回 `~/…`；
/// 其余情况返回完整路径。
pub fn display_path(path: &Path, root: Option<&Path>) -> String {
    if let Some(root) = root
        && let Ok(relative) = path.strip_prefix(root)
    {
        return relative.display().to_string();
    }

    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }

    path.display().to_string()
}

/// 返回规范化后的绝对路径，解析符号链接（例如 macOS 上 `/var` 解析为 `/private/var`）。
///
/// 文件不存在时，规范化其所在目录并保留原文件名，使占位标签的路径也能与已有路径比较；
/// 所在目录同样无法规范化时返回原路径。
pub fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = winc_config::files::canonicalize(path) {
        return canonical;
    }

    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => winc_config::files::canonicalize(parent)
            .map_or_else(|_| path.to_path_buf(), |parent| parent.join(name)),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests;
