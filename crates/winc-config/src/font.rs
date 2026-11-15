//! 字体设置：字体名与字体文件路径的区分和解析。

use std::path::PathBuf;

/// 解析用户填写的字体设置，返回按字体名还是按文件路径加载。
///
/// 含路径分隔符（`/` 或 `\`），或扩展名为 `.ttf`、`.ttc`、`.otf`（不区分大小写）时按文件路径，
/// 其余按字体名交给系统查找（macOS 用 CoreText，Windows 用 DirectWrite，Linux 用 fontconfig）。
/// `font` 为 `None` 或只有空白时使用 `fallback`。
pub fn resolve_font(font: Option<&str>, fallback: &str) -> FontRequest {
    let Some(text) = font.map(str::trim).filter(|t| !t.is_empty()) else {
        return FontRequest::Name(fallback.to_string());
    };

    let lower = text.to_ascii_lowercase();
    let path_like = text.contains('/')
        || text.contains('\\')
        || lower.ends_with(".ttf")
        || lower.ends_with(".ttc")
        || lower.ends_with(".otf");

    if path_like {
        FontRequest::Path(PathBuf::from(text))
    } else {
        FontRequest::Name(text.to_string())
    }
}

/// 解析用户填写的字体列表（如 `JetBrains Mono, Noto Sans SC`），按书写顺序返回各字体。
///
/// 以英文逗号分隔，各项去掉首尾空白和成对的引号，空项略去；每一项按 [`resolve_font`] 区分字体名
/// 与文件路径。第一个字体缺字时依次用后面的字体。`font` 为 `None` 或没有字体时返回空列表。
pub fn resolve_font_list(font: Option<&str>) -> Vec<FontRequest> {
    (font_list(font).into_iter())
        .map(|name| resolve_font(Some(name), ""))
        .collect()
}

/// 字体列表中的各项，按书写顺序，规则同 [`resolve_font_list`]，但不区分字体名与路径。
pub fn font_list(font: Option<&str>) -> Vec<&str> {
    (font.unwrap_or_default().split(','))
        .map(unquote)
        .filter(|name| !name.is_empty())
        .collect()
}

/// 去掉 `text` 首尾的空白和成对的引号（`"` 或 `'`）。
fn unquote(text: &str) -> &str {
    let text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text.strip_prefix(quote).and_then(|t| t.strip_suffix(quote)) {
            return inner.trim();
        }
    }
    text
}

/// 字体设置解析后的去向：交给系统按名字查找，或者直接加载某个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontRequest {
    /// 字体名，交给系统按名字查找。
    Name(String),
    /// 字体文件路径，直接加载该文件。
    Path(PathBuf),
}

#[cfg(test)]
mod tests;
