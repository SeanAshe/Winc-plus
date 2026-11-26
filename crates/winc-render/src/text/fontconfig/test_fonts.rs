//! 测试用的隔离 fontconfig 环境：临时目录里的字体文件和配置，以及让查询使用它的入口。
//!
//! fontconfig 的配置在进程里第一次初始化时确定，之后不能替换。用到私有配置的测试因此在子进程
//! 里运行自己：父进程写入临时配置，用 `FONTCONFIG_FILE` 启动同一个测试，再按子进程的结果判断
//! 成败；子进程里 [`Isolated::new`] 直接使用父进程建好的目录，测试本体照常执行。字体由测试代码
//! 按 sfnt 格式现造，只含最少的名字表、OS/2 和字形表，不依赖系统里装了什么字体。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// 父进程传给子进程的临时目录。
const DIR_ENV: &str = "WINC_TEST_FONTCONFIG_DIR";
/// 没有字体的配置：父进程传给子进程的临时目录。
const EMPTY_ENV: &str = "WINC_TEST_EMPTY_FONTCONFIG_DIR";

/// 测试字体中的字族名。
pub(crate) const DEFAULT: &str = "Winc Default";
pub(crate) const TARGET: &str = "Winc Target";
pub(crate) const STYLE: &str = "Winc Style";
pub(crate) const ITALIC: &str = "Winc Italic";
pub(crate) const LITERAL: &str = "Winc Literal:Test,Back\\slash=A";
pub(crate) const OPTION: &str = "-Winc Option";
/// 被 fontconfig 配置排除的字体。
pub(crate) const EXCLUDED: &str = "Winc Excluded";
/// 一个 `.ttc` 集合里的字族，集合中有常规体和粗体两个字体。
pub(crate) const COLLECTION: &str = "Winc Collection";

/// `monospace` 回退顺序中的字体：配置依次列出 [`CHAIN_FIRST`]、[`CHAIN_REPEAT`]、[`CHAIN_NEXT`]。
pub(crate) const CHAIN_FIRST: &str = "Winc Chain First";
/// 字符都已由 [`CHAIN_FIRST`] 覆盖的字体：回退顺序中略去。
pub(crate) const CHAIN_REPEAT: &str = "Winc Chain Repeat";
/// 含有 [`CHAIN_FIRST`] 所缺字符 [`CHAIN_SYMBOL`] 的字体。
pub(crate) const CHAIN_NEXT: &str = "Winc Chain Next";
/// 只有 [`CHAIN_NEXT`] 含有的字符（Nerd Font 图标所在的私用区）。
pub(crate) const CHAIN_SYMBOL: char = '\u{e62b}';

/// 指向未知名字的后备字体的别名：它的目标字体和任意替换得到的字体是同一个。
pub(crate) const ALIAS: &str = "Winc Alias";
/// 指向另一个字族的别名。
pub(crate) const OTHER_ALIAS: &str = "Winc Other Alias";
/// 指向没有安装的字族的别名：不能解析成任意后备字体。
pub(crate) const MISSING_ALIAS: &str = "Winc Missing Alias";
/// 别名 [`MISSING_ALIAS`] 指向的字族，测试环境里没有这个字体。
pub(crate) const MISSING_TARGET: &str = "Winc Uninstalled Target";

/// 一个隔离的 fontconfig 环境，丢弃时删除临时目录。
pub(crate) struct Isolated {
    dir: PathBuf,
    /// 未知名字在配置里得到的后备字体。
    fallback: PathBuf,
}

impl Isolated {
    /// 父进程创建临时目录、字体和配置，并在子进程里重跑当前测试；子进程返回可用的环境。
    ///
    /// 父进程在子进程成功后返回 `None`，当前测试本体跳过；子进程里读 [`DIR_ENV`]，返回 `Some`
    /// 并让测试本体执行。配置把 [`STYLE`] 追加为所有模式的后备字族，因此未知名字的后备字体
    /// 就是 [`STYLE`] 的常规体，不必先查询一遍才能写别名。
    pub(crate) fn new() -> Option<Isolated> {
        let dir = isolated_dir(DIR_ENV, |dir| {
            let fonts = dir.join("fonts");
            std::fs::create_dir_all(&fonts).unwrap();
            std::fs::create_dir_all(dir.join("cache")).unwrap();
            write_fonts(&fonts);
            let config = dir.join("fonts.conf");
            write_config(
                &config,
                dir,
                &[
                    (ALIAS, STYLE),
                    (OTHER_ALIAS, DEFAULT),
                    (MISSING_ALIAS, MISSING_TARGET),
                ],
            )
            .unwrap();
            config
        })?;
        let fallback = dir.join("fonts").join(file_name(STYLE, "Regular"));
        Some(Isolated { dir, fallback })
    }

