//! 在文件树中拖动文件或目录，移入其他目录（按着 `⌥` 松开时复制）。
//!
//! 从系统文件管理器等来源拖入窗口的文件和目录见 `file_drop.rs`。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use winc_editor::{EntryKind, Workspace};
use winc_i18n::t;

use super::*;

/// 判定为拖动的鼠标移动距离（逻辑像素，经验值）：按下后移动不足此距离时，松开按单击处理。
const DRAG_THRESHOLD: f32 = 4.0;

/// 拖动时停在收起的目录上多久之后自动展开该目录。
///
/// 取 600 ms（经验值）：足以与路过区分，又不会让用户等待；展开后可以继续把项目放入其中的
/// 子目录。
const EXPAND_DELAY: Duration = Duration::from_millis(600);

/// 触发自动滚动的区域高度（逻辑像素）：鼠标距文件树上下边缘不足此距离时开始滚动。
///
/// 取一行的高度 [`TREE_ROW`]。
const EDGE: f32 = TREE_ROW;
/// 鼠标贴着文件树上下边缘时每帧的滚动量（逻辑像素，经验值）。
///
/// 滚动量随鼠标向 [`EDGE`] 区域内侧移动而线性减小，到区域内侧边界时为 0。
const EDGE_SPEED: f32 = 12.0;

/// 文件树中被按住的项。
pub struct TreeDrag {
    /// 被按住的项的路径。
    pub from: PathBuf,
    /// 被按住的项是文件还是目录。
    pub kind: EntryKind,
    /// 被按住的项的显示名。
    pub name: String,
    /// 拖动时一起移动的各项：按在多选的一行上时是选中的各项（去掉上级目录也选中了的），
    /// 否则只有 `from`。
    pub paths: Vec<PathBuf>,
    /// 按下点的窗口坐标（逻辑像素）。
    origin: (f32, f32),
    /// 是否已形成拖动：鼠标移动超过 `DRAG_THRESHOLD` 之后才是拖动，之前松开按单击处理。
    pub active: bool,
    /// 按在固定于顶部的目录上时，该目录的层级（从 0 起）。
    pub(in crate::app) sticky: Option<usize>,
    /// 按下的是双击：松开时在普通标签中打开文件。
    pub(in crate::app) keep: bool,
    /// 此刻松开会放入的目录。
    ///
    /// 放置没有效果时为 `None`：目标在某个被拖项自身或其子目录内、被拖项都已在目标目录中，
    /// 或鼠标在文件树之外。
    pub target: Option<PathBuf>,
    /// 鼠标停留的收起目录，以及开始停留的时刻。
    resting: Option<(PathBuf, Instant)>,
}

impl TreeDrag {
    /// 创建对 `ws` 第 `index` 行的按住状态，`origin` 为按下点的窗口坐标（逻辑像素）。
    pub(in crate::app) fn new(ws: &Workspace, index: usize, origin: (f32, f32)) -> Self {
        let entry = &ws.rows()[index];
        let from = entry.path.clone();
        // 拖动合并显示的一行时，移动的是选中的那一级目录。
        let segment = ws.segment().filter(|_| ws.selected == Some(index));
        let dragged = segment.unwrap_or(&from);
        let name = winc_editor::segment_name(dragged).into_owned();
        let paths = if ws.multi_selected() && ws.is_selected(index) {
            ws.selection_roots()
        } else {
            vec![dragged.to_path_buf()]
        };
        TreeDrag {
            from,
            kind: entry.kind,
            name,
            paths,
            origin,
            active: false,
            sticky: None,
            keep: false,
            target: None,
            resting: None,
        }
    }
}

impl App {
    /// 返回正在拖动的文件树项；没有按住，或尚未形成拖动时返回 `None`。
    pub(crate) fn tree_dragging(&self) -> Option<&TreeDrag> {
        self.tree.drag.as_ref().filter(|d| d.active)
    }

    /// 是否按住了文件树中的一个文件、尚未形成拖动：此时松开就会打开该文件。
    pub(crate) fn tree_open_pending(&self) -> bool {
        self.tree
            .drag
            .as_ref()
            .is_some_and(|d| !d.active && d.kind == EntryKind::File)
    }

