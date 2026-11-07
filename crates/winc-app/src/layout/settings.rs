//! 设置卡片的几何：分类、各项设置的行和控件、快捷键一页、步进器、主题缩略图。

use super::*;

use std::borrow::Cow;

use winc_i18n::t;

/// 设置的一个分类，对应左侧分类栏中的一项和右侧的一页。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// 外观：主题、界面字体和字号。
    #[default]
    Appearance,
    /// 编辑器：编辑区字体和字号、Tab 宽度、保存与滚动相关的选项。
    Editor,
    /// 格式化：保存时的格式化和修复，以及各语言的工具命令。
    Format,
    /// 模块：可选模块的开关。
    Modules,
    /// 快捷键：只列出键位，不含设置项，整页由 [`KeysLayout`] 单独布局。
    Keys,
    /// 关于：版本和应用内更新。
    About,
}

impl SettingsPage {
    /// 全部分类，顺序与分类栏从上到下的顺序一致。
    pub const ALL: [SettingsPage; 6] = [
        SettingsPage::Appearance,
        SettingsPage::Editor,
        SettingsPage::Format,
        SettingsPage::Modules,
        SettingsPage::Keys,
        SettingsPage::About,
    ];

    /// 返回分类的标题。
    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::Appearance => t!("settings-page-appearance"),
            SettingsPage::Editor => t!("settings-page-editor"),
            SettingsPage::Format => t!("settings-page-format"),
            SettingsPage::Modules => t!("settings-page-modules"),
            SettingsPage::Keys => t!("settings-page-keys"),
            SettingsPage::About => t!("settings-page-about"),
        }
    }

    /// 返回这一页的各组设置项，组内的项从上到下排列。
    ///
    /// 格式化页中每种语言占一行，`languages` 是要列出的语言。
    pub fn sections(self, languages: &ToolLanguages) -> Vec<Section> {
        let plain = |items: Vec<Setting>| Section { title: None, items };
        match self {
            SettingsPage::Appearance => {
                let mut sections = vec![
                    plain(vec![
                        Setting::Theme,
                        Setting::Language,
                        Setting::UiFont,
                        Setting::UiSize,
                    ]),
                    plain(vec![Setting::StatusBar, Setting::RestoreWindow]),
                    Section {
                        title: Some(t!("settings-light-theme")),
                        items: vec![Setting::LightTheme],
                    },
                    Section {
                        title: Some(t!("settings-dark-theme")),
                        items: vec![Setting::DarkTheme],
                    },
                ];
                // macOS 上标题栏总是和内容融为一体，没有该项。
                if !cfg!(target_os = "macos") {
                    sections.insert(1, plain(vec![Setting::SystemTitleBar]));
                }
                sections
            }
            SettingsPage::Editor => vec![
                plain(vec![
                    Setting::EditorFont,
                    Setting::EditorSize,
                    Setting::TabWidth,
                ]),
                plain(vec![
                    Setting::LineNumbers,
                    Setting::WordWrap,
                    Setting::Minimap,
                    Setting::StickyScroll,
                    Setting::RenderWhitespace,
                ]),
                plain(vec![
                    Setting::AutoSave,
                    Setting::HotExit,
                    Setting::RestoreFolder,
                ]),
                plain(vec![
                    Setting::CompactFolders,
                    Setting::DiffInline,
                    Setting::RemoteImages,
                ]),
            ],
            SettingsPage::Modules => vec![plain(vec![Setting::GitModule, Setting::TerminalModule])],
            SettingsPage::Format => {
                let mut sections = vec![
                    plain(vec![
                        Setting::FormatOnSave,
                        Setting::FixOnSave,
                        Setting::SkipOnSave,
                    ]),
                    Section {
                        title: Some(t!("settings-section-format-tools")),
                        items: languages
                            .format
                            .iter()
                            .map(|&id| Setting::FormatCommand(id))
                            .chain([Setting::AddLanguage])
                            .collect(),
                    },
                ];
                // 没有列出的语言时只有「添加语言…」，直接跟在组标题下面。
                sections.push(Section {
                    title: Some(t!("settings-section-fix-tools")),
                    items: languages
                        .fix
                        .iter()
                        .map(|&id| Setting::FixCommand(id))
                        .chain([Setting::AddFixLanguage])
                        .collect(),
                });
                sections
            }
            SettingsPage::Keys => Vec::new(),
            SettingsPage::About => vec![
                plain(vec![Setting::Version]),
                plain(vec![Setting::Updates, Setting::CheckUpdates]),
            ],
        }
    }

    /// 返回这一页的第一项；切换到这一页时选中第一项。
    pub fn first(self) -> Setting {
        match self {
            SettingsPage::Keys => Setting::KeySearch,
            page => page.sections(&ToolLanguages::default())[0].items[0],
        }
    }
}

