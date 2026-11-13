//! `term` 模块的测试：喂进字节，检查屏幕、光标、回滚缓冲、命令块和事件。

use super::*;
use crate::protocol::{Attrs, CommandMark, Underline};

mod blocks;

/// shell 集成上报命令行时附带的随机数（16 个十六进制字符）。
const NONCE: &str = "0123456789abcdef";

/// 创建开启了 shell 集成的终端：命令行须带 [`NONCE`] 才可信。
fn term(cols: usize, rows: usize) -> Terminal {
    let mut t = Terminal::new(cols, rows, 1000);
    t.set_nonce(NONCE.into());
    t
}

fn feed(t: &mut Terminal, s: &str) {
    t.feed(s.as_bytes());
}

fn size(cols: u16, rows: u16) -> Size {
    Size {
        cols,
        rows,
        width: cols * 8,
        height: rows * 16,
    }
}

fn history_text(t: &Terminal) -> Vec<String> {
    t.history(0, u64::MAX)
        .1
        .iter()
        .map(|l| l.trimmed().to_string())
        .collect()
}

/// 只靠帧还原出来的屏幕，还原方式与主程序相同：先按 `scroll` 上移，再换上 `lines`。
#[derive(Default)]
struct Mirror {
    rows: Vec<Line>,
}

impl Mirror {
    fn apply(&mut self, frame: &Frame) {
        let rows = usize::from(frame.rows);
        if frame.full || self.rows.len() != rows {
            self.rows = vec![Line::default(); rows];
        }
        let scroll = usize::from(frame.scroll).min(rows);
        self.rows.rotate_left(scroll);
        for (row, line) in &frame.lines {
            self.rows[usize::from(*row)] = line.clone();
        }
    }

    /// 取一帧应用到镜像，再逐行核对镜像与终端中对应的行。
    fn check(&mut self, t: &mut Terminal) {
        if let Some(frame) = t.take_frame() {
            self.apply(&frame);
        }
        for r in 0..t.rows() {
            assert_eq!(self.rows[r], t.line(r), "第 {r} 行和帧还原的不一样");
        }
    }
}

#[test]
fn text_and_newlines() {
    let mut t = term(10, 3);
    feed(&mut t, "hello\r\nworld");
    assert_eq!(t.screen_text(), "hello\nworld");
    assert_eq!(t.cursor(), (1, 5));
}

#[test]
fn writing_past_the_last_column_wraps_softly() {
    let mut t = term(5, 3);
    feed(&mut t, "abcde");
    // 延迟换行：写满后光标停在最后一格，下一个字要写入时才换行。
    assert_eq!(t.cursor(), (0, 4));
    assert!(!t.line(0).wrapped);
    feed(&mut t, "fg");
    assert_eq!(t.screen_text(), "abcde\nfg");
    assert!(t.line(0).wrapped);
    assert_eq!(t.cursor(), (1, 2));

    // 关掉自动换行时一直写在最后一格。
    let mut t = term(5, 3);
    feed(&mut t, "\x1b[?7labcdefg");
    assert_eq!(t.screen_text(), "abcdg");
}

#[test]
fn wide_characters_take_two_cells_and_never_split() {
    let mut t = term(5, 3);
    feed(&mut t, "ab中");
    assert_eq!(t.cursor(), (0, 4));
    feed(&mut t, "c文");
    // 「文」宽 2 格，放不下第 5 格（只有 1 格）：留一个占位空格，换到下一行。
    assert_eq!(t.screen_text(), "ab中c\n文");
    assert_eq!(t.cursor(), (1, 2));

    // 覆盖宽字符的一半时，另一半也擦掉。
    let mut t = term(6, 2);
    feed(&mut t, "中文\x1b[1;2Hx");
    assert_eq!(t.screen_text(), " x文");
}

#[test]
fn lines_scrolled_off_the_top_go_to_the_history() {
    let mut t = term(10, 3);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5");
    assert_eq!(t.screen_text(), "3\n4\n5");
    assert_eq!(history_text(&t), ["1", "2"]);
    assert_eq!(t.take_frame().unwrap().history, (0, 2));

    // 上限之外的旧行丢弃，行号仍然往上数。
    let mut t = Terminal::new(10, 2, 3);
    for i in 0..10 {
        feed(&mut t, &format!("{i}\r\n"));
    }
    let (start, lines) = t.history(0, u64::MAX);
    assert_eq!(start, 6);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].trimmed(), "6");
}

