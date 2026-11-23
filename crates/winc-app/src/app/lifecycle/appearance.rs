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
        if let Some(renderer) = &mut self.frame.renderer {
            renderer.set_zoom(zoom);
        }
        #[cfg(target_os = "macos")]
        if let Some(lights) = &self.link.traffic_lights {
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
        let Some(renderer) = &self.frame.renderer else {
            return;
        };
        self.metrics.ui = renderer.metrics(FontKind::Ui, self.fonts.ui);
        self.metrics.mono = renderer.metrics(FontKind::Mono, self.fonts.code);
        self.metrics.cell = self.metrics.mono.cell;
        self.metrics.terminal = renderer.metrics(FontKind::Terminal, self.fonts.terminal);
        self.metrics.terminal_cell = self.metrics.terminal.cell;
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

    /// 返回配置中的编辑区字体、界面字体和终端字体的列表；未配置时为空。
    ///
    /// 未配置时渲染层使用内置的系统字体路径表。按名字查询 CoreText 一次约 4–7 ms，
    /// 路径表只需几次 `stat`，因此未配置时不调用 CoreText。终端字体与编辑区字体相同时同样为
    /// 空，不重复加载。
    pub(in crate::app) fn font_specs(&self) -> [Vec<FontSpec>; 3] {
        let terminal = configured_fonts(&self.config.terminal_font);
        let editor = configured_fonts(&self.config.editor_font);
        let terminal = if terminal == editor {
            Vec::new()
        } else {
            terminal
        };
        [editor, configured_fonts(&self.config.ui_font), terminal]
    }

    /// 返回界面和编辑区文字的字重（`ui.fontWeight`），未配置时为 400。
    pub(in crate::app) fn font_weight(&self) -> u16 {
        self.config.ui_font_weight.unwrap_or(Config::FONT_WEIGHT)
    }

    /// 返回终端文字的字重（`terminal.fontWeight`）；没有调整过时同全局字重。
    pub(in crate::app) fn terminal_font_weight(&self) -> u16 {
        self.config
            .terminal_font_weight
            .unwrap_or_else(|| self.font_weight())
    }

    /// 应用主题选择：`Auto` 交由系统决定，`Dark`、`Light` 固定为指定外观。
    pub(in crate::app) fn apply_theme(&mut self, choice: ThemeChoice) {
        self.theme_choice = choice;
        // 系统菜单、提示框和面板按窗口外观显示深浅。
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        if let Some(window) = &self.window.handle {
            window.set_theme(forced_theme(&choice));
        }

        let current = if self.palette.dark {
            Theme::Dark
        } else {
            Theme::Light
        };
        let theme = forced_theme(&choice).or_else(|| self.system_theme());
        self.set_theme(theme.unwrap_or(current));
    }

    /// 系统当前的外观，取不到时为深色；还没有窗口时返回 `None`。
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    pub(in crate::app) fn system_theme(&self) -> Option<Theme> {
        let window = self.window.handle.as_ref()?;
        Some(window.theme().unwrap_or(Theme::Dark))
    }

    /// 门户报告的系统外观（见 `crate::color_scheme`），取不到时为深色。
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(in crate::app) fn system_theme(&self) -> Option<Theme> {
        Some(crate::color_scheme::current().unwrap_or(Theme::Dark))
    }

    /// 系统的外观变成了 `theme`：没有固定主题且深浅不同时跟随。
    pub(crate) fn system_theme_changed(&mut self, theme: Theme) {
        if forced_theme(&self.theme_choice).is_none() && self.palette.dark != (theme == Theme::Dark)
        {
            self.set_theme(theme);
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
        // 系统标题栏的深浅。
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(window) = &self.window.handle {
            window.set_theme(Some(theme));
        }
        self.terminal_theme_changed();

        if let Some(renderer) = &mut self.frame.renderer {
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
        let terminal_weight = self.terminal_font_weight();

        let prose = configured_fonts(&self.config.markdown_font);
        if let Some(renderer) = &mut self.frame.renderer {
            if renderer
                .set_fonts(&mono, &ui, &terminal, weight, terminal_weight)
                .is_err()
            {
                return Err(winc_i18n::t!("workspace-font-unusable").to_owned());
            }
            renderer.set_prose_fonts(&prose);
        }
        // 预览的排版和测量结果按旧字体算出，全部重排。
        self.preview.markdown.layout = None;
        self.preview.markdown.memo = Default::default();

        self.apply_font_sizes();
        Ok(())
    }
}

/// 把配置中以逗号分隔的字体名或字体文件路径转成 `FontSpec` 的列表，按书写顺序。
///
/// 未配置或只有空白时返回空列表，使用渲染层的默认字体。
pub(in crate::app) fn configured_fonts(configured: &Option<String>) -> Vec<FontSpec> {
    let requests = resolve_font_list(configured.as_deref());
    let spec = |request| match request {
        winc_config::FontRequest::Name(name) => FontSpec::Name(name),
        winc_config::FontRequest::Path(path) => FontSpec::Path(path),
    };
    requests.into_iter().map(spec).collect()
}

/// 配置的字体列表中第一个不可用的字体，及其写法和原因；都可用或未配置时返回 `None`。
pub(in crate::app) fn unusable_font(
    configured: &Option<String>,
) -> Option<(FontSpec, String, FontError)> {
    configured_fonts(configured).into_iter().find_map(|spec| {
        let error = spec.check().err()?;
        let name = match &spec {
            FontSpec::Name(name) => name.clone(),
            FontSpec::Path(path) => path.display().to_string(),
        };
        Some((spec, name, error))
    })
}

/// 返回启动时固定给窗口的主题。
///
/// 配置中的设置优先，其次读取环境变量 `WINC_THEME=dark|light`；都没有时返回 `None`，由系统
/// 决定（界面跟随系统外观，见 [`App::system_theme_changed`]）。
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
