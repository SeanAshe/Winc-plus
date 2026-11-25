//! 文件编码的识别、解码与编码。
//!
//! 读取时先识别 BOM（UTF-8、UTF-16），再判断是否为合法的 UTF-8；都不是时按内容猜测编码（GBK、
//! Big5、Shift JIS 等）。写回时沿用原来的编码，用户也可以指定编码。打开文件和搜索（本机的，
//! 以及远端主机上的）都按这里的规则解码，搜出的行号、列与打开后的文本一致。
//!
//! 编解码使用 `encoding_rs`，猜测编码使用 `chardetng`。`encoding_rs` 遵循网页规范，不做 UTF-16
//! 编码，因此 UTF-16 的写出由本模块自行实现。

use std::borrow::Cow;
use std::sync::OnceLock;

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
    /// 其他字符写成 `\uXXXX` 转义的文本（Java 的 `.properties`）：读入时把转义换成字符，保存时
    /// 写回转义。
    UnicodeEscapes {
        /// 写回转义时十六进制是否用小写。
        lowercase: bool,
        /// 文件按 ISO-8859-1 存放：U+00FF 以内的字符直接写成一个字节，不转义；否则文件只含 ASCII。
        latin1: bool,
    },
}

impl Encoding {
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
    pub fn key(self) -> Option<&'static str> {
        match self {
            Encoding::Utf8 => Some("UTF-8"),
            Encoding::Utf8Bom => Some("UTF-8 BOM"),
            Encoding::Utf16Le => Some("UTF-16LE"),
            Encoding::Utf16Be => Some("UTF-16BE"),
            Encoding::UnicodeEscapes { lowercase, latin1 } => Some(match (latin1, lowercase) {
                (false, false) => "UNICODE-ESCAPES",
                (false, true) => "UNICODE-ESCAPES-LOWER",
                (true, false) => "UNICODE-ESCAPES-LATIN1",
                (true, true) => "UNICODE-ESCAPES-LATIN1-LOWER",
            }),
            Encoding::Legacy(e) => Some(e.name()),
            Encoding::Lossy | Encoding::Binary => None,
        }
    }

    /// 按 [`Encoding::key`] 写出的名字还原编码；不认识的名字返回 `None`。
    pub fn from_key(key: &str) -> Option<Encoding> {
        match key {
            "UTF-8" => Some(Encoding::Utf8),
            "UTF-8 BOM" => Some(Encoding::Utf8Bom),
            "UTF-16LE" => Some(Encoding::Utf16Le),
            "UTF-16BE" => Some(Encoding::Utf16Be),
            "UNICODE-ESCAPES"
            | "UNICODE-ESCAPES-LOWER"
            | "UNICODE-ESCAPES-LATIN1"
            | "UNICODE-ESCAPES-LATIN1-LOWER" => Some(Encoding::UnicodeEscapes {
                lowercase: key.ends_with("-LOWER"),
                latin1: key.contains("-LATIN1"),
            }),
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

    /// 编码菜单中对应的一项：各种写法的 Unicode 转义对应同一项。
    pub fn choice(self) -> Encoding {
        match self {
            Encoding::UnicodeEscapes { .. } => Encoding::UnicodeEscapes {
                lowercase: false,
                latin1: false,
            },
            other => other,
        }
    }
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

/// 直接设置地区（顶级域名写法，如 `cn`）：远端主机上的搜索沿用本机的地区，两边猜出的编码
/// 相同。只有第一次调用有效。
pub fn set_region(region: &str) {
    let _ = REGION.set(region.to_owned());
}

/// 返回已设置的地区；没有设置时返回 `None`。
pub fn region() -> Option<&'static str> {
    REGION.get().map(String::as_str)
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
pub const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// UTF-16 小端的字节顺序标记（BOM）：`FF FE`。
const UTF16LE_BOM: [u8; 2] = [0xFF, 0xFE];
/// UTF-16 大端的字节顺序标记（BOM）：`FE FF`。
const UTF16BE_BOM: [u8; 2] = [0xFE, 0xFF];

/// 是否以 UTF-16 的 BOM 开头。
///
/// 这样的文件含大量 NUL，但不是二进制文件。
pub fn utf16_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&UTF16LE_BOM) || bytes.starts_with(&UTF16BE_BOM)
}

