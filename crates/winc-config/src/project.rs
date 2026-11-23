//! 项目文件（`.winc-project`）：一组目录的定义，可提交到版本库。
//!
//! 文件为 JSONC，`folders` 列出各目录：`path` 为相对项目文件所在目录的路径或绝对路径，`name`
//! 为可选的显示名；远端目录另写 `host`（`ssh` 主机名），`path` 为远端绝对路径。第一项为打开的
//! 目录，其余为附加目录。打开的标签等现场记在状态文件中（见 [`crate::Session`]），不写入项目
//! 文件。改写时只替换 `folders` 的值，保留其余内容和注释。
//!
//! 其他编辑器的项目文件只读：VS Code 的 `.code-workspace`、Sublime Text 的 `.sublime-project`
//! 同样列出 `folders`；IDEA 的项目见 [`foreign`]。

mod foreign;
#[cfg(test)]
mod tests;

pub use foreign::idea_folders;

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::Json;
use crate::files::write_atomic;

/// 项目文件的扩展名，不含点。
pub const PROJECT_EXTENSION: &str = "winc-project";

/// 项目文件中的一个目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFolder {
    /// 目录的路径：相对路径已按项目文件所在的目录展开。
    pub path: PathBuf,
    /// 文件中写的显示名；没有写时为 `None`。
    pub name: Option<String>,
}

/// 读取项目文件失败的原因。
#[derive(Debug)]
pub enum ProjectError {
    /// 读不了文件。
    Io(io::Error),
    /// 不是有效的项目文件：不是 JSON，或没有列出任何目录。
    Invalid,
}

/// 项目文件属于哪个编辑器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    /// Winc 的 `.winc-project`，可以改写。
    Winc,
    /// VS Code 的 `.code-workspace`。
    VsCode,
    /// Sublime Text 的 `.sublime-project`。
    Sublime,
    /// IDEA 基于文件的项目（`.ipr`）。
    Idea,
}

/// 能打开的项目文件的扩展名（不含点），Winc 的在前。
pub const PROJECT_EXTENSIONS: &[&str] = &[
    PROJECT_EXTENSION,
    "code-workspace",
    "sublime-project",
    "ipr",
];

/// 按扩展名（不区分大小写）判断 `path` 是哪个编辑器的项目文件；都不是时返回 `None`。
pub fn project_kind(path: &Path) -> Option<ProjectKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        PROJECT_EXTENSION => Some(ProjectKind::Winc),
        "code-workspace" => Some(ProjectKind::VsCode),
        "sublime-project" => Some(ProjectKind::Sublime),
        "ipr" => Some(ProjectKind::Idea),
        _ => None,
    }
}

/// `path` 是否为 Winc 的项目文件：扩展名为 [`PROJECT_EXTENSION`]，不区分大小写。
pub fn is_project_file(path: &Path) -> bool {
    project_kind(path) == Some(ProjectKind::Winc)
}

/// 读取项目文件 `path`（种类见 [`project_kind`]），返回其中的目录，按文件中的顺序。
///
/// # Errors
///
/// 读不了文件时返回 [`ProjectError::Io`]，内容无效或不是项目文件时返回 [`ProjectError::Invalid`]。
pub fn read_project(path: &Path) -> Result<Vec<ProjectFolder>, ProjectError> {
    let kind = project_kind(path).ok_or(ProjectError::Invalid)?;
    let text = std::fs::read_to_string(path).map_err(ProjectError::Io)?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let folders = match kind {
        ProjectKind::Idea => foreign::parse_ipr(&text, dir),
        ProjectKind::Winc | ProjectKind::VsCode | ProjectKind::Sublime => parse_project(&text, dir),
    };
    folders.ok_or(ProjectError::Invalid)
}

/// 解析项目文件的内容 `text`，相对路径按目录 `dir` 展开；无效或没有目录时返回 `None`。
///
/// 各项的位置写在 `path` 中，远端的另有 `host`；VS Code 也可以写成 `uri`：本机的 `file://`，
/// 或 SSH 远端的 `vscode-remote://ssh-remote+主机/路径`。
pub fn parse_project(text: &str, dir: &Path) -> Option<Vec<ProjectFolder>> {
    let (root, _) = Json::parse_jsonc(text)?;
    let entries = root.as_object()?.get("folders")?.as_array()?;
    let folders: Vec<ProjectFolder> = entries
        .iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let host = entry.get("host").and_then(Json::as_str);
            let path = match (host, entry.get("path").and_then(Json::as_str)) {
                (_, Some("")) => return None,
                (Some(host), Some(path)) => {
                    let target = winc_backend::Target::new(host)?;
                    winc_backend::virtual_path(&target, path.as_bytes())
                }
                (None, Some(path)) => dir.join(path),
                (_, None) => foreign::folder_uri(entry.get("uri")?.as_str()?)?,
            };
            let name = entry.get("name").and_then(Json::as_str);
            Some(ProjectFolder {
                path,
                name: name.filter(|n| !n.trim().is_empty()).map(str::to_owned),
            })
        })
        .collect();
    (!folders.is_empty()).then_some(folders)
}

