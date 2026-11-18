//! 内置的键位方案：IDEA 和 VS Code，取各自在三个系统上的默认快捷键。
//!
//! 每张表一行是一个命令在一个生效范围内的键，分 macOS 和其他系统两列，Linux 上与 Windows
//! 不同的键另列一张表。两套方案都没有的命令（终端面板内的切换、查找条的开关等）的键写在
//! [`COMMON`]，两套方案共用。

use super::*;

/// 内置的键位方案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// IDEA 的默认快捷键。
    #[default]
    Idea,
    /// VS Code 的默认快捷键。
    VsCode,
}

impl Preset {
    /// 全部方案，按设置中列出的顺序。
    pub const ALL: [Preset; 2] = [Preset::Idea, Preset::VsCode];

    /// 返回配置文件中的写法。
    pub fn key(self) -> &'static str {
        match self {
            Preset::Idea => "idea",
            Preset::VsCode => "vscode",
        }
    }

    /// 按配置文件中的写法 `key` 返回方案；不认识时返回 `None`。
    pub fn from_key(key: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|preset| preset.key() == key)
    }

    /// 返回方案的名称。
    pub fn title(self) -> &'static str {
        match self {
            Preset::Idea => "IntelliJ IDEA",
            Preset::VsCode => "VS Code",
        }
    }

    /// 返回方案在 `os` 上的整张绑定表，生效范围小的绑定在前（见 [`lookup`]）。
    pub fn bindings(self, os: Os) -> Vec<Binding> {
        let (table, linux) = match self {
            Preset::Idea => (IDEA, IDEA_LINUX),
            Preset::VsCode => (VSCODE, VSCODE_LINUX),
        };
        let mut out = Vec::new();
        for (rows, linux) in [(COMMON, COMMON_LINUX), (table, linux)] {
            for row in rows {
                let keys = match os {
                    Os::Mac => row.mac,
                    Os::Linux => linux
                        .iter()
                        .find(|l| l.action == row.action && l.when == row.when)
                        .map_or(row.pc, |l| l.pc),
                    Os::Windows => row.pc,
                };
                for key in keys {
                    let parsed = parse_spec(key);
                    debug_assert!(parsed.is_some(), "{key}");
                    if let Some((first, second)) = parsed {
                        out.push(Binding {
                            action: row.action,
                            first,
                            second,
                            when: row.when,
                        });
                    }
                }
            }
        }
        out.sort_by_key(|b| b.when.breadth());
        out
    }
}

/// 方案中的一行：命令 `action` 在 `when` 范围内的键，`mac` 是 macOS 上的，`pc` 是其他系统上的。
pub(super) struct PresetRow {
    action: Action,
    when: When,
    mac: &'static [&'static str],
    pc: &'static [&'static str],
}

/// 返回方案中的一行（见 [`PresetRow`]）。
const fn row(
    action: Action,
    when: When,
    mac: &'static [&'static str],
    pc: &'static [&'static str],
) -> PresetRow {
    PresetRow {
        action,
        when,
        mac,
        pc,
    }
}

use Action::*;
use When::*;

