//! 解析器的测试：寄存器、次数、操作符、动作、文本对象的组合，等待后续按键、取消与无效序列。

use super::*;
use crate::command::Shape;
use crate::key::notation;

/// 在 `mode` 下把 `keys`（按键记法）依次交给一个新的解析器，返回每个按键的结果。
fn feed(mode: Mode, keys: &str) -> Vec<Outcome> {
    let mut parser = Parser::default();
    (notation::parse(keys).into_iter())
        .map(|key| parser.feed(mode, key))
        .collect()
}

/// 返回 `keys` 在 `mode` 下解析出的唯一一条命令；此前的每个按键都必须是等待后续按键。
fn command(mode: Mode, keys: &str) -> Command {
    let outcomes = feed(mode, keys);
    let (last, pending) = outcomes.split_last().expect("按键序列不能为空");
    assert!(
        pending.iter().all(|o| *o == Outcome::Pending),
        "{keys}：{outcomes:?}"
    );
    match last {
        Outcome::Command(command) => *command,
        other => panic!("{keys}：最后一个按键的结果是 {other:?}"),
    }
}

/// 返回 `keys` 在普通模式下解析出的命令。
fn normal(keys: &str) -> Command {
    command(Mode::Normal, keys)
}

/// 返回 `keys` 在普通模式下最后一个按键的结果。
fn last(keys: &str) -> Outcome {
    *feed(Mode::Normal, keys).last().expect("按键序列不能为空")
}

fn motion(motion: Motion) -> Movement {
    Movement { motion, arg: None }
}

fn operator(operator: Operator, target: Target) -> Body {
    Body::Operator {
        operator,
        target,
        force: None,
    }
}

fn action(action: Action) -> Body {
    Body::Action { action, arg: None }
}

#[test]
fn single_keys_are_motions_and_actions() {
    let w = normal("w");
    assert_eq!(w.body, Body::Motion(motion(Motion::WordForward)));
    assert_eq!((w.register, w.count), (None, None));
    assert_eq!(normal("p").body, action(Action::PasteAfter));
    assert_eq!(normal("<C-r>").body, action(Action::Redo));
    assert_eq!(normal("<BS>").body, Body::Motion(motion(Motion::Backspace)));
    assert_eq!(normal("<Space>").body, Body::Motion(motion(Motion::Space)));
    assert_eq!(normal("gg").body, Body::Motion(motion(Motion::FirstLine)));
}

#[test]
fn synonyms_expand_to_operator_commands() {
    let right = Target::Motion(motion(Motion::Right));
    let line_end = Target::Motion(motion(Motion::LineEnd));
    for (keys, expected) in [
        ("x", operator(Operator::Delete, right)),
        ("<Del>", operator(Operator::Delete, right)),
        (
            "X",
            operator(Operator::Delete, Target::Motion(motion(Motion::Left))),
        ),
        ("D", operator(Operator::Delete, line_end)),
        ("C", operator(Operator::Change, line_end)),
        ("s", operator(Operator::Change, right)),
        ("S", operator(Operator::Change, Target::Lines)),
        ("Y", operator(Operator::Yank, Target::Lines)),
    ] {
        assert_eq!(normal(keys).body, expected, "{keys}");
    }
    let x = normal("\"a3x");
    assert_eq!((x.register, x.count), (Some('a'), Some(3)));
    // 等待动作时这些按键不是动作。
    assert_eq!(last("dx"), Outcome::Invalid);
    assert_eq!(last("dD"), Outcome::Invalid);
}

#[test]
fn counts_accumulate_digits_and_zero_is_a_motion_without_one() {
    assert_eq!(normal("3w").count, Some(3));
    assert_eq!(normal("10w").count, Some(10));
    assert_eq!(normal("105x").count, Some(105));
    let zero = normal("0");
    assert_eq!(zero.body, Body::Motion(motion(Motion::LineStart)));
    assert_eq!(zero.count, None);
    // `g` 之后的数字属于按键序列，不是次数。
    assert_eq!(
        normal("g0").body,
        Body::Motion(motion(Motion::DisplayStart))
    );
    assert_eq!(last("2g3"), Outcome::Invalid);
}

