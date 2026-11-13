//! 配置、状态和文档共用的文件操作：winc 的配置目录、原子写入用的临时文件和落盘。

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use winc_backend::local::{create_temporary, replace_file, sync_dir, sync_file};

/// 便携数据目录的名称，位于 `winc.exe` 所在的目录中（见 [`portable_dir`]）。
const PORTABLE_DIR: &str = "data";

/// 返回便携版的数据目录：Windows 上 `winc.exe` 所在的目录中有 `data` 目录时为该目录，否则为
/// `None`。结果在首次调用时确定。
///
/// 存在时配置、状态、未保存内容的备份、崩溃日志和单实例锁都放在其中，不写入用户目录。只在
/// Windows 上生效：便携版以 zip 分发，解压后的目录即程序所在的目录。
pub fn portable_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if !cfg!(windows) {
            return None;
        }
        portable_dir_beside(&std::env::current_exe().ok()?)
    })
    .clone()
}

/// 返回可执行文件 `exe` 旁边的便携数据目录；不存在或不是目录时返回 `None`。
pub(crate) fn portable_dir_beside(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?.join(PORTABLE_DIR);
    dir.is_dir().then_some(dir)
}

/// 便携版首次运行时导入用户目录中已有的配置和状态，返回是否导入了文件。
///
/// 便携数据目录中既没有 `config.json` 也没有 `state.json` 时，从用户目录复制这两个文件（存在哪个
/// 复制哪个）：由安装版或不使用数据目录的便携版换用时，设置和最近打开的目录不会丢失。只读取
/// 用户目录，不修改其中的文件。不是便携版时不做处理。
pub fn import_into_portable() -> bool {
    let Some(dir) = portable_dir() else {
        return false;
    };
    let state = crate::state::user_state_dir();
    import_missing(
        &dir,
        [(user_config_dir(), "config.json"), (state, "state.json")],
    )
}

/// 目录 `into` 中没有 `sources` 列出的任何文件时，把各来源目录中存在的同名文件复制进来，返回
/// 是否复制了文件；`into` 中已有其中任意一个时不做处理。
pub(crate) fn import_missing(into: &Path, sources: [(Option<PathBuf>, &str); 2]) -> bool {
    if sources.iter().any(|(_, name)| into.join(name).exists()) {
        return false;
    }
    let mut copied = false;
    for (dir, name) in sources {
        let Some(from) = dir.map(|dir| dir.join(name)).filter(|from| from.is_file()) else {
            continue;
        };
        copied |= std::fs::copy(&from, into.join(name)).is_ok();
    }
    copied
}

/// 返回 winc 的配置目录：便携版为便携数据目录（见 [`portable_dir`]），否则为用户目录中的配置
/// 目录，规则与 [`config_path`](crate::config_path) 相同；无法确定用户目录时返回 `None`。
pub(crate) fn config_dir() -> Option<PathBuf> {
    portable_dir().or_else(user_config_dir)
}

/// 返回用户目录中 winc 的配置目录，不考虑便携版；无法确定用户目录时返回 `None`。
pub(crate) fn user_config_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);

    if cfg!(target_os = "macos") {
        home().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("winc")
        })
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(home)
            .map(|dir| dir.join("winc"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".config")))
            .map(|dir| dir.join("winc"))
    }
}

/// 规范化路径：解开符号链接、`.` 和 `..`，规则同 [`std::fs::canonicalize`]。
///
/// Windows 上 [`std::fs::canonicalize`] 返回带 `\\?\` 前缀的路径：该前缀显示出来不直观，其他程序
/// 可能不认。不需要该前缀也能表示时去掉它，规则见 `simplify_verbatim`。
///
/// # Errors
///
/// 路径不存在或无法读取时返回对应的 `std::io::Error`。
#[allow(clippy::disallowed_methods)] // 唯一直接调用 `std::fs::canonicalize` 的地方
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    // 远端主机上的路径在连接时已取得真实路径，原样返回；本机的文件系统中没有它。
    if winc_backend::is_remote(path.as_ref()) {
        return Ok(path.as_ref().to_path_buf());
    }
    let path = std::fs::canonicalize(path)?;
    Ok(if cfg!(windows) {
        simplify_verbatim(path)
    } else {
        path
    })
}

/// 去掉 Windows 路径中不必要的 `\\?\` 前缀，见 [`canonicalize`]；去掉后含义可能改变时原样返回。
///
/// 网络路径换回 `\\服务器\共享`，盘符开头的路径去掉前缀；去掉后的路径不超过 `MAX_PATH`、各段不
/// 以点或空格结尾时才去掉。
fn simplify_verbatim(path: PathBuf) -> PathBuf {
    /// 不带 `\\?\` 前缀时路径的最大长度（`MAX_PATH`，含结尾的 NUL）。
    const MAX_PATH: usize = 260;
    let Some(text) = path.to_str() else {
        return path;
    };
    let plain = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let drive = rest.as_bytes();
        if drive.len() < 3
            || !drive[0].is_ascii_alphabetic()
            || drive[1] != b':'
            || drive[2] != b'\\'
        {
            return path;
        }
        rest.to_owned()
    } else {
        return path;
    };
    let awkward = plain
        .split('\\')
        .any(|part| part.ends_with('.') || part.ends_with(' '));
    if plain.len() >= MAX_PATH || awkward {
        return path;
    }
    PathBuf::from(plain)
}

/// 原子地写入 `path`：先写同目录下的临时文件并落盘，再改名覆盖，中途断电不会留下半截文件。
///
/// `path` 是符号链接时写到链接指向的文件，链接本身保留；文件已存在时新文件沿用原来的权限。
/// 目录不存在时先创建；失败时删除临时文件。
///
/// # Errors
///
/// 目录创建失败、临时文件写入或落盘失败、改名失败时返回对应的 `std::io::Error`。
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let path = canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or(Cow::Borrowed("file"), |n| n.to_string_lossy());
    let existing = std::fs::metadata(&path).ok();
    let (temp, mut file) = create_temporary(dir, &name, existing.is_some())?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        sync_file(&file)?;
        if let Some(metadata) = &existing {
            file.set_permissions(metadata.permissions())?;
        }
        replace_file(&temp, &path)?;
        sync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests;
