//! 虚拟路径：远端主机上的文件在本机的表示，及其与连接目标、远端路径之间的换算。
//!
//! 虚拟路径位于一个本机上不存在、也无法创建的根之下，根由固定前缀和连接目标组成：
//!
//! - Unix：`/dev/null/winc-remote:<目标>/…`。`/dev/null` 是字符设备而不是目录，其下的任何路径
//!   对包括 root 在内的所有用户都以「不是目录」失败，也无法被创建。
//! - Windows：设备命名空间中的 `\\.\winc-remote-<目标>\…`，没有同名的设备。
//!
//! `join`、`parent`、`strip_prefix` 等 `Path` 操作在虚拟路径上与本机路径行为相同。

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::Target;

/// 虚拟根所在的位置（Unix）：一个不是目录的设备文件，见模块说明。
#[cfg(unix)]
const BASE: [&str; 2] = ["dev", "null"];
/// 虚拟根的固定前缀，其后是连接目标。Unix 上以 `:` 结尾：远端路径写作 `主机:/路径`。
#[cfg(unix)]
const PREFIX: &str = "winc-remote:";
/// 虚拟根的固定前缀，其后是连接目标。Windows 的设备名不能含 `:`，以 `-` 结尾。
#[cfg(windows)]
const PREFIX: &str = "winc-remote-";

/// 虚拟路径指向的位置：连接目标和远端主机上的路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 连接目标。
    pub target: Target,
    /// 远端主机上的绝对路径（字节，以 `/` 开头，不含 `.`、`..` 和重复的 `/`；根目录是 `/`）。
    pub path: Vec<u8>,
}

impl Location {
    /// 返回最后一段的名字；根目录没有名字，返回空。
    pub fn name(&self) -> &[u8] {
        let start = self
            .path
            .iter()
            .rposition(|&b| b == b'/')
            .map_or(0, |at| at + 1);
        &self.path[start..]
    }

    /// 返回相对于目录 `root` 的路径（不以 `/` 开头，就是 `root` 本身时为空）；不在同一连接上
    /// 或不在 `root` 之下时返回 `None`。
    pub fn relative_to(&self, root: &Location) -> Option<&[u8]> {
        if self.target != root.target {
            return None;
        }
        let rest = self.path.strip_prefix(root.path.as_slice())?;
        match rest {
            [] => Some(rest),
            [b'/', rest @ ..] => Some(rest),
            // 根目录以 `/` 结尾，其下的路径去掉它之后直接就是相对路径。
            _ if root.path == b"/" => Some(rest),
            // 只是名字的前缀相同（`/a/bc` 与 `/a/b`），不在其下。
            _ => None,
        }
    }
}

/// 返回连接 `target` 上的远端路径 `remote` 对应的虚拟路径。
///
/// `remote` 按 `/` 分段，空段和 `.` 忽略，因此相对路径按相对于根目录处理。
pub fn virtual_path(target: &Target, remote: &[u8]) -> PathBuf {
    let mut path = root(target);
    let segments = remote.split(|&b| b == b'/');
    for segment in segments.filter(|s| !s.is_empty() && *s != b".") {
        path.push(segment_text(segment));
    }
    path
}

/// 返回目录 `dir` 下的相对路径 `relative`（以 `/` 分段，例如用户输入的 `a/b.rs`）。
///
/// `dir` 是本机路径时与 `Path::join` 相同。`dir` 是虚拟路径时 `relative` 按远端的名字处理：
/// 在需要转义的系统上，其中的字符转义后才成为虚拟路径的一段，直接 `join` 会得到指向别处或无法
/// 解析的路径。
pub fn join(dir: &Path, relative: &str) -> PathBuf {
    match locate(dir) {
        None => dir.join(relative),
        Some(at) => {
            let mut remote = at.path;
            remote.push(b'/');
            remote.extend_from_slice(relative.as_bytes());
            virtual_path(&at.target, &remote)
        }
    }
}