    /// 返回别名 [`ALIAS`] 指向的字族。
    pub(crate) fn alias(&self) -> &'static str {
        STYLE
    }

    /// 返回别名 [`OTHER_ALIAS`] 指向的字族。
    pub(crate) fn other_alias(&self) -> &'static str {
        DEFAULT
    }

    /// 返回未知名字在 fontconfig 中得到的后备字体。
    pub(crate) fn fallback(&self) -> &Path {
        &self.fallback
    }

    /// 返回字族 `family` 的常规体字体文件。
    pub(crate) fn regular(&self, family: &str) -> PathBuf {
        if family == COLLECTION {
            self.dir.join("fonts").join("collection.ttc")
        } else {
            self.dir.join("fonts").join(file_name(family, "Regular"))
        }
    }

    /// 返回字族 `family` 的粗体字体文件。
    pub(crate) fn bold(&self, family: &str) -> PathBuf {
        self.dir.join("fonts").join(file_name(family, "Bold"))
    }

    /// 返回字族 `family` 的斜体字体文件。
    pub(crate) fn italic(&self, family: &str) -> PathBuf {
        self.dir.join("fonts").join(file_name(family, "Italic"))
    }

    /// 返回被配置排除的字体文件。
    pub(crate) fn excluded(&self) -> PathBuf {
        self.dir
            .join("fonts")
            .join("excluded")
            .join(file_name(EXCLUDED, "Regular"))
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 没有字体的隔离配置，用来检查 fontconfig 返回空字体集合时的路径。
pub(crate) struct EmptyConfig;

impl EmptyConfig {
    /// 在子进程里返回空配置的标记；父进程和 [`Isolated`] 一样启动子进程后返回 `None`。
    pub(crate) fn new() -> Option<EmptyConfig> {
        isolated_dir(EMPTY_ENV, |dir| {
            std::fs::create_dir_all(dir.join("fonts")).unwrap();
            std::fs::create_dir_all(dir.join("cache")).unwrap();
            let config = dir.join("fonts.conf");
            write_empty_config(&config, dir).unwrap();
            config
        })?;
        Some(EmptyConfig)
    }
}

/// 父进程准备配置并在子进程里重跑当前测试；子进程里读 `dir_env`，返回临时目录。
///
/// 父进程在子进程成功后返回 `None`；没有 libfontconfig 时也返回 `None`，测试直接跳过。
fn isolated_dir(dir_env: &str, setup: impl FnOnce(&Path) -> PathBuf) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(dir_env) {
        return Some(PathBuf::from(dir));
    }
    // 没有 libfontconfig 的系统上测试无事可做；在父进程里先判断，子进程里不必再区分。
    if !super::available() {
        return None;
    }

    let dir = temp_dir();
    let config = setup(&dir);
    // 测试线程的名字就是测试的完整路径，用它让子进程只重跑当前测试。
    let test = std::thread::current()
        .name()
        .expect("测试线程有名字")
        .to_owned();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &test, "--quiet"])
        .env(dir_env, &dir)
        .env("FONTCONFIG_FILE", &config)
        .status()
        .expect("启动隔离的 fontconfig 测试进程");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "隔离的 fontconfig 测试失败：{test}");
    None
}

