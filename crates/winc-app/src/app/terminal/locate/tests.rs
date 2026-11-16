//! 位置识别的测试：常见编译器的 `文件:行:列` 写法、不误认时间与端口、格数而非字数。

use super::*;

fn found(text: &str) -> Vec<(String, usize, Option<usize>, &str)> {
    let line = Line {
        text: text.into(),
        ..Line::default()
    };
    locations(&line, false)
        .into_iter()
        .map(|l| {
            let cells: String = text.chars().skip(l.start).take(l.end - l.start).collect();
            (l.path, l.line.unwrap(), l.col, &*cells.leak())
        })
        .collect()
}

#[test]
fn recognizes_the_usual_compiler_formats() {
    type Expected<'a> = &'a [(&'a str, usize, Option<usize>, &'a str)];
    let cases: &[(&str, Expected)] = &[
        (
            "  --> src/main.rs:3:5",
            &[("src/main.rs", 3, Some(5), "src/main.rs:3:5")],
        ),
        (
            "a.c:10:2: error: expected ';'",
            &[("a.c", 10, Some(2), "a.c:10:2")],
        ),
        (
            "lib/x.rs:7:fn main() {",
            &[("lib/x.rs", 7, None, "lib/x.rs:7")],
        ),
        (
            "src/a.ts(3,15): error TS2322",
            &[("src/a.ts", 3, Some(15), "src/a.ts(3,15)")],
        ),
        ("Foo.cs(12): warning", &[("Foo.cs", 12, None, "Foo.cs(12)")]),
        (
            r#"  File "/usr/lib/x.py", line 42, in <module>"#,
            &[("/usr/lib/x.py", 42, None, "/usr/lib/x.py")],
        ),
        (
            "./build.sh:3: and ../up/b.go:4:1",
            &[
                ("./build.sh", 3, None, "./build.sh:3"),
                ("../up/b.go", 4, Some(1), "../up/b.go:4:1"),
            ],
        ),
    ];
    for (text, expected) in cases {
        let expected: Vec<_> = expected
            .iter()
            .map(|&(p, l, c, s)| (p.to_string(), l, c, s))
            .collect();
        assert_eq!(found(text), expected, "{text:?}");
    }
}

#[test]
fn leaves_times_ports_and_prose_alone() {
    for text in [
        "finished at 12:30:45",
        "listening on localhost:8080",
        "see https://example.com/a",
        "warning: unused variable: `x`",
        "dir/:3",
        "File \"<stdin>\", line 1",
        "a.rs:0:1",
    ] {
        assert!(found(text).is_empty(), "{text:?}: {:?}", found(text));
    }
}

#[test]
fn links_also_take_bare_paths_and_urls() {
    let line = Line {
        text: "M src/main.rs, see https://example.com/a_(b). (http://x.io/y)".into(),
        ..Line::default()
    };
    let paths: Vec<_> = locations(&line, true)
        .into_iter()
        .map(|l| (l.path, l.line))
        .collect();
    assert!(paths.contains(&("src/main.rs".into(), None)), "{paths:?}");
    let found: Vec<String> = urls(&line).into_iter().map(|u| u.2).collect();
    assert_eq!(found, ["https://example.com/a_(b)", "http://x.io/y"]);
    let none = Line {
        text: "nohttps://x and http:// alone".into(),
        ..Line::default()
    };
    assert!(urls(&none).is_empty());
}

#[test]
fn columns_count_cells_not_chars() {
    let line = Line {
        text: "错误 src/x.rs:1:2".into(),
        ..Line::default()
    };
    let l = &locations(&line, false)[0];
    assert_eq!((l.start, l.end), (5, 17));
}

#[test]
fn a_url_wrapped_over_several_rows_is_taken_whole() {
    let row = |text: &str, wrapped: bool| Line {
        text: text.into(),
        wrapped,
        ..Line::default()
    };
    let rows = [
        row("open https://exam", true),
        row("ple.com/device?co", true),
        row("de=AB-12 now", false),
    ];
    let rows: Vec<&Line> = rows.iter().collect();
    let url = "https://example.com/device?code=AB-12";
    // 鼠标在哪一行，返回的就是网址在那一行里占的格子。
    assert_eq!(url_across(&rows, 0, 8), Some((5, 17, url.into())));
    assert_eq!(url_across(&rows, 1, 0), Some((0, 17, url.into())));
    assert_eq!(url_across(&rows, 2, 7), Some((0, 8, url.into())));
    assert_eq!(url_across(&rows, 0, 2), None);
    assert_eq!(url_across(&rows, 2, 9), None);
}
