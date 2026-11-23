//! 在后台读好展开目录、定位文件要用到的目录。
//!
//! 界面线程上先问要读哪些（[`Workspace::fetch_to_expand`]、[`Workspace::fetch_to_reveal`]），读
//! 交给后台（[`DirFetch::run`]），读好后放进缓存（[`Workspace::adopt`]）再展开或定位：这一步
//! 只用缓存，不访问磁盘。

use super::*;

/// 要在后台读的目录，见 [`Workspace::fetch_to_expand`]、[`Workspace::fetch_to_reveal`]。
#[derive(Debug)]
pub struct DirFetch {
    root: PathBuf,
    /// 先读的目录；读到的子目录中已展开的（`expanded`）继续读，它们随后也要显示。
    dirs: Vec<PathBuf>,
    expanded: HashSet<PathBuf>,
    /// 是否继续读唯一的子目录：展开时它们合并显示为一行。
    chain: bool,
    /// 递归展开时最多找的目录数和目录项数；`None` 为不递归。
    limits: Option<(usize, usize)>,
}

/// 后台读好的目录内容，见 [`DirFetch::run`]，交给 [`Workspace::adopt`]。
#[derive(Debug)]
pub struct DirFetched {
    root: PathBuf,
    /// 读到的各目录的内容；读不出来的不在其中。
    listed: Vec<(PathBuf, Vec<DirEntry>)>,
    /// 递归展开时要展开的各级目录，见 [`Workspace::expand_dirs`]。
    expand: Vec<PathBuf>,
    /// 递归展开时是否因目录太多只找了一部分。
    limited: bool,
}

/// 递归展开的范围，见 [`Workspace::expand_dirs`]。
#[derive(Debug, Default)]
pub struct Expansion {
    /// 要展开的各级目录；为空时只展开那一行本身。
    pub dirs: Vec<PathBuf>,
    /// 是否因目录太多只展开了一部分（见 [`EXPAND_DIRS`]）。
    pub limited: bool,
}

impl DirFetch {
    /// 读出各目录的内容；递归展开时同时找出要展开的各级目录。
    ///
    /// 访问磁盘，耗时没有上限，在后台线程上调用。
    pub fn run(self) -> DirFetched {
        let mut fetched = DirFetched {
            root: self.root,
            listed: Vec::new(),
            expand: Vec::new(),
            limited: false,
        };
        let mut queue = self.dirs;
        if let Some((dirs, entries)) = self.limits {
            for dir in queue.clone() {
                let (found, limited) = descend(&dir, dirs, entries);
                fetched.limited |= limited;
                queue.extend(found.iter().cloned());
                fetched.expand.extend(found);
            }
        }

        let mut seen: HashSet<PathBuf> = HashSet::new();
        while let Some(dir) = queue.pop() {
            if !seen.insert(dir.clone()) {
                continue;
            }
            let Ok(children) = list_dir(&dir) else {
                continue;
            };
            let sole = sole_folder(&children).filter(|_| self.chain);
            queue.extend(sole.map(|only| only.path.clone()));
            let open = |child: &&DirEntry| {
                child.kind == EntryKind::Dir && self.expanded.contains(&child.path)
            };
            queue.extend(children.iter().filter(open).map(|child| child.path.clone()));
            fetched.listed.push((dir, children));
        }
        fetched
    }
}

/// 返回递归展开 `path` 时要展开的各级目录（含它自己），以及是否因超过上限只找了一部分。
///
/// 遵守忽略规则，不进入符号链接；`path` 本身被忽略时不往下找，返回空。
fn descend(path: &Path, dirs: usize, entries: usize) -> (Vec<PathBuf>, bool) {
    let listed = path.parent().is_some_and(|parent| {
        walker(parent)
            .max_depth(Some(1))
            .build()
            .filter_map(Result::ok)
            .any(|e| e.path() == path)
    });
    if !listed {
        return (Vec::new(), false);
    }

    let mut walk = walker(path);
    walk.sort_by_file_name(|a, b| a.cmp(b));
    let (mut found, mut seen) = (Vec::new(), 0);
    for item in walk.build().filter_map(Result::ok) {
        seen += 1;
        let dir = item.file_type().is_some_and(|kind| kind.is_dir());
        if seen > entries || (dir && found.len() == dirs) {
            return (found, true);
        }
        if dir {
            found.push(item.into_path());
        }
    }
    (found, false)
}

