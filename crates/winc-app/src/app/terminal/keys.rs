//! 键盘和剪贴板：按键编码、输入法提交的文字、粘贴、复制、全选，滚动和滚动条。

use super::*;
use winit::keyboard::PhysicalKey;

impl App {
    /// 把字节写给正在看的标签里的程序：回到最新的输出；程序的输出到了下一帧马上能发，不等这一帧
    /// 画完。
    pub(in crate::app) fn terminal_input(&mut self, bytes: Vec<u8>) {
        let t = &mut self.terminal;
        let (Some(client), Some(tab)) = (&t.client, t.tabs.get_mut(t.active)) else {
            return;
        };
        if tab.ended.is_some() || bytes.is_empty() {
            return;
        }
        tab.scroll = 0;
        tab.scroll_frac = 0.0;
        tab.jump = None;
        let mut rest = bytes.len() as u64;
        for chunk in bytes.chunks(INPUT_CHUNK) {
            rest -= chunk.len() as u64;
            client.send(&ToHelper::Input {
                tab: tab.id,
                bytes: chunk.to_vec(),
                rest,
            });
        }
        if tab.credit_due {
            tab.credit_due = false;
            client.send(&ToHelper::Credit { tab: tab.id });
        }
        self.request_redraw();
    }

    /// 粘贴到终端（括号粘贴见 [`input::paste`]）。
    ///
    /// 有文字时粘贴文字（同时有图片也取文字）。没有文字、有图片时，把图片存成临时文件，把路径
    /// 粘进去：终端里的程序读不到剪贴板图片，路径是它能打开的内容。
    ///
    /// 程序没有开启括号粘贴、要粘贴的文字不止一行时先确认（[`input::paste_needs_confirm`]）：
    /// 弹出确认框（macOS 上是系统的，其他平台画在窗口里）。
    pub(in crate::app) fn terminal_paste(&mut self) {
        if let Some(text) = self.clipboard_text().filter(|text| !text.is_empty()) {
            self.queue_terminal_paste(text);
            return;
        }
        self.paste_terminal_image();
    }

    /// 把剪贴板图片存成临时文件，把路径粘贴给终端里的程序。没有图片时返回 `false`。
    fn paste_terminal_image(&mut self) -> bool {
        let Some(saved) = self.clipboard_image_file() else {
            return false;
        };
        match saved {
            Ok(path) => self.queue_terminal_paste(path.display().to_string()),
            Err(error) => {
                self.flash(t!("terminal-paste-image-failed", error = error.to_string()));
            }
        }
        true
    }

    /// 把 `text` 交给当前标签；需要确认时先弹出确认框。
    fn queue_terminal_paste(&mut self, text: String) {
        let Some(tab) = self.terminal.tab().map(|t| (t.id, t.modes)) else {
            return;
        };
        let (id, modes) = tab;
        if input::paste_needs_confirm(&text, modes) {
            let message = t!("terminal-paste-confirm", count = text.lines().count());
            if self.window.is_some() {
                // 没有弹出时不粘贴，并说明原因。
                let detail = t!("terminal-paste-confirm-detail");
                self.show_confirm(&message, detail, t!("cmd-paste"), move |ok| {
                    ok.then(|| UserEvent::TerminalPaste(id, text.clone()))
                });
                return;
            }
        }
        self.confirmed_terminal_paste(id, text);
    }

    /// 把 `text` 粘贴到标签 `id`；该标签已不是当前标签（确认期间切换或关闭了）时不粘贴。
    pub(crate) fn confirmed_terminal_paste(&mut self, id: u16, text: String) {
        let Some(modes) = self.terminal.tab().filter(|t| t.id == id).map(|t| t.modes) else {
            return;
        };
        let mut bytes = Vec::new();
        input::paste(&text, modes, &mut bytes);
        self.terminal_input(bytes);
    }

