//! 终端标签沿排列方向的滚动与拖动排序，及横向标签的新建、关闭和换位动画。
//!
//! 动画参数与编辑区的标签栏相同。放不下时标签栏沿排列方向滚动；切换标签、
//! 新建或关闭标签时滚到正在看的标签。

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::super::animation::{Clock, EXIT_FADE, MODULE_FADE, Presence, TAB_SLIDE};
use super::{App, Focus};
use crate::field::TextField;

/// 标签最窄宽度（逻辑像素）。
pub(crate) const TAB_MIN: f32 = 120.0;
/// 标签最宽宽度（逻辑像素）：标签少时不再变宽。
pub(crate) const TAB_MAX: f32 = 220.0;
/// 标签多、面板窄时一起变窄，最窄多少：还看得见两三个字和关闭按钮。
pub(crate) const TAB_SQUEEZED: f32 = 72.0;
/// 横向标签之间的间距（逻辑像素）。
pub(crate) const TAB_GAP: f32 = 4.0;
/// 竖向终端条目之间的间距（逻辑像素）。
///
/// 取自调参：底部和右侧面板中，相邻条目可区分，列表仍保持紧凑。
pub(crate) const LIST_GAP: f32 = 2.0;
/// 拖动起始距离（逻辑像素），与编辑区标签的拖动阈值相同。
const THRESHOLD: f32 = 4.0;

/// 刚关闭的标签：内容在原位淡出，之后宽度收起。
pub(crate) struct Ghost {
    /// 画在第几个标签前面（关掉前它的位置）。
    pub at: usize,
    /// 标签上的文字。
    pub label: String,
    /// 收起前沿排列方向的长度（逻辑像素）。
    pub size: f32,
    /// 关掉时是否是正在看的标签。
    pub active: bool,
    /// 内容淡出的时钟。
    pub fade: Clock,
    /// 宽度收起的时钟，在 `fade` 结束时开始。
    pub clock: Clock,
}

/// 按住的标签。
#[derive(Clone, Copy)]
struct Drag {
    /// 标签编号。
    id: u16,
    /// 按下点到标签起点的距离（沿排列方向，逻辑像素）。
    grab: f32,
    /// 按下时沿排列方向的位置（窗口坐标，逻辑像素）。
    pressed: f32,
    /// 是否已经开始拖动（沿排列方向移动超过 [`THRESHOLD`]）。
    moving: bool,
}

/// 标签栏的动画和拖动。
#[derive(Default)]
pub(crate) struct Strip {
    /// 上一帧各标签沿排列方向的长度，横向为宽度，竖向为高度（逻辑像素）。
    pub sizes: Vec<f32>,
    /// 上一帧第一个标签沿排列方向的起点（窗口坐标，逻辑像素）。
    pub start: f32,
    /// 是否沿竖向排列，用于将鼠标坐标换算为排列方向上的位置。
    vertical: bool,
    /// 标签栏沿排列方向的滚动量（逻辑像素）。
    pub scroll: f32,
    /// 上一帧的最大滚动量（逻辑像素）；放得下时为 0。
    pub max_scroll: f32,
    /// 上一帧正在看的标签、标签数和可视长度：变化时显示当前标签。
    pub revealed: Option<(u16, usize, f32)>,
    /// 上一帧见过的标签，和这一帧比认出新建的。
    known: Option<Vec<u16>>,
    /// 正在长出来的标签：（标签编号，进度时钟）。
    opening: Vec<(u16, Clock)>,
    /// 刚关掉、还在原位收窄淡出的标签。
    pub closing: Vec<Ghost>,
    /// 离开原位的标签：（标签号，起点相对新位置的偏移，时钟）。
    slides: Vec<(u16, f32, Clock)>,
    /// 按住的标签；没按住时为 `None`。
    drag: Option<Drag>,
    /// 上一帧是否有停止按钮；尚未绘制时为 `None`。
    stop_shown: Option<bool>,
    /// 停止按钮正在出现或消失的时钟。
    stop_motion: Option<Clock>,
    /// 右侧标签列表的展开、收起；与其他侧栏元素共用显示比例和淡入、淡出曲线。
    pub position: Presence,
    /// 测试里没有窗口、没有时钟：设置后动画都停在这一时刻。
    #[cfg(test)]
    pub frozen: Option<std::time::Duration>,
}

