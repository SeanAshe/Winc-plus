//! PDF 预览右下角工具栏的自动隐藏：打开文档时显示，空闲 [`TOOLS_IDLE`] 后淡出，指针移到工具栏
//! 附近（[`TOOLS_REACH`]）时淡入，停在附近期间一直显示。

use std::time::{Duration, Instant};

use winc_render::Rect;

use crate::app::App;
use crate::app::animation::{Clock, POPUP_IN, POPUP_OUT};

/// 指针离开工具栏附近后，工具栏保留的时长（与常见 PDF 阅读器相同）。
const TOOLS_IDLE: Duration = Duration::from_secs(2);
/// 工具栏附近的范围：工具栏向左、向上各外扩的距离（逻辑像素，约为工具栏高度的两倍），向右、
/// 向下到预览区域的边缘。
const TOOLS_REACH: f32 = 72.0;

/// 右下角工具栏的显示状态。
#[derive(Debug, Default)]
pub(crate) struct Tools {
    /// 上一帧工具栏的位置（窗口坐标），隐藏时也照常更新；没有工具栏时为空矩形。
    pub(crate) rect: Rect,
    /// 是否显示；淡出途中为假，按钮不响应点击。
    pub(crate) shown: bool,
    /// 到这一时刻隐藏；指针在工具栏附近时顺延。
    until: Option<Instant>,
    /// 淡入（真）或淡出的过渡。
    pub(crate) motion: Option<(bool, Clock)>,
}

impl Tools {
    /// 返回这一帧工具栏的不透明度（0 到 1）。
    pub(crate) fn opacity(&self) -> f32 {
        match self.motion {
            Some((true, clock)) => clock.progress(),
            Some((false, clock)) => 1.0 - clock.progress(),
            None if self.shown => 1.0,
            None => 0.0,
        }
    }

    /// 窗口坐标 `(x, y)` 是否在工具栏附近；`area` 是预览区域。
    fn near(&self, area: Rect, (x, y): (f32, f32)) -> bool {
        let rect = self.rect;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return false;
        }
        let (left, top) = (rect.x - TOOLS_REACH, rect.y - TOOLS_REACH);
        Rect::new(left, top, area.right() - left, area.bottom() - top).contains(x, y)
    }
}

impl App {
    /// 显示 PDF 预览的工具栏，[`TOOLS_IDLE`] 后隐藏。
    pub(crate) fn reveal_pdf_tools(&mut self) {
        let until = Instant::now() + TOOLS_IDLE;
        self.preview.pdf.tools.until = Some(until);
        if !self.preview.pdf.tools.shown {
            let clock = self.clock(POPUP_IN);
            let tools = &mut self.preview.pdf.tools;
            tools.shown = true;
            tools.motion = clock.map(|clock| (true, clock));
            self.request_redraw();
        }
    }

    /// 指针移到 `(x, y)`（窗口坐标）：在 PDF 预览的工具栏附近时显示工具栏。
    pub(crate) fn track_pdf_tools(&mut self, x: f32, y: f32) {
        let pdf = &self.preview.pdf;
        if pdf.doc.is_some() && pdf.tools.near(pdf.area, (x, y)) {
            self.reveal_pdf_tools();
        }
    }

    /// 到时隐藏 PDF 预览的工具栏，指针仍在工具栏附近时顺延；返回下次需要检查的时刻。
    pub(crate) fn pdf_tools_tick(&mut self, now: Instant) -> Option<Instant> {
        let until = self.preview.pdf.tools.until?;
        if now < until {
            return Some(until);
        }
        let pdf = &self.preview.pdf;
        if self.pointer.cursor_inside && pdf.tools.near(pdf.area, self.pointer.mouse) {
            let until = now + TOOLS_IDLE;
            self.preview.pdf.tools.until = Some(until);
            return Some(until);
        }
        let clock = self.clock(POPUP_OUT);
        let tools = &mut self.preview.pdf.tools;
        tools.until = None;
        tools.shown = false;
        tools.motion = clock.map(|clock| (false, clock));
        self.request_redraw();
        None
    }
}
