//! 文件编码的识别、解码与编码。
//!
//! 读取时先识别 BOM（UTF-8、UTF-16），再判断是否为合法的 UTF-8；都不是时按内容猜测编码（GBK、
//! Big5、Shift JIS 等）。写回时沿用原来的编码，用户也可以通过「通过编码重新打开」
//! 「通过编码保存」指定编码。
//!
//! 编解码使用 `encoding_rs`，猜测编码使用 `chardetng`。`encoding_rs` 遵循网页规范，不做 UTF-16
//! 编码，因此 UTF-16 的写出由本模块自行实现。

use std::borrow::Cow;
use std::sync::OnceLock;

use winc_i18n::t;

/// 文件的编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// 不带 BOM 的 UTF-8。
    Utf8,
    /// 带 BOM 的 UTF-8，BOM 为 `EF BB BF`。
    Utf8Bom,
    /// 带 BOM 的 UTF-16 小端。
    ///
    /// 没有 BOM 的 UTF-16 无法识别：内容含大量 NUL，会被判为二进制文件。
    Utf16Le,
    /// 带 BOM 的 UTF-16 大端。
    Utf16Be,
    /// 其他编码（由 `encoding_rs` 提供），如 GBK、Big5、Shift JIS、Windows 1252。
    Legacy(&'static encoding_rs::Encoding),
    /// 无法按任何编码无损解码。
    ///
    /// 非法字节显示为 U+FFFD，文档只读，避免保存时破坏原文件。
    Lossy,
    /// 二进制文件，用户选择了「以文本方式打开」。
    ///
    /// 与 `Lossy` 一样只读。
    Binary,
}

/// 编码菜单中列出的编码及其完整名称，仅包含常用项。
///
/// `encoding_rs` 支持的编码中，部分 DOS 代码页等不常用项不列入。
pub const CHOICES: &[(Encoding, ChoiceName)] = &[
    (Encoding::Utf8, ChoiceName(|| "UTF-8")),
    (Encoding::Utf8Bom, ChoiceName(|| "UTF-8 BOM")),
    (Encoding::Utf16Le, ChoiceName(|| "UTF-16 LE")),
    (Encoding::Utf16Be, ChoiceName(|| "UTF-16 BE")),
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

impl Encoding {
    /// 返回状态栏上显示的短名称，如 `UTF-8`、`GBK`。
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Lossy => t!("editor-encoding-lossy"),
            Encoding::Binary => t!("editor-encoding-binary"),
            Encoding::Legacy(e) => CHOICES
                .iter()
                .find(|(c, _)| *c == self)
                .map_or(e.name(), |(_, long)| short(long)),
        }
    }

    /// 按该编码把 `text` 编码为字节，并带上该编码的 BOM。
    ///
    /// # Errors
    ///
    /// 编码表示不了文本中的某个字符时，返回第一个这样的字符。
    pub fn encode(self, text: &str) -> Result<Vec<u8>, char> {
        encode(text, self)
    }

    /// 返回写进备份头的名字，[`Encoding::from_key`] 能还原。
    ///
    /// 不能写回的 `Lossy`、`Binary` 返回 `None`。
    pub(crate) fn key(self) -> Option<&'static str> {
        match self {
            Encoding::Utf8 => Some("UTF-8"),
            Encoding::Utf8Bom => Some("UTF-8 BOM"),
            Encoding::Utf16Le => Some("UTF-16LE"),
            Encoding::Utf16Be => Some("UTF-16BE"),
            Encoding::Legacy(e) => Some(e.name()),
            Encoding::Lossy | Encoding::Binary => None,
        }
    }

    /// 按 [`Encoding::key`] 写出的名字还原编码；不认识的名字返回 `None`。
    pub(crate) fn from_key(key: &str) -> Option<Encoding> {
        match key {
            "UTF-8" => Some(Encoding::Utf8),
            "UTF-8 BOM" => Some(Encoding::Utf8Bom),
            "UTF-16LE" => Some(Encoding::Utf16Le),
            "UTF-16BE" => Some(Encoding::Utf16Be),
            name => encoding_rs::Encoding::for_label_no_replacement(name.as_bytes())
                // 只认 `key` 写出的规范名字；`UTF-8` 等已在上面的分支里认出，不会到这里。
                .filter(|e| e.name() == name)
                .map(Encoding::Legacy),
        }
    }

    /// 是否可以按该编码写回文件。
    pub fn writable(self) -> bool {
        !matches!(self, Encoding::Lossy | Encoding::Binary)
    }
}

