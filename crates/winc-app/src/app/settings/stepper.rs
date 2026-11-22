//! 设置页中的数值设置：字号、字重、不透明度、制表符长度的显示值和逐档调整。

use std::ops::RangeInclusive;

use super::*;

/// 设置页中背景图片不透明度每次调整的量（百分比）。
const OPACITY_STEP: f32 = 5.0;

/// 设置页中字重每次调整的量。
const FONT_WEIGHT_STEP: u16 = 100;

/// 把 `value` 加 1（`up` 为 `true`）或减 1，结果限制在 `range` 之内。
pub(super) fn step(value: f32, up: bool, range: RangeInclusive<f32>) -> f32 {
    let next = if up { value + 1.0 } else { value - 1.0 };
    next.clamp(*range.start(), *range.end())
}

/// 把字重 `current` 按 [`FONT_WEIGHT_STEP`] 调粗（`up` 为 `true`）或调细一档，限制在 `range` 内。
///
/// 不在档位上的值移到该方向上最近的一档。
fn step_weight(current: u16, up: bool, range: RangeInclusive<f32>) -> u16 {
    let next = if up {
        (current / FONT_WEIGHT_STEP + 1) * FONT_WEIGHT_STEP
    } else {
        current.saturating_sub(1) / FONT_WEIGHT_STEP * FONT_WEIGHT_STEP
    };
    next.clamp(*range.start() as u16, *range.end() as u16)
}

impl App {
    /// 返回数值设置当前的显示值，以及是否已到范围的下限、上限（到限的一侧按钮变淡）。
    pub(crate) fn setting_value(&self, setting: Setting) -> (String, bool, bool) {
        let (value, range) = match setting {
            Setting::EditorSize => (
                self.config.editor_size.unwrap_or(CODE_FONT),
                Config::EDITOR_SIZE_RANGE,
            ),
            Setting::UiSize => (
                self.config.ui_size.unwrap_or(UI_FONT),
                Config::UI_SIZE_RANGE,
            ),
            Setting::FontWeight => (f32::from(self.font_weight()), Config::FONT_WEIGHT_RANGE),
            Setting::BackgroundOpacity => (
                (self.config.ui_background_opacity).unwrap_or(Config::BACKGROUND_OPACITY),
                Config::BACKGROUND_OPACITY_RANGE,
            ),
            Setting::TerminalSize => (
                ViewFonts::of(&self.config).terminal,
                Config::EDITOR_SIZE_RANGE,
            ),
            Setting::TerminalFontWeight => (
                f32::from(self.terminal_font_weight()),
                Config::FONT_WEIGHT_RANGE,
            ),
            Setting::MarkdownSize => (
                crate::app::preview::markdown::laid::body_size(self),
                Config::EDITOR_SIZE_RANGE,
            ),
            Setting::TabWidth => (
                self.config.tab_width.unwrap_or(self.editor.tab_width) as f32,
                Config::TAB_WIDTH_RANGE,
            ),
            _ => return (String::new(), true, true),
        };

        (
            format!("{value:.0}"),
            value <= *range.start(),
            value >= *range.end(),
        )
    }

    /// 对字号、Tab 宽度这类数值设置加一（`up` 为 `true`）或减一，并写回配置。
    pub(in crate::app) fn settings_adjust(&mut self, setting: Setting, up: bool) {
        match setting {
            Setting::EditorSize => {
                let current = self.config.editor_size.unwrap_or(CODE_FONT);
                self.config.editor_size = Some(step(current, up, Config::EDITOR_SIZE_RANGE));
                self.apply_fonts().ok();
            }
            Setting::UiSize => {
                let current = self.config.ui_size.unwrap_or(UI_FONT);
                self.config.ui_size = Some(step(current, up, Config::UI_SIZE_RANGE));
                self.apply_fonts().ok();
            }
            // 配置中的值不在档位上时，移到该方向上最近的一档。
            Setting::FontWeight => {
                let current = self.font_weight();
                let next = step_weight(current, up, Config::FONT_WEIGHT_RANGE);
                self.config.ui_font_weight = (next != Config::FONT_WEIGHT).then_some(next);
                self.apply_fonts().ok();
            }
            // 同终端字号：调整过就记下，不再随全局字重变化。
            Setting::TerminalFontWeight => {
                let current = self.terminal_font_weight();
                let next = step_weight(current, up, Config::FONT_WEIGHT_RANGE);
                self.config.terminal_font_weight = Some(next);
                self.apply_fonts().ok();
            }
            Setting::BackgroundOpacity => {
                let range = Config::BACKGROUND_OPACITY_RANGE;
                let current =
                    (self.config.ui_background_opacity).unwrap_or(Config::BACKGROUND_OPACITY);
                let next = current + if up { OPACITY_STEP } else { -OPACITY_STEP };
                self.config.ui_background_opacity = Some(next.clamp(*range.start(), *range.end()));
            }
            // 只改字号，无需重新加载字体。
            Setting::TerminalSize => {
                let current = ViewFonts::of(&self.config).terminal;
                self.config.terminal_size = Some(step(current, up, Config::EDITOR_SIZE_RANGE));
                self.apply_font_sizes();
            }
            Setting::MarkdownSize => {
                use crate::app::preview::markdown::laid::{body_size, scaled_body_size};
                // 调回随编辑区变化的字号时清除设置，之后继续随编辑区字号变化。
                let next = step(body_size(self), up, Config::EDITOR_SIZE_RANGE);
                self.config.markdown_size = (next != scaled_body_size(self)).then_some(next);
                self.request_redraw();
            }
            Setting::TabWidth => {
                let current = self.config.tab_width.unwrap_or(self.editor.tab_width);
                let next = step(current as f32, up, Config::TAB_WIDTH_RANGE) as usize;
                self.config.tab_width = Some(next);
                self.editor.tab_width = next;
                self.ensure_caret_visible();
            }
            _ => return,
        }

        self.save_config();
        self.request_redraw();
    }
}