#[test]
fn a_scroll_region_scrolls_alone_and_skips_the_history() {
    let mut t = term(10, 5);
    feed(
        &mut t,
        "top\x1b[2;4r\x1b[2;1Ha\r\nb\r\nc\r\nd\x1b[5;1Hbottom",
    );
    assert_eq!(t.screen_text(), "top\nb\nc\nd\nbottom");
    assert!(history_text(&t).is_empty());

    // 滚动区域内插入行（IL），再删除行（DL）。
    feed(&mut t, "\x1b[2;1H\x1b[L");
    assert_eq!(t.screen_text(), "top\n\nb\nc\nbottom");
    feed(&mut t, "\x1b[2M");
    assert_eq!(t.screen_text(), "top\nc\n\n\nbottom");
}

#[test]
fn erasing_keeps_the_current_background() {
    let mut t = term(4, 2);
    feed(&mut t, "\x1b[41m\x1b[2K\x1b[0m");
    let line = t.line(0);
    assert_eq!(line.text, "    ");
    assert_eq!(line.runs[0].style.bg, Color::Indexed(1));

    let mut t = term(8, 2);
    feed(&mut t, "abcdefgh\r\x1b[3C\x1b[K");
    assert_eq!(t.screen_text(), "abc");
    feed(&mut t, "\x1b[1K");
    assert_eq!(t.screen_text(), "");
    feed(&mut t, "xyzw\x1b[2D\x1b[X");
    assert_eq!(t.screen_text(), "   xy w");
}

#[test]
fn inserting_and_deleting_characters() {
    let mut t = term(8, 1);
    feed(&mut t, "abcdef\x1b[1;3H\x1b[2@");
    assert_eq!(t.screen_text(), "ab  cdef");
    feed(&mut t, "\x1b[3P");
    assert_eq!(t.screen_text(), "abdef");
    feed(&mut t, "\x1b[4h12\x1b[4l");
    assert_eq!(t.screen_text(), "ab12def");
}

#[test]
fn sgr_sets_every_kind_of_colour_and_attribute() {
    let mut t = term(10, 1);
    feed(
        &mut t,
        "\x1b[1;3;38;5;196;48:2::1:2:3;4:3;58;2;9;8;7mX\x1b[22;23;24;39;49;59mY\x1b[97;104mZ",
    );
    let line = t.line(0);
    let x = line.runs[0].style;
    assert_eq!(x.fg, Color::Indexed(196));
    assert_eq!(x.bg, Color::Rgb(1, 2, 3));
    assert_eq!(x.underline_color, Color::Rgb(9, 8, 7));
    assert!(x.attrs.has(Attrs::BOLD | Attrs::ITALIC));
    assert_eq!(x.attrs.underline(), Underline::Curly);
    assert_eq!(line.runs[1].style, Style::default());
    assert_eq!(line.runs[2].style.fg, Color::Indexed(15));
    assert_eq!(line.runs[2].style.bg, Color::Indexed(12));
}

#[test]
fn queries_are_answered() {
    let mut t = term(20, 5);
    feed(
        &mut t,
        "\x1b[c\x1b[3;5H\x1b[6n\x1b[?2004h\x1b[?2004$p\x1b[?9999$p",
    );
    assert_eq!(
        String::from_utf8(t.take_responses()).unwrap(),
        "\x1b[?62;22c\x1b[3;5R\x1b[?2004;1$y\x1b[?9999;0$y"
    );

    feed(&mut t, "\x1b[>q");
    assert!(
        String::from_utf8(t.take_responses())
            .unwrap()
            .starts_with("\x1bP>|winc ")
    );

    feed(&mut t, "\x1b[18t");
    assert_eq!(t.take_responses(), b"\x1b[8;5;20t");

    feed(&mut t, "\x1bP$qm\x1b\\\x1bP+q544e\x1b\\");
    assert_eq!(t.take_responses(), b"\x1bP1$r0m\x1b\\\x1bP0+r544e\x1b\\");
}

#[test]
fn termcap_reply_echoes_only_hex_names() {
    let mut t = term(20, 5);
    feed(&mut t, "\x1bP+q544e;436f\x1b\\");
    assert_eq!(t.take_responses(), b"\x1bP0+r544e\x1b\\\x1bP0+r436f\x1b\\");

    // ^U 清除回答的前缀、回车提交整行：原样带回会使 shell 执行一条命令。
    feed(&mut t, "\x1bP+q\x15touch pwned\r;544e;5\x1b\\");
    assert_eq!(
        t.take_responses(),
        b"\x1bP0+r\x1b\\\x1bP0+r544e\x1b\\\x1bP0+r\x1b\\"
    );
}

