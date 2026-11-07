//! 从项目文件识别常用命令：只读取项目根目录的标记文件，不执行任何程序。
//!
//! 只列有证据能用的命令：项目脚本是作者声明的；运行类命令要找到入口，测试类命令要找到测试，证据不足
//! 就不列。不检查工具是否已安装：主程序的 `PATH` 不一定和用户的 shell 相同，提前判断容易误判。
//!
//! 结果只作为建议，运行前仍由用户确认。脚本名和包名来自项目文件，内容不可信，含 shell 特殊字符的
//! 名字直接跳过（[`safe_name`]）。

use std::path::{Path, PathBuf};

use crate::json::Json;

/// 一个来源最多列出的脚本或目标数（个）：下拉菜单一屏放得下，其余由用户写进配置文件。
const MAX_SCRIPTS: usize = 6;

/// 总共最多列出的命令数（个）：多语言仓库的下拉菜单也保持在一屏以内。
const MAX_COMMANDS: usize = 12;

/// 优先列出的脚本和目标，按日常使用的顺序排列。
const COMMON_SCRIPTS: &[&str] = &["dev", "start", "serve", "run", "build", "test", "lint"];

/// 根目录下决定识别结果的文件名；这些文件变化时要重新识别。
const MARKERS: &[&str] = &[
    "package.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "deno.json",
    "deno.jsonc",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "manage.py",
    "main.py",
    "app.py",
    "uv.lock",
    "poetry.lock",
    "pytest.ini",
    "pom.xml",
    "mvnw",
    "build.gradle",
    "build.gradle.kts",
    "gradlew",
    "Gemfile",
    "Rakefile",
    "composer.json",
    "artisan",
    "Package.swift",
    "pubspec.yaml",
    "mix.exs",
    "build.zig",
    "CMakeLists.txt",
    "Makefile",
    "makefile",
    "justfile",
];

/// 根目录下决定识别结果的文件扩展名。
const MARKER_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj", "sln"];

/// `path` 是否是项目根目录 `root` 下决定识别结果的文件。
pub(super) fn is_marker(root: &Path, path: &Path) -> bool {
    if path.parent() != Some(root) {
        return false;
    }
    let named = (path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| MARKERS.contains(&name));
    let extension = path.extension().and_then(|extension| extension.to_str());
    named || extension.is_some_and(|extension| MARKER_EXTENSIONS.contains(&extension))
}

/// 返回项目 `root` 的常用命令，每项是（名字，命令）。
pub(super) fn detect_commands(root: &Path) -> Vec<(String, String)> {
    let sources = [
        node, deno, rust, go, python, maven, gradle, dotnet, ruby, php, swift, dart, elixir, zig,
        cmake, make, just,
    ];
    let mut commands: Vec<(String, String)> = Vec::new();
    for found in sources.into_iter().flat_map(|source| source(root)) {
        // `deno.json` 和 `deno.jsonc` 可能声明同一个任务。
        if !commands.contains(&found) {
            commands.push(found);
        }
    }
    commands.truncate(MAX_COMMANDS);
    // 不同来源可能同名（`npm run test` 和 `cargo test`）：都标上工具，名字唯一且能看出区别。
    let labelled = commands.iter().map(|(name, command)| {
        let shared = commands.iter().filter(|(other, _)| other == name).count() > 1;
        let label = if shared {
            format!("{name} ({})", tool(command))
        } else {
            name.clone()
        };
        (label, command.clone())
    });
    labelled.collect()
}

/// 返回命令实际使用的工具：`-m` 指定的模块，否则是程序名（去掉 `./`）。
fn tool(command: &str) -> &str {
    let mut words = command.split_whitespace();
    let module = words.clone().skip_while(|word| *word != "-m").nth(1);
    let program = words.next().unwrap_or_default();
    module.unwrap_or_else(|| program.trim_start_matches("./"))
}

fn entry(name: &str, command: impl Into<String>) -> (String, String) {
    (name.to_owned(), command.into())
}

