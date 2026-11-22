//! `.properties` 按 Unicode 转义读写的测试：读入时显示字符，保存时写回转义且不改变含义。

use super::*;

/// `.properties` 中的 `\uXXXX` 转义读入时换成字符，编辑后保存写回转义，未修改的行逐字节不变。
#[test]
fn properties_files_show_unicode_escapes_as_characters() {
    let dir = scratch("unicode-escapes");
    let path = dir.join("messages.properties");
    fs::write(
        &path,
        "# \\u6CE8\\u91CA\nhi=\\u4F60\\u597D\nemoji=\\uD83D\\uDE00\n",
    )
    .unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(
        doc.encoding,
        Encoding::UnicodeEscapes {
            lowercase: false,
            latin1: false,
        }
    );
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "# 注释\nhi=你好\nemoji=😀\n"
    );
    let end = doc.text.end_pos();
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(end),
        "bye=再见\n",
        winc_core::EditKind::Other,
    ));
    doc.save().unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# \\u6CE8\\u91CA\nhi=\\u4F60\\u597D\nemoji=\\uD83D\\uDE00\nbye=\\u518D\\u89C1\n"
    );

    // 其他程序改了文件：重新读入后仍显示字符。
    fs::write(&path, "hi=\\u55E8\n").unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    let file = fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(later).unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "hi=嗨\n");

    // 用户改选 UTF-8：显示原文的转义，之后重读也不再转换。
    doc.reopen_with_encoding(Encoding::Utf8).unwrap();
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "hi=\\u55E8\n");
    doc.reopen_with_encoding(Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: false,
    })
    .unwrap();
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "hi=嗨\n");
    fs::remove_dir_all(&dir).unwrap();
}

/// `.properties` 中有转义才转换：小写的转义按小写写回；没有转义，或不是 `.properties` 的文件按
/// 原文显示。
#[test]
fn only_properties_files_with_escapes_convert() {
    let dir = scratch("unicode-escapes-kept");
    let open = |name: &str, text: &str| {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        Document::open(DocumentId(1), path).unwrap()
    };
    let lower = open("lower.properties", "a=\\u4f60\n");
    assert_eq!(
        lower.encoding,
        Encoding::UnicodeEscapes {
            lowercase: true,
            latin1: false,
        }
    );
    assert_eq!(lower.text_range(0, usize::MAX).unwrap(), "a=你\n");
    for (name, text) in [
        ("plain.properties", "a=b 好\n"),
        ("code.java", "s=\"\\u4F60\";\n"),
    ] {
        let doc = open(name, text);
        assert_eq!(doc.encoding, Encoding::Utf8, "{name}");
        assert_eq!(doc.text_range(0, usize::MAX).unwrap(), text, "{name}");
    }
    // Git 中的旧版本按文档的编码解码，与编辑区中的文字一致。
    let escapes = Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: false,
    };
    assert_eq!(
        crate::encoding::decode_known(b"a=\\u4F60\n", escapes),
        "a=你\n"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// 直接写的字符和转义混用的文件也按字符显示；保存时未修改的行按原来的写法写回，修改和新增的行按
/// 文件中多数行的写法写。
#[test]
fn mixed_properties_keep_untouched_lines_as_written() {
    let dir = scratch("unicode-escapes-mixed");
    let save = |name: &str, text: &str, added: &str| {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
        let shown = doc.text_range(0, usize::MAX).unwrap();
        let end = doc.text.end_pos();
        doc.apply(winc_core::edit::replace_selection(
            doc.text.rope().unwrap(),
            Selection::point(end),
            added,
            winc_core::EditKind::Other,
        ));
        doc.save().unwrap();
        (shown, fs::read_to_string(&path).unwrap())
    };
    // 多数是转义：新增的行写成转义，直接写的「简体中文」原样保留；ASCII 字符的转义不换。
    let escaped = "a=\\u4F60\\u597D\nb=\\u0041\nname=简体中文\nemoji=\\uD83D\\uDE00\n";
    let (shown, written) = save("escaped.properties", escaped, "c=再见\n");
    assert_eq!(shown, "a=你好\nb=\\u0041\nname=简体中文\nemoji=😀\n");
    assert_eq!(written, format!("{escaped}c=\\u518D\\u89C1\n"));
    // 多数直接写：新增的行也直接写。
    let direct = "a=你好\nb=世界\nc=\\u4F60\n";
    let (shown, written) = save("direct.properties", direct, "c=再见\n");
    assert_eq!(shown, "a=你好\nb=世界\nc=你\n");
    assert_eq!(written, format!("{direct}c=再见\n"));
    fs::remove_dir_all(&dir).unwrap();
}

/// ISO-8859-1 的 `.properties`：西欧字母直接写成一个字节，其他字符写成转义，保存时按原来的写法
/// 写回。落单的反斜杠后面的字符是字符本身，写成转义时去掉反斜杠，含义不变。
#[test]
fn latin1_properties_and_backslashes_round_trip() {
    let dir = scratch("unicode-escapes-latin1");
    let path = dir.join("de.properties");
    fs::write(&path, b"gr\xFC\xDF=\\u4F60\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    let latin1 = Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: true,
    };
    assert_eq!(doc.encoding, latin1);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "grüß=你\n");
    let end = doc.text.end_pos();
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(end),
        "x=\\好\n",
        winc_core::EditKind::Other,
    ));
    doc.save().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"gr\xFC\xDF=\\u4F60\nx=\\u597D\n");
    let doc = Document::open(DocumentId(2), path).unwrap();
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "grüß=你\nx=好\n");
    fs::remove_dir_all(&dir).unwrap();
}

