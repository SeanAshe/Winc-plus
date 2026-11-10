//! 日常开发：只测改动涉及的 crate（`cargo xtask test`），清掉调试构建里过时的目标文件
//! （`cargo xtask tidy`）。
//!
//! macOS 上调试构建的调试信息留在 `target/debug/deps` 的各个 `.rcgu.o` 里（`split-debuginfo =
//! "unpacked"`），cargo 不清理；数量达到数十万个后，每次启动需要数十秒。改成 `packed` 可避免堆积，
//! 但每次链接都要多执行几秒 `dsymutil`，因此继续用 `unpacked`，定期删除没有可执行文件引用的文件。

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::root;

/// workspace 里的一个 crate：包名、所在目录（相对根目录）、依赖的 workspace 内的包。
struct Member {
    /// 包名。
    name: String,
    /// 所在目录（相对 workspace 根目录）。
    dir: PathBuf,
    /// 依赖的 workspace 内包名。
    deps: Vec<String>,
}

/// 各 crate 允许依赖的 workspace 内的包；未列出的 crate 不允许依赖任何内部包。
///
/// 依赖只能指向更底层的 crate：`winc-core`、`winc-render` 不依赖任何内部包，`winc-editor` 不依赖
/// 窗口与渲染，只有 `winc-app` 同时依赖编辑器与渲染层。新增内部依赖时必须在此登记。
const LAYERS: &[(&str, &[&str])] = &[
    ("winc", &["winc-app"]),
    (
        "winc-app",
        &[
            "winc-config",
            "winc-core",
            "winc-editor",
            "winc-i18n",
            "winc-preview",
            "winc-pty",
            "winc-render",
            "winc-syntax",
            "winc-typeset",
            "winc-version",
        ],
    ),
    (
        "winc-editor",
        &[
            "winc-config",
            "winc-core",
            "winc-i18n",
            "winc-preview",
            "winc-syntax",
        ],
    ),
    (
        "winc-typeset",
        &["winc-i18n", "winc-preview", "winc-render", "winc-syntax"],
    ),
    ("winc-config", &["winc-i18n", "winc-syntax"]),
    ("winc-preview", &["winc-i18n"]),
    ("winc-syntax", &["winc-core"]),
    ("winc-pty", &["winc-version"]),
    // 仅测试依赖。
    ("winc-watch", &["winc-config"]),
    ("xtask", &["winc-config", "winc-preview", "winc-version"]),
];

/// 返回不在 [`LAYERS`] 允许范围内的依赖，每项写成「包 → 依赖」；全部合规时为空。
fn layer_violations(members: &[Member]) -> Vec<String> {
    let mut violations = Vec::new();
    for member in members {
        let allowed = LAYERS
            .iter()
            .find(|(name, _)| *name == member.name)
            .map_or(&[][..], |(_, deps)| deps);
        for dep in &member.deps {
            if !allowed.contains(&dep.as_str()) {
                violations.push(format!("{} → {dep}", member.name));
            }
        }
    }
    violations
}

/// 读出 workspace 的成员：`crates/` 下的每个目录和 `xtask`。
///
/// 依赖取每行第一个以 `winc-` 开头、且不同于包自身的标识符；写成 `名字.workspace = true` 或
/// `名字 = …` 都能识别。
///
/// # Errors
///
/// 读不到 `crates/` 或某个 `Cargo.toml`、包里没有包名时返回原因。
fn members(root: &Path) -> Result<Vec<Member>, String> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("crates"))
        .map_err(|e| format!("无法读取 crates/：{e}"))?
        .flatten()
        .map(|e| PathBuf::from("crates").join(e.file_name()))
        .filter(|d| root.join(d).join("Cargo.toml").is_file())
        .collect();
    dirs.push(PathBuf::from("xtask"));
    dirs.sort();

    let mut members = Vec::new();
    for dir in dirs {
        let manifest = std::fs::read_to_string(root.join(&dir).join("Cargo.toml"))
            .map_err(|e| format!("无法读取 {}/Cargo.toml：{e}", dir.display()))?;
        let name = manifest
            .lines()
            .find_map(|l| l.trim().strip_prefix("name = "))
            .map(|n| n.trim_matches('"').to_owned())
            .ok_or_else(|| format!("{}/Cargo.toml 里没有包名", dir.display()))?;
        let deps = manifest
            .lines()
            .filter_map(|l| {
                let key = l.split(['=', '.', ' ']).next()?.trim();
                (key.starts_with("winc-") && key != name).then(|| key.to_owned())
            })
            .collect();
        members.push(Member { name, dir, deps });
    }
    Ok(members)
}

