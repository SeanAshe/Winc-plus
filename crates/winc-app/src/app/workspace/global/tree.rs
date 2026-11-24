//! 搜索结果以树形显示：文件按所在目录排成树（同文件视图，只有一个子目录的几级合成一行），目录
//! 可以折叠；以及列表、树形两种显示方式的切换。
//!
//! 树按结果的版本排一次，记下全部展开时的顺序；折叠只是生成结果行时跳过收起的目录和文件，目录的
//! 下标因此不随折叠变化，折叠的动画可以沿用旧的结果行。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use winc_editor::FileResult;

use super::*;
use crate::app::git::log::LogFileRow;

/// 结果的显示方式（见 [`GlobalSearch::layout`]）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ResultLayout {
    /// 按目录排成树；`false` 时每个文件一行，后面淡色写所在目录。
    pub tree: bool,
    /// 树里只有一个子目录的几级合成一行（同文件视图的设置）。
    pub compact: bool,
}

/// 结果按目录排成的树。
#[derive(Debug, Default)]
pub(crate) struct ResultTree {
    /// 各目录，[`ResultRow::Dir`] 的参数是这里的下标。
    pub(crate) dirs: Vec<ResultDir>,
    /// 全部展开时各目录、文件的顺序。
    order: Vec<TreeItem>,
    /// 各文件（结果的下标）在树里的层数。
    depths: Vec<usize>,
}

/// 树里的一个目录。
#[derive(Debug)]
pub(crate) struct ResultDir {
    /// 绝对路径：折叠和选中都按它记，结果陆续送达、重新排树以后仍然认得。
    pub(crate) path: PathBuf,
    /// 显示的名字：合成一行的几级用路径分隔符连起来。
    pub(crate) label: String,
    /// 缩进的层数。
    pub(crate) depth: usize,
    /// 目录里（含各级子目录）的文件，结果的下标。
    pub(crate) files: Vec<usize>,
    /// 目录里（含各级子目录）的匹配总数，行右端的计数。
    pub(crate) matches: usize,
    /// 全部展开时这个目录之后第一个不在它里面的位置（`order` 的下标）：收起时从这里接着排。
    end: usize,
}

/// 全部展开时的一项。
#[derive(Debug, Clone, Copy)]
enum TreeItem {
    Dir(usize),
    File(usize),
}

impl ResultTree {
    /// 把结果 `results` 按相对路径排成树。
    fn new(results: &[FileResult], compact: bool) -> ResultTree {
        let items = (results.iter().enumerate())
            .map(|(f, file)| (f, Path::new(file.relative.as_str())))
            .collect();
        let rows = crate::app::git::log::path_tree_rows(items, compact, &HashSet::new());
        let mut tree = ResultTree {
            depths: vec![0; results.len()],
            ..ResultTree::default()
        };
        // 正在排的各级目录（`dirs` 的下标）：遇到更浅的一行时，比它深的目录都排完了。
        let mut open: Vec<usize> = Vec::new();
        for row in rows {
            let depth = match &row {
                LogFileRow::Dir { depth, .. } | LogFileRow::File { depth, .. } => *depth,
            };
            for d in open.drain(depth.min(open.len())..) {
                tree.dirs[d].end = tree.order.len();
            }
            match row {
                LogFileRow::Dir {
                    dir, label, depth, ..
                } => {
                    let d = tree.dirs.len();
                    tree.dirs.push(ResultDir {
                        // 目录的绝对路径在排进第一个文件时按它的路径推出来（见下）。
                        path: PathBuf::from(dir),
                        label,
                        depth,
                        files: Vec::new(),
                        matches: 0,
                        end: 0,
                    });
                    tree.order.push(TreeItem::Dir(d));
                    open.push(d);
                }
                LogFileRow::File { index: f, depth } => {
                    let file = &results[f];
                    let parts = Path::new(&file.relative).components().count();
                    for &d in &open {
                        let dir = &mut tree.dirs[d];
                        if dir.files.is_empty() {
                            // 目录是文件相对路径的前几级：从文件的绝对路径往上退相差的级数。
                            let up = parts.saturating_sub(dir.path.components().count());
                            let at = file.path.ancestors().nth(up);
                            dir.path = at.map_or_else(|| dir.path.clone(), Path::to_path_buf);
                        }
                        dir.files.push(f);
                        dir.matches += file.matches.len();
                    }
                    tree.depths[f] = depth;
                    tree.order.push(TreeItem::File(f));
                }
            }
        }
        for d in open {
            tree.dirs[d].end = tree.order.len();
        }
        tree
    }

