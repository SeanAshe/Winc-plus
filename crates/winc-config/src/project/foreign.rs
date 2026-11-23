//! 其他编辑器的项目：VS Code 写成 `file://` 的位置，以及 IDEA 的模块。
//!
//! IDEA 的项目在 `.ipr`（基于文件）或 `.idea/modules.xml`（基于目录）中列出模块文件（`.iml`），
//! 模块文件中的 `<content url>` 是模块的内容根目录。路径中的 `$PROJECT_DIR$` 指项目目录，
//! `$MODULE_DIR$` 指模块文件所在的目录；模块文件放在 `.idea` 中时它指项目目录。

use std::path::{Component, Path, PathBuf};

use super::ProjectFolder;

/// VS Code 写成 `uri` 的目录对应的路径：本机的 `file://`，以及 SSH 远端的
/// `vscode-remote://ssh-remote+主机/路径`（对应的虚拟路径）；其他位置返回 `None`。
pub(super) fn folder_uri(uri: &str) -> Option<PathBuf> {
    if let Some(rest) = uri.strip_prefix("vscode-remote://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = percent_decode(authority.strip_prefix("ssh-remote+")?)?;
        let target = winc_backend::Target::new(&host)?;
        let path = percent_decode(path)?;
        return Some(winc_backend::virtual_path(&target, path.as_bytes()));
    }
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let path = percent_decode(rest)?;
    // Windows 的位置写成 `/C:/dir`，去掉盘符前的 `/`。
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':';
    let path = if drive { &path[1..] } else { &path[..] };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// 还原 `%XX` 转义；结果不是 UTF-8 或转义不完整时返回 `None`。
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 解析 IDEA 基于文件的项目（`.ipr`）：项目目录 `dir` 在前，其后是在它之外的模块内容根目录。
pub(super) fn parse_ipr(text: &str, dir: &Path) -> Option<Vec<ProjectFolder>> {
    if !text.contains("<project") {
        return None;
    }
    let root = std::iter::once(dir.to_path_buf());
    let folders = root
        .chain(outer_roots(text, dir))
        .map(|path| ProjectFolder { path, name: None });
    Some(folders.collect())
}

/// 基于目录的 IDEA 项目 `dir`（含 `.idea/modules.xml`）中在 `dir` 之外的模块内容根目录；不是
/// IDEA 项目或没有这样的目录时为空。
pub fn idea_folders(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_to_string(dir.join(".idea").join("modules.xml")) {
        Ok(text) => outer_roots(&text, dir),
        Err(_) => Vec::new(),
    }
}

/// 项目文件 `xml` 中各模块的内容根目录里，不在项目目录 `dir` 之内的，按出现的顺序，不重复。
fn outer_roots(xml: &str, dir: &Path) -> Vec<PathBuf> {
    let dir = lexical(dir);
    let mut roots: Vec<PathBuf> = Vec::new();
    for module in attributes(xml, "module", "filepath") {
        let file = lexical(&expand(&module, &dir, &dir));
        let Ok(iml) = std::fs::read_to_string(&file) else {
            continue;
        };
        let parent = file.parent().unwrap_or(&dir);
        let module_dir = if parent.file_name().is_some_and(|name| name == ".idea") {
            parent.parent().unwrap_or(&dir)
        } else {
            parent
        };
        for url in attributes(&iml, "content", "url") {
            let Some(path) = url.strip_prefix("file://") else {
                continue;
            };
            let root = lexical(&expand(path, &dir, module_dir));
            if !root.starts_with(&dir) && !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

/// 展开路径中的 `$PROJECT_DIR$`（`project`）和 `$MODULE_DIR$`（`module`）。
fn expand(value: &str, project: &Path, module: &Path) -> PathBuf {
    let text = value
        .replace("$PROJECT_DIR$", &project.to_string_lossy())
        .replace("$MODULE_DIR$", &module.to_string_lossy());
    PathBuf::from(text)
}

/// 去掉路径中的 `.`，并按字面消去 `..`，不访问文件系统。
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `xml` 中各个 `<tag …>` 元素的 `name` 属性值，按出现的顺序，已还原 XML 的字符实体。
fn attributes(xml: &str, tag: &str, name: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let key = format!("{name}=\"");
    let mut values = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        rest = &rest[at + open.len()..];
        // `<module` 也是 `<modules` 的开头：标签名之后须是空白、`/` 或 `>`。
        if !rest.starts_with([' ', '\t', '\r', '\n', '/', '>']) {
            continue;
        }
        let element = &rest[..rest.find('>').unwrap_or(rest.len())];
        let value = element
            .match_indices(&key)
            .find(|&(at, _)| at == 0 || element[..at].ends_with([' ', '\t', '\r', '\n']))
            .and_then(|(at, _)| {
                let value = &element[at + key.len()..];
                value.find('"').map(|end| &value[..end])
            });
        if let Some(value) = value {
            values.push(unescape(value));
        }
    }
    values
}

/// 还原 XML 的五个预定义字符实体。
fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