    /// 处理按住文件树项时的鼠标移动，(`x`, `y`) 为鼠标的窗口坐标（逻辑像素）。
    ///
    /// 移动距离超过 [`DRAG_THRESHOLD`] 后形成拖动，之后随鼠标位置重新计算放置目标。
    pub(in crate::app) fn drag_tree_item(&mut self, x: f32, y: f32) {
        let Some(drag) = &mut self.tree.drag else {
            return;
        };
        if !drag.active {
            let (ox, oy) = drag.origin;
            if (x - ox).hypot(y - oy) < DRAG_THRESHOLD {
                return;
            }
            drag.active = true;
        }

        let paths = drag.paths.clone();
        let target = self.drop_target(&paths, x, y, self.modifiers.alt_key());
        // 只有鼠标所在行是收起的目录、且该目录正是当前放置目标时才计时；停留的目录改变时重新计时。
        let collapsed = self
            .tree_row_at(y)
            .and_then(|i| self.editor.workspace.as_ref()?.rows().get(i))
            .filter(|e| e.kind == EntryKind::Dir && !e.expanded)
            .map(|e| e.path.clone())
            .filter(|path| target.as_ref() == Some(path));

        let Some(drag) = &mut self.tree.drag else {
            return;
        };
        drag.target = target;
        if collapsed.as_ref() != drag.resting.as_ref().map(|(path, _)| path) {
            drag.resting = collapsed.map(|path| (path, Instant::now()));
        }
        self.request_redraw();
    }

    /// 处理鼠标松开。
    ///
    /// 未形成拖动时按单击处理：只选中这一行，文件打开，目录展开或收起。已形成拖动时把被拖的
    /// 各项放入目标目录。
    pub(in crate::app) fn release_tree_item(&mut self) {
        let Some(drag) = self.tree.drag.take() else {
            return;
        };

        if !drag.active {
            if let Some(ws) = &mut self.editor.workspace
                && ws.multi_selected()
            {
                let index = ws.rows().iter().position(|e| e.path == drag.from);
                ws.select(index);
            }
            if let Some(level) = drag.sticky
                && let Some(ws) = &self.editor.workspace
                && let Some(index) = ws.rows().iter().position(|e| e.path == drag.from)
            {
                self.settle_sticky(index, level);
            }
            self.open_tree_entry(&drag.from, drag.keep);
            return;
        }

        if self.layout.terminal.contains(self.mouse.0, self.mouse.1) {
            // 拖进终端：插入路径，不移动文件。
            if self.terminal_takes_drop(self.mouse.0, self.mouse.1) {
                for path in &drag.paths {
                    self.drop_path_on_terminal(path);
                }
            }
        } else if self.modifiers.alt_key() {
            // 按着 `⌥` 松开：复制。松开前才按下 `⌥` 时放置目标没有按复制计算过，这里重新计算。
            let (x, y) = self.mouse;
            if let Some(dir) = self.drop_target(&drag.paths, x, y, true) {
                self.copy_entries_into(drag.paths, dir);
            }
        } else if let Some(dir) = drag.target {
            self.ask_move(drag.paths, dir);
        }
        self.set_hover(self.hover_at(self.mouse.0, self.mouse.1));
        self.request_redraw();
    }

    /// 取消拖动（按下 Esc 或窗口失去焦点），不移动任何文件。
    pub(in crate::app) fn cancel_tree_drag(&mut self) {
        if self.tree.drag.take().is_some_and(|d| d.active) {
            self.set_hover(self.hover_at(self.mouse.0, self.mouse.1));
            self.request_redraw();
        }
    }

    /// 处理对 `path` 的单击：文件在编辑区打开，焦点保留在文件树，便于用方向键继续浏览；
    /// 目录展开或收起。
    ///
    /// 启用预览标签时文件在预览标签中打开（见 [`App::preview_path`]），`keep`（双击）时在普通
    /// 标签中打开。
    fn open_tree_entry(&mut self, path: &Path, keep: bool) {
        let Some(ws) = &self.editor.workspace else {
            return;
        };
        let Some(index) = ws.rows().iter().position(|e| e.path == path) else {
            return;
        };

        if ws.rows()[index].kind == EntryKind::Dir {
            // 按着 `⌥` 单击：递归展开或收起。
            if self.modifiers.alt_key() {
                self.toggle_dir_recursively(index);
            } else {
                self.set_dir_expanded(index, None);
            }
        } else {
            if keep || !self.preview_on_click() {
                self.open_path(path);
            } else {
                self.preview_path(path);
            }
            self.focus = Focus::Tree;
        }
    }