#[test]
fn kitty_keyboard_flags_are_a_stack_per_screen() {
    let mut t = term(10, 2);
    feed(&mut t, "\x1b[>31u\x1b[?u");
    // 只支持第 1 级。
    assert_eq!(t.take_responses(), b"\x1b[?1u");
    assert_eq!(t.take_frame().unwrap().keyboard, 1);
    feed(&mut t, "\x1b[?1049h\x1b[?u");
    assert_eq!(t.take_responses(), b"\x1b[?0u");
    feed(&mut t, "\x1b[?1049l\x1b[<u\x1b[?u");
    assert_eq!(t.take_responses(), b"\x1b[?0u");
}

#[test]
fn colour_queries_use_the_theme() {
    let mut t = term(10, 2);
    feed(&mut t, "\x1b]11;?\x07");
    assert!(t.take_responses().is_empty(), "没有主题时不回答");

    t.set_theme(Some(Theme {
        bg: [0x18, 0x1a, 0x1e],
        palette: [[0xff, 0, 0]; 16],
        ..Theme::default()
    }));
    feed(&mut t, "\x1b]11;?\x1b\\\x1b]4;1;?\x07");
    // 回答的结束符和查询相同；Windows 上一律用 ST，见 `osc_dispatch`。
    let bell = if cfg!(windows) { "\x1b\\" } else { "\x07" };
    assert_eq!(
        String::from_utf8(t.take_responses()).unwrap(),
        format!("\x1b]11;rgb:1818/1a1a/1e1e\x1b\\\x1b]4;1;rgb:ffff/0000/0000{bell}")
    );
}

#[test]
fn hyperlinks_number_their_targets_once() {
    let mut t = term(20, 1);
    let open = "\x1b]8;;https://a.example/x;y\x1b\\";
    feed(&mut t, &format!("{open}ab\x1b]8;;\x1b\\c{open}d"));
    assert_eq!(
        t.take_events(),
        [Event::Link(1, "https://a.example/x;y".into())]
    );
    let line = t.line(0);
    let links: Vec<u16> = line.runs.iter().map(|r| r.style.link).collect();
    assert_eq!(links, [1, 0, 1]);
}

/// 喂进一条完整的命令：提示符 `A`、命令 `B`、开始运行 `C`、结束 `D`。
fn command(t: &mut Terminal, cmd: &str, output: &str, exit: i32) {
    let url = cmd.replace(' ', "%20");
    feed(
        t,
        &format!(
            "\x1b]133;A\x07$ \x1b]133;B\x07{cmd}\r\n\x1b]133;C;nonce={NONCE};cmdline_url={url}\x07{output}\x1b]133;D;{exit}\x07"
        ),
    );
}

#[test]
fn spammed_titles_and_links_stay_bounded() {
    let mut t = term(40, 4);
    let mut bytes = String::new();
    for i in 0..20_000 {
        bytes.push_str(&format!("\x1b]0;title {i}\x07"));
    }
    feed(&mut t, &bytes);
    // 连续修改的标题只保留最后一个。
    assert_eq!(t.take_events(), [Event::Title("title 19999".into())]);

    // 不同的链接地址记到上限为止，之后的不再是链接。
    let long = "x".repeat(1000);
    let mut bytes = String::new();
    for i in 0..6000 {
        bytes.push_str(&format!(
            "\x1b]8;;https://e.com/{long}/{i}\x07a\x1b]8;;\x07"
        ));
    }
    feed(&mut t, &bytes);
    let links = t
        .take_events()
        .iter()
        .filter(|e| matches!(e, Event::Link(..)))
        .count();
    assert!(links > 1000 && links < 4300, "{links}");
    // 太长的地址不当链接。
    feed(
        &mut t,
        &format!("\x1b]8;;https://e.com/{}\x07a", "y".repeat(3000)),
    );
    assert!(t.take_events().is_empty());
    assert!(t.line(t.cursor().0).runs.iter().all(|r| r.style.link == 0));
}

#[test]
fn line_drawing_charset_and_repeat() {
    let mut t = term(10, 1);
    feed(&mut t, "\x1b(0lqk\x1b(Bx\x1b[3b");
    assert_eq!(t.screen_text(), "┌─┐xxxx");
}

#[test]
fn tabs_and_origin_mode() {
    let mut t = term(20, 5);
    feed(&mut t, "a\tb\x1b[3g\r\x1b[5C\x1bH\rc\td");
    assert_eq!(t.screen_text(), "c    d  b");

    feed(&mut t, "\x1b[2;4r\x1b[?6h\x1b[1;1Hx\x1b[9;1Hy");
    // 原点模式下行号从滚动区域算起，也出不了区域。
    assert_eq!(t.screen_text(), "c    d  b\nx\n\ny");
}