impl Strip {
    /// 设置排列方向；方向变化时结束旧方向上的动画和拖动。
    pub fn orient(&mut self, vertical: bool) {
        if self.vertical == vertical {
            return;
        }
        self.vertical = vertical;
        self.sizes.clear();
        self.scroll = 0.0;
        self.max_scroll = 0.0;
        self.revealed = None;
        self.known = None;
        self.opening.clear();
        self.closing.clear();
        self.slides.clear();
        self.drag = None;
    }

    /// 窗口坐标沿标签排列方向的位置（逻辑像素）。
    pub fn along(&self, (x, y): (f32, f32)) -> f32 {
        if self.vertical { y } else { x }
    }

    /// 按滚轮位移 `delta`（逻辑像素）沿排列方向滚动标签栏。
    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).clamp(0.0, self.max_scroll);
    }

    /// 第 `index` 个标签起点相对第一个标签起点的距离（逻辑像素）。
    fn slot(&self, index: usize) -> f32 {
        self.sizes[..index].iter().map(|w| w + self.gap()).sum()
    }

    /// 当前排列方向上标签之间的间距（逻辑像素）。
    fn gap(&self) -> f32 {
        if self.vertical { LIST_GAP } else { TAB_GAP }
    }

    /// 标签 `id` 长出来的程度（0 到 1）。
    pub fn grow(&self, id: u16) -> f32 {
        self.opening
            .iter()
            .find(|(o, _)| *o == id)
            .map_or(1.0, |(_, c)| c.progress())
    }

    /// 标签 `id` 当前相对它的位置的偏移。
    pub fn slide(&self, id: u16) -> f32 {
        self.slides
            .iter()
            .find(|s| s.0 == id)
            .map_or(0.0, |s| s.2.between(s.1, 0.0))
    }

    /// 停止按钮的显示比例（0 到 1）：`running` 表示配置是否在运行，决定动画结束时按钮是否保留。
    pub fn stop_share(&self, running: bool) -> f32 {
        match (&self.stop_motion, running) {
            (Some(clock), true) => clock.progress(),
            (Some(clock), false) => clock.fade_out(),
            (None, true) => 1.0,
            (None, false) => 0.0,
        }
    }

    /// 是否有出现、消失或滑动的动画还没结束。
    pub fn animating(&self) -> bool {
        !self.opening.is_empty()
            || !self.closing.is_empty()
            || !self.slides.is_empty()
            || self.stop_motion.is_some()
            || self.position.moving()
    }

    /// 去掉结束了的动画。
    pub fn settle(&mut self) {
        self.opening.retain(|(_, c)| !c.done());
        self.closing.retain(|g| !g.clock.done());
        self.slides.retain(|s| !s.2.done());
        self.stop_motion = self.stop_motion.take().filter(|clock| !clock.done());
    }
}

impl App {
    /// 排版前记录标签位置变化：先按显示比例让出宽度，内容使用与侧栏相同的淡入、淡出曲线。
    pub(crate) fn note_terminal_tab_position(&mut self) {
        let vertical = self.config.terminal_vertical_tabs == Some(true);
        let animate = self.animations_on();
        let position = &self.terminal.strip.position;
        let clock = if animate
            && position.changed(vertical)
            && (!self.terminal.strip.sizes.is_empty() || position.moving())
        {
            self.strip_clock().map(|clock| clock.lasting(MODULE_FADE))
        } else {
            None
        };
        if !animate {
            self.terminal.strip.position = Presence::default();
        }
        self.terminal.strip.position.track(vertical, clock);
    }

    fn strip_clock(&mut self) -> Option<Clock> {
        #[cfg(test)]
        if let Some(at) = self.terminal.strip.frozen {
            return Some(Clock::frozen(at, TAB_SLIDE));
        }
        self.clock(TAB_SLIDE)
    }

