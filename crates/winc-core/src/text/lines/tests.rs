//! `lines` 模块的测试：行与位置的换算、默认方法与 `Rope` 实现的结果一致、行尾风格识别，
//! 以及列布局与显示列换算。

use super::*;

#[test]
fn rope_lines_strip_endings() {
    let text = Rope::from_str("ab\r\ncd\nef");
    assert_eq!(text.line_len(0), 2);
    assert_eq!(line_slice(&text, 0), "ab");
    assert_eq!(text.line_content(1).slice(), "cd");
    assert_eq!(text.line_end(0), 2);
    assert_eq!(text.line_start(1), 4);
    assert_eq!(text.line_span(0), 0..4);
    assert_eq!(text.line_span(2), 7..9);
    assert_eq!(text.pos_to_line_col(3), (0, 2));
    assert_eq!(text.line_col_to_pos(1, 99), 6);
    assert_eq!(text.end_pos(), 9);
}

/// 只实现必需方法的最简文本，用于检查默认方法与 `Rope` 的专门实现结果一致。
struct Plain(Rope);

impl Lines for Plain {
    fn line_count(&self) -> usize {
        self.0.line_count()
    }

    fn line_start(&self, line: usize) -> usize {
        self.0.line_start(line)
    }

    fn line_of(&self, pos: usize) -> usize {
        self.0.line_of(pos)
    }

    fn line_content(&self, line: usize) -> Line<'_> {
        Line::Owned(line_slice(&self.0, line).to_string())
    }

    fn substring(&self, from: usize, to: usize) -> String {
        self.0.substring(from, to)
    }

    fn end_pos(&self) -> usize {
        self.0.end_pos()
    }
}

#[test]
fn default_methods_match_rope() {
    let rope = Rope::from_str("中文\r\nab\n\nend");
    let plain = Plain(rope.clone());

    for line in 0..rope.line_count() {
        assert_eq!(plain.line_len(line), rope.line_len(line));
        assert_eq!(plain.line_span(line), rope.line_span(line));
    }

    for pos in 0..=rope.end_pos() {
        assert_eq!(plain.pos_to_line_col(pos), rope.pos_to_line_col(pos));
        assert_eq!(plain.char_at(pos).is_some(), rope.char_at(pos).is_some());
    }

    fn collect(text: &impl Lines) -> Vec<(usize, usize, String)> {
        let mut out = Vec::new();
        text.visit_lines(1, usize::MAX, |l, s, t| {
            out.push((l, s, t.to_owned()));
            ControlFlow::Continue(())
        });
        out
    }

    let (from_rope, from_plain) = (collect(&rope), collect(&plain));
    assert_eq!(from_rope, from_plain);
    assert_eq!(from_rope[0], (1, 4, "ab".to_owned()));

    // 截断到 4 字节时不切开字符：“中文”只保留“中”（3 字节）。
    let mut prefixes = Vec::new();
    rope.visit_lines(0, 4, |_, _, t| {
        prefixes.push(t.to_owned());
        ControlFlow::Continue(())
    });
    assert_eq!(prefixes, ["中", "ab", "", "end"]);
    assert_eq!(
        rope.visit_lines(0, usize::MAX, |l, _, _| if l == 2 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }),
        Some(2)
    );
}

#[test]
fn only_lf_cr_and_crlf_break_lines() {
    // 分页符（U+000C）和 Unicode 行分隔符（U+2028）不是换行符，留在行内；行号只由 LF、CRLF
    // 和 CR 决定。
    let text = Rope::from_str("a\x0Cb\nc\u{2028}d\re\r\nf");
    assert_eq!(text.line_count(), 4);
    assert_eq!(text.line_content(0).slice().to_string(), "a\x0Cb");
    assert_eq!(text.line_content(1).slice().to_string(), "c\u{2028}d");
}

#[test]
fn detect_line_endings() {
    assert_eq!(
        LineEnding::detect(&Rope::from_str("a\r\nb\r\nc")),
        LineEnding::CrLf
    );
    assert_eq!(
        LineEnding::detect(&Rope::from_str("a\rb\rc")),
        LineEnding::Cr
    );
    assert_eq!(
        LineEnding::detect(&Rope::from_str("a\nb\n")),
        LineEnding::Lf
    );
    assert_eq!(LineEnding::detect(&Rope::from_str("abc")), LineEnding::Lf);
}