/// 格式化页列出的语言（语言 id）：格式化工具一组，修复工具一组。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolLanguages {
    /// 「格式化工具」一组列出的语言 id。
    pub format: Vec<&'static str>,
    /// 「修复工具」一组列出的语言 id。
    pub fix: Vec<&'static str>,
}

/// 一页中的一组设置项，共用一块底色，可以带标题。
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// 组标题；为 `None` 时不显示标题。
    pub title: Option<&'static str>,
    /// 组内的设置项，从上到下排列。
    pub items: Vec<Setting>,
}

/// 一个设置项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Setting {
    /// 主题。
    #[default]
    Theme,
    /// 浅色外观时用的主题。
    LightTheme,
    /// 深色外观时用的主题。
    DarkTheme,
    /// 界面语言。
    Language,
    /// 编辑区字体。
    EditorFont,
    /// 编辑区字号。
    EditorSize,
    /// Tab 宽度。
    TabWidth,
    /// 自动保存。
    AutoSave,
    /// 热退出：退出时不逐个询问是否保存，下次打开时恢复未保存的修改。
    HotExit,
    /// 启动时打开上次退出时打开的目录和标签。
    RestoreFolder,
    /// 粘滞滚动：滚动时在编辑区顶部钉住当前所在的函数、类和标题。
    StickyScroll,
    /// 自动换行。
    WordWrap,
    /// 编辑区右侧的缩略图。
    Minimap,
    /// 显示行号。
    LineNumbers,
    /// 差异单栏显示。
    DiffInline,
    /// 何时显示空格和制表符。
    RenderWhitespace,
    /// 显示状态栏。
    StatusBar,
    /// 记住窗口大小和位置。
    RestoreWindow,
    /// 文件树把连续的单子目录合并显示为一行。
    CompactFolders,
    /// Markdown 预览联网加载网址的图片。
    RemoteImages,
    /// 定时检查更新。
    CheckUpdates,
    /// 「关于」页的版本号，只读。
    Version,
    /// 「关于」页的更新状态和「检查更新」按钮。
    Updates,
    /// 界面字体。
    UiFont,
    /// 界面字号。
    UiSize,
    /// 用系统的标题栏和边框，而不是由应用在顶栏中绘制窗口按钮（macOS 以外的平台）。
    SystemTitleBar,
    /// 保存时格式化。
    FormatOnSave,
    /// 保存时修复问题。
    FixOnSave,
    /// 保存时跳过的语言。
    SkipOnSave,
    /// 某种语言（语言 id）的格式化命令。
    FormatCommand(&'static str),
    /// 某种语言（语言 id）的修复命令。
    FixCommand(&'static str),
    /// 格式化工具一组下面的「添加语言…」。
    AddLanguage,
    /// 修复工具一组下面的「添加语言…」。
    AddFixLanguage,
    /// Git 模块的开关。
    GitModule,
    /// 终端模块的开关。
    TerminalModule,
    /// 「快捷键」页顶部的搜索框。
    ///
    /// 该页没有设置项，选中这一项即表示停留在该页。
    KeySearch,
}

impl Setting {
    /// 返回全部设置项，顺序为从第一页到最后一页、每页从上到下。
    ///
    /// 键盘上下移动按此顺序进行，越过一页的首尾时切换到相邻页。
    pub fn all(languages: &ToolLanguages) -> Vec<Setting> {
        SettingsPage::ALL
            .into_iter()
            .flat_map(|page| match page {
                // 「快捷键」页没有设置项，移动到该页即选中该页的搜索框。
                SettingsPage::Keys => vec![Setting::KeySearch],
                page => page
                    .sections(languages)
                    .into_iter()
                    .flat_map(|section| section.items)
                    .collect(),
            })
            .collect()
    }

    /// 返回该设置项所在的页。
    pub fn page(self) -> SettingsPage {
        match self {
            Setting::Theme
            | Setting::Language
            | Setting::LightTheme
            | Setting::DarkTheme
            | Setting::UiFont
            | Setting::UiSize
            | Setting::SystemTitleBar
            | Setting::StatusBar
            | Setting::RestoreWindow => SettingsPage::Appearance,
            Setting::EditorFont
            | Setting::EditorSize
            | Setting::TabWidth
            | Setting::AutoSave
            | Setting::HotExit
            | Setting::RestoreFolder
            | Setting::CompactFolders
            | Setting::StickyScroll
            | Setting::WordWrap
            | Setting::Minimap
            | Setting::LineNumbers
            | Setting::DiffInline
            | Setting::RenderWhitespace
            | Setting::RemoteImages => SettingsPage::Editor,
            Setting::Version | Setting::Updates | Setting::CheckUpdates => SettingsPage::About,
            Setting::FormatOnSave
            | Setting::FixOnSave
            | Setting::SkipOnSave
            | Setting::FormatCommand(_)
            | Setting::FixCommand(_)
            | Setting::AddLanguage
            | Setting::AddFixLanguage => SettingsPage::Format,
            Setting::GitModule | Setting::TerminalModule => SettingsPage::Modules,
            Setting::KeySearch => SettingsPage::Keys,
        }
    }

    /// 返回设置项左侧显示的名称。
    pub fn label(self) -> &'static str {
        match self {
            Setting::Theme => t!("settings-theme"),
            Setting::Language => t!("settings-language"),
            Setting::LightTheme => t!("settings-light-theme"),
            Setting::DarkTheme => t!("settings-dark-theme"),
            Setting::UiFont => t!("settings-ui-font"),
            Setting::UiSize => t!("settings-ui-size"),
            Setting::SystemTitleBar => t!("settings-system-title-bar"),
            Setting::EditorFont => t!("settings-editor-font"),
            Setting::EditorSize => t!("settings-editor-size"),
            Setting::TabWidth => t!("settings-tab-width"),
            Setting::AutoSave => t!("settings-auto-save"),
            Setting::HotExit => t!("settings-hot-exit"),
            Setting::RestoreFolder => t!("settings-restore-folder"),
            Setting::StickyScroll => t!("settings-sticky-scroll"),
            Setting::WordWrap => t!("settings-word-wrap"),
            Setting::Minimap => t!("settings-minimap"),
            Setting::LineNumbers => t!("settings-line-numbers"),
            Setting::DiffInline => t!("settings-diff-inline"),
            Setting::RenderWhitespace => t!("settings-render-whitespace"),
            Setting::StatusBar => t!("settings-status-bar"),
            Setting::RestoreWindow => t!("settings-restore-window"),
            Setting::CompactFolders => t!("settings-compact-folders"),
            Setting::RemoteImages => t!("settings-remote-images"),
            Setting::CheckUpdates => t!("settings-check-updates"),
            Setting::Version => t!("settings-version"),
            Setting::Updates => t!("settings-updates"),
            Setting::FormatOnSave => t!("settings-format-on-save"),
            Setting::FixOnSave => t!("settings-fix-on-save"),
            Setting::SkipOnSave => t!("settings-skip-on-save"),
            Setting::FormatCommand(id) | Setting::FixCommand(id) => {
                winc_syntax::find(id).map_or(id, |l| l.name)
            }
            Setting::AddLanguage | Setting::AddFixLanguage => t!("settings-add-language"),
            Setting::GitModule => "Git",
            Setting::TerminalModule => t!("settings-terminal-module"),
            Setting::KeySearch => t!("settings-key-search"),
        }
    }

    /// 返回名称下方的说明文字；没有说明时返回 `None`。
    ///
    /// 格式化命令的说明随状态变化，不在此处给出。终端模块的说明含开关终端的快捷键，按当前系统和
    /// 键位表给出。
    pub fn description(self) -> Option<Cow<'static, str>> {
        if self == Setting::TerminalModule {
            let text = match crate::keymap::shortcut(crate::keymap::Action::ToggleTerminal) {
                Some(key) => Cow::Owned(t!("settings-terminal-module-desc-key", key = key)),
                None => Cow::Borrowed(t!("settings-terminal-module-desc")),
            };
            return Some(text);
        }
        let text = match self {
            Setting::SystemTitleBar => Some(t!("settings-system-title-bar-desc")),
            Setting::AutoSave => Some(t!("settings-auto-save-desc")),
            Setting::HotExit => Some(t!("settings-hot-exit-desc")),
            Setting::RestoreFolder => Some(t!("settings-restore-folder-desc")),
            Setting::StickyScroll => Some(t!("settings-sticky-scroll-desc")),
            Setting::WordWrap => Some(t!("settings-word-wrap-desc")),
            Setting::CompactFolders => Some(t!("settings-compact-folders-desc")),
            Setting::Minimap => Some(t!("settings-minimap-desc")),
            Setting::LineNumbers => Some(t!("settings-line-numbers-desc")),
            Setting::DiffInline => Some(t!("settings-diff-inline-desc")),
            Setting::RenderWhitespace => Some(t!("settings-render-whitespace-desc")),
            Setting::StatusBar => Some(t!("settings-status-bar-desc")),
            Setting::RestoreWindow => Some(t!("settings-restore-window-desc")),
            Setting::RemoteImages => Some(t!("settings-remote-images-desc")),
            Setting::CheckUpdates => Some(t!("settings-check-updates-desc")),
            // 绘制时换成当前的更新状态（`UpdateState`）。
            Setting::Updates => Some(t!("settings-updates-off")),
            Setting::FormatOnSave => Some(t!("settings-format-on-save-desc")),
            Setting::FixOnSave => Some(t!("settings-fix-on-save-desc")),
            Setting::SkipOnSave => Some(t!("settings-skip-on-save-desc")),
            Setting::GitModule => Some(t!("settings-git-module-desc")),
            _ => None,
        };
        text.map(Cow::Borrowed)
    }

    /// 返回设置项右侧的控件种类。
    pub fn control(self) -> SettingControl {
        match self {
            Setting::EditorFont | Setting::UiFont => SettingControl::Font,
            Setting::EditorSize | Setting::UiSize | Setting::TabWidth => SettingControl::Stepper,
            Setting::Theme => SettingControl::Themes,
            Setting::LightTheme | Setting::DarkTheme => SettingControl::ThemeList,
            Setting::SystemTitleBar
            | Setting::AutoSave
            | Setting::HotExit
            | Setting::RestoreFolder
            | Setting::CompactFolders
            | Setting::StickyScroll
            | Setting::WordWrap
            | Setting::Minimap
            | Setting::LineNumbers
            | Setting::DiffInline
            | Setting::StatusBar
            | Setting::RestoreWindow
            | Setting::RemoteImages
            | Setting::CheckUpdates
            | Setting::FormatOnSave
            | Setting::FixOnSave
            | Setting::GitModule
            | Setting::TerminalModule => SettingControl::Switch,
            Setting::FormatCommand(_) | Setting::FixCommand(_) => SettingControl::Command,
            Setting::SkipOnSave | Setting::Language | Setting::RenderWhitespace => {
                SettingControl::Choice
            }
            Setting::AddLanguage | Setting::AddFixLanguage => SettingControl::Action,
            Setting::KeySearch => SettingControl::Search,
            Setting::Version => SettingControl::Value,
            Setting::Updates => SettingControl::Button,
        }
    }
}