    /// 第 `f` 个文件在树里的层数。
    pub(crate) fn depth(&self, f: usize) -> usize {
        self.depths.get(f).copied().unwrap_or(0)
    }
}

impl GlobalSearch {
    /// 按树排出结果行：收起的目录只有它自己一行，收起的文件不列出匹配。
    pub(super) fn push_tree_rows(&mut self) {
        let Some(tree) = &self.tree else {
            return;
        };
        let mut i = 0;
        while let Some(&item) = tree.order.get(i) {
            i += 1;
            match item {
                TreeItem::Dir(d) => {
                    self.rows.push(ResultRow::Dir(d));
                    if self.collapsed.contains(&tree.dirs[d].path) {
                        i = tree.dirs[d].end;
                    }
                }
                TreeItem::File(f) => {
                    self.rows.push(ResultRow::File(f));
                    let file = &self.engine.results[f];
                    if !self.collapsed.contains(&file.path) {
                        let matches = (0..file.matches.len()).map(|m| ResultRow::Match(f, m));
                        self.rows.extend(matches);
                    }
                }
            }
        }
    }

    /// 按现在的显示方式和结果准备好树：以列表显示时不要树。
    pub(super) fn sync_tree(&mut self) {
        if !self.layout.tree {
            self.tree = None;
            return;
        }
        self.tree = Some(ResultTree::new(&self.engine.results, self.layout.compact));
    }

    /// 第 `d` 个目录（以树形显示时）。
    pub(crate) fn dir(&self, d: usize) -> Option<&ResultDir> {
        self.tree.as_ref()?.dirs.get(d)
    }

    /// 结果行 `row` 缩进的层数：列表里文件为 0；匹配比所在的文件深一层。
    pub(crate) fn row_depth(&self, row: ResultRow) -> usize {
        let file = |f| self.tree.as_ref().map_or(0, |t| t.depth(f));
        match row {
            ResultRow::Dir(d) => self.dir(d).map_or(0, |d| d.depth),
            ResultRow::File(f) => file(f),
            ResultRow::Match(f, _) => file(f) + 1,
        }
    }

    /// 结果行 `row` 对应的路径：目录行是目录，文件、匹配行是文件。
    pub(crate) fn row_path(&self, row: ResultRow) -> Option<&Path> {
        match row {
            ResultRow::Dir(d) => Some(self.dir(d)?.path.as_path()),
            ResultRow::File(f) | ResultRow::Match(f, _) => {
                Some(self.engine.results.get(f)?.path.as_path())
            }
        }
    }

    /// 结果行 `row` 包含的文件（结果的下标）：目录行是其中各级的文件，文件、匹配行是所在的文件。
    pub(crate) fn row_files(&self, row: ResultRow) -> Vec<usize> {
        match row {
            ResultRow::Dir(d) => self.dir(d).map_or_else(Vec::new, |d| d.files.clone()),
            ResultRow::File(f) | ResultRow::Match(f, _) => vec![f],
        }
    }

    /// 第 `index` 个结果行的上一级：目录、文件行是所在的目录，匹配行是所在的文件；最上层的
    /// 没有上一级。
    pub(crate) fn parent_row(&self, index: usize) -> Option<usize> {
        let depth = self.row_depth(*self.rows.get(index)?);
        (0..index)
            .rev()
            .find(|&i| self.row_depth(self.rows[i]) < depth)
    }

    /// 以树形显示时第 `d` 个目录里的各级目录和文件的路径（不含它自己）：按住 ⌥ 一起折叠、展开。
    fn inner_paths(&self, d: usize) -> Vec<PathBuf> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let start = (tree.order.iter()).position(|i| matches!(i, TreeItem::Dir(x) if *x == d));
        let Some(start) = start else {
            return Vec::new();
        };
        tree.order[start + 1..tree.dirs[d].end]
            .iter()
            .map(|item| match *item {
                TreeItem::Dir(x) => tree.dirs[x].path.clone(),
                TreeItem::File(f) => self.engine.results[f].path.clone(),
            })
            .collect()
    }
}

impl App {
    /// 结果按目录排成树（`true`）还是每个文件一行：记在状态文件里，所有窗口共用。
    pub(crate) fn search_tree(&self) -> bool {
        self.state.search_tree
    }

    /// 结果现在应该怎么显示：显示方式和文件视图「合并只有一个子目录的目录」的设置。
    fn result_layout(&self) -> ResultLayout {
        ResultLayout {
            tree: self.search_tree(),
            compact: self.config.compact_folders != Some(false),
        }
    }

