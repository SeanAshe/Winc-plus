//! 协议编解码的往返测试：每种消息编码后解码应得到相同的值，坏数据必须报错。

use super::codec::kind;
use super::*;

fn sample_line() -> Line {
    Line {
        text: "a中b\u{301} ".into(),
        runs: vec![
            Run {
                start: 0,
                style: Style::default(),
            },
            Run {
                start: 1,
                style: Style {
                    fg: Color::Indexed(9),
                    bg: Color::Rgb(1, 2, 3),
                    underline_color: Color::Rgb(250, 0, 7),
                    attrs: Attrs(Attrs::BOLD | Attrs::ITALIC),
                    link: 3,
                },
            },
        ],
        wrapped: true,
        marks: Marks {
            flags: Marks::DONE | Marks::PROMPT,
            exit: -2,
            block: 70_000,
        },
    }
}

fn to_helper() -> Vec<ToHelper> {
    let size = Size {
        cols: 80,
        rows: 24,
        width: 1280,
        height: 768,
    };
    vec![
        ToHelper::Spawn {
            tab: 7,
            program: Some(b"/bin/zsh".to_vec()),
            args: vec![b"-c".to_vec(), vec![0xff, 0]],
            login: true,
            cwd: "/tmp/目录".into(),
            env: vec![(b"PATH".to_vec(), b"/usr/bin".to_vec())],
            scrollback: 10_000,
            size,
            integration: Some(b"/Users/a/Library/Application Support/winc/shell".to_vec()),
        },
        ToHelper::Spawn {
            tab: 8,
            program: None,
            args: vec![],
            login: false,
            cwd: vec![],
            env: vec![],
            scrollback: 0,
            size,
            integration: None,
        },
        ToHelper::Input {
            tab: 1,
            bytes: b"ls\r\x1b[A".to_vec(),
            rest: 7,
        },
        ToHelper::Resize { tab: 1, size },
        ToHelper::Credit { tab: 65535 },
        ToHelper::History {
            tab: 2,
            id: 9,
            start: 1 << 40,
            end: (1 << 40) + 50,
        },
        ToHelper::Clear { tab: 3 },
        ToHelper::Close { tab: 4 },
        ToHelper::Theme(Theme {
            fg: [1, 2, 3],
            bg: [4, 5, 6],
            cursor: [7, 8, 9],
            palette: [[10, 11, 12]; 16],
        }),
    ]
}

fn from_helper() -> Vec<FromHelper> {
    vec![
        FromHelper::Hello {
            magic: MAGIC,
            version: VERSION,
            build: "0.1.81".into(),
        },
        FromHelper::Frame {
            tab: 1,
            frame: Frame {
                cols: 80,
                rows: 24,
                full: true,
                scroll: 2,
                lines: vec![(0, sample_line()), (23, Line::default())],
                cursor: Cursor {
                    row: 3,
                    col: 4,
                    shape: CursorShape::Bar,
                    blink: true,
                    visible: false,
                },
                modes: Modes(Modes::APP_CURSOR | Modes::MOUSE_SGR),
                keyboard: 1,
                history: (5, 99),
            },
        },
        FromHelper::History {
            tab: 1,
            id: 3,
            start: 12,
            lines: vec![sample_line(), Line::default()],
            more: true,
        },
        FromHelper::Blocks {
            tab: 2,
            blocks: vec![
                Block {
                    id: 1,
                    prompt: 3,
                    command: 4,
                    output: Some(5),
                    end: Some(1 << 33),
                    exit: Some(-1),
                    duration: Some(12_400),
                    text: "cargo xtask test".into(),
                    cwd: b"/Users/a/winc".to_vec(),
                },
                Block {
                    id: 2,
                    prompt: 9,
                    command: 9,
                    ..Block::default()
                },
            ],
            early: true,
        },
        FromHelper::Blocks {
            tab: 3,
            blocks: vec![],
            early: false,
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Title("vim — 标题".into()),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Cwd(b"/Users/a b".to_vec()),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Bell,
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Notify {
                title: "构建".into(),
                body: "完成".into(),
            },
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Clipboard("复制".into()),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Link(2, "https://example.com".into()),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Command(CommandMark::Done(Some(101))),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Command(CommandMark::Done(None)),
        },
        FromHelper::Event {
            tab: 1,
            event: Event::Command(CommandMark::Output),
        },
        FromHelper::Process {
            tab: 1,
            name: "cargo".into(),
        },
        FromHelper::Exited { tab: 1, code: 130 },
        FromHelper::Disconnected { tab: 1 },
        FromHelper::Failed {
            tab: 1,
            message: "未找到程序".into(),
        },
    ]
}

