//! 文件编码：识别、解码与编码的规则在 `winc_search::text`，这里加上界面上的名称（编码菜单、
//! 状态栏）。

pub use winc_search::text::*;

use winc_i18n::t;

/// 编码菜单中列出的编码及其完整名称，仅包含常用项。
///
/// `encoding_rs` 支持的编码中，部分 DOS 代码页等不常用项不列入。
pub const CHOICES: &[(Encoding, ChoiceName)] = &[
    (Encoding::Utf8, ChoiceName(|| "UTF-8")),
    (Encoding::Utf8Bom, ChoiceName(|| "UTF-8 BOM")),
    (Encoding::Utf16Le, ChoiceName(|| "UTF-16 LE")),
    (Encoding::Utf16Be, ChoiceName(|| "UTF-16 BE")),
    (
        Encoding::UnicodeEscapes {
            lowercase: false,
            latin1: false,
        },
        ChoiceName(|| t!("editor-encoding-unicode-escapes")),
    ),
    (
        Encoding::Legacy(encoding_rs::GB18030),
        ChoiceName(|| t!("editor-encoding-gb18030")),
    ),
    (
        Encoding::Legacy(encoding_rs::GBK),
        ChoiceName(|| t!("editor-encoding-gbk")),
    ),
    (
        Encoding::Legacy(encoding_rs::BIG5),
        ChoiceName(|| t!("editor-encoding-big5")),
    ),
    (
        Encoding::Legacy(encoding_rs::SHIFT_JIS),
        ChoiceName(|| t!("editor-encoding-shift-jis")),
    ),
    (
        Encoding::Legacy(encoding_rs::EUC_JP),
        ChoiceName(|| t!("editor-encoding-euc-jp")),
    ),
    (
        Encoding::Legacy(encoding_rs::EUC_KR),
        ChoiceName(|| t!("editor-encoding-euc-kr")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1252),
        ChoiceName(|| t!("editor-encoding-windows-1252")),
    ),
    (
        Encoding::Legacy(encoding_rs::MACINTOSH),
        ChoiceName(|| t!("editor-encoding-mac-roman")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1250),
        ChoiceName(|| t!("editor-encoding-windows-1250")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1251),
        ChoiceName(|| t!("editor-encoding-windows-1251")),
    ),
    (
        Encoding::Legacy(encoding_rs::KOI8_R),
        ChoiceName(|| t!("editor-encoding-koi8-r")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1253),
        ChoiceName(|| t!("editor-encoding-windows-1253")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1254),
        ChoiceName(|| t!("editor-encoding-windows-1254")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1255),
        ChoiceName(|| t!("editor-encoding-windows-1255")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1256),
        ChoiceName(|| t!("editor-encoding-windows-1256")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1257),
        ChoiceName(|| t!("editor-encoding-windows-1257")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_1258),
        ChoiceName(|| t!("editor-encoding-windows-1258")),
    ),
    (
        Encoding::Legacy(encoding_rs::WINDOWS_874),
        ChoiceName(|| t!("editor-encoding-windows-874")),
    ),
];

/// 编码菜单中一种编码的完整名称，取用时按当前界面语言返回文案。
///
/// 解引用为 `str`，用法与字符串相同。
#[derive(Clone, Copy)]
pub struct ChoiceName(fn() -> &'static str);

impl std::ops::Deref for ChoiceName {
    type Target = str;

    fn deref(&self) -> &str {
        (self.0)()
    }
}

/// 返回状态栏上显示的短名称，如 `UTF-8`、`GBK`。
pub fn label(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf8Bom => "UTF-8 BOM",
        Encoding::Utf16Le => "UTF-16 LE",
        Encoding::Utf16Be => "UTF-16 BE",
        Encoding::Lossy => t!("editor-encoding-lossy"),
        Encoding::Binary => t!("editor-encoding-binary"),
        Encoding::UnicodeEscapes { .. } => t!("editor-encoding-unicode-escapes-short"),
        Encoding::Legacy(e) => CHOICES
            .iter()
            .find(|(c, _)| *c == encoding)
            .map_or(e.name(), |(_, long)| short(long)),
    }
}

/// 返回长名称括号中的部分，例如「简体中文 (GBK)」返回 `GBK`；没有括号时返回原字符串。
fn short(long: &'static str) -> &'static str {
    long.rsplit_once(" (")
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .unwrap_or(long)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_encodings_show_the_short_name() {
        assert_eq!(label(Encoding::Legacy(encoding_rs::GBK)), "GBK");
        assert_eq!(label(Encoding::Legacy(encoding_rs::BIG5)), "Big5");
        assert_eq!(
            label(Encoding::Legacy(encoding_rs::WINDOWS_1252)),
            "Windows 1252"
        );
    }
}