#[test]
fn counts_saturate_at_the_limit() {
    assert_eq!(normal("999999999x").count, Some(MAX_COUNT));
    assert_eq!(normal("1000000000x").count, Some(MAX_COUNT));
    assert_eq!(normal("99999999999999999999999x").count, Some(MAX_COUNT));
    assert_eq!(normal("99999d99999w").count, Some(MAX_COUNT));
    assert_eq!(normal("99999999x").count, Some(99_999_999));
}

#[test]
fn delete_removes_the_last_digit_of_a_count() {
    assert_eq!(normal("12<Del>x").count, Some(1));
    assert_eq!(normal("123<Del><Del>4x").count, Some(14));
    // 删除仅有的一位数字之后没有未完成的命令，再按 `<Del>` 是删除字符的命令。
    assert_eq!(
        feed(Mode::Normal, "1<Del><Del>"),
        [
            Outcome::Pending,
            Outcome::Cancelled,
            Outcome::Command(normal("x")),
        ]
    );
    assert_eq!(last("10<Del><Del>x"), Outcome::Command(normal("x")));
    // 其余部分仍未完成时继续等待。
    let a = normal("\"a5<Del>x");
    assert_eq!((a.register, a.count), (Some('a'), None));
    assert_eq!(normal("d2<Del>w").count, None);
    // 没有次数时 `<Del>` 不是动作，等待动作时无效。
    assert_eq!(last("d<Del>"), Outcome::Invalid);
    assert_eq!(last("d2<Del>x"), Outcome::Invalid);
}

#[test]
fn registers_and_counts_combine_in_any_order() {
    let a = normal("\"ax");
    assert_eq!((a.register, a.count), (Some('a'), None));
    let a = normal("\"a3x");
    assert_eq!((a.register, a.count), (Some('a'), Some(3)));
    let a = normal("3\"ax");
    assert_eq!((a.register, a.count), (Some('a'), Some(3)));
    // 寄存器前后的次数相乘，寄存器取最后一个。
    let a = normal("2\"a3x");
    assert_eq!((a.register, a.count), (Some('a'), Some(6)));
    let b = normal("2\"a3\"b4x");
    assert_eq!((b.register, b.count), (Some('b'), Some(24)));
    assert_eq!(normal("\"Ayy").register, Some('A'));
    for name in "\"-_+*/:.%07".chars() {
        assert_eq!(normal(&format!("\"{name}p")).register, Some(name));
    }
}

#[test]
fn invalid_register_names_discard_the_prefix() {
    assert_eq!(last("\"!"), Outcome::Invalid);
    assert_eq!(last("3\"<Tab>"), Outcome::Invalid);
    assert_eq!(last("\"<Left>"), Outcome::Invalid);
    assert_eq!(last("\"<C-a>"), Outcome::Invalid);
    assert_eq!(last("\"="), Outcome::Unsupported);
    assert_eq!(last("\"#"), Outcome::Unsupported);
    // 之后的按键是一条新的命令，不带寄存器。
    let outcomes = feed(Mode::Normal, "\"!x");
    assert_eq!(outcomes[2], Outcome::Command(normal("x")));
}

#[test]
fn operators_take_motions_and_text_objects() {
    let dw = normal("dw");
    assert_eq!(
        dw.body,
        operator(
            Operator::Delete,
            Target::Motion(motion(Motion::WordForward))
        )
    );
    let object = |shape, around| Target::Object(TextObject { shape, around });
    assert_eq!(
        normal("ciw").body,
        operator(Operator::Change, object(Shape::Word, false))
    );
    for keys in ["ya(", "ya)", "yab"] {
        assert_eq!(
            normal(keys).body,
            operator(Operator::Yank, object(Shape::Paren, true)),
            "{keys}"
        );
    }
    assert_eq!(
        normal("di\"").body,
        operator(Operator::Delete, object(Shape::DoubleQuote, false))
    );
    assert_eq!(normal("d2aw").count, Some(2));
    assert_eq!(
        normal("dgn").body,
        operator(Operator::Delete, object(Shape::NextMatch, false))
    );
    assert_eq!(
        normal("<lt>j").body,
        operator(Operator::Outdent, Target::Motion(motion(Motion::Down)))
    );
    assert_eq!(
        normal("gUgg").body,
        operator(
            Operator::Uppercase,
            Target::Motion(motion(Motion::FirstLine))
        )
    );
    assert_eq!(
        normal("g~ge").body,
        operator(
            Operator::ToggleCase,
            Target::Motion(motion(Motion::WordEndBackward))
        )
    );
    assert_eq!(
        normal("d0").body,
        operator(Operator::Delete, Target::Motion(motion(Motion::LineStart)))
    );
}