/// 解析虚拟路径 `path`；`path` 是本机路径时返回 `None`。
///
/// `..` 在远端的根目录处不再上移，结果不会落在该连接的虚拟根之外。
pub fn locate(path: &Path) -> Option<Location> {
    let mut components = path.components();
    let target = leading_target(&mut components)?;
    let mut segments: Vec<Vec<u8>> = Vec::new();
    for component in components {
        match component {
            Component::Normal(name) => segments.push(segment_bytes(name)?),
            Component::ParentDir => _ = segments.pop(),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let mut path = Vec::new();
    for segment in &segments {
        path.push(b'/');
        path.extend_from_slice(segment);
    }
    if path.is_empty() {
        path.push(b'/');
    }
    Some(Location { target, path })
}

/// 是否为虚拟路径，即指向远端主机上的文件。
pub fn is_remote(path: &Path) -> bool {
    leading_target(&mut path.components()).is_some()
}

/// 确认 `path` 是本机路径。
///
/// # Errors
///
/// `path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。只能访问本机文件的操作（例如写入
/// 配置文件）据此拒绝远端路径，避免把虚拟路径交给本机的文件系统。
pub fn ensure_local(path: &Path) -> io::Result<()> {
    if is_remote(path) {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(())
}

/// 返回虚拟路径 `path` 的显示写法 `目标:/路径`；`path` 是本机路径时返回 `None`。
///
/// 远端路径中不构成有效 UTF-8 的字节显示为替换字符，结果只用于显示。
pub fn display(path: &Path) -> Option<String> {
    let location = locate(path)?;
    let remote = String::from_utf8_lossy(&location.path);
    Some(format!("{}:{remote}", location.target.as_str()))
}

/// 返回 `path` 在文件所在的机器上的写法：本机路径原样返回，虚拟路径返回远端的路径。
///
/// 用于把路径交给用户（复制路径）或在远端执行的程序。不构成有效 UTF-8 的字节换成替换字符，
/// 结果不能再换算回原路径。
pub fn native_lossy(path: &Path) -> Cow<'_, str> {
    match locate(path) {
        None => path.to_string_lossy(),
        Some(location) => String::from_utf8_lossy(&location.path).into_owned().into(),
    }
}

/// 返回 `path` 相对于目录 `root` 的写法；不在 `root` 之下、两者不在同一台机器上时返回 `None`，
/// 就是 `root` 本身时为空字符串。
///
/// 远端路径一律以 `/` 分隔；本机路径使用本机的分隔符。
pub fn relative_lossy(path: &Path, root: &Path) -> Option<String> {
    match (locate(path), locate(root)) {
        (None, None) => {
            let relative = path.strip_prefix(root).ok()?;
            Some(relative.to_string_lossy().into_owned())
        }
        (Some(path), Some(root)) => {
            let relative = path.relative_to(&root)?;
            Some(String::from_utf8_lossy(relative).into_owned())
        }
        _ => None,
    }
}

/// 返回连接 `target` 的虚拟根。
fn root(target: &Target) -> PathBuf {
    #[cfg(unix)]
    let text = format!("/{}/{}/{PREFIX}{}", BASE[0], BASE[1], target.as_str());
    // Windows 的设备名中不能有 `:`：端口之前的 `:` 写成 `+`（连接目标中没有这个字符）。
    #[cfg(windows)]
    let text = format!(r"\\.\{PREFIX}{}\", target.as_str().replace(':', "+"));
    PathBuf::from(text)
}

/// 消耗 `components` 开头属于虚拟根的部分并返回连接目标；不是虚拟路径时返回 `None`。
#[cfg(unix)]
fn leading_target(components: &mut std::path::Components<'_>) -> Option<Target> {
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    for base in BASE {
        if components.next() != Some(Component::Normal(base.as_ref())) {
            return None;
        }
    }
    let Component::Normal(first) = components.next()? else {
        return None;
    };
    Target::new(first.to_str()?.strip_prefix(PREFIX)?)
}

/// 消耗 `components` 开头属于虚拟根的部分并返回连接目标；不是虚拟路径时返回 `None`。
#[cfg(windows)]
fn leading_target(components: &mut std::path::Components<'_>) -> Option<Target> {
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let std::path::Prefix::DeviceNS(device) = prefix.kind() else {
        return None;
    };
    Target::new(&device.to_str()?.strip_prefix(PREFIX)?.replace('+', ":"))
}

/// 把远端的文件名 `name` 换算为虚拟路径中的名字，与 [`virtual_path`] 对各段的换算相同。
pub(crate) fn segment_name(name: &[u8]) -> std::ffi::OsString {
    segment_text(name).into()
}

/// 把远端路径的一段换算为虚拟路径的一段。Unix 的路径分段可以是任意字节，原样使用。
#[cfg(unix)]
fn segment_text(segment: &[u8]) -> &std::ffi::OsStr {
    std::os::unix::ffi::OsStrExt::from_bytes(segment)
}

/// 把远端路径的一段换算为虚拟路径的一段，见 [`escape::encode`](crate::escape::encode)。
#[cfg(windows)]
fn segment_text(segment: &[u8]) -> String {
    crate::escape::encode(segment)
}

/// 把虚拟路径的一段还原为远端路径的一段。
#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "与 Windows 上可能失败的实现保持相同的签名"
)]
fn segment_bytes(name: &std::ffi::OsStr) -> Option<Vec<u8>> {
    Some(std::os::unix::ffi::OsStrExt::as_bytes(name).to_vec())
}

/// 把虚拟路径的一段还原为远端路径的一段；不是 [`escape::encode`](crate::escape::encode) 的
/// 结果时返回 `None`。
#[cfg(windows)]
fn segment_bytes(name: &std::ffi::OsStr) -> Option<Vec<u8>> {
    crate::escape::decode(name.to_str()?)
}
