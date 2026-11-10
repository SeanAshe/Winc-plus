//! 运行外部程序（格式化工具、`git` 等）的公共部分：用户 shell 的环境（[`Shell`]）、在 `PATH`
//! 里找程序，以及带超时和取消的运行（`process::run`、[`Cancel`]）。
//!
//! 从图形界面启动的应用只继承系统提供的最小环境，`PATH` 里缺少用户自己装的工具，因此第一次
//! 用到时向用户的登录 shell 查询一次环境（见 `environment::resolve`），之后各处共用。

pub(crate) mod environment;
pub(crate) mod process;

pub use process::Cancel;

use std::ffi::{OsStr, OsString};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use environment::Vars;

/// 运行外部程序所用的环境，各线程、各功能（格式化、`git`）共用同一个实例。
#[derive(Debug, Clone)]
pub struct Shell(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    /// 第一次用到时才获取：需要运行一次登录 shell，用不到外部程序的用户不必付出这一开销。
    vars: OnceLock<Vars>,
    /// 测试中直接给定，不运行用户的登录 shell。
    fixed: Option<Vars>,
    /// `PATH` 各目录的文件清单，取到环境时列出，[`Shell::list`] 更新；尚未取到环境时为 `None`。
    listing: Mutex<Option<Arc<Listing>>>,
}

impl Default for Shell {
    fn default() -> Self {
        Shell::with(None)
    }
}

impl Shell {
    /// 使用给定的环境变量创建，不运行登录 shell（测试用）。
    pub fn with_vars(vars: Vec<(OsString, OsString)>) -> Self {
        Shell::with(Some(vars))
    }

    fn with(fixed: Option<Vars>) -> Self {
        let listing = fixed
            .as_ref()
            .map(|vars| Arc::new(Listing::read(path_value(vars, cfg!(windows)))));
        Shell(Arc::new(Shared {
            vars: OnceLock::new(),
            fixed,
            listing: Mutex::new(listing),
        }))
    }

    /// 是否已取到环境：已取到时在 `PATH` 里找程序无需再等待。
    pub fn is_ready(&self) -> bool {
        self.0.fixed.is_some() || self.0.vars.get().is_some()
    }

    /// 等到取到环境；已取到时立即返回。
    ///
    /// 第一次调用要运行登录 shell，会等待，应在后台线程调用。
    pub fn resolve(&self) {
        self.vars();
    }

    /// 返回运行外部程序所用的环境。
    ///
    /// 第一次调用时要运行登录 shell，最多等待 5 秒（`environment::SHELL_TIMEOUT`），随后列出
    /// `PATH` 的清单；同时调用的多个线程等待的是同一次运行。
    fn vars(&self) -> &Vars {
        match &self.0.fixed {
            Some(vars) => vars,
            None => self.0.vars.get_or_init(|| {
                let vars = environment::resolve();
                let listing = Listing::read(path_value(&vars, cfg!(windows)));
                *self.listing() = Some(Arc::new(listing));
                vars
            }),
        }
    }

    fn listing(&self) -> std::sync::MutexGuard<'_, Option<Arc<Listing>>> {
        self.0
            .listing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// 在 `PATH` 里找程序 `name`（Windows 上按 `PATHEXT` 补扩展名）；找不到时返回 `None`。
    ///
    /// 逐个目录检查文件，探测次数与 `PATH` 的目录数成正比，应在后台线程调用；第一次调用要先
    /// 取到环境，会等待。
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        let found = find_in_path(name, path_value(self.vars(), cfg!(windows)));
        // 清单中没有的程序是列出之后才安装的，此时更新清单。
        if found.is_some() && self.peek(name) == Some(None) {
            self.list();
        }
        found
    }

    /// 按 `PATH` 的清单找程序 `name`，不等待：尚未取到环境时返回 `None`。
    ///
    /// 清单已排除不存在的文件，只检查各目录中同名的文件是否可执行，耗时与 `PATH` 的目录数
    /// 无关，可以在界面线程上调用。列出之后才安装的程序要到下一次 [`Shell::list`] 才能找到。
    pub fn peek(&self, name: &str) -> Option<Option<PathBuf>> {
        let listing = self.listing().clone()?;
        Some(listing.find(name))
    }

    /// 重新列出 `PATH` 各目录的文件，返回清单是否有变化。
    ///
    /// 要读取每个目录，应在后台线程调用；第一次调用要先取到环境，会等待。
    pub fn list(&self) -> bool {
        let listing = Listing::read(path_value(self.vars(), cfg!(windows)));
        let mut current = self.listing();
        if current.as_deref() == Some(&listing) {
            return false;
        }
        *current = Some(Arc::new(listing));
        true
    }

    /// 返回运行 `program` 的命令：环境换成用户 shell 的环境，并设置 `NO_COLOR=1`。
    ///
    /// 第一次调用要先取到环境，会等待。
    pub fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(self.vars().iter().map(|(k, v)| (k, v)))
            .env("NO_COLOR", "1");
        command
    }
}