/// 名字能否不加引号拼进命令行：只含字母、数字和常见的分隔符，且不以 `-` 开头。
fn safe_name(name: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "_-:.+/@".contains(c);
    !name.is_empty() && !name.starts_with('-') && name.chars().all(allowed)
}

/// 把脚本或目标的名字排好序：常用的在前，去掉不安全的名字，保留前几个。
fn pick_scripts<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let names: Vec<&str> = names.into_iter().filter(|name| safe_name(name)).collect();
    let common = (COMMON_SCRIPTS.iter().copied()).filter(|name| names.contains(name));
    let others = names
        .iter()
        .copied()
        .filter(|n| !COMMON_SCRIPTS.contains(n));
    common.chain(others).take(MAX_SCRIPTS).collect()
}

fn read(root: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(root.join(name)).ok()
}

/// 读取 JSON 文件里对象 `key` 的所有键。
fn json_keys(root: &Path, file: &str, key: &str) -> Vec<String> {
    let parsed = read(root, file).and_then(|text| Json::parse_jsonc(&text));
    let object = parsed.as_ref().and_then(|(json, _)| json.as_object());
    let scripts = object
        .and_then(|object| object.get(key))
        .and_then(Json::as_object);
    scripts.map_or_else(Vec::new, |scripts| scripts.keys().cloned().collect())
}

fn node(root: &Path) -> Vec<(String, String)> {
    let runner = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ]
    .into_iter()
    .find(|(lock, _)| root.join(lock).exists())
    .map_or("npm", |(_, runner)| runner);
    let scripts = json_keys(root, "package.json", "scripts");
    let names = pick_scripts(scripts.iter().map(String::as_str));
    let run = |name: &&str| entry(name, format!("{runner} run {name}"));
    names.iter().map(run).collect()
}

fn deno(root: &Path) -> Vec<(String, String)> {
    let mut tasks = json_keys(root, "deno.json", "tasks");
    tasks.extend(json_keys(root, "deno.jsonc", "tasks"));
    let names = pick_scripts(tasks.iter().map(String::as_str));
    names
        .iter()
        .map(|name| entry(name, format!("deno task {name}")))
        .collect()
}

/// 返回 Cargo 项目的常用命令。
///
/// 只有 `[workspace]` 的 Cargo.toml 不能直接 `cargo run`：`default-members` 只有一个可运行的成员时
/// 可以，否则为每个可运行的成员列出 `cargo run -p`。
fn rust(root: &Path) -> Vec<(String, String)> {
    let Some(text) = read(root, "Cargo.toml") else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    if text.lines().any(|line| line.trim() == "[package]") {
        if cargo_runnable(root, &text) {
            commands.push(entry("run", "cargo run"));
        }
    } else {
        let defaults = toml_strings(&text, "default-members");
        let members = if defaults.is_empty() {
            toml_strings(&text, "members")
        } else {
            defaults.clone()
        };
        let runnable = runnable_members(root, &members);
        if runnable.len() == 1 && !defaults.is_empty() {
            commands.push(entry("run", "cargo run"));
        } else {
            for name in runnable.iter().take(MAX_SCRIPTS) {
                commands.push(entry(
                    &format!("run {name}"),
                    format!("cargo run -p {name}"),
                ));
            }
        }
    }
    commands.push(entry("check", "cargo check"));
    commands.push(entry("test", "cargo test"));
    commands
}

/// 读取 TOML 中字符串数组 `key` 的各项，只处理 `Cargo.toml` 里 `members` 这样的简单写法。
fn toml_strings(text: &str, key: &str) -> Vec<String> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let value = line.trim_start().strip_prefix(key).map(str::trim_start);
        if value.is_some_and(|value| value.starts_with('=')) {
            let list = text[offset..].split(']').next().unwrap_or_default();
            return (list.split('"').skip(1).step_by(2).map(str::to_owned)).collect();
        }
        offset += line.len();
    }
    Vec::new()
}

