//! 设置中的开关：读出当前的状态，切换并让改动生效。

use super::*;

impl App {
    /// 开关 `setting` 当前是否开启。
    pub(crate) fn switch_on(&self, setting: Setting) -> bool {
        match setting {
            Setting::FormatOnSave => self.config.format_on_save == Some(true),
            Setting::FixOnSave => self.config.fix_on_save == Some(true),
            Setting::HotExit => self.config.hot_exit != Some(false),
            Setting::ConfirmClose => self.config.confirm_close == Some(true),
            Setting::RestoreFolder => self.config.restore_folder != Some(false),
            Setting::NewFileOnStartup => self.config.new_file_on_startup == Some(true),
            Setting::DetectLanguageOnPaste => self.config.detect_language_on_paste != Some(false),
            Setting::StickyScroll => self.config.sticky_scroll != Some(false),
            Setting::VimMode => self.config.vim_enabled == Some(true),
            Setting::VimRelativeNumber => self.config.vim_relative_number == Some(true),
            Setting::VimYankHighlight => self.config.vim_yank_highlight != Some(0),
            Setting::WordWrap => self.config.word_wrap == Some(true),
            Setting::WholeLines => self.config.whole_lines == Some(true),
            Setting::Minimap => self.config.minimap == Some(true),
            Setting::Outline => self.config.outline != Some(false),
            Setting::LineNumbers => self.config.line_numbers != Some(false),
            Setting::DiffInline => self.config.diff_inline == Some(true),
            Setting::StatusBar => self.config.status_bar != Some(false),
            Setting::RestoreWindow => self.config.restore_window != Some(false),
            Setting::Animations => self.animations_on(),
            Setting::SmoothScrolling => self.smooth_scrolling(),
            Setting::CompactFolders => self.config.compact_folders != Some(false),
            Setting::PreviewOnClick => self.config.preview_on_click == Some(true),
            Setting::RemoteImages => self.config.markdown_remote_images != Some(false),
            Setting::MarkdownLineNumbers => self.config.markdown_line_numbers != Some(false),
            Setting::MarkdownShortcuts => self.config.markdown_shortcuts != Some(false),
            Setting::MarkdownFocus => self.config.markdown_focus == Some(true),
            Setting::MarkdownTypewriter => self.config.markdown_typewriter == Some(true),
            Setting::MarkdownSelectionToolbar => {
                self.config.markdown_selection_toolbar != Some(false)
            }
            Setting::CheckUpdates => self.config.update_auto_check != Some(false),
            Setting::SystemTitleBar => self.config.ui_native_title_bar == Some(true),
            Setting::GitModule => self.git_enabled(),
            Setting::GitConfirmPush => self.config.git_confirm_push != Some(false),
            Setting::GitRememberCredentials => self.config.git_remember_credentials != Some(false),
            Setting::SystemProxy => self.config.system_proxy != Some(false),
            Setting::GitInlineBlame => self.config.git_inline_blame != Some(false),
            Setting::GitAutoFetch => self.config.git_auto_fetch == Some(true),
            Setting::GitTree => !self.git.commit.flat(),
            Setting::GitStagingArea => self.config.git_staging_area == Some(true),
            Setting::TerminalModule => self.terminal_enabled(),
            Setting::RemoteModule => self.remote_enabled(),
            Setting::ExplorerMenu => self.settings.integration.menu,
            Setting::FileAssociations => self.settings.integration.types,
            Setting::TerminalCopyOnSelect => self.config.terminal_copy_on_select == Some(true),
            Setting::TerminalConfirmClose => self.config.terminal_confirm_close != Some(false),
            Setting::TerminalPrompt => self.config.terminal_prompt != Some(false),
            Setting::TerminalApplicationPriority => {
                self.config.terminal_application_priority == Some(true)
            }
            Setting::TerminalShellIntegration => {
                self.config.terminal_shell_integration != Some(false)
            }
            _ => false,
        }
    }

    /// 切换开关 `setting` 的状态。
    pub(in crate::app) fn settings_toggle(&mut self, setting: Setting) {
        if setting == Setting::StickyScroll {
            self.caches.sticky_cache = None;
        }
        self.settings_set_switch(setting, !self.switch_on(setting));
    }

