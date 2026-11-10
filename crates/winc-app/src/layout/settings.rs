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
    /// 终端：默认 shell、面板位置、字体、字号和终端行为。
    Terminal,
    /// 模块：可选模块的开关。
    Modules,
    /// 快捷键：只列出键位，不含设置项，整页由 [`KeysLayout`] 单独布局。
    Keys,
    /// 关于：版本和应用内更新。
    About,
}

impl SettingsPage {
    /// 全部分类，顺序与分类栏从上到下的顺序一致。
    pub const ALL: [SettingsPage; 7] = [
        SettingsPage::Appearance,
        SettingsPage::Editor,
        SettingsPage::Format,
        SettingsPage::Terminal,
        SettingsPage::Modules,
        SettingsPage::Keys,
        SettingsPage::About,
    ];

    /// 返回分类栏列出的分类，顺序同 [`Self::ALL`]；终端模块关闭时（`terminal` 为 `false`）
    /// 不列出「终端」。
    pub fn listed(terminal: bool) -> impl Iterator<Item = SettingsPage> {
        Self::ALL
            .into_iter()
            .filter(move |&page| terminal || page != SettingsPage::Terminal)
    }

    /// 返回分类的标题。
    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::Appearance => t!("settings-page-appearance"),
            SettingsPage::Editor => t!("settings-page-editor"),
            SettingsPage::Format => t!("settings-page-format"),
            SettingsPage::Terminal => t!("settings-page-terminal"),
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
                        Setting::Zoom,
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
                    Setting::Outline,
                    Setting::CompactFolders,
                    Setting::PreviewOnClick,
                    Setting::DiffInline,
                    Setting::RemoteImages,
                ]),
            ],
            SettingsPage::Terminal => {
                let mut behavior = vec![
                    Setting::TerminalCopyOnSelect,
                    Setting::TerminalConfirmClose,
                    Setting::TerminalPrompt,
                    Setting::TerminalShellIntegration,
                ];
                // 只有 macOS 能区分左右 ⌥，其他平台不列出该项。
                if cfg!(target_os = "macos") {
                    behavior.push(Setting::TerminalOptionAsMeta);
                }
                vec![
                    plain(vec![Setting::TerminalShell, Setting::TerminalPosition]),
                    plain(vec![Setting::TerminalFont, Setting::TerminalSize]),
                    plain(behavior),
                ]
            }
            SettingsPage::Modules => vec![
                plain(vec![Setting::GitModule, Setting::TerminalModule]),
                plain(vec![Setting::GitConfirmPush]),
            ],
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
                plain(vec![Setting::Repository, Setting::License]),
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
    /// 文件视图底部的大纲。
    Outline,
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
    /// 预览标签：在文件树中单击文件时在预览标签中打开。
    PreviewOnClick,
    /// Markdown 预览联网加载网址的图片。
    RemoteImages,
    /// 定时检查更新。
    CheckUpdates,
    /// 「关于」页的版本号，只读。
    Version,
    /// 「关于」页的更新状态和「检查更新」按钮。
    Updates,
    /// 「关于」页的源代码仓库（GitHub）链接。
    Repository,
    /// 「关于」页的开源许可：许可证名称，链接到许可证全文。
    License,
    /// 界面字体。
    UiFont,
    /// 界面字号。
    UiSize,
    /// 界面缩放（`ui.zoom`）：整个窗口按同一比例放大或缩小。
    Zoom,
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
    /// 新建终端时运行的 shell（`terminal.shell`）。
    TerminalShell,
    /// 终端面板的位置：编辑区下方或右侧。
    TerminalPosition,
    /// 终端字体。
    TerminalFont,
    /// 终端字号。
    TerminalSize,
    /// 终端中选中文字时自动复制。
    TerminalCopyOnSelect,
    /// 关闭仍有程序运行的终端标签前确认。
    TerminalConfirmClose,
    /// 用户未设置提示符时使用 winc 的提示符。
    TerminalPrompt,
    /// 注入 shell 集成，按命令分块。
    TerminalShellIntegration,
    /// 终端中用作 Meta 键的 ⌥（只在 macOS 上列出）。
    TerminalOptionAsMeta,
    /// Git 模块的开关。
    GitModule,
    /// 发布、推送或同步分支前确认（`git.confirmPush`）。
    GitConfirmPush,
    /// 终端模块的开关。
    TerminalModule,
    /// 「快捷键」页顶部的搜索框。
    ///
    /// 该页没有设置项，选中这一项即表示停留在该页。
    KeySearch,
}

