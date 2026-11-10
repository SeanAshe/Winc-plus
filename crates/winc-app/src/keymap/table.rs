//! 默认键位表：每个命令在三个系统上的键，一个键一条 [`Binding`]。

use super::*;

/// 一条绑定：在 `when` 范围内按下该键（或该组合键）执行 `action`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// 要执行的命令。
    pub action: Action,
    /// 第一段按键。
    pub first: Stroke,
    /// 组合键的第二段；单按即可触发时为 `None`。
    pub second: Option<Stroke>,
    /// 生效范围。
    pub when: When,
}

impl Binding {
    /// 返回绑定在 `os` 上的显示文字；两段组合键的两段之间用空格连接。
    pub fn label(&self, os: Os) -> String {
        match self.second {
            Some(second) => format!("{} {}", self.first.label(os), second.label(os)),
            None => self.first.label(os),
        }
    }
}

/// 建表用的绑定集合，可以含单段和两段按键。
struct Table(Vec<Binding>);

impl Table {
    fn one(&mut self, action: Action, when: When, stroke: Stroke) {
        self.0.push(Binding {
            action,
            first: stroke,
            second: None,
            when,
        });
    }

    fn chord(&mut self, action: Action, when: When, first: Stroke, second: Stroke) {
        self.0.push(Binding {
            action,
            first,
            second: Some(second),
            when,
        });
    }
}

