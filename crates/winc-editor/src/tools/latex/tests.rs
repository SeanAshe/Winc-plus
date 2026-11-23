//! `latex` 的测试：找程序、`% !TEX` 指令、主文件、引擎和编译命令。不需要装 TeX。

use super::*;
#[cfg(unix)]
use crate::test_util::scratch;
#[cfg(unix)]
use crate::tools::format::tests::fake_tool;

#[cfg(unix)] // 用不带扩展名的脚本冒充程序；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn programs_are_found_in_order_of_preference() {
    let root = scratch("latex-find");
    let bin = root.join("bin");
    let shell = || Shell::with_vars(vec![("PATH".into(), bin.clone().into())]);
    assert_eq!(find(&shell(), None), Err(Missing::NotInstalled));

    // 只有引擎：列出装了的引擎。
    let xe = fake_tool(&bin, "xelatex", "true");
    let pdf = fake_tool(&bin, "pdflatex", "true");
    assert_eq!(
        find(&shell(), None),
        Ok(Toolchain::Engines(vec![
            (Engine::Pdf, pdf),
            (Engine::Xe, xe)
        ]))
    );
    // `tectonic` 优先于引擎，`latexmk` 又优先于它。
    let tectonic = fake_tool(&bin, "tectonic", "true");
    assert_eq!(
        find(&shell(), None),
        Ok(Toolchain::Tectonic(tectonic.clone()))
    );
    let latexmk = fake_tool(&bin, "latexmk", "true");
    assert_eq!(find(&shell(), None), Ok(Toolchain::Latexmk(latexmk)));

    // 设置的程序按名字认；不存在时说明是哪个。
    assert_eq!(
        find(&shell(), Some(" tectonic ")),
        Ok(Toolchain::Tectonic(tectonic))
    );
    let path = bin.join("xelatex");
    assert_eq!(
        find(&shell(), Some(path.to_str().unwrap())),
        Ok(Toolchain::Engines(vec![(Engine::Xe, path)]))
    );
    assert_eq!(
        find(&shell(), Some("/no/such/latexmk")),
        Err(Missing::NotFound("/no/such/latexmk".into()))
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn magic_comments_are_read_from_the_first_lines() {
    let text = "% !TEX root = ../main.tex\n%!TeX TS-program = XeLaTeX\n\\section{a}\n";
    assert_eq!(magic(text, "root"), Some("../main.tex"));
    assert_eq!(magic(text, "program"), Some("XeLaTeX"));
    assert_eq!(magic(text, "spellcheck"), None);
    // 太靠后的、不在注释里的不算。
    let late = format!("{}% !TEX root = a.tex\n", "x\n".repeat(MAGIC_LINES));
    assert_eq!(magic(&late, "root"), None);
    assert_eq!(magic("!TEX root = a.tex", "root"), None);
}

#[test]
fn the_root_is_the_document_that_includes_the_file() {
    let main = PathBuf::from("/p/main.tex");
    let chapter = PathBuf::from("/p/chapters/intro.tex");
    let main_text = "\\documentclass{book}\n% \\input{chapters/old}\n\\include{chapters/intro}\n";
    let other = (
        PathBuf::from("/p/other.tex"),
        "\\documentclass{article}\n".to_owned(),
    );
    let candidates = vec![other, (main.clone(), main_text.to_owned())];
    // 本身是完整的文档：编译自己。
    assert_eq!(root_of(&main, main_text, &candidates), Some(main.clone()));
    // 章节文件：找引入它的文档；注释掉的引入不算。
    assert_eq!(
        root_of(&chapter, "\\section{引言}", &candidates),
        Some(main.clone())
    );
    let old = PathBuf::from("/p/chapters/old.tex");
    assert_eq!(root_of(&old, "x", &candidates), None);
    // `% !TEX root` 优先，按文件所在的目录解析。
    let text = "% !TEX root = ../main.tex\n\\section{a}";
    assert_eq!(root_of(&chapter, text, &[]), Some(main));
    // 注释掉的 `\documentclass` 不算完整的文档。
    assert_eq!(root_of(&old, "% \\documentclass{article}", &[]), None);
}

#[test]
fn the_engine_follows_the_magic_comment_then_the_packages() {
    assert_eq!(engine_of("\\documentclass{article}"), Engine::Pdf);
    assert_eq!(engine_of("\\documentclass[UTF8]{ctexart}"), Engine::Xe);
    assert_eq!(
        engine_of("\\usepackage[T1]{fontenc}\n\\usepackage{amsmath, fontspec}"),
        Engine::Xe
    );
    assert_eq!(
        engine_of("% \\usepackage{ctex}\n\\usepackage{amsmath}"),
        Engine::Pdf
    );
    assert_eq!(
        engine_of("% !TEX program = lualatex\n\\usepackage{ctex}"),
        Engine::Lua
    );
    // `\inputenc` 这类前缀相同的命令不当作 `\input`。
    assert_eq!(arguments("\\inputenc{x}\\input{y}", "input"), ["y"]);
}

#[test]
fn jobs_write_into_the_output_directory() {
    let root = Path::new("/p/paper.tex");
    let out = Path::new("/cache/latex/paper-1");
    let args = |job: &Job| -> Vec<String> {
        job.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    };

    let latexmk = Toolchain::Latexmk("/bin/latexmk".into());
    let run = job(&latexmk, root, Engine::Xe, out, None);
    assert_eq!(
        args(&run),
        [
            "-xelatex",
            "-synctex=1",
            "-interaction=nonstopmode",
            "-file-line-error",
            "-outdir=/cache/latex/paper-1",
            "./paper.tex"
        ]
    );
    assert_eq!(run.dir, Path::new("/p"));
    assert_eq!(run.pdf, out.join("paper.pdf"));
    assert_eq!(run.synctex, out.join("paper.synctex.gz"));
    assert_eq!(run.log, out.join("paper.log"));
    // 用户同意过的 `latexmkrc` 设置了 `$pdf_mode` 时按它，不指定引擎。
    let rc = |run| {
        Some(ProjectRc {
            text: "$pdf_mode = 5;\nsystem('x');",
            run,
        })
    };
    let run = job(&latexmk, root, Engine::Pdf, out, rc(true));
    assert_eq!(args(&run)[0], "-synctex=1");
    // 没有同意的不读取，只沿用其中选的引擎；文件名不会被当作选项。
    let run = job(&latexmk, root, Engine::Pdf, out, rc(false));
    let unread = args(&run);
    assert_eq!(unread[0], "-norc");
    assert!(unread.contains(&"-xelatex".to_owned()) && !unread.contains(&"-pdf".to_owned()));
    assert_eq!(unread.last().unwrap(), "./paper.tex");
    assert_eq!(pdf_mode("$pdf_mode=4;"), Some(Engine::Lua));
    assert_eq!(pdf_mode("$pdf_mode = 3;"), None);
    assert_eq!(pdf_mode("# no mode"), None);

    let tectonic = Toolchain::Tectonic("/bin/tectonic".into());
    let run = job(&tectonic, root, Engine::Pdf, out, None);
    assert_eq!(
        args(&run),
        [
            "-Z",
            "continue-on-errors",
            "--synctex",
            "--keep-logs",
            "--outdir",
            "/cache/latex/paper-1",
            "./paper.tex"
        ]
    );

    // 只有引擎：用文档要的那个，没有时用装了的第一个。
    let engines = Toolchain::Engines(vec![
        (Engine::Pdf, "/bin/pdflatex".into()),
        (Engine::Xe, "/bin/xelatex".into()),
    ]);
    assert_eq!(
        job(&engines, root, Engine::Xe, out, None).program,
        Path::new("/bin/xelatex")
    );
    let run = job(&engines, root, Engine::Lua, out, None);
    assert_eq!(run.program, Path::new("/bin/pdflatex"));
    assert!(args(&run).contains(&"-output-directory=/cache/latex/paper-1".to_owned()));
}

#[test]
fn output_directories_are_stable_and_per_root() {
    let cache = Path::new("/cache");
    let a = out_dir(cache, Path::new("/p/paper.tex"));
    assert_eq!(a, out_dir(cache, Path::new("/p/paper.tex")));
    assert_ne!(a, out_dir(cache, Path::new("/q/paper.tex")));
    assert!(a.starts_with("/cache/latex"));
    assert!(
        a.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("paper-")
    );
}

#[test]
fn log_errors_carry_their_file_and_line() {
    let dir = Path::new("/p");
    let log = "This is pdfTeX\n(./main.tex\n\
        ./chapters/intro.tex:12: Undefined control sequence.\nl.12 \\foo\n\n\
        ./main.tex:3: LaTeX Error: File `missing.sty' not found.\n";
    assert_eq!(
        problems(log, dir),
        [
            Problem {
                file: Some("/p/chapters/intro.tex".into()),
                line: Some(12),
                message: "Undefined control sequence.".into(),
            },
            Problem {
                file: Some("/p/main.tex".into()),
                line: Some(3),
                message: "LaTeX Error: File `missing.sty' not found.".into(),
            },
        ]
    );
    // 没有 `-file-line-error` 时：`! 信息`，行号在随后的 `l.行`；同一条只记一次。
    let log = "! Missing $ inserted.\n<inserted text>\n$\nl.7 a_\n             b\n\
        ! Missing $ inserted.\nl.7 a_\n";
    assert_eq!(
        problems(log, dir),
        [Problem {
            file: None,
            line: Some(7),
            message: "Missing $ inserted.".into()
        }]
    );
    // 折成 79 个字的行接起来。
    let long = format!("./main.tex:9: {}", "x".repeat(79 - 14));
    let log = format!("{long}\nyz\n");
    assert_eq!(
        problems(&log, dir)[0].message,
        format!("{}yz", "x".repeat(65))
    );
}

#[test]
fn progress_is_read_from_the_output() {
    let mut reader = Reader::default();
    assert_eq!(
        reader.feed("note: downloading tcolorbox.sty\nnote: Running"),
        Some(Progress::Download("tcolorbox.sty".into()))
    );
    // 半行留到下一段接上。
    assert_eq!(reader.feed(" TeX ...\n"), None);
    assert_eq!(
        reader.feed("note: Rerunning TeX because main.aux changed\n"),
        Some(Progress::Pass(2))
    );
    assert_eq!(
        reader.feed("Run number 3 of rule 'pdflatex'\n"),
        Some(Progress::Pass(3))
    );
    // 页号接在同一行里，取最后一个。
    assert_eq!(reader.feed("(./main.aux) [1] [2"), Some(Progress::Page(2)));
}

/// 用脚本冒充的编译程序：执行 `body`，在产物目录写日志 `log`。
#[cfg(unix)]
fn fake_compile(name: &str, log: &str, body: &str) -> (std::path::PathBuf, Job) {
    let root = scratch(name);
    let tool = fake_tool(&root.join("bin"), "tectonic", body);
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}").unwrap();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("main.log"), log).unwrap();
    let job = job(&Toolchain::Tectonic(tool), &main, Engine::Pdf, &out, None);
    (root, job)
}

