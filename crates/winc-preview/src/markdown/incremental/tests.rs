//! 局部重解析的测试：结果与全文解析逐项相同，可能影响窗口之外的改动改为解析全文。

use super::*;

/// 按 `blanks` 解析 `text`：为真时同可编辑的预览，插入空行处的空段落。
fn full(text: &str, blanks: bool) -> Markdown {
    let parsed = parse(text);
    if blanks {
        parsed.with_blank_lines(text)
    } else {
        parsed
    }
}

/// 把 `doc` 的 `old` 换成 `insert`，返回改动后的原文和局部重解析的结果；改为解析全文时结果为
/// `None`。
fn reparse(doc: &str, old: Range<usize>, insert: &str, blanks: bool) -> (String, Option<Markdown>) {
    let next = format!("{}{insert}{}", &doc[..old.start], &doc[old.end..]);
    let new = old.start..old.start + insert.len();
    let line_of_byte = |b: usize| memchr::memchr_iter(b'\n', &next.as_bytes()[..b]).count();
    let slice = |r: Range<usize>| next[r].to_owned();
    let mut parsed = full(doc, blanks);
    let local = parsed.reparse_local((old, new), (next.len(), usize::MAX), line_of_byte, slice);
    (next.clone(), local.then_some(parsed))
}

/// 断言改动按局部重解析完成（`local` 为 `Some(true)`）或改为解析全文（`Some(false)`），
/// `None` 时两者皆可；局部重解析的结果与全文解析相同。
fn check_with(doc: &str, old: Range<usize>, insert: &str, local: Option<bool>) {
    for blanks in [false, true] {
        let (next, parsed) = reparse(doc, old.clone(), insert, blanks);
        if let Some(local) = local {
            assert_eq!(parsed.is_some(), local, "{next:?}");
        }
        if let Some(parsed) = parsed {
            assert_eq!(parsed, full(&next, blanks), "{next:?}");
        }
    }
}

/// 同 [`check_with`]，断言是否按局部重解析完成。
fn check(doc: &str, old: Range<usize>, insert: &str, local: bool) {
    check_with(doc, old, insert, Some(local));
}

/// 返回 `doc` 中 `anchor` 之后的空范围（`after` 为真）或之前的空范围。
fn at(doc: &str, anchor: &str, after: bool) -> Range<usize> {
    let at = doc.find(anchor).unwrap() + if after { anchor.len() } else { 0 };
    at..at
}

#[test]
fn ordinary_edits_are_parsed_locally() {
    let doc = concat!(
        "---\n",
        "title: 示例文档\n",
        "---\n\n",
        "# 一级标题\n\n",
        "第一段正文，包含 **加粗** 与 $R_0$。\n\n",
        "第二段第一行  \n",
        "第二段第二行\n\n",
        "- 列表甲\n",
        "- 列表乙\n\n",
        "> 引用第一行\n",
        "> 引用第二行\n\n",
        "| 列一 | 列二 |\n",
        "|---|---|\n",
        "| 甲 | 乙 |\n\n",
        "尾段正文。\n\n\n",
        "最后一段。\n",
    );
    check(doc, at(doc, "包含", false), "新插入的中文 ", true);
    check(doc, at(doc, "。", true), "   ", true);
    // 段落中间回车，拆成两段；多回车几次，空行处插入空段落。
    check(doc, at(doc, "与 $R_0$", false), "\n\n", true);
    check(doc, at(doc, "与 $R_0$", false), "\n\n\n", true);
    // 在段落之间的空行上输入。
    let blank = doc.find("\n\n\n").unwrap() + 2;
    check(doc, blank..blank, "新段落", true);
    check(doc, at(doc, "列表甲", true), "补充\n- 列表插项", true);
    check(doc, at(doc, "引用第一行", true), " 续写", true);
    check(doc, at(doc, "| 甲", true), "  测试中", true);
    check(doc, at(doc, "最后一段", true), "新增尾部", true);
    let comma = doc.find("，包含").unwrap();
    check(doc, comma..comma + "，包含".len(), "", true);
}

#[test]
fn edits_that_reach_outside_the_window_are_parsed_in_full() {
    let doc = concat!(
        "---\n",
        "title: 示例\n",
        "---\n\n",
        "# 标题一\n\n",
        "正文段落内容。\n\n",
        "[TOC]\n\n",
        "## 标题二\n\n",
        "带脚注引用[^km]的段落。\n\n",
        "另一段普通正文。\n\n",
        "[^km]: 脚注定义。\n",
    );
    check(doc, at(doc, "正文段落内容", true), " 局部编辑", true);
    // 有目录时改标题：目录随之改变。
    check(doc, at(doc, "标题一", true), "改名", false);
    check(doc, at(doc, "示例", false), "新", false);
    check(doc, at(doc, "正文段落内容", false), "```rust\n", false);
    check(doc, at(doc, "正文段落内容", false), "[^new]", false);

    // 脚注定义在文首时，拆分后面的段落不平移脚注的行号。
    let doc = concat!(
        "[^top]: 文首脚注定义。\n\n",
        "# 标题\n\n",
        "中间段落正文。\n\n",
        "末尾段落带引用[^top]。\n",
    );
    check(doc, at(doc, "正文", false), "\n\n拆出新段", true);
}

