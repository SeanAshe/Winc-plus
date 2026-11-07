//! 设置浮层的状态和操作，改动立刻生效并写回配置文件。
//!
//! [`SettingsState`] 保存选中的分类、页内滚动位置和正在编辑的控件；改动先更新运行时状态，
//! 再写回 [`winc_config::Config`]。关闭浮层时先离开输入框。

mod formatters;

use std::ops::RangeInclusive;

use winc_editor::format::{
    Auto, FormatError, PRESETS, Plan, Program, Task, knows_how_to_install, presets,
};
use winc_i18n::t;
use winc_syntax::Language;

use super::editor::format::{task_commands, task_commands_mut};
use super::input::context_menu::{MenuTarget, menu_below, tool_setting as tool_setting_of};
use super::*;
use crate::keymap::{self, Os, Stroke};
use crate::layout::{KeysLayout, ToolLanguages};

/// 设置浮层的状态。
#[derive(Debug, Default)]
pub struct SettingsState {
    /// 浮层是否打开。
    ///
    /// 打开时覆盖整个窗口，键盘和鼠标都先交给浮层。
    pub open: bool,
    /// 当前选中的设置项，由键盘上下键或鼠标点击选中；右侧显示的是该设置项所在的那一页。
    pub selected: Setting,
    /// 是否用键盘移动过选中项。
    ///
    /// 只有此时才给选中的那一行铺底色，使用鼠标时不显示。
    pub keyboard: bool,
    /// 正在编辑字体名或格式化命令时，对应的设置项和输入框的当前内容。
    pub editing: Option<(Setting, TextField)>,
    /// 开始编辑前的字体设置；按 `Esc` 或关闭浮层时退回该值。
    pub editing_original: Option<String>,
    /// 字体名无法识别等错误：出错的设置项和原因，原因用错误色写在该行输入框的左侧。
    ///
    /// 离开输入框后，输入退回原值，原因仍留在该行，换页、再次编辑或关闭浮层时清除。
    pub error: Option<(Setting, String)>,
    /// 当前页内容的纵向滚动位置（逻辑像素），换页时回到顶部。
    pub scroll: f32,
    /// 鼠标悬停的部件。
    pub hover: Option<SettingsPart>,
    /// 格式化页列出的语言。
    ///
    /// 打开设置时确定，打开期间只会增加，不会减少，已列出的语言也不换位置。
    pub languages: ToolLanguages,
    /// 格式化页每一行（格式化工具、修复工具）上显示的内容。
    pub tool_rows: Vec<(Setting, FormatRow)>,
    /// 当前帧绘制出的「安装方法…」链接的位置，以及链接所属的设置项。
    ///
    /// 文字宽度要到绘制时才知道，悬停和点击使用这里记录的位置。
    pub install_links: Vec<(Setting, Rect)>,
    /// 快捷键页的搜索框：按命令名或按键搜索。
    pub keys_query: TextField,
    /// 是否正在录制按键：按下的键写入搜索框，不执行。
    pub recording: bool,
    /// 搜索框中的内容是否为录制的按键（按键搜索，而不是按名称搜索）。
    pub keys_recorded: bool,
    /// 浮层正在出现（`true`）或消失（`false`）的动画：出现时遮罩淡入，卡片整张淡入、从 95%
    /// 放大到原大；消失时整张淡出、略微缩小。
    ///
    /// 消失期间 `open` 已经是 `false`，键盘和鼠标已经交回主界面，此时仍需绘制最后一帧。
    pub motion: Option<(bool, super::animation::Clock)>,
    /// 「模式」换了以后，凸起的那一段从原来那一段（`ThemeChoice`）滑过去。
    pub mode_slide: Option<(ThemeChoice, super::animation::Clock)>,
    /// 刚切换的开关（`Setting`）：圆钮滑到另一侧，底色渐变到新状态的颜色。
    pub switch_slide: Option<(Setting, super::animation::Clock)>,
    /// 出现、消失动画用的快照，动画开头画一次存下来，动画结束就释放。
    pub cache: Option<DialogCache>,
    /// 设置页滚动期间保存的窗口画面（含遮罩），用于代替逐帧重绘浮层下方的界面。
    pub scroll_base: Option<winc_render::Snapshot>,
    /// 最近一次滚动的时刻；未在滚动时为 `None`。
    pub scrolled_at: Option<Instant>,
}

/// 停止滚动后保留快照的时长（毫秒）。
///
/// 取值约为 60 Hz 下 9 帧的间隔，滚动事件短暂间断时不必重新保存快照。这段时间内浮层下方的变化在
/// 释放快照后统一绘制。
const SCROLL_BASE_KEEP: Duration = Duration::from_millis(150);