/// 返回长名称括号中的部分，例如「简体中文 (GBK)」返回 `GBK`；没有括号时返回原字符串。
fn short(long: &'static str) -> &'static str {
    long.rsplit_once(" (")
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .unwrap_or(long)
}

/// 用户所在的地区（顶级域名写法，如 `cn`），猜测编码时作参考；启动时按系统语言设置一次。
///
/// `chardetng` 以网站的顶级域名作为提示。很短的 GBK 文本与 EUC-KR、Big5 难以区分，知道用户
/// 使用简体中文时倾向于判为 GBK。
static REGION: OnceLock<String> = OnceLock::new();

/// 按系统语言（如 `zh-Hans-CN`、`zh_TW.UTF-8`、`ja`）设置地区。
///
/// 只有第一次调用有效；无法得出地区时不设置。
pub fn set_region_from_language(language: &str) {
    if let Some(region) = region_of(language) {
        let _ = REGION.set(region);
    }
}

/// 返回系统语言对应的地区（顶级域名写法）：带地区码时取地区码；简体中文、繁体中文、日文、
/// 韩文没有地区码时按语言补全；其他语言返回 `None`。
fn region_of(language: &str) -> Option<String> {
    let language = language.split(['.', '@']).next()?;
    let parts: Vec<&str> = language.split(['-', '_']).collect();
    if let Some(region) = parts
        .iter()
        .skip(1)
        .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return Some(region.to_ascii_lowercase());
    }
    let script = parts.get(1).map(|s| s.to_ascii_lowercase());
    match (parts[0].to_ascii_lowercase().as_str(), script.as_deref()) {
        ("zh", Some("hant")) => Some("tw".into()),
        ("zh", _) => Some("cn".into()),
        ("ja", _) => Some("jp".into()),
        ("ko", _) => Some("kr".into()),
        _ => None,
    }
}

/// UTF-8 的字节顺序标记（BOM）：`EF BB BF`。
///
/// 打开文件和全局搜索都用它。
pub(crate) const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// UTF-16 小端的字节顺序标记（BOM）：`FF FE`。
const UTF16LE_BOM: [u8; 2] = [0xFF, 0xFE];
/// UTF-16 大端的字节顺序标记（BOM）：`FE FF`。
const UTF16BE_BOM: [u8; 2] = [0xFE, 0xFF];

/// 是否以 UTF-16 的 BOM 开头。
///
/// 这样的文件含大量 NUL，但不是二进制文件。
pub(crate) fn utf16_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&UTF16LE_BOM) || bytes.starts_with(&UTF16BE_BOM)
}

/// 识别不带 BOM 的 UTF-16 时看的文件前缀长度（字节）。
const UNMARKED_SNIFF: usize = 512;

/// 不带 BOM 的 UTF-16 的字节序：开头 [`UNMARKED_SNIFF`] 字节里有 NUL，并且每两个字节中恰好
/// 高位那个是 NUL（小端在后、大端在前）。只认得出以 ASCII 字符为主的文字；不足这么长、字节数
/// 又是奇数的不是 UTF-16。
pub(crate) fn utf16_unmarked(bytes: &[u8]) -> Option<Encoding> {
    let head = &bytes[..bytes.len().min(UNMARKED_SNIFF)];
    if !head.contains(&0) || head.len() % 2 == 1 {
        return None;
    }
    let fits =
        |odd: bool| (head.iter().enumerate()).all(|(i, &b)| (b == 0) == ((i % 2 == 1) == odd));
    if fits(true) {
        Some(Encoding::Utf16Le)
    } else if fits(false) {
        Some(Encoding::Utf16Be)
    } else {
        None
    }
}

/// 是否是 UTF-16 的文字（带 BOM，或 [`utf16_unmarked`]）：含大量 NUL，但不是二进制文件。
pub(crate) fn utf16_text(bytes: &[u8]) -> bool {
    utf16_bom(bytes) || utf16_unmarked(bytes).is_some()
}

/// 整个文件的字节解码得到的文字。
pub(crate) struct Decoded<'a> {
    /// 去掉了开头 BOM 的文字。
    pub text: Cow<'a, str>,
    /// 识别出的编码。
    pub encoding: Encoding,
}

impl Decoded<'_> {
    /// 是否无法无损解码（非法字节已替换为 U+FFFD）；这样的文件只读。
    pub fn lossy(&self) -> bool {
        self.encoding == Encoding::Lossy
    }
}

/// 按打开文件时的规则把字节解码为文字。
///
/// 依次判断 BOM（UTF-8、UTF-16）、是否为合法的 UTF-8、是否大体为 UTF-8、按内容猜测的编码。
/// 猜出的编码必须能无损解码整个文件才采用，否则按替换字符解码（[`Encoding::Lossy`]）。
/// 合法的 UTF-8 不复制。
pub(crate) fn decode(bytes: &[u8]) -> Decoded<'_> {
    decode_in(bytes, REGION.get().map(String::as_str))
}

