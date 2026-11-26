//! 美化与压缩 JSON 的测试：记号逐字节保留、缩进与换行、转换范围的确定，以及各种无效的写法。

use ropey::Rope;

use super::*;
use crate::editing::edit::History;

/// 按 `layout` 转换 `text` 中 `selection` 范围的 JSON（每级缩进两个空格，换行为 `\n`），
/// 返回转换后的文本和选区；内容已是该排版时返回 `Ok(None)`。
///
/// 撤销后必须还原为原文，否则断言失败。
fn convert(
    text: &str,
    selection: Selection,
    layout: JsonLayout,
) -> Result<Option<(String, Selection)>, JsonError> {
    let mut rope = Rope::from_str(text);
    let Some(transaction) = reformat_json(&rope, selection, layout, "  ", "\n", true)? else {
        return Ok(None);
    };
    transaction.apply(&mut rope);
    let after = rope.to_string();
    let mut history = History::default();
    history.push(transaction.clone());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), text, "撤销后应恢复原文");
    Ok(Some((after, transaction.after)))
}

/// 美化 `text` 中 `selection` 范围的 JSON，返回转换后的文本；内容没有变化时返回原文。
fn pretty(text: &str, selection: Selection) -> Result<String, JsonError> {
    let converted = convert(text, selection, JsonLayout::Pretty)?;
    Ok(converted.map_or_else(|| text.to_owned(), |(after, _)| after))
}

/// 压缩 `text` 中 `selection` 范围的 JSON，返回转换后的文本；内容没有变化时返回原文。
fn compact(text: &str, selection: Selection) -> Result<String, JsonError> {
    let converted = convert(text, selection, JsonLayout::Compact)?;
    Ok(converted.map_or_else(|| text.to_owned(), |(after, _)| after))
}

/// 返回选中整个 `text` 的选区。
fn all(text: &str) -> Selection {
    Selection::new(0, text.chars().count())
}

/// 返回 `text` 中第一个 `needle` 的字符偏移。
fn pos(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap()].chars().count()
}

#[test]
fn pretty_and_compact_keep_every_token_verbatim() {
    // 键的顺序不变；超出 `f64` 精度的整数、小数末尾的 0、转义原样保留；空的容器不换行。
    let source =
        r#"{"b":1,"a":[1,2.50,{"x":null}],"n":12345678901234567890,"s":"é\"\\","e":{},"l":[]}"#;
    let expected = r#"{
  "b": 1,
  "a": [
    1,
    2.50,
    {
      "x": null
    }
  ],
  "n": 12345678901234567890,
  "s": "é\"\\",
  "e": {},
  "l": []
}"#;
    let converted = convert(source, all(source), JsonLayout::Pretty).unwrap();
    let length = expected.chars().count();
    assert_eq!(
        converted,
        Some((expected.to_owned(), Selection::new(0, length)))
    );
    // 压缩是美化的逆过程；字符串中的空白不动。
    assert_eq!(compact(expected, all(expected)).unwrap(), source);
    let spaced = "[ \"a b\" ,\t{ \"c\" :\r\n 1 } ]";
    assert_eq!(compact(spaced, all(spaced)).unwrap(), "[\"a b\",{\"c\":1}]");
    // 已是目标排版时不产生修改。
    assert_eq!(
        convert(expected, all(expected), JsonLayout::Pretty),
        Ok(None)
    );
    assert_eq!(convert(source, all(source), JsonLayout::Compact), Ok(None));
    // 单独的标量也是有效的 JSON。
    assert_eq!(convert("\"a\"", all("\"a\""), JsonLayout::Pretty), Ok(None));
}

#[test]
fn pretty_lines_follow_the_line_indentation_unit_and_line_ending() {
    let text = "\tlet data = {\"a\":[1]};\n";
    let selection = Selection::new(pos(text, "{"), pos(text, ";"));
    let mut rope = Rope::from_str(text);
    let layout = JsonLayout::Pretty;
    let transaction = reformat_json(&rope, selection, layout, "    ", "\r\n", true)
        .unwrap()
        .unwrap();
    transaction.apply(&mut rope);
    assert_eq!(
        rope.to_string(),
        "\tlet data = {\r\n\t    \"a\": [\r\n\t        1\r\n\t    ]\r\n\t};\n"
    );
    // 选区两端的空白不计入，转换后选中新的内容。
    let padded = "x =  [1, 2]  ;";
    let converted = convert(padded, Selection::new(3, 13), JsonLayout::Compact).unwrap();
    assert_eq!(
        converted,
        Some(("x =  [1,2]  ;".to_owned(), Selection::new(5, 10)))
    );
}

