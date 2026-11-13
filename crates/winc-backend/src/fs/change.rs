//! 改动文件系统的操作：建目录、新建文件、改名、复制、删除。
//!
//! 函数名和语义与 `std::fs` 中的同名函数相同。虚拟路径上的操作在远端执行，此时还可能返回
//! 连接的错误（见 [`fs`](crate::fs) 的说明）。改名要求两个路径在同一台机器上，否则返回
//! [`io::ErrorKind::Unsupported`]；复制可以跨机器（本机与远端之间、两台远端之间），内容经
//! 本进程中转。新建文件和覆盖写入的内容连同路径必须放进一帧（约 1 MiB）。

#[cfg(test)]
mod tests;

use std::io::{self, Read, Write};
use std::path::Path;

use crate::{locate, remote};

/// 创建目录 `path`；上级目录必须已存在。
///
/// # Errors
///
/// 与 `std::fs::create_dir` 相同。
pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::create_dir(path),
        Some(at) => remote::connection(&at.target)?.create_dir(&at.path, false),
    }
}

/// 创建目录 `path` 及其不存在的各级上级目录；目录已存在时成功。
///
/// # Errors
///
/// 与 `std::fs::create_dir_all` 相同。
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::create_dir_all(path),
        Some(at) => remote::connection(&at.target)?.create_dir(&at.path, true),
    }
}

/// 新建文件 `path` 并写入 `contents`；`path` 上已有任何条目时不覆盖。
///
/// 写入失败时删除写了一部分的文件。
///
/// # Errors
///
/// `path` 已存在时返回 [`io::ErrorKind::AlreadyExists`]；其余与创建和写入文件相同。
pub fn create_new(path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(at) = locate(path) {
        return remote::connection(&at.target)?.create_new(&at.path, contents);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(error) = file.write_all(contents) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// 把 `contents` 写入 `path`，文件已存在时覆盖全部内容。
///
/// 不经过临时文件，写入中途失败时文件只剩部分内容；文档的保存使用 [`save`](super::save)。
///
/// # Errors
///
/// 与 `std::fs::write` 相同。
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::write(path, contents),
        Some(at) => remote::connection(&at.target)?.write(&at.path, contents.as_ref()),
    }
}

/// 把 `from` 改名为 `to`。
///
/// # Errors
///
/// 与 `std::fs::rename` 相同。
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    match remote::pair(from, to)? {
        None => std::fs::rename(from, to),
        Some((connection, from, to)) => connection.rename(&from, &to),
    }
}

/// 把文件 `from` 的内容和权限复制到 `to`，返回复制的字节数；`to` 已存在时覆盖。
///
/// # Errors
///
/// 与 `std::fs::copy` 相同。
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    copy_watched(from, to, &|_| true)
}

/// 同 [`copy`]，复制期间把新复制的字节数报告给 `watch`；`watch` 返回假时停止复制。
///
/// 两者不在同一台机器上时内容分段传输，每段报告一次，停止后 `to` 不出现残缺的文件。在同一台
/// 机器上时由那台机器一次复制完：开始之前报告一次（0 字节），完成之后报告一次，开始之后不能
/// 停止。
///
/// # Errors
///
/// `watch` 返回假而停止时返回 [`io::ErrorKind::Interrupted`]，其余与 `std::fs::copy` 相同。
pub fn copy_watched(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
    watch: &dyn Fn(u64) -> bool,
) -> io::Result<u64> {
    let (from, to) = (from.as_ref(), to.as_ref());
    if crosses(from, to) {
        return transfer(from, to, watch);
    }
    if !watch(0) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let copied = match remote::pair(from, to)? {
        None => std::fs::copy(from, to),
        Some((connection, from, to)) => connection.copy(&from, &to),
    }?;
    watch(copied);
    Ok(copied)
}

/// 是否 `from` 与 `to` 不在同一台机器上。
fn crosses(from: &Path, to: &Path) -> bool {
    let host = |path| locate(path).map(|at| at.target);
    host(from) != host(to)
}

