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

#[test]
fn names_carried_between_hosts_fit_each_side() {
    let down = |segment| carry(segment, true, false);
    // 远端的原名在本机能用就原样还原。
    assert_eq!(down("50%25.txt").as_deref(), Some("50%.txt"));
    assert_eq!(down("报告.md").as_deref(), Some("报告.md"));
    // 本机不允许的字符、结尾的点和空格、设备名、不是 UTF-8 的字节写成 `%XX`。
    assert_eq!(down("a%3Ab").as_deref(), Some("a%3Ab"));
    assert_eq!(down("a%5Cb").as_deref(), Some("a%5Cb"));
    assert_eq!(down("notes?.md").as_deref(), Some("notes%3F.md"));
    assert_eq!(down("a<b>|c\"d*").as_deref(), Some("a%3Cb%3E%7Cc%22d%2A"));
    assert_eq!(down("log. ").as_deref(), Some("log%2E%20"));
    assert_eq!(down("con.txt").as_deref(), Some("%63on.txt"));
    assert_eq!(down("Lpt9").as_deref(), Some("%4Cpt9"));
    assert_eq!(down("console.txt").as_deref(), Some("console.txt"));
    assert_eq!(down("caf%E9.txt").as_deref(), Some("caf%E9.txt"));
    assert_eq!(down("bad%zz"), None);
    // 本机的名字到远端时转义；两侧相同时不换算。
    assert_eq!(carry("50%.txt", false, true).as_deref(), Some("50%25.txt"));
    assert_eq!(carry("50%25.txt", true, true), None);
    assert_eq!(carry("50%.txt", false, false), None);
}