/// 把所有消息编进一个字节流，再逐字节喂给解码器，解码结果必须与原消息相同。
fn round_trip<M: Message + PartialEq + std::fmt::Debug>(messages: &[M]) {
    let mut bytes = Vec::new();
    for message in messages {
        message.encode(&mut bytes);
    }

    let mut decoder = Decoder::default();
    let mut decoded = Vec::new();
    for byte in &bytes {
        decoder.push(std::slice::from_ref(byte));
        while let Some(message) = decoder.next_message::<M>().unwrap() {
            decoded.push(message);
        }
    }
    assert_eq!(&decoded, messages);

    // 一次性全部喂入时结果相同。
    let mut decoder = Decoder::default();
    decoder.push(&bytes);
    let mut decoded = Vec::new();
    while let Some(message) = decoder.next_message::<M>().unwrap() {
        decoded.push(message);
    }
    assert_eq!(&decoded, messages);
}

#[test]
fn messages_to_the_helper_round_trip() {
    round_trip(&to_helper());
}

#[test]
fn messages_from_the_helper_round_trip() {
    round_trip(&from_helper());
}

#[test]
fn bad_data_is_an_error_not_a_panic() {
    // 长度为 0 和超过上限的长度都必须报错。
    for head in [0u32, u32::MAX] {
        let mut decoder = Decoder::default();
        decoder.push(&head.to_le_bytes());
        assert!(decoder.next_message::<ToHelper>().is_err());
    }

    // 每条消息截掉末尾一个字节、多出一个字节或类型改错时都必须报错。
    for message in from_helper() {
        let mut bytes = Vec::new();
        message.encode(&mut bytes);
        let body = &bytes[4..];
        assert!(FromHelper::decode(&body[..body.len() - 1]).is_err());
        let mut longer = body.to_vec();
        longer.push(0);
        assert!(FromHelper::decode(&longer).is_err());
        let mut wrong = body.to_vec();
        wrong[0] = 200;
        assert!(FromHelper::decode(&wrong).is_err());
    }

    // 个数字段很大但后面没有数据时，不按个数分配内存。
    let mut body = vec![kind::LINES];
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&0u64.to_le_bytes());
    body.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(FromHelper::decode(&body).is_err());
}

#[test]
fn underline_style_lives_beside_the_other_bits() {
    let mut attrs = Attrs(Attrs::BOLD);
    attrs.set_underline(Underline::Curly);
    assert!(attrs.has(Attrs::BOLD));
    assert_eq!(attrs.underline(), Underline::Curly);
    attrs.set_underline(Underline::None);
    assert_eq!(attrs, Attrs(Attrs::BOLD));
}

#[test]
fn block_encoded_len_matches_the_wire() {
    let message = |blocks: Vec<Block>| {
        let mut out = Vec::new();
        FromHelper::Blocks {
            tab: 0,
            blocks,
            early: false,
        }
        .encode(&mut out);
        out.len()
    };
    let block = Block {
        output: Some(5),
        exit: Some(1),
        duration: Some(3),
        text: "ls -a".into(),
        cwd: b"/tmp".to_vec(),
        ..Block::default()
    };
    assert_eq!(
        message(vec![block.clone()]) - message(Vec::new()),
        block.encoded_len()
    );
}