#[test]
fn reflow_follows_soft_wraps_and_the_cursor() {
    let mut t = term(10, 4);
    feed(&mut t, "0123456789abcde\r\nxyz");
    assert_eq!(t.screen_text(), "0123456789\nabcde\nxyz");

    t.resize(size(20, 4));
    assert_eq!(t.screen_text(), "0123456789abcde\nxyz");
    assert_eq!(t.cursor(), (1, 3));

    t.resize(size(4, 4));
    // 容纳不下的行移入回滚缓冲，光标所在的行留在屏幕上。
    assert_eq!(t.screen_text(), "4567\n89ab\ncde\nxyz");
    assert_eq!(history_text(&t), ["0123"]);
    assert_eq!(t.cursor(), (3, 3));

    t.resize(size(10, 4));
    assert_eq!(t.screen_text(), "0123456789\nabcde\nxyz");
    assert!(history_text(&t).is_empty());
    assert_eq!(t.cursor(), (2, 3));
}

#[test]
fn a_cursor_waiting_to_wrap_keeps_waiting_through_a_resize() {
    // 写满一行、光标等着折行时改尺寸：下一个字接在后面，不盖掉最后一个字。
    for (cols, rows, want) in [
        (10, 8, "0123456789\nX"),
        (20, 5, "0123456789X"),
        (6, 5, "012345\n6789X"),
    ] {
        let mut t = term(10, 5);
        feed(&mut t, "0123456789");
        t.resize(size(cols, rows));
        feed(&mut t, "X");
        assert_eq!(t.screen_text(), want, "{cols}x{rows}");
    }
}

#[test]
fn the_saved_cursor_follows_the_content_through_a_resize() {
    // 在备用屏里改了高度：回滚缓冲拉回三行，退回主屏后光标还在提示符后面。
    let mut t = term(10, 5);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n$ ");
    feed(&mut t, "\x1b[?1049h");
    t.resize(size(10, 10));
    feed(&mut t, "\x1b[?1049l");
    assert_eq!(t.cursor(), (7, 2));

    // DECSC 之后改宽度，折开的行合回一行，DECRC 回到同一个字后面。
    let mut t = term(10, 4);
    feed(&mut t, "0123456789abcdefghij\r\n$ \x1b7");
    t.resize(size(20, 4));
    feed(&mut t, "\x1b8X");
    assert_eq!(t.screen_text(), "0123456789abcdefghij\n$ X");
}

#[test]
fn entering_the_alternate_screen_again_clears_it() {
    let mut t = term(10, 3);
    feed(&mut t, "$ \x1b[?1049hold");
    feed(&mut t, "\x1b[?1049h\x1b[Hnew");
    assert_eq!(t.screen_text(), "new");
    // 退回主屏后，shell 的屏幕内容和光标都还在。
    feed(&mut t, "\x1b[?1049l");
    assert_eq!(t.screen_text(), "$");
}

#[test]
fn decaln_resets_the_margins_and_homes_the_cursor() {
    let mut t = term(4, 4);
    feed(&mut t, "\x1b[2;3r\x1b[3;3H\x1b#8");
    assert_eq!(t.cursor(), (0, 0));
    assert_eq!(t.screen_text(), "EEEE\nEEEE\nEEEE\nEEEE");
    // 滚动区域是整屏：在最后一行换行整屏上滚。
    feed(&mut t, "\x1b[4;1H\n");
    assert_eq!(t.screen_text(), "EEEE\nEEEE\nEEEE");
}

#[test]
fn reflow_moves_wide_characters_whole() {
    let mut t = term(6, 3);
    feed(&mut t, "ab中文字");
    assert_eq!(t.screen_text(), "ab中文\n字");
    t.resize(size(5, 3));
    assert_eq!(t.screen_text(), "ab中\n文字");
    t.resize(size(6, 3));
    assert_eq!(t.screen_text(), "ab中文\n字");
}

#[test]
fn shrinking_pushes_rows_into_the_history_and_growing_pulls_them_back() {
    let mut t = term(10, 4);
    feed(&mut t, "1\r\n2\r\n3\r\n4");
    t.resize(size(10, 2));
    assert_eq!(t.screen_text(), "3\n4");
    assert_eq!(history_text(&t), ["1", "2"]);
    t.resize(size(10, 5));
    assert_eq!(t.screen_text(), "1\n2\n3\n4");
    assert_eq!(t.cursor(), (3, 1));

    // 光标下面的空行先去掉，不把有内容的行顶走。
    let mut t = term(10, 5);
    feed(&mut t, "$ ");
    t.resize(size(10, 2));
    assert_eq!(t.screen_text(), "$");
    assert!(history_text(&t).is_empty());
}