/// 返回一个本次测试专用的临时目录路径，并清除上次运行留下的同名目录。
fn temp_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("winc-fontconfig-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 在 `fonts` 目录里写入全部测试字体。
fn write_fonts(fonts: &Path) {
    for (family, subfamily, weight, italic) in [
        (DEFAULT, "Regular", 400, false),
        (TARGET, "Regular", 400, false),
        (TARGET, "Bold", 700, false),
        (STYLE, "Regular", 400, false),
        (ITALIC, "Regular", 400, false),
        (ITALIC, "Italic", 400, true),
        (LITERAL, "Regular", 400, false),
        (OPTION, "Regular", 400, false),
    ] {
        std::fs::write(
            fonts.join(file_name(family, subfamily)),
            font(family, subfamily, weight, italic),
        )
        .unwrap();
    }
    for (family, chars) in [
        (CHAIN_FIRST, &['a', 'b'][..]),
        (CHAIN_REPEAT, &['a'][..]),
        (CHAIN_NEXT, &['a', CHAIN_SYMBOL][..]),
    ] {
        std::fs::write(
            fonts.join(file_name(family, "Regular")),
            font_covering(family, "Regular", 400, false, chars),
        )
        .unwrap();
    }
    let excluded = fonts.join("excluded");
    std::fs::create_dir_all(&excluded).unwrap();
    std::fs::write(
        excluded.join(file_name(EXCLUDED, "Regular")),
        font(EXCLUDED, "Regular", 400, false),
    )
    .unwrap();
    std::fs::write(
        fonts.join("collection.ttc"),
        collection(&[
            (COLLECTION, "Regular", 400, false),
            (COLLECTION, "Bold", 700, false),
        ]),
    )
    .unwrap();
}

/// 写入只包含测试字体目录、缓存目录、通用后备字族和别名的配置。
fn write_config(path: &Path, dir: &Path, aliases: &[(&str, &str)]) -> std::io::Result<()> {
    let fonts = xml(&dir.join("fonts").display().to_string());
    let cache = xml(&dir.join("cache").display().to_string());
    let excluded = xml(&dir.join("fonts/excluded/*").display().to_string());
    let mut rules = String::new();
    for (name, target) in aliases {
        rules.push_str(&format!(
            r#"  <alias binding="same">
    <family>{}</family>
    <prefer><family>{}</family></prefer>
  </alias>
"#,
            xml(name),
            xml(target)
        ));
    }
    // 用户常见的写法：`monospace` 依次列出几个字体，后面的字体补上前面的字体所缺的字符。
    rules.push_str(&format!(
        r#"  <alias>
    <family>monospace</family>
    <prefer><family>{}</family><family>{}</family><family>{}</family></prefer>
  </alias>
"#,
        xml(CHAIN_FIRST),
        xml(CHAIN_REPEAT),
        xml(CHAIN_NEXT)
    ));
    // 真实的 fontconfig 配置会给每个模式追加通用的后备字族；别名要和这些替换区分开。
    let fallback = xml(STYLE);
    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>{fonts}</dir>
  <cachedir>{cache}</cachedir>
  <selectfont>
    <rejectfont><glob>{excluded}</glob></rejectfont>
  </selectfont>
{rules}  <match target="pattern">
    <edit name="family" mode="append" binding="weak"><string>{fallback}</string></edit>
  </match>
</fontconfig>
"#
        ),
    )
}

/// 写入一个没有任何字体目录的配置：`FcFontList` 会返回空的字体集合。
fn write_empty_config(path: &Path, dir: &Path) -> std::io::Result<()> {
    let fonts = xml(&dir.join("fonts").display().to_string());
    let cache = xml(&dir.join("cache").display().to_string());
    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>{fonts}</dir>
  <cachedir>{cache}</cachedir>
</fontconfig>
"#
        ),
    )
}

/// 转义 XML 文本中的特殊字符。
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 返回字族 `family`、变体 `subfamily` 的字体文件名。
fn file_name(family: &str, subfamily: &str) -> String {
    let family: String = family
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{family}-{subfamily}.ttf")
}

/// 把一个最小的常规体字体写到 `path`；测试用。
pub(crate) fn write_font(path: &Path, family: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, font(family, "Regular", 400, false)).unwrap();
}

/// 返回一个最小的 TrueType 字体：一个字族、一个字重，没有可显示的字符。
fn font(family: &str, subfamily: &str, weight: u16, italic: bool) -> Vec<u8> {
    font_covering(family, subfamily, weight, italic, &[])
}

/// 同 [`font`]，字体含有字符 `chars`：每个字符对应一个空白字形，fontconfig 的字符集合和字体的
/// 字符映射表都包含这些字符。
fn font_covering(
    family: &str,
    subfamily: &str,
    weight: u16,
    italic: bool,
    chars: &[char],
) -> Vec<u8> {
    let bold = weight >= 600;
    // `.notdef` 之后每个字符一个字形。
    let glyphs = chars.len() as u16 + 1;
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2(weight, bold, italic)),
        (b"cmap", cmap(chars)),
        (b"glyf", Vec::new()),
        (b"head", head(bold, italic)),
        (b"hhea", hhea(glyphs)),
        (b"hmtx", hmtx(glyphs)),
        (b"loca", loca(glyphs)),
        (b"maxp", maxp(glyphs)),
        (b"name", name(family, subfamily)),
        (b"post", post()),
    ];
    tables.sort_by_key(|(tag, _)| *tag);

    let count = tables.len() as u16;
    let mut power = 1u16;
    while power * 2 <= count {
        power *= 2;
    }
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, count);
    be16(&mut out, power * 16);
    be16(&mut out, power.trailing_zeros() as u16);
    be16(&mut out, count * 16 - power * 16);

    let mut offset = 12 + 16 * u32::from(count);
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset);
        be32(&mut out, data.len() as u32);
        body.extend_from_slice(data);
        let padded = (data.len() + 3) & !3;
        body.resize(body.len() + padded - data.len(), 0);
        offset += padded as u32;
    }
    out.extend_from_slice(&body);
    out
}

