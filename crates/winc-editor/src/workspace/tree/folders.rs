//! 附加的文件夹：一个工作区在根目录之外再显示的目录。
//!
//! 根目录决定工作区的名称和所在的系统；附加的文件夹只参与浏览和查找。有附加的文件夹时，文件树
//! 最上一层是各个文件夹（根目录在前），各自可以收起。

use super::*;

impl Workspace {
    /// 返回附加的文件夹，按添加的顺序。
    pub fn extra_folders(&self) -> &[PathBuf] {
        &self.extra
    }

    /// 返回工作区的全部文件夹：根目录在前，其后是附加的文件夹。
    pub fn folders(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.root.as_path()).chain(self.extra.iter().map(PathBuf::as_path))
    }

    /// 返回 `path` 所在的文件夹；不在任何一个文件夹内时返回 `None`。
    pub fn folder_of(&self, path: &Path) -> Option<&Path> {
        self.folders().find(|folder| path.starts_with(folder))
    }

    /// 返回 `path` 在快速打开和全局搜索中显示的相对路径：以 `/` 分隔，附加的文件夹中的以它的
    /// 前缀开头；不在任何一个文件夹内时返回完整路径。
    pub fn labelled_path(&self, path: &Path) -> String {
        let prefixes = folder_prefixes(&self.root, &self.extra);
        let extra: Vec<_> = prefixes
            .into_iter()
            .zip(self.extra.iter().cloned())
            .collect();
        labelled_path(&self.root, &extra, path)
    }

    /// 返回 `path` 所在的文件夹，不在任何一个文件夹内时返回根目录。
    pub(super) fn folder_or_root(&self, path: &Path) -> &Path {
        self.folder_of(path).unwrap_or(&self.root)
    }

    /// `path` 是根目录或附加的文件夹本身时返回 `InvalidInput`：它们不能改名、移动。
    pub(super) fn refuse_folder(&self, path: &Path) -> io::Result<()> {
        if self.folders().any(|folder| folder == path) {
            let reason = t!("workspace-tree-cannot-move");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, reason));
        }
        Ok(())
    }

    /// `path` 是否为在文件树中占一行的文件夹：有附加的文件夹时的根目录和各个附加的文件夹。
    pub fn is_folder_row(&self, path: &Path) -> bool {
        !self.extra.is_empty() && self.folders().any(|folder| folder == path)
    }

    /// 目录 `path` 是否可以加为附加的文件夹：是可读的目录，不与已有的文件夹重叠，并与根目录在
    /// 同一个系统上。
    pub fn can_add_folder(&self, path: &Path) -> bool {
        let Ok(path) = winc_config::files::canonicalize(path) else {
            return false;
        };
        let overlaps = |folder: &Path| path.starts_with(folder) || folder.starts_with(&path);
        let place = |path: &Path| winc_backend::locate(path).map(|at| at.target);
        winc_backend::fs::is_dir(&path)
            && !self.folders().any(overlaps)
            && place(&path) == place(&self.root)
    }

    /// 把目录 `path` 加为附加的文件夹并展开，返回规范化后的路径。
    ///
    /// # Errors
    ///
    /// `path` 不是可读的目录时返回对应的 `std::io::Error`；已在某个文件夹之内、包含某个文件夹，
    /// 或与根目录不在同一个系统（本机、同一个连接）时返回 `InvalidInput`。
    pub fn add_folder(&mut self, path: &Path) -> io::Result<PathBuf> {
        let path = winc_config::files::canonicalize(path)?;
        list_dir(&path)?;
        let overlaps = |folder: &Path| path.starts_with(folder) || folder.starts_with(&path);
        let place = |path: &Path| winc_backend::locate(path).map(|at| at.target);
        if self.folders().any(overlaps) || place(&path) != place(&self.root) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.expanded.insert(path.clone());
        self.extra.push(path.clone());
        self.rebuild();
        Ok(path)
    }

    /// 移除附加的文件夹 `path`，返回是否移除了；根目录不能移除。
    ///
    /// 只改变工作区，不改动磁盘上的内容。
    pub fn remove_folder(&mut self, path: &Path) -> bool {
        let Some(at) = self.extra.iter().position(|folder| folder == path) else {
            return false;
        };
        self.extra.remove(at);
        self.expanded.retain(|dir| !dir.starts_with(path));
        self.children.retain(|dir, _| !dir.starts_with(path));
        self.marked.retain(|item| !item.starts_with(path));
        if self
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.starts_with(path))
        {
            self.anchor = None;
        }
        // 只剩根目录时它不再占一行，内容直接列出；名字属于这组目录，随之取消。
        if self.extra.is_empty() {
            self.expanded.insert(self.root.clone());
            self.set_alias(None);
        }
        self.rebuild();
        true
    }

    /// 返回用户给这组目录起的名字。
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// 给这组目录起名为 `alias`，显示名随之改变；`None` 或空白时恢复为根目录的名字。
    pub fn set_alias(&mut self, alias: Option<&str>) {
        let alias = alias.map(str::trim).filter(|alias| !alias.is_empty());
        self.alias = alias.map(str::to_owned);
        self.name = (self.alias.clone()).unwrap_or_else(|| self.own_name.clone());
    }

    /// 返回文件树最上一层的各个文件夹；没有附加的文件夹时返回 `None`。
    pub(super) fn folder_rows(&self) -> Option<Rc<[DirEntry]>> {
        if self.extra.is_empty() {
            return None;
        }
        // 同名的文件夹在名字后注明上级目录。
        let same = |folder: &Path| {
            let name = segment_name(folder);
            self.folders()
                .filter(|other| segment_name(other) == name)
                .count()
                > 1
        };
        let rows = self.folders().map(|folder| {
            let name = segment_name(folder);
            let parent = folder.parent().filter(|_| same(folder)).map(segment_name);
            DirEntry {
                path: folder.to_path_buf(),
                name: match parent {
                    Some(parent) => format!("{name} ({parent})"),
                    None => name.into_owned(),
                },
                kind: EntryKind::Dir,
                symlink: false,
            }
        });
        Some(rows.collect())
    }
}

