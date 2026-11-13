//! 工作区文件树 [`Workspace`]：按需读取目录并缓存，展开和折叠不再访问磁盘。
//!
//! 同时提供目录读取（[`list_dir`]）、自然排序（[`natural_cmp`]），以及新建、改名、移动、
//! 复制文件和目录的操作。

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ignore::WalkBuilder;
use winc_i18n::t;

use super::copying::{self, Copying};

/// 文件树里一项的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// 目录，可以展开。
    Dir,
    /// 文件，不能展开。
    File,
}

/// 文件树里可见的一行。
///
/// 行不另存路径和名字，而是指向目录缓存里那一项：通过 [`Deref`](std::ops::Deref) 到
/// [`DirEntry`]，`entry.path`、`entry.name`、`entry.kind` 读的就是缓存中的值。这样每次重新拍平
/// 只复制一个引用计数，不再为每个可见行分配路径和名字；克隆一行（文件树收起动画保存子行时）
/// 同样只增加引用计数。缓存中的目录被重新读取后，旧行仍然持有旧的列表，内容保持拍平时的样子。
#[derive(Clone)]
pub struct Entry {
    /// 这一行所在目录的缓存列表。
    siblings: Rc<[DirEntry]>,
    /// 这一行在 `siblings` 中的下标。
    index: usize,
    /// 缩进层级，根目录的直接子项为 0。
    pub depth: usize,
    /// 目录是否已展开；文件行总是 `false`。
    pub expanded: bool,
    /// 所在目录对应行的下标；根目录的直接子项为 `None`。
    pub parent: Option<usize>,
    /// 这一行之后第一个不在它下面的可见行的下标：`index + 1..end` 是它展开后的全部子孙。
    pub end: usize,
    /// 合并显示在这一行的连续目录，从最上一级到这一行本身；没有合并时为 `None`。
    ///
    /// 这一行的路径、名字和展开状态都是最下一级目录的。
    chain: Option<Rc<[PathBuf]>>,
}

impl Entry {
    /// 合并显示在这一行的连续目录，从最上一级到这一行本身；没有合并时为空。
    pub fn chain(&self) -> &[PathBuf] {
        self.chain.as_deref().unwrap_or_default()
    }

    /// 这一行显示的名字：合并的目录按 `/` 连接，没有合并时就是名字本身。
    pub fn label(&self) -> Cow<'_, str> {
        match self.chain() {
            [] => Cow::Borrowed(&self.name),
            chain => {
                let names: Vec<_> = chain.iter().map(|path| segment_name(path)).collect();
                Cow::Owned(names.join("/"))
            }
        }
    }
}

/// 返回路径最后一段的显示名。
pub fn segment_name(path: &Path) -> Cow<'_, str> {
    path.file_name()
        .map_or(Cow::Borrowed(""), |name| name.to_string_lossy())
}

impl std::ops::Deref for Entry {
    type Target = DirEntry;

    fn deref(&self) -> &DirEntry {
        &self.siblings[self.index]
    }
}

impl std::fmt::Debug for Entry {
    /// 只输出这一行本身，不输出整个目录列表。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("path", &self.path)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .field("expanded", &self.expanded)
            .finish()
    }
}

/// 目录里的一项，由 [`list_dir`] 读出。
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// 绝对路径。
    pub path: PathBuf,
    /// 文件名（不含路径）。
    pub name: String,
    /// 这一项的类别：目录或文件。
    pub kind: EntryKind,
    /// 是否为符号链接；类别按它指向的对象给出。
    pub symlink: bool,
}

/// 一个打开的工作区：目录树、展开状态和目录缓存。
///
/// 目录内容按需读取并缓存在 `children` 里，展开和折叠只重新拍平可见行，不再访问磁盘。
pub struct Workspace {
    /// 根目录（规范化后的绝对路径）。
    pub root: PathBuf,
    /// 根目录的显示名，标题栏和状态栏使用。
    pub name: String,
    /// 选中行在 [`Self::rows`] 中的下标（从 0 起）。
    ///
    /// 多选时是当前行：键盘从这里移动，新建项的位置由它决定；它本身可能不在选区里（⌘ 单击取消
    /// 选中的那一行）。
    ///
    /// 只读；改选中用 [`Self::select`] 等方法，它们同时维护多选。
    pub selected: Option<usize>,
    /// 多选时选中的各项（绝对路径）；为空时只有 [`Self::selected`] 那一行算选中。
    marked: HashSet<PathBuf>,
    /// 按 ⇧ 扩展选区的起点（绝对路径）。
    anchor: Option<PathBuf>,
    /// 选中行合并显示的目录中选中的那一级；选中最下一级或选中行没有合并时为 `None`。
    segment: Option<PathBuf>,
    /// 是否把连续的单子目录合并显示为一行。
    compact: bool,
    /// 已展开的目录（绝对路径）。
    expanded: HashSet<PathBuf>,
    /// 读过的目录内容，已排序。
    children: HashMap<PathBuf, Rc<[DirEntry]>>,
    /// 需要显示、但当前线程不能等待读取的目录（见 [`Workspace::take_missing`]）。
    missing: Missing,
    /// 按展开状态拍平后的可见行。
    rows: Vec<Entry>,
}

