//! 随 Git、终端模块开关或切换标签而出现、消失的界面元素及其动画。
//!
//! 涉及侧栏视图切换中的「提交」一段、状态栏右端的终端开关和设置分类栏中的「终端」一项、状态栏
//! 左侧的分支名，以及标签栏右端「源代码 / 分栏 / 预览」三个按钮（仅可预览的文件显示）。触发
//! 变化的来源很多（开关模块、切换目录、仓库状态读取完成、切换标签），因此
//! [`App::track_modules`] 在每帧绘制前与上一帧比较：新出现的元素淡入并展开，消失的元素淡出并
//! 收起，相邻元素随之滑动。

use super::*;

/// 模块里的东西出现、消失的时长（经验值）：和侧栏开关差不多，比标签滑动慢一点，挪开的距离
/// 也比标签大。
pub(in crate::app) const MODULE_FADE: Duration = Duration::from_millis(180);

/// 界面元素的显示状态及其出现、消失动画。
///
/// 出现时宽度先展开，内容随后淡入（[`reveal`]）；消失时内容先在原宽度内淡出（[`EXIT_FADE`]），
/// 宽度随后收起。
#[derive(Debug, Default)]
pub struct Presence {
    /// 上一帧是否显示。
    shown: bool,
    /// 动画开始时的显示比例与宽度的时钟；不在动画中时为 `None`。
    ///
    /// 中途反向时从当时的比例继续。
    motion: Option<(f32, Clock)>,
    /// 消失时内容淡出的时钟；出现途中与不在动画中时为 `None`。
    fade: Option<Clock>,
}

impl Presence {
    /// 返回宽度的显示比例：0 表示不显示，1 表示完全显示。
    ///
    /// 不在动画中时取 `present`，即本帧是否应显示；启动后第一帧、测试中尚未与上一帧比较时
    /// 同样得到正确的比例。
    pub fn share(&self, present: bool) -> f32 {
        match self.motion {
            Some((from, clock)) => clock.between(from, if self.shown { 1.0 } else { 0.0 }),
            None => f32::from(u8::from(present)),
        }
    }

    /// 返回内容的不透明度，0 到 1；`present` 的含义同 [`Presence::share`]。
    pub fn opacity(&self, present: bool) -> f32 {
        reveal(self.share(present)) * self.fade.map_or(1.0, |c| c.fade_out())
    }

    /// 本帧的显示状态 `present` 是否与上一帧不同。
    pub(in crate::app) fn changed(&self, present: bool) -> bool {
        self.shown != present
    }

    /// 是否正在出现或消失。
    pub fn moving(&self) -> bool {
        self.motion.is_some()
    }

    /// 与上一帧比较显示状态，变化时从当前的显示比例开始向另一端过渡。
    ///
    /// `clock` 为 `None`（没有窗口、动态效果关闭、启动后第一帧）时直接到终态。
    pub(in crate::app) fn track(&mut self, present: bool, clock: Option<Clock>) {
        if self.motion.is_some_and(|(_, c)| c.done()) {
            self.motion = None;
            self.fade = None;
        }
        if self.shown == present {
            return;
        }
        let from = self.share(self.shown);
        self.shown = present;
        self.fade = clock.filter(|_| !present).map(|c| c.lasting(EXIT_FADE));
        self.motion = clock.map(|c| match self.fade {
            Some(fade) => (from, fade.then(c.duration())),
            None => (from, c),
        });
    }
}

/// Git、终端模块带来的几样东西出现、消失的动画。
#[derive(Debug, Default)]
pub struct ModuleFx {
    /// 第一帧只记下有哪些，不做动画：启动时这些元素已经存在。
    primed: bool,
    /// 视图切换里的「提交」一段。
    pub commit_tab: Presence,
    /// 状态栏右端的终端开关和设置分类栏中的「终端」一项：二者都随终端模块出现、消失。
    pub terminal: Presence,
    /// 设置分类栏中的「Git」一项，随 Git 模块出现、消失。
    pub git: Presence,
    /// 状态栏的分支名。
    pub branch: Presence,
    /// 最近一帧画的分支名：分支名消失（模块关了、仓库状态清掉了）以后淡出用。
    pub branch_label: Option<String>,
    /// 标签栏右端「源代码 / 分栏 / 预览」三个按钮的显示状态。
    pub preview: Presence,
    /// 最近一帧绘制时三个按钮中选中的模式，供切换到不可预览的文件后按钮收起的动画使用。
    pub preview_mode: Option<winc_editor::PreviewMode>,
    /// 标签栏右端「在浏览器中打开」按钮的显示状态。
    pub browser: Presence,
}

impl ModuleFx {
    /// 是否有东西正在出现或消失。
    pub fn moving(&self) -> bool {
        self.commit_tab.moving()
            || self.terminal.moving()
            || self.git.moving()
            || self.branch.moving()
            || self.preview.moving()
            || self.browser.moving()
    }
}

impl App {
    /// 每帧绘制前调用：与上一帧比较「提交」一段、终端开关、分支名、预览切换和「在浏览器中打开」
    /// 是否出现或消失。
    pub(in crate::app) fn track_modules(&mut self) {
        let commit = self.git.root.is_some();
        let terminal = self.terminal_enabled();
        let git = self.git_enabled();
        let branch = self.git.status.as_ref().is_some_and(|s| s.head.is_some());
        let preview = self.editor.active().is_some_and(|d| d.can_preview());
        let browser = self.browser_page().is_some();
        let fx = &self.module_fx;
        let changed = fx.commit_tab.shown != commit
            || fx.terminal.shown != terminal
            || fx.git.shown != git
            || fx.branch.shown != branch
            || fx.preview.shown != preview
            || fx.browser.shown != browser;
        // 同一帧变了的几样（开关 Git 模块时「提交」一段和分支名）共用一个时钟。
        let primed = std::mem::replace(&mut self.module_fx.primed, true);
        let clock = if primed && changed {
            self.clock(MODULE_FADE)
        } else {
            None
        };
        let fx = &mut self.module_fx;
        fx.commit_tab.track(commit, clock);
        fx.terminal.track(terminal, clock);
        fx.git.track(git, clock);
        fx.branch.track(branch, clock);
        if !branch && !fx.branch.moving() {
            fx.branch_label = None;
        }
        fx.preview.track(preview, clock);
        fx.browser.track(browser, clock);
        if !preview && !fx.preview.moving() {
            fx.preview_mode = None;
        }
    }
}
