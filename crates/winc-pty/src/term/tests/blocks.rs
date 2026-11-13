//! 命令块的测试：提示符、命令、输出与结束的标记，以及清屏和重排之后的块。

use super::*;

#[test]
fn combining_marks_join_the_previous_character() {
    let mut t = term(10, 2);
    feed(&mut t, "e\u{301}x");
    assert_eq!(t.line(0).text, "e\u{301}x");
    assert_eq!(t.cursor(), (0, 2));
}

#[test]
fn the_alternate_screen_keeps_the_shell_intact() {
    let mut t = term(10, 3);
    feed(&mut t, "$ vim");
    feed(&mut t, "\x1b[?1049h\x1b[Hfile");
    assert_eq!(t.screen_text(), "file");
    assert!(t.modes().has(Modes::ALT_SCREEN));
    feed(&mut t, "\x1b[?1049l");
    assert_eq!(t.screen_text(), "$ vim");
    assert_eq!(t.cursor(), (0, 5));
    assert!(!t.modes().has(Modes::ALT_SCREEN));
}

#[test]
fn osc_sequences_become_events() {
    let mut t = term(20, 2);
    feed(
        &mut t,
        concat!(
            "\x1b]2;vim; main.rs\x07",
            "\x1b]7;file://mac/Users/a%20b\x1b\\",
            "\x1b]52;c;aGk=\x07",
            "\x1b]52;c;?\x07",
            "\x1b]9;done\x07",
            "\x1b]9;4;1;50\x07",
            "\x1b]777;notify;Build;ok\x07",
            "\x07",
        ),
    );
    assert_eq!(
        t.take_events(),
        [
            Event::Title("vim; main.rs".into()),
            Event::Cwd(b"/Users/a b".to_vec()),
            Event::Clipboard("hi".into()),
            Event::Notify {
                title: String::new(),
                body: "done".into()
            },
            Event::Notify {
                title: "Build".into(),
                body: "ok".into()
            },
            Event::Bell,
        ]
    );
}

#[test]
fn shell_integration_marks_rows_and_reports_commands() {
    let mut t = term(20, 4);
    feed(
        &mut t,
        "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07a b\r\n\x1b]133;D;1\x07\x1b]133;A\x07$ ",
    );
    assert_eq!(t.line(0).marks.flags, Marks::PROMPT | Marks::COMMAND);
    assert_eq!(t.line(1).marks.flags, Marks::OUTPUT);
    let done = t.line(2).marks;
    assert_eq!(done.flags, Marks::DONE | Marks::PROMPT);
    assert_eq!(done.exit, 1);
    assert_eq!(
        t.take_events(),
        [
            Event::Command(CommandMark::Prompt),
            Event::Command(CommandMark::Command),
            Event::Command(CommandMark::Output),
            Event::Command(CommandMark::Done(Some(1))),
            Event::Command(CommandMark::Prompt),
        ]
    );
}

#[test]
fn blocks_are_cut_from_the_marks() {
    let mut t = term(30, 6);
    feed(&mut t, "\x1b]7;file://host/Users/a%20b\x07");
    command(&mut t, "ls -a", "x\r\ny\r\n", 0);
    command(&mut t, "false", "", 1);
    feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07");
    let blocks = t.take_blocks().expect("有块");
    assert_eq!(blocks.len(), 3);
    let ls = &blocks[0];
    assert_eq!(
        (ls.prompt, ls.command, ls.output, ls.end),
        (0, 0, Some(1), Some(3))
    );
    assert_eq!((ls.exit, ls.text.as_str()), (Some(0), "ls -a"));
    assert_eq!(ls.cwd, b"/Users/a b");
    assert!(ls.duration.is_some());
    // 没有输出：开始运行、结束和下一个提示符在同一行。
    let f = &blocks[1];
    assert_eq!(
        (f.prompt, f.output, f.end, f.exit),
        (3, Some(4), Some(4), Some(1))
    );
    let typing = &blocks[2];
    assert_eq!((typing.prompt, typing.output, typing.end), (4, None, None));
    assert!(blocks[0].id != blocks[1].id && blocks[1].id != blocks[2].id);
    // 块没有变化时不再上报。
    assert_eq!(t.take_blocks(), None);

    // 行滚进回滚缓冲后行号不变，块的行号也不用改。
    feed(&mut t, "\r\n\x1b]133;C\x07");
    for i in 0..10 {
        feed(&mut t, &format!("{i}\r\n"));
    }
    let blocks = t.take_blocks().expect("开始运行了");
    assert!(blocks[2].running());
    assert_eq!(blocks[0].output, Some(1));
    assert_eq!(t.take_blocks(), None);
}

