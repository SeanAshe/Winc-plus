//! 附加的文件夹：一个工作区在根目录之外再显示的目录。
//!
//! 根目录决定工作区的名称和所在的系统；附加的文件夹只参与浏览和查找。有附加的文件夹时，文件树
//! 最上一层是各个文件夹（根目录在前），各自可以收起。
//!
//! 文件夹按打开时给的路径记录，经过符号链接时保留链接；判断是否为同一个、是否重叠时按真实路径。

use super::*;

/// 工作区中写法与真实路径不同的文件夹（经符号链接打开等）：各项是（真实路径，打开时的路径）。
///
/// 文件树中的各项都在打开时的路径下面；规范化后的路径（文档、外部程序给出的路径）经
/// [`FolderLinks::logical`] 换回同一写法，才能与文件树中的项对应。可以复制给后台任务使用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderLinks(pub(super) Vec<(PathBuf, PathBuf)>);

impl FolderLinks {
    /// 把落在某个文件夹真实路径下的 `path` 换成该文件夹打开时的写法；不在其中时返回 `None`。
    ///
    /// 只比较路径，不访问文件系统。
    pub fn logical(&self, path: &Path) -> Option<PathBuf> {
        let (real, given) = (self.0.iter()).find(|(real, _)| path.starts_with(real))?;
        let rest = path.strip_prefix(real).ok()?;
        Some(match rest.as_os_str().is_empty() {
            true => given.clone(),
            false => given.join(rest),
        })
    }

    /// 返回按 `given` 打开的文件夹的真实路径；`given` 不是写法与真实路径不同的文件夹时返回
    /// `None`。
    pub fn real_of(&self, given: &Path) -> Option<&Path> {
        let link = self.0.iter().find(|(_, other)| other == given);
        link.map(|(real, _)| real.as_path())
    }

    /// 返回 `path` 作为文档路径的写法：规范化（见 [`crate::canonical_path`]），落在经符号链接
    /// 打开的文件夹中时换回打开时的写法。
    ///
    /// 访问文件系统（解开符号链接），耗时与路径的层数有关。
    pub fn document_path(&self, path: &Path) -> PathBuf {
        let canonical = crate::canonical_path(path);
        self.logical(&canonical).unwrap_or(canonical)
    }
}

/// 返回打开文件夹 `path` 时记录的路径，以及与之不同时的真实路径。
///
/// 记录的写法见 [`crate::given_path`]；远端主机上的路径原样使用。
///
/// # Errors
///
/// 本机的 `path` 不存在或无法读取时返回对应的 `std::io::Error`。
pub(super) fn folder_path(path: &Path) -> io::Result<(PathBuf, Option<PathBuf>)> {
    if winc_backend::is_remote(path) {
        return Ok((path.to_path_buf(), None));
    }
    let real = winc_config::files::canonicalize(path)?;
    let given = crate::given_path(path);
    Ok(match given == real {
        true => (given, None),
        false => (given, Some(real)),
    })
}

impl Workspace {
    /// 返回附加的文件夹，按添加的顺序。
    pub fn extra_folders(&self) -> &[PathBuf] {
        &self.extra
    }

    /// 返回工作区的全部文件夹：根目录在前，其后是附加的文件夹。
    pub fn folders(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.root.as_path()).chain(self.extra.iter().map(PathBuf::as_path))
    }

    /// 返回写法与真实路径不同的文件夹，见 [`FolderLinks`]。
    pub fn links(&self) -> &FolderLinks {
        &self.links
    }

    /// 返回文件夹 `folder` 的真实路径：经符号链接打开的取链接指向的目录，其余原样返回。
    fn real_folder<'a>(&'a self, folder: &'a Path) -> &'a Path {
        let link = self.links.0.iter().find(|(_, given)| given == folder);
        link.map_or(folder, |(real, _)| real.as_path())
    }

    /// 真实路径为 `real` 的目录是否与已有的某个文件夹重叠（相同、包含或被包含）。
    fn overlaps(&self, real: &Path) -> bool {
        self.folders().any(|folder| {
            let folder = self.real_folder(folder);
            real.starts_with(folder) || folder.starts_with(real)
        })
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
        let Ok((path, real)) = folder_path(path) else {
            return false;
        };
        let place = |path: &Path| winc_backend::locate(path).map(|at| at.target);
        winc_backend::fs::is_dir(&path)
            && !self.overlaps(real.as_deref().unwrap_or(&path))
            && place(&path) == place(&self.root)
    }

    /// 把目录 `path` 加为附加的文件夹并展开，返回记录的路径（见 [`crate::given_path`]）。
    ///
    /// # Errors
    ///
    /// `path` 不是可读的目录时返回对应的 `std::io::Error`；已在某个文件夹之内、包含某个文件夹，
    /// 或与根目录不在同一个系统（本机、同一个连接）时返回 `InvalidInput`。远端目录在界面线程上
    /// 无法读取（`WouldBlock`），先加入，内容在后台读取。
    pub fn add_folder(&mut self, path: &Path) -> io::Result<PathBuf> {
        let (path, real) = folder_path(path)?;
        if let Err(e) = list_dir(&path)
            && e.kind() != io::ErrorKind::WouldBlock
        {
            return Err(e);
        }
        let place = |path: &Path| winc_backend::locate(path).map(|at| at.target);
        if self.overlaps(real.as_deref().unwrap_or(&path)) || place(&path) != place(&self.root) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.expanded.insert(path.clone());
        self.extra.push(path.clone());
        self.links.0.extend(real.map(|real| (real, path.clone())));
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
        self.links.0.retain(|(_, given)| given != path);
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
        // 只剩根目录时它不再占一行，内容直接列出；名字属于这组目录，随之取消；有项目文件时保留。
        if self.extra.is_empty() && self.project.is_none() {
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

    /// 返回这组目录所属的项目文件。
    pub fn project(&self) -> Option<&Path> {
        self.project.as_deref()
    }

    /// 记下这组目录所属的项目文件 `file`，显示名随之改为项目文件的名字；`None` 时取消。
    pub fn set_project(&mut self, file: Option<PathBuf>) {
        let stem = file.as_deref().and_then(Path::file_stem);
        let name = stem.map(|stem| stem.to_string_lossy().into_owned());
        self.set_alias(name.as_deref());
        self.project = file;
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