/// 设置项右边的控件种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingControl {
    /// 字体名输入框。
    Font,
    /// 加减数值。
    Stepper,
    /// 外观（跟随系统、浅色、深色）的缩略图，在名称右边。
    Themes,
    /// 一排主题缩略图，在名称下面。
    ThemeList,
    /// 开关。
    Switch,
    /// 格式化工具：点击后弹出菜单的值；选择「自定义命令…」时换成比字体名输入框更宽的输入框。
    Command,
    /// 组下方的文字按钮（「添加语言…」）。
    Action,
    /// 点击后弹出菜单的值，显示当前选择（「语言」「排除的语言」）。
    Choice,
    /// 「快捷键」页的搜索框；不按设置行排列，整页单独布局（[`KeysLayout`]）。
    Search,
    /// 右侧一段只读的文字（版本号）。
    Value,
    /// 右侧的按钮（「检查更新」）。
    Button,
}

/// 返回「语言」中选项 `choice` 的名称。
///
/// 返回「空白符」设置中选项 `choice` 的名字。
pub fn whitespace_label(choice: winc_config::RenderWhitespace) -> &'static str {
    use winc_config::RenderWhitespace::{All, None, Selection, Trailing};
    match choice {
        None => t!("settings-render-whitespace-none"),
        Selection => t!("settings-render-whitespace-selection"),
        Trailing => t!("settings-render-whitespace-trailing"),
        All => t!("settings-render-whitespace-all"),
    }
}