/// 与 [`decode`] 相同，但猜测编码时参考 `region`（顶级域名写法，如 `cn`、`tw`）。
fn decode_in<'a>(bytes: &'a [u8], region: Option<&str>) -> Decoded<'a> {
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        return match std::str::from_utf8(rest) {
            Ok(text) => Decoded {
                text: Cow::Borrowed(text),
                encoding: Encoding::Utf8Bom,
            },
            Err(_) => lossy(rest),
        };
    }
    for (bom, encoding, codec) in [
        (UTF16LE_BOM, Encoding::Utf16Le, encoding_rs::UTF_16LE),
        (UTF16BE_BOM, Encoding::Utf16Be, encoding_rs::UTF_16BE),
    ] {
        if let Some(rest) = bytes.strip_prefix(&bom) {
            return match codec.decode_without_bom_handling_and_without_replacement(rest) {
                Some(text) => Decoded {
                    text: Cow::Owned(text.into_owned()),
                    encoding,
                },
                // 含不完整或损坏的字节时仍按 UTF-16 解码，损坏处替换为 U+FFFD，文档只读；
                // 按 UTF-8 解码会得到夹杂大量 NUL 的乱码。
                None => {
                    let (text, _) = codec.decode_without_bom_handling(rest);
                    Decoded {
                        text: Cow::Owned(text.into_owned()),
                        encoding: Encoding::Lossy,
                    }
                }
            };
        }
    }
    // 以 ASCII 为主的 UTF-16 夹着 NUL，也是合法的 UTF-8，要在那之前认出来。
    if let Some(encoding) = utf16_unmarked(bytes) {
        let (text, damaged) = decode_as(bytes, encoding);
        return Decoded {
            text: Cow::Owned(text),
            encoding: if damaged { Encoding::Lossy } else { encoding },
        };
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Decoded {
            text: Cow::Borrowed(text),
            encoding: Encoding::Utf8,
        };
    }

    // 大体是 UTF-8、只有少数损坏的字节（拼接、截断造成）时仍按 UTF-8 显示（只读），
    // 避免被猜成其他编码而使整个文件显示为乱码。
    if mostly_utf8(bytes) {
        return lossy(bytes);
    }

    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(detection_sample(bytes), true);
    let guess = detector.guess(region.map(str::as_bytes), false);
    // `encoding_rs` 的 GBK 解码器与 GB18030 相同（识别四字节扩展），编码器却只能写出 GBK 的
    // 两字节形式：猜成 GBK 的文件含四字节字符（emoji、生僻字）时保存会被拒绝，记成 GB18030
    // 才能写回。
    let encoding = if guess == encoding_rs::GBK {
        encoding_rs::GB18030
    } else {
        guess
    };
    match encoding.decode_without_bom_handling_and_without_replacement(bytes) {
        Some(text) if encoding != encoding_rs::UTF_8 => Decoded {
            text: Cow::Owned(text.into_owned()),
            encoding: Encoding::Legacy(encoding),
        },
        _ => lossy(bytes),
    }
}

/// 猜测编码时交给 `chardetng` 的最多字节数（字节，64 KiB）。
///
/// 只影响猜测：猜出的编码仍须能无损解码整个文件才采用，否则按替换字符解码。样本长度固定，猜测的
/// 耗时因此与文件大小无关，16 MB 的文件同样只看 64 KiB；`chardetng` 逐字节统计整个样本，判定
/// 结果不随样本长度改变。
const DETECT_SAMPLE: usize = 64 << 10;

/// 返回交给 `chardetng` 猜测编码的一段字节：从第一个非 ASCII 字节所在行的行首起，最多
/// [`DETECT_SAMPLE`] 字节。
///
/// 纯 ASCII 的部分不提供区分编码的信息，跳过；保留所在行的行首，使非 ASCII 字符前后的拉丁字母
/// 仍参与判断。
fn detection_sample(bytes: &[u8]) -> &[u8] {
    let first = bytes.iter().position(|b| !b.is_ascii()).unwrap_or(0);
    let start = memchr::memrchr(b'\n', &bytes[..first]).map_or(0, |i| i + 1);
    &bytes[start..bytes.len().min(start + DETECT_SAMPLE)]
}