/// 很长的字段不把整列撑宽：列宽不超过 `MAX_COLUMN_WIDTH`，超出的字段只推后同一行中其后的列。
#[test]
fn a_long_field_pushes_only_its_own_row() {
    let long = "x".repeat(MAX_COLUMN_WIDTH + 8);
    let rope = Rope::from_str(&format!("a,b\n{long},c\n"));
    let columns = Columns::measure(',', rope.chunks(), 4);
    assert_eq!(columns.starts, [0, MAX_COLUMN_WIDTH + 2]);

    let mut positions = Vec::new();
    columns.layout(line_slice(&rope, 0), 4, usize::MAX, &mut positions, None);
    assert_eq!(positions[2], MAX_COLUMN_WIDTH + 2, "短行仍对齐");
    columns.layout(line_slice(&rope, 1), 4, usize::MAX, &mut positions, None);
    let c = long.len() + 1;
    assert_eq!(positions[c], long.len() + 1, "长字段之后紧跟分隔符的下一格");
}

#[test]
fn csv_columns_line_up() {
    let rope = Rope::from_str("id,name,city\n1,\"Doe, John\",北京\n22,Al,LA\n");
    let columns = Columns::measure(',', rope.chunks(), 4);
    // 第 0 列最宽的字段占 2 个显示列（`id`、`22`），第 1 列最宽的占 11 个（`"Doe, John"`，
    // 引号内的逗号不分列）。
    assert_eq!(columns.starts, [0, 4, 17]);

    // 分隔符紧跟内容，下一列的字符从该列的起始显示列开始。
    let line = line_slice(&rope, 1);
    let mut positions = Vec::new();
    columns.layout(line, 4, usize::MAX, &mut positions, None);
    assert_eq!(&positions[..3], [0, 1, 4]);
    assert_eq!(positions[14], 17, "北京从第三列开始");
    assert_eq!(*positions.last().unwrap(), 21, "北京两个字各占两格");

    let aligned = Aligned {
        columns: &columns,
        tab_width: 4,
    };
    assert_eq!(aligned.col_to_cell(line, 2), 4);
    // 显示列落在第 0 列与第 1 列之间的空隙：距下一列起点更近时取逗号之后（列 2），否则取逗号
    // 之前（列 1）。
    assert_eq!(aligned.cell_to_col(line, 3), 2);
    assert_eq!(aligned.cell_to_col(line, 1), 1);
    assert_eq!(aligned.cell_to_col(line, 99), line.len_chars());

    // 只排前 `limit` 个字符时，末项是这些字符绘制结束处的显示列。
    let mut fields = Vec::new();
    columns.layout(line, 4, 3, &mut positions, Some(&mut fields));
    assert_eq!(
        (&positions[..], &fields[..]),
        (&[0, 1, 4, 5][..], &[0, 0, 1][..])
    );

    let quoted = Rope::from_str("a,\",\",b");
    columns.layout(
        line_slice(&quoted, 0),
        4,
        usize::MAX,
        &mut positions,
        Some(&mut fields),
    );
    assert_eq!(fields, [0, 0, 1, 1, 1, 1, 2]);
    assert_eq!(positions.len(), fields.len() + 1);
}

#[test]
fn cells_handle_tabs_and_wide_chars() {
    let text = Rope::from_str("\tx中文y");
    let line = line_slice(&text, 0);
    assert_eq!(col_to_cell(line, 1, 4), 4);
    assert_eq!(col_to_cell(line, 3, 4), 7);
    assert_eq!(col_to_cell(line, 5, 4), 10);
    assert_eq!(col_to_cell(line, usize::MAX, 4), 10);

    let chars: Vec<char> = "\tx中文y".chars().collect();
    assert_eq!(slice_cells(&chars[..3], 0, 4), 7);
    assert_eq!(slice_cells(&chars, 0, 4), 10);
    assert_eq!(slice_cells(&[], 3, 4), 3);
    // 从第 2 格起排：制表符只排到第 4 格。
    assert_eq!(slice_cells(&chars[..2], 2, 4), 5);

    assert_eq!(cell_to_col(line, 0, 4), 0);
    assert_eq!(cell_to_col(line, 1, 4), 0);
    assert_eq!(cell_to_col(line, 3, 4), 1);
    assert_eq!(cell_to_col(line, 5, 4), 2);
    assert_eq!(cell_to_col(line, 6, 4), 2);
    assert_eq!(cell_to_col(line, 7, 4), 3);
    assert_eq!(cell_to_col(line, 100, 4), 5);
}

