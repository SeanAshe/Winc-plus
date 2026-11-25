//! 设置项的取值：菜单类设置显示的文字，以及哪些设置项改过、怎样恢复默认。

use super::*;

impl App {
    /// 返回点击后弹出菜单的设置项 `setting` 显示的文字，以及是否写成淡色（没有选择时）。
    pub(crate) fn choice_text(&self, setting: Setting) -> (String, bool) {
        if setting == Setting::Language {
            let choice = self.config.language.unwrap_or_default();
            (crate::layout::locale_label(choice).to_owned(), false)
        } else if setting == Setting::RenderWhitespace {
            let choice = self.config.render_whitespace.unwrap_or_default();
            (crate::layout::whitespace_label(choice).to_owned(), false)
        } else if setting == Setting::AutoSave {
            let choice = self.config.auto_save.unwrap_or_default();
            (crate::layout::auto_save_label(choice).to_owned(), false)
        } else if setting == Setting::SmoothCaret {
            let choice = self.config.smooth_caret_movement.unwrap_or_default();
            (crate::layout::smooth_caret_label(choice).to_owned(), false)
        } else if setting == Setting::DropFolder {
            let choice = self.config.drop_folder.unwrap_or_default();
            (crate::layout::drop_folder_label(choice).to_owned(), false)
        } else if setting == Setting::FindInFiles {
            let choice = self.search_style();
            (crate::layout::search_style_label(choice).to_owned(), false)
        } else if setting == Setting::GitOpenWorktree {
            let choice = self.config.git_open_worktree.unwrap_or_default();
            (crate::layout::open_worktree_label(choice).to_owned(), false)
        } else if setting == Setting::GitPullMode {
            let choice = self.config.git_pull.unwrap_or_default();
            (crate::layout::pull_mode_label(choice).to_owned(), false)
        } else if setting == Setting::VimClipboard {
            let choice = self.config.vim_clipboard.unwrap_or_default();
            (crate::layout::vim_clipboard_label(choice).to_owned(), false)
        } else if let Setting::VimKey(c) = setting {
            let owner = crate::layout::vim_key_owner_label(self.vim_owns_key(c));
            (owner.to_owned(), false)
        } else if setting == Setting::Keymap {
            (crate::keymap::current_preset().title().to_owned(), false)
        } else if setting == Setting::Zoom {
            (format!("{}%", self.config.ui_zoom.unwrap_or(100)), false)
        } else if setting == Setting::DefaultLanguage {
            let name = match self.default_language() {
                Some(language) => language.name,
                None => crate::app::MenuCommand::UsePlainText.label(),
            };
            (name.to_owned(), false)
        } else if setting == Setting::TextWidth {
            let choice = self.config.markdown_text_width.unwrap_or_default();
            (crate::layout::text_width_label(choice).to_owned(), false)
        } else if setting == Setting::BackgroundFit {
            let choice = self.config.ui_background_fit.unwrap_or_default();
            (
                crate::layout::background_fit_label(choice).to_owned(),
                false,
            )
        } else if setting == Setting::BackgroundImage {
            let name = (self.config.ui_background_image.as_deref())
                .map(|path| winc_editor::display_name(std::path::Path::new(path)).into_owned());
            let none = name.is_none();
            (
                name.unwrap_or_else(|| t!("settings-background-none").to_owned()),
                none,
            )
        } else if setting == Setting::TerminalPosition {
            let text = if self.terminal_right() {
                t!("settings-terminal-position-right")
            } else {
                t!("settings-terminal-position-bottom")
            };
            (text.to_owned(), false)
        } else if setting == Setting::TerminalTabPosition {
            let text = if self.config.terminal_vertical_tabs == Some(true) {
                t!("settings-terminal-tab-position-right")
            } else {
                t!("settings-terminal-tab-position-top")
            };
            (text.to_owned(), false)
        } else if setting == Setting::TerminalOptionAsMeta {
            let choice = (self.config.option_as_meta).unwrap_or(winc_config::OptionAsMeta::Left);
            (
                crate::layout::option_as_meta_label(choice).to_owned(),
                false,
            )
        } else {
            let none = || t!("settings-skip-on-save-none").to_owned();
            (self.skipped_on_save().unwrap_or_else(none), false)
        }
    }

    /// `setting` 是否写在配置文件里；没有对应配置项的为否。
    pub(crate) fn setting_is_set(&self, setting: Setting) -> bool {
        setting.key().is_some_and(|key| self.config.is_set(key))
    }

    /// 返回 `setting` 当前取值的文字，只用来比较两份配置下的取值是否相同。
    fn setting_shown(&self, setting: Setting) -> String {
        match setting.control() {
            SettingControl::Switch => self.switch_on(setting).to_string(),
            SettingControl::Stepper => self.setting_value(setting).0,
            SettingControl::Choice => self.choice_text(setting).0,
            SettingControl::Themes => {
                format!("{:?}", self.config.theme.unwrap_or(ThemeChoice::Auto))
            }
            _ => self.setting_is_set(setting).to_string(),
        }
    }

    /// 重新找出取值与默认不同的设置项（`SettingsState::changed`）：写在配置文件里，并且去掉
    /// 这一项后取值会变的。
    pub(in crate::app) fn refresh_changed_settings(&mut self) {
        let modules = crate::layout::Modules {
            git: true,
            terminal: true,
        };
        let mut changed = Vec::new();
        for setting in Setting::all(&self.settings.listed, modules) {
            let Some(key) = setting.key().filter(|key| self.config.is_set(key)) else {
                continue;
            };
            let now = self.setting_shown(setting);
            let mut plain = self.config.clone();
            plain.unset(key);
            let written = std::mem::replace(&mut self.config, plain);
            let default = self.setting_shown(setting);
            self.config = written;
            if now != default {
                changed.push(setting);
            }
        }
        // 各键的归属合在一项配置里，逐键比较默认的归属。
        let keys = self.settings.listed.vim_keys.iter();
        changed.extend(
            keys.filter(|&&c| self.vim_key_changed(c))
                .map(|&c| Setting::VimKey(c)),
        );
        self.settings.changed = changed;
    }

    /// 把 `setting` 恢复为默认：去掉配置文件中的这一项，效果与在设置页中改回默认相同。
    pub(in crate::app) fn settings_reset(&mut self, setting: Setting) {
        if let Setting::VimKey(c) = setting {
            self.settings_leave_field();
            self.reset_vim_key(c);
            return;
        }
        let Some(key) = setting.key().filter(|key| self.config.is_set(key)) else {
            return;
        };
        self.settings_leave_field();
        let mut config = self.config.clone();
        config.unset(key);
        self.adopt_config(config);
        self.settings.error = None;
        self.save_config();
    }
}