    /// 复制选区（⌘C）。
    ///
    /// 没有选区时什么也不做；选区里有还没取回的历史行时先要，回来再复制。
    pub(in crate::app) fn terminal_copy(&mut self) {
        let t = &mut self.terminal;
        let Some(tab) = t.tabs.get(t.active) else {
            return;
        };
        match tab.selected_text() {
            Ok(Some(text)) => {
                self.set_clipboard(text, false);
                self.flash(winc_i18n::t!("terminal-selection-copied"));
            }
            Ok(None) => {}
            Err(range) => {
                let id = tab.id;
                if !t.waiting(id, Request::Copy) {
                    t.request(id, range, Request::Copy);
                }
            }
        }
    }

    /// 选中全部（⌘A）：回滚缓冲加屏幕。
    pub(in crate::app) fn terminal_select_all(&mut self) {
        if let Some(tab) = self.terminal.tab_mut() {
            tab.select_all();
            self.request_redraw();
        }
    }

    /// 输入法提交的文字。
    pub(in crate::app) fn terminal_commit(&mut self, text: &str) {
        self.clear_terminal_selection();
        self.terminal_input(text.as_bytes().to_vec());
    }

    /// 清除当前标签的选区。
    fn clear_terminal_selection(&mut self) {
        if let Some(tab) = self.terminal.tab_mut() {
            tab.selection = None;
        }
    }

    /// 滚动条的（轨道，往下滚过的高度，内容高度），绘制、点击和拖动共用。
    ///
    /// 轨道是内容区域加右边的留白；内容按显示行算，最新的一屏在最下面。
    pub(crate) fn terminal_scrollbar(&self) -> Option<(Rect, f32, f32)> {
        let tab = self.terminal.tab()?;
        let lh = self.fonts.terminal_line_height;
        let card = self.layout.terminal;
        let view = self.terminal_view();
        let track = Rect::new(view.x, view.y, card.right() - view.x, view.h);
        let (position, limit) = tab.scroll_position();
        let max = limit as f32 * lh;
        Some((track, max - position as f32 * lh, max + track.h))
    }