    /// 绘制标签栏前调用：配置开始或结束运行时启动停止按钮的淡入、淡出，首次绘制时直接到位。
    pub(crate) fn note_stop_button(&mut self) {
        let running = self.run_config_running();
        if self
            .terminal
            .strip
            .stop_shown
            .is_some_and(|shown| shown != running)
        {
            self.terminal.strip.stop_motion = self.strip_clock();
        }
        self.terminal.strip.stop_shown = Some(running);
    }

    /// 绘制标签前记录增减：多出一个标签时沿排列方向展开、淡入。
    ///
    /// 一次多出多个（重新启动）、第一次画时直接到位。
    pub(crate) fn note_terminal_tabs(&mut self) {
        let vertical = self.terminal_tab_list().is_some();
        self.terminal.strip.orient(vertical);
        if vertical {
            return;
        }
        let ids: Vec<u16> = self.terminal.tabs.iter().map(|t| t.id).collect();
        let new: Vec<u16> = match &self.terminal.strip.known {
            Some(known) => ids
                .iter()
                .filter(|id| !known.contains(id))
                .copied()
                .collect(),
            None => Vec::new(),
        };
        if let [id] = new[..]
            && let Some(clock) = self.strip_clock()
        {
            self.terminal.strip.opening.push((id, clock));
        }
        let strip = &mut self.terminal.strip;
        strip.opening.retain(|(id, _)| ids.contains(id));
        strip.slides.retain(|s| ids.contains(&s.0));
        strip.known = Some(ids);
    }

    /// 关闭第 `index` 个标签前记录动画：在原位淡出、收起，后续标签沿排列方向补位。
    ///
    /// 在从列表里删掉之前调用。
    pub(super) fn animate_terminal_close(&mut self, index: usize) {
        if self.terminal_tab_list().is_some() {
            return;
        }
        let Some(size) = self.terminal.strip.sizes.get(index).copied() else {
            return;
        };
        let Some(fade) = self.strip_clock().map(|c| c.lasting(EXIT_FADE)) else {
            return;
        };
        let clock = fade.then(TAB_SLIDE);
        let t = &mut self.terminal;
        let label = t.tabs[index].label().to_owned();
        for ghost in &mut t.strip.closing {
            if ghost.at > index {
                ghost.at -= 1;
            }
        }
        t.strip.closing.push(Ghost {
            at: index,
            label,
            size,
            active: index == t.active,
            fade,
            clock,
        });
        t.strip.sizes.remove(index);
    }

    /// 切换到第 `index` 个标签并记录按下点 `at`（沿排列方向的窗口坐标，逻辑像素）。
    pub(crate) fn press_terminal_tab(&mut self, index: usize, at: f32) {
        self.activate_terminal_tab(index);
        let strip = &self.terminal.strip;
        if index >= strip.sizes.len() {
            return;
        }
        let Some(tab) = self.terminal.tabs.get(index) else {
            return;
        };
        let start = strip.start + strip.slot(index) + strip.slide(tab.id);
        let id = tab.id;
        let strip = &mut self.terminal.strip;
        strip.slides.retain(|s| s.0 != id);
        strip.drag = Some(Drag {
            id,
            grab: at - start,
            pressed: at,
            moving: false,
        });
    }

    /// 给第 `index` 个标签改名字（双击标签、标签的右键菜单、命令面板）：标签上换成输入框，
    /// 填好现在的名字并全选。
    pub(crate) fn open_terminal_rename(&mut self, index: usize) {
        // 正在改另一个的名字：先按输入的名字算。面板保存时（命令面板里选的）先打开，不往看不见的
        // 输入框里打字。
        self.finish_terminal_rename(true);
        if index >= self.terminal.tabs.len() {
            return;
        }
        if !self.terminal.open {
            self.set_terminal_open(true);
        }
        let tab = &self.terminal.tabs[index];
        let mut field = TextField::default();
        field.set_text(tab.label());
        field.select_all();
        self.terminal.rename = Some((tab.id, field));
        self.terminal.strip.drag = None;
        self.activate_terminal_tab(index);
        self.ime.field_preedit.clear();
        self.focus = Focus::TerminalRename;
        self.request_redraw();
    }

