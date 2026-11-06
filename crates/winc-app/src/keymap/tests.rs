//! `keymap` 的测试：三个系统上绑定表不冲突、组合键等第二段、「快捷键」页的行合并与搜索，
//! 差异标签接手的命令清单，以及输入法接管的按键。

use super::*;

const ALL_OS: [Os; 3] = [Os::Mac, Os::Windows, Os::Linux];

fn press(os: Os, mods: Mods, key: KeyCode, context: Context) -> Lookup {
    lookup(&bindings(os), Stroke { key, mods }, None, context)
}

fn runs(os: Os, mods: Mods, key: KeyCode, context: Context) -> Option<Action> {
    match press(os, mods, key, context) {
        Lookup::Run(actions) => actions.first().copied(),
        _ => None,
    }
}

#[test]
fn no_two_bindings_fight_over_a_key() {
    for os in ALL_OS {
        let table = bindings(os);
        for (i, a) in table.iter().enumerate() {
            for b in &table[i + 1..] {
                let same_key = a.first == b.first && a.second == b.second;
                if !same_key || a.when.mask() & b.when.mask() == 0 {
                    continue;
                }
                // 范围重叠时，前面那条的范围必须更小（优先匹配），且两者不能完全相同。
                let narrower = a.when.mask() & !b.when.mask() == 0 && a.when != b.when;
                assert!(
                    narrower || a.action == b.action,
                    "{os:?}: {} 同时是 {:?}（{:?}）和 {:?}（{:?}）",
                    a.label(os),
                    a.action,
                    a.when,
                    b.action,
                    b.when
                );
            }
            // 组合键的第一段不能同时对应一个命令，否则永远等不到第二段。终端里自己的单键
            // （⌘K 清屏）例外：在终端里本来就不等第二段，单键优先（`lookup`）。
            if a.second.is_some() {
                assert!(
                    !table.iter().any(|b| b.first == a.first
                        && b.second.is_none()
                        && b.when != When::Terminal
                        && b.when.mask() & a.when.mask() != 0),
                    "{os:?}: {} 的第一下自己也是命令",
                    a.label(os)
                );
            }
        }
    }
}

#[test]
fn every_action_is_listed_once_in_order() {
    for (i, a) in ACTIONS.iter().enumerate() {
        assert!(!ACTIONS[i + 1..].contains(a), "{a:?}");
    }
    for os in ALL_OS {
        for b in bindings(os) {
            assert!(ACTIONS.contains(&b.action), "{:?}", b.action);
        }
    }
}

