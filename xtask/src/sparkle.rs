//! 应用内更新用的 Sparkle：下载固定版本、校验、放进应用包，以及 `Info.plist` 里的更新配置。
//!
//! 发布包按 [`VERSION`] 和 `SHA256` 固定，下载到 `target/sparkle/<版本>/`，校验不通过时不解压。
//! 更新通道和公钥取自环境变量 `WINC_UPDATE_CHANNELS`、`WINC_UPDATE_PUBLIC_KEY`；两者缺一时
//! 应用不检查更新，应用包中仍然包含框架。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{root, run};

/// 固定使用的 Sparkle 版本（发布包文件名与 [`SHA256`] 都按它确定）。
pub const VERSION: &str = "2.10.1";

/// `Sparkle-<版本>.tar.xz` 的 SHA-256。
const SHA256: &str = "1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27";

/// 返回解压好的发布包目录；目录不存在时先下载、校验并解压。
///
/// 目录中有 `Sparkle.framework` 和 `bin/` 下的 `generate_keys`、`sign_update`、
/// `generate_appcast`。
///
/// # Errors
///
/// 下载失败、SHA-256 不匹配或解压失败时返回原因。
pub fn unpacked() -> Result<PathBuf, String> {
    let dir = root().join("target/sparkle").join(VERSION);
    let unpacked = dir.join("x");
    // 发布缓存还原 `target/` 时不保留框架里的符号链接。这样的目录仍然存在，
    // 但 `codesign` 会报 bundle format unrecognized。符号链接还在才复用。
    if framework_intact(&unpacked) {
        return Ok(unpacked);
    }
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(&dir).map_err(io)?;
    let archive = dir.join(format!("Sparkle-{VERSION}.tar.xz"));
    if !archive.is_file() {
        let url = format!(
            "https://github.com/sparkle-project/Sparkle/releases/download/{VERSION}/Sparkle-{VERSION}.tar.xz"
        );
        println!("下载 {url}");
        let partial = dir.join("download.part");
        run(Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error", "-o"])
            .arg(&partial)
            .arg(&url))?;
        std::fs::rename(&partial, &archive).map_err(io)?;
    }
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&archive)
        .output()
        .map_err(|e| format!("无法运行 shasum：{e}"))?;
    let digest = String::from_utf8_lossy(&output.stdout);
    if digest.split_whitespace().next() != Some(SHA256) {
        // 校验不通过的文件已无法使用，移除后下次运行时重新下载。
        let _ = std::fs::remove_file(&archive);
        return Err(format!("{} 的 SHA-256 不对，已删除", archive.display()));
    }
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(io)?;
    run(Command::new("tar")
        .arg("-xJf")
        .arg(&archive)
        .arg("-C")
        .arg(&unpacked))?;
    Ok(unpacked)
}

/// `Sparkle.framework` 的标准布局是否还在：`Versions/Current` 和顶层 `Sparkle` 都是符号链接。
fn framework_intact(unpacked: &Path) -> bool {
    let framework = unpacked.join("Sparkle.framework");
    framework.join("Versions/Current").is_symlink() && framework.join("Sparkle").is_symlink()
}

/// 把 `Sparkle.framework` 原样复制进应用包的 `Contents/Frameworks/`。
///
/// 通过 `ditto` 复制，保留框架中的符号链接和原有签名；分发前由发布者用 Developer ID 重新签名。
///
/// # Errors
///
/// 取不到框架或复制失败时返回原因。
pub fn embed(contents: &Path) -> Result<(), String> {
    let framework = unpacked()?.join("Sparkle.framework");
    let frameworks = contents.join("Frameworks");
    std::fs::create_dir_all(&frameworks).map_err(|e| e.to_string())?;
    run(Command::new("ditto")
        .arg(&framework)
        .arg(frameworks.join("Sparkle.framework")))
}