    /// 改完名字：`keep` 为真时（↩、点其他位置）用输入的名字，去掉首尾空白，空的恢复自动的名字，
    /// 与自动的名字相同时仍按自动处理；Esc 不改。
    ///
    /// 焦点在输入框上时回到终端。
    pub(crate) fn finish_terminal_rename(&mut self, keep: bool) {
        let Some((id, field)) = self.terminal.rename.take() else {
            return;
        };
        if keep && let Some(tab) = self.terminal.tabs.iter_mut().find(|t| t.id == id) {
            let name = field.text().trim();
            // 没有自定名字、输入框里仍是自动的名字：不改，标签继续跟随标题和进程变化。
            if !tab.name.is_empty() || name != tab.label() {
                tab.name = name.to_owned();
            }
        }
        if self.focus == Focus::TerminalRename {
            self.focus = Focus::Terminal;
        }
        self.ime.field_preedit.clear();
        self.request_redraw();
    }

    /// 标签上改名字的输入框里的按键。
    pub(in crate::app) fn on_key_terminal_rename(
        &mut self,
        event: &KeyEvent,
        base: &str,
        cmd: bool,
    ) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.finish_terminal_rename(false),
            Key::Named(NamedKey::Enter) => self.finish_terminal_rename(true),
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 按住标签移动：跟着鼠标走，越过相邻标签的中线时换位。
    ///
    /// 返回是否在按住标签。
    pub(crate) fn drag_terminal_tab(&mut self, at: f32) -> bool {
        let Some(drag) = &mut self.terminal.strip.drag else {
            return false;
        };
        if !drag.moving && (at - drag.pressed).abs() < THRESHOLD {
            return true;
        }
        drag.moving = true;
        let drag = *drag;
        let Some(mut index) = self.terminal.tabs.iter().position(|t| t.id == drag.id) else {
            self.terminal.strip.drag = None;
            return true;
        };
        if self.terminal.strip.sizes.len() != self.terminal.tabs.len() {
            return true;
        }
        let start = self.dragged_start(index, drag.grab);
        let size = self.terminal.strip.sizes[index];
        let center = |strip: &Strip, i: usize| strip.slot(i) + strip.sizes[i] / 2.0;
        loop {
            let strip = &self.terminal.strip;
            let to = if index > 0 && start < center(strip, index - 1) {
                index - 1
            } else if index + 1 < strip.sizes.len() && start + size > center(strip, index + 1) {
                index + 1
            } else {
                break;
            };
            // 被换的标签从原来的一侧滑到新位置。
            let other = self.terminal.tabs[to].id;
            let from = if to < index { -1.0 } else { 1.0 } * (size + strip.gap());
            let t = &mut self.terminal;
            t.tabs.swap(index, to);
            t.strip.sizes.swap(index, to);
            if t.active == index {
                t.active = to;
            } else if t.active == to {
                t.active = index;
            }
            self.slide_terminal_tab(other, from);
            index = to;
        }
        self.request_redraw();
        true
    }

    /// 松开：拖着的标签从松开处滑回它的位置。
    pub(crate) fn release_terminal_tab(&mut self) {
        let Some(drag) = self.terminal.strip.drag.take() else {
            return;
        };
        if !drag.moving {
            return;
        }
        if let Some(index) = self.terminal.tabs.iter().position(|t| t.id == drag.id)
            && index < self.terminal.strip.sizes.len()
        {
            let from = self.dragged_start(index, drag.grab) - self.terminal.strip.slot(index);
            self.slide_terminal_tab(drag.id, from);
        }
        self.request_redraw();
    }

    /// 正在拖动的标签（编号，沿排列方向的起点坐标）；没有拖动时为 `None`。
    pub(crate) fn dragged_terminal_tab(&self) -> Option<(u16, f32)> {
        let drag = self.terminal.strip.drag.filter(|d| d.moving)?;
        let index = self.terminal.tabs.iter().position(|t| t.id == drag.id)?;
        (index < self.terminal.strip.sizes.len()).then(|| {
            (
                drag.id,
                self.terminal.strip.start + self.dragged_start(index, drag.grab),
            )
        })
    }

