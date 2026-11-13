//! 从磁盘映像或下载位置直接运行时，将应用安装到「应用程序」文件夹。
//!
//! macOS 应用通常在「应用程序」文件夹中只安装一个副本：更新时就地替换，「打开方式」等系统列表
//! 中也只列出该副本。直接运行磁盘映像中的应用时，系统同样将其登记为应用，列表中会同时出现多个版本；
//! 磁盘映像又是只读的，无法就地更新。带隔离属性（从网络下载）的应用在下载位置直接打开时，系统将
//! 其复制到随机的只读位置运行（App Translocation），同样无法更新。
//!
//! 仅在这两种情况下询问是否移到「应用程序」文件夹：复制到该文件夹（已有的旧版本先移到废纸篓），
//! 退出后从新位置重新启动，并推出磁盘映像或将下载位置的原件移到废纸篓。应用位于其他可写位置（例如
//! 自行放置的文件夹、开发时的构建目录）时可以正常运行和更新，不询问。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：应用程序在本机的安装位置"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use objc2::runtime::Bool;
use winc_i18n::t;

use super::files::{ns_string, rust_string};
use super::*;

/// 需要安装到「应用程序」文件夹的应用本次运行的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 应用包（`winc.app`）的路径；被系统复制到随机位置运行时是原来的路径。
    pub bundle: PathBuf,
    /// 应用包所在的磁盘映像的挂载点；不在磁盘映像中时为 `None`。
    pub image: Option<PathBuf>,
}

/// 应用从磁盘映像运行，或被系统复制到随机位置运行时，返回原来的位置；其他情况（位于「应用
/// 程序」文件夹或其他位置，或不是以 `.app` 包运行）返回 `None`。
pub fn needs_install() -> Option<Location> {
    let (bundle, translocated) = original_path(&bundle_path()?);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if inside_applications(&bundle, home.as_deref()) {
        return None;
    }
    let image = volume_of(&bundle).filter(|volume| is_disk_image(volume));
    (image.is_some() || translocated).then_some(Location { bundle, image })
}

/// `bundle` 是否位于「应用程序」文件夹：`/Applications` 或用户的 `~/Applications`（`home` 是用户
/// 目录），包括其中的子文件夹（例如 `/Applications/Utilities`）。
pub(super) fn inside_applications(bundle: &Path, home: Option<&Path>) -> bool {
    let user = home.map(|home| home.join("Applications"));
    [Some(Path::new("/Applications")), user.as_deref()]
        .into_iter()
        .flatten()
        .any(|folder| bundle.starts_with(folder))
}

/// `bundle` 所在卷的挂载点：位于 `/Volumes` 下时为 `/Volumes/<卷名>`，否则为 `None`（启动磁盘）。
pub(super) fn volume_of(bundle: &Path) -> Option<PathBuf> {
    let mut parts = bundle.strip_prefix("/Volumes").ok()?.components();
    let name = parts.next()?;
    Some(Path::new("/Volumes").join(name))
}

/// 将应用复制到「应用程序」文件夹，返回复制后的路径。
///
/// 目标为 `/Applications`，没有写入权限时为 `~/Applications`。目标位置已有同名应用时先移到
/// 废纸篓；该应用正在运行时不替换，返回原因。复制使用 `ditto`，签名和扩展属性原样保留；复制后
/// 去除隔离属性，使新位置的应用不被系统复制到随机位置运行。
///
/// # Errors
///
/// 目标正在运行、旧版本无法移到废纸篓、复制失败时返回原因。
pub fn move_to_applications(location: &Location) -> Result<PathBuf, String> {
    let unknown = || t!("app-move-unknown-location").to_owned();
    let name = location.bundle.file_name().ok_or_else(unknown)?;
    let target = applications_folder().ok_or_else(unknown)?.join(name);
    if target.exists() {
        if is_running(&target) {
            return Err(t!("app-move-target-running").to_owned());
        }
        trash(&target)?;
    }
    let copied = Command::new("/usr/bin/ditto")
        .arg(&location.bundle)
        .arg(&target)
        .status()
        .map_err(|e| t!("app-move-copy-failed", status = e.to_string()))?;
    if !copied.success() {
        return Err(t!("app-move-copy-failed", status = copied.to_string()));
    }
    // 去除隔离属性失败不影响使用：应用仍可运行，仅在首次打开时由系统额外检查一次。
    let _ = Command::new("/usr/bin/xattr")
        .args(["-d", "-r", "com.apple.quarantine"])
        .arg(&target)
        .status();
    Ok(target)
}

