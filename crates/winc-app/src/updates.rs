//! 应用内更新（macOS 以外的平台）：以 AppImage 运行时（`appimage`）、由安装包安装时
//! （`installer`）检查新版本，下载核对后安装；其他安装方式由包管理器更新。
//!
//! 更新通道和 EdDSA 公钥在构建时由 `WINC_UPDATE_CHANNELS`、`WINC_UPDATE_PUBLIC_KEY` 给出，两者
//! 都有才启用；应用在启动后 [`FIRST_CHECK`]、之后每隔 [`CHECK_INTERVAL`] 检查一次。用户选择
//! 状态栏的提示后才下载（[`install`]），核对长度和签名后提示重新启动（[`restart`]）。

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, mpsc};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};

use crate::web::{command, fetch};
use crate::winc::UpdateState;

/// 「关于」页更新按钮这次点击该做什么，与状态栏上的更新按钮一致。
///
/// 还没找到新版本时检查；找到之后下载安装；已经准备好时重新启动。正在检查或下载时不响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateClick {
    /// 不支持更新，或正在检查、下载。
    None,
    /// 检查有没有新版本。
    Check,
    /// 下载并安装上次找到的版本；安装失败时也是再试一次。
    Install,
    /// 重新启动以完成更新。
    Restart,
}

impl UpdateClick {
    /// 由当前的更新状态 `state` 得出按钮该做的事。
    pub(crate) fn from_state(state: &UpdateState) -> Self {
        match state {
            UpdateState::Off | UpdateState::Checking | UpdateState::Downloading(..) => Self::None,
            UpdateState::Ready(_) => Self::Restart,
            UpdateState::Found(_) | UpdateState::InstallFailed(_) => Self::Install,
            UpdateState::Idle | UpdateState::Latest | UpdateState::Failed => Self::Check,
        }
    }
}
#[cfg(unix)]
use appimage as platform;
#[cfg(windows)]
use installer as platform;
pub use platform::FEED_FILE;

/// 启动后第一次检查前的等待时间。
///
/// 该时长覆盖打开窗口与恢复标签的耗时，避免检查与启动阶段争用网络和线程。
const FIRST_CHECK: Duration = Duration::from_secs(5);
/// 两次检查之间的间隔：86 400 秒，即 24 小时。
///
/// 该值决定了更新提示的最长延迟；按天更新即可满足发布节奏。
const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);
/// 请求一个通道的 appcast 的等待上限（秒）。
///
/// 超过该时长仍未返回时，该通道按没有回应处理。
const FEED_TIMEOUT: u64 = 15;
/// 下载时读取已下载大小的间隔。
///
/// 取 200 毫秒：进度每秒更新五次，看起来是连续的，读取文件大小的开销可以忽略。
const PROGRESS_POLL: Duration = Duration::from_millis(200);
/// 下载更新包的等待上限（秒）。
///
/// 超过该时长仍未返回时按下载失败处理；该时长按慢速网络下传输完整个安装包留出余量。
const DOWNLOAD_TIMEOUT: u64 = 1800;
/// appcast 中 Sparkle 扩展元素的 XML 命名空间。
///
/// appcast 格式来自 Sparkle，各更新项用该命名空间下的元素和属性描述。
const SPARKLE: &str = "http://www.andymatuschak.org/xml-namespaces/sparkle";

/// appcast 中的一个更新包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// 版本号，如 `0.1.369`。
    pub version: String,
    /// 下载地址。
    pub url: String,
    /// 文件大小（字节）。
    pub length: u64,
    /// 整个文件的 Ed25519 签名。
    pub signature: [u8; 64],
}

/// 应用交给更新器的回调，可在任意线程调用。
struct Hooks {
    /// 通知应用找到的新版本号；没有新版本时为 `None`，应用据此撤下提示。
    found: Box<dyn Fn(Option<String>) + Send + Sync>,
    /// 通知应用更新状态的变化。
    state: Box<dyn Fn(UpdateState) + Send + Sync>,
}

/// 启用后的更新器。
struct Updater {
    /// 各通道 appcast 的完整下载地址。
    feeds: Vec<String>,
    /// 核对签名用的公钥。
    key: VerifyingKey,
    /// 由应用自己更新的安装路径：AppImage 文件，或安装包安装的 `winc.exe`。
    target: PathBuf,
    hooks: Hooks,
}

