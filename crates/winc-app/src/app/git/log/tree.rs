//! 从日志打开的差异左侧的文件栏：列出这个提交（或比较）改动的文件，点击在同一个差异标签中
//! 切换文件。
//!
//! 文件的排法和详情一栏相同（[`log_file_rows`]），收起的目录各记各的。侧栏的提交视图始终是
//! 工作区的改动。

use super::details::{RowsKey, RowsShape, log_file_rows};
use super::*;
use crate::app::diff::DIFF_FILES_MIN;
use crate::app::diff_parts;

/// 差异标签对应的文件列表：所属的提交或比较，以及当前打开的文件。
pub(crate) struct LogDiffTree {
    /// 差异所属的提交或比较，以及其中改动的文件。
    pub(super) source: Arc<LogDiffSource>,
    /// 当前打开的文件在 `source.files` 中的下标。
    pub(super) selected: usize,
}

/// 文件栏的默认宽度（逻辑像素）：文件名、目录和增删统计在一行里写得下。
const FILES_W: f32 = 280.0;
/// 拖动后文件栏的最大宽度（逻辑像素）；最小宽度见 [`DIFF_FILES_MIN`]。
const FILES_MAX: f32 = 560.0;
/// 文件栏里标题行的高度和它下面的间距（逻辑像素），与详情一栏的文件列表相同。
pub(crate) const FILES_TITLE: f32 = 26.0;

/// 文件栏的状态。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FilesPanel {
    /// 拖过的宽度（逻辑像素）；`None` 是默认宽度。
    pub width: Option<f32>,
    /// 用户展开（`false`）或收起了文件栏；`None` 时按文件数：两个以上展开。
    pub closed: Option<bool>,
    /// 列表滚动的距离（逻辑像素）。
    pub scroll: f32,
    /// 最大滚动距离（逻辑像素），绘制时按行数计算。
    pub max: f32,
    /// 目录写不下而截短、或改过名的文件（提交的文件列表中的下标）：悬停时提示完整路径。绘制时
    /// 更新。
    pub long: Vec<usize>,
    /// 收起的目录。
    pub(super) folds: FileFolds,
}

impl App {
    /// 在差异标签中打开当前文件列表中的第 `file` 个文件（在后台读取）。
    pub(in crate::app) fn open_log_diff_file(&mut self, file: usize) {
        if let Some(tree) = &self.git.log.tree {
            self.load_log_diff(Arc::clone(&tree.source), file);
        }
    }

    /// 记录差异标签显示的是 `source` 的第 `file` 个文件；换了提交时文件栏回到顶部。
    pub(super) fn select_log_diff(&mut self, source: Arc<LogDiffSource>, file: usize) {
        let keep = (self.git.log.tree.as_ref())
            .is_some_and(|tree| Arc::ptr_eq(&tree.source, &source) || tree.source == source);
        if !keep {
            self.git.log.files.scroll = 0.0;
        }
        // 正在看的文件所在的目录收起了：展开。
        if let Some(path) = source.files.get(file).map(|f| &f.path) {
            let rel = path.strip_prefix(&source.root).unwrap_or(path);
            self.git.log.files.folds.reveal(&source.id(), rel);
        }
        self.git.log.tree = Some(LogDiffTree {
            source,
            selected: file,
        });
    }

    /// 返回按文件栏的顺序（收起的目录也算），与当前文件相邻的文件：（下标，在这个顺序中的
    /// 位置）；`forward` 为 `true` 时取下一个，没有时返回 `None`。
    pub(super) fn log_diff_neighbor(&self, forward: bool) -> Option<(usize, usize)> {
        let tree = self.git.log.tree.as_ref()?;
        let source = &tree.source;
        let rows = log_file_rows(
            Some(&source.root),
            &source.files,
            self.diff_files_shape(),
            &HashSet::new(),
        );
        let order: Vec<usize> = rows.iter().filter_map(LogFileRow::file).collect();
        let at = order.iter().position(|&i| i == tree.selected)?;
        let next = if forward { at + 1 } else { at.checked_sub(1)? };
        order.get(next).map(|&i| (i, next))
    }

