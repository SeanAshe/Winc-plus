//! `diff` 模块的测试：逐行差分的应用、位置映射、行改动报告与按块套用。

use super::*;
use crate::text::EditBuffer;
use ropey::Rope;

fn map_position(changes: &[Change], pos: usize) -> usize {
    map_positions(changes, &[pos])[0]
}

fn apply(old: &str, changes: &[Change]) -> String {
    let mut rope = Rope::from_str(old);
    for change in changes {
        rope.replace(
            change.start,
            change.removed.chars().count(),
            &change.inserted,
        );
    }
    rope.to_string()
}

/// xorshift 伪随机数发生器，固定种子使测试结果可复现。
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

#[test]
fn changes_cover_only_the_lines_that_differ() {
    assert!(line_changes("a\nb\n", "a\nb\n").is_empty());

    let changes = line_changes("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
    assert_eq!(
        changes,
        [
            Change {
                start: 8,
                removed: String::new(),
                inserted: "e\n".into(),
            },
            Change {
                start: 2,
                removed: "b\n".into(),
                inserted: "B\n".into(),
            },
        ]
    );
    assert_eq!(apply("a\nb\nc\nd\n", &changes), "a\nB\nc\nd\ne\n");

    // 覆盖的边界：只多出末尾换行、全部删除、从空文本开始、中文按字符计位置、CRLF、单独的 CR。
    for (old, new) in [
        ("a\nb", "a\nb\n"),
        ("x\ny\n", ""),
        ("", "fn main() {}\n"),
        ("中文\n一\n二\n", "中文\n一\n三\n"),
        ("a\r\nb\r\n", "a\r\nc\r\n"),
        ("a\rb\rc\r", "a\rB\rc\r"),
    ] {
        let changes = line_changes(old, new);
        assert_eq!(apply(old, &changes), new, "{old:?} → {new:?}");
    }
    let changes = line_changes("中文\n一\n二\n", "中文\n一\n三\n");
    assert_eq!(changes[0].start, 5);
    // 单独的 `\r` 也算换行，只替换中间一行。
    assert_eq!(line_changes("a\rb\rc\r", "a\rB\rc\r")[0].removed, "b\r");
}

#[test]
fn random_texts_round_trip() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let words = ["fn", "x", "    ", "}", "", "let a = 1;", "\t", "中"];
    let text = |rng: &mut Rng| -> String {
        (0..rng.below(30))
            .map(|_| format!("{}\n", words[rng.below(words.len())]))
            .collect()
    };
    for _ in 0..500 {
        let (old, new) = (text(&mut rng), text(&mut rng));
        let changes = line_changes(&old, &new);
        assert_eq!(apply(&old, &changes), new, "{old:?} → {new:?}");
        // 修改按从后往前的顺序排列，前后两处互不重叠。
        for pair in changes.windows(2) {
            assert!(pair[1].start + pair[1].removed.chars().count() <= pair[0].start);
        }
    }
}

#[test]
fn too_many_differences_become_one_change() {
    let old: String = (0..1500).map(|i| format!("old {i}\n")).collect();
    let new: String = (0..1500).map(|i| format!("new {i}\n")).collect();
    let changes = line_changes(&old, &new);
    assert_eq!(changes.len(), 1);
    assert_eq!(apply(&old, &changes), new);
}

#[test]
fn positions_follow_the_text_they_were_on() {
    let old = "fn a() {\n  let x = 1;\n  call(x)\n}\nfn b() {}\n";
    let new = "fn a() {\n    let x = 1;\n    call(x);\n}\nfn b() {}\n";
    let changes = line_changes(old, new);
    let at = |text: &str, needle: &str| text.find(needle).unwrap();
    let map = |pos| map_position(&changes, pos);

    // 没改的行原样平移。
    assert_eq!(map(0), 0);
    assert_eq!(map(at(old, "fn b")), at(new, "fn b"));
    // 缩进变化后，光标仍在同一个字符之前；位于行首缩进内的光标移到该行首个字符之前。
    assert_eq!(map(at(old, "x = 1")), at(new, "x = 1"));
    assert_eq!(map(at(old, "  let")), at(new, "let"));
    // 行尾位置跟随该行最后一个字符，不会移到下一行。
    assert_eq!(map(at(old, "\n}")), at(new, ";\n}"));
    // 前面某行增加了一个字符（分号）后，后面各行的位置按增加量平移。
    assert_eq!(map(at(old, "}\nfn b")), at(new, "}\nfn b"));
}

