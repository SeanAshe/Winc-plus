//! 设置浮层的状态和操作，改动立刻生效并写回配置文件。
//!
//! [`SettingsState`] 保存选中的分类、页内滚动位置和正在编辑的控件；改动先更新运行时状态，
//! 再写回 [`winc_config::Config`]。关闭浮层时先离开输入框。

mod adopt;
mod card;
mod formatters;
pub(super) mod shortcuts;
mod stepper;
mod switches;
mod transfer;
mod values;

use stepper::step;
pub(in crate::app) use transfer::{EXPORTED_SETTINGS, SETTINGS_EXTENSIONS};

use winc_editor::format::{
    Auto, FormatError, PRESETS, Plan, Program, Task, knows_how_to_install, presets,
};
use winc_i18n::t;
use winc_syntax::Language;

use super::editor::format::{task_commands, task_commands_mut};
use super::input::context_menu::{MenuTarget, menu_below, tool_setting as tool_setting_of};
use super::*;
use crate::keymap::{self, Os, Stroke};
use crate::layout::{KeyRows, KeysLayout, Listed, SettingsView};
use crate::updates::UpdateClick;

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
    /// 「系统集成」一组的状态（Windows），打开设置和切换开关时从注册表读出。
    pub(crate) integration: crate::shell_integration::Integration,
    /// 取值与默认不同的设置项：名称后面有「恢复默认」。打开设置和配置变化时重新算。
    pub changed: Vec<Setting>,
    /// 随状态列出的行。
    ///
    /// 格式化页的语言在打开设置时确定，打开期间只会增加，不会减少，已列出的语言也不换位置；
    /// Vim 的冲突键随 Vim 模式和键位表的变化重新算。
    pub listed: Listed,
    /// 格式化页每一行（格式化工具、修复工具）上显示的内容。
    pub tool_rows: Vec<(Setting, FormatRow)>,
    /// 正在后台查询 WSL 发行版中装了哪些工具，查完后重新计算各行。
    pub probing: bool,
    /// 分类栏顶部的搜索框：按设置项的名称和说明搜索，不为空白时正文换成搜索结果。
    pub search: TextField,
    /// 搜索开始前选中的设置项，以及搜索时自动选中的第一条结果：清空搜索框时仍选中后者（没有另选
    /// 结果）则回到前者。进入结果或换页后为 `None`。
    pub before_search: Option<(Setting, Setting)>,
    /// 「关于」页顶部的产品图标：渲染时的边长（物理像素）和 RGBA 像素；关闭浮层时释放。
    ///
    /// 渲染失败时像素为空，不再重试，图标不显示。
    pub logo: Option<(u32, Vec<u8>)>,
    /// 这次显示「关于」页是否已经播放过图标的动画；离开该页或重新打开设置后再播放一次。
    pub logo_played: bool,
    /// 快捷键页的搜索框：按命令名或按键搜索。
    pub keys_query: TextField,
    /// 是否正在录制按键：按下的键写入搜索框，不执行。
    pub recording: bool,
    /// 搜索框中的内容是否为录制的按键（按键搜索，而不是按名称搜索）。
    pub keys_recorded: bool,
    /// 「快捷键」页中正在修改的一行。
    pub(crate) key_edit: Option<shortcuts::KeyEdit>,
    /// 「快捷键」页中一行展开、收起的过渡：同时收起的另一行和时钟；展开的是 `key_edit` 的一行。
    pub key_slide: Option<(Option<keymap::Action>, super::animation::Clock)>,
    /// 「导入…」菜单列出的来源，打开菜单时查找。
    pub(crate) import_sources: Vec<crate::keymap::import::Source>,
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
    /// 设置页滚动或拖动期间保存的窗口画面（含遮罩），用于代替逐帧重绘浮层下方的界面。
    pub scroll_base: Option<winc_render::Snapshot>,
    /// 最近一次滚动或拖动的时刻；两者都未进行时为 `None`。
    pub scrolled_at: Option<Instant>,
    /// 卡片相对窗口中央的偏移（逻辑像素），拖动标题行时改变；窗口存在期间保留，再次打开时
    /// 卡片仍在原处。
    pub offset: (f32, f32),
}

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

impl Listed {
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
    let (spec, name, error) = unusable_font(value)?;
    let name = name.as_str();
    Some(match (error, spec) {
        (FontError::NotFound, FontSpec::Name(_)) => t!("settings-font-not-installed", name = name),
        (FontError::NotFound, FontSpec::Path(_)) => t!("settings-font-file-not-found", name = name),
        (FontError::Unsupported, _) => t!("settings-font-unsupported", name = name),
    })
}

