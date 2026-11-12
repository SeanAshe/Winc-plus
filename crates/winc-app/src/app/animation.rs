//! 动画共用的时钟。
//!
//! 侧栏开关、目录展开收起、代码折叠展开、`⌘P` 面板高度变化和标签滑动都用 [`Clock`] 计算进度；
//! 设置浮层、`⌘P` 面板和跳转条等输入条的出现、消失使用同一条曲线（[`dialog_curve`]、
//! [`CardFx`]）。动画结束后由 [`App::settle_animations`] 统一清除。
//! 文件树工具栏的收起动画目标随宽度连续变化，不使用时钟，见 `tool_fold.rs`；
//! 开关 Git、终端模块时随之出现、消失的界面元素见 `presence.rs`。

use std::time::{Duration, Instant};

use super::*;

/// 侧栏展开、收起的动画时长（取自调参）。
pub(super) const SIDEBAR_ANIMATION: Duration = Duration::from_millis(200);
/// 目录展开、收起的动画时长（取自调参）。
pub(super) const TREE_ANIMATION: Duration = Duration::from_millis(160);
/// 编辑区跳转式滚动（翻页、跳到行、查找跳转）的平滑滚动时长。
///
/// 取自调参：该时长足以看清滚动方向，连续翻页时也不会明显滞后于按键。
pub(super) const SCROLL_GLIDE: Duration = Duration::from_millis(125);
/// 差异跳转到一处改动时该段改动的高亮时长。
///
/// 长于 [`SCROLL_GLIDE`]：滚动结束后高亮仍持续约 475 ms，跳转距离很近时同样能看清落点。
pub(super) const DIFF_PULSE: Duration = Duration::from_millis(600);
/// 代码折叠、展开的动画时长，取 [`TREE_ANIMATION`]，使两种展开收起的节奏统一。
pub(super) const FOLD_ANIMATION: Duration = TREE_ANIMATION;
/// `⌘P` 面板高度变化的动画时长，取 [`TREE_ANIMATION`]。
pub(super) const PICKER_ANIMATION: Duration = TREE_ANIMATION;
/// 标签滑动的动画时长（取自调参）：用于为拖动中的标签让位，以及松开后标签移到目标位置。
pub(super) const TAB_SLIDE: Duration = Duration::from_millis(150);
/// 元素消失时内容淡出的时长。
///
/// 取自调参：内容先在原宽度内淡出，之后宽度收起，收起途中不显示被部分裁去的文字或图标。
pub(super) const EXIT_FADE: Duration = Duration::from_millis(70);
/// 元素出现时内容开始淡入的显示比例（0 到 1）。
///
/// 取自调参：宽度按缓出曲线展开，达到该比例时约经过三分之一的时长，此后被裁去的内容很少。
const REVEAL_FROM: f32 = 0.7;
/// 日志详情切换到另一个提交时内容淡入的时长。
///
/// 取自调参：该淡入仅用于缓和内容的突然替换，因此短于视图切换的动画。
pub(super) const DETAILS_FADE: Duration = Duration::from_millis(120);
/// Markdown 预览的标题列表展开的时长，取 [`TREE_ANIMATION`]。
///
/// 展开时卡片淡入，横线从指示条滑到各行。
pub(super) const TOC_FADE: Duration = TREE_ANIMATION;
/// 在 Markdown 预览的标题列表中点击标题后，正文中该标题底色淡出的时长。
///
/// 取自调参：取 1.2 秒时，滚动结束后仍能看清被标出的标题，底色也不会长时间停留。
pub(super) const TOC_FLASH: Duration = Duration::from_millis(1200);
/// 切换侧栏视图时，视图切换中选中段滑到新位置的时长，取 [`TAB_SLIDE`]。
pub(super) const VIEW_SLIDE: Duration = TAB_SLIDE;
/// 设置中开关切换时圆钮滑动、底色渐变的时长，取 [`TAB_SLIDE`]。
///
/// macOS 系统开关的动画约为 0.2 秒；此处的开关更小，滑动距离更短，因此时长略短。
pub(super) const SWITCH_SLIDE: Duration = TAB_SLIDE;
/// 位置浮窗（定义、用法、书签）出现的动画时长（取自调参）：浮窗从名字一侧滑出并淡入。
pub(super) const POPUP_IN: Duration = Duration::from_millis(150);
/// 位置浮窗消失的动画时长（取自调参）：短于 [`POPUP_IN`]，浮窗在原位置淡出。
pub(super) const POPUP_OUT: Duration = Duration::from_millis(110);
/// 可编辑的预览重排后各块滑到新位置、新显示的标记淡入的时长，取 [`TREE_ANIMATION`]。
pub(crate) const MARKDOWN_MOTION: Duration = TREE_ANIMATION;
/// PDF 预览与图片预览缩放、PDF 缩略图栏滑入滑出的时长，取 [`TREE_ANIMATION`]。
pub(crate) const PDF_MOTION: Duration = TREE_ANIMATION;
/// 设置浮层出现的时长（取自调参）：卡片面积远大于小浮窗，时长相应加长。
pub(super) const DIALOG_IN: Duration = Duration::from_millis(220);
/// 设置浮层消失的时长（取自调参），短于 [`DIALOG_IN`]。
pub(super) const DIALOG_OUT: Duration = Duration::from_millis(150);

