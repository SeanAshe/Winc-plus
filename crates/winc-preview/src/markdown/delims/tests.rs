//! `delims` 的测试：LaTeX 定界符换成 `$$`、跳过的地方，以及解析出的公式。

use super::super::{Block, Style, parse};
use super::*;

/// 各块的公式原文；不是公式的块为 `None`。
fn maths(text: &str) -> Vec<Option<String>> {
    let doc = parse(text);
    doc.blocks
        .iter()
        .map(|b| match b {
            Block::Math(latex, _) => Some(latex.clone()),
            _ => None,
        })
        .collect()
}

/// 第一个段落里标为公式的文字。
fn inline_maths(text: &str) -> Vec<String> {
    let doc = parse(text);
    let Some(Block::Paragraph(inline)) = doc.blocks.first() else {
        panic!("不是段落：{:?}", doc.blocks);
    };
    inline
        .runs()
        .filter(|(_, style)| style.has(Style::MATH))
        .map(|(range, _)| inline.text[range].to_owned())
        .collect()
}

#[test]
fn delimiters_become_dollars_of_the_same_length() {
    let text = "a \\(x^2\\) b\n\n\\[\ny\n\\]\n";
    let masked = mask(text);
    assert_eq!(masked, "a $$x^2$$ b\n\n$$\ny\n$$\n");
    assert_eq!(masked.len(), text.len());
    let env = "\\begin{align*}\na &= b\n\\end{align*}\n";
    assert_eq!(mask(env), "$$xxxxxxxxxxxx\na &= b\nxxxxxxxxxx$$\n");
    // 没有定界符时不复制。
    assert!(matches!(mask("plain $x$ text"), Cow::Borrowed(_)));
}

#[test]
fn bracket_and_paren_delimiters_are_math() {
    assert_eq!(
        maths("\\[\na^2 + b^2 = c^2\n\\]\n"),
        [Some("a^2 + b^2 = c^2".to_owned())]
    );
    assert_eq!(
        maths("\\[ A\\mathbf{x} \\]"),
        [Some("A\\mathbf{x}".to_owned())]
    );
    assert_eq!(
        inline_maths("设 \\(x_1\\) 与 \\( y \\) 为实数"),
        ["x_1", " y "]
    );
    // `\(…\)` 跨软换行仍在段落里；`\[…\]` 把段落分开，和 `$$` 一样。
    assert_eq!(inline_maths("a \\(x +\ny\\) b"), ["x +\ny"]);
    assert_eq!(
        maths("前文 \\[x\\] 后文"),
        [None, Some("x".to_owned()), None]
    );
    // 被公式分开的后半段不以空格开头。
    let doc = parse("前文\n\\[x\\]\n后文");
    assert!(
        matches!(doc.blocks.last(), Some(Block::Paragraph(p)) if p.text == "后文"),
        "{:?}",
        doc.blocks
    );
}

#[test]
fn environments_keep_their_begin_and_end() {
    let text = "\\begin{equation}\nE = mc^2 \\label{eq}\n\\end{equation}\n";
    assert_eq!(
        maths(text),
        [Some(
            "\\begin{equation}\nE = mc^2 \\label{eq}\n\\end{equation}".to_owned()
        )]
    );
    let text = "\\begin{align*}\na &= b \\\\\nc &= d\n\\end{align*}";
    assert_eq!(
        maths(text),
        [Some(
            "\\begin{align*}\na &= b \\\\\nc &= d\n\\end{align*}".to_owned()
        )]
    );
    // 不是公式的环境、矩阵这类只能写在公式里的环境不认。
    assert_eq!(maths("\\begin{itemize}\nx\n\\end{itemize}"), [None]);
    assert_eq!(maths("\\begin{pmatrix}\nx\n\\end{pmatrix}"), [None]);
}

#[test]
fn code_escapes_links_and_blank_lines_stay_text() {
    // 行内代码、代码块、链接里的不换。
    assert_eq!(mask("`\\(x\\)`"), "`\\(x\\)`");
    assert_eq!(mask("```\n\\[x\\]\n```\n"), "```\n\\[x\\]\n```\n");
    let link = "[维基](https://en.wikipedia.org/wiki/Set_\\(mathematics\\))";
    assert_eq!(mask(link), link);
    // 转义的反斜杠后面是普通的括号。
    assert_eq!(mask("\\\\(x\\\\)"), "\\\\(x\\\\)");
    // 空行结束段落，公式不能跨过；没有内容的不算。
    assert_eq!(mask("\\[\nx\n\n\\]"), "\\[\nx\n\n\\]");
    assert_eq!(mask("\\(  \\)"), "\\(  \\)");
    // 只有开头没有收尾。
    assert_eq!(mask("\\[1] 参考文献"), "\\[1] 参考文献");
    // 已经用 `$` 写的公式里的不换。
    assert_eq!(mask("$\\(x\\)$"), "$\\(x\\)$");
}

#[test]
fn table_cells_keep_display_math_inline() {
    let doc = parse("| a |\n|---|\n| \\[x\\] |\n");
    let Some(Block::Table(_, rows, _)) = doc.blocks.first() else {
        panic!("不是表格：{:?}", doc.blocks);
    };
    let cell = &rows[1][0];
    let math: Vec<_> = (cell.runs())
        .filter(|(_, style)| style.has(Style::MATH))
        .map(|(range, _)| &cell.text[range])
        .collect();
    assert_eq!(math, ["x"]);
}

#[test]
fn unclosed_openers_only_search_their_paragraph() {
    // 很多没有收尾的 `\[`：每个只在自己那一段里找，不扫到文末。
    let line = "\\[1] 参考文献\n\n";
    let text = line.repeat(20_000);
    let started = std::time::Instant::now();
    assert!(matches!(mask(&text), Cow::Borrowed(_)));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // 收尾在下一段的不算。
    assert_eq!(mask("\\(x\n\ny\\)"), "\\(x\n\ny\\)");
    assert_eq!(mask("\\begin{equation"), "\\begin{equation");
}