impl Setting {
    /// 名称或说明中是否含有 `query`；`query` 须已去掉首尾空白并转为小写，比较时不区分大小写。
    pub fn matches(self, query: &str) -> bool {
        let has = |text: &str| text.to_lowercase().contains(query);
        has(self.label()) || self.description().is_some_and(|text| has(&text))
    }

    /// 返回搜索 `query` 时列出的设置项，顺序与搜索结果中从上到下的顺序相同（见
    /// [`search_sections`]）；键盘上下移动按此顺序进行。
    pub fn found(query: &str, languages: &ToolLanguages, terminal: bool) -> Vec<Setting> {
        (search_sections(query, languages, terminal).into_iter())
            .flat_map(|section| section.items)
            .collect()
    }

    /// 返回列出的各页（见 [`SettingsPage::listed`]）的全部设置项，顺序为从第一页到最后一页、
    /// 每页从上到下。
    ///
    /// 键盘上下移动按此顺序进行，越过一页的首尾时切换到相邻页。
    pub fn all(languages: &ToolLanguages, terminal: bool) -> Vec<Setting> {
        SettingsPage::listed(terminal)
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
            | Setting::Zoom
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
            | Setting::PreviewOnClick
            | Setting::StickyScroll
            | Setting::WordWrap
            | Setting::Minimap
            | Setting::Outline
            | Setting::LineNumbers
            | Setting::DiffInline
            | Setting::RenderWhitespace
            | Setting::RemoteImages => SettingsPage::Editor,
            Setting::Version
            | Setting::Updates
            | Setting::CheckUpdates
            | Setting::Repository
            | Setting::License => SettingsPage::About,
            Setting::FormatOnSave
            | Setting::FixOnSave
            | Setting::SkipOnSave
            | Setting::FormatCommand(_)
            | Setting::FixCommand(_)
            | Setting::AddLanguage
            | Setting::AddFixLanguage => SettingsPage::Format,
            Setting::TerminalShell
            | Setting::TerminalPosition
            | Setting::TerminalFont
            | Setting::TerminalSize
            | Setting::TerminalCopyOnSelect
            | Setting::TerminalConfirmClose
            | Setting::TerminalPrompt
            | Setting::TerminalShellIntegration
            | Setting::TerminalOptionAsMeta => SettingsPage::Terminal,
            Setting::GitModule | Setting::GitConfirmPush | Setting::TerminalModule => {
                SettingsPage::Modules
            }
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
            Setting::Zoom => t!("settings-ui-zoom"),
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
            Setting::Outline => t!("settings-outline"),
            Setting::CompactFolders => t!("settings-compact-folders"),
            Setting::PreviewOnClick => t!("settings-preview-on-click"),
            Setting::RemoteImages => t!("settings-remote-images"),
            Setting::CheckUpdates => t!("settings-check-updates"),
            Setting::Version => t!("settings-version"),
            Setting::Repository => "GitHub",
            Setting::License => t!("settings-license"),
            Setting::Updates => t!("settings-updates"),
            Setting::FormatOnSave => t!("settings-format-on-save"),
            Setting::FixOnSave => t!("settings-fix-on-save"),
            Setting::SkipOnSave => t!("settings-skip-on-save"),
            Setting::FormatCommand(id) | Setting::FixCommand(id) => {
                winc_syntax::find(id).map_or(id, |l| l.name)
            }
            Setting::AddLanguage | Setting::AddFixLanguage => t!("settings-add-language"),
            Setting::TerminalShell => t!("settings-terminal-shell"),
            Setting::TerminalPosition => t!("settings-terminal-position"),
            Setting::TerminalFont => t!("settings-terminal-font"),
            Setting::TerminalSize => t!("settings-terminal-size"),
            Setting::TerminalCopyOnSelect => t!("settings-terminal-copy-on-select"),
            Setting::TerminalConfirmClose => t!("settings-terminal-confirm-close"),
            Setting::TerminalPrompt => t!("settings-terminal-prompt"),
            Setting::TerminalShellIntegration => t!("settings-terminal-shell-integration"),
            Setting::TerminalOptionAsMeta => t!("settings-terminal-option-as-meta"),
            Setting::GitModule => "Git",
            Setting::GitConfirmPush => t!("settings-git-confirm-push"),
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
            Setting::Outline => Some(t!("settings-outline-desc")),
            Setting::CompactFolders => Some(t!("settings-compact-folders-desc")),
            Setting::PreviewOnClick => Some(t!("settings-preview-on-click-desc")),
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
            Setting::GitConfirmPush => Some(t!("settings-git-confirm-push-desc")),
            Setting::TerminalCopyOnSelect => Some(t!("settings-terminal-copy-on-select-desc")),
            Setting::TerminalConfirmClose => Some(t!("settings-terminal-confirm-close-desc")),
            Setting::TerminalPrompt => Some(t!("settings-terminal-prompt-desc")),
            Setting::TerminalShellIntegration => {
                Some(t!("settings-terminal-shell-integration-desc"))
            }
            Setting::TerminalOptionAsMeta => Some(t!("settings-terminal-option-as-meta-desc")),
            _ => None,
        };
        text.map(Cow::Borrowed)
    }

    /// 返回设置项右侧的控件种类。
    pub fn control(self) -> SettingControl {
        match self {
            Setting::EditorFont | Setting::UiFont | Setting::TerminalFont => SettingControl::Font,
            Setting::EditorSize | Setting::UiSize | Setting::TerminalSize | Setting::TabWidth => {
                SettingControl::Stepper
            }
            Setting::Theme => SettingControl::Themes,
            Setting::LightTheme | Setting::DarkTheme => SettingControl::ThemeList,
            Setting::SystemTitleBar
            | Setting::AutoSave
            | Setting::HotExit
            | Setting::RestoreFolder
            | Setting::CompactFolders
            | Setting::PreviewOnClick
            | Setting::StickyScroll
            | Setting::WordWrap
            | Setting::Minimap
            | Setting::Outline
            | Setting::LineNumbers
            | Setting::DiffInline
            | Setting::StatusBar
            | Setting::RestoreWindow
            | Setting::RemoteImages
            | Setting::CheckUpdates
            | Setting::FormatOnSave
            | Setting::FixOnSave
            | Setting::GitModule
            | Setting::GitConfirmPush
            | Setting::TerminalModule
            | Setting::TerminalCopyOnSelect
            | Setting::TerminalConfirmClose
            | Setting::TerminalPrompt
            | Setting::TerminalShellIntegration => SettingControl::Switch,
            Setting::FormatCommand(_) | Setting::FixCommand(_) | Setting::TerminalShell => {
                SettingControl::Command
            }
            Setting::SkipOnSave
            | Setting::Language
            | Setting::Zoom
            | Setting::RenderWhitespace
            | Setting::TerminalPosition
            | Setting::TerminalOptionAsMeta => SettingControl::Choice,
            Setting::AddLanguage | Setting::AddFixLanguage => SettingControl::Action,
            Setting::KeySearch => SettingControl::Search,
            Setting::Version => SettingControl::Product,
            Setting::Repository | Setting::License => SettingControl::Link,
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
    /// 格式化工具、默认 shell：点击后弹出菜单的值；选择「自定义命令…」时换成比字体名输入框
    /// 更宽的输入框。
    Command,
    /// 组下方的文字按钮（「添加语言…」）。
    Action,
    /// 点击后弹出菜单的值，显示当前选择（如「语言」「排除的语言」）。
    Choice,
    /// 「快捷键」页的搜索框；不按设置行排列，整页单独布局（[`KeysLayout`]）。
    Search,
    /// 「关于」页顶部的产品信息：图标、名称和版本号，水平居中，没有左侧的名称；点击版本号复制
    /// （见 [`ProductParts`]）。
    Product,
    /// 右侧一段链接文字，后跟外链箭头；点击后在浏览器中打开。
    Link,
    /// 右侧的按钮（「检查更新」；找到新版本后改为下载安装、重新启动，与状态栏相同）。
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

/// 返回「Meta 键」设置中选项 `choice` 的名字。
pub fn option_as_meta_label(choice: winc_config::OptionAsMeta) -> &'static str {
    use winc_config::OptionAsMeta::{Both, Left, None, Right};
    match choice {
        Left => t!("settings-terminal-option-as-meta-left"),
        Right => t!("settings-terminal-option-as-meta-right"),
        Both => t!("settings-terminal-option-as-meta-both"),
        None => t!("settings-terminal-option-as-meta-none"),
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

/// 设置页滚动条的轨道：贴着滚动区域 `area`（正文或快捷键列表）的右边，宽 4，上下各让出 4。
pub fn settings_track(area: Rect) -> Rect {
    Rect::new(area.right() - 7.0, area.y + 4.0, 4.0, area.h - 8.0)
}

/// 设置页滚动条的滑块：`area` 为滚动区域，`scroll`、`max` 为滚动量和最大滚动量；内容放得下时
/// 返回 `None`。
///
/// 绘制、点击和拖动都用这份几何。滑块的长度和位置按轨道折算，滚到底时滑块到达轨道末端。
pub fn settings_thumb(area: Rect, scroll: f32, max: f32) -> Option<Rect> {
    if max <= 0.0 {
        return None;
    }
    let track = settings_track(area);
    let thumb = scrollbar_thumb(track, scroll, max + track.h)?;
    Some(Rect::new(track.x, thumb.y, track.w, thumb.h))
}

/// 链接文字与其后外链箭头之间的距离，加上箭头的宽度（逻辑像素）。
pub const LINK_ICON_W: f32 = 6.0 + 9.0;

/// 产品名称，与 README 的标题和 macOS 应用包的显示名一致：显示在「关于」页顶部、开始页，
/// 以及没有打开目录时的窗口标题中。命令行命令仍是小写的 `winc`。
pub const PRODUCT_NAME: &str = "Winc";

/// 「关于」页顶部产品信息的各部分：图标、名称所在的行、版本号所在的行，水平居中。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProductParts {
    /// 产品图标，边长 [`SETTINGS_LOGO`]。
    pub logo: Rect,
    /// 名称所在的行。
    pub name: Rect,
    /// 版本号所在的行。
    pub version: Rect,
}

impl ProductParts {
    /// 根据产品信息一行 `row` 计算各部分的位置（见 [`SETTINGS_PRODUCT_H`]）。
    pub fn new(row: Rect) -> ProductParts {
        let x = (row.x + (row.w - SETTINGS_LOGO) / 2.0).round();
        let logo = Rect::new(x, row.y + 20.0, SETTINGS_LOGO, SETTINGS_LOGO);
        let name = Rect::new(row.x, logo.bottom() + 10.0, row.w, 28.0);
        let version = Rect::new(row.x, name.bottom() + 2.0, row.w, 24.0);
        ProductParts {
            logo,
            name,
            version,
        }
    }

    /// 返回版本号实际占的区域：宽为文字宽 `text_w` 加左右各 10 的内边距，水平居中；悬停底色和
    /// 点击范围都取这一块。
    pub fn version_area(&self, text_w: f32) -> Rect {
        let w = (text_w + 20.0).min(self.version.w);
        let x = self.version.x + (self.version.w - w) / 2.0;
        Rect::new(x, self.version.y, w, self.version.h)
    }
}

/// 返回「关于」页顶部版本号一行的文字，例如「版本 0.1.368 (arm64)」。
pub fn version_line() -> String {
    t!("settings-version-line", version = version_label())
}

/// 返回「关于」页中链接的显示名（如「SeanAshe/Winc-plus」）；不是链接时返回 `None`。
///
/// 仓库和许可证取自 `Cargo.toml` 的 `repository` 和 `license`。
pub fn about_value(setting: Setting) -> Option<String> {
    let text = match setting {
        Setting::Repository => env!("CARGO_PKG_REPOSITORY"),
        Setting::License => env!("CARGO_PKG_LICENSE"),
        _ => return None,
    };
    let text = text.trim_start_matches("https://");
    Some(text.strip_prefix("github.com/").unwrap_or(text).to_owned())
}

/// 返回「关于」页中链接的地址；不是链接时返回 `None`。许可证链接到仓库中的许可证全文。
pub fn about_link(setting: Setting) -> Option<String> {
    let repository = env!("CARGO_PKG_REPOSITORY");
    Some(match setting {
        Setting::Repository => repository.to_owned(),
        Setting::License => format!("{repository}/blob/HEAD/LICENSE"),
        _ => return None,
    })
}

/// 返回链接实际占的区域：右缘与控件 `control` 对齐，宽为内容宽 `content` 加左右内边距，不超过
/// 控件；悬停底色和点击范围都取这一块。
pub fn value_area(control: Rect, content: f32) -> Rect {
    let w = (content + 10.0 + SETTINGS_GLYPH_PAD).min(control.w);
    Rect::new(control.right() - w, control.y, w, control.h)
}

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
    /// 分类栏顶部的搜索框。
    SearchField,
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
    /// 点击后在浏览器中打开的链接。
    Link(Setting),
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

/// 计算设置浮层布局所需的界面状态。
#[derive(Debug, Clone, Copy)]
pub struct SettingsView<'a> {
    /// 当前页。
    pub page: SettingsPage,
    /// 分类栏顶部搜索框中的文字；不为空白时正文换成搜索结果。
    pub query: &'a str,
    /// 格式化页列出的语言。
    pub languages: &'a ToolLanguages,
    /// 分类栏中「终端」一项的显示比例（0 到 1，开关终端模块时渐变，见
    /// [`SettingsPage::listed`]）。
    pub terminal: f32,
    /// 正在输入的设置项：输入格式化命令时换成更宽的输入框。
    pub editing: Option<Setting>,
    /// 正文的滚动量（像素）。
    pub scroll: f32,
}

/// 返回搜索 `query` 时正文列出的各组：各页中名称或说明含有 `query` 的设置项
/// （[`Setting::matches`]），按页和页内的顺序排列。
///
/// 有标题的组（主题、格式化工具）保留原标题，其余各组合并为一组，以页名为标题；「快捷键」页和
/// 组下方的文字按钮不参与搜索。`terminal` 为终端模块是否开启（见 [`SettingsPage::listed`]）。
pub fn search_sections(query: &str, languages: &ToolLanguages, terminal: bool) -> Vec<Section> {
    let query = query.trim().to_lowercase();
    let mut found = Vec::new();
    for page in SettingsPage::listed(terminal).filter(|&page| page != SettingsPage::Keys) {
        let mut plain = Vec::new();
        let mut titled = Vec::new();
        for section in page.sections(languages) {
            let items: Vec<Setting> = (section.items.into_iter())
                .filter(|s| s.control() != SettingControl::Action && s.matches(&query))
                .collect();
            match section.title {
                _ if items.is_empty() => {}
                Some(title) => titled.push(Section {
                    title: Some(title),
                    items,
                }),
                None => plain.extend(items),
            }
        }
        if !plain.is_empty() {
            found.push(Section {
                title: Some(page.title()),
                items: plain,
            });
        }
        found.extend(titled);
    }
    found
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
    /// 分类栏顶部的搜索框。
    pub search: Rect,
    /// 分类栏中的每一项及其位置。
    pub pages: Vec<(SettingsPage, Rect)>,
    /// 正在显示的页。
    pub page: SettingsPage,
    /// 是否在显示搜索结果（搜索框不为空白）：此时正文是 [`search_sections`] 的各组，分类栏中
    /// 不标出当前页。
    pub searching: bool,
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
    /// `window` 是窗口矩形，`view` 是当前的界面状态，`measure` 返回一段说明文字的宽度（逻辑
    /// 像素），说明放不下一行时折行，该行随之加高。
    pub fn compute(
        window: Rect,
        view: &SettingsView<'_>,
        measure: &mut dyn FnMut(&str) -> f32,
    ) -> Self {
        let &SettingsView {
            page,
            query,
            languages,
            terminal,
            editing,
            scroll,
        } = view;
        let searching = !query.trim().is_empty();
        let w = SETTINGS_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let h = SETTINGS_H.min(window.h - SETTINGS_MARGIN * 2.0).max(0.0);
        let card = Rect::new(
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + (window.h - h) / 2.0).round(),
            w,
            h,
        );

        let nav = Rect::new(card.x, card.y, SETTINGS_NAV_W.min(card.w), card.h);
        // 搜索框与右侧标题行垂直居中对齐，各页列在其下。「终端」一项的高度按显示比例收放，
        // 下方各项随之滑动。
        let search = Rect::new(
            nav.x + 8.0,
            card.y + (SETTINGS_HEADER - SETTINGS_CONTROL_H) / 2.0,
            (nav.w - 16.0).max(0.0),
            SETTINGS_CONTROL_H,
        );
        let mut y = search.bottom() + 8.0;
        let pages = SettingsPage::ALL
            .into_iter()
            .filter_map(|page| {
                let share = if page == SettingsPage::Terminal {
                    terminal
                } else {
                    1.0
                };
                let h = SETTINGS_NAV_ROW * share;
                let row = Rect::new(nav.x + 8.0, y, (nav.w - 16.0).max(0.0), h);
                y += (SETTINGS_NAV_ROW + 2.0) * share;
                (share > 0.0).then_some((page, row))
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

        let sections = if searching {
            search_sections(query, languages, terminal > 0.0)
        } else {
            page.sections(languages)
        };
        for (i, section) in sections.into_iter().enumerate() {
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
                let editing = editing == Some(setting);
                let h = row_height(setting, Rect::new(x, y, group_w, 0.0), editing, measure);
                let row = Rect::new(x, y, group_w, h);
                let control = control_rect(setting, row, editing);
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
            search,
            pages,
            page,
            searching,
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
        if self.search.contains(x, y) {
            return Some(SettingsPart::SearchField);
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
            SettingControl::Product => (ProductParts::new(row.row).version.contains(x, y))
                .then_some(SettingsPart::Copy(row.setting)),
            SettingControl::Link => Some(SettingsPart::Link(row.setting)),
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
/// 格式化命令行在名称下方显示所用的命令，因此总是两行高；有说明的行按说明折成的行数加高
/// （见 [`description_lines`]）；主题一行是几排缩略图，高度按主题个数和组宽算。
///
/// `row` 给出该行的横向位置和宽度，`editing` 为该行是否正在输入，`measure` 量说明文字的宽度。
fn row_height(
    setting: Setting,
    row: Rect,
    editing: bool,
    measure: &mut dyn FnMut(&str) -> f32,
) -> f32 {
    match setting.control() {
        SettingControl::ThemeList => {
            let n = crate::theme::themes(setting == Setting::DarkTheme).len();
            let rows = n.div_ceil(THEME_COLUMNS) as f32;
            let tile = theme_tile_size((row.w - SETTINGS_INSET * 2.0).max(0.0));
            rows * tile.1 + (rows - 1.0) * THEME_GRID_GAP + THEME_GRID_PAD * 2.0
        }
        SettingControl::Command if setting != Setting::TerminalShell => SETTINGS_ROW_TALL,
        SettingControl::Product => SETTINGS_PRODUCT_H,
        _ => match setting.description() {
            Some(text) => {
                let room = description_room(row, control_rect(setting, row, editing));
                let lines = description_lines(setting, &text, room, measure).len();
                SETTINGS_ROW_TALL + lines.saturating_sub(1) as f32 * SETTINGS_DESC_LINE
            }
            None => SETTINGS_ROW,
        },
    }
}

/// 返回一行中说明文字可用的宽度（逻辑像素）：从名称的左缘到控件左侧，留出 16 的间隔。
pub fn description_room(row: Rect, control: Rect) -> f32 {
    (control.x - row.x - SETTINGS_INSET - 16.0).max(0.0)
}

/// 把 `setting` 的说明 `text` 按宽度 `room` 折成多行，最多 [`SETTINGS_DESC_LINES`] 行；
/// 放不下时最后一行是剩下的全部文字，由绘制时在末尾截短。
///
/// 「软件更新」一行下方是单行的更新状态，不折行。
pub fn description_lines<'a>(
    setting: Setting,
    text: &'a str,
    room: f32,
    measure: &mut dyn FnMut(&str) -> f32,
) -> Vec<&'a str> {
    if setting == Setting::Updates {
        return vec![text];
    }
    let mut lines = crate::layout::wrap_text(text, room, measure);
    if lines.len() > SETTINGS_DESC_LINES {
        // 各行都是 `text` 的子串：最后一行从它在 `text` 中的位置一直取到末尾。
        let start = lines[SETTINGS_DESC_LINES - 1].as_ptr() as usize - text.as_ptr() as usize;
        lines.truncate(SETTINGS_DESC_LINES - 1);
        lines.push(&text[start..]);
    }
    lines
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
        SettingControl::Command | SettingControl::Choice | SettingControl::Link => {
            (SETTINGS_CHOICE_W, SETTINGS_CONTROL_H)
        }
        // 产品信息铺满整行。
        SettingControl::Product => return row,
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
    // 可见内容的右缘对齐组内分隔线的右端：文字、图标类控件的底色向外多出内边距。
    let pad = match setting.control() {
        SettingControl::Choice | SettingControl::Link | SettingControl::Stepper => {
            SETTINGS_GLYPH_PAD
        }
        SettingControl::Font | SettingControl::Command if !editing => SETTINGS_GLYPH_PAD,
        _ => 0.0,
    };
    Rect::new(
        row.right() - SETTINGS_INSET + pad - w,
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
