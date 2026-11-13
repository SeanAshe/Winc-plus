//! 跳转过渡与绘制时的像素对齐。
//!
//! 编辑区与各区域的字段都保存目标位置。绘制前 [`App::begin_scroll_frame`] 换入此刻屏幕上应显示的
//! 位置并对齐到逻辑像素，绘制后 [`App::end_scroll_frame`] 换回目标位置：连续的跳转都按目标计算，
//! 绘制与命中测试使用同一个位置。

use super::*;

use crate::app::animation::SCROLL_GLIDE;
use crate::app::git::commit::COMMIT_ROW;
use crate::app::scroll::editor::GLIDE_MIN_ROWS;

/// 只保存一个竖直滚动位置的区域，见 [`App::pane_scroll`]；绘制前逐一对齐。
const PANES: [ScrollTarget; 9] = [
    ScrollTarget::Picker,
    ScrollTarget::Tabs,
    ScrollTarget::Commit,
    ScrollTarget::Outline,
    ScrollTarget::Results,
    ScrollTarget::Tree,
    ScrollTarget::LogDetails,
    ScrollTarget::Markdown,
    ScrollTarget::Pdf,
];

/// 区域的跳转过渡：区域的滚动位置已经是目标，绘制时按时钟从 `from` 插值到目标。
#[derive(Debug, Clone)]
pub(crate) struct PaneGlide {
    /// 正在滚动的区域。
    pub(crate) target: ScrollTarget,
    /// 开始时屏幕上的滚动位置（逻辑像素）。
    pub(crate) from: f32,
    /// 本次过渡的时钟。
    pub(crate) clock: Clock,
}

/// 一帧绘制期间当前文档换入的滚动位置。
struct EditorFrame {
    /// 换入时的当前文档。
    doc: DocumentId,
    /// 纵向的（目标，换入值）（显示行）。
    row: (f64, f64),
    /// 横向的（目标，换入值）（逻辑像素）。
    x: (f32, f32),
}

/// 一帧绘制期间换入的滚动位置，绘制后由 [`App::end_scroll_frame`] 换回。
pub(in crate::app) struct ScrollFrame {
    /// 当前文档换入的位置；没有打开文档时为 `None`。
    editor: Option<EditorFrame>,
    /// 与 [`PANES`] 一一对应的（目标，换入值）（逻辑像素）；位置未变的区域为 `None`。
    panes: [Option<(f32, f32)>; PANES.len()],
}

/// 返回把滚动位置 `row`（显示行）对齐到逻辑像素后的值；`lh` 为行高（逻辑像素）。
///
/// 文字的基线按逻辑像素取整，底色、选区和光标按物理像素取整。缩放系数为整数时，滚动位置落在
/// 逻辑像素上，两者在滚动中保持相同的相对位置。
fn snap_rows(row: f64, lh: f32) -> f64 {
    let lh = f64::from(lh);
    if lh <= 0.0 {
        return row;
    }
    (row * lh).round() / lh
}

impl App {
    /// 在区域 `target` 的滚动位置从 `from`（逻辑像素）变为当前值后调用，距离较远时启动跳转过渡。
    ///
    /// 距离不小于 [`GLIDE_MIN_ROWS`] 行（按该区域的行高换算）时过渡，否则直接到达；同一区域已在
    /// 过渡时从屏幕上的当前位置继续。动态效果关闭或没有窗口时不播放动画。
    pub(in crate::app) fn glide_pane(&mut self, target: ScrollTarget, from: f32) {
        let Some(to) = self.pane_scroll(target) else {
            return;
        };
        let existing = self
            .scroll
            .pane_glides
            .iter()
            .position(|g| g.target == target);
        let shown = existing.map_or(from, |i| {
            let glide = &self.scroll.pane_glides[i];
            glide.clock.between(glide.from, from)
        });
        if (to - shown).abs() < GLIDE_MIN_ROWS as f32 * self.pane_row(target) {
            // 距离较近时直接到达；进行中的过渡保留，继续滚向新目标。
            return;
        }
        let Some(clock) = self.clock(SCROLL_GLIDE) else {
            return;
        };
        if let Some(i) = existing {
            self.scroll.pane_glides.swap_remove(i);
        }
        self.scroll.pane_glides.push(PaneGlide {
            target,
            from: shown,
            clock,
        });
        self.request_redraw();
    }

    /// 把区域 `target` 滚动到 `to`（逻辑像素，按 [`Self::set_pane_scroll`] 限制范围），距离较远时
    /// 启动跳转过渡（见 [`Self::glide_pane`]）。
    pub(in crate::app) fn glide_pane_to(&mut self, target: ScrollTarget, to: f32) {
        let Some(before) = self.pane_scroll(target) else {
            return;
        };
        self.set_pane_scroll(target, to);
        self.glide_pane(target, before);
    }

