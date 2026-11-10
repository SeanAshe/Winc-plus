//! 提示框：确认操作、选择怎么处理（保存修改、在哪里打开目录……）。
//!
//! 调用方用 [`App::show_alert`]、[`App::show_confirm`] 给出文字和按钮，以及选中按钮以后要
//! 处理的事件。macOS 上弹出系统的提示框（sheet），选中后把事件送回事件循环；其他平台在窗口
//! 里画一张卡片，由应用自己处理点击和按键，选中后直接处理事件。打开期间是
//! 模态的：悬停、点击、按键只给提示框。

use winc_i18n::t;
use winit::keyboard::{Key, NamedKey};

use super::*;

/// 按钮的快捷键（↩ 以外的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlertKey {
    /// Esc：「取消」。
    Escape,
    /// ⌘ 加此字母（其他平台是 Ctrl），例如「不保存」的 ⌘D。
    Command(char),
}

/// 提示框上的一个按钮。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlertButton {
    /// 按钮上的文字。
    pub title: String,
    /// 快捷键；`None` 时第一个按钮按 ↩ 触发（默认按钮），其余没有快捷键。
    pub key: Option<AlertKey>,
    /// 是否为危险操作：文字用错误色（macOS 11 起系统画成红字）。
    pub destructive: bool,
}

impl AlertButton {
    /// 普通按钮。
    pub(crate) fn new(title: &str) -> AlertButton {
        AlertButton {
            title: title.to_owned(),
            key: None,
            destructive: false,
        }
    }

    /// 「取消」按钮，按 Esc 触发。
    pub(crate) fn cancel(title: &str) -> AlertButton {
        AlertButton {
            key: Some(AlertKey::Escape),
            ..AlertButton::new(title)
        }
    }

    /// 危险操作的按钮。
    pub(crate) fn destructive(title: &str) -> AlertButton {
        AlertButton {
            destructive: true,
            ..AlertButton::new(title)
        }
    }
}

/// 选中按钮以后要处理的事件：参数是按钮的序号和「不再询问」勾选框是否勾上，返回 `None` 时
/// 不做任何事。
type AlertDone = Box<dyn Fn(usize, bool) -> Option<UserEvent>>;

/// 窗口里画出来的提示框（没有系统提示框时）。
pub(crate) struct InAppAlert {
    /// 粗体的主句。
    pub message: String,
    /// 下方的说明，可以为空。
    pub detail: String,
    /// 按钮，顺序同 [`App::show_alert`] 的参数：第一个在最右边。
    pub buttons: Vec<AlertButton>,
    /// 勾选框的文字（例如「不再询问」）；没有勾选框时为 `None`。
    pub suppression: Option<String>,
    /// 勾选框是否勾上。
    pub checked: bool,
    /// 鼠标停在的部分。
    pub hover: Option<AlertPart>,
    /// 绘制时记下的几何。
    pub parts: AlertParts,
    done: AlertDone,
}

impl std::fmt::Debug for InAppAlert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InAppAlert")
            .field("message", &self.message)
            .field("buttons", &self.buttons)
            .finish_non_exhaustive()
    }
}

/// 提示框上能点的部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlertPart {
    /// 第几个按钮（序号同 `buttons`）。
    Button(usize),
    /// 勾选框（连同它的文字）。
    Checkbox,
}

/// 提示框绘制时记下的几何。
#[derive(Debug, Clone, Default)]
pub(crate) struct AlertParts {
    /// 各按钮，序号同 `buttons`。
    pub buttons: Vec<Rect>,
    /// 勾选框连同文字；没有勾选框时为 `None`。
    pub checkbox: Option<Rect>,
}

impl InAppAlert {
    /// 点 `(x, y)` 落在哪个部分上。
    fn part_at(&self, x: f32, y: f32) -> Option<AlertPart> {
        if let Some(i) = self.parts.buttons.iter().position(|r| r.contains(x, y)) {
            return Some(AlertPart::Button(i));
        }
        self.parts
            .checkbox
            .filter(|r| r.contains(x, y))
            .map(|_| AlertPart::Checkbox)
    }

    /// 按键 `key`（`command` 为按着 ⌘，其他平台是 Ctrl）对应的按钮。
    fn button_for(&self, key: &Key, command: bool) -> Option<usize> {
        match key {
            Key::Named(NamedKey::Enter) => {
                self.buttons.first().filter(|b| b.key.is_none()).map(|_| 0)
            }
            Key::Named(NamedKey::Escape) => self
                .buttons
                .iter()
                .position(|b| b.key == Some(AlertKey::Escape)),
            Key::Character(c) if command => {
                let c = c.chars().next()?.to_ascii_lowercase();
                self.buttons
                    .iter()
                    .position(|b| b.key == Some(AlertKey::Command(c)))
            }
            _ => None,
        }
    }
}

impl App {
    /// 弹出提示框，返回是否弹出；已经有一个提示框或面板时不弹出，并提示用户先关掉它。
    ///
    /// `message` 是粗体的主句，`detail` 是下方的说明（空串不显示）。`buttons` 的第一个是默认
    /// 按钮，排在最右边，其余向左排。`suppression` 是勾选框的文字和一开始是否勾上；选中按钮后
    /// 以按钮的序号和勾选框状态调用 `done`。
    pub(in crate::app) fn show_alert(
        &mut self,
        message: &str,
        detail: &str,
        buttons: Vec<AlertButton>,
        suppression: Option<(&str, bool)>,
        done: impl Fn(usize, bool) -> Option<UserEvent> + 'static,
    ) -> bool {
        let shown = self.present_alert(message, detail, buttons, suppression, done);
        self.dialog_shown(shown)
    }

