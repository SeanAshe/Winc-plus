//! 命令：显示名、英文名、分组、所属模块、能否在差异和终端里执行，以及是否在命令面板中列出。

use winc_i18n::t;

use super::*;

mod english;

/// 可以用快捷键或命令面板执行的命令。
///
/// 变体按所属分组（[`Group`]）依次排列；各命令的显示名见 [`Action::title`] 和 [`Action::english`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    ShowCommands,
    QuickOpen,
    GoToSymbol,
    GoToLine,
    GoToDefinition,
    FindReferences,
    GoBack,
    GoForward,
    Settings,
    ReleaseNotes,
    KeyboardShortcuts,
    RecordKeys,

    NewFile,
    NewWindow,
    OpenFile,
    OpenFolder,
    /// 把一个目录加为窗口的附加目录。
    AddFolder,
    RemoteConnect,
    RemoteDisconnect,
    OpenRecent,
    Save,
    SaveAs,
    CloseTab,
    CloseOtherTabs,
    CloseAllTabs,
    ReopenClosed,
    NextTab,
    PreviousTab,
    SplitRight,
    SplitDown,
    MoveToSplitRight,
    MoveToSplitDown,
    Unsplit,
    NextPane,
    PreviousPane,
    CloseFolder,
    CloseWindow,
    CopyPath,
    CopyRelativePath,
    CopyReference,
    CopyAbsoluteReference,
    RevealInFinder,
    CopyActivePath,
    CopyKeyPath,
    RevealActiveFile,
    OpenInTerminal,
    Quit,

    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    ToggleComment,
    /// Markdown 中切换粗体（`**`）。
    MarkdownBold,
    /// Markdown 中切换斜体（`*`）。
    MarkdownItalic,
    /// Markdown 中切换行内代码（`` ` ``）。
    MarkdownCode,
    /// Markdown 中切换删除线（`~~`）。
    MarkdownStrikethrough,
    /// Markdown 中切换高亮（`==`）。
    MarkdownHighlight,
    /// Markdown 中设为一级标题；已经是时改回正文。
    MarkdownHeading1,
    /// Markdown 中设为二级标题。
    MarkdownHeading2,
    /// Markdown 中设为三级标题。
    MarkdownHeading3,
    /// Markdown 中设为四级标题。
    MarkdownHeading4,
    /// Markdown 中设为五级标题。
    MarkdownHeading5,
    /// Markdown 中设为六级标题。
    MarkdownHeading6,
    /// Markdown 中标题改回正文。
    MarkdownParagraph,
    /// Markdown 中选中的各行加上或去掉一层引用。
    MarkdownQuote,
    /// Markdown 中选中的各行改为有序列表，或去掉列表记号。
    MarkdownOrderedList,
    /// Markdown 中选中的各行改为无序列表，或去掉列表记号。
    MarkdownBulletList,
    /// Markdown 中把选中的各行放进代码块，或插入一个空的代码块。
    MarkdownCodeBlock,
    /// Markdown 中把选中的各行放进公式块，或插入一个空的公式块。
    MarkdownMathBlock,
    /// Markdown 中插入两列的表格。
    MarkdownTable,
    /// Markdown 中把选中的文字做成链接；剪贴板里是网址时作为地址。
    MarkdownLink,
    /// Markdown 中插入图片；选中的文字作为替代文字。
    MarkdownImage,
    /// Markdown 中切换专注模式。
    MarkdownFocusMode,
    /// Markdown 中切换打字机模式。
    MarkdownTypewriterMode,
    /// Markdown 中粘贴为纯文本。
    MarkdownPastePlain,
    DeleteLine,
    DeleteAllLeft,
    IndentLines,
    OutdentLines,
    MoveLinesUp,
    MoveLinesDown,
    CopyLinesUp,
    CopyLinesDown,
    JoinLines,
    ExpandSelection,
    ShrinkSelection,
    AddCursorAbove,
    AddCursorBelow,
    AddNextOccurrence,
    SkipToNextOccurrence,
    SelectAllOccurrences,
    RemoveSecondaryCursors,
    AddCursorsToLineEnds,
    CursorUndo,
    CursorRedo,
    ColumnSelectUp,
    ColumnSelectDown,
    ColumnSelectLeft,
    ColumnSelectRight,
    JumpToBracket,
    TransformToUppercase,
    TransformToLowercase,
    TransformToTitlecase,
    SortLinesAscending,
    SortLinesDescending,
    TrimTrailingWhitespace,
    IndentUsingSpaces,
    IndentUsingTabs,
    ConvertIndentationToSpaces,
    ConvertIndentationToTabs,
    DetectIndentation,
    ChangeEndOfLine,
    ChangeLanguageMode,
    Fold,
    Unfold,
    ToggleFold,
    FoldRecursively,
    UnfoldRecursively,
    FoldAll,
    UnfoldAll,
    FoldLevel1,
    FoldLevel2,
    FoldLevel3,
    FoldLevel4,
    FoldLevel5,
    FoldLevel6,
    FoldLevel7,
    ToggleBookmark,
    NextBookmark,
    PreviousBookmark,
    ListBookmarks,
    ClearBookmarks,
    Format,
    FormatSelection,
    Fix,

    Find,
    Replace,
    FindNext,
    FindPrevious,
    HighlightWord,
    ClearWordHighlights,
    ReplaceOne,
    ReplaceAll,
    SelectAllMatches,
    ToggleCase,
    ToggleWord,
    ToggleRegex,
    FilterLines,
    FindInFiles,
    ReplaceInFiles,
    ReplaceAllInFiles,
    NextResult,
    PreviousResult,

    RenameFile,
    DeleteFile,
    DeleteFilePermanently,
    OpenSelected,
    PreviewSelected,
    FindInFolder,

    ToggleSidebar,
    FocusSidebar,
    ShowFiles,
    ShowOutline,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    ThemeAuto,
    ThemeLight,
    ThemeDark,
    ToggleStickyScroll,
    /// 开启或关闭 Vim 模式。
    ToggleVimMode,
    ToggleWordWrap,
    /// 显示或隐藏编辑区右侧的缩略图。
    ToggleMinimap,
    /// 显示或隐藏行号。
    ToggleLineNumbers,
    /// 显示或隐藏状态栏。
    ToggleStatusBar,
    /// 全部显示或不显示空白符。
    ToggleWhitespace,
    TogglePreview,
    /// 把当前标签设为只读或恢复为可以编辑。
    ToggleReadOnly,
    /// 把窗口里的全部标签设为只读或恢复。
    ToggleReadOnlyAll,
    NextDifference,
    PreviousDifference,
    /// 差异里的下一个文件。
    NextDiffFile,
    /// 差异里的上一个文件。
    PreviousDiffFile,
    /// 切换差异的单栏显示和并排显示。
    ToggleDiffLayout,
    GitShowChanges,
    /// 更改列表：显示选中文件的差异，焦点移到差异上。
    ChangesShowDiff,
    /// 更改列表：在编辑区打开选中的文件。
    ChangesOpenSource,
    /// 更改列表：回滚选中的文件（先确认）。
    ChangesRollback,
    GitShowLog,
    GitGoToRevision,
    /// 在只读标签中打开 Git 操作的输出。
    GitShowOutput,
    /// 克隆仓库。
    GitClone,
    /// 在当前目录中初始化仓库。
    GitInit,
    /// 打开提交视图并展开位置选择器。
    GitSwitchRepository,
    GitSwitchWorktree,
    GitNewWorktree,
    /// 展开分支选择器（提交视图里从分支行，否则从状态栏）。
    GitSwitchBranch,
    /// 打开新建分支的对话框（从当前分支）。
    GitNewBranch,
    /// 获取远程仓库的更新。
    GitFetch,
    /// 拉取当前分支的上游。
    GitPull,
    /// 拉取当前分支的上游并变基。
    GitPullRebase,
    /// 推送当前分支（没有上游时推上去并设为上游）。
    GitPush,
    /// 和上游同步：先拉取再推送。
    GitSync,
    /// 储藏工作区里的更改（输入说明）。
    GitStash,
    /// 把全部更改加入暂存区，未跟踪的文件除外。
    GitStageAll,
    /// 把暂存区中的全部内容移出暂存区。
    GitUnstageAll,
    /// 把当前文件的更改加入暂存区。
    GitStageFile,
    GitFileHistory,
    /// 当前文件中选中的几行的历史。
    GitSelectionHistory,
    GitBlame,
    ShowCommit,
    GitRevertChange,
    ToggleTerminal,
    /// 显示或隐藏终端面板，与焦点位置无关。
    ToggleTerminalPanel,
    /// 启用终端模块；模块关闭时列出，使内置终端可以从命令面板找到。
    EnableTerminal,
    /// 运行当前选中的配置；运行中时重新运行。
    RunConfiguration,
    /// 停止当前选中的配置。
    StopConfiguration,
    /// 打开运行配置文件。
    EditRunConfigurations,
    ClearTerminal,
    /// 复制终端的选区。
    TerminalCopy,
    /// 把剪贴板粘贴到终端（见 `App::terminal_paste`）。
    TerminalPaste,
    NewTerminal,
    CloseTerminal,
    NextTerminal,
    PreviousTerminal,
    SplitTerminal,
    NextTerminalPane,
    PreviousTerminalPane,
    FocusTerminal1,
    FocusTerminal2,
    FocusTerminal3,
    FocusTerminal4,
    FocusTerminal5,
    FocusTerminal6,
    FocusTerminal7,
    FocusTerminal8,
    FocusTerminal9,
    FindInTerminal,
    MaximizeTerminal,
    /// 把终端翻到回滚缓冲的最上面。
    ScrollTerminalTop,
    /// 把终端翻回最新的输出。
    ScrollTerminalBottom,
    ToggleTerminalPosition,
    InstallShellCommand,
    UninstallShellCommand,
    RenameTerminal,
    PreviousCommand,
    NextCommand,
    FoldOutput,
    UnfoldOutput,
    NextError,
    PreviousError,
    SendToTerminal,
}