/// 返回环境 `vars` 中 `PATH` 的值。
///
/// Windows 上环境变量名不区分大小写，系统默认写作 `Path`；`ignore_case` 为真时按此比较。
fn path_value(vars: &Vars, ignore_case: bool) -> Option<&OsStr> {
    vars.iter()
        .find(|(name, _)| {
            if ignore_case {
                name.eq_ignore_ascii_case("PATH")
            } else {
                name == "PATH"
            }
        })
        .map(|(_, value)| value.as_os_str())
}

/// 在 `path`（`PATH` 的值）中查找程序；只考虑绝对路径的目录。
fn find_in_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let names = file_names(name, std::env::var_os("PATHEXT").as_deref());
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| is_executable(candidate))
}

/// `PATH` 中各个绝对路径的目录及其文件名的散列（升序），按 `PATH` 的顺序排列，重复的目录只保留
/// 第一个。
///
/// 散列为 `None` 的目录无法列出（存在但没有读取权限），查找时直接检查其中的文件。清单只用于
/// 排除不存在的文件，命中后仍检查文件本身，因此散列冲突和文件名的大小写不影响查找结果。
#[derive(Debug, PartialEq)]
struct Listing(Vec<(PathBuf, Option<Vec<u64>>)>);

impl Listing {
    /// 列出 `path`（`PATH` 的值）中各目录的文件。
    fn read(path: Option<&OsStr>) -> Listing {
        let mut dirs: Vec<(PathBuf, Option<Vec<u64>>)> = Vec::new();
        for dir in path.into_iter().flat_map(std::env::split_paths) {
            if dir.is_absolute() && dirs.iter().all(|(listed, _)| *listed != dir) {
                let names = names_in(&dir);
                dirs.push((dir, names));
            }
        }
        Listing(dirs)
    }

    /// 查找程序 `name`，目录和文件名的先后顺序与 [`find_in_path`] 相同。
    fn find(&self, name: &str) -> Option<PathBuf> {
        let names = file_names(name, std::env::var_os("PATHEXT").as_deref());
        let keys: Vec<u64> = names.iter().map(|name| name_key(name)).collect();
        self.0
            .iter()
            .flat_map(|(dir, listed)| {
                let listed =
                    move |key| listed.as_ref().is_none_or(|l| l.binary_search(key).is_ok());
                let present = names.iter().zip(&keys).filter(move |(_, key)| listed(key));
                present.map(move |(name, _)| dir.join(name))
            })
            .find(|candidate| is_executable(candidate))
    }
}

/// 返回目录 `dir` 中各文件名的散列（升序）：目录不存在时为空，无法列出时返回 `None`。
fn names_in(dir: &Path) -> Option<Vec<u64>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            let absent = matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory);
            return absent.then(Vec::new);
        }
    };
    let mut keys = Vec::new();
    for entry in entries {
        keys.push(name_key(&entry.ok()?.file_name().to_string_lossy()));
    }
    keys.sort_unstable();
    keys.dedup();
    Some(keys)
}

/// 返回文件名转为小写后的散列。
fn name_key(name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    name.to_lowercase().hash(&mut hasher);
    hasher.finish()
}

/// 返回程序在磁盘上可能使用的文件名。
///
/// Windows 上命令里写 `prettier`，磁盘上的文件可能是 `prettier.cmd`、`rustfmt.exe`：按
/// `PATHEXT` 依次追加扩展名（没有 `PATHEXT` 时使用 `.COM;.EXE;.BAT;.CMD`）；已带扩展名的原样返回。
/// 其他系统上就是程序名本身。
pub(crate) fn file_names(name: &str, pathext: Option<&OsStr>) -> Vec<String> {
    if cfg!(not(windows)) || Path::new(name).extension().is_some() {
        return vec![name.to_owned()];
    }
    let pathext = pathext.map_or_else(|| ".COM;.EXE;.BAT;.CMD".into(), OsStr::to_string_lossy);
    windows_names(name, &pathext)
}

/// 按以 `;` 分隔的 `pathext` 生成文件名：忽略不以 `.` 开头的项，扩展名转为小写。
fn windows_names(name: &str, pathext: &str) -> Vec<String> {
    pathext
        .split(';')
        .filter(|ext| ext.starts_with('.'))
        .map(|ext| format!("{name}{}", ext.to_ascii_lowercase()))
        .collect()
}

/// 是否为可执行文件：Unix 上要求是普通文件且设置了执行权限位，其他平台只要求是普通文件。
pub(crate) fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

#[cfg(test)]
mod tests;