#[cfg(unix)]
#[test]
fn errors_without_a_file_belong_to_the_root_or_to_the_file_on_stderr() {
    let shell = Shell::with_vars(vec![("PATH".into(), "/bin:/usr/bin".into())]);
    let log = "! Undefined control sequence.\nl.3 \\oops\n";
    // 只有日志：错误算主文件的。
    let (root, run) = fake_compile("latex-errors-root", log, "exit 1");
    let compiled = compile(&shell, &run, &Cancel::default(), |_| {}).unwrap();
    assert_eq!(
        compiled.problems[0].file.as_deref(),
        Some(run.root.as_path())
    );
    assert_eq!(compiled.problems[0].line, Some(3));
    let _ = std::fs::remove_dir_all(&root);
    // 标准错误写了文件（`tectonic`）：按它，同一条只记一次。
    let body = "echo 'error: chapters/a.tex:3: Undefined control sequence' >&2\n\
                echo 'error: chapters/a.tex:3: Undefined control sequence' >&2\nexit 1";
    let (root, run) = fake_compile("latex-errors-stderr", log, body);
    let compiled = compile(&shell, &run, &Cancel::default(), |_| {}).unwrap();
    let file = run.dir.join("chapters/a.tex");
    assert_eq!(compiled.problems.len(), 1);
    assert_eq!(compiled.problems[0].file.as_deref(), Some(file.as_path()));
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn stalled_compiles_stop_and_say_why() {
    let shell = Shell::with_vars(vec![("PATH".into(), "/bin:/usr/bin".into())]);
    // 一直没有输出：超过上限后停下，报告原因而不是当作被取消。
    let (root, run) = fake_compile("latex-stall", "", "sleep 30");
    let started = std::time::Instant::now();
    let compiled = compile(&shell, &run, &Cancel::default(), |_| {}).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(!compiled.pdf && compiled.failure.is_some(), "{compiled:?}");
    // 一直有输出的不停。
    let body = "for i in 1 2 3; do echo \"[$i]\"; sleep 0.6; done";
    let (other, run) = fake_compile("latex-busy", "", body);
    let compiled = compile(&shell, &run, &Cancel::default(), |_| {}).unwrap();
    assert!(compiled.failure.is_none(), "{compiled:?}");
    // 调用方取消：不报告结果。
    let (third, run) = fake_compile("latex-cancel", "", "sleep 30");
    let cancel = Cancel::default();
    cancel.cancel();
    assert!(compile(&shell, &run, &cancel, |_| {}).is_none());
    for dir in [root, other, third] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