/// 尚未读取的目录：拍平时遇到的记入 `wanted`，交给后台读取后移入 `requested`，读好后移除。
#[derive(Default)]
struct Missing {
    wanted: Vec<PathBuf>,
    requested: HashSet<PathBuf>,
    /// 展开时未能读取的目录：读好后继续展开它下面连续的单子目录（见 `expand_chain`）。
    chains: HashSet<PathBuf>,
    /// 要选中、但所在目录尚未读好的路径（见 [`Workspace::reveal`]）。
    reveal: Option<PathBuf>,
}

impl Missing {
    /// 记下要交给后台读取的目录 `dir`；已记下或已交出的不重复。
    fn want(&mut self, dir: PathBuf) {
        if !self.requested.contains(&dir) && !self.wanted.contains(&dir) {
            self.wanted.push(dir);
        }
    }
}

/// 递归展开最多展开的目录数（个，经验值）：再多的行也无法浏览，读取也会明显卡住界面。
pub const EXPAND_DIRS: usize = 1000;

/// 递归展开最多查看的目录项数（个，经验值）：放着大量文件的目录不拖慢界面。
const EXPAND_ENTRIES: usize = 50_000;

/// 文件树中不显示的目录和文件名（按名字精确匹配），快速打开的文件索引同样跳过。
pub(crate) const HIDDEN: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    ".idea",
    ".DS_Store",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
];

/// 返回遍历工作区目录的配置，快速打开的文件索引和全局搜索共用，两者看到的文件相同。
///
/// 遵守各级 `.gitignore`、`.ignore` 和全局的忽略规则，不进入指向目录的符号链接，跳过
/// [`HIDDEN`] 中的名字；指向文件的符号链接是否收录由 [`is_listed_file`] 判断。
///
/// 在版本库内时 `.gitignore` 只到版本库的根目录为止，与 Git 相同，上级目录的规则不影响版本库。
/// 不在版本库内时 `.gitignore` 同样生效：下载的源码包也不会列出 `node_modules` 这类目录。
pub(crate) fn walker(dir: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(dir);
    // `hidden(false)` 关闭 `ignore` 对以 `.` 开头的名字的过滤：`.github`、`.editorconfig`
    // 照常收录，只有 [`HIDDEN`] 中的名字由 `filter_entry` 跳过。
    builder
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(in_repository(dir))
        .follow_links(false)
        .filter_entry(|entry| !HIDDEN.contains(&entry.file_name().to_string_lossy().as_ref()));
    builder
}

/// 同 [`walker`]，但不遵守忽略规则，只跳过 [`HIDDEN`] 中的名字。
pub(crate) fn walker_with_ignored(dir: &Path) -> WalkBuilder {
    let mut builder = walker(dir);
    builder
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false);
    builder
}

/// `dir` 或它的某一级上级目录是否是版本库的根目录。
fn in_repository(dir: &Path) -> bool {
    dir.ancestors().any(|dir| {
        winc_backend::fs::exists(dir.join(".git")) || winc_backend::fs::exists(dir.join(".jj"))
    })
}

/// 判断类型为 `kind` 的目录项 `path` 是否作为文件收录：普通文件，或指向普通文件的符号链接。
///
/// 只有符号链接才多做一次 stat。指向目录的符号链接不收录、不进入，避免链接成环时无限遍历，
/// 以及同一棵目录被重复收录。
pub(crate) fn is_listed_file(kind: impl Into<winc_backend::fs::Kind>, path: &Path) -> bool {
    let kind = kind.into();
    kind.is_file() || kind.is_symlink() && winc_backend::fs::is_file(path)
}

impl Workspace {
    /// 打开目录作为工作区，只读取顶层，返回就绪的文件树。
    ///
    /// # Errors
    ///
    /// 目录不存在、没有读权限，或读取顶层目录失败时返回对应的 `std::io::Error`。
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = winc_config::files::canonicalize(root)?;
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        // 远端主机上的目录在名称后标出主机：标题栏和状态栏上一眼能看出不是本机的目录。
        let name = match winc_backend::locate(&root) {
            None => name,
            Some(at) if at.path == b"/" => format!("/ [SSH: {}]", at.target.as_str()),
            Some(at) => format!("{name} [SSH: {}]", at.target.as_str()),
        };