/// 返回一个最小的 TrueType 集合（`.ttc`），每个元素是一个字体。
pub(crate) fn collection(faces: &[(&str, &str, u16, bool)]) -> Vec<u8> {
    let mut fonts: Vec<Vec<u8>> = faces
        .iter()
        .map(|(family, subfamily, weight, italic)| font(family, subfamily, *weight, *italic))
        .collect();
    let header = 12 + 4 * fonts.len() as u32;
    let mut offsets = Vec::new();
    let mut start = header;
    for font in &fonts {
        offsets.push(start);
        start += font.len() as u32;
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    be32(&mut out, 0x0001_0000);
    be32(&mut out, fonts.len() as u32);
    for offset in &offsets {
        be32(&mut out, *offset);
    }
    for (font, start) in fonts.iter_mut().zip(offsets) {
        // 集合中的表偏移从整个文件开头算起，不是从各字体自己的目录开头算起。
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        for index in 0..count {
            let at = 12 + 16 * index + 8;
            let offset = u32::from_be_bytes(font[at..at + 4].try_into().unwrap());
            font[at..at + 4].copy_from_slice(&(start + offset).to_be_bytes());
        }
        out.extend_from_slice(font);
    }
    out
}

/// 返回表数据的校验和：按 4 字节大端字求和，末尾不足 4 字节时补零。
fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// `head` 表：字体整体参数；字重和斜体只影响 `macStyle`，fontconfig 主要读 `OS/2`。
fn head(bold: bool, italic: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(54);
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0);
    be32(&mut out, 0x5F0F_3CF5);
    be16(&mut out, 0x0003);
    be16(&mut out, 1000);
    out.extend_from_slice(&[0; 16]);
    for value in [0i16, 0, 1000, 1000] {
        be16(&mut out, value as u16);
    }
    be16(&mut out, u16::from(bold) | u16::from(italic) << 1);
    be16(&mut out, 8);
    be16(&mut out, 2);
    be16(&mut out, 0);
    be16(&mut out, 0);
    assert_eq!(out.len(), 54);
    out
}

/// `hhea` 表：水平排版的整体参数，`glyphs` 个字形各有前进宽度。
fn hhea(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    be32(&mut out, 0x0001_0000);
    for value in [800i16, -200, 0] {
        be16(&mut out, value as u16);
    }
    be16(&mut out, 500);
    for value in [0i16, 0, 500] {
        be16(&mut out, value as u16);
    }
    be16(&mut out, 1);
    be16(&mut out, 0);
    be16(&mut out, 0);
    out.extend_from_slice(&[0; 8]);
    be16(&mut out, 0);
    be16(&mut out, glyphs);
    assert_eq!(out.len(), 36);
    out
}

/// `maxp` 表：`glyphs` 个字形（第一个是 `.notdef`）。
fn maxp(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    be32(&mut out, 0x0001_0000);
    be16(&mut out, glyphs);
    out.extend_from_slice(&[0; 26]);
    assert_eq!(out.len(), 32);
    out
}

/// `hmtx` 表：`glyphs` 个字形的前进宽度。
fn hmtx(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * usize::from(glyphs));
    for _ in 0..glyphs {
        be16(&mut out, 500);
        be16(&mut out, 0);
    }
    out
}

/// `loca` 表：`glyphs` 个空字形的偏移（短格式）。
fn loca(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 * usize::from(glyphs) + 2);
    for _ in 0..=glyphs {
        be16(&mut out, 0);
    }
    out
}

