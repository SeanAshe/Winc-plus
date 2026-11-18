//! 从本机安装的 IDEA 系列编辑器或 VS Code 导入快捷键：取得它们当前使用的键位方案，以及用户在
//! 方案之上改过的键。
//!
//! 只读取用户目录中的配置文件，不修改。认不出的操作和按键跳过，导入结果中记有跳过的项数。

mod idea;
mod ids;
mod vscode;

use std::path::{Path, PathBuf};

use super::*;

/// 可以导入快捷键的一处来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// 来源的编辑器。
    pub kind: Editor,
    /// 显示的名称，如「IntelliJ IDEA 2025.3」。
    pub name: String,
    /// 配置所在的目录。
    pub dir: PathBuf,
}

/// 导入快捷键的编辑器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
    /// IDEA 系列编辑器，配置目录中的键位方案为 XML。
    Idea,
    /// VS Code，用户目录中的 `keybindings.json`。
    VsCode,
}

/// 导入的结果：使用的键位方案、在方案之上改过的键和跳过的项数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    /// 键位方案。
    pub preset: Preset,
    /// 改过的键。
    pub custom: Custom,
    /// 认不出的操作或按键的项数。
    pub skipped: usize,
}

/// IDEA 系列编辑器的配置目录名前缀和显示的名称，目录名中前缀之后是版本号（如 `2025.3`）。
const IDEA_PRODUCTS: &[(&str, &str)] = &[
    ("IntelliJIdea", "IntelliJ IDEA"),
    ("IdeaIC", "IntelliJ IDEA CE"),
    ("PyCharm", "PyCharm"),
    ("PyCharmCE", "PyCharm CE"),
    ("WebStorm", "WebStorm"),
    ("GoLand", "GoLand"),
    ("CLion", "CLion"),
    ("RustRover", "RustRover"),
    ("PhpStorm", "PhpStorm"),
    ("Rider", "Rider"),
    ("RubyMine", "RubyMine"),
    ("DataGrip", "DataGrip"),
];

/// VS Code 各版本的用户目录名和显示的名称。
const VSCODE_PRODUCTS: &[(&str, &str)] = &[
    ("Code", "VS Code"),
    ("Code - Insiders", "VS Code Insiders"),
    ("VSCodium", "VSCodium"),
];

/// 返回本机可以导入快捷键的来源：IDEA 系列编辑器各取最新的版本，VS Code 取各版本的用户目录。
pub fn find_sources() -> Vec<Source> {
    let Some(root) = winc_config::files::user_config_root() else {
        return Vec::new();
    };
    let mut sources = idea_sources(&root.join("JetBrains"));
    for &(dir, name) in VSCODE_PRODUCTS {
        let dir = root.join(dir).join("User");
        if dir.is_dir() {
            sources.push(Source {
                kind: Editor::VsCode,
                name: name.to_owned(),
                dir,
            });
        }
    }
    sources
}

/// 返回 IDEA 系列编辑器的配置目录 `root` 下各产品最新版本的来源，按产品在
/// [`IDEA_PRODUCTS`] 中的顺序排列。
fn idea_sources(root: &Path) -> Vec<Source> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    // 各产品最新的版本号和目录。
    let mut newest: Vec<(usize, Vec<u32>, String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // 前缀可能是另一个产品的前缀（`PyCharm` 与 `PyCharmCE`），取版本号能读出的那个。
        let found = (IDEA_PRODUCTS.iter().enumerate()).find_map(|(i, &(prefix, _))| {
            let version = name.strip_prefix(prefix)?;
            let parts: Option<Vec<u32>> = version.split('.').map(|p| p.parse().ok()).collect();
            parts
                .filter(|p| !p.is_empty())
                .map(|p| (i, p, version.to_owned()))
        });
        let Some((product, parts, version)) = found else {
            continue;
        };
        match newest.iter_mut().find(|(p, ..)| *p == product) {
            Some(slot) if slot.1 >= parts => {}
            Some(slot) => *slot = (product, parts, version, entry.path()),
            None => newest.push((product, parts, version, entry.path())),
        }
    }
    newest.sort_by_key(|(product, ..)| *product);
    (newest.into_iter())
        .map(|(product, _, version, dir)| Source {
            kind: Editor::Idea,
            name: format!("{} {version}", IDEA_PRODUCTS[product].1),
            dir,
        })
        .collect()
}

/// 读取来源 `source` 中的快捷键，按键写法取 `os` 上的；读不了时返回界面语言的原因。
pub fn read(source: &Source, os: Os) -> Result<Imported, String> {
    match source.kind {
        Editor::Idea => idea::read(&source.dir, os),
        Editor::VsCode => vscode::read(&source.dir, os),
    }
}

#[cfg(test)]
mod tests;