        let mut workspace = Workspace {
            root: root.clone(),
            name,
            selected: None,
            marked: HashSet::new(),
            anchor: None,
            segment: None,
            compact: true,
            expanded: HashSet::from([root]),
            children: HashMap::new(),
            missing: Missing::default(),
            rows: Vec::new(),
        };
        workspace.rebuild();
        Ok(workspace)
    }

    /// 返回文件树当前可见的行（按显示顺序）。
    pub fn rows(&self) -> &[Entry] {
        &self.rows
    }

    /// 设置是否把连续的单子目录合并显示为一行。
    pub fn set_compact(&mut self, compact: bool) {
        if self.compact != compact {
            self.compact = compact;
            self.rebuild();
        }
    }

    /// 显示路径 `path` 的行的下标（合并显示时也算）；不在可见行里（没展开到那里、不在工作区里）
    /// 时返回 `None`。
    ///
    /// 按路径的各级逐层往下找，每层跳过不是 `path` 上级的兄弟连同其子孙（`Entry::end`），耗时与
    /// 层数和每层的项数有关，与整棵树的行数无关：拖动时每帧都要找放置目标所在的行。
    pub fn row_of(&self, path: &Path) -> Option<usize> {
        let (mut i, mut end) = (0, self.rows.len());
        while i < end {
            let entry = &self.rows[i];
            if entry.path == path || entry.chain().iter().any(|p| p == path) {
                return Some(i);
            }
            if entry.expanded && path.starts_with(&entry.path) {
                (i, end) = (i + 1, entry.end);
            } else {
                i = entry.end.max(i + 1);
            }
        }
        None
    }

    /// 切换第 `index` 行的展开状态；文件行和越界下标不做任何事。
    pub fn toggle(&mut self, index: usize) {
        let Some(entry) = self.rows.get(index) else {
            return;
        };

        if entry.kind != EntryKind::Dir {
            return;
        }

        let path = entry.path.clone();

        if !self.expanded.remove(&path) {
            self.expand_chain(path);
        }

        self.rebuild();
    }

    /// 展开第 `index` 行的目录和它下面的各级目录；文件行和越界下标不做任何事。
    ///
    /// 遵守忽略规则，不进入符号链接，也不展开文件树中不显示的名字；这一行本身被忽略时只展开
    /// 一层。返回是否因目录太多只展开了一部分（见 [`EXPAND_DIRS`]）。
    pub fn expand_recursively(&mut self, index: usize) -> bool {
        self.expand_within(index, EXPAND_DIRS, EXPAND_ENTRIES)
    }

    /// 同 [`Self::expand_recursively`]，上限是目录数 `dirs` 和目录项数 `entries`。
    fn expand_within(&mut self, index: usize, dirs: usize, entries: usize) -> bool {
        let Some(entry) = self.rows.get(index).filter(|e| e.kind == EntryKind::Dir) else {
            return false;
        };
        let path = entry.path.clone();
        let listed = path.parent().is_some_and(|parent| {
            walker(parent)
                .max_depth(Some(1))
                .build()
                .filter_map(Result::ok)
                .any(|e| e.path() == path)
        });
        if !listed {
            self.expand_chain(path);
            self.rebuild();
            return false;
        }

        let mut walk = walker(&path);
        walk.sort_by_file_name(|a, b| a.cmp(b));
        let (mut expanded, mut seen, mut limited) = (0, 0, false);
        for item in walk.build().filter_map(Result::ok) {
            seen += 1;
            let dir = item.file_type().is_some_and(|kind| kind.is_dir());
            if seen > entries || (dir && expanded == dirs) {
                limited = true;
                break;
            }
            if dir {
                expanded += 1;
                self.expanded.insert(item.into_path());
            }
        }
        self.rebuild();
        limited
    }

    /// 收起第 `index` 行的目录和它下面的各级目录，不访问磁盘；文件行和越界下标不做任何事。
    pub fn collapse_recursively(&mut self, index: usize) {
        let Some(entry) = self.rows.get(index).filter(|e| e.kind == EntryKind::Dir) else {
            return;
        };
        let path = entry.path.clone();
        self.expanded.retain(|p| !p.starts_with(&path));
        self.rebuild();
    }

    /// 展开目录 `path`；合并显示时，连同它下面连续的单子目录一起展开，一次就能看到最下一级的内容。
    fn expand_chain(&mut self, mut path: PathBuf) {
        while self.compact {
            let children = cached_children(&mut self.children, &mut self.missing, &path);
            if !self.children.contains_key(&path) {
                self.missing.chains.insert(path.clone());
            }
            let Some(only) = sole_folder(&children) else {
                break;
            };
            let next = only.path.clone();
            self.expanded.insert(path);
            path = next;
        }
        self.expanded.insert(path);
    }

    /// 展开 `path` 的所有上级目录并选中它，使其在树中可见；`path` 不在工作区内时不做任何事。
    pub fn reveal(&mut self, path: &Path) {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return;
        };
        let mut current = self.root.clone();

        // 远端的目录由后台读取：各级上级目录的路径已经知道，一起要，不必等上一级读好才发现
        // 下一级。
        let remote = winc_backend::is_remote(&self.root);
        for component in relative.parent().into_iter().flat_map(Path::components) {
            current.push(component);
            self.expanded.insert(current.clone());
            if remote && !self.children.contains_key(&current) {
                self.missing.want(current.clone());
            }
        }

        self.rebuild();
        match self.row_of(path) {
            Some(index) => self.select_segment(index, Some(path)),
            None => {
                self.select(None);
                // 它所在的目录还在等待后台读取：读好后再选中。
                if !self.missing.wanted.is_empty() || !self.missing.requested.is_empty() {
                    self.missing.reveal = Some(path.to_path_buf());
                }
            }
        }
    }

    /// 重新读取全部已缓存的目录，已不是目录的展开项被移除。
    ///
    /// 选中的行尽量保持不变。
    ///
    /// 就在调用的线程上读；界面线程上用 [`Workspace::scan`] 把读交给后台。
    pub fn refresh(&mut self) {
        let listing = self.scan(None).run();
        self.apply_scan(listing);
    }

    /// 通知 `paths` 在磁盘上发生了变化（被其他程序新建、删除或改名），返回文件树是否随之变化。
    ///
    /// 只重新读取已缓存且受影响的目录，见 [`Workspace::scan`]、[`Workspace::apply_scan`]。
    pub fn refresh_paths(&mut self, paths: &[PathBuf]) -> bool {
        let listing = self.scan(Some(paths)).run();
        self.apply_scan(listing)
    }

    /// 折叠全部目录，只保留根目录展开。
    pub fn collapse_all(&mut self) {
        self.expanded.retain(|p| *p == self.root);
        self.rebuild();
    }

    /// 返回选中的行；没有选中或下标越界时返回 `None`。
    ///
    /// 多选时是当前行。
    pub fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|i| self.rows.get(i))
    }

    /// 只选中第 `index` 行（`None` 为都不选中），清掉多选；这一行也是之后按 ⇧ 扩展选区的起点。
    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.rows.len());
        self.marked.clear();
        self.segment = None;
        self.anchor = self.selected_entry().map(|e| e.path.clone());
    }

    /// 只选中第 `index` 行，以及这一行合并显示的目录中的 `path`；`path` 为 `None` 或不在其中时
    /// 选中这一行本身。
    pub fn select_segment(&mut self, index: usize, path: Option<&Path>) {
        self.select(Some(index));
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let inner = path.filter(|&p| p != entry.path && entry.chain().iter().any(|c| c == p));
        self.segment = inner.map(Path::to_path_buf);
    }

    /// 选中行合并显示的目录中选中的那一级；选中最下一级或选中行没有合并时为 `None`。
    pub fn segment(&self) -> Option<&Path> {
        self.segment.as_deref()
    }

    /// 当前行实际指向的路径：合并显示时是选中的那一级目录；没有选中时为 `None`。
    pub fn current_path(&self) -> Option<&Path> {
        self.segment
            .as_deref()
            .or_else(|| self.selected_entry().map(|e| e.path.as_path()))
    }

    /// 把第 `index` 行加入或移出选区（⌘ 单击）；这一行成为当前行和扩展选区的起点。
    ///
    /// 选区因此变空时改为不选中。
    pub fn toggle_selected(&mut self, index: usize) {
        let Some(path) = self.rows.get(index).map(|e| e.path.clone()) else {
            return;
        };
        self.segment = None;
        if self.marked.is_empty()
            && let Some(current) = self.selected_entry()
        {
            self.marked.insert(current.path.clone());
        }
        if !self.marked.remove(&path) {
            self.marked.insert(path.clone());
        }
        if self.marked.is_empty() {
            self.select(None);
            return;
        }
        self.selected = Some(index);
        self.anchor = Some(path);
    }

    /// 选中从扩展选区的起点到第 `index` 行之间的各行（⇧ 单击、⇧↑↓），代替原来的选区；
    /// 第 `index` 行成为当前行，起点不变。
    ///
    /// 没有起点时从当前行起，也没有当前行时只选这一行。
    pub fn select_range(&mut self, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.segment = None;
        let from = self
            .anchor
            .as_ref()
            .and_then(|a| self.rows.iter().position(|e| e.path == *a))
            .or(self.selected)
            .unwrap_or(index);
        let (lo, hi) = (from.min(index), from.max(index));
        self.marked = self.rows[lo..=hi].iter().map(|e| e.path.clone()).collect();
        self.anchor = Some(self.rows[from].path.clone());
        self.selected = Some(index);
    }

    /// 选中所有可见的行；当前行不变（没有时为第一行）。
    pub fn select_all(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.segment = None;
        self.marked = self.rows.iter().map(|e| e.path.clone()).collect();
        self.selected = self.selected.or(Some(0));
    }

    /// 选中 `paths` 里看得见的各项（一次移动了几项以后）；第一项成为当前行。
    ///
    /// 都看不见时不改选中。
    pub fn select_paths(&mut self, paths: &[PathBuf]) {
        let Some(first) = paths
            .iter()
            .find_map(|p| self.rows.iter().position(|e| e.path == *p))
        else {
            return;
        };
        self.select(Some(first));
        if paths.len() > 1 {
            let visible: HashSet<&Path> = self.rows.iter().map(|e| e.path.as_path()).collect();
            self.marked = paths
                .iter()
                .filter(|p| visible.contains(p.as_path()))
                .cloned()
                .collect();
        }
    }

    /// 第 `index` 行是否选中。
    pub fn is_selected(&self, index: usize) -> bool {
        if self.marked.is_empty() {
            return self.selected == Some(index);
        }
        self.rows
            .get(index)
            .is_some_and(|e| self.marked.contains(&e.path))
    }

    /// 是否选中了不止一项。
    pub fn multi_selected(&self) -> bool {
        self.marked.len() > 1
    }

    /// 返回选中的各项的路径，按显示顺序；没有选中时为空。
    pub fn selection(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            return self
                .current_path()
                .map(Path::to_path_buf)
                .into_iter()
                .collect();
        }
        self.rows
            .iter()
            .filter(|e| self.marked.contains(&e.path))
            .map(|e| e.path.clone())
            .collect()
    }

    /// 同 [`Self::selection`]，但去掉上级目录也选中了的项：删除、移动、复制时只需处理上级目录。
    pub fn selection_roots(&self) -> Vec<PathBuf> {
        let all = self.selection();
        // 按路径的各段排序后，目录下的项紧跟在目录之后：一遍就能认出上级目录也选中了的项，
        // 不必两两比较（全选上万项时是平方级）。
        let mut sorted: Vec<&PathBuf> = all.iter().collect();
        sorted.sort_unstable();
        let mut roots = std::collections::HashSet::new();
        let mut last: Option<&PathBuf> = None;
        for path in sorted {
            if last.is_some_and(|root| path.starts_with(root)) {
                continue;
            }
            roots.insert(path);
            last = Some(path);
        }
        all.iter().filter(|p| roots.contains(p)).cloned().collect()
    }

    /// 返回新建文件或目录的默认位置：选中目录时为该目录，选中文件时为其所在目录，
    /// 没有选中时为根目录。
    pub fn target_dir(&self) -> PathBuf {
        if let Some(segment) = &self.segment {
            return segment.clone();
        }
        match self.selected_entry() {
            Some(e) if e.kind == EntryKind::Dir => e.path.clone(),
            Some(e) => e
                .path
                .parent()
                .map_or_else(|| self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// 在 `dir` 中新建文件或目录（`name` 可以含 `/`，一次创建多级目录），刷新文件树并选中新建项。
    ///
    /// 返回新路径；目标已存在时返回错误，不覆盖。
    ///
    /// # Errors
    ///
    /// 名称无效或越出工作区时返回 `InvalidInput`，已存在时返回 `AlreadyExists`，创建失败时
    /// 返回对应的 `std::io::Error`。
    pub fn create(&mut self, dir: &Path, name: &str, kind: EntryKind) -> io::Result<PathBuf> {
        self.create_entry(dir, name, kind, &[])
    }

    /// 在 `dir` 中新建内容为 `contents` 的文件，其余与 [`Workspace::create`] 相同（粘贴文本、
    /// 图片为新文件时使用）。
    ///
    /// # Errors
    ///
    /// 与 [`Workspace::create`] 相同；写入失败时删除写了一半的文件。
    pub fn create_file(&mut self, dir: &Path, name: &str, contents: &[u8]) -> io::Result<PathBuf> {
        self.create_entry(dir, name, EntryKind::File, contents)
    }

    /// 新建文件（内容为 `contents`）或目录，见 [`Workspace::create`]。
    fn create_entry(
        &mut self,
        dir: &Path,
        name: &str,
        kind: EntryKind,
        contents: &[u8],
    ) -> io::Result<PathBuf> {
        let path = create_at(&self.root, dir, name, kind, contents)?;
        self.created(&path);
        Ok(path)
    }

    /// 在磁盘上新建了 `path` 之后更新文件树：重新读取，展开到它所在的位置并选中它。
    pub fn created(&mut self, path: &Path) {
        self.refresh();
        self.reveal(path);
    }

    /// 把 `path` 改名为同一目录下的 `name`，返回新路径。
    ///
    /// 已展开的子目录改名后仍然展开，改完选中新的一行；只改大小写同样可行，不区分大小写的文件系统上
    /// 新名字「已经存在」的正是它自己。
    ///
    /// # Errors
    ///
    /// 名称无效时返回 `InvalidInput`，目标已被其他项占用时返回 `AlreadyExists`。
    pub fn rename(&mut self, path: &Path, name: &str) -> io::Result<PathBuf> {
        let target = rename_at(&self.root, path, name)?;
        if target != path {
            self.relocated(path, &target);
        }
        Ok(target)
    }

    /// 把 `path` 移动到目录 `dir` 中，名字不变，返回新路径；`path` 可以在工作区外。
    ///
    /// 已展开的子目录移动后仍然展开，树会展开到新位置并选中它；`path` 已在该目录中时不做任何事。
    ///
    /// # Errors
    ///
    /// 移动工作区根目录、`dir` 不在工作区内、移入自身内部时返回 `InvalidInput`，目标已存在时返回
    /// `AlreadyExists`。
    pub fn move_into(&mut self, path: &Path, dir: &Path) -> io::Result<PathBuf> {
        let target = move_at(&self.root, path, dir)?;
        if target != path {
            self.relocated(path, &target);
        }
        Ok(target)
    }

    /// 把文件或目录 `source` 复制进工作区里的目录 `dir`（见 [`copy_entry`]），刷新文件树，展开到
    /// 副本所在位置并选中它，返回副本的路径。
    ///
    /// # Errors
    ///
    /// 同 [`copy_entry`]。
    pub fn copy_into(&mut self, source: &Path, dir: &Path) -> io::Result<PathBuf> {
        let target = copy_entry(&self.root, source, dir, &Copying::default())?;
        self.created(&target);
        Ok(target)
    }

    /// 在 `from` 已于磁盘上移动到 `to` 之后更新文件树：展开状态随之迁移，重新读取树，
    /// 展开到新位置并选中它。
    pub fn relocated(&mut self, from: &Path, to: &Path) {
        let moved: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter(|p| p.starts_with(from))
            .cloned()
            .collect();
        for old in moved {
            self.expanded.remove(&old);
            if let Ok(rest) = old.strip_prefix(from) {
                self.expanded.insert(to.join(rest));
            }
        }

        self.refresh();
        self.reveal(to);
    }

    /// 按展开状态重新拍平可见行。
    ///
    /// 选中项保持不变；选中项被折叠隐藏时，改为选中最近的可见上级目录。
    /// 多选中不再可见的项移出选区。
    fn rebuild(&mut self) {
        let selected = self.current_path().map(Path::to_path_buf);
        let mut rows = Vec::with_capacity(self.rows.len());
        flatten(
            &mut self.children,
            &mut self.missing,
            &self.expanded,
            self.compact,
            &self.root,
            None,
            &mut rows,
        );
        self.rows = rows;
        self.selected = selected.as_ref().and_then(|path| {
            self.rows.iter().rposition(|e| {
                path.starts_with(&e.path) || e.chain().iter().any(|member| member == path)
            })
        });
        // 选中的目录仍合并在这一行时继续选中它；已有自己的一行或已不存在时选中这一行。
        self.segment = self.selected_entry().and_then(|entry| {
            let path = selected.as_ref()?;
            (*path != entry.path && entry.chain().contains(path)).then(|| path.clone())
        });
        // 折叠或删除后看不见的项不再算选中。
        if !self.marked.is_empty() {
            let visible: HashSet<&Path> = self.rows.iter().map(|e| e.path.as_path()).collect();
            self.marked.retain(|p| visible.contains(p.as_path()));
        }
    }
}

/// 返回目录 `dir` 的内容；未缓存时先读取并写入 `cache`，无法读取的目录按空目录处理。
///
/// 当前线程不能等待读取时（界面线程上的远端目录）不写入缓存，把目录记入 `missing`，这次按空
/// 目录处理，读好后由 [`Workspace::apply_scan`] 换入。
fn cached_children(
    cache: &mut HashMap<PathBuf, Rc<[DirEntry]>>,
    missing: &mut Missing,
    dir: &Path,
) -> Rc<[DirEntry]> {
    if let Some(children) = cache.get(dir) {
        return Rc::clone(children);
    }
    let children: Rc<[DirEntry]> = match list_dir(dir) {
        Ok(children) => children.into(),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            missing.want(dir.to_path_buf());
            return Rc::from([]);
        }
        Err(_) => Rc::from([]),
    };
    cache.insert(dir.to_path_buf(), Rc::clone(&children));
    children
}

/// 返回可以和所在目录合并显示的子目录：目录的内容 `children` 只有一项，且是目录。
///
/// 指向目录的符号链接不合并，避免链接成环时无限展开。
fn sole_folder(children: &[DirEntry]) -> Option<&DirEntry> {
    match children {
        [only] if only.kind == EntryKind::Dir && !only.symlink => Some(only),
        _ => None,
    }
}

/// 把 `dir` 下已展开的部分按显示顺序追加到 `rows`；未缓存的目录先读取并写入 `cache`。
///
/// `compact` 为真时，已展开的目录中唯一的一项也是目录时与它合并为一行。收起的目录不读取内容，
/// 因此不会访问磁盘去判断能否合并。
fn flatten(
    cache: &mut HashMap<PathBuf, Rc<[DirEntry]>>,
    missing: &mut Missing,
    expanded: &HashSet<PathBuf>,
    compact: bool,
    dir: &Path,
    parent: Option<usize>,
    rows: &mut Vec<Entry>,
) {
    let depth = parent.map_or(0, |p| rows[p].depth + 1);
    let children = cached_children(cache, missing, dir);

    for index in 0..children.len() {
        let (mut siblings, mut at) = (Rc::clone(&children), index);
        let mut chain = vec![];
        while compact
            && siblings[at].kind == EntryKind::Dir
            && expanded.contains(&siblings[at].path)
        {
            let inner = cached_children(cache, missing, &siblings[at].path);
            if sole_folder(&inner).is_none() {
                break;
            }
            chain.push(siblings[at].path.clone());
            (siblings, at) = (inner, 0);
        }
        let tail = siblings[at].path.clone();
        let kind = siblings[at].kind;
        let chain = (!chain.is_empty()).then(|| {
            chain.push(tail.clone());
            Rc::from(chain)
        });
        let open = kind == EntryKind::Dir && expanded.contains(&tail);
        let row = rows.len();
        rows.push(Entry {
            siblings,
            index: at,
            depth,
            expanded: open,
            parent,
            end: row + 1,
            chain,
        });

        if open {
            flatten(cache, missing, expanded, compact, &tail, Some(row), rows);
            rows[row].end = rows.len();
        }
    }
}

/// 在工作区 `root` 里的目录 `dir` 中新建文件（内容为 `contents`）或目录 `name`，返回其路径；规则见
/// [`Workspace::create`]。
///
/// 只访问磁盘、不改文件树，可以在后台线程上调用；完成后把路径交给 [`Workspace::created`]。
///
/// # Errors
///
/// 同 [`Workspace::create`]。
pub fn create_at(
    root: &Path,
    dir: &Path,
    name: &str,
    kind: EntryKind,
    contents: &[u8],
) -> io::Result<PathBuf> {
    let name = name.trim().trim_matches('/');
    let invalid = name.is_empty()
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");

    if invalid || !dir.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            t!("workspace-tree-invalid-name"),
        ));
    }

    // 名字是用户输入的：经 `join` 换成路径，远端的名字按需转义。
    let path = winc_backend::join(dir, name);

    if winc_backend::fs::exists(&path) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            t!("workspace-tree-name-taken"),
        ));
    }

    match kind {
        EntryKind::Dir => winc_backend::fs::create_dir_all(&path)?,
        EntryKind::File => {
            if let Some(parent) = path.parent() {
                winc_backend::fs::create_dir_all(parent)?;
            }
            winc_backend::fs::create_new(&path, contents)?;
        }
    }
    Ok(path)
}