/// `cmap` 表：一个 Windows Unicode BMP 编码的格式 4 子表，把 `chars` 依次映射到第 1 个起的字形。
fn cmap(chars: &[char]) -> Vec<u8> {
    let mut chars: Vec<u16> = chars.iter().map(|&c| c as u16).collect();
    chars.sort_unstable();
    // 每个字符一段，最后是格式要求的 0xFFFF 一段。
    let segments = chars.len() as u16 + 1;
    let search = 2 * (1u16 << (15 - segments.leading_zeros()));
    let mut table = Vec::new();
    be16(&mut table, 4);
    be16(&mut table, 16 + 8 * segments);
    be16(&mut table, 0);
    be16(&mut table, 2 * segments);
    be16(&mut table, search);
    be16(&mut table, (search / 2).trailing_zeros() as u16);
    be16(&mut table, 2 * segments - search);
    for &c in &chars {
        be16(&mut table, c);
    }
    be16(&mut table, 0xFFFF);
    be16(&mut table, 0);
    for &c in &chars {
        be16(&mut table, c);
    }
    be16(&mut table, 0xFFFF);
    for (glyph, &c) in (1u16..).zip(&chars) {
        be16(&mut table, glyph.wrapping_sub(c));
    }
    be16(&mut table, 1);
    for _ in 0..segments {
        be16(&mut table, 0);
    }

    let mut out = Vec::with_capacity(12 + table.len());
    be16(&mut out, 0);
    be16(&mut out, 1);
    be16(&mut out, 3);
    be16(&mut out, 1);
    be32(&mut out, 12);
    out.extend_from_slice(&table);
    out
}

/// `name` 表：字族名、子族名、全名和 PostScript 名，都用 Windows 的 Unicode 编码。
fn name(family: &str, subfamily: &str) -> Vec<u8> {
    let full = format!("{family} {subfamily}");
    let postscript: String = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let postscript = format!("{postscript}-{subfamily}");
    let entries = [
        (1u16, family),
        (2, subfamily),
        (4, &full),
        (6, &postscript),
        (16, family),
        (17, subfamily),
    ];

    let mut records = Vec::new();
    let mut storage = Vec::new();
    let mut offset = 0u16;
    for (id, value) in entries {
        let mut bytes = Vec::new();
        for unit in value.encode_utf16() {
            be16(&mut bytes, unit);
        }
        be16(&mut records, 3);
        be16(&mut records, 1);
        be16(&mut records, 0x0409);
        be16(&mut records, id);
        be16(&mut records, bytes.len() as u16);
        be16(&mut records, offset);
        offset += bytes.len() as u16;
        storage.extend_from_slice(&bytes);
    }

    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, entries.len() as u16);
    be16(&mut out, 6 + 12 * entries.len() as u16);
    out.extend_from_slice(&records);
    out.extend_from_slice(&storage);
    out
}

/// `OS/2` 表：fontconfig 从中读取字重和斜体。
fn os2(weight: u16, bold: bool, italic: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    be16(&mut out, 4);
    be16(&mut out, 500);
    be16(&mut out, weight);
    be16(&mut out, 5);
    be16(&mut out, 0);
    for value in [650i16, 600, 0, 75] {
        be16(&mut out, value as u16);
    }
    for value in [650i16, 600, 0, 350] {
        be16(&mut out, value as u16);
    }
    for value in [50i16, 250] {
        be16(&mut out, value as u16);
    }
    be16(&mut out, 0);
    out.extend_from_slice(&[0; 10]);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(b"WINC");
    let mut selection = if bold { 0x20 } else { 0 };
    if italic {
        selection |= 0x01;
    }
    if !bold && !italic {
        selection |= 0x40;
    }
    be16(&mut out, selection);
    be16(&mut out, 0xFFFF);
    be16(&mut out, 0);
    for value in [800i16, -200, 0] {
        be16(&mut out, value as u16);
    }
    be16(&mut out, 800);
    be16(&mut out, 200);
    be32(&mut out, 1);
    be32(&mut out, 0);
    be16(&mut out, 500);
    be16(&mut out, 700);
    be16(&mut out, 0);
    be16(&mut out, 32);
    be16(&mut out, 1);
    assert_eq!(out.len(), 96);
    out
}

/// `post` 表：版本 3，不包含字形名。
fn post() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    be32(&mut out, 0x0003_0000);
    be32(&mut out, 0);
    be16(&mut out, (-100i16) as u16);
    be16(&mut out, 50);
    be32(&mut out, 0);
    for _ in 0..4 {
        be32(&mut out, 0);
    }
    assert_eq!(out.len(), 32);
    out
}
