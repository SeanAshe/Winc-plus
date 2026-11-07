//! 辅助进程的单元测试：覆盖历史回答分条、粘贴的收发判断、行的编码长度和待写入的字节。

use super::*;
use winc_pty::protocol::{Color, Run, Style};

fn line(text: &str) -> Line {
    Line {
        text: text.into(),
        ..Line::default()
    }
}

/// 分条按行数累加字节数，行号跟着走，`more` 只在最后一段为 `false`。
#[test]
fn a_long_history_reply_is_split_without_losing_lines() {
    let lines: Vec<Line> = (0..10).map(|i| line(&format!("line{i}"))).collect();
    let limit = lines[0].encoded_len() * 3;
    let chunks = split_lines(100, lines, limit);
    let starts: Vec<u64> = chunks.iter().map(|c| c.0).collect();
    assert_eq!(starts, [100, 103, 106, 109]);
    let more: Vec<bool> = chunks.iter().map(|c| c.2).collect();
    assert_eq!(more, [true, true, true, false]);
    let text: Vec<&str> = chunks
        .iter()
        .flat_map(|c| &c.1)
        .map(Line::trimmed)
        .collect();
    assert_eq!(text.len(), 10);
    assert_eq!(text[9], "line9");
}

#[test]
fn an_empty_history_reply_still_answers_once() {
    let chunks = split_lines(7, Vec::new(), 1);
    assert_eq!(chunks, [(7, Vec::new(), false)]);
}

/// `encoded_len` 必须与 `FromHelper::History` 编码后增加的字节数相同。
#[test]
fn encoded_len_matches_the_wire() {
    let mut l = line("héllo");
    l.runs.push(Run {
        start: 0,
        style: Style {
            fg: Color::Rgb(1, 2, 3),
            bg: Color::Indexed(4),
            ..Style::default()
        },
    });
    let message = |lines: Vec<Line>| {
        let mut out = Vec::new();
        FromHelper::History {
            tab: 0,
            id: 0,
            start: 0,
            lines,
            more: false,
        }
        .encode(&mut out);
        out.len()
    };
    assert_eq!(
        message(vec![l.clone()]) - message(Vec::new()),
        l.encoded_len()
    );
}

#[test]
fn colorfgbg_follows_the_terminal_background() {
    let mut env = vec![(b"COLORFGBG".to_vec(), b"15;0".to_vec())];
    set_colorfgbg(&mut env, [0xfb, 0xfb, 0xfc]);
    assert_eq!(lookup(&env, b"COLORFGBG"), Some(&b"0;15"[..]));
    set_colorfgbg(&mut env, [0x1b, 0x1e, 0x24]);
    assert_eq!(lookup(&env, b"COLORFGBG"), Some(&b"15;0"[..]));
    assert_eq!(env.iter().filter(|(key, _)| key == b"COLORFGBG").count(), 1);
}

#[test]
fn a_paste_is_kept_or_dropped_whole() {
    const MB: usize = 1 << 20;
    // 前面已积压 15 MiB：3 MiB 的粘贴整次丢弃，后面两段也不收，下一次单独按键仍然收下。
    let mut pasting = None;
    let pending = 15 * MB;
    assert!(!admit(&mut pasting, pending, MB, 2 * MB as u64));
    assert!(!admit(&mut pasting, pending - MB, MB, MB as u64));
    assert!(!admit(&mut pasting, 0, MB, 0));
    assert!(admit(&mut pasting, 0, 1, 0));
    // 已收下的粘贴：后面几段仍然收下，即使这期间又积压了其他输入。
    assert!(admit(&mut pasting, 0, MB, 2 * MB as u64));
    assert!(admit(&mut pasting, 15 * MB, MB, MB as u64));
    assert!(admit(&mut pasting, 16 * MB, MB, 0));
    assert_eq!(pasting, None);
}

/// 回归测试：不带 close-on-exec 的描述符由 `close_inherited_on_exec` 补上标志。
///
/// 触发条件：主程序把不带该标志的描述符（例如系统库打开的）原样传给辅助进程，这些描述符会
/// 一路传进每个终端里的 shell。
#[cfg(unix)]
#[test]
#[allow(unsafe_code)] // 测试要自己 `dup` 一个不带 close-on-exec 的描述符
fn inherited_descriptors_are_closed_on_exec() {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open("/dev/null").unwrap();
    // SAFETY: `dup` 复制一个有效的描述符，新描述符不带 close-on-exec；用完下面关掉。
    let fd = unsafe { libc::dup(file.as_raw_fd()) };
    assert!(fd > 2);
    // SAFETY: 只读描述符的标志位。
    let flags = |fd| unsafe { libc::fcntl(fd, libc::F_GETFD) };
    assert_eq!(flags(fd) & libc::FD_CLOEXEC, 0);
    assert!(pty::close_inherited_on_exec() > 0);
    assert_ne!(flags(fd) & libc::FD_CLOEXEC, 0);
    // SAFETY: `fd` 是上面 `dup` 出来的，只关这一次。
    unsafe { libc::close(fd) };
}

#[test]
fn pending_input_keeps_the_unwritten_bytes_in_order() {
    let mut input = PendingInput::default();
    input.extend(b"abcdef");
    input.advance(2);
    assert_eq!(input.bytes(), b"cdef");
    input.extend(b"gh");
    assert_eq!(input.bytes(), b"cdefgh");
    // 已写入的部分超过一半：移动剩余的字节，内容不变。
    input.advance(3);
    assert_eq!(input.start, 0);
    assert_eq!(input.bytes(), b"fgh");
    assert_eq!(input.len(), 3);
    input.advance(3);
    assert!(input.is_empty());
}

#[test]
fn pending_input_moves_little_while_writing_in_small_pieces() {
    let mut input = PendingInput::default();
    input.extend(&vec![b'x'; 1 << 16]);
    // 前半段写入期间不移动。
    for _ in 0..(1 << 15) - 1 {
        input.advance(1);
    }
    assert_eq!(input.start, (1 << 15) - 1);
    input.advance(1);
    assert_eq!(input.start, 0);
}

#[test]
fn pending_input_releases_the_memory_of_a_large_paste() {
    let mut input = PendingInput::default();
    input.extend(&vec![b'x'; 1 << 20]);
    input.advance(1 << 20);
    assert!(input.buf.capacity() <= winc_pty::protocol::IDLE_BUFFER);
}