/// 把工作区 `root` 中的 `path` 改名为同一目录下的 `name`，返回新路径；规则见
/// [`Workspace::rename`]。名字没有变化时不访问磁盘，返回原路径。
///
/// 只访问磁盘、不改文件树，可以在后台线程上调用；完成后把两个路径交给
/// [`Workspace::relocated`]。
///
/// # Errors
///
/// 同 [`Workspace::rename`]。
pub fn rename_at(root: &Path, path: &Path, name: &str) -> io::Result<PathBuf> {
    let name = name.trim();
    let invalid = name.is_empty() || name == "." || name == ".." || name.contains('/');

    if invalid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            t!("workspace-tree-invalid-name"),
        ));
    }
    if path == root || !path.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            t!("workspace-tree-cannot-rename"),
        ));
    }

    let target = winc_backend::join(path.parent().unwrap_or(root), name);
    if target == path {
        return Ok(target);
    }

    let case_only = occupied(&target) && same_entry(path, &target);
    if occupied(&target) && !case_only {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            t!("workspace-tree-name-taken"),
        ));
    }

    if case_only {
        rename_case(path, &target)?;
    } else {
        winc_backend::fs::rename(path, &target)?;
    }
    Ok(target)
}

/// 把 `path` 移动到工作区 `root` 里的目录 `dir` 中，名字不变，返回新路径；规则见
/// [`Workspace::move_into`]。已在该目录中时不访问磁盘，返回原路径。
///
/// 只访问磁盘、不改文件树，可以在后台线程上调用；完成后把两个路径交给
/// [`Workspace::relocated`]。
///
/// # Errors
///
/// 同 [`Workspace::move_into`]。
pub fn move_at(root: &Path, path: &Path, dir: &Path) -> io::Result<PathBuf> {
    let invalid = |reason| Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    if path == root || !dir.starts_with(root) {
        return invalid(t!("workspace-tree-cannot-move"));
    }
    if dir.starts_with(path) {
        return invalid(t!("workspace-tree-move-into-itself"));
    }
    let Some(name) = path.file_name() else {
        return invalid(t!("workspace-tree-cannot-move"));
    };

    let target = dir.join(name);
    if target == path {
        return Ok(target);
    }
    if occupied(&target) {
        let dir_name = dir.file_name().unwrap_or(dir.as_os_str()).to_string_lossy();
        let message = t!("workspace-tree-name-taken-in", folder = dir_name);
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, message));
    }

    winc_backend::fs::rename(path, &target)?;
    Ok(target)
}