/// 两套方案共用的键：终端面板、查找条和搜索面板的开关、「快捷键」页。
pub(super) const COMMON: &[PresetRow] = &[
    row(FocusTerminal1, Terminal, &["ctrl+1"], &["ctrl+shift+1"]),
    row(FocusTerminal2, Terminal, &["ctrl+2"], &["ctrl+shift+2"]),
    row(FocusTerminal3, Terminal, &["ctrl+3"], &["ctrl+shift+3"]),
    row(FocusTerminal4, Terminal, &["ctrl+4"], &["ctrl+shift+4"]),
    row(FocusTerminal5, Terminal, &["ctrl+5"], &["ctrl+shift+5"]),
    row(FocusTerminal6, Terminal, &["ctrl+6"], &["ctrl+shift+6"]),
    row(FocusTerminal7, Terminal, &["ctrl+7"], &["ctrl+shift+7"]),
    row(FocusTerminal8, Terminal, &["ctrl+8"], &["ctrl+shift+8"]),
    row(FocusTerminal9, Terminal, &["ctrl+9"], &["ctrl+shift+9"]),
    row(NewTerminal, Terminal, &["cmd+t"], &["ctrl+shift+t"]),
    row(CloseTerminal, Terminal, &["cmd+w"], &["ctrl+shift+w"]),
    row(FindInTerminal, Terminal, &["cmd+f"], &["ctrl+shift+f"]),
    row(
        ScrollTerminalTop,
        Terminal,
        &["cmd+home"],
        &["ctrl+shift+home"],
    ),
    row(
        ScrollTerminalBottom,
        Terminal,
        &["cmd+end"],
        &["ctrl+shift+end"],
    ),
    row(Copy, Terminal, &["cmd+c"], &["ctrl+shift+c"]),
    row(SelectAll, Terminal, &["cmd+a"], &["ctrl+shift+a"]),
    row(NextTerminal, Terminal, &["shift+cmd+]"], &["ctrl+shift+]"]),
    row(
        PreviousTerminal,
        Terminal,
        &["shift+cmd+["],
        &["ctrl+shift+["],
    ),
    row(PreviousCommand, Terminal, &["cmd+up"], &["ctrl+up"]),
    row(NextCommand, Terminal, &["cmd+down"], &["ctrl+down"]),
    row(FoldOutput, Terminal, &["alt+cmd+["], &[]),
    row(UnfoldOutput, Terminal, &["alt+cmd+]"], &[]),
    row(RecordKeys, KeysPage, &["alt+cmd+k"], &["alt+k"]),
    row(PreviewSelected, Tree, &["space"], &["space"]),
    row(ToggleCase, Panel, &["alt+cmd+c"], &["alt+c"]),
    row(ToggleWord, Panel, &["alt+cmd+w"], &["alt+w"]),
    row(ToggleRegex, Panel, &["alt+cmd+r"], &["alt+r"]),
    row(ToggleCase, EditorOrFind, &["alt+cmd+c"], &["alt+c"]),
    row(ToggleWord, EditorOrFind, &["alt+cmd+w"], &["alt+w"]),
    row(ToggleRegex, EditorOrFind, &["alt+cmd+r"], &["alt+r"]),
    row(
        ReplaceAllInFiles,
        Panel,
        &["alt+cmd+enter"],
        &["ctrl+alt+enter"],
    ),
    row(
        ReplaceAll,
        EditorOrFind,
        &["alt+cmd+enter"],
        &["ctrl+alt+enter"],
    ),
    row(ReplaceAll, ReplaceInput, &["cmd+enter"], &[]),
    row(
        ReplaceOne,
        EditorOrFind,
        &["shift+cmd+1"],
        &["ctrl+shift+1"],
    ),
    row(
        SelectAllMatches,
        EditorOrFind,
        &["alt+enter"],
        &["alt+enter"],
    ),
    row(ClearTerminal, Terminal, &["cmd+k"], &["ctrl+shift+k"]),
    row(Paste, Terminal, &[], &["ctrl+shift+v"]),
];

/// 共用的键在 Linux 上与 Windows 不同的部分。
pub(super) const COMMON_LINUX: &[PresetRow] = &[];