/// 返回 `path` 相对于根目录 `root` 的路径，以 `/` 分隔；在附加目录中时相对于该目录，前面加上
/// 它的前缀（`extra` 是各个附加目录的前缀和目录）。不在任何一个目录中时返回完整路径。
pub(crate) fn labelled_path(root: &Path, extra: &[(String, PathBuf)], path: &Path) -> String {
    let folder = extra.iter().find(|(_, folder)| path.starts_with(folder));
    let relative = path.strip_prefix(folder.map_or(root, |(_, folder)| folder.as_path()));
    let label = folder.map(|(label, _)| Cow::Borrowed(label.as_str()));
    (label.into_iter())
        .chain((relative.unwrap_or(path).components()).map(|c| c.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/")
}

/// 返回各个附加目录 `extra` 在相对路径中使用的前缀，与 `extra` 一一对应：快速打开的文件表和全局
/// 搜索的结果用它标明文件所在的目录。
///
/// 前缀是目录名；与根目录 `root` 下的同名项或其他前缀相同时加上上级目录的名字，仍相同时改为
/// 加序号。
pub(crate) fn folder_prefixes(root: &Path, extra: &[PathBuf]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::with_capacity(extra.len());
    for folder in extra {
        let name = segment_name(folder).into_owned();
        let taken = |label: &str, labels: &[String]| {
            labels.iter().any(|other| other == label) || winc_backend::fs::exists(root.join(label))
        };
        let mut label = name.clone();
        if taken(&label, &labels) {
            let parent = folder.parent().map(segment_name).unwrap_or_default();
            label = format!("{name} ({parent})");
        }
        let mut n = 2;
        while taken(&label, &labels) {
            label = format!("{name} ({n})");
            n += 1;
        }
        labels.push(label);
    }
    labels
}