#[test]
fn default_key_labels_per_system() {
    let (cmd, ctrl, shift, alt) = (Mods::META, Mods::CTRL, Mods::SHIFT, Mods::ALT);
    let label = |os: Os, action: Action| {
        bindings(os)
            .iter()
            .filter(|b| b.action == action)
            .map(|b| b.label(os))
            .collect::<Vec<_>>()
    };
    assert_eq!(label(Os::Mac, Action::ShowCommands), ["⇧⌘P", "F1"]);
    // 多光标的默认键位。
    assert_eq!(label(Os::Mac, Action::AddCursorBelow), ["⌥⌘↓"]);
    assert_eq!(label(Os::Windows, Action::AddCursorAbove), ["Ctrl+Alt+↑"]);
    assert_eq!(label(Os::Mac, Action::NextDifference)[0], "F7");
    assert_eq!(label(Os::Mac, Action::PreviousDifference)[0], "⇧F7");
    assert_eq!(label(Os::Mac, Action::CloseTerminal), ["⌘W"]);
    assert_eq!(label(Os::Windows, Action::CloseTerminal), ["Ctrl+Shift+W"]);
    assert_eq!(label(Os::Linux, Action::CloseTerminal), ["Ctrl+Shift+W"]);
    assert_eq!(label(Os::Linux, Action::AddCursorAbove), ["Shift+Alt+↑"]);
    assert_eq!(label(Os::Mac, Action::AddNextOccurrence), ["⌘D"]);
    assert_eq!(label(Os::Mac, Action::SkipToNextOccurrence), ["⌘K ⌘D"]);
    assert_eq!(label(Os::Mac, Action::SelectAllOccurrences), ["⇧⌘L"]);
    assert_eq!(
        label(Os::Windows, Action::SelectAllOccurrences),
        ["Ctrl+Shift+L"]
    );
    assert_eq!(label(Os::Mac, Action::RemoveSecondaryCursors), ["⎋"]);
    assert_eq!(label(Os::Mac, Action::AddCursorsToLineEnds), ["⌥⇧I"]);
    assert_eq!(label(Os::Mac, Action::CursorUndo), ["⌘U"]);
    assert_eq!(label(Os::Mac, Action::ColumnSelectDown), ["⌥⇧⌘↓"]);
    assert_eq!(
        label(Os::Windows, Action::ColumnSelectLeft),
        ["Ctrl+Shift+Alt+←"]
    );
    assert!(label(Os::Linux, Action::ColumnSelectUp).is_empty());
    assert_eq!(label(Os::Mac, Action::SelectAllMatches), ["⌥↩"]);
    assert_eq!(
        label(Os::Windows, Action::ShowCommands),
        ["Ctrl+Shift+P", "F1"]
    );
    assert_eq!(label(Os::Mac, Action::Settings), ["⌘,"]);
    assert_eq!(
        label(Os::Windows, Action::Settings),
        ["Ctrl+,", "Ctrl+Alt+S"]
    );
    assert_eq!(label(Os::Linux, Action::Settings), ["Ctrl+,", "Ctrl+Alt+S"]);
    // 备用键在编辑区之外同样生效。
    assert_eq!(
        runs(Os::Windows, ctrl | alt, KeyCode::Char('s'), Context::Other),
        Some(Action::Settings)
    );
    assert_eq!(label(Os::Mac, Action::Format), ["⌥⇧F"]);
    assert_eq!(label(Os::Linux, Action::Format), ["Ctrl+Shift+I"]);
    assert_eq!(label(Os::Mac, Action::FormatSelection), ["⌘K ⌘F"]);
    assert_eq!(label(Os::Mac, Action::GoBack), ["⌃-"]);
    assert_eq!(label(Os::Windows, Action::GoBack), ["Alt+←"]);
    assert_eq!(label(Os::Linux, Action::GoBack), ["Ctrl+Alt+-"]);
    assert_eq!(label(Os::Mac, Action::CopyPath), ["⌥⌘C", "⌘K ⌥⌘C"]);
    assert_eq!(label(Os::Linux, Action::RevealInFinder), ["Ctrl+Alt+R"]);
    assert_eq!(label(Os::Windows, Action::CloseTab), ["Ctrl+F4", "Ctrl+W"]);
    assert!(label(Os::Mac, Action::OpenFolder).is_empty());
    assert_eq!(label(Os::Linux, Action::OpenFolder), ["Ctrl+K Ctrl+O"]);
    assert_eq!(label(Os::Mac, Action::NextTab), ["⌥⌘→", "⇧⌘]"]);
    assert!(label(Os::Windows, Action::Quit).is_empty());
    assert_eq!(label(Os::Mac, Action::JoinLines), ["⌃J"]);
    assert!(label(Os::Windows, Action::JoinLines).is_empty());
    assert_eq!(label(Os::Mac, Action::TrimTrailingWhitespace), ["⌘K ⌘X"]);
    assert_eq!(
        label(Os::Linux, Action::TrimTrailingWhitespace),
        ["Ctrl+K Ctrl+X"]
    );
    assert_eq!(label(Os::Mac, Action::JumpToBracket), ["⇧⌘\\"]);
    assert_eq!(label(Os::Mac, Action::Fold), ["⌥⌘["]);
    assert_eq!(label(Os::Windows, Action::Unfold), ["Ctrl+Shift+]"]);
    assert_eq!(label(Os::Mac, Action::FoldAll), ["⌘K ⌘0"]);
    assert_eq!(label(Os::Linux, Action::FoldLevel3), ["Ctrl+K Ctrl+3"]);
    assert_eq!(label(Os::Windows, Action::JumpToBracket), ["Ctrl+Shift+\\"]);
    assert_eq!(label(Os::Mac, Action::ExpandSelection), ["⌃⇧⌘→", "⌃⇧→"]);
    assert_eq!(label(Os::Linux, Action::ShrinkSelection), ["Shift+Alt+←"]);
    assert!(label(Os::Mac, Action::SortLinesAscending).is_empty());
    assert_eq!(label(Os::Mac, Action::NextBookmark), ["⌥⇧⌘J"]);

    // 按键事件里的字母总是小写（`Stroke::from_event`），表里写大写的那条永远按不到。
    for os in ALL_OS {
        for b in bindings(os) {
            for stroke in [Some(b.first), b.second].into_iter().flatten() {
                if let KeyCode::Char(c) = stroke.key {
                    assert_eq!(c, c.to_ascii_lowercase(), "{os:?}: {}", b.label(os));
                }
            }
        }
    }
    assert_eq!(
        runs(
            Os::Mac,
            cmd | alt | shift,
            KeyCode::Char('j'),
            Context::Editor
        ),
        Some(Action::NextBookmark)
    );

    // ⌘J 开关终端，在终端里也生效；其他系统上 Ctrl+J 在终端里是 shell 的换行。
    for context in [Context::Editor, Context::Terminal] {
        assert_eq!(
            runs(Os::Mac, Mods::META, KeyCode::Char('j'), context),
            Some(Action::ToggleTerminal)
        );
    }
    assert_eq!(
        runs(Os::Linux, Mods::CTRL, KeyCode::Char('j'), Context::Editor),
        Some(Action::ToggleTerminal)
    );
    assert_eq!(
        runs(Os::Linux, Mods::CTRL, KeyCode::Char('j'), Context::Terminal),
        None
    );

    // 同一个键在不同的地方。
    assert_eq!(
        runs(Os::Mac, cmd | alt, KeyCode::Char('c'), Context::Find),
        Some(Action::ToggleCase)
    );
    assert_eq!(
        runs(Os::Mac, cmd | alt, KeyCode::Char('c'), Context::Tree),
        Some(Action::CopyPath)
    );
    assert_eq!(
        runs(Os::Mac, cmd | alt, KeyCode::Char('c'), Context::Panel),
        Some(Action::ToggleCase)
    );
    assert_eq!(
        runs(Os::Mac, cmd, KeyCode::Backspace, Context::Editor),
        Some(Action::DeleteAllLeft)
    );
    assert_eq!(
        runs(Os::Mac, cmd, KeyCode::Backspace, Context::Tree),
        Some(Action::DeleteFile)
    );
    assert_eq!(
        runs(Os::Mac, Mods::NONE, KeyCode::Enter, Context::Tree),
        Some(Action::RenameFile)
    );
    assert_eq!(
        runs(Os::Mac, Mods::NONE, KeyCode::Enter, Context::Editor),
        None
    );
    assert_eq!(
        runs(Os::Windows, Mods::NONE, KeyCode::Enter, Context::Tree),
        Some(Action::OpenSelected)
    );
    // ⌘- 缩小字号，⌃- 后退；Linux 上 Ctrl+- 缩小字号，Ctrl+⇧- 前进。
    assert_eq!(
        runs(Os::Mac, cmd, KeyCode::Char('-'), Context::Editor),
        Some(Action::ZoomOut)
    );
    assert_eq!(
        runs(Os::Mac, ctrl, KeyCode::Char('-'), Context::Editor),
        Some(Action::GoBack)
    );
    assert_eq!(
        runs(Os::Linux, ctrl | shift, KeyCode::Char('-'), Context::Editor),
        Some(Action::GoForward)
    );
    // 不带修饰键的字符是打字。
    assert_eq!(
        press(Os::Mac, Mods::NONE, KeyCode::Char('g'), Context::Editor),
        Lookup::None
    );
    assert_eq!(
        press(Os::Mac, alt, KeyCode::Char('c'), Context::Editor),
        Lookup::None
    );
}