    /// 绘制前让结果行跟上结果、显示方式和设置的变化。
    pub(crate) fn sync_result_rows(&mut self) {
        self.panels.global.layout = self.result_layout();
        self.panels.global.sync_rows();
    }

    /// 在树形和列表之间切换结果（标题行的按钮），记进状态文件；选中的行仍然选中、露出来。
    pub(in crate::app) fn toggle_search_tree(&mut self) {
        let tree = !self.search_tree();
        self.update_state(move |state| state.search_tree = tree);
        let g = &mut self.panels.global;
        g.overflow = None;
        // 目录是树形时才有的：切到列表以后选中它所在的第一个文件。
        if !tree
            && let Some(row) = g.selected_row().map(|i| g.rows()[i])
            && let ResultRow::Dir(d) = row
        {
            let first = g.dir(d).and_then(|d| d.files.first().copied());
            g.selected = first.map(|f| (g.engine.results[f].path.clone(), None));
        }
        self.sync_result_rows();
        if let Some(index) = self.panels.global.selected_row() {
            self.scroll_result_into_view(index);
        }
        self.clamp_results_scroll();
        self.request_redraw();
    }

    /// 折叠（`collapse`）或展开第 `index` 个结果行（目录或文件），并播放动画：其中的行从这一行
    /// 下方逐渐露出或收回，下方的行随之移动。`deep` 时其中各级目录和文件一起折叠或展开（按住 ⌥）。
    pub(in crate::app) fn fold_result_row(&mut self, index: usize, collapse: bool, deep: bool) {
        let g = &mut self.panels.global;
        let Some(row) = g.rows().get(index).copied() else {
            return;
        };
        let Some(path) = g.row_path(row).map(Path::to_path_buf) else {
            return;
        };
        let mut paths = vec![path];
        if let (true, ResultRow::Dir(d)) = (deep, row) {
            paths.extend(g.inner_paths(d));
        }
        let mut changed = false;
        for path in paths {
            changed |= if collapse {
                g.collapsed.insert(path)
            } else {
                g.collapsed.remove(&path)
            };
        }
        if !changed {
            return;
        }
        let before = std::mem::take(&mut g.rows);
        g.rebuild_rows();
        let clock = self.clock(TREE_ANIMATION);
        let g = &mut self.panels.global;
        let count = before.len().abs_diff(g.rows.len());
        // 动画期间按行数较多的状态绘制：展开时绘制展开后的行，折叠时绘制折叠前的行。
        let rows = if collapse { before } else { g.rows.clone() };
        let parent = rows.iter().position(|&r| r == row);
        g.motion = match (parent, clock) {
            (Some(parent), Some(clock)) if count > 0 => Some(ResultMotion {
                rows,
                parent,
                count,
                expanding: !collapse,
                clock,
            }),
            _ => None,
        };
        self.clamp_results_scroll();
    }

    /// 标题行的「全部折叠」：所有文件（以树形显示时还有目录）一起折叠，都已折叠时全部展开。
    ///
    /// 折叠时选中的行改为它最上一级的那一行：其余的都折进去了，看不见。
    pub(in crate::app) fn toggle_all_results(&mut self) {
        let g = &mut self.panels.global;
        let top = g.selected_row().map(|mut i| {
            while let Some(parent) = g.parent_row(i) {
                i = parent;
            }
            i
        });
        let top = top
            .and_then(|i| g.row_path(g.rows()[i]))
            .map(Path::to_path_buf);
        if g.all_collapsed() {
            g.collapsed.clear();
        } else {
            g.collapsed = g.foldable_paths().cloned().collect();
            g.selected = top.map(|path| (path, None));
        }
        g.rebuild_rows();
        g.scroll = 0.0;
    }

    /// 第 `index` 个结果行（目录或文件）下面是否接着它里面的行（展开着、里面有东西）。
    pub(in crate::app) fn result_row_has_children(&self, index: usize) -> bool {
        let g = &self.panels.global;
        let depth = |i: usize| g.rows().get(i).map(|&r| g.row_depth(r));
        matches!((depth(index), depth(index + 1)), (Some(a), Some(b)) if b > a)
    }

    /// 第 `index` 个结果行（目录或文件）是否收起。
    pub(in crate::app) fn result_row_collapsed(&self, index: usize) -> bool {
        let g = &self.panels.global;
        let path = g.rows().get(index).and_then(|&r| g.row_path(r));
        path.is_some_and(|p| g.collapsed.contains(p))
    }
}
