//! `lang` 模块的测试：按文件名与扩展名识别语言、Dockerfile 指令的适用范围，以及按 id 与名字
//! 查找语言。
//!
//! 关键字与内置名表按常用词抽样检查，覆盖相近语言不混入、词表扩充后仍能识别常见词两项要求。

use super::*;

#[test]
fn detects_by_name_and_extension() {
    // 样本覆盖常见扩展名、大写扩展名、无扩展名的文件名（`Makefile`、`Cargo.lock`）和以点开头的
    // 配置文件名；`README` 没有扩展名。
    assert_eq!(detect("main.PY").id, "python");
    assert_eq!(detect("Makefile").name, "Makefile");
    assert_eq!(detect("build.sh").name, "Shell");
    assert_eq!(detect("Cargo.lock").name, "TOML");
    assert_eq!(detect("rustfmt.toml").name, "TOML");
    assert_eq!(detect("ci.yml").name, "YAML");
    assert_eq!(detect("setup.cfg").name, "INI");
    assert_eq!(detect(".env").name, "Dotenv");
    assert_eq!(detect(".gitignore").name, "Ignore");
    assert_eq!(detect("main.cpp").name, "C++");
    assert_eq!(detect("main.c").name, "C");
    assert_eq!(detect("App.kt").name, "Kotlin");
    assert_eq!(detect("ContentView.swift").name, "Swift");
    assert_eq!(detect("index.html").name, "HTML");
    assert_eq!(detect("style.less").name, "Less");
    assert_eq!(detect("README").id, "text");
    assert_eq!(detect("a.ts").indent_unit, "  ");
    assert_eq!(detect("Makefile").indent_unit, "\t");
    assert_eq!(detect("rules.mk").indent_unit, "\t");
}

#[test]
fn dockerfile_directives_are_keywords_only_for_dockerfile() {
    let dockerfile = detect("Dockerfile");
    assert!(dockerfile.keywords.contains(&"FROM"));
    assert!(dockerfile.keywords.contains(&"then"));
    // Dockerfile 指令不属于 Shell 和 Makefile。
    for shell in [detect("build.sh"), detect("Makefile")] {
        assert!(!shell.keywords.contains(&"FROM"), "{}", shell.name);
        assert!(!shell.keywords.contains(&"RUN"), "{}", shell.name);
        assert!(shell.keywords.contains(&"then"), "{}", shell.name);
    }
}

#[test]
fn finds_by_id_or_name_ignoring_case() {
    assert_eq!(find("json").map(|l| l.name), Some("JSON"));
    assert_eq!(find("C++").map(|l| l.id), Some("cpp"));
    assert_eq!(find("objective-c").map(|l| l.name), Some("Objective-C"));
    assert!(find("text").is_none() && find("Plain Text").is_none());
    assert!(find("cobol").is_none());
    // 各语言的 id 和名字互不相同，按哪个查找都能找到同一种语言。
    for language in languages() {
        assert_eq!(find(language.id).map(|l| l.id), Some(language.id));
        assert_eq!(find(language.name).map(|l| l.id), Some(language.id));
    }
}

#[test]
fn related_languages_keep_separate_keywords() {
    let has = |file: &str, word: &str| detect(file).keywords.contains(&word);
    // C 中 `new`、`class`、`this` 是合法的标识符；Objective-C 沿用 C。
    for word in ["new", "class", "this", "template", "namespace"] {
        assert!(!has("a.c", word), "C: {word}");
        assert!(!has("a.m", word), "Objective-C: {word}");
        assert!(has("a.cpp", word), "C++: {word}");
    }
    assert!(has("a.c", "restrict") && !has("a.cpp", "restrict"));
    assert!(has("a.cpp", "static_cast") && has("a.cpp", "decltype"));

    // Java 中 `object`、`val`、`is` 是合法的标识符。
    for word in ["fun", "val", "when", "is", "object"] {
        assert!(!has("A.java", word), "Java: {word}");
        assert!(has("a.kt", word), "Kotlin: {word}");
    }
    for word in ["instanceof", "synchronized", "assert", "record"] {
        assert!(has("A.java", word), "Java: {word}");
    }
    for word in ["implements", "throws", "static", "void"] {
        assert!(!has("a.kt", word), "Kotlin: {word}");
    }
    for word in ["def", "trait", "match", "given"] {
        assert!(has("a.scala", word), "Scala: {word}");
    }
    for word in ["def", "in", "as", "trait"] {
        assert!(has("build.gradle", word), "Groovy: {word}");
    }
    // ArkTS 在 TypeScript 之上补充 `struct` 和 `constructor`，不影响 TypeScript 的词表。
    for word in ["struct", "constructor"] {
        assert!(
            has("Index.ets", word) && !has("a.ts", word),
            "ArkTS: {word}"
        );
    }
}

