//! 日志列表右边的几列：作者、日期、提交号。可以在标题栏的「⋯」菜单里显示、隐藏，作者一列可以
//! 拖动左边缘改宽度。

use super::*;
use crate::app::input::context_menu::MenuEntry;

/// 日志列表右边的一列，按从左到右的顺序声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogColumn {
    /// 作者：头像和名字。
    Author,
    /// 提交时间。
    Date,
    /// 提交号（短）。
    Hash,
}

impl LogColumn {
    /// 全部的列，菜单里的顺序。
    pub(crate) const ALL: [LogColumn; 3] = [LogColumn::Author, LogColumn::Date, LogColumn::Hash];

    /// 记进状态文件的名字。
    fn key(self) -> &'static str {
        match self {
            LogColumn::Author => "author",
            LogColumn::Date => "date",
            LogColumn::Hash => "hash",
        }
    }

    /// 菜单里的文字。
    pub(crate) fn label(self) -> &'static str {
        match self {
            LogColumn::Author => t!("menu-log-column-author"),
            LogColumn::Date => t!("menu-log-column-date"),
            LogColumn::Hash => t!("menu-log-column-hash"),
        }
    }
}

/// 作者一列可以拖到的最窄、最宽（逻辑像素）：最窄只放得下头像和几个字，最宽约为默认宽度的
/// 两倍，再宽说明一列就太挤了。
const AUTHOR_RANGE: (f32, f32) = (60.0, 300.0);

impl App {
    /// 日志列表是否显示 `column` 一列；没有选过时显示作者和日期。
    pub(crate) fn log_column_shown(&self, column: LogColumn) -> bool {
        match &self.state.log_columns {
            Some(shown) => shown.iter().any(|c| c == column.key()),
            None => column != LogColumn::Hash,
        }
    }

    /// 显示或隐藏 `column` 一列；所有窗口共用，下次启动沿用。
    pub(in crate::app) fn toggle_log_column(&mut self, column: LogColumn) {
        let on = !self.log_column_shown(column);
        let mut shown: Vec<String> = (LogColumn::ALL.into_iter())
            .filter(|&c| {
                if c == column {
                    on
                } else {
                    self.log_column_shown(c)
                }
            })
            .map(|c| c.key().to_owned())
            .collect();
        shown.dedup();
        self.update_state(|state| state.log_columns = Some(shown));
        self.request_redraw();
    }

    /// 标题栏「⋯」弹出的菜单：各列，显示着的打勾。
    pub(in crate::app) fn log_columns_menu(&self) -> Vec<MenuEntry> {
        let item = |c| MenuEntry::Item(MenuCommand::LogColumn(c), true);
        LogColumn::ALL.into_iter().map(item).collect()
    }

    /// 作者名字的显示程度（0 到 1），列表宽 `list_w`（逻辑像素）：拖过宽度以后总是显示；否则
    /// 在 [`AUTHOR_MIN_LIST`] 以下 [`AUTHOR_BLEND`] 的范围内从仅头像逐渐展宽为头像加名字，名字
    /// 同步淡入，拖动分隔线或缩放窗口时名字不会在某个宽度突然出现或消失。
    pub(crate) fn log_author_show(&self, list_w: f32) -> f32 {
        if self.git.log.author_w.is_some() {
            return 1.0;
        }
        ((list_w - (AUTHOR_MIN_LIST - AUTHOR_BLEND)) / AUTHOR_BLEND).clamp(0.0, 1.0)
    }

    /// 按下作者一列的左边缘：双击恢复默认宽度，否则从画着的宽度开始拖动。
    pub(in crate::app) fn press_log_author_edge(&mut self, x: f32, double: bool) {
        if double {
            self.git.log.author_w = None;
            self.request_redraw();
            return;
        }
        let full = self.git.log.author_w.unwrap_or(AUTHOR_DEFAULT);
        let shown = log_author_width(full, self.log_author_show(self.log_parts().list.w));
        self.pointer.drag = crate::app::Drag::LogColumn {
            start_x: x,
            start_width: shown,
        };
    }

    /// 是否正在拖动作者一列的左边缘。
    pub(crate) fn log_column_dragging(&self) -> bool {
        matches!(self.pointer.drag, crate::app::Drag::LogColumn { .. })
    }

    /// 拖着作者一列的左边缘到横坐标 `x`：往左拖变宽，限制在 [`AUTHOR_RANGE`] 之内。
    pub(in crate::app) fn drag_log_author_edge(&mut self, x: f32, start: (f32, f32)) {
        let (start_x, start_width) = start;
        let (min, max) = AUTHOR_RANGE;
        self.git.log.author_w = Some((start_width - (x - start_x)).clamp(min, max));
        self.request_redraw();
    }
}

/// 作者一列默认的宽度（逻辑像素）。
pub(crate) const AUTHOR_DEFAULT: f32 = 150.0;
/// 作者一列完整显示头像和名字所需的最小列表宽度（逻辑像素）。
///
/// 列表窄于 `AUTHOR_MIN_LIST - AUTHOR_BLEND` 时作者一列只显示头像（`AVATAR_W` 宽），仍可辨认
/// 提交者；宽度在该值与 `AUTHOR_MIN_LIST` 之间时，从仅头像逐渐展开为头像加名字。
pub(crate) const AUTHOR_MIN_LIST: f32 = 620.0;
/// 作者一列从仅头像过渡到头像加名字的列表宽度范围（逻辑像素）。
pub(crate) const AUTHOR_BLEND: f32 = 120.0;
/// 仅显示头像时作者一列的宽度（逻辑像素）。
pub(crate) const AVATAR_W: f32 = 26.0;

/// 作者一列完全展开时宽 `full`、名字显示程度为 `show` 时画出来的宽度（逻辑像素）。
pub(crate) fn log_author_width(full: f32, show: f32) -> f32 {
    AVATAR_W + (full - AVATAR_W) * show
}