/// 「跟随系统」使用当前界面语言，各语言的名称以该语言自身书写，不随界面语言变化。
pub fn locale_label(choice: winc_i18n::LocaleChoice) -> &'static str {
    match choice {
        winc_i18n::LocaleChoice::Auto => t!("settings-language-auto"),
        winc_i18n::LocaleChoice::Fixed(locale) => locale.native_name(),
    }
}

/// 返回一段界面文字的函数；文字随当前界面语言变化，常量表中以此代替字符串。
pub type Label = fn() -> &'static str;

/// 主题缩略图及返回其名称的函数，顺序为从左到右。
pub const THEME_CHOICES: [(ThemeChoice, Label); 3] = [
    (ThemeChoice::Auto, || t!("settings-theme-auto")),
    (ThemeChoice::Light, || t!("settings-theme-light")),
    (ThemeChoice::Dark, || t!("settings-theme-dark")),
];

/// 「关于」页的版本号：上游版本、这个 fork 的数字和处理器架构，例如 `0.1.502_115`（`x86_64`）。
pub fn version_label() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("{} ({arch})", crate::release_version::display_version())
}

/// 设置浮层中可以单独悬停和点击的部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPart {
    /// 右上角的关闭按钮。
    Close,
    /// 分类栏中的一项。
    Page(SettingsPage),
    /// 字体名、格式化命令的输入框。
    Field(Setting),
    /// 字体输入框右端的下拉按钮。
    FontMenu(Setting),
    /// 开关。
    Switch(Setting),
    /// 整行是按钮的一行，或点击后弹出菜单的值。
    Action(Setting),
    /// 某种语言名称下方的「安装方法…」，参数是所在的设置行。
    Install(Setting),
    /// 点击后复制到剪贴板的值（版本号）。
    Copy(Setting),
    /// 「快捷键」页的搜索框。
    KeySearch,
    /// 搜索框右端的「录制按键」。
    Record,
    /// 数值的减号。
    Decrease(Setting),
    /// 数值的加号。
    Increase(Setting),
    /// 一张主题缩略图（连同下方的名称）。
    Theme(ThemeChoice),
    /// 深色（`true`）或浅色主题列表里的一张缩略图（连同下方的名称）。
    NamedTheme(bool, usize),
}