/// 把目录 `folders` 写入项目文件 `path`，返回是否改写了文件。
///
/// 文件已列出同样的目录时不改写；文件中已有的显示名按目录保留。文件已存在时只替换 `folders`
/// 的值，其余内容和注释不变。
///
/// # Errors
///
/// 读写失败时返回对应的 `io::Error`；已有的文件不是有效的 JSON 时返回 `InvalidData`，不覆盖它。
pub fn write_project(path: &Path, folders: &[PathBuf]) -> io::Result<bool> {
    let dir = path.parent().unwrap_or(Path::new(""));
    let old = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let listed = old.as_deref().and_then(|text| parse_project(text, dir));
    if let Some(listed) = &listed
        && listed.len() == folders.len()
        && listed
            .iter()
            .zip(folders)
            .all(|(a, b)| same_dir(&a.path, b))
    {
        return Ok(false);
    }
    let name_of = |folder: &Path| {
        let listed = listed.iter().flatten();
        listed
            .filter(|f| same_dir(&f.path, folder))
            .find_map(|f| f.name.clone())
    };
    let entries: Vec<Entry> = folders
        .iter()
        .map(|folder| {
            let name = name_of(folder);
            match winc_backend::locate(folder) {
                Some(at) => Entry {
                    host: Some(at.target.as_str().to_owned()),
                    path: String::from_utf8_lossy(&at.path).into_owned(),
                    name,
                },
                None => Entry {
                    host: None,
                    path: portable_path(&relative(dir, folder)),
                    name,
                },
            }
        })
        .collect();
    let text = match old {
        None => format!("{{\n    \"folders\": {}\n}}\n", folders_value(&entries)),
        Some(text) => {
            if Json::parse_jsonc(&text).is_none() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            replace_folders(&text, &folders_value(&entries))
        }
    };
    write_atomic(path, &text)?;
    Ok(true)
}

/// `a` 和 `b` 是否为同一个目录：都能规范化时比较规范化后的路径。
fn same_dir(a: &Path, b: &Path) -> bool {
    match (crate::files::canonicalize(a), crate::files::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 写入文件的一个目录：远端主机、路径和显示名。
struct Entry {
    host: Option<String>,
    path: String,
    name: Option<String>,
}

/// `folders` 的值：每个目录一行，缩进与新建的文件一致。
fn folders_value(entries: &[Entry]) -> String {
    let text = |value: &str| Json::String(value.to_owned()).compact();
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            let mut fields = Vec::new();
            if let Some(host) = &entry.host {
                fields.push(format!("\"host\": {}", text(host)));
            }
            fields.push(format!("\"path\": {}", text(&entry.path)));
            if let Some(name) = &entry.name {
                fields.push(format!("\"name\": {}", text(name)));
            }
            format!("        {{ {} }}", fields.join(", "))
        })
        .collect();
    format!("[\n{}\n    ]", lines.join(",\n"))
}

/// 在顶层对象中把 `folders` 的值换成 `value`；没有这一项时加在对象开头。
fn replace_folders(text: &str, value: &str) -> String {
    match folders_span(text) {
        Some((start, end)) => format!("{}{value}{}", &text[..start], &text[end..]),
        None => {
            let open = text.find('{').map_or(0, |at| at + 1);
            let rest = text[open..].trim_start();
            let comma = if rest.starts_with('}') { "" } else { "," };
            format!(
                "{}\n    \"folders\": {value}{comma}{}",
                &text[..open],
                &text[open..]
            )
        }
    }
}

/// 返回顶层对象中 `folders` 的值在 `text` 中的字节范围；没有这一项时返回 `None`。
fn folders_span(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut at = skip_blank(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at += 1;
    loop {
        at = skip_blank(bytes, at);
        match bytes.get(at)? {
            b'}' => return None,
            b',' => {
                at += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = skip_string(bytes, at)?;
        let key = &text[at + 1..key_end - 1];
        at = skip_blank(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        let start = skip_blank(bytes, at + 1);
        let end = skip_value(bytes, start)?;
        if key == "folders" {
            return Some((start, end));
        }
        at = end;
    }
}

/// 从 `at` 起跳过空白和注释，返回之后的位置。
fn skip_blank(bytes: &[u8], mut at: usize) -> usize {
    loop {
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(b' ' | b'\t' | b'\r' | b'\n'), _) => at += 1,
            (Some(b'/'), Some(b'/')) => {
                while bytes.get(at).is_some_and(|&b| b != b'\n') {
                    at += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                at += 2;
                while at < bytes.len() && !(bytes[at] == b'*' && bytes.get(at + 1) == Some(&b'/')) {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
            }
            _ => return at,
        }
    }
}

/// `at` 处是字符串的左引号时，返回右引号之后的位置。
fn skip_string(bytes: &[u8], mut at: usize) -> Option<usize> {
    at += 1;
    while let Some(&b) = bytes.get(at) {
        match b {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// 返回从 `at` 开始的值的末尾位置（不含其后的空白、逗号）。
fn skip_value(bytes: &[u8], mut at: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut end = at;
    while let Some(&b) = bytes.get(at) {
        match b {
            b'"' => {
                at = skip_string(bytes, at)?;
                end = at;
                continue;
            }
            b'/' if matches!(bytes.get(at + 1), Some(b'/' | b'*')) => {
                at = skip_blank(bytes, at);
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return Some(end),
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            b',' if depth == 0 => return Some(end),
            _ => {}
        }
        at += 1;
        if !matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            end = at;
        }
    }
    (depth == 0).then_some(end)
}

/// `path` 相对目录 `dir` 的路径；两者不在同一个根（如 Windows 的不同盘）时返回 `path` 本身。
fn relative(dir: &Path, path: &Path) -> PathBuf {
    let dir: Vec<Component<'_>> = dir.components().collect();
    let path_parts: Vec<Component<'_>> = path.components().collect();
    if dir.first() != path_parts.first() {
        return path.to_path_buf();
    }
    let common = dir
        .iter()
        .zip(&path_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..dir.len() {
        out.push("..");
    }
    for part in &path_parts[common..] {
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// 写入文件的路径：相对路径以 `/` 分隔，各系统通用；绝对路径原样。
fn portable_path(path: &Path) -> String {
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}
