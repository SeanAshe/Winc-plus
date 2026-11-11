//! 命令表的测试：记录的写法、各模式下的前缀关系，以及前缀树的查找。

use super::*;
use crate::key::{Key, notation};

#[test]
fn every_entry_is_written_in_canonical_notation() {
    for entry in ENTRIES {
        let keys = notation::parse(entry.keys);
        assert!(!keys.is_empty(), "{entry:?}");
        assert_eq!(notation::write(&keys), entry.keys, "{entry:?}");
        assert!(
            Mode::ALL.iter().any(|m| entry.modes.contains(*m)),
            "{entry:?}"
        );
    }
}

#[test]
fn no_entry_is_a_prefix_of_another_in_the_same_mode() {
    let (_, rejected) = Table::build();
    let rejected: Vec<_> = (rejected.iter())
        .map(|(mode, index)| (mode, ENTRIES[*index].keys))
        .collect();
    assert!(rejected.is_empty(), "按键序列重复或互为前缀：{rejected:?}");
}

#[test]
fn prefix_kinds_stay_out_of_the_modes_that_do_not_take_them() {
    for entry in ENTRIES {
        match entry.kind {
            // 等待动作时不能再指定寄存器。
            Kind::Register => assert!(!entry.modes.contains(Mode::OperatorPending)),
            Kind::Force(_) => assert_eq!(entry.modes, Modes::O),
            Kind::TextObject(_) => assert!(!entry.modes.contains(Mode::Normal)),
            Kind::Operator(_) => assert!(!entry.modes.contains(Mode::OperatorPending)),
            Kind::Action(_) => assert!(!entry.modes.contains(Mode::OperatorPending)),
            // 成对符号只在等待动作与可视模式下出现。
            Kind::Surround => assert!(!entry.modes.contains(Mode::Normal)),
            Kind::Motion(_)
            | Kind::SmallDelete
            | Kind::LineOperator(_)
            | Kind::WideOperator(..)
            | Kind::Synonym(..) => {}
        }
    }
}

/// 在 `mode` 的前缀树中依次查找 `keys`，返回最后一步的结果。
fn walk(mode: Mode, keys: &str) -> Lookup {
    let table = Table::get();
    let mut node = ROOT;
    let mut last = Lookup::None;
    for key in notation::parse(keys) {
        last = table.step(mode, node, key);
        match last {
            Lookup::Prefix(next) => node = next,
            Lookup::Entry(_) | Lookup::None => node = ROOT,
        }
    }
    last
}

/// 返回 `mode` 下 `keys` 匹配的记录的种类。
fn kind(mode: Mode, keys: &str) -> Option<Kind> {
    match walk(mode, keys) {
        Lookup::Entry(index) => Table::get().entry(index).map(|e| e.kind),
        _ => None,
    }
}

#[test]
fn the_same_keys_mean_different_things_in_different_modes() {
    assert_eq!(
        kind(Mode::Normal, "x"),
        Some(Kind::Synonym(
            Operator::Delete,
            Shorthand::Motion(Motion::Right)
        ))
    );
    assert_eq!(
        kind(Mode::Visual, "x"),
        Some(Kind::Operator(Operator::Delete))
    );
    assert_eq!(kind(Mode::OperatorPending, "x"), None);
    assert_eq!(kind(Mode::Normal, "u"), Some(Kind::Action(Action::Undo)));
    assert_eq!(
        kind(Mode::Visual, "u"),
        Some(Kind::Operator(Operator::Lowercase))
    );
    assert_eq!(kind(Mode::Normal, "v"), Some(Kind::Action(Action::Visual)));
    assert_eq!(
        kind(Mode::OperatorPending, "v"),
        Some(Kind::Force(Force::Charwise))
    );
    assert_eq!(
        kind(Mode::Normal, "<C-r>"),
        Some(Kind::Action(Action::Redo))
    );
    assert_eq!(
        kind(Mode::Insert, "<C-r>"),
        Some(Kind::Action(Action::InsertRegister))
    );
    assert_eq!(kind(Mode::Insert, "<C-a>"), None);
    assert_eq!(
        kind(Mode::Insert, "<Esc>"),
        Some(Kind::Action(Action::Escape))
    );
    assert_eq!(
        kind(Mode::Normal, "<BS>"),
        Some(Kind::Motion(Motion::Backspace))
    );
    assert_eq!(kind(Mode::Visual, "<BS>"), Some(Kind::SmallDelete));

    // `i` 在普通模式下是命令，在可视模式和等待动作时是文本对象的前缀。
    assert_eq!(kind(Mode::Normal, "i"), Some(Kind::Action(Action::Insert)));
    assert!(matches!(walk(Mode::Visual, "i"), Lookup::Prefix(_)));
    let inner_word = TextObject {
        shape: Shape::Word,
        around: false,
    };
    assert_eq!(kind(Mode::Visual, "iw"), Some(Kind::TextObject(inner_word)));
    assert_eq!(
        kind(Mode::OperatorPending, "iw"),
        Some(Kind::TextObject(inner_word))
    );
    assert_eq!(walk(Mode::Insert, "i"), Lookup::None);
}