/// 设置浮层的整套几何。
///
/// 绘制、悬停和点击都从这里取位置，保证三者一致。
///
/// 卡片在整个窗口中居中（它覆盖侧栏和编辑区），大小固定，换页时不变化；窗口放不下时缩小到窗口以内。
/// 左侧是分类栏，右侧上方是标题和关闭按钮，下方是这一页的各组设置项，放不下时滚动。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsLayout {
    /// 设置卡片。
    pub card: Rect,
    /// 左侧的分类栏。
    pub nav: Rect,
    /// 分类栏中的每一项及其位置。
    pub pages: Vec<(SettingsPage, Rect)>,
    /// 正在显示的页。
    pub page: SettingsPage,
    /// 右侧顶部：「设置 › 外观」和关闭按钮。
    pub header: Rect,
    /// 关闭按钮。
    pub close: Rect,
    /// 标题下方可以滚动的区域。
    pub body: Rect,
    /// 每组的底色区域。
    pub groups: Vec<Rect>,
    /// 组上方的标题：标题所在行的位置和标题文字。
    pub headings: Vec<(Rect, &'static str)>,
    /// 组内行与行之间的细线。
    pub separators: Vec<Rect>,
    /// 各设置行。
    pub rows: Vec<SettingRow>,
    /// 内容的总高度（像素），用于计算最大滚动量。
    pub content_h: f32,
}

/// 一行设置：名称在左，控件靠右。
///
/// `row` 是整行，选中时的圆角底色铺满整行。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingRow {
    /// 这一行对应的设置项。
    pub setting: Setting,
    /// 整行的矩形。
    pub row: Rect,
    /// 控件的矩形。
    pub control: Rect,
}

impl SettingsLayout {
    /// 计算设置浮层的几何。
    ///
    /// `window` 是窗口矩形，`page` 是当前页，`languages` 是格式化页列出的语言，`editing` 是
    /// 正在输入的设置项（输入格式化命令时换成更宽的输入框），`scroll` 是正文的滚动量（像素）。
    pub fn compute(
        window: Rect,
        page: SettingsPage,
        languages: &ToolLanguages,
        editing: Option<Setting>,
        scroll: f32,
    ) -> Self {
        let w = SETTINGS_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let h = SETTINGS_H.min(window.h - SETTINGS_MARGIN * 2.0).max(0.0);
        let card = Rect::new(
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + (window.h - h) / 2.0).round(),
            w,
            h,
        );

        let nav = Rect::new(card.x, card.y, SETTINGS_NAV_W.min(card.w), card.h);
        // 分类栏的第一项与右侧标题行垂直居中对齐。
        let first = card.y + (SETTINGS_HEADER - SETTINGS_NAV_ROW) / 2.0;
        let pages = SettingsPage::ALL
            .into_iter()
            .enumerate()
            .map(|(i, page)| {
                let y = first + i as f32 * (SETTINGS_NAV_ROW + 2.0);
                let row = Rect::new(nav.x + 8.0, y, (nav.w - 16.0).max(0.0), SETTINGS_NAV_ROW);
                (page, row)
            })
            .collect();