    /// 文件栏里文件的排法：不把哪个文件排在最前。
    fn diff_files_shape(&self) -> RowsShape {
        RowsShape {
            flat: self.diff_files_flat(),
            pinned: false,
            compact: self.log_files_compact(),
        }
    }

    /// 文件栏里各行：排成树时有目录行，收起的目录里的文件不列出。
    pub(crate) fn log_files_rows(&self) -> Rc<[LogFileRow]> {
        let Some(tree) = &self.git.log.tree else {
            return Rc::from([]);
        };
        let source = &tree.source;
        let folds = &self.git.log.files.folds;
        let shape = self.diff_files_shape();
        let key = RowsKey {
            panel: true,
            id: source.id(),
            len: source.files.len(),
            shape,
            folds: folds.version(),
        };
        let empty = HashSet::new();
        let folded = folds.of(&key.id).unwrap_or(&empty);
        let build = || log_file_rows(Some(&source.root), &source.files, shape, folded);
        self.cached_file_rows(key.clone(), build)
    }

    /// 收起或展开文件栏里第 `row` 行的目录 `dir`（相对仓库根目录），动画同详情一栏。
    pub(in crate::app) fn toggle_log_files_dir(&mut self, row: usize, dir: &str) {
        let Some(tree) = &self.git.log.tree else {
            return;
        };
        let id = tree.source.id();
        let before = self.log_files_rows();
        self.git.log.files.folds.toggle(&id, dir);
        let after = self.log_files_rows();
        self.git.log.panel_fold = self.file_fold(row, before, after);
        self.request_redraw();
    }

    /// 文件栏里第 `i` 个文件的悬停提示：截短或改过名时写完整路径，否则为 `None`。
    pub(crate) fn log_diff_file_tip(&self, i: usize) -> Option<String> {
        let tree = self.git.log.tree.as_ref()?;
        let file = tree
            .source
            .files
            .get(i)
            .filter(|_| self.git.log.files.long.contains(&i))?;
        Some(super::details::file_tip(file, &tree.source.root))
    }

    /// 文件栏要列出的：提交改动的文件、仓库的根目录和正在看的文件（下标）。
    pub(crate) fn log_diff_tree_source(&self) -> Option<(&[CommitFile], &Path, usize)> {
        let tree = self.git.log.tree.as_ref()?;
        let source = &tree.source;
        Some((&source.files, &source.root, tree.selected))
    }

    /// 文件栏是否平铺：切换过时按这里记下的，否则同提交视图。
    pub(crate) fn diff_files_flat(&self) -> bool {
        !self.state.diff_files_tree.unwrap_or(self.state.commit_tree)
    }

    /// 在平铺与树之间切换文件栏，记进状态文件；正在看的文件保持露出。
    pub(in crate::app) fn toggle_diff_files_layout(&mut self) {
        let tree = self.diff_files_flat();
        self.update_state(move |state| state.diff_files_tree = Some(tree));
        self.reveal_log_files_selected();
        self.request_redraw();
    }

    /// 文件栏是否展开。
    pub(crate) fn log_files_open(&self) -> bool {
        let files = self
            .git
            .log
            .tree
            .as_ref()
            .map_or(0, |t| t.source.files.len());
        !self.git.log.files.closed.unwrap_or(files < 2)
    }

    /// 把文件栏的宽度和露出的比例写进当前从日志打开的差异（[`DiffView::panel`]）。
    pub(crate) fn sync_log_files_panel(&mut self) {
        let Some(id) = self.git.log.preview else {
            return;
        };
        let width = self.git.log.files.width.unwrap_or(FILES_W);
        let shown = self.log_files_shown();
        let doc = self.editor.documents.iter_mut().find(|d| d.id == id);
        if let Some(view) = doc.and_then(|d| d.diff.as_deref_mut()) {
            view.panel = width;
            view.panel_shown = shown;
        }
    }