    /// 处理拖动中与时间相关的操作，返回下一次需要唤醒的时刻。
    ///
    /// - 鼠标在收起的目录上停留满 [`EXPAND_DELAY`] 后，展开该目录。
    /// - 鼠标靠近文件树上下边缘时，自动滚动文件树。
    ///
    /// 没有形成拖动，或无需再次唤醒时返回 `None`。
    pub(in crate::app) fn tree_drag_tick(&mut self, now: Instant) -> Option<Instant> {
        let resting = self.tree_dragging()?.resting.clone();
        let mut wake = None;

        if let Some((dir, since)) = resting {
            let due = since + EXPAND_DELAY;
            if now < due {
                wake = Some(due);
            } else {
                let index = self
                    .editor
                    .workspace
                    .as_ref()
                    .and_then(|ws| ws.rows().iter().position(|e| e.path == dir));
                if let Some(index) = index {
                    self.set_dir_expanded(index, Some(true));
                }
                if let Some(drag) = &mut self.tree.drag {
                    drag.resting = None;
                }
            }
        }

        let (x, y) = self.mouse;
        let tree = self.layout.tree;
        let push = if !tree.contains(x, y) {
            0.0
        } else if y < tree.y + EDGE {
            y - (tree.y + EDGE)
        } else if y > tree.bottom() - EDGE {
            y - (tree.bottom() - EDGE)
        } else {
            0.0
        };
        let rows = self.editor.workspace.as_ref().map_or(0, |w| w.rows().len());
        let scroll =
            (self.tree.scroll + push / EDGE * EDGE_SPEED).clamp(0.0, self.tree_max_scroll(rows));
        if scroll != self.tree.scroll {
            self.tree.scroll = scroll;
            // 文件树滚动后，鼠标下方的行改变，需要重新计算放置目标。
            self.drag_tree_item(x, y);
            let next = now + self.frame_interval;
            wake = Some(wake.map_or(next, |w: Instant| w.min(next)));
        }

        wake
    }

    /// 返回在 (`x`, `y`) 处放下时的目标目录；鼠标在文件树之外时返回 `None`。
    ///
    /// - 鼠标在目录行上：该目录。
    /// - 鼠标在文件行上：该文件所在的目录。
    /// - 鼠标在文件树的空白处：根目录。
    pub(super) fn tree_dir_at(&self, x: f32, y: f32) -> Option<PathBuf> {
        let ws = self.editor.workspace.as_ref()?;
        if !self.layout.tree.contains(x, y) && !self.in_tree_overflow(x, y) {
            return None;
        }

        let Some(index) = self.tree_row_at(y) else {
            return Some(ws.root.clone());
        };
        let entry = &ws.rows()[index];
        Some(match entry.kind {
            // 合并显示的一行放进鼠标所在的那一级目录，其余位置放进这一行本身。
            EntryKind::Dir => self
                .tree_segment_at(index, x)
                .unwrap_or_else(|| entry.path.clone()),
            EntryKind::File => entry.path.parent()?.to_path_buf(),
        })
    }

    /// 返回在 (`x`, `y`) 处松开时 `paths` 的目标目录（见 [`Self::tree_dir_at`]）；放置没有效果时
    /// 返回 `None`：鼠标在文件树之外、目标是某一项自身或其子目录，以及移动（`copy` 为 `false`）
    /// 时各项都已在目标目录中（放下后位置不变；复制时在原处复制出副本）。
    fn drop_target(&self, paths: &[PathBuf], x: f32, y: f32, copy: bool) -> Option<PathBuf> {
        let dir = self.tree_dir_at(x, y)?;
        let inside = paths.iter().any(|from| dir.starts_with(from));
        let stays = paths
            .iter()
            .all(|from| from.parent() == Some(dir.as_path()));
        (!inside && (copy || !stays)).then_some(dir)
    }

    /// 在后台把 `paths` 中的各项复制进目录 `dir`，完成后选中各个副本（[`App::copied_into_tree`]）。
    ///
    /// 同名时副本改名（见 [`winc_editor::copy_entry`]）。目录可能很大，复制时间与内容大小成正比，
    /// 因此不在界面线程上执行。某一项复制失败时提示原因，其余各项仍然复制。
    pub(in crate::app) fn copy_entries_into(&mut self, paths: Vec<PathBuf>, dir: PathBuf) {
        let Some(root) = self.editor.root().map(Path::to_path_buf) else {
            return;
        };
        let copy = move || {
            let (mut copies, mut failures) = (Vec::new(), Vec::new());
            for from in &paths {
                match winc_editor::copy_entry(&root, from, &dir) {
                    Ok(copy) => copies.push(copy),
                    Err(e) => failures.push(t!(
                        "workspace-tree-copy-failed",
                        name = winc_editor::display_name(from),
                        error = e.to_string()
                    )),
                }
            }
            (copies, failures)
        };
        match self.proxy.clone() {
            Some(proxy) => winc_editor::background::spawn(move || {
                let (copies, failures) = copy();
                let _ = proxy.send_event(UserEvent::Copied(copies, failures));
            }),
            None => {
                let (copies, failures) = copy();
                self.copied_into_tree(copies, failures);
            }
        }
    }

