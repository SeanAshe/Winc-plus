//! 设置的分类：各页的标题和每页分组列出的设置项。

use super::*;

/// 设置的一个分类，对应左侧分类栏中的一项和右侧的一页。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsPage {
    /// 通用：界面语言和缩放、启动、保存与退出、更新。
    #[default]
    General,
    /// 外观：主题、界面字体、界面的组成部分和背景图片。
    Appearance,
    /// 编辑器：编辑区的字体和显示的内容、新建文件的语言、Vim 模式。
    Editor,
    /// 文件：标签页、目录和侧栏中的文件视图。
    Files,
    /// Markdown：预览、写作辅助和 LaTeX 程序。
    Markdown,
    /// 格式化：保存时的格式化和修复，以及各语言的工具命令。
    Format,
    /// Git：Git 模块的选项；模块关闭时不列出。
    Git,
    /// 终端：默认 shell、面板位置、字体、字号、字重和终端行为。
    Terminal,
    /// 模块：可选模块的开关。
    Modules,
    /// 快捷键：只列出键位，不含设置项，整页由 [`KeysLayout`] 单独布局。
    Keys,
    /// 关于：版本和应用内更新。
    About,
}

/// 各可选模块是否开启：决定分类栏列出哪些页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modules {
    pub git: bool,
    pub terminal: bool,
}

impl SettingsPage {
    /// 全部分类，顺序与分类栏从上到下的顺序一致。
    pub const ALL: [SettingsPage; 11] = [
        SettingsPage::General,
        SettingsPage::Appearance,
        SettingsPage::Editor,
        SettingsPage::Files,
        SettingsPage::Markdown,
        SettingsPage::Format,
        SettingsPage::Git,
        SettingsPage::Terminal,
        SettingsPage::Modules,
        SettingsPage::Keys,
        SettingsPage::About,
    ];

    /// 返回分类栏列出的分类，顺序同 [`Self::ALL`]；模块关闭时不列出它的那一页。
    pub fn listed(modules: Modules) -> impl Iterator<Item = SettingsPage> {
        Self::ALL.into_iter().filter(move |&page| match page {
            SettingsPage::Git => modules.git,
            SettingsPage::Terminal => modules.terminal,
            _ => true,
        })
    }