/// 更新器进行中的任务与已找到的更新包。
#[derive(Default)]
struct Work {
    /// 是否有检查或下载正在进行；同一时间只进行一项任务。
    busy: bool,
    /// 上次检查找到的更新包。
    item: Option<Item>,
    /// 已核对签名、等待进程退出后运行的程序：替换完成的 AppImage 或下载完成的安装包。
    ready: Option<PathBuf>,
}

static UPDATER: OnceLock<Updater> = OnceLock::new();
static WORK: Mutex<Work> = Mutex::new(Work {
    busy: false,
    item: None,
    ready: None,
});
/// 进程退出后是否启动新版本：由 [`restart`] 置位，由 [`cancel_restart`] 清除。
static RELAUNCH: AtomicBool = AtomicBool::new(false);

/// 返回是否启用了应用内更新。
///
/// 构建时同时给出了更新通道和公钥、且当前安装由应用自己更新时才启用。
///
/// `tick` 在启动后 [`FIRST_CHECK`]、之后每隔 [`CHECK_INTERVAL`] 在后台线程上调用。
/// 是否调用 [`check`] 由应用按设置决定。
pub fn start(
    found: impl Fn(Option<String>) + Send + Sync + 'static,
    state: impl Fn(UpdateState) + Send + Sync + 'static,
    tick: impl Fn() + Send + 'static,
) -> bool {
    let feeds = feeds(option_env!("WINC_UPDATE_CHANNELS").unwrap_or_default());
    let key = option_env!("WINC_UPDATE_PUBLIC_KEY").and_then(public_key);
    let (Some(key), Some(target)) = (key, platform::locate()) else {
        return false;
    };
    if feeds.is_empty() {
        return false;
    }
    let hooks = Hooks {
        found: Box::new(found),
        state: Box::new(state),
    };
    let updater = Updater {
        feeds,
        key,
        target,
        hooks,
    };
    if UPDATER.set(updater).is_err() {
        return true;
    }
    report(UpdateState::Idle);
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            tick();
            std::thread::sleep(CHECK_INTERVAL);
        }
    });
    true
}

/// 在后台检查更新。
pub fn check() {
    let Some(updater) = UPDATER.get() else {
        return;
    };
    if !begin() {
        return;
    }
    report(UpdateState::Checking);
    std::thread::spawn(move || {
        // 外层 `None` 表示各通道都没有回应；内层 `None` 表示没有更新的版本。
        let newer = race(&updater.feeds, &updater.key).map(|xml| {
            newest(&xml, std::env::consts::ARCH)
                .filter(|item| newer(&item.version, &crate::release_version::update_version()))
        });
        work(|w| {
            w.busy = false;
            if let Some(found) = &newer {
                w.item.clone_from(found);
            }
        });
        match newer {
            None => report(UpdateState::Failed),
            Some(None) => {
                (updater.hooks.found)(None);
                report(UpdateState::Latest);
            }
            Some(Some(item)) => {
                (updater.hooks.found)(Some(item.version.clone()));
                report(UpdateState::Found(item.version));
            }
        }
    });
}

/// 在后台下载上次检查找到的更新包，核对后准备安装。
///
/// 成功后状态为 [`UpdateState::Ready`]。没有找到过更新包、已有检查或下载在进行时不执行任何操作。
pub fn install() {
    let Some(updater) = UPDATER.get() else {
        return;
    };
    let Some(item) = work(|w| w.item.clone()) else {
        return;
    };
    if !begin() {
        return;
    }
    report(UpdateState::Downloading(item.version.clone(), 0));
    std::thread::spawn(move || {
        // 百分比变了才通知应用，每次通知都会重画各个窗口。
        let shown = std::cell::Cell::new(0);
        let progress = |percent: u8| {
            if percent != shown.replace(percent) {
                report(UpdateState::Downloading(item.version.clone(), percent));
            }
        };
        let result = platform::prepare(&item, &updater.target, &updater.key, &progress);
        work(|w| {
            w.busy = false;
            if let Ok(program) = &result {
                w.ready = Some(program.clone());
            }
        });
        match result {
            Ok(_) => report(UpdateState::Ready(item.version)),
            Err(e) => {
                eprintln!("winc: 无法安装更新 {}：{e}", item.version);
                report(UpdateState::InstallFailed(e));
            }
        }
    });
}

/// 要求进程退出后启动新版本；调用方随后正常退出。
pub fn restart() {
    RELAUNCH.store(true, Ordering::Relaxed);
}

/// 清除退出后启动新版本的要求。
pub fn cancel_restart() {
    RELAUNCH.store(false, Ordering::Relaxed);
}

