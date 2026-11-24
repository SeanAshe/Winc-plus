//! 详情栏：选中和复制文字、改动的文件（提示、菜单、打开）。

use super::*;
use crate::app::git::commit::{NaturalName, only_subdir};
use std::collections::BTreeMap;

/// 文件 `file` 的完整路径（相对仓库根目录 `root`），改过名时写成「原来的 → 现在的」：截短的
/// 文件行悬停时提示。
pub(crate) fn file_tip(file: &CommitFile, root: &Path) -> String {
    let rel = |p: &Path| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    };
    match &file.from {
        Some(from) => format!("{} → {}", rel(from), rel(&file.path)),
        None => rel(&file.path),
    }
}

/// 排好的文件各行的缓存：键见 [`RowsKey`]。
pub(crate) type FileRowsCache = Option<(RowsKey, Rc<[LogFileRow]>)>;

/// 排文件各行的依据：哪个列表（差异的文件栏为真）、哪组文件、文件数、排法、收起目录的版本。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowsKey {
    pub panel: bool,
    pub id: String,
    pub len: usize,
    pub shape: RowsShape,
    pub folds: u64,
}

/// 文件怎么排：平铺，文件历史查看的文件（第一个）是否排在最前，只有一个子目录的几级是否合成
/// 一行（同文件视图的设置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowsShape {
    pub flat: bool,
    pub pinned: bool,
    pub compact: bool,
}

/// 详情里改动的文件排成的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogFileRow {
    /// 一个目录：相对仓库根目录的路径、显示的名字（合起来的几级）、缩进的层数、是否展开。
    Dir {
        dir: String,
        label: String,
        depth: usize,
        open: bool,
    },
    /// 一个文件：在文件列表里的下标、缩进的层数。
    File { index: usize, depth: usize },
}

/// 文件列表、分支面板里一行名字的过渡状态的键（见 `TreeLabels`）：目录（相对仓库根目录）、
/// 文件，或分支面板的一行（分支、标签的全名，目录的路径）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RowName {
    Dir(String),
    File(PathBuf),
    Ref(String),
}

impl LogFileRow {
    /// 文件行的文件下标；目录行为 `None`。
    pub(crate) fn file(&self) -> Option<usize> {
        match self {
            LogFileRow::File { index, .. } => Some(*index),
            LogFileRow::Dir { .. } => None,
        }
    }
}

/// 一个文件列表里收起的目录：属于哪组文件（换了提交或比较就不算了）和收起的目录。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FileFolds {
    id: String,
    dirs: HashSet<String>,
    /// 每次收起、展开加一：排好的各行按它判断是否要重排。
    version: u64,
}

impl FileFolds {
    /// 收起、展开过几次。
    pub(super) fn version(&self) -> u64 {
        self.version
    }

    /// 文件组 `id` 里收起的目录。
    pub(super) fn of(&self, id: &str) -> Option<&HashSet<String>> {
        (self.id == id).then_some(&self.dirs)
    }

    /// 收起或展开文件组 `id` 里的目录 `dir`。
    pub(super) fn toggle(&mut self, id: &str, dir: &str) {
        if self.id != id {
            self.id = id.to_owned();
            self.dirs.clear();
        }
        if !self.dirs.remove(dir) {
            self.dirs.insert(dir.to_owned());
        }
        self.version += 1;
    }

    /// 展开文件组 `id` 里装着 `path`（相对仓库根目录）的各级目录。
    pub(super) fn reveal(&mut self, id: &str, path: &Path) {
        if self.id != id {
            return;
        }
        let before = self.dirs.len();
        self.dirs.retain(|dir| !path.starts_with(dir));
        if self.dirs.len() != before {
            self.version += 1;
        }
    }
}

/// 文件 `file` 相对仓库根目录 `root` 的路径；不在根目录下时只取文件名。
fn relative<'a>(root: Option<&Path>, file: &'a CommitFile) -> &'a Path {
    let path = &file.path;
    match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(rel) => rel,
        None => path.file_name().map_or(path.as_path(), Path::new),
    }
}

