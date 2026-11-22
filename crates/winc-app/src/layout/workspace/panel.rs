//! 全局搜索面板的几何：工具按钮、查找和替换的输入框、结果行上的按钮、全部替换的确认卡片。

use super::*;

/// 搜索面板顶部一行的工具按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelTool {
    /// 重新搜索。
    #[default]
    Refresh,
    /// 清除搜索结果。
    Clear,
    /// 在对话框中打开搜索。
    OpenDialog,
    /// 折叠全部结果；全部结果都已折叠时变为「展开」。
    CollapseAll,
}

impl PanelTool {
    /// 全部工具按钮，顺序为从左到右。
    pub const ALL: [PanelTool; 4] = [
        PanelTool::Refresh,
        PanelTool::Clear,
        PanelTool::OpenDialog,
        PanelTool::CollapseAll,
    ];
}

/// 搜索面板中可以点击的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelButton {
    /// 顶部一行的工具按钮。
    Tool(PanelTool),
    /// 展开或收起替换输入。
    Expand,
    /// 区分大小写。
    Case,
    /// 全字匹配。
    Word,
    /// 使用正则表达式。
    Regex,
    /// 也搜索被忽略的文件。
    Ignored,
    /// 全部替换。
    ReplaceAll,
    /// 结果统计行右端的「过滤」按钮：展开或收起「包含」「排除」两个输入。
    Details,
}

/// 悬停结果行时出现的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    /// 替换该行的匹配。
    Replace,
    /// 把该行从结果中移除。
    Dismiss,
}

/// 搜索面板输入框的高度（逻辑像素），与标题行的工具按钮同高。
pub const PANEL_FIELD: f32 = BUTTON_SMALL;
/// 输入框里开关、按钮的高度（逻辑像素）。
const PANEL_TOGGLE_H: f32 = PANEL_FIELD - 4.0;
/// 输入框之间的纵向间距（逻辑像素），与侧栏其他视图相同。
const PANEL_GAP: f32 = SIDEBAR_GAP;
/// 搜索面板的左对齐线（相对面板左边缘，逻辑像素）：输入框里的文字、结果统计的文字、文件行的
/// 图标都从这里开始。
///
/// 输入框本身从 8 像素开始（与标题行的项目名、右端的按钮对齐），左端 2 像素处是展开替换的
/// 箭头。
pub const PANEL_LEFT: f32 = 32.0;
/// 文件名和匹配预览的起点（相对面板左边缘，逻辑像素），比 `PANEL_LEFT` 多 18 像素，即一个
/// 文件图标加间隔。
pub const PANEL_TEXT: f32 = PANEL_LEFT + 18.0;
/// 「展开替换」箭头和文件行箭头的中心（相对面板左边缘，逻辑像素）。
pub const PANEL_CHEVRON: f32 = 20.0;