/// 事件循环结束、进程退出前调用：把准备好的更新交给 `platform::after_exit`。
/// 该函数在要求过重新启动时于本进程结束后启动新版本。
pub fn after_exit() {
    if let Some(program) = work(|w| w.ready.take()) {
        platform::after_exit(&program, RELAUNCH.load(Ordering::Relaxed));
    }
}

/// 调用应用注册的回调，通知更新状态的变化。
fn report(state: UpdateState) {
    if let Some(updater) = UPDATER.get() {
        (updater.hooks.state)(state);
    }
}

/// 持有 [`WORK`] 锁执行 `f`；锁中毒时取回内部数据继续执行。
fn work<T>(f: impl FnOnce(&mut Work) -> T) -> T {
    f(&mut WORK.lock().unwrap_or_else(PoisonError::into_inner))
}

/// 标记开始一项任务，返回是否成功开始；已有任务在进行时返回 `false`。
fn begin() -> bool {
    work(|w| !std::mem::replace(&mut w.busy, true))
}

/// 由以空格分隔的 `名字=地址` 通道列表得出各通道 appcast 的完整地址。
fn feeds(channels: &str) -> Vec<String> {
    channels
        .split_whitespace()
        .filter_map(|pair| pair.split_once('=').map(|(_, base)| base))
        .filter(|base| !base.is_empty())
        .map(|base| format!("{}/{FEED_FILE}", base.trim_end_matches('/')))
        .collect()
}

/// 由 base64 编码的 32 字节公钥得出 [`VerifyingKey`]。
fn public_key(base64: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = decode_base64(base64)?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// 解码标准 base64，允许 `=` 补齐并忽略空白。
///
/// 出现其他字符时返回 `None`。
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = acc << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// 并发请求各通道的 appcast，返回最先取到的、签名核对通过的内容中签名覆盖的部分。
///
/// 内容须以 `<rss>` 为根元素；所有通道都失败时返回 `None`。
fn race(feeds: &[String], key: &VerifyingKey) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    for feed in feeds {
        let (tx, feed, key) = (tx.clone(), feed.clone(), *key);
        std::thread::spawn(move || {
            let body = fetch(&feed, FEED_TIMEOUT, None)
                .and_then(|bytes| signed_part(&bytes, &key).map(<[u8]>::to_vec))
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .filter(|text| text.contains("<rss") && text.contains("</rss>"));
            let _ = tx.send(body);
        });
    }
    drop(tx);
    rx.iter().flatten().next()
}

/// appcast 末尾 Sparkle 格式的签名注释所签的内容。
///
/// 注释以 `<!-- sparkle-signatures:` 开头，下一行起是 `edSignature: …`、`length: …`。签名用
/// `key` 核对通过时返回开头的 `length` 个字节；没有签名或核对失败时返回 `None`。
pub fn signed_part<'a>(feed: &'a [u8], key: &VerifyingKey) -> Option<&'a [u8]> {
    const MARK: &[u8] = b"<!-- sparkle-signatures:";
    let at = feed.windows(MARK.len()).rposition(|w| w == MARK)?;
    let comment = std::str::from_utf8(&feed[at + MARK.len()..]).ok()?;
    let comment = &comment[..comment.find("-->")?];
    let field = |name: &str| {
        comment
            .lines()
            .filter_map(|line| line.trim().split_once(':'))
            .find(|(k, _)| k.trim() == name)
            .map(|(_, v)| v.trim())
    };
    let signature: [u8; 64] = decode_base64(field("edSignature")?)?.try_into().ok()?;
    let length: usize = field("length")?.parse().ok()?;
    let signed = feed.get(..length).filter(|_| length <= at)?;
    key.verify_strict(signed, &Signature::from_bytes(&signature))
        .ok()
        .map(|()| signed)
}

/// 把 `url` 下载到 `path`：优先使用 `curl`，未安装时使用 `wget`。
///
/// 下载期间每隔 [`PROGRESS_POLL`] 按 `path` 的大小与预期的 `length`（字节）算出已下载的百分比，
/// 交给 `progress`。
///
/// # Errors
///
/// 命令无法运行时返回错误信息；下载失败时返回命令的错误输出或退出状态。
fn download(url: &str, path: &Path, length: u64, progress: &dyn Fn(u8)) -> Result<(), String> {
    let timeout = DOWNLOAD_TIMEOUT.to_string();
    let curl = command("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--max-time", &timeout, "--output"])
        .arg(path)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let child = match curl {
        Err(e) if e.kind() == io::ErrorKind::NotFound => command("wget")
            .args(["--quiet", "--timeout", &timeout, "--output-document"])
            .arg(path)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl、wget 都无法运行：{e}"))?,
        result => result.map_err(|e| format!("无法运行 curl：{e}"))?,
    };
    let output = watch(child, path, length, progress).map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if message.is_empty() {
        format!("下载失败（{}）", output.status)
    } else {
        message
    })
}