/// 界面缩放的档位（百分比）：设置菜单中列出，快捷键依次经过。
///
/// 100% 附近每档 10%，便于微调；更大的倍数每档 25%，较少几次即可放大到上限。
pub(crate) const ZOOM_STEPS: [u16; 9] = [70, 80, 90, 100, 110, 125, 150, 175, 200];

impl App {
    /// 打开设置浮层；已经打开时不处理。
    ///
    /// 每次打开都从第一页的顶部开始。
    pub(crate) fn open_settings(&mut self) {
        if self.settings.open {
            return;
        }

        // 先关闭其他浮层和菜单：设置浮层位于最上层，不与它们同时显示。
        if self.overlays.picker.open {
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
        self.settings.logo_played = false;
        self.settings.search.set_text("");
        self.settings.before_search = None;
        self.settings.error = self.broken_font(self.settings.selected.page());
        self.settings.keyboard = false;
        self.settings.hover = None;
        self.settings.listed = Listed::default();
        self.refresh_vim_key_rows();
        // 每次打开设置重新查询发行版中的工具：其间新安装的能列出来。
        self.format.formatter.forget_wsl();
        self.refresh_format_rows(true);
        self.settings.integration = crate::shell_integration::read();
        self.refresh_changed_settings();
        // 列出字体要读取字体文件，先在后台读好，打开字体菜单时不必等待；「默认 Shell」菜单中的
        // shell 同样在后台检测。
        std::thread::spawn(winc_render::installed_fonts);
        self.detect_terminal_shells();
        // 能否找到工具取决于用户 shell 中的 `PATH`，因此在后台获取该环境，取到之后再补上提醒。
        if let Some(proxy) = self.link.proxy.clone() {
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
        self.cancel_key_edit();
        if self.settings_dragging() {
            self.pointer.drag = Drag::None;
        }
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
        self.settings.logo = None;
        self.ime.field_preedit.clear();

        self.set_recording(false);
        if matches!(
            self.focus,
            Focus::Settings | Focus::Setting | Focus::KeySearch | Focus::SettingsSearch
        ) {
            self.focus = Focus::Editor;
        }

        self.request_redraw();
    }

    /// 返回设置浮层中窗口坐标 (`x`, `y`) 处的部件。
    ///
    /// 先判断上一帧绘制出的「安装方法…」和「恢复默认」，再判断布局中的部件。滚动到标题之下而
    /// 不可见的链接不可点击。
    pub(super) fn settings_part_at(&self, x: f32, y: f32) -> Option<SettingsPart> {
        let layout = self.settings_layout();
        if layout.page == SettingsPage::Keys
            && !layout.searching
            && let Some(part) = self.keys_part_at(layout.body, x, y)
        {
            return Some(part);
        }
        let link = self
            .hits
            .install_links
            .iter()
            .find(|(_, r)| r.contains(x, y) && layout.body.contains(x, y));
        if let Some(&(setting, _)) = link {
            return Some(SettingsPart::Install(setting));
        }
        let reset = (self.hits.reset_links.iter())
            .find(|(_, r)| r.contains(x, y) && layout.body.contains(x, y));
        if let Some(&(setting, _)) = reset {
            return Some(SettingsPart::Reset(setting));
        }
        match layout.part_at(x, y)? {
            // 版本号和链接只有文字那一块可点，与悬停底色一致。
            part @ (SettingsPart::Copy(setting) | SettingsPart::Link(setting)) => {
                let area = self.settings_value_area(&layout, setting)?;
                area.contains(x, y).then_some(part)
            }
            // 不支持应用内更新时不显示「检查更新」按钮。
            SettingsPart::Action(Setting::Updates) if !self.updates_supported() => None,
            part => Some(part),
        }
    }

    /// 返回「关于」页中版本号或链接 `setting` 实际占的区域（悬停底色和点击范围）；不在布局
    /// `layout` 中时返回 `None`。
    pub(in crate::app) fn settings_value_area(
        &self,
        layout: &SettingsLayout,
        setting: Setting,
    ) -> Option<Rect> {
        let row = layout.row(setting)?;
        let measure = |text: &str| {
            (self.frame.renderer.as_ref())
                .map_or(0.0, |r| r.measure(FontKind::Ui, self.fonts.ui, text))
        };
        Some(if setting == Setting::Version {
            let text_w = measure(&crate::layout::version_line());
            crate::layout::ProductParts::new(row.row).version_area(text_w)
        } else {
            let text = crate::layout::about_value(setting).unwrap_or_default();
            let content = measure(&text) + crate::layout::LINK_ICON_W;
            crate::layout::value_area(row.control, content)
        })
    }

    /// 返回边长 `size`（物理像素）的产品图标（RGBA），首次用到或缩放改变时从应用图标的 SVG
    /// 渲染并留存。
    pub(crate) fn product_logo(&mut self, size: u32) -> &[u8] {
        const LOGO: &str = crate::app::setup::ICON;
        if self.settings.logo.as_ref().is_none_or(|&(s, _)| s != size) {
            let rgba = winc_preview::svg::load(LOGO, None, None)
                .map(|picture| picture.render(size, size).rgba)
                .unwrap_or_default();
            self.settings.logo = Some((size, rgba));
        }
        self.settings.logo.as_ref().map_or(&[], |(_, rgba)| rgba)
    }

    /// 每帧绘制前调用：「关于」页刚显示出来时播放一次图标逐层出现的动画。
    ///
    /// 设置浮层出现期间画的是开头的快照，动画等浮层出现之后再开始；动态效果关闭时不播放。
    pub(in crate::app) fn track_about_logo(&mut self) {
        if !self.settings.open {
            return;
        }
        let about = self.settings.selected.page() == crate::layout::SettingsPage::About
            && !self.settings_searching();
        if !about {
            self.settings.logo_played = false;
            self.motion.about_logo = None;
        } else if !std::mem::replace(&mut self.settings.logo_played, true) {
            let wait = match &self.settings.motion {
                Some((true, clock)) => super::animation::DIALOG_IN.mul_f32(1.0 - clock.linear()),
                _ => std::time::Duration::ZERO,
            };
            let total = wait + crate::app::setup::LOGO_BUILD;
            let delay = wait.as_secs_f32() / total.as_secs_f32();
            self.motion.about_logo = self.clock(total).map(|clock| (clock, delay));
        }
    }

    /// 返回「关于」页图标逐层出现的进度，0 到 1；没有在播放时为 `None`。
    pub(crate) fn about_logo_build(&self) -> Option<f32> {
        let (clock, delay) = self.motion.about_logo?;
        Some(((clock.linear() - delay) / (1.0 - delay)).clamp(0.0, 1.0))
    }

    /// 是否支持应用内更新：不支持时「关于」页只写明原因，不显示「检查更新」按钮。
    pub(crate) fn updates_supported(&self) -> bool {
        use crate::winc::UpdateState;
        !matches!(*self.link.shared.update_state.borrow(), UpdateState::Off)
    }

    /// 打开「关于」页的链接 `setting`：更新日志在阅读标签中打开，其余的在浏览器中打开，打不开
    /// 时在状态栏说明原因。
    pub(in crate::app) fn settings_open_link(&mut self, setting: Setting) {
        self.settings.selected = setting;
        if setting == Setting::ReleaseNotes {
            return self.settings_open_release_notes();
        }
        let Some(url) = crate::layout::about_link(setting) else {
            return;
        };
        if let Err(e) = crate::system::open_url(&url) {
            self.flash(t!("settings-link-open-failed", error = e.to_string()));
        }
    }

    /// 返回设置浮层当前帧的几何布局，绘制、悬停和点击共用同一份。
    ///
    /// 说明文字用渲染器量宽度后折行；绘制期间渲染器不在 `App` 中，绘制时改用
    /// [`Self::settings_layout_with`]，以同一份字形缓存量出相同的结果。
    pub(crate) fn settings_layout(&self) -> SettingsLayout {
        let size = self.fonts.ui_small;
        let renderer = self.frame.renderer.as_ref();
        self.settings_layout_with(&mut |text: &str| {
            renderer.map_or(0.0, |r| r.measure(FontKind::Ui, size, text))
        })
    }

    /// 同 [`Self::settings_layout`]，说明文字（界面小字号）的宽度由 `measure` 给出。
    pub(crate) fn settings_layout_with(
        &self,
        measure: &mut dyn FnMut(&str) -> f32,
    ) -> SettingsLayout {
        let view = SettingsView {
            page: self.settings.selected.page(),
            query: self.settings.search.text(),
            listed: &self.settings.listed,
            terminal: self
                .motion
                .module_fx
                .terminal
                .share(self.terminal_enabled()),
            git: self.motion.module_fx.git.share(self.git_enabled()),
            editing: self.settings.editing.as_ref().map(|(s, _)| *s),
            scroll: self.settings.scroll,
            offset: self.settings.offset,
        };
        SettingsLayout::compute(self.layout.window, &view, measure)
    }

    /// 是否在显示搜索结果：分类栏顶部的搜索框不为空白。
    pub(crate) fn settings_searching(&self) -> bool {
        !self.settings.search.text().trim().is_empty()
    }

    /// 各可选模块是否开启，决定设置中列出哪些页。
    pub(crate) fn settings_modules(&self) -> crate::layout::Modules {
        crate::layout::Modules {
            git: self.git_enabled(),
            terminal: self.terminal_enabled(),
        }
    }

    /// 返回键盘上下移动经过的设置项：搜索时为搜索结果，否则为各页的全部设置项。
    fn settings_items(&self) -> Vec<Setting> {
        let (listed, modules) = (&self.settings.listed, self.settings_modules());
        if self.settings_searching() {
            Setting::found(self.settings.search.text(), listed, modules)
        } else {
            Setting::all(listed, modules)
        }
    }

    /// 把焦点移到分类栏顶部的搜索框，全选其中的文字。
    pub(super) fn focus_settings_search(&mut self) {
        self.settings_leave_field();
        self.set_recording(false);
        self.settings.search.select_all();
        self.focus = Focus::SettingsSearch;
        self.reset_blink();
        self.request_redraw();
    }

    /// 搜索框的内容变化后：结果从顶部显示，选中第一条结果；清空后回到搜索之前选中的项。
    pub(in crate::app) fn settings_search_changed(&mut self) {
        self.settings.scroll = 0.0;
        self.settings.keyboard = false;
        if self.settings_searching() {
            if let Some(&first) = self.settings_items().first() {
                let before = (self.settings.before_search).map_or(self.settings.selected, |b| b.0);
                self.settings.selected = first;
                self.settings.before_search = Some((before, first));
            }
        } else if let Some((before, first)) = self.settings.before_search.take()
            && self.settings.selected == first
        {
            self.settings.selected = before;
        }
        self.request_redraw();
    }

    /// 从搜索框进入结果列表：键盘焦点回到浮层，选中第一条结果；没有结果时不处理。搜索框为空时
    /// 回到当前页。
    pub(super) fn settings_enter_results(&mut self) {
        if !self.settings_searching() {
            self.settings_show_page(self.settings.selected.page());
            self.settings.keyboard = true;
            return;
        }
        let Some(&first) = self.settings_items().first() else {
            return;
        };
        self.settings.before_search = None;
        self.settings.selected = first;
        self.settings.keyboard = true;
        self.focus = Focus::Settings;
        self.settings_scroll_into_view();
        self.request_redraw();
    }

    /// 切换到 `page`，并选中该页的第一项。
    ///
    /// 正在编辑时先离开输入框（见 [`App::settings_leave_field`]）。进入快捷键页时，光标直接
    /// 位于搜索框中。
    pub(super) fn settings_show_page(&mut self, page: SettingsPage) {
        // 未列出的页（模块关闭后正在淡出的那一项）不切换过去。
        if !SettingsPage::listed(self.settings_modules()).any(|listed| listed == page) {
            return;
        }
        self.settings_leave_field();
        self.settings.before_search = None;
        // 换页即离开搜索结果；选中的结果在该页时仍选中它。
        if self.settings_searching() {
            self.settings.search.set_text("");
            self.settings.scroll = 0.0;
        }
        if self.settings.selected.page() != page {
            self.settings.selected = page.first();
            self.settings.scroll = 0.0;
            self.settings.error = self.broken_font(page);
        }
        self.set_recording(false);
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
        let open = self.settings.key_edit.as_ref().map(|edit| edit.action);
        let (closing, t) = match self.settings.key_slide {
            Some((closing, clock)) => (closing, clock.progress()),
            None => (None, 1.0),
        };
        self.keys_layout_with(KeyRows { open, closing, t })
    }

    /// 返回快捷键页各行高度按 `rows` 计算时的几何布局。
    pub(crate) fn keys_layout_with(&self, rows: KeyRows) -> KeysLayout {
        let query = self.settings.keys_query.text();
        let recorded = self.settings.keys_recorded;
        let keep = |row: &keymap::Row| keymap::row_matches(row, query, Os::CURRENT, recorded);
        let body = self.settings_layout().body;
        let scroll = self.settings.scroll;
        let layout = KeysLayout::compute(body, keymap::current_rows(), keep, rows, scroll);
        // 收起一行后列表变短，滚动量超出范围时内容贴住底部。
        let max = layout.max_scroll();
        if scroll <= max {
            return layout;
        }
        KeysLayout::compute(body, keymap::current_rows(), keep, rows, max)
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

    /// 开始（`on` 为 `true`）或停止录制按键。
    ///
    /// macOS 上菜单栏中带快捷键的菜单项（「设置…」的 ⌘,、「退出」的 ⌘Q 等）先于窗口收到按键；
    /// 录制期间暂停这些快捷键，按键仍然送到窗口、写进搜索框。
    pub(in crate::app) fn set_recording(&mut self, on: bool) {
        if self.settings.recording == on {
            return;
        }
        self.settings.recording = on;
        // 没有窗口（测试中）时没有菜单栏，也不在主线程上。
        #[cfg(target_os = "macos")]
        if self.window.handle.is_some() {
            crate::macos::suspend_menu_shortcuts(on);
        }
    }

    /// 开始或停止录制按键；开始时清空搜索框。
    pub(super) fn toggle_recording(&mut self) {
        self.set_recording(!self.settings.recording);
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

        let all = self.settings_items();
        if all.is_empty() {
            return;
        }
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
        // 搜索结果在同一列中，跨页移动不回到顶部。
        if all[next].page() != self.settings.selected.page() && !self.settings_searching() {
            self.settings.scroll = 0.0;
            self.settings.error = self.broken_font(all[next].page());
        }
        self.settings.selected = all[next];
        self.set_recording(false);
        self.focus = if all[next] == Setting::KeySearch {
            Focus::KeySearch
        } else {
            Focus::Settings
        };
        self.settings_scroll_into_view();
        self.request_redraw();
    }

    /// 打开设置，切到 `setting` 所在的页，选中并滚动到这一项；选中的一行铺上底色，便于找到。
    pub(in crate::app) fn show_setting(&mut self, setting: Setting) {
        self.open_settings();
        self.settings_show_page(setting.page());
        self.settings.selected = setting;
        self.settings.keyboard = true;
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
    pub(in crate::app) fn settings_scroll_by(&mut self, dy: f32) {
        let (_, max) = self.settings_scroll_area();
        // 列表收短后（如快捷键页收起一行）滚动量可能超出范围，从有效范围内的位置起算。
        self.settings.scroll = (self.settings.scroll.min(max) - dy).clamp(0.0, max);
        self.settings.scrolled_at = Some(Instant::now());
        self.request_redraw();
    }

    /// 当前设置页的滚动区域和最大滚动量：快捷键页为命令列表，其他页为正文。
    pub(in crate::app) fn settings_scroll_area(&self) -> (Rect, f32) {
        if self.settings.selected.page() == SettingsPage::Keys {
            let keys = self.keys_layout();
            (keys.list, keys.max_scroll())
        } else {
            let layout = self.settings_layout();
            (layout.body, layout.max_scroll())
        }
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
            | SettingControl::Product
            | SettingControl::Link
            | SettingControl::Button => {}
        }
    }

    /// 处理「关于」页更新按钮的点击，和状态栏上的更新按钮做同一件事。
    ///
    /// 还没找到新版本时检查；找到之后下载安装，下载完成（Windows 上为已下载）后重新启动。
    /// macOS 上这些步骤都在 Sparkle 的更新窗口里进行。不支持更新，或正在检查、下载时忽略本次点击。
    pub(in crate::app) fn settings_check_updates(&mut self) {
        self.settings.selected = Setting::Updates;
        let click = self.update_click();
        #[cfg(target_os = "macos")]
        if click != UpdateClick::None {
            crate::macos::check_for_updates(true);
        }
        #[cfg(not(target_os = "macos"))]
        match click {
            UpdateClick::None => {}
            UpdateClick::Check => crate::updates::check(),
            UpdateClick::Install => crate::updates::install(),
            UpdateClick::Restart => {
                crate::updates::restart();
                self.request_quit();
            }
        }
    }

    /// 「关于」页更新按钮这次点击该做什么。
    pub(crate) fn update_click(&self) -> UpdateClick {
        UpdateClick::from_state(&self.link.shared.update_state.borrow())
    }

    /// 把版本信息（[`crate::system::info::report`]：版本、架构和系统）复制到剪贴板，供问题反馈
    /// 使用，并在状态栏提示；`setting` 是「关于」页的版本号一行。
    pub(in crate::app) fn settings_copy_value(&mut self, setting: Setting) {
        self.settings.selected = setting;
        let report = crate::system::info::report(self.display_protocol());
        self.set_clipboard(report, false);
        self.flash(t!("settings-version-info-copied"));
    }

    /// 窗口所用的显示协议：Linux 上为 `Wayland` 或 `X11`，其他平台和没有窗口时为 `None`。
    fn display_protocol(&self) -> Option<&'static str> {
        #[cfg(target_os = "linux")]
        {
            use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            let window = self.window.handle.as_ref()?;
            match window.display_handle().ok()?.as_raw() {
                RawDisplayHandle::Wayland(_) => Some("Wayland"),
                RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => Some("X11"),
                _ => None,
            }
        }
        #[cfg(not(target_os = "linux"))]
        None
    }

    /// 处理 `↩` 或空格：输入框进入编辑，开关切换，格式化命令、「添加语言…」「排除的语言」弹出菜单，
    /// 数值项和主题项向前一步（等同 `→`），版本号复制到剪贴板，链接在浏览器中打开，搜索项把焦点移到
    /// 搜索框。
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
            SettingControl::Product => self.settings_copy_value(setting),
            SettingControl::Link => self.settings_open_link(setting),
            SettingControl::Stepper | SettingControl::Themes | SettingControl::ThemeList => {
                self.settings_step(true)
            }
            SettingControl::Search => self.focus = Focus::KeySearch,
        }
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
            None if setting == Setting::TerminalShell => self.config.terminal_shell.clone(),
            None if setting == Setting::GitPath => self.config.git_path.clone(),
            None if setting == Setting::LatexPath => self.config.latex_path.clone(),
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
        let fonts = [
            Setting::EditorFont,
            Setting::UiFont,
            Setting::TerminalFont,
            Setting::MarkdownFont,
        ];
        fonts
            .into_iter()
            .filter(|setting| setting.page() == page)
            .find_map(|setting| font_error(self.font_value(setting)).map(|error| (setting, error)))
    }

