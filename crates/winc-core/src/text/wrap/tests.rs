//! `wrap` 模块的测试：空白与标点处的断行、中日韩文字的断行禁则，
//! 以及行内位置与显示列的换算。

use super::*;

fn split(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let b = breaks(&chars, width, 4);
    (0..b.rows())
        .map(|r| chars[b.range(r, chars.len())].iter().collect())
        .collect()
}

#[test]
fn breaks_after_spaces_and_keeps_the_indent() {
    assert_eq!(split("short", 10), ["short"]);
    assert_eq!(split("hello world again", 11), ["hello world ", "again"]);
    assert_eq!(split("hello world again", 9), ["hello ", "world ", "again"]);
    // 后续各段缩进到与第一段缩进相同的显示列。
    let chars: Vec<char> = "    let value = compute(alpha, beta);".chars().collect();
    let b = breaks(&chars, 20, 4);
    assert_eq!(b.indent, 4);
    assert!(b.rows() >= 2);
    for r in 0..b.rows() {
        let seg = b.range(r, chars.len());
        let cells = b.indent_of(r) + seg.len();
        assert!(cells <= 20, "第 {r} 段 {cells} 格");
    }
    // 单词比行宽还长时在行宽处硬断。
    assert_eq!(split("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    // 缩进之后是超长单词：第一段不能只剩缩进，单词从第一段起就硬断。
    assert_eq!(
        split("    aaaaaaaaaaaaaaaaaaaa", 10),
        ["    aaaaaa", "aaaaaa", "aaaaaa", "aa"]
    );
}

/// 从某个后续段的开头断起，或只断行首到某个断点为止的部分，结果都与整行断行的对应部分相同：
/// 长行只计算屏幕上可见的若干段时依赖该性质。
#[test]
fn breaking_part_of_a_line_matches_the_whole_line() {
    let lines = [
        "    let value = compute(alpha, beta); ".repeat(40),
        "INSERT INTO t VALUES (1,'订单，备注（加急）'),(2,'a b  c'),".repeat(30),
        format!("\t{}", "word\tx  ".repeat(120)),
        "中文没有空格的长句子，也有标点。".repeat(50),
        // 行宽之外的一长串空白留在上一段末尾。
        format!("{}{}tail", "abc def ".repeat(20), " ".repeat(300)),
    ];
    for text in &lines {
        let chars: Vec<char> = text.chars().collect();
        for width in [7, 20, 33, 80] {
            let whole = breaks(&chars, width, 4);
            let rows = whole.rows();
            for from in (1..rows).step_by(3) {
                for to in [from + 1, from + 5, rows] {
                    let to = to.min(rows);
                    let (a, b) = (whole.start_of(from), whole.range(to - 1, chars.len()).end);
                    let part = breaks_from(&chars[a..b], width, 4, whole.indent);
                    let expected: Vec<usize> =
                        (whole.starts[from..to - 1].iter().map(|s| s - a)).collect();
                    assert_eq!(part.starts, expected, "宽 {width}，第 {from}..{to} 段");
                    assert_eq!(part.indent_of(0), whole.indent_of(from));
                    assert_eq!(part.indent_of(1), whole.indent_of(from + 1));
                }
            }
            for to in (2..rows).step_by(4) {
                let head = breaks(&chars[..whole.start_of(to)], width, 4);
                assert_eq!(
                    head.starts,
                    whole.starts[..to - 1],
                    "宽 {width}，前 {to} 段"
                );
                assert_eq!((head.indent, head.head), (whole.indent, 0));
            }
        }
    }
}

/// 不建立字符表的逐段断行与 [`breaks`] 给出相同的断点和缩进。
///
/// 样本由固定的伪随机序列生成，混有 ASCII、汉字、全角标点、制表符、连续空白、不占宽度的
/// 字符和 4 字节的字符，覆盖按字节计算与按字符计算两条路径以及两者的交界。
#[test]
fn each_break_matches_breaks() {
    let pieces = [
        "word",
        " ",
        "  ",
        "\t",
        "x",
        ",",
        "(",
        ")",
        "中文",
        "，",
        "（",
        "」",
        "é",
        "\u{301}",
        "\u{200b}",
        "😀",
        "INSERT INTO t VALUES ",
        "'a b'",
        "-",
        "/",
        ";",
        "\u{1}",
        "          ",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for round in 0..600 {
        let mut text = String::new();
        // 一部分样本以缩进开头，一部分只含 ASCII。
        if round % 3 == 0 {
            text.push_str(&" ".repeat(next(12)));
        }
        let kinds = if round % 4 == 0 { 9 } else { pieces.len() };
        for _ in 0..next(120) + 1 {
            text.push_str(pieces[next(kinds)]);
        }
        let chars: Vec<char> = text.chars().collect();
        for width in [1, 2, 5, 16, 40, 97] {
            let whole = breaks(&chars, width, 4);
            let mut starts = Vec::new();
            let indent = each_break(&text, width, 4, |start| starts.push(start));
            assert_eq!(starts, whole.starts, "宽 {width}：{text:?}");
            assert_eq!(indent, whole.indent, "宽 {width}：{text:?}");
        }
    }
}

/// ASCII 字符按字节计算的显示宽度与 [`char_cells`] 相同。
#[test]
fn ascii_cells_match_char_cells() {
    let bytes: Vec<u8> = (0..128).collect();
    for cell in [0, 3, 8] {
        for (i, &b) in bytes.iter().enumerate() {
            assert_eq!(
                Ascii(&bytes).cells(i, cell, 4),
                char_cells(char::from(b), cell, 4),
                "字符 {b:#x}"
            );
        }
    }
}

#[test]
fn chinese_breaks_anywhere_but_not_before_closing_punctuation() {
    // 每个汉字占 2 个显示列，行宽 6 容纳 3 个字；「，」不能在行首，与前一个字一起移到下一段。
    assert_eq!(split("一二三四五六", 6), ["一二三", "四五六"]);
    assert_eq!(split("一二三，四五", 6), ["一二", "三，四", "五"]);
    // 全角左括号不能在行末。
    assert_eq!(split("一二（三四", 6), ["一二", "（三四"]);
}

#[test]
fn code_breaks_after_punctuation_and_positions_map_back() {
    let parts = split("call(alpha,beta,gamma)", 12);
    assert_eq!(parts.concat(), "call(alpha,beta,gamma)");
    assert!(parts.iter().all(|p| p.chars().count() <= 12));
    assert_eq!(parts[0], "call(alpha,");

    let text = "  aaaa bbbb cccc";
    let chars: Vec<char> = text.chars().collect();
    let b = breaks(&chars, 8, 4);
    assert_eq!(b.starts, vec![7, 12]);
    assert_eq!((b.row_of(6), b.row_of(7), b.row_of(15)), (0, 1, 2));
    let slice = RopeSlice::from(text);
    // 第二段从缩进处（2 个显示列）开始。
    assert_eq!(cell_in_row(slice, &b, 7, 4), 2);
    assert_eq!(cell_in_row(slice, &b, 9, 4), 4);
    // 缩进超过行宽的一半时不缩进；行宽为 0 时按 1 处理，字符逐个硬断。
    assert_eq!(
        breaks(&"        abcdefgh".chars().collect::<Vec<_>>(), 10, 4).indent,
        0
    );
    assert_eq!(breaks(&['a', 'b'], 0, 4).rows(), 2);
}
