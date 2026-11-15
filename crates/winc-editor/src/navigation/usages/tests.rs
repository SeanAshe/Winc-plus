//! `usages` 模块的测试：名字的边界、注释与字符串、无关语言、其他同名定义、大小写、局部变量的
//! 出现位置。

use super::*;

/// 返回 `file` 语言中查找 `name` 时，`text` 里各处出现是否算用法：（行号，列号，是否算）。
fn judge(origin: &str, file: &str, name: &str, text: &str) -> Vec<(usize, usize, bool)> {
    let filter = Filter::new(name, winc_syntax::detect(origin)).unwrap();
    let rope = Rope::from_str(text);
    let found = filter.occurrences(&rope);
    let keep = filter.usages(winc_syntax::detect(file), &rope, &found);
    found
        .into_iter()
        .zip(keep)
        .map(|((line, col), keep)| (line, col, keep))
        .collect()
}

#[test]
fn comments_strings_and_other_definitions_are_not_usages() {
    let text = "struct Item0 {}\n// Item0 is old\n/* Item0\n   Item0 */\nlet s = \"Item0\";\nfn f(x: Item0) {}\n";
    assert_eq!(
        judge("a.rs", "b.rs", "Item0", text),
        [
            (0, 7, false),
            (1, 3, false),
            (2, 3, false),
            (3, 3, false),
            (4, 9, false),
            (5, 8, true),
        ]
    );
}

#[test]
fn unrelated_languages_are_skipped() {
    let filter = Filter::new("helper", winc_syntax::detect("a.ts")).unwrap();
    let rope = Rope::from_str("x = helper() + 1\n");
    for (file, expected) in [
        ("b.js", true),
        ("App.vue", true),
        ("README.md", false),
        ("data.json", false),
        ("x.py", false),
    ] {
        let language = winc_syntax::detect(file);
        assert_eq!(
            filter.usages(language, &rope, &[(0, 4)]),
            [expected],
            "{file}"
        );
    }
    // C 与 C++ 互相引用。
    let filter = Filter::new("run", winc_syntax::detect("a.h")).unwrap();
    assert_eq!(
        filter.usages(
            winc_syntax::detect("b.cpp"),
            &Rope::from_str("run();"),
            &[(0, 0)]
        ),
        [true]
    );
}

#[test]
fn names_follow_the_language() {
    // CSS 类名含 `-`：`my-class-x` 中的 `my-class` 不算；HTML 属性值中的类名算。
    let text = "<div class=\"my-class my-class-x\"></div>\n";
    let filter = Filter::new("my-class", winc_syntax::detect("a.css")).unwrap();
    let rope = Rope::from_str(text);
    let found = filter.occurrences(&rope);
    assert_eq!(found, [(0, 12)]);
    assert_eq!(
        filter.usages(winc_syntax::detect("a.html"), &rope, &found),
        [true]
    );

    // Ruby 的 `valid?` 与 `valid` 是两个名字；PHP 的 `$total` 与函数 `total` 是两个名字。
    assert_eq!(
        judge("a.rb", "a.rb", "valid", "valid? || valid\n"),
        [(0, 10, true)]
    );
    assert_eq!(
        judge("a.php", "a.php", "$total", "$total = total($total);\n"),
        [(0, 0, false), (0, 15, true)]
    );
}

#[test]
fn case_insensitive_languages() {
    // SQL 的表名不区分大小写。
    assert_eq!(
        judge(
            "a.sql",
            "b.sql",
            "users",
            "SELECT * FROM USERS JOIN users_log\n"
        ),
        [(0, 14, true)]
    );
    // PHP 的变量区分大小写，函数不区分。
    let php = winc_syntax::detect("a.php");
    assert!(!Filter::new("$x", php).unwrap().ignore_case());
    assert!(Filter::new("render", php).unwrap().ignore_case());
}

#[test]
fn locals_are_scoped_to_their_function() {
    let scope = |file: &str, text: &str, line: usize| {
        let language = winc_syntax::detect(file);
        let rope = Rope::from_str(text);
        let blocks = crate::scopes::fold_ranges(&rope, language, 4, &mut |_| false);
        function_scope(&rope, language, &blocks, line)
    };
    let rust = "fn a() {\n    let x = 1;\n    if x > 0 {\n        let y = x;\n    }\n}\n\nfn b() {\n    let x = 2;\n}\n";
    assert_eq!(scope("a.rs", rust, 1), Some((0, 4)));
    // `if` 块中的定义取所在的函数，而不是 `if` 块。
    assert_eq!(scope("a.rs", rust, 3), Some((0, 4)));
    assert_eq!(scope("a.rs", rust, 8), Some((7, 8)));

    // Python 中 `if` 里赋值的变量在函数中的其他位置同样可见。
    let python = "def f():\n    if ok:\n        total = 1\n    return total\n";
    assert_eq!(scope("a.py", python, 2), Some((0, 3)));
    // 不在函数中：整个文件。
    assert_eq!(scope("a.py", "total = 1\nprint(total)\n", 0), None);
}

/// C：`enum` 块中的成员是定义，逐行书写的实参和类型名是用法。
#[test]
fn c_enumerators_and_type_names() {
    let text = "typedef enum {\n    STATE_RUN,\n} state_t;\nstatic state_t s;\nvoid f(void) {\n    g(\n        STATE_RUN,\n        s\n    );\n    return s;\n}\n";
    assert_eq!(
        judge("a.c", "b.c", "STATE_RUN", text),
        [(1, 4, false), (6, 8, true)]
    );
    assert_eq!(
        judge("a.c", "b.c", "state_t", text),
        [(2, 2, false), (3, 7, true)]
    );

    let filter = Filter::new("STATE_RUN", winc_syntax::detect("a.c")).unwrap();
    let rope = Rope::from_str(text);
    let c = winc_syntax::detect("b.c");
    assert_eq!(
        filter.definitions(c, &rope, &[(1, 0), (6, 0)]),
        [true, false]
    );
    let filter = Filter::new("s", winc_syntax::detect("a.c")).unwrap();
    assert_eq!(
        filter.definitions(c, &rope, &[(3, 0), (9, 0)]),
        [true, false]
    );
}