    /// 拖动标签的起点相对第一个标签起点的位置，限制在全部标签的范围中。
    fn dragged_start(&self, index: usize, grab: f32) -> f32 {
        let strip = &self.terminal.strip;
        let size = strip.sizes[index];
        let total = strip.slot(strip.sizes.len()) - strip.gap();
        (strip.along(self.pointer.mouse) - grab - strip.start).clamp(0.0, (total - size).max(0.0))
    }

    /// 让标签 `id` 从距新位置 `from` 处滑过去；还在滑时从当前的偏移继续滑动。
    fn slide_terminal_tab(&mut self, id: u16, from: f32) {
        if self.terminal_tab_list().is_some() {
            return;
        }
        let from = from + self.terminal.strip.slide(id);
        let clock = self.strip_clock();
        let strip = &mut self.terminal.strip;
        strip.slides.retain(|s| s.0 != id);
        if let Some(clock) = clock {
            strip.slides.push((id, from, clock));
        }
    }
}

/// 标签加起来放不下 `room` 时一起变窄：最宽的先窄，窄到一样宽以后一起窄，最窄到
/// [`TAB_SQUEEZED`]（仍然放不下时，超出的部分被标签栏裁掉）。
pub(crate) fn fit_tabs(widths: &mut [f32], room: f32) {
    let gaps = TAB_GAP * widths.len().saturating_sub(1) as f32;
    let room = room - gaps;
    if widths.iter().sum::<f32>() <= room {
        return;
    }
    let cap = shared_limit(widths, room);
    for w in widths {
        *w = w.min(cap).max(TAB_SQUEEZED).floor();
    }
}

/// 找一个上限：比它小的项不动，其余的项都取它，加起来正好等于 `room`。
///
/// 先按平分算，再把较小的项省下的部分分给其余项，直到不再变化（每轮至少多一项不动，最多循环
/// 项数轮）。
fn shared_limit(values: &[f32], room: f32) -> f32 {
    let mut cap = room / values.len().max(1) as f32;
    loop {
        let narrow = values.iter().filter(|&&w| w < cap);
        let (sum, count) = narrow.fold((0.0, 0), |(s, n), w| (s + w, n + 1));
        let next = (room - sum) / (values.len() - count).max(1) as f32;
        if next <= cap || count == values.len() {
            return cap;
        }
        cap = next;
    }
}

/// 按名字需要的宽度 `names`（含内边距）排出放进 `room` 的各标签宽度。
///
/// 放得下时每个标签宽 [`TAB_MIN`]～[`TAB_MAX`]。放不下时先收掉名字之外的留白：留白最多的先收，
/// 名字仍然完整；留白收完还放不下时，再按 [`fit_tabs`] 截短最长的名字。
pub(crate) fn fit_named_tabs(names: &[f32], room: f32) -> Vec<f32> {
    let names: Vec<f32> = (names.iter())
        .map(|&w| w.clamp(TAB_SQUEEZED, TAB_MAX))
        .collect();
    let wanted: Vec<f32> = names.iter().map(|&w| w.max(TAB_MIN)).collect();
    let gaps = TAB_GAP * names.len().saturating_sub(1) as f32;
    let spare = room - gaps - names.iter().sum::<f32>();
    if spare <= 0.0 {
        let mut widths = names;
        fit_tabs(&mut widths, room);
        return widths;
    }
    let blanks: Vec<f32> = wanted.iter().zip(&names).map(|(w, n)| w - n).collect();
    let limit = shared_limit(&blanks, spare);
    (names.iter().zip(&blanks))
        .map(|(name, blank)| (name + blank.min(limit)).floor())
        .collect()
}

/// 标签宽度 `widths` 是否已窄于名字宽度 `names` 对应的期望宽度，即标签栏空间不足。
///
/// [`fit_named_tabs`] 向下取整造成的不足 1 个逻辑像素的差值不算收窄。
pub(crate) fn tabs_narrowed(names: &[f32], widths: &[f32]) -> bool {
    let wanted = names
        .iter()
        .map(|&w| w.clamp(TAB_SQUEEZED, TAB_MAX).max(TAB_MIN));
    wanted
        .zip(widths)
        .any(|(wanted, width)| *width <= wanted - 1.0)
}

#[cfg(test)]
mod tests;
