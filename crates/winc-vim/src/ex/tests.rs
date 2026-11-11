use super::*;

fn address(base: Base, offset: isize, moves: bool) -> Address {
    Address {
        base,
        offset,
        moves,
    }
}

#[test]
fn ranges_are_split_into_addresses() {
    let line = parse("2,$-1d x").unwrap();
    assert_eq!((line.cmd, line.bang, line.args), (Cmd::Delete, false, "x"));
    assert_eq!(
        line.addresses,
        [
            address(Base::Number(2), 0, false),
            address(Base::Last, -1, false)
        ]
    );

    let line = parse("%s/a/b/g").unwrap();
    assert_eq!((line.cmd, line.args), (Cmd::Substitute, "/a/b/g"));
    assert_eq!(line.addresses.len(), 2);

    let line = parse("'a;+2 y").unwrap();
    assert_eq!(
        line.addresses,
        [
            address(Base::Mark('a'), 0, true),
            address(Base::Current, 2, false)
        ]
    );

    let line = parse("/one/,?two\\?x?+ j!").unwrap();
    assert_eq!((line.cmd, line.bang), (Cmd::Join, true));
    assert_eq!(
        line.addresses,
        [
            address(Base::Forward("one".into()), 0, false),
            address(Base::Backward("two?x".into()), 1, false)
        ]
    );

    // 分隔符一侧省略的地址是当前行。
    let line = parse(",3>").unwrap();
    assert_eq!(line.cmd, Cmd::ShiftRight(1));
    assert_eq!(
        line.addresses,
        [
            address(Base::Current, 0, false),
            address(Base::Number(3), 0, false)
        ]
    );
    assert_eq!(parse("'").unwrap_err(), Message::BadRange);
}

#[test]
fn names_are_recognised_by_their_shortest_abbreviation() {
    let cmd = |text: &str| parse(text).map(|line| line.cmd);
    for (text, expected) in [
        ("s", Cmd::Substitute),
        ("se", Cmd::Set),
        ("sor", Cmd::Sort),
        ("d", Cmd::Delete),
        ("di", Cmd::Registers),
        ("m0", Cmd::Move),
        ("ma a", Cmd::Mark),
        ("marks", Cmd::Marks),
        ("ka", Cmd::Mark),
        ("t.", Cmd::Copy),
        ("co 3", Cmd::Copy),
        ("pu", Cmd::Put),
        ("p", Cmd::Print),
        ("norm dd", Cmd::Normal),
        ("noh", Cmd::NoHighlight),
        ("g/x/d", Cmd::Global),
        ("v/x/d", Cmd::GlobalInverse),
        ("w", Cmd::Write),
        ("wa", Cmd::WriteAll),
        ("wq", Cmd::WriteQuit),
        ("x", Cmd::WriteQuit),
        ("q!", Cmd::Quit),
        ("qa", Cmd::QuitAll),
        ("e!", Cmd::Edit),
        ("bn", Cmd::BufferNext),
        ("bN", Cmd::BufferPrevious),
        ("b 2", Cmd::Buffer),
        ("ls", Cmd::Buffers),
        ("tabn", Cmd::TabNext),
        ("tabnew", Cmd::TabNew),
        ("tabo", Cmd::TabOnly),
        (">>", Cmd::ShiftRight(2)),
        ("<", Cmd::ShiftLeft(1)),
        ("&&", Cmd::RepeatSubstitute),
        ("5", Cmd::Goto),
        ("", Cmd::Goto),
    ] {
        assert_eq!(cmd(text), Ok(expected), "{text}");
    }
    for text in ["so", "n", "r", "f", "zzz"] {
        assert_eq!(cmd(text), Err(Message::BadCommand), "{text}");
    }
}

#[test]
fn bars_separate_commands_unless_escaped() {
    assert_eq!(split_bar("a | d"), ("a ".to_owned(), Some(" d")));
    assert_eq!(split_bar(r"a\|b"), ("a|b".to_owned(), None));
    assert!(Cmd::Global.takes_bar() && !Cmd::Delete.takes_bar());
}

#[test]
fn completion_covers_names_options_and_paths() {
    let names = complete("2,3tab").unwrap();
    assert_eq!((names.kind, names.start), (Completing::Command, 3));
    assert!(names.candidates.contains(&"tabnew".to_owned()));
    assert!(names.candidates.windows(2).all(|pair| pair[0] < pair[1]));

    let options = complete("set nosm").unwrap();
    assert_eq!((options.kind, options.start), (Completing::Option, 6));
    assert_eq!(options.candidates, ["smartcase"]);

    let path = complete("e src/ma").unwrap();
    assert_eq!((path.kind, path.start), (Completing::Path, 2));
    assert_eq!(complete("d x"), None);
}
