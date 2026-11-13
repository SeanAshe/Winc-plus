//! 终端面板的标签栏：新建的标签从零宽长出来，关掉的在原位收窄、淡出，右边的跟着让开；按住
//! 标签左右拖动调整顺序，越过相邻标签的中线时和它换位，被换的那个滑过去。
//!
//! 动画参数与编辑区的标签栏相同。放不下时标签栏可以用滚轮横向滚动，不显示滚动条；切换标签、
//! 新建或关闭标签时滚到正在看的标签。

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::super::animation::{Clock, EXIT_FADE, TAB_SLIDE};
use super::{App, Focus};
use crate::field::TextField;

/// 标签最窄宽度（逻辑像素）。
pub(crate) const TAB_MIN: f32 = 120.0;
/// 标签最宽宽度（逻辑像素）：标签少时不再变宽。
pub(crate) const TAB_MAX: f32 = 220.0;
/// 标签多、面板窄时一起变窄，最窄多少：还看得见两三个字和关闭按钮。
pub(crate) const TAB_SQUEEZED: f32 = 72.0;
/// 标签之间的间距（逻辑像素）。
pub(crate) const TAB_GAP: f32 = 4.0;
/// 按下以后横向移动超过多远才算拖动（逻辑像素，经验值），不足时仍按单击处理。
const THRESHOLD: f32 = 4.0;

/// 刚关闭的标签：内容在原位淡出，之后宽度收起。
pub(crate) struct Ghost {
    /// 画在第几个标签前面（关掉前它的位置）。
    pub at: usize,
    /// 标签上的文字。
    pub label: String,
    /// 收窄前的宽度（逻辑像素）。
    pub width: f32,
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
    /// 按下点到标签左边的距离。
    grab: f32,
    /// 按下时的横坐标（逻辑像素），用来判断是否超过了拖动阈值。
    start_x: f32,
    /// 是否已经开始拖动（横向移动超过 [`THRESHOLD`]）。
    moving: bool,
}

/// 标签栏的动画和拖动。
#[derive(Default)]
pub(crate) struct Strip {
    /// 上一帧各个标签到位以后的宽度，拖动时算位置用。
    pub widths: Vec<f32>,
    /// 上一帧第一个标签的左边。
    pub left: f32,
    /// 标签栏的横向滚动量（逻辑像素）。
    pub scroll: f32,
    /// 上一帧的最大滚动量（逻辑像素）；放得下时为 0。
    pub max_scroll: f32,
    /// 上一帧正在看的标签和标签数：变了就滚到正在看的标签。
    pub revealed: Option<(u16, usize)>,
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
    /// 测试里没有窗口、没有时钟：设置后动画都停在这一时刻。
    #[cfg(test)]
    pub frozen: Option<std::time::Duration>,
}

