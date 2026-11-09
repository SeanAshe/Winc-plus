//! `LargeText` 的测试：行索引与 `Rope` 逐行对照、随机改动的结果、分段扫描、末尾追加、
//! 整篇重写、超长行、非法 UTF-8 与截短。

use std::fmt::Write as _;
use std::path::PathBuf;

use ropey::Rope;

use super::*;

fn write_temp(name: &str, content: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("winc-large-{name}-{}", std::process::id()));
    std::fs::write(&path, content).unwrap();
    path
}

fn open(name: &str, content: &[u8]) -> (LargeText, PathBuf) {
    let path = write_temp(name, content);
    (LargeText::open(File::open(&path).unwrap()).unwrap(), path)
}

/// 长短行混排，既有按行数也有按字节数产生的检查点。
fn sample(lines: usize) -> String {
    let mut content = String::new();

    for i in 0..lines {
        let width = if i % 97 == 0 { 3000 } else { i % 40 };
        writeln!(content, "{i} 中文{}", "x".repeat(width)).unwrap();
    }

    content
}

fn assert_same(large: &LargeText, rope: &Rope, lines: impl Iterator<Item = usize>) {
    assert_eq!(large.end_pos(), rope.end_pos());
    assert_eq!(large.line_count(), rope.line_count());

    for line in lines {
        assert_eq!(
            large.line_start(line),
            rope.line_start(line),
            "第 {line} 行的行首"
        );
        assert_eq!(
            large.line_content(line).slice(),
            rope.line_content(line).slice(),
            "第 {line} 行的内容"
        );
        let pos = rope.line_col_to_pos(line, 3);
        assert_eq!(large.line_of(pos), rope.line_of(pos), "位置 {pos} 所在的行");
    }
}

