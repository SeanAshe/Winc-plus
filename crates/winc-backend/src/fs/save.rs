//! 保存：把内容原子地写入文件，全部步骤在文件所在的机器上执行。
//!
//! 先写同目录下的临时文件，落盘后恢复原文件的权限和属组，再改名替换目标文件；断电或进程中止后
//! 磁盘上是完整的旧文件或新文件。改名替换不可行或会改变文件属性时退回原地写入，见 [`save`]。

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use super::Metadata;
use crate::local::{create_temporary, replace_file, sync_dir, sync_file};
use crate::{locate, remote};

/// UTF-8 的字节序标记。
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// 写入时的缓冲区大小（字节，1 MiB）：内容按片段写出，缓冲区合并小片段，减少系统调用次数。
const BUFFER: usize = 1 << 20;

/// 替换已有文件之前的回调，参数依次为目标文件和写好的临时文件。
pub type BeforeReplace<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

/// [`save`] 的选项。
#[derive(Clone, Copy, Default)]
pub struct SaveOptions<'a> {
    /// 是否先写入 UTF-8 的字节序标记。
    pub bom: bool,
    /// 是否全部内容都在内存中：为真时，改名替换不可行或会改变文件属性的情况改为原地写入；
    /// 为假时内容引用着目标文件本身（大文件的片段），无法原地写入，这些情况返回错误。
    pub in_memory: bool,
    /// 是否在目标已存在且 `in_memory` 为真时直接原地写入：改名替换会丢失文件属性的文件系统使用。
    pub prefer_in_place: bool,
    /// 替换已有文件之前调用，用于把目标文件的属性复制到临时文件；返回错误时放弃保存。
    pub before_replace: Option<BeforeReplace<'a>>,
}