/// 只改大小写的改名：先改成同一目录下的临时名字，再改成 `target`。
///
/// 不区分大小写的文件系统上，有的环境（部分网络位置、Wine）把只改大小写的改名当作改成自己，
/// 名字不变也不报错。临时名字换不成 `target` 时改回原来的名字。
fn rename_case(path: &Path, target: &Path) -> io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.{}.rename", std::process::id()));
    winc_backend::fs::rename(path, &temp)?;
    if let Err(e) = winc_backend::fs::rename(&temp, target) {
        let _ = winc_backend::fs::rename(&temp, path);
        return Err(e);
    }
    Ok(())
}

/// 是否 `path` 上已有任何条目。
///
/// 失效的符号链接也算：`fs::rename` 会覆盖它。
fn occupied(path: &Path) -> bool {
    winc_backend::fs::symlink_metadata(path).is_ok()
}

/// 把工作区 `root` 外（或内）的文件或目录 `source` 复制进 `root` 里的目录 `dir`，返回副本的路径。
///
/// 目录连同其中的内容一起复制，符号链接复制为链接本身；`dir` 中已有同名项时不覆盖，副本改名为
/// 「名字 2.扩展名」「名字 3.扩展名」等。进度记入 `copying`，经它可以取消。只访问磁盘、不改
/// 文件树，可以在后台线程上调用。
///
/// # Errors
///
/// `dir` 不在工作区内或复制进 `source` 内时返回 `InvalidInput`，被取消时返回 `Interrupted`
/// （已复制出的部分随之删除），其余返回 `std::io::Error`。
pub fn copy_entry(
    root: &Path,
    source: &Path,
    dir: &Path,
    copying: &Copying,
) -> io::Result<PathBuf> {
    let invalid = |reason| Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
    if !dir.starts_with(root) {
        return invalid(t!("workspace-tree-target-outside"));
    }
    if dir.starts_with(source) {
        return invalid(t!("workspace-tree-copy-into-itself"));
    }
    let Some(name) = source.file_name() else {
        return invalid(t!("workspace-tree-cannot-copy"));
    };
    let target = free_name(dir, Path::new(name));
    let links: copying::Links = |from, to| winc_backend::fs::copy_symlink(from, to);
    copying::copy_item(source, &target, links, copying)?;
    Ok(target)
}