fn whole(large: &LargeText) -> String {
    let mut out = Vec::new();
    large.write_to(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn matches_rope_line_by_line() {
    let content = sample(3000);
    let (large, path) = open("lines", content.as_bytes());
    let rope = Rope::from_str(&content);

    assert!(large.original.checkpoints.len() > 3000 / 64);
    assert!(large.is_utf8() && !large.has_overlong_lines());
    assert_same(&large, &rope, (0..rope.line_count()).step_by(7));
    assert_eq!(large.substring(10, 5000), rope.substring(10, 5000));
    assert_eq!(large.byte_len(0, large.end_pos()), content.len() as u64);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn segmented_index_matches_single_pass() {
    let content = sample(5000);
    let path = write_temp("segments", content.as_bytes());
    let single = Original::open(File::open(&path).unwrap(), 1).unwrap();
    let parallel = Original::open(File::open(&path).unwrap(), 7).unwrap();

    assert_eq!(
        (parallel.newlines, parallel.chars),
        (single.newlines, single.chars)
    );
    assert_eq!(parallel.longest_line, single.longest_line);

    for line in (0..single.newlines).step_by(3) {
        assert_eq!(
            parallel.locate_line(line),
            single.locate_line(line),
            "第 {line} 行"
        );
    }

    std::fs::remove_file(path).unwrap();
}

/// 简单的伪随机数发生器（xorshift），使测试结果可复现。
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
fn random_edits_match_rope() {
    let content = sample(2000);
    let (mut large, path) = open("edits", content.as_bytes());
    let mut rope = Rope::from_str(&content);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // 样本不含单独的 `\r`：`Rope` 把 `\r` 按换行处理，大文件只按 `\n` 分行。
    let snippets = ["a", "中", "\n", "xy\nz", "👍🏽\n\n", "line\n", ""];

    for round in 0..400 {
        let len = rope.end_pos();
        let start = rng.below(len + 1);
        let removed = if rng.below(3) == 0 {
            rng.below((len - start).min(200) + 1)
        } else {
            0
        };
        let inserted = snippets[rng.below(snippets.len())];

        large.replace(start, removed, inserted);
        rope.replace(start, removed, inserted);
        // 只重算改动之后的前缀和，结果与整个重建相同。
        let (chars, lines) = (large.chars_before.clone(), large.lines_before.clone());
        large.rebuild();
        assert_eq!(
            (chars, lines),
            (large.chars_before.clone(), large.lines_before.clone())
        );

        if round % 20 == 0 {
            let lines = rope.line_count();
            assert_same(&large, &rope, (0..8).map(|_| rng.below(lines)));

            let a = rng.below(rope.end_pos() + 1);
            let b = (a + rng.below(500)).min(rope.end_pos());
            assert_eq!(large.substring(a, b), rope.substring(a, b));
        }
    }

    assert_eq!(whole(&large), rope.to_string());
    assert!(large.pieces.len() < 900, "片段数 {}", large.pieces.len());

    let mut visited = Vec::new();
    large.visit_lines(0, usize::MAX, |l, s, t| {
        visited.push((l, s, t.to_owned()));
        ControlFlow::Continue(())
    });
    let expected: Vec<_> = (0..rope.line_count())
        .map(|l| {
            (
                l,
                rope.line_start(l),
                rope.line_content(l).slice().to_string(),
            )
        })
        .collect();
    assert_eq!(visited, expected);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn growing_files_extend_the_index() {
    use std::io::Write as _;

    // 原文件不以换行结尾，新内容接在最后一行之后。
    let mut expected = sample(3000).trim_end_matches('\n').to_owned();
    let (mut large, path) = open("grow", expected.as_bytes());
    let append = |text: &[u8]| {
        let mut file = File::options().append(true).open(&path).unwrap();
        file.write_all(text).unwrap();
    };

    // 再追加 500 行，最后一个字只写入前两个字节。
    let more = format!(" 接着写\n{}", sample(500));
    append(more.as_bytes());
    append(&"末".as_bytes()[..2]);
    let added = large.grow().unwrap();
    expected.push_str(&more);
    assert_eq!(added as usize, more.chars().count());
    let rope = Rope::from_str(&expected);
    assert_same(&large, &rope, (0..rope.line_count()).step_by(3));
    assert_eq!(whole(&large), expected);
    // 末尾有一个不完整的字符尚未接入，但已接入的部分没有变化，仍然可以写出。
    assert!(!large.original_changed());

    // 补全被截断的字「末」：接入索引。
    append(&"末".as_bytes()[2..]);
    assert_eq!(large.grow(), Some(1));
    expected.push('末');
    assert_eq!(whole(&large), expected);
    assert!(!large.original_changed());

    // 文件没有变长，或文档已被修改时，均不接入。
    assert_eq!(large.grow(), None);
    large.replace(0, 1, "x");
    append(b"more\n");
    assert_eq!(large.grow(), None);

    std::fs::remove_file(path).unwrap();

    // 文档内容被全部删除（片段表为空，也没有新增输入）同样视为已修改：
    // 已删除的内容不能随新增部分重新出现。
    let (mut large, path) = open("grow-cleared", sample(100).as_bytes());
    large.replace(0, large.end_pos(), "");
    let mut file = File::options().append(true).open(&path).unwrap();
    file.write_all(b"again\n").unwrap();
    assert_eq!(large.grow(), None);
    assert_eq!(large.end_pos(), 0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rewritten_files_are_not_taken_for_appends() {
    use std::io::Write as _;

    // 整个文件被重写一遍（如 shell 重定向 `> log` 或 `cp` 覆盖）且比原来长。
    let (mut large, path) = open("rewrite", sample(2000).as_bytes());
    let rewritten = "y".repeat(4096) + &sample(3000);
    std::fs::write(&path, rewritten.as_bytes()).unwrap();
    assert_eq!(large.grow(), None, "前面变过了，不能当成长了几行");
    assert!(large.original_changed());

    // 只在末尾增长：接入索引。
    let (mut large, path) = open("append", sample(2000).as_bytes());
    let mut file = File::options().append(true).open(&path).unwrap();
    file.write_all(b"tail\n").unwrap();
    assert_eq!(large.grow(), Some(5));
    assert!(whole(&large).ends_with("tail\n"));

    // 修改之后原文件又在末尾追加内容：不再接入索引，但片段引用的部分没有变化，仍然可以写出。
    large.replace(0, 1, "x");
    file.write_all(b"more\n").unwrap();
    assert!(!large.original_changed());
    let text = whole(&large);
    assert!(text.starts_with('x') && text.ends_with("tail\n"));
    // 整个文件被重写为更长的内容：原有的末尾已变化，不能写出。
    std::fs::write(&path, "z".repeat(1 << 20)).unwrap();
    assert!(large.original_changed());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn typing_extends_one_piece() {
    let (mut large, path) = open("typing", b"hello\nworld\n");

    for (i, c) in "abc".chars().enumerate() {
        large.replace(3 + i, 0, &c.to_string());
    }

    assert_eq!(whole(&large), "helabclo\nworld\n");
    assert_eq!(large.pieces.len(), 3);

    large.replace(0, large.end_pos(), "");
    assert_eq!((large.end_pos(), large.line_count()), (0, 1));
    assert_eq!(large.line_content(0).slice(), "");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn crlf_and_bom() {
    let (large, path) = open("crlf", "\u{feff}ab\r\n中文\r\nend".as_bytes());
    let rope = Rope::from_str("ab\r\n中文\r\nend");
    assert!(large.has_bom() && large.is_crlf());
    assert_same(&large, &rope, 0..3);
    assert_eq!(large.substring(4, 8), "中文\r\n");
    assert_eq!(whole(&large), "ab\r\n中文\r\nend");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn overlong_lines_are_capped_but_positions_stay_exact() {
    // 开头多一个 ASCII 字符，使上限恰好切开一个两字节字符。
    let content = format!("a{}\nshort\n", "é".repeat(64));
    let path = write_temp("long", content.as_bytes());
    let large = LargeText::open_with(File::open(&path).unwrap(), 64).unwrap();
    let rope = Rope::from_str(&content);

    assert!(large.has_overlong_lines());
    // 被切开的字符不计入：可访问 1 + 63 / 2 = 32 个字符（整数除法）。
    assert_eq!(large.line_len(0), 32);
    assert_eq!(large.line_start(1), rope.line_start(1));
    assert_eq!(large.line_content(1).slice(), "short");
    assert_eq!(
        large.substring(0, large.line_start(1)),
        content[..content.find('\n').unwrap() + 1]
    );

    let mut starts = Vec::new();
    large.visit_lines(0, usize::MAX, |_, s, _| {
        starts.push(s);
        ControlFlow::Continue(())
    });
    assert_eq!(starts, vec![0, rope.line_start(1), rope.line_start(2)]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn invalid_utf8_and_truncation_do_not_panic() {
    let (large, path) = open("bad", b"caf\xe9 ok\nline2\nline3\n");
    assert!(!large.is_utf8());
    assert_eq!(large.line_content(0).slice(), "caf\u{fffd} ok");
    assert_eq!(large.line_start(1), 8);
    assert_eq!(large.substring(4, 7), " ok");

    // 打开后文件被截短：读到的行变短，不会出错，并且能检测到原文件已变化。
    std::fs::write(&path, b"caf").unwrap();
    assert!(large.original_changed());
    assert_eq!(large.line_content(2).slice(), "");
    assert_eq!(large.line_count(), 4);
    // 写出时读不全就报错，避免用不完整的内容替换原文件。
    let mut out = Vec::new();
    let error = large.write_to(&mut out).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn byte_helpers() {
    assert_eq!(complete_prefix("aé".as_bytes()), 3);
    assert_eq!(complete_prefix(&"aé".as_bytes()[..2]), 1);
    assert_eq!(complete_prefix(&"中".as_bytes()[..2]), 0);
    assert_eq!(byte_of_char("a中b".as_bytes(), 2), 4);
    assert_eq!(byte_of_char(b"a\xffb", 2), 2);
    assert_eq!(char_count(b"a\xff\xfeb"), 4);
    assert_eq!(char_count("a中".as_bytes()), 2);
}

#[test]
fn batched_edits_match_one_at_a_time() {
    let content = sample(400);
    let (mut large, path) = open("batched", content.as_bytes());
    let mut rope = Rope::from_str(&content);
    // 先修改若干分散的位置，使片段表中有新增的片段，再做整批修改。
    for &(start, removed, inserted) in &[(10, 3, "ab"), (5000, 0, "中\n文"), (200, 7, "")] {
        large.replace(start, removed, inserted);
        rope.replace(start, removed, inserted);
    }
    let len = rope.len_chars();
    // 从前往后（全部替换）：每处的位置是应用前面各处修改后的位置。
    let mut ascending = Vec::new();
    let mut shift = 0i64;
    for i in 0..300 {
        let at = (i * len / 300) as i64 + shift;
        let inserted = ["", "x", "新\n行", "yy"][i % 4];
        ascending.push((at as usize, i % 3, inserted));
        shift += inserted.chars().count() as i64 - (i % 3) as i64;
    }
    // 从后往前（撤销）；顺序不规则时逐处修改。
    let len = len as i64 + shift;
    let descending: Vec<_> = (0..200)
        .rev()
        .map(|i| ((i * len / 200) as usize, 2, "z\n"))
        .collect();
    let scattered = [(50, 1, "a"), (10, 0, "b"), (30, 2, "")];
    for changes in [&ascending[..], &descending, &scattered] {
        large.replace_each(changes);
        rope.replace_each(changes);
        assert_same(&large, &rope, 0..rope.len_lines());
        assert_eq!(whole(&large), rope.to_string());
    }
    let _ = std::fs::remove_file(path);
}

/// 回归：每次取行都从磁盘读出并解码整行时，一次按键（换算行列、查找字素边界）会多次读取
/// 光标所在的长行。最近读出的一行应被缓存，并在片段表变化时失效。
#[test]
fn the_last_line_read_is_kept_until_an_edit() {
    let (mut large, path) = open("last-line", "第一行 abc\n第二行\r\n".as_bytes());
    assert_eq!(large.line_len(0), 7);
    assert_eq!(large.line_content(0).slice(), "第一行 abc");
    // 行尾的 `\r` 不计入行内容。
    assert_eq!(large.line_len(1), 3);
    assert_eq!(large.line_end(1), large.line_start(1) + 3);

    // 缓存的是第 0 行；改动后缓存失效，读到的是修改后的内容。
    assert_eq!(large.line_len(0), 7);
    large.replace(1, 0, "XY");
    assert_eq!(large.line_content(0).slice(), "第XY一行 abc");
    // 再次读取同一行时使用缓存，不再读文件：文件被替换后读到的仍是缓存的行。
    std::fs::write(&path, "完全不同的内容\n").unwrap();
    assert_eq!(large.line_content(0).slice(), "第XY一行 abc");
    assert_eq!(large.line_len(0), 9);
    std::fs::remove_file(path).unwrap();
}

/// 回归：后台保存在写出之后、重命名之前不复查原文件时，期间被其他程序修改的原文件会被替换，
/// 该程序的修改随之丢失，写出的内容还可能混有修改前后的字节。
#[test]
fn writing_out_fails_when_the_original_changed_meanwhile() {
    let (large, path) = open("changed-meanwhile", b"0123456789\n");
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    std::io::Write::write_all(&mut &file, b"abcdefghij\n").unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    file.set_modified(later).unwrap();
    assert!(large.original_changed());
    let error = large.write_to(&mut Vec::new()).unwrap_err();
    assert_eq!(error.to_string(), t!("editor-original-changed"));
    std::fs::remove_file(path).unwrap();
}

/// 用 [`Lines::visit_short_lines`] 从第 `from` 行起遍历到末尾，返回各行的（行号，行首位置，内容）。
fn short_lines(
    text: &impl Lines,
    from: usize,
    max_bytes: usize,
) -> Vec<(usize, usize, Option<String>)> {
    let mut seen = Vec::new();
    text.visit_short_lines(from, max_bytes, |line, start, content| {
        seen.push((line, start, content.map(str::to_owned)));
        ControlFlow::Continue(())
    });
    seen
}

/// 限制每行字节数的遍历读到上限即交出该行，再按索引跳到下一行；结果与整行读取后截断相同。
///
/// 样本含跨越多个读取块的超长行、多字节字符、编辑产生的新增片段，以及不以换行结尾的超长末行。
#[test]
fn capped_visits_skip_the_rest_of_long_lines() {
    let long = |i: usize| format!("{i} INSERT 值{}", "中x".repeat(70_000 + i));
    let mut content = String::new();
    for i in 0..40 {
        let line = if i % 3 == 0 {
            long(i)
        } else {
            format!("{i} 短行")
        };
        content.push_str(&line);
        content.push('\n');
    }
    content.push_str(&long(99));
    let (mut large, path) = open("capped-visits", content.as_bytes());
    let mut rope = Rope::from_str(&content);
    let at = rope.line_start(6) + 1000;
    for text in [&mut large as &mut dyn EditBuffer, &mut rope] {
        text.replace(at, 10, "插入\n新行");
    }

    let visit = |text: &dyn VisitAll, from: usize, cap: usize, stop: usize| {
        let mut seen = Vec::new();
        let stopped = text.visit_all(from, cap, &mut |l, s, t| {
            seen.push((l, s, t.to_owned()));
            if l >= stop {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        (seen, stopped)
    };
    for cap in [7, 1000, winc_core::text::MAX_LINE_BYTES] {
        for (from, stop) in [(0, usize::MAX), (5, 12), (38, usize::MAX), (41, usize::MAX)] {
            assert_eq!(
                visit(&large, from, cap, stop),
                visit(&rope, from, cap, stop),
                "从第 {from} 行起，每行最多 {cap} 字节"
            );
        }
        // 超过上限的行不交出内容：大文件由行首位置得出行的字节数，不读取这些行，
        // 结果与逐行读取的普通文本相同。
        for from in [0, 5, 6, 38, 41] {
            assert_eq!(
                short_lines(&large, from, cap),
                short_lines(&rope, from, cap),
                "从第 {from} 行起，超过 {cap} 字节的行不交出内容"
            );
        }
        // 只读行首得到的内容与按同一上限遍历时交出的内容相同。
        let (seen, _) = visit(&large, 0, cap, usize::MAX);
        for (line, _, content) in seen {
            assert_eq!(
                large.line_prefix(line, cap).slice(),
                content.as_str(),
                "第 {line} 行"
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// 以同一方式调用两种文本的 [`Lines::visit_lines`]。
trait VisitAll {
    fn visit_all(
        &self,
        from: usize,
        cap: usize,
        f: &mut dyn FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize>;
}

impl<T: Lines> VisitAll for T {
    fn visit_all(
        &self,
        from: usize,
        cap: usize,
        f: &mut dyn FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        self.visit_lines(from, cap, f)
    }
}

/// 超长行内部记有位置，按字符位置查找时从行内最近的记录开始读；结果与 `Rope` 一致，并行扫描
/// 分段与末尾追加后同样如此。
#[test]
fn positions_inside_long_lines_are_indexed() {
    let mut content = String::new();
    for i in 0..6 {
        let unit = if i % 2 == 0 { "中x" } else { "ab" };
        writeln!(content, "{i} {}", unit.repeat(150_000)).unwrap();
    }
    let path = write_temp("long-marks", content.as_bytes());
    let large = LargeText::open_with(File::open(&path).unwrap(), LINE_CAP).unwrap();
    let rope = Rope::from_str(&content);
    assert!(
        large.original.marks.len() > 6,
        "{}",
        large.original.marks.len()
    );
    let mut rng = Rng(7);
    for _ in 0..300 {
        let a = rng.below(rope.end_pos());
        let b = (a + rng.below(300)).min(rope.end_pos());
        assert_eq!(large.substring(a, b), rope.substring(a, b), "{a}..{b}");
        assert_eq!(large.line_of(a), rope.line_of(a), "位置 {a}");
    }
    std::fs::remove_file(path).unwrap();
}