#[test]
fn chords_wait_for_the_second_key() {
    let table = bindings(Os::Mac);
    let k = Stroke {
        key: KeyCode::Char('k'),
        mods: Mods::META,
    };
    assert_eq!(lookup(&table, k, None, Context::Editor), Lookup::Chord);
    let s = Stroke {
        key: KeyCode::Char('s'),
        mods: Mods::META,
    };
    assert_eq!(
        lookup(&table, s, Some(k), Context::Editor),
        Lookup::Run(vec![Action::KeyboardShortcuts])
    );
    // 第二段对不上：不执行任何命令。
    assert_eq!(
        lookup(&table, k, Some(k), Context::Editor),
        Lookup::Run(Vec::new())
    );
    // 编辑区用 `⌘K ⌥⌘C` 复制路径，文件树中没有该组合。
    let copy = Stroke {
        key: KeyCode::Char('c'),
        mods: Mods::META | Mods::ALT,
    };
    assert_eq!(
        lookup(&table, copy, Some(k), Context::Editor),
        Lookup::Run(vec![Action::CopyPath])
    );
    assert_eq!(
        lookup(&table, copy, Some(k), Context::Tree),
        Lookup::Run(Vec::new())
    );
}

#[test]
fn rows_merge_identical_keys_and_split_different_ones() {
    let all: Vec<Row> = rows(&bindings(Os::Mac), Os::Mac)
        .into_iter()
        .flat_map(|(_, r)| r)
        .collect();
    let of = |action| {
        all.iter()
            .filter(|r| r.action == action)
            .collect::<Vec<_>>()
    };
    // 查找条和搜索面板里都是 ⌥⌘C：一行。
    let case = of(Action::ToggleCase);
    assert_eq!(case.len(), 1);
    assert_eq!(case[0].keys, ["⌥⌘C"]);
    assert_eq!(case[0].when, When::Global);
    // 复制路径在编辑区与非编辑区的键不同：拆成两行，各自标注生效范围。
    let copy = of(Action::CopyPath);
    assert_eq!(copy.len(), 2);
    assert_eq!(copy[0].when.note(), Some("非编辑区"));
    assert_eq!(copy[1].when.note(), Some("编辑区、查找栏"));
    // 没有键的也列。
    assert!(of(Action::Fix)[0].keys.is_empty());
    // 「常用」组中只在编辑区生效的命令标注生效范围；「编辑」组的命令不标注。
    assert_eq!(of(Action::GoToDefinition)[0].when.note(), Some("编辑区"));
    assert_eq!(of(Action::ToggleComment)[0].when.note(), None);
}