    pub(super) fn settings_set_switch(&mut self, setting: Setting, on: bool) {
        if self.switch_on(setting) == on {
            return;
        }
        self.vim_forget_option(setting);
        match setting {
            Setting::FormatOnSave => self.config.format_on_save = Some(on),
            Setting::FixOnSave => self.config.fix_on_save = Some(on),
            Setting::HotExit => self.config.hot_exit = Some(on),
            Setting::ConfirmClose => self.config.confirm_close = Some(on),
            Setting::RestoreFolder => self.config.restore_folder = Some(on),
            Setting::NewFileOnStartup => self.config.new_file_on_startup = Some(on),
            Setting::DetectLanguageOnPaste => self.config.detect_language_on_paste = Some(on),
            Setting::StickyScroll => self.config.sticky_scroll = Some(on),
            Setting::VimMode => {
                self.config.vim_enabled = Some(on);
                self.vim_apply_config();
            }
            Setting::VimRelativeNumber => self.config.vim_relative_number = Some(on),
            // 关闭记为 0 毫秒，打开恢复默认的时长。
            Setting::VimYankHighlight => self.config.vim_yank_highlight = (!on).then_some(0),
            Setting::WordWrap => self.config.word_wrap = Some(on),
            Setting::WholeLines => self.config.whole_lines = Some(on),
            Setting::Minimap => self.config.minimap = Some(on),
            Setting::Outline => self.config.outline = Some(on),
            Setting::LineNumbers => self.config.line_numbers = Some(on),
            Setting::DiffInline => self.config.diff_inline = Some(on),
            Setting::StatusBar => self.config.status_bar = Some(on),
            Setting::RestoreWindow => self.config.restore_window = Some(on),
            Setting::Animations => self.config.ui_animations = Some(on),
            Setting::SmoothScrolling => self.config.smooth_scrolling = Some(on),
            Setting::CompactFolders => self.config.compact_folders = Some(on),
            Setting::PreviewOnClick => self.config.preview_on_click = Some(on),
            Setting::RemoteImages => self.config.markdown_remote_images = Some(on),
            Setting::MarkdownLineNumbers => self.config.markdown_line_numbers = Some(on),
            Setting::MarkdownShortcuts => self.config.markdown_shortcuts = Some(on),
            Setting::MarkdownFocus => self.config.markdown_focus = Some(on),
            Setting::MarkdownTypewriter => self.config.markdown_typewriter = Some(on),
            Setting::MarkdownSelectionToolbar => self.config.markdown_selection_toolbar = Some(on),
            Setting::CheckUpdates => self.config.update_auto_check = Some(on),
            Setting::SystemTitleBar => self.config.ui_native_title_bar = Some(on),
            Setting::GitModule => self.config.git_enabled = Some(on),
            Setting::GitConfirmPush => self.config.git_confirm_push = Some(on),
            Setting::GitRememberCredentials => {
                self.config.git_remember_credentials = Some(on);
                if !on {
                    crate::app::git::question::forget_credentials();
                }
            }
            Setting::GitTree if on == self.git.commit.flat() => self.toggle_commit_layout(),
            Setting::GitTree => {}
            Setting::SystemProxy => {
                self.config.system_proxy = Some(on);
                winc_editor::proxy::follow_system_proxy(on);
            }
            Setting::GitStagingArea => self.config.git_staging_area = Some(on),
            Setting::GitInlineBlame => self.config.git_inline_blame = Some(on),
            Setting::GitAutoFetch => self.config.git_auto_fetch = Some(on),
            Setting::TerminalModule => self.config.terminal_enabled = Some(on),
            Setting::RemoteModule => self.config.remote_enabled = Some(on),
            Setting::ExplorerMenu => {
                crate::shell_integration::show_menu(on);
                self.settings.integration = crate::shell_integration::read();
            }
            Setting::FileAssociations => {
                crate::shell_integration::list_types(on);
                self.settings.integration = crate::shell_integration::read();
            }
            Setting::TerminalCopyOnSelect => self.config.terminal_copy_on_select = Some(on),
            Setting::TerminalConfirmClose => self.config.terminal_confirm_close = Some(on),
            Setting::TerminalPrompt => self.config.terminal_prompt = Some(on),
            Setting::TerminalApplicationPriority => {
                self.config.terminal_application_priority = Some(on)
            }
            Setting::TerminalShellIntegration => self.config.terminal_shell_integration = Some(on),
            _ => return,
        }
        // 由命令切换（如 ⌥Z 自动换行）时看不到开关，不必播放动画；开关在设置浮层和「首次设置」
        // 中可见。
        let visible = self.settings.open || self.setup_shown();
        self.settings.switch_slide = self
            .clock(super::animation::SWITCH_SLIDE)
            .filter(|_| visible)
            .map(|clock| (setting, clock));
        if setting == Setting::WordWrap {
            self.word_wrap_changed();
        }
        if matches!(
            setting,
            Setting::Minimap | Setting::LineNumbers | Setting::StatusBar | Setting::Outline
        ) {
            self.relayout();
        }
        // 关掉大纲时焦点在大纲上的，交给文件树。
        if setting == Setting::Outline && !on && self.focus == Focus::Outline {
            self.focus = Focus::Tree;
        }
        if setting == Setting::DiffInline {
            self.apply_diff_layout();
        }
        if setting == Setting::GitStagingArea {
            self.apply_staging_view();
        }
        // 关闭预览标签时，现有的预览标签成为普通标签。
        if setting == Setting::PreviewOnClick && !on {
            self.preview_tab = None;
        }
        // 关掉以后不再沿用：之后再打开时从下一次关闭窗口开始记录。
        if setting == Setting::RestoreWindow && !on {
            self.update_state(move |state| state.window = None);
        }
        if setting == Setting::CompactFolders {
            self.apply_compact_folders();
        }
        if setting == Setting::RemoteImages {
            self.preview.markdown.forget_images();
        }
        if setting == Setting::GitModule {
            self.git_folder_changed();
        }
        if setting == Setting::RemoteModule && !on {
            self.stop_remote();
        }
        if setting == Setting::SystemTitleBar {
            self.sync_title_bar();
            self.relayout();
        }
        // 关闭终端模块时辅助进程和里面的 shell 一起结束；开着时等用户按 ⌃` 再启动。
        if setting == Setting::TerminalModule {
            if !on {
                self.stop_terminal();
            }
            self.reload_run_configs();
        }
        self.save_config();
        self.request_redraw();
    }
}