#[test]
fn only_a_width_change_renumbers_the_history() {
    let mut t = term(10, 4);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\n$ ");
    t.take_frame();
    // 只改行数：历史行和行号都不变，主程序缓存的不用丢弃。
    t.resize(size(10, 2));
    let frame = t.take_frame().unwrap();
    assert!(!frame.full);
    assert_eq!((frame.rows, frame.lines.len()), (2, 2), "整屏重发");
    let (start, lines) = t.history(0, 2);
    assert_eq!((start, lines[1].trimmed()), (0, "2"));
    t.resize(size(10, 6));
    assert!(!t.take_frame().unwrap().full);
    // 改宽度要重排，行号会变。
    t.resize(size(8, 6));
    assert!(t.take_frame().unwrap().full);
}

#[test]
fn frames_carry_only_what_changed() {
    let mut t = term(10, 3);
    let mut mirror = Mirror::default();
    feed(&mut t, "a\r\nb\r\nc");
    mirror.check(&mut t);
    assert!(t.take_frame().is_none(), "没变化时没有帧");

    // 滚动一行：只发上移的行数和新的一行。
    feed(&mut t, "\r\nd");
    let frame = t.take_frame().unwrap();
    assert_eq!(frame.scroll, 1);
    assert_eq!(frame.lines.len(), 1);
    mirror.apply(&frame);
    mirror.check(&mut t);

    // 只动了光标也要发。
    feed(&mut t, "\x1b[1;1H");
    let frame = t.take_frame().unwrap();
    assert!(frame.lines.is_empty());
    assert_eq!((frame.cursor.row, frame.cursor.col), (0, 0));
}

#[test]
fn reset_and_soft_reset() {
    let mut t = term(10, 3);
    feed(&mut t, "\x1b[1mhi\x1b[?25l\x1b[2;3r\x1b[!p");
    assert!(t.take_frame().unwrap().cursor.visible);
    // 设滚动区域时光标回到左上角，`x` 盖掉了 `h`。
    feed(&mut t, "x");
    assert_eq!(t.screen_text(), "xi");
    assert_eq!(t.line(0).runs[0].style, Style::default());
    feed(&mut t, "\x1bc");
    assert_eq!(t.screen_text(), "");
    assert_eq!(t.cursor(), (0, 0));
}

/// 核对宽字符的两半成对：前半之后紧跟后半，后半之前紧挨前半，且格宽与字符宽度一致。
fn consistent(t: &Terminal, context: &str) {
    for screen in [&t.emu.primary, &t.emu.alternate] {
        for (r, line) in screen.rows.iter().enumerate() {
            assert_eq!(line.cells.len(), t.cols(), "{context}：第 {r} 行格数不对");
            for (c, cell) in line.cells.iter().enumerate() {
                let wide = cell.attrs.has(row::WIDE);
                if !cell.attrs.has(row::SPACER) {
                    assert_eq!(
                        wide,
                        crate::term::width::char_width(cell.ch) == 2,
                        "{context}：第 {r} 行第 {c} 格的宽度标记不对"
                    );
                }
                let next_spacer = line
                    .cells
                    .get(c + 1)
                    .is_some_and(|n| n.attrs.has(row::SPACER));
                assert_eq!(
                    wide, next_spacer,
                    "{context}：第 {r} 行第 {c} 格的宽字符不成对"
                );
                if cell.attrs.has(row::SPACER) {
                    assert!(
                        c > 0 && line.cells[c - 1].attrs.has(row::WIDE),
                        "{context}：第 {r} 行第 {c} 格"
                    );
                }
            }
        }
    }
}