#[test]
fn a_huge_command_line_is_cut_short() {
    // 粘贴进来的超长命令行按上限截短，截断不得切开一个 UTF-8 字符。
    let mut t = term(30, 6);
    // 命令行按 URL 编码报上来：「中」是 %E4%B8%AD。
    let url = format!("a{}", "%E4%B8%AD".repeat(40_000));
    feed(
        &mut t,
        &format!("\x1b]133;A\x07$ \x1b]133;C;nonce={NONCE};cmdline_url={url}\x07"),
    );
    let text = &t.take_blocks().unwrap()[0].text;
    assert!(text.len() <= 64 * 1024 && text.len() > 60 * 1024);
    assert!(text.starts_with("a中中"));
}

#[test]
fn blocks_scrolled_out_of_the_history_go_away() {
    // 回滚缓冲只留 10 行：此后打出 40 行，两个块的提示符都滚出了回滚缓冲。
    let mut t = Terminal::new(30, 4, 10);
    command(&mut t, "ls", "x\r\n", 0);
    feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07\r\n");
    assert_eq!(t.take_blocks().expect("有块").len(), 2);
    // 行先滚进回滚缓冲：屏幕上已经没有带标记的行，块仍然存在且没有变化。
    for i in 0..4 {
        feed(&mut t, &format!("{i}\r\n"));
    }
    assert_eq!(t.take_blocks(), None);
    for i in 0..40 {
        feed(&mut t, &format!("{i}\r\n"));
    }
    assert_eq!(t.take_blocks(), Some(Vec::new()));
    assert_eq!(t.take_blocks(), None);
}

#[test]
fn blocks_survive_a_reflow_and_go_away_with_a_clear() {
    let mut t = term(10, 4);
    command(&mut t, "echo", "0123456789abc\r\n", 0);
    feed(&mut t, "\x1b]133;A\x07$ ");
    let before = t.take_blocks().unwrap();
    assert_eq!(before[0].end, Some(3));
    // 变宽以后折的那一行连起来，下一个提示符往上挪了一行。
    t.resize(size(20, 4));
    let after = t.take_blocks().expect("重排后重数");
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].text, "echo");
    assert_eq!(after[0].end, Some(2));
    // 一行写满折到下一行时，标记记在折行后逻辑行的第一行上。
    t.resize(size(4, 4));
    let narrow = t.take_blocks().unwrap();
    assert_eq!(narrow.len(), 2);
    assert_eq!(narrow[0].id, before[0].id);

    // 清屏连标记一起擦掉。
    feed(&mut t, "\x1b[3J\x1b[H\x1b[2J");
    assert_eq!(t.take_blocks(), Some(vec![]));
}

#[test]
fn a_prompt_without_a_command_does_not_end_up_running() {
    let mut t = term(20, 4);
    // 在提示符上直接回车：没有 `C`，由下一个提示符结束这一块。
    feed(&mut t, "\x1b]133;A\x07$ \r\n\x1b]133;A\x07$ ");
    let blocks = t.take_blocks().unwrap();
    assert_eq!(blocks[0].end, Some(1));
    assert!(!blocks[0].running() && blocks[0].exit.is_none());
    // `cmdline=` 是原样的命令行，含分号时也整条保留。
    feed(
        &mut t,
        &format!("ls\r\n\x1b]133;C;nonce={NONCE};cmdline=echo a; echo b\x07"),
    );
    assert_eq!(t.take_blocks().unwrap()[1].text, "echo a; echo b");
}

#[test]
fn a_command_line_without_the_nonce_is_not_kept() {
    // 程序输出中的命令行（`cat` 的文件）：「重新运行」会把它写入 shell 的输入，不能记录。
    let forged = [
        "\x1b]133;C;cmdline_url=rm%20-rf%20~\x07".to_owned(),
        "\x1b]133;C;nonce=fedcba9876543210;cmdline=rm -rf ~\x07".to_owned(),
        format!("\x1b]133;C;cmdline=rm -rf ~;nonce={NONCE}\x07"),
        format!("\x1b]133;C;nonce={NONCE}0;cmdline=rm -rf ~\x07"),
    ];
    for marker in &forged {
        let mut t = term(30, 4);
        feed(&mut t, &format!("\x1b]133;A\x07$ {marker}"));
        assert_eq!(t.take_blocks().unwrap()[0].text, "", "{marker:?}");
    }
    // 没有开启 shell 集成（没有随机数）时，所有命令行都不可信。
    let mut t = Terminal::new(30, 4, 100);
    feed(&mut t, "\x1b]133;A\x07$ \x1b]133;C;nonce=;cmdline=ls\x07");
    assert_eq!(t.take_blocks().unwrap()[0].text, "");

    // 命令运行期间输出的伪造命令行不清除 shell 报告的命令行。
    let mut t = term(30, 4);
    command(&mut t, "cat notes", "", 0);
    feed(&mut t, "\x1b]133;C;cmdline=rm -rf ~\x07");
    assert_eq!(t.take_blocks().unwrap()[0].text, "cat notes");
}