/// 返回成员目录 `members`（可以以 `/*` 结尾）中可运行的包名，按目录名排序。
fn runnable_members(root: &Path, members: &[String]) -> Vec<String> {
    let mut dirs = Vec::new();
    for member in members {
        match member.strip_suffix("/*") {
            Some(parent) => {
                let entries = std::fs::read_dir(root.join(parent)).into_iter().flatten();
                let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                found.sort();
                dirs.extend(found);
            }
            None => dirs.push(root.join(member)),
        }
    }
    let names = dirs.iter().filter_map(|dir| {
        let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        cargo_runnable(dir, &text)
            .then(|| package_name(&text))
            .flatten()
    });
    names.filter(|name| safe_name(name)).collect()
}

/// 目录 `dir` 的 Cargo 包（清单内容 `text`）有没有可执行目标。
fn cargo_runnable(dir: &Path, text: &str) -> bool {
    dir.join("src/main.rs").exists() || dir.join("src/bin").is_dir() || text.contains("[[bin]]")
}

/// 返回 `[package]` 一节里的 `name`。
fn package_name(text: &str) -> Option<String> {
    let section = text.split_once("[package]")?.1;
    let line = section.lines().find(|line| {
        let name = line.trim_start().strip_prefix("name");
        name.is_some_and(|name| name.trim_start().starts_with('='))
    })?;
    line.split('"').nth(1).map(str::to_owned)
}

fn go(root: &Path) -> Vec<(String, String)> {
    if !root.join("go.mod").exists() {
        return Vec::new();
    }
    let mut commands = Vec::new();
    if go_main(root) {
        commands.push(entry("run", "go run ."));
    } else {
        // 入口放在 `cmd/<名字>/` 是 Go 项目的通行布局。
        let entries = std::fs::read_dir(root.join("cmd"))
            .into_iter()
            .flatten()
            .flatten();
        let mut dirs: Vec<PathBuf> = entries.map(|entry| entry.path()).collect();
        dirs.sort();
        let mains = dirs.iter().filter(|dir| go_main(dir));
        let names = mains
            .filter_map(|dir| dir.file_name()?.to_str())
            .filter(|n| safe_name(n));
        for name in names.take(MAX_SCRIPTS) {
            commands.push(entry(
                &format!("run {name}"),
                format!("go run ./cmd/{name}"),
            ));
        }
    }
    commands.push(entry("build", "go build ./..."));
    commands.push(entry("test", "go test ./..."));
    commands
}

/// 目录 `dir` 里有没有属于 `main` 包的 Go 文件。
fn go_main(dir: &Path) -> bool {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    entries.map(|entry| entry.path()).any(|path| {
        path.extension().is_some_and(|extension| extension == "go")
            && std::fs::read_to_string(&path)
                .is_ok_and(|text| text.lines().any(|line| line.trim() == "package main"))
    })
}

fn python(root: &Path) -> Vec<(String, String)> {
    let exists = |name: &str| root.join(name).exists();
    let markers = [
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "manage.py",
        "main.py",
        "app.py",
    ];
    if !markers.into_iter().any(exists) {
        return Vec::new();
    }
    // Windows 上的解释器叫 `python`，其他系统上 `python` 不一定存在。
    let interpreter = if cfg!(windows) { "python" } else { "python3" };
    let runner = if exists("uv.lock") {
        "uv run python"
    } else if exists("poetry.lock") {
        "poetry run python"
    } else {
        interpreter
    };
    let mut commands = Vec::new();
    if exists("manage.py") {
        commands.push(entry("runserver", format!("{runner} manage.py runserver")));
    }
    for script in ["main.py", "app.py"] {
        if exists(script) {
            commands.push(entry("run", format!("{runner} {script}")));
            break;
        }
    }
    // pytest 不是标准库，要在配置或依赖里看到它才列；否则有测试目录时用标准库的 unittest。
    let mentioned = |file: &str| read(root, file).is_some_and(|text| text.contains("pytest"));
    let pytest = exists("pytest.ini")
        || exists("conftest.py")
        || mentioned("pyproject.toml")
        || mentioned("requirements.txt");
    if pytest {
        commands.push(entry("test", format!("{runner} -m pytest")));
    } else if exists("tests") {
        commands.push(entry("test", format!("{runner} -m unittest")));
    }
    commands
}

