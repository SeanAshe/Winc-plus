//! `edit` 模块的测试：换行的缩进规则、缩进与注释切换、行的移动与删除、粘贴的缩进调整，
//! 以及撤销栈的合并与裁剪。

use ropey::Rope;

use super::*;

/// 测试用的编辑参数：4 空格缩进、LF 换行、`#` 行注释、单引号界定字符串。
const OPTIONS: EditOptions<'static> = EditOptions {
    tab_width: 4,
    indent_unit: "    ",
    line_ending: "\n",
    indent_after: &[':', '{'],
    tag_indent: false,
    line_comment: Some("# "),
    single_quote_string: true,
};

fn run(
    text: &str,
    selection: Selection,
    f: impl Fn(&Rope, Selection) -> Option<Transaction>,
) -> (String, Selection, String) {
    let original = Rope::from_str(text);
    let mut rope = original.clone();
    let transaction = f(&rope, selection).expect("transaction");
    transaction.apply(&mut rope);
    let after = rope.to_string();

    let mut history = History::default();
    history.push(transaction.clone());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), text, "撤销后应恢复原文");

    (after, transaction.after, original.to_string())
}

#[test]
fn newline_indents_after_colon() {
    let (out, sel, _) = run("    if x:", Selection::point(9), |t, s| {
        Some(newline(t, s, &OPTIONS))
    });
    assert_eq!(out, "    if x:\n        ");
    assert_eq!(sel, Selection::point(18));
}

#[test]
fn newline_inside_unclosed_brackets_indents() {
    let newline_at_end = |text: &str| {
        let end = text.chars().count();
        run(text, Selection::point(end), |t, s| {
            Some(newline(t, s, &OPTIONS))
        })
        .0
    };
    assert_eq!(newline_at_end("    foo(a,"), "    foo(a,\n        ");
    assert_eq!(newline_at_end("x = [1,"), "x = [1,\n    ");
    // 已闭合的括号和引号内的括号不计入；冒号与未闭合括号同时出现时只多缩进一级。
    assert_eq!(newline_at_end("    foo(a)"), "    foo(a)\n    ");
    assert_eq!(newline_at_end("print(\"(\")"), "print(\"(\")\n");
    assert_eq!(newline_at_end("print('(')"), "print('(')\n");
    assert_eq!(
        newline_at_end("x = 1  # (see below"),
        "x = 1  # (see below\n"
    );
    assert_eq!(newline_at_end("if (a:"), "if (a:\n    ");
    // 没有需要多缩进的字符（纯文本）：括号不参与判断。
    let prose = EditOptions {
        indent_after: &[],
        ..OPTIONS
    };
    let (out, _, _) = run("note (draft", Selection::point(11), |t, s| {
        Some(newline(t, s, &prose))
    });
    assert_eq!(out, "note (draft\n");
}

#[test]
fn newline_indents_only_after_opening_tags() {
    let html = EditOptions {
        indent_unit: "  ",
        indent_after: &['{'],
        tag_indent: true,
        line_comment: None,
        ..OPTIONS
    };
    let newline_at = |line: &str, pos: usize| {
        run(line, Selection::point(pos), |t, s| {
            Some(newline(t, s, &html))
        })
        .0
    };
    let at_end = |line: &str| newline_at(line, line.chars().count());
    assert_eq!(at_end("<div class=\"a\">"), "<div class=\"a\">\n  ");
    assert_eq!(at_end("  <ul> "), "  <ul> \n    ");
    assert_eq!(at_end("<svg:g id='x'>"), "<svg:g id='x'>\n  ");
    // 已闭合、结束、自闭合的标签，空元素、注释和声明之后沿用原缩进。
    for line in [
        "  <p>文字</p>",
        "</div>",
        "<br/>",
        "<img src=\"a.png\">",
        "<BR>",
        "<!DOCTYPE html>",
        "<!-- 注释 -->",
        "<?xml version=\"1.0\"?>",
        "a < b >",
    ] {
        let indent = &line[..line.len() - line.trim_start().len()];
        assert_eq!(at_end(line), format!("{line}\n{indent}"), "{line}");
    }
    // 结束标签就在光标之后：只断开，不增加缩进。
    assert_eq!(newline_at("<p></p>", 3), "<p>\n</p>");
}