        let right = Rect::new(nav.right(), card.y, (card.w - nav.w).max(0.0), card.h);
        let header = Rect::new(right.x, right.y, right.w, SETTINGS_HEADER.min(right.h));
        let close = Rect::new(
            header.right() - 12.0 - BUTTON,
            header.y + (header.h - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );
        let body = Rect::new(
            right.x,
            header.bottom(),
            right.w,
            (right.bottom() - header.bottom()).max(0.0),
        );

        let (x, group_w) = (
            body.x + SETTINGS_PAD,
            (body.w - SETTINGS_PAD * 2.0).max(0.0),
        );
        let mut y = body.y - scroll;
        let mut groups = Vec::new();
        let mut headings = Vec::new();
        let mut separators = Vec::new();
        let mut rows = Vec::new();

        for (i, section) in page.sections(languages).into_iter().enumerate() {
            if i > 0 {
                y += SETTINGS_GROUP_GAP;
            }
            if let Some(title) = section.title {
                // 与组内名称左对齐。
                let line = Rect::new(x + SETTINGS_INSET, y, group_w, SETTINGS_TITLE);
                headings.push((line, title));
                y += SETTINGS_TITLE;
            }
            let (in_group, below): (Vec<Setting>, Vec<Setting>) = section
                .items
                .into_iter()
                .partition(|s| s.control() != SettingControl::Action);

            let top = y;
            for (n, setting) in in_group.into_iter().enumerate() {
                if n > 0 {
                    let inner = group_w - SETTINGS_INSET * 2.0;
                    separators.push(Rect::new(x + SETTINGS_INSET, y, inner.max(0.0), 1.0));
                }
                let h = row_height(setting, group_w);
                let row = Rect::new(x, y, group_w, h);
                let control = control_rect(setting, row, editing == Some(setting));
                rows.push(SettingRow {
                    setting,
                    row,
                    control,
                });
                y += h;
            }
            if y > top {
                groups.push(Rect::new(x, top, group_w, y - top));
            }

            // 组下方的文字按钮：左边与组对齐，悬停底色只覆盖按钮自身的大小。
            for setting in below {
                y += 8.0;
                let row = Rect::new(x, y, SETTINGS_LINK_W.min(group_w), SETTINGS_LINK_H);
                rows.push(SettingRow {
                    setting,
                    row,
                    control: row,
                });
                y += SETTINGS_LINK_H;
            }
        }

        SettingsLayout {
            card,
            nav,
            pages,
            page,
            header,
            close,
            body,
            groups,
            headings,
            separators,
            rows,
            content_h: y + scroll - body.y + SETTINGS_BOTTOM,
        }
    }

    /// 返回 `setting` 所在的行；没有时返回 `None`。
    pub fn row(&self, setting: Setting) -> Option<&SettingRow> {
        self.rows.iter().find(|r| r.setting == setting)
    }

    /// 返回正文的最大滚动量（像素）。
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.body.h).max(0.0)
    }

    /// 返回 `(x, y)` 处可点击的部件；落在控件之外时返回 `None`。
    pub fn part_at(&self, x: f32, y: f32) -> Option<SettingsPart> {
        if self.close.contains(x, y) {
            return Some(SettingsPart::Close);
        }

        if let Some(&(page, _)) = self.pages.iter().find(|(_, r)| r.contains(x, y)) {
            return Some(SettingsPart::Page(page));
        }

        if !self.body.contains(x, y) {
            return None;
        }

        let row = self.rows.iter().find(|r| r.control.contains(x, y))?;

        match row.setting.control() {
            SettingControl::Font if font_menu_button(row.control).contains(x, y) => {
                Some(SettingsPart::FontMenu(row.setting))
            }
            SettingControl::Font | SettingControl::Command => {
                Some(SettingsPart::Field(row.setting))
            }
            SettingControl::Switch => Some(SettingsPart::Switch(row.setting)),
            SettingControl::Action | SettingControl::Choice | SettingControl::Button => {
                Some(SettingsPart::Action(row.setting))
            }
            SettingControl::Stepper => {
                let parts = StepperParts::new(row.control);
                if parts.minus.contains(x, y) {
                    Some(SettingsPart::Decrease(row.setting))
                } else if parts.plus.contains(x, y) {
                    Some(SettingsPart::Increase(row.setting))
                } else {
                    None
                }
            }
            SettingControl::Themes => (0..THEME_CHOICES.len())
                .find(|&i| theme_segment(row.control, i).contains(x, y))
                .map(|i| SettingsPart::Theme(THEME_CHOICES[i].0)),
            SettingControl::ThemeList => {
                let dark = row.setting == Setting::DarkTheme;
                (0..crate::theme::themes(dark).len())
                    .find(|&i| theme_tile(row.control, i).contains(x, y))
                    .map(|i| SettingsPart::NamedTheme(dark, i))
            }
            SettingControl::Value => Some(SettingsPart::Copy(row.setting)),
            SettingControl::Search => None,
        }
    }
}