/// 应用内更新的配置：各通道的地址前缀和签名公钥。
#[derive(Debug, Default)]
pub struct UpdateConfig {
    /// 各更新通道的名字与地址前缀，地址前缀不以 `/` 结尾。
    pub channels: Vec<(String, String)>,
    /// Sparkle 的签名公钥（Ed25519，`generate_keys` 输出的 Base64）。
    pub public_key: String,
}

impl UpdateConfig {
    /// 创建从环境变量 `WINC_UPDATE_CHANNELS`、`WINC_UPDATE_PUBLIC_KEY` 读到的更新配置。
    ///
    /// 变量未设置时对应项为空。
    pub fn load() -> UpdateConfig {
        UpdateConfig::parse("", |key| std::env::var(key).ok())
    }

    /// 创建按 `键=值` 格式的文本 `text` 解析出的更新配置。
    ///
    /// `env` 按名字返回环境变量的值，非空时优先于 `text` 中的同名项。
    fn parse(text: &str, env: impl Fn(&str) -> Option<String>) -> UpdateConfig {
        let value = |key: &str| {
            env(key).unwrap_or_else(|| {
                text.lines()
                    .filter_map(|line| line.trim().split_once('='))
                    .find(|(k, _)| k.trim() == key)
                    .map(|(_, v)| v.trim().trim_matches('"').to_owned())
                    .unwrap_or_default()
            })
        };
        let channels = value("WINC_UPDATE_CHANNELS")
            .split_whitespace()
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, base)| (name.to_owned(), base.trim_end_matches('/').to_owned()))
            .collect();
        UpdateConfig {
            channels,
            public_key: value("WINC_UPDATE_PUBLIC_KEY"),
        }
    }

    /// 是否同时配好了通道和公钥（应用据此检查更新）。
    pub fn enabled(&self) -> bool {
        !self.channels.is_empty() && !self.public_key.is_empty()
    }

    /// 返回写进 `Info.plist` 的更新配置项；更新未启用时返回空字符串。
    ///
    /// 应用按时自行检查（`SUEnableAutomaticChecks` 为 false），检查时同时请求各通道的 appcast。
    /// 只安装与公钥签名匹配的更新，appcast 本身也必须签名（`SURequireSignedFeed`），解压前
    /// 校验（`SUVerifyUpdateBeforeExtraction`），不提供自动下载并安装的选项
    /// （`SUAllowsAutomaticUpdates`）。
    ///
    /// `arch` 是应用的架构，取 `arm64` 或 `x86_64`；两种架构各有一份 appcast，见 [`feed_url`]。
    pub fn plist_entries(&self, arch: &str) -> String {
        if !self.enabled() {
            return String::new();
        }
        let feeds: String = self
            .channels
            .iter()
            .map(|(_, base)| {
                format!(
                    "\t\t<string>{}</string>\n",
                    xml_escape(&feed_url(base, arch))
                )
            })
            .collect();
        let first = feed_url(&self.channels[0].1, arch);
        format!(
            "\t<key>SUAllowsAutomaticUpdates</key>\n\t<false/>\n\
             \t<key>SUEnableAutomaticChecks</key>\n\t<false/>\n\
             \t<key>SUFeedURL</key>\n\t<string>{}</string>\n\
             \t<key>SUPublicEDKey</key>\n\t<string>{}</string>\n\
             \t<key>SURequireSignedFeed</key>\n\t<true/>\n\
             \t<key>SUVerifyUpdateBeforeExtraction</key>\n\t<true/>\n\
             \t<key>WincUpdateFeeds</key>\n\t<array>\n{feeds}\t</array>\n",
            xml_escape(&first),
            xml_escape(&self.public_key),
        )
    }
}

/// 返回通道 `base` 中 macOS 版架构 `arch` 的 appcast 地址。
///
/// `arch` 取 `arm64` 或 `x86_64`。两种架构的安装包和 appcast 都分开：同一版本在一个 appcast
/// 中有两项时，Sparkle 无法可靠地挑出本机架构的那一项，Apple 芯片的 Mac 可能装上 Intel 版。
pub fn feed_url(base: &str, arch: &str) -> String {
    format!("{base}/appcast-macos-{arch}.xml")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests;
