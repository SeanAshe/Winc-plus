//! 数据行与源文本行的对应、字段起点的测试。

use super::*;
use crate::table::read_delimited;

/// 引号内的换行（含回车换行）使其后各行数据的起始行号后移；回车换行算一个行尾。
#[test]
fn rows_know_the_line_they_start_on() {
    let sheet = read_delimited("a,b\r\n\"x\ny\r\nz\",1\n\nlast,2\n", false);
    assert_eq!(sheet.delimiter, Some(','));
    assert_eq!(sheet.row_count(), 4);
    let lines: Vec<usize> = (0..4).map(|row| sheet.source_line(row)).collect();
    assert_eq!(lines, [0, 1, 4, 5]);
    let rows: Vec<usize> = (0..7).map(|line| sheet.row_at_line(line)).collect();
    assert_eq!(
        rows,
        [0, 1, 1, 1, 2, 3, 3],
        "跨行的数据各行都对应它；最后一行之后取最后一行"
    );

    // 没有跨行的字段：数据行号就是行号。
    let plain = read_delimited("a\nb\nc\n", false);
    assert_eq!((plain.source_line(2), plain.row_at_line(1)), (2, 1));
}

/// 字段的起点与读表的规则相同：引号只在开头时括起字段，括起的分隔符不分列。
#[test]
fn fields_are_found_by_column_and_by_offset() {
    let line = "1,\"a, \"\"b\"\"\",x\"y,z";
    let start = |column| field_start(line.chars(), ',', column);
    assert_eq!(
        (start(0), start(1), start(2), start(3)),
        (Some(0), Some(2), Some(13), Some(17))
    );
    assert_eq!(start(4), None);
    let at = |offset| field_at(line.chars(), ',', offset);
    assert_eq!(
        (at(0), at(1), at(2), at(12), at(13), at(17), at(99)),
        (0, 0, 1, 1, 2, 3, 3)
    );

    // 不在引号内的行尾结束这行数据；引号内的换行不结束。
    let quoted = "\"a\nb\",c\nnext,d";
    assert_eq!(field_start(quoted.chars(), ',', 1), Some(6));
    assert_eq!(field_start(quoted.chars(), ',', 2), None);
}