#[test]
fn newline_between_braces() {
    let (out, sel, _) = run("fn f() {}", Selection::point(8), |t, s| {
        Some(newline(t, s, &OPTIONS))
    });
    assert_eq!(out, "fn f() {\n    \n}");
    assert_eq!(sel, Selection::point(13));

    // 选中括号之间的文本后按回车：文本被替换，右括号仍移到下一行。
    let (out, _, _) = run("{abc}", Selection::new(1, 4), |t, s| {
        Some(newline(t, s, &OPTIONS))
    });
    assert_eq!(out, "{\n    \n}");
}

#[test]
fn outdent_in_a_tab_indented_file_removes_a_tab_worth_of_spaces() {
    let tabs = EditOptions {
        indent_unit: "\t",
        ..OPTIONS
    };
    let (out, _, _) = run("\tx\n    y\n  z", Selection::new(0, 12), |t, s| {
        outdent(t, s, &tabs)
    });
    assert_eq!(out, "x\ny\nz");
}

#[test]
fn move_lines_swaps_with_the_neighbour_and_keeps_line_endings() {
    let mut text = Rope::from_str("a\r\nbb\r\nccc\r\n");
    // 光标在 `bb` 的第二个字符上，向上移动：与 `a` 对调，光标随之移动。
    let sel = Selection::point(4);
    let tx = move_lines(&text, sel, false).unwrap();
    tx.apply(&mut text);
    assert_eq!(text.to_string(), "bb\r\na\r\nccc\r\n");
    assert_eq!(tx.after, Selection::point(1));
    // 已在首行，无法继续向上移动。
    assert!(move_lines(&text, tx.after, false).is_none());

    // 选中两行向下移动。
    let mut text = Rope::from_str("1\n2\n3\n4");
    let sel = Selection::new(0, 3);
    let tx = move_lines(&text, sel, true).unwrap();
    tx.apply(&mut text);
    assert_eq!(text.to_string(), "3\n1\n2\n4");
    assert_eq!(tx.after, Selection::new(2, 5));
    // 已在末行，无法继续向下移动。
    let last = Selection::point(text.len_chars());
    assert!(move_lines(&text, last, true).is_none());

    // 选中一整行（到下一行行首）、下一行是最后一行：选区不越过文本末尾。
    let mut text = Rope::from_str("a\nb\n");
    let tx = move_lines(&text, Selection::new(2, 4), true).unwrap();
    tx.apply(&mut text);
    assert_eq!(text.to_string(), "a\n\nb");
    assert_eq!(tx.after, Selection::new(3, 4));
}

#[test]
fn copy_lines_up_and_down() {
    let opts = OPTIONS;
    let mut text = Rope::from_str("ab\ncd\n");
    let sel = Selection::new(0, 1);
    let tx = copy_lines(&text, sel, true, &opts);
    tx.apply(&mut text);
    assert_eq!(text.to_string(), "ab\nab\ncd\n");
    assert_eq!(tx.after, Selection::new(3, 4));

    let mut text = Rope::from_str("ab\ncd\n");
    let sel = Selection::point(4);
    let tx = copy_lines(&text, sel, false, &opts);
    tx.apply(&mut text);
    assert_eq!(text.to_string(), "ab\ncd\ncd\n");
    assert_eq!(tx.after, sel);
}

#[test]
fn smart_backspace_in_indent() {
    let (out, _, _) = run("        x", Selection::point(8), |t, s| {
        backspace(t, s, false, &OPTIONS)
    });
    assert_eq!(out, "    x");
    let (out, _, _) = run("      x", Selection::point(6), |t, s| {
        backspace(t, s, false, &OPTIONS)
    });
    assert_eq!(out, "    x");
}