/// 简单的伪随机数（xorshift），给定种子时序列固定，测试可重复。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// 随机拼接的输入（大量转义序列片段、宽字符、组合字符、控制字符）与随机改尺寸、清屏混合：
/// 要求不 panic，光标不出界，每一帧还原出的屏幕与终端一致。
#[test]
fn random_input_never_breaks_the_grid() {
    const PIECES: &[&str] = &[
        "a",
        "Z",
        " ",
        "中",
        "😀",
        "\u{301}",
        "\r",
        "\n",
        "\t",
        "\x08",
        "\x1b",
        "\x1b[",
        "\x1b[?",
        "1",
        "3",
        "9",
        ";",
        ":",
        "2",
        "5",
        "m",
        "H",
        "J",
        "K",
        "L",
        "M",
        "P",
        "@",
        "X",
        "r",
        "S",
        "T",
        "h",
        "l",
        "A",
        "B",
        "C",
        "D",
        "G",
        "d",
        "b",
        "u",
        "\x1b[?1049h",
        "\x1b[?1049l",
        "\x1b[?7l",
        "\x1b[?7h",
        "\x1b[?6h",
        "\x1b[4h",
        "\x1b7",
        "\x1b8",
        "\x1bM",
        "\x1bD",
        "\x1b(0",
        "\x1b(B",
        "\x1b]8;;x\x07",
        "\x1b]8;;\x07",
        "\x1b]133;A\x07",
        "\x1b]133;D;2\x07",
        "\x1b[38:2::1:2:3m",
        "\x1b[58;5;3m",
        "\x1b[3J",
        "\x1bc",
        "\x1b#8",
        "\x1bP$qm\x1b\\",
    ];

    for seed in 1..=150u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut t = Terminal::new(1 + rng.below(30), 1 + rng.below(12), 50);
        let mut mirror = Mirror::default();

        for step in 0..60 {
            let mut chunk = String::new();
            for _ in 0..rng.below(80) {
                let piece = PIECES[rng.below(PIECES.len())];
                chunk.push_str(piece);
                t.feed(piece.as_bytes());
                consistent(&t, &format!("种子 {seed}：{chunk:?}"));
            }
            if step % 7 == 6 {
                let before = t.screen_text();
                let new = size(1 + rng.below(30) as u16, 1 + rng.below(12) as u16);
                t.resize(new);
                consistent(&t, &format!("种子 {seed}：{before:?} 改成 {new:?}"));
            }
            if rng.below(4) == 0 {
                t.clear();
                consistent(&t, &format!("种子 {seed}：清屏"));
            }

            let (row, col) = t.cursor();
            assert!(row < t.rows() && col < t.cols(), "种子 {seed}：光标出界");
            for r in 0..t.rows() {
                let line = t.line(r);
                assert!(
                    row::line_cells(&line) <= t.cols(),
                    "种子 {seed}：第 {r} 行超宽"
                );
            }
            mirror.check(&mut t);
            let _ = t.take_events();
            let _ = t.take_responses();
        }
    }
}

/// 回归：OSC 9、777 的通知正文曾被整段留在内存里并发给主程序；正文须截到可读长度。
#[test]
fn notifications_are_cut_to_a_readable_length() {
    let mut t = term(20, 5);
    let long = "通".repeat(5000);
    feed(
        &mut t,
        &format!("\x1b]9;{long}\x07\x1b]777;notify;{long};{long}\x07"),
    );
    for event in t.take_events() {
        let Event::Notify { title, body } = event else {
            panic!("{event:?}");
        };
        assert!(title.chars().count() <= 1024);
        assert_eq!(body.chars().count(), 1024);
    }
}

/// 测试用的 base64 编码（标准字母表，补 `=`）。
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 只统计内容，去掉回滚缓冲里的空行。
fn history_lines(t: &Terminal) -> Vec<String> {
    let mut lines = history_text(t);
    lines.retain(|line| !line.is_empty());
    lines
}

#[test]
fn refreshing_the_screen_with_newlines_keeps_earlier_output() {
    // Vite 的做法：先用换行把屏幕内容推进回滚缓冲，再回到左上角擦除到末尾。
    let mut t = term(80, 10);
    for i in 0..30 {
        feed(&mut t, &format!("build {i}\r\n"));
    }
    for i in 0..3 {
        feed(&mut t, &"\n".repeat(9));
        feed(&mut t, "\x1b[1;1H\x1b[0J");
        feed(&mut t, &format!("hmr update (x{i})\r\n"));
    }
    let history = history_lines(&t);
    assert_eq!(history[0], "build 0");
    assert!(history.iter().any(|line| line == "build 29"));
}

#[test]
fn erasing_the_screen_moves_its_content_into_the_scrollback() {
    let mut t = term(20, 4);
    feed(&mut t, "one\r\ntwo\r\n");
    feed(&mut t, "\x1b[2J\x1b[H");
    assert_eq!(history_text(&t), ["one", "two"]);
    assert_eq!(t.cursor(), (0, 0));
    assert!(t.screen_text().trim().is_empty());

    // 行号连续：之后的输出接在后面。
    feed(&mut t, "three\r\n\x1b[2J");
    assert_eq!(history_text(&t), ["one", "two", "three"]);
}

#[test]
fn erasing_an_empty_screen_adds_nothing() {
    let mut t = term(20, 4);
    feed(&mut t, "\x1b[2J\x1b[2J");
    assert!(history_text(&t).is_empty());
    // 重复清屏不产生重复的空行。
    feed(&mut t, "x\x1b[2J\x1b[2J");
    assert_eq!(history_text(&t), ["x"]);
}

#[test]
fn output_after_clearing_the_scrollback_is_kept_by_the_next_erase() {
    let mut t = term(20, 4);
    feed(&mut t, "\x1b[3Jnew\x1b[2J");
    assert_eq!(history_text(&t), ["new"]);
}