    /// 拖着滚动条的滑块：滑块上边跟着鼠标（`grab` 是按下点到滑块上边的距离）。
    pub(in crate::app) fn drag_terminal_scrollbar(&mut self, y: f32, grab: f32) {
        let Some((track, _, content)) = self.terminal_scrollbar() else {
            return;
        };
        let Some(thumb) = crate::layout::scrollbar_thumb(track, 0.0, content) else {
            return;
        };
        let max = content - track.h;
        let top = crate::layout::scroll_for_thumb(track.y, track.h, thumb.h, y, grab, max);
        let lh = self.fonts.terminal_line_height;
        if let Some(tab) = self.terminal.tab_mut() {
            tab.set_scroll_position(f64::from((max - top) / lh));
        }
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// ⌘K：清掉屏幕和回滚缓冲，提示符留在第一行。
    pub(in crate::app) fn clear_terminal(&mut self) {
        if let (Some(client), Some(tab)) = (&self.terminal.client, self.terminal.tab()) {
            client.send(&ToHelper::Clear { tab: tab.id });
        }
    }

    /// 往上（`lines` 为正）或往下翻回滚缓冲。
    pub(in crate::app) fn scroll_terminal(&mut self, lines: isize) {
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        tab.scroll = tab.scroll.saturating_add_signed(lines);
        tab.scroll_frac = 0.0;
        tab.clamp_scroll();
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// 焦点在终端时的按键（快捷键已经先处理过）。
    pub(in crate::app) fn on_key_terminal(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if !matches!(
            event.logical_key,
            Key::Named(
                NamedKey::Shift
                    | NamedKey::Control
                    | NamedKey::Alt
                    | NamedKey::Super
                    | NamedKey::Meta
            )
        ) && self.terminal_error_key(event.logical_key == Key::Named(NamedKey::Enter))
        {
            return;
        }
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        // shell 退出、伪终端断开（或者辅助进程退出）以后按 ↩ 重新开始。
        if tab.ended.is_some() || self.terminal.crashed {
            if event.logical_key == Key::Named(NamedKey::Enter) {
                self.request_terminal_restart();
            }
            return;
        }
        // ⇧PageUp、⇧PageDown 翻回滚缓冲（备用屏上没有回滚缓冲，交给程序）。
        let page = tab.grid_rows().saturating_sub(1).max(1) as isize;
        if self.modifiers.shift_key() && !tab.modes.has(Modes::ALT_SCREEN) {
            match event.logical_key {
                Key::Named(NamedKey::PageUp) => return self.scroll_terminal(page),
                Key::Named(NamedKey::PageDown) => return self.scroll_terminal(-page),
                _ => {}
            }
        }
        let (modes, keyboard) = (tab.modes, tab.keyboard);
        // 剪贴板里只有图片时，Ctrl+V 粘贴图片路径。这个键平时交给终端里的程序（Windows、Linux
        // 上的粘贴是 Ctrl+Shift+V），程序读不到剪贴板图片。有文字时仍把按键发给程序。
        if self.image_paste_key(event)
            && self.clipboard_text().is_none_or(|text| text.is_empty())
            && self.paste_terminal_image()
        {
            return;
        }
        if let Some(bytes) = self.encode_terminal_key(event, modes, keyboard) {
            // 输入文字时清除选区，否则 ⌘C 会复制旧内容；方向键和快捷键保留选区。
            let mods = self.terminal_mods();
            let typed = event.text.as_deref().is_some_and(|text| {
                !text.chars().any(char::is_control)
                    && !mods.ctrl
                    && !mods.alt
                    && !self.modifiers.super_key()
            });
            if typed {
                self.clear_terminal_selection();
            }
            self.terminal_input(bytes);
        }
    }

    /// 当前的修饰键，按 `terminal.optionAsMeta` 决定 ⌥ 算不算 Meta。
    pub(super) fn terminal_mods(&self) -> Mods {
        let m = self.modifiers;
        let meta = m.alt_key()
            && match self.config.option_as_meta.unwrap_or(OptionAsMeta::Left) {
                OptionAsMeta::Left => self.option_side.0,
                OptionAsMeta::Right => self.option_side.1,
                OptionAsMeta::Both => true,
                OptionAsMeta::None => false,
            };
        Mods {
            shift: m.shift_key(),
            alt: meta,
            ctrl: m.control_key(),
        }
    }

    /// Ctrl+V，没有 Shift、Alt、⌘。macOS 上粘贴是 ⌘V，已经由快捷键处理。
    fn image_paste_key(&self, event: &KeyEvent) -> bool {
        let m = self.modifiers;
        m.control_key()
            && !m.shift_key()
            && !m.alt_key()
            && !m.super_key()
            && matches!(
                event.key_without_modifiers(),
                Key::Character(text) if text.eq_ignore_ascii_case("v")
            )
    }

    /// 把一次按键编码成写给终端的字节；这个键在终端里没有意义时返回 `None`。
    fn encode_terminal_key(&self, event: &KeyEvent, modes: Modes, keyboard: u8) -> Option<Vec<u8>> {
        let m = self.modifiers;
        let option = m.alt_key();
        let mods = self.terminal_mods();

        // macOS 上文本框的按键约定：⌘← ⌘→ 到行首行尾，⌘⌫ 删到行首，⌥← ⌥→ 按词移动。
        // 其余带 ⌘ 的组合不是快捷键就不发。
        if cfg!(target_os = "macos") && m.super_key() {
            return match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(vec![0x01]),
                Key::Named(NamedKey::ArrowRight) => Some(vec![0x05]),
                Key::Named(NamedKey::Backspace) => Some(vec![0x15]),
                _ => None,
            };
        }
        if option && !mods.ctrl {
            match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => return Some(b"\x1bb".to_vec()),
                Key::Named(NamedKey::ArrowRight) => return Some(b"\x1bf".to_vec()),
                Key::Named(NamedKey::Backspace) => return Some(b"\x1b\x7f".to_vec()),
                _ => {}
            }
        }

        let key = match &event.logical_key {
            Key::Named(named) => match named {
                NamedKey::Enter => TermKey::Enter,
                NamedKey::Tab => TermKey::Tab,
                NamedKey::Backspace => TermKey::Backspace,
                NamedKey::Escape => TermKey::Escape,
                NamedKey::ArrowUp => TermKey::Up,
                NamedKey::ArrowDown => TermKey::Down,
                NamedKey::ArrowLeft => TermKey::Left,
                NamedKey::ArrowRight => TermKey::Right,
                NamedKey::Home => TermKey::Home,
                NamedKey::End => TermKey::End,
                NamedKey::PageUp => TermKey::PageUp,
                NamedKey::PageDown => TermKey::PageDown,
                NamedKey::Insert => TermKey::Insert,
                NamedKey::Delete => TermKey::Delete,
                NamedKey::Space if mods.ctrl || mods.alt => TermKey::Char(' '),
                NamedKey::Space => TermKey::Text(" "),
                NamedKey::F1 => TermKey::F(1),
                NamedKey::F2 => TermKey::F(2),
                NamedKey::F3 => TermKey::F(3),
                NamedKey::F4 => TermKey::F(4),
                NamedKey::F5 => TermKey::F(5),
                NamedKey::F6 => TermKey::F(6),
                NamedKey::F7 => TermKey::F(7),
                NamedKey::F8 => TermKey::F(8),
                NamedKey::F9 => TermKey::F(9),
                NamedKey::F10 => TermKey::F(10),
                NamedKey::F11 => TermKey::F(11),
                NamedKey::F12 => TermKey::F(12),
                _ => return None,
            },
            _ if mods.ctrl || mods.alt => {
                // ⌃、Meta 要的是键上的字符：⌥ 已经把它变成了别的字（⌥b 是 ∫），用不含修饰键的
                // 那个，再补上 ⇧。
                let Key::Character(base) = event.key_without_modifiers() else {
                    return None;
                };
                let c = latin(base.chars().next()?, event.physical_key);
                let c = if mods.shift { shifted(c) } else { c };
                TermKey::Char(c)
            }
            _ => {
                let text = event.text.as_deref()?;
                if text.chars().any(char::is_control) {
                    return None;
                }
                let mut bytes = Vec::new();
                input::encode(TermKey::Text(text), mods, modes, keyboard, &mut bytes);
                return Some(bytes);
            }
        };
        let mut bytes = Vec::new();
        input::encode(key, mods, modes, keyboard, &mut bytes).then_some(bytes)
    }
}

