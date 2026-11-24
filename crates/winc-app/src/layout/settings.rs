//! 设置卡片的几何：分类、各项设置的行和控件、快捷键一页。

use super::*;

use std::borrow::Cow;

use winc_i18n::t;

mod about;
mod choices;
mod controls;
mod keys;
mod pages;

pub use about::*;
pub use choices::*;
pub use controls::*;
pub use keys::*;
pub use pages::*;

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
    Theme,
    /// 浅色外观时用的主题。
    LightTheme,
    /// 深色外观时用的主题。
    DarkTheme,
    /// 界面语言。
    #[default]
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
    /// 关闭窗口或退出前先确认。
    ConfirmClose,
    /// 启动时打开上次退出时打开的目录和标签。
    RestoreFolder,
    /// 启动时没有可恢复的内容时新建文件（`editor.newFileOnStartup`）。
    NewFileOnStartup,
    /// 新建文件的默认语言（`editor.defaultLanguage`）。
    DefaultLanguage,
    /// 粘贴时按内容识别语言（`editor.detectLanguageOnPaste`）。
    DetectLanguageOnPaste,
    /// 粘滞滚动：滚动时在编辑区顶部钉住当前所在的函数、类和标题。
    StickyScroll,
    /// 光标移动时滑到新位置的方式。
    SmoothCaret,
    /// Vim 模式。
    VimMode,
    /// Vim 模式下无名寄存器与系统剪贴板的同步方式。
    VimClipboard,
    /// Vim 模式下的相对行号。
    VimRelativeNumber,
    /// Vim 模式下复制之后高亮被复制的范围。
    VimYankHighlight,
    /// 自动换行。
    WordWrap,
    /// 超长行显示全部内容（`wholeLines`）。
    WholeLines,
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
    /// 播放界面的过渡动画。
    Animations,
    /// 鼠标滚轮平滑滚动（`editor.smoothScrolling`）。
    SmoothScrolling,
    /// 窗口的背景图片（`ui.backgroundImage`）。
    BackgroundImage,
    /// 背景图片的显示方式（`ui.backgroundFit`）。
    BackgroundFit,
    /// 背景图片的不透明度（`ui.backgroundOpacity`）。
    BackgroundOpacity,
    /// 文件树把连续的单子目录合并显示为一行。
    CompactFolders,
    /// 预览标签：在文件树中单击文件时在预览标签中打开。
    PreviewOnClick,
    /// Markdown 预览联网加载网址的图片。
    RemoteImages,
    /// Markdown 预览正文的最大宽度（`markdown.textWidth`）。
    TextWidth,
    /// Markdown 预览正文的字体（`markdown.font`）。
    MarkdownFont,
    /// Markdown 预览正文的字号（`markdown.size`）。
    MarkdownSize,
    /// Markdown 预览显示行号（`markdown.lineNumbers`）。
    MarkdownLineNumbers,
    /// Markdown 文件的源代码也使用写作快捷键（`markdown.shortcuts`）。
    MarkdownShortcuts,
    /// 编辑 Markdown 时淡化光标所在段落以外的文字（`markdown.focusMode`）。
    MarkdownFocus,
    /// 编辑 Markdown 时光标所在的行停在中间（`markdown.typewriterMode`）。
    MarkdownTypewriter,
    /// 选中文字后显示格式工具栏（`markdown.selectionToolbar`）。
    MarkdownSelectionToolbar,
    /// 编译 LaTeX 的程序（`latex.path`）。
    LatexPath,
    /// 定时检查更新。
    CheckUpdates,
    /// 联网的命令跟随系统的代理设置（`network.systemProxy`）。
    SystemProxy,
    /// 「关于」页的版本号，只读。
    Version,
    /// 「关于」页的更新状态和「检查更新」按钮。
    Updates,
    /// 「关于」页的「更新日志」：在阅读标签中打开，列出各版的更改。
    ReleaseNotes,
    /// 「关于」页的官网链接。
    Website,
    /// 「关于」页的源代码仓库（GitHub）链接。
    Repository,
    /// 「关于」页的社区群组（Telegram）链接。
    Community,
    /// 「关于」页的开源许可：许可证名称，链接到许可证全文。
    License,
    /// 界面字体。
    UiFont,
    /// 界面字号。
    UiSize,
    /// 全部文字的字重。
    FontWeight,
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
    /// 终端标签的位置：顶部或右侧。
    TerminalTabPosition,
    /// 终端字体。
    TerminalFont,
    /// 终端字号。
    TerminalSize,
    /// 终端字重（`terminal.fontWeight`）。
    TerminalFontWeight,
    /// 终端中选中文字时自动复制。
    TerminalCopyOnSelect,
    /// 关闭仍有程序运行的终端标签前确认。
    TerminalConfirmClose,
    /// 用户未设置提示符时使用 winc 的提示符。
    TerminalPrompt,
    /// 注入 shell 集成，按命令分块。
    TerminalShellIntegration,
    /// 暂停内容增强的快捷键，保留终端管理和复制、粘贴。
    TerminalApplicationPriority,
    /// 终端中用作 Meta 键的 ⌥（只在 macOS 上列出）。
    TerminalOptionAsMeta,
    /// Git 模块的开关。
    GitModule,
    /// 发布、推送或同步分支前确认（`git.confirmPush`）。
    GitConfirmPush,
    /// 允许记住 HTTP(S) 密码（`git.rememberCredentials`）。
    GitRememberCredentials,
    /// 光标所在行末尾的追溯提示（`git.inlineBlame`）。
    GitInlineBlame,
    /// 定时在后台获取（`git.autoFetch`）。
    GitAutoFetch,
    /// 提交视图的更改列表按目录排成树；记在状态文件里，与「⋯」菜单中的是同一个开关。
    GitTree,
    /// 提交视图显示暂存区（`git.stagingArea`）；关闭时用勾选决定提交的内容。
    GitStagingArea,
    /// 点击工作树时在哪个窗口打开它（`git.openWorktree`）。
    GitOpenWorktree,
    /// 拉取、同步时合并还是变基（`git.pull`）。
    GitPullMode,
    /// 把目录拖进已打开目录的窗口时怎样处理（`dropFolder`）。
    DropFolder,
    /// 「在文件中查找」打开在侧栏还是对话框中。
    FindInFiles,
    /// `git` 程序的路径（`git.path`）；未设置时在 `PATH` 里查找。
    GitPath,
    /// 终端模块的开关。
    TerminalModule,
    /// 远程模块的开关。
    RemoteModule,
    /// 资源管理器的右键菜单（Windows），记在注册表里。
    ExplorerMenu,
    /// 把 Winc 列为各文件类型的候选程序（Windows 上由安装程序装在当前用户之下时）。
    FileAssociations,
    /// 键位方案；只在搜索结果中作为一行列出，「快捷键」页顶部另有一份。
    Keymap,
    /// 搜索结果中名称或按键与搜索内容相符的命令：点击后在「快捷键」页中查看。
    KeyMatches,
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
    pub fn found(query: &str, languages: &ToolLanguages, modules: Modules) -> Vec<Setting> {
        (search_sections(query, languages, modules).into_iter())
            .flat_map(|section| section.items)
            .collect()
    }

    /// 返回列出的各页（见 [`SettingsPage::listed`]）的全部设置项，顺序为从第一页到最后一页、
    /// 每页从上到下。
    ///
    /// 键盘上下移动按此顺序进行，越过一页的首尾时切换到相邻页。
    pub fn all(languages: &ToolLanguages, modules: Modules) -> Vec<Setting> {
        SettingsPage::listed(modules)
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
        use Setting as S;
        match self {
            S::Language | S::RestoreFolder | S::NewFileOnStartup | S::RestoreWindow => {
                SettingsPage::General
            }
            S::AutoSave | S::HotExit | S::ConfirmClose | S::CheckUpdates | S::SystemProxy => {
                SettingsPage::General
            }
            S::Theme | S::LightTheme | S::DarkTheme | S::UiFont | S::UiSize | S::FontWeight => {
                SettingsPage::Appearance
            }
            S::Zoom | S::StatusBar | S::Animations | S::SmoothScrolling | S::SystemTitleBar => {
                SettingsPage::Appearance
            }
            S::BackgroundImage | S::BackgroundFit | S::BackgroundOpacity => {
                SettingsPage::Appearance
            }
            S::EditorFont | S::EditorSize | S::TabWidth | S::LineNumbers | S::WordWrap => {
                SettingsPage::Editor
            }
            S::Minimap | S::StickyScroll | S::RenderWhitespace | S::WholeLines => {
                SettingsPage::Editor
            }
            S::SmoothCaret => SettingsPage::Editor,
            S::DefaultLanguage | S::DetectLanguageOnPaste | S::VimMode | S::VimClipboard => {
                SettingsPage::Editor
            }
            S::VimRelativeNumber | S::VimYankHighlight => SettingsPage::Editor,
            S::PreviewOnClick | S::DropFolder | S::CompactFolders | S::Outline | S::FindInFiles => {
                SettingsPage::Files
            }
            S::MarkdownFont | S::MarkdownSize | S::TextWidth | S::MarkdownLineNumbers => {
                SettingsPage::Markdown
            }
            S::RemoteImages | S::MarkdownShortcuts | S::MarkdownFocus | S::MarkdownTypewriter => {
                SettingsPage::Markdown
            }
            S::MarkdownSelectionToolbar | S::LatexPath => SettingsPage::Markdown,
            S::FormatOnSave | S::FixOnSave | S::SkipOnSave | S::FormatCommand(_) => {
                SettingsPage::Format
            }
            S::FixCommand(_) | S::AddLanguage | S::AddFixLanguage => SettingsPage::Format,
            S::TerminalShell | S::TerminalPosition | S::TerminalTabPosition | S::TerminalFont => {
                SettingsPage::Terminal
            }
            S::TerminalSize | S::TerminalFontWeight | S::TerminalCopyOnSelect => {
                SettingsPage::Terminal
            }
            S::TerminalConfirmClose | S::TerminalPrompt | S::TerminalShellIntegration => {
                SettingsPage::Terminal
            }
            S::TerminalApplicationPriority | S::TerminalOptionAsMeta => SettingsPage::Terminal,
            S::GitPath | S::GitOpenWorktree | S::GitStagingArea | S::GitTree | S::DiffInline => {
                SettingsPage::Git
            }
            S::GitInlineBlame | S::GitPullMode | S::GitConfirmPush | S::GitAutoFetch => {
                SettingsPage::Git
            }
            S::GitRememberCredentials => SettingsPage::Git,
            S::ExplorerMenu | S::FileAssociations => SettingsPage::General,
            S::GitModule | S::TerminalModule | S::RemoteModule => SettingsPage::Modules,
            S::Version | S::Updates | S::ReleaseNotes | S::Website | S::Repository => {
                SettingsPage::About
            }
            S::Community | S::License => SettingsPage::About,
            S::Keymap | S::KeyMatches | S::KeySearch => SettingsPage::Keys,
        }
    }

    /// 返回该设置项在配置文件中的名字（节名加键名，如 `editor.size`）；不是配置文件中单独
    /// 一项的（各语言的工具命令、记在状态文件里的、按钮和链接）为 `None`。
    pub fn key(self) -> Option<&'static str> {
        use Setting as S;
        Some(match self {
            S::Theme => "theme",
            S::LightTheme => "lightTheme",
            S::DarkTheme => "darkTheme",
            S::Language => "language",
            S::EditorFont => "editor.font",
            S::EditorSize => "editor.size",
            S::TabWidth => "editor.tabWidth",
            S::AutoSave => "editor.autoSave",
            S::HotExit => "editor.hotExit",
            S::ConfirmClose => "editor.confirmClose",
            S::RestoreFolder => "editor.restoreFolder",
            S::NewFileOnStartup => "editor.newFileOnStartup",
            S::DefaultLanguage => "editor.defaultLanguage",
            S::DetectLanguageOnPaste => "editor.detectLanguageOnPaste",
            S::StickyScroll => "editor.stickyScroll",
            S::SmoothCaret => "editor.smoothCaretMovement",
            S::WordWrap => "editor.wordWrap",
            S::WholeLines => "editor.wholeLines",
            S::Minimap => "editor.minimap",
            S::Outline => "editor.outline",
            S::LineNumbers => "editor.lineNumbers",
            S::DiffInline => "editor.diffInline",
            S::RenderWhitespace => "editor.renderWhitespace",
            S::StatusBar => "editor.statusBar",
            S::RestoreWindow => "editor.restoreWindow",
            S::SmoothScrolling => "editor.smoothScrolling",
            S::DropFolder => "editor.dropFolder",
            S::FindInFiles => "editor.findInFiles",
            S::VimMode => "vim.enabled",
            S::VimClipboard => "vim.clipboard",
            S::VimRelativeNumber => "vim.relativeNumber",
            S::VimYankHighlight => "vim.yankHighlight",
            S::UiFont => "ui.font",
            S::UiSize => "ui.size",
            S::FontWeight => "ui.fontWeight",
            S::Zoom => "ui.zoom",
            S::Animations => "ui.animations",
            S::SystemTitleBar => "ui.titleBarStyle",
            S::BackgroundImage => "ui.backgroundImage",
            S::BackgroundFit => "ui.backgroundFit",
            S::BackgroundOpacity => "ui.backgroundOpacity",
            S::CompactFolders => "tree.compactFolders",
            S::PreviewOnClick => "tree.previewOnClick",
            S::RemoteImages => "markdown.remoteImages",
            S::TextWidth => "markdown.textWidth",
            S::MarkdownFont => "markdown.font",
            S::MarkdownSize => "markdown.size",
            S::MarkdownLineNumbers => "markdown.lineNumbers",
            S::MarkdownShortcuts => "markdown.shortcuts",
            S::MarkdownFocus => "markdown.focusMode",
            S::MarkdownTypewriter => "markdown.typewriterMode",
            S::MarkdownSelectionToolbar => "markdown.selectionToolbar",
            S::LatexPath => "latex.path",
            S::CheckUpdates => "update.autoCheck",
            S::SystemProxy => "network.systemProxy",
            S::FormatOnSave => "format.onSave",
            S::FixOnSave => "format.fixOnSave",
            S::SkipOnSave => "format.onSaveExclude",
            S::TerminalShell => "terminal.shell",
            S::TerminalPosition => "terminal.position",
            S::TerminalTabPosition => "terminal.verticalTabs",
            S::TerminalFont => "terminal.font",
            S::TerminalSize => "terminal.size",
            S::TerminalFontWeight => "terminal.fontWeight",
            S::TerminalCopyOnSelect => "terminal.copyOnSelect",
            S::TerminalConfirmClose => "terminal.confirmClose",
            S::TerminalPrompt => "terminal.prompt",
            S::TerminalShellIntegration => "terminal.shellIntegration",
            S::TerminalApplicationPriority => "terminal.applicationPriority",
            S::TerminalOptionAsMeta => "terminal.optionAsMeta",
            S::GitModule => "git.enabled",
            S::GitConfirmPush => "git.confirmPush",
            S::GitRememberCredentials => "git.rememberCredentials",
            S::GitInlineBlame => "git.inlineBlame",
            S::GitAutoFetch => "git.autoFetch",
            S::GitStagingArea => "git.stagingArea",
            S::GitOpenWorktree => "git.openWorktree",
            S::GitPullMode => "git.pull",
            S::GitPath => "git.path",
            S::TerminalModule => "terminal.enabled",
            S::RemoteModule => "remote.enabled",
            S::GitTree | S::FormatCommand(_) | S::FixCommand(_) | S::AddLanguage => return None,
            S::AddFixLanguage | S::Version | S::Updates | S::ReleaseNotes => return None,
            S::Website | S::Repository | S::Community | S::License | S::KeySearch => return None,
            S::Keymap | S::KeyMatches => return None,
            S::ExplorerMenu | S::FileAssociations => return None,
        })
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
            Setting::FontWeight => t!("settings-font-weight"),
            Setting::Zoom => t!("settings-ui-zoom"),
            Setting::SystemTitleBar => t!("settings-system-title-bar"),
            Setting::EditorFont => t!("settings-editor-font"),
            Setting::EditorSize => t!("settings-editor-size"),
            Setting::TabWidth => t!("settings-tab-width"),
            Setting::AutoSave => t!("settings-auto-save"),
            Setting::HotExit => t!("settings-hot-exit"),
            Setting::ConfirmClose => t!("settings-confirm-close"),
            Setting::RestoreFolder => t!("settings-restore-folder"),
            Setting::NewFileOnStartup => t!("settings-new-file-on-startup"),
            Setting::DefaultLanguage => t!("settings-default-language"),
            Setting::DetectLanguageOnPaste => t!("settings-detect-language-on-paste"),
            Setting::DropFolder => t!("settings-drop-folder"),
            Setting::FindInFiles => t!("settings-find-in-files"),
            Setting::StickyScroll => t!("settings-sticky-scroll"),
            Setting::SmoothCaret => t!("settings-smooth-caret"),
            Setting::VimMode => t!("settings-vim-mode"),
            Setting::VimClipboard => t!("settings-vim-clipboard"),
            Setting::VimRelativeNumber => t!("settings-vim-relative-number"),
            Setting::VimYankHighlight => t!("settings-vim-yank-highlight"),
            Setting::WordWrap => t!("settings-word-wrap"),
            Setting::WholeLines => t!("settings-whole-lines"),
            Setting::Minimap => t!("settings-minimap"),
            Setting::LineNumbers => t!("settings-line-numbers"),
            Setting::DiffInline => t!("settings-diff-inline"),
            Setting::RenderWhitespace => t!("settings-render-whitespace"),
            Setting::StatusBar => t!("settings-status-bar"),
            Setting::RestoreWindow => t!("settings-restore-window"),
            Setting::Animations => t!("settings-animations"),
            Setting::SmoothScrolling => t!("settings-smooth-scrolling"),
            Setting::BackgroundImage => t!("settings-background-image"),
            Setting::BackgroundFit => t!("settings-background-fit"),
            Setting::BackgroundOpacity => t!("settings-background-opacity"),
            Setting::Outline => t!("settings-outline"),
            Setting::CompactFolders => t!("settings-compact-folders"),
            Setting::PreviewOnClick => t!("settings-preview-on-click"),
            Setting::RemoteImages => t!("settings-remote-images"),
            Setting::TextWidth => t!("settings-text-width"),
            Setting::MarkdownFont => t!("settings-markdown-font"),
            Setting::MarkdownSize => t!("settings-markdown-size"),
            Setting::MarkdownLineNumbers => t!("settings-markdown-line-numbers"),
            Setting::MarkdownShortcuts => t!("settings-markdown-shortcuts"),
            Setting::MarkdownFocus => t!("settings-markdown-focus"),
            Setting::MarkdownTypewriter => t!("settings-markdown-typewriter"),
            Setting::MarkdownSelectionToolbar => t!("settings-markdown-selection-toolbar"),
            Setting::CheckUpdates => t!("settings-check-updates"),
            Setting::Version => t!("settings-version"),
            Setting::Repository => "GitHub",
            Setting::Website => t!("settings-website"),
            Setting::Community => "Telegram",
            Setting::License => t!("settings-license"),
            Setting::Updates => t!("settings-updates"),
            Setting::ReleaseNotes => t!("settings-release-notes"),
            Setting::FormatOnSave => t!("settings-format-on-save"),
            Setting::FixOnSave => t!("settings-fix-on-save"),
            Setting::SkipOnSave => t!("settings-skip-on-save"),
            Setting::FormatCommand(id) | Setting::FixCommand(id) => {
                winc_syntax::find(id).map_or(id, |l| l.name)
            }
            Setting::AddLanguage | Setting::AddFixLanguage => t!("settings-add-language"),
            Setting::TerminalShell => t!("settings-terminal-shell"),
            Setting::TerminalPosition => t!("settings-terminal-position"),
            Setting::TerminalTabPosition => t!("settings-terminal-tab-position"),
            Setting::TerminalFont => t!("settings-terminal-font"),
            Setting::TerminalSize => t!("settings-terminal-size"),
            Setting::TerminalFontWeight => t!("settings-terminal-font-weight"),
            Setting::TerminalCopyOnSelect => t!("settings-terminal-copy-on-select"),
            Setting::TerminalConfirmClose => t!("settings-terminal-confirm-close"),
            Setting::TerminalPrompt => t!("settings-terminal-prompt"),
            Setting::TerminalShellIntegration => t!("settings-terminal-shell-integration"),
            Setting::TerminalApplicationPriority => t!("settings-terminal-application-priority"),
            Setting::TerminalOptionAsMeta => t!("settings-terminal-option-as-meta"),
            Setting::GitModule => "Git",
            Setting::GitConfirmPush => t!("settings-git-confirm-push"),
            Setting::GitRememberCredentials => t!("settings-git-remember-credentials"),
            Setting::SystemProxy => t!("settings-system-proxy"),
            Setting::GitInlineBlame => t!("settings-git-inline-blame"),
            Setting::GitAutoFetch => t!("settings-git-auto-fetch"),
            Setting::GitTree => t!("settings-git-tree"),
            Setting::GitStagingArea => t!("settings-git-staging-area"),
            Setting::GitOpenWorktree => t!("settings-git-open-worktree"),
            Setting::GitPullMode => t!("settings-git-pull"),
            Setting::GitPath => t!("settings-git-path"),
            Setting::LatexPath => t!("settings-latex-path"),
            Setting::TerminalModule => t!("settings-terminal-module"),
            Setting::RemoteModule => t!("settings-remote-module"),
            Setting::ExplorerMenu => t!("settings-explorer-menu"),
            Setting::FileAssociations => t!("settings-file-associations"),
            Setting::KeySearch => t!("settings-key-search"),
            Setting::Keymap => t!("settings-keys-keymap"),
            Setting::KeyMatches => t!("settings-key-matches"),
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
            Setting::Zoom => Some(t!("settings-ui-zoom-desc")),
            Setting::AutoSave => Some(t!("settings-auto-save-desc")),
            Setting::HotExit => Some(t!("settings-hot-exit-desc")),
            Setting::ConfirmClose => Some(t!("settings-confirm-close-desc")),
            Setting::RestoreFolder => Some(t!("settings-restore-folder-desc")),
            Setting::NewFileOnStartup => Some(t!("settings-new-file-on-startup-desc")),
            Setting::DefaultLanguage => Some(t!("settings-default-language-desc")),
            Setting::DetectLanguageOnPaste => Some(t!("settings-detect-language-on-paste-desc")),
            Setting::DropFolder => Some(t!("settings-drop-folder-desc")),
            Setting::FindInFiles => Some(t!("settings-find-in-files-desc")),
            Setting::StickyScroll => Some(t!("settings-sticky-scroll-desc")),
            Setting::VimMode => Some(t!("settings-vim-mode-desc")),
            Setting::VimClipboard => Some(t!("settings-vim-clipboard-desc")),
            Setting::VimRelativeNumber => Some(t!("settings-vim-relative-number-desc")),
            Setting::VimYankHighlight => Some(t!("settings-vim-yank-highlight-desc")),
            Setting::WordWrap => Some(t!("settings-word-wrap-desc")),
            Setting::WholeLines => Some(t!("settings-whole-lines-desc")),
            Setting::Outline => Some(t!("settings-outline-desc")),
            Setting::CompactFolders => Some(t!("settings-compact-folders-desc")),
            Setting::PreviewOnClick => Some(t!("settings-preview-on-click-desc")),
            Setting::Minimap => Some(t!("settings-minimap-desc")),
            Setting::LineNumbers => Some(t!("settings-line-numbers-desc")),
            Setting::DiffInline => Some(t!("settings-diff-inline-desc")),
            Setting::RenderWhitespace => Some(t!("settings-render-whitespace-desc")),
            Setting::StatusBar => Some(t!("settings-status-bar-desc")),
            Setting::RestoreWindow => Some(t!("settings-restore-window-desc")),
            Setting::Animations => Some(t!("settings-animations-desc")),
            Setting::SmoothScrolling => Some(t!("settings-smooth-scrolling-desc")),
            Setting::BackgroundImage => Some(t!("settings-background-image-desc")),
            Setting::BackgroundFit => Some(t!("settings-background-fit-desc")),
            Setting::BackgroundOpacity => Some(t!("settings-background-opacity-desc")),
            Setting::RemoteImages => Some(t!("settings-remote-images-desc")),
            Setting::SmoothCaret => Some(t!("settings-smooth-caret-desc")),
            Setting::TextWidth => Some(t!("settings-text-width-desc")),
            Setting::MarkdownLineNumbers => Some(t!("settings-markdown-line-numbers-desc")),
            Setting::MarkdownShortcuts => Some(t!("settings-markdown-shortcuts-desc")),
            Setting::MarkdownFocus => Some(t!("settings-markdown-focus-desc")),
            Setting::MarkdownTypewriter => Some(t!("settings-markdown-typewriter-desc")),
            Setting::MarkdownSelectionToolbar => {
                Some(t!("settings-markdown-selection-toolbar-desc"))
            }
            Setting::CheckUpdates => Some(t!("settings-check-updates-desc")),
            // 绘制时换成当前的更新状态（`UpdateState`）。
            Setting::Updates => Some(t!("settings-updates-off")),
            Setting::Keymap => Some(t!("settings-keys-keymap-description")),
            Setting::KeyMatches => Some(t!("settings-key-matches-desc")),
            Setting::FormatOnSave => Some(t!("settings-format-on-save-desc")),
            Setting::FixOnSave => Some(t!("settings-fix-on-save-desc")),
            Setting::SkipOnSave => Some(t!("settings-skip-on-save-desc")),
            Setting::GitModule => Some(t!("settings-git-module-desc")),
            Setting::RemoteModule => Some(t!("settings-remote-module-desc")),
            Setting::ExplorerMenu => Some(t!("settings-explorer-menu-desc")),
            Setting::FileAssociations => Some(t!("settings-file-associations-desc")),
            Setting::GitConfirmPush => Some(t!("settings-git-confirm-push-desc")),
            Setting::GitRememberCredentials => Some(t!("settings-git-remember-credentials-desc")),
            Setting::SystemProxy => Some(t!("settings-system-proxy-desc")),
            Setting::GitInlineBlame => Some(t!("settings-git-inline-blame-desc")),
            Setting::GitAutoFetch => Some(t!("settings-git-auto-fetch-desc")),
            Setting::GitTree => Some(t!("settings-git-tree-desc")),
            Setting::GitStagingArea => Some(t!("settings-git-staging-area-desc")),
            Setting::GitOpenWorktree => Some(t!("settings-git-open-worktree-desc")),
            Setting::GitPullMode => Some(t!("settings-git-pull-desc")),
            Setting::GitPath => Some(t!("settings-git-path-desc")),
            Setting::LatexPath => Some(t!("settings-latex-path-desc")),
            Setting::TerminalShell => Some(t!("settings-terminal-shell-desc")),
            Setting::TerminalCopyOnSelect => Some(t!("settings-terminal-copy-on-select-desc")),
            Setting::TerminalConfirmClose => Some(t!("settings-terminal-confirm-close-desc")),
            Setting::TerminalPrompt => Some(t!("settings-terminal-prompt-desc")),
            Setting::TerminalShellIntegration => {
                Some(t!("settings-terminal-shell-integration-desc"))
            }
            Setting::TerminalOptionAsMeta => Some(t!("settings-terminal-option-as-meta-desc")),
            Setting::TerminalApplicationPriority => {
                Some(t!("settings-terminal-application-priority-desc"))
            }
            _ => None,
        };
        text.map(Cow::Borrowed)
    }

    /// 返回设置项右侧的控件种类。
    pub fn control(self) -> SettingControl {
        match self {
            Setting::EditorFont
            | Setting::UiFont
            | Setting::TerminalFont
            | Setting::MarkdownFont => SettingControl::Font,
            Setting::EditorSize
            | Setting::UiSize
            | Setting::FontWeight
            | Setting::BackgroundOpacity
            | Setting::TerminalSize
            | Setting::TerminalFontWeight
            | Setting::MarkdownSize
            | Setting::TabWidth => SettingControl::Stepper,
            Setting::Theme => SettingControl::Themes,
            Setting::LightTheme | Setting::DarkTheme => SettingControl::ThemeList,
            Setting::SystemTitleBar
            | Setting::HotExit
            | Setting::ConfirmClose
            | Setting::RestoreFolder
            | Setting::NewFileOnStartup
            | Setting::DetectLanguageOnPaste
            | Setting::CompactFolders
            | Setting::PreviewOnClick
            | Setting::StickyScroll
            | Setting::VimMode
            | Setting::VimRelativeNumber
            | Setting::VimYankHighlight
            | Setting::WordWrap
            | Setting::WholeLines
            | Setting::Minimap
            | Setting::Outline
            | Setting::LineNumbers
            | Setting::DiffInline
            | Setting::StatusBar
            | Setting::RestoreWindow
            | Setting::Animations
            | Setting::SmoothScrolling
            | Setting::RemoteImages
            | Setting::MarkdownLineNumbers
            | Setting::MarkdownShortcuts
            | Setting::MarkdownFocus
            | Setting::MarkdownTypewriter
            | Setting::MarkdownSelectionToolbar
            | Setting::CheckUpdates
            | Setting::FormatOnSave
            | Setting::FixOnSave
            | Setting::GitModule
            | Setting::GitConfirmPush
            | Setting::GitRememberCredentials
            | Setting::SystemProxy
            | Setting::GitInlineBlame
            | Setting::GitAutoFetch
            | Setting::GitTree
            | Setting::GitStagingArea
            | Setting::ExplorerMenu
            | Setting::FileAssociations
            | Setting::TerminalModule
            | Setting::RemoteModule
            | Setting::TerminalCopyOnSelect
            | Setting::TerminalConfirmClose
            | Setting::TerminalPrompt
            | Setting::TerminalShellIntegration
            | Setting::TerminalApplicationPriority => SettingControl::Switch,
            Setting::FormatCommand(_)
            | Setting::FixCommand(_)
            | Setting::TerminalShell
            | Setting::GitPath
            | Setting::LatexPath => SettingControl::Command,
            Setting::SkipOnSave
            | Setting::Language
            | Setting::BackgroundImage
            | Setting::BackgroundFit
            | Setting::TextWidth
            | Setting::DefaultLanguage
            | Setting::Zoom
            | Setting::RenderWhitespace
            | Setting::VimClipboard
            | Setting::TerminalPosition
            | Setting::TerminalTabPosition
            | Setting::TerminalOptionAsMeta
            | Setting::DropFolder
            | Setting::FindInFiles
            | Setting::AutoSave
            | Setting::SmoothCaret
            | Setting::GitOpenWorktree
            | Setting::GitPullMode
            | Setting::Keymap => SettingControl::Choice,
            Setting::AddLanguage | Setting::AddFixLanguage => SettingControl::Action,
            Setting::KeySearch => SettingControl::Search,
            Setting::Version => SettingControl::Product,
            Setting::ReleaseNotes
            | Setting::Website
            | Setting::Repository
            | Setting::Community
            | Setting::License => SettingControl::Link,
            Setting::Updates | Setting::KeyMatches => SettingControl::Button,
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

/// 设置浮层中可以单独悬停和点击的部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPart {
    /// 右上角的关闭按钮。
    Close,
    /// 分类栏顶部的搜索框。
    SearchField,
    /// 分类栏中的一项。
    Page(SettingsPage),
    /// 名称后面的「恢复默认」，改过的项才有：点击后恢复默认。
    Reset(Setting),
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
    /// 「快捷键」页「键位方案」一行的值：点击后弹出方案和导入的菜单。
    Keymap,
    /// 「快捷键」页列表中的一行，参数是该行的命令：点击后修改该命令的快捷键。
    KeyRow(keymap::Action),
    /// 悬停在一行上时出现的「编辑」。
    KeyEdit(keymap::Action),
    /// 悬停在改过的一行上时出现的「恢复默认」。
    KeyReset(keymap::Action),
    /// 修改快捷键时第几个已有的键上的 ×。
    KeyRemove(usize),
    /// 修改快捷键时的「保存」。
    KeySave,
    /// 修改快捷键时的「取消」。
    KeyCancel,
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
    /// 分类栏中「Git」一项的显示比例，同 [`Self::terminal`]。
    pub git: f32,
    /// 正在输入的设置项：输入格式化命令时换成更宽的输入框。
    pub editing: Option<Setting>,
    /// 正文的滚动量（像素）。
    pub scroll: f32,
    /// 卡片相对窗口中央的偏移（逻辑像素），见 [`settings_card`]。
    pub offset: (f32, f32),
}

/// 返回设置卡片在窗口 `window` 中的位置：从中央移动 `offset`，四周至少留出 `SETTINGS_MARGIN`。
pub fn settings_card(window: Rect, (dx, dy): (f32, f32)) -> Rect {
    let w = SETTINGS_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
    let h = SETTINGS_H.min(window.h - SETTINGS_MARGIN * 2.0).max(0.0);
    // 窗口过小时上下限颠倒，以下限为准，不用 `clamp`。
    let place = |start: f32, span: f32, size: f32, d: f32| {
        let centered = start + (span - size) / 2.0 + d;
        centered
            .min(start + span - SETTINGS_MARGIN - size)
            .max(start + SETTINGS_MARGIN)
            .round()
    };
    Rect::new(
        place(window.x, window.w, w, dx),
        place(window.y, window.h, h, dy),
        w,
        h,
    )
}

/// 返回搜索 `query` 时正文列出的各组：每页中名称或说明含有 `query` 的设置项
/// （[`Setting::matches`]）合为一组，以页名为标题；页名或组标题含有 `query` 时是该页或该组的
/// 全部设置项。
///
/// 各组按页的顺序排列，组内按页内的顺序；组下方的文字按钮不参与搜索。「快捷键」页列出键位方案
/// （[`Setting::Keymap`]），以及有命令的名称或按键与 `query` 相符时的 [`Setting::KeyMatches`]。
/// `modules` 为各模块是否开启（见 [`SettingsPage::listed`]）。
pub fn search_sections(query: &str, languages: &ToolLanguages, modules: Modules) -> Vec<Section> {
    let query = query.trim().to_lowercase();
    let query = query.as_str();
    let named = move |title: &str| title.to_lowercase().contains(query);
    let found = SettingsPage::listed(modules).map(|page| {
        let whole = named(page.title());
        let items: Vec<Setting> = if page == SettingsPage::Keys {
            let keymap = (whole || Setting::Keymap.matches(query)).then_some(Setting::Keymap);
            let commands = (crate::keymap::count_matches(query) > 0).then_some(Setting::KeyMatches);
            keymap.into_iter().chain(commands).collect()
        } else {
            let items = page.sections(languages).into_iter().flat_map(|section| {
                let whole = whole || section.title.is_some_and(named);
                (section.items.into_iter())
                    .filter(|s| s.control() != SettingControl::Action)
                    .filter(move |s| whole || s.matches(query))
            });
            items.collect()
        };
        Section {
            title: Some(page.title()),
            items,
        }
    });
    let found: Vec<Section> = found.filter(|s| !s.items.is_empty()).collect();
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
            git,
            editing,
            scroll,
            offset,
        } = view;
        let searching = !query.trim().is_empty();
        let card = settings_card(window, offset);

        let nav = Rect::new(card.x, card.y, SETTINGS_NAV_W.min(card.w), card.h);
        // 搜索框与右侧标题行垂直居中对齐，各页列在其下。「Git」「终端」两项的高度按显示比例
        // 收放，下方各项随之滑动。
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
                let share = match page {
                    SettingsPage::Terminal => terminal,
                    SettingsPage::Git => git,
                    _ => 1.0,
                };
                let h = SETTINGS_NAV_ROW * share;
                let row = Rect::new(nav.x + 8.0, y, (nav.w - 16.0).max(0.0), h);
                y += (SETTINGS_NAV_ROW + SETTINGS_NAV_GAP) * share;
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
        let mut y = body.y - super::drawn_scroll(scroll);
        let mut groups = Vec::new();
        let mut headings = Vec::new();
        let mut separators = Vec::new();
        let mut rows = Vec::new();

        let sections = if searching {
            let modules = Modules {
                git: git > 0.0,
                terminal: terminal > 0.0,
            };
            search_sections(query, languages, modules)
        } else {
            page.sections(languages)
        };
        for (i, section) in sections.into_iter().enumerate() {
            if i > 0 {
                y += SETTINGS_GROUP_GAP;
            }
            if let Some(title) = section.title {
                // 与组的底色左对齐。
                let line = Rect::new(x, y, group_w, SETTINGS_TITLE);
                headings.push((line, title));
                y += SETTINGS_TITLE;
            }
            let (in_group, below): (Vec<Setting>, Vec<Setting>) = section
                .items
                .into_iter()
                .partition(|s| s.control() != SettingControl::Action);

            // 一组里各项都有说明时才写说明：各行等高。
            let described = in_group.iter().all(|s| s.description().is_some());
            let top = y;
            for (n, setting) in in_group.into_iter().enumerate() {
                if n > 0 {
                    let inner = group_w - SETTINGS_INSET * 2.0;
                    separators.push(Rect::new(x + SETTINGS_INSET, y, inner.max(0.0), 1.0));
                }
                let editing = editing == Some(setting);
                let place = (Rect::new(x, y, group_w, 0.0), editing, described);
                let h = row_height(setting, place, measure);
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

    /// 卡片顶部与标题行同高的一条，横跨两栏：其中没有部件的地方可以拖动卡片。
    pub fn title_strip(&self) -> Rect {
        Rect::new(self.card.x, self.card.y, self.card.w, self.header.h)
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

/// 返回一行的高度。
///
/// 格式化命令行在名称下方显示所用的命令，因此总是两行高；写说明的行按说明折成的行数加高
/// （见 [`description_lines`]）；主题一行是几排缩略图，高度按主题个数和组宽算。
///
/// `row` 给出该行的横向位置和宽度，`editing` 为该行是否正在输入，`described` 为所在的组是否写
/// 说明，`measure` 量说明文字的宽度。
fn row_height(
    setting: Setting,
    (row, editing, described): (Rect, bool, bool),
    measure: &mut dyn FnMut(&str) -> f32,
) -> f32 {
    match setting.control() {
        SettingControl::ThemeList => {
            let n = crate::theme::themes(setting == Setting::DarkTheme).len();
            let rows = n.div_ceil(THEME_COLUMNS) as f32;
            let tile = theme_tile_size((row.w - SETTINGS_INSET * 2.0).max(0.0));
            rows * tile.1 + (rows - 1.0) * THEME_GRID_GAP + THEME_GRID_PAD * 2.0
        }
        SettingControl::Command
            if !matches!(
                setting,
                Setting::TerminalShell | Setting::GitPath | Setting::LatexPath
            ) =>
        {
            SETTINGS_ROW_TALL
        }
        SettingControl::Product => SETTINGS_PRODUCT_H,
        _ => match setting.description().filter(|_| described) {
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