#[test]
fn split_and_joined_lines_keep_the_word() {
    let old = "call(first, second, third)\n";
    let new = "call(\n    first,\n    second,\n    third,\n)\n";
    let changes = line_changes(old, new);
    let pos = old.find("second").unwrap() + 2;
    let mapped = map_position(&changes, pos);
    assert_eq!(new.chars().skip(mapped).take(4).collect::<String>(), "cond");

    let back = line_changes(new, old);
    let pos = new.find("third").unwrap();
    assert_eq!(map_position(&back, pos), old.find("third").unwrap());
}

#[test]
fn positions_mapped_together_match_one_by_one() {
    let old = "a\n  b(x)\nc\n  d\ne\n";
    let new = "a\n    b(x);\nc\n    d;\nE\n";
    let changes = line_changes(old, new);
    let positions: Vec<usize> = (0..=old.len()).rev().collect();
    let together = map_positions(&changes, &positions);
    for (&pos, mapped) in positions.iter().zip(together) {
        assert_eq!(mapped, map_positions(&changes, &[pos])[0], "{pos}");
    }
}

#[test]
fn blank_lines_and_indentation_keep_the_caret_on_its_line() {
    let old = "a {\n  x\n  \n  b\n}\n";
    let new = "a {\n    x\n\n    b\n}\n";
    let changes = line_changes(old, new);
    assert_eq!(changes.len(), 1);
    // 只有空白的行变成空行，光标留在该行，不移到下一行的字符之前。
    assert_eq!(map_position(&changes, 9), new.find("\n\n").unwrap() + 1);
    // 位于缩进内的光标停在该行首个字符之前。
    assert_eq!(map_position(&changes, 12), new.find('b').unwrap());
}

#[test]
fn reindenting_everything_keeps_the_caret_on_its_word() {
    // 每一行都重新缩进，整篇合并为一段改动（超过 `MAX_EDITS`，不再逐行细分），其中少数行多了
    // 分号。去掉空白后按内容重新对应，光标不会因前面累计增加的分号而逐渐偏离。
    let old: String = (0..600).map(|i| format!("  let v{i} = {i}\n")).collect();
    let new: String = (0..600)
        .map(|i| {
            let semi = if i % 50 == 0 { ";" } else { "" };
            format!("    let v{i} = {i}{semi}\n")
        })
        .collect();
    let changes = line_changes(&old, &new);
    assert_eq!(changes.len(), 1);
    let pos = old.find("v450 =").unwrap();
    assert_eq!(map_position(&changes, pos), new.find("v450 =").unwrap());
}

