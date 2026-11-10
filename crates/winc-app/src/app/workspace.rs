//! 工作区：打开的目录与其中的文件。
//!
//! 这一组模块覆盖工作区的全部功能：目录的现场（`session`）、文件树与拖动（`tree`、`TreeState::drag`）、
//! 工具栏收起（`tool_fold`）、两个文件的比较（`compare`）、监视磁盘变化（`watch`）、打开与保存
//! （`loading`、`files`、`background_save`）、标签切换与拖动（`tab_drag`）、快速打开（`picker`）、
//! 最近打开的目录（`recent`）、全局搜索（`global`）和拖入文件（`file_drop`）。

use super::*;

pub(super) mod background_save;
pub(super) mod compare;
pub(super) mod file_drop;
pub(super) mod files;
pub(super) mod global;
pub(super) mod loading;
pub(super) mod outline;
pub(crate) mod picker;
pub(super) mod recent;
pub(super) mod session;
pub(super) mod tab_drag;
pub(crate) mod tab_names;
pub(super) mod tool_fold;
pub(super) mod tree;
pub(super) mod tree_drag;
pub(super) mod tree_label;
pub(super) mod watch;

/// 侧栏的界面状态：宽度、是否显示、当前视图，以及显示隐藏和切换视图的过渡。
pub struct Sidebar {
    pub(in crate::app) width: f32,
    /// 侧栏是否应显示；动画进行中取动画的终点。
    pub(in crate::app) visible: bool,
    pub(in crate::app) motion: Option<SidebarMotion>,
    /// 侧栏当前显示文件树还是全局搜索。
    pub(crate) view: SidebarView,
    /// 切换侧栏视图时，视图切换器中选中的一段从上一个视图滑入：记录上一个视图和动画时钟。
    pub(crate) view_slide: Option<(SidebarView, crate::app::animation::Clock)>,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            width: DEFAULT_PANEL_WIDTH,
            visible: false,
            motion: None,
            view: SidebarView::Files,
            view_slide: None,
        }
    }
}
