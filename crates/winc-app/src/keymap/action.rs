//! 命令：显示名、英文名、分组、所属模块、能否在差异和终端里执行，以及是否在命令面板中列出。

use winc_i18n::t;

use super::*;

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
    /// 可编辑的 Markdown 预览中切换粗体（`**`）。
    MarkdownBold,
    /// 可编辑的 Markdown 预览中切换斜体（`*`）。
    MarkdownItalic,
    /// 可编辑的 Markdown 预览中切换行内代码（`` ` ``）。
    MarkdownCode,
    /// 可编辑的 Markdown 预览中切换删除线（`~~`）。
    MarkdownStrikethrough,
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
    NextDifference,
    PreviousDifference,
    /// 差异里的下一个文件。
    NextDiffFile,
    /// 差异里的上一个文件。
    PreviousDiffFile,
    /// 切换差异的单栏显示和并排显示。
    ToggleDiffLayout,
    GitShowChanges,
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
    GitFileHistory,
    GitBlame,
    ShowCommit,
    GitRevertChange,
    ToggleTerminal,
    /// 运行当前选中的配置；运行中时重新运行。
    RunConfiguration,
    /// 停止当前选中的配置。
    StopConfiguration,
    /// 打开运行配置文件。
    EditRunConfigurations,
    ClearTerminal,
    NewTerminal,
    CloseTerminal,
    NextTerminal,
    PreviousTerminal,
    FindInTerminal,
    MaximizeTerminal,
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
            GitFileHistory => t!("cmd-git-file-history"),
            GitBlame => t!("cmd-git-blame"),
            ShowCommit => t!("cmd-show-commit"),
            GitRevertChange => t!("cmd-git-revert-change"),
            ToggleTerminal => t!("cmd-toggle-terminal"),
            RunConfiguration => t!("cmd-run-configuration"),
            StopConfiguration => t!("cmd-stop-configuration"),
            EditRunConfigurations => t!("cmd-edit-run-configurations"),
            ClearTerminal => t!("cmd-clear-terminal"),
            NewTerminal => t!("cmd-new-terminal"),
            CloseTerminal => t!("cmd-close-terminal"),
            NextTerminal => t!("cmd-next-terminal"),
            PreviousTerminal => t!("cmd-previous-terminal"),
            FindInTerminal => t!("cmd-find-in-terminal"),
            MaximizeTerminal => t!("cmd-maximize-terminal"),
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
        }
    }

    /// 返回命令的英文名；在命令面板中输入英文也能搜到该命令。
    pub fn english(self) -> &'static str {
        use Action::*;
        match self {
            ShowCommands => "Show All Commands",
            QuickOpen => "Go to File",
            GoToSymbol => "Go to Symbol in Editor",
            GoToLine => "Go to Line",
            GoToDefinition => "Go to Definition",
            FindReferences => "Find All References",
            GoBack => "Go Back",
            GoForward => "Go Forward",
            Settings => "Settings",
            ReleaseNotes => "Release Notes",
            KeyboardShortcuts => "Keyboard Shortcuts",
            RecordKeys => "Record Keys",
            NewFile => "New File",
            NewWindow => "New Window",
            OpenFile => "Open",
            OpenFolder => "Open Folder",
            RemoteConnect => "Remote: Connect to SSH Host…",
            RemoteDisconnect => "Remote: Disconnect",
            OpenRecent => "Open Recent Folder",
            Save => "Save",
            SaveAs => "Save As",
            CloseTab => "Close Editor",
            CloseOtherTabs => "Close Other Editors",
            CloseAllTabs => "Close All Editors",
            ReopenClosed => "Reopen Closed Editor",
            NextTab => "Open Next Editor",
            PreviousTab => "Open Previous Editor",
            CloseFolder => "Close Folder",
            CloseWindow => "Close Window",
            CopyPath => "Copy Path",
            CopyRelativePath => "Copy Relative Path",
            CopyReference => "Copy Reference",
            CopyAbsoluteReference => "Copy Absolute Reference",
            RevealInFinder => "Reveal in File Explorer",
            CopyActivePath => "Copy Path of Active File",
            CopyKeyPath => "Copy Key Path",
            RevealActiveFile => "Reveal Active File",
            OpenInTerminal => "Open in External Terminal",
            Quit => "Quit",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            SelectAll => "Select All",
            ToggleComment => "Toggle Line Comment",
            MarkdownBold => "Markdown: Toggle Bold",
            MarkdownItalic => "Markdown: Toggle Italic",
            MarkdownCode => "Markdown: Toggle Inline Code",
            MarkdownStrikethrough => "Markdown: Toggle Strikethrough",
            DeleteLine => "Delete Line",
            DeleteAllLeft => "Delete All Left",
            IndentLines => "Indent Line",
            OutdentLines => "Outdent Line",
            MoveLinesUp => "Move Line Up",
            MoveLinesDown => "Move Line Down",
            CopyLinesUp => "Copy Line Up",
            CopyLinesDown => "Copy Line Down",
            JoinLines => "Join Lines",
            ExpandSelection => "Expand Selection",
            ShrinkSelection => "Shrink Selection",
            AddCursorAbove => "Add Cursor Above",
            AddCursorBelow => "Add Cursor Below",
            AddNextOccurrence => "Add Selection to Next Find Match",
            SkipToNextOccurrence => "Move Last Selection to Next Find Match",
            SelectAllOccurrences => "Select All Occurrences of Find Match",
            RemoveSecondaryCursors => "Remove Secondary Cursors",
            AddCursorsToLineEnds => "Add Cursors to Line Ends",
            CursorUndo => "Cursor Undo",
            CursorRedo => "Cursor Redo",
            ColumnSelectUp => "Column Select Up",
            ColumnSelectDown => "Column Select Down",
            ColumnSelectLeft => "Column Select Left",
            ColumnSelectRight => "Column Select Right",
            JumpToBracket => "Go to Bracket",
            TransformToUppercase => "Transform to Uppercase",
            TransformToLowercase => "Transform to Lowercase",
            TransformToTitlecase => "Transform to Title Case",
            SortLinesAscending => "Sort Lines Ascending",
            SortLinesDescending => "Sort Lines Descending",
            TrimTrailingWhitespace => "Trim Trailing Whitespace",
            IndentUsingSpaces => "Indent Using Spaces",
            IndentUsingTabs => "Indent Using Tabs",
            ConvertIndentationToSpaces => "Convert Indentation to Spaces",
            ConvertIndentationToTabs => "Convert Indentation to Tabs",
            DetectIndentation => "Detect Indentation from Content",
            ChangeEndOfLine => "Change End of Line Sequence",
            ChangeLanguageMode => "Change Language Mode",
            Fold => "Fold",
            Unfold => "Unfold",
            ToggleFold => "Toggle Fold",
            FoldRecursively => "Fold Recursively",
            UnfoldRecursively => "Unfold Recursively",
            FoldAll => "Fold All",
            UnfoldAll => "Unfold All",
            FoldLevel1 => "Fold Level 1",
            FoldLevel2 => "Fold Level 2",
            FoldLevel3 => "Fold Level 3",
            FoldLevel4 => "Fold Level 4",
            FoldLevel5 => "Fold Level 5",
            FoldLevel6 => "Fold Level 6",
            FoldLevel7 => "Fold Level 7",
            ToggleBookmark => "Toggle Bookmark",
            NextBookmark => "Next Bookmark",
            PreviousBookmark => "Previous Bookmark",
            ListBookmarks => "List Bookmarks",
            ClearBookmarks => "Clear Bookmarks",
            Format => "Format Document",
            FormatSelection => "Format Selection",
            Fix => "Fix All Problems",
            Find => "Find",
            Replace => "Replace",
            FindNext => "Find Next",
            FindPrevious => "Find Previous",
            HighlightWord => "Toggle Word Highlight",
            ClearWordHighlights => "Clear Word Highlights",
            ReplaceOne => "Replace",
            ReplaceAll => "Replace All",
            SelectAllMatches => "Select All Matches",
            ToggleCase => "Toggle Case Sensitive",
            ToggleWord => "Toggle Whole Word",
            ToggleRegex => "Toggle Regex",
            FilterLines => "Show Only Matching Lines",
            FindInFiles => "Find in Files",
            ReplaceInFiles => "Replace in Files",
            ReplaceAllInFiles => "Replace All in Files",
            NextResult => "Focus Next Search Result",
            PreviousResult => "Focus Previous Search Result",
            RenameFile => "Rename",
            DeleteFile => "Delete",
            OpenSelected => "Open",
            PreviewSelected => "Open and Keep Focus",
            FindInFolder => "Find in Folder",
            ToggleSidebar => "Toggle Sidebar",
            FocusSidebar => "Focus Sidebar",
            ShowFiles => "Show Explorer",
            ShowOutline => "Show Outline",
            IncreaseFontSize => "Increase Font Size",
            DecreaseFontSize => "Decrease Font Size",
            ResetFontSize => "Reset Font Size",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ResetZoom => "Reset Zoom",
            ThemeAuto => "Theme: Auto",
            ThemeLight => "Theme: Light",
            ThemeDark => "Theme: Dark",
            ToggleStickyScroll => "Toggle Editor Sticky Scroll",
            ToggleVimMode => "Toggle Vim Mode",
            ToggleWordWrap => "Toggle Word Wrap",
            ToggleMinimap => "Toggle Minimap",
            ToggleLineNumbers => "Toggle Line Numbers",
            ToggleStatusBar => "Toggle Status Bar",
            ToggleWhitespace => "Toggle Render Whitespace",
            NextDifference => "Go to Next Difference",
            PreviousDifference => "Go to Previous Difference",
            NextDiffFile => "Go to Next File in Diff",
            PreviousDiffFile => "Go to Previous File in Diff",
            ToggleDiffLayout => "Toggle Inline Diff",
            GitShowChanges => "Git: Open Changes",
            GitShowLog => "Git: Show Log",
            GitGoToRevision => "Git: Go to Hash, Branch or Tag",
            GitShowOutput => "Git: Show Output",
            GitClone => "Git: Clone Repository…",
            GitInit => "Git: Initialize Repository",
            GitSwitchRepository => "Git: Switch Repository",
            GitSwitchWorktree => "Git: Switch Worktree…",
            GitNewWorktree => "Git: New Worktree…",
            GitSwitchBranch => "Git: Switch Branch…",
            GitNewBranch => "Git: New Branch…",
            GitFetch => "Git: Fetch",
            GitPull => "Git: Pull",
            GitPullRebase => "Git: Pull with Rebase",
            GitPush => "Git: Push",
            GitSync => "Git: Sync",
            GitStash => "Git: Stash Changes…",
            GitFileHistory => "Git: Show File History",
            GitBlame => "Git: Toggle Blame",
            ShowCommit => "Git: Commit",
            GitRevertChange => "Git: Revert Change",
            ToggleTerminal => "Toggle Terminal",
            RunConfiguration => "Run Configuration",
            StopConfiguration => "Stop Configuration",
            EditRunConfigurations => "Edit Run Configurations",
            ClearTerminal => "Terminal: Clear",
            NewTerminal => "New Terminal",
            CloseTerminal => "Terminal: Close Tab",
            NextTerminal => "Terminal: Next Tab",
            PreviousTerminal => "Terminal: Previous Tab",
            FindInTerminal => "Terminal: Find",
            MaximizeTerminal => "Terminal: Maximize Panel",
            ToggleTerminalPosition => "Terminal: Toggle Panel Position",
            InstallShellCommand => "Shell Command: Install winc Command",
            UninstallShellCommand => "Shell Command: Uninstall winc Command",
            RenameTerminal => "Terminal: Rename",
            PreviousCommand => "Terminal: Previous Command",
            NextCommand => "Terminal: Next Command",
            FoldOutput => "Terminal: Fold Command Output",
            UnfoldOutput => "Terminal: Unfold Command Output",
            NextError => "Terminal: Next Error",
            PreviousError => "Terminal: Previous Error",
            SendToTerminal => "Terminal: Run Selected Text",
            TogglePreview => "Toggle Preview",
        }
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
            NewFile | OpenFile | OpenFolder | RemoteConnect | RemoteDisconnect | Save | SaveAs
            | CloseTab | CloseOtherTabs | CloseAllTabs | ReopenClosed | NextTab | PreviousTab
            | CloseFolder | CloseWindow | CopyPath | CopyRelativePath | RevealInFinder
            | CopyActivePath | CopyKeyPath | RevealActiveFile | OpenInTerminal | Quit => {
                Group::File
            }
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
            RenameFile | DeleteFile | OpenSelected | PreviewSelected | FindInFolder => Group::Tree,
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
            | NextDifference
            | PreviousDifference
            | NextDiffFile
            | PreviousDiffFile
            | ToggleDiffLayout
            | GitShowChanges
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
            | GitFileHistory
            | GitBlame
            | ShowCommit
            | GitRevertChange
            | ToggleTerminal
            | RunConfiguration
            | StopConfiguration
            | EditRunConfigurations
            | ClearTerminal
            | NewTerminal
            | CloseTerminal
            | NextTerminal
            | PreviousTerminal
            | FindInTerminal
            | MaximizeTerminal
            | ToggleTerminalPosition
            | RenameTerminal
            | PreviousCommand
            | NextCommand
            | FoldOutput
            | UnfoldOutput
            | NextError
            | PreviousError
            | SendToTerminal => Group::View,
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
            | MarkdownStrikethrough => false,
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
            | Action::GitFileHistory
            | Action::GitBlame
            | Action::ShowCommit
            | Action::GitRevertChange => Some(Module::Git),
            Action::ToggleTerminal
            | Action::RunConfiguration
            | Action::StopConfiguration
            | Action::EditRunConfigurations
            | Action::ClearTerminal
            | Action::NewTerminal
            | Action::CloseTerminal
            | Action::NextTerminal
            | Action::PreviousTerminal
            | Action::FindInTerminal
            | Action::MaximizeTerminal
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
        ShowCommit,
        GitShowChanges,
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
        GitFileHistory,
        GitBlame,
        GitRevertChange,
        ToggleTerminal,
        RunConfiguration,
        StopConfiguration,
        EditRunConfigurations,
        NewTerminal,
        ClearTerminal,
        FindInTerminal,
        MaximizeTerminal,
        ToggleTerminalPosition,
        InstallShellCommand,
        UninstallShellCommand,
        RenameTerminal,
        CloseTerminal,
        NextTerminal,
        PreviousTerminal,
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