fn maven(root: &Path) -> Vec<(String, String)> {
    let Some(pom) = read(root, "pom.xml") else {
        return Vec::new();
    };
    let tool = if root.join("mvnw").exists() {
        "./mvnw"
    } else {
        "mvn"
    };
    let mut commands = Vec::new();
    if pom.contains("spring-boot-maven-plugin") {
        commands.push(entry("run", format!("{tool} spring-boot:run")));
    }
    commands.push(entry("build", format!("{tool} compile")));
    if root.join("src/test").is_dir() {
        commands.push(entry("test", format!("{tool} test")));
    }
    commands
}

fn gradle(root: &Path) -> Vec<(String, String)> {
    let Some(build) = read(root, "build.gradle").or_else(|| read(root, "build.gradle.kts")) else {
        return Vec::new();
    };
    let tool = if root.join("gradlew").exists() {
        "./gradlew"
    } else {
        "gradle"
    };
    // `application` 插件才有 `run` 任务；只看声明插件的写法，不看单词是否出现。
    let application = build.lines().any(|line| {
        let line = line.trim();
        line == "application"
            || line.contains("id 'application'")
            || line.contains("id \"application\"")
            || line.contains("id(\"application\")")
    });
    let mut commands = Vec::new();
    if build.contains("org.springframework.boot") {
        commands.push(entry("run", format!("{tool} bootRun")));
    } else if application {
        commands.push(entry("run", format!("{tool} run")));
    }
    commands.push(entry("build", format!("{tool} build")));
    if root.join("src/test").is_dir() {
        commands.push(entry("test", format!("{tool} test")));
    }
    commands
}

fn dotnet(root: &Path) -> Vec<(String, String)> {
    let entries = std::fs::read_dir(root).into_iter().flatten().flatten();
    let files: Vec<PathBuf> = (entries.map(|entry| entry.path()))
        .filter(|path| {
            let extension = path.extension().and_then(|extension| extension.to_str());
            extension.is_some_and(|extension| MARKER_EXTENSIONS.contains(&extension))
        })
        .collect();
    if files.is_empty() {
        return Vec::new();
    }
    let texts: Vec<String> = (files.iter())
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect();
    let any = |needle: &str| texts.iter().any(|text| text.contains(needle));
    let mut commands = Vec::new();
    // 只有可执行项目能 `dotnet run`，解决方案和类库不能。
    if any("<OutputType>Exe") || any("<OutputType>WinExe") || any("Microsoft.NET.Sdk.Web") {
        commands.push(entry("run", "dotnet run"));
    }
    commands.push(entry("build", "dotnet build"));
    // 测试项目引用这个包；解决方案里按惯例以 Tests 命名。
    if any("Microsoft.NET.Test.Sdk") || any("Tests.csproj") || any("Test.csproj") {
        commands.push(entry("test", "dotnet test"));
    }
    commands
}

fn ruby(root: &Path) -> Vec<(String, String)> {
    if !root.join("Gemfile").exists() {
        return Vec::new();
    }
    let mut commands = Vec::new();
    if root.join("bin/rails").exists() {
        commands.push(entry("server", "bundle exec rails server"));
    }
    if root.join("spec").exists() {
        commands.push(entry("test", "bundle exec rspec"));
    } else if read(root, "Rakefile").is_some_and(|text| text.contains("test")) {
        commands.push(entry("test", "bundle exec rake test"));
    }
    commands
}

