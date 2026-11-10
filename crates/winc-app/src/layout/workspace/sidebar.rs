//! 侧栏顶上的几何：文件树工具栏、折叠工具、视图切换的分段按钮、红绿灯让出的位置。

use super::*;
use winc_i18n::t;

/// 文件树工具栏上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeTool {
    /// 新建文件。
    NewFile,
    /// 新建目录。
    NewFolder,
    /// 刷新目录。
    Refresh,
    /// 折叠所有目录。
    CollapseAll,
    /// 在文件树中定位当前文件。
    Locate,
}

impl TreeTool {
    /// 全部工具，顺序与工具栏从左到右的顺序一致。
    pub const ALL: [TreeTool; 5] = [
        TreeTool::NewFile,
        TreeTool::NewFolder,
        TreeTool::Refresh,
        TreeTool::CollapseAll,
        TreeTool::Locate,
    ];

    /// 返回悬停提示文字。
    pub fn label(self) -> &'static str {
        match self {
            TreeTool::NewFile => t!("workspace-tree-tool-new-file"),
            TreeTool::NewFolder => t!("workspace-tree-tool-new-folder"),
            TreeTool::Refresh => t!("workspace-tree-tool-refresh"),
            TreeTool::CollapseAll => t!("workspace-tree-tool-collapse-all"),
            TreeTool::Locate => t!("workspace-tree-tool-locate"),
        }
    }
}

/// 项目名（含箭头）所占宽度的下限（逻辑像素）：侧栏最窄时仍要露出名字的开头和折叠箭头，
/// 工具栏不占用这部分宽度。
pub const TREE_NAME_MIN: f32 = 48.0;

/// 返回文件树工具栏的按钮布局：位于项目名所在行的右端，从左到右依次是新建文件、新建目录、刷新、
/// 全部折叠、定位。
///
/// 放不下时从右向左收起：留下的按钮仍然显示，收起的按钮移入「⋯」菜单。返回留下的按钮（从左到右）
/// 和「⋯」按钮的位置，全部放得下时后者为 `None`。`name_right` 是项目名和箭头的右边界，
/// 工具栏不会越过该边界。
pub fn tree_tools(header: Rect, name_right: f32) -> (Vec<(Rect, TreeTool)>, Option<Rect>) {
    let right = header.right() - 8.0;
    // 可容纳的按钮数；有按钮被收起时，其中一格留给「⋯」，刚好全部放下时不显示「⋯」。
    let room = ((right - name_right + BUTTON_GAP) / (BUTTON_SMALL + BUTTON_GAP))
        .floor()
        .max(0.0) as usize;
    let all = TreeTool::ALL.len();
    let button = |i, count| toolbar_button(header, right, i, count);

    if room >= all {
        let buttons = (0..all)
            .map(|i| (button(i, all), TreeTool::ALL[i]))
            .collect();
        return (buttons, None);
    }

    if room == 0 {
        return (Vec::new(), None);
    }

    // 最右侧一格留给「⋯」，其余各格按顺序留给最靠前的按钮。
    let kept = room - 1;
    let buttons = (0..kept)
        .map(|i| (button(i, room), TreeTool::ALL[i]))
        .collect();
    (buttons, Some(button(kept, room)))
}

/// 返回收起动画途中文件树工具栏的绘制位置：`shown` 是各工具（按 [`TreeTool::ALL`] 的顺序）
/// 显示出来的程度，`more` 是「⋯」按钮的，0 表示收起，1 表示完全显示。
///
/// 从右往左排：「⋯」固定在最右一格，每个工具按自身程度占用对应格数。正在收起的工具向右滑到右侧
/// 相邻的按钮上（最终叠进「⋯」），左边的按钮跟着右移补上空位；程度都是 0 或 1 时与 [`TreeState::tools`]
/// 的结果相同。返回显示程度大于 0 的工具（从左到右）及其程度，以及「⋯」按钮。
pub fn folding_tools(
    header: Rect,
    shown: &[f32; TreeTool::ALL.len()],
    more: f32,
) -> (Vec<(Rect, TreeTool, f32)>, Option<Rect>) {
    let right = header.right() - 8.0;
    let step = BUTTON_SMALL + BUTTON_GAP;
    let y = header.y + (header.h - BUTTON_SMALL) / 2.0;

    let mut cursor = right - more * step;
    let mut tools = Vec::new();
    for (i, &tool) in TreeTool::ALL.iter().enumerate().rev() {
        if shown[i] <= 0.0 {
            continue;
        }
        cursor -= shown[i] * step;
        let rect = Rect::new(cursor + BUTTON_GAP, y, BUTTON_SMALL, BUTTON_SMALL);
        tools.push((rect, tool, shown[i]));
    }
    tools.reverse();

    let more = (more > 0.0).then(|| toolbar_button(header, right, 0, 1));
    (tools, more)
}

/// 返回右对齐于 `right` 的一排 `count` 个工具栏按钮中第 `i` 个（从 0 起）的位置，在 `bar`
/// 内垂直居中。
pub(in crate::layout) fn toolbar_button(bar: Rect, right: f32, i: usize, count: usize) -> Rect {
    let step = BUTTON_SMALL + BUTTON_GAP;
    let x = right - (count - i) as f32 * step + BUTTON_GAP;
    let y = bar.y + (bar.h - BUTTON_SMALL) / 2.0;
    Rect::new(x, y, BUTTON_SMALL, BUTTON_SMALL)
}

