//! 版本信息：Winc 的版本和架构、操作系统的名称和版本，Linux 上还有桌面环境和显示协议。
//!
//! 点击「关于」页的版本号时复制，供用户粘贴到问题反馈中。内容固定用英文，不随界面语言变化：
//! 维护者按同一种格式阅读。各平台的系统版本都取自系统接口或固定的小文件，不启动外部进程。

/// 返回复制到剪贴板的版本信息，每项一行，例如：
///
/// ```text
/// Winc 0.1.970 (arm64)
/// macOS 15.1
/// ```
///
/// `display` 是窗口所用的显示协议（`Wayland`、`X11`），只在 Linux 上写出；系统版本读取失败时写
/// `unknown`。读取一个系统接口或系统自带的小文件，耗时不随外部规模增长，可以在界面线程上调用。
pub fn report(display: Option<&str>) -> String {
    let system = os_version().unwrap_or_else(|| format!("{} unknown", std::env::consts::OS));
    let desktop = desktop(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        display,
    );
    let mut report = format!("Winc {}\n{system}", crate::layout::version_label());
    if let Some(desktop) = desktop.filter(|_| cfg!(target_os = "linux")) {
        report.push_str(&format!(" ({desktop})"));
    }
    report
}

/// 返回桌面环境与显示协议的说明，例如「GNOME, Wayland」；两者都不知道时返回 `None`。
///
/// `current` 是环境变量 `XDG_CURRENT_DESKTOP` 的值，可以列出几项（如 `ubuntu:GNOME`），按
/// freedesktop.org 的约定由具体到一般排列，取最后一项。
fn desktop(current: Option<&str>, display: Option<&str>) -> Option<String> {
    let name = current
        .and_then(|value| value.rsplit(':').find(|part| !part.is_empty()))
        .map(str::trim);
    match (name, display) {
        (Some(name), Some(display)) => Some(format!("{name}, {display}")),
        (Some(name), None) => Some(name.to_owned()),
        (None, Some(display)) => Some(display.to_owned()),
        (None, None) => None,
    }
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    crate::macos::os_version().map(|version| format!("macOS {version}"))
}

/// 读取 `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` 中的版本，命名规则见
/// [`windows_name`]。注册表中的值不受程序兼容模式的影响。
#[cfg(windows)]
fn os_version() -> Option<String> {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build: u32 = registry::string(key, "CurrentBuildNumber")?.parse().ok()?;
    let release = registry::string(key, "DisplayVersion");
    let update = registry::dword(key, "UBR");
    Some(windows_name(build, release.as_deref(), update))
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok();
    text.as_deref().and_then(os_release_name).or_else(kernel)
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn os_version() -> Option<String> {
    kernel()
}

#[cfg(not(any(unix, windows)))]
fn os_version() -> Option<String> {
    None
}

/// 返回 Windows 的名称：版本号 `build` 不小于 22000 时为 Windows 11，否则为 Windows 10，其后是
/// 发布版本 `release`（如 `23H2`）和 `build.update`。
///
/// 注册表中的 `ProductName` 在 Windows 11 上仍写 Windows 10，因此按版本号区分：Windows 11 的第一个
/// 正式版本号是 22000。
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn windows_name(build: u32, release: Option<&str>, update: Option<u32>) -> String {
    let name = if build >= 22000 {
        "Windows 11"
    } else {
        "Windows 10"
    };
    let release = release.map(|r| format!(" {r}")).unwrap_or_default();
    let update = update.map(|u| format!(".{u}")).unwrap_or_default();
    format!("{name}{release} (build {build}{update})")
}

/// 返回 os-release 文件内容 `text` 中的系统名称：`PRETTY_NAME`，没有时为 `NAME` 和 `VERSION_ID`。
///
/// 值的写法依据 os-release(5)：可以用单引号或双引号括起，双引号内的 `\` 转义 `$`、`` ` ``、`"`、
/// `\`；以 `#` 开头的行是注释。
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn os_release_name(text: &str) -> Option<String> {
    let value = |key: &str| {
        let line = text.lines().map(str::trim).find_map(|line| {
            let rest = line.strip_prefix(key)?.strip_prefix('=')?;
            (!line.starts_with('#')).then_some(rest)
        })?;
        unquote(line).filter(|value| !value.is_empty())
    };
    value("PRETTY_NAME").or_else(|| match (value("NAME")?, value("VERSION_ID")) {
        (name, Some(version)) => Some(format!("{name} {version}")),
        (name, None) => Some(name),
    })
}

/// 去掉 os-release 中一个值的引号并处理双引号内的转义；引号不成对时返回 `None`。
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn unquote(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('"') {
        let inner = inner.strip_suffix('"')?;
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match (c, chars.clone().next()) {
                ('\\', Some(next @ ('$' | '`' | '"' | '\\'))) => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push(c),
            }
        }
        return Some(out);
    }
    if let Some(inner) = value.strip_prefix('\'') {
        return inner.strip_suffix('\'').map(str::to_owned);
    }
    Some(value.to_owned())
}

/// 返回内核的名称和版本，例如「Linux 6.8.0」；读取失败时返回 `None`。
#[cfg(all(unix, not(target_os = "macos")))]
#[allow(unsafe_code)] // `uname` 只能通过系统接口调用
fn kernel() -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::utsname>::zeroed();
    // SAFETY: `info` 是足够大的可写缓冲区，`uname` 成功时写满整个结构体。
    if unsafe { libc::uname(info.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `uname` 已成功，结构体已初始化。
    let info = unsafe { info.assume_init() };
    let text = |field: &[libc::c_char]| {
        // SAFETY: `uname` 写出的各字段都在定长数组内以 NUL 结尾。
        let text = unsafe { std::ffi::CStr::from_ptr(field.as_ptr()) };
        text.to_str().ok().map(str::to_owned)
    };
    Some(format!("{} {}", text(&info.sysname)?, text(&info.release)?))
}

#[cfg(windows)]
mod registry;

#[cfg(test)]
mod tests;