impl Strip {
    /// 按滚轮的位移 `delta`（逻辑像素）横向滚动标签栏。
    pub fn wheel(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).clamp(0.0, self.max_scroll);
    }

    /// 第 `index` 个标签左边相对第一个标签左边的距离（逻辑像素）。
    fn slot(&self, index: usize) -> f32 {
        self.widths[..index].iter().map(|w| w + TAB_GAP).sum()
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

    /// 画标签栏前调用：比上一帧正好多出一个标签时，它从零宽长出来、淡入。
    ///
    /// 一次多出多个（重新启动）、第一次画时直接到位。
    pub(crate) fn note_terminal_tabs(&mut self) {
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

    /// 即将关闭第 `index` 个标签：标签内容先在原位淡出，之后宽度收起，右侧的标签随收起的右边
    /// 滑回。
    ///
    /// 在从列表里删掉之前调用。
    pub(super) fn animate_terminal_close(&mut self, index: usize) {
        let Some(width) = self.terminal.strip.widths.get(index).copied() else {
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
            width,
            active: index == t.active,
            fade,
            clock,
        });
        t.strip.widths.remove(index);
    }

    /// 在第 `index` 个标签上按下：切过去，记下按住的位置；横向移动超过一点才算拖动。
    pub(crate) fn press_terminal_tab(&mut self, index: usize, x: f32) {
        self.activate_terminal_tab(index);
        let strip = &self.terminal.strip;
        if index >= strip.widths.len() {
            return;
        }
        let Some(tab) = self.terminal.tabs.get(index) else {
            return;
        };
        let left = strip.left + strip.slot(index) + strip.slide(tab.id);
        let id = tab.id;
        let strip = &mut self.terminal.strip;
        strip.slides.retain(|s| s.0 != id);
        strip.drag = Some(Drag {
            id,
            grab: x - left,
            start_x: x,
            moving: false,
        });
    }

    /// 给第 `index` 个标签改名字（双击标签、标签的右键菜单、命令面板）：标签上换成输入框，
    /// 填好现在的名字并全选。
    pub(crate) fn open_terminal_rename(&mut self, index: usize) {
        // 正在改另一个的名字：先按输入的名字算。面板收着时（命令面板里选的）先打开，不往看不见的
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

    /// 改完名字：`keep` 为真时（↩、点其他位置）用输入的名字，去掉首尾空白，空的恢复自动的名字；
    /// Esc 不改。
    ///
    /// 焦点在输入框上时回到终端。
    pub(crate) fn finish_terminal_rename(&mut self, keep: bool) {
        let Some((id, field)) = self.terminal.rename.take() else {
            return;
        };
        if keep && let Some(tab) = self.terminal.tabs.iter_mut().find(|t| t.id == id) {
            tab.name = field.text().trim().to_owned();
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
    pub(crate) fn drag_terminal_tab(&mut self, x: f32) -> bool {
        let Some(drag) = &mut self.terminal.strip.drag else {
            return false;
        };
        if !drag.moving && (x - drag.start_x).abs() < THRESHOLD {
            return true;
        }
        drag.moving = true;
        let drag = *drag;
        let Some(mut index) = self.terminal.tabs.iter().position(|t| t.id == drag.id) else {
            self.terminal.strip.drag = None;
            return true;
        };
        if self.terminal.strip.widths.len() != self.terminal.tabs.len() {
            return true;
        }
        let left = self.dragged_left(index, drag.grab);
        let width = self.terminal.strip.widths[index];
        let center = |strip: &Strip, i: usize| strip.slot(i) + strip.widths[i] / 2.0;
        loop {
            let strip = &self.terminal.strip;
            let to = if index > 0 && left < center(strip, index - 1) {
                index - 1
            } else if index + 1 < strip.widths.len() && left + width > center(strip, index + 1) {
                index + 1
            } else {
                break;
            };
            // 被换的标签从原来的一侧滑到新位置。
            let other = self.terminal.tabs[to].id;
            let from = if to < index { -1.0 } else { 1.0 } * (width + TAB_GAP);
            let t = &mut self.terminal;
            t.tabs.swap(index, to);
            t.strip.widths.swap(index, to);
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
            && index < self.terminal.strip.widths.len()
        {
            let from = self.dragged_left(index, drag.grab) - self.terminal.strip.slot(index);
            self.slide_terminal_tab(drag.id, from);
        }
        self.request_redraw();
    }

    /// 拖着的标签（标签号，左边的横坐标）；没在拖时 `None`。
    pub(crate) fn dragged_terminal_tab(&self) -> Option<(u16, f32)> {
        let drag = self.terminal.strip.drag.filter(|d| d.moving)?;
        let index = self.terminal.tabs.iter().position(|t| t.id == drag.id)?;
        (index < self.terminal.strip.widths.len()).then(|| {
            (
                drag.id,
                self.terminal.strip.left + self.dragged_left(index, drag.grab),
            )
        })
    }

    /// 拖着的第 `index` 个标签的左边（相对第一个标签左边），夹在全部标签占的范围里。
    fn dragged_left(&self, index: usize, grab: f32) -> f32 {
        let strip = &self.terminal.strip;
        let width = strip.widths[index];
        let total = strip.slot(strip.widths.len()) - TAB_GAP;
        (self.pointer.mouse.0 - grab - strip.left).clamp(0.0, (total - width).max(0.0))
    }

    /// 让标签 `id` 从距新位置 `from` 处滑过去；还在滑时从当前的偏移继续滑动。
    fn slide_terminal_tab(&mut self, id: u16, from: f32) {
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
