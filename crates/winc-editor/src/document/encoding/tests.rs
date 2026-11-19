//! 编码识别与解码的测试：BOM、不带 BOM 的 UTF-16、合法与损坏的 UTF-8、按地区猜测旧编码、无法
//! 表示的字符。

use super::*;

#[test]
fn utf8_and_boms_are_recognized() {
    assert_eq!(decode(b"abc").encoding, Encoding::Utf8);
    assert!(matches!(decode(b"abc").text, Cow::Borrowed(_)));
    let bom = decode(b"\xEF\xBB\xBFabc");
    assert_eq!(
        (bom.encoding, bom.text.as_ref()),
        (Encoding::Utf8Bom, "abc")
    );

    let text = "中文 ok\n";
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = encode(text, encoding).unwrap();
        assert!(utf16_bom(&bytes));
        let decoded = decode(&bytes);
        assert_eq!((decoded.encoding, decoded.text.as_ref()), (encoding, text));
    }
    // 不完整的 UTF-16：无法无损解码。
    assert!(decode(b"\xFF\xFEa").lossy());
}

/// 不带 BOM、以 ASCII 为主的 UTF-16 按 NUL 的位置认出字节序；含其他字符、NUL 位置不齐的不认。
#[test]
fn utf16_without_a_bom_is_recognized_by_its_nuls() {
    let unmarked = |text: &str, encoding| encode(text, encoding).unwrap()[2..].to_vec();
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = unmarked("plain ok\n", encoding);
        assert_eq!(utf16_unmarked(&bytes), Some(encoding));
        assert!(utf16_text(&bytes) && !crate::looks_binary(&bytes));
        let decoded = decode(&bytes);
        assert_eq!(
            (decoded.encoding, decoded.text.as_ref()),
            (encoding, "plain ok\n")
        );
    }
    assert_eq!(
        utf16_unmarked(&unmarked("中文 ok", Encoding::Utf16Le)),
        None
    );
    assert_eq!(utf16_unmarked(b"abc"), None);
    assert_eq!(utf16_unmarked(&[1, 0, 2]), None);
    assert_eq!(utf16_unmarked(b"MZ\x90\x00\x03\x00\x00\x00"), None);
    assert!(crate::looks_binary(b"MZ\x90\x00\x03\x00\x00\x00"));
}

#[test]
fn legacy_encodings_are_guessed_and_written_back() {
    let samples = [
        (
            "你好，世界。这是一段用来猜编码的中文，里面有足够多的汉字。\n",
            encoding_rs::GBK,
        ),
        (
            "這是一段繁體中文的文字，用來測試編碼的猜測是否正確。\n",
            encoding_rs::BIG5,
        ),
        (
            "これは日本語の文章です。エンコーディングを推測します。\n",
            encoding_rs::SHIFT_JIS,
        ),
        (
            "Ceci est un texte français avec des accents : é, è, à, ç.\n",
            encoding_rs::WINDOWS_1252,
        ),
    ];
    for (text, codec) in samples {
        let (bytes, _, errors) = codec.encode(text);
        assert!(!errors);
        let decoded = decode(&bytes);
        let Encoding::Legacy(guess) = decoded.encoding else {
            panic!("{text} 没认出来：{:?}", decoded.encoding);
        };
        assert_eq!(decoded.text, text, "{}", guess.name());
        // 写回后与原字节完全一致。
        assert_eq!(
            encode(&decoded.text, decoded.encoding).unwrap(),
            bytes.as_ref()
        );
    }
    assert_eq!(Encoding::Legacy(encoding_rs::GBK).label(), "GBK");
    assert_eq!(Encoding::Legacy(encoding_rs::BIG5).label(), "Big5");
    assert_eq!(
        Encoding::Legacy(encoding_rs::WINDOWS_1252).label(),
        "Windows 1252"
    );
}

#[test]
fn short_text_is_guessed_by_the_users_region() {
    // 「你好」两个字的 GBK 编码：不知道地区时难以区分，简体中文用户则判为 GBK。
    let bytes = b"needle \xC4\xE3\xBA\xC3\n";
    let guessed = decode_in(bytes, Some("cn"));
    // 猜成 GBK 时记为 GB18030：解码结果相同，编码时多出的四字节形式也能写回。
    assert_eq!(guessed.encoding, Encoding::Legacy(encoding_rs::GB18030));
    assert_eq!(guessed.text, "needle 你好\n");
    assert_ne!(
        decode_in(bytes, None).encoding,
        Encoding::Legacy(encoding_rs::GBK)
    );
    // 地区不影响西欧文字：没有带地区的提示时同样判为 Windows 1252。
    assert_eq!(
        decode_in(b"caf\xe9\n", Some("cn")).encoding,
        Encoding::Legacy(encoding_rs::WINDOWS_1252)
    );
}

#[test]
fn regions_come_from_the_system_language() {
    for (language, region) in [
        ("zh-Hans-CN", Some("cn")),
        ("zh-Hans", Some("cn")),
        ("zh-Hant", Some("tw")),
        ("zh-Hant-HK", Some("hk")),
        ("zh_TW.UTF-8", Some("tw")),
        ("ja", Some("jp")),
        ("ko-KR", Some("kr")),
        ("en-US", Some("us")),
        ("en", None),
        ("C", None),
        ("", None),
    ] {
        assert_eq!(region_of(language).as_deref(), region, "{language}");
    }
}

#[test]
fn damaged_utf8_stays_utf8() {
    // 中文 UTF-8 文本中间有一个损坏的字节：仍按 UTF-8 处理，损坏的字节显示为 U+FFFD，
    // 不猜成其他编码。
    let mut bytes = "这是一段中文，中间坏了一个字节，其余都是正常的 UTF-8。"
        .as_bytes()
        .to_vec();
    bytes.insert(12, 0xFF);
    let decoded = decode(&bytes);
    assert!(decoded.lossy());
    assert!(decoded.text.starts_with("这是一段\u{fffd}中文"));
    // 只有一个非 ASCII 字节、没有合法的多字节字符：判为单字节编码。
    assert!(!decode(b"caf\xe9\n").lossy());
}

#[test]
fn characters_the_encoding_cannot_hold_are_reported() {
    let gbk = Encoding::Legacy(encoding_rs::GBK);
    assert_eq!(encode("中文 😀 x", gbk), Err('😀'));
    assert_eq!(
        encode("é", Encoding::Legacy(encoding_rs::SHIFT_JIS)),
        Err('é')
    );
    // GB18030 能编码任意 Unicode 字符。
    assert!(encode("😀", Encoding::Legacy(encoding_rs::GB18030)).is_ok());
}

#[test]
fn decoding_as_a_chosen_encoding() {
    let (bytes, _, _) = encoding_rs::GBK.encode("中文");
    assert_eq!(
        decode_as(&bytes, Encoding::Legacy(encoding_rs::GBK)),
        ("中文".to_owned(), false)
    );
    // 按 UTF-8 解码：有无法解码的字节。
    assert!(decode_as(&bytes, Encoding::Utf8).1);
    // 选了 UTF-8 BOM 时去掉 BOM。
    assert_eq!(decode_as(b"\xEF\xBB\xBFx", Encoding::Utf8Bom).0, "x");
}