    /// 返回区域 `target` 一行的高度（逻辑像素），用于换算过渡的最小距离。
    fn pane_row(&self, target: ScrollTarget) -> f32 {
        match target {
            ScrollTarget::Picker => PICKER_ROW,
            ScrollTarget::Commit => COMMIT_ROW,
            ScrollTarget::Markdown | ScrollTarget::Pdf | ScrollTarget::LogDetails => {
                self.fonts.line_height
            }
            _ => TREE_ROW,
        }
    }

    /// 结束区域 `target` 的跳转过渡，滚动位置停在此刻屏幕上显示的位置。
    ///
    /// `target` 为编辑区时结束编辑区的平滑滚动（见 [`Self::settle_editor_glide`]）。
    pub(in crate::app) fn settle_glide_of(&mut self, target: ScrollTarget) {
        if let ScrollTarget::Editor(_) = target {
            self.settle_editor_glide();
            return;
        }
        let Some(i) = self
            .scroll
            .pane_glides
            .iter()
            .position(|g| g.target == target)
        else {
            return;
        };
        let glide = self.scroll.pane_glides.swap_remove(i);
        if let Some(to) = self.pane_scroll(target) {
            self.set_pane_raw(target, glide.clock.between(glide.from, to));
        }
    }

    /// 结束编辑区与各区域的全部跳转过渡，滚动位置停在此刻屏幕上显示的位置。
    ///
    /// 在按下鼠标、开始触摸、拖动滚动条时调用，使命中测试与屏幕上看到的内容一致。
    pub(in crate::app) fn settle_glides(&mut self) {
        self.settle_editor_glide();
        while let Some(glide) = self.scroll.pane_glides.first().map(|g| g.target) {
            self.settle_glide_of(glide);
        }
    }

    /// 绘制前调用：换入编辑区与各区域此刻应显示的滚动位置（过渡中的插值），并对齐到逻辑像素。
    ///
    /// 绘制后由 [`Self::end_scroll_frame`] 换回目标位置。正在折叠或展开时取消编辑区的平滑滚动，
    /// 因为折叠动画假定滚动位置不变。
    pub(in crate::app) fn begin_scroll_frame(&mut self) -> ScrollFrame {
        if self.motion.fold_motion.is_some() {
            self.scroll.glide = None;
        }
        let lh = self.fonts.line_height;
        let glide = self.scroll.glide.clone();
        let editor = self.editor.active_mut().map(|doc| {
            let target = doc.scroll_line;
            let shown = match &glide {
                Some(g) if g.doc == doc.id => g.shown(target),
                _ => target,
            };
            let row = snap_rows(shown, lh);
            set_scroll_line(doc, row);
            let x = (doc.scroll_x, doc.scroll_x.round());
            doc.scroll_x = x.1;
            EditorFrame {
                doc: doc.id,
                row: (target, row),
                x,
            }
        });

        let mut panes = [None; PANES.len()];
        for (slot, target) in panes.iter_mut().zip(PANES) {
            let Some(to) = self.pane_scroll(target) else {
                continue;
            };
            let glide = self.scroll.pane_glides.iter().find(|g| g.target == target);
            let shown = glide.map_or(to, |g| g.clock.between(g.from, to)).round();
            if shown != to {
                self.set_pane_raw(target, shown);
                *slot = Some((to, shown));
            }
        }
        ScrollFrame { editor, panes }
    }

    /// 绘制完一帧后换回目标位置，见 [`Self::begin_scroll_frame`]。
    ///
    /// 绘制期间位置被改写（例如绘制时按新的内容收紧了上限）时保留改写后的值，并结束该处的过渡。
    pub(in crate::app) fn end_scroll_frame(&mut self, frame: ScrollFrame) {
        if let Some(EditorFrame {
            doc: id,
            row: (target, row),
            x: (x, shown_x),
        }) = frame.editor
            && let Some(doc) = self.editor.active_mut().filter(|d| d.id == id)
        {
            if doc.scroll_line == row {
                set_scroll_line(doc, target);
            } else {
                self.scroll.glide = None;
            }
            if let Some(doc) = self.editor.active_mut()
                && doc.scroll_x == shown_x
            {
                doc.scroll_x = x;
            }
        }
        for (slot, target) in frame.panes.into_iter().zip(PANES) {
            let Some((to, shown)) = slot else {
                continue;
            };
            if self.pane_scroll(target) == Some(shown) {
                self.set_pane_raw(target, to);
            } else {
                self.scroll.pane_glides.retain(|g| g.target != target);
            }
        }
    }
}