/// 改动的文件 `files` 怎么排：平铺时按原来的顺序一个文件一行；否则同文件视图排成树，先是目录、
/// 再是文件，各按名字排，`folded` 里的目录收起。`root` 是仓库的根目录。`pinned` 时第一个文件
/// （文件历史查看的文件）和装着它的目录排在各自那一层的最前。
pub(crate) fn log_file_rows(
    root: Option<&Path>,
    files: &[CommitFile],
    shape: RowsShape,
    folded: &HashSet<String>,
) -> Vec<LogFileRow> {
    if shape.flat {
        let file = |index| LogFileRow::File { index, depth: 0 };
        return (0..files.len()).map(file).collect();
    }
    let items = files
        .iter()
        .map(|f| relative(root, f))
        .enumerate()
        .collect();
    let mut rows = Vec::new();
    let tree = Tree { shape, folded };
    tree.push_level(&mut rows, items, Path::new(""), 0);
    rows
}

/// 把 `items`（下标、相对路径）同文件视图排成树：先是目录、再是文件，各按名字排；`compact` 时
/// 只有一个子目录的几级合成一行，`folded` 里的目录（相对路径）收起。侧栏搜索的树形结果也这样排。
pub(crate) fn path_tree_rows(
    items: Vec<(usize, &Path)>,
    compact: bool,
    folded: &HashSet<String>,
) -> Vec<LogFileRow> {
    let shape = RowsShape {
        flat: false,
        pinned: false,
        compact,
    };
    let mut rows = Vec::new();
    Tree { shape, folded }.push_level(&mut rows, items, Path::new(""), 0);
    rows
}

/// 排成树时各层共用的：排法和收起的目录。
struct Tree<'a> {
    shape: RowsShape,
    folded: &'a HashSet<String>,
}

impl Tree<'_> {
    /// 把 `dir` 下面的 `items`（下标、相对 `dir` 的路径）排成第 `depth` 层。
    fn push_level<'a>(
        &self,
        rows: &mut Vec<LogFileRow>,
        items: Vec<(usize, &'a Path)>,
        dir: &Path,
        depth: usize,
    ) {
        let mut files = Vec::new();
        let mut dirs: BTreeMap<NaturalName<'a>, Vec<(usize, &'a Path)>> = BTreeMap::new();
        for (index, path) in items {
            let mut parts = path.components();
            match (parts.next(), parts.as_path()) {
                (Some(first), rest) if !rest.as_os_str().is_empty() => {
                    let dir = dirs.entry(NaturalName(first.as_os_str())).or_default();
                    dir.push((index, rest));
                }
                _ => files.push((index, path)),
            }
        }
        let pinned = |items: &[(usize, &Path)]| {
            self.shape.pinned && items.iter().any(|&(index, _)| index == 0)
        };
        let mut dirs: Vec<_> = dirs.into_iter().collect();
        dirs.sort_by_key(|(_, sub)| !pinned(sub));
        for (NaturalName(name), mut sub) in dirs {
            let mut label = PathBuf::from(name);
            while let Some(next) = only_subdir(&sub).filter(|_| self.shape.compact) {
                label.push(next);
                for (_, p) in &mut sub {
                    let rest: &'a Path = p;
                    *p = rest.strip_prefix(next).unwrap_or(Path::new(""));
                }
            }
            let full = dir.join(&label);
            let key = full.to_string_lossy().into_owned();
            let open = !self.folded.contains(&key);
            // 合成一行的几级按 `/` 连接，同文件视图（`Entry::label`）：Windows 上也不写成 `\`。
            let parts: Vec<_> = label.iter().map(|part| part.to_string_lossy()).collect();
            rows.push(LogFileRow::Dir {
                dir: key,
                label: parts.join("/"),
                depth,
                open,
            });
            if open {
                self.push_level(rows, sub, &full, depth + 1);
            }
        }
        let pin = |index: usize| self.shape.pinned && index == 0;
        files.sort_by(|&(ia, a), &(ib, b)| {
            let name = |p: &Path| p.to_string_lossy().into_owned();
            (pin(ib).cmp(&pin(ia))).then_with(|| winc_editor::natural_cmp(&name(a), &name(b)))
        });
        let file = |&(index, _): &(usize, &Path)| LogFileRow::File { index, depth };
        rows.extend(files.iter().map(file));
    }
}