/// 把 `from` 的内容经本进程传到另一台机器上的 `to`，返回传输的字节数。
///
/// 内容边读边写，不整个留在内存中，每次读取 [`remote::MAX_READ`] 字节（读取远端文件时一次
/// 往返取回的上限）；写入按 [`save`](super::save) 的流程，中途失败或被 `watch` 停止时 `to`
/// 不出现残缺的文件。源文件在 Unix 上时把它的权限位带过去（脚本的可执行位）。
fn transfer(from: &Path, to: &Path, watch: &dyn Fn(u64) -> bool) -> io::Result<u64> {
    let mut opened = super::open(from, 0)?;
    // 只带读写执行位：设置用户、设置组的位不带到另一台机器上。
    let mode = opened.metadata.unix.map(|unix| unix.mode & 0o777);
    let (mut copied, mut stopped) = (0, false);
    let mut reader = opened.reader()?;
    let options = super::SaveOptions::default();
    let saved = super::save(to, &options, |out| {
        let mut buffer = vec![0; remote::MAX_READ as usize];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            out.write_all(&buffer[..count])?;
            copied += count as u64;
            if !watch(count as u64) {
                stopped = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
    });
    if stopped {
        return Err(io::ErrorKind::Interrupted.into());
    }
    match saved {
        Ok(_) => {}
        Err(super::SaveError::Io(error)) => return Err(error),
        Err(super::SaveError::ReadOnly) => return Err(io::ErrorKind::PermissionDenied.into()),
        Err(other) => return Err(io::Error::other(other.to_string())),
    }
    if let Some(mode) = mode {
        set_mode(to, mode)?;
    }
    Ok(copied)
}

/// 把 `path` 的权限位设为 `mode`（低 12 位）；本机是 Windows 时什么都不做。
///
/// # Errors
///
/// 与 `std::fs::set_permissions` 相同。
pub fn set_mode(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    let (path, mode) = (path.as_ref(), mode & 0o7777);
    if let Some(at) = locate(path) {
        return remote::connection(&at.target)?.set_mode(&at.path, mode);
    }
    #[cfg(unix)]
    return std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode));
    #[cfg(not(unix))]
    return Ok(());
}

/// 创建指向 `target` 的符号链接 `link`；`target` 原样写入，不解析。
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    if let Some(at) = locate(link) {
        let target = target.as_os_str().as_encoded_bytes();
        return remote::connection(&at.target)?.symlink(target, &at.path);
    }
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(target, link);
}

/// 把符号链接 `from` 复制为指向同一目标的符号链接 `to`，不复制目标的内容。
///
/// 两者不在同一台机器上时目标的写法原样带过去：指向所复制的目录以内的相对链接仍然有效，
/// 其余的在另一台机器上可能没有对应的目标。
///
/// Windows 上链接分为指向目录的和指向文件的两种，按 `from` 此刻指向的目标选择；创建符号链接
/// 需要相应的权限。
///
/// # Errors
///
/// 读取或创建链接失败时返回对应的 `io::Error`。
pub fn copy_symlink(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    if crosses(from, to) {
        return symlink(&super::read_link(from)?, to);
    }
    if let Some((connection, from, to)) = remote::pair(from, to)? {
        return connection.copy_symlink(&from, &to);
    }
    let target = std::fs::read_link(from)?;
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, to);
    #[cfg(windows)]
    return if std::fs::metadata(from).is_ok_and(|m| m.is_dir()) {
        std::os::windows::fs::symlink_dir(target, to)
    } else {
        std::os::windows::fs::symlink_file(target, to)
    };
}

/// 删除文件 `path`（符号链接删除链接本身）。
///
/// # Errors
///
/// 与 `std::fs::remove_file` 相同。
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::remove_file(path),
        Some(at) => remote::connection(&at.target)?.remove_file(&at.path),
    }
}

/// 删除 `path`：文件和符号链接删除其本身，目录连同其中的全部内容一起删除。不可恢复。
///
/// # Errors
///
/// `path` 不存在，或其中有无法删除的项时返回对应的 `std::io::Error`；后一种情况下已删除的
/// 部分不恢复。
pub fn remove_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None if std::fs::symlink_metadata(path)?.is_dir() => std::fs::remove_dir_all(path),
        None => std::fs::remove_file(path),
        Some(at) => remote::connection(&at.target)?.remove_all(&at.path),
    }
}