/// 返回目录 `dir` 中名为 `name` 的一项的路径；已被占用时依次改为「名字 2.扩展名」「名字 3.扩展
/// 名」……，直到找到未被占用的名字。
///
/// 以 `.` 开头、没有扩展名的名字（`.env`）整个当作名字。
fn free_name(dir: &Path, name: &Path) -> PathBuf {
    let first = dir.join(name);
    if !occupied(&first) {
        return first;
    }
    let stem = name
        .file_stem()
        .unwrap_or(name.as_os_str())
        .to_string_lossy();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    (2..)
        .map(|n| dir.join(format!("{stem} {n}{}", ext.as_deref().unwrap_or(""))))
        .find(|p| !occupied(p))
        .expect("总能找到没被占用的名字")
}

/// 是否两个路径指向磁盘上的同一项：不区分大小写的文件系统上，`a` 和 `A` 就是同一项。
///
/// 区分大小写的文件系统上它们是两个文件，把其中一个改成另一个的名字会覆盖对方。
#[cfg(unix)]
fn same_entry(a: &Path, b: &Path) -> bool {
    let identity = |path: &Path| {
        let unix = winc_backend::fs::symlink_metadata(path).ok()?.unix?;
        Some((unix.dev, unix.ino))
    };
    identity(a).is_some_and(|a| Some(a) == identity(b))
}