#[test]
fn prefix_and_indent() {
    let text = Rope::from_str("\t  # x");
    let line = line_slice(&text, 0);
    assert_eq!(leading_whitespace(line), 3);
    assert_eq!(indent_cells(line, 4), 6);
    assert!(starts_with(line.slice(3..), "# "));
    assert!(!starts_with(line.slice(3..), "# xy"));
}

#[test]
fn prefix_cells_match_slice_cells_for_every_column() {
    for line in ["plain ascii", "a\tb\t\tc", "中文a好", "\u{1}x\u{301}y", ""] {
        let chars: Vec<char> = line.chars().collect();
        let prefix = prefix_cells(&chars, 4);
        assert_eq!(prefix.len(), chars.len() + 1);
        for col in 0..=chars.len() {
            assert_eq!(
                prefix[col],
                slice_cells(&chars[..col], 0, 4),
                "{line:?} {col}"
            );
        }
    }
}

#[test]
fn emoji_sequences_take_two_cells() {
    let cells = |text: &str| text_cells(text.chars(), 0, 4);
    for unit in [
        "❤\u{fe0f}",
        "⭐\u{fe0f}",
        "👍\u{1f3fd}",
        "✌\u{1f3fd}",
        "1\u{fe0f}\u{20e3}",
        "🇨🇳",
        "👨\u{200d}👩\u{200d}👧",
        "👩\u{200d}❤\u{fe0f}\u{200d}👨",
        "🏳\u{fe0f}\u{200d}🌈",
        "🧑\u{1f3ff}\u{200d}💻",
    ] {
        assert_eq!(cells(unit), 2, "{unit:?}");
        // 前后的文字各占自己的列。
        assert_eq!(cells(&format!("a{unit}中b")), 6, "{unit:?}");
    }
    // 不成序列的字符按各自的宽度：单独的符号、连接符、修饰符，文字之后的连接符。
    assert_eq!((cells("❤"), cells("👍"), cells("\u{200d}")), (1, 2, 0));
    assert_eq!(
        (cells("\u{1f3fd}"), cells("a\u{fe0f}"), cells("😀😀")),
        (2, 1, 4)
    );
    assert_eq!(cells("a\u{200d}😀"), 3);

    // 逐列的表、列与显示列的换算和从中间算起的结果都与从行首算起一致。
    let chars: Vec<char> = "a❤\u{fe0f}b👨\u{200d}👩c".chars().collect();
    assert_eq!(prefix_cells(&chars, 4), [0, 1, 2, 3, 4, 6, 6, 6, 7]);
    assert_eq!(slice_cells(&chars, 0, 4), 7);
    for index in 0..chars.len() {
        let mut behind = Behind::before(&chars, index);
        let width = behind.cells(chars[index], 0, 4);
        let table = prefix_cells(&chars, 4);
        assert_eq!(width, table[index + 1] - table[index], "{index}");
    }
}

#[test]
fn one_sequence_is_measured_from_its_first_character() {
    let cells = |text: &str| sequence_cells(text.chars(), 0, 4);
    assert_eq!(cells("❤\u{fe0f}b"), 2);
    assert_eq!(cells("👨\u{200d}👩\u{200d}👧b"), 2);
    assert_eq!(cells("🇨🇳🇺🇸"), 2);
    assert_eq!(cells("1\u{fe0f}\u{20e3}2"), 2);
    // 不成序列时只算第一个字符；制表符按所在的显示列补齐。
    assert_eq!(
        (cells("ab"), cells("中a"), cells("😀😀"), cells("")),
        (1, 2, 2, 0)
    );
    assert_eq!(sequence_cells("\tx".chars(), 1, 4), 3);
}