/// 返回浮层卡片（设置、`⌘P` 面板、输入条）出现、消失途中的（不透明度，缩放）。
///
/// 出现时整体淡入并从 95% 放大到原大，消失时整体淡出并略微缩小。
pub fn dialog_curve(opening: bool, clock: Clock) -> (f32, f32) {
    let t = clock.progress();
    if opening {
        (t, 0.95 + 0.05 * t)
    } else {
        (1.0 - t, 1.0 - 0.02 * t)
    }
}

/// `⌘P` 面板、输入条（跳转到行、新建分支等）出现和消失的动画，使用 [`dialog_curve`]。
///
/// 关闭的途径较多（`Esc`、回车、点击外部、焦点移走），因此不在各处记录，而由
/// [`App::track_card`] 每帧绘制时与上一帧比较：打开时淡入，关闭时用最后一帧保存的卡片淡出。
#[derive(Debug, Default)]
pub struct CardFx {
    /// 上一帧绘制时是否处于打开状态。
    shown: bool,
    /// 出现（`true`）或消失的动画；不在动画中时为 `None`。
    pub motion: Option<(bool, Clock)>,
    /// 最近一帧绘制的卡片（位置为逻辑像素，不含阴影），用于关闭后的淡出。
    pub shot: Option<(winc_render::Rect, winc_render::Snapshot)>,
    /// 淡入途中存放卡片下方内容（连同阴影）的缓冲，每帧复用；淡入结束后释放。
    pub base: Option<winc_render::Snapshot>,
}

impl CardFx {
    /// 卡片已关闭且正在淡出时，返回动画时钟、卡片位置和最后一帧的卡片画面。
    pub fn closing(&self) -> Option<(Clock, winc_render::Rect, &winc_render::Snapshot)> {
        match (self.motion, &self.shot) {
            (Some((false, clock)), Some((rect, shot))) if !self.shown => Some((clock, *rect, shot)),
            _ => None,
        }
    }

    fn settle(&mut self) {
        if self.motion.is_some_and(|(_, clock)| clock.done()) {
            self.motion = None;
        }
        if self.motion.is_none() {
            self.base = None;
        }
        if !self.shown && self.motion.is_none() {
            self.shot = None;
        }
    }
}

/// 浮层卡片的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    /// `⌘P` 面板（文件、命令、符号）。
    ///
    /// 不含位置浮窗，位置浮窗使用单独的动画。
    Picker,
    /// 编辑区顶部的输入条（跳转到行、新建分支、储藏等）。
    Bar,
    /// 窗口正中的功能对话框（新建工作树）。
    Dialog,
    /// 窗口内绘制的提示框（平台没有系统提示框时使用）。
    Alert,
}

impl Card {
    /// 返回卡片的圆角半径：输入条和面板用较小的 `RADIUS_BAR`，对话框和提示框用 `RADIUS_CARD`。
    pub fn radius(self) -> f32 {
        match self {
            Card::Picker | Card::Bar => crate::theme::RADIUS_BAR,
            Card::Dialog | Card::Alert => crate::theme::RADIUS_CARD,
        }
    }
}

/// 一段动画的时钟：从开始起经过 `duration` 结束，进度按三次缓出（先快后慢）计算。
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    start: Instant,
    duration: Duration,
    /// 测试中固定的已用时间，使截图每次都相同。
    #[cfg(test)]
    frozen: Option<Duration>,
}

impl Clock {
    /// 创建从当前时刻开始、时长为 `duration` 的时钟。
    pub(super) fn new(duration: Duration) -> Clock {
        Clock {
            start: Instant::now(),
            duration,
            #[cfg(test)]
            frozen: None,
        }
    }

    /// 返回已用时间固定为 `at` 的时钟，测试中的进度不随实际时间变化。
    #[cfg(test)]
    pub(crate) fn stopped(duration: Duration, at: Duration) -> Clock {
        Clock {
            frozen: Some(at),
            ..Clock::new(duration)
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(test)]
        if let Some(at) = self.frozen {
            return at;
        }
        self.start.elapsed()
    }

