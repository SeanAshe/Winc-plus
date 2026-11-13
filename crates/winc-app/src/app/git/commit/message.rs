//! 提交信息输入框的按键处理。

use super::*;

impl App {
    /// 提交信息里的按键：`⌘↩` 提交；`↑` `↓` 在行之间移动，已在第一行（最后一行）时翻阅此前的
    /// 提交信息；`Tab` 移到改动列表；`Esc` 回到编辑区。
    ///
    /// 其余按键交给输入框。
    pub(in crate::app) fn on_key_commit_message(
        &mut self,
        event: &KeyEvent,
        base: &str,
        cmd: bool,
    ) {
        let shift = self.pointer.modifiers.shift_key();
        if !self.commit_message_key(&event.logical_key, cmd, shift) {
            self.on_key_field(event, base, cmd);
        }
        self.request_redraw();
    }

    /// 提交信息里自己处理的键；不是这些键时返回 `false`，交给输入框的通用处理。
    pub(in crate::app) fn commit_message_key(&mut self, key: &Key, cmd: bool, shift: bool) -> bool {
        match key {
            Key::Named(NamedKey::Enter) if cmd && self.commit_pushes() => {
                self.confirm_commit_and_push();
            }
            Key::Named(NamedKey::Enter) if cmd => self.git_commit(),
            Key::Named(NamedKey::Enter) => self.edit_field(|f| f.insert("\n")),
            // 提交信息不接受制表符，因此 `Tab` 移到改动列表；列表中按 `Tab` 返回提交信息。
            Key::Named(NamedKey::Tab) if !cmd => {
                self.ime.field_preedit.clear();
                self.focus = Focus::CommitList;
                if self.git.commit.selected.is_none()
                    && let Some(first) = self.commit_files().into_iter().next()
                {
                    self.select_commit_file(first);
                }
            }
            Key::Named(NamedKey::Escape) => {
                self.ime.field_preedit.clear();
                if self.editor.active.is_some() {
                    self.focus = Focus::Editor;
                }
            }
            // ↑ ↓ 在行之间移动。信息为空、或者正在翻阅此前的信息时，↑ ↓ 翻阅提交过的信息；
            // 写了一半的信息不会因为在第一行按了 ↑ 就被换掉。
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if !cmd => {
                let down = *key == Key::Named(NamedKey::ArrowDown);
                let message = &self.git.commit.message;
                let browse = !shift && (message.text().is_empty() || message.recalling());
                if browse {
                    self.edit_field(|f| f.recall(!down));
                } else {
                    self.git.commit.message.move_vertical(down, shift);
                }
                self.reset_blink();
            }
            _ => return false,
        }
        true
    }
}