#[test]
fn blocks_fit_in_one_message() {
    let total = |blocks: &[Block]| blocks.iter().map(Block::encoded_len).sum::<usize>();
    // 每个块都报告一条 60 KiB 的命令行，合计超过上限：旧块先丢弃命令行，块本身都保留。
    let mut t = Terminal::new(30, 4, 1000);
    t.set_nonce(NONCE.into());
    let long = "x".repeat(60 * 1024);
    for _ in 0..160 {
        feed(
            &mut t,
            &format!("\x1b]133;A\x07$ \x1b]133;C;nonce={NONCE};cmdline={long}\x07\r\n"),
        );
    }
    let blocks = t.take_blocks().unwrap();
    assert_eq!(blocks.len(), 160);
    assert!(total(&blocks) <= 8 << 20);
    assert_eq!(blocks[0].text, "");
    assert_eq!(blocks[159].text, long);

    // 每一行都是提示符：块本身也容纳不下时，保留最新的块。
    let mut t = Terminal::new(30, 4, 200_000);
    feed(&mut t, &"\x1b]133;A\x07\r\n".repeat(160_000));
    let blocks = t.take_blocks().unwrap();
    assert!(blocks.len() > 100_000 && blocks.len() < 160_000);
    assert!(total(&blocks) <= 8 << 20);
    assert_eq!(blocks.last().unwrap().prompt, 159_999);
}

#[test]
fn a_prompt_the_shell_redraws_is_cleared_not_reflowed() {
    // 两行的提示符，第一行用点填满整行宽。
    let prompt = "out\r\n\x1b]133;A;redraw=1\x07TOP-----END\r\n> ";
    let mut t = term(11, 4);
    feed(&mut t, prompt);
    t.resize(size(8, 4));
    // 提示符擦成空行、不折行，光标还在第二行：shell 往上退一行正好到第一行重画。
    assert_eq!(t.screen_text(), "out");
    assert_eq!(t.cursor(), (2, 2));
    feed(&mut t, "\x1b[A\r\x1b[JTOP--END\r\n> ");
    assert_eq!(t.screen_text(), "out\nTOP--END\n>");
    // 标记留在提示符那一行，命令块仍然有效。
    assert_eq!(t.take_blocks().unwrap()[0].prompt, 1);

    // 命令开始运行以后（收到 `C`）不再算停在提示符上，按普通内容重排。
    feed(&mut t, "\x1b]133;C\x07");
    t.resize(size(6, 4));
    assert_eq!(t.screen_text(), "out\nTOP--E\nND\n>");

    // 输入一长串命令后光标超出新宽度：光标仍留在这一行，不随空格折到下一行。
    let mut t = term(40, 4);
    feed(
        &mut t,
        "out\r\n\x1b]133;A;redraw=1\x07> echo hello world 1234",
    );
    t.resize(size(10, 4));
    assert_eq!(t.cursor().0, 1);
    feed(&mut t, "\r\x1b[J> echo hel\r\nlo world 1\r\n234");
    assert_eq!(t.screen_text(), "out\n> echo hel\nlo world 1\n234");

    // shell 不重画提示符（没带 `redraw=1`）时按普通内容重排。
    let mut t = term(11, 4);
    feed(&mut t, &prompt.replace(";redraw=1", ""));
    t.resize(size(8, 4));
    assert_eq!(t.screen_text(), "out\nTOP-----\nEND\n>");
}

#[test]
fn a_combining_mark_at_the_margin_without_autowrap_joins_the_last_character() {
    let mut t = term(5, 2);
    feed(&mut t, "\x1b[?7labcde\u{301}");
    assert_eq!(t.screen_text(), "abcde\u{301}");
    // 自动换行关闭时后续的字覆盖最后一格；重新打开自动换行也不补折。
    feed(&mut t, "f\x1b[?7hg");
    assert_eq!(t.screen_text(), "abcdg");
}

#[test]
fn clearing_keeps_the_prompt_row() {
    let mut t = term(10, 3);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n$ ls");
    t.clear();
    assert_eq!(t.screen_text(), "$ ls");
    assert_eq!(t.cursor(), (0, 4));
    assert!(history_text(&t).is_empty());
    assert!(t.take_frame().unwrap().full);
}