/// 可选模块。
///
/// 属于模块的命令只在模块开着时出现在命令面板里、响应快捷键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    /// Git 模块。
    Git,
    /// 终端模块。
    Terminal,
    /// 远程模块。
    Remote,
}

impl Action {
    /// 按显示顺序聚焦第 1 至第 9 个终端的命令，供数字键绑定和编号查询共用。
    pub const TERMINAL_FOCUS: [Self; 9] = [
        Self::FocusTerminal1,
        Self::FocusTerminal2,
        Self::FocusTerminal3,
        Self::FocusTerminal4,
        Self::FocusTerminal5,
        Self::FocusTerminal6,
        Self::FocusTerminal7,
        Self::FocusTerminal8,
        Self::FocusTerminal9,
    ];

    /// 返回终端聚焦命令的标签下标（从 0 起），其他命令返回 `None`。
    pub fn terminal_index(self) -> Option<usize> {
        Self::TERMINAL_FOCUS
            .iter()
            .position(|action| *action == self)
    }

    /// 返回命令在当前界面语言中的名称，命令面板、「快捷键」页和菜单都使用它。
    pub fn title(self) -> &'static str {
        use Action::*;
        match self {
            ShowCommands => t!("cmd-show-commands"),
            QuickOpen => t!("cmd-quick-open"),
            GoToSymbol => t!("cmd-go-to-symbol"),
            GoToLine => t!("cmd-go-to-line"),
            GoToDefinition => t!("cmd-go-to-definition"),
            FindReferences => t!("cmd-find-references"),
            GoBack => t!("cmd-go-back"),
            GoForward => t!("cmd-go-forward"),
            Settings => t!("cmd-settings"),
            ReleaseNotes => t!("cmd-release-notes"),
            KeyboardShortcuts => t!("cmd-keyboard-shortcuts"),
            RecordKeys => t!("cmd-record-keys"),
            NewFile => t!("cmd-new-file"),
            NewWindow => t!("cmd-new-window"),
            OpenFile => t!("cmd-open-file"),
            OpenFolder => t!("cmd-open-folder"),
            AddFolder => t!("cmd-add-folder"),
            RemoteConnect => t!("cmd-remote-connect"),
            RemoteDisconnect => t!("cmd-remote-disconnect"),
            OpenRecent => t!("cmd-open-recent"),
            Save => t!("cmd-save"),
            SaveAs => t!("cmd-save-as"),
            CloseTab => t!("cmd-close-tab"),
            CloseOtherTabs => t!("cmd-close-other-tabs"),
            CloseAllTabs => t!("cmd-close-all-tabs"),
            ReopenClosed => t!("cmd-reopen-closed"),
            NextTab => t!("cmd-next-tab"),
            PreviousTab => t!("cmd-previous-tab"),
            SplitRight => t!("cmd-split-right"),
            SplitDown => t!("cmd-split-down"),
            MoveToSplitRight => t!("cmd-move-to-split-right"),
            MoveToSplitDown => t!("cmd-move-to-split-down"),
            Unsplit => t!("cmd-unsplit"),
            NextPane => t!("cmd-next-pane"),
            PreviousPane => t!("cmd-previous-pane"),
            CloseFolder => t!("cmd-close-folder"),
            CloseWindow => t!("cmd-close-window"),
            CopyPath => t!("cmd-copy-path"),
            CopyRelativePath => t!("cmd-copy-relative-path"),
            CopyReference => t!("cmd-copy-reference"),
            CopyAbsoluteReference => t!("cmd-copy-absolute-reference"),
            RevealInFinder => crate::system::reveal_label(),
            CopyActivePath => t!("cmd-copy-active-path"),
            CopyKeyPath => t!("cmd-copy-key-path"),
            OpenInTerminal => t!("cmd-open-in-terminal"),
            RevealActiveFile => {
                if cfg!(target_os = "macos") {
                    t!("cmd-reveal-active-file-in-finder")
                } else {
                    t!("cmd-reveal-active-file-in-folder")
                }
            }
            Quit => t!("cmd-quit"),
            Undo => t!("cmd-undo"),
            Redo => t!("cmd-redo"),
            Cut => t!("cmd-cut"),
            Copy => t!("cmd-copy"),
            Paste => t!("cmd-paste"),
            SelectAll => t!("cmd-select-all"),
            ToggleComment => t!("cmd-toggle-comment"),
            MarkdownBold => t!("cmd-markdown-bold"),
            MarkdownItalic => t!("cmd-markdown-italic"),
            MarkdownCode => t!("cmd-markdown-code"),
            MarkdownStrikethrough => t!("cmd-markdown-strikethrough"),
            MarkdownHighlight => t!("cmd-markdown-highlight"),
            MarkdownHeading1 => t!("cmd-markdown-heading-1"),
            MarkdownHeading2 => t!("cmd-markdown-heading-2"),
            MarkdownHeading3 => t!("cmd-markdown-heading-3"),
            MarkdownHeading4 => t!("cmd-markdown-heading-4"),
            MarkdownHeading5 => t!("cmd-markdown-heading-5"),
            MarkdownHeading6 => t!("cmd-markdown-heading-6"),
            MarkdownParagraph => t!("cmd-markdown-paragraph"),
            MarkdownQuote => t!("cmd-markdown-quote"),
            MarkdownOrderedList => t!("cmd-markdown-ordered-list"),
            MarkdownBulletList => t!("cmd-markdown-bullet-list"),
            MarkdownCodeBlock => t!("cmd-markdown-code-block"),
            MarkdownMathBlock => t!("cmd-markdown-math-block"),
            MarkdownTable => t!("cmd-markdown-table"),
            MarkdownLink => t!("cmd-markdown-link"),
            MarkdownImage => t!("cmd-markdown-image"),
            MarkdownFocusMode => t!("cmd-markdown-focus-mode"),
            MarkdownTypewriterMode => t!("cmd-markdown-typewriter-mode"),
            MarkdownPastePlain => t!("cmd-markdown-paste-plain"),
            DeleteLine => t!("cmd-delete-line"),
            DeleteAllLeft => t!("cmd-delete-all-left"),
            IndentLines => t!("cmd-indent-lines"),
            OutdentLines => t!("cmd-outdent-lines"),
            MoveLinesUp => t!("cmd-move-lines-up"),
            MoveLinesDown => t!("cmd-move-lines-down"),
            CopyLinesUp => t!("cmd-copy-lines-up"),
            CopyLinesDown => t!("cmd-copy-lines-down"),
            JoinLines => t!("cmd-join-lines"),
            ExpandSelection => t!("cmd-expand-selection"),
            ShrinkSelection => t!("cmd-shrink-selection"),
            AddCursorAbove => t!("cmd-add-cursor-above"),
            AddCursorBelow => t!("cmd-add-cursor-below"),
            AddNextOccurrence => t!("cmd-add-next-occurrence"),
            SkipToNextOccurrence => t!("cmd-skip-to-next-occurrence"),
            SelectAllOccurrences => t!("cmd-select-all-occurrences"),
            RemoveSecondaryCursors => t!("cmd-remove-secondary-cursors"),
            AddCursorsToLineEnds => t!("cmd-add-cursors-to-line-ends"),
            CursorUndo => t!("cmd-cursor-undo"),
            CursorRedo => t!("cmd-cursor-redo"),
            ColumnSelectUp => t!("cmd-column-select-up"),
            ColumnSelectDown => t!("cmd-column-select-down"),
            ColumnSelectLeft => t!("cmd-column-select-left"),
            ColumnSelectRight => t!("cmd-column-select-right"),
            JumpToBracket => t!("cmd-jump-to-bracket"),
            TransformToUppercase => t!("cmd-transform-to-uppercase"),
            TransformToLowercase => t!("cmd-transform-to-lowercase"),
            TransformToTitlecase => t!("cmd-transform-to-titlecase"),
            SortLinesAscending => t!("cmd-sort-lines-ascending"),
            SortLinesDescending => t!("cmd-sort-lines-descending"),
            TrimTrailingWhitespace => t!("cmd-trim-trailing-whitespace"),
            IndentUsingSpaces => t!("cmd-indent-using-spaces"),
            IndentUsingTabs => t!("cmd-indent-using-tabs"),
            ConvertIndentationToSpaces => t!("cmd-convert-indentation-to-spaces"),
            ConvertIndentationToTabs => t!("cmd-convert-indentation-to-tabs"),
            DetectIndentation => t!("cmd-detect-indentation"),
            ChangeEndOfLine => t!("cmd-change-end-of-line"),
            ChangeLanguageMode => t!("cmd-change-language-mode"),
            Fold => t!("cmd-fold"),
            Unfold => t!("cmd-unfold"),
            ToggleFold => t!("cmd-toggle-fold"),
            FoldRecursively => t!("cmd-fold-recursively"),
            UnfoldRecursively => t!("cmd-unfold-recursively"),
            FoldAll => t!("cmd-fold-all"),
            UnfoldAll => t!("cmd-unfold-all"),
            FoldLevel1 => t!("cmd-fold-level-1"),
            FoldLevel2 => t!("cmd-fold-level-2"),
            FoldLevel3 => t!("cmd-fold-level-3"),
            FoldLevel4 => t!("cmd-fold-level-4"),
            FoldLevel5 => t!("cmd-fold-level-5"),
            FoldLevel6 => t!("cmd-fold-level-6"),
            FoldLevel7 => t!("cmd-fold-level-7"),
            ToggleBookmark => t!("cmd-toggle-bookmark"),
            NextBookmark => t!("cmd-next-bookmark"),
            PreviousBookmark => t!("cmd-previous-bookmark"),
            ListBookmarks => t!("cmd-list-bookmarks"),
            ClearBookmarks => t!("cmd-clear-bookmarks"),
            Format => t!("cmd-format"),
            FormatSelection => t!("cmd-format-selection"),
            Fix => t!("cmd-fix"),
            Find => t!("cmd-find"),
            Replace => t!("cmd-replace"),
            FindNext => t!("cmd-find-next"),
            FindPrevious => t!("cmd-find-previous"),
            HighlightWord => t!("cmd-highlight-word"),
            ClearWordHighlights => t!("cmd-clear-word-highlights"),
            ReplaceOne => t!("cmd-replace-one"),
            ReplaceAll => t!("cmd-replace-all"),
            SelectAllMatches => t!("cmd-select-all-matches"),
            ToggleCase => t!("cmd-toggle-case"),
            ToggleWord => t!("cmd-toggle-word"),
            ToggleRegex => t!("cmd-toggle-regex"),
            FilterLines => t!("cmd-filter-lines"),
            FindInFiles => t!("cmd-find-in-files"),
            ReplaceInFiles => t!("cmd-replace-in-files"),
            ReplaceAllInFiles => t!("cmd-replace-all-in-files"),
            NextResult => t!("cmd-next-result"),
            PreviousResult => t!("cmd-previous-result"),
            RenameFile => t!("cmd-rename-file"),
            DeleteFile => t!("cmd-delete-file"),
            DeleteFilePermanently => t!("cmd-delete-file-permanently"),
            OpenSelected => t!("cmd-open-selected"),
            PreviewSelected => t!("cmd-preview-selected"),
            FindInFolder => t!("cmd-find-in-folder"),
            ToggleSidebar => t!("cmd-toggle-sidebar"),
            FocusSidebar => t!("cmd-focus-sidebar"),
            ShowFiles => t!("cmd-show-files"),
            ShowOutline => t!("cmd-show-outline"),
            IncreaseFontSize => t!("cmd-increase-font-size"),
            DecreaseFontSize => t!("cmd-decrease-font-size"),
            ResetFontSize => t!("cmd-reset-font-size"),
            ZoomIn => t!("cmd-zoom-in"),
            ZoomOut => t!("cmd-zoom-out"),
            ResetZoom => t!("cmd-reset-zoom"),
            ThemeAuto => t!("cmd-theme-auto"),
            ThemeLight => t!("cmd-theme-light"),
            ThemeDark => t!("cmd-theme-dark"),
            ToggleStickyScroll => t!("cmd-toggle-sticky-scroll"),
            ToggleVimMode => t!("cmd-toggle-vim-mode"),
            ToggleWordWrap => t!("cmd-toggle-word-wrap"),
            ToggleMinimap => t!("cmd-toggle-minimap"),
            ToggleLineNumbers => t!("cmd-toggle-line-numbers"),
            ToggleStatusBar => t!("cmd-toggle-status-bar"),
            ToggleWhitespace => t!("cmd-toggle-whitespace"),
            NextDifference => t!("cmd-next-difference"),
            PreviousDifference => t!("cmd-previous-difference"),
            NextDiffFile => t!("cmd-next-diff-file"),
            PreviousDiffFile => t!("cmd-previous-diff-file"),
            ToggleDiffLayout => t!("cmd-toggle-diff-layout"),
            GitShowChanges => t!("cmd-git-show-changes"),
            ChangesShowDiff => t!("cmd-changes-show-diff"),
            ChangesOpenSource => t!("cmd-changes-open-source"),
            ChangesRollback => t!("cmd-changes-rollback"),
            GitShowLog => t!("cmd-git-show-log"),
            GitGoToRevision => t!("cmd-git-go-to-revision"),
            GitShowOutput => t!("cmd-git-show-output"),
            GitClone => t!("cmd-git-clone"),
            GitInit => t!("cmd-git-init"),
            GitSwitchRepository => t!("cmd-git-switch-repository"),
            GitSwitchWorktree => t!("cmd-git-switch-worktree"),
            GitNewWorktree => t!("cmd-git-new-worktree"),
            GitSwitchBranch => t!("cmd-git-switch-branch"),
            GitNewBranch => t!("cmd-git-new-branch"),
            GitFetch => t!("cmd-git-fetch"),
            GitPull => t!("cmd-git-pull"),
            GitPullRebase => t!("cmd-git-pull-rebase"),
            GitPush => t!("cmd-git-push"),
            GitSync => t!("cmd-git-sync"),
            GitStash => t!("cmd-git-stash"),
            GitStageAll => t!("cmd-git-stage-all"),
            GitUnstageAll => t!("cmd-git-unstage-all"),
            GitStageFile => t!("cmd-git-stage-file"),
            GitFileHistory => t!("cmd-git-file-history"),
            GitSelectionHistory => t!("cmd-git-selection-history"),
            GitBlame => t!("cmd-git-blame"),
            ShowCommit => t!("cmd-show-commit"),
            GitRevertChange => t!("cmd-git-revert-change"),
            ToggleTerminal => t!("cmd-toggle-terminal"),
            ToggleTerminalPanel => t!("cmd-toggle-terminal-panel"),
            EnableTerminal => t!("cmd-enable-terminal"),
            RunConfiguration => t!("cmd-run-configuration"),
            StopConfiguration => t!("cmd-stop-configuration"),
            EditRunConfigurations => t!("cmd-edit-run-configurations"),
            ClearTerminal => t!("cmd-clear-terminal"),
            TerminalCopy => t!("cmd-terminal-copy"),
            TerminalPaste => t!("cmd-terminal-paste"),
            NewTerminal => t!("cmd-new-terminal"),
            CloseTerminal => t!("cmd-close-terminal"),
            NextTerminal => t!("cmd-next-terminal"),
            PreviousTerminal => t!("cmd-previous-terminal"),
            SplitTerminal => t!("cmd-split-terminal"),
            NextTerminalPane => t!("cmd-next-terminal-pane"),
            PreviousTerminalPane => t!("cmd-previous-terminal-pane"),
            FocusTerminal1 => t!("cmd-terminal-1"),
            FocusTerminal2 => t!("cmd-terminal-2"),
            FocusTerminal3 => t!("cmd-terminal-3"),
            FocusTerminal4 => t!("cmd-terminal-4"),
            FocusTerminal5 => t!("cmd-terminal-5"),
            FocusTerminal6 => t!("cmd-terminal-6"),
            FocusTerminal7 => t!("cmd-terminal-7"),
            FocusTerminal8 => t!("cmd-terminal-8"),
            FocusTerminal9 => t!("cmd-terminal-9"),
            FindInTerminal => t!("cmd-find-in-terminal"),
            MaximizeTerminal => t!("cmd-maximize-terminal"),
            ScrollTerminalTop => t!("cmd-scroll-terminal-top"),
            ScrollTerminalBottom => t!("cmd-scroll-terminal-bottom"),
            ToggleTerminalPosition => t!("cmd-toggle-terminal-position"),
            InstallShellCommand => t!("cmd-install-shell-command"),
            UninstallShellCommand => t!("cmd-uninstall-shell-command"),
            RenameTerminal => t!("cmd-rename-terminal"),
            PreviousCommand => t!("cmd-previous-command"),
            NextCommand => t!("cmd-next-command"),
            FoldOutput => t!("cmd-fold-output"),
            UnfoldOutput => t!("cmd-unfold-output"),
            NextError => t!("cmd-next-error"),
            PreviousError => t!("cmd-previous-error"),
            SendToTerminal => t!("cmd-send-to-terminal"),
            TogglePreview => t!("cmd-toggle-preview"),
            ToggleReadOnly => t!("cmd-toggle-read-only"),
            ToggleReadOnlyAll => t!("cmd-toggle-read-only-all"),
        }
    }

    /// 返回命令在配置文件中的名字：变体名，首字母小写（如 `quickOpen`）。
    pub fn id(self) -> String {
        let name = format!("{self:?}");
        let mut chars = name.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_ascii_lowercase().to_string() + chars.as_str()
        })
    }

    /// 按配置文件中的名字 `id` 返回命令；不认识时返回 `None`。
    pub fn from_id(id: &str) -> Option<Action> {
        ACTIONS.iter().copied().find(|action| action.id() == id)
    }

    /// 返回命令所属的分组。
    pub fn group(self) -> Group {
        use Action::*;
        match self {
            ShowCommands
            | QuickOpen
            | GoToSymbol
            | GoToLine
            | GoToDefinition
            | FindReferences
            | GoBack
            | GoForward
            | Settings
            | ReleaseNotes
            | KeyboardShortcuts
            | RecordKeys
            | InstallShellCommand
            | UninstallShellCommand => Group::General,
            NewFile | OpenFile | OpenFolder | AddFolder | RemoteConnect | RemoteDisconnect
            | Save | SaveAs | CloseTab | CloseOtherTabs | CloseAllTabs | ReopenClosed | NextTab
            | PreviousTab | CloseFolder | CloseWindow | CopyPath | CopyRelativePath
            | RevealInFinder | CopyActivePath | CopyKeyPath | RevealActiveFile | OpenInTerminal
            | Quit => Group::File,
            NewWindow | OpenRecent | CopyReference | CopyAbsoluteReference => Group::File,
            Undo
            | Redo
            | Cut
            | Copy
            | Paste
            | SelectAll
            | ToggleComment
            | MarkdownBold
            | MarkdownItalic
            | MarkdownCode
            | MarkdownStrikethrough
            | MarkdownHighlight
            | MarkdownHeading1
            | MarkdownHeading2
            | MarkdownHeading3
            | MarkdownHeading4
            | MarkdownHeading5
            | MarkdownHeading6
            | MarkdownParagraph
            | MarkdownQuote
            | MarkdownOrderedList
            | MarkdownBulletList
            | MarkdownCodeBlock
            | MarkdownMathBlock
            | MarkdownTable
            | MarkdownLink
            | MarkdownImage
            | MarkdownFocusMode
            | MarkdownTypewriterMode
            | MarkdownPastePlain
            | DeleteLine
            | DeleteAllLeft
            | IndentLines
            | OutdentLines
            | MoveLinesUp
            | MoveLinesDown
            | CopyLinesUp
            | CopyLinesDown
            | JoinLines
            | ExpandSelection
            | ShrinkSelection
            | AddCursorAbove
            | AddCursorBelow
            | AddNextOccurrence
            | SkipToNextOccurrence
            | SelectAllOccurrences
            | RemoveSecondaryCursors
            | AddCursorsToLineEnds
            | CursorUndo
            | CursorRedo
            | ColumnSelectUp
            | ColumnSelectDown
            | ColumnSelectLeft
            | ColumnSelectRight
            | JumpToBracket
            | TransformToUppercase
            | TransformToLowercase
            | TransformToTitlecase
            | SortLinesAscending
            | SortLinesDescending
            | TrimTrailingWhitespace
            | IndentUsingSpaces
            | IndentUsingTabs
            | ConvertIndentationToSpaces
            | ConvertIndentationToTabs
            | DetectIndentation
            | ChangeEndOfLine
            | ChangeLanguageMode
            | Fold
            | Unfold
            | ToggleFold
            | FoldRecursively
            | UnfoldRecursively
            | FoldAll
            | UnfoldAll
            | FoldLevel1
            | FoldLevel2
            | FoldLevel3
            | FoldLevel4
            | FoldLevel5
            | FoldLevel6
            | FoldLevel7
            | ToggleBookmark
            | NextBookmark
            | PreviousBookmark
            | ListBookmarks
            | ClearBookmarks
            | Format
            | FormatSelection
            | Fix => Group::Edit,
            Find | Replace | FindNext | FindPrevious | HighlightWord | ClearWordHighlights
            | ReplaceOne | ReplaceAll | SelectAllMatches | ToggleCase | ToggleWord
            | ToggleRegex | FilterLines | FindInFiles | ReplaceInFiles | ReplaceAllInFiles
            | NextResult | PreviousResult => Group::Find,
            RenameFile
            | DeleteFile
            | DeleteFilePermanently
            | OpenSelected
            | PreviewSelected
            | FindInFolder => Group::Tree,
            ToggleSidebar
            | FocusSidebar
            | ShowFiles
            | ShowOutline
            | IncreaseFontSize
            | DecreaseFontSize
            | ResetFontSize
            | ZoomIn
            | ZoomOut
            | ResetZoom
            | ThemeAuto
            | ThemeLight
            | ThemeDark
            | ToggleStickyScroll
            | ToggleVimMode
            | ToggleWordWrap
            | ToggleMinimap
            | ToggleLineNumbers
            | ToggleStatusBar
            | ToggleWhitespace
            | TogglePreview
            | ToggleReadOnly
            | ToggleReadOnlyAll
            | NextDifference
            | PreviousDifference
            | NextDiffFile
            | PreviousDiffFile
            | ToggleDiffLayout
            | GitShowChanges
            | ChangesShowDiff
            | ChangesOpenSource
            | ChangesRollback
            | GitShowLog
            | GitGoToRevision
            | GitShowOutput
            | GitClone
            | GitInit
            | GitSwitchRepository
            | GitSwitchWorktree
            | GitNewWorktree
            | GitSwitchBranch
            | GitNewBranch
            | GitFetch
            | GitPull
            | GitPullRebase
            | GitPush
            | GitSync
            | GitStash
            | GitStageAll
            | GitUnstageAll
            | GitStageFile
            | GitFileHistory
            | GitSelectionHistory
            | GitBlame
            | ShowCommit
            | GitRevertChange
            | ToggleTerminal
            | ToggleTerminalPanel
            | EnableTerminal
            | RunConfiguration
            | StopConfiguration
            | EditRunConfigurations
            | ClearTerminal
            | TerminalCopy
            | TerminalPaste
            | NewTerminal
            | CloseTerminal
            | NextTerminal
            | PreviousTerminal
            | SplitTerminal
            | NextTerminalPane
            | PreviousTerminalPane
            | FocusTerminal1
            | FocusTerminal2
            | FocusTerminal3
            | FocusTerminal4
            | FocusTerminal5
            | FocusTerminal6
            | FocusTerminal7
            | FocusTerminal8
            | FocusTerminal9
            | FindInTerminal
            | MaximizeTerminal
            | ScrollTerminalTop
            | ScrollTerminalBottom
            | ToggleTerminalPosition
            | RenameTerminal
            | PreviousCommand
            | NextCommand
            | FoldOutput
            | UnfoldOutput
            | NextError
            | PreviousError
            | SendToTerminal
            | SplitRight
            | SplitDown
            | MoveToSplitRight
            | MoveToSplitDown
            | Unsplit
            | NextPane
            | PreviousPane => Group::View,
        }
    }

    /// 是否是在差异里导航的命令：跳到改动、切换文件。
    pub fn navigates_diff(self) -> bool {
        use Action::*;
        matches!(
            self,
            NextDifference | PreviousDifference | NextDiffFile | PreviousDiffFile
        )
    }

    /// 是否为只在编辑 Markdown 时可用的命令：行内格式、段落类型、插入块和链接，以及专注、
    /// 打字机模式。
    pub fn markdown_only(self) -> bool {
        use Action::*;
        matches!(
            self,
            MarkdownBold
                | MarkdownItalic
                | MarkdownCode
                | MarkdownStrikethrough
                | MarkdownHighlight
                | MarkdownHeading1
                | MarkdownHeading2
                | MarkdownHeading3
                | MarkdownHeading4
                | MarkdownHeading5
                | MarkdownHeading6
                | MarkdownParagraph
                | MarkdownQuote
                | MarkdownOrderedList
                | MarkdownBulletList
                | MarkdownCodeBlock
                | MarkdownMathBlock
                | MarkdownTable
                | MarkdownLink
                | MarkdownImage
                | MarkdownFocusMode
                | MarkdownTypewriterMode
                | MarkdownPastePlain
        )
    }

    /// 光标在差异标签右侧时，是否把命令交给右侧对应的源文档执行。
    ///
    /// 「编辑」分组的命令默认都交给右侧，新加的编辑命令在差异里也能直接用；另外加上保存和
    /// 「选中全部匹配」。排除的是不改动文字、只作用于编辑器视图或文件设置的命令：折叠与书签
    /// 只对普通编辑器的视图有意义；缩进方式、缩进转换、检测缩进、换行符和语言模式改的是整份
    /// 文件的设置，不在差异里执行。
    pub fn edits_in_diff(self) -> bool {
        use Action::*;
        match self {
            Save | SelectAllMatches => true,
            Fold
            | Unfold
            | ToggleFold
            | FoldRecursively
            | UnfoldRecursively
            | FoldAll
            | UnfoldAll
            | FoldLevel1
            | FoldLevel2
            | FoldLevel3
            | FoldLevel4
            | FoldLevel5
            | FoldLevel6
            | FoldLevel7
            | ToggleBookmark
            | NextBookmark
            | PreviousBookmark
            | ListBookmarks
            | ClearBookmarks
            | IndentUsingSpaces
            | IndentUsingTabs
            | ConvertIndentationToSpaces
            | ConvertIndentationToTabs
            | DetectIndentation
            | ChangeEndOfLine
            | ChangeLanguageMode
            | MarkdownBold
            | MarkdownItalic
            | MarkdownCode
            | MarkdownStrikethrough
            | MarkdownHighlight
            | MarkdownHeading1
            | MarkdownHeading2
            | MarkdownHeading3
            | MarkdownHeading4
            | MarkdownHeading5
            | MarkdownHeading6
            | MarkdownParagraph
            | MarkdownQuote
            | MarkdownOrderedList
            | MarkdownBulletList
            | MarkdownCodeBlock
            | MarkdownMathBlock
            | MarkdownTable
            | MarkdownLink
            | MarkdownImage
            | MarkdownFocusMode
            | MarkdownTypewriterMode
            | MarkdownPastePlain => false,
            _ => self.group() == Group::Edit,
        }
    }

    /// 焦点在终端里时，带 ⌘ 的全局快捷键是否还作用于这个命令。
    ///
    /// 编辑和查找的命令针对编辑区里的文档，在终端里按下不能改到看不见的文档，交给终端
    /// （粘贴由终端自己处理；复制、全选在终端里另有绑定）。
    pub fn reaches_terminal(self) -> bool {
        self == Action::Paste || !matches!(self.group(), Group::Edit | Group::Find)
    }

    /// 命令属于哪个可选模块；不属于任何模块的命令总是可用。
    pub fn module(self) -> Option<Module> {
        match self {
            Action::GitShowChanges
            | Action::ChangesShowDiff
            | Action::ChangesOpenSource
            | Action::ChangesRollback
            | Action::GitShowLog
            | Action::GitGoToRevision
            | Action::GitShowOutput
            | Action::GitClone
            | Action::GitInit
            | Action::GitSwitchRepository
            | Action::GitSwitchWorktree
            | Action::GitNewWorktree
            | Action::GitSwitchBranch
            | Action::GitNewBranch
            | Action::GitFetch
            | Action::GitPull
            | Action::GitPullRebase
            | Action::GitPush
            | Action::GitSync
            | Action::GitStash
            | Action::GitStageAll
            | Action::GitUnstageAll
            | Action::GitStageFile
            | Action::GitFileHistory
            | Action::GitSelectionHistory
            | Action::GitBlame
            | Action::ShowCommit
            | Action::GitRevertChange => Some(Module::Git),
            Action::ToggleTerminal
            | Action::ToggleTerminalPanel
            | Action::RunConfiguration
            | Action::StopConfiguration
            | Action::EditRunConfigurations
            | Action::ClearTerminal
            | Action::TerminalCopy
            | Action::TerminalPaste
            | Action::NewTerminal
            | Action::CloseTerminal
            | Action::NextTerminal
            | Action::PreviousTerminal
            | Action::SplitTerminal
            | Action::NextTerminalPane
            | Action::PreviousTerminalPane
            | Action::FocusTerminal1
            | Action::FocusTerminal2
            | Action::FocusTerminal3
            | Action::FocusTerminal4
            | Action::FocusTerminal5
            | Action::FocusTerminal6
            | Action::FocusTerminal7
            | Action::FocusTerminal8
            | Action::FocusTerminal9
            | Action::FindInTerminal
            | Action::MaximizeTerminal
            | Action::ScrollTerminalTop
            | Action::ScrollTerminalBottom
            | Action::ToggleTerminalPosition
            | Action::RenameTerminal
            | Action::PreviousCommand
            | Action::NextCommand
            | Action::FoldOutput
            | Action::UnfoldOutput
            | Action::NextError
            | Action::PreviousError
            | Action::SendToTerminal => Some(Module::Terminal),
            Action::RemoteConnect | Action::RemoteDisconnect => Some(Module::Remote),
            _ => None,
        }
    }

    /// 是否在命令面板中列出。
    ///
    /// 不列出的命令：命令面板自身（`ShowCommands`）、`RecordKeys`、剪贴板命令、`Quit`，以及
    /// 只在特定位置有意义、离开该位置就无法执行的命令（文件树中选中的项、查找条和搜索面板的
    /// 开关与翻页）。
    pub fn in_palette(self) -> bool {
        use Action::*;
        !matches!(
            self,
            ShowCommands
                | RecordKeys
                | Cut
                | Copy
                | Paste
                | TerminalCopy
                | TerminalPaste
                | FindNext
                | FindPrevious
                | ReplaceOne
                | ReplaceAll
                | SelectAllMatches
                | ColumnSelectUp
                | ColumnSelectDown
                | ColumnSelectLeft
                | ColumnSelectRight
                | ToggleCase
                | ToggleWord
                | ToggleRegex
                | ReplaceAllInFiles
                | NextResult
                | PreviousResult
                | RenameFile
                | DeleteFile
                | DeleteFilePermanently
                | OpenSelected
                | PreviewSelected
                | FindInFolder
                | Quit
        )
    }
}