    /// 返回应用缓出曲线之后的进度：0 表示刚开始，1 表示已结束。
    pub fn progress(&self) -> f32 {
        1.0 - (1.0 - self.linear()).powi(3)
    }

    /// 返回未加缓动的进度（经过的时间占时长的比例）：0 表示刚开始，1 表示已结束。
    ///
    /// 需要其他曲线时（例如消失时先慢后快）由调用方在此基础上计算。
    pub fn linear(&self) -> f32 {
        (self.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    }

    /// 返回从 `from` 过渡到 `to` 的动画在当前进度下的值。
    pub fn between(&self, from: f32, to: f32) -> f32 {
        from + (to - from) * self.progress()
    }

    /// 返回滚动位置（显示行）从 `from` 过渡到 `to` 在当前进度下的值。
    pub fn between_rows(&self, from: f64, to: f64) -> f64 {
        from + (to - from) * f64::from(self.progress())
    }

    /// 返回淡出途中的不透明度：按未加缓动的进度从 1 平滑降到 0。
    pub fn fade_out(&self) -> f32 {
        1.0 - stage(self.linear(), 0.0, 1.0)
    }

    /// 返回先淡入、停留、再淡出途中的不透明度，淡入和淡出各用 `edge`。
    pub fn pulse(&self, edge: Duration) -> f32 {
        let edge = (edge.as_secs_f32() / self.duration.as_secs_f32()).min(0.5);
        let t = self.linear();
        stage(t, 0.0, edge) * (1.0 - stage(t, 1.0 - edge, 1.0))
    }

    /// 创建与 `self` 同时开始、时长为 `duration` 的时钟。
    pub fn lasting(self, duration: Duration) -> Clock {
        Clock { duration, ..self }
    }

    /// 创建在 `self` 结束时开始、时长为 `duration` 的时钟；开始之前进度为 0。
    pub fn then(self, duration: Duration) -> Clock {
        Clock {
            start: self.start + self.duration,
            duration,
            #[cfg(test)]
            frozen: self.frozen.map(|at| at.saturating_sub(self.duration)),
        }
    }

    /// 返回时长。
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// 是否已经过 `duration`，即动画已结束。
    pub fn done(&self) -> bool {
        self.elapsed() >= self.duration
    }
}

/// 返回 `t` 在 `start` 到 `end` 区间内的平滑过渡值，0 到 1。
///
/// `t` 不大于 `start` 时为 0，不小于 `end` 时为 1，区间两端的变化率为 0。
pub fn stage(t: f32, start: f32, end: f32) -> f32 {
    let x = ((t - start) / (end - start)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// 返回显示比例为 `share` 的元素中内容的不透明度，0 到 1。
///
/// 比例小于 [`REVEAL_FROM`] 时内容不显示，此后平滑淡入，展开途中内容不与相邻元素重叠。
pub fn reveal(share: f32) -> f32 {
    stage(share, REVEAL_FROM, 1.0)
}

impl App {
    /// 在绘制 `card` 之前与上一帧的状态比较：刚打开（`open`）时开始淡入，刚关闭时开始淡出。
    pub(crate) fn track_card(&mut self, card: Card, open: bool) {
        let shown = self.card_fx(card).shown;
        if shown != open {
            let duration = if open { DIALOG_IN } else { DIALOG_OUT };
            let motion = self.clock(duration).map(|clock| (open, clock));
            let fx = self.card_fx(card);
            fx.shown = open;
            fx.motion = motion;
            if open {
                fx.shot = None;
            }
        }
    }

    /// 返回 `card` 的动画状态。
    pub(crate) fn card_fx(&mut self, card: Card) -> &mut CardFx {
        match card {
            Card::Picker => &mut self.picker.card,
            Card::Bar => &mut self.bar_card,
            Card::Dialog => &mut self.dialog_card,
            Card::Alert => &mut self.alert_card,
        }
    }

    /// 创建一段时长为 `duration` 的动画时钟，并按窗口所在显示器更新帧间隔 `frame_interval`。
    ///
    /// 没有窗口，或动态效果已关闭（见 [`App::animations_on`]）时返回 `None`，调用方应直接跳到
    /// 动画终点。
    pub(super) fn clock(&mut self, duration: Duration) -> Option<Clock> {
        let window = self.window.handle.as_ref()?;
        if !self.animations_on() {
            return None;
        }

        self.frame_interval = frame_interval(window);
        Some(Clock::new(duration))
    }

    /// 是否播放过渡动画：以设置（`ui.animations`）为准，没有设置时跟随系统的「减弱动态效果」。
    ///
    /// 系统的设置只在 macOS 上读取，测试中视为未开启，使结果不随运行测试的机器变化。
    pub(crate) fn animations_on(&self) -> bool {
        let reduced = !cfg!(test) && system_reduces_motion();
        self.config.ui_animations.unwrap_or(!reduced)
    }

    /// 清除已经结束的动画。
    ///
    /// 每帧绘制之前调用，使动画的最后一帧按终点状态绘制。
    pub(super) fn settle_animations(&mut self) {
        fn settle<T>(slot: &mut Option<T>, clock: impl Fn(&T) -> Clock) {
            if slot.as_ref().is_some_and(|m| clock(m).done()) {
                *slot = None;
            }
        }

        settle(&mut self.sidebar.motion, |m| m.clock);
        settle(&mut self.tree.motion, |m| m.clock);
        self.settle_log_tree();
        self.tree.labels.settle();
        self.commit.names.settle();
        settle(&mut self.commit.shift, |s| s.clock);
        settle(&mut self.commit.actions_slide, |s| s.1);
        settle(&mut self.commit.motion, |m| m.clock);
        settle(&mut self.commit.message_motion, |m| m.1);
        settle(&mut self.commit.button_motion, |m| m.1);
        settle(&mut self.commit.amend_motion, |m| m.1);
        settle(&mut self.commit.stash_motion, |m| m.1);
        settle(&mut self.outline.motion, |m| m.1);
        settle(&mut self.outline.fold, |f| f.clock);
        settle(&mut self.markdown.toc_fade, |c| *c);
        settle(&mut self.markdown.toc_flash, |f| f.1);
        settle(&mut self.markdown.anchor_flash, |f| f.1);
        settle(&mut self.commit.card.motion, |m| m.1);
        settle(&mut self.commit.undo.motion, |m| m.1);
        settle(&mut self.global.field_motion, |m| m.2);
        settle(&mut self.global.motion, |m| m.clock);
        settle(&mut self.fold_motion, |m| m.clock);
        self.finish_diff_fold(false);
        settle(&mut self.scroll_glide, |g| g.clock);
        settle(&mut self.picker.motion, |m| m.clock);
        settle(&mut self.pdf.zoom_motion, |m| m.clock);
        settle(&mut self.picture_view.zoom_motion, |m| m.clock);
        settle(&mut self.pdf.strip_motion, |m| m.1);
        settle(&mut self.picker.fold, |f| f.clock);
        if self.picker.places_in.is_some_and(|c| c.done()) {
            self.picker.places_in = None;
        }
        if self.branch.finished.is_some_and(|c| c.done()) {
            self.branch.finished = None;
        }
        settle(&mut self.picker.places_out, |g| g.clock);
        if self.notice.entering.is_some_and(|c| c.done()) {
            self.notice.entering = None;
        }
        settle(&mut self.notice.leaving, |g| g.clock);
        settle(&mut self.diff.fade, |f| f.clock);
        if self.diff.pulse.is_some_and(|c| c.done()) {
            self.diff.pulse = None;
        }
        settle(&mut self.sidebar.view_slide, |s| s.1);
        settle(&mut self.settings.motion, |m| m.1);
        self.picker.card.settle();
        self.bar_card.settle();
        self.dialog_card.settle();
        self.alert_card.settle();
        if self.settings.motion.is_none() {
            self.settings.cache = None;
        }
        settle(&mut self.settings.mode_slide, |s| s.1);
        settle(&mut self.settings.switch_slide, |s| s.1);
        settle(&mut self.log.fade, |c| *c);
        self.settle_hint();
        self.tabs.slides.retain(|s| !s.clock.done());
        self.tabs.closing.retain(|g| !g.clock.done());
        self.tabs.opening.retain(|(_, c)| !c.done());
        self.terminal.strip.settle();
        settle(&mut self.terminal.motion, |m| m.clock);
        settle(&mut self.terminal.grow, |g| g.1);
        settle(&mut self.markdown.motion, |m| m.clock);
        self.status.motion.settle();
    }

    /// 一帧绘制完成后，如果仍有动画在进行，则安排下一帧。
    ///
    /// 存在显示链接时，由显示链接在下一次屏幕刷新时唤醒界面；否则按计时器，
    /// 在本帧开始时刻 `started` 之后经过一个 `frame_interval` 唤醒。
    pub(super) fn schedule_frame(&mut self, started: Instant) {
        let animating = self.animating();
        #[cfg(any(target_os = "macos", windows))]
        if let Some(link) = &mut self.display_link {
            link.set_running(animating);
            self.next_frame = None;
            return;
        }
        self.next_frame = animating.then(|| started + self.frame_interval);
    }

    /// 本帧未能绘制时（呈现缓冲均被合成器占用，见 `Renderer::draw`）安排重绘。
    ///
    /// 不立即重试，避免界面线程空转等待。存在显示链接时由显示链接在下一次刷新时请求重绘；
    /// 显示链接没有回调时（窗口被遮挡），由计时器在两个 `frame_interval` 后重试，
    /// 绘制成功后 [`Self::schedule_frame`] 会取消该计时器。
    pub(super) fn retry_frame(&mut self, started: Instant) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(link) = &mut self.display_link {
            link.set_running(true);
            self.next_frame = Some(started + self.frame_interval * 2);
            return;
        }
        self.next_frame = Some(started + self.frame_interval);
    }

    /// 是否有动画正在进行，包括滚动条的淡入、淡出和变色。
    pub(super) fn animating(&self) -> bool {
        self.sidebar.motion.is_some()
            || self.tree.motion.is_some()
            || self.log_tree_animating()
            || self.tree.labels.moving()
            || self.commit.names.moving()
            || self.commit.actions_slide.is_some()
            || self.commit.shift.is_some()
            || self.commit.motion.is_some()
            || self.commit.message_motion.is_some()
            || self.commit.button_motion.is_some()
            || self.commit.amend_motion.is_some()
            || self.commit.stash_motion.is_some()
            || self.outline.motion.is_some()
            || self.outline.fold.is_some()
            || self.markdown.toc_fade.is_some()
            || self.markdown.toc_flash.is_some()
            || self.markdown.anchor_flash.is_some()
            || self.commit.card.motion.is_some()
            || self.commit.undo.motion.is_some()
            || self.global.field_motion.is_some()
            || self.global.motion.is_some()
            || self.drag_autoscrolling()
            || self.global.rows_moving()
            || self.fold_motion.is_some()
            || self.diff_fold.is_some()
            || self.scroll_glide.is_some()
            || self.wheel.glide.is_some()
            || self.picker.motion.is_some()
            || self.pdf.zoom_motion.is_some()
            || self.picture_view.zoom_motion.is_some()
            || self.pdf.strip_motion.is_some()
            || self.picker.fold.is_some()
            || self.picker.places_in.is_some()
            || self.picker.places_out.is_some()
            || self.notice.entering.is_some()
            || self.notice.leaving.is_some()
            || self.diff.fade.is_some()
            || self.diff.pulse.is_some()
            || self.sidebar.view_slide.is_some()
            || self.settings.motion.is_some()
            || self.picker.card.motion.is_some()
            || self.bar_card.motion.is_some()
            || self.dialog_card.motion.is_some()
            || self.alert_card.motion.is_some()
            || self.settings.mode_slide.is_some()
            || self.settings.switch_slide.is_some()
            || self.log.fade.is_some()
            || self.hint_animating()
            || !self.tabs.slides.is_empty()
            || !self.tabs.closing.is_empty()
            || !self.tabs.opening.is_empty()
            || self.terminal.strip.animating()
            || self.terminal.motion.is_some()
            || self.terminal.grow.is_some()
            || self.module_fx.moving()
            || self.tool_fold.moving()
            || self.scrollbars_animating()
            || self.markdown.motion.is_some()
            || self.status.motion.moving()
            || ((self.branch.running.is_some()
                || self.branch.finished.is_some()
                || self.commit_refreshing())
                && self.commit_shown())
    }
}

mod presence;
pub use presence::*;

/// 工具栏按钮悬停提示的计时状态。
#[derive(Default)]
pub struct Tooltip {
    /// 鼠标开始悬停在工具栏按钮上的时刻；悬停足够久后才显示提示。
    pub(crate) since: Option<Instant>,
    /// 提示是否已绘制出来；已绘制时无需再为提示唤醒事件循环。
    pub(crate) shown: bool,
}

/// 光标闪烁的状态：当前是否显示，以及闪烁周期的起点。
pub struct Blink {
    /// 是否绘制光标：处于闪烁周期的显示阶段时为真；窗口没有焦点时为假。
    pub(crate) visible: bool,
    /// 光标闪烁周期的起点；按键、点击等操作会重置该起点，使光标立即显示。
    pub(in crate::app) epoch: Instant,
}

impl Default for Blink {
    fn default() -> Self {
        Self {
            visible: true,
            epoch: Instant::now(),
        }
    }
}

/// 系统是否开启了「减弱动态效果」；没有读取该设置的平台上为否。
fn system_reduces_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests;
