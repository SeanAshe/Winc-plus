//! `cells` 的测试：按行拆分单元格、Tab 与回车在单元格之间移动，以及增删行和列。

use super::*;

fn contents(line: &str) -> Vec<&str> {
    cells(line)
        .iter()
        .map(|c| &line[c.content.clone()])
        .collect()
}

#[test]
fn rows_split_into_cells() {
    assert_eq!(contents("| a | b |"), ["a", "b"]);
    assert_eq!(contents("a | b"), ["a", "b"]);
    assert_eq!(contents("| a \\| b | `c` |"), ["a \\| b", "`c`"]);
    assert_eq!(cells("|  | x |")[0].content, 2..2);
    // 只有 `|` 时没有单元格；空行是一个空单元格；末尾的 `\\|` 中 `|` 未被转义。
    assert!(cells("|").is_empty());
    assert_eq!(contents(""), [""]);
    assert_eq!(contents("| a \\\\|  "), ["a \\\\"]);
    assert!(is_delimiter("|---|:-:|"));
    assert!(is_delimiter("--- | ---"));
    assert!(is_delimiter("  | :--- | ---: |  "));
    assert!(!is_delimiter(""));
    assert!(!is_delimiter("|   |"));
    assert!(!is_delimiter("| : |"));
    assert!(!is_delimiter("| --- \\| --- |"));
    assert!(!is_delimiter("| a | b |"));
    // 与切分单元格相同，按 Unicode 去除两侧空白（全角空格）。
    assert!(is_delimiter("|\u{3000}---\u{3000}|"));
}

/// 在 `text` 的光标处（用 `^` 标出）走一步，返回改完的文字，选区用 `[` `]` 标出。
fn walk(text: &str, step_by: Step) -> String {
    let caret = text.find('^').unwrap();
    let text = text.replace('^', "");
    let target = step(&text, caret, step_by).unwrap();
    let mut text = text;
    if let Some((at, row)) = target.insert {
        text.insert_str(at, &row);
    }
    text.insert(target.select.end, ']');
    text.insert(target.select.start, '[');
    text
}

#[test]
fn tab_and_enter_move_between_cells() {
    let table = "| a | b |\n|---|---|\n| c^ | d |\n| e | f |";
    assert_eq!(
        walk(table, Step::Next),
        "| a | b |\n|---|---|\n| c | [d] |\n| e | f |"
    );
    assert_eq!(
        walk(table, Step::Previous),
        "| a | [b] |\n|---|---|\n| c | d |\n| e | f |"
    );
    assert_eq!(
        walk(table, Step::Down),
        "| a | b |\n|---|---|\n| c | d |\n| [e] | f |"
    );
    // 行末的 Tab 移到下一行的第一格。
    let table = "| a | b^ |\n|---|---|\n| c | d |";
    assert_eq!(walk(table, Step::Next), "| a | b |\n|---|---|\n| [c] | d |");
    // 在最后一行继续向下：新增一行。
    let table = "| a | b |\n|---|---|\n| c | d^ |\n";
    assert_eq!(
        walk(table, Step::Next),
        "| a | b |\n|---|---|\n| c | d |\n| [] |  |\n"
    );
    assert_eq!(
        walk(table, Step::Down),
        "| a | b |\n|---|---|\n| c | d |\n|  | [] |\n"
    );
    // 表头下面那一行不在任何单元格中。
    assert!(step("| a |\n|---|\n| b |", 7, Step::Next).is_none());
}

/// 在 `text` 的 `^` 处按 `how` 修改表格，返回修改后的文字，光标位置用 `^` 标出。
fn edit(text: &str, how: Change) -> Option<String> {
    let caret = text.find('^').unwrap();
    let (mut text, at) = change(&text.replace('^', ""), caret, how)?;
    text.insert(at, '^');
    Some(text)
}

#[test]
fn rows_and_columns_are_added_and_removed() {
    use super::super::Align;
    let table = "| a | b |\n|---|---|\n| c^ | d |\n\n后文";
    assert_eq!(
        edit(table, Change::RowBelow).unwrap(),
        "| a | b |\n|---|---|\n| c | d |\n| ^ |  |\n\n后文"
    );
    assert_eq!(
        edit(table, Change::RowAbove).unwrap(),
        "| a | b |\n|---|---|\n| ^ |  |\n| c | d |\n\n后文"
    );
    assert_eq!(
        edit(table, Change::ColumnRight).unwrap(),
        "| a |  | b |\n|---|---|---|\n| c | ^ | d |\n\n后文"
    );
    assert_eq!(
        edit(table, Change::ColumnLeft).unwrap(),
        "|  | a | b |\n|---|---|---|\n| ^ | c | d |\n\n后文"
    );
    assert_eq!(
        edit(table, Change::DeleteColumn).unwrap(),
        "| b |\n|---|\n| ^d |\n\n后文"
    );
    assert_eq!(
        edit(table, Change::DeleteRow).unwrap(),
        "| ^a | b |\n|---|---|\n\n后文"
    );
    assert_eq!(
        edit(table, Change::Align(Align::Center)).unwrap(),
        "| a | b |\n|:---:|---|\n| c^ | d |\n\n后文"
    );
    assert_eq!(
        edit("| a | b |\n|:---:|---|\n| c^ |", Change::Align(Align::None)).unwrap(),
        "| a | b |\n|---|---|\n| c^ |"
    );
    // 光标在表头上：插入到表头下方，不能在其上方插入，也不能删除。
    let head = "| a^ | b |\n|---|---|\n| c | d |";
    assert_eq!(
        edit(head, Change::RowBelow).unwrap(),
        "| a | b |\n|---|---|\n| ^ |  |\n| c | d |"
    );
    assert_eq!(edit(head, Change::RowAbove), None);
    assert_eq!(edit(head, Change::DeleteRow), None);
    // 两端没有 `|` 的写法，以及只剩一列的表格。
    assert_eq!(
        edit("a^ | b\n--- | ---", Change::ColumnLeft).unwrap(),
        "| ^ |a | b\n|---|--- | ---"
    );
    assert_eq!(edit("| a^ |\n|---|", Change::DeleteColumn), None);
    assert_eq!(edit("| a |\n|---^|", Change::RowBelow), None);
}

#[test]
fn spreadsheet_cells_become_a_table() {
    let text = "名称\t数量\r\n苹果\t3\r\n a|b \t\r\n";
    assert_eq!(
        from_tsv(text).as_deref(),
        Some("| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 3 |\n| a\\|b |  |\n")
    );
    // 只有一行、一列或列数不同时不是表格。
    assert_eq!(from_tsv("a\tb"), None);
    assert_eq!(from_tsv("a\nb"), None);
    assert_eq!(from_tsv("a\tb\nc"), None);
    assert_eq!(from_tsv("\tgcc a.c\n\tstrip a.out"), None);
}