/// IDEA 方案。
pub(super) const IDEA: &[PresetRow] = &[
    row(MarkdownBold, Markdown, &["cmd+b"], &["ctrl+b"]),
    row(MarkdownItalic, Markdown, &["cmd+i"], &["ctrl+i"]),
    row(MarkdownCode, Markdown, &["shift+cmd+c"], &["ctrl+shift+c"]),
    row(
        MarkdownStrikethrough,
        Markdown,
        &["shift+cmd+s"],
        &["ctrl+shift+s"],
    ),
    row(RenameFile, Tree, &["shift+f6"], &["shift+f6"]),
    row(
        DeleteFile,
        Tree,
        &["delete", "backspace", "cmd+backspace"],
        &["delete"],
    ),
    row(
        OpenSelected,
        Tree,
        &["enter", "f4", "cmd+down"],
        &["enter", "f4"],
    ),
    row(FindInFolder, Tree, &["shift+cmd+f"], &["ctrl+shift+f"]),
    row(Copy, Tree, &["cmd+c"], &["ctrl+c"]),
    row(ChangesShowDiff, Changes, &["cmd+d"], &["ctrl+d"]),
    row(ChangesOpenSource, Changes, &["f4", "cmd+down"], &["f4"]),
    row(ChangesRollback, Changes, &["alt+cmd+z"], &["ctrl+alt+z"]),
    row(
        MaximizeTerminal,
        Terminal,
        &["shift+cmd+'"],
        &["ctrl+shift+'"],
    ),
    row(NextError, Terminal, &["f2"], &["f2"]),
    row(PreviousError, Terminal, &["shift+f2"], &["shift+f2"]),
    row(
        QuickOpen,
        Global,
        &["shift+cmd+o", "shift shift"],
        &["ctrl+shift+n", "shift shift"],
    ),
    row(GoToSymbol, Global, &["cmd+f12"], &["ctrl+f12"]),
    row(GoToLine, Global, &["cmd+l"], &["ctrl+g"]),
    row(GoToDefinition, Editor, &["cmd+b"], &["ctrl+b"]),
    row(FindReferences, Editor, &["alt+f7"], &["alt+f7"]),
    row(
        GoBack,
        Global,
        &["alt+cmd+left", "cmd+["],
        &["ctrl+alt+left"],
    ),
    row(
        GoForward,
        Global,
        &["alt+cmd+right", "cmd+]"],
        &["ctrl+alt+right"],
    ),
    row(ShowCommands, Global, &["shift+cmd+a"], &["ctrl+shift+a"]),
    row(Settings, Global, &["cmd+,"], &["ctrl+alt+s"]),
    row(Format, Editor, &["alt+cmd+l"], &["ctrl+alt+l"]),
    row(Undo, Editor, &["cmd+z"], &["ctrl+z"]),
    row(Redo, Editor, &["shift+cmd+z"], &["ctrl+shift+z"]),
    row(Cut, Editor, &["cmd+x"], &["ctrl+x", "shift+delete"]),
    row(Copy, Editor, &["cmd+c"], &["ctrl+c", "ctrl+insert"]),
    row(SelectAll, Editor, &["cmd+a"], &["ctrl+a"]),
    row(ToggleComment, Editor, &["cmd+/"], &["ctrl+/"]),
    row(DeleteLine, Editor, &["cmd+backspace"], &["ctrl+y"]),
    row(MoveLinesUp, Editor, &["shift+alt+up"], &["shift+alt+up"]),
    row(
        MoveLinesDown,
        Editor,
        &["shift+alt+down"],
        &["shift+alt+down"],
    ),
    row(CopyLinesDown, Editor, &["cmd+d"], &["ctrl+d"]),
    row(JoinLines, Editor, &["ctrl+shift+j"], &["ctrl+shift+j"]),
    row(ExpandSelection, Editor, &["alt+up"], &["ctrl+w"]),
    row(ShrinkSelection, Editor, &["alt+down"], &["ctrl+shift+w"]),
    row(AddNextOccurrence, Editor, &["ctrl+g"], &["alt+j"]),
    row(
        SelectAllOccurrences,
        Editor,
        &["ctrl+cmd+g"],
        &["ctrl+shift+alt+j"],
    ),
    row(RemoveSecondaryCursors, Editor, &["escape"], &["escape"]),
    row(ClearWordHighlights, Editor, &["escape"], &["escape"]),
    row(
        AddCursorsToLineEnds,
        Editor,
        &["shift+alt+g"],
        &["shift+alt+g"],
    ),
    row(CursorUndo, Editor, &["ctrl+shift+g"], &["shift+alt+j"]),
    row(JumpToBracket, Editor, &["ctrl+m"], &["ctrl+shift+m"]),
    row(Fold, Editor, &["cmd+-"], &["ctrl+-"]),
    row(Unfold, Editor, &["cmd+="], &["ctrl+="]),
    row(FoldRecursively, Editor, &["alt+cmd+-"], &["ctrl+alt+-"]),
    row(UnfoldRecursively, Editor, &["alt+cmd+="], &["ctrl+alt+="]),
    row(FoldAll, Editor, &["shift+cmd+-"], &["ctrl+shift+-"]),
    row(UnfoldAll, Editor, &["shift+cmd+="], &["ctrl+shift+="]),
    row(
        FoldLevel1,
        Editor,
        &["alt+cmd+numpad_multiply 1"],
        &["ctrl+shift+numpad_multiply 1"],
    ),
    row(
        FoldLevel2,
        Editor,
        &["alt+cmd+numpad_multiply 2"],
        &["ctrl+shift+numpad_multiply 2"],
    ),
    row(
        FoldLevel3,
        Editor,
        &["alt+cmd+numpad_multiply 3"],
        &["ctrl+shift+numpad_multiply 3"],
    ),
    row(
        FoldLevel4,
        Editor,
        &["alt+cmd+numpad_multiply 4"],
        &["ctrl+shift+numpad_multiply 4"],
    ),
    row(
        FoldLevel5,
        Editor,
        &["alt+cmd+numpad_multiply 5"],
        &["ctrl+shift+numpad_multiply 5"],
    ),
    row(ToggleBookmark, Editor, &["f3"], &["f11"]),
    row(ListBookmarks, Editor, &["cmd+f3"], &["shift+f11"]),
    row(
        CopyReference,
        Editor,
        &["shift+alt+cmd+c"],
        &["ctrl+shift+alt+c"],
    ),
    row(HighlightWord, Editor, &["shift+cmd+f7"], &["ctrl+shift+f7"]),
    row(NextDifference, Editor, &["f7"], &["f7"]),
    row(PreviousDifference, Editor, &["shift+f7"], &["shift+f7"]),
    row(
        NextDiffFile,
        Editor,
        &["ctrl+shift+right"],
        &["shift+alt+right"],
    ),
    row(
        PreviousDiffFile,
        Editor,
        &["ctrl+shift+left"],
        &["shift+alt+left"],
    ),
    row(GitRevertChange, Editor, &["alt+cmd+z"], &["ctrl+alt+z"]),
    row(FindNext, EditorOrFind, &["cmd+g"], &["f3"]),
    row(FindPrevious, EditorOrFind, &["shift+cmd+g"], &["shift+f3"]),
    row(Find, Global, &["cmd+f"], &["ctrl+f"]),
    row(Replace, Global, &["cmd+r"], &["ctrl+r"]),
    row(FindInFiles, Global, &["shift+cmd+f"], &["ctrl+shift+f"]),
    row(ReplaceInFiles, Global, &["shift+cmd+r"], &["ctrl+shift+r"]),
    row(NextResult, Global, &["alt+cmd+down"], &["ctrl+alt+down"]),
    row(PreviousResult, Global, &["alt+cmd+up"], &["ctrl+alt+up"]),
    row(
        NewFile,
        Global,
        &["cmd+n", "shift+cmd+n"],
        &["alt+insert", "ctrl+shift+alt+insert"],
    ),
    row(Save, Global, &["cmd+s"], &["ctrl+s"]),
    row(CloseTab, Global, &["cmd+w"], &["ctrl+f4"]),
    row(
        NextTab,
        Global,
        &["ctrl+right", "shift+cmd+]"],
        &["alt+right"],
    ),
    row(
        PreviousTab,
        Global,
        &["ctrl+left", "shift+cmd+["],
        &["alt+left"],
    ),
    row(CopyPath, Global, &["shift+cmd+c"], &["ctrl+shift+c"]),
    row(Paste, Global, &["cmd+v"], &["ctrl+v", "shift+insert"]),
    row(Quit, Global, &["cmd+q"], &[]),
    row(
        ToggleSidebar,
        Global,
        &["shift+cmd+f12"],
        &["ctrl+shift+f12"],
    ),
    row(ShowFiles, Global, &["cmd+1"], &["alt+1"]),
    row(ShowOutline, Global, &["cmd+7"], &["alt+7"]),
    row(GitShowChanges, Global, &["cmd+0"], &["alt+0"]),
    row(ShowCommit, Global, &["cmd+k"], &["ctrl+k"]),
    row(GitShowLog, Global, &["cmd+9"], &["alt+9"]),
    row(GitPull, Global, &["cmd+t"], &["ctrl+t"]),
    row(GitPush, Global, &["shift+cmd+k"], &["ctrl+shift+k"]),
    row(ToggleTerminal, Everywhere, &["alt+f12"], &["alt+f12"]),
    row(RunConfiguration, Global, &["ctrl+r"], &["shift+f10"]),
    row(StopConfiguration, Global, &["cmd+f2"], &["ctrl+f2"]),
    row(
        IncreaseFontSize,
        Global,
        &["ctrl+shift+."],
        &["shift+alt+."],
    ),
    row(
        DecreaseFontSize,
        Global,
        &["ctrl+shift+,"],
        &["shift+alt+,"],
    ),
    row(ZoomIn, Everywhere, &["ctrl+alt+="], &["shift+alt+="]),
    row(ZoomOut, Everywhere, &["ctrl+alt+-"], &["shift+alt+-"]),
    row(ResetZoom, Everywhere, &["ctrl+alt+0"], &["shift+alt+0"]),
];

