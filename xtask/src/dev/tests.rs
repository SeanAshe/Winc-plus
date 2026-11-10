//! 影响范围计算的测试：改动的文件如何带出依赖这些包的包，以及全局文件的影响。

use super::*;

fn member(name: &str, dir: &str, deps: &[&str]) -> Member {
    Member {
        name: name.to_owned(),
        dir: PathBuf::from(dir),
        deps: deps.iter().map(|d| (*d).to_owned()).collect(),
    }
}

#[test]
fn changes_pull_in_the_crates_that_depend_on_them() {
    // 依赖链为 `winc-core` ← `winc-editor` ← {`winc-app`, `xtask`}；`winc-render` 不依赖任何包。
    let members = [
        member("winc-app", "crates/winc-app", &["winc-editor", "winc-core"]),
        member("winc-core", "crates/winc-core", &[]),
        member("winc-editor", "crates/winc-editor", &["winc-core"]),
        member("winc-render", "crates/winc-render", &[]),
        member("xtask", "xtask", &["winc-editor"]),
    ];
    let affected = |files: &[&str]| {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        affected(&members, &files)
    };
    assert_eq!(
        affected(&["crates/winc-render/src/lib.rs"]),
        ["winc-render"]
    );
    assert_eq!(affected(&["crates/winc-app/src/app.rs"]), ["winc-app"]);
    assert_eq!(
        affected(&["crates/winc-core/src/text.rs"]),
        ["winc-app", "winc-core", "winc-editor", "xtask"]
    );
    assert!(affected(&["docs/GIT.md"]).is_empty());
    // 根目录的 `Cargo.lock`、`VERSION` 变了时全部 5 个包都要测。
    assert_eq!(affected(&["Cargo.lock"]).len(), 5);
    assert_eq!(affected(&["VERSION"]).len(), 5);
}

#[test]
fn workspace_dependencies_follow_the_layers() {
    let members = members(&root()).unwrap();
    assert_eq!(layer_violations(&members), Vec::<String>::new());
    // 未登记的依赖会被报告。
    let extra = [member("winc-core", "crates/winc-core", &["winc-render"])];
    assert_eq!(layer_violations(&extra), ["winc-core → winc-render"]);
}

/// `winc-app` 的逻辑层（`app/`）只在帧入口调用绘制层，并以字段持有绘制缓存的类型。
#[test]
fn app_logic_does_not_reference_the_view() {
    const ALLOWED: [&str; 5] = [
        "view::draw",
        "view::base",
        "view::LineTokens",
        "view::MinimapRows",
        "view::RulerCache",
    ];
    fn scan(dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name != "snapshot" && name != "tests" {
                    scan(&path, found);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" {
                scan_file(&path, found);
            }
        }
    }
    fn scan_file(path: &Path, found: &mut Vec<String>) {
        {
            let text = std::fs::read_to_string(path).unwrap();
            for (number, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or(line);
                let mut rest = code;
                while let Some(at) = rest.find("view::") {
                    let before = rest[..at].chars().next_back();
                    let reference = &rest[at..];
                    if !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                        && !ALLOWED.iter().any(|a| reference.starts_with(a))
                    {
                        found.push(format!("{}:{}", path.display(), number + 1));
                    }
                    rest = &rest[at + 6..];
                }
            }
        }
    }
    let src = root().join("crates/winc-app/src");
    let mut found = Vec::new();
    scan(&src.join("app"), &mut found);
    scan_file(&src.join("app.rs"), &mut found);
    assert_eq!(found, Vec::<String>::new());
}
