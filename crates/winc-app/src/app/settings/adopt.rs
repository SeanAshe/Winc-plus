//! 采用配置文件中的新配置：按变化的项更新本窗口。

use super::*;

impl App {
    /// 改用配置文件中刚写好的配置 `config`，按变化的项更新本窗口。
    ///
    /// `config` 由另一个窗口写入，或者是本窗口写回时文件中带有手动修改的项。更新的内容包括字体和
    /// 字号、Tab 宽度、主题和各开关，效果与在本窗口的设置页中修改相同，只是不再写文件。
    pub(crate) fn adopt_config(&mut self, config: Config) {
        let old = std::mem::replace(&mut self.config, config);
        let new = &self.config;
        if old.keymap != new.keymap || old.shortcuts != new.shortcuts {
            for problem in crate::keymap::install_from(new) {
                eprintln!("winc: {problem}");
            }
            self.keymap_changed();
        }
        let new = &self.config;
        let zoom = old.ui_zoom != new.ui_zoom;
        let fonts = (old.editor_font != new.editor_font || old.ui_font != new.ui_font)
            || (old.editor_size != new.editor_size || old.ui_size != new.ui_size)
            || (old.terminal_font != new.terminal_font || old.terminal_size != new.terminal_size)
            || (old.markdown_font != new.markdown_font || old.markdown_size != new.markdown_size)
            || (old.ui_font_weight != new.ui_font_weight
                || old.editor_font_weight != new.editor_font_weight
                || old.terminal_font_weight != new.terminal_font_weight);
        let theme = (old.theme != new.theme || old.dark_theme != new.dark_theme)
            || old.light_theme != new.light_theme;
        let tab_width = (old.tab_width != new.tab_width)
            .then_some(new.tab_width)
            .flatten();
        let terminal = old.terminal_minimum_contrast != new.terminal_minimum_contrast
            || old.terminal_bold_is_bright != new.terminal_bold_is_bright;
        let (wrap, sticky) = (
            old.word_wrap != new.word_wrap,
            old.sticky_scroll != new.sticky_scroll,
        );
        let align = (old.align_columns != new.align_columns).then_some(new.align_columns);
        let images = old.markdown_remote_images != new.markdown_remote_images;
        let minimap = old.minimap != new.minimap
            || old.line_numbers != new.line_numbers
            || old.status_bar != new.status_bar;
        let compact = old.compact_folders != new.compact_folders;
        let diff_layout = old.diff_inline != new.diff_inline;
        let diff_collapse = old.diff_collapse_unchanged != new.diff_collapse_unchanged;
        let staging_view = old.git_staging_area != new.git_staging_area;
        let combined_view = old.git_combined_diff != new.git_combined_diff;
        // 窗口字体同步。
        let glyph_language = old.language != new.language
            && crate::set_locale(new.language.unwrap_or_default().resolve());
        let git = old.git_enabled != new.git_enabled;
        let git_program = old.git_path != new.git_path;
        let scan_depth = old.repository_scan_depth != new.repository_scan_depth;
        let forget_credentials = old.git_remember_credentials != Some(false)
            && new.git_remember_credentials == Some(false);
        let title_bar = old.ui_native_title_bar != new.ui_native_title_bar;
        let stop_terminal =
            old.terminal_enabled != new.terminal_enabled && new.terminal_enabled == Some(false);
        let stop_remote =
            old.remote_enabled != new.remote_enabled && new.remote_enabled != Some(true);

        if fonts || glyph_language {
            // 别的窗口已经用上了这些字体，这里也用得上；万一用不上，留着原来的字体。
            let _ = self.apply_fonts();
        }
        if zoom {
            self.apply_zoom();
        }
        if let Some(width) = tab_width {
            self.editor.tab_width = width;
            self.ensure_caret_visible();
        }
        if theme {
            self.apply_theme(self.config.theme.unwrap_or(ThemeChoice::Auto));
        } else if terminal {
            self.terminal_theme_changed();
        }
        if sticky {
            self.caches.sticky_cache = None;
        }
        if wrap {
            self.word_wrap_changed();
        }
        if let Some(align) = align {
            self.editor.set_align_columns(align != Some(false));
        }
        if minimap {
            self.relayout();
        }
        if compact {
            self.apply_compact_folders();
        }
        if diff_collapse {
            self.apply_diff_collapse();
        }
        if diff_layout {
            self.apply_diff_layout();
        }
        if staging_view {
            self.apply_staging_view();
        }
        if combined_view {
            self.apply_combined_view();
        }
        if old.git_log_placement != self.config.git_log_placement {
            self.apply_log_placement(false);
        }
        winc_editor::proxy::follow_system_proxy(self.config.system_proxy != Some(false));
        if images {
            self.preview.markdown.forget_images();
        }
        if git {
            self.git_folder_changed();
        } else if git_program {
            self.git_program_changed();
        } else if scan_depth {
            self.git_scan_repositories();
        }
        if forget_credentials {
            crate::app::git::question::forget_credentials();
        }
        if stop_terminal {
            self.stop_terminal();
        }
        if stop_remote {
            self.stop_remote();
        }
        // 模块关闭后它的那一页不再列出：设置停在该页时改到「模块」页。
        let page = self.settings.selected.page();
        if self.settings.open && !SettingsPage::listed(self.settings_modules()).any(|p| p == page) {
            self.settings_show_page(SettingsPage::Modules);
        }
        self.reload_run_configs();
        if title_bar {
            self.sync_title_bar();
        }
        self.vim_apply_config();
        if self.settings.open {
            self.refresh_changed_settings();
        }
        self.relayout();
        self.request_redraw();
    }
}