#[test]
fn an_endless_osc_is_cut_off_instead_of_buffered() {
    // 拆在两段之间的 OSC 照常认。
    let mut t = term(20, 3);
    feed(&mut t, "\x1b");
    feed(&mut t, "]0;hi\x07");
    assert_eq!(t.take_events(), [Event::Title("hi".into())]);

    // 超长且迟迟不结束：超出上限的部分不交给 `vte`，结束之后的字照常显示，OSC 内容不外泄。
    let chunk = "a".repeat(64 << 10);
    for end in ["\x07", "\x1b\\", "\x1b[1m"] {
        let mut t = term(20, 3);
        feed(&mut t, "\x1b]0;");
        for _ in 0..(3 * MAX_OSC / chunk.len()) {
            feed(&mut t, &chunk);
        }
        feed(&mut t, end);
        feed(&mut t, "ok");
        assert_eq!(t.screen_text(), "ok", "以 {end:?} 结束");
        assert!(t.osc == Osc::Outside);
        // 截下来的那部分仍然处理：标题按标题自身的上限截短。
        let title = t.take_events().into_iter().find_map(|e| match e {
            Event::Title(title) => Some(title),
            _ => None,
        });
        assert!(title.is_some_and(|t| t.len() <= 1024));
    }

    // 超长的 OSC 52 不写剪贴板，截短的内容不完整。
    let mut t = term(20, 3);
    feed(&mut t, "\x1b]52;c;");
    for _ in 0..(3 * MAX_OSC / chunk.len()) {
        feed(&mut t, &chunk);
    }
    feed(&mut t, "\x07");
    assert!(
        t.take_events()
            .iter()
            .all(|e| !matches!(e, Event::Clipboard(_)))
    );
}

/// 回归：`vte` 收集 OSC 的缓冲只增不减，遇到过长的 OSC 52 后一直占用内存。
///
/// 过长的 OSC 结束后替换解析器以释放缓冲；替换前后都要正确解析，包括长 OSC 本身、紧跟的
/// 文字和下一个序列。
#[test]
fn long_oscs_are_parsed_whole_and_then_released() {
    let text = "x".repeat(100 << 10);
    let encoded = base64_encode(text.as_bytes());
    for end in ["\x07", "\x1b\\"] {
        let mut t = term(20, 5);
        feed(
            &mut t,
            &format!("\x1b]52;c;{encoded}{end}ok\x1b]0;title\x07"),
        );
        assert_eq!(
            t.take_events(),
            [Event::Clipboard(text.clone()), Event::Title("title".into())]
        );
        assert_eq!(t.line(0).trimmed(), "ok");
    }
}

#[test]
fn the_clear_command_leaves_nothing_behind_in_either_order() {
    for sequence in ["\x1b[H\x1b[2J\x1b[3J", "\x1b[3J\x1b[H\x1b[2J"] {
        let mut t = term(20, 4);
        feed(&mut t, "one\r\ntwo\r\n");
        feed(&mut t, sequence);
        assert!(history_text(&t).is_empty(), "{sequence:?}");
        assert!(t.screen_text().trim().is_empty());
    }
}

#[test]
fn repainting_the_prompt_keeps_nothing() {
    // zsh 收到 SIGWINCH 或 `zle reset-prompt` 时的重画序列。
    let mut t = at_prompt_on_first_row();
    feed(&mut t, "\r\r\x1b[0m\x1b[27m\x1b[24m\x1b[JP> ");
    feed(&mut t, "\r\r\x1b[0m\x1b[27m\x1b[24m\x1b[JQ> ");
    assert!(history_text(&t).is_empty());
    assert!(t.screen_text().contains("Q>"));
}

#[test]
fn clearing_at_the_prompt_keeps_earlier_output_but_not_the_prompt() {
    // `⌃L`：之前的输出进回滚缓冲，提示符由 shell 重画。
    let mut t = term(40, 6);
    feed(&mut t, "output\r\n\x1b]133;A;redraw=1\x07P> \x1b]133;B\x07");
    feed(&mut t, "\x1b[H\x1b[2JP> ");
    assert_eq!(history_text(&t), ["output"]);
    assert!(t.screen_text().starts_with("P>"));
}

#[test]
fn the_prompt_mark_survives_clearing_the_screen() {
    let mut t = at_prompt_on_first_row();
    let before = t.take_blocks().unwrap();
    feed(&mut t, "\x1b[H\x1b[2JP> ");
    let after = t.take_blocks().unwrap_or_else(|| before.clone());
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, before[0].id);
    // 标记仍在屏幕上：改宽度时仍然先擦掉提示符。
    t.resize(size(20, 6));
    assert!(history_text(&t).is_empty());
}