/// 等下载命令 `child` 结束，期间按 `path` 的大小报告进度（见 [`download`]），返回命令的输出。
///
/// 两个命令都只在出错时写标准错误，内容很少，结束前不读也不会写满管道。
#[allow(clippy::disallowed_methods, reason = "只访问本机：更新包下载到本机")]
fn watch(
    mut child: Child,
    path: &Path,
    length: u64,
    progress: &dyn Fn(u8),
) -> io::Result<std::process::Output> {
    while child.try_wait()?.is_none() {
        let done = std::fs::metadata(path).map_or(0, |m| m.len());
        progress(percent(done, length));
        std::thread::sleep(PROGRESS_POLL);
    }
    child.wait_with_output()
}

/// 已下载 `done` 字节、共 `length` 字节时的百分比，0 到 100；`length` 为 0 时为 0。
pub(crate) fn percent(done: u64, length: u64) -> u8 {
    if length == 0 {
        return 0;
    }
    u8::try_from(done.min(length) * 100 / length).unwrap_or(100)
}

/// 核对下载的内容：长度与 appcast 中的一致，且签名由 `key` 对应的私钥签出。
///
/// # Errors
///
/// 长度或签名不符时返回错误信息。
fn verify(item: &Item, bytes: &[u8], key: &VerifyingKey) -> Result<(), String> {
    if bytes.len() as u64 != item.length {
        return Err(format!(
            "大小不对：应为 {} 字节，下载到 {} 字节",
            item.length,
            bytes.len()
        ));
    }
    key.verify_strict(bytes, &Signature::from_bytes(&item.signature))
        .map_err(|_| "签名不对".to_owned())
}

/// 返回 appcast 中本机架构 `arch`（如 `x86_64`）上版本号最大的一项。
///
/// 没有匹配的更新项或格式不符合时返回 `None`。
///
/// 每项一个 `enclosure`：`sparkle:os` 为本平台（`platform` 模块的 `OS`），`arch` 为架构，
/// `length` 和 `sparkle:edSignature`。
pub fn newest(xml: &str, arch: &str) -> Option<Item> {
    #[cfg(windows)]
    let installer = platform::current_kind().map(|kind| kind.name());
    #[cfg(not(windows))]
    let installer = None;
    newest_for_installer(xml, arch, installer)
}

/// 按架构和安装类型筛选更新包；不需要安装包的平台不指定安装类型。
fn newest_for_installer(xml: &str, arch: &str, installer: Option<&str>) -> Option<Item> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let items = doc.descendants().filter(|n| n.has_tag_name("item"));
    let parsed = items.filter_map(|node| {
        let text = |name: &str| {
            node.children()
                .find(|c| c.has_tag_name((SPARKLE, name)))
                .and_then(|c| c.text())
                .map(str::trim)
        };
        let version = text("shortVersionString").or_else(|| text("version"))?;
        let enclosure = node.children().find(|c| {
            c.has_tag_name("enclosure")
                && c.attribute((SPARKLE, "os")) == Some(platform::OS)
                && c.attribute("arch") == Some(arch)
                && installer.is_none_or(|kind| c.attribute("installer") == Some(kind))
        })?;
        let signature = enclosure.attribute((SPARKLE, "edSignature"))?;
        Some(Item {
            version: version.to_owned(),
            url: enclosure.attribute("url")?.to_owned(),
            length: enclosure.attribute("length")?.parse().ok()?,
            signature: decode_base64(signature)?.try_into().ok()?,
        })
    });
    parsed.max_by(|a, b| winc_config::compare_versions(&a.version, &b.version))
}

/// 判断 `version` 是否比 `current` 新，比较包含 `SemVer` 预发布优先级。
pub fn newer(version: &str, current: &str) -> bool {
    winc_config::compare_versions(version, current).is_gt()
}

#[cfg(unix)]
mod appimage;
#[cfg(windows)]
mod installer;
#[cfg(test)]
mod tests;
