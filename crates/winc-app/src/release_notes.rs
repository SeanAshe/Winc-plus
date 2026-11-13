//! 更新日志：从发布站点读取，按版本取舍后整理成阅读标签显示的 Markdown。
//!
//! 更新日志每种语言一份，各版以「`## 版本号 · 日期`」一行开头，从新到旧排列。只在用户打开更新
//! 说明时请求。内容来自网络：长度有上限，格式不符时不显示，显示前使其中的图片和 HTML 失效
//! （见 [`inert`]）。

use winc_config::compare_versions;
use winc_i18n::Locale;

#[cfg(test)]
mod tests;

/// 更新日志所在的地址，其后接语言对应的文件名。
const NOTES: &str = "https://winc.si/releases/notes/";

/// 请求更新日志的等待上限（秒），与请求 appcast 的上限相同：两者是同一站点上大小相近的文件。
const TIMEOUT: u64 = 15;

/// 更新日志的长度上限（字节），1 MiB。
///
/// 站点只保留最近若干版，正常的日志为数十 KB；超过上限的应答不是更新日志，不显示。
const MAX_LENGTH: usize = 1 << 20;

/// 要显示的版本范围。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Scope {
    /// 日志中的全部版本。
    #[default]
    All,
    /// 更新之后：比上次运行的版本新、且不比当前版本新的各版；上次运行的版本没有记录时只有
    /// 当前版本。
    Since(Option<String>),
}

/// 返回语言 `locale` 的更新日志的地址。
pub(crate) fn url(locale: Locale) -> String {
    let file = match locale {
        Locale::ZhCn => "zh-cn.md",
        Locale::EnUs => "en.md",
    };
    format!("{NOTES}{file}")
}

/// 返回语言 `locale` 的、在浏览器中查看完整更新日志的页面地址。
pub(crate) fn page(locale: Locale) -> &'static str {
    match locale {
        Locale::ZhCn => "https://winc.si/zh-cn/downloads#changelog",
        Locale::EnUs => "https://winc.si/downloads#changelog",
    }
}

/// 取回语言 `locale` 的更新日志，返回其中 `scope` 范围内各版的 Markdown；`missing` 见
/// [`select`]。
///
/// 等待网络应答，在后台线程调用。请求失败、应答超过 [`MAX_LENGTH`] 或内容不是更新日志时返回
/// `None`。
pub(crate) fn load(locale: Locale, scope: &Scope, missing: &str) -> Option<String> {
    let bytes = crate::web::fetch(&url(locale), TIMEOUT, Some(MAX_LENGTH as u64))?;
    let text = std::str::from_utf8(&bytes).ok()?;
    select(text, scope, winc_version::VERSION, missing)
}

/// 返回更新日志 `text` 中 `scope` 范围内各版的 Markdown，`current` 是当前版本。
///
/// 范围内没有任何一版时（日志尚未包含当前版本）返回全部版本，并在最前面加一段 `missing`
/// 说明这一点。`text` 超过 [`MAX_LENGTH`]、第一个非空行不是版本标题，或没有任何一版时返回
/// `None`。
pub(crate) fn select(text: &str, scope: &Scope, current: &str, missing: &str) -> Option<String> {
    if text.len() > MAX_LENGTH {
        return None;
    }
    let releases = releases(text)?;
    let wanted = |version: &str| match scope {
        Scope::All => true,
        Scope::Since(Some(previous)) => {
            compare_versions(version, previous).is_gt()
                && compare_versions(version, current).is_le()
        }
        Scope::Since(None) => compare_versions(version, current).is_eq(),
    };
    let chosen: Vec<&str> = releases
        .iter()
        .filter(|(version, _)| wanted(version))
        .map(|(_, body)| *body)
        .collect();
    if chosen.is_empty() {
        let all: String = releases.iter().map(|(_, body)| *body).collect();
        return Some(format!("{missing}\n\n{}", inert(&all)));
    }
    Some(inert(&chosen.concat()))
}

/// 把更新日志 `text` 分成各版，返回每一版的（版本号，包括标题行在内的文字），顺序不变。
///
/// 第一个非空行不是版本标题，或没有任何一版时返回 `None`。
fn releases(text: &str) -> Option<Vec<(&str, &str)>> {
    let mut found: Vec<(&str, usize)> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        match version_of(line) {
            Some(version) => found.push((version, at)),
            None if found.is_empty() && !line.trim().is_empty() => return None,
            None => {}
        }
        at += line.len();
    }
    let ends = found
        .iter()
        .skip(1)
        .map(|&(_, start)| start)
        .chain([text.len()]);
    let releases: Vec<_> = found
        .iter()
        .zip(ends)
        .map(|(&(version, start), end)| (version, &text[start..end]))
        .collect();
    (!releases.is_empty()).then_some(releases)
}

/// 返回版本标题行 `line`（`## 版本号 · 日期`）中的版本号；不是版本标题时返回 `None`。
///
/// 版本号只含 ASCII 字母、数字、`.`、`-` 和 `+`，且以数字开头。
fn version_of(line: &str) -> Option<&str> {
    let version = line.strip_prefix("## ")?.split_whitespace().next()?;
    let valid = version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    valid.then_some(version)
}

/// 使 Markdown 文本 `text` 中的图片和 HTML 失效，其余内容不变。
///
/// 更新日志只使用标题、列表、行内代码、链接和加粗。内容来自网络，图片会使预览向任意地址发出
/// 请求，因此把代码之外的 `<` 写成字符引用，把 `![` 中的 `!` 转义；围栏代码块和行内代码中的
/// 内容原样保留。
fn inert(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        let start = line.trim_start();
        if start.starts_with("```") || start.starts_with("~~~") {
            fenced = !fenced;
            out.push_str(line);
        } else if fenced {
            out.push_str(line);
        } else {
            inert_line(line, &mut out);
        }
    }
    out
}

/// 把一行 `line` 追加到 `out`，使其中行内代码之外的图片和 HTML 失效（见 [`inert`]）。
fn inert_line(line: &str, out: &mut String) {
    let mut rest = line;
    while let Some(at) = rest.find(['`', '<', '!']) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if rest.starts_with('<') {
            out.push_str("&lt;");
            rest = &rest[1..];
        } else if rest.starts_with("![") {
            out.push_str("\\!");
            rest = &rest[1..];
        } else if rest.starts_with('!') {
            out.push('!');
            rest = &rest[1..];
        } else {
            // 行内代码以一串反引号开始，到下一串同样长度的反引号结束；没有结束时这串反引号是
            // 普通文字。
            let ticks = rest.len() - rest.trim_start_matches('`').len();
            let span = closing(&rest[ticks..], ticks).map_or(ticks, |end| ticks + end);
            out.push_str(&rest[..span]);
            rest = &rest[span..];
        }
    }
    out.push_str(rest);
}

/// 返回 `text` 中第一串恰好 `ticks` 个反引号结束的位置（字节偏移）；没有时返回 `None`。
fn closing(text: &str, ticks: usize) -> Option<usize> {
    let mut at = 0;
    while let Some(found) = text[at..].find('`') {
        let start = at + found;
        let run = text[start..].len() - text[start..].trim_start_matches('`').len();
        if run == ticks {
            return Some(start + run);
        }
        at = start + run;
    }
    None
}