/// 设置浮层出现、消失的动画用的两张快照：动画开头画一次，途中每帧直接拿来用。
#[derive(Debug)]
pub struct DialogCache {
    /// 浮层底下的整个窗口（主界面，不含浮层）。
    pub base: winc_render::Snapshot,
    /// 画好的卡片：原大、不透明，不含阴影。
    pub card: winc_render::Snapshot,
}

/// 格式化页中一种语言的一行（格式化工具或修复工具）：右侧显示工具，名称下方显示来源和提醒。
#[derive(Debug, Clone, PartialEq)]
pub struct FormatRow {
    /// 按钮上显示的文字：工具名（例如「Ruff」）；自己写的命令为「自定义」；
    /// 没有可用工具时为「未设置」。
    pub tool: String,
    /// 是否没有可用的工具（按钮上写「未设置」）：此时按钮文字画成淡色。
    pub unset: bool,
    /// 名称下方那一行：工具的确定方式（例如「自动 · 根据 pyproject.toml」），
    /// 指定了命令时为命令本身。
    pub detail: String,
    /// 自动选择会选中的工具（例如「Ruff（pyproject.toml）」），显示在菜单中「自动」之下；
    /// 选不出来时为 `None`。
    pub auto: Option<String>,
    /// 项目设置（`.winc/config.json`）中写的命令。
    ///
    /// 存在时，该项目内使用此命令，不看用户设置。
    pub project: Option<String>,
    /// 「未安装」等提醒，接在 `detail` 之后。
    pub warning: Option<String>,
    /// 没装但知道安装方法的工具，菜单中有「安装方法…」。
    pub install: Vec<String>,
    /// 能否从列表中移除（菜单中有「移除」）：只有格式化工具一行，且这种语言是用户自己加的
    /// （或写过命令），不看用户设置时不会列出。
    pub removable: bool,
}

impl ToolLanguages {
    /// 返回 `task`（格式化或修复）那一组列出的语言。
    fn of(&mut self, task: Task) -> &mut Vec<&'static str> {
        match task {
            Task::Format => &mut self.format,
            Task::Fix => &mut self.fix,
        }
    }
}

impl SettingsState {
    /// 返回设置项 `setting`（`FormatCommand` 或 `FixCommand`）那一行上显示的内容。
    pub fn tool_row(&self, setting: Setting) -> Option<&FormatRow> {
        self.tool_rows
            .iter()
            .find(|(s, _)| *s == setting)
            .map(|(_, r)| r)
    }
}

/// 返回工具设置项对应的（任务种类，语言 ID）；不是工具设置项时返回 `None`。
fn tool_setting(setting: Setting) -> Option<(Task, &'static str)> {
    match setting {
        Setting::FormatCommand(id) => Some((Task::Format, id)),
        Setting::FixCommand(id) => Some((Task::Fix, id)),
        _ => None,
    }
}

/// 返回字体设置值 `value` 不能用的原因；未设置或能用时返回 `None`。
fn font_error(value: &Option<String>) -> Option<String> {
    let spec = configured_font(value)?;
    let error = spec.check().err()?;
    let name = value.as_deref().unwrap_or_default().trim();
    Some(match (error, spec) {
        (FontError::NotFound, FontSpec::Name(_)) => t!("settings-font-not-installed", name = name),
        (FontError::NotFound, FontSpec::Path(_)) => t!("settings-font-file-not-found", name = name),
        (FontError::Unsupported, _) => t!("settings-font-unsupported", name = name),
    })
}

/// 把 `value` 加 1（`up` 为 `true`）或减 1，结果限制在 `range` 之内。
fn step(value: f32, up: bool, range: RangeInclusive<f32>) -> f32 {
    let next = if up { value + 1.0 } else { value - 1.0 };
    next.clamp(*range.start(), *range.end())
}

impl App {
    /// 打开设置浮层；已经打开时不处理。
    ///
    /// 每次打开都从第一页的顶部开始。
    pub(crate) fn open_settings(&mut self) {
        if self.settings.open {
            return;
        }

        // 先关闭其他浮层和菜单：设置浮层位于最上层，不与它们同时显示。
        if self.picker_open {
            self.close_picker();
        }
        self.close_form_dialog();
        self.finish_name_input();

        self.settings.open = true;
        self.settings.motion = self
            .clock(super::animation::DIALOG_IN)
            .map(|clock| (true, clock));
        self.settings.cache = None;
        self.settings.scroll = 0.0;
        self.settings.selected = Setting::default();
        self.settings.error = self.broken_font(self.settings.selected.page());
        self.settings.keyboard = false;
        self.settings.hover = None;
        self.settings.languages = ToolLanguages::default();
        self.refresh_format_rows(true);
        // 列出字体要读取字体文件，先在后台读好，打开字体菜单时不必等待。
        std::thread::spawn(winc_render::installed_fonts);
        // 能否找到工具取决于用户 shell 中的 `PATH`，因此在后台获取该环境，取到之后再补上提醒。
        if let Some(proxy) = self.proxy.clone() {
            self.format.formatter.prepare(move || {
                let _ = proxy.send_event(UserEvent::FormatterReady);
            });
        }
        self.focus = Focus::Settings;
        self.set_hover(Hover::None);
        self.request_redraw();
    }

