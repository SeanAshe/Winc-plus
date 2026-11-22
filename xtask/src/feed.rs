//! Linux 版（AppImage）和 Windows 版（安装包）的 appcast：`appcast-linux.xml`、
//! `appcast-linux-arm64.xml`、`appcast-windows.xml`。应用内更新读取本平台的那一个，从其中取
//! 更新包与签名。
//!
//! 每个更新包一项，文件名须为 `winc-<版本>-<架构>.AppImage` 或 `winc-<版本>-<架构>-setup.exe`。
//! 更新包和 appcast 都用 `sign_update` 签名（EdDSA），`sign_update` 把 appcast 的签名写在文件末尾的
//! 注释中。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::sparkle;

/// 由应用完成更新的平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux 版，更新包是 AppImage。
    Linux,
    /// Windows 版，更新包是安装包。
    Windows,
}

impl Platform {
    /// 架构 `arch` 的应用内更新请求的 appcast 文件名；Linux 的两种架构各有一份。
    fn feed_file(self, arch: &str) -> &'static str {
        match (self, arch) {
            (Self::Linux, "aarch64") => "appcast-linux-arm64.xml",
            (Self::Linux, _) => "appcast-linux.xml",
            (Self::Windows, _) => "appcast-windows.xml",
        }
    }

    /// `enclosure` 元素的 `sparkle:os` 属性值。
    fn os(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    /// 更新包文件名中架构之后的部分。
    fn suffix(self) -> &'static str {
        match self {
            Self::Linux => ".AppImage",
            Self::Windows => "-setup.exe",
        }
    }
}

/// 一个更新包。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Package {
    /// 版本号。
    version: String,
    /// 架构，如 `x86_64`、`aarch64`（与 Rust 的 `std::env::consts::ARCH` 相同）。
    arch: String,
    /// 文件名。
    name: String,
    /// 文件大小（字节）。
    length: u64,
    /// `sign_update` 给出的签名（base64）。
    signature: String,
}

/// 为 `files` 中属于 `platform` 的更新包生成签名的 appcast，写到 `out` 下，下载地址是
/// `<base>/<文件名>`；返回写出的各个文件，见 [`Platform::feed_file`]。
///
/// # Errors
///
/// 文件名不合规定、取不到 Sparkle 的工具、签名或写入失败时返回原因。
pub fn write(
    platform: Platform,
    out: &Path,
    base: &str,
    files: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    if files.is_empty() {
        return Err("没有给出更新包".into());
    }
    let sign_update = sparkle::unpacked()?.join("bin/sign_update");
    let mut packages = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} 没有文件名", file.display()))?;
        let suffix = if platform == Platform::Windows && name.ends_with(".msi") {
            ".msi"
        } else {
            platform.suffix()
        };
        let (version, arch) = parse_name(&name, suffix)
            .ok_or_else(|| format!("{name} 不是 winc-<版本>-<架构>{suffix}"))?;
        let (signature, length) = sign(&sign_update, file)?;
        packages.push(Package {
            version,
            arch,
            name,
            length,
            signature,
        });
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut names: Vec<&str> = (packages.iter())
        .map(|package| platform.feed_file(&package.arch))
        .collect();
    names.sort_unstable();
    names.dedup();
    let mut written = Vec::new();
    for name in names {
        let listed: Vec<Package> = (packages.iter())
            .filter(|package| platform.feed_file(&package.arch) == name)
            .cloned()
            .collect();
        let path = out.join(name);
        let feed = feed(platform, &listed, base.trim_end_matches('/'));
        std::fs::write(&path, &feed).map_err(|e| e.to_string())?;
        sign_feed(&sign_update, &path)?;
        written.push(path);
    }
    Ok(written)
}

/// 由文件名得出（版本，架构）；格式不符时返回 `None`。
fn parse_name(name: &str, suffix: &str) -> Option<(String, String)> {
    let rest = name
        .strip_prefix("winc-")
        .or_else(|| name.strip_prefix("Winc-"))?
        .strip_suffix(suffix)?;
    let (version, arch) = if let Some(version) = rest
        .strip_suffix("-linux-x64")
        .or_else(|| rest.strip_suffix("-windows-x64"))
    {
        (version, "x86_64")
    } else if let Some(version) = rest.strip_suffix("-linux-arm64") {
        (version, "aarch64")
    } else {
        rest.rsplit_once('-')?
    };
    let valid = |s: &str, extra: char| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == extra)
    };
    let valid_version = !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    (valid_version && valid(arch, '_')).then(|| (version.to_owned(), arch.to_owned()))
}