#[test]
fn counts_before_and_after_an_operator_multiply() {
    assert_eq!(normal("2dw").count, Some(2));
    assert_eq!(normal("d3w").count, Some(3));
    assert_eq!(normal("2d3w").count, Some(6));
    assert_eq!(normal("d10w").count, Some(10));
    let c = normal("3\"a2d2w");
    assert_eq!((c.register, c.count), (Some('a'), Some(12)));
    assert_eq!(normal("2d3d").count, Some(6));
    assert_eq!(normal("2g~3~").count, Some(6));
}

#[test]
fn doubled_operators_work_on_lines() {
    for (keys, expected) in [
        ("dd", Operator::Delete),
        ("cc", Operator::Change),
        ("yy", Operator::Yank),
        (">>", Operator::Indent),
        ("<lt><lt>", Operator::Outdent),
        ("g~~", Operator::ToggleCase),
        ("g~g~", Operator::ToggleCase),
        ("guu", Operator::Lowercase),
        ("gugu", Operator::Lowercase),
        ("gUU", Operator::Uppercase),
        ("gUgU", Operator::Uppercase),
    ] {
        assert_eq!(
            normal(keys).body,
            operator(expected, Target::Lines),
            "{keys}"
        );
    }
    assert_eq!(normal("3dd").count, Some(3));
    assert_eq!(normal("g~3~").count, Some(3));
    // 另一个操作符不是叠写形式。
    assert_eq!(last("dy"), Outcome::Invalid);
    assert_eq!(last("gUgu"), Outcome::Invalid);
    assert_eq!(last("gUu"), Outcome::Invalid);
    assert_eq!(last("g~U"), Outcome::Invalid);
}

#[test]
fn keys_after_an_invalid_sequence_start_a_new_command() {
    // 等待动作时输入另一个操作符：两者都丢弃，后一个不成为新的操作符。
    assert_eq!(
        feed(Mode::Normal, "dyw"),
        [
            Outcome::Pending,
            Outcome::Invalid,
            Outcome::Command(normal("w")),
        ]
    );
    assert_eq!(
        feed(Mode::Normal, "3gZx"),
        [
            Outcome::Pending,
            Outcome::Pending,
            Outcome::Invalid,
            Outcome::Command(normal("x")),
        ]
    );
    // 等待动作时不能指定寄存器：`"` 无效，其后的 `a` 是进入插入模式的命令。
    assert_eq!(
        feed(Mode::Normal, "d\"a"),
        [
            Outcome::Pending,
            Outcome::Invalid,
            Outcome::Command(normal("a")),
        ]
    );
    assert_eq!(last("dx"), Outcome::Invalid);
    assert_eq!(last("di"), Outcome::Pending);
    assert_eq!(last("diZ"), Outcome::Invalid);
}

#[test]
fn a_forced_motion_type_keeps_counts_and_the_last_one_wins() {
    let forced = |keys: &str| match normal(keys).body {
        Body::Operator { force, .. } => force,
        other => panic!("{keys}：{other:?}"),
    };
    assert_eq!(forced("dj"), None);
    assert_eq!(forced("dvj"), Some(Force::Charwise));
    assert_eq!(forced("dVl"), Some(Force::Linewise));
    assert_eq!(forced("d<C-v>j"), Some(Force::Blockwise));
    assert_eq!(forced("dvVj"), Some(Force::Linewise));
    assert_eq!(forced("dvd"), Some(Force::Charwise));
    for keys in ["d2v3j", "2dv3j", "6dvj", "dv6j", "d6vj"] {
        assert_eq!(normal(keys).count, Some(6), "{keys}");
    }
    // 没有操作符时 `v` 是进入可视模式的命令。
    assert_eq!(normal("v").body, action(Action::Visual));
}