/// 侧栏搜索面板的布局。
///
/// 从上到下依次是：标题行（左边是项目和分支，右端是工具按钮）、查找（左端是展开替换的箭头，
/// 右端是 Aa、ab、.* 三个开关和「过滤」按钮）、替换（展开时）、包含和排除（展开时）、结果统计
/// （有内容时才有这一行）、结果列表。输入框和标题行左右对齐：左边和项目名、右边和工具按钮
/// 对齐。输入框的矩形是外框，文字在其内部从左对齐线开始。
#[derive(Debug, Clone, Copy, Default)]
pub struct PanelParts {
    /// 标题行，工具按钮位于其右端。
    pub header: Rect,
    /// 顶部的工具按钮及其位置。
    pub tools: [(Rect, PanelTool); PanelTool::ALL.len()],
    /// 查找输入框左端的「展开替换」箭头。
    pub expand: Rect,
    /// 查找输入框。
    pub query: Rect,
    /// 查找输入框内的「区分大小写」开关。
    pub case_toggle: Rect,
    /// 查找输入框内的「全字匹配」开关。
    pub word_toggle: Rect,
    /// 查找输入框内的「使用正则表达式」开关。
    pub regex_toggle: Rect,
    /// 替换输入框；收起替换时尺寸为 0。
    pub replace: Rect,
    /// 替换行露出的部分；收起替换时尺寸为 0。
    ///
    /// 展开、收起途中替换输入框从查找输入框下方滑出或收回，只绘制露出的部分。
    pub replace_band: Rect,
    /// 替换输入框内的「全部替换」按钮，和查找输入框里的「过滤」按钮对齐；收起替换时尺寸为 0。
    pub replace_all: Rect,
    /// 「排除」输入框内的「也搜索被忽略的文件」开关；收起时尺寸为 0。
    pub ignored_toggle: Rect,
    /// 查找输入框最右端的「过滤」按钮：展开或收起「包含」「排除」。
    pub details: Rect,
    /// 「包含」输入框；收起时尺寸为 0。
    pub include: Rect,
    /// 「排除」输入框；收起时尺寸为 0。
    pub exclude: Rect,
    /// 「包含」「排除」两行露出的部分，含义同 `replace_band`；收起时尺寸为 0。
    pub details_band: Rect,
    /// 结果统计行；没有内容（还没输入）时高度为 0。
    pub summary: Rect,
    /// 结果列表区域。
    pub results: Rect,
}

impl PanelParts {
    /// 计算搜索面板各部件的位置。
    ///
    /// `panel` 为侧栏中顶栏以下的区域；`replace_shown` 为替换行展开的比例（0 为收起，
    /// 1 为完全展开），`details_shown` 为「包含」「排除」两行展开的比例；
    /// `summary` 表示结果统计行是否有内容。展开途中各行只占比例对应的高度，
    /// 下方的行随之移动；输入框尺寸不变，从上一行下方滑出。`heights` 是查找、替换输入框的高度，
    /// 文字折成多行时大于 [`PANEL_FIELD`]。
    pub fn new(
        panel: Rect,
        replace_shown: f32,
        details_shown: f32,
        summary: bool,
        heights: (f32, f32),
    ) -> PanelParts {
        let header = Rect::new(panel.x, panel.y, panel.w, SIDEBAR_HEADER);
        let right = header.right() - 8.0;
        let count = PanelTool::ALL.len();
        let tools =
            std::array::from_fn(|i| (toolbar_button(header, right, i, count), PanelTool::ALL[i]));

        let left = panel.x + 8.0;
        let width = (right - left).max(0.0);
        let mut y = header.bottom() + SIDEBAR_FIRST;

        let query = Rect::new(left, y, width, heights.0);
        let inset = (PANEL_FIELD - PANEL_TOGGLE_H) / 2.0;
        // 箭头在输入框的高度上居中，其余按钮留在第一行。
        let expand = Rect::new(
            left + inset,
            y + (heights.0 - PANEL_TOGGLE_H) / 2.0,
            20.0,
            PANEL_TOGGLE_H,
        );
        // 开关、「过滤」在输入框内靠右，从右往左排：都是正方形，彼此相隔 `BUTTON_GAP`。
        let toggle = |right: f32, y: f32| {
            Rect::new(
                right - PANEL_TOGGLE_H,
                y + inset,
                PANEL_TOGGLE_H,
                PANEL_TOGGLE_H,
            )
        };
        let details = toggle(query.right() - inset, y);
        let regex_toggle = toggle(details.x - BUTTON_GAP, y);
        let word_toggle = toggle(regex_toggle.x - BUTTON_GAP, y);
        let case_toggle = toggle(word_toggle.x - BUTTON_GAP, y);
        y = query.bottom();

        // 计算展开比例为 `share` 的一段：返回露出的部分，并把 `y` 移到其下沿；
        // 输入框相对完全展开时上移未露出的高度（`full - shown`），使其下沿与露出部分对齐。
        let band = |y: &mut f32, share: f32, full: f32| {
            let shown = full * share.clamp(0.0, 1.0);
            let band = Rect::new(panel.x, *y, panel.w, shown);
            *y += shown;
            (band, shown - full)
        };

        let row = PANEL_GAP + PANEL_FIELD;
        let (replace, replace_all, replace_band) = if replace_shown > 0.0 {
            let (area, shift) = band(&mut y, replace_shown, PANEL_GAP + heights.1);
            let top = area.y + shift + PANEL_GAP;
            let field = Rect::new(left, top, width, heights.1);
            (field, toggle(field.right() - inset, top), area)
        } else {
            Default::default()
        };

        let (include, exclude, details_band, ignored_toggle) = if details_shown > 0.0 {
            let (area, shift) = band(&mut y, details_shown, 2.0 * row);
            let include = Rect::new(left, area.y + shift + PANEL_GAP, width, PANEL_FIELD);
            let exclude = Rect::new(left, include.bottom() + PANEL_GAP, width, PANEL_FIELD);
            let toggle = toggle(exclude.right() - inset, exclude.y);
            (include, exclude, area, toggle)
        } else {
            Default::default()
        };

        // 统计行高 24 像素（与文件树一行同高）；没有内容时不占地方，结果列表紧接在输入框下方。
        let summary = if summary {
            Rect::new(panel.x, y + SIDEBAR_LIST_GAP, panel.w, 24.0)
        } else {
            Rect::new(panel.x, y + SIDEBAR_LIST_GAP, panel.w, 0.0)
        };
        let results = Rect::new(
            panel.x,
            summary.bottom(),
            panel.w,
            (panel.bottom() - summary.bottom()).max(0.0),
        );

        PanelParts {
            header,
            tools,
            expand,
            query,
            case_toggle,
            word_toggle,
            regex_toggle,
            ignored_toggle,
            replace,
            replace_all,
            replace_band,
            details,
            include,
            exclude,
            details_band,
            summary,
            results,
        }
    }

