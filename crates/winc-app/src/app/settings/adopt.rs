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
        let zoom = old.ui_zoom != new.ui_zoom;
        let fonts = (old.editor_font != new.editor_font || old.ui_font != new.ui_font)
            || (old.editor_size != new.editor_size || old.ui_size != new.ui_size)
            || (old.terminal_font != new.terminal_font || old.terminal_size != new.terminal_size);
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
        let images = old.markdown_remote_images != new.markdown_remote_images;
        let minimap = old.minimap != new.minimap
            || old.line_numbers != new.line_numbers
            || old.status_bar != new.status_bar;
        let compact = old.compact_folders != new.compact_folders;
        let diff_layout = old.diff_inline != new.diff_inline;
        let staging_view = old.git_staging_area != new.git_staging_area;
        // 窗口字体同步。
        let glyph_language = old.language != new.language
            && crate::set_locale(new.language.unwrap_or_default().resolve());
        let git = old.git_enabled != new.git_enabled;
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
        if minimap {
            self.relayout();
        }
        if compact {
            self.apply_compact_folders();
        }
        if diff_layout {
            self.apply_diff_layout();
        }
        if staging_view {
            self.apply_staging_view();
        }
        winc_editor::git::follow_system_proxy(self.config.git_system_proxy != Some(false));
        if images {
            self.preview.markdown.forget_images();
        }
        if git {
            self.git_folder_changed();
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
        self.relayout();
        self.request_redraw();
    }
}
