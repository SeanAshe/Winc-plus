//! `detect` 的测试：覆盖各语言的命令和证据要求、工作区、名字校验、同名和数量上限。

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// 新建空的临时目录；用例并行运行且名字可能相同，用序号区分。
fn temporary(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let process = std::process::id();
    let root = std::env::temp_dir().join(format!("winc-detect-{name}-{process}-{serial}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn commands(root: &Path) -> Vec<String> {
    detect_commands(root).into_iter().map(|(_, c)| c).collect()
}

/// 在 `files`（路径，内容）组成的项目里识别命令。
fn commands_in(name: &str, files: &[(&str, &str)]) -> Vec<String> {
    let root = temporary(name);
    for (file, text) in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let found = commands(&root);
    std::fs::remove_dir_all(&root).unwrap();
    found
}

/// 没有标记文件时什么也不识别。
#[test]
fn empty_projects_have_no_commands() {
    assert!(commands_in("empty", &[("README.md", "")]).is_empty());
}

/// 一组识别用例：名字，项目文件（路径，内容），应识别出的命令。
type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a [&'a str]);

/// 有证据时各语言列出标准命令。
#[test]
fn each_language_gets_its_standard_commands() {
    let cases: &[Case] = &[
        (
            "go",
            &[("go.mod", ""), ("main.go", "package main\n")],
            &["go run .", "go build ./...", "go test ./..."],
        ),
        (
            "swift",
            &[
                ("Package.swift", ".executableTarget(name: \"a\")"),
                ("Tests/a.swift", ""),
            ],
            &["swift run", "swift build", "swift test"],
        ),
        (
            "dotnet",
            &[
                ("App.csproj", "<OutputType>Exe</OutputType>"),
                ("Tests.csproj", "Microsoft.NET.Test.Sdk"),
            ],
            &["dotnet run", "dotnet build", "dotnet test"],
        ),
        (
            "web",
            &[("App.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\">")],
            &["dotnet run", "dotnet build"],
        ),
        (
            "zig",
            &[("build.zig", "b.step(\"run\", x); b.step(\"test\", y);")],
            &["zig build run", "zig build", "zig build test"],
        ),
        (
            "cmake",
            &[("CMakeLists.txt", "enable_testing()")],
            &[
                "cmake -S . -B build",
                "cmake --build build",
                "ctest --test-dir build",
            ],
        ),
        (
            "maven",
            &[("pom.xml", "<project/>"), ("src/test/A.java", "")],
            &["mvn compile", "mvn test"],
        ),
        (
            "spring",
            &[("pom.xml", "spring-boot-maven-plugin"), ("mvnw", "")],
            &["./mvnw spring-boot:run", "./mvnw compile"],
        ),
        (
            "gradle-kts",
            &[
                ("build.gradle.kts", "plugins {\n    application\n}\n"),
                ("gradlew", ""),
            ],
            &["./gradlew run", "./gradlew build"],
        ),
        (
            "boot",
            &[
                ("build.gradle", "id 'org.springframework.boot'"),
                ("src/test/A.java", ""),
            ],
            &["gradle bootRun", "gradle build", "gradle test"],
        ),
        (
            "dart",
            &[
                ("pubspec.yaml", "name: a\n"),
                ("bin/a.dart", ""),
                ("test/a.dart", ""),
            ],
            &["dart run", "dart test"],
        ),
        (
            "flutter",
            &[
                ("pubspec.yaml", "dependencies:\n  flutter:\n"),
                ("lib/main.dart", ""),
            ],
            &["flutter run"],
        ),
        (
            "phoenix",
            &[("mix.exs", "{:phoenix, \"~> 1.7\"}"), ("test/a.exs", "")],
            &["mix phx.server", "mix test"],
        ),
        (
            "rails",
            &[("Gemfile", ""), ("bin/rails", ""), ("spec/a_spec.rb", "")],
            &["bundle exec rails server", "bundle exec rspec"],
        ),
        (
            "rake",
            &[("Gemfile", ""), ("Rakefile", "task :test")],
            &["bundle exec rake test"],
        ),
        (
            "laravel",
            &[
                ("artisan", ""),
                ("composer.json", r#"{"scripts": {"test": "x"}}"#),
            ],
            &["php artisan serve", "composer run test"],
        ),
        (
            "deno",
            &[("deno.json", r#"{"tasks": {"dev": "x"}}"#)],
            &["deno task dev"],
        ),
    ];
    for (name, files, expected) in cases {
        assert_eq!(commands_in(name, files), *expected, "{name}");
    }
}

/// 没有证据时不列运行和测试命令：库、没有测试的项目只留构建命令，或者什么也不列。
#[test]
fn commands_without_evidence_are_left_out() {
    let cases: &[Case] = &[
        (
            "lib-rust",
            &[
                ("Cargo.toml", "[package]\nname = \"a\"\n"),
                ("src/lib.rs", ""),
            ],
            &["cargo check", "cargo test"],
        ),
        (
            "lib-go",
            &[("go.mod", ""), ("lib.go", "package lib\n")],
            &["go build ./...", "go test ./..."],
        ),
        (
            "lib-swift",
            &[("Package.swift", ".library(name: \"a\")")],
            &["swift build"],
        ),
        (
            "lib-dotnet",
            &[("Lib.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\">")],
            &["dotnet build"],
        ),
        ("sln", &[("App.sln", "")], &["dotnet build"]),
        ("zig", &[("build.zig", "")], &["zig build"]),
        (
            "cmake",
            &[("CMakeLists.txt", "project(a)")],
            &["cmake -S . -B build", "cmake --build build"],
        ),
        ("maven", &[("pom.xml", "<project/>")], &["mvn compile"]),
        (
            "gradle-word",
            &[("build.gradle", "// this application is great\n")],
            &["gradle build"],
        ),
        ("dart", &[("pubspec.yaml", "name: a\n")], &[]),
        ("mix", &[("mix.exs", "")], &[]),
        ("rake", &[("Gemfile", ""), ("Rakefile", "task :build")], &[]),
    ];
    for (name, files, expected) in cases {
        assert_eq!(commands_in(name, files), *expected, "{name}");
    }
}

/// Go 的入口可以在 `cmd/<名字>/` 下。
#[test]
fn go_commands_are_found_under_cmd() {
    assert_eq!(
        commands_in(
            "go-cmd",
            &[
                ("go.mod", ""),
                ("cmd/api/main.go", "package main\n"),
                ("cmd/doc/doc.go", "package doc\n")
            ]
        ),
        ["go run ./cmd/api", "go build ./...", "go test ./..."]
    );
}

/// Python 按锁文件选运行方式；看到 pytest 才用 pytest，否则有测试目录时用 unittest。
#[test]
fn python_projects_follow_their_tools() {
    let python = if cfg!(windows) { "python" } else { "python3" };
    assert_eq!(
        commands_in("py", &[("main.py", ""), ("tests/test_a.py", "")]),
        [format!("{python} main.py"), format!("{python} -m unittest")]
    );
    assert_eq!(
        commands_in(
            "py-req",
            &[("requirements.txt", "pytest\n"), ("tests/a.py", "")]
        ),
        [format!("{python} -m pytest")]
    );
    assert_eq!(
        commands_in(
            "uv",
            &[
                ("pyproject.toml", "[tool.pytest.ini_options]\n"),
                ("uv.lock", "")
            ]
        ),
        ["uv run python -m pytest"]
    );
    assert_eq!(
        commands_in("django", &[("manage.py", ""), ("poetry.lock", "")]),
        ["poetry run python manage.py runserver"]
    );
    assert!(commands_in("py-bare", &[("requirements.txt", "flask\n")]).is_empty());
}

/// Makefile 和 justfile 的目标：常用的在前，变量赋值和特殊目标不算，带参数的不列。
#[test]
fn make_and_just_list_their_recipes() {
    let makefile = "CC := gcc\n.PHONY: test\nlib:\n\ttrue\ntest: lib\n\ttrue\nbuild:\n\ttrue\n";
    assert_eq!(
        commands_in("make", &[("Makefile", makefile)]),
        ["make build", "make test", "make lib"]
    );
    let justfile = "greet name:\n  echo\nrun:\n  true\n";
    assert_eq!(commands_in("just", &[("justfile", justfile)]), ["just run"]);
}

/// 含 shell 特殊字符的名字不会拼进命令。
#[test]
fn unsafe_names_are_skipped() {
    let scripts = r#"{"scripts": {"ok": "x", "a;rm -rf": "x", "$(evil)": "x", "-flag": "x"}}"#;
    assert_eq!(
        commands_in("unsafe", &[("package.json", scripts)]),
        ["npm run ok"]
    );
    let makefile = "a b:\n\ttrue\n`evil`:\n\ttrue\nfine:\n\ttrue\n";
    assert_eq!(
        commands_in("unsafe-make", &[("Makefile", makefile)]),
        ["make fine"]
    );
    assert!(safe_name("api:check"));
    assert!(!safe_name(""));
}

/// Cargo 项目和坏的 `package.json`。
#[test]
fn cargo_packages_get_standard_commands() {
    assert_eq!(
        commands_in(
            "cargo",
            &[
                ("package.json", "{"),
                ("Cargo.toml", "[package]\nname = \"a\"\n"),
                ("src/main.rs", ""),
            ]
        ),
        ["cargo run", "cargo check", "cargo test"]
    );
}

const MEMBER: &str = "[package]\nname = \"NAME\"\n\n[dependencies]\nname = \"x\"\n";

/// 工作区列出可运行的成员，通配符和多行写法都能读，库不列。
#[test]
fn workspaces_list_their_runnable_members() {
    let (b, a) = (
        MEMBER.replace("NAME", "pkg-b"),
        MEMBER.replace("NAME", "pkg-a"),
    );
    let (lib, tool) = (
        MEMBER.replace("NAME", "pkg-lib"),
        MEMBER.replace("NAME", "pkg-tool"),
    );
    let found = commands_in(
        "workspace",
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\n    \"crates/*\",\n    \"tool\",\n]\n",
            ),
            ("crates/b/Cargo.toml", &b),
            ("crates/b/src/main.rs", ""),
            ("crates/a/Cargo.toml", &a),
            ("crates/a/src/main.rs", ""),
            ("crates/lib/Cargo.toml", &lib),
            ("tool/Cargo.toml", &tool),
            ("tool/src/main.rs", ""),
        ],
    );
    assert_eq!(
        found,
        [
            "cargo run -p pkg-a",
            "cargo run -p pkg-b",
            "cargo run -p pkg-tool",
            "cargo check",
            "cargo test",
        ]
    );
}

/// `default-members` 只有一个可运行的成员时可以直接 `cargo run`，这也是 Winc 自己的写法。
#[test]
fn default_members_allow_a_plain_cargo_run() {
    let app = MEMBER.replace("NAME", "pkg-app");
    let found = commands_in(
        "default-members",
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n",
            ),
            ("crates/app/Cargo.toml", &app),
            ("crates/app/src/main.rs", ""),
        ],
    );
    assert_eq!(found, ["cargo run", "cargo check", "cargo test"]);
}

/// 不同来源的同名命令都保留并标上工具，其余名字不变；完全相同的命令只留一个；总数有上限。
#[test]
fn names_stay_unique_and_the_list_stays_short() {
    let root = temporary("duplicate");
    std::fs::write(root.join("package.json"), r#"{"scripts": {"test": "x"}}"#).unwrap();
    std::fs::write(root.join("go.mod"), "").unwrap();
    let names: Vec<String> = detect_commands(&root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["test (npm)", "build", "test (go)"]);

    let tasks = r#"{"tasks": {"lint": "x"}}"#;
    std::fs::write(root.join("deno.json"), tasks).unwrap();
    std::fs::write(root.join("deno.jsonc"), tasks).unwrap();
    let names: Vec<String> = detect_commands(&root).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["test (npm)", "lint", "build", "test (go)"]);
    std::fs::remove_file(root.join("deno.json")).unwrap();
    std::fs::remove_file(root.join("deno.jsonc")).unwrap();

    let scripts = r#"{"scripts": {"a": "", "b": "", "c": "", "d": "", "e": "", "f": ""}}"#;
    std::fs::write(root.join("package.json"), scripts).unwrap();
    std::fs::write(root.join("composer.json"), scripts).unwrap();
    assert_eq!(detect_commands(&root).len(), MAX_COMMANDS);
    std::fs::remove_dir_all(&root).unwrap();
}

/// 同名标记的工具：`-m` 指定的模块优先，否则是程序名。
#[test]
fn the_label_names_the_tool() {
    let cases = [
        ("uv run python -m pytest", "pytest"),
        ("python3 -m unittest", "unittest"),
        ("./mvnw test", "mvnw"),
        ("make test", "make"),
    ];
    for (command, expected) in cases {
        assert_eq!(tool(command), expected);
    }
}

/// 只有项目根目录下的标记文件才影响识别结果。
#[test]
fn only_markers_in_the_root_count() {
    let root = Path::new("/p");
    assert!(is_marker(root, Path::new("/p/pom.xml")));
    assert!(is_marker(root, Path::new("/p/App.csproj")));
    assert!(!is_marker(root, Path::new("/p/sub/pom.xml")));
    assert!(!is_marker(root, Path::new("/p/readme.md")));
}