    /// 拖进文件树的各项复制完成：刷新文件树并选中各个副本 `copies`，提示复制失败的各项。
    pub(in crate::app) fn copied_into_tree(&mut self, copies: Vec<PathBuf>, failures: Vec<String>) {
        for failure in failures {
            self.notify(failure);
        }
        if !copies.is_empty()
            && let Some(ws) = &mut self.editor.workspace
        {
            ws.refresh();
            // 展开到各个副本所在的位置，再一起选中（只能选中显示出来的行）。
            for copy in &copies {
                ws.reveal(copy);
            }
            ws.select_paths(&copies);
            self.tree.motion = None;
            self.focus = Focus::Tree;
            if let Some(index) = ws.selected {
                self.scroll_tree_to(index, false);
            }
        }
        self.picker_touched(copies);
        self.relayout();
        self.request_redraw();
    }

    /// 确认后把 `paths` 里的各项移入 `dir`。
    ///
    /// 先弹出提示框，用户确认后才移动。移动没有撤销，确认可以避免误拖。
    /// 配置项 `tree.confirmDragAndDrop` 为 `false`（提示框上勾选过「不再询问」）时不询问，
    /// 直接移动。
    fn ask_move(&mut self, paths: Vec<PathBuf>, dir: PathBuf) {
        if self.config.confirm_drag_and_drop != Some(false) && self.window.handle.is_some() {
            use crate::app::AlertButton;

            let to = winc_editor::display_name(&dir);
            let message = match paths.as_slice() {
                [from] => t!(
                    "workspace-tree-move-confirm-one",
                    name = winc_editor::display_name(from),
                    folder = to
                ),
                many => t!(
                    "workspace-tree-move-confirm-many",
                    count = many.len(),
                    folder = to
                ),
            };
            let buttons = vec![
                AlertButton::new(t!("workspace-tree-move")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let asked = Some((t!("workspace-tree-dont-ask"), false));
            self.show_alert(&message, "", buttons, asked, move |index, never| {
                (index == 0).then(|| UserEvent::Move(paths.clone(), dir.clone(), never))
            });
            return;
        }

        // 不需要询问、没有窗口（测试中）时直接移动。
        self.move_entries(&paths, &dir);
    }

    /// 处理提示框上的「移动」按钮。
    ///
    /// `never_ask` 为 `true`（勾选了「不再询问」）时写入配置，之后拖动不再询问。
    pub(in crate::app) fn confirm_move(&mut self, paths: &[PathBuf], dir: &Path, never_ask: bool) {
        if never_ask {
            self.config.confirm_drag_and_drop = Some(false);
            self.save_config();
        }
        self.move_entries(paths, dir);
    }

    /// 把 `paths` 里的各项移入目录 `dir`，并同步更新文件树和已打开文档的路径；移动后选中移过去
    /// 的各项。
    ///
    /// 文档的内容和修改状态不变。某一项移动失败时提示原因，其余各项仍然移动。
    pub(in crate::app) fn move_entries(&mut self, paths: &[PathBuf], dir: &Path) {
        // 先等待后台保存完成，否则保存会在原位置重新写出文件。
        self.wait_for_saves();
        let mut moved = Vec::new();
        for from in paths {
            let Some(result) = self
                .editor
                .workspace
                .as_mut()
                .map(|ws| ws.move_into(from, dir))
            else {
                return;
            };
            match result {
                Ok(target) => {
                    self.paths_moved(from, &target);
                    // 从别的窗口剪切来的：原窗口打开的文件、文件树也要跟着改。
                    let elsewhere = self
                        .editor
                        .workspace
                        .as_ref()
                        .is_some_and(|ws| !from.starts_with(&ws.root));
                    if elsewhere {
                        let request = crate::winc::Request::Moved(from.clone(), target.clone());
                        self.shared.request(request);
                    }
                    moved.push(target);
                }
                Err(e) => {
                    let name = winc_editor::display_name(from);
                    self.notify(t!(
                        "workspace-tree-move-failed",
                        name = name,
                        error = e.to_string()
                    ));
                }
            }
        }

        if !moved.is_empty()
            && let Some(ws) = &mut self.editor.workspace
        {
            ws.select_paths(&moved);
            self.tree.motion = None;
            self.focus = Focus::Tree;
            if let Some(index) = ws.selected {
                self.scroll_tree_to(index, false);
            }
        }

        self.relayout();
        self.request_redraw();
    }
}