/// 返回运行 Sparkle 的 `sign_update` 的命令。
///
/// 私钥取自环境变量 `WINC_SPARKLE_KEY_FILE` 指向的文件（GitHub Actions 中由 Secret 写成临时
/// 文件）；没有设置时 `sign_update` 从钥匙串中读取。
fn sign_command(sign_update: &Path) -> Command {
    let mut command = Command::new(sign_update);
    if let Some(key) = std::env::var_os("WINC_SPARKLE_KEY_FILE").filter(|k| !k.is_empty()) {
        command.arg("--ed-key-file").arg(key);
    }
    command
}

/// 用 `sign_update` 签名文件 `feed`。
///
/// `sign_update` 把签名注释（`<!-- sparkle-signatures: …`）写在 `feed` 末尾，不输出签名。
fn sign_feed(sign_update: &Path, feed: &Path) -> Result<(), String> {
    let output = sign_command(sign_update)
        .arg(feed)
        .output()
        .map_err(|e| format!("无法运行 {}：{e}", sign_update.display()))?;
    let text = std::fs::read_to_string(feed).map_err(|e| e.to_string())?;
    if !output.status.success() || !text.contains("<!-- sparkle-signatures:") {
        return Err(format!(
            "sign_update 没有签名 {}：{}",
            feed.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// 用 Sparkle 的 `sign_update` 签名更新包 `file`，返回（签名，大小）。
fn sign(sign_update: &Path, file: &Path) -> Result<(String, u64), String> {
    let output = sign_command(sign_update)
        .arg(file)
        .output()
        .map_err(|e| format!("无法运行 {}：{e}", sign_update.display()))?;
    if !output.status.success() {
        return Err(format!(
            "sign_update 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_signature(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "读不懂 sign_update 的输出".into())
}

/// 读 `sign_update` 的输出 `sparkle:edSignature="…" length="…"`；格式不符时返回 `None`。
fn parse_signature(output: &str) -> Option<(String, u64)> {
    let attribute = |name: &str| {
        let start = output.find(&format!("{name}=\""))? + name.len() + 2;
        let end = start + output[start..].find('"')?;
        Some(output[start..end].to_owned())
    };
    Some((
        attribute("sparkle:edSignature")?,
        attribute("length")?.parse().ok()?,
    ))
}

/// 返回 appcast 的内容（不含末尾的签名），每个更新包一项。
///
/// 各项按版本号从新到旧排列。
fn feed(platform: Platform, packages: &[Package], base: &str) -> String {
    let mut sorted = packages.to_vec();
    sorted.sort_by(|a, b| winc_config::compare_versions(&b.version, &a.version));
    let items: String = sorted
        .iter()
        .map(|p| {
            format!(
                "    <item>\n      <title>{v}</title>\n      <sparkle:version>{v}</sparkle:version>\n      \
                 <sparkle:shortVersionString>{v}</sparkle:shortVersionString>\n      \
                 <enclosure url=\"{url}\" length=\"{len}\" type=\"application/octet-stream\" \
                 sparkle:os=\"{os}\" arch=\"{arch}\"{installer} sparkle:edSignature=\"{sig}\"/>\n    </item>\n",
                v = escape(&p.version),
                os = platform.os(),
                url = escape(&format!("{base}/{}", p.name)),
                len = p.length,
                arch = escape(&p.arch),
                sig = escape(&p.signature),
                installer = if platform == Platform::Windows {
                    if p.name.ends_with(".msi") { " installer=\"msi\"" } else { " installer=\"nsis\"" }
                } else { "" },
            )
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <rss version=\"2.0\" xmlns:sparkle=\"http://www.andymatuschak.org/xml-namespaces/sparkle\">\n  \
         <channel>\n    <title>winc</title>\n{items}  </channel>\n</rss>\n"
    )
}

/// XML 属性和文字中需要转义的字符。
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