    /// 关闭设置浮层。
    ///
    /// 正在编辑的字体名按 `Esc` 处理：没有按回车确认就不生效。
    pub(super) fn close_settings(&mut self) {
        if !self.settings.open {
            return;
        }

        self.settings_cancel_field();
        self.settings.open = false;
        self.settings.motion = self
            .clock(super::animation::DIALOG_OUT)
            .map(|clock| (false, clock));
        self.settings.cache = None;
        self.settings.scroll_base = None;
        self.settings.scrolled_at = None;
        self.settings.mode_slide = None;
        self.settings.switch_slide = None;
        self.settings.error = None;
        self.settings.hover = None;
        self.field_preedit.clear();

        self.settings.recording = false;
        if matches!(
            self.focus,
            Focus::Settings | Focus::Setting | Focus::KeySearch
        ) {
            self.focus = Focus::Editor;
        }

        self.request_redraw();
    }

    /// 返回设置浮层中窗口坐标 (`x`, `y`) 处的部件。
    ///
    /// 先判断上一帧绘制出的「安装方法…」链接，再判断布局中的部件。滚动到标题之下而不可见的
    /// 链接不可点击。
    pub(super) fn settings_part_at(&self, x: f32, y: f32) -> Option<SettingsPart> {
        let layout = self.settings_layout();
        if layout.page == SettingsPage::Keys {
            let (search, record) = KeysLayout::header(layout.body);
            if record.contains(x, y) {
                return Some(SettingsPart::Record);
            }
            if search.contains(x, y) {
                return Some(SettingsPart::KeySearch);
            }
        }
        let link = self
            .settings
            .install_links
            .iter()
            .find(|(_, r)| r.contains(x, y) && layout.body.contains(x, y));
        match link {
            Some(&(setting, _)) => Some(SettingsPart::Install(setting)),
            None => layout.part_at(x, y),
        }
    }

    /// 返回设置浮层当前帧的几何布局，绘制、悬停和点击共用同一份。
    pub(crate) fn settings_layout(&self) -> SettingsLayout {
        let page = self.settings.selected.page();
        let languages = &self.settings.languages;
        let editing = self.settings.editing.as_ref().map(|(s, _)| *s);
        let scroll = self.settings.scroll;
        SettingsLayout::compute(self.layout.window, page, languages, editing, scroll)
    }

    /// 切换到 `page`，并选中该页的第一项。
    ///
    /// 正在编辑时先离开输入框（见 [`App::settings_leave_field`]）。进入快捷键页时，光标直接
    /// 位于搜索框中。
    pub(super) fn settings_show_page(&mut self, page: SettingsPage) {
        self.settings_leave_field();
        if self.settings.selected.page() != page {
            self.settings.selected = page.first();
            self.settings.scroll = 0.0;
            self.settings.error = self.broken_font(page);
        }
        self.settings.recording = false;
        self.focus = if page == SettingsPage::Keys {
            Focus::KeySearch
        } else {
            Focus::Settings
        };
        self.reset_blink();
        self.request_redraw();
    }

    /// 返回快捷键页当前帧的几何布局，内容按搜索框中的文字筛选。
    pub(crate) fn keys_layout(&self) -> KeysLayout {
        let query = self.settings.keys_query.text();
        let recorded = self.settings.keys_recorded;
        let groups = keymap::current_rows()
            .iter()
            .map(|(group, rows)| {
                let rows = rows
                    .iter()
                    .filter(|row| keymap::row_matches(row, query, Os::CURRENT, recorded))
                    .collect();
                (*group, rows)
            })
            .collect();
        KeysLayout::compute(self.settings_layout().body, groups, self.settings.scroll)
    }

    /// 录制按键时处理一次按键 `stroke`：写入搜索框。
    ///
    /// 最多记录两个按键（两步组合键），第三个按键重新开始记录。
    pub(super) fn record_stroke(&mut self, stroke: Stroke) {
        let label = stroke.label(Os::CURRENT);
        let field = &mut self.settings.keys_query;
        let text = if self.settings.keys_recorded && field.text().split(' ').count() == 1 {
            format!("{} {label}", field.text())
        } else {
            label
        };
        field.set_text(&text);
        field.set_caret(text.len(), false);
        self.settings.keys_recorded = true;
        self.settings.scroll = 0.0;
        self.request_redraw();
    }