/// IDEA 方案在 Linux 上与 Windows 不同的键。
pub(super) const IDEA_LINUX: &[PresetRow] = &[
    row(ZoomIn, Everywhere, &[], &["ctrl+shift+alt+="]),
    row(ZoomOut, Everywhere, &[], &["ctrl+shift+alt+-"]),
    row(ResetZoom, Everywhere, &[], &["ctrl+shift+alt+0"]),
];

/// VS Code 方案。
pub(super) const VSCODE: &[PresetRow] = &[
    row(NextError, Terminal, &["f8"], &["f8"]),
    row(PreviousError, Terminal, &["shift+f8"], &["shift+f8"]),
    row(RenameFile, Tree, &["enter"], &["f2"]),
    row(DeleteFile, Tree, &["cmd+backspace", "delete"], &["delete"]),
    row(OpenSelected, Tree, &["cmd+down"], &["enter"]),
    row(FindInFolder, Tree, &["shift+alt+f"], &["shift+alt+f"]),
    row(FindNext, EditorOrFind, &["cmd+g", "f3"], &["f3"]),
    row(
        FindPrevious,
        EditorOrFind,
        &["shift+cmd+g", "shift+f3"],
        &["shift+f3"],
    ),
    row(GoToDefinition, Editor, &["f12"], &["f12"]),
    row(FindReferences, Editor, &["shift+f12"], &["shift+f12"]),
    row(Undo, Editor, &["cmd+z"], &["ctrl+z"]),
    row(Redo, Editor, &["shift+cmd+z"], &["ctrl+y", "ctrl+shift+z"]),
    row(Cut, Editor, &["cmd+x"], &["ctrl+x"]),
    row(Copy, Editor, &["cmd+c"], &["ctrl+c"]),
    row(Copy, Tree, &["cmd+c"], &["ctrl+c"]),
    row(SelectAll, Editor, &["cmd+a"], &["ctrl+a"]),
    row(ToggleComment, Editor, &["cmd+/"], &["ctrl+/"]),
    row(DeleteLine, Editor, &["shift+cmd+k"], &["ctrl+shift+k"]),
    row(DeleteAllLeft, Editor, &["cmd+backspace"], &[]),
    row(IndentLines, Editor, &["cmd+]"], &["ctrl+]"]),
    row(OutdentLines, Editor, &["cmd+["], &["ctrl+["]),
    row(MoveLinesUp, Editor, &["alt+up"], &["alt+up"]),
    row(MoveLinesDown, Editor, &["alt+down"], &["alt+down"]),
    row(CopyLinesUp, Editor, &["shift+alt+up"], &["shift+alt+up"]),
    row(
        CopyLinesDown,
        Editor,
        &["shift+alt+down"],
        &["shift+alt+down"],
    ),
    row(Format, Editor, &["shift+alt+f"], &["shift+alt+f"]),
    row(
        FormatSelection,
        Editor,
        &["cmd+k cmd+f"],
        &["ctrl+k ctrl+f"],
    ),
    row(
        TrimTrailingWhitespace,
        Editor,
        &["cmd+k cmd+x"],
        &["ctrl+k ctrl+x"],
    ),
    row(ChangeLanguageMode, Global, &["cmd+k m"], &["ctrl+k m"]),
    row(Fold, Editor, &["alt+cmd+["], &["ctrl+shift+["]),
    row(Unfold, Editor, &["alt+cmd+]"], &["ctrl+shift+]"]),
    row(ToggleFold, Editor, &["cmd+k cmd+l"], &["ctrl+k ctrl+l"]),
    row(
        FoldRecursively,
        Editor,
        &["cmd+k cmd+["],
        &["ctrl+k ctrl+["],
    ),
    row(
        UnfoldRecursively,
        Editor,
        &["cmd+k cmd+]"],
        &["ctrl+k ctrl+]"],
    ),
    row(FoldAll, Editor, &["cmd+k cmd+0"], &["ctrl+k ctrl+0"]),
    row(UnfoldAll, Editor, &["cmd+k cmd+j"], &["ctrl+k ctrl+j"]),
    row(FoldLevel1, Editor, &["cmd+k cmd+1"], &["ctrl+k ctrl+1"]),
    row(FoldLevel2, Editor, &["cmd+k cmd+2"], &["ctrl+k ctrl+2"]),
    row(FoldLevel3, Editor, &["cmd+k cmd+3"], &["ctrl+k ctrl+3"]),
    row(FoldLevel4, Editor, &["cmd+k cmd+4"], &["ctrl+k ctrl+4"]),
    row(FoldLevel5, Editor, &["cmd+k cmd+5"], &["ctrl+k ctrl+5"]),
    row(FoldLevel6, Editor, &["cmd+k cmd+6"], &["ctrl+k ctrl+6"]),
    row(FoldLevel7, Editor, &["cmd+k cmd+7"], &["ctrl+k ctrl+7"]),
    row(JoinLines, Editor, &["ctrl+j"], &[]),
    row(JumpToBracket, Editor, &["shift+cmd+\\"], &["ctrl+shift+\\"]),
    row(
        ExpandSelection,
        Editor,
        &["ctrl+shift+cmd+right", "ctrl+shift+right"],
        &["shift+alt+right"],
    ),
    row(
        ShrinkSelection,
        Editor,
        &["ctrl+shift+cmd+left", "ctrl+shift+left"],
        &["shift+alt+left"],
    ),
    row(AddCursorAbove, Editor, &["alt+cmd+up"], &["ctrl+alt+up"]),
    row(
        AddCursorBelow,
        Editor,
        &["alt+cmd+down"],
        &["ctrl+alt+down"],
    ),
    row(AddNextOccurrence, Editor, &["cmd+d"], &["ctrl+d"]),
    row(
        SkipToNextOccurrence,
        Editor,
        &["cmd+k cmd+d"],
        &["ctrl+k ctrl+d"],
    ),
    row(
        SelectAllOccurrences,
        Editor,
        &["shift+cmd+l"],
        &["ctrl+shift+l"],
    ),
    row(RemoveSecondaryCursors, Editor, &["escape"], &["escape"]),
    row(
        AddCursorsToLineEnds,
        Editor,
        &["shift+alt+i"],
        &["shift+alt+i"],
    ),
    row(CursorUndo, Editor, &["cmd+u"], &["ctrl+u"]),
    row(
        ColumnSelectUp,
        Editor,
        &["shift+alt+cmd+up"],
        &["ctrl+shift+alt+up"],
    ),
    row(
        ColumnSelectDown,
        Editor,
        &["shift+alt+cmd+down"],
        &["ctrl+shift+alt+down"],
    ),
    row(
        ColumnSelectLeft,
        Editor,
        &["shift+alt+cmd+left"],
        &["ctrl+shift+alt+left"],
    ),
    row(
        ColumnSelectRight,
        Editor,
        &["shift+alt+cmd+right"],
        &["ctrl+shift+alt+right"],
    ),
    row(CopyPath, NotEditor, &["alt+cmd+c"], &[]),
    row(CopyPath, EditorOrFind, &["cmd+k alt+cmd+c"], &[]),
    row(CopyRelativePath, NotEditor, &["shift+alt+cmd+c"], &[]),
    row(
        CopyRelativePath,
        EditorOrFind,
        &["cmd+k shift+alt+cmd+c"],
        &[],
    ),
    row(RevealInFinder, NotEditor, &["alt+cmd+r"], &["shift+alt+r"]),
    row(MarkdownBold, Markdown, &["cmd+b"], &["ctrl+b"]),
    row(MarkdownItalic, Markdown, &["cmd+i"], &["ctrl+i"]),
    row(CopyActivePath, Global, &["cmd+k p"], &["ctrl+k p"]),
    row(RevealActiveFile, Global, &["cmd+k r"], &["ctrl+k r"]),
    row(OpenInTerminal, Global, &["shift+cmd+c"], &["ctrl+shift+c"]),
    row(ToggleWordWrap, Global, &["alt+z"], &["alt+z"]),
    row(TogglePreview, Editor, &["shift+cmd+v"], &["ctrl+shift+v"]),
    row(
        ShowCommands,
        Global,
        &["shift+cmd+p", "f1"],
        &["ctrl+shift+p", "f1"],
    ),
    row(QuickOpen, Global, &["cmd+p"], &["ctrl+p", "ctrl+e"]),
    row(GoToSymbol, Global, &["shift+cmd+o"], &["ctrl+shift+o"]),
    row(GoToLine, Global, &["ctrl+g"], &["ctrl+g"]),
    row(GoBack, Global, &["ctrl+-"], &["alt+left"]),
    row(GoForward, Global, &["ctrl+shift+-"], &["alt+right"]),
    row(Settings, Global, &["cmd+,"], &["ctrl+,"]),
    row(
        KeyboardShortcuts,
        Global,
        &["cmd+k cmd+s"],
        &["ctrl+k ctrl+s"],
    ),
    row(NewFile, Global, &["cmd+n"], &["ctrl+n"]),
    row(NewWindow, Global, &["shift+cmd+n"], &["ctrl+shift+n"]),
    row(OpenFile, Global, &["cmd+o"], &["ctrl+o"]),
    row(OpenRecent, Global, &["ctrl+r"], &["ctrl+r"]),
    row(Save, Global, &["cmd+s"], &["ctrl+s"]),
    row(SaveAs, Global, &["shift+cmd+s"], &["ctrl+shift+s"]),
    row(CloseTab, Global, &["cmd+w"], &["ctrl+f4", "ctrl+w"]),
    row(CloseWindow, Global, &["shift+cmd+w"], &["ctrl+shift+w"]),
    row(CloseOtherTabs, Global, &["alt+cmd+t"], &[]),
    row(CloseAllTabs, Global, &["cmd+k cmd+w"], &["ctrl+k ctrl+w"]),
    row(ReopenClosed, Global, &["shift+cmd+t"], &["ctrl+shift+t"]),
    row(
        NextTab,
        Global,
        &["alt+cmd+right", "shift+cmd+]"],
        &["ctrl+pagedown"],
    ),
    row(
        PreviousTab,
        Global,
        &["alt+cmd+left", "shift+cmd+["],
        &["ctrl+pageup"],
    ),
    row(CloseFolder, Global, &["cmd+k f"], &["ctrl+k f"]),
    row(Quit, Global, &["cmd+q"], &[]),
    row(Paste, Global, &["cmd+v"], &["ctrl+v"]),
    row(Find, Global, &["cmd+f"], &["ctrl+f"]),
    row(Replace, Global, &["alt+cmd+f"], &["ctrl+h"]),
    row(FindInFiles, Global, &["shift+cmd+f"], &["ctrl+shift+f"]),
    row(ReplaceInFiles, Global, &["shift+cmd+h"], &["ctrl+shift+h"]),
    row(NextResult, Global, &["f4"], &["f4"]),
    row(PreviousResult, Global, &["shift+f4"], &["shift+f4"]),
    row(NextDifference, Editor, &["f7", "alt+f5"], &["f7", "alt+f5"]),
    row(
        PreviousDifference,
        Editor,
        &["shift+f7", "shift+alt+f5"],
        &["shift+f7", "shift+alt+f5"],
    ),
    row(
        GitRevertChange,
        Editor,
        &["cmd+k cmd+r"],
        &["ctrl+k ctrl+r"],
    ),
    row(ToggleSidebar, Global, &["cmd+b"], &["ctrl+b"]),
    row(FocusSidebar, Global, &["cmd+0"], &["ctrl+0"]),
    row(ShowFiles, Global, &["shift+cmd+e"], &["ctrl+shift+e"]),
    row(ShowCommit, Global, &["ctrl+shift+g"], &["ctrl+shift+g"]),
    row(ToggleTerminal, Everywhere, &["ctrl+`"], &["ctrl+`"]),
    row(ToggleTerminalPanel, Global, &["cmd+j"], &["ctrl+j"]),
    row(StopConfiguration, Global, &["shift+f5"], &["shift+f5"]),
    row(CopyPath, Global, &[], &["shift+alt+c"]),
    row(CopyRelativePath, Global, &[], &["ctrl+k ctrl+shift+c"]),
    row(OpenFolder, Global, &[], &["ctrl+k ctrl+o"]),
    row(
        ZoomIn,
        Everywhere,
        &["cmd+=", "shift+cmd+=", "cmd+numpad_add"],
        &["ctrl+=", "ctrl+shift+=", "ctrl+numpad_add"],
    ),
    row(
        ZoomOut,
        Everywhere,
        &["cmd+-", "shift+cmd+-", "cmd+numpad_subtract"],
        &["ctrl+-", "ctrl+shift+-", "ctrl+numpad_subtract"],
    ),
    row(ResetZoom, Everywhere, &["cmd+numpad0"], &["ctrl+numpad0"]),
    row(RunConfiguration, Global, &["ctrl+f5"], &["ctrl+f5"]),
];