// 各语言的常用词抽样：扩充或调整关键字、内置名表后，这些词要仍然认得出来。同时检查相近语言的
// 表没有互相混入（Shell 与 Makefile、C 与 Objective-C、fish 与 Shell）。
#[test]
fn keyword_tables_cover_common_words() {
    let has = |file: &str, word: &str| detect(file).keywords.contains(&word);
    let builtin = |file: &str, word: &str| detect(file).builtins.contains(&word);
    for word in [
        "case", "when", "in", "like", "between", "union", "exists", "with", "desc",
    ] {
        assert!(has("a.sql", word), "SQL: {word}");
    }
    assert!(builtin("a.sql", "varchar") && builtin("a.sql", "timestamp"));
    for word in ["ADD", "EXPOSE", "USER", "LABEL", "VOLUME", "AS"] {
        assert!(has("Dockerfile", word), "Dockerfile: {word}");
    }
    for word in ["satisfies", "infer", "asserts", "override"] {
        assert!(has("a.ts", word), "TypeScript: {word}");
    }
    for word in [
        "int8", "uint32", "float32", "any", "cap", "recover", "min", "clear",
    ] {
        assert!(builtin("a.go", word), "Go: {word}");
    }
    for word in ["until", "select", "declare", "break"] {
        assert!(has("a.sh", word), "Shell: {word}");
    }
    for word in ["ifeq", "endif", "include", "define", "then"] {
        assert!(has("Makefile", word), "Makefile: {word}");
    }
    assert!(!has("a.sh", "ifeq"));
    assert!(builtin("a.m", "YES") && builtin("a.m", "size_t") && builtin("a.mm", "nullptr"));
    assert!(detect("a.m").decorators);
    assert_eq!(detect("Jenkinsfile").name, "Groovy");
    assert_eq!(detect("config.fish").name, "Fish");
    assert!(has("config.fish", "end") && !has("a.sh", "end"));
}

#[test]
fn detects_common_languages_and_names() {
    for (file, name) in [
        ("Program.cs", "C#"),
        ("index.php", "PHP"),
        ("Gemfile", "Ruby"),
        ("config.ru", "Ruby"),
        ("init.lua", "Lua"),
        ("a.pl", "Perl"),
        ("a.R", "R"),
        ("main.dart", "Dart"),
        ("build.zig", "Zig"),
        ("Main.hs", "Haskell"),
        ("mix.exs", "Elixir"),
        ("a.erl", "Erlang"),
        ("a.jl", "Julia"),
        ("core.clj", "Clojure"),
        ("a.ml", "OCaml"),
        ("a.fs", "F#"),
        ("a.ps1", "PowerShell"),
        ("a.bat", "Batch"),
        ("CMakeLists.txt", "CMake"),
        ("main.tf", "HCL"),
        ("a.proto", "Protocol Buffers"),
        ("schema.graphql", "GraphQL"),
        ("flake.nix", "Nix"),
        ("App.svelte", "Svelte"),
        ("Index.ets", "ArkTS"),
        ("Index.ETS", "ArkTS"),
        ("paper.tex", "LaTeX"),
        ("Dockerfile.dev", "Dockerfile"),
        (".env.staging", "Dotenv"),
        ("BUILD.bazel", "Python"),
        ("kernel.cu", "C++"),
        ("tsconfig.json", "JSON"),
        ("App.xaml", "XML"),
    ] {
        assert_eq!(detect(file).name, name, "{file}");
    }
    // vim 的俄语教程 `tutor.ru` 不是 Ruby。
    assert_eq!(detect("tutor.ru").id, "text");
}

#[test]
fn detects_scripts_by_shebang() {
    let by_shebang = |line: &str| detect_with_first_line("script", line).name;
    assert_eq!(by_shebang("#!/bin/bash"), "Shell");
    assert_eq!(by_shebang("#!/usr/bin/env python3.12"), "Python");
    assert_eq!(by_shebang("#!/usr/bin/env -S node --flag"), "JavaScript");
    assert_eq!(by_shebang("#!/usr/bin/env ts-node"), "TypeScript");
    assert_eq!(by_shebang("#!/usr/bin/env tsx"), "TypeScript");
    assert_eq!(by_shebang("#!/usr/bin/ruby -w"), "Ruby");
    assert_eq!(by_shebang("#!/usr/bin/env pwsh"), "PowerShell");
    assert_eq!(by_shebang("not a shebang"), "Plain Text");
    // 按文件名认得出时不看首行。
    assert_eq!(detect_with_first_line("a.py", "#!/bin/sh").name, "Python");
}

#[test]
fn names_and_related_languages() {
    let name = |file: &str, line: &str, col: usize| {
        let chars: Vec<char> = line.chars().collect();
        detect(file)
            .name_range(&chars, col)
            .map(|r| chars[r].iter().collect::<String>())
    };
    assert_eq!(name("a.css", ".my-class {", 3).as_deref(), Some("my-class"));
    assert_eq!(
        name("a.css", "  --main-color: red", 4).as_deref(),
        Some("--main-color")
    );
    assert_eq!(name("a.php", "echo $total;", 7).as_deref(), Some("$total"));
    assert_eq!(name("a.rb", "if valid? then", 4).as_deref(), Some("valid?"));
    assert_eq!(name("a.rb", "@name = 1", 3).as_deref(), Some("@name"));
    assert_eq!(name("a.js", "const $el = 1", 7).as_deref(), Some("$el"));
    assert_eq!(name("a.clj", "(if-let [x]", 2).as_deref(), Some("if-let"));
    // 光标紧挨在名字之后也取这个名字；其他语言中 `-` 不属于名字。
    assert_eq!(name("a.rs", "a-b", 1).as_deref(), Some("a"));
    assert_eq!(name("a.rs", "x  ", 2), None);
    assert!(detect("a.rs").is_name("名字") && !detect("a.rs").is_name("1x"));

    assert!(detect("a.c").is_related(detect("b.hpp")));
    assert!(detect("a.ts").is_related(detect("App.vue")));
    assert!(detect("Index.ets").is_related(detect("a.ts")));
    assert!(detect("a.ts").is_related(detect("Index.ets")));
    assert!(!detect("Index.ets").is_related(detect("a.rs")));
    assert!(detect("a.css").is_related(detect("index.html")));
    assert!(!detect("a.rs").is_related(detect("README.md")));
}
