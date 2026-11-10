//! 输入框：找出有焦点的单行输入框，编辑、复制和剪切其中的文字。

use super::*;

impl App {
    /// 返回 `Focus::Prompt` 对应的输入框：文件树中正在输入名字的那一行，或跳转条；都没有时
    /// 返回 `None`。
    fn prompt_field_mut(&mut self) -> Option<&mut TextField> {
        if let Some(entry) = &mut self.name_input {
            return Some(&mut entry.field);
        }

        self.goto_input.as_mut()
    }

    /// 返回当前持有焦点的输入框；焦点不在输入框上时返回 `None`。
    pub(super) fn focused_field(&mut self) -> Option<&mut TextField> {
        match self.focus {
            Focus::Search => Some(&mut self.find.input),
            Focus::Replace => Some(&mut self.find.replace_input),
            Focus::Prompt => self.prompt_field_mut(),
            Focus::GlobalQuery => Some(&mut self.global.query),
            Focus::CommitMessage => Some(&mut self.commit.message),
            Focus::GlobalReplace => Some(&mut self.global.replace),
            Focus::GlobalInclude => Some(&mut self.global.include),
            Focus::GlobalExclude => Some(&mut self.global.exclude),
            Focus::Picker => Some(&mut self.picker.input),
            Focus::Setting => self.settings.editing.as_mut().map(|(_, field)| field),
            Focus::KeySearch => Some(&mut self.settings.keys_query),
            Focus::SettingsSearch => Some(&mut self.settings.search),
            Focus::LogFilter => Some(&mut self.log.filter),
            Focus::Dialog => self.form_dialog.as_mut().map(|d| {
                if d.focus == crate::layout::DialogPart::Second {
                    &mut d.second
                } else {
                    &mut d.name
                }
            }),
            Focus::TerminalFind => self.terminal.find.as_mut().map(|f| &mut f.field),
            Focus::TerminalRename => self.terminal.rename.as_mut().map(|(_, field)| field),
            Focus::Editor
            | Focus::Tree
            | Focus::Results
            | Focus::Settings
            | Focus::CommitList
            | Focus::Outline
            | Focus::Terminal => None,
        }
    }

    /// 修改当前输入框；`edit` 返回文字是否发生变化，变化时同步依赖该文字的状态。
    pub(super) fn edit_field(&mut self, edit: impl FnOnce(&mut TextField) -> bool) {
        let Some(field) = self.focused_field() else {
            return;
        };

        if edit(field) {
            match self.focus {
                Focus::Search => {
                    self.search.query = self.find.input.text().to_owned();
                    self.search_changed();
                }
                Focus::Replace => {
                    self.search.replacement = self.find.replace_input.text().to_owned()
                }
                Focus::GlobalQuery | Focus::GlobalInclude | Focus::GlobalExclude => {
                    self.global_changed(false);
                }
                Focus::Picker => self.picker_refresh(),
                Focus::Setting => self.settings_live_font(),
                Focus::LogFilter => self.log_filter_changed(),
                Focus::TerminalFind => self.terminal_find_changed(),
                Focus::SettingsSearch => self.settings_search_changed(),
                // 筛选后列表变短，滚动位置回到顶部。
                Focus::KeySearch => {
                    self.settings.scroll = 0.0;
                    self.settings.keys_recorded = false;
                }
                _ => {}
            }
        }

        self.reset_blink();
        self.request_redraw();
    }

    /// 复制输入框中选中的文字；`cut` 为 `true` 时同时删除选中的文字。
    pub(super) fn copy_field(&mut self, cut: bool) {
        let Some(text) = self
            .focused_field()
            .filter(|f| !f.is_masked())
            .map(|f| f.selected_text().to_owned())
            .filter(|t| !t.is_empty())
        else {
            return;
        };

        self.set_clipboard(text, false);

        if cut {
            self.edit_field(|f| f.delete(Unit::Char, false));
        }
    }
}