#[test]
fn search_matches_names_and_keys() {
    let all: Vec<Row> = rows(&bindings(Os::Mac), Os::Mac)
        .into_iter()
        .flat_map(|(_, r)| r)
        .collect();
    let found = |q: &str| {
        all.iter()
            .filter(|r| row_matches(r, q, Os::Mac, false))
            .map(|r| r.action)
            .collect::<Vec<_>>()
    };
    assert!(found("格式化").contains(&Action::Format));
    assert!(found("format sel").contains(&Action::FormatSelection));
    assert_eq!(found("⌘K ⌘S"), [Action::KeyboardShortcuts]);
    assert!(found("cmd k").contains(&Action::FormatSelection));
    assert!(found("shift cmd p").contains(&Action::ShowCommands));
    assert!(found("没有这个").is_empty());
    // 录制的键：只匹配以所录各段开头的键，⌘K 不会匹配 ⌥⌘K。
    let recorded = |q: &str| {
        all.iter()
            .filter(|r| row_matches(r, q, Os::Mac, true))
            .map(|r| r.action)
            .collect::<Vec<_>>()
    };
    assert_eq!(recorded("⌘K ⌘S"), [Action::KeyboardShortcuts]);
    assert!(recorded("⌘K").contains(&Action::FormatSelection));
    assert!(!recorded("⌘K").contains(&Action::RecordKeys));
    assert!(recorded("⌘S").contains(&Action::Save));
    assert!(!recorded("⌘S").contains(&Action::KeyboardShortcuts));
    let linux: Vec<Row> = rows(&bindings(Os::Linux), Os::Linux)
        .into_iter()
        .flat_map(|(_, r)| r)
        .collect();
    assert!(linux.iter().any(
            |r| r.action == Action::Format && row_matches(r, "ctrl+shift+i", Os::Linux, false)
        ));
}

