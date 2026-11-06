//! 关于页显示 `上游_自己的数字`。更新比较用 `上游.数字`：已安装的旧版本也按四段数字
//! 认出它更新了。上游三段或自己的数字变大，都比当前版本新。

/// 关于页和启动页上的版本，例如 `0.1.502_115`。
pub(crate) fn display_version() -> String {
    format!(
        "{}_{}",
        env!("CARGO_PKG_VERSION"),
        env!("WINC_FORK_VERSION")
    )
}

/// 写进 appcast、并用来和当前安装比较的版本，例如 `0.1.502.115`。
///
/// macOS 由 Sparkle 比较版本，这个四段数字只给其他平台的应用内更新。
#[cfg(not(target_os = "macos"))]
pub(crate) fn update_version() -> &'static str {
    concat!(env!("CARGO_PKG_VERSION"), ".", env!("WINC_FORK_VERSION"))
}

/// 把四段数字版本显示成 `上游_自己的数字`。其他写法原样保留。
pub(crate) fn present_version(version: &str) -> String {
    let mut parts = version.split('.');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(major), Some(minor), Some(patch), Some(fork), None)
            if [major, minor, patch, fork]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit())) =>
        {
            format!("{major}.{minor}.{patch}_{fork}")
        }
        _ => version.to_owned(),
    }
}
