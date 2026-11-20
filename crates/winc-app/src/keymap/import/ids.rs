//! 命令与 IDEA、VS Code 中同一操作的标识，导入快捷键时用来对应命令。

use super::*;

use Action::*;

/// 命令与其他编辑器中同一操作的标识：IDEA 的 action id，VS Code 的命令 id。
const IDS: &[(Action, &[&str], &[&str])] = &[
    (QuickOpen, &["GotoFile"], &["workbench.action.quickOpen"]),
    (
        GoToSymbol,
        &["FileStructurePopup"],
        &["workbench.action.gotoSymbol"],
    ),
    (GoToLine, &["GotoLine"], &["workbench.action.gotoLine"]),
    (
        GoToDefinition,
        &["GotoDeclaration"],
        &["editor.action.revealDefinition"],
    ),
    (
        FindReferences,
        &["FindUsages", "ShowUsages"],
        &[
            "editor.action.goToReferences",
            "references-view.findReferences",
        ],
    ),
    (GoBack, &["Back"], &["workbench.action.navigateBack"]),
    (
        GoForward,
        &["Forward"],
        &["workbench.action.navigateForward"],
    ),
    (
        ShowCommands,
        &["GotoAction"],
        &["workbench.action.showCommands"],
    ),
    (
        Settings,
        &["ShowSettings"],
        &["workbench.action.openSettings"],
    ),
    (
        KeyboardShortcuts,
        &[],
        &["workbench.action.openGlobalKeybindings"],
    ),
    (RecordKeys, &[], &["keybindings.editor.recordSearchKeys"]),
    (Format, &["ReformatCode"], &["editor.action.formatDocument"]),
    (
        FormatSelection,
        &["ReformatCode"],
        &["editor.action.formatSelection"],
    ),
    (Fix, &["ShowIntentionActions"], &["editor.action.quickFix"]),
    (Find, &["Find"], &["actions.find"]),
    (
        Replace,
        &["Replace"],
        &["editor.action.startFindReplaceAction"],
    ),
    (
        FindNext,
        &["FindNext"],
        &["editor.action.nextMatchFindAction"],
    ),
    (
        FindPrevious,
        &["FindPrevious"],
        &["editor.action.previousMatchFindAction"],
    ),
    (HighlightWord, &["HighlightUsagesInFile"], &[]),
    (ReplaceOne, &[], &["editor.action.replaceOne"]),
    (ReplaceAll, &[], &["editor.action.replaceAll"]),
    (
        SelectAllMatches,
        &["SelectAllOccurrences"],
        &["editor.action.selectAllMatches"],
    ),
    (
        ToggleCase,
        &[],
        &["toggleFindCaseSensitive", "toggleSearchCaseSensitive"],
    ),
    (
        ToggleWord,
        &[],
        &["toggleFindWholeWord", "toggleSearchWholeWord"],
    ),
    (ToggleRegex, &[], &["toggleFindRegex", "toggleSearchRegex"]),
    (
        FindInFiles,
        &["FindInPath"],
        &["workbench.action.findInFiles"],
    ),
    (
        ReplaceInFiles,
        &["ReplaceInPath"],
        &["workbench.action.replaceInFiles"],
    ),
    (ReplaceAllInFiles, &[], &["search.action.replaceAll"]),
    (
        NextResult,
        &["NextOccurence"],
        &["search.action.focusNextSearchResult"],
    ),
    (
        PreviousResult,
        &["PreviousOccurence"],
        &["search.action.focusPreviousSearchResult"],
    ),
    (
        NewFile,
        &["NewScratchFile", "NewElement"],
        &["workbench.action.files.newUntitledFile"],
    ),
    (NewWindow, &[], &["workbench.action.newWindow"]),
    (
        OpenFile,
        &["OpenFile"],
        &[
            "workbench.action.files.openFile",
            "workbench.action.files.openFileFolder",
        ],
    ),
    (
        OpenFolder,
        &["OpenFile"],
        &["workbench.action.files.openFolder"],
    ),
    (
        OpenRecent,
        &["ManageRecentProjects"],
        &["workbench.action.openRecent"],
    ),
    (Save, &["SaveAll"], &["workbench.action.files.save"]),
    (SaveAs, &[], &["workbench.action.files.saveAs"]),
    (
        CloseTab,
        &["CloseContent", "CloseEditor"],
        &["workbench.action.closeActiveEditor"],
    ),
    (
        CloseOtherTabs,
        &["CloseAllEditorsButActive"],
        &["workbench.action.closeOtherEditors"],
    ),
    (
        CloseAllTabs,
        &["CloseAllEditors"],
        &["workbench.action.closeAllEditors"],
    ),
    (
        ReopenClosed,
        &["ReopenClosedTab"],
        &["workbench.action.reopenClosedEditor"],
    ),
    (NextTab, &["NextTab"], &["workbench.action.nextEditor"]),
    (
        PreviousTab,
        &["PreviousTab"],
        &["workbench.action.previousEditor"],
    ),
    (
        CloseFolder,
        &["CloseProject"],
        &["workbench.action.closeFolder"],
    ),
    (
        CloseWindow,
        &["CloseProject"],
        &["workbench.action.closeWindow"],
    ),
    (
        CopyPath,
        &["CopyPaths", "CopyAbsolutePath"],
        &["copyFilePath"],
    ),
    (
        CopyRelativePath,
        &["CopyContentRootPath", "CopySourceRootPath"],
        &["copyRelativeFilePath"],
    ),
    (CopyReference, &["CopyReference"], &[]),
    (CopyAbsoluteReference, &["CopyReference"], &[]),
    (RevealInFinder, &["RevealIn"], &["revealFileInOS"]),
    (
        CopyActivePath,
        &["CopyPaths"],
        &["workbench.action.files.copyPathOfActiveFile"],
    ),
    (
        RevealActiveFile,
        &["RevealIn"],
        &["workbench.action.files.revealActiveFileInWindows"],
    ),
    (
        OpenInTerminal,
        &["Terminal.OpenInTerminal"],
        &["workbench.action.terminal.openNativeConsole"],
    ),
    (Quit, &["Exit"], &["workbench.action.quit"]),
    (Undo, &["$Undo"], &["undo"]),
    (Redo, &["$Redo"], &["redo"]),
    (Cut, &["$Cut"], &["editor.action.clipboardCutAction"]),
    (Copy, &["$Copy"], &["editor.action.clipboardCopyAction"]),
    (Paste, &["$Paste"], &["editor.action.clipboardPasteAction"]),
    (SelectAll, &["$SelectAll"], &["editor.action.selectAll"]),
    (
        ToggleComment,
        &["CommentByLineComment"],
        &["editor.action.commentLine"],
    ),
    (
        MarkdownBold,
        &["org.intellij.plugins.markdown.ui.actions.styling.ToggleBoldAction"],
        &[],
    ),
    (
        MarkdownItalic,
        &["org.intellij.plugins.markdown.ui.actions.styling.ToggleItalicAction"],
        &[],
    ),
    (
        MarkdownCode,
        &["org.intellij.plugins.markdown.ui.actions.styling.ToggleCodeSpanAction"],
        &[],
    ),
    (
        MarkdownStrikethrough,
        &["org.intellij.plugins.markdown.ui.actions.styling.ToggleStrikethroughAction"],
        &[],
    ),
    (
        DeleteLine,
        &["EditorDeleteLine"],
        &["editor.action.deleteLines"],
    ),
    (
        DeleteAllLeft,
        &["EditorDeleteToLineStart"],
        &["deleteAllLeft"],
    ),
    (
        IndentLines,
        &["EditorIndentSelection"],
        &["editor.action.indentLines"],
    ),
    (
        OutdentLines,
        &["EditorUnindentSelection"],
        &["editor.action.outdentLines"],
    ),
    (
        MoveLinesUp,
        &["MoveLineUp"],
        &["editor.action.moveLinesUpAction"],
    ),
    (
        MoveLinesDown,
        &["MoveLineDown"],
        &["editor.action.moveLinesDownAction"],
    ),
    (CopyLinesUp, &[], &["editor.action.copyLinesUpAction"]),
    (
        CopyLinesDown,
        &["EditorDuplicate"],
        &["editor.action.copyLinesDownAction"],
    ),
    (
        JoinLines,
        &["EditorJoinLines"],
        &["editor.action.joinLines"],
    ),
    (
        ExpandSelection,
        &["EditorSelectWord"],
        &["editor.action.smartSelect.expand"],
    ),
    (
        ShrinkSelection,
        &["EditorUnSelectWord"],
        &["editor.action.smartSelect.shrink"],
    ),
    (
        AddCursorAbove,
        &["EditorCloneCaretAbove"],
        &["editor.action.insertCursorAbove"],
    ),
    (
        AddCursorBelow,
        &["EditorCloneCaretBelow"],
        &["editor.action.insertCursorBelow"],
    ),
    (
        AddNextOccurrence,
        &["SelectNextOccurrence"],
        &["editor.action.addSelectionToNextFindMatch"],
    ),
    (
        SkipToNextOccurrence,
        &[],
        &["editor.action.moveSelectionToNextFindMatch"],
    ),
    (
        SelectAllOccurrences,
        &["SelectAllOccurrences"],
        &["editor.action.selectHighlights"],
    ),
    (
        RemoveSecondaryCursors,
        &["EditorEscape"],
        &["removeSecondaryCursors"],
    ),
    (
        AddCursorsToLineEnds,
        &["EditorAddCaretPerSelectedLine"],
        &["editor.action.insertCursorAtEndOfEachLineSelected"],
    ),
    (CursorUndo, &["UnselectPreviousOccurrence"], &["cursorUndo"]),
    (CursorRedo, &[], &["cursorRedo"]),
    (
        ColumnSelectUp,
        &["EditorCloneCaretAbove"],
        &["cursorColumnSelectUp"],
    ),
    (
        ColumnSelectDown,
        &["EditorCloneCaretBelow"],
        &["cursorColumnSelectDown"],
    ),
    (ColumnSelectLeft, &[], &["cursorColumnSelectLeft"]),
    (ColumnSelectRight, &[], &["cursorColumnSelectRight"]),
    (
        JumpToBracket,
        &["EditorMatchBrace"],
        &["editor.action.jumpToBracket"],
    ),
    (TransformToUppercase, &["EditorToggleCase"], &[]),
    (
        TrimTrailingWhitespace,
        &[],
        &["editor.action.trimTrailingWhitespace"],
    ),
    (
        ChangeLanguageMode,
        &[],
        &["workbench.action.editor.changeLanguageMode"],
    ),
    (Fold, &["CollapseRegion"], &["editor.fold"]),
    (Unfold, &["ExpandRegion"], &["editor.unfold"]),
    (
        ToggleFold,
        &["CollapseExpandRegion"],
        &["editor.toggleFold"],
    ),
    (
        FoldRecursively,
        &["CollapseRegionRecursively"],
        &["editor.foldRecursively"],
    ),
    (
        UnfoldRecursively,
        &["ExpandRegionRecursively"],
        &["editor.unfoldRecursively"],
    ),
    (FoldAll, &["CollapseAllRegions"], &["editor.foldAll"]),
    (UnfoldAll, &["ExpandAllRegions"], &["editor.unfoldAll"]),
    (FoldLevel1, &["ExpandAllToLevel1"], &["editor.foldLevel1"]),
    (FoldLevel2, &["ExpandAllToLevel2"], &["editor.foldLevel2"]),
    (FoldLevel3, &["ExpandAllToLevel3"], &["editor.foldLevel3"]),
    (FoldLevel4, &["ExpandAllToLevel4"], &["editor.foldLevel4"]),
    (FoldLevel5, &["ExpandAllToLevel5"], &["editor.foldLevel5"]),
    (FoldLevel6, &["ExpandAllToLevel6"], &["editor.foldLevel6"]),
    (FoldLevel7, &["ExpandAllToLevel7"], &["editor.foldLevel7"]),
    (ToggleBookmark, &["ToggleBookmark"], &[]),
    (NextBookmark, &["GotoNextBookmark"], &[]),
    (PreviousBookmark, &["GotoPreviousBookmark"], &[]),
    (ListBookmarks, &["ShowBookmarks"], &[]),
    (
        RenameFile,
        &["RenameElement", "RenameFile"],
        &["renameFile"],
    ),
    (DeleteFile, &["$Delete"], &["moveFileToTrash"]),
    (DeleteFilePermanently, &[], &["deleteFile"]),
    (
        OpenSelected,
        &["EditSource"],
        &["explorer.openAndPassFocus", "list.select"],
    ),
    (PreviewSelected, &[], &["list.toggleExpand"]),
    (
        FindInFolder,
        &["FindInPath"],
        &["filesExplorer.findInFolder"],
    ),
    (
        ToggleSidebar,
        &["HideAllWindows"],
        &["workbench.action.toggleSidebarVisibility"],
    ),
    (
        FocusSidebar,
        &["ActivateProjectToolWindow"],
        &["workbench.action.focusSideBar"],
    ),
    (
        ShowFiles,
        &["ActivateProjectToolWindow"],
        &["workbench.view.explorer"],
    ),
    (ShowOutline, &["ActivateStructureToolWindow"], &[]),
    (
        IncreaseFontSize,
        &["EditorIncreaseFontSizeGlobal", "EditorIncreaseFontSize"],
        &["editor.action.fontZoomIn"],
    ),
    (
        DecreaseFontSize,
        &["EditorDecreaseFontSizeGlobal", "EditorDecreaseFontSize"],
        &["editor.action.fontZoomOut"],
    ),
    (
        ResetFontSize,
        &["EditorResetFontSize"],
        &["editor.action.fontZoomReset"],
    ),
    (ZoomIn, &["ZoomInIdeAction"], &["workbench.action.zoomIn"]),
    (
        ZoomOut,
        &["ZoomOutIdeAction"],
        &["workbench.action.zoomOut"],
    ),
    (
        ResetZoom,
        &["ResetIdeScaleAction"],
        &["workbench.action.zoomReset"],
    ),
    (ToggleVimMode, &["VimPluginToggle"], &[]),
    (
        ToggleWordWrap,
        &["EditorToggleUseSoftWraps"],
        &["editor.action.toggleWordWrap"],
    ),
    (ToggleLineNumbers, &["EditorToggleShowLineNumbers"], &[]),
    (ToggleWhitespace, &["EditorToggleShowWhitespaces"], &[]),
    (TogglePreview, &[], &["markdown.showPreview"]),
    (ToggleReadOnly, &["ToggleReadOnlyAttribute"], &[]),
    (ShowCommit, &["CheckinProject"], &["workbench.view.scm"]),
    (
        GitShowChanges,
        &["ActivateCommitToolWindow"],
        &["workbench.view.scm"],
    ),
    (ChangesShowDiff, &["Compare.SameVersion", "ShowDiff"], &[]),
    (ChangesOpenSource, &["EditSource"], &[]),
    (ChangesRollback, &["ChangesView.Revert"], &[]),
    (GitShowLog, &["ActivateVersionControlToolWindow"], &[]),
    (GitGoToRevision, &["Vcs.Log.GoToRef"], &[]),
    (GitSwitchBranch, &["Git.Branches"], &[]),
    (GitFetch, &["Git.Fetch"], &[]),
    (GitPull, &["Vcs.UpdateProject"], &[]),
    (GitPush, &["Vcs.Push"], &[]),
    (GitStash, &["Git.Stash"], &[]),
    (GitFileHistory, &["Vcs.ShowTabbedFileHistory"], &[]),
    (GitSelectionHistory, &["Vcs.ShowHistoryForBlock"], &[]),
    (GitBlame, &["Annotate"], &[]),
    (
        GitRevertChange,
        &["Vcs.RollbackChangedLines"],
        &["git.revertSelectedRanges"],
    ),
    (
        ToggleTerminal,
        &["ActivateTerminalToolWindow"],
        &["workbench.action.terminal.toggleTerminal"],
    ),
    (
        ToggleTerminalPanel,
        &["HideActiveWindow"],
        &["workbench.action.togglePanel"],
    ),
    (
        RunConfiguration,
        &["Run", "RunClass"],
        &["workbench.action.debug.run"],
    ),
    (
        StopConfiguration,
        &["Stop"],
        &["workbench.action.debug.stop"],
    ),
    (EditRunConfigurations, &["editRunConfigurations"], &[]),
    (
        NewTerminal,
        &["Terminal.NewTab"],
        &["workbench.action.terminal.new"],
    ),
    (
        ClearTerminal,
        &["Terminal.ClearBuffer"],
        &["workbench.action.terminal.clear"],
    ),
    (
        TerminalCopy,
        &["Terminal.CopySelectedText"],
        &[
            "workbench.action.terminal.copySelection",
            "workbench.action.terminal.copyAndClearSelection",
        ],
    ),
    (TerminalPaste, &[], &["workbench.action.terminal.paste"]),
    (
        FindInTerminal,
        &["Find"],
        &["workbench.action.terminal.focusFind"],
    ),
    (
        MaximizeTerminal,
        &["MaximizeToolWindow"],
        &["workbench.action.toggleMaximizedPanel"],
    ),
    (
        ScrollTerminalTop,
        &[],
        &["workbench.action.terminal.scrollToTop"],
    ),
    (
        ScrollTerminalBottom,
        &[],
        &["workbench.action.terminal.scrollToBottom"],
    ),
    (
        CloseTerminal,
        &["CloseContent"],
        &["workbench.action.terminal.kill"],
    ),
    (
        NextTerminal,
        &["NextTab"],
        &["workbench.action.terminal.focusNext"],
    ),
    (
        PreviousTerminal,
        &["PreviousTab"],
        &["workbench.action.terminal.focusPrevious"],
    ),
    (
        FocusTerminal1,
        &[],
        &["workbench.action.terminal.focusAtIndex1"],
    ),
    (
        FocusTerminal2,
        &[],
        &["workbench.action.terminal.focusAtIndex2"],
    ),
    (
        FocusTerminal3,
        &[],
        &["workbench.action.terminal.focusAtIndex3"],
    ),
    (
        FocusTerminal4,
        &[],
        &["workbench.action.terminal.focusAtIndex4"],
    ),
    (
        FocusTerminal5,
        &[],
        &["workbench.action.terminal.focusAtIndex5"],
    ),
    (
        FocusTerminal6,
        &[],
        &["workbench.action.terminal.focusAtIndex6"],
    ),
    (
        FocusTerminal7,
        &[],
        &["workbench.action.terminal.focusAtIndex7"],
    ),
    (
        FocusTerminal8,
        &[],
        &["workbench.action.terminal.focusAtIndex8"],
    ),
    (
        FocusTerminal9,
        &[],
        &["workbench.action.terminal.focusAtIndex9"],
    ),
    (
        PreviousCommand,
        &[],
        &["workbench.action.terminal.scrollToPreviousCommand"],
    ),
    (
        NextCommand,
        &[],
        &["workbench.action.terminal.scrollToNextCommand"],
    ),
    (
        NextError,
        &["GotoNextError"],
        &["editor.action.marker.nextInFiles"],
    ),
    (
        PreviousError,
        &["GotoPreviousError"],
        &["editor.action.marker.prevInFiles"],
    ),
    (
        SendToTerminal,
        &["Terminal.RunSelection"],
        &["workbench.action.terminal.runSelectedText"],
    ),
    (
        NextDifference,
        &["NextDiff"],
        &[
            "workbench.action.compareEditor.nextChange",
            "editor.action.dirtydiff.next",
        ],
    ),
    (
        PreviousDifference,
        &["PreviousDiff"],
        &[
            "workbench.action.compareEditor.previousChange",
            "editor.action.dirtydiff.previous",
        ],
    ),
    (NextDiffFile, &["NextDiffFile", "Diff.NextChange"], &[]),
    (PreviousDiffFile, &["PreviousDiffFile"], &[]),
    (ToggleDiffLayout, &["Diff.ToggleDiffAligningMode"], &[]),
];

/// 返回 IDEA 中 action id 为 `id` 的操作对应的命令；同一个 id 对应多个命令时取第一个。
pub(super) fn from_idea(id: &str) -> Option<Action> {
    IDS.iter()
        .find(|(_, idea, _)| idea.contains(&id))
        .map(|(a, ..)| *a)
}

/// 返回 VS Code 中命令 id 为 `id` 的操作对应的命令；同一个 id 对应多个命令时取第一个。
pub(super) fn from_vscode(id: &str) -> Option<Action> {
    IDS.iter()
        .find(|(_, _, code)| code.contains(&id))
        .map(|(a, ..)| *a)
}