impl Workspace {
    /// 返回展开第 `index` 行之前要在后台读的目录；`all` 为真时是递归展开。
    ///
    /// 这一行不是目录、下标越界，或不递归而它的内容已经读过时返回 `None`：直接展开即可。远端
    /// 的目录不递归时也返回 `None`，展开后由 [`Workspace::take_missing`] 补入内容。
    pub fn fetch_to_expand(&self, index: usize, all: bool) -> Option<DirFetch> {
        self.fetch_within(index, all.then_some((EXPAND_DIRS, EXPAND_ENTRIES)))
    }

    /// 同 [`Self::fetch_to_expand`]；`limits` 是递归展开的目录数和目录项数上限，`None` 为不递归。
    pub(super) fn fetch_within(
        &self,
        index: usize,
        limits: Option<(usize, usize)>,
    ) -> Option<DirFetch> {
        let entry = self.rows.get(index).filter(|e| e.kind == EntryKind::Dir)?;
        let known = self.children.contains_key(&entry.path);
        if limits.is_none() && (known || winc_backend::is_remote(&self.root)) {
            return None;
        }
        Some(DirFetch {
            root: self.root.clone(),
            dirs: vec![entry.path.clone()],
            expanded: self.expanded.clone(),
            chain: self.compact,
            limits,
        })
    }

    /// 返回在树中定位 `path` 之前要在后台读的目录：它的各级上级目录中还没有读过的。
    ///
    /// 都已读过、`path` 不在工作区内，或工作区在远端（定位后由 [`Workspace::take_missing`] 补入
    /// 内容）时返回 `None`：直接调用 [`Workspace::reveal`] 即可。
    pub fn fetch_to_reveal(&self, path: &Path) -> Option<DirFetch> {
        if winc_backend::is_remote(&self.root) {
            return None;
        }
        let folder = self.folder_of(path)?;
        let dirs: Vec<PathBuf> = (path.ancestors().skip(1))
            .take_while(|dir| dir.starts_with(folder))
            .filter(|dir| !self.children.contains_key(*dir))
            .map(Path::to_path_buf)
            .collect();
        (!dirs.is_empty()).then(|| DirFetch {
            root: self.root.clone(),
            dirs,
            expanded: self.expanded.clone(),
            chain: false,
            limits: None,
        })
    }

    /// 把后台读好的目录内容 `fetched` 放进缓存，返回递归展开的范围；不改可见的行。
    ///
    /// 已有缓存的目录保留原来的内容：它由文件监视保持最新。读的是另一个工作区（期间换了目录）
    /// 时什么也不做，返回 `None`。
    pub fn adopt(&mut self, fetched: DirFetched) -> Option<Expansion> {
        if fetched.root != self.root {
            return None;
        }
        for (dir, children) in fetched.listed {
            self.children.entry(dir).or_insert_with(|| children.into());
        }
        Some(Expansion {
            dirs: fetched.expand,
            limited: fetched.limited,
        })
    }

    /// 展开第 `index` 行的目录和 `dirs` 中的各级目录（[`Expansion::dirs`]）；`dirs` 为空时只展开
    /// 这一行。文件行和越界下标不做任何事。
    ///
    /// 各目录的内容已由 [`Workspace::adopt`] 放进缓存时不访问磁盘。
    pub fn expand_dirs(&mut self, index: usize, dirs: Vec<PathBuf>) {
        let Some(entry) = self.rows.get(index).filter(|e| e.kind == EntryKind::Dir) else {
            return;
        };
        match dirs.is_empty() {
            true => self.expand_chain(entry.path.clone()),
            false => self.expanded.extend(dirs),
        }
        self.rebuild();
    }
}
