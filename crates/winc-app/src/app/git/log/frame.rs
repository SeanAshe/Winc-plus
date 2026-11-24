//! 日志视图在绘制时更新的状态：拆分后每个窗格各有一份，换前台窗格时与 [`LogState`] 交换。

use super::*;

/// 一个日志视图的过渡、悬停提示和详情选区等绘制时更新的状态（字段含义见 [`LogState`]）。
#[derive(Default)]
pub(crate) struct LogFrame {
    pills: crate::app::StatusMotion<LogPill>,
    details_names: crate::app::TreeLabels<RowName>,
    panel_names: crate::app::TreeLabels<RowName>,
    branch_names: crate::app::TreeLabels<RowName>,
    shown: Option<String>,
    fade: Option<crate::app::animation::Clock>,
    clipped: Option<usize>,
    long_files: Vec<usize>,
    long_branches: Vec<usize>,
    runs: Vec<TextRun>,
    text_sel: Option<(TextPoint, TextPoint)>,
    details_max: f32,
    branches_max: f32,
    author_fit: Option<(AuthorFitKey, f32)>,
    date_fit: Option<((u32, winc_i18n::Locale), f32)>,
}

impl std::fmt::Debug for LogFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogFrame")
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

impl LogState {
    /// 与 `other` 交换日志视图在绘制时更新的状态（见 [`LogFrame`]）。
    pub(crate) fn exchange_frame(&mut self, other: &mut LogFrame) {
        use std::mem::swap;
        swap(&mut self.pills, &mut other.pills);
        swap(&mut self.details_names, &mut other.details_names);
        swap(&mut self.panel_names, &mut other.panel_names);
        swap(&mut self.branch_names, &mut other.branch_names);
        swap(&mut self.shown, &mut other.shown);
        swap(&mut self.fade, &mut other.fade);
        swap(&mut self.clipped, &mut other.clipped);
        swap(&mut self.long_files, &mut other.long_files);
        swap(&mut self.long_branches, &mut other.long_branches);
        swap(&mut self.runs, &mut other.runs);
        swap(&mut self.text_sel, &mut other.text_sel);
        swap(&mut self.details_max, &mut other.details_max);
        swap(&mut self.branches_max, &mut other.branches_max);
        swap(&mut self.author_fit, &mut other.author_fit);
        swap(&mut self.date_fit, &mut other.date_fit);
    }
}
