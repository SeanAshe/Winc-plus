//! 超长行的测试：展开后的列换算、按列读取、随修改移动，以及换行时按整行断开。

use winc_core::{Lines, Selection};

use crate::document::Document;
use crate::{Command, DocumentId};

/// 第 1 行超长：含制表符和宽字符，共 30 005 个字符。
fn sample() -> Document {
    let long = format!("a\t{}中文\t{}", "x".repeat(15_000), "y".repeat(15_000));
    Document::from_text(DocumentId(1), &format!("short\n{long}\nlast\n"))
}

#[test]
fn whole_lines_map_columns_like_short_ones() {
    let mut doc = sample();
    let naive = |col: usize| {
        let content = doc.text().line_content(1);
        winc_core::text::col_to_cell(content.slice(), col, 4)
    };
    let expected: Vec<usize> = [0, 2, 1024, 15_003, 15_005, 20_000, 30_005]
        .map(naive)
        .to_vec();
    assert!(!doc.shows_whole_line(1));
    doc.show_whole_line(1);
    assert!(doc.shows_whole_line(1) && !doc.shows_whole_line(0));
    for (col, cells) in [0, 2, 1024, 15_003, 15_005, 20_000, 30_005]
        .into_iter()
        .zip(expected)
    {
        assert_eq!(doc.col_to_cell(1, col, 4), cells, "{col}");
        assert_eq!(doc.cell_to_col(1, cells, 4), col, "{cells}");
    }
    // 落在宽字符中间时取较近的一侧，超出行尾时取行尾。
    let wide = doc.col_to_cell(1, 15_002, 4);
    assert_eq!(doc.cell_to_col(1, wide + 1, 4), 15_002);
    assert_eq!(doc.cell_to_col(1, usize::MAX / 2, 4), 30_005);
    // 换算到行尾之后列索引已完整，宽度是准确的。
    let width = (doc.col_to_cell(1, 30_005, 4), true);
    assert_eq!(doc.whole_line_cells(1, 4), Some(width));
    // 不超长的行没有列索引。
    assert_eq!(doc.whole_line_cells(0, 4), None);
}

#[test]
fn spans_cover_the_requested_cells() {
    let mut doc = sample();
    assert!(doc.long_line_span(1, 0..10, 4).is_none());
    doc.show_whole_line(1);
    let span = doc.long_line_span(1, 20_000..20_200, 4).unwrap();
    assert!(span.cell <= 20_000 && !span.ends);
    assert_eq!(doc.col_to_cell(1, span.col, 4), span.cell);
    let end = doc.col_to_cell(1, span.col + span.chars.len(), 4);
    assert!(end >= 20_200 && span.chars.len() <= 3 * 1024);
    let start = doc.text().line_start(1) + span.col;
    assert_eq!(
        span.text,
        doc.text().substring(start, start + span.chars.len())
    );
    // 最后一段延伸到行尾。
    let tail = doc.long_line_span(1, 30_000..40_000, 4).unwrap();
    assert!(tail.ends && tail.col + tail.chars.len() == 30_005);
}

#[test]
fn whole_lines_follow_edits_and_wrap_in_full() {
    let mut doc = sample();
    doc.ensure_wrap(Some(100), 4);
    // 没有展开时只断开前 `MAX_LINE_CHARS` 个字符。
    let capped = doc.rows_of(1);
    assert!((100..=102).contains(&capped), "{capped}");
    doc.show_whole_line(1);
    let whole = doc.rows_of(1);
    assert!((300..=303).contains(&whole), "{whole}");
    assert_eq!(doc.row_of(2), 1 + whole);
    // 靠后的段可以读取。
    let window = doc.line_window(1, 250..251).unwrap();
    assert!(window.col > winc_core::text::MAX_LINE_CHARS);

    // 在上方插入一行：展开的行随之下移，仍按整行断开。
    doc.selection = Selection::point(0);
    doc.execute(Command::Insert("new\n".into()), 10, 4);
    assert!(doc.shows_whole_line(2) && !doc.shows_whole_line(1));
    assert_eq!(doc.rows_of(2), whole);
    // 在该行内输入：重新断开后仍是整行。
    let end = doc.text().line_end(2);
    doc.selection = Selection::point(end);
    doc.execute(Command::Insert("z".repeat(200)), 10, 4);
    assert_eq!(doc.rows_of(2), whole + 2);

    // 全部展开：其余超长行同样按整行断开。
    let mut doc = sample();
    doc.ensure_wrap(Some(100), 4);
    assert!(doc.show_whole_lines(true) && !doc.show_whole_lines(true));
    assert_eq!(doc.rows_of(1), whole);
    assert!(doc.show_whole_lines(false));
    assert_eq!(doc.rows_of(1), capped);
}

#[test]
fn the_column_index_grows_as_needed_and_follows_edits() {
    // 第 0、2 行各 400 001 个字符，中间有一个宽字符。
    let long = format!("{}中{}", "x".repeat(200_000), "y".repeat(200_000));
    let mut doc = Document::from_text(DocumentId(1), &format!("{long}\nshort\n{long}\n"));
    doc.show_whole_lines(true);
    let epoch = doc.long_lines_epoch();
    // 宽度先按每个字符一列估计，不扫描文字。
    assert_eq!(doc.whole_line_cells(0, 4), Some((400_001, false)));
    assert_eq!(doc.long_lines_epoch(), epoch);
    // 读取开头的列：只扫描到用到的位置，宽度仍是估计值。
    let span = doc.long_line_span(0, 0..300, 4).unwrap();
    assert_eq!((span.col, span.cell, span.ends), (0, 0, false));
    assert_eq!(doc.whole_line_cells(0, 4), Some((400_001, false)));
    assert!(doc.long_lines_epoch() > epoch);
    // 读取行尾的列：索引完整，宽度准确，比字符数多出宽字符的一列。
    assert!(doc.long_line_span(0, 400_000..400_100, 4).unwrap().ends);
    assert_eq!(doc.whole_line_cells(0, 4), Some((400_002, true)));

    // 修改第 2 行：第 0 行的索引保留，第 2 行的重新建立。
    doc.long_line_span(2, 400_000..400_100, 4).unwrap();
    doc.selection = Selection::point(doc.text().line_start(2));
    doc.execute(Command::Insert("z".into()), 10, 4);
    assert_eq!(doc.whole_line_cells(0, 4), Some((400_002, true)));
    assert_eq!(doc.whole_line_cells(2, 4), Some((400_002, false)));
    assert_eq!(doc.col_to_cell(2, 400_002, 4), 400_003);
    // 在第 1 行之后插入一行：其后的索引随行号移动。
    doc.selection = Selection::point(doc.text().line_end(1));
    doc.execute(Command::Insert("\nnew".into()), 10, 4);
    assert_eq!(doc.whole_line_cells(3, 4), Some((400_003, true)));
    assert_eq!(doc.whole_line_cells(2, 4), None);
}