/// 返回 `os` 上的整张绑定表。
///
/// 生效范围小的绑定写在前面，匹配时按表中顺序取用。
pub fn bindings(os: Os) -> Vec<Binding> {
    use Action::*;
    use KeyCode::*;
    use When::*;

    let mac = os == Os::Mac;
    let linux = os == Os::Linux;
    // 主修饰键：macOS 上是 ⌘，其他系统上是 Ctrl；macOS 上的 ⌃ 另用 `ctrl` 表示。
    let cmd = if mac { Mods::META } else { Mods::CTRL };
    let (ctrl, shift, alt) = (Mods::CTRL, Mods::SHIFT, Mods::ALT);
    let none = Mods::NONE;
    let key = |mods: Mods, code: KeyCode| Stroke { key: code, mods };
    let c = |mods: Mods, ch: char| key(mods, Char(ch));
    let chord_k = c(cmd, 'k');

    let mut t = Table(Vec::new());

    // 范围小的绑定写在前面：终端、快捷键那一页、文件树、搜索面板、查找条、编辑区。终端里的
    // 标签和查找在 macOS 上沿用编辑区标签的键（⌘ 不与 shell 冲突），其他系统上单按 Ctrl 的
    // 组合要留给 shell（Ctrl+C 中断、Ctrl+W 删词、Ctrl+A 到行首），终端自己的命令加 Shift。
    let term = if mac { cmd } else { ctrl | shift };
    t.one(NewTerminal, Terminal, c(term, 't'));
    t.one(CloseTerminal, Terminal, c(term, 'w'));
    t.one(FindInTerminal, Terminal, c(term, 'f'));
    t.one(MaximizeTerminal, Terminal, key(cmd | shift, Enter));
    t.one(Copy, Terminal, c(term, 'c'));
    t.one(SelectAll, Terminal, c(term, 'a'));
    if !mac {
        t.one(Paste, Terminal, c(term, 'v'));
    }
    if mac {
        t.one(NextTerminal, Terminal, key(cmd | alt, Right));
        t.one(NextTerminal, Terminal, c(cmd | shift, ']'));
        t.one(PreviousTerminal, Terminal, key(cmd | alt, Left));
        t.one(PreviousTerminal, Terminal, c(cmd | shift, '['));
    } else {
        t.one(NextTerminal, Terminal, key(ctrl, PageDown));
        t.one(PreviousTerminal, Terminal, key(ctrl, PageUp));
    }
    // 命令块：跳到上一条、下一条命令，折起输出（与编辑区折叠使用同样的键），在最后一条
    // 失败命令的报错之间跳。
    let (jump, fold) = if mac {
        (cmd, cmd | alt)
    } else {
        (ctrl, ctrl | shift)
    };
    t.one(PreviousCommand, Terminal, key(jump, Up));
    t.one(NextCommand, Terminal, key(jump, Down));
    t.one(FoldOutput, Terminal, c(fold, '['));
    t.one(UnfoldOutput, Terminal, c(fold, ']'));
    t.one(NextError, Terminal, key(none, F(8)));
    t.one(PreviousError, Terminal, key(shift, F(8)));
    // 把编辑区选中的代码发到终端执行。
    t.one(SendToTerminal, Editor, key(ctrl | shift, Enter));
    t.one(
        RecordKeys,
        KeysPage,
        c(if mac { cmd | alt } else { alt }, 'k'),
    );
    if mac {
        t.one(RenameFile, Tree, key(none, Enter));
        t.one(DeleteFile, Tree, key(cmd, Backspace));
        t.one(DeleteFile, Tree, key(none, Delete));
        t.one(OpenSelected, Tree, key(cmd, Down));
    } else {
        t.one(RenameFile, Tree, key(none, F(2)));
        t.one(DeleteFile, Tree, key(none, Delete));
        t.one(OpenSelected, Tree, key(none, Enter));
    }
    t.one(PreviewSelected, Tree, key(none, Space));
    t.one(FindInFolder, Tree, c(shift | alt, 'f'));

    // 查找条和搜索面板的三个开关：macOS 上 ⌥⌘C / W / R，其他系统上 Alt+C / W / R。
    let toggle = if mac { cmd | alt } else { alt };
    for when in [Panel, EditorOrFind] {
        t.one(ToggleCase, when, c(toggle, 'c'));
        t.one(ToggleWord, when, c(toggle, 'w'));
        t.one(ToggleRegex, when, c(toggle, 'r'));
    }
    // 「只显示匹配的行」与三个开关并排；⌥⌘L 已被书签占用，因此使用 ⌥⌘F（Filter）。
    t.one(FilterLines, EditorOrFind, c(toggle, 'f'));
    let replace_all = key(if mac { cmd | alt } else { ctrl | alt }, Enter);
    t.one(ReplaceAllInFiles, Panel, replace_all);
    t.one(ReplaceAll, EditorOrFind, replace_all);
    if mac {
        t.one(ReplaceAll, ReplaceInput, key(cmd, Enter));
    }
    t.one(ReplaceOne, EditorOrFind, c(cmd | shift, '1'));
    // 查找条开着时 ⌥↩ 把全部查找结果变成选区（多光标）。
    t.one(SelectAllMatches, EditorOrFind, key(alt, Enter));
    if mac {
        t.one(FindNext, EditorOrFind, c(cmd, 'g'));
        t.one(FindPrevious, EditorOrFind, c(cmd | shift, 'g'));
    }
    t.one(FindNext, EditorOrFind, key(none, F(3)));
    t.one(FindPrevious, EditorOrFind, key(shift, F(3)));
    // 固定高亮使用带修饰键的组合：不带 ⌘ 的可输入字符键会占用正文中的输入（如 `*`、`(`）。
    t.one(HighlightWord, Editor, key(cmd | shift, F(7)));
    t.one(ClearWordHighlights, Editor, key(shift, Escape));

    t.one(GoToDefinition, Editor, key(none, F(12)));
    t.one(FindReferences, Editor, key(shift, F(12)));
    t.one(Undo, Editor, c(cmd, 'z'));
    if mac {
        t.one(Redo, Editor, c(cmd | shift, 'z'));
    } else {
        t.one(Redo, Editor, c(ctrl, 'y'));
        t.one(Redo, Editor, c(ctrl | shift, 'z'));
    }
    t.one(Cut, Editor, c(cmd, 'x'));
    t.one(Copy, Editor, c(cmd, 'c'));
    t.one(Copy, Tree, c(cmd, 'c'));
    t.one(SelectAll, Editor, c(cmd, 'a'));
    t.one(ToggleComment, Editor, c(cmd, '/'));
    t.one(DeleteLine, Editor, c(cmd | shift, 'k'));
    if mac {
        t.one(DeleteAllLeft, Editor, key(cmd, Backspace));
    }
    t.one(IndentLines, Editor, c(cmd, ']'));
    t.one(OutdentLines, Editor, c(cmd, '['));
    t.one(MoveLinesUp, Editor, key(alt, Up));
    t.one(MoveLinesDown, Editor, key(alt, Down));
    let copy_lines = if linux {
        ctrl | shift | alt
    } else {
        shift | alt
    };
    t.one(CopyLinesUp, Editor, key(copy_lines, Up));
    t.one(CopyLinesDown, Editor, key(copy_lines, Down));
    if linux {
        t.one(Format, Editor, c(ctrl | shift, 'i'));
    } else {
        t.one(Format, Editor, c(shift | alt, 'f'));
    }
    t.chord(FormatSelection, Editor, chord_k, c(cmd, 'f'));
    t.chord(TrimTrailingWhitespace, Editor, chord_k, c(cmd, 'x'));
    t.chord(ChangeLanguageMode, Global, chord_k, c(none, 'm'));
    // 折叠：macOS 上 ⌥⌘[ / ⌥⌘]，其他系统上 Ctrl+Shift+[ / ]；其余折叠命令都在两段组合键
    // ⌘K 之后。
    t.one(Fold, Editor, c(fold, '['));
    t.one(Unfold, Editor, c(fold, ']'));
    t.chord(ToggleFold, Editor, chord_k, c(cmd, 'l'));
    t.chord(FoldRecursively, Editor, chord_k, c(cmd, '['));
    t.chord(UnfoldRecursively, Editor, chord_k, c(cmd, ']'));
    // 书签：⌥⌘K 添加或移除，⌥⌘L 列出全部，上一处、下一处用 ⌥⌘J 和 ⇧⌥⌘J。
    t.one(ToggleBookmark, Editor, c(cmd | alt, 'k'));
    t.one(ListBookmarks, Editor, c(cmd | alt, 'l'));
    t.one(PreviousBookmark, Editor, c(cmd | alt, 'j'));
    t.one(NextBookmark, Editor, c(cmd | alt | shift, 'j'));
    t.chord(ClearBookmarks, Editor, chord_k, c(cmd | alt, 'k'));
    t.chord(FoldAll, Editor, chord_k, c(cmd, '0'));
    t.chord(UnfoldAll, Editor, chord_k, c(cmd, 'j'));
    let levels = [
        FoldLevel1, FoldLevel2, FoldLevel3, FoldLevel4, FoldLevel5, FoldLevel6, FoldLevel7,
    ];
    for (level, digit) in levels.into_iter().zip('1'..='7') {
        t.chord(level, Editor, chord_k, c(cmd, digit));
    }
    if mac {
        t.one(JoinLines, Editor, c(ctrl, 'j'));
    }
    t.one(JumpToBracket, Editor, c(cmd | shift, '\\'));
    if mac {
        t.one(ExpandSelection, Editor, key(ctrl | shift | cmd, Right));
        t.one(ExpandSelection, Editor, key(ctrl | shift, Right));
        t.one(ShrinkSelection, Editor, key(ctrl | shift | cmd, Left));
        t.one(ShrinkSelection, Editor, key(ctrl | shift, Left));
    } else {
        t.one(ExpandSelection, Editor, key(shift | alt, Right));
        t.one(ShrinkSelection, Editor, key(shift | alt, Left));
    }

    // 多光标：⌥⌘↑ / ⌥⌘↓ 在上下行添加光标（Windows 上 Ctrl+Alt，Linux 上
    // Shift+Alt），⌘D 添加下一处匹配，⌘K ⌘D 跳过这一处，⇧⌘L 选中全部匹配，Esc 只保留主光标。
    let add_cursor = match os {
        Os::Mac => cmd | alt,
        Os::Windows => ctrl | alt,
        Os::Linux => shift | alt,
    };
    t.one(AddCursorAbove, Editor, key(add_cursor, Up));
    t.one(AddCursorBelow, Editor, key(add_cursor, Down));
    t.one(AddNextOccurrence, Editor, c(cmd, 'd'));
    t.chord(SkipToNextOccurrence, Editor, chord_k, c(cmd, 'd'));
    t.one(SelectAllOccurrences, Editor, c(cmd | shift, 'l'));
    t.one(RemoveSecondaryCursors, Editor, key(none, Escape));
    // ⇧⌥I 在选中的每一行行尾加光标，⌘U / ⇧⌘U 光标撤销、重做，⇧⌥⌘ 加方向键用键盘框一列
    // （Windows 上 Ctrl+Shift+Alt；Linux 上这些组合键用于复制行，不设置）。
    t.one(AddCursorsToLineEnds, Editor, c(shift | alt, 'i'));
    t.one(CursorUndo, Editor, c(cmd, 'u'));
    t.one(CursorRedo, Editor, c(cmd | shift, 'u'));
    if !linux {
        let column = if mac {
            cmd | shift | alt
        } else {
            ctrl | shift | alt
        };
        t.one(ColumnSelectUp, Editor, key(column, Up));
        t.one(ColumnSelectDown, Editor, key(column, Down));
        t.one(ColumnSelectLeft, Editor, key(column, Left));
        t.one(ColumnSelectRight, Editor, key(column, Right));
    }

    // 复制路径、在文件管理器中显示：不在编辑区时作用于选中的项（文件树中）或当前文件；
    // 在编辑区中 ⌥⌘C 是查找的开关，复制路径要先按 ⌘K。
    match os {
        Os::Mac => {
            t.one(CopyPath, NotEditor, c(cmd | alt, 'c'));
            t.chord(CopyPath, EditorOrFind, chord_k, c(cmd | alt, 'c'));
            t.one(CopyRelativePath, NotEditor, c(cmd | alt | shift, 'c'));
            t.chord(
                CopyRelativePath,
                EditorOrFind,
                chord_k,
                c(cmd | alt | shift, 'c'),
            );
            t.one(RevealInFinder, NotEditor, c(cmd | alt, 'r'));
        }
        Os::Windows => {
            t.one(CopyPath, Global, c(shift | alt, 'c'));
            t.chord(CopyRelativePath, Global, chord_k, c(ctrl | shift, 'c'));
            t.one(RevealInFinder, NotEditor, c(shift | alt, 'r'));
        }
        Os::Linux => {
            t.one(CopyPath, NotEditor, c(ctrl | alt, 'c'));
            t.chord(CopyPath, EditorOrFind, chord_k, c(ctrl | alt, 'c'));
            t.one(CopyRelativePath, NotEditor, c(ctrl | shift | alt, 'c'));
            t.chord(
                CopyRelativePath,
                EditorOrFind,
                chord_k,
                c(ctrl | shift | alt, 'c'),
            );
            t.one(RevealInFinder, NotEditor, c(ctrl | alt, 'r'));
        }
    }
    // 可编辑的 Markdown 预览中的行内格式。范围小于编辑区，写在同键的绑定之前；其余位置这些键
    // 执行表中靠后的绑定（⌘B 显示或隐藏侧栏，⌘I 复制代码引用）。
    t.one(MarkdownBold, Markdown, c(cmd, 'b'));
    t.one(MarkdownItalic, Markdown, c(cmd, 'i'));
    t.one(MarkdownCode, Markdown, c(cmd, 'e'));
    t.one(MarkdownStrikethrough, Markdown, c(cmd | shift, 'x'));
    // 复制代码引用（`路径:行`）：⌘I 是相对路径，⌘L 是绝对路径（其他系统上 Ctrl）。
    t.one(CopyReference, Editor, c(cmd, 'i'));
    t.one(CopyAbsoluteReference, Editor, c(cmd, 'l'));
    t.chord(CopyActivePath, Global, chord_k, c(none, 'p'));
    t.chord(RevealActiveFile, Global, chord_k, c(none, 'r'));
    // 在外部终端中打开：macOS 上 ⇧⌘C，其他系统上 Ctrl+Shift+C。
    t.one(
        OpenInTerminal,
        Global,
        c(if mac { cmd | shift } else { ctrl | shift }, 'c'),
    );

    // 切换自动换行：⌥Z（其他系统上 Alt+Z）。
    t.one(ToggleWordWrap, Global, c(alt, 'z'));

    // SVG 在源代码和预览之间切换：macOS 上 ⇧⌘V，其他系统上 Ctrl+Shift+V。
    t.one(
        TogglePreview,
        Editor,
        c(if mac { cmd | shift } else { ctrl | shift }, 'v'),
    );

    // 所有区域都生效的绑定。
    t.one(ShowCommands, Global, c(cmd | shift, 'p'));
    t.one(ShowCommands, Global, key(none, F(1)));
    t.one(QuickOpen, Global, c(cmd, 'p'));
    if !mac {
        t.one(QuickOpen, Global, c(ctrl, 'e'));
    }
    t.one(GoToSymbol, Global, c(cmd | shift, 'o'));
    t.one(GoToLine, Global, c(ctrl, 'g'));
    match os {
        Os::Mac => {
            t.one(GoBack, Global, c(ctrl, '-'));
            t.one(GoForward, Global, c(ctrl | shift, '-'));
        }
        Os::Windows => {
            t.one(GoBack, Global, key(alt, Left));
            t.one(GoForward, Global, key(alt, Right));
        }
        Os::Linux => {
            t.one(GoBack, Global, c(ctrl | alt, '-'));
            t.one(GoForward, Global, c(ctrl | shift, '-'));
        }
    }
    t.one(Settings, Global, c(cmd, ','));
    if !mac {
        t.one(Settings, Global, c(ctrl | alt, 's'));
    }
    t.chord(KeyboardShortcuts, Global, chord_k, c(cmd, 's'));

    t.one(NewFile, Global, c(cmd, 'n'));
    t.one(NewWindow, Global, c(cmd | shift, 'n'));
    t.one(OpenFile, Global, c(cmd, 'o'));
    if !mac {
        t.chord(OpenFolder, Global, chord_k, c(ctrl, 'o'));
    }
    // macOS 上 ⌃R，其他系统上 Ctrl+R。单按 ⌃ 或 Ctrl 的组合在终端里交给 shell（反向搜索历史）。
    t.one(OpenRecent, Global, c(ctrl, 'r'));
    t.one(Save, Global, c(cmd, 's'));
    t.one(SaveAs, Global, c(cmd | shift, 's'));
    if os == Os::Windows {
        t.one(CloseTab, Global, key(ctrl, F(4)));
    }
    // 没有打开的标签时，macOS 上的 ⌘W 也关闭窗口；这条随打开的标签数变化，不在表里，
    // 见 `App::keyboard_action`。
    t.one(CloseTab, Global, c(cmd, 'w'));
    t.one(CloseWindow, Global, c(cmd | shift, 'w'));
    if mac {
        t.one(CloseOtherTabs, Global, c(cmd | alt, 't'));
    }
    t.chord(CloseAllTabs, Global, chord_k, c(cmd, 'w'));
    t.one(ReopenClosed, Global, c(cmd | shift, 't'));
    if mac {
        t.one(NextTab, Global, key(cmd | alt, Right));
        t.one(NextTab, Global, c(cmd | shift, ']'));
        t.one(PreviousTab, Global, key(cmd | alt, Left));
        t.one(PreviousTab, Global, c(cmd | shift, '['));
    } else {
        t.one(NextTab, Global, key(ctrl, PageDown));
        t.one(PreviousTab, Global, key(ctrl, PageUp));
    }
    t.chord(CloseFolder, Global, chord_k, c(none, 'f'));
    match os {
        Os::Mac => t.one(Quit, Global, c(cmd, 'q')),
        Os::Linux => t.one(Quit, Global, c(ctrl, 'q')),
        Os::Windows => {}
    }

    t.one(Paste, Global, c(cmd, 'v'));
    t.one(Find, Global, c(cmd, 'f'));
    if mac {
        t.one(Replace, Global, c(cmd | alt, 'f'));
    } else {
        t.one(Replace, Global, c(ctrl, 'h'));
    }
    t.one(FindInFiles, Global, c(cmd | shift, 'f'));
    t.one(ReplaceInFiles, Global, c(cmd | shift, 'h'));
    t.one(NextResult, Global, key(none, F(4)));
    t.one(PreviousResult, Global, key(shift, F(4)));
    // 同一命令有两组绑定，提示中显示先登记的 `F7` / `⇧F7`。
    t.one(NextDifference, Editor, key(none, F(7)));
    t.one(PreviousDifference, Editor, key(shift, F(7)));
    t.one(NextDifference, Editor, key(alt, F(5)));
    t.one(PreviousDifference, Editor, key(alt | shift, F(5)));
    t.one(NextDiffFile, Editor, key(none, F(8)));
    t.one(PreviousDiffFile, Editor, key(shift, F(8)));
    t.chord(GitRevertChange, Editor, chord_k, c(cmd, 'r'));

    t.one(ToggleSidebar, Global, c(cmd, 'b'));
    t.one(FocusSidebar, Global, c(cmd, '0'));
    t.one(ShowFiles, Global, c(cmd | shift, 'e'));
    t.one(ShowCommit, Global, c(ctrl | shift, 'g'));
    // 大纲与 Git 日志（`⌘9`）同为按数字键打开的工具视图。
    t.one(ShowOutline, Global, c(cmd, '7'));
    // 打开 Git 工具窗口（日志）的键。
    t.one(GitShowLog, Global, c(cmd, '9'));
    // ⌃` 开关终端；在终端里按也要先给它，所以是 `Everywhere`。
    t.one(ToggleTerminal, Everywhere, c(ctrl, '`'));
    // ⌘J 也开关底部面板。是全局的：macOS 上带 ⌘，在终端里也生效；其他系统上是
    // Ctrl+J，焦点在终端时交给 shell（Ctrl+J 是换行）。
    t.one(ToggleTerminal, Global, c(cmd, 'j'));
    t.one(RunConfiguration, Global, key(none, F(5)));
    t.one(StopConfiguration, Global, key(shift, F(5)));
    t.one(ClearTerminal, Terminal, c(term, 'k'));
    t.one(IncreaseFontSize, Global, c(cmd, '='));
    t.one(IncreaseFontSize, Global, c(cmd | shift, '='));
    t.one(IncreaseFontSize, Global, key(cmd, Numpad('+')));
    t.one(DecreaseFontSize, Global, c(cmd, '-'));
    if !linux {
        t.one(DecreaseFontSize, Global, c(cmd | shift, '-'));
    }
    t.one(DecreaseFontSize, Global, key(cmd, Numpad('-')));
    t.one(ResetFontSize, Global, key(cmd, Numpad('0')));
    // 界面缩放：整个窗口放大、缩小，与调字号的键分开；焦点在终端时同样生效。
    let zoom = match os {
        Os::Mac => ctrl | alt,
        Os::Windows => alt | shift,
        Os::Linux => ctrl | alt | shift,
    };
    t.one(ZoomIn, Everywhere, c(zoom, '='));
    t.one(ZoomOut, Everywhere, c(zoom, '-'));
    t.one(ResetZoom, Everywhere, c(zoom, '0'));

    t.0
}