impl App {
    /// 绘制前记下详情一栏这一帧要画哪个提交（`oid`）：换成了另一个提交（不是从没有到有）时，
    /// 内容从四成不透明淡入（`DETAILS_FADE`）；动态效果关闭、没有窗口时不淡入。
    pub(crate) fn note_log_details_shown(&mut self, oid: Option<String>) {
        if oid == self.git.log.shown {
            return;
        }
        if self.git.log.shown.is_some() && oid.is_some() {
            self.git.log.fade = self.clock(crate::app::animation::DETAILS_FADE);
        }
        self.git.log.shown = oid;
    }

    /// 详情里 `(x, y)` 处的字的位置。
    ///
    /// `near` 为 `true` 时（拖着选的时候）不在任何一行上也取
    /// 最近的一行，拖到头尾以外时停在头尾；否则只认正落在某一行上的位置。
    pub(crate) fn log_text_at(&self, x: f32, y: f32, near: bool) -> Option<TextPoint> {
        let runs = &self.git.log.runs;
        // 同一行上可能有几段（作者、邮箱）：取开头在鼠标左边的最后一段。
        let on = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.row.y <= y && y < r.row.bottom())
            .filter(|(_, r)| r.x - 2.0 <= x)
            .map(|(i, _)| i)
            .next_back()
            .or_else(|| {
                runs.iter()
                    .position(|r| r.row.y <= y && y < r.row.bottom())
                    .filter(|_| near)
            });
        let run = match on {
            Some(i) => i,
            None if near => {
                let first = runs.first()?;
                if y < first.row.y {
                    return Some(TextPoint { run: 0, ch: 0 });
                }
                let last = runs.len() - 1;
                if y >= runs[last].row.bottom() {
                    return Some(TextPoint {
                        run: last,
                        ch: runs[last].edges.len(),
                    });
                }
                // 两行之间的空隙：算上面那一行的行尾。
                let above = runs.iter().rposition(|r| r.row.bottom() <= y)?;
                return Some(TextPoint {
                    run: above,
                    ch: runs[above].edges.len(),
                });
            }
            None => return None,
        };
        // 只在字上（不在行尾以外的空白里）才算按在字上；拖着的时候行尾以外算到行尾。
        let r = &runs[run];
        let end = r.edges.last().copied().unwrap_or(r.x);
        if !near && (x < r.x - 2.0 || x > end + 4.0) {
            return None;
        }
        Some(TextPoint {
            run,
            ch: r.index_at(x),
        })
    }

    /// 第 `run` 行选中了第几个字到第几个字（`len` 是这一行的字数）；这一行没选中时为 `None`。
    pub(crate) fn log_selected_chars(&self, run: usize, len: usize) -> Option<(usize, usize)> {
        let (a, b) = self.git.log.text_sel?;
        let (from, to) = (a.min(b), a.max(b));
        if run < from.run || run > to.run {
            return None;
        }
        let start = if run == from.run { from.ch } else { 0 };
        let end = if run == to.run { to.ch } else { len };
        (start < end).then_some((start, end))
    }

    /// 详情里选中的字；没选中时为 `None`。
    ///
    /// 一段话折成的几行直接连起来，不同的行之间换行；截短显示的邮箱在整行选中时给出完整内容。
    pub(crate) fn log_selected_text(&self) -> Option<String> {
        let (a, b) = self.git.log.text_sel?;
        let (from, to) = (a.min(b), a.max(b));
        let mut out = String::new();
        for run in from.run..=to.run.min(self.git.log.runs.len().saturating_sub(1)) {
            let r = self.git.log.runs.get(run)?;
            let len = r.edges.len();
            let (start, end) = self.log_selected_chars(run, len).unwrap_or((0, 0));
            if run > from.run && !r.soft {
                out.push('\n');
            }
            match &r.full {
                Some(full) if start == 0 && end == len => out.push_str(full),
                _ => out.extend(r.text.chars().skip(start).take(end - start)),
            }
        }
        (!out.trim().is_empty()).then_some(out)
    }

    /// 在详情里按下鼠标（没按在可以点的东西上）：按在字上开始选字，`count` 为 2（双击）时选中
    /// 这一整行；按在空白处清掉选中。
    pub(in crate::app) fn press_log_text(&mut self, x: f32, y: f32, count: u8) -> bool {
        self.focus = Focus::Editor;
        let Some(at) = self.log_text_at(x, y, false) else {
            self.git.log.text_sel = None;
            self.request_redraw();
            return false;
        };
        self.git.log.text_sel = Some(if count >= 2 {
            let len = self.git.log.runs[at.run].edges.len();
            (
                TextPoint { run: at.run, ch: 0 },
                TextPoint {
                    run: at.run,
                    ch: len,
                },
            )
        } else {
            (at, at)
        });
        self.request_redraw();
        count < 2
    }

    /// 拖着选字时鼠标到了 `(x, y)`。
    pub(in crate::app) fn drag_log_text(&mut self, x: f32, y: f32) {
        if let (Some((anchor, _)), Some(head)) =
            (self.git.log.text_sel, self.log_text_at(x, y, true))
        {
            self.git.log.text_sel = Some((anchor, head));
            self.request_redraw();
        }
    }

    /// 从第 `row` 行的线 `line` 顺着往下（往父提交的方向）走到的提交：这条线上的下一个提交，
    /// 或者这条线合进别的线时合进去的那个提交。
    ///
    /// 线从第 `row` 行自己的圆点出去时不算这一行。
    /// 还没取回来时为 `None`。
    pub(crate) fn log_line_target(&self, row: usize, line: u32) -> Option<usize> {
        let view = self.editor.active()?.log.as_deref()?;
        view.graph
            .iter()
            .enumerate()
            .skip(row)
            .find(|(j, g)| {
                if g.line == line {
                    *j > row
                } else {
                    g.up.iter().any(|s| s.line == line && s.to == g.lane)
                }
            })
            .map(|(j, _)| j)
    }

    /// 详情（或比较）里第 `i` 个改动的文件。
    pub(crate) fn log_file(&self, i: usize) -> Option<&CommitFile> {
        let view = self.editor.active()?.log.as_deref()?;
        if let Some(compare) = &view.compare {
            return compare.files.as_ref()?.get(i);
        }
        let oid = view.selected_oid()?;
        view.details
            .as_ref()
            .filter(|d| d.oid == oid)?
            .files
            .as_ref()?
            .get(i)
    }

    /// 详情里文件悬停提示的完整路径（相对仓库根目录）；改过名的写「旧路径 → 新路径」。
    pub(crate) fn log_file_tip(&self, i: usize) -> Option<String> {
        Some(file_tip(self.log_file(i)?, self.active_log_root()?))
    }

    /// 在日志详情里右键的位置：点着的文件（下标和路径）。
    ///
    /// 点着了就让键盘也选上它。
    pub(in crate::app) fn log_file_context_target(
        &mut self,
        x: f32,
        y: f32,
    ) -> Option<(usize, PathBuf)> {
        if !self.log_active() {
            return None;
        }
        let LogTarget::File(i) = crate::layout::hit(&self.hits.log, x, y)? else {
            return None;
        };
        let path = self.log_file(i)?.path.clone();
        self.focus = Focus::Editor;
        self.git.log.file = Some(i);
        Some((i, path))
    }

    /// 日志详情里一个文件的右键菜单命令。
    pub(in crate::app) fn run_log_file_command(
        &mut self,
        command: MenuCommand,
        i: usize,
        path: PathBuf,
    ) {
        match command {
            MenuCommand::GitShowDiff => self.open_log_file(i),
            MenuCommand::GitOpenFile => {
                if self.may_be_file(&path) {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                }
            }
            MenuCommand::FileHistory => self.git_file_history(&path),
            MenuCommand::LogCompareWithLocal => self.compare_log_file_with_local(path),
            MenuCommand::CopyPath | MenuCommand::CopyRelativePath => {
                self.copy_path(&path, command == MenuCommand::CopyRelativePath);
            }
            MenuCommand::Reveal => self.reveal_in_file_manager(&path),
            MenuCommand::RevealInTree => {
                self.show_files();
                self.reveal_path(&path);
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// 详情里列着的改动的文件（比较两个版本时是两者之间的）；还没取回来时为 `None`。
    fn log_shown_files(&self) -> Option<&[CommitFile]> {
        let view = self.editor.active().and_then(|d| d.log.as_deref())?;
        if let Some(compare) = &view.compare {
            return compare.files.as_deref();
        }
        let oid = view.selected_oid()?;
        let details = view.details.as_ref().filter(|d| d.oid == oid)?;
        details.files.as_deref()
    }

    /// 详情里改动的文件排成的各行（见 [`log_file_rows`]）。
    pub(crate) fn log_file_layout(&self) -> Rc<[LogFileRow]> {
        self.log_file_rows_of(self.log_shown_files().unwrap_or_default())
    }

    /// 按 `key` 取排好的文件各行；没有排过、或排的是别的文件时由 `build` 重排并记下。
    pub(super) fn cached_file_rows(
        &self,
        key: RowsKey,
        build: impl FnOnce() -> Vec<LogFileRow>,
    ) -> Rc<[LogFileRow]> {
        let mut cache = self.git.log.rows_cache.borrow_mut();
        if let Some((cached, rows)) = cache.as_ref()
            && *cached == key
        {
            return Rc::clone(rows);
        }
        let rows: Rc<[LogFileRow]> = build().into();
        *cache = Some((key, Rc::clone(&rows)));
        rows
    }

    /// 详情里改动的文件是否平铺：切换过时按这里记下的，否则同提交视图。
    pub(crate) fn log_details_flat(&self) -> bool {
        !self.state.log_files_tree.unwrap_or(self.state.commit_tree)
    }

    /// 在平铺与树之间切换详情里的文件，记进状态文件。
    pub(in crate::app) fn toggle_log_details_layout(&mut self) {
        let tree = self.log_details_flat();
        self.update_state(move |state| state.log_files_tree = Some(tree));
        self.request_redraw();
    }

    /// 文件列表里的名字本帧是否播放过渡；没有窗口、动态效果关闭时为假。
    pub(crate) fn animate_log_names(&mut self) -> bool {
        #[cfg(test)]
        if self.git.log.details_names.animate_without_window {
            return true;
        }
        self.clock(crate::app::status::motion::STATUS_SLIDE)
            .is_some()
    }

    /// 只有一个子目录的几级是否合成一行：同文件视图的设置。
    pub(super) fn log_files_compact(&self) -> bool {
        self.config.compact_folders != Some(false)
    }

    /// 详情里的文件属于哪组（选中的提交或比较的两个版本），以及怎么排：文件历史查看的文件
    /// 排在最前（比较两个版本时没有）。
    fn log_details_shape(&self) -> (String, RowsShape) {
        let view = self.editor.active().and_then(|d| d.log.as_deref());
        let pinned = view.is_some_and(|v| v.query.path.is_some() && v.compare.is_none());
        let id = view.map_or_else(String::new, |v| match &v.compare {
            Some(c) => format!("{}..{}", c.from, c.to),
            None => v
                .details
                .as_ref()
                .map_or_else(String::new, |d| d.oid.clone()),
        });
        let shape = RowsShape {
            flat: self.log_details_flat(),
            pinned,
            compact: self.log_files_compact(),
        };
        (id, shape)
    }

    /// 详情里文件 `files`（选中的提交或比较的文件）排成的各行，收起的目录里的文件不列出。
    pub(crate) fn log_file_rows_of(&self, files: &[CommitFile]) -> Rc<[LogFileRow]> {
        let (id, shape) = self.log_details_shape();
        let folds = &self.git.log.details_folds;
        let key = RowsKey {
            panel: false,
            id,
            len: files.len(),
            shape,
            folds: folds.version(),
        };
        let empty = HashSet::new();
        let folded = folds.of(&key.id).unwrap_or(&empty);
        let root = self.active_log_root();
        let build = || log_file_rows(root, files, shape, folded);
        self.cached_file_rows(key.clone(), build)
    }

    /// 收起或展开详情里第 `row` 行的目录 `dir`（相对仓库根目录），其中各行从这一行下方逐渐露出
    /// （或收回）；动态效果关闭时直接到位。
    pub(in crate::app) fn toggle_log_details_dir(&mut self, row: usize, dir: &str) {
        let (id, _) = self.log_details_shape();
        let before = self.log_file_layout();
        self.git.log.details_folds.toggle(&id, dir);
        let after = self.log_file_layout();
        self.git.log.details_fold = self.file_fold(row, before, after);
        self.request_redraw();
    }

    /// 文件列表第 `parent` 行的目录从 `before` 变成 `after` 时的动画；没有行要露出、收回或者
    /// 动态效果关闭时为 `None`。
    pub(super) fn file_fold(
        &mut self,
        parent: usize,
        before: Rc<[LogFileRow]>,
        after: Rc<[LogFileRow]>,
    ) -> Option<PanelFold<Rc<[LogFileRow]>>> {
        let expanding = after.len() > before.len();
        let count = after.len().abs_diff(before.len());
        let rows = if expanding { after } else { before };
        let clock = self.clock(crate::app::animation::TREE_ANIMATION)?;
        (count > 0).then_some(PanelFold {
            rows,
            parent,
            count,
            expanding,
            clock,
        })
    }

    /// 详情里露出来的各文件从上到下的顺序（文件列表里的下标）：键盘上下移动按它。
    pub(crate) fn log_file_order(&self) -> Vec<usize> {
        let rows = self.log_file_layout();
        rows.iter().filter_map(LogFileRow::file).collect()
    }

    /// 详情里排在最前的文件（收起的目录也算）：回车、双击提交时打开它。
    pub(in crate::app) fn log_first_file(&self) -> Option<usize> {
        let files = self.log_shown_files()?;
        let (_, shape) = self.log_details_shape();
        let rows = log_file_rows(self.active_log_root(), files, shape, &HashSet::new());
        rows.iter().find_map(LogFileRow::file)
    }

    /// 让详情里第 `i` 个文件露出来：按上一帧画出来的文件行推算它在哪，在详情一栏外面就滚。
    pub(super) fn reveal_log_file(&mut self, i: usize) {
        let area = self.log_parts().details;
        // 最下面一行小字（比较的提示）占 30 逻辑像素。
        let bottom = area.bottom() - 30.0;
        // 目录标题行和文件行一样高：按两个文件在各行里的位置之差推算。
        let rows = self.log_file_layout();
        let at = |i: usize| rows.iter().position(|r| r.file() == Some(i));
        let Some(top) = self.hits.log.iter().find_map(|(rect, t)| match t {
            LogTarget::File(j) => {
                let (to, from) = (at(i)?, at(*j)?);
                Some(rect.y + (to as f32 - from as f32) * LOG_FILE_ROW)
            }
            _ => None,
        }) else {
            return;
        };
        let Some(view) = self.editor.active_mut().and_then(|d| d.log.as_deref_mut()) else {
            return;
        };
        if top < area.y {
            view.details_scroll = (view.details_scroll - (area.y - top) - 8.0).max(0.0);
        } else if top + LOG_FILE_ROW > bottom {
            view.details_scroll += top + LOG_FILE_ROW - bottom + 8.0;
        }
    }

    /// 双击了第 `row` 个提交：打开它改动的第一个文件；文件还没取回来就等取回来再开。
    pub(in crate::app) fn open_log_row_first_file(&mut self, row: usize) {
        let Some(oid) = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref()?.commits.get(row).map(|c| c.oid.clone()))
        else {
            return;
        };
        if let Some(first) = self.log_first_file() {
            self.open_log_file(first);
        } else {
            self.git.log.open_first = Some(oid);
        }
    }
}