/// 返回 ⌃、Meta 组合按键时使用的字符：按物理键位换成美式键盘上的字符。
///
/// 俄文、希腊文等键盘上键面印的不是拉丁字母（⌃C 所在的键印的是「С」），只用键面的字符
/// 发不出 ⌃C。这里按 `key` 给出的键位取字：`KeyC` 换成 `c`，字母、数字与标点同理；键面
/// 已经是 ASCII 时原样返回。
fn latin(c: char, key: PhysicalKey) -> char {
    if c.is_ascii() {
        return c;
    }
    crate::keymap::us_char(key).unwrap_or(c)
}

/// ⇧ 加上键上的字符（美式键盘的排法）：Meta 组合要发 `ESC !` 这样的字符时用。
fn shifted(c: char) -> char {
    const PAIRS: [(char, char); 21] = [
        ('1', '!'),
        ('2', '@'),
        ('3', '#'),
        ('4', '$'),
        ('5', '%'),
        ('6', '^'),
        ('7', '&'),
        ('8', '*'),
        ('9', '('),
        ('0', ')'),
        ('-', '_'),
        ('=', '+'),
        ('[', '{'),
        (']', '}'),
        ('\\', '|'),
        (';', ':'),
        ('\'', '"'),
        (',', '<'),
        ('.', '>'),
        ('/', '?'),
        ('`', '~'),
    ];
    if c.is_ascii_lowercase() {
        return c.to_ascii_uppercase();
    }
    PAIRS
        .iter()
        .find(|&&(from, _)| from == c)
        .map_or(c, |&(_, to)| to)
}

#[cfg(test)]
mod tests;