    /// 开始或停止录制按键；开始时清空搜索框。
    pub(super) fn toggle_recording(&mut self) {
        self.settings.recording = !self.settings.recording;
        if self.settings.recording {
            self.settings.keys_query.set_text("");
            self.settings.keys_recorded = false;
            self.settings.scroll = 0.0;
        }
        self.focus = Focus::KeySearch;
        self.reset_blink();
        self.request_redraw();
    }

    /// 上下移动选中项，`down` 为 `true` 时向下；到达首尾后停止。
    ///
    /// 正在编辑时先离开输入框（见 [`App::settings_leave_field`]）。
    pub(super) fn settings_move(&mut self, down: bool) {
        self.settings_leave_field();

        let all = Setting::all(&self.settings.languages);
        let position = all
            .iter()
            .position(|s| *s == self.settings.selected)
            .unwrap_or(0);
        let next = if down {
            (position + 1).min(all.len() - 1)
        } else {
            position.saturating_sub(1)
        };

        self.settings.keyboard = true;
        if all[next].page() != self.settings.selected.page() {
            self.settings.scroll = 0.0;
            self.settings.error = self.broken_font(all[next].page());
        }
        self.settings.selected = all[next];
        self.settings.recording = false;
        self.focus = if all[next] == Setting::KeySearch {
            Focus::KeySearch
        } else {
            Focus::Settings
        };
        self.settings_scroll_into_view();
        self.request_redraw();
    }

    /// 滚动设置页，使选中的设置项进入可见范围。
    fn settings_scroll_into_view(&mut self) {
        let layout = self.settings_layout();
        let Some(row) = layout.row(self.settings.selected) else {
            return;
        };

        // 行的位置已减去滚动量，先换算回内容坐标再比较；上方多露出 `SETTINGS_GROUP_GAP`，
        // 使分组的上沿可见。
        let top = row.row.y - layout.body.y + self.settings.scroll - SETTINGS_GROUP_GAP;
        let bottom = row.row.bottom() - layout.body.y + self.settings.scroll;

        let scroll = scroll_to_show(self.settings.scroll, top, bottom, layout.body.h);
        self.settings.scroll = scroll.clamp(0.0, layout.max_scroll());
    }

    /// 按滚轮增量 `dy`（逻辑像素）滚动当前页，滚动位置限制在有效范围内。
    pub(super) fn settings_scroll_by(&mut self, dy: f32) {
        let max = if self.settings.selected.page() == SettingsPage::Keys {
            self.keys_layout().max_scroll()
        } else {
            self.settings_layout().max_scroll()
        };
        self.settings.scroll = (self.settings.scroll - dy).clamp(0.0, max);
        self.settings.scrolled_at = Some(Instant::now());
        self.request_redraw();
    }

    /// 当前帧是否可以使用保存的快照：设置页正在滚动，且快照与窗口的尺寸、缩放比例相同。
    pub(crate) fn settings_reuses_base(&self, bounds: Rect, scale: f32) -> bool {
        self.settings.open
            && self.settings.motion.is_none()
            && self.settings.scrolled_at.is_some()
            && (self.settings.scroll_base.as_ref()).is_some_and(|b| b.covers(bounds, scale))
    }

    /// 滚动结束后释放快照并请求重绘，显示滚动期间浮层下方的变化；返回下一次唤醒的时刻。
    pub(crate) fn settings_scroll_tick(&mut self, now: Instant) -> Option<Instant> {
        let due = self.settings.scrolled_at? + SCROLL_BASE_KEEP;
        if now < due {
            return Some(due);
        }
        self.settings.scrolled_at = None;
        self.settings.scroll_base = None;
        self.request_redraw();
        None
    }

    /// 处理 `←` `→`（`forward` 为 `true` 时为 `→`）：数值减一或加一，主题在缩略图之间移动一张。
    pub(super) fn settings_step(&mut self, forward: bool) {
        let setting = self.settings.selected;
        self.settings.keyboard = true;

        match setting.control() {
            SettingControl::Stepper => self.settings_adjust(setting, forward),
            SettingControl::ThemeList => {
                let dark = setting == Setting::DarkTheme;
                let current = self.theme_slot(dark);
                // 到两端后停止，不绕回。
                let next = if forward {
                    (current + 1).min(crate::theme::themes(dark).len() - 1)
                } else {
                    current.saturating_sub(1)
                };
                self.set_named_theme(dark, next);
            }
            SettingControl::Themes => {
                let current = THEME_CHOICES
                    .iter()
                    .position(|(c, _)| *c == self.theme_choice)
                    .unwrap_or(0);
                // 到两端后停止，不绕回。
                let next = if forward {
                    (current + 1).min(THEME_CHOICES.len() - 1)
                } else {
                    current.saturating_sub(1)
                };
                self.set_theme_choice(THEME_CHOICES[next].0);
            }
            SettingControl::Switch => self.settings_set_switch(setting, forward),
            SettingControl::Font if forward => self.settings_open_menu(setting),
            SettingControl::Font
            | SettingControl::Command
            | SettingControl::Action
            | SettingControl::Choice
            | SettingControl::Search
            | SettingControl::Value
            | SettingControl::Button => {}
        }
    }