/// 安排本进程退出后从 `target` 重新启动，并清理原来的位置 `location`：在磁盘映像中时推出磁盘
/// 映像；在下载位置时立即将原件移到废纸篓（运行中移动应用自身的包不影响运行）。
///
/// 重新启动由一个等待本进程结束的 `sh` 子进程完成：本进程仍在运行时打开新位置的应用，系统将其
/// 视为同一个应用并激活本进程，而不会启动新的实例。
///
/// # Errors
///
/// 启动子进程失败时返回原因；原件无法移到废纸篓不视为失败，原件保留在原处。
pub fn relaunch_from(target: &Path, location: &Location) -> Result<(), String> {
    if location.image.is_none() {
        let _ = trash(&location.bundle);
    }
    // `$1` 为本进程号，`$2` 为新位置，`$3` 为要推出的磁盘映像（可以为空）。
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
[ -n "$3" ] && /usr/bin/hdiutil detach "$3" -quiet
/usr/bin/open "$2""#;
    let image = location.image.as_deref().unwrap_or(Path::new(""));
    Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .arg(std::process::id().to_string())
        .arg(target)
        .arg(image)
        .spawn()
        .map(drop)
        .map_err(|e| t!("app-move-relaunch-failed", error = e.to_string()))
}

/// 应用包的路径；不是以 `.app` 包运行时返回 `None`。
fn bundle_path() -> Option<PathBuf> {
    // SAFETY: `NSBundle` 的公开 API，在主线程调用；返回的字符串由主程序包持有，由 `rust_string`
    // 立即拷贝。
    let path = unsafe {
        let main: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let path: *mut AnyObject = msg_send![main.as_ref()?, bundlePath];
        rust_string(path.as_ref()?)?
    };
    let path = PathBuf::from(path);
    (path.extension()? == "app").then_some(path)
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    /// `SecTranslocateIsTranslocatedURL`（`<Security/SecTranslocate.h>`，macOS 10.12 起）。
    fn SecTranslocateIsTranslocatedURL(
        path: *const std::ffi::c_void,
        translocated: *mut bool,
        error: *mut *const std::ffi::c_void,
    ) -> bool;
    /// `SecTranslocateCreateOriginalPathForURL`：返回 +1 的 `CFURLRef`，失败时为空。
    fn SecTranslocateCreateOriginalPathForURL(
        path: *const std::ffi::c_void,
        error: *mut *const std::ffi::c_void,
    ) -> *mut AnyObject;
}

/// 返回应用包 `path` 原来的路径，以及它是否被系统复制到随机位置运行（App Translocation）；没有
/// 被复制或查询失败时原样返回 `path`。
fn original_path(path: &Path) -> (PathBuf, bool) {
    let unchanged = || (path.to_path_buf(), false);
    let Some(text) = path.to_str().and_then(ns_string) else {
        return unchanged();
    };
    // SAFETY: `fileURLWithPath:` 返回自动释放的 NSURL，与 `CFURLRef` 免费桥接，在本函数内有效。
    // 两个 Security 函数按头文件的声明调用，错误参数可以为空；返回的 +1 对象交给 `Retained`。
    unsafe {
        let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: &*text];
        if url.is_null() {
            return unchanged();
        }
        let mut translocated = false;
        let checked = SecTranslocateIsTranslocatedURL(
            url.cast(),
            &raw mut translocated,
            std::ptr::null_mut(),
        );
        if !checked || !translocated {
            return unchanged();
        }
        let original = SecTranslocateCreateOriginalPathForURL(url.cast(), std::ptr::null_mut());
        let Some(original) = Retained::from_raw(original) else {
            return unchanged();
        };
        let text: *mut AnyObject = msg_send![&*original, path];
        match text.as_ref().and_then(|text| rust_string(text)) {
            Some(text) => (PathBuf::from(text), true),
            None => unchanged(),
        }
    }
}