    /// 返回分类的标题。
    pub fn title(self) -> &'static str {
        match self {
            SettingsPage::General => t!("settings-page-general"),
            SettingsPage::Appearance => t!("settings-page-appearance"),
            SettingsPage::Editor => t!("settings-page-editor"),
            SettingsPage::Files => t!("settings-page-files"),
            SettingsPage::Markdown => t!("settings-page-markdown"),
            SettingsPage::Format => t!("settings-page-format"),
            SettingsPage::Git => t!("settings-page-git"),
            SettingsPage::Terminal => t!("settings-page-terminal"),
            SettingsPage::Modules => t!("settings-page-modules"),
            SettingsPage::Keys => t!("settings-page-keys"),
            SettingsPage::About => t!("settings-page-about"),
        }
    }

    /// 返回这一页的各组设置项，组内的项从上到下排列。
    ///
    /// 格式化页中每种语言占一行，Vim 分组下每个冲突的键占一行，由 `listed` 给出。
    pub fn sections(self, listed: &Listed) -> Vec<Section> {
        use Setting as S;
        let plain = |items: Vec<Setting>| Section { title: None, items };
        let group = |title: &'static str, items: Vec<Setting>| Section {
            title: Some(title),
            items,
        };
        match self {
            SettingsPage::General => {
                // 「关闭窗口后退出」只在 macOS 上列出：其他平台上关闭最后一个窗口总是退出。
                let mut saving = vec![S::AutoSave, S::HotExit, S::ConfirmClose];
                if cfg!(target_os = "macos") {
                    saving.push(S::QuitOnLastWindowClose);
                }
                let mut sections = vec![
                    group(t!("settings-group-language"), vec![S::Language]),
                    group(
                        t!("settings-group-launch"),
                        vec![S::RestoreFolder, S::NewFileOnStartup, S::RestoreWindow],
                    ),
                    group(t!("settings-group-saving"), saving),
                    group(
                        t!("settings-group-network"),
                        vec![S::CheckUpdates, S::SystemProxy],
                    ),
                ];
                // 只在 Windows 上列出；各文件类型的登记由安装程序写在当前用户之下时才能改。
                if crate::shell_integration::available() {
                    let mut items = vec![S::ExplorerMenu];
                    if crate::shell_integration::types_available() {
                        items.push(S::FileAssociations);
                    }
                    sections.push(group(t!("settings-group-integration"), items));
                }
                sections.push(group(
                    t!("settings-group-settings-file"),
                    vec![S::ExportSettings, S::ImportSettings],
                ));
                sections
            }
            SettingsPage::Appearance => {
                // 平滑滚动只在 macOS 上列出：其他平台上滚轮原本就一格一格立即滚动。系统标题栏只在
                // 其他平台列出：macOS 上标题栏总是和内容融为一体。
                let mut interface = vec![S::Zoom, S::StatusBar, S::Animations];
                if cfg!(target_os = "macos") {
                    interface.push(S::SmoothScrolling);
                } else {
                    interface.push(S::SystemTitleBar);
                }
                vec![
                    group(t!("settings-group-theme"), vec![S::Theme]),
                    group(t!("settings-light-theme"), vec![S::LightTheme]),
                    group(t!("settings-dark-theme"), vec![S::DarkTheme]),
                    group(
                        t!("settings-group-font"),
                        vec![S::UiFont, S::UiSize, S::FontWeight],
                    ),
                    group(t!("settings-group-interface"), interface),
                    group(
                        t!("settings-group-background"),
                        vec![S::BackgroundImage, S::BackgroundFit, S::BackgroundOpacity],
                    ),
                ]
            }
            SettingsPage::Editor => vec![
                group(
                    t!("settings-group-font-indent"),
                    vec![S::EditorFont, S::EditorSize, S::TabWidth],
                ),
                group(
                    t!("settings-group-display"),
                    vec![
                        S::LineNumbers,
                        S::WordWrap,
                        S::AlignColumns,
                        S::Minimap,
                        S::StickyScroll,
                        S::SmoothCaret,
                        S::RenderWhitespace,
                        S::WholeLines,
                        S::FullTabNames,
                    ],
                ),
                group(t!("settings-group-behavior"), vec![S::DragAndDrop]),
                group(
                    t!("settings-group-new-files"),
                    vec![S::DefaultLanguage, S::DetectLanguageOnPaste],
                ),
                group(
                    t!("settings-group-vim"),
                    vec![
                        S::VimMode,
                        S::VimClipboard,
                        S::VimRelativeNumber,
                        S::VimYankHighlight,
                    ],
                ),
                // Vim 模式开着、有冲突的键时列出。
                group(
                    t!("settings-group-vim-keys"),
                    listed.vim_keys.iter().map(|&c| S::VimKey(c)).collect(),
                ),
            ]
            .into_iter()
            .filter(|section| !section.items.is_empty())
            .collect(),
            SettingsPage::Files => vec![
                group(
                    t!("settings-group-opening"),
                    vec![S::PreviewOnClick, S::DropFolder],
                ),
                group(
                    t!("settings-group-sidebar"),
                    vec![S::CompactFolders, S::AutoReveal, S::Outline, S::FindInFiles],
                ),
            ],
            SettingsPage::Markdown => vec![
                group(
                    t!("settings-group-font"),
                    vec![S::MarkdownFont, S::MarkdownSize],
                ),
                group(
                    t!("settings-group-preview"),
                    vec![S::TextWidth, S::MarkdownLineNumbers, S::RemoteImages],
                ),
                group(
                    t!("settings-group-writing"),
                    vec![
                        S::MarkdownShortcuts,
                        S::MarkdownFocus,
                        S::MarkdownTypewriter,
                        S::MarkdownSelectionToolbar,
                    ],
                ),
                group(t!("settings-group-latex"), vec![S::LatexPath]),
            ],
            SettingsPage::Terminal => {
                let mut behavior = vec![
                    S::TerminalCopyOnSelect,
                    S::TerminalConfirmClose,
                    S::TerminalApplicationPriority,
                ];
                // 其他平台的 Alt 一律视为 Meta，不列出该项。
                if cfg!(target_os = "macos") {
                    behavior.push(S::TerminalOptionAsMeta);
                }
                vec![
                    group(
                        t!("settings-group-shell"),
                        vec![
                            S::TerminalShell,
                            S::TerminalPrompt,
                            S::TerminalShellIntegration,
                        ],
                    ),
                    group(
                        t!("settings-group-display"),
                        vec![
                            S::TerminalPosition,
                            S::TerminalTabPosition,
                            S::TerminalFont,
                            S::TerminalSize,
                            S::TerminalFontWeight,
                        ],
                    ),
                    group(t!("settings-group-behavior"), behavior),
                ]
            }
            SettingsPage::Git => vec![
                group(
                    t!("settings-group-general"),
                    vec![S::GitPath, S::GitOpenWorktree, S::GitScanDepth],
                ),
                group(
                    t!("settings-group-commit"),
                    vec![S::GitStagingArea, S::GitTree],
                ),
                group(
                    t!("settings-group-diff"),
                    vec![
                        S::GitCombinedDiff,
                        S::DiffInline,
                        S::DiffCollapse,
                        S::GitInlineBlame,
                    ],
                ),
                group(
                    t!("settings-group-remote"),
                    vec![
                        S::GitPullMode,
                        S::GitConfirmPush,
                        S::GitRememberCredentials,
                        S::GitAutoFetch,
                    ],
                ),
            ],
            SettingsPage::Modules => vec![group(
                t!("settings-page-modules"),
                vec![S::GitModule, S::TerminalModule, S::RemoteModule],
            )],
            SettingsPage::Format => {
                let tools = |ids: &[&'static str], make: fn(&'static str) -> Setting, add| {
                    ids.iter().map(|&id| make(id)).chain([add]).collect()
                };
                vec![
                    group(
                        t!("settings-group-on-save"),
                        vec![S::FormatOnSave, S::FixOnSave, S::SkipOnSave],
                    ),
                    // 没有列出的语言时只有「添加语言…」，直接跟在组标题下面。
                    group(
                        t!("settings-section-format-tools"),
                        tools(&listed.format, S::FormatCommand, S::AddLanguage),
                    ),
                    group(
                        t!("settings-section-fix-tools"),
                        tools(&listed.fix, S::FixCommand, S::AddFixLanguage),
                    ),
                ]
            }
            SettingsPage::Keys => Vec::new(),
            SettingsPage::About => vec![
                plain(vec![S::Version]),
                group(t!("settings-group-updates"), vec![S::Updates]),
                group(
                    t!("settings-group-links"),
                    vec![
                        S::ReleaseNotes,
                        S::Website,
                        S::Repository,
                        S::Community,
                        S::License,
                    ],
                ),
            ],
        }
    }

    /// 返回这一页的第一项；切换到这一页时选中第一项。
    pub fn first(self) -> Setting {
        match self {
            SettingsPage::Keys => Setting::KeySearch,
            page => page.sections(&Listed::default())[0].items[0],
        }
    }
}
