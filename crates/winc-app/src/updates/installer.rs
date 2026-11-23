//! Windows：按安装包留下的标记区分 NSIS、MSI，便携版不检查更新。
//!
//! 更新包通过签名核对后交给独立的 PowerShell 进程；它等应用退出后运行相应的静默安装命令，
//! 要求过重新启动时再启动新版本。安装目录当前用户写不了时（例如 `C:\Program Files\winc`）先请求提升权限。
//!
//! 核对和安装之间更新包一直放在临时目录里：核对时记下内容的摘要，更新进程安装前以不允许
//! 写入和删除的方式打开它、再比对一次，安装结束才放开。安装后主程序没有变化（静默安装会
//! 跳过被占用的文件）算作失败。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：更新包和安装标记在本机"
)]

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};

use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};
use winc_i18n::t;

use super::{Item, cannot_write, download, verify};

/// appcast 中本平台更新包的系统名称。
pub const OS: &str = "windows";
/// Windows 的更新清单；每种安装包各有一项。
pub const FEED_FILE: &str = "appcast-windows.xml";

/// 通过了签名核对的更新包和它内容的 SHA-256（小写十六进制）；只记在内存里。
static VERIFIED: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

/// 返回 `bytes` 的 SHA-256，小写十六进制。
fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 安装包在主程序旁写入的安装类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// 当前用户的 NSIS 安装。
    Nsis,
    /// 计算机级 MSI 安装。
    Msi,
}

impl Kind {
    /// appcast 中的安装类型名称。
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Nsis => "nsis",
            Self::Msi => "msi",
        }
    }

    /// 下载到本地的安装包扩展名。
    fn extension(self) -> &'static str {
        match self {
            Self::Nsis => "exe",
            Self::Msi => "msi",
        }
    }
}

/// 读取主程序旁的标记；没有标记或类型不认识时视为便携版。
fn kind_at(exe: &Path) -> Option<Kind> {
    let marker = std::fs::read_to_string(exe.parent()?.join("winc-installation")).ok()?;
    match marker.trim() {
        "nsis" => Some(Kind::Nsis),
        "msi" => Some(Kind::Msi),
        _ => None,
    }
}

/// 当前运行程序的安装类型。
pub(super) fn current_kind() -> Option<Kind> {
    kind_at(&std::env::current_exe().ok()?)
}

/// 正在运行的已安装程序；便携版返回 `None`。
pub fn locate() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .filter(|exe| kind_at(exe).is_some())
}

/// 下载并核对安装包，下载期间把已下载的百分比交给 `progress`；失败时删除整个临时目录。
///
/// # Errors
/// 安装类型不明、下载或签名核对失败时返回原因。
pub fn prepare(
    item: &Item,
    exe: &Path,
    key: &VerifyingKey,
    progress: &dyn Fn(u8),
) -> Result<PathBuf, String> {
    let kind = kind_at(exe).ok_or_else(|| t!("settings-updates-error-location").to_owned())?;
    let dir = std::env::temp_dir().join("winc-update");
    prepare_in(&dir, item, key, kind, progress)
}

/// [`prepare`]，显式指定输出目录和安装类型，供隔离测试使用。
fn prepare_in(
    dir: &Path,
    item: &Item,
    key: &VerifyingKey,
    kind: Kind,
    progress: &dyn Fn(u8),
) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| cannot_write(dir, &e))?;
    let setup = dir.join(format!("winc-setup.{}", kind.extension()));
    let result = download(&item.url, &setup, item.length, progress).and_then(|()| {
        let bytes = std::fs::read(&setup).map_err(|e| e.to_string())?;
        verify(item, &bytes, key)?;
        Ok(digest(&bytes))
    });
    match result {
        Ok(digest) => {
            let mut verified = VERIFIED.lock().unwrap_or_else(PoisonError::into_inner);
            *verified = Some((setup.clone(), digest));
            Ok(setup)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(dir);
            Err(e)
        }
    }
}

/// 启动独立的更新进程，由它等待本进程退出后安装，并按 `relaunch` 决定是否重启。
///
/// 脚本以 `-Command` 传入，参数经环境变量传递：执行策略只限制脚本文件，不限制命令行输入，
/// 因此不需要 `-ExecutionPolicy Bypass`。安全软件的行为规则会拦截由图形界面程序发起、带
/// `-ExecutionPolicy Bypass` 的 PowerShell 进程。
pub fn after_exit(setup: &Path, relaunch: bool) {
    let result = (|| -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let kind = kind_at(&exe).ok_or_else(|| t!("settings-updates-error-location").to_owned())?;
        // 只安装这次运行中核对过的那一个：摘要交给更新进程，它在安装前再比对一次。
        let verified = VERIFIED.lock().unwrap_or_else(PoisonError::into_inner);
        let digest = (verified.as_ref())
            .filter(|(path, _)| path == setup)
            .map(|(_, digest)| digest.clone())
            .ok_or_else(|| t!("settings-updates-error-signature").to_owned())?;
        drop(verified);
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(include_str!("installer/update.ps1"))
            .env("WINC_UPDATE_PARENT", std::process::id().to_string())
            .env("WINC_UPDATE_SETUP", setup)
            .env("WINC_UPDATE_SHA256", digest)
            .env("WINC_UPDATE_EXE", exe)
            .env("WINC_UPDATE_KIND", kind.name())
            .env("WINC_UPDATE_RELAUNCH", if relaunch { "1" } else { "0" })
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("winc: 无法运行安装包：{e}");
    }
}

#[cfg(test)]
mod tests;