#[test]
fn erasing_keeps_the_scroll_region_and_ignores_it_for_the_scrollback() {
    let mut t = term(20, 6);
    feed(&mut t, "top\r\n\x1b[3;5r\x1b[2;1Hmiddle\x1b[2J");
    assert_eq!(history_text(&t), ["top", "middle"]);
    // 区域没有变化：第 3 到 5 行内的滚动不进回滚缓冲。
    feed(&mut t, "\x1b[5;1Hx\r\n");
    assert_eq!(history_text(&t), ["top", "middle"]);
}

#[test]
fn the_alternate_screen_is_not_kept_when_erased() {
    let mut t = term(20, 4);
    feed(&mut t, "\x1b[?1049hfull screen\x1b[2J");
    assert!(history_text(&t).is_empty());
}

#[test]
fn clearing_from_the_top_left_corner_keeps_the_screen() {
    // `console.clear()` 的序列。
    let mut t = term(20, 4);
    feed(&mut t, "one\r\ntwo\r\n\x1b[1;1H\x1b[0J");
    assert_eq!(history_text(&t), ["one", "two"]);
    assert!(t.screen_text().trim().is_empty());
}

#[test]
fn clearing_from_elsewhere_keeps_nothing() {
    let mut t = term(20, 4);
    feed(&mut t, "one\r\ntwo\x1b[2;1H\x1b[0J");
    assert!(history_text(&t).is_empty());
    assert!(t.screen_text().contains("one"));
    assert!(!t.screen_text().contains("two"));

    let mut t = term(20, 4);
    feed(&mut t, "\x1b[?1049hfull\x1b[H\x1b[J");
    assert!(history_text(&t).is_empty());

    // 先清回滚缓冲，再从左上角清屏。
    let mut t = term(20, 4);
    feed(&mut t, "one\r\n\x1b[3J\x1b[H\x1b[J");
    assert!(history_text(&t).is_empty());
    assert!(t.screen_text().trim().is_empty());
}

#[test]
fn redrawing_the_same_screen_is_kept_once() {
    let mut t = term(20, 4);
    for _ in 0..5 {
        feed(&mut t, "\x1b[H\x1b[Jsame\r\nframe\r\n");
    }
    feed(&mut t, "\x1b[H\x1b[J");
    assert_eq!(history_text(&t), ["same", "frame"]);
}

#[test]
fn different_screens_are_all_kept() {
    let mut t = term(20, 4);
    for frame in ["a", "b", "a"] {
        feed(&mut t, &format!("{frame}\x1b[2J\x1b[H"));
    }
    assert_eq!(history_text(&t), ["a", "b", "a"]);
}

#[test]
fn the_same_screen_is_kept_again_after_other_output_scrolled() {
    let mut t = term(20, 2);
    feed(&mut t, "same\x1b[2J\x1b[H");
    // 其他输出滚入回滚缓冲后，相同的画面不再是连续重复。
    feed(&mut t, "x\r\ny\r\nz\x1b[2J\x1b[H");
    feed(&mut t, "same\x1b[2J\x1b[H");
    let history = history_text(&t);
    assert_eq!(
        history.iter().filter(|line| *line == "same").count(),
        2,
        "{history:?}"
    );
}

#[test]
fn clearing_the_scrollback_forgets_the_last_kept_screen() {
    let mut t = term(20, 4);
    feed(&mut t, "same\x1b[2J\x1b[H\x1b[3J");
    feed(&mut t, "same\x1b[2J\x1b[H");
    assert_eq!(history_text(&t), ["same"]);
}

/// 一个停在第一行提示符上的终端。
fn at_prompt_on_first_row() -> Terminal {
    let mut t = term(40, 6);
    feed(&mut t, "\x1b]133;A;redraw=1\x07P> \x1b]133;B\x07");
    t
}

#[test]
fn resizing_forgets_the_last_kept_screen() {
    let mut t = term(20, 4);
    feed(&mut t, "same\x1b[2J\x1b[H");
    t.resize(size(20, 5));
    feed(&mut t, "same\x1b[2J\x1b[H");
    // 尺寸变化后不再和上一次比较，第二次同样保留。
    let history = history_text(&t);
    let same = history.iter().filter(|line| *line == "same").count();
    assert_eq!(same, 2, "{history:?}");
}

#[test]
fn clearing_on_the_alternate_screen_leaves_the_program_alone() {
    let mut t = term(40, 6);
    feed(&mut t, "history\r\n\x1b[?1049hfirst\r\nsecond");
    let before = t.screen_text();
    let cursor = t.cursor();
    t.clear();
    assert_eq!(t.screen_text(), before);
    assert_eq!(t.cursor(), cursor);
    assert!(history_text(&t).is_empty());
}