#[test]
fn indent_and_outdent_block() {
    let (out, sel, _) = run("a\n\nb\nc", Selection::new(0, 5), |t, s| {
        Some(indent(t, s, &OPTIONS))
    });
    assert_eq!(out, "    a\n\n    b\nc");
    assert_eq!(sel, Selection::new(0, 13));

    let (out, _, _) = run("    a\n  b\nc", Selection::new(0, 9), |t, s| {
        outdent(t, s, &OPTIONS)
    });
    assert_eq!(out, "a\nb\nc");
}

#[test]
fn outdent_keeps_the_last_line_when_the_selection_ends_in_its_indent() {
    // 选区结尾在末行缩进的第 2 列：去掉一级缩进后停在剩余缩进末尾，末行仍在选区内，
    // 再按一次也会减少该行的缩进。
    let text = "    a\n        b";
    let (out, sel, _) = run(text, Selection::new(0, 8), |t, s| outdent(t, s, &OPTIONS));
    assert_eq!(out, "a\n    b");
    assert_eq!(sel, Selection::new(0, 6));
    assert_eq!(selected_lines(&Rope::from_str(&out), sel), (0, 1));

    // 光标一端在前（从下往上选）时同样处理。
    let (_, sel, _) = run(text, Selection::new(8, 0), |t, s| outdent(t, s, &OPTIONS));
    assert_eq!(sel, Selection::new(6, 0));

    // 末行的缩进全部去掉后已无可退，结尾回到行首。
    let (out, sel, _) = run("    a\n    b", Selection::new(0, 8), |t, s| {
        outdent(t, s, &OPTIONS)
    });
    assert_eq!(out, "a\nb");
    assert_eq!(sel, Selection::new(0, 2));
}

#[test]
fn comment_toggle_round_trip() {
    let (out, _, _) = run("  a\n    b", Selection::new(0, 9), |t, s| {
        toggle_comment(t, s, &OPTIONS)
    });
    assert_eq!(out, "  # a\n  #   b");

    let (out, _, _) = run("  # a\n  #   b", Selection::new(0, 13), |t, s| {
        toggle_comment(t, s, &OPTIONS)
    });
    assert_eq!(out, "  a\n    b");
}

#[test]
fn delete_lines_and_to_line_start() {
    let (out, sel, _) = run("ab\ncd\nef", Selection::point(4), delete_lines);
    assert_eq!(out, "ab\nef");
    assert_eq!(sel, Selection::point(4));

    let (out, _, _) = run("ab\ncd", Selection::point(4), delete_lines);
    assert_eq!(out, "ab");

    // `⌘⌫`：删到行首；已在行首时与上一行相接；有选区时删除选区。
    let (out, sel, _) = run("ab\n  cd", Selection::point(6), delete_to_line_start);
    assert_eq!((out.as_str(), sel), ("ab\nd", Selection::point(3)));
    let (out, sel, _) = run("ab\ncd", Selection::point(3), delete_to_line_start);
    assert_eq!((out.as_str(), sel), ("abcd", Selection::point(2)));
    let (out, _, _) = run("ab\ncd", Selection::new(1, 4), delete_to_line_start);
    assert_eq!(out, "ad");
}

#[test]
fn indent_lines_ignores_where_the_caret_is() {
    let (out, sel, _) = run("ab\ncd", Selection::point(1), |t, s| {
        Some(indent_lines(t, s, &OPTIONS))
    });
    assert_eq!((out.as_str(), sel), ("    ab\ncd", Selection::point(5)));
    // 同样的光标位置，Tab 键只在光标处插入缩进。
    let (out, _, _) = run("ab\ncd", Selection::point(1), |t, s| {
        Some(indent(t, s, &OPTIONS))
    });
    assert_eq!(out, "a   b\ncd");
}

