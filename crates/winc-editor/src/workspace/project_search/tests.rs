//! 全局搜索与跨文件替换的测试：忽略规则与 glob、大小写与全词、预览截断、结果上限、按块
//! 读取与大文件、跨行匹配，以及替换后保持编码和换行符。

use super::*;
use crate::test_util::scratch;

fn write(dir: &Path, name: &str, text: &[u8]) {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn query(text: &str) -> Query {
    Query {
        text: text.into(),
        ..Query::default()
    }
}

fn search(dir: &Path, query: &Query) -> ProjectSearch {
    let mut search = ProjectSearch::default();
    search.start(dir, query, Vec::new(), || {});
    search.wait();
    search
}

fn files(search: &ProjectSearch) -> Vec<&str> {
    search.results.iter().map(|f| f.relative.as_str()).collect()
}

#[test]
fn ignored_files_are_searched_only_when_asked() {
    let dir = scratch("include-ignored");
    write(&dir, ".gitignore", b"dist/\n");
    write(&dir, "src/main.rs", b"needle\n");
    write(&dir, "dist/out.js", b"needle\n");
    write(&dir, ".git/config", b"needle\n");
    let names = |include_ignored: bool, exclude: &str| {
        let q = Query {
            include_ignored,
            exclude: exclude.to_owned(),
            ..query("needle")
        };
        let found = search(&dir, &q);
        files(&found)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(false, ""), ["src/main.rs"]);
    // `.git` 这类目录始终不搜索。
    assert_eq!(names(true, ""), ["dist/out.js", "src/main.rs"]);
    // 「排除」规则仍然生效。
    assert_eq!(names(true, "*.js"), ["src/main.rs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finds_matches_and_respects_ignore_rules() {
    let dir = scratch("ignore");
    write(&dir, ".gitignore", b"target/\n*.log\n!keep.log\n");
    write(
        &dir,
        "src/main.rs",
        b"fn main() {\n    let needle = 1;\n}\n",
    );
    write(&dir, "src/lib.rs", b"// needle needle\n");
    write(&dir, "src/10.rs", b"needle\n");
    write(&dir, "src/9.rs", b"needle\n");
    write(&dir, "target/out.rs", b"needle\n");
    write(&dir, "debug.log", b"needle\n");
    write(&dir, "keep.log", b"needle\n");
    write(&dir, ".github/ci.yml", b"needle\n");
    write(&dir, ".git/config", b"needle\n");
    write(&dir, "__pycache__/x.py", b"needle\n");
    write(&dir, "data.bin", b"needle\0\0");

    let found = search(&dir, &query("needle"));
    // 隐藏目录会搜索；版本库目录、被忽略的文件和二进制文件不搜；结果按路径自然排序。
    assert_eq!(
        files(&found),
        [
            ".github/ci.yml",
            "keep.log",
            "src/9.rs",
            "src/10.rs",
            "src/lib.rs",
            "src/main.rs"
        ]
    );
    assert_eq!(found.total(), 7);
    assert!(!found.is_running() && !found.is_truncated());

    let main = found
        .results
        .iter()
        .find(|f| f.relative == "src/main.rs")
        .unwrap();
    assert_eq!(main.matches[0].line, 1);
    assert_eq!(main.matches[0].cols, 8..14);
    assert_eq!(main.matches[0].preview, "let needle = 1;");
    assert_eq!(
        &main.matches[0].preview[main.matches[0].highlight.clone()],
        "needle"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn include_and_exclude_globs() {
    let dir = scratch("globs");
    write(&dir, "src/a.rs", b"x\n");
    write(&dir, "src/a.ts", b"x\n");
    write(&dir, "src/deep/b.rs", b"x\n");
    write(&dir, "web/src/c.ts", b"x\n");
    write(&dir, "docs/d.md", b"x\n");

    let with = |include: &str, exclude: &str| {
        let q = Query {
            include: include.into(),
            exclude: exclude.into(),
            ..query("x")
        };
        files(&search(&dir, &q))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    assert_eq!(with("*.rs", ""), ["src/a.rs", "src/deep/b.rs"]);
    // 不含 `/` 的目录名在任意一级生效；以 `./` 开头的只从根目录起算。
    assert_eq!(
        with("src", ""),
        ["src/a.rs", "src/a.ts", "src/deep/b.rs", "web/src/c.ts"]
    );
    assert_eq!(with("./src", ""), ["src/a.rs", "src/a.ts", "src/deep/b.rs"]);
    assert_eq!(with("src/deep, docs", ""), ["docs/d.md", "src/deep/b.rs"]);
    assert_eq!(with("", "*.ts, docs"), ["src/a.rs", "src/deep/b.rs"]);
    assert_eq!(
        with("src", "deep"),
        ["src/a.rs", "src/a.ts", "web/src/c.ts"]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn options_errors_and_previews() {
    let dir = scratch("options");
    let long = format!("{}Needle at the end{}\n", "x".repeat(300), "y".repeat(300));
    write(&dir, "a.txt", b"Needle needle\n\tneedles\n");
    write(&dir, "long.txt", long.as_bytes());
    write(&dir, "crlf.txt", b"one\r\ntwo\rneedle\r\n");
    let (gbk, _, _) = encoding_rs::GBK.encode("needle 你好，这是一段用 GBK 写的中文。\n");
    write(&dir, "gbk.txt", &gbk);

    let q = Query {
        case_sensitive: true,
        whole_word: true,
        ..query("needle")
    };
    let found = search(&dir, &q);
    assert_eq!(files(&found), ["a.txt", "crlf.txt", "gbk.txt"]);
    let a = &found.results[0];
    assert_eq!(a.matches.len(), 1);
    assert_eq!(a.matches[0].cols, 7..13);
    // `\r\n` 和单独的 `\r` 都算换行，行号与打开后一致。
    assert_eq!(found.results[1].matches[0].line, 2);
    // GBK 文件先识别编码再搜索，结果中的中文是正确的。
    assert!(!found.results[2].lossy);
    assert!(found.results[2].matches[0].preview.contains("你好"));

    // 超长的行：匹配之前的部分截去并以「…」代替；之后的部分截到长度上限。
    let found = search(&dir, &query("needle at"));
    let long = found
        .results
        .iter()
        .find(|f| f.relative == "long.txt")
        .unwrap();
    let m = &long.matches[0];
    assert!(m.preview.starts_with('…'));
    assert!(m.preview.chars().count() <= PREVIEW_CHARS + 1);
    assert_eq!(&m.preview[m.highlight.clone()], "Needle at");
    assert_eq!(m.cols, 300..309);

    // 预览中制表符换成空格，行首空白去掉。
    let found = search(&dir, &query("needles"));
    assert_eq!(found.results[0].matches[0].preview, "needles");

    // 正则表达式有误时给出说明，不搜索。
    let bad = Query {
        regex: true,
        ..query("(")
    };
    let found = search(&dir, &bad);
    assert!(found.error().unwrap().starts_with("正则表达式有误"));
    assert!(!found.is_filter_error());
    assert!(found.results.is_empty());

    // 「包含」里的 glob 有误时同样不搜索，并能分辨出错在 glob 上。
    let bad = Query {
        include: "a[".to_owned(),
        ..query("needle")
    };
    let found = search(&dir, &bad);
    assert!(found.error().unwrap().starts_with("包含规则有误"));
    assert!(found.is_filter_error());
    assert!(found.results.is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unsaved_documents_are_searched_in_memory_and_results_are_capped() {
    let dir = scratch("dirty");
    write(&dir, "a.rs", b"old\n");
    write(&dir, "many.txt", "hit ".repeat(MAX_RESULTS + 50).as_bytes());

    // 有未保存修改的文档按内存中的内容搜索：磁盘上是 `old`，内存中第二行是 `new hit`。
    let mut found = ProjectSearch::default();
    let dirty = vec![(dir.join("a.rs"), Rope::from_str("first\nnew hit\n"))];
    found.start(&dir, &query("new"), dirty, || {});
    found.wait();
    assert_eq!(files(&found), ["a.rs"]);
    assert_eq!(found.results[0].matches[0].line, 1);
    assert!(search(&dir, &query("old")).results.len() == 1);

    // 结果达到上限即停止，并标记结果不全。
    let found = search(&dir, &query("hit"));
    assert_eq!(found.total(), MAX_RESULTS);
    assert!(found.is_truncated());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn one_file_with_more_matches_than_the_limit_stops_at_the_limit() {
    let dir = scratch("one-big-file");
    let line = "e ".repeat(MAX_RESULTS * 2);
    write(&dir, "min.js", line.as_bytes());

    let found = search(&dir, &query("e"));
    assert_eq!(found.total(), MAX_RESULTS);
    assert_eq!(found.results[0].matches.len(), MAX_RESULTS);
    assert!(found.is_truncated());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn large_files_are_read_on_their_own_and_still_searched() {
    let dir = scratch("large");
    // 一个超过缓冲容量上限（`BUFFER_MAX`）的文件和一个不超过的文件都要搜到；读过大文件之后，
    // 小文件仍使用缓冲。
    let big = format!("{}\nneedle at the end\n", "x".repeat(BUFFER_MAX + 10));
    write(&dir, "big.txt", big.as_bytes());
    write(&dir, "small.txt", b"one needle\n");
    let gbk = [&b"y".repeat(BUFFER_MAX + 10)[..], b"\nneedle \xC4\xE3\n"].concat();
    write(&dir, "big-gbk.txt", &gbk);

    let found = search(&dir, &query("needle"));
    assert_eq!(files(&found), ["big-gbk.txt", "big.txt", "small.txt"]);
    assert_eq!(found.results[0].matches[0].line, 1);
    assert_eq!(found.results[1].matches[0].line, 1);
    assert_eq!(found.results[2].matches[0].line, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replace_in_files_keeps_encoding_and_line_endings() {
    let dir = scratch("replace");
    write(&dir, "a.txt", b"\xEF\xBB\xBFfoo1 foo2\r\nbar foo3\r\n");
    write(&dir, "gbk.txt", b"foo \xC4\xE3\n");

    let q = Query {
        regex: true,
        ..query(r"foo(\d)")
    };
    let a = &search(&dir, &q).results[0];
    assert!(a.path.ends_with("a.txt"));
    assert_eq!(replace_in_file(&a.path, &q, "baz$1", &a.matches), Ok(3));
    assert_eq!(
        fs::read(dir.join("a.txt")).unwrap(),
        b"\xEF\xBB\xBFbaz1 baz2\r\nbar baz3\r\n"
    );

    // 其他编码的文件按原编码写回，未修改的字节保持不变。
    let plain = query("foo");
    let first = [at(0, 0..3)];
    assert_eq!(
        replace_in_file(&dir.join("gbk.txt"), &plain, "x", &first),
        Ok(1)
    );
    assert_eq!(fs::read(dir.join("gbk.txt")).unwrap(), b"x \xC4\xE3\n");

    // 无法解码的文件（UTF-8 中有非法字节）不修改，保持原样。
    let mut damaged = "中文中文中文中文中文".as_bytes().to_vec();
    damaged.splice(0..0, b"foo ".iter().copied());
    damaged.push(0xFF);
    write(&dir, "bad.txt", &damaged);
    assert!(replace_in_file(&dir.join("bad.txt"), &plain, "x", &first).is_err());
    assert_eq!(fs::read(dir.join("bad.txt")).unwrap(), damaged);

    // 已打开的文档：作为一次可撤销的修改，不写盘。
    let mut doc = Document::from_text(DocumentId(1), "foo foo\n");
    let both = [at(0, 0..3), at(0, 4..7)];
    assert_eq!(replace_in_document(&mut doc, &plain, "x", &both), Ok(2));
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "x x\n");
    doc.execute(crate::Command::Undo, 10, 4);
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "foo foo\n");

    let _ = fs::remove_dir_all(&dir);
}

/// 构造一处匹配（测试用）：只有位置，没有预览。
fn at(line: usize, cols: Range<usize>) -> LineMatch {
    LineMatch {
        line,
        cols,
        preview: String::new(),
        highlight: 0..0,
    }
}

#[test]
fn replacing_skips_the_matches_removed_from_the_results() {
    let dir = scratch("dismissed");
    write(&dir, "a.txt", b"foo foo\nfoo\n");
    let q = query("foo");
    let mut found = search(&dir, &q);
    let path = dir.join("a.txt");

    // 从结果中移除第二处匹配，替换时不会改动它。
    found.remove_match(&path, 1);
    let left = &found.results[0].matches;
    assert_eq!(replace_in_file(&path, &q, "bar", left), Ok(2));
    assert_eq!(fs::read_to_string(&path).unwrap(), "bar foo\nbar\n");

    // 正则在长行上有 2000 处匹配：每处按各自的分组展开。
    let re = Query {
        regex: true,
        ..query(r"k(\d+)")
    };
    let line: String = (0..2000).map(|i| format!("k{i} ")).collect();
    let mut doc = Document::from_text(DocumentId(1), &line);
    let all: Vec<LineMatch> = (0..2000)
        .scan(0, |col, i| {
            let len = format!("k{i}").chars().count();
            let m = at(0, *col..*col + len);
            *col += len + 1;
            Some(m)
        })
        .collect();
    assert_eq!(replace_in_document(&mut doc, &re, "v$1", &all), Ok(2000));
    let text = doc.text().substring(0, doc.text().end_pos());
    assert!(text.starts_with("v0 v1 v2 ") && text.ends_with("v1999 "));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refresh_several_files_at_once() {
    let dir = scratch("batch");
    for name in ["a.txt", "b.txt", "c.txt"] {
        write(&dir, name, b"foo\n");
    }
    let mut found = search(&dir, &query("foo"));
    assert_eq!(found.total(), 3);
    let version = found.version();

    // 一个文件多了匹配、一个被删除、一个已打开且有未保存的修改，一次全部重新搜索。
    write(&dir, "a.txt", b"foo foo\n");
    fs::remove_file(dir.join("b.txt")).unwrap();
    found.refresh_files([
        (dir.join("a.txt"), None),
        (dir.join("b.txt"), None),
        (dir.join("c.txt"), Some(Rope::from_str("x\nfoo\nfoo\n"))),
    ]);
    // 在后台搜索，搜完之前结果不变。
    assert!(found.is_running());
    assert_eq!(files(&found), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(found.version(), version);
    found.wait();
    assert!(!found.is_running());
    assert_eq!(files(&found), ["a.txt", "c.txt"]);
    assert_eq!(found.total(), 4);
    assert_eq!(found.results[1].matches[0].line, 1);
    assert_eq!(found.version(), version + 1);

    // 新的搜索作废旧的匹配器：清空之后重新搜索不再改动结果。
    found.clear();
    found.refresh_files([(dir.join("a.txt"), None)]);
    assert!(!found.is_running());
    assert!(found.results.is_empty() && found.total() == 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refreshes_in_the_background_merge_and_yield_to_direct_ones() {
    let dir = scratch("pending");
    for name in ["a.txt", "b.txt", "c.txt"] {
        write(&dir, name, b"foo\n");
    }
    let mut found = search(&dir, &query("foo"));

    // 上一批还没搜完又来一批：合并，两批的文件都重新搜索。
    write(&dir, "a.txt", b"foo foo\n");
    write(&dir, "b.txt", b"foo foo foo\n");
    found.refresh_files([(dir.join("a.txt"), None)]);
    found.refresh_files([(dir.join("b.txt"), None)]);
    // 搜完之前直接重新搜索的文件以直接搜到的为准，后台对它的结果作废。
    found.refresh_files([(dir.join("c.txt"), None)]);
    found.refresh_file(&dir.join("c.txt"), Some(&Rope::from_str("foo\nfoo\n")));
    assert_eq!(found.total(), 1 + 1 + 2);
    found.wait();
    let counts: Vec<_> = found.results.iter().map(|f| f.matches.len()).collect();
    assert_eq!(counts, [2, 3, 2]);
    assert_eq!(found.total(), 7);

    // 整体搜索进行中时不处理：它会读到新内容。
    found.start(&dir, &query("foo"), Vec::new(), || {});
    found.refresh_files([(dir.join("a.txt"), Some(Rope::from_str("")))]);
    found.wait();
    assert_eq!(found.total(), 2 + 3 + 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replace_single_matches_and_refresh_one_file() {
    let dir = scratch("one");
    write(&dir, "a.txt", b"foo foo\nfoo\n");
    let q = query("foo");
    let mut found = search(&dir, &q);
    assert_eq!(found.total(), 3);

    // 替换第二处，只重新搜索这一个文件。
    let path = dir.join("a.txt");
    assert_eq!(replace_in_file(&path, &q, "bar", &[at(0, 4..7)]), Ok(1));
    assert_eq!(fs::read_to_string(&path).unwrap(), "foo bar\nfoo\n");
    found.refresh_file(&path, None);
    assert_eq!(found.total(), 2);
    assert_eq!(found.results[0].matches[1].line, 1);

    // 该位置已不再是匹配：不修改。
    assert_eq!(replace_in_file(&path, &q, "bar", &[at(0, 4..7)]), Ok(0));
    assert_eq!(fs::read_to_string(&path).unwrap(), "foo bar\nfoo\n");

    // 移除一处匹配；文件的最后一处匹配被移除后，该文件也从结果中移除。
    found.remove_match(&path, 0);
    assert_eq!(found.total(), 1);
    found.remove_match(&path, 0);
    assert!(found.results.is_empty() && found.total() == 0);

    // 替换预览：正则展开分组，普通文本原样输出。
    let re = Query {
        regex: true,
        ..query(r"(\w+)\.unwrap\(\)")
    };
    assert_eq!(
        ReplacePreview::new(&re, "$1?").apply("x.unwrap()", 0..10),
        "x?"
    );
    assert_eq!(ReplacePreview::new(&q, "$1").apply("foo", 0..3), "$1");
    // 使用环视的正则在预览文本内原地展开，环视能看到匹配之前的字符。
    let behind = Query {
        regex: true,
        ..query(r"(?<=let )(\w+)")
    };
    let preview = ReplacePreview::new(&behind, "${1}_new");
    assert_eq!(preview.apply("let total = 1;", 4..9), "total_new");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reads_files_exactly_like_opening_them() {
    // 全局搜索读到的文本（去 BOM、解码、分行）与打开后文档中的相同，被当作二进制而跳过的
    // 文件也相同。
    let dir = scratch("same-as-open");
    let mut near_end = vec![b'a'; 8191];
    near_end.push(0);
    let mut past_end = vec![b'a'; 8192];
    past_end.extend_from_slice(b"\0x\n");
    let samples: [(&str, Vec<u8>); 8] = [
        ("plain", b"one\ntwo\n".to_vec()),
        ("bom-crlf", b"\xEF\xBB\xBFone\r\ntwo\r\n".to_vec()),
        ("lone-cr", b"one\rtwo\rthree".to_vec()),
        ("lossy", b"caf\xE9\nok\n".to_vec()),
        ("lossy-bom", b"\xEF\xBB\xBFcaf\xE9\n".to_vec()),
        ("empty", Vec::new()),
        ("nul-near-end", near_end),
        ("nul-past-sniff", past_end),
    ];

    for (name, bytes) in samples {
        let path = dir.join(name);
        fs::write(&path, &bytes).unwrap();

        let opened = Document::open(DocumentId(1), path.clone());
        assert_eq!(looks_binary(&bytes), opened.is_err(), "{name}");
        let Ok(doc) = opened else {
            continue;
        };

        let decoded = decode(&bytes);
        let mut lines = Vec::new();
        let _ = for_each_line(&decoded.text, 0, true, |_, line| {
            lines.push(line.to_owned());
            ControlFlow::Continue(())
        });
        let text = doc.text();
        let opened: Vec<String> = (0..text.line_count())
            .map(|i| text.line_content(i).slice().to_string())
            .collect();
        assert_eq!(lines, opened, "{name}");
        assert_eq!(decoded.encoding, doc.encoding, "{name}");
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reading_in_chunks_finds_what_the_opened_file_shows() {
    let dir = scratch("chunks");
    // 测试中每块只有 `CHUNK`（7）字节：`\r\n`、多字节字符、BOM 都会落在块的边界上。
    let mut samples: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("mixed", "中a\nb中\r\n中\rd中\n\n中中中中中中\r".into()),
        ("bom", "\u{feff}中x\r\n\r\n末尾中".into()),
        (
            "gbk-late",
            [&"中文\n".repeat(3000).into_bytes()[..], b"\xD6\xD0\n"].concat(),
        ),
        (
            "utf16",
            [
                &[0xFF, 0xFE][..],
                &"中\n中"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<u8>>(),
            ]
            .concat(),
        ),
        (
            "long-line",
            format!("{}中\n中", "a".repeat(5000)).into_bytes(),
        ),
        ("crlf-z", "a\r\nz\r\nxz\r\nb\rz\nyz中\r\n\r\nz".into()),
    ];
    // 大文件：按需从磁盘读取，只按 `\n` 分行，去掉行尾的 `\r`，非法字节替换为 U+FFFD。
    let mut large = [
        &b"\xEF\xBB\xBF"[..],
        "中".as_bytes(),
        b"\xFF\r\nz",
        "中".as_bytes(),
        b"\rz\n",
    ]
    .concat();
    let filler = [&[b'a'; 1023][..], b"\n"].concat();
    while large.len() as u64 <= LARGE_FILE {
        large.extend_from_slice(&filler);
    }
    large.extend_from_slice(&[&b"z\r\n"[..], "中\rz\n中".as_bytes(), b"\xC3z\xE4\xB8"].concat());
    samples.push(("large", large));

    let options = Options {
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    // 整块查找候选位置（`block_find`）不能漏掉任何匹配：覆盖行首行尾、整词、`.`、大小写、`\A`、
    // 环视、反向引用。
    let patterns = [
        "z$|中",
        "中.z",
        "^中",
        "^$",
        "中$",
        r"\bb\b",
        "c.d",
        "(?i)D中$",
        r"\A中|x",
        r"(?<=中)b",
        r"(?<!\s)中$",
        r"(中)\1",
        r"\bz(?=中)",
        r"(?i)(?<![a-z])D",
        r"\s*中",
        r"[^a]z",
    ];
    for (name, bytes) in samples {
        let path = dir.join(name);
        fs::write(&path, &bytes).unwrap();
        let doc = Document::open(DocumentId(1), path.clone()).unwrap();
        let text = doc.text();
        let lines: Vec<String> = (0..text.line_count())
            .map(|i| text.line_content(i).slice().to_string())
            .collect();
        // 大文件每块只有几个字节时搜一遍耗时较长，只取前两个模式。
        let count = if name == "large" { 2 } else { patterns.len() };

        for pattern in &patterns[..count] {
            let matcher = Matcher::new(pattern, options).unwrap();
            let (found, lossy) = search_disk(
                &matcher,
                &mut Vec::new(),
                &path,
                usize::MAX,
                &AtomicBool::new(false),
            )
            .unwrap();
            let found: Vec<(usize, Range<usize>)> =
                found.into_iter().map(|m| (m.line, m.cols)).collect();

            let mut expected = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                let mut out = Vec::new();
                let _ = collect(&matcher, i, line, &mut out, usize::MAX);
                expected.extend(out.into_iter().map(|m| (m.line, m.cols)));
            }
            assert_eq!(found, expected, "{name} {pattern}");
            assert_eq!(lossy, doc.encoding == crate::Encoding::Lossy, "{name}");
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn large_utf16_is_searched_on_decoded_lines() {
    let dir = scratch("large-utf16-search");
    let line = format!("KEY_{}\t平\r\n", "a".repeat(240));
    let mut units = vec![0xFF, 0xFE];
    let encoded: Vec<u8> = line.encode_utf16().flat_map(u16::to_le_bytes).collect();
    while units.len() as u64 <= LARGE_FILE {
        units.extend_from_slice(&encoded);
    }
    let tail = "NEEDLE\t找到了\r\n";
    units.extend(tail.encode_utf16().flat_map(u16::to_le_bytes));
    write(&dir, "CommonLocalization.txt", &units);

    let found = search(&dir, &query("找到了"));
    let file = &found.results[0];
    assert_eq!(file.matches.len(), 1);
    let newlines = units[2..]
        .chunks_exact(2)
        .filter(|c| u16::from_le_bytes([c[0], c[1]]) == u16::from(b'\n'))
        .count();
    assert_eq!(file.matches[0].line, newlines - 1);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn large_files_are_searched_and_replaced_too() {
    let dir = scratch("large-replace");
    let mut text = b"needle\r\n".to_vec();
    let filler = [&[b'a'; 1023][..], b"\n"].concat();
    while text.len() as u64 <= LARGE_FILE {
        text.extend_from_slice(&filler);
    }
    text.extend_from_slice(b"x needle");
    write(&dir, "big.log", &text);

    let query = query("needle");
    let found = search(&dir, &query);
    let file = &found.results[0];
    assert_eq!(file.matches.len(), 2);
    let last = text.iter().filter(|&&b| b == b'\n').count();
    assert_eq!(
        (file.matches[1].line, file.matches[1].cols.clone()),
        (last, 2..8)
    );

    let count = replace_in_file(&file.path, &query, "pin", &file.matches).unwrap();
    assert_eq!(count, 2);
    let after = fs::read(&file.path).unwrap();
    assert!(after.starts_with(b"pin\r\n") && after.ends_with(b"x pin"));
    assert_eq!(after.len(), text.len() - 6);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn regexes_with_newlines_match_across_lines_in_every_file() {
    let dir = scratch("multiline");
    write(&dir, "crlf.txt", b"x\r\nfn a()\r\n{\r\n}");
    write(&dir, "cr.txt", "中fn b()\r{".as_bytes());
    write(&dir, "gbk.txt", b"\xD6\xD0fn c()\n{");
    write(&dir, "open.txt", b"fn d()\n{");
    let mut query = query("fn \\w+\\(\\)\\n\\{");
    query.regex = true;

    // 有未保存修改的文档按内存中的内容搜索。
    let dirty = vec![(dir.join("open.txt"), Rope::from_str("// 改过\nfn e()\r\n{"))];
    let mut found = ProjectSearch::default();
    found.start(&dir, &query, dirty, || {});
    found.wait();
    let got: Vec<(&str, usize, Range<usize>, &str)> = found
        .results
        .iter()
        .flat_map(|f| {
            f.matches.iter().map(|m| {
                let text = &m.preview[m.highlight.clone()];
                (f.relative.as_str(), m.line, m.cols.clone(), text)
            })
        })
        .collect();
    // 列的结尾按文件中的位置计算：`\r\n` 是两个字符。预览取起始行。
    assert_eq!(
        got,
        [
            ("cr.txt", 0, 1..9, "fn b()"),
            ("crlf.txt", 1, 0..9, "fn a()"),
            ("gbk.txt", 0, 1..9, "fn c()"),
            ("open.txt", 1, 0..9, "fn e()"),
        ]
    );

    // 替换：只替换找到的匹配，换行符保持文件原有的写法。
    let file = &found.results[1];
    let count = replace_in_file(&file.path, &query, "fn z() {", &file.matches).unwrap();
    assert_eq!(count, 1);
    assert_eq!(fs::read(&file.path).unwrap(), b"x\r\nfn z() {\r\n}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lookaround_works_in_every_file_and_in_unsaved_ones() {
    let dir = scratch("fancy");
    write(&dir, "a.rs", b"let price = $100;\nlet cost = 200;");
    write(&dir, "b.txt", "\u{feff}中文 $300 中文".as_bytes());
    write(&dir, "open.rs", b"$1");
    let mut query = query(r"(?<=\$)\d+");
    query.regex = true;

    let dirty = vec![(dir.join("open.rs"), Rope::from_str("改了 $400"))];
    let mut found = ProjectSearch::default();
    found.start(&dir, &query, dirty, || {});
    found.wait();
    let got: Vec<(&str, usize, Range<usize>)> = found
        .results
        .iter()
        .flat_map(|f| {
            f.matches
                .iter()
                .map(|m| (f.relative.as_str(), m.line, m.cols.clone()))
        })
        .collect();
    assert_eq!(
        got,
        [
            ("a.rs", 0, 13..16),
            ("b.txt", 0, 4..7),
            ("open.rs", 0, 4..7)
        ]
    );

    // 含反向引用的正则：替换结果写回文件。
    let mut twice = super::tests::query(r"\b(\w+) \1\b");
    twice.regex = true;
    write(&dir, "c.txt", b"the the end\n");
    let found = search(&dir, &twice);
    let file = found
        .results
        .iter()
        .find(|f| f.relative == "c.txt")
        .unwrap();
    assert_eq!(
        replace_in_file(&file.path, &twice, "$1", &file.matches),
        Ok(1)
    );
    assert_eq!(fs::read_to_string(&file.path).unwrap(), "the end\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn splits_lines_like_the_rope() {
    let text = "a\nb\r\nc\rd\u{2028}e\u{85}f\x0Bg";
    let mut lines = Vec::new();
    let _ = for_each_line(text, 0, true, |i, line| {
        lines.push((i, line.to_owned()));
        ControlFlow::Continue(())
    });
    let rope = Rope::from_str(text);
    assert_eq!(lines.len(), rope.line_count());

    for (i, line) in lines {
        assert_eq!(line, rope.line_content(i).slice().to_string());
    }
}

#[test]
fn batches_merge_in_the_same_order_as_all_at_once() {
    let file = |relative: &str, matches: usize| FileResult {
        path: PathBuf::from(relative),
        relative: relative.to_owned(),
        matches: vec![
            LineMatch {
                line: 0,
                cols: 0..1,
                preview: String::new(),
                highlight: 0..0,
            };
            matches
        ],
        lossy: false,
    };
    let names = [
        "src/10.rs",
        "a.rs",
        "src/9.rs",
        "B.rs",
        "src/a/x.rs",
        "z",
        "src/2.rs",
    ];
    let (mut one, mut one_total) = (Vec::new(), 0);
    let (mut merged, mut merged_total) = (Vec::new(), 0);
    let all = names
        .iter()
        .enumerate()
        .map(|(i, name)| file(name, i + 1))
        .collect();
    merge_sorted(&mut one, &mut one_total, all);
    // 分批合并：第一批合并进空列表，之后几批插在已有结果的中间和两端。
    for batch in [&names[..3], &names[3..4], &names[4..]] {
        let batch = batch
            .iter()
            .map(|name| file(name, names.iter().position(|n| n == name).unwrap() + 1))
            .collect();
        merge_sorted(&mut merged, &mut merged_total, batch);
    }
    let order = |results: &[FileResult]| -> Vec<String> {
        results.iter().map(|f| f.relative.clone()).collect()
    };
    assert_eq!(order(&merged), order(&one));
    assert_eq!(merged_total, one_total);
    assert_eq!(merged_total, (1..=names.len()).sum::<usize>());
    assert_eq!(
        order(&merged),
        [
            "a.rs",
            "B.rs",
            "src/2.rs",
            "src/9.rs",
            "src/10.rs",
            "src/a/x.rs",
            "z"
        ]
    );
}

#[cfg(unix)]
#[test]
fn searches_links_to_files_but_does_not_enter_links_to_directories() {
    use std::os::unix::fs::symlink;

    let dir = scratch("links");
    write(&dir, "lib/a.rs", b"needle\n");
    symlink(dir.join("lib/a.rs"), dir.join("link.rs")).unwrap();
    symlink(dir.join("lib"), dir.join("lib-link")).unwrap();
    symlink(dir.join("missing.rs"), dir.join("broken.rs")).unwrap();

    let found = search(&dir, &query("needle"));
    assert_eq!(files(&found), ["lib/a.rs", "link.rs"]);

    let _ = fs::remove_dir_all(&dir);
}

/// 回归：正则回溯超过上限的行按没有匹配处理，结果不完整且没有提示。
#[test]
fn regex_backtracking_limits_are_reported_across_files() {
    let dir = scratch("gave-up");
    write(
        &dir,
        "slow.txt",
        format!("{}bc\n", "a".repeat(40)).as_bytes(),
    );
    write(&dir, "fine.txt", b"aac\n");
    let found = search(
        &dir,
        &Query {
            regex: true,
            ..query(r"(a|aa)+\1c")
        },
    );
    assert_eq!(files(&found), ["fine.txt"]);
    assert!(found.gave_up() && !found.is_truncated());
    let found = search(&dir, &query("aac"));
    assert!(!found.gave_up());
    let _ = fs::remove_dir_all(&dir);
}

/// 必含的文字只决定跳过哪些内容，结果与不给这项信息时相同；测试中每块 7 字节，换行符和名字都会
/// 落在块的边界上。
#[test]
fn required_text_does_not_change_the_results() {
    let dir = scratch("required-text");
    write(
        &dir,
        "a.rs",
        b"// load\nfn other() {}\nfn load() {}\nlet x = load();\n  const load = 1;\n",
    );
    write(
        &dir,
        "b.c",
        b"load\r\nint load(int a) {\r\n}\r#define load 1\r",
    );
    write(&dir, "c.rb", b"valid?\ndef valid?\ndef valid?x\nend");
    write(
        &dir,
        "d.sql",
        b"select load;\nCREATE TABLE LOAD (id int);\nfn Load\n",
    );
    write(
        &dir,
        "e.txt",
        "\u{4e2d}\u{6587} load\nfn load\u{4e2d}\nfn load \u{6587}\n".as_bytes(),
    );
    write(&dir, "f.txt", b"nothing here\n");
    let found = |query: &Query| -> Vec<(String, Vec<LineMatch>)> {
        search(&dir, query)
            .results
            .iter()
            .map(|f| (f.relative.clone(), f.matches.clone()))
            .collect()
    };

    for (name, ignore_case, hits) in [("load", false, 5), ("LOAD", true, 7), ("valid?", false, 1)] {
        let fast = crate::definition::query(name, ignore_case);
        assert_eq!(fast.required, name);
        let plain = Query {
            required: String::new(),
            ..fast.clone()
        };
        let results = found(&fast);
        assert_eq!(results, found(&plain), "{name}");
        let total: usize = results.iter().map(|(_, m)| m.len()).sum();
        assert_eq!(total, hits, "{name}: {results:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}