/// 差异右侧接手的命令正好是这份清单：「编辑」分组去掉折叠、书签和缩进、换行符设置，
/// 加上保存和「选中全部匹配」。
#[test]
fn edits_in_diff_matches_the_diff_command_list() {
    use Action::*;
    let expected = [
        Save,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        SelectAll,
        ToggleComment,
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
        SelectAllMatches,
        JumpToBracket,
        TransformToUppercase,
        TransformToLowercase,
        TransformToTitlecase,
        SortLinesAscending,
        SortLinesDescending,
        TrimTrailingWhitespace,
        Format,
        FormatSelection,
        Fix,
    ];
    for &action in ACTIONS {
        assert_eq!(
            action.edits_in_diff(),
            expected.contains(&action),
            "{action:?}"
        );
    }
}

#[test]
fn the_tree_gets_plain_letters_and_paging_keys() {
    // 以下按键由文件树自行处理：键入字母、数字跳到对应行，`PageUp`、`PageDown` 翻一屏。
    // `Home`、`End` 在键位表中没有绑定。
    for os in ALL_OS {
        for key in [
            KeyCode::Char('d'),
            KeyCode::Char('7'),
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert_eq!(
                runs(os, Mods::NONE, key, Context::Tree),
                None,
                "{os:?} {key:?}"
            );
        }
    }
}

#[test]
fn a_key_taken_by_the_input_method_still_matches_with_ctrl() {
    use winit::keyboard::{
        Key, KeyCode as Code, KeyLocation, ModifiersState, NamedKey, PhysicalKey,
    };

    let stroke = |physical, m| {
        let key = Key::Named(NamedKey::Process);
        Stroke::from_event(&key, "", physical, KeyLocation::Standard, m)
    };
    let comma = PhysicalKey::Code(Code::Comma);

    assert_eq!(
        stroke(comma, ModifiersState::CONTROL),
        Some(Stroke {
            key: KeyCode::Char(','),
            mods: Mods::CTRL,
        })
    );
    assert_eq!(
        stroke(
            PhysicalKey::Code(Code::KeyS),
            ModifiersState::CONTROL | ModifiersState::ALT
        ),
        Some(Stroke {
            key: KeyCode::Char('s'),
            mods: Mods::CTRL | Mods::ALT,
        })
    );
    // 没按 Ctrl 是在打字；不对应字符的键没有快捷键可匹配。
    assert_eq!(stroke(comma, ModifiersState::empty()), None);
    assert_eq!(stroke(comma, ModifiersState::SHIFT), None);
    assert_eq!(
        stroke(PhysicalKey::Code(Code::F1), ModifiersState::CONTROL),
        None
    );
}

#[test]
fn diff_navigation_keys_reach_their_commands_from_key_events() {
    use winit::keyboard::{
        Key, KeyCode as Code, KeyLocation, ModifiersState, NamedKey, PhysicalKey,
    };

    let shift = ModifiersState::SHIFT;
    let alt = ModifiersState::ALT;
    let cases = [
        (
            NamedKey::F7,
            Code::F7,
            ModifiersState::empty(),
            Action::NextDifference,
        ),
        (NamedKey::F7, Code::F7, shift, Action::PreviousDifference),
        (NamedKey::F5, Code::F5, alt, Action::NextDifference),
        (
            NamedKey::F5,
            Code::F5,
            alt | shift,
            Action::PreviousDifference,
        ),
        (
            NamedKey::F8,
            Code::F8,
            ModifiersState::empty(),
            Action::NextDiffFile,
        ),
        (NamedKey::F8, Code::F8, shift, Action::PreviousDiffFile),
    ];
    for os in ALL_OS {
        let table = bindings(os);
        for (named, code, mods, action) in cases {
            let stroke = Stroke::from_event(
                &Key::Named(named),
                "",
                PhysicalKey::Code(code),
                KeyLocation::Standard,
                mods,
            )
            .unwrap();
            let found = lookup(&table, stroke, None, Context::Editor);
            assert!(
                matches!(&found, Lookup::Run(actions) if actions.contains(&action)),
                "{os:?} {named:?} {mods:?}"
            );
            assert!(action.navigates_diff());
        }
    }
}