/// 返回和 `HEAD` 相比改动过的文件（相对根目录），包括还没跟踪的新文件。
///
/// `base` 给出时和它比较。
///
/// # Errors
///
/// `git` 无法运行或退出状态非零时返回原因。
fn changed_files(root: &Path, base: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map_err(|e| format!("无法运行 git：{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "git {} 失败：{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    let diff = git(&["diff", "--name-only", base.unwrap_or("HEAD"), "--"])?;
    let untracked = git(&["ls-files", "--others", "--exclude-standard"])?;
    for line in diff.lines().chain(untracked.lines()) {
        if !line.is_empty() {
            files.insert(PathBuf::from(line));
        }
    }
    Ok(files.into_iter().collect())
}

/// 返回要测的包：改动的文件所在的包，以及直接、间接依赖这些包的包。
///
/// 根目录的 `Cargo.toml`、`Cargo.lock`、`VERSION`、`.cargo/`、`rust-toolchain.toml` 变了时全部要测；
/// 不属于任何包的文档文件不算。
fn affected(members: &[Member], changed: &[PathBuf]) -> Vec<String> {
    let global = [
        "Cargo.toml",
        "Cargo.lock",
        "VERSION",
        "rust-toolchain.toml",
        ".cargo",
    ];
    if changed
        .iter()
        .any(|f| global.iter().any(|g| f.starts_with(g)))
    {
        return members.iter().map(|m| m.name.clone()).collect();
    }
    let mut chosen: HashSet<&str> = members
        .iter()
        .filter(|m| changed.iter().any(|f| f.starts_with(&m.dir)))
        .map(|m| m.name.as_str())
        .collect();
    // 反复把依赖了已选包的包加进来，直到不再变化，得到依赖链上的全部包。
    loop {
        let before = chosen.len();
        for m in members {
            if m.deps.iter().any(|d| chosen.contains(d.as_str())) {
                chosen.insert(&m.name);
            }
        }
        if chosen.len() == before {
            break;
        }
    }
    members
        .iter()
        .filter(|m| chosen.contains(m.name.as_str()))
        .map(|m| m.name.clone())
        .collect()
}

/// `cargo xtask test [--all] [--base <提交>] [-- 传给 cargo test 的参数]`。
///
/// 只测改动涉及的包，测完清理过时的目标文件；`--` 之后的参数原样交给 `cargo test`，例如测试名的
/// 过滤条件。
///
/// # Errors
///
/// 参数不对、`git` 或 `cargo` 无法运行时返回原因。测试失败时返回 `Ok(false)`。
pub fn test(args: &[String]) -> Result<bool, String> {
    let root = root();
    let mut all = false;
    let mut base = None;
    let mut rest: &[String] = &[];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--all" => all = true,
            "--base" => {
                i += 1;
                base = Some(args.get(i).ok_or("--base 后面要跟提交或分支")?.as_str());
            }
            "--" => {
                rest = &args[i + 1..];
                break;
            }
            other => {
                return Err(format!(
                    "test 不认识参数 {other}（测试名的过滤条件写在 -- 之后）"
                ));
            }
        }
        i += 1;
    }

    let members = members(&root)?;
    let violations = layer_violations(&members);
    if !violations.is_empty() {
        return Err(format!(
            "依赖不符合分层规则（见 `LAYERS`）：{}",
            violations.join("、")
        ));
    }
    let packages = if all {
        members.iter().map(|m| m.name.clone()).collect()
    } else {
        affected(&members, &changed_files(&root, base)?)
    };
    if packages.is_empty() {
        println!("没有改动涉及任何 crate（只改了文档之类），不用测试。要全部测试用 --all。");
        return Ok(true);
    }
    println!("测试：{}", packages.join("、"));

    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command.current_dir(&root).args(["test", "--no-fail-fast"]);
    for p in &packages {
        command.args(["-p", p]);
    }
    if !rest.is_empty() {
        command.arg("--").args(rest);
    }
    let passed = command
        .status()
        .map_err(|e| format!("无法运行 cargo：{e}"))?
        .success();

    if let Err(e) = tidy() {
        eprintln!("xtask：清理目标文件失败：{e}");
    }
    Ok(passed)
}

/// `cargo xtask tidy`：删除 `target/debug/deps` 中已没有被任何可执行文件引用的 `.rcgu.o`。
///
/// 仅在 macOS 上执行，其他平台直接返回。当前的可执行文件引用的目标文件保留，崩溃时的调用栈和
/// 调试器需要使用。
///
/// # Errors
///
/// 读不了目录、`nm` 无法运行时返回原因。
pub fn tidy() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let debug = root().join("target").join("debug");
    let deps = debug.join("deps");
    let Ok(entries) = std::fs::read_dir(&deps) else {
        return Ok(());
    };
    let mut objects = Vec::new();
    let mut executables = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".rcgu.o") {
            objects.push(path);
        } else if is_executable(&path) {
            executables.push(path);
        }
    }
    for dir in [debug.clone(), debug.join("examples")] {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if is_executable(&entry.path()) {
                executables.push(entry.path());
            }
        }
    }

    let mut referenced: HashSet<PathBuf> = HashSet::new();
    for exe in &executables {
        let output = Command::new("nm")
            .args(["-ap"])
            .arg(exe)
            .output()
            .map_err(|e| format!("无法运行 nm：{e}"))?;
        // 调试映射里每个目标文件一行：`… OSO /…/deps/winc-….rcgu.o`。
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((_, path)) = line.split_once(" OSO ") {
                referenced.insert(PathBuf::from(path.trim()));
            }
        }
    }

    let (mut removed, mut bytes) = (0usize, 0u64);
    for object in objects {
        if !referenced.contains(&object) {
            bytes += std::fs::metadata(&object).map_or(0, |m| m.len());
            if std::fs::remove_file(&object).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        println!(
            "清理了 {removed} 个过时的目标文件（{:.1} MB）",
            bytes as f64 / 1e6
        );
    }
    Ok(())
}

/// 是否是可执行的普通文件（测试程序、`winc`）。
///
/// 目录、`.d` 依赖文件、库不算。
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.extension().is_none()
        && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests;