#[test]
fn pasted_blocks_move_to_the_caret_indent() {
    let paste_at = |text: &str, pos: usize, pasted: &str| {
        run(text, Selection::point(pos), |t, s| {
            Some(paste(t, s, pasted))
        })
        .0
    };
    let block = "    if x:\n        y()\n\n    z()";

    // 光标在 8 个空格的缩进上：整块移到 8 个空格，相对缩进不变，空行不留空白。
    assert_eq!(
        paste_at("def f():\n        ", 17, block),
        "def f():\n        if x:\n            y()\n\n        z()"
    );
    // 光标在行首（空行）：原样粘贴。
    assert_eq!(paste_at("", 0, block), block);
    // CRLF 换行的内容同样能识别空行。
    assert_eq!(
        paste_at("  ", 2, "    a\r\n\r\n      b"),
        "  a\r\n\r\n    b"
    );
    // 只用 CR 换行（经典 Mac 的行尾风格）的内容同样按行调整。
    assert_eq!(paste_at("  ", 2, "    a\r\r      b\r"), "  a\r\r    b\r");
    // 以换行符结尾的单行同样调整。
    assert_eq!(paste_at("  ", 2, "    a\n"), "  a\n");
    // Tab 缩进：按字符比较，不换算成空格。
    assert_eq!(paste_at("\t", 1, "\t\ta\n\t\t\tb"), "\ta\n\t\tb");

    // 缩进本来就相同：不重复增加第一行的缩进。
    assert_eq!(paste_at("    ", 4, "    a\n    b"), "    a\n    b");
    // 不调整：第一行不带缩进（从行中间开始复制）、光标之前有非空白字符、只有一行。
    assert_eq!(
        paste_at("    ", 4, "if x:\n        y()"),
        "    if x:\n        y()"
    );
    assert_eq!(paste_at("ab ", 3, "    a\n    b"), "ab     a\n    b");
    assert_eq!(paste_at("  ", 2, "    a"), "      a");
}

#[test]
fn paste_lines_above() {
    let (out, sel, _) = run("ab\ncd", Selection::point(4), |t, s| {
        Some(insert_lines_above(t, s, "xy\n"))
    });
    assert_eq!(out, "ab\nxy\ncd");
    assert_eq!(sel, Selection::point(7));
}

#[test]
fn typing_merges_and_seals_after_save() {
    let mut rope = Rope::new();
    let mut history = History::default();

    for (i, c) in "abc".chars().enumerate() {
        let t = replace_selection(&rope, Selection::point(i), &c.to_string(), EditKind::Typing);
        t.apply(&mut rope);
        history.push(t);
    }

    assert!(history.is_dirty());
    // 连续键入的几个字符合并为一处修改。
    assert_eq!(history.undo.len(), 1);
    assert_eq!(history.undo[0].changes.len(), 1);
    assert_eq!(history.undo[0].changes[0].inserted, "abc");
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "");
    assert!(!history.is_dirty());

    history.redo(&mut rope).unwrap();
    history.mark_saved();
    assert!(!history.is_dirty());

    let t = replace_selection(&rope, Selection::point(3), "d", EditKind::Typing);
    t.apply(&mut rope);
    history.push(t);
    assert!(history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "abc");
    assert!(!history.is_dirty());
}

#[test]
fn a_sealed_version_can_be_marked_saved_later() {
    let mut rope = Rope::new();
    let mut history = History::default();
    let typed = |history: &mut History, rope: &mut Rope, at: usize, text: &str| {
        let t = replace_selection(rope, Selection::point(at), text, EditKind::Typing);
        t.apply(rope);
        history.push(t);
    };

    // 后台保存：发起时封口记下版本，之后键入的字符另起一个撤销单元；写完把那个版本记为已保存。
    typed(&mut history, &mut rope, 0, "ab");
    let version = history.seal();
    typed(&mut history, &mut rope, 2, "c");
    history.mark_saved_version(version);
    assert!(history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "ab");
    assert!(!history.is_dirty());
}