#[test]
fn character_arguments_follow_the_keys_that_take_them() {
    let find = |keys: &str| match normal(keys).body {
        Body::Motion(movement) => movement,
        other => panic!("{keys}：{other:?}"),
    };
    assert_eq!(
        find("fx"),
        Movement {
            motion: Motion::FindForward,
            arg: Some('x'),
        }
    );
    assert_eq!(find("T中").arg, Some('中'));
    assert_eq!(find("f<Tab>").arg, Some('\t'));
    assert_eq!(find("f<CR>").arg, Some('\r'));
    assert_eq!(find("f<Space>").arg, Some(' '));
    assert_eq!(find("f<lt>").arg, Some('<'));
    // 参数位置上的数字、`"` 与操作符的按键都是字符。
    assert_eq!(find("f3").arg, Some('3'));
    assert_eq!(find("t\"").arg, Some('"'));
    assert_eq!(find("fd").arg, Some('d'));
    assert_eq!(
        normal("3rx").body,
        Body::Action {
            action: Action::ReplaceChar,
            arg: Some('x'),
        }
    );
    assert_eq!(
        normal("2d3tx").body,
        operator(
            Operator::Delete,
            Target::Motion(Movement {
                motion: Motion::TillForward,
                arg: Some('x'),
            })
        )
    );
    assert_eq!(last("f<Left>"), Outcome::Invalid);
    assert_eq!(last("f<C-a>"), Outcome::Invalid);
    assert_eq!(last("r<F1>"), Outcome::Invalid);
    assert_eq!(last("df<M-x>"), Outcome::Invalid);
}

#[test]
fn escape_cancels_whatever_is_pending() {
    for keys in [
        "3", "\"", "\"a", "d", "g", "f", "dg", "d2", "df", "dv", "2\"a3d4",
    ] {
        let outcomes = feed(Mode::Normal, &format!("{keys}<Esc>x"));
        let (rest, tail) = outcomes.split_at(outcomes.len() - 2);
        assert!(rest.iter().all(|o| *o == Outcome::Pending), "{keys}");
        assert_eq!(
            tail,
            [Outcome::Cancelled, Outcome::Command(normal("x"))],
            "{keys}"
        );
    }
    // 没有未完成的命令时，普通模式的 `<Esc>` 没有定义。
    assert_eq!(last("<Esc>"), Outcome::Unbound);
    assert_eq!(last("x<Esc>"), Outcome::Unbound);
}

#[test]
fn unbound_keys_are_reported_only_when_idle() {
    assert_eq!(last("\\"), Outcome::Unbound);
    assert_eq!(last("中"), Outcome::Unbound);
    assert_eq!(last("<C-S-p>"), Outcome::Unbound);
    assert_eq!(last("<S-Home>"), Outcome::Unbound);
    assert_eq!(last("<F5>"), Outcome::Unbound);
    assert_eq!(last("3\\"), Outcome::Invalid);
    assert_eq!(last("\"a\\"), Outcome::Invalid);
    assert_eq!(last("dZ"), Outcome::Invalid);
    assert_eq!(last("d<F5>"), Outcome::Invalid);
    // 带修饰键的数字不是次数。
    assert_eq!(last("<S-1>"), Outcome::Unbound);
    assert_eq!(last("<C-1>"), Outcome::Unbound);
}

#[test]
fn unsupported_commands_are_read_to_the_end() {
    assert_eq!(last("Q"), Outcome::Unsupported);
    assert_eq!(last("3K"), Outcome::Unsupported);
    assert_eq!(last("\"aU"), Outcome::Unsupported);
    // 不支持的操作符读完范围之后才报告，范围的按键不被当作其他命令。
    assert_eq!(
        feed(Mode::Normal, "gqiwx"),
        [
            Outcome::Pending,
            Outcome::Pending,
            Outcome::Pending,
            Outcome::Unsupported,
            Outcome::Command(normal("x")),
        ]
    );
    assert_eq!(last("gqq"), Outcome::Unsupported);
    assert_eq!(last("gqgq"), Outcome::Unsupported);
    assert_eq!(last("2gq3j"), Outcome::Unsupported);
    assert_eq!(last("gq<Esc>"), Outcome::Cancelled);
    // 之后的命令不受影响。
    assert_eq!(
        *feed(Mode::Normal, "gqjdd").last().unwrap(),
        Outcome::Command(normal("dd"))
    );
}

