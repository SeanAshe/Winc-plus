//! `editorconfig` 的测试：通配模式的匹配规则、逐级向上查找与优先级、只写一半的缩进设置、
//! 保存前的清理。

use std::fs;

use super::*;

fn glob(pattern: &str, path: &str) -> bool {
    Glob::new(pattern).unwrap().matches(path)
}

#[test]
fn globs_follow_the_editorconfig_rules() {
    assert!(glob("*", "a.py") && glob("*", "src/deep/a.py"));
    assert!(glob("*.py", "src/a.py") && !glob("*.py", "a.pyc"));
    assert!(glob("*.{js,ts}", "web/app.ts") && !glob("*.{js,ts}", "app.rs"));
    assert!(glob("{package.json,.travis.yml}", "package.json"));
    assert!(glob("Makefile", "sub/Makefile"));
    // 含 `/` 的模式从 `.editorconfig` 所在目录起匹配。
    assert!(glob("lib/**.js", "lib/a/b.js") && !glob("lib/**.js", "x/lib/a.js"));
    assert!(glob("a/**/b", "a/b") && glob("a/**/b", "a/x/y/b"));
    assert!(glob("/build/*", "build/out") && !glob("/build/*", "build/a/out"));
    assert!(glob("src/*.rs", "src/main.rs") && !glob("src/*.rs", "src/a/main.rs"));
    assert!(glob("a?c", "abc") && !glob("a?c", "a/c"));
    assert!(glob("[abc].txt", "b.txt") && !glob("[!abc].txt", "b.txt"));
    assert!(glob("[!abc].txt", "d.txt"));
    assert!(glob("file{1..3}.txt", "file2.txt") && !glob("file{1..3}.txt", "file4.txt"));
    assert!(glob("{single}.txt", "{single}.txt"));
    assert!(glob(r"\*.txt", "*.txt") && !glob(r"\*.txt", "a.txt"));
}

#[test]
fn nearer_files_and_later_sections_win() {
    let dir = crate::test_util::scratch("editorconfig");
    fs::create_dir_all(dir.join("web/src")).unwrap();
    fs::write(
        dir.join(".editorconfig"),
        "root = true\n\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = lf\n\
             trim_trailing_whitespace = true\ninsert_final_newline = true\n\n\
             [*.md]\ntrim_trailing_whitespace = false\n\n[Makefile]\nindent_style = tab\n",
    )
    .unwrap();
    fs::write(
        dir.join("web/.editorconfig"),
        "# 子目录里两格\n[*.{js,ts}]\nindent_size = 2\nend_of_line = unset\n",
    )
    .unwrap();

    let py = lookup(&dir.join("a.py"));
    assert_eq!(py.indent(Indent::Tabs), Some(Indent::Spaces(4)));
    assert_eq!(py.tab_width(), Some(4));
    assert_eq!(py.end_of_line, Some(LineEnding::Lf));
    assert!(py.tidies());

    let md = lookup(&dir.join("docs/readme.md"));
    assert_eq!(md.trim_trailing_whitespace, Some(false));
    assert_eq!(md.insert_final_newline, Some(true));

    let make = lookup(&dir.join("Makefile"));
    assert_eq!(make.indent(Indent::Spaces(4)), Some(Indent::Tabs));

    let ts = lookup(&dir.join("web/src/app.ts"));
    assert_eq!(ts.indent(Indent::Tabs), Some(Indent::Spaces(2)));
    assert_eq!(ts.tab_width(), Some(2));
    assert_eq!(ts.end_of_line, None);

    // 向上查找遇到 `root = true` 即停止：更外层的配置不生效。
    fs::write(
        dir.join("web/.editorconfig"),
        "root = true\n[*.ts]\ntab_width = 8\n",
    )
    .unwrap();
    let ts = lookup(&dir.join("web/src/app.ts"));
    assert_eq!(ts.indent(Indent::Tabs), None);
    assert_eq!(ts.tab_width(), Some(8));
    assert!(!ts.tidies());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn half_written_indentation_keeps_the_other_half() {
    let mut config = FileConfig::default();
    config.set("indent_size", "2");
    assert_eq!(config.indent(Indent::Spaces(4)), Some(Indent::Spaces(2)));
    assert_eq!(config.indent(Indent::Tabs), Some(Indent::Tabs));
    assert_eq!(config.tab_width(), Some(2));

    let mut config = FileConfig::default();
    config.set("indent_style", "SPACE");
    assert_eq!(config.indent(Indent::Spaces(3)), Some(Indent::Spaces(3)));
    assert_eq!(config.indent(Indent::Tabs), Some(Indent::Spaces(4)));
    config.set("indent_size", "nonsense");
    assert_eq!(config.indent(Indent::Spaces(3)), Some(Indent::Spaces(3)));
}

#[test]
fn tidying_before_save() {
    let mut config = FileConfig::default();
    config.set("trim_trailing_whitespace", "true");
    config.set("insert_final_newline", "true");
    let text = "a  \r\nb\t\n  \nlast ";
    assert_eq!(tidy(text, &config, LineEnding::Lf, &[]), "a\r\nb\n\nlast\n");
    // `keep` 中的行（第 2 行，从 0 起）保留行尾空白：自动保存时光标所在的行可能正在输入。
    assert_eq!(
        tidy(text, &config, LineEnding::Lf, std::slice::from_ref(&(2..3))),
        "a\r\nb\n  \nlast\n"
    );
    // 多光标时每个光标各保护一段行，`keep` 中的范围互不重叠。
    assert_eq!(
        tidy(text, &config, LineEnding::Lf, &[0..1, 2..4]),
        "a  \r\nb\n  \nlast \n"
    );

    config.set("end_of_line", "crlf");
    assert_eq!(
        tidy(text, &config, LineEnding::Lf, &[]),
        "a\r\nb\r\n\r\nlast\r\n"
    );

    // 空文件不补换行；末尾已有换行时不重复补。
    assert_eq!(tidy("", &config, LineEnding::Lf, &[]), "");
    assert_eq!(tidy("x\r\n", &config, LineEnding::Lf, &[]), "x\r\n");
}