/// 返回工具栏收起后移入「⋯」菜单的按钮；`kept` 是工具栏留下的按钮数。
pub fn folded_tools(kept: usize) -> &'static [TreeTool] {
    &TreeTool::ALL[kept.min(TreeTool::ALL.len())..]
}

/// 侧栏显示的内容：文件树、全局搜索，或 Git 的提交视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarView {
    /// 文件树。
    #[default]
    Files,
    /// 全局搜索面板。
    Search,
    /// Git 的提交视图：改动列表、提交信息。
    ///
    /// 只在 Git 模块开着、目录在仓库里时有。
    Commit,
}

impl SidebarView {
    /// 返回悬停提示文字（含快捷键）。
    pub fn label(self) -> String {
        match self {
            SidebarView::Files => with_key(t!("workspace-sidebar-files"), Some(Action::ShowFiles)),
            SidebarView::Search => {
                with_key(t!("workspace-sidebar-search"), Some(Action::FindInFiles))
            }
            SidebarView::Commit => {
                with_key(t!("workspace-sidebar-commit"), Some(Action::ShowCommit))
            }
        }
    }
}

/// 红绿灯右端距窗口左边缘的距离（逻辑像素），以 `f32` 的位存放；见 [`TRAFFIC_LIGHTS_END`]。
static LIGHTS_END: AtomicU32 = AtomicU32::new(TRAFFIC_LIGHTS_END.to_bits());

/// 返回红绿灯右端距窗口左边缘的距离（逻辑像素）；没有红绿灯时为 0。
pub fn traffic_lights_end() -> f32 {
    f32::from_bits(LIGHTS_END.load(Ordering::Relaxed))
}

/// 记录红绿灯右端的实际位置（窗口创建、摆放红绿灯后调用），返回位置是否有变化。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn set_traffic_lights_end(end: f32) -> bool {
    LIGHTS_END.swap(end.to_bits(), Ordering::Relaxed) != end.to_bits()
}

/// 总是有的两个侧栏视图。
pub const BASE_VIEWS: [SidebarView; 2] = [SidebarView::Files, SidebarView::Search];

/// 返回红绿灯右侧第一个按钮的左边缘：侧栏收起时是侧栏开关，展开时是侧栏顶部的「文件」切换。
///
/// 两个按钮里的图标宽度不同（16 和 14），因此按图标可见的左边缘对齐，而不是按按钮对齐：侧栏
/// 收起和展开时，红绿灯到第一个图标的距离都是 [`LIGHTS_GAP`]。没有红绿灯时按钮距卡片左边缘 8。
pub(in crate::layout) fn first_button_x(icon_width: f32) -> f32 {
    let end = traffic_lights_end();
    if end > 0.0 {
        end + LIGHTS_GAP - (BUTTON - icon_width) / 2.0
    } else {
        ISLAND_MARGIN + 8.0
    }
}

/// 视图切换（分段控件）里一段的宽度（逻辑像素）：图标左右宽松，段与段之间隔
/// [`SEGMENT_SPACING`]，外面的底槽每边再宽 2。
pub const SEGMENT_W: f32 = 28.0;
/// 视图切换里一段的高度（逻辑像素）：22 高的横条，图标上下各留 4。
pub const SEGMENT_H: f32 = 22.0;
/// 分段控件里相邻两段之间的缝：选中一段凸起时与旁边一段分开，不贴在一起。
pub const SEGMENT_SPACING: f32 = 2.0;
/// 视图切换和侧栏开关之间至少隔多远（侧栏最窄时）。
pub const SEGMENT_GAP: f32 = 6.0;

/// 返回顶栏里视图切换（「文件」「搜索」以及模块提供的视图）每一段的位置，顺序同 `views`：
/// 分段控件靠侧栏 `sidebar` 的右端（离右边 8 像素，与第二行右端的按钮对齐），侧栏窄到挨着
/// 侧栏开关 `toggle` 时停在开关右边 [`SEGMENT_GAP`] 处，与开关垂直居中对齐。
///
/// 每一段带一个显示比例（0–1），平时是 1：模块的视图出现、消失途中，对应的一段和前面的缝
/// 按比例变窄，底槽跟着伸缩，整个控件仍靠右；超出侧栏的部分由调用方裁掉。
pub fn view_tabs(
    toggle: Rect,
    sidebar: Rect,
    views: &[(SidebarView, f32)],
) -> Vec<(Rect, SidebarView)> {
    // 每段前面的缝；第一段前面没有。
    let gap = |i: usize| if i == 0 { 0.0 } else { SEGMENT_SPACING };
    let widths = views
        .iter()
        .enumerate()
        .map(|(i, v)| (SEGMENT_W + gap(i)) * v.1);
    let track_w = widths.sum::<f32>() + 4.0;
    let left = (sidebar.right() - 8.0 - track_w).max(toggle.right() + SEGMENT_GAP) + 2.0;
    let y = toggle.y + (toggle.h - SEGMENT_H) / 2.0;
    let mut x = left;
    views
        .iter()
        .enumerate()
        .map(|(i, &(view, share))| {
            x += gap(i) * share;
            let rect = Rect::new(x, y, SEGMENT_W * share, SEGMENT_H);
            x += rect.w;
            (rect, view)
        })
        .collect()
}
