//! 预览中图片和链接地址的去向：本地文件的路径、网络图片的下载和缓存。

use std::path::{Path, PathBuf};

use winc_i18n::t;
use winc_preview::image::{self, Picture};
use winc_preview::svg;

use crate::app::preview::images::system_glyphs;

/// 网址图片的下载缓存目录，文件名为网址的哈希。
const REMOTE_CACHE: &str = "winc-markdown-images";

/// 下载一张图片的最长等待时间（秒），传给 `curl` 的 `--max-time`。
const REMOTE_TIMEOUT: &str = "20";

/// 单张图片的最大下载大小（字节，20 MB），传给 `curl` 的 `--max-filesize`。
const REMOTE_MAX: &str = "20000000";

/// 下载一张图片时最多跟随的重定向次数，传给 `curl` 的 `--max-redirs`。
///
/// 图片托管服务通常经一到两次重定向到达内容分发的地址；取 5 次留出余量，并限制一个地址能
/// 引出的请求数。
const REMOTE_REDIRECTS: &str = "5";

/// `url` 是否为需要联网的地址。
pub(super) fn is_remote(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// 返回文档 `doc`（未保存过的文档为 `None`）中图片地址 `url` 对应的文件：本地图片见
/// [`local_target`]；网址的图片为下载的缓存文件，`remote` 为假（不允许联网）时返回 `None`。
pub(crate) fn image_path(doc: Option<&Path>, url: &str, remote: bool) -> Option<PathBuf> {
    if is_remote(url) {
        // FNV-1a：同一网址在每次启动时都映射到同一个文件，已下载的图片无需重新下载。
        let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        // 扩展名统一按位图写，读取时按内容判断真实格式。
        return remote.then(|| {
            std::env::temp_dir()
                .join(REMOTE_CACHE)
                .join(format!("{hash:016x}.png"))
        });
    }
    local_target(doc?, url)
}

/// 缓存图片的最长未使用时间（30 天），超过时删除。
pub(in crate::app) const CACHE_AGE: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 3600);

/// 缓存目录占用的磁盘上限（字节），200 MB。
const CACHE_MAX: u64 = 200 * 1024 * 1024;

/// 把 `url` 下载到 `path`。
///
/// 已下载过时只更新修改时间（清理缓存时据此判断最久未使用的文件）。先写入同目录的临时
/// 文件，下载完成后再改名，中途失败不会留下不完整的文件。每次启动后首次下载前清理一次缓存
/// （[`clean_cache`]）。
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
pub(super) fn download(url: &str, path: &Path) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or(t!("preview-markdown-cache-folder-invalid"))?;
    static CLEANED: std::sync::Once = std::sync::Once::new();
    CLEANED.call_once(|| clean_cache(dir, std::time::SystemTime::now()));
    if path.is_file() {
        if let Ok(file) = std::fs::File::options().write(true).open(path) {
            let _ = file.set_modified(std::time::SystemTime::now());
        }
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = path.with_extension(format!("part{}", std::process::id()));
    let mut command = crate::web::curl();
    command
        .args(["-fsSL", "--proto", "=http,https"])
        .args(["--max-redirs", REMOTE_REDIRECTS])
        .args(["--max-time", REMOTE_TIMEOUT])
        .args(["--max-filesize", REMOTE_MAX, "-o"])
        .arg(&part)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(t!("preview-markdown-download-failed").to_owned());
    }
    winc_config::files::replace_file(&part, path).map_err(|e| e.to_string())
}

/// 删除 `dir` 中超过 [`CACHE_AGE`] 未使用的图片，以及超过一小时未下载完成的临时文件；
/// 剩余文件超过 [`CACHE_MAX`] 时从最久未使用的开始删除。
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
pub(in crate::app) fn clean_cache(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        let partial = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.starts_with("part"));
        let stale = if partial {
            age > std::time::Duration::from_secs(3600)
        } else {
            age > CACHE_AGE
        };
        if stale {
            let _ = std::fs::remove_file(&path);
        } else if !partial {
            kept.push((age, meta.len(), path));
        }
    }
    let mut total: u64 = kept.iter().map(|k| k.1).sum();
    // 最久未使用的排在前面。
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, len, path) in kept {
        if total <= CACHE_MAX {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// 读取下载的图片：内容为 SVG（徽章多为 SVG）时按 SVG 读取，否则按位图读取。
///
/// SVG 中的文字（徽章上的字）用系统字体绘制，与直接预览 SVG 文件时相同。
#[allow(clippy::disallowed_methods, reason = "只访问本机：图片缓存在本机")]
pub(super) fn load_downloaded(path: &Path) -> Result<Picture, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(4).any(|w| w == b"<svg") {
        let text = String::from_utf8_lossy(&bytes);
        return svg::load(&text, None, Some(system_glyphs()));
    }
    image::load(path)
}

/// 返回文档 `doc` 中地址 `url` 指向的本地文件；网址、`mailto:` 等返回 `None`。
///
/// 去掉 `#…`、`?…`，相对路径按文档所在目录解析，并还原 `%20` 等转义。`file://主机/路径`
/// 取其中的路径：本地文件都在编辑区中打开，不交给系统，因为系统打开 `.app`、`.command`
/// 时会运行这些文件。
pub(crate) fn local_target(doc: &Path, url: &str) -> Option<PathBuf> {
    let url = match url.strip_prefix("file://") {
        Some(rest) => &rest[rest.find('/')?..],
        None if url.is_empty()
            || url.contains("://")
            || url.starts_with("mailto:")
            || url.starts_with("data:") =>
        {
            return None;
        }
        None => url,
    };
    let path = url.split(['#', '?']).next().unwrap_or_default();
    if path.is_empty() {
        return None;
    }
    let path = percent_decode(path);
    if winc_backend::names_host(&path) {
        return None;
    }
    let path = Path::new(&path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        doc.parent()?.join(path)
    })
}

/// 还原网址里 `%xx` 形式的转义；不是合法 UTF-8 时原样返回。
pub(crate) fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}