    /// 处理「关于」页「检查更新」项的点击：macOS 上由 Sparkle 弹出更新窗口，其他平台在后台
    /// 检查更新，结果显示在该页和状态栏上。
    ///
    /// 当前版本不检查更新，或正在检查、下载时忽略本次点击。
    pub(in crate::app) fn settings_check_updates(&mut self) {
        self.settings.selected = Setting::Updates;
        if self.can_check_updates() {
            #[cfg(target_os = "macos")]
            crate::macos::check_for_updates(true);
            #[cfg(not(target_os = "macos"))]
            crate::updates::check();
        }
    }

    /// 是否可以检查更新：当前版本启用了更新检查，且没有正在检查、下载或已下载完成的更新。
    pub(crate) fn can_check_updates(&self) -> bool {
        use crate::winc::UpdateState;
        !matches!(
            *self.shared.update_state.borrow(),
            UpdateState::Off
                | UpdateState::Checking
                | UpdateState::Downloading(_)
                | UpdateState::Ready(_)
        )
    }

    /// 将值 `setting`（版本号）复制到剪贴板，并在状态栏提示。
    pub(in crate::app) fn settings_copy_value(&mut self, setting: Setting) {
        self.settings.selected = setting;
        let version = crate::layout::version_label();
        self.set_clipboard(version.clone(), false);
        self.flash(t!("settings-version-copied", version = version));
    }

    /// 处理 `↩` 或空格：输入框进入编辑，开关切换，格式化命令、「添加语言…」「排除的语言」弹出菜单，
    /// 数值项和主题项向前一步（等同 `→`），版本号复制到剪贴板，搜索项把焦点移到搜索框。
    pub(super) fn settings_activate(&mut self) {
        self.settings.keyboard = true;
        let setting = self.settings.selected;

        match setting.control() {
            SettingControl::Font => self.settings_start_editing(setting),
            SettingControl::Switch => self.settings_toggle(setting),
            SettingControl::Command
            | SettingControl::Action
            | SettingControl::Choice
            | SettingControl::Button => self.settings_open_menu(setting),
            SettingControl::Value => self.settings_copy_value(setting),
            SettingControl::Stepper | SettingControl::Themes | SettingControl::ThemeList => {
                self.settings_step(true)
            }
            SettingControl::Search => self.focus = Focus::KeySearch,
        }
    }

    /// 开关 `setting` 当前是否开启。
    pub(crate) fn switch_on(&self, setting: Setting) -> bool {
        match setting {
            Setting::FormatOnSave => self.config.format_on_save == Some(true),
            Setting::FixOnSave => self.config.fix_on_save == Some(true),
            Setting::AutoSave => self.config.auto_save == Some(true),
            Setting::HotExit => self.config.hot_exit != Some(false),
            Setting::RestoreFolder => self.config.restore_folder != Some(false),
            Setting::StickyScroll => self.config.sticky_scroll != Some(false),
            Setting::WordWrap => self.config.word_wrap == Some(true),
            Setting::Minimap => self.config.minimap == Some(true),
            Setting::LineNumbers => self.config.line_numbers != Some(false),
            Setting::DiffInline => self.config.diff_inline == Some(true),
            Setting::StatusBar => self.config.status_bar != Some(false),
            Setting::RestoreWindow => self.config.restore_window != Some(false),
            Setting::CompactFolders => self.config.compact_folders != Some(false),
            Setting::RemoteImages => self.config.markdown_remote_images != Some(false),
            Setting::CheckUpdates => self.config.update_auto_check != Some(false),
            Setting::SystemTitleBar => self.config.ui_native_title_bar == Some(true),
            Setting::GitModule => self.git_enabled(),
            Setting::TerminalModule => self.terminal_enabled(),
            _ => false,
        }
    }

    /// 切换开关 `setting` 的状态。
    pub(super) fn settings_toggle(&mut self, setting: Setting) {
        if setting == Setting::StickyScroll {
            self.sticky_cache = None;
        }
        self.settings_set_switch(setting, !self.switch_on(setting));
    }