fn php(root: &Path) -> Vec<(String, String)> {
    let mut commands = Vec::new();
    if root.join("artisan").exists() {
        commands.push(entry("serve", "php artisan serve"));
    }
    let scripts = json_keys(root, "composer.json", "scripts");
    let names = pick_scripts(scripts.iter().map(String::as_str));
    commands.extend(
        names
            .iter()
            .map(|name| entry(name, format!("composer run {name}"))),
    );
    commands
}

fn swift(root: &Path) -> Vec<(String, String)> {
    let Some(package) = read(root, "Package.swift") else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    if package.contains(".executable") {
        commands.push(entry("run", "swift run"));
    }
    commands.push(entry("build", "swift build"));
    if root.join("Tests").is_dir() {
        commands.push(entry("test", "swift test"));
    }
    commands
}

fn dart(root: &Path) -> Vec<(String, String)> {
    let Some(pubspec) = read(root, "pubspec.yaml") else {
        return Vec::new();
    };
    let flutter = pubspec
        .lines()
        .any(|line| line.trim_start().starts_with("flutter:"));
    let (tool, entry_point) = if flutter {
        ("flutter", "lib/main.dart")
    } else {
        ("dart", "bin")
    };
    let mut commands = Vec::new();
    if root.join(entry_point).exists() {
        commands.push(entry("run", format!("{tool} run")));
    }
    if root.join("test").is_dir() {
        commands.push(entry("test", format!("{tool} test")));
    }
    commands
}

fn elixir(root: &Path) -> Vec<(String, String)> {
    let Some(mix) = read(root, "mix.exs") else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    if mix.contains(":phoenix") {
        commands.push(entry("run", "mix phx.server"));
    }
    if root.join("test").is_dir() {
        commands.push(entry("test", "mix test"));
    }
    commands
}

fn zig(root: &Path) -> Vec<(String, String)> {
    let Some(build) = read(root, "build.zig") else {
        return Vec::new();
    };
    // `zig build` 的步骤由构建脚本定义，名字写成字符串。
    let mut commands = Vec::new();
    if build.contains("\"run\"") {
        commands.push(entry("run", "zig build run"));
    }
    commands.push(entry("build", "zig build"));
    if build.contains("\"test\"") {
        commands.push(entry("test", "zig build test"));
    }
    commands
}

fn cmake(root: &Path) -> Vec<(String, String)> {
    let Some(lists) = read(root, "CMakeLists.txt") else {
        return Vec::new();
    };
    let mut commands = vec![
        entry("configure", "cmake -S . -B build"),
        entry("build", "cmake --build build"),
    ];
    if lists.contains("enable_testing") || lists.contains("add_test") {
        commands.push(entry("test", "ctest --test-dir build"));
    }
    commands
}

/// 返回 `text` 里顶格写的 `名字:` 规则，用于 Makefile 和 justfile。
fn recipe_names(text: &str) -> Vec<&str> {
    let names = text.lines().filter_map(|line| {
        let (name, rest) = line.split_once(':')?;
        // `:=` 是变量赋值；以 `.` 开头的是 make 的特殊目标（`.PHONY`）。
        (!rest.starts_with('=') && !name.starts_with('.')).then_some(name)
    });
    let mut unique: Vec<&str> = Vec::new();
    for name in names {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    unique
}

fn make(root: &Path) -> Vec<(String, String)> {
    let Some(text) = read(root, "Makefile").or_else(|| read(root, "makefile")) else {
        return Vec::new();
    };
    let names = pick_scripts(recipe_names(&text));
    names
        .iter()
        .map(|name| entry(name, format!("make {name}")))
        .collect()
}

fn just(root: &Path) -> Vec<(String, String)> {
    let Some(text) = read(root, "justfile") else {
        return Vec::new();
    };
    let names = pick_scripts(recipe_names(&text));
    names
        .iter()
        .map(|name| entry(name, format!("just {name}")))
        .collect()
}

#[cfg(test)]
mod tests;