#[test]
fn line_hunks_report_replacements_insertions_and_deletions() {
    let old = "a\nb\nc\nd\ne\n";
    let new = "a\nB\nc\nd\nx\ny\ne\n";
    assert_eq!(
        line_hunks(old, new),
        [
            LineHunk {
                old: 1..2,
                new: 1..2
            },
            LineHunk {
                old: 4..4,
                new: 4..6
            },
        ]
    );
    assert_eq!(
        line_hunks("a\nb\n", "a\n"),
        [LineHunk {
            old: 1..2,
            new: 1..1
        }]
    );
    assert!(line_hunks("same\n", "same\n").is_empty());
    // 只差换行符也算一处改动。
    assert_eq!(line_hunks("a\r\n", "a\n").len(), 1);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn inline_changes_mark_whole_words() {
    let (old, new) = inline_changes("let total = count + 1;", "let sum = count + 2;");
    assert_eq!(old, [4..9, 20..21]);
    assert_eq!(new, [4..7, 18..19]);
    // 汉字按字符计位置，连续的汉字算一个词。
    let (old, new) = inline_changes("打开文件", "打开目录");
    assert_eq!((old, new), (vec![0..4], vec![0..4]));
    let (old, new) = inline_changes("a(b)", "a(b, c)");
    assert_eq!(old, Vec::<Range<usize>>::new());
    assert_eq!(new, [3..6]);
    let (old, new) = inline_changes("", "new");
    assert_eq!((old, new), (vec![], vec![0..3]));
}

#[test]
fn pick_changes_applies_only_the_chosen_changes() {
    let old = "a\nb\nc\nd\ne\n";
    let new = "a\nB\nc\nd\nx\ne\n";
    let (a, b) = (lines(old), lines(new));
    let hunks = line_hunks(old, new);
    let key = |h: &LineHunk| {
        hunk_key(
            a[h.old.clone()].iter().copied(),
            b[h.new.clone()].iter().copied(),
        )
    };
    let first = key(&hunks[0]);
    let only = |keep: bool| if keep { Pick::Apply } else { Pick::Skip };
    assert_eq!(
        pick_changes(old, new, |k, _| only(k == first)),
        "a\nB\nc\nd\ne\n"
    );
    assert_eq!(
        pick_changes(old, new, |k, _| only(k != first)),
        "a\nb\nc\nd\nx\ne\n"
    );
    assert_eq!(pick_changes(old, new, |_, _| Pick::Apply), new);
    assert_eq!(pick_changes(old, new, |_, _| Pick::Skip), old);
    // 按旧文本的行区间认：第二段在旧文本里是插在第 4 行的位置。
    assert_eq!(
        pick_changes(old, new, |_, o| only(o != (4..4))),
        "a\nB\nc\nd\ne\n"
    );
    // 插入与删除互换后是不同的改动，键也不同。
    assert_ne!(hunk_key(["a\n"], []), hunk_key([], ["a\n"]));
}

/// 返回只跳过 `old`、`new` 所列行的 [`Pick`]。
fn skipping(old: &[usize], new: &[usize]) -> Pick {
    Pick::Lines(LineSet {
        old: old.iter().copied().collect(),
        new: new.iter().copied().collect(),
    })
}

#[test]
fn pick_changes_applies_single_lines_of_a_change() {
    let old = "a\nb\nc\nz\n";
    let new = "a\nB\nC\nD\nz\n";
    let apply = |old_lines: &[usize], new_lines: &[usize]| {
        pick_changes(old, new, |_, _| skipping(old_lines, new_lines))
    };
    // 不跳过任何行等于应用整段；跳过全部行等于不应用。
    assert_eq!(apply(&[], &[]), new);
    assert_eq!(apply(&[0, 1], &[0, 1, 2]), old);
    // 只加入部分新行，旧行全部删除。
    assert_eq!(apply(&[], &[1]), "a\nB\nD\nz\n");
    // 保留的旧行排在加入的新行之前。
    assert_eq!(apply(&[1], &[1, 2]), "a\nc\nB\nz\n");
}

#[test]
fn pick_changes_keeps_kept_lines_apart_at_the_end_of_the_file() {
    // 旧文本末尾没有换行符，保留它之后再接新行时需要补上。
    assert_eq!(
        pick_changes("a\nb", "a\nc\n", |_, _| skipping(&[0], &[])),
        "a\nb\nc\n"
    );
    assert_eq!(
        pick_changes("a\r\nb", "a\r\nc\r\n", |_, _| skipping(&[0], &[])),
        "a\r\nb\r\nc\r\n"
    );
    // 新文本末尾没有换行符，未加入最后一行时前面的行保持原样。
    assert_eq!(
        pick_changes("a\n", "a\nb\nc", |_, _| skipping(&[], &[1])),
        "a\nb\n"
    );
}

#[test]
fn staged_picks_tell_which_parts_of_each_change_are_staged() {
    let old = "a\nb\nc\nd\n";
    let new = "a\nB\nc\nD1\nD2\ne\n";
    let picks = |staged: &str| -> Vec<Pick> {
        staged_picks(old, new, staged)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    };
    let rebuild = |staged: &str| {
        let picks = staged_picks(old, new, staged);
        pick_changes(old, new, |key, _| {
            (picks.iter())
                .find(|(k, _)| *k == key)
                .map_or(Pick::Apply, |(_, p)| p.clone())
        })
    };
    // 只暂存了第一处：第一处整体应用，第二处整体不应用，应用结果等于暂存区。
    let first = "a\nB\nc\nd\n";
    assert_eq!(picks(first), [Pick::Apply, Pick::Skip]);
    assert_eq!(rebuild(first), first);
    // 第二处只暂存了一行新行：按行记录未加入的新行和保留的旧行。
    let one_line = "a\nb\nc\nD1\n";
    let expected = LineSet {
        old: BTreeSet::new(),
        new: [1, 2].into_iter().collect(),
    };
    assert_eq!(picks(one_line), [Pick::Skip, Pick::Lines(expected)]);
    assert_eq!(rebuild(one_line), one_line);
    // 全部已暂存：每处都应用；各处的键与差异视图中的键相同。
    assert_eq!(picks(new), [Pick::Apply, Pick::Apply]);
    let keys: Vec<u64> = staged_picks(old, new, new)
        .iter()
        .map(|(k, _)| *k)
        .collect();
    let (a, b) = (lines(old), lines(new));
    let expected: Vec<u64> = hunks(&a, &b)
        .into_iter()
        .map(|(o, n)| hunk_key(a[o].iter().copied(), b[n].iter().copied()))
        .collect();
    assert_eq!(keys, expected);
}
