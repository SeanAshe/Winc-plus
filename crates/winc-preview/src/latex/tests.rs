//! `latex` 的测试：转换出的 Markdown 和原文各行对应的结果行。

use super::*;

fn md(text: &str) -> String {
    to_markdown(text).markdown
}

/// 去掉每行行尾的空白和多余的空行，便于比较。
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut out: Vec<&str> = Vec::new();
    for line in lines {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last() == Some(&"") {
        out.pop();
    }
    out.join("\n")
}

#[test]
fn preamble_gives_the_title_block_and_body_follows() {
    let text = r"\documentclass{article}
\usepackage{amsmath}
\title{论文}
\author{张三 \and 李四}
\date{2026}
\begin{document}
\maketitle
\section{引言}
正文 % 注释
第二行。
\end{document}
之后不显示
";
    assert_eq!(
        tidy(&md(text)),
        "# 论文\n\n张三, 李四\n\n2026\n\n## 引言\n正文 第二行。"
    );
}

#[test]
fn styles_quotes_dashes_and_escapes() {
    assert_eq!(
        tidy(&md(
            "\\textbf{粗} \\emph{斜} \\texttt{a\\_b} \\underline{线}"
        )),
        "**粗** *斜* `a_b` <u>线</u>"
    );
    assert_eq!(
        tidy(&md("``引号'' 和 `单' 1--2 a---b")),
        "“引号” 和 ‘单’ 1–2 a—b"
    );
    // Markdown 的记号字符转义，方括号写成字符引用，不会成为 `\[` 公式。
    assert_eq!(
        tidy(&md("a*b* [1] 50\\% \\& \\$5")),
        "a\\*b\\* &#91;1&#93; 50% \\& \\$5"
    );
    // 行首的数字加句点不成编号列表。
    assert_eq!(tidy(&md("2026. 年")), "2026\\. 年");
    assert_eq!(tidy(&md("caf\\'e na\\\"{\\i}ve \\c{c}")), "café naïve ç");
    assert_eq!(tidy(&md("\\LaTeX{} 与 \\TeX")), "LaTeX 与 TeX");
}

#[test]
fn math_keeps_its_source_with_bracket_delimiters() {
    assert_eq!(
        tidy(&md("设 $ x + y $ 与 $$a=b$$")),
        "设 \\( x + y \\) 与 \\[a=b\\]"
    );
    assert_eq!(
        tidy(&md(
            "\\begin{align*}\na &= b \\\\ % 注释\nc &= d\n\\end{align*}"
        )),
        "\\begin{align*}\na &= b \\\\\nc &= d\n\\end{align*}"
    );
    assert_eq!(tidy(&md("\\[ \\frac{1}{2} \\]")), "\\[ \\frac{1}{2} \\]");
    // 转换出的公式经 Markdown 预览解析后是公式。
    let doc = crate::markdown::parse(&md("\\begin{equation}\nE = mc^2\n\\end{equation}"));
    assert!(matches!(
        doc.blocks.as_slice(),
        [crate::markdown::Block::Math(..)]
    ));
}

#[test]
fn lists_nest_inside_their_items() {
    let text = r"\begin{itemize}
  \item 一
  \begin{enumerate}
    \item 甲
    \item 乙
  \end{enumerate}
  \item[\textbf{注}] 二
\end{itemize}
后文";
    assert_eq!(
        tidy(&md(text)),
        "- 一\n  1. 甲\n  2. 乙\n- **注** 二\n\n后文"
    );
    let text = "\\begin{description}\n\\item[术语] 解释\n\\end{description}";
    assert_eq!(tidy(&md(text)), "- **术语** 解释");
}

#[test]
fn tables_become_pipe_tables() {
    let text = r"\begin{tabular}{|l|c|r|}
\hline
名 & 值 & 备注 \\
\hline
a & $1$ & \multicolumn{1}{c}{x} \\
b & 2 \\
\hline
\end{tabular}";
    assert_eq!(
        tidy(&md(text)),
        "| 名 | 值 | 备注 |\n| :-- | :-: | --: |\n| a | \\(1\\) | x |\n| b | 2 |  |"
    );
}

#[test]
fn code_figures_links_notes_and_theorems() {
    let text = "\\begin{lstlisting}[language=Python]\nprint(\"%\")\n\\end{lstlisting}";
    assert_eq!(tidy(&md(text)), "```python\nprint(\"%\")\n```");
    let text = r"\begin{figure}[h]
\centering
\includegraphics[width=.5\textwidth]{img/a.png}
\caption{示意}\label{fig:a}
\end{figure}
见图 \ref{fig:a}。";
    assert_eq!(
        tidy(&md(text)),
        "![](<img/a.png>)\n\n*示意*\n\n见图 fig:a。"
    );
    assert_eq!(
        tidy(&md(
            "\\href{https://a.dev/x\\_y}{网站} 与 \\url{https://b.dev}"
        )),
        "[网站](<https://a.dev/x_y>) 与 [https://b.dev](<https://b.dev>)"
    );
    assert_eq!(
        tidy(&md("正文\\footnote{脚注} 引用 \\cite{knuth, lamport}")),
        "正文[^1] 引用 &#91;knuth, lamport&#93;\n\n[^1]: 脚注"
    );
    let text = r"\newtheorem{thm}{定理}
\begin{thm}[勾股]
直角三角形。
\end{thm}
\begin{proof}
显然。
\end{proof}";
    assert_eq!(
        tidy(&md(text)),
        "**定理 (勾股).** 直角三角形。\n\n*Proof.* 显然。 ∎"
    );
    assert_eq!(
        tidy(&md("\\input{chapters/intro}")),
        "[chapters/intro](<chapters/intro.tex>)"
    );
}

#[test]
fn unknown_commands_keep_their_last_argument() {
    assert_eq!(
        tidy(&md("\\textcolor{red}{红} \\foo[x]{a}{b} \\bar 字")),
        "红 b 字"
    );
    assert_eq!(tidy(&md("\\vspace{1em}\\noindent 正文")), "正文");
}

#[test]
fn source_lines_map_to_markdown_lines() {
    let text = r"\documentclass{article}
\begin{document}
第一段

\section{标题}
第二段
\end{document}
";
    let out = to_markdown(text);
    let line_of = |needle: &str| {
        out.markdown
            .lines()
            .position(|l| l.contains(needle))
            .unwrap()
    };
    // 导言区都对应第 0 行；正文各行对应它的内容所在的行。
    let map = &out.map;
    assert_eq!(map.lines[0], 0);
    assert_eq!(map.lines[2], line_of("第一段"));
    assert_eq!(map.lines[4], line_of("## 标题"));
    assert_eq!(map.lines[5], line_of("第二段"));
    assert_eq!(map.lines.len(), text.matches('\n').count() + 1);
    assert_eq!(map.line_of(2.0), line_of("第一段") as f64);
    assert_eq!(map.count, out.markdown.lines().count() + 1);
}

#[test]
fn deep_nesting_and_unclosed_input_do_not_panic() {
    // 超过嵌套上限的一层整个跳过。
    let deep = format!("{}x{}", "{".repeat(5000), "}".repeat(5000));
    assert!(!md(&deep).contains(['{', '}']));
    for text in [
        "\\begin{itemize}\n\\item a",
        "$x",
        "\\textbf{a",
        "\\begin{tabular}{l",
        "\\",
    ] {
        let _ = md(text);
    }
}