/// 「通过编码保存 → Unicode 转义」把直接写的字符写成转义；编码的名字写进备份后能还原。
#[test]
fn saving_with_unicode_escapes_writes_escapes() {
    let dir = scratch("unicode-escapes-save");
    let path = dir.join("zh.properties");
    fs::write(&path, "hi=你好\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Utf8);
    let escapes = Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: false,
    };
    assert!(doc.set_save_encoding(escapes));
    doc.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "hi=\\u4F60\\u597D\n");
    for lowercase in [false, true] {
        for latin1 in [false, true] {
            let encoding = Encoding::UnicodeEscapes { lowercase, latin1 };
            assert_eq!(Encoding::from_key(encoding.key().unwrap()), Some(encoding));
        }
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// 多个光标中有的选了文字、有的没有：只转换选中的部分。
#[test]
fn unicode_commands_convert_only_selections_when_some_cursors_select() {
    let mut doc = Document::from_text(DocumentId(1), "a=\\u4F60\nb=\\u597D\n");
    doc.set_selections(vec![Selection::new(0, 8), Selection::point(12)], 0);
    doc.execute(Command::UnescapeUnicode { whole: false }, 10, 4);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "a=你\nb=\\u597D\n");
}

/// 保存时不改变 `.properties` 的含义：ASCII 字符的转义（`\u0020` 是键中的空格）不换成字符；只用
/// `\r` 换行的文件逐行保留原来的写法；ISO-8859-1 的文件改选「Unicode 转义」保存仍按它存。
#[test]
fn unicode_escapes_keep_the_meaning_of_properties() {
    let dir = scratch("unicode-escapes-meaning");
    let edit = |name: &str, bytes: &[u8], at: usize, added: &str, choose: bool| {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
        let shown = doc.text_range(0, usize::MAX).unwrap();
        if choose {
            assert!(doc.set_save_encoding(Encoding::UnicodeEscapes {
                lowercase: false,
                latin1: false,
            }));
        }
        let at = at.min(doc.text.end_pos());
        doc.apply(winc_core::edit::replace_selection(
            doc.text.rope().unwrap(),
            Selection::point(at),
            added,
            winc_core::EditKind::Other,
        ));
        doc.save().unwrap();
        (shown, doc.encoding, fs::read(&path).unwrap())
    };
    let utf8 = Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: false,
    };
    let (shown, _, written) = edit(
        "ascii.properties",
        b"my\\u0020key=\\u4F60\\u003D\n",
        0,
        "x",
        false,
    );
    assert_eq!(shown, "my\\u0020key=你\\u003D\n");
    assert_eq!(written, b"xmy\\u0020key=\\u4F60\\u003D\n");

    let (shown, encoding, written) = edit(
        "cr.properties",
        "a=你\rb=\\u597D\rc=世\r".as_bytes(),
        usize::MAX,
        "d=家\r",
        false,
    );
    assert_eq!((shown.as_str(), encoding), ("a=你\rb=好\rc=世\r", utf8));
    assert_eq!(written, "a=你\rb=\\u597D\rc=世\rd=家\r".as_bytes());

    let (_, encoding, written) = edit(
        "latin1.properties",
        b"gr\xFC\xDF=\\u4F60\n",
        usize::MAX,
        "x=\u{e9}\n",
        true,
    );
    let latin1 = Encoding::UnicodeEscapes {
        lowercase: false,
        latin1: true,
    };
    assert_eq!(encoding, latin1);
    assert_eq!(written, b"gr\xFC\xDF=\\u4F60\nx=\xE9\n");

    // 不是合法 UTF-8 的文件改按「Unicode 转义」重新打开时按 ISO-8859-1 读，不变成只读。
    let path = dir.join("plain.properties");
    fs::write(&path, b"x=\xE9\n").unwrap();
    let mut doc = Document::open(DocumentId(2), path).unwrap();
    doc.reopen_with_encoding(utf8).unwrap();
    assert_eq!(doc.encoding, latin1);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "x=é\n");
    fs::remove_dir_all(&dir).unwrap();
}