    /// 字体设置项 `setting` 在配置中的值：以逗号分隔的字体列表，未设置时为 `None`。
    pub(in crate::app) fn font_value(&self, setting: Setting) -> &Option<String> {
        match setting {
            Setting::UiFont => &self.config.ui_font,
            Setting::TerminalFont => &self.config.terminal_font,
            Setting::MarkdownFont => &self.config.markdown_font,
            _ => &self.config.editor_font,
        }
    }

    fn font_setting(&mut self, setting: Setting) -> &mut Option<String> {
        match setting {
            Setting::UiFont => &mut self.config.ui_font,
            Setting::TerminalFont => &mut self.config.terminal_font,
            Setting::MarkdownFont => &mut self.config.markdown_font,
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
    /// 键入到一半的名字（例如 `Menl`）原本就找不到，每敲一个字符就换一次字体会使界面反复变化。
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

    /// 提交正在编辑的字体名或命令。
    pub(super) fn settings_commit_field(&mut self) {
        let Some((setting, field)) = self.settings.editing.as_ref() else {
            return;
        };
        let (setting, text) = (*setting, field.text().trim().to_owned());
        let value = (!text.is_empty()).then_some(text);

        match tool_setting(setting) {
            Some((task, id)) => self.settings_commit_command(task, id, value),
            None if setting == Setting::TerminalShell => {
                self.settings.editing = None;
                self.settings.editing_original = None;
                self.settings.error = None;
                self.focus = Focus::Settings;
                self.set_terminal_shell(value);
            }
            None if setting == Setting::GitPath => {
                self.settings.editing = None;
                self.settings.editing_original = None;
                self.settings.error = None;
                self.focus = Focus::Settings;
                self.set_git_path(value);
            }
            None if setting == Setting::LatexPath => {
                self.settings.editing = None;
                self.settings.editing_original = None;
                self.settings.error = None;
                self.focus = Focus::Settings;
                self.set_latex_path(value);
            }
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
    /// 名字无法识别就留在输入框里，配置退回原值。
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

    /// 字体菜单中选中字体 `name`：换掉列表中的第一个字体，其后的后备字体保留（`name` 原在其中
    /// 时从中移去）。
    pub(in crate::app) fn settings_choose_primary_font(&mut self, setting: Setting, name: &str) {
        let current = self.font_value(setting).clone();
        let list = winc_config::font_list(current.as_deref());
        let rest = list.into_iter().skip(1).filter(|&n| n != name);
        let list: Vec<&str> = std::iter::once(name).chain(rest).collect();
        self.settings_choose_font(setting, Some(list.join(", ")));
    }

    /// 「后备字体」中点击字体 `name`：不在列表中时加到末尾，已在其中时移去。第一个字体不变。
    pub(in crate::app) fn settings_toggle_fallback_font(&mut self, setting: Setting, name: &str) {
        let current = self.font_value(setting).clone();
        let mut list = winc_config::font_list(current.as_deref());
        if list.first().is_none_or(|&first| first == name) {
            return;
        }
        match list.iter().position(|&n| n == name) {
            Some(at) => _ = list.remove(at),
            None => list.push(name),
        }
        self.settings_choose_font(setting, Some(list.join(", ")));
    }

    /// 放大或缩小字号（`⌘+` `⌘−`），`⌘0` 还原（`up` 为 `None`）。
    ///
    /// 焦点在终端时调整终端字号，否则调整编辑区字号，两者互不影响；还原即回到默认字号。改动
    /// 写回配置，界面字号不变。
    pub(super) fn step_font_size(&mut self, up: Option<bool>) {
        // 当前标签是 PDF 或图片时缩放预览。
        if !self.focus.in_terminal() && (self.zoom_pdf(up, None) || self.zoom_picture(up, None)) {
            return;
        }
        self.step_font_size_in(self.focus.in_terminal(), up);
    }

    /// 同 [`Self::step_font_size`]，由 `terminal` 指定调整终端还是编辑区的字号。
    pub(super) fn step_font_size_in(&mut self, terminal: bool, up: Option<bool>) {
        let pick = |fonts: ViewFonts| if terminal { fonts.terminal } else { fonts.code };
        let current = pick(ViewFonts::of(&self.config));
        let next = up.map(|up| step(current, up, Config::EDITOR_SIZE_RANGE));
        if next == Some(current) {
            let size = current.to_string();
            self.flash(if terminal {
                t!("settings-terminal-font-size-limit", size = size)
            } else {
                t!("settings-font-size-limit", size = size)
            });
            return;
        }
        if terminal {
            self.config.terminal_size = next;
        } else {
            self.config.editor_size = next;
        }

        let size = pick(ViewFonts::of(&self.config)).to_string();
        self.apply_font_sizes();
        self.save_config();
        self.flash(if terminal {
            t!("settings-terminal-font-size", size = size)
        } else {
            t!("settings-font-size", size = size)
        });
    }

    /// 按档位（[`ZOOM_STEPS`]）放大或缩小界面（`up` 为 `true` 时放大），`up` 为 `None` 时还原为
    /// 100%。
    ///
    /// 配置文件中写了档位以外的值时，移到该方向上最近的档位。改动写回配置。
    pub(super) fn step_zoom(&mut self, up: Option<bool>) {
        let current = self.config.ui_zoom.unwrap_or(100);
        let next = match up {
            Some(true) => ZOOM_STEPS.into_iter().find(|&zoom| zoom > current),
            Some(false) => ZOOM_STEPS.into_iter().rev().find(|&zoom| zoom < current),
            None => Some(100),
        };
        let Some(next) = next else {
            self.flash(t!("settings-zoom-limit", zoom = current.to_string()));
            return;
        };
        self.set_zoom(next);
        self.flash(t!("settings-zoom", zoom = next.to_string()));
    }

    /// 把界面缩放设为 `zoom`（百分比）并写回配置；100% 时从配置中去掉该项。
    pub(in crate::app) fn set_zoom(&mut self, zoom: u16) {
        self.config.ui_zoom = (zoom != 100).then_some(zoom);
        self.apply_zoom();
        self.save_config();
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

    /// 设置 `git` 程序的路径；`None` 表示在 `PATH` 里查找。随后重新查找 `git` 并刷新。
    pub(in crate::app) fn set_git_path(&mut self, path: Option<String>) {
        if self.config.git_path == path {
            return;
        }
        self.config.git_path = path;
        self.save_config();
        self.git_program_changed();
        self.request_redraw();
    }

    /// 设置编译 LaTeX 的程序，`None` 表示自动查找。
    pub(in crate::app) fn set_latex_path(&mut self, path: Option<String>) {
        if self.config.latex_path == path {
            return;
        }
        self.config.latex_path = path;
        self.save_config();
        self.preview.latex.forget_tool();
        self.find_latex_tool();
        self.request_redraw();
    }

    /// 设置点击工作树时在哪个窗口打开它。
    pub(in crate::app) fn set_open_worktree(&mut self, choice: winc_config::WorktreeOpen) {
        self.config.git_open_worktree = Some(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 设置拉取、同步时合并还是变基。
    pub(in crate::app) fn set_pull_mode(&mut self, choice: winc_config::PullMode) {
        self.config.git_pull = Some(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 设置 Vim 模式下无名寄存器与系统剪贴板的同步方式。
    pub(in crate::app) fn set_vim_clipboard(&mut self, choice: winc_config::VimClipboard) {
        self.config.vim_clipboard = Some(choice);
        self.vim_apply_config();
        self.save_config();
        self.request_redraw();
    }

    /// 设置何时显示空格和制表符。
    pub(in crate::app) fn set_render_whitespace(&mut self, choice: winc_config::RenderWhitespace) {
        self.vim_forget_option(Setting::RenderWhitespace);
        self.config.render_whitespace = Some(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 设置新建终端时运行的命令（`terminal.shell`）；`None` 表示系统默认。已打开的终端不受影响。
    pub(in crate::app) fn set_terminal_shell(&mut self, command: Option<String>) {
        self.settings.selected = Setting::TerminalShell;
        self.config.terminal_shell = command;
        self.save_config();
        self.request_redraw();
    }

    /// 设置终端面板的位置：`right` 为编辑区右侧，否则为下方。
    pub(in crate::app) fn set_terminal_position(&mut self, right: bool) {
        if right != self.terminal_right() {
            self.toggle_terminal_position();
        }
    }

    /// 设置终端标签的位置：`right` 为右侧，否则为顶部。
    pub(in crate::app) fn set_terminal_tab_position(&mut self, right: bool) {
        self.settings.selected = Setting::TerminalTabPosition;
        if right == (self.config.terminal_vertical_tabs == Some(true)) {
            return;
        }
        self.config.terminal_vertical_tabs = Some(right);
        self.relayout();
        self.save_config();
        self.request_redraw();
    }

    /// 设置终端中用作 Meta 键的 ⌥。
    pub(in crate::app) fn set_option_as_meta(&mut self, choice: winc_config::OptionAsMeta) {
        self.config.option_as_meta = Some(choice);
        self.save_config();
        self.request_redraw();
    }

    /// 把配置中修改过的项写回配置文件，再改用文件中的配置，使程序运行期间手动修改的项生效。
    ///
    /// 写入失败时提示原因，不影响继续使用，下次写回时再写入这些项。原文件无法解析或含有注释时，
    /// 先把原文件另存为一个副本再写入（写出的文件不保留注释），并提示副本的位置。版本号
    /// （`config_rev`）加一，应用层据此把新配置发给其他窗口。
    pub(super) fn save_config(&mut self) {
        self.store.config_rev += 1;
        // `:set` 的改动只在本次运行中生效：写入时换回配置文件中的值。
        self.vim_swap_options();
        self.write_config();
        self.vim_swap_options();
        if self.settings.open {
            self.refresh_changed_settings();
        }
    }

    /// 把配置写入配置文件。
    fn write_config(&mut self) {
        let Some(path) = &self.store.config_path else {
            return;
        };

        match self.config.save_to(path, &self.store.saved_config) {
            Ok(saved) => {
                if let Some(backup) = saved.backup {
                    let path = backup.path.display().to_string();
                    self.notify(
                        NoticeKind::Warning,
                        if backup.comments {
                            t!("settings-config-comments-backed-up", path = path)
                        } else {
                            t!("settings-config-backed-up", path = path)
                        },
                    );
                }
                self.store.saved_config = saved.config.clone();
                if saved.config != self.config {
                    self.adopt_config(saved.config);
                }
            }
            Err(e) => self.notify(
                NoticeKind::Error,
                t!("settings-config-write-failed", error = e.to_string()),
            ),
        }
    }

    /// 设置光标移动的动画方式，并写回配置。
    pub(in crate::app) fn set_smooth_caret(&mut self, style: winc_config::SmoothCaret) {
        self.config.smooth_caret_movement = Some(style);
        self.motion.caret_motion = None;
        self.save_config();
        self.request_redraw();
    }
}

/// 配置与状态文件的写回：文件的路径、配置文件当前的内容和配置的版本号。
pub(crate) struct Store {
    /// 配置文件当前的内容，即上次读入或写出时的配置。
    ///
    /// 写回时只写入 `config` 中与本字段不同的项，
    /// 因此运行期间在配置文件中手动修改的其他项保持不变。
    pub(crate) saved_config: Config,
    /// 配置写回的文件；`None` 时只修改内存中的配置（测试使用，避免改动用户的配置）。
    pub config_path: Option<PathBuf>,
    /// 状态写回的文件；`None` 时只记录在内存中（测试和性能测量使用，避免改动用户的历史）。
    pub state_path: Option<PathBuf>,
    /// 向后台提交过几次状态文件的读写；只采用最后一次送回的状态。
    pub(crate) state_requests: u64,
    /// 配置的版本号：本窗口每写入一次配置文件加 1，应用层据此把新配置发给其他窗口。
    pub(crate) config_rev: u64,
}

#[cfg(test)]
mod tests;