#[test]
fn without_a_selection_the_json_around_the_caret_is_converted() {
    // 日志行中包住光标的最外层对象；字符串中的括号不算，行内其余文字不动。
    let log = r#"12:00 INFO msg="{oops" data={"id":7,"tags":["}"]} done"#;
    let expected =
        "12:00 INFO msg=\"{oops\" data={\n  \"id\": 7,\n  \"tags\": [\n    \"}\"\n  ]\n} done";
    for at in [
        pos(log, "{\"id"),
        pos(log, "7"),
        pos(log, "\"}\""),
        pos(log, " done"),
    ] {
        assert_eq!(pretty(log, Selection::point(at)).unwrap(), expected, "{at}");
    }
    assert_eq!(
        pretty(log, Selection::point(pos(log, "INFO"))),
        Err(JsonError::NotFound)
    );
    // 外层在光标之前出错时，转换包住光标的内层。
    let nested = r#"x {"a" {"b":1}} y"#;
    assert_eq!(
        compact(nested, Selection::point(pos(nested, "1"))).unwrap(),
        nested
    );
    assert_eq!(
        pretty(nested, Selection::point(pos(nested, "1"))).unwrap(),
        "x {\"a\" {\n  \"b\": 1\n}} y"
    );

    // 整个文件是一个 JSON 值时转换整个文件，光标在哪一行都一样。
    let file = "{\n  \"a\": 1,\n  \"b\": [\n    true\n  ]\n}\n";
    assert_eq!(
        compact(file, Selection::point(pos(file, "\"a\""))).unwrap(),
        "{\"a\":1,\"b\":[true]}\n"
    );
    // 不读取整个文件时，只看光标所在的行。
    let rope = Rope::from_str(file);
    let caret = Selection::point(pos(file, "\"a\""));
    let compact_line = reformat_json(&rope, caret, JsonLayout::Compact, "  ", "\n", false);
    assert_eq!(compact_line.err(), Some(JsonError::NotFound));
    let caret = Selection::point(pos(file, "["));
    let transaction = reformat_json(&rope, caret, JsonLayout::Compact, "  ", "\n", false);
    assert_eq!(transaction.err(), Some(JsonError::Incomplete));
}

#[test]
fn invalid_json_reports_where_it_goes_wrong() {
    let invalid = |text: &'static str, needle: &str| {
        let at = text.find(needle).unwrap();
        (
            text,
            Err::<String, _>(JsonError::Invalid(text[..at].chars().count())),
        )
    };
    // 位置按字符计，不按字节计。
    for (text, error) in [
        invalid("{\"名称\":1,}", "}"),
        invalid("[01]", "1"),
        invalid("[1.]", "]"),
        invalid("[.5]", "."),
        invalid("[+1]", "+"),
        invalid("[-a]", "a"),
        invalid("[1e]", "]"),
        invalid("['a']", "'"),
        invalid("[tru]", "]"),
        invalid("[NaN]", "N"),
        invalid("[\"a\tb\"]", "\t"),
        invalid("[\"\\x\"]", "x"),
        invalid("[\"\\u12G4\"]", "G"),
        invalid("{\"a\" 1}", "1"),
        invalid("{1:2}", "1"),
        invalid("[1 2]", "2"),
        invalid("[1,]", "]"),
        invalid("{\"a\":1} x", "x"),
        invalid("[1] // note", "//"),
    ] {
        assert_eq!(pretty(text, all(text)), error, "{text}");
    }
    for text in ["{\"a\":", "[1, 2", "\"abc", "[\"\\u12", "[-"] {
        assert_eq!(
            pretty(text, all(text)),
            Err(JsonError::Incomplete),
            "{text}"
        );
    }
    assert_eq!(pretty(" \n ", all(" \n ")), Err(JsonError::NotFound));

    let deep = format!(
        "{}{}",
        "[".repeat(JSON_MAX_DEPTH),
        "]".repeat(JSON_MAX_DEPTH)
    );
    assert_eq!(compact(&deep, all(&deep)).unwrap(), deep);
    let deeper = format!("[{deep}]");
    assert_eq!(compact(&deeper, all(&deeper)), Err(JsonError::TooDeep));
}