#[test]
fn a_running_command_clearing_the_screen_is_still_kept() {
    let mut t = term(40, 6);
    feed(&mut t, "\x1b]133;A;redraw=1\x07P> \x1b]133;B\x07");
    feed(&mut t, "\x1b]133;C\x07\r\nlog line\x1b[2J\x1b[H");
    assert_eq!(history_text(&t), ["P>", "log line"]);
}

#[test]
fn clearing_keeps_every_line_of_the_prompt() {
    let mut t = term(40, 6);
    feed(
        &mut t,
        "output\r\n\x1b]133;A\x07line one\r\nP> \x1b]133;B\x07",
    );
    t.clear();
    assert!(history_text(&t).is_empty());
    let screen = t.screen_text();
    assert!(!screen.contains("output"), "{screen}");
    assert!(screen.starts_with("line one"), "{screen}");
    assert!(screen.contains("P>"), "{screen}");
    assert_eq!(t.cursor().0, 1);
    // 标记留在提示符第一行。
    let blocks = t.take_blocks().unwrap();
    assert_eq!(blocks.len(), 1);
}

#[test]
fn clearing_without_a_prompt_keeps_the_cursor_row() {
    let mut t = term(40, 6);
    feed(&mut t, "one\r\ntwo");
    t.clear();
    assert_eq!(t.screen_text().trim(), "two");
    assert_eq!(t.cursor(), (0, 3));
}

#[test]
fn a_prompt_that_repaints_stops_counting_once_a_command_runs() {
    // 本地 zsh 的提示符会自行重画；`ssh` 运行期间远程 shell 的 `A` 不会。
    let mut t = term(20, 4);
    feed(
        &mut t,
        "\x1b]133;A;redraw=1\x07$ \x1b]133;B\x07\x1b]133;C\x07\r\n",
    );
    feed(&mut t, "\x1b]133;A\x07remote$ ");
    t.resize(size(10, 4));
    assert!(t.screen_text().contains("remote$"), "{}", t.screen_text());
}

#[test]
fn output_cannot_claim_that_a_prompt_repaints() {
    // 命令输出里的 `redraw=1`（`cat` 录下的日志）不生效，改宽度时不擦除屏幕。
    let mut t = term(20, 4);
    feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07\x1b]133;C\x07\r\n");
    feed(&mut t, "\x1b]133;A;redraw=1\x07log line");
    t.resize(size(10, 4));
    assert!(t.screen_text().contains("log line"), "{}", t.screen_text());
}

#[test]
fn the_local_prompt_repaints_again_after_a_command_finishes() {
    let mut t = term(20, 4);
    feed(
        &mut t,
        "\x1b]133;A;redraw=1\x07$ \x1b]133;B\x07\x1b]133;C\x07\r\n",
    );
    feed(&mut t, "\x1b]133;D;0\x07\x1b]133;A;redraw=1\x07P> ");
    t.resize(size(10, 4));
    // 提示符被擦除，等待 shell 收到 SIGWINCH 后重画。
    assert!(!t.screen_text().contains("P>"), "{}", t.screen_text());
}

#[test]
fn output_without_a_final_newline_stays_separate_from_the_next_prompt() {
    // zsh 对 `printf foo` 的真实输出：`PROMPT_SP` 的填充折行后回到行首，再发 `D` 和 `A`。
    let mut t = term(20, 10);
    feed(&mut t, "\x1b]133;A;redraw=1\x07P> \x1b]133;B\x07");
    feed(&mut t, "\r\n\x1b]133;C\x07foo%");
    feed(&mut t, &" ".repeat(19));
    feed(
        &mut t,
        "\r \r\x1b]133;D;0\x07\x1b]133;A;redraw=1\x07\r\x1b[JP> cmd\x1b]133;B\x07",
    );
    // 又运行了一条命令：`foo` 下面的提示符已不是当前提示符，改宽度时不会被擦除。
    feed(
        &mut t,
        "\r\n\x1b]133;C\x07bar\r\n\x1b]133;D;0\x07\x1b]133;A;redraw=1\x07P> ",
    );
    t.resize(size(60, 10));
    let screen = t.screen_text();
    let lines: Vec<&str> = screen.lines().map(str::trim_end).collect();
    assert!(
        lines.iter().any(|line| line.starts_with("foo%")),
        "{lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("foo%") && line.contains("P>")),
        "{lines:?}"
    );
    assert!(lines.contains(&"P> cmd"), "{lines:?}");
}