#[test]
fn undo_reverts_every_change_in_reverse_order() {
    let mut rope = Rope::from_str("ab");
    let mut history = History::default();
    let changes = vec![
        Change {
            start: 0,
            removed: "a".to_string(),
            inserted: "xyz".to_string(),
        },
        Change {
            start: 3,
            removed: "b".to_string(),
            inserted: String::new(),
        },
    ];
    let mut t = Transaction::new(
        changes,
        Selection::new(0, 2),
        Selection::point(3),
        EditKind::Other,
    );
    t.extra_before = vec![Selection::point(1)];
    t.apply(&mut rope);
    assert_eq!(rope.to_string(), "xyz");
    history.push(t);

    let restored = history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "ab");
    assert_eq!(restored.selection, Selection::new(0, 2));
    assert_eq!(restored.extra, [Selection::point(1)]);
    assert_eq!(restored.first, 0);
    history.redo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "xyz");
}

#[test]
fn trimming_the_oldest_edits_keeps_the_dirty_mark_right() {
    let mut rope = Rope::new();
    let mut history = History::default();
    let push = |history: &mut History, rope: &mut Rope, c: &str| {
        let at = rope.len_chars();
        let t = replace_selection(&*rope, Selection::point(at), c, EditKind::Other);
        t.apply(rope);
        history.push(t);
    };

    // 保存时的内容（`a`）随最早的事务一起丢弃：撤到底也回不去，一直算有修改。
    for c in ["a", "b", "c"] {
        push(&mut history, &mut rope, c);
        if c == "a" {
            history.mark_saved();
        }
    }
    let one = history.undo_bytes / 3;
    history.trim(one);
    assert_eq!((history.undo.len(), history.undo_bytes), (1, one));
    assert!(history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "ab");
    assert!(history.undo(&mut rope).is_none(), "最早的两次已丢弃");
    assert!(history.is_dirty());
    assert_eq!(history.undo_bytes, 0);

    // 保存时的内容恰是撤到底时的内容：撤到底后不算有修改。
    history.clear();
    push(&mut history, &mut rope, "d");
    history.mark_saved();
    push(&mut history, &mut rope, "e");
    history.trim(one);
    assert_eq!(history.undo.len(), 1);
    assert!(history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "abd");
    assert!(!history.is_dirty());
    history.redo(&mut rope).unwrap();
    assert!(history.is_dirty());
    assert_eq!(history.undo_bytes, one);

    // 栈顶的一个总会保留，即使栈顶事务本身就超过上限。
    history.trim(0);
    assert_eq!(history.undo.len(), 1);
}

#[test]
fn newline_and_paste_on_a_very_long_line_only_look_near_the_caret() {
    // 很长的一行：光标之前的内容逐个字符扫描，不整行复制，结果和短行一样。
    let long = format!("    foo({}", "a, ".repeat(100_000));
    let end = long.chars().count();
    let (out, _, _) = run(&long, Selection::point(end), |t, s| {
        Some(newline(t, s, &OPTIONS))
    });
    assert_eq!(out, format!("{long}\n        "));
    let tag = EditOptions {
        tag_indent: true,
        ..OPTIONS
    };
    let html = format!("<p>{}<div>", "x".repeat(100_000));
    let end = html.chars().count();
    let (out, _, _) = run(&html, Selection::point(end), |t, s| {
        Some(newline(t, s, &tag))
    });
    assert_eq!(out, format!("{html}\n    "));
    // 光标之前有非空白字符：多行代码原样粘贴。
    let (out, _, _) = run(&long, Selection::point(end.min(10)), |t, s| {
        Some(paste(t, s, "  a\n  b"))
    });
    assert!(out.starts_with("    foo(a,  a\n  b"), "{}", &out[..20]);
}

#[test]
fn undo_size_counts_each_small_allocation() {
    // 转换行尾：每行一处修改，删除一个 `\n`、插入 `\r\n`。按文本长度只算 3 字节，实际各占一块
    // 至少 16 字节的堆内存。
    let changes: Vec<Change> = (0..1000)
        .map(|i| Change {
            start: i * 2,
            removed: "\n".to_owned(),
            inserted: "\r\n".to_owned(),
        })
        .collect();
    assert_eq!(
        history::changes_size(&changes),
        1000 * (size_of::<Change>() + 16 + 16)
    );
}

