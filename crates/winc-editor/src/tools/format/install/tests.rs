//! 安装说明的测试：每个预设都有安装方式、本机可用的命令排在前面、项目内安装优先、路径引用，
//! 以及缺少已知工具时的处理。

use super::*;
use crate::test_util::scratch;
use crate::tools::format::PRESETS;

#[test]
fn every_preset_says_how_to_install_it() {
    for preset in PRESETS {
        assert!(knows(preset.program()), "{}", preset.program());
    }
    for install in INSTALLS {
        assert!(!install.ways.is_empty() && install.url.starts_with("https://"));
    }
}

#[test]
fn available_ways_come_first() {
    let dir = scratch("install-plain");
    let text = guide(&["ruff"], &dir, |via| via == "pipx").unwrap();
    let pipx = text.find("pipx install ruff").unwrap();
    let uv = text.find("uv tool install ruff").unwrap();
    assert!(text.starts_with("未找到「ruff」。"));
    assert!(text.contains("本机可用") && pipx < uv, "{text}");
    assert!(text.contains("https://docs.astral.sh/ruff/installation/"));

    // 一个包管理器都没有：仍然列出全部命令，并说明需要先安装对应的包管理器。
    let text = guide(&["ruff"], &dir, |_| false).unwrap();
    assert!(text.contains("需先安装") && text.contains("uv tool install ruff"));
    // 其他操作系统的安装方式不列出。
    if !cfg!(windows) {
        assert!(!guide(&["jq"], &dir, |_| true).unwrap().contains("winget"));
    }
    assert_eq!(guide(&["my-formatter"], &dir, |_| true), None);
}

#[test]
fn paths_are_quoted_for_the_shell() {
    assert_eq!(shell_quote("/Users/me/my-app"), "/Users/me/my-app");
    assert_eq!(shell_quote("/Users/me/my app"), "'/Users/me/my app'");
    assert_eq!(shell_quote("/tmp/it's"), r"'/tmp/it'\''s'");
}

#[test]
fn several_tools_are_listed_one_by_one() {
    let dir = scratch("install-several");
    let text = guide(&["black", "ruff"], &dir, |_| true).unwrap();
    assert!(text.starts_with("未找到格式化工具。安装以下任一工具后"));
    assert!(text.contains("\nblack\n") && text.contains("\nruff\n"));
    assert!(text.contains("    在终端中运行") && text.contains("        uv tool install ruff"));
}

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn project_dependencies_come_first() {
    let root = scratch("install-project");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"devDependencies": {"prettier": "^3"}}"#,
    )
    .unwrap();
    std::fs::write(root.join("pnpm-lock.yaml"), "").unwrap();

    let text = guide(&["prettier"], &src, |_| true).unwrap();
    let project = text.find("pnpm install").unwrap();
    assert!(
        project < text.find("npm install --global").unwrap(),
        "{text}"
    );
    assert!(text.contains("package.json 已声明此依赖"));
    assert!(text.contains(&format!("cd {}\n", root.display())), "{text}");
    // 依赖中没有声明该包：说明改为添加开发依赖。
    let text = guide(&["biome"], &src, |_| true).unwrap();
    assert!(
        text.contains("pnpm add --save-dev @biomejs/biome"),
        "{text}"
    );

    std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
    assert!(!guide(&["ruff"], &src, |_| true).unwrap().contains("--dev"));
    std::fs::write(root.join("uv.lock"), "").unwrap();
    assert!(
        guide(&["ruff"], &src, |_| true)
            .unwrap()
            .contains("uv add --dev ruff")
    );
}