/// 挂载点 `volume` 是否属于磁盘映像：`hdiutil info` 列出的已挂载映像中包含它。
fn is_disk_image(volume: &Path) -> bool {
    let Ok(output) = Command::new("/usr/bin/hdiutil").arg("info").output() else {
        return false;
    };
    let volume = volume.to_string_lossy();
    // 每个分区一行，以制表符分隔，最后一列是挂载点（卷名中可以有空格）。
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.rsplit('\t').next() == Some(&*volume))
}

/// 复制的目标文件夹：`/Applications` 可写时为它，否则为 `~/Applications`（不存在时创建）；都无法
/// 使用时返回 `None`。
fn applications_folder() -> Option<PathBuf> {
    let system = Path::new("/Applications");
    if writable(system) {
        return Some(system.to_path_buf());
    }
    let user = PathBuf::from(std::env::var_os("HOME")?).join("Applications");
    std::fs::create_dir_all(&user).ok()?;
    Some(user)
}

/// 当前用户能否在文件夹 `folder` 中新建和删除条目。
fn writable(folder: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` 是以 NUL 结尾的有效 C 字符串，在调用期间有效。
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// 应用包 `bundle` 是否正在运行（不计本进程）。
fn is_running(bundle: &Path) -> bool {
    let Some(target) = bundle.to_str() else {
        return false;
    };
    // SAFETY: `NSWorkspace` 的公开 API，在主线程调用；数组和其中的对象先判空再使用，字符串由
    // `rust_string` 立即拷贝。
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let Some(workspace) = workspace.as_ref() else {
            return false;
        };
        let apps: *mut AnyObject = msg_send![workspace, runningApplications];
        let Some(apps) = apps.as_ref() else {
            return false;
        };
        let count: usize = msg_send![apps, count];
        let me = std::process::id() as i32;
        (0..count).any(|i| {
            let app: *mut AnyObject = msg_send![apps, objectAtIndex: i];
            let Some(app) = app.as_ref() else {
                return false;
            };
            let pid: i32 = msg_send![app, processIdentifier];
            let url: *mut AnyObject = msg_send![app, bundleURL];
            let path: *mut AnyObject = match url.as_ref() {
                Some(url) => msg_send![url, path],
                None => std::ptr::null_mut(),
            };
            pid != me && path.as_ref().and_then(|p| rust_string(p)).as_deref() == Some(target)
        })
    }
}

/// 将 `path` 移到废纸篓。
fn trash(path: &Path) -> Result<(), String> {
    let failed = || t!("app-move-trash-failed", path = path.display().to_string());
    let text = path.to_str().and_then(ns_string).ok_or_else(failed)?;
    // SAFETY: `NSFileManager` 和 `NSURL` 的公开 API，在主线程调用；结果参数和错误参数可以为空。
    let moved: Bool = unsafe {
        let manager: *mut AnyObject = msg_send![class!(NSFileManager), defaultManager];
        let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: &*text];
        let null: *mut AnyObject = std::ptr::null_mut();
        msg_send![manager, trashItemAtURL: url, resultingItemURL: null, error: null]
    };
    if moved.as_bool() {
        Ok(())
    } else {
        Err(failed())
    }
}

#[cfg(test)]
mod tests;