/// 全部命令。
///
/// 命令面板按此顺序列出（最常用的在前），「快捷键」页先分组、再按此顺序排列。
pub const ACTIONS: &[Action] = {
    use Action::*;
    &[
        QuickOpen,
        GoToSymbol,
        GoToLine,
        GoToDefinition,
        FindReferences,
        GoBack,
        GoForward,
        ShowCommands,
        Settings,
        ReleaseNotes,
        KeyboardShortcuts,
        RecordKeys,
        Format,
        FormatSelection,
        Fix,
        Find,
        Replace,
        FindNext,
        FindPrevious,
        HighlightWord,
        ClearWordHighlights,
        ReplaceOne,
        ReplaceAll,
        SelectAllMatches,
        ToggleCase,
        ToggleWord,
        ToggleRegex,
        FilterLines,
        FindInFiles,
        ReplaceInFiles,
        ReplaceAllInFiles,
        NextResult,
        PreviousResult,
        NewFile,
        NewWindow,
        OpenFile,
        OpenFolder,
        AddFolder,
        RemoteConnect,
        RemoteDisconnect,
        OpenRecent,
        Save,
        SaveAs,
        CloseTab,
        CloseOtherTabs,
        CloseAllTabs,
        ReopenClosed,
        NextTab,
        PreviousTab,
        SplitRight,
        SplitDown,
        MoveToSplitRight,
        MoveToSplitDown,
        Unsplit,
        NextPane,
        PreviousPane,
        CloseFolder,
        CloseWindow,
        CopyPath,
        CopyRelativePath,
        CopyReference,
        CopyAbsoluteReference,
        RevealInFinder,
        CopyActivePath,
        CopyKeyPath,
        RevealActiveFile,
        OpenInTerminal,
        Quit,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        SelectAll,
        ToggleComment,
        MarkdownBold,
        MarkdownItalic,
        MarkdownCode,
        MarkdownStrikethrough,
        MarkdownHighlight,
        MarkdownHeading1,
        MarkdownHeading2,
        MarkdownHeading3,
        MarkdownHeading4,
        MarkdownHeading5,
        MarkdownHeading6,
        MarkdownParagraph,
        MarkdownQuote,
        MarkdownOrderedList,
        MarkdownBulletList,
        MarkdownCodeBlock,
        MarkdownMathBlock,
        MarkdownTable,
        MarkdownLink,
        MarkdownImage,
        MarkdownFocusMode,
        MarkdownTypewriterMode,
        MarkdownPastePlain,
        DeleteLine,
        DeleteAllLeft,
        IndentLines,
        OutdentLines,
        MoveLinesUp,
        MoveLinesDown,
        CopyLinesUp,
        CopyLinesDown,
        JoinLines,
        ExpandSelection,
        ShrinkSelection,
        AddCursorAbove,
        AddCursorBelow,
        AddNextOccurrence,
        SkipToNextOccurrence,
        SelectAllOccurrences,
        RemoveSecondaryCursors,
        AddCursorsToLineEnds,
        CursorUndo,
        CursorRedo,
        ColumnSelectUp,
        ColumnSelectDown,
        ColumnSelectLeft,
        ColumnSelectRight,
        JumpToBracket,
        TransformToUppercase,
        TransformToLowercase,
        TransformToTitlecase,
        SortLinesAscending,
        SortLinesDescending,
        TrimTrailingWhitespace,
        IndentUsingSpaces,
        IndentUsingTabs,
        ConvertIndentationToSpaces,
        ConvertIndentationToTabs,
        DetectIndentation,
        ChangeEndOfLine,
        ChangeLanguageMode,
        Fold,
        Unfold,
        ToggleFold,
        FoldRecursively,
        UnfoldRecursively,
        FoldAll,
        UnfoldAll,
        FoldLevel1,
        FoldLevel2,
        FoldLevel3,
        FoldLevel4,
        FoldLevel5,
        FoldLevel6,
        FoldLevel7,
        ToggleBookmark,
        NextBookmark,
        PreviousBookmark,
        ListBookmarks,
        ClearBookmarks,
        RenameFile,
        DeleteFile,
        DeleteFilePermanently,
        OpenSelected,
        PreviewSelected,
        FindInFolder,
        ToggleSidebar,
        FocusSidebar,
        ShowFiles,
        ShowOutline,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ThemeAuto,
        ThemeLight,
        ThemeDark,
        ToggleStickyScroll,
        ToggleVimMode,
        ToggleWordWrap,
        ToggleMinimap,
        ToggleLineNumbers,
        ToggleStatusBar,
        ToggleWhitespace,
        TogglePreview,
        ToggleReadOnly,
        ToggleReadOnlyAll,
        ShowCommit,
        GitShowChanges,
        ChangesShowDiff,
        ChangesOpenSource,
        ChangesRollback,
        GitShowLog,
        GitGoToRevision,
        GitShowOutput,
        GitClone,
        GitInit,
        GitSwitchRepository,
        GitSwitchWorktree,
        GitNewWorktree,
        GitSwitchBranch,
        GitNewBranch,
        GitFetch,
        GitPull,
        GitPullRebase,
        GitPush,
        GitSync,
        GitStash,
        GitStageAll,
        GitUnstageAll,
        GitStageFile,
        GitFileHistory,
        GitSelectionHistory,
        GitBlame,
        GitRevertChange,
        ToggleTerminal,
        ToggleTerminalPanel,
        EnableTerminal,
        RunConfiguration,
        StopConfiguration,
        EditRunConfigurations,
        NewTerminal,
        ClearTerminal,
        TerminalCopy,
        TerminalPaste,
        FindInTerminal,
        MaximizeTerminal,
        ScrollTerminalTop,
        ScrollTerminalBottom,
        ToggleTerminalPosition,
        InstallShellCommand,
        UninstallShellCommand,
        RenameTerminal,
        CloseTerminal,
        NextTerminal,
        PreviousTerminal,
        SplitTerminal,
        NextTerminalPane,
        PreviousTerminalPane,
        FocusTerminal1,
        FocusTerminal2,
        FocusTerminal3,
        FocusTerminal4,
        FocusTerminal5,
        FocusTerminal6,
        FocusTerminal7,
        FocusTerminal8,
        FocusTerminal9,
        PreviousCommand,
        NextCommand,
        FoldOutput,
        UnfoldOutput,
        NextError,
        PreviousError,
        SendToTerminal,
        NextDifference,
        PreviousDifference,
        NextDiffFile,
        PreviousDiffFile,
        ToggleDiffLayout,
    ]
};
