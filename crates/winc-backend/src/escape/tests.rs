//! 远端文件名与 Windows 路径分段的换算：可逆，且结果不含分隔符。

use super::*;

#[test]
fn ordinary_names_are_kept_and_special_bytes_are_escaped() {
    assert_eq!(encode("报告 v2.md".as_bytes()), "报告 v2.md");
    assert_eq!(encode(br"a\b"), "a%5Cb");
    assert_eq!(encode(b"100%"), "100%25");
    assert_eq!(encode(b"a:b"), "a%3Ab");
    // 不构成有效 UTF-8 的字节逐个转义，两侧的有效文字保留。
    assert_eq!(encode(b"caf\xE9.txt"), "caf%E9.txt");
}

#[test]
fn every_name_survives_a_round_trip() {
    let names: [&[u8]; 7] = [
        b"plain.rs",
        br"back\slash",
        b"%41 is not A",
        b"\xFF\xFE\x80",
        "混合\u{1F600}".as_bytes(),
        b"trailing%",
        b"c:drive",
    ];
    for name in names {
        let segment = encode(name);
        assert!(!segment.contains(['\\', ':']), "{segment}");
        assert_eq!(decode(&segment).as_deref(), Some(name), "{segment}");
    }
}

#[test]
fn malformed_escapes_are_rejected() {
    for segment in ["%", "%4", "%ZZ", "a%+1"] {
        assert_eq!(decode(segment), None, "{segment}");
    }
}