/// 是否大体是 UTF-8：合法的非 ASCII 字符数不少于损坏处（非法字节序列）个数的 8 倍。
///
/// GBK、Big5 等编码的字节恰好构成合法 UTF-8 多字节字符的情况很少，单字节编码（如 Windows 1252）
/// 的非 ASCII 字节几乎都构不成；8 倍用于容忍少量拼接、截断造成的损坏。该倍数取自调参，作用范围
/// 限于打开文件时的编码猜测。
fn mostly_utf8(mut bytes: &[u8]) -> bool {
    let (mut good, mut bad) = (0usize, 0usize);
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                good += text.chars().filter(|c| !c.is_ascii()).count();
                break;
            }
            Err(e) => {
                let (valid, rest) = bytes.split_at(e.valid_up_to());
                // `valid` 是合法的 UTF-8，统计其中的非 ASCII 字符。
                good += std::str::from_utf8(valid)
                    .map_or(0, |t| t.chars().filter(|c| !c.is_ascii()).count());
                bad += 1;
                match e.error_len() {
                    Some(n) => bytes = &rest[n..],
                    // 末尾是不完整的字符。
                    None => break,
                }
            }
        }
    }
    good >= 8 * bad
}

/// 把 `bytes` 按替换字符解码，编码记为 [`Encoding::Lossy`]。
fn lossy(bytes: &[u8]) -> Decoded<'_> {
    Decoded {
        text: String::from_utf8_lossy(bytes),
        encoding: Encoding::Lossy,
    }
}

/// 把不知道编码的字节（例如没在编辑器里打开的文件）解码成文字：规则同打开文件，识别 BOM、
/// UTF-8 和常见的旧编码。
///
/// 返回文字和识别出的编码。
pub fn decode_unknown(bytes: &[u8]) -> (String, Encoding) {
    let decoded = decode(bytes);
    (decoded.text.into_owned(), decoded.encoding)
}

/// 按已知的编码 `encoding` 把字节解码成文字，无法解码的字节换成 U+FFFD。
pub fn decode_known(bytes: &[u8], encoding: Encoding) -> String {
    decode_as(bytes, encoding).0
}

/// 按用户指定的编码（「通过编码重新打开」）解码，并去掉该编码自己的 BOM。
///
/// 返回（文字，是否有无法解码的字节）。无法解码的字节替换为 U+FFFD，此时第二个值为 `true`，
/// 以这种方式打开的文档只读。
pub(crate) fn decode_as(bytes: &[u8], encoding: Encoding) -> (String, bool) {
    let (codec, bom): (&'static encoding_rs::Encoding, &[u8]) = match encoding {
        Encoding::Utf8 | Encoding::Lossy | Encoding::Binary => (encoding_rs::UTF_8, &[]),
        Encoding::Utf8Bom => (encoding_rs::UTF_8, &UTF8_BOM),
        Encoding::Utf16Le => (encoding_rs::UTF_16LE, &UTF16LE_BOM),
        Encoding::Utf16Be => (encoding_rs::UTF_16BE, &UTF16BE_BOM),
        Encoding::Legacy(e) => (e, &[]),
    };
    let bytes = bytes.strip_prefix(bom).unwrap_or(bytes);
    let (text, had_errors) = codec.decode_without_bom_handling(bytes);
    (text.into_owned(), had_errors)
}

/// 按 `encoding` 把 `text` 编码为字节（带上该编码的 BOM）。
///
/// UTF-8 不经过这里（直接把 rope 逐块写出）。
///
/// # Errors
///
/// `encoding` 表示不了文本中的字符时，返回第一个这样的字符。
pub(crate) fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, char> {
    match encoding {
        Encoding::Utf8 | Encoding::Lossy | Encoding::Binary => Ok(text.as_bytes().to_vec()),
        Encoding::Utf8Bom => {
            let mut out = UTF8_BOM.to_vec();
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        // `encoding_rs` 遵循网页规范，不做 UTF-16 编码，因此这里自行实现。
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let le = encoding == Encoding::Utf16Le;
            let mut out = if le { UTF16LE_BOM } else { UTF16BE_BOM }.to_vec();
            out.reserve(text.len() * 2);
            for unit in text.encode_utf16() {
                out.extend_from_slice(&if le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            Ok(out)
        }
        Encoding::Legacy(codec) => {
            let (bytes, _, had_errors) = codec.encode(text);
            if !had_errors {
                return Ok(bytes.into_owned());
            }
            // `encoding_rs` 会把无法表示的字符替换为 `&#…;`，因此逐个字符单独编码，找出第一个
            // 无法表示的字符。
            let mut buf = [0u8; 4];
            Err(text
                .chars()
                .find(|c| codec.encode(c.encode_utf8(&mut buf)).2)
                .unwrap_or('\u{fffd}'))
        }
    }
}

#[cfg(test)]
mod tests;