    /// 同 [`App::show_alert`]，没有弹出时不提示：调用方自己处理（下次再问、说明另外的原因）。
    pub(in crate::app) fn present_alert(
        &mut self,
        message: &str,
        detail: &str,
        buttons: Vec<AlertButton>,
        suppression: Option<(&str, bool)>,
        done: impl Fn(usize, bool) -> Option<UserEvent> + 'static,
    ) -> bool {
        #[cfg(target_os = "macos")]
        if let (Some(window), Some(proxy)) = (self.window.handle.clone(), self.proxy.clone()) {
            use crate::macos::{MOD_COMMAND, alert};
            let keys: Vec<Option<String>> = buttons
                .iter()
                .map(|b| match b.key {
                    Some(AlertKey::Escape) => Some("\u{1b}".to_owned()),
                    Some(AlertKey::Command(c)) => Some(c.to_string()),
                    None => None,
                })
                .collect();
            let native: Vec<crate::macos::AlertButton<'_>> = buttons
                .iter()
                .zip(&keys)
                .map(|(b, key)| crate::macos::AlertButton {
                    title: &b.title,
                    key: key.as_deref().map(|k| {
                        let modifiers = if b.key == Some(AlertKey::Escape) {
                            0
                        } else {
                            MOD_COMMAND
                        };
                        (k, modifiers)
                    }),
                    destructive: b.destructive,
                })
                .collect();
            return alert(
                &window,
                message,
                detail,
                &native,
                suppression,
                move |i, c| {
                    if let Some(event) = done(i, c) {
                        let _ = proxy.send_event(event);
                    }
                },
            );
        }

        if self.alert.is_some() {
            return false;
        }
        self.alert = Some(InAppAlert {
            message: message.to_owned(),
            detail: detail.to_owned(),
            buttons,
            suppression: suppression.map(|(text, _)| text.to_owned()),
            checked: suppression.is_some_and(|(_, checked)| checked),
            hover: None,
            parts: AlertParts::default(),
            done: Box::new(done),
        });
        self.set_hover(Hover::None);
        self.request_redraw();
        true
    }

    /// 弹出确认危险操作的提示框。
    ///
    /// `action` 是红字的确认按钮（默认按钮），另一个是「取消」（Esc）；选择后以「是否确认」
    /// 调用 `done`。返回值同 [`App::show_alert`]。
    pub(in crate::app) fn show_confirm(
        &mut self,
        message: &str,
        detail: &str,
        action: &str,
        done: impl Fn(bool) -> Option<UserEvent> + 'static,
    ) -> bool {
        let buttons = vec![
            AlertButton::destructive(action),
            AlertButton::cancel(t!("common-cancel")),
        ];
        self.show_alert(message, detail, buttons, None, move |i, _| done(i == 0))
    }

    /// 选中第 `index` 个按钮：关掉提示框，处理它的事件。
    fn press_alert(&mut self, index: usize) {
        let Some(alert) = self.alert.take() else {
            return;
        };
        self.set_hover(self.hover_at(self.mouse.0, self.mouse.1));
        self.request_redraw();
        if let Some(event) = (alert.done)(index, alert.checked) {
            self.handle_event(event);
        }
    }

    /// 鼠标移到 `(x, y)`：窗口内的提示框打开时返回 `true`，并只更新提示框的悬停状态。
    pub(in crate::app) fn alert_mouse_move(&mut self, x: f32, y: f32) -> bool {
        let Some(alert) = &mut self.alert else {
            return false;
        };
        let part = alert.part_at(x, y);
        if part != alert.hover {
            alert.hover = part;
            self.request_redraw();
        }
        true
    }

    /// 鼠标按下：窗口内的提示框打开时返回 `true`。
    ///
    /// 点在按钮上选中它，点在勾选框上切换；点在空白处不做什么（提示框要用按钮关掉）。
    pub(in crate::app) fn alert_mouse_down(&mut self) -> bool {
        let (x, y) = self.mouse;
        let Some(alert) = &mut self.alert else {
            return false;
        };
        match alert.part_at(x, y) {
            Some(AlertPart::Button(i)) => self.press_alert(i),
            Some(AlertPart::Checkbox) => {
                alert.checked = !alert.checked;
                self.request_redraw();
            }
            None => {}
        }
        true
    }

    /// 按键：窗口内的提示框打开时返回 `true`，其他快捷键都不处理。
    ///
    /// ↩ 选中默认按钮，Esc 选中「取消」，⌘ 加字母（其他平台是 Ctrl）选中绑了该字母的按钮，
    /// 空格切换勾选框。
    pub(in crate::app) fn alert_key(&mut self, key: &Key, command: bool) -> bool {
        let Some(alert) = &mut self.alert else {
            return false;
        };
        if let Some(i) = alert.button_for(key, command) {
            self.press_alert(i);
        } else if *key == Key::Named(NamedKey::Space) && alert.suppression.is_some() {
            alert.checked = !alert.checked;
            self.request_redraw();
        }
        true
    }
}