    fn settings_set_switch(&mut self, setting: Setting, on: bool) {
        if self.switch_on(setting) == on {
            return;
        }
        match setting {
            Setting::FormatOnSave => self.config.format_on_save = Some(on),
            Setting::FixOnSave => self.config.fix_on_save = Some(on),
            Setting::AutoSave => self.config.auto_save = Some(on),
            Setting::HotExit => self.config.hot_exit = Some(on),
            Setting::RestoreFolder => self.config.restore_folder = Some(on),
            Setting::StickyScroll => self.config.sticky_scroll = Some(on),
            Setting::WordWrap => self.config.word_wrap = Some(on),
            Setting::Minimap => self.config.minimap = Some(on),
            Setting::LineNumbers => self.config.line_numbers = Some(on),
            Setting::DiffInline => self.config.diff_inline = Some(on),
            Setting::StatusBar => self.config.status_bar = Some(on),
            Setting::RestoreWindow => self.config.restore_window = Some(on),
            Setting::CompactFolders => self.config.compact_folders = Some(on),
            Setting::RemoteImages => self.config.markdown_remote_images = Some(on),
            Setting::CheckUpdates => self.config.update_auto_check = Some(on),
            Setting::SystemTitleBar => self.config.ui_native_title_bar = Some(on),
            Setting::GitModule => self.config.git_enabled = Some(on),
            Setting::TerminalModule => self.config.terminal_enabled = Some(on),
            _ => return,
        }
        // 由命令切换（如 ⌥Z 自动换行）时设置浮层未打开，看不到开关，不必播放动画。
        self.settings.switch_slide = self
            .clock(super::animation::SWITCH_SLIDE)
            .filter(|_| self.settings.open)
            .map(|clock| (setting, clock));
        if setting == Setting::WordWrap {
            self.word_wrap_changed();
        }
        if matches!(
            setting,
            Setting::Minimap | Setting::LineNumbers | Setting::StatusBar
        ) {
            self.relayout();
        }
        if setting == Setting::DiffInline {
            self.apply_diff_layout();
        }
        // 关掉以后不再沿用：之后再打开时从下一次关闭窗口开始记录。
        if setting == Setting::RestoreWindow && !on {
            self.update_state(|state| state.window = None);
        }
        if setting == Setting::CompactFolders {
            self.apply_compact_folders();
        }
        if setting == Setting::RemoteImages {
            self.markdown.forget_images();
        }
        if setting == Setting::GitModule {
            self.git_folder_changed();
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

    /// 开始编辑字体名或格式化命令：输入框中是当前的设置并全选，直接键入即替换。
    pub(super) fn settings_start_editing(&mut self, setting: Setting) {
        if matches!(&self.settings.editing, Some((s, _)) if *s == setting) {
            self.focus = Focus::Setting;
            return;
        }

        self.settings_leave_field();

        let current = match tool_setting(setting) {
            Some((task, id)) => task_commands(&self.config, task).get(id).cloned(),
            None => self.font_setting(setting).clone(),
        };
        let mut field = TextField::default();
        field.set_text(current.as_deref().unwrap_or(""));
        field.select_all();

        self.settings.selected = setting;
        self.settings.editing = Some((setting, field));
        self.settings.editing_original = current;
        self.settings.error = None;
        self.focus = Focus::Setting;
        self.reset_blink();
        self.request_redraw();
    }

    /// 返回 `page` 上配置了但用不了的字体，以及原因；没有时返回 `None`。
    ///
    /// 这样的字体在启动时已改用默认字体，但设置中仍是原来的名字，因此在该行写明原因。
    fn broken_font(&self, page: SettingsPage) -> Option<(Setting, String)> {
        [Setting::EditorFont, Setting::UiFont]
            .into_iter()
            .filter(|setting| setting.page() == page)
            .find_map(|setting| {
                let value = match setting {
                    Setting::UiFont => &self.config.ui_font,
                    _ => &self.config.editor_font,
                };
                font_error(value).map(|error| (setting, error))
            })
    }

    fn font_setting(&mut self, setting: Setting) -> &mut Option<String> {
        match setting {
            Setting::UiFont => &mut self.config.ui_font,
            _ => &mut self.config.editor_font,
        }
    }

    /// 离开正在编辑的输入框：先提交；字体名不对时放弃本次输入，字体设置退回开始编辑前的值，
    /// 出错原因仍显示在该行。
    ///
    /// 换页、点击其他控件、按 `Tab` 时调用，不会因为名字不对而把用户困在输入框中；只有按 `↩`
    /// 提交时，名字不对才停留在输入框里。
    pub(super) fn settings_leave_field(&mut self) {
        if self.settings.editing.is_none() {
            return;
        }
        self.settings_commit_field();
        let error = self.settings.error.take();
        self.settings_cancel_field();
        self.settings.error = error;
    }

    /// 键入时只更新配置中的值，不改变正在使用的字体。
    ///
    /// 键入到一半的名字（例如 `Menl`）本来就找不到，每敲一个字符就换一次字体会使界面反复变化。
    /// 回车提交时才真正换字体。
    pub(super) fn settings_live_font(&mut self) {
        let Some((setting, field)) = self.settings.editing.as_ref() else {
            return;
        };

        let setting = *setting;
        if setting.control() != SettingControl::Font {
            return;
        }
        let text = field.text().trim().to_owned();
        *self.font_setting(setting) = (!text.is_empty()).then_some(text);
        self.settings.error = None;
    }

    /// 放弃编辑：字体设置退回开始编辑前的值（键入过程中配置已被改为中间状态）。
    ///
    /// 格式化命令在键入时不改配置，因此不需要退回。
    pub(super) fn settings_cancel_field(&mut self) {
        let Some((setting, _)) = self.settings.editing.take() else {
            return;
        };

        let original = self.settings.editing_original.take();
        if setting.control() == SettingControl::Font {
            *self.font_setting(setting) = original;
        }
        self.settings.error = None;

        if self.focus == Focus::Setting {
            self.focus = Focus::Settings;
        }

        self.request_redraw();
    }

    /// 提交正在编辑的字体名或格式化命令。
    pub(super) fn settings_commit_field(&mut self) {
        let Some((setting, field)) = self.settings.editing.as_ref() else {
            return;
        };
        let (setting, text) = (*setting, field.text().trim().to_owned());
        let value = (!text.is_empty()).then_some(text);

        match tool_setting(setting) {
            Some((task, id)) => self.settings_commit_command(task, id, value),
            None => self.settings_commit_font(setting, value),
        }
    }

    /// 提交格式化（修复）命令并写回配置；`command` 为 `None`（清空）表示自动选择。
    ///
    /// 命令对应的程序找不到时同样保存（工具可能只装在其他项目里，或稍后才安装），
    /// 提醒显示在名字下方。
    fn settings_commit_command(&mut self, task: Task, id: &'static str, command: Option<String>) {
        let commands = task_commands_mut(&mut self.config, task);
        match command {
            Some(command) => commands.insert(id.to_owned(), command),
            None => commands.remove(id),
        };
        self.settings.editing = None;
        self.settings.editing_original = None;
        self.settings.error = None;
        self.focus = Focus::Settings;
        self.save_config();
        self.refresh_format_rows(false);
        self.request_redraw();
    }

    /// 提交字体名：立刻换字体并写回配置。
    ///
    /// 名字认不出来就留在输入框里，配置退回原值。
    ///
    /// 清空表示使用平台默认字体，这是允许的：配置中留空即默认，输入框中显示「系统默认」。
    fn settings_commit_font(&mut self, setting: Setting, value: Option<String>) {
        // 无法识别的字体名要在这里拦截：渲染层找不到字体时会静默回退到默认字体，
        // 用户会以为设置已生效，但画面没有变化。
        if let Some(message) = font_error(&value) {
            self.settings.error = Some((setting, message));
            self.request_redraw();
            return;
        }

        let original = self.settings.editing_original.clone();
        *self.font_setting(setting) = value;

        match self.apply_fonts() {
            Ok(()) => {
                self.settings.editing = None;
                self.settings.editing_original = None;
                self.settings.error = None;
                self.focus = Focus::Settings;
                self.save_config();
            }
            Err(message) => {
                // 无法应用时退回原值，配置中不保留无法使用的字体。
                *self.font_setting(setting) = original;
                self.settings.error = Some((setting, message));
            }
        }

        self.request_redraw();
    }

    /// 应用字体菜单中选中的字体；`None` 表示系统默认。
    ///
    /// 字体不能使用时配置保持原值，原因显示在该行。
    pub(in crate::app) fn settings_choose_font(&mut self, setting: Setting, value: Option<String>) {
        self.settings.editing_original = self.font_setting(setting).clone();
        self.settings_commit_font(setting, value);
        self.settings.editing_original = None;
    }

    /// 放大或缩小编辑区的字号（`⌘+` `⌘−`），`⌘0` 还原（`up` 为 `None`）。
    ///
    /// 改的就是设置中的「编辑器字号」，并写回配置；界面字号不变。
    pub(super) fn zoom(&mut self, up: Option<bool>) {
        let current = self.config.editor_size.unwrap_or(CODE_FONT);
        self.config.editor_size = up.map(|up| step(current, up, Config::EDITOR_SIZE_RANGE));

        let code = self.config.editor_size.unwrap_or(CODE_FONT);
        if code == current && up.is_some() {
            self.flash(t!("settings-zoom-limit", size = code.to_string()));
            return;
        }

        let ui = self.config.ui_size.unwrap_or(UI_FONT);
        self.apply_font_sizes(code, ui);
        self.save_config();
        self.flash(t!("settings-zoom", size = code.to_string()));
    }

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
    pub(super) fn settings_adjust(&mut self, setting: Setting, up: bool) {
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

    /// 选中深色（`dark`）或浅色主题列表里的第 `index` 个并记住。
    ///
    /// 外观固定为另一种时切过去，选中后立刻生效；跟随系统时只换该外观的主题，当前不是那种外观时
    /// 在状态栏说明它何时生效。
    ///
    /// # Panics
    ///
    /// `index` 超出该外观主题列表的范围时 panic。
    pub(super) fn set_named_theme(&mut self, dark: bool, index: usize) {
        let id = crate::theme::themes(dark)[index].id.to_owned();
        if dark {
            self.config.dark_theme = Some(id);
        } else {
            self.config.light_theme = Some(id);
        }
        let choice = match self.theme_choice {
            ThemeChoice::Auto => ThemeChoice::Auto,
            _ if dark => ThemeChoice::Dark,
            _ => ThemeChoice::Light,
        };
        if choice != self.theme_choice {
            self.config.theme = Some(choice);
        }
        self.apply_theme(choice);
        if self.palette.dark != dark {
            self.flash(if dark {
                t!("settings-theme-used-when-dark")
            } else {
                t!("settings-theme-used-when-light")
            });
        }
        self.save_config();
        self.request_redraw();
    }

    /// 切换主题并记住选择。
    pub(super) fn set_theme_choice(&mut self, choice: ThemeChoice) {
        if choice != self.theme_choice {
            self.settings.mode_slide = self
                .clock(super::animation::VIEW_SLIDE)
                .map(|clock| (self.theme_choice, clock));
        }
        self.config.theme = Some(choice);
        self.apply_theme(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 切换界面语言并记住选择。
    ///
    /// 界面语言对整个进程生效，各窗口在下一帧使用新语言的文案；菜单栏在下次启动时更新。
    pub(in crate::app) fn set_locale_choice(&mut self, choice: winc_i18n::LocaleChoice) {
        self.config.language = Some(choice);
        if crate::set_locale(choice.resolve()) {
            // 汉字改按新的语言选字形，字体要重新加载；新字体总能加载，失败时留着原来的字体。
            let _ = self.apply_fonts();
        }
        self.save_config();
        self.request_redraw();
    }

    /// 设置何时显示空格和制表符。
    pub(in crate::app) fn set_render_whitespace(&mut self, choice: winc_config::RenderWhitespace) {
        self.config.render_whitespace = Some(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 把配置中修改过的项写回配置文件，再改用文件中的配置，使程序运行期间手动修改的项生效。
    ///
    /// 写入失败时提示原因，不影响继续使用，下次写回时再写入这些项。原文件无法解析或含有注释时，
    /// 先把原文件另存为一个副本再写入（写出的文件不保留注释），并提示副本的位置。版本号
    /// （`config_rev`）加一，应用层据此把新配置发给其他窗口。
    pub(super) fn save_config(&mut self) {
        self.config_rev += 1;
        let Some(path) = &self.config_path else {
            return;
        };

        match self.config.save_to(path, &self.saved_config) {
            Ok(saved) => {
                if let Some(backup) = saved.backup {
                    let path = backup.path.display().to_string();
                    self.notify(if backup.comments {
                        t!("settings-config-comments-backed-up", path = path)
                    } else {
                        t!("settings-config-backed-up", path = path)
                    });
                }
                self.saved_config = saved.config.clone();
                if saved.config != self.config {
                    self.adopt_config(saved.config);
                }
            }
            Err(e) => self.notify(t!("settings-config-write-failed", error = e.to_string())),
        }
    }
}

impl App {
    /// 改用配置文件中刚写好的配置 `config`，按变化的项更新本窗口。
    ///
    /// `config` 由另一个窗口写入，或者是本窗口写回时文件中带有手动修改的项。更新的内容包括字体和
    /// 字号、Tab 宽度、主题和各开关，效果与在本窗口的设置页中修改相同，只是不再写文件。
    pub(crate) fn adopt_config(&mut self, config: Config) {
        let old = std::mem::replace(&mut self.config, config);
        let new = &self.config;
        let fonts = (old.editor_font != new.editor_font || old.ui_font != new.ui_font)
            || (old.editor_size != new.editor_size || old.ui_size != new.ui_size);
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
        // 窗口字体同步。
        let glyph_language = old.language != new.language
            && crate::set_locale(new.language.unwrap_or_default().resolve());
        let git = old.git_enabled != new.git_enabled;
        let title_bar = old.ui_native_title_bar != new.ui_native_title_bar;
        let stop_terminal =
            old.terminal_enabled != new.terminal_enabled && new.terminal_enabled == Some(false);

        if fonts || glyph_language {
            // 别的窗口已经用上了这些字体，这里也用得上；万一用不上，留着原来的字体。
            let _ = self.apply_fonts();
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
            self.sticky_cache = None;
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
        if images {
            self.markdown.forget_images();
        }
        if git {
            self.git_folder_changed();
        }
        if stop_terminal {
            self.stop_terminal();
        }
        self.reload_run_configs();
        if title_bar {
            self.sync_title_bar();
        }
        self.relayout();
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
