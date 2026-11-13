//! 回收站（macOS 以外的 Unix 平台），按 freedesktop.org 的 Trash 规范。
//!
//! 删除的项在文件管理器中可见、可还原。
//! 位于家目录所在文件系统的项移入 `$XDG_DATA_HOME/Trash`，该变量未设置时
//! 移入 `~/.local/share/Trash`。其他文件系统上的项移入该文件系统顶层的 `.Trash-`，
//! 不跨文件系统复制内容。每项内容存放于 `files/`，同名 `.trashinfo` 存放于 `info/`，
//! 记录原路径与删除时间。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：回收站只接收本机的文件"
)]

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// 把 `path`（文件或目录）移到回收站，返回它在回收站 `files/` 里的位置。
///
/// # Errors
///
/// `path` 不存在、回收站或记录无法创建、移动失败时返回 `io::Error`；失败时原来的项不动。
pub fn move_to_trash(path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let device = fs::symlink_metadata(&path)?.dev();
    let home = home_trash();
    // 家目录的回收站可能尚未创建，此时用最近的已存在上级目录的设备号判断它所在的文件系统。
    // 设备号与 `path` 一致才能直接移动，否则改用 `path` 所在文件系统顶层的回收站。
    let home_device = home
        .ancestors()
        .find_map(|p| fs::metadata(p).ok())
        .map(|m| m.dev());
    if home_device == Some(device) {
        return trash_into(&home, &path, false);
    }
    let top = mount_point(&path, device);
    #[allow(unsafe_code)] // getuid
    // SAFETY: `getuid` 没有前提条件，总是成功，返回当前进程的有效用户 ID。
    let uid = unsafe { libc::getuid() };
    trash_into(&top.join(format!(".Trash-{uid}")), &path, true)
}

/// 家目录的回收站。
fn home_trash() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    data.join("Trash")
}

/// `path` 所在文件系统的挂载点。
///
/// 从 `path` 的各级上级目录向上查找，直到设备号与 `device` 不同的目录或根目录为止。
fn mount_point(path: &Path, device: u64) -> PathBuf {
    let mut top = path.to_path_buf();
    for dir in path.ancestors().skip(1) {
        match fs::metadata(dir) {
            Ok(m) if m.dev() == device => top = dir.to_path_buf(),
            _ => break,
        }
    }
    top
}

/// 把 `path` 移入回收站目录 `trash`（含 `files/` 与 `info/` 子目录），返回它在 `files/` 中的位置。
///
/// 与已有名字重名时依次改用「名字 2」「名字 3」：先以独占方式创建 `info/` 中的记录占用
/// 该名字，再移动内容，失败时删除记录。`private` 为真时回收站目录的权限为 700，
/// 这是 freedesktop.org Trash 规范对文件系统顶层回收站目录的要求。
///
/// # Errors
///
/// `path` 没有文件名、目录无法创建、记录无法写入或移动失败时返回 `io::Error`，且不留记录。
pub(crate) fn trash_into(trash: &Path, path: &Path, private: bool) -> io::Result<PathBuf> {
    let files = trash.join("files");
    let info = trash.join("info");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    if private {
        builder.mode(0o700);
    }
    builder.create(&files)?;
    builder.create(&info)?;

    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "没有名字的路径"))?
        .to_string_lossy()
        .into_owned();
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.clone(), String::new()),
    };
    let contents = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode_path(path),
        local_now()
    );
    for n in 1.. {
        let candidate = if n == 1 {
            name.clone()
        } else {
            format!("{stem} {n}{extension}")
        };
        let record = info.join(format!("{candidate}.trashinfo"));
        let target = files.join(&candidate);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&record)
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        // 记录已存在但 `files/` 中没有同名内容时，该记录是其他程序遗留的；`files/` 中也可能
        // 已有同名内容，因此改试下一个名字。
        if fs::symlink_metadata(&target).is_ok() {
            drop(file);
            let _ = fs::remove_file(&record);
            continue;
        }
        let moved = file
            .write_all(contents.as_bytes())
            .and_then(|()| fs::rename(path, &target));
        if let Err(e) = moved {
            let _ = fs::remove_file(&record);
            return Err(e);
        }
        return Ok(target);
    }
    unreachable!("名字的序号用不完")
}

/// `.trashinfo` 中的路径：按网址规则编码（字母、数字、`-_.~/` 以外的字节写成 `%XX`）。
///
/// 也用于拼 `file://` 网址（另见 `crate::system::reveal_in_file_manager`）。
pub(crate) fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 按 `.trashinfo` 要求的「YYYY-MM-DDThh:mm:ss」格式返回当前本地时间。
#[allow(unsafe_code)] // localtime_r
fn local_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::zeroed();
    // SAFETY: `localtime_r` 只读 `now`、只写传入的结构体，失败时返回空指针。
    let filled = unsafe { !libc::localtime_r(&now, tm.as_mut_ptr()).is_null() };
    if !filled {
        return String::from("1970-01-01T00:00:00");
    }
    // SAFETY: 调用成功时结构体已填好。
    let tm = unsafe { tm.assume_init() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests;