/// 「快捷键」页的几何：顶部固定的搜索框（右端是「录制按键」），下方按分组列出命令和键位，可以滚动。
///
/// 页面布局相同：组上方是小标题，组内每行一个命令，行与行之间有一条细线。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeysLayout {
    /// 搜索框。
    pub search: Rect,
    /// 搜索框右端的「录制按键」按钮。
    pub record: Rect,
    /// 搜索框下方可以滚动的区域。
    pub list: Rect,
    /// 各组的标题：标题所在行的位置和标题文字。
    pub headings: Vec<(Rect, &'static str)>,
    /// 各组的底色区域。
    pub groups: Vec<Rect>,
    /// 组内行与行之间的细线。
    pub separators: Vec<Rect>,
    /// 各命令行的位置和对应的键位行。
    pub rows: Vec<(Rect, &'static keymap::Row)>,
    /// 内容的总高度（像素），用于计算最大滚动量。
    pub content_h: f32,
}

impl KeysLayout {
    /// 计算「快捷键」页的几何。
    ///
    /// `body` 是设置标题下方的区域，`groups` 是筛选后的各组，`scroll` 是列表的滚动量（像素）。
    pub fn compute(
        body: Rect,
        groups: Vec<(keymap::Group, Vec<&'static keymap::Row>)>,
        scroll: f32,
    ) -> Self {
        let x = body.x + SETTINGS_PAD;
        let w = (body.w - SETTINGS_PAD * 2.0).max(0.0);
        let (search, record) = Self::header(body);
        let top = search.bottom() + 12.0;
        let list = Rect::new(body.x, top, body.w, (body.bottom() - top).max(0.0));

        let mut layout = KeysLayout {
            search,
            record,
            list,
            ..KeysLayout::default()
        };
        let mut y = list.y - scroll;
        for (i, (group, rows)) in groups
            .into_iter()
            .filter(|(_, r)| !r.is_empty())
            .enumerate()
        {
            if i > 0 {
                y += SETTINGS_GROUP_GAP;
            }
            let title = Rect::new(x + SETTINGS_INSET, y, w, SETTINGS_TITLE);
            layout.headings.push((title, group.title()));
            y += SETTINGS_TITLE;
            let block = y;
            for (n, row) in rows.into_iter().enumerate() {
                if n > 0 {
                    let inner = (w - SETTINGS_INSET * 2.0).max(0.0);
                    layout
                        .separators
                        .push(Rect::new(x + SETTINGS_INSET, y, inner, 1.0));
                }
                layout.rows.push((Rect::new(x, y, w, KEYS_ROW), row));
                y += KEYS_ROW;
            }
            layout.groups.push(Rect::new(x, block, w, y - block));
        }
        layout.content_h = y + scroll - list.y + SETTINGS_BOTTOM;
        layout
    }

    /// 返回 `body` 中搜索框和其右端「录制按键」按钮的位置。
    ///
    /// 两者固定在页面顶部，与列表内容和滚动无关；悬停、点击只需这两个矩形，不必筛选整张键位表。
    pub fn header(body: Rect) -> (Rect, Rect) {
        let x = body.x + SETTINGS_PAD;
        let w = (body.w - SETTINGS_PAD * 2.0).max(0.0);
        let search = Rect::new(x, body.y + 4.0, w, KEYS_SEARCH_H);
        let record = Rect::new(
            search.right() - 4.0 - KEYS_RECORD,
            search.y + (search.h - KEYS_RECORD) / 2.0,
            KEYS_RECORD,
            KEYS_RECORD,
        );
        (search, record)
    }

    /// 返回列表的最大滚动量（像素）。
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.list.h).max(0.0)
    }
}

/// 返回宽 `width` 的组里一行的高度。
///
/// 格式化命令行在名称下方显示所用的命令，因此总是两行高；主题一行是几排缩略图，高度按
/// 主题个数和组宽算。
fn row_height(setting: Setting, width: f32) -> f32 {
    match setting.control() {
        SettingControl::ThemeList => {
            let n = crate::theme::themes(setting == Setting::DarkTheme).len();
            let rows = n.div_ceil(THEME_COLUMNS) as f32;
            let tile = theme_tile_size((width - SETTINGS_INSET * 2.0).max(0.0));
            rows * tile.1 + (rows - 1.0) * THEME_GRID_GAP + THEME_GRID_PAD * 2.0
        }
        SettingControl::Command => SETTINGS_ROW_TALL,
        _ if setting.description().is_some() => SETTINGS_ROW_TALL,
        _ => SETTINGS_ROW,
    }
}

/// 返回字体输入框 `control` 右端下拉按钮的位置。
pub(crate) fn font_menu_button(control: Rect) -> Rect {
    const WIDTH: f32 = 26.0;
    Rect::new(control.right() - WIDTH, control.y, WIDTH, control.h)
}

/// 返回模式分段控件的宽度：各段加上段间距和底槽的内边距。
fn segmented_width() -> f32 {
    let n = SETTINGS_SEGMENT_W.len() as f32;
    let w: f32 = SETTINGS_SEGMENT_W.iter().sum();
    w + (n - 1.0) * SEGMENT_SPACING + 4.0
}

