//! 连按两下同一个修饰键（如 Shift）：单按修饰键后松开记作一下，两下构成键位表中的组合键。

use std::time::{Duration, Instant};

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::keymap::{KeyCode, Lookup, Mods, Stroke};

/// 连按两下修饰键的最长间隔，取常见的系统双击间隔。
const DOUBLE_TAP: Duration = Duration::from_millis(500);

/// 单按修饰键的状态。
#[derive(Debug, Default)]
pub(crate) struct Taps {
    /// 正按着、此后还没有按过别的键的修饰键。
    down: Option<Mods>,
    /// 上一下单按的修饰键和松开的时刻。
    last: Option<(Mods, Instant)>,
}

impl App {
    /// 记录按键事件 `event` 中单按的修饰键；连按两下时执行键位表中对应的命令。
    pub(in crate::app) fn track_tap(&mut self, event: &KeyEvent) {
        let bit = match event.logical_key {
            Key::Named(NamedKey::Shift) => Mods::SHIFT,
            Key::Named(NamedKey::Control) => Mods::CTRL,
            Key::Named(NamedKey::Alt) => Mods::ALT,
            Key::Named(NamedKey::Super) => Mods::META,
            _ => {
                if event.state == ElementState::Pressed {
                    self.taps = Taps::default();
                }
                return;
            }
        };
        match event.state {
            ElementState::Pressed if !event.repeat => {
                // 修饰键状态的更新可能在按键事件之前或之后到达，两种都算单按。
                let held = Mods::from_winit(self.pointer.modifiers);
                let alone = held == bit || held == Mods::NONE;
                self.taps.down = alone.then_some(bit);
            }
            ElementState::Pressed => {}
            ElementState::Released if self.taps.down.take() == Some(bit) => self.tap(bit),
            ElementState::Released => self.taps.last = None,
        }
    }

    /// 单按了一下修饰键 `bit`：与上一下同键且间隔不超过 [`DOUBLE_TAP`] 时执行对应的命令。
    fn tap(&mut self, bit: Mods) {
        // 设置浮层和对话框是模态的，快捷键不传到下层。
        if self.settings.open || self.overlays.form_dialog.is_some() {
            return;
        }
        let now = Instant::now();
        let stroke = Stroke {
            key: KeyCode::Tap,
            mods: bit,
        };
        match self.taps.last.take() {
            Some((last, at)) if last == bit && now.duration_since(at) <= DOUBLE_TAP => {
                if let Lookup::Run(actions) = self.lookup_shortcut(stroke, Some(stroke)) {
                    let _ = (actions.into_iter()).any(|a| self.run_action(self.keyboard_action(a)));
                }
            }
            _ => self.taps.last = Some((bit, now)),
        }
    }
}
