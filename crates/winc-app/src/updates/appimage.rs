//! Linux 等 Unix 平台：以 AppImage 运行时（环境变量 `APPIMAGE` 指向 AppImage 文件）由应用自己
//! 更新。更新包是新版的 AppImage，下载到原文件旁边，核对后改名替换原文件；重新启动时等本进程
//! 结束后运行新文件。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：更新包和程序本身在本机"
)]

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ed25519_dalek::VerifyingKey;

use super::{Item, download, verify};

/// appcast 中本平台更新包的 `sparkle:os`。
pub const OS: &str = "linux";
/// 各通道地址下本平台的 appcast 文件名；macOS 的 `appcast.xml` 由 Sparkle 的工具整个重新
/// 生成，各平台分开放。
pub const FEED_FILE: &str = "appcast-linux.xml";

/// 正在运行的 AppImage 文件；不是以 AppImage 运行时返回 `None`。
pub fn locate() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
}

/// 下载 `item` 到 `appimage` 旁边，核对长度和签名后替换 `appimage`，返回重新启动时运行的
/// 程序（即 `appimage`）；失败时不留下文件。下载期间把已下载的百分比交给 `progress`。
///
/// # Errors
///
/// `appimage` 没有所在目录或文件名、写不了下载的文件、`url` 取不到、长度或签名不对、改名失败时
/// 返回错误说明。
pub fn prepare(
    item: &Item,
    appimage: &Path,
    key: &VerifyingKey,
    progress: &dyn Fn(u8),
) -> Result<PathBuf, String> {
    let dir = appimage.parent().ok_or("AppImage 没有所在目录")?;
    let name = appimage.file_name().ok_or("AppImage 没有文件名")?;
    let part = dir.join(format!(".{}.update", name.to_string_lossy()));
    // 先建好文件：所在目录不能写时尽早说明，不必等下载完。
    std::fs::File::create(&part).map_err(|e| format!("无法写入「{}」：{e}", dir.display()))?;
    let result = download(&item.url, &part, item.length, progress).and_then(|()| {
        let bytes = std::fs::read(&part).map_err(|e| e.to_string())?;
        verify(item, &bytes, key)?;
        let mode = std::fs::metadata(appimage).map_or(0o755, |m| m.permissions().mode()) | 0o755;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
        std::fs::rename(&part, appimage).map_err(|e| e.to_string())
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| appimage.to_owned())
}

/// 进程退出前：要求过重新启动（`relaunch`）时，交给一个 shell 等本进程结束后运行新的
/// AppImage；否则不做什么，下次启动时即是新版本。
///
/// 等到本进程结束再启动，新版本才不会把自己交给仍在运行的旧进程（单实例，见 `instance`）。
pub fn after_exit(appimage: &Path, relaunch: bool) {
    if !relaunch {
        return;
    }
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$2""#;
    let spawned = Command::new("sh")
        .args(["-c", script, "sh"])
        .arg(std::process::id().to_string())
        .arg(appimage)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    if let Err(e) = spawned {
        eprintln!("winc: 无法重新启动：{e}");
    }
}
