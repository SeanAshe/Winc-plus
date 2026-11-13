//! 远端文件名与 Windows 路径分段之间的可逆换算。
//!
//! 远端的文件名是任意字节（不含 `/` 和 NUL），Windows 的路径分段是 Unicode 文本且以 `\` 分隔。
//! 换算与平台无关，只在 Windows 上使用。

#[cfg(test)]
mod tests;

/// 把远端文件名 `name` 换算为 Windows 路径的一段。
///
/// `\`、`:`、`%` 和不构成有效 UTF-8 的字节写成 `%XX`（两位大写十六进制），其余字符原样保留；
/// [`decode`] 还原出相同的字节。`:` 也要转义：以「字母加 `:`」开头的一段会被当作盘符，接到
/// 路径之后时取代整个路径。
pub(crate) fn encode(name: &[u8]) -> String {
    let mut out = String::with_capacity(name.len());
    for chunk in name.utf8_chunks() {
        for c in chunk.valid().chars() {
            if matches!(c, '\\' | ':' | '%') {
                push_escaped(&mut out, c as u8);
            } else {
                out.push(c);
            }
        }
        for &byte in chunk.invalid() {
            push_escaped(&mut out, byte);
        }
    }
    out
}

/// 把 [`encode`] 的结果 `segment` 还原为远端文件名；`%` 之后不是两位十六进制数字时返回 `None`。
pub(crate) fn decode(segment: &str) -> Option<Vec<u8>> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // `from_str_radix` 接受正负号，先确认两位都是十六进制数字。
            let digits = segment.get(i + 1..i + 3)?;
            if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(digits, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

fn push_escaped(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 0x0F)]));
}
