//! 外观：界面缩放、字体、字号和主题。

use super::*;

impl App {
    /// 返回界面缩放倍数（配置中的 `ui.zoom` 除以 100，未设置时为 1）。
    pub(crate) fn zoom(&self) -> f32 {
        f32::from(self.config.ui_zoom.unwrap_or(100)) / 100.0
    }

    /// 把界面缩放倍数交给渲染器和红绿灯；新窗口建好时与缩放改变时调用。
    pub(in crate::app) fn sync_zoom(&mut self) {
        let zoom = self.zoom();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_zoom(zoom);
        }
        #[cfg(target_os = "macos")]
        if let Some(lights) = &self.traffic_lights {
            lights.set_zoom(f64::from(zoom));
        }
        self.ime.last_area = None;
    }

    /// 按配置应用界面缩放：整个窗口按新的倍数绘制，窗口的物理尺寸不变。
    ///
    /// 布局按新的逻辑尺寸重新计算，红绿灯随顶栏的高度重新摆放；编辑区首行不变，滚动位置只限制
    /// 回有效范围。
    pub(in crate::app) fn apply_zoom(&mut self) {
        self.sync_zoom();
        self.place_traffic_lights();
        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 按当前字号重新测量字体度量；字号或字体变化后调用。
    pub(in crate::app) fn measure_fonts(&mut self) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        self.ui_metrics = renderer.metrics(FontKind::Ui, self.fonts.ui);
        self.mono_metrics = renderer.metrics(FontKind::Mono, self.fonts.code);
        self.cell = self.mono_metrics.cell;
        self.terminal_metrics = renderer.metrics(FontKind::Terminal, self.fonts.terminal);
        self.terminal_cell = self.terminal_metrics.cell;
    }

    /// 按配置应用新的字号（逻辑像素）。
    ///
    /// 行高、可见行数和行号栏宽度随之变化；滚动位置限制回有效范围，并滚动到光标可见。终端的
    /// 行列数在下一次排版后随之变化。
    pub(in crate::app) fn apply_font_sizes(&mut self) {
        self.fonts = ViewFonts::of(&self.config);
        self.measure_fonts();

        self.clamp_scroll();
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 返回配置中的编辑区字体、界面字体和终端字体；未配置时为 `None`。
    ///
    /// 未配置时渲染层使用内置的系统字体路径表。按名字查询 CoreText 一次约 4–7 ms，
    /// 路径表只需几次 `stat`，因此未配置时不调用 CoreText。终端字体与编辑区字体相同时同样为
    /// `None`，不重复加载。
    pub(in crate::app) fn font_specs(&self) -> [Option<FontSpec>; 3] {
        let terminal = configured_font(&self.config.terminal_font);
        let editor = configured_font(&self.config.editor_font);
        let terminal = terminal.filter(|spec| Some(spec) != editor.as_ref());
        [editor, configured_font(&self.config.ui_font), terminal]
    }

    /// 返回全部文字的字重（`ui.fontWeight`），未配置时为 400。
    pub(in crate::app) fn font_weight(&self) -> u16 {
        self.config.ui_font_weight.unwrap_or(Config::FONT_WEIGHT)
    }

    /// 应用主题选择：`Auto` 交由系统决定，`Dark`、`Light` 固定为指定外观。
    pub(in crate::app) fn apply_theme(&mut self, choice: ThemeChoice) {
        self.theme_choice = choice;
        // 同步窗口外观：系统菜单、提示框和面板按窗口外观显示深浅，需与界面配色一致。
        if let Some(window) = &self.window.handle {
            window.set_theme(forced_theme(&choice));
        }

        match choice {
            ThemeChoice::Auto => {
                // 改为跟随系统：先查询一次当前外观，此后系统切换外观时会收到 `ThemeChanged`。
                let dark = self
                    .window
                    .handle
                    .as_ref()
                    .and_then(|w| w.theme())
                    .map_or(self.palette.dark, |theme| theme != Theme::Light);
                self.set_theme(if dark { Theme::Dark } else { Theme::Light });
            }
            ThemeChoice::Dark => self.set_theme(Theme::Dark),
            ThemeChoice::Light => self.set_theme(Theme::Light),
        }
    }

    /// 深色（`dark`）或浅色外观时用的主题在列表（[`crate::theme::themes`]）里的位置。
    pub(crate) fn theme_slot(&self, dark: bool) -> usize {
        let id = if dark {
            &self.config.dark_theme
        } else {
            &self.config.light_theme
        };
        crate::theme::theme_index(dark, id.as_deref())
    }

    /// 深色（`dark`）或浅色外观时用的主题。
    pub(crate) fn theme_palette(&self, dark: bool) -> &'static Palette {
        crate::theme::themes(dark)[self.theme_slot(dark)].palette
    }

    /// 切到 `theme`：换成对应的主题，终端跟着换配色，并重新摆放标题栏按钮。
    pub(in crate::app) fn set_theme(&mut self, theme: Theme) {
        self.palette = self.theme_palette(theme == Theme::Dark);
        self.terminal_theme_changed();

        if let Some(renderer) = &mut self.renderer {
            renderer.set_background(self.palette.frame_bg);
        }

        self.place_traffic_lights();
        self.request_redraw();
    }

    /// 按配置重新加载字体并刷新度量。
    ///
    /// # Errors
    ///
    /// 渲染层无法使用配置的字体时，返回面向用户的错误说明。
    pub(in crate::app) fn apply_fonts(&mut self) -> Result<(), String> {
        let [mono, ui, terminal] = self.font_specs();
        let weight = self.font_weight();

        if let Some(renderer) = &mut self.renderer
            && renderer
                .set_fonts(mono.as_ref(), ui.as_ref(), terminal.as_ref(), weight)
                .is_err()
        {
            return Err(winc_i18n::t!("workspace-font-unusable").to_owned());
        }

        self.apply_font_sizes();
        Ok(())
    }
}

/// 把配置中的字体名或字体文件路径转成 `FontSpec`。
///
/// 未配置或只有空白时返回 `None`，使用渲染层的默认字体。
pub(in crate::app) fn configured_font(configured: &Option<String>) -> Option<FontSpec> {
    let name = configured
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;

    Some(match resolve_font(Some(name), "") {
        winc_config::FontRequest::Name(name) => FontSpec::Name(name),
        winc_config::FontRequest::Path(path) => FontSpec::Path(path),
    })
}

/// 返回启动时固定给窗口的主题。
///
/// 配置中的设置优先，其次读取环境变量 `WINC_THEME=dark|light`；都没有时返回 `None`，由系统
/// 决定（窗口跟随系统外观，切换时会收到 `ThemeChanged`）。
pub(in crate::app) fn forced_theme(choice: &ThemeChoice) -> Option<Theme> {
    match choice {
        ThemeChoice::Dark => Some(Theme::Dark),
        ThemeChoice::Light => Some(Theme::Light),
        ThemeChoice::Auto => match std::env::var("WINC_THEME").as_deref() {
            Ok("dark") => Some(Theme::Dark),
            Ok("light") => Some(Theme::Light),
            _ => None,
        },
    }
}
