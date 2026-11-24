//! 工具自动选择的测试：总是使用的预设、最近的迹象优先、只按声明的依赖判断、项目内程序的查找，
//! 以及 Rust 版次的判定。

use super::*;
use crate::test_util::scratch;
use crate::tools::format::tests::fake_tool;

/// 没有迹象时按顺序尝试的几个工具的名称。
fn installed(detected: Detected) -> Vec<&'static str> {
    match detected {
        Detected::Installed(presets) => presets.iter().map(|p| p.name).collect(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn rust_and_go_are_always_formatted_others_use_a_sign_or_what_is_installed() {
    let dir = scratch("detect-always");
    let detected = |language| detect(Task::Format, language, &dir);
    assert!(matches!(detected("rust"), Detected::Preset(p, None) if p.name == "rustfmt"));
    assert!(matches!(detected("go"), Detected::Preset(p, None) if p.name == "gofmt"));
    assert_eq!(installed(detected("javascript")), ["Prettier", "Biome"]);
    assert_eq!(installed(detected("python")), ["Black", "Ruff"]);
    assert_eq!(installed(detected("shell")), ["shfmt"]);
    // `jq` 会改写内容，不作为回退候选。
    assert_eq!(installed(detected("json")), ["Prettier", "Biome"]);
    assert_eq!(detected("xml"), Detected::NoPreset);
    assert_eq!(detected("makefile"), Detected::NoPreset);
}

#[test]
fn the_nearest_sign_wins() {
    let root = scratch("detect-nearest");
    let app = root.join("packages/app/src");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(root.join(".prettierrc"), "{}").unwrap();
    std::fs::write(root.join("packages/app/biome.json"), "{}").unwrap();

    let reason = |dir: &Path| match detect(Task::Format, "typescript", dir) {
        Detected::Preset(p, reason) => (p.name, reason.unwrap()),
        other => panic!("{other:?}"),
    };
    assert_eq!(reason(&app), ("Biome", "biome.json".to_owned()));
    assert_eq!(reason(&root), ("Prettier", ".prettierrc".to_owned()));
    // 距离相同时按预设的顺序。
    std::fs::write(root.join("packages/app/.prettierrc.json"), "{}").unwrap();
    assert_eq!(reason(&app), ("Prettier", ".prettierrc.json".to_owned()));
}

#[test]
fn package_json_counts_only_declared_dependencies() {
    let root = scratch("detect-package");
    std::fs::write(
        root.join("package.json"),
        r#"{"name": "x", "devDependencies": {"eslint": "^9"}}"#,
    )
    .unwrap();
    // 间接依赖被安装进 `node_modules/.bin`，不算在使用。
    fake_tool(&root.join("node_modules/.bin"), "prettier", "cat");
    assert!(matches!(
        detect(Task::Format, "javascript", &root),
        Detected::Installed(_)
    ));

    std::fs::write(
        root.join("package.json"),
        r#"{"devDependencies": {"prettier": "^3"}}"#,
    )
    .unwrap();
    assert!(matches!(
        detect(Task::Format, "css", &root),
        Detected::Preset(p, Some(r)) if p.name == "Prettier" && r == "package.json"
    ));

    std::fs::write(
        root.join("package.json"),
        r#"{"prettier": "@company/style"}"#,
    )
    .unwrap();
    assert!(
        matches!(detect(Task::Format, "yaml", &root), Detected::Preset(p, _) if p.name == "Prettier")
    );
    std::fs::write(root.join("package.json"), "{ 不是 JSON").unwrap();
    assert!(matches!(
        detect(Task::Format, "yaml", &root),
        Detected::Installed(_)
    ));
}

/// Astro 只有声明了 `prettier-plugin-astro` 才用 Prettier；只装了 Prettier 时不回退到它，
/// 否则会报找不到 `.astro` 的解析器。
#[test]
fn astro_uses_prettier_only_with_the_plugin() {
    let root = scratch("detect-astro");
    assert_eq!(detect(Task::Format, "astro", &root), Detected::NoPreset);

    std::fs::write(
        root.join("package.json"),
        r#"{"devDependencies": {"prettier": "^3", "prettier-plugin-astro": "^0.14"}}"#,
    )
    .unwrap();
    assert!(matches!(
        detect(Task::Format, "astro", &root),
        Detected::Preset(p, Some(reason)) if p.name == "Prettier" && reason == "package.json"
            // Prettier 3 不自动加载依赖中的插件，命令中必须指名。
            && p.command.contains("--plugin prettier-plugin-astro")
    ));

    std::fs::write(
        root.join("package.json"),
        r#"{"devDependencies": {"prettier": "^3"}}"#,
    )
    .unwrap();
    assert_eq!(detect(Task::Format, "astro", &root), Detected::NoPreset);
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn black_is_preferred_when_the_project_configures_both() {
    let root = scratch("detect-python");
    std::fs::write(
        root.join("pyproject.toml"),
        "[tool.ruff]\nline-length = 100\n\n[tool.black]\nline-length = 100\n",
    )
    .unwrap();
    assert!(
        matches!(detect(Task::Format, "python", &root), Detected::Preset(p, _) if p.name == "Black")
    );

    std::fs::write(
        root.join("pyproject.toml"),
        "[tool.ruff.lint]\nselect = []\n",
    )
    .unwrap();
    assert!(
        matches!(detect(Task::Format, "python", &root), Detected::Preset(p, _) if p.name == "Ruff")
    );

    std::fs::remove_file(root.join("pyproject.toml")).unwrap();
    fake_tool(&root.join(".venv/bin"), "black", "cat");
    assert!(matches!(
        detect(Task::Format, "python", &root),
        Detected::Preset(p, Some(r)) if p.name == "Black" && r == ".venv"
    ));
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn project_programs_are_found_from_the_file_upwards() {
    let root = scratch("find-in-project");
    let deep = root.join("a/b/c");
    std::fs::create_dir_all(&deep).unwrap();
    let tool = fake_tool(&root.join("a/node_modules/.bin"), "prettier", "cat");
    assert_eq!(
        find_in_project("prettier", &deep),
        Some((tool, root.join("a")))
    );
    assert_eq!(find_in_project("prettier", &root), None);
    // 不可执行的文件不算。
    std::fs::create_dir_all(root.join(".venv/bin")).unwrap();
    std::fs::write(root.join(".venv/bin/ruff"), "").unwrap();
    assert_eq!(find_in_project("ruff", &deep), None);
}

#[test]
fn rust_edition_follows_cargo_then_rustfmt_toml() {
    let root = scratch("edition");
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nedition = \"2024\"\n",
    );
    write(
        "crates/a/Cargo.toml",
        "[package]\nname = \"a\"\nedition.workspace = true\n",
    );
    write(
        "crates/b/Cargo.toml",
        "[package]\nname = \"b\"\nedition = { workspace = true }\n",
    );
    write(
        "crates/c/Cargo.toml",
        "[package]\nname = \"c\" # edition = \"2018\"\nedition = '2021'\n",
    );
    write("crates/d/Cargo.toml", "[package]\nname = \"d\"\n");
    write(
        "solo/Cargo.toml",
        "[workspace]\n\n[workspace.package]\nedition = \"2021\"\n\n[package]\nname = \"solo\"\nedition.workspace = true\n",
    );

    let edition = |dir: &str| rust_edition(&root.join(dir));
    assert_eq!(edition("crates/a/src").as_deref(), Some("2024"));
    assert_eq!(edition("crates/b/src").as_deref(), Some("2024"));
    assert_eq!(edition("crates/c/src").as_deref(), Some("2021"));
    // 没写 edition 的 crate 为 2015（Cargo 清单格式的缺省值）。
    assert_eq!(edition("crates/d").as_deref(), Some("2015"));
    // 工作区的根本身就是该 crate。
    assert_eq!(edition("solo/src").as_deref(), Some("2021"));

    let alone = scratch("edition-alone");
    std::fs::create_dir_all(alone.join("loose")).unwrap();
    std::fs::write(alone.join("loose/rustfmt.toml"), "edition = \"2018\"\n").unwrap();
    assert_eq!(rust_edition(&alone.join("loose")).as_deref(), Some("2018"));
    assert_eq!(rust_edition(&alone), None);
}