/// 是否两个路径指向同一项：非 Unix 平台（即 Windows）按文件系统不区分大小写处理，
/// 名字只有大小写不同的两个路径视为同一项。
#[cfg(not(unix))]
fn same_entry(a: &Path, b: &Path) -> bool {
    let lower = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    a.parent() == b.parent() && lower(a) == lower(b)
}

/// 读取目录的直接子项：跳过 `HIDDEN` 中的名字，目录在前，同类按名字自然排序（见
/// [`natural_cmp`]）。
///
/// # Errors
///
/// 目录不存在或没有读权限时返回对应的 `std::io::Error`。
pub fn list_dir(dir: &Path) -> io::Result<Vec<DirEntry>> {
    let mut children: Vec<DirEntry> = winc_backend::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.name.to_string_lossy().into_owned();

            if HIDDEN.contains(&name.as_str()) {
                return None;
            }

            let path = entry.path();
            // 只有符号链接需要再调用一次 `stat`，判断它指向的是否为目录。
            let symlink = entry.kind.is_symlink();
            let is_dir = if symlink {
                winc_backend::fs::is_dir(&path)
            } else {
                entry.kind.is_dir()
            };
            let kind = if is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            Some(DirEntry {
                path,
                name,
                kind,
                symlink,
            })
        })
        .collect();

    children.sort_by(|a, b| {
        (a.kind != EntryKind::Dir)
            .cmp(&(b.kind != EntryKind::Dir))
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(children)
}

/// 按自然顺序比较两个名字：连续数字按数值比较，其余字符不区分大小写，因此 `file2` 排在
/// `file10` 之前。
///
/// 这是全序，只有两个名字完全相同时才返回 `Equal`：按上述规则相同时，先比较第一处数值相同、
/// 位数不同的数字（位数少的在前，`file1` 排在 `file01` 之前），再按原样逐字节比较（`A.rs`
/// 排在 `a.rs` 之前）。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (whole_a, whole_b) = (a, b);
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    // 第一处数值相同、位数不同的数字的比较结果，规则相同时据此区分。
    let mut digits = Ordering::Equal;

    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return digits.then_with(|| whole_a.cmp(whole_b)),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let ((x, x_len), (y, y_len)) = (take_number(&mut a), take_number(&mut b));
                match x.cmp(&y) {
                    Ordering::Equal => digits = digits.then(x_len.cmp(&y_len)),
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                match x.to_lowercase().cmp(y.to_lowercase()) {
                    Ordering::Equal => {}
                    other => return other,
                }

                a.next();
                b.next();
            }
        }
    }
}

/// 读取 `chars` 开头的连续数字并前移迭代器，返回其数值（超过 `u128` 的范围时饱和）和位数。
fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> (u128, usize) {
    let (mut n, mut len): (u128, usize) = (0, 0);

    while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(d as u128);
        len += 1;
        chars.next();
    }

    (n, len)
}

mod scan;
pub use scan::{DirListing, DirScan};

#[cfg(test)]
mod tests;
