//! LaTeX 的预览：转换成 Markdown 后排版，预览只读，分栏时按原文的行对齐滚动。

use super::*;

/// 导言区较长的 LaTeX 样本：转换后导言区不占行，原文与结果的行号相差很多。
fn paper() -> String {
    let mut text = String::from("\\documentclass{article}\n");
    for i in 0..40 {
        text.push_str(&format!("\\newcommand{{\\m{i}}}{{x}}\n"));
    }
    text.push_str(
        r"\title{示例论文}
\begin{document}
\maketitle
\section{引言}
设 $a^2+b^2=c^2$。

",
    );
    for i in 0..60 {
        text.push_str(&format!("第 {i} 段。\n\n"));
    }
    text.push_str(
        "\\section{结论}\n\\begin{itemize}\n\\item 一\n\\end{itemize}\n\\[ E = mc^2 \\]\n",
    );
    // 「结论」之后还有足够的内容，预览能滚到它的位置。
    for i in 0..60 {
        text.push_str(&format!("后记 {i}。\n\n"));
    }
    text.push_str("\\end{document}\n");
    text
}

/// 等找完 TeX（没有找到）、换成阅读视图，再排好版。
fn settle_without_tex(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.latex_waiting() {
        assert!(std::time::Instant::now() < deadline, "仍在等编译");
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.advance_previews();
    }
    settle(app);
}