/// [`save`] 失败的原因。
#[derive(Debug)]
pub enum SaveError {
    /// 目标文件只读。
    ReadOnly,
    /// 改名替换会改变文件的身份（有多个硬链接，或属主不是当前用户），而内容无法原地写入。
    IdentityWouldChange,
    /// 无法创建临时文件：重试后名字仍被占用。
    TemporaryFile,
    /// 输入输出错误。
    Io(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("the file is read-only"),
            Self::IdentityWouldChange => {
                f.write_str("replacing the file would change its identity")
            }
            Self::TemporaryFile => f.write_str("cannot create a temporary file"),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// 把 `write` 写出的内容保存到 `path`，返回写入后的文件信息。
///
/// 文件所在的目录不存在时先创建。已有文件的权限和属组保留（macOS 上还有扩展属性和 ACL）；
/// 新文件使用默认权限。写入失败时删除临时文件，目标文件不变。
///
/// # Errors
///
/// 见 [`SaveError`]；`path` 是虚拟路径时还有以 [`SaveError::Io`] 返回的连接的错误。
pub fn save(
    path: impl AsRef<Path>,
    options: &SaveOptions<'_>,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> Result<Metadata, SaveError> {
    let path = path.as_ref();
    // 远端：内容传给服务进程，由它在文件所在的机器上执行下面的同一套流程。两个只用于 WSL
    // 的选项不传递。
    if let Some(at) = locate(path) {
        let connection = remote::connection(&at.target)?;
        return connection.save(&at.path, options.bom, options.in_memory, write);
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or(Cow::Borrowed("file"), |n| n.to_string_lossy());
    // 文件所在的目录可能已被删除（例如在文件树中删除了整个目录，而该文件的标签仍保留）：
    // 重新创建后再保存。
    std::fs::create_dir_all(dir)?;
    let existing = std::fs::metadata(path).ok();
    // `rename` 只检查目录的写权限，不检查目标文件的权限；目标文件只读时必须在此拒绝，
    // 否则该文件会被替换。
    if existing
        .as_ref()
        .is_some_and(|m| m.permissions().readonly())
    {
        return Err(SaveError::ReadOnly);
    }
    let in_place = options.in_memory && existing.is_some();
    if in_place && options.prefer_in_place {
        return Ok(write_in_place(path, options.bom, write)?);
    }
    if existing.as_ref().is_some_and(rename_changes_identity) {
        if options.in_memory {
            return Ok(write_in_place(path, options.bom, write)?);
        }
        return Err(SaveError::IdentityWouldChange);
    }
    // 临时文件名带随机后缀且只新建、不跟随符号链接（`create_new`）：目录其他用户也能写时，
    // 对方预先放一个同名的符号链接就能让保存写进链接指向的文件。
    let (tmp, file) = match create_temporary(dir, &name, existing.is_some()) {
        Ok(created) => created,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(SaveError::TemporaryFile),
        // 目录不可写而文件可写：无法创建临时文件，改为原地写入。
        Err(e) if in_place && e.kind() == io::ErrorKind::PermissionDenied => {
            return Ok(write_in_place(path, options.bom, write)?);
        }
        Err(e) => return Err(e.into()),
    };
    let result = (|| {
        let file = write_content(file, options.bom, write)?;
        if let Some(metadata) = &existing {
            // 临时文件新建时只给自己读写，写完再换成原文件的权限，写的过程中其他用户读不到内容。
            file.set_permissions(metadata.permissions())?;
            // 新文件的属组是当前用户的默认组（macOS 上是目录的属组），改回原文件的属组；当前用户
            // 不在该组里时系统拒绝，沿用新文件的属组。
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let _ = std::os::unix::fs::fchown(&file, None, Some(metadata.gid()));
            }
            #[cfg(target_vendor = "apple")]
            copy_extended_attributes(path, &file);
            if let Some(before_replace) = options.before_replace {
                before_replace(path, &tmp)?;
            }
        }
        // 关闭之后再取文件信息：部分系统在关闭句柄时才更新修改时间。临时文件的名字是随机的，
        // 其他程序不会在此期间修改它；改名不改变文件信息。
        drop(file);
        let written = Metadata::from(&std::fs::metadata(&tmp)?);
        replace_file(&tmp, path)?;
        sync_dir(dir);
        Ok(written)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(SaveError::Io)
}

/// 向 `file` 写入字节序标记（`bom` 为真时）和 `write` 写出的内容，同步到磁盘后返回该文件。
fn write_content(
    file: std::fs::File,
    bom: bool,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<std::fs::File> {
    let mut out = io::BufWriter::with_capacity(BUFFER, file);
    if bom {
        out.write_all(&UTF8_BOM)?;
    }
    write(&mut out)?;
    let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    sync_file(&file)?;
    Ok(file)
}

/// 原地写入 `path`：截断后写入新内容，再同步到磁盘，返回写入后的文件信息。
///
/// 不经过临时文件：写入中途失败（磁盘已满、进程中止）时文件只剩部分内容，因此只在改名替换不可行
/// 或会改变文件属性时使用，见 [`save`]。
fn write_in_place(
    path: &Path,
    bom: bool,
    write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<Metadata> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    // 关闭之后再取文件信息，原因见 [`save`] 中的说明。
    drop(write_content(file, bom, write)?);
    Ok(Metadata::from(&std::fs::metadata(path)?))
}

/// 改名替换是否会改变已有文件 `metadata` 的身份：有多个硬链接（改名只替换一个名字，其他名字
/// 仍指向旧内容），或属主不是当前用户（改名后属主变为当前用户）。
#[cfg(unix)]
// 本函数只有对 `libc::geteuid` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn rename_changes_identity(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: `geteuid` 没有参数，不访问内存，总是成功。
    let user = unsafe { libc::geteuid() };
    metadata.nlink() > 1 || metadata.uid() != user
}

/// 改名替换是否会改变已有文件 `metadata` 的身份；非 Unix 平台不检查。
#[cfg(not(unix))]
fn rename_changes_identity(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// 把 `source` 的扩展属性（包括彩色标签）和 ACL 复制到 `target`。
///
/// 尽力而为：读不到原文件或文件系统不支持时不复制，保存本身不受影响。
#[cfg(target_vendor = "apple")]
// 本函数只有对 `libc::fcopyfile` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
fn copy_extended_attributes(source: &Path, target: &std::fs::File) {
    use std::os::fd::AsRawFd;
    let Ok(source) = std::fs::File::open(source) else {
        return;
    };
    // SAFETY: 两个参数都是调用期间被借用、保持打开的有效文件描述符；状态参数传空指针，
    // `fcopyfile` 允许这样做且不会写入；失败时只返回 -1。
    unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            target.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL | libc::COPYFILE_XATTR,
        );
    }
}
