//! Windows 任务栏跳转列表中列出的最近目录：各条目显示的标题和启动参数。
//!
//! 条目取自状态文件的最近列表（`State::recent`），顺序相同。与平台接口无关的部分放在这里，
//! 在各平台上都能测试。

use std::path::{Path, PathBuf};

/// 返回 `folders` 各目录在跳转列表中显示的标题，顺序与 `folders` 相同。
///
/// 标题为目录名；多个目录同名时在其后附上所在目录（用户主目录缩写为 `~`），使各条目可以区分。
/// 没有目录名的路径（如根目录）显示完整路径。
pub(crate) fn labels(folders: &[PathBuf]) -> Vec<String> {
    let name = |folder: &Path| folder.file_name().map(|n| n.to_string_lossy().into_owned());
    let names: Vec<Option<String>> = folders.iter().map(|f| name(f)).collect();
    let shared = |name: &Option<String>| names.iter().filter(|n| *n == name).count() > 1;
    folders
        .iter()
        .zip(&names)
        .map(|(folder, name)| match (name, folder.parent()) {
            (Some(name), Some(parent)) if shared(&Some(name.clone())) => {
                format!("{name} — {}", winc_editor::display_path(parent, None))
            }
            (Some(name), _) => name.clone(),
            (None, _) => winc_editor::display_path(folder, None),
        })
        .collect()
}

/// 返回在 Windows 上把目录 `folder` 作为单个命令行参数传给 winc 的写法：两侧加引号。
///
/// 结尾的反斜杠后面紧跟引号时，引号会被当作转义的字面量；因此把结尾的反斜杠写两遍（如盘符的
/// 根目录 `C:\`）。Windows 的路径中不含引号，无需其他转义。
pub(crate) fn argument(folder: &Path) -> String {
    let path = folder.to_string_lossy();
    let slashes = path.len() - path.trim_end_matches('\\').len();
    format!("\"{path}{}\"", "\\".repeat(slashes))
}

#[cfg(test)]
mod tests;
