//! 处理从系统文件管理器等来源拖入窗口的文件和目录。
//!
//! 拖动经过窗口时标出松手后的落点：落在文件树上时复制进落点所在的目录，落在终端上时输入转义
//! 后的路径，落在其他位置（编辑区等）时打开。
//!
//! 系统只在拖入窗口、拖出窗口和松手时各通知一次（`HoveredFile`、`HoveredFileCancelled`、
//! `DroppedFile`），拖动移动过程中既没有通知也没有鼠标移动事件。经过窗口期间按 [`POLL`] 的间隔
//! 查询鼠标位置，落点随之更新。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

/// 拖动文件经过窗口时查询鼠标位置的间隔。
///
/// 取 16 667 µs，即 60 Hz 的一帧；系统在拖动期间不发送鼠标移动事件，落点更新频率不能低于帧率。
const POLL: Duration = Duration::from_micros(16_667);

/// 拖入窗口的文件或目录此刻松手时的落点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropZone {
    /// 文件树中的目录：文件复制进去。
    Tree(PathBuf),
    /// 终端：输入转义过的路径。
    Terminal,
    /// 其他位置：文件在编辑区打开，目录切换为工作区。
    Open,
}

impl App {
    /// 返回在 (`x`, `y`) 处松手时拖入的文件或目录的落点。
    ///
    /// 文件树显示搜索面板或侧栏收起时，落在侧栏位置上也算打开。
    pub(in crate::app) fn drop_zone(&self, x: f32, y: f32) -> DropZone {
        if self.terminal_takes_drop(x, y) {
            return DropZone::Terminal;
        }
        let on_tree =
            self.panels.sidebar.view == SidebarView::Files && self.layout.sidebar.right() > 0.0;
        match on_tree.then(|| self.tree_dir_at(x, y)).flatten() {
            Some(dir) => DropZone::Tree(dir),
            None => DropZone::Open,
        }
    }

    /// 拖动中的落点（见 [`DropZone`]），绘制时据此铺底色；没有拖动时为 `None`。
    pub(crate) fn file_hover(&self) -> Option<&DropZone> {
        self.panels.file_hover.as_ref()
    }

    /// 文件或目录拖入了窗口（`HoveredFile`；同时拖动多项时每项触发一次）：开始跟踪落点。
    pub(in crate::app) fn hover_file(&mut self) {
        self.update_file_hover();
    }

    /// 拖出了窗口或松了手：不再标出落点。
    pub(in crate::app) fn end_file_hover(&mut self) {
        if self.panels.file_hover.take().is_some() {
            self.request_redraw();
        }
    }

    /// 拖动文件经过窗口时，按 [`POLL`] 的间隔查询鼠标位置并更新落点，返回下一次查询的时刻；
    /// 没有拖动时返回 `None`。
    pub(in crate::app) fn file_hover_tick(&mut self, now: Instant) -> Option<Instant> {
        self.panels.file_hover.as_ref()?;
        self.update_file_hover();
        Some(now + POLL)
    }

    /// 按鼠标当前位置重新计算落点，落点变化时才请求重绘。
    fn update_file_hover(&mut self) {
        let (x, y) = self.drop_point();
        let zone = Some(self.drop_zone(x, y));
        if zone != self.panels.file_hover {
            self.panels.file_hover = zone;
            self.request_redraw();
        }
    }

    /// 返回拖入中的文件此刻的位置（逻辑像素）。
    ///
    /// 系统在拖动期间不发送鼠标移动事件，需要向系统查询；查不到时（测试中没有窗口，或平台不
    /// 支持）使用最后一次鼠标移动的位置。macOS 的坐标已经是点，再除以界面缩放；Windows 的
    /// 坐标是物理像素，除以渲染器的缩放（系统缩放乘以界面缩放），与 `CursorMoved` 一致。
    fn drop_point(&self) -> (f32, f32) {
        #[cfg(target_os = "macos")]
        if let Some((x, y)) = self
            .window
            .handle
            .as_ref()
            .and_then(|w| crate::macos::mouse_location(w))
        {
            return (x / self.zoom(), y / self.zoom());
        }
        #[cfg(windows)]
        if let Some((x, y)) = self
            .window
            .handle
            .as_ref()
            .and_then(|w| crate::windows::mouse_location(w))
        {
            let scale = self
                .frame
                .renderer
                .as_ref()
                .map_or(1.0, |r| r.scale_factor());
            return (x / scale, y / scale);
        }
        self.pointer.mouse
    }

    /// 处理拖进窗口松手的文件或目录 `path`，规则见 [`Self::drop_file_at`]。
    pub(in crate::app) fn drop_file(&mut self, path: &Path) {
        self.end_file_hover();
        let (x, y) = self.drop_point();
        self.drop_file_at(path, x, y);
    }

    /// 处理在 (`x`, `y`) 处放下的外部文件或目录 `path`，按 [`Self::drop_zone`] 的结果选择处理方式。
    ///
    /// 落在文件树上时，`path` 已在目标目录中时不复制，只在树中选中它。
    pub(in crate::app) fn drop_file_at(&mut self, path: &Path, x: f32, y: f32) {
        let dir = match self.drop_zone(x, y) {
            DropZone::Tree(dir) => dir,
            DropZone::Terminal => {
                self.drop_path_on_terminal(path);
                return;
            }
            DropZone::Open => {
                self.open_path(path);
                return;
            }
        };
        // 文件树里的路径是规范化的（例如 `/private/var` 而不是 `/var`），比较之前先规范化。
        let path = &winc_config::files::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        let Some(ws) = self.editor.workspace.as_mut() else {
            return;
        };
        if path.parent() != Some(dir.as_path()) {
            self.copy_entries_into(vec![path.to_path_buf()], dir);
            return;
        }
        ws.reveal(path);
        self.panels.tree.motion = None;
        self.focus = Focus::Tree;
        if let Some(index) = ws.selected {
            self.scroll_tree_to(index, false);
        }
        self.relayout();
        self.request_redraw();
    }
}