#[test]
fn visual_mode_applies_operators_to_the_selection() {
    let visual = |keys: &str| command(Mode::Visual, keys);
    assert_eq!(
        visual("d").body,
        operator(Operator::Delete, Target::Selection)
    );
    assert_eq!(
        visual("x").body,
        operator(Operator::Delete, Target::Selection)
    );
    assert_eq!(
        visual("<Del>").body,
        operator(Operator::Delete, Target::Selection)
    );
    assert_eq!(
        visual("u").body,
        operator(Operator::Lowercase, Target::Selection)
    );
    assert_eq!(
        visual("U").body,
        operator(Operator::Uppercase, Target::Selection)
    );
    assert_eq!(
        visual("~").body,
        operator(Operator::ToggleCase, Target::Selection)
    );
    assert_eq!(
        visual("g~").body,
        operator(Operator::ToggleCase, Target::Selection)
    );
    let y = visual("3\"ay");
    assert_eq!((y.register, y.count), (Some('a'), Some(3)));

    assert_eq!(visual("3j").body, Body::Motion(motion(Motion::Down)));
    assert_eq!(
        visual("iw").body,
        Body::Object(TextObject {
            shape: Shape::Word,
            around: false
        })
    );
    // `X`、`Y`、`D`、`C` 作用于整行，按块时作用于块；`R` 总是作用于整行。
    assert_eq!(
        visual("D").body,
        operator(Operator::Delete, Target::SelectedLines { to_end: true })
    );
    assert_eq!(
        visual("Y").body,
        operator(Operator::Yank, Target::SelectedLines { to_end: false })
    );
    assert_eq!(
        visual("R").body,
        Body::Operator {
            operator: Operator::Change,
            target: Target::Selection,
            force: Some(Force::Linewise),
        }
    );
    // `S{符号}` 在选区的两端加上成对的符号；等待动作时 `ys`、`ds`、`cs` 各自读取符号。
    assert_eq!(
        visual("S)").body,
        operator(Operator::Surround(')'), Target::Selection)
    );
    // `<BS>` 删除选区，未指定寄存器时写入 `"-`。
    let small = visual("<BS>");
    assert_eq!(small.body, operator(Operator::Delete, Target::Selection));
    assert_eq!(small.register, Some('-'));
    assert_eq!(visual("\"a<BS>").register, Some('a'));
    // 大写的写法先把选区扩展到整行；`X`、`D`、`Y`、`C` 在按块时作用于块。
    for (keys, expected, to_end) in [
        ("X", Operator::Delete, false),
        ("D", Operator::Delete, true),
        ("Y", Operator::Yank, false),
        ("C", Operator::Change, true),
    ] {
        let body = operator(expected, Target::SelectedLines { to_end });
        assert_eq!(visual(keys).body, body, "{keys}");
    }
    assert_eq!(visual("v").body, action(Action::Visual));
    assert_eq!(
        visual("rx").body,
        Body::Action {
            action: Action::ReplaceChar,
            arg: Some('x'),
        }
    );
    // 操作符立即完成，其后的按键是新的命令。
    assert_eq!(
        feed(Mode::Visual, "dw"),
        [Outcome::Command(visual("d")), Outcome::Command(visual("w")),]
    );
}

#[test]
fn escape_in_visual_mode_leaves_it_unless_a_key_sequence_is_open() {
    let escape = action(Action::Escape);
    assert_eq!(command(Mode::Visual, "<Esc>").body, escape);
    // 只输入了寄存器或次数时仍然离开可视模式。
    assert_eq!(command(Mode::Visual, "\"a<Esc>").body, escape);
    assert_eq!(command(Mode::Visual, "2<Esc>").body, escape);
    // 多键序列或参数未输入完时只取消该序列。
    for keys in ["g<Esc>", "f<Esc>", "\"<Esc>", "i<Esc>", "r<Esc>"] {
        assert_eq!(
            feed(Mode::Visual, keys),
            [Outcome::Pending, Outcome::Cancelled],
            "{keys}"
        );
    }
}