/// 识别不带 BOM 的 UTF-16 时看的文件前缀长度（字节）。
const UNMARKED_SNIFF: usize = 512;

/// 不带 BOM 的 UTF-16 的字节序：开头 512 字节里有 NUL，并且每两个字节中恰好
/// 高位那个是 NUL（小端在后、大端在前）。只认得出以 ASCII 字符为主的文字；不足该长度、字节数
/// 又是奇数的不是 UTF-16。
pub fn utf16_unmarked(bytes: &[u8]) -> Option<Encoding> {
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
pub fn utf16_text(bytes: &[u8]) -> bool {
    utf16_bom(bytes) || utf16_unmarked(bytes).is_some()
}

/// 整个文件的字节解码得到的文字。
pub struct Decoded<'a> {
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
pub fn decode(bytes: &[u8]) -> Decoded<'_> {
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
pub fn decode_as(bytes: &[u8], encoding: Encoding) -> (String, bool) {
    if let Encoding::UnicodeEscapes { latin1, .. } = encoding {
        // 依据 `java.util.PropertyResourceBundle`：不是合法的 UTF-8 时按 ISO-8859-1 读。
        let text = match std::str::from_utf8(bytes) {
            Ok(text) if !latin1 => text.to_owned(),
            _ => bytes.iter().map(|&b| char::from(b)).collect(),
        };
        return (winc_core::transform::unescape_unicode_text(&text), false);
    }
    let (codec, bom): (&'static encoding_rs::Encoding, &[u8]) = match encoding {
        Encoding::Utf8 | Encoding::Lossy | Encoding::Binary | Encoding::UnicodeEscapes { .. } => {
            (encoding_rs::UTF_8, &[])
        }
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
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, char> {
    match encoding {
        Encoding::Utf8 | Encoding::Lossy | Encoding::Binary => Ok(text.as_bytes().to_vec()),
        Encoding::UnicodeEscapes { lowercase, latin1 } => {
            let written = winc_core::transform::escape_unicode_text(text, lowercase, latin1);
            Ok(if latin1 {
                // 写出的字符都在 U+00FF 以内，一个字符一个字节。
                written.chars().map(|c| u32::from(c) as u8).collect()
            } else {
                written.into_bytes()
            })
        }
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

/// 按 Unicode 转义读入的文件中，含转义或其他字符的各行在文件中原来的写法。
///
/// 保存时未修改的行按原来的写法写回，修改或新增的行按文件中多数行的写法写，见 [`encode_with`]。
#[derive(Debug, Default)]
pub struct EscapeOrigin {
    /// 显示的一行（不含换行符）对应的文件中的写法；同样内容的行按出现的顺序排列。
    lines: std::collections::HashMap<String, Vec<String>>,
    /// 修改或新增的行是否直接写字符：直接写字符的行多于含转义的行时为真。
    raw: bool,
    /// 文件是否按 ISO-8859-1 存放；与保存的编码不同时原来的写法不能用。
    latin1: bool,
}

/// 按 `encoding` 解码后的 `.properties` 文字 `text` 中有 `\uXXXX` 转义时，返回把转义换成字符后的
/// 文字、对应的 [`Encoding::UnicodeEscapes`] 和各行原来的写法；没有转义时返回 `None`。
///
/// 文件按 UTF-8 或 ISO-8859-1（识别为 Windows-1252）存放；直接写的字符和转义可以混用。
/// `forced` 是用户选择的、或文档已在使用的 Unicode 转义：此时没有转义也转换。
pub fn unicode_escapes_view(
    text: &str,
    encoding: Encoding,
    forced: Option<Encoding>,
) -> Option<(String, Encoding, EscapeOrigin)> {
    use winc_core::transform::unescape_unicode_text;
    let (base, latin1) = match encoding {
        // 只有 ASCII 时两种存法的字节相同，沿用原来的。
        Encoding::Utf8 => {
            let forced = matches!(forced, Some(Encoding::UnicodeEscapes { latin1: true, .. }));
            (Cow::Borrowed(text), forced && text.is_ascii())
        }
        Encoding::Legacy(codec) if codec == encoding_rs::WINDOWS_1252 => {
            let (bytes, _, lossy) = codec.encode(text);
            if lossy {
                return None;
            }
            (
                Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect()),
                true,
            )
        }
        _ => return None,
    };
    let shown = unescape_unicode_text(&base);
    if shown == base && forced.is_none() {
        return None;
    }
    // 转义换成字符不跨行，两边的行一一对应。按行数决定新写的行用哪种写法，行数相同时写成转义。
    let mut origin = EscapeOrigin {
        latin1,
        ..EscapeOrigin::default()
    };
    let (mut escaped, mut direct) = (0, 0);
    for ((raw, _), (line, _)) in split_lines(&base).zip(split_lines(&shown)) {
        escaped += usize::from(raw != line);
        direct += usize::from(!raw.is_ascii());
        if raw != line || !line.is_ascii() {
            origin
                .lines
                .entry(line.to_owned())
                .or_default()
                .push(raw.to_owned());
        }
    }
    origin.raw = !latin1 && direct > escaped;
    let lowercase = match forced {
        Some(Encoding::UnicodeEscapes { lowercase, .. }) if !base.contains("\\u") => lowercase,
        _ => lowercase_escapes(&base),
    };
    Some((
        shown,
        Encoding::UnicodeEscapes { lowercase, latin1 },
        origin,
    ))
}

/// `text` 中的 `\uXXXX` 转义是否多数用小写的十六进制字母。
fn lowercase_escapes(text: &str) -> bool {
    let (mut lower, mut upper) = (0, 0);
    for (at, _) in text.match_indices("\\u") {
        for c in text[at + 2..].chars().take(4) {
            lower += usize::from(matches!(c, 'a'..='f'));
            upper += usize::from(matches!(c, 'A'..='F'));
        }
    }
    lower > upper
}

/// 同 [`encode`]，按 Unicode 转义保存时按各行原来的写法 `origin` 写：未修改的行按原来的写法
/// 写回，其余的行按文件中多数行的写法写。
///
/// # Errors
///
/// 同 [`encode`]。
pub fn encode_with(
    text: &str,
    encoding: Encoding,
    origin: Option<&EscapeOrigin>,
) -> Result<Vec<u8>, char> {
    let (Encoding::UnicodeEscapes { lowercase, latin1 }, Some(origin)) = (encoding, origin) else {
        return encode(text, encoding);
    };
    if origin.latin1 != latin1 {
        return encode(text, encoding);
    }
    let mut used: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut out = String::with_capacity(text.len());
    for (content, end) in split_lines(text) {
        match origin.lines.get(content) {
            Some(raws) => {
                let n = used.entry(content).or_default();
                out.push_str(&raws[*n % raws.len()]);
                *n += 1;
            }
            None if origin.raw => out.push_str(content),
            None => {
                let escaped = winc_core::transform::escape_unicode_text(content, lowercase, latin1);
                out.push_str(&escaped);
            }
        }
        out.push_str(end);
    }
    Ok(if latin1 {
        // 原样写回的行和转义后的文字都在 U+00FF 以内，一个字符一个字节。
        out.chars().map(|c| u32::from(c) as u8).collect()
    } else {
        out.into_bytes()
    })
}

/// 把 `text` 按 `\r\n`、`\n`、`\r`（`.properties` 识别的换行符）分行，返回各行的内容和换行符。
fn split_lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let text = rest?;
        let Some(at) = text.find(['\r', '\n']) else {
            rest = None;
            return Some((text, ""));
        };
        let end = if text[at..].starts_with("\r\n") { 2 } else { 1 };
        rest = Some(&text[at + end..]);
        Some((&text[..at], &text[at..at + end]))
    })
}

#[cfg(test)]
mod tests;