/// 返回一行中控件的位置：靠右、垂直居中；主题缩略图占满控件区的高度。
fn control_rect(setting: Setting, row: Rect, editing: bool) -> Rect {
    let (w, h) = match setting.control() {
        // 整行都可点击。
        SettingControl::Action | SettingControl::Search => return row,
        // 与「外观」页同一组的模式分段控件同宽。
        SettingControl::Font => (segmented_width(), SETTINGS_CONTROL_H),
        SettingControl::Choice if setting == Setting::Language => {
            (segmented_width(), SETTINGS_CONTROL_H)
        }
        SettingControl::Command if editing => (SETTINGS_COMMAND_W, SETTINGS_CONTROL_H),
        SettingControl::Command | SettingControl::Choice | SettingControl::Value => {
            (SETTINGS_CHOICE_W, SETTINGS_CONTROL_H)
        }
        SettingControl::Button => (SETTINGS_BUTTON_W, SETTINGS_CONTROL_H),
        SettingControl::Switch => (SETTINGS_SWITCH_W, SETTINGS_SWITCH_H),
        SettingControl::Stepper => (SETTINGS_STEP_W * 2.0 + SETTINGS_VALUE_W, SETTINGS_CONTROL_H),
        // 没有名称（组标题就是名称），缩略图排满整行。
        SettingControl::ThemeList => {
            return row.inset(SETTINGS_INSET, THEME_GRID_PAD);
        }
        SettingControl::Themes => (segmented_width(), SETTINGS_CONTROL_H),
    };
    // 控件宽度不超过行宽减去左右内边距，并给左侧名称至少留出 80 像素。
    let w = w.min((row.w - SETTINGS_INSET * 2.0 - 80.0).max(0.0));
    Rect::new(
        row.right() - SETTINGS_INSET + 4.0 - w,
        row.y + (row.h - h) / 2.0,
        w,
        h,
    )
}

/// 加减控件：左右两个按钮，中间是数值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepperParts {
    /// 左侧的减号按钮。
    pub minus: Rect,
    /// 中间显示数值的区域。
    pub value: Rect,
    /// 右侧的加号按钮。
    pub plus: Rect,
}

impl StepperParts {
    /// 根据控件的矩形 `control` 计算减号、数值和加号的位置。
    pub fn new(control: Rect) -> StepperParts {
        let minus = Rect::new(control.x, control.y, SETTINGS_STEP_W, control.h);
        let plus = Rect::new(
            control.right() - SETTINGS_STEP_W,
            control.y,
            SETTINGS_STEP_W,
            control.h,
        );
        let value = Rect::new(
            minus.right(),
            control.y,
            (plus.x - minus.right()).max(0.0),
            control.h,
        );
        StepperParts { minus, value, plus }
    }
}

/// 主题缩略图每排几张。
///
/// 浅色、深色两组用同一个格子，上下两组的缩略图对齐。
pub const THEME_COLUMNS: usize = 4;
/// 主题缩略图之间横竖的间距（逻辑像素，经验值）：留出能分清两张缩略图的缝，同时保证一行
/// 装得下 `THEME_COLUMNS` 张。
const THEME_GRID_GAP: f32 = 14.0;
/// 主题缩略图和组上下边的距离（逻辑像素）：与行内控件的 `SETTINGS_INSET` 相仿，组的上沿不贴
/// 住缩略图。
const THEME_GRID_PAD: f32 = 14.0;
/// 缩略图下面一行名称的高度（逻辑像素，含与缩略图之间的空）：够放下一行说明文字。
const THEME_NAME_H: f32 = 24.0;

/// 宽 `width` 的格子里一张主题缩略图（连同下方的名称）的宽和高；缩略图是 16∶10。
fn theme_tile_size(width: f32) -> (f32, f32) {
    let columns = THEME_COLUMNS as f32;
    let w = ((width - (columns - 1.0) * THEME_GRID_GAP) / columns).floor();
    (w, (w * 0.625).round() + THEME_NAME_H)
}

/// 返回主题格子 `control` 里第 `index` 张缩略图（从 0 起，连同下方一行名称）占的区域。
pub fn theme_tile(control: Rect, index: usize) -> Rect {
    let (w, h) = theme_tile_size(control.w);
    let col = (index % THEME_COLUMNS) as f32;
    let row = (index / THEME_COLUMNS) as f32;
    Rect::new(
        control.x + col * (w + THEME_GRID_GAP),
        control.y + row * (h + THEME_GRID_GAP),
        w,
        h,
    )
}

/// 返回缩略图本身（不含名称）的区域，位于 `tile` 的上部。
pub fn theme_thumb(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y, tile.w, tile.h - THEME_NAME_H)
}

/// 返回「模式」分段控件 `control` 里第 `index` 段的区域；底槽四周留 2 像素，相邻两段隔
/// `SEGMENT_SPACING`。
///
/// # Panics
///
/// `index` 不小于 3（即超出 `SETTINGS_SEGMENT_W` 的长度）时 panic。
pub fn theme_segment(control: Rect, index: usize) -> Rect {
    let before: f32 = SETTINGS_SEGMENT_W[..index].iter().sum();
    Rect::new(
        control.x + 2.0 + before + index as f32 * SEGMENT_SPACING,
        control.y + 2.0,
        SETTINGS_SEGMENT_W[index],
        control.h - 4.0,
    )
}