#[test]
fn insert_mode_has_no_counts_or_registers() {
    assert_eq!(command(Mode::Insert, "<Esc>").body, action(Action::Escape));
    assert_eq!(feed(Mode::Insert, "<Esc><Esc>").len(), 2);
    for keys in ["3", "0", "\"", "d", "x", "中", "<Del>", "<Left>", "<C-a>"] {
        assert_eq!(feed(Mode::Insert, keys), [Outcome::Unbound], "{keys}");
    }
    // 插入模式的 `Ctrl` 键：`Ctrl-R` 之后是寄存器名。
    assert_eq!(
        command(Mode::Insert, "<C-w>").body,
        action(Action::InsertDeleteWord)
    );
    assert_eq!(
        command(Mode::Insert, "<C-r>a").body,
        Body::Action {
            action: Action::InsertRegister,
            arg: Some('a')
        }
    );
}

#[test]
fn pending_keys_are_kept_for_display() {
    let mut parser = Parser::default();
    let mut shown = Vec::new();
    for key in notation::parse("2\"a3d4f") {
        assert_eq!(parser.feed(Mode::Normal, key), Outcome::Pending);
        shown.push((
            notation::write(parser.pending()),
            parser.is_operator_pending(),
        ));
    }
    let shown: Vec<_> = shown.iter().map(|(s, o)| (s.as_str(), *o)).collect();
    assert_eq!(
        shown,
        [
            ("2", false),
            ("2\"", false),
            ("2\"a", false),
            ("2\"a3", false),
            ("2\"a3d", true),
            ("2\"a3d4", true),
            ("2\"a3d4f", true),
        ]
    );
    assert!(matches!(
        parser.feed(Mode::Normal, Key::char('x')),
        Outcome::Command(_)
    ));
    assert!(parser.pending().is_empty());
    assert!(!parser.is_operator_pending());

    // `<Del>` 删除的数字不再显示；达到上限之后的数字不改变次数，也不记录。
    for key in notation::parse("12<Del>") {
        parser.feed(Mode::Normal, key);
    }
    assert_eq!(notation::write(parser.pending()), "1");
    parser.reset();
    assert!(parser.pending().is_empty());
    for key in notation::parse(&"9".repeat(500)) {
        assert_eq!(parser.feed(Mode::Normal, key), Outcome::Pending);
    }
    assert_eq!(parser.pending().len(), 9);
    // 记录的按键数有上限，解析不受影响。
    parser.reset();
    for key in notation::parse(&"\"a".repeat(200)) {
        assert_eq!(parser.feed(Mode::Normal, key), Outcome::Pending);
    }
    assert_eq!(parser.pending().len(), MAX_PENDING);
    let outcome = parser.feed(Mode::Normal, Key::char('x'));
    assert_eq!(outcome, Outcome::Command(normal("\"ax")));
}

#[test]
fn a_mode_change_discards_the_pending_command() {
    let mut parser = Parser::default();
    assert_eq!(parser.feed(Mode::Normal, Key::char('d')), Outcome::Pending);
    assert_eq!(parser.feed(Mode::Normal, Key::char('g')), Outcome::Pending);
    // 可视模式下 `d` 直接作用于选区，普通模式下输入的 `dg` 不再有效。
    let outcome = parser.feed(Mode::Visual, Key::char('d'));
    assert_eq!(outcome, Outcome::Command(command(Mode::Visual, "d")));

    assert_eq!(parser.feed(Mode::Visual, Key::char('i')), Outcome::Pending);
    assert_eq!(parser.feed(Mode::Insert, Key::char('w')), Outcome::Unbound);
    assert!(parser.pending().is_empty());

    // 等待动作由解析器自身保存，传入该模式等同于普通模式。
    assert_eq!(parser.feed(Mode::Normal, Key::char('d')), Outcome::Pending);
    let outcome = parser.feed(Mode::OperatorPending, Key::char('w'));
    assert_eq!(outcome, Outcome::Command(normal("dw")));
    let outcome = parser.feed(Mode::OperatorPending, Key::char('x'));
    assert_eq!(outcome, Outcome::Command(normal("x")));
}