#[test]
fn operators_are_found_while_one_is_pending() {
    // 普通模式的操作符在等待动作时也能查到，用于识别叠写形式；可视模式专用的写法不在其中。
    for keys in [
        "d", "c", "y", ">", "<lt>", "g~", "gu", "gU", "g?", "gc", "=", "gq",
    ] {
        assert!(
            matches!(kind(Mode::Normal, keys), Some(Kind::Operator(_))),
            "{keys}"
        );
        assert_eq!(
            kind(Mode::OperatorPending, keys),
            kind(Mode::Normal, keys),
            "{keys}"
        );
    }
    assert_eq!(kind(Mode::OperatorPending, "~"), None);
    assert_eq!(kind(Mode::OperatorPending, "U"), None);
}

#[test]
fn lookups_walk_multi_key_sequences_and_reject_unknown_keys() {
    let table = Table::get();
    assert!(matches!(walk(Mode::Normal, "g"), Lookup::Prefix(_)));
    assert_eq!(
        kind(Mode::Normal, "gg"),
        Some(Kind::Motion(Motion::FirstLine))
    );
    assert_eq!(
        kind(Mode::Normal, "ge"),
        Some(Kind::Motion(Motion::WordEndBackward))
    );
    assert_eq!(walk(Mode::Normal, "gZ"), Lookup::None);
    assert_eq!(walk(Mode::Normal, "\\"), Lookup::None);
    assert_eq!(
        kind(Mode::Normal, "<Left>"),
        Some(Kind::Motion(Motion::Left))
    );
    assert_eq!(walk(Mode::Normal, "<S-Home>"), Lookup::None);

    // 越界的节点与下标不 panic。
    assert_eq!(
        table.step(Mode::Normal, u16::MAX, Key::char('g')),
        Lookup::None
    );
    assert!(table.entry(u16::MAX).is_none());
    assert!(table.keys(u16::MAX).is_empty());
    let index = u16::try_from(ENTRIES.len() - 1).unwrap();
    assert_eq!(
        table.keys(index),
        notation::parse(ENTRIES[usize::from(index)].keys)
    );
}

#[test]
fn only_implemented_commands_are_marked_supported() {
    // 之后的阶段实现的命令不标为支持；标为不支持的命令保持不支持。
    for entry in ENTRIES {
        match entry.kind {
            Kind::Action(Action::Later) => {
                assert_ne!(entry.support, Support::Supported, "{entry:?}");
            }
            Kind::Action(Action::ExMode | Action::Keyword | Action::UndoLine)
            | Kind::Operator(Operator::Format | Operator::Fold | Operator::Filter) => {
                assert_eq!(entry.support, Support::Unsupported, "{entry:?}");
            }
            _ => assert_eq!(entry.support, Support::Supported, "{entry:?}"),
        }
    }
}

#[test]
fn register_names_follow_the_supported_set() {
    for c in "\"-_+*/:.%09azAZ".chars() {
        assert_eq!(register_name(c), RegisterName::Valid, "{c}");
    }
    assert_eq!(register_name('='), RegisterName::Unsupported);
    assert_eq!(register_name('#'), RegisterName::Unsupported);
    for c in "!@$^&()[]{}<>,;'`~?| \t中é".chars() {
        assert_eq!(register_name(c), RegisterName::Invalid, "{c}");
    }
}
