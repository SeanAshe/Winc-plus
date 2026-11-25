//! 按块读文件的测试：每块只有 [`CHUNK`]（7）字节，`\r\n`、多字节字符、BOM 和超长行都落在块的
//! 边界上，搜出的与整个文件一次解码、逐行匹配的结果相同。

use std::sync::atomic::AtomicBool;

use super::*;
use crate::matcher::Options;

/// 整个文件一次解码、逐行匹配的结果；大文件按大文件的规则分行（只认 `\n`，去掉行尾的 `\r`，非法
/// 字节替换为 U+FFFD）。
fn expected(matcher: &Matcher, bytes: &[u8]) -> Vec<(usize, std::ops::Range<usize>)> {
    let mut lines = Vec::new();
    if bytes.len() as u64 > LARGE_FILE {
        let body = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);
        let mut parts = body.split(|&b| b == b'\n').peekable();
        while let Some(line) = parts.next() {
            let ended = parts.peek().is_some();
            let line = &line[..visible_len(line, ended, LINE_CAP)];
            lines.push(String::from_utf8_lossy(line).into_owned());
        }
    } else {
        let decoded = decode(bytes);
        let _ = for_each_line(&decoded.text, 0, true, |_, line| {
            lines.push(line.to_owned());
            ControlFlow::Continue(())
        });
    }
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let _ = collect(matcher, index, line, &mut out, usize::MAX);
    }
    out.into_iter().map(|m| (m.line, m.cols)).collect()
}

#[test]
fn reading_in_chunks_finds_what_decoding_the_whole_file_finds() {
    let dir = std::env::temp_dir().join(format!("winc-search-chunks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut samples: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("mixed", "中a\nb中\r\n中\rd中\n\n中中中中中中\r".into()),
        ("bom", "\u{feff}中x\r\n\r\n末尾中".into()),
        (
            "gbk-late",
            [&"中文\n".repeat(3000).into_bytes()[..], b"\xD6\xD0\n"].concat(),
        ),
        (
            "utf16",
            [
                &[0xFF, 0xFE][..],
                &"中\n中"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<u8>>(),
            ]
            .concat(),
        ),
        (
            "long-line",
            format!("{}中\n中", "a".repeat(5000)).into_bytes(),
        ),
        ("crlf-z", "a\r\nz\r\nxz\r\nb\rz\nyz中\r\n\r\nz".into()),
    ];
    let mut large = [
        &b"\xEF\xBB\xBF"[..],
        "中".as_bytes(),
        b"\xFF\r\nz",
        "中".as_bytes(),
        b"\rz\n",
    ]
    .concat();
    let filler = [&[b'a'; 1023][..], b"\n"].concat();
    while large.len() as u64 <= LARGE_FILE {
        large.extend_from_slice(&filler);
    }
    large.extend_from_slice(&[&b"z\r\n"[..], "中\rz\n中".as_bytes(), b"\xC3z\xE4\xB8"].concat());
    samples.push(("large", large));

    let options = Options {
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    // 整块查找候选位置（`block_find`）不能漏掉任何匹配：覆盖行首行尾、整词、`.`、大小写、`\A`、
    // 环视、反向引用。
    let patterns = [
        "z$|中",
        "中.z",
        "^中",
        "^$",
        "中$",
        r"\bb\b",
        "c.d",
        "(?i)D中$",
        r"\A中|x",
        r"(?<=中)b",
        r"(?<!\s)中$",
        r"(中)\1",
        r"\bz(?=中)",
        r"(?i)(?<![a-z])D",
        r"\s*中",
        r"[^a]z",
    ];
    for (name, bytes) in samples {
        let path = dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        // 大文件每块只有几个字节时搜一遍耗时较长，只取前两个模式。
        let count = if name == "large" { 2 } else { patterns.len() };
        for pattern in &patterns[..count] {
            let matcher = Matcher::new(pattern, options).unwrap();
            let stop = AtomicBool::new(false);
            let (found, _) = search_file(&matcher, &mut Vec::new(), &path, usize::MAX, &stop)
                .unwrap_or_else(|| panic!("{name} 没有搜索"));
            let found: Vec<_> = found.into_iter().map(|m| (m.line, m.cols)).collect();
            assert_eq!(found, expected(&matcher, &bytes), "{name} {pattern}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