#[test]
fn the_cursor_blinks_until_a_program_asks_for_a_steady_one() {
    let blink = |t: &mut Terminal| t.take_frame().map(|frame| frame.cursor.blink);
    let mut t = term(20, 4);
    assert_eq!(blink(&mut t), Some(true));
    feed(&mut t, "\x1b[2 q");
    assert_eq!(blink(&mut t), Some(false));
    feed(&mut t, "\x1b[1 q");
    assert_eq!(blink(&mut t), Some(true));
    feed(&mut t, "\x1b[?12l");
    assert_eq!(blink(&mut t), Some(false));
    // 完全重置后恢复默认。
    feed(&mut t, "\x1bc");
    assert_eq!(blink(&mut t), Some(true));
}

#[test]
fn emoji_clusters_move_the_cursor_two_cells() {
    for text in [
        "⚠\u{fe0f}",
        "👍\u{1f3fd}",
        "🧑\u{200d}💻",
        "👨\u{200d}👩\u{200d}👧",
    ] {
        let mut t = term(20, 2);
        feed(&mut t, text);
        assert_eq!(t.cursor(), (0, 2), "{text}");
        feed(&mut t, "x");
        assert_eq!(t.cursor(), (0, 3), "{text}");
        // 文字完整保留，供复制。
        assert_eq!(t.screen_text().trim(), format!("{text}x"), "{text}");
    }
}

#[test]
fn a_full_width_line_with_emoji_does_not_wrap() {
    // 工具按字素簇算出刚好写满一行时，不能多出一行。
    let mut t = term(10, 3);
    feed(&mut t, "12345678⚠\u{fe0f}");
    assert_eq!(t.cursor().0, 0);
    feed(&mut t, "\r\nnext");
    assert_eq!(t.cursor().0, 1);
}

#[test]
fn widening_at_the_last_column_wraps_like_a_wide_character() {
    let mut t = term(4, 3);
    feed(&mut t, "abc⚠\u{fe0f}");
    assert_eq!(t.cursor(), (1, 2));
    let screen = t.screen_text();
    let lines: Vec<&str> = screen.lines().collect();
    assert_eq!(lines[0].trim_end(), "abc");
    assert_eq!(lines[1].trim_end(), "⚠\u{fe0f}");
}

#[test]
fn a_control_sequence_ends_the_cluster() {
    // 光标移动之后的 VS16 不属于移动之前的字。
    let mut t = term(20, 2);
    feed(&mut t, "⚠\x1b[1;5H\u{fe0f}");
    assert_eq!(t.cursor(), (0, 4));
}

#[test]
fn reflow_keeps_the_width_of_emoji_clusters() {
    let mut t = term(4, 3);
    feed(&mut t, "⚠\u{fe0f}🧑\u{200d}💻ab");
    t.resize(size(20, 3));
    assert_eq!(t.screen_text().trim(), "⚠\u{fe0f}🧑\u{200d}💻ab");
    assert_eq!(t.cursor(), (0, 6));
}

#[test]
fn the_grapheme_mode_reports_as_always_on() {
    let mut t = term(20, 2);
    feed(&mut t, "\x1b[?2027$p");
    assert_eq!(t.take_responses(), b"\x1b[?2027;3$y");
}

fn theme(bg: [u8; 3]) -> Theme {
    Theme {
        fg: [0; 3],
        bg,
        cursor: [0; 3],
        palette: [[0; 3]; 16],
    }
}

#[test]
fn the_color_scheme_is_answered_and_followed_when_requested() {
    let mut t = term(20, 2);
    // 没有主题时不回答。
    feed(&mut t, "\x1b[?996n");
    assert!(t.take_responses().is_empty());

    t.set_theme(Some(theme([20, 20, 20])));
    feed(&mut t, "\x1b[?996n");
    assert_eq!(t.take_responses(), b"\x1b[?997;1n");

    // 没有要求通知时，主题变化不发送。
    t.set_theme(Some(theme([250, 250, 250])));
    assert!(t.take_responses().is_empty());
    feed(&mut t, "\x1b[?2031h\x1b[?2031$p");
    assert_eq!(t.take_responses(), b"\x1b[?2031;1$y");

    // 明暗变化时通知一次，同样明暗的主题不重复通知。
    t.set_theme(Some(theme([10, 10, 10])));
    assert_eq!(t.take_responses(), b"\x1b[?997;1n");
    t.set_theme(Some(theme([30, 30, 30])));
    assert!(t.take_responses().is_empty());

    feed(&mut t, "\x1b[?2031l");
    t.set_theme(Some(theme([250, 250, 250])));
    assert!(t.take_responses().is_empty());
}