/// 在 `rope` 的 `at` 处把 `removed` 个字符替换为 `text`，应用后入栈。
fn replaced(
    history: &mut History,
    rope: &mut Rope,
    at: usize,
    removed: usize,
    text: &str,
    kind: EditKind,
) {
    let t = replace_selection(&*rope, Selection::new(at, at + removed), text, kind);
    t.apply(rope);
    history.push(t);
}

#[test]
fn a_group_is_one_undo_unit_whatever_the_kinds_and_intervals() {
    let mut rope = Rope::from_str("one two three");
    let mut history = History::default();

    // 一次删除，之后在别处键入，再隔开超过合并间隔后换行：类别、位置、间隔都不满足键入的合并条件。
    history.begin_group();
    replaced(&mut history, &mut rope, 4, 4, "", EditKind::Other);
    replaced(&mut history, &mut rope, 0, 0, "x", EditKind::Typing);
    let mut late = replace_selection(&rope, Selection::point(1), "\n", EditKind::Other);
    late.time += history::MERGE_WINDOW * 3;
    late.apply(&mut rope);
    history.push(late);
    history.end_group();

    assert_eq!(rope.to_string(), "x\none three");
    assert_eq!(history.undo.len(), 1);
    assert_eq!(history.undo_bytes, history.undo[0].bytes);
    assert_eq!(
        history.undo[0].bytes,
        history::changes_size(&history.undo[0].changes)
    );

    // 撤销回到第一个事务之前的选区，重做回到最后一个事务之后的选区。
    let restored = history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "one two three");
    assert_eq!(restored.selection, Selection::new(4, 8));
    assert_eq!(restored.first, 0);
    assert!(!history.is_dirty());
    assert!(history.undo(&mut rope).is_none());
    let restored = history.redo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "x\none three");
    assert_eq!(restored.selection, Selection::point(2));
    assert_eq!(history.undo_bytes, history.undo[0].bytes);
}

#[test]
fn a_group_does_not_merge_with_typing_around_it() {
    let mut rope = Rope::new();
    let mut history = History::default();
    let typed = |history: &mut History, rope: &mut Rope, text: &str| {
        let at = rope.len_chars();
        replaced(history, rope, at, 0, text, EditKind::Typing);
    };

    // 分组前后的键入都与分组内的键入首尾相接，且在合并间隔之内。
    typed(&mut history, &mut rope, "a");
    history.begin_group();
    typed(&mut history, &mut rope, "b");
    typed(&mut history, &mut rope, "c");
    history.end_group();
    typed(&mut history, &mut rope, "d");
    typed(&mut history, &mut rope, "e");

    let units: Vec<&str> = (history.undo.iter())
        .map(|t| t.changes[0].inserted.as_str())
        .collect();
    assert_eq!(units, ["a", "bc", "de"]);
    // 分组内紧邻的插入记为一处修改。
    assert_eq!(history.undo[1].changes.len(), 1);

    history.undo(&mut rope).unwrap();
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "a");
}

#[test]
fn an_empty_group_leaves_typing_merged() {
    let mut rope = Rope::new();
    let mut history = History::default();
    replaced(&mut history, &mut rope, 0, 0, "a", EditKind::Typing);
    history.begin_group();
    history.end_group();
    // 多出的一次结束不影响之后的分组。
    history.end_group();
    replaced(&mut history, &mut rope, 1, 0, "b", EditKind::Typing);
    assert_eq!(history.undo.len(), 1);

    history.begin_group();
    replaced(&mut history, &mut rope, 0, 0, "x", EditKind::Other);
    replaced(&mut history, &mut rope, 3, 0, "y", EditKind::Other);
    history.end_group();
    assert_eq!(history.undo.len(), 2);
}