    /// 文件栏露出的比例：收起为 0，展开为 1，展开、收起途中在两者之间。
    pub(crate) fn log_files_shown(&self) -> f32 {
        let open = f32::from(u8::from(self.log_files_open()));
        let motion = self.git.log.files_motion;
        motion.map_or(open, |(from, clock)| clock.between(from, open))
    }

    /// 滚动文件栏，使正在看的文件露出来。
    pub(in crate::app) fn reveal_log_files_selected(&mut self) {
        let Some(selected) = self.git.log.tree.as_ref().map(|t| t.selected) else {
            return;
        };
        let view = self.editor.active().and_then(|d| d.diff.as_deref());
        let Some(area) = view.map(|v| diff_parts(self.layout.editor, v, self.metrics.cell).files)
        else {
            return;
        };
        let rows = self.log_files_rows();
        let Some(at) = rows.iter().position(|r| r.file() == Some(selected)) else {
            return;
        };
        // 列表在标题行下面，与绘制时的排法相同（见 `view::diff::files`）。
        let list_h = (area.h - 10.0 - FILES_TITLE).max(LOG_FILE_ROW);
        let (top, bottom) = (at as f32 * LOG_FILE_ROW, (at + 1) as f32 * LOG_FILE_ROW);
        let files = &mut self.git.log.files;
        if top < files.scroll {
            files.scroll = top;
        } else if bottom > files.scroll + list_h {
            files.scroll = bottom - list_h;
        }
    }

    /// 文件栏现在画出来的宽度（逻辑像素）；收起时为 0。
    pub(in crate::app) fn log_files_width(&self) -> f32 {
        let view = self.editor.active().and_then(|d| d.diff.as_deref());
        view.map_or(0.0, |view| {
            crate::app::diff_parts(self.layout.editor, view, self.metrics.cell)
                .files
                .w
        })
    }

    /// 展开或收起文件栏：从左边滑进、滑出，与日志的分支面板相同。
    pub(in crate::app) fn toggle_log_files_panel(&mut self) {
        let from = self.log_files_shown();
        self.git.log.files.closed = Some(self.log_files_open());
        let clock = self.clock(crate::app::animation::SIDEBAR_ANIMATION);
        self.git.log.files_motion = clock.map(|clock| (from, clock));
        self.sync_log_files_panel();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 拖动文件栏的右边缘到 `x`：按下时在 `start_x`，文件栏当时宽 `start_width`。
    pub(in crate::app) fn drag_log_files_edge(
        &mut self,
        x: f32,
        (start_x, start_width): (f32, f32),
    ) {
        let width = (start_width + (x - start_x)).clamp(DIFF_FILES_MIN, FILES_MAX);
        self.git.log.files.width = Some(width.round());
        self.sync_log_files_panel();
        // 按排版允许的宽度记下：窗口放不下拖出的宽度时，记下的就是画出来的宽度。
        let drawn = self.log_files_width();
        if drawn > 0.0 {
            self.git.log.files.width = Some(drawn);
            self.sync_log_files_panel();
        }
        self.request_redraw();
    }

    /// 双击文件栏的右边缘：恢复默认宽度。
    pub(in crate::app) fn reset_log_files_width(&mut self) {
        self.git.log.files.width = None;
        self.sync_log_files_panel();
        self.request_redraw();
    }

    /// 窗口坐标 (`x`, `y`) 是否在当前差异的文件栏里。
    pub(in crate::app) fn in_log_files(&self, x: f32, y: f32) -> bool {
        let view = self.editor.active().and_then(|d| d.diff.as_deref());
        view.filter(|v| v.panel > 0.0).is_some_and(|view| {
            let parts = crate::app::diff_parts(self.layout.editor, view, self.metrics.cell);
            parts.files.contains(x, y)
        })
    }
}