/// VS Code 方案在 Linux 上与 Windows 不同的键。
pub(super) const VSCODE_LINUX: &[PresetRow] = &[
    row(CopyLinesUp, Editor, &[], &["ctrl+shift+alt+up"]),
    row(CopyLinesDown, Editor, &[], &["ctrl+shift+alt+down"]),
    row(Format, Editor, &[], &["ctrl+shift+i"]),
    row(AddCursorAbove, Editor, &[], &["shift+alt+up"]),
    row(AddCursorBelow, Editor, &[], &["shift+alt+down"]),
    row(ColumnSelectUp, Editor, &[], &[]),
    row(ColumnSelectDown, Editor, &[], &[]),
    row(ColumnSelectLeft, Editor, &[], &[]),
    row(ColumnSelectRight, Editor, &[], &[]),
    row(CopyPath, NotEditor, &[], &["ctrl+alt+c"]),
    row(CopyPath, EditorOrFind, &[], &["ctrl+k ctrl+alt+c"]),
    row(CopyRelativePath, NotEditor, &[], &["ctrl+shift+alt+c"]),
    row(
        CopyRelativePath,
        EditorOrFind,
        &[],
        &["ctrl+k ctrl+shift+alt+c"],
    ),
    row(RevealInFinder, NotEditor, &[], &["ctrl+alt+r"]),
    row(GoBack, Global, &[], &["ctrl+alt+-"]),
    row(GoForward, Global, &[], &["ctrl+shift+-"]),
    row(CloseTab, Global, &[], &["ctrl+w"]),
    row(Quit, Global, &[], &["ctrl+q"]),
    row(CopyPath, Global, &[], &[]),
    row(CopyRelativePath, Global, &[], &[]),
    row(
        ZoomOut,
        Everywhere,
        &[],
        &["ctrl+-", "ctrl+numpad_subtract"],
    ),
];