#[test]
fn nested_groups_end_with_the_outermost() {
    let mut rope = Rope::from_str("abc");
    let mut history = History::default();

    history.begin_group();
    replaced(&mut history, &mut rope, 0, 1, "", EditKind::Other);
    history.begin_group();
    replaced(&mut history, &mut rope, 1, 1, "", EditKind::Other);
    history.end_group();
    replaced(&mut history, &mut rope, 0, 0, "z", EditKind::Other);
    history.end_group();
    assert_eq!(rope.to_string(), "zb");
    assert_eq!(history.undo.len(), 1);

    replaced(&mut history, &mut rope, 2, 0, "!", EditKind::Other);
    assert_eq!(history.undo.len(), 2);
    history.undo(&mut rope).unwrap();
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "abc");
}

#[test]
fn sealing_inside_a_group_starts_a_new_unit() {
    let mut rope = Rope::new();
    let mut history = History::default();

    // 一次插入的中途发起保存：保存之前、之后的输入各成一个撤销单元，撤销一次正好回到写出的内容。
    history.begin_group();
    replaced(&mut history, &mut rope, 0, 0, "ab", EditKind::Typing);
    replaced(&mut history, &mut rope, 2, 0, "\n", EditKind::Other);
    let version = history.seal();
    replaced(&mut history, &mut rope, 3, 0, "c", EditKind::Typing);
    replaced(&mut history, &mut rope, 4, 0, "\n", EditKind::Other);
    history.end_group();
    history.mark_saved_version(version);

    assert_eq!(history.undo.len(), 2);
    assert!(history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "ab\n");
    assert!(!history.is_dirty());
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "");

    // 分组期间撤销之后入栈的事务同样另起一个单元。
    history.begin_group();
    replaced(&mut history, &mut rope, 0, 0, "x", EditKind::Other);
    history.undo(&mut rope).unwrap();
    replaced(&mut history, &mut rope, 0, 0, "y", EditKind::Other);
    replaced(&mut history, &mut rope, 1, 0, "z", EditKind::Other);
    history.end_group();
    assert_eq!(history.undo.len(), 1);
    assert_eq!(rope.to_string(), "yz");
}

#[test]
fn the_byte_limit_trims_whole_units_while_grouping() {
    let mut rope = Rope::new();
    let mut history = History::default();
    let append = |history: &mut History, rope: &mut Rope, c: &str| {
        let at = rope.len_chars();
        replaced(history, rope, at, 0, c, EditKind::Other);
    };

    // 三个分组，各含两处不相邻的插入（文末与文首）；占用按单元累计。
    for [last, first] in [["a", "b"], ["c", "d"], ["e", "f"]] {
        history.begin_group();
        append(&mut history, &mut rope, last);
        replaced(&mut history, &mut rope, 0, 0, first, EditKind::Other);
        history.end_group();
    }
    assert_eq!(rope.to_string(), "fdbace");
    assert_eq!(history.undo.len(), 3);
    let unit = history.undo[0].bytes;
    assert_eq!(history.undo_bytes, unit * 3);

    // 上限只容得下一个单元时丢弃较早的三个；分组期间栈顶的单元超过上限也保留。
    history.begin_group();
    append(&mut history, &mut rope, "g");
    history.trim(unit);
    assert_eq!(history.undo.len(), 1);
    append(&mut history, &mut rope, "h");
    history.trim(0);
    history.end_group();
    assert_eq!(history.undo.len(), 1);
    assert_eq!(history.undo_bytes, history.undo[0].bytes);
    history.undo(&mut rope).unwrap();
    assert_eq!(rope.to_string(), "fdbace");
    assert!(history.undo(&mut rope).is_none());
    assert_eq!(history.undo_bytes, 0);
}

#[test]
fn the_selection_before_the_next_redo_is_available() {
    let mut rope = Rope::from_str("abc");
    let mut history = History::default();
    assert_eq!(history.redo_origin(), None);
    let t = replace_selection(&rope, Selection::new(1, 2), "xy", EditKind::Other);
    t.apply(&mut rope);
    history.push(t);
    assert_eq!(history.redo_origin(), None);
    history.undo(&mut rope).unwrap();
    assert_eq!(history.redo_origin(), Some(Selection::new(1, 2)));
    history.redo(&mut rope).unwrap();
    assert_eq!(history.redo_origin(), None);
}