/// 列表能跨空行延续：改动使窗口与相邻的列表接续时，结果仍与全文解析相同（窗口包含该列表，
/// 或改为解析全文）。
#[test]
fn lists_that_continue_across_blank_lines_stay_whole() {
    // 后一块是列表：段落改成列表项后与它合成一个列表。
    let doc = "start\n\nparagraph\n\n- b\n";
    check(doc, at(doc, "paragraph", false), "- ", false);
    // 段落改成列表项后，后一块的缩进成为列表项的内容。
    let doc = "start\n\npara\n\n  cont\n";
    check(doc, at(doc, "para", false), "- ", false);
    // 前一块是列表：段落改成列表项或缩进后接在列表之后。
    let doc = "intro\n\n- a\n\nparagraph\n\nend\n";
    check_with(doc, at(doc, "paragraph", false), "- ", None);
    check_with(doc, at(doc, "paragraph", false), "  ", None);
    // 缩进的段落改成列表项：列表的范围包含缩进。
    let doc = "    code\n\n  continued\n\nend\n";
    check_with(doc, at(doc, "continued", false), "1. ", None);
    // 与列表之间不会接续的改动仍按局部重解析。
    let doc = "start\n\nparagraph\n\n- b\n";
    check(doc, at(doc, "graph", false), "x", true);
}

#[test]
fn blank_lines_at_the_end_stay_in_step() {
    // 在文末空行上输入：原来的空段落属于窗口，不再留在叶子块中。
    let doc = "> quote\n\n\n";
    check(doc, doc.len()..doc.len(), "x", true);
    let doc = "para two\nline\n\n\n";
    check(doc, at(doc, " two", false), "#", true);
}

#[test]
fn a_window_over_the_limit_is_parsed_in_full() {
    let doc = "first\n\nsecond paragraph\n\nthird\n";
    let old = at(doc, "paragraph", false);
    let next = format!("{}x{}", &doc[..old.start], &doc[old.end..]);
    let line_of_byte = |b: usize| memchr::memchr_iter(b'\n', &next.as_bytes()[..b]).count();
    let slice = |r: Range<usize>| next[r].to_owned();
    let window = "second xparagraph\n\n".len();
    for (limit, local) in [(window - 1, false), (window, true)] {
        let mut parsed = parse(doc);
        let new = old.start..old.start + 1;
        let edit = (old.clone(), new);
        assert_eq!(
            parsed.reparse_local(edit, (next.len(), limit), line_of_byte, slice),
            local,
            "{limit}"
        );
    }
}

/// 随机生成的文档和改动：局部重解析的结果都与全文解析相同。
#[test]
fn random_edits_match_a_full_parse() {
    let pieces = [
        "para one",
        "para two\nline",
        "- item",
        "  continued",
        "1. first",
        "- [ ] task",
        "> quote",
        "| a | b |\n|---|---|\n| 1 | 2 |",
        "# head",
        "    indented",
        "text *em*",
        "***",
        "",
    ];
    let inserts = [
        "x", " ", "\n", "\n\n", "- ", "  ", "    ", "1. ", "> ", "#", "*", "|", "\n- y", "===",
        "\n---",
    ];
    // 固定种子的 xorshift，每次运行生成同样的用例。
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut local = 0;
    for _ in 0..4000 {
        let mut doc = String::new();
        for i in 0..2 + next(6) {
            if i > 0 {
                doc.push_str(if next(4) == 0 { "\n" } else { "\n\n" });
            }
            doc.push_str(pieces[next(pieces.len())]);
        }
        doc.push_str(["\n", "\n\n\n"][next(2)]);
        let starts: Vec<usize> = (doc.char_indices().map(|(i, _)| i))
            .chain([doc.len()])
            .collect();
        let start = starts[next(starts.len())];
        let removed = match next(5) {
            0 => doc[start..].chars().next().map_or(0, char::len_utf8),
            _ => 0,
        };
        let insert = if removed > 0 && next(2) == 0 {
            ""
        } else {
            inserts[next(inserts.len())]
        };
        let blanks = next(2) == 0;
        let (changed, parsed) = reparse(&doc, start..start + removed, insert, blanks);
        if let Some(parsed) = parsed {
            local += 1;
            assert_eq!(parsed, full(&changed, blanks), "{doc:?} → {changed:?}");
        }
    }
    // 大部分改动仍按局部重解析完成。
    assert!(local > 2000, "{local}");
}