#[test]
fn latex_previews_read_only_and_scrolls_by_source_lines() {
    let dir = sample_folder("latex-preview");
    let path = dir.join("paper.tex");
    let text = paper();
    std::fs::write(&path, &text).unwrap();
    let mut app = app_with(&path, false);
    // 没有 TeX：显示转换出的阅读视图。
    app.config.latex_path = Some("/no/such/latexmk".to_owned());
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_kind(), Some(winc_editor::PreviewKind::Latex));

    // 只看预览：标题、章节和公式都排了版，导言区的定义不显示；预览不能编辑。
    app.set_preview_mode(PreviewMode::Preview);
    settle_without_tex(&mut app);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(
        shown.contains("示例论文") && shown.contains("结论"),
        "{shown}"
    );
    assert!(!shown.contains("newcommand"), "{shown}");
    assert!(!app.markdown_live());
    let parsed = app.preview.markdown.parsed.clone().unwrap();
    let maths = parsed
        .blocks
        .iter()
        .filter(|b| matches!(b, winc_preview::markdown::Block::Math(..)));
    assert_eq!(maths.count(), 1);
    app.run(Command::Insert("x".into()));
    assert_eq!(doc_text(&app), text);

    // 分栏：左侧滚到 `\section{结论}` 那一行，右侧滚到「结论」标题处。
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    let line = text.lines().position(|l| l == "\\section{结论}").unwrap();
    app.scroll_text_to(line as f32);
    settle(&mut app);
    let heading = crate::app::preview::markdown::laid::anchor_top(&app, "结论").unwrap();
    let scroll = app.preview.markdown.scroll();
    assert!(
        (scroll - heading).abs() < 2.0 * app.fonts.line_height,
        "{scroll} {heading}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：一篇常见写法的 LaTeX 论文只看预览（`latex-preview`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_latex_preview() {
    let dir = sample_folder("latex-shot");
    let path = dir.join("paper.tex");
    let text = r"\documentclass{article}
\usepackage{amsmath,amsthm}
\newtheorem{thm}{定理}
\title{勾股定理的几种证明}
\author{张三 \and 李四}
\date{2026 年 11 月}
\begin{document}
\maketitle
\begin{abstract}
本文整理了勾股定理的三种证明，并给出 \LaTeX{} 写法的对照。
\end{abstract}

\section{引言}
在直角三角形中，设直角边为 $a$、$b$，斜边为 $c$，则有\footnote{又称毕达哥拉斯定理。}
\begin{equation}
a^2 + b^2 = c^2 \label{eq:pyth}
\end{equation}
式 \eqref{eq:pyth} 是 \textbf{欧氏几何} 的基本结论，见 \cite{euclid}。

\begin{thm}[勾股]
直角三角形两直角边的平方和等于斜边的平方。
\end{thm}
\begin{proof}
由面积关系 \( (a+b)^2 = 4 \cdot \tfrac{1}{2}ab + c^2 \) 即得。
\end{proof}

\section{几种证明}
\begin{enumerate}
  \item \emph{面积法}：拼成正方形；
  \item 相似三角形法；
  \item 向量法：
  \begin{itemize}
    \item 内积 $\mathbf{a} \cdot \mathbf{b} = 0$
  \end{itemize}
\end{enumerate}

\begin{tabular}{lcr}
\hline
方法 & 难度 & 篇幅 \\
\hline
面积法 & 易 & 短 \\
向量法 & 中 & 中 \\
\hline
\end{tabular}

\begin{verbatim}
c = sqrt(a**2 + b**2)
\end{verbatim}
\end{document}
";
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    app.config.latex_path = Some("/no/such/latexmk".to_owned());
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle_without_tex(&mut app);
    shoot(&mut app, "latex-preview");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用脚本冒充的 `latexmk`：写出一份两页的 PDF、`SyncTeX` 对照和带一条错误的日志，以失败退出。
/// 返回脚本的路径。
#[cfg(target_os = "macos")]
fn fake_latexmk(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::write(dir.join("sample.pdf"), super::sample_pdf(2)).unwrap();
    // 第 3 行排在第 1 页纵 119 bp，第 5 行排在第 2 页纵 300 bp（换算成 sp）。
    let sp = |bp: f64| (bp * 65781.76).round() as i64;
    let synctex = format!(
        "SyncTeX Version:1\nInput:1:./main.tex\nMagnification:1000\nUnit:1\nX Offset:0\n\
         Y Offset:0\nContent:\n{{1\nx1,3:{},{}\n}}1\n{{2\nx1,5:{},{}\n}}2\n",
        sp(100.0),
        sp(119.0),
        sp(100.0),
        sp(300.0),
    );
    std::fs::write(dir.join("sample.synctex"), synctex).unwrap();
    let script = dir.join("bin/latexmk");
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let d = dir.display();
    let lines = [
        "#!/bin/sh".to_owned(),
        r#"for a in "$@"; do case "$a" in -outdir=*) out="${a#-outdir=}";; esac; done"#.to_owned(),
        r#"mkdir -p "$out""#.to_owned(),
        // 像 `tectonic` 一样先在标准错误报告下载，停顿，测试能看到进度。
        "echo 'note: downloading tcolorbox.sty' >&2".to_owned(),
        "sleep 0.3".to_owned(),
        format!(r#"cp '{d}/sample.pdf' "$out/main.pdf""#),
        format!(r#"cp '{d}/sample.synctex' "$out/main.synctex.gz""#),
        r#"printf './main.tex:4: Undefined control sequence.\nl.4 \\oops\n' > "$out/main.log""#
            .to_owned(),
        "exit 1".to_owned(),
    ];
    winc_backend::testing::write_script(&script, &(lines.join("\n") + "\n"));
    script
}

/// 推进预览（开始编译），等后台的编译完成，期间仍然绘制。
#[cfg(target_os = "macos")]
fn wait_for_latex(app: &mut App) {
    app.advance_previews();
    // 真的 TeX 第一次编译要下载宏包，留足时间。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while app.preview.latex.running() {
        assert!(std::time::Instant::now() < deadline, "编译没有结束");
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.advance_previews();
        paint(app);
    }
    app.advance_previews();
    app.wait_for_pdf();
    paint(app);
}

#[cfg(target_os = "macos")]
#[test]
fn compiled_pdfs_replace_the_reading_view() {
    let dir = sample_folder("latex-compile");
    let main = dir.join("main.tex");
    // 后面补上足够多的空行，左侧能滚到第 5 行。
    let text = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n第一页\n\\oops\n第二页\n\
            {}\\end{{document}}\n",
        "\n".repeat(120)
    );
    std::fs::write(&main, text).unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    paint(&mut app);
    let id = app.editor.active().unwrap().id;

    // 分栏：第一次编译时显示说明页（不先显示阅读视图），显示进行到哪一步；编译完换成 PDF。
    app.set_preview_mode(PreviewMode::Split);
    paint(&mut app);
    assert!(app.latex_pdf().is_none() && app.latex_waiting());
    assert!(crate::app::preview::markdown::shown_area(&app).is_none());
    app.advance_previews();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(app.latex_status()).is_some_and(|(text, _)| text.contains("下载宏包 tcolorbox.sty"))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "{:?}",
            app.latex_status()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.advance_previews();
    }
    wait_for_latex(&mut app);
    assert!(!app.latex_waiting());
    assert!(app.latex_pdf().is_some());
    assert!(crate::app::preview::markdown::shown_area(&app).is_none());
    assert!(matches!(&app.preview.pdf.pages, Some(Ok(pages)) if pages.len() == 2));

    // 出错时仍显示这次写出的 PDF；右上角写错误数，行号栏标出第 4 行，点击跳过去。
    assert_eq!(app.latex_status(), Some(("1 个错误".to_owned(), true)));
    assert_eq!(app.latex_problem_lines(id), [3]);
    // 悬停在行号栏的红点上：说明这一行的错误。
    let (l, lh) = (app.layout, app.fonts.line_height);
    let dot = (
        l.gutter.x + app.blame_width() + crate::theme::GUTTER_PADDING / 2.0 + 1.0,
        l.text.y + 3.5 * lh - app.editor.active().unwrap().scroll_line as f32 * lh,
    );
    app.on_mouse_move(dot.0, dot.1);
    assert_eq!(app.pointer.hover, Hover::LatexProblem(3));
    let (_, tip) = app.latex_problem_tip(3).unwrap();
    assert!(tip.contains("Undefined control sequence"), "{tip}");
    app.on_mouse_move(0.0, 0.0);
    // 点错误数列出各处错误，选一项跳过去。
    assert_eq!(app.latex_problem_count(), 1);
    assert!(
        app.latex_problem_label(0)
            .title
            .contains("Undefined control sequence")
    );
    assert_eq!(
        app.latex_problem_label(0).subtitle.as_deref(),
        Some("main.tex:4")
    );
    app.show_latex_problem(0);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head).0, 3);

    // 左侧滚到第 5 行：右侧滚到它排出的第 2 页。
    app.scroll_text_to(4.0);
    app.advance_previews();
    paint(&mut app);
    // 对齐到第 2 页纵 300 bp 往上让出一行处；内容不够高时停在最底。
    let pdf = &app.preview.pdf;
    let want = (pdf.place_y(1, (300.0 - 12.0) / 792.0).unwrap()).min(pdf.max_scroll());
    assert!((pdf.scroll - want).abs() < 1.0, "{} {want}", pdf.scroll);
    assert!(pdf.scroll > pdf.place_y(0, 0.5).unwrap());

    // ⌘ 点击第 1 页上第 3 行排出的位置：光标回到第 3 行。
    app.preview.pdf.scroll = 0.0;
    paint(&mut app);
    let area = app.preview.pdf.area;
    let top = app.preview.pdf.place_y(0, 119.0 / 792.0).unwrap() + crate::app::preview::pdf::GAP;
    assert!(app.latex_source_at(area.x + area.w / 2.0, area.y + top));
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head).0, 2);

    // 保存后重新编译。
    app.latex_saved(id);
    assert_eq!(app.latex_status().map(|s| s.1), Some(false));
    wait_for_latex(&mut app);
    assert!(app.latex_pdf().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn a_declined_latexmkrc_can_be_asked_about_again() {
    use winit::keyboard::{Key, NamedKey};
    let dir = sample_folder("latex-rc-skipped");
    let main = dir.join("main.tex");
    let text = "\\documentclass{article}\n\\begin{document}\n正文\n\\oops\n\\end{document}\n";
    std::fs::write(&main, text).unwrap();
    std::fs::write(dir.join("latexmkrc"), "$pdf_mode = 5;\n").unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    paint(&mut app);
    let id = app.editor.active().unwrap().id;
    app.set_preview_mode(PreviewMode::Split);
    paint(&mut app);

    // 第一次编译没有运行它，编译完询问；取消后同一份内容不再自动询问。
    wait_for_latex(&mut app);
    assert!(app.overlays.alert.is_some());
    assert!(app.alert_key(&Key::Named(NamedKey::Escape), false));
    app.latex_saved(id);
    wait_for_latex(&mut app);
    assert!(app.overlays.alert.is_none());

    // 错误列表的末尾留有再问一次的入口。
    assert!(app.latex_rc_skipped());
    app.open_latex_problems();
    app.run_menu_command(MenuCommand::LatexRunRc);
    assert!(app.overlays.alert.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_tex_the_reading_view_stays() {
    let dir = sample_folder("latex-missing");
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n正文\n\\end{document}\n",
    )
    .unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some("/no/such/latexmk".to_owned());
    app.set_preview_mode(PreviewMode::Split);
    settle_without_tex(&mut app);
    assert!(app.latex_pdf().is_none());
    assert_eq!(app.latex_status(), None);
    assert!(crate::app::preview::markdown::shown_area(&app).is_some());
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("正文"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：分栏显示编译出的 PDF，右上角写错误数，行号栏标出出错的行（`latex-pdf`）。
#[cfg(target_os = "macos")]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_latex_pdf() {
    let dir = sample_folder("latex-pdf-shot");
    let main = dir.join("main.tex");
    let text =
        "\\documentclass{article}\n\\begin{document}\n第一页\n\\oops\n第二页\n\\end{document}\n";
    std::fs::write(&main, text).unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    paint(&mut app);
    wait_for_latex(&mut app);
    let bar = app.hits.latex_status;
    app.on_mouse_move(bar.x + bar.w / 2.0, bar.y + bar.h / 2.0);
    paint(&mut app);
    shoot(&mut app, "latex-pdf");
    // 点错误数：下方列出各处错误。
    app.open_latex_problems();
    paint(&mut app);
    shoot(&mut app, "latex-problems");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用真的 TeX 编译：环境变量 `WINC_TEST_TECTONIC` 给出 `tectonic` 程序时运行，截图
/// `latex-real`。两页的文档，第 4 行有一处错误。
#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 TeX，手动运行"]
fn real_tex_compiles_and_syncs() {
    let Ok(program) = std::env::var("WINC_TEST_TECTONIC") else {
        return;
    };
    let dir = sample_folder("latex-real");
    let main = dir.join("main.tex");
    let paragraph = "The quick brown fox jumps over the lazy dog. ".repeat(12);
    let body: String = (0..40)
        .map(|i| format!("Paragraph {i}. {paragraph}\n\n"))
        .collect();
    let text = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\nFirst line.\n\\oops\n\
         {body}\\end{{document}}\n"
    );
    std::fs::write(&main, &text).unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some(program);
    paint(&mut app);
    let id = app.editor.active().unwrap().id;
    app.set_preview_mode(PreviewMode::Split);
    paint(&mut app);
    wait_for_latex(&mut app);
    assert!(app.latex_pdf().is_some(), "{:?}", app.latex_status_tip());
    assert!(matches!(&app.preview.pdf.pages, Some(Ok(pages)) if pages.len() >= 2));
    assert_eq!(app.latex_status(), Some(("1 个错误".to_owned(), true)));
    assert_eq!(app.latex_problem_lines(id), [3]);

    // 左侧把第 25 段滚到顶端：右侧滚到它所在的页，⌘ 点击那里回到同一段。
    let last = text
        .lines()
        .position(|l| l.starts_with("Paragraph 25."))
        .unwrap();
    app.scroll_text_to(last as f32);
    app.advance_previews();
    paint(&mut app);
    let pdf = &app.preview.pdf;
    let area = pdf.area;
    let (page, _, _) = pdf
        .page_at(area.x + area.w / 2.0, area.y + area.h / 3.0)
        .unwrap();
    assert!(page >= 1, "{page}");
    let y = area.y + crate::app::preview::pdf::GAP + 24.0;
    assert!(app.latex_source_at(area.x + area.w / 2.0, y));
    let doc = app.editor.active().unwrap();
    let line = doc.text().pos_to_line_col(doc.selection.head).0;
    assert_eq!(line, last);
    app.wait_for_pdf();
    paint(&mut app);
    shoot(&mut app, "latex-real");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 等设置页找完 TeX 程序。
#[cfg(target_os = "macos")]
fn wait_for_tool(app: &mut App) {
    for _ in 0..400 {
        app.advance_previews();
        if app.latex_tool_label() != "自动查找" {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// 打开设置时找一次 TeX 程序，「LaTeX 程序」一行留空时写出找到的程序或没找到。
#[cfg(target_os = "macos")]
#[test]
fn settings_show_which_tex_was_found() {
    let dir = sample_folder("latex-settings");
    let mut app = app_with(&dir.join("a.txt"), false);
    assert_eq!(app.latex_tool_label(), "自动查找");
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    app.open_settings();
    wait_for_tool(&mut app);
    assert_eq!(app.latex_tool_label(), "自动：latexmk");
    // 改成不存在的程序：随即重新找，写「未找到」。
    app.set_latex_path(Some("/no/such/latexmk".to_owned()));
    wait_for_tool(&mut app);
    assert_eq!(app.latex_tool_label(), "未找到程序");
    // 找程序还没回来时又改了设置：按旧设置找到的结果不算。
    app.set_latex_path(None);
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    app.find_latex_tool();
    app.set_latex_path(Some("/no/such/latexmk".to_owned()));
    wait_for_tool(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(200));
    app.advance_previews();
    assert_eq!(app.latex_tool_label(), "未找到程序");
    // 在这一行里填路径：点击后是输入框，回车保存，随即按它找程序。
    let script = fake_latexmk(&dir).display().to_string();
    app.settings_start_editing(crate::layout::Setting::LatexPath);
    let editing = app
        .settings
        .editing
        .as_ref()
        .map(|(_, f)| f.text().to_string());
    assert_eq!(editing.as_deref(), Some("/no/such/latexmk"));
    app.edit_field(|f| {
        f.set_text(&script);
        true
    });
    app.settings_commit_field();
    assert_eq!(app.config.latex_path.as_deref(), Some(script.as_str()));
    wait_for_tool(&mut app);
    assert_eq!(app.latex_tool_label(), "自动：latexmk");
    // 清空回到自动查找。
    app.settings_start_editing(crate::layout::Setting::LatexPath);
    app.edit_field(|f| {
        f.set_text("");
        true
    });
    app.settings_commit_field();
    assert_eq!(app.config.latex_path, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：设置里「LaTeX 程序」一行，自动找到了程序（`latex-setting`）。
#[cfg(target_os = "macos")]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_latex_setting() {
    let dir = sample_folder("latex-setting-shot");
    let mut app = app_with(&dir.join("a.txt"), false);
    let script = fake_latexmk(&dir);
    app.config.latex_path = Some(script.display().to_string());
    app.open_settings();
    wait_for_tool(&mut app);
    // 找到之后清空设置：一行显示自动找到的程序。
    app.config.latex_path = None;
    app.focus_settings_search();
    app.edit_field(|f| {
        f.set_text("LaTeX");
        true
    });
    paint(&mut app);
    shoot(&mut app, "latex-setting");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：第一次编译时的说明页，显示正在下载的宏包（`latex-waiting`）。
#[cfg(target_os = "macos")]
#[test]
#[ignore = "截图，手动运行"]
fn shoot_latex_waiting() {
    let dir = sample_folder("latex-waiting-shot");
    let main = dir.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n",
    )
    .unwrap();
    let mut app = app_with(&main, false);
    app.config.latex_path = Some(fake_latexmk(&dir).display().to_string());
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Split);
    app.advance_previews();
    while !(app.latex_status()).is_some_and(|(text, _)| text.contains("tcolorbox")) {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.advance_previews();
    }
    paint(&mut app);
    shoot(&mut app, "latex-waiting");
    wait_for_latex(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 编译状态换了文字：宽度从当前宽度过渡到新宽度；刚出现、动画关闭时直接用新宽度。
#[test]
fn status_width_eases_between_texts() {
    let dir = sample_folder("latex-status-width");
    let mut app = app_with(&dir.join("a.txt"), false);
    // 刚出现：直接是文字的宽度。
    assert_eq!(app.latex_status_width(120.0), 120.0);
    // 测试里没有窗口，不播放动画：换了文字直接是新宽度。
    assert_eq!(app.latex_status_width(200.0), 200.0);
    assert!(app.preview.latex.status_motion.is_none());
    // 过渡中按时钟取两者之间的宽度。
    let clock = crate::app::animation::Clock::new(std::time::Duration::from_secs(60));
    app.preview.latex.status_motion = Some((200.0, 80.0, clock));
    let shown = app.latex_status_width(80.0);
    assert!((80.0..=200.0).contains(&shown), "{shown}");
    // 不显示以后再出现：不从旧宽度过渡。
    app.hide_latex_status();
    assert_eq!(app.latex_status_width(150.0), 150.0);
    let _ = std::fs::remove_dir_all(&dir);
}