    /// 返回面板中 `(x, y)` 处的按钮；没有时返回 `None`。
    ///
    /// `replace_open` 为 `false` 时，不包含「全部替换」按钮。
    pub fn button_at(&self, x: f32, y: f32, replace_open: bool) -> Option<PanelButton> {
        [
            (self.expand, PanelButton::Expand),
            (self.case_toggle, PanelButton::Case),
            (self.word_toggle, PanelButton::Word),
            (self.regex_toggle, PanelButton::Regex),
            (self.ignored_toggle, PanelButton::Ignored),
            (self.details, PanelButton::Details),
        ]
        .into_iter()
        .chain(self.tools.map(|(r, t)| (r, PanelButton::Tool(t))))
        .chain(replace_open.then_some((self.replace_all, PanelButton::ReplaceAll)))
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, b)| b)
    }

    /// 返回输入框中放文字的区域：从 `PANEL_LEFT` 处的左对齐线开始（左侧让出展开替换的箭头），
    /// 右侧让出按钮。
    ///
    /// `right` 是右侧按钮区域的左边缘，文字区域的右边缘与该边缘相距 4 像素。
    pub fn text_area(field: Rect, right: f32) -> Rect {
        let x = field.x - 8.0 + PANEL_LEFT;
        Rect::new(x, field.y, (right - 4.0 - x).max(0.0), field.h)
    }
}

/// 返回结果行悬停时右端的按钮：「替换」（展开了替换输入时才有）和「移除」（把该行从结果中移除）。
///
/// 依次返回替换按钮和移除按钮的矩形。按钮比行的底色（上下、右边各让出 1）再小一圈：和输入框
/// 里的开关一样 20 见方，四周离底色边缘都是 1，悬停的底色套在行的底色里面，不上下冒出去、
/// 右边也不多出一截。
pub fn row_actions(row: Rect, replace_open: bool) -> (Option<Rect>, Rect) {
    const B: f32 = PANEL_TOGGLE_H;
    let y = row.y + (row.h - B) / 2.0;
    let dismiss = Rect::new(row.right() - 9.0 - B, y, B, B);
    let replace = replace_open.then(|| Rect::new(dismiss.x - BUTTON_GAP - B, y, B, B));
    (replace, dismiss)
}
