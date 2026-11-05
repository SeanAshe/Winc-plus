//! 字体：按需 mmap 字体文件，不扫描系统字体库。
//!
//! 槽位 0 是等宽字体，槽位 1 是界面字体，槽位 2 是等宽字体的斜体，其后是按优先级排列的后备
//! 字体；斜体和后备字体在首次需要时才映射文件。候选路径按平台给出，用户配置的字体排在最前面。

use std::fs::File;
use std::path::PathBuf;

use swash::{CacheKey, FontRef, StringId};

use crate::FontSpec;
use crate::cache::FastMap;
use crate::text::font_path::FontFile;
use crate::text::system_outline::SystemOutlines;

/// 字体种类：编辑区、界面、注释斜体和数学字体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontKind {
    /// 编辑区的等宽字体。
    Mono,
    /// 界面字体（文件树、标签、状态栏）。
    Ui,
    /// 等宽字体的斜体，用于注释。
    ///
    /// 字体没有斜体或斜体的字宽与正体不同时，把正体斜切代替。
    MonoItalic,
    /// 数学字体（Markdown 预览里的公式）：macOS 自带的 STIX Two Math 这类，字母、数字、
    /// 运算符风格一致。
    ///
    /// 系统没有时用界面字体。
    Math,
}

/// 选后备字体时最多尝试的 fontconfig 结果数（经验值）：系统里没有合适的字体时不逐个打开全部字体。
const SORTED_FONTS: usize = 64;

/// 一个已经打开的字体文件。
pub(crate) struct Face {
    /// 字体文件的内容，见 [`FontData`]。
    data: FontData,
    /// 所选字体在文件中的偏移（字节）。
    offset: u32,
    /// swash 用来区分不同字体的缓存键。
    key: CacheKey,
    /// 可变字体的归一化坐标：请求的是命名实例（如 `PingFangSC-Regular`）时取该实例的坐标，
    /// 否则把 `wght` 轴固定为 400 字重；不是可变字体，或以不固定字重的方式打开时为空。
    ///
    /// 系统等宽字体 `SFNSMono.ttf` 的 `wght` 轴默认值约为 295（取自该字体的 `fvar` 表），
    /// 低于常规字重 400，因此需要固定。
    pub(crate) coords: Vec<i16>,
    /// 字形只放在 `hvgl` 表中时，由系统提供轮廓；否则为 `None`，由 swash 读取。
    pub(crate) system: Option<SystemOutlines>,
}

impl Face {
    /// 返回指向映射数据的字体引用。
    pub(crate) fn font(&self) -> FontRef<'_> {
        FontRef {
            data: &self.data,
            offset: self.offset,
            key: self.key,
        }
    }

    /// 打开 `file` 指定的字体（`.ttc` 集合中按 PostScript 名选取），可变字体不是命名实例时
    /// 把字重固定为 400（见 [`Self::coords`]）。
    ///
    /// 文件无法打开、映射或解析，或字体中没有可绘制的字形数据时返回 `None`。
    pub(crate) fn open(file: &FontFile) -> Option<Face> {
        Self::open_with(file, true)
    }

    /// 同 [`Self::open`]；`pin_weight` 为假时不固定字重，可变字体不是命名实例时坐标为空，由
    /// 调用方按需要的字重另算（SVG 文字的轮廓，见 `outline` 模块）。
    pub(crate) fn open_with(file: &FontFile, pin_weight: bool) -> Option<Face> {
        let data = load(&File::open(&file.path).ok()?)?;
        let selected = select(&data, file.postscript.as_deref())?;
        let font = selected.font;
        let (offset, key) = (font.offset, font.key);
        // 系统提供轮廓时，CoreText 字体就是所请求的实例（或默认实例），字宽要与之一致，
        // 不再固定字重。
        let coords = match selected.instance {
            Some(coords) => coords,
            None if pin_weight && !selected.system && weight_axis(&font) => font
                .variations()
                .normalized_coords([("wght", 400.0)])
                .collect(),
            None => Vec::new(),
        };
        let system = match selected.system {
            true => {
                let name = file.postscript.clone().or_else(|| postscript_name(&font))?;
                Some(SystemOutlines::new(&file.path, &name, &font)?)
            }
            false => None,
        };

        Some(Face {
            data,
            offset,
            key,
            coords,
            system,
        })
    }

    /// 轮廓能否按字重调整：字体有 `wght`（字重）轴，且轮廓由 swash 读取。
    ///
    /// 系统提供的轮廓固定为所请求的实例，不能按字重调整。
    pub(crate) fn has_weight_axis(&self) -> bool {
        self.system.is_none() && weight_axis(&self.font())
    }
}

/// 字体是否有 `wght`（字重）轴。
fn weight_axis(font: &FontRef) -> bool {
    font.variations()
        .any(|v| v.tag() == swash::tag_from_bytes(b"wght"))
}

/// swash 能读的字形表：TrueType 轮廓、CFF 轮廓、彩色位图。
///
/// 这几种之外的字形格式 swash 无法绘制：这样的字体字符映射和字宽都正常，排版不出错，但每个字
/// 都是空白。
const GLYPH_TABLES: [&[u8; 4]; 5] = [b"glyf", b"CFF ", b"CFF2", b"sbix", b"CBDT"];

/// Apple 未公开格式的字形表。
///
/// macOS 15 起系统自带的苹方只有该表，轮廓由系统提供（见 `SystemOutlines`）。
const SYSTEM_GLYPH_TABLE: &[u8; 4] = b"hvgl";

/// 从字体文件中选出的字体。
pub(crate) struct Selected<'a> {
    /// 选中的字体。
    pub font: FontRef<'a>,
    /// 请求的 PostScript 名是可变字体的命名实例时，该实例的归一化坐标；否则为 `None`。
    pub instance: Option<Vec<i16>>,
    /// 字形只放在 `SYSTEM_GLYPH_TABLE`（`hvgl` 表）中，轮廓要由系统提供。
    pub system: bool,
}

/// 返回字体文件数据 `data` 中 PostScript 名为 `postscript` 的字体。
///
/// 名字先与各字体自身的 PostScript 名比对，再与可变字体的命名实例比对（例如苹方的
/// `PingFangSC-Regular` 是 `PingFangSC-Medium` 字体中的一个实例）。`postscript` 为 `None` 或
/// 文件中没有该名字时取第一个字体；数据无法解析或选中的字体没有可绘制的字形时返回 `None`。
pub(crate) fn select<'a>(data: &'a [u8], postscript: Option<&str>) -> Option<Selected<'a>> {
    let first = FontRef::from_index(data, 0)?;
    let (font, instance) = postscript
        .and_then(|postscript| find(data, postscript))
        .unwrap_or((first, None));
    let has = |tag: &[u8; 4]| font.table(swash::tag_from_bytes(tag)).is_some();
    let system = if GLYPH_TABLES.iter().any(|tag| has(tag)) {
        false
    } else if cfg!(target_os = "macos") && has(SYSTEM_GLYPH_TABLE) {
        true
    } else {
        return None;
    };
    Some(Selected {
        font,
        instance,
        system,
    })
}

/// 在字体文件数据 `data` 中查找 PostScript 名为 `postscript` 的字体，返回该字体和命名实例的
/// 归一化坐标（不是命名实例时为 `None`）；找不到时返回 `None`。
fn find<'a>(data: &'a [u8], postscript: &str) -> Option<(FontRef<'a>, Option<Vec<i16>>)> {
    let count = swash::FontDataRef::new(data).map_or(1, |d| d.len());
    let fonts = || (0..count).filter_map(|i| FontRef::from_index(data, i));
    let named = |name: Option<swash::LocalizedString>| {
        name.is_some_and(|name| name.chars().eq(postscript.chars()))
    };
    if let Some(font) = fonts().find(|font| {
        named(
            font.localized_strings()
                .find_by_id(StringId::PostScript, None),
        )
    }) {
        return Some((font, None));
    }
    fonts().find_map(|font| {
        let instance = font.instances().find(|instance| {
            instance_postscript(&font, instance.index()).is_some_and(|name| name == postscript)
        })?;
        Some((font, Some(instance.normalized_coords().collect())))
    })
}

/// 返回可变字体 `font` 中第 `index` 个命名实例的 PostScript 名；没有时返回 `None`。
///
/// 直接读 `fvar` 表中实例记录末尾的名字编号：swash 0.2 读该编号时少加了记录本身的偏移，读到的
/// 是表头中的数据。实例的坐标由 swash 读取。
fn instance_postscript(font: &FontRef, index: usize) -> Option<String> {
    let fvar = font.table(swash::tag_from_bytes(b"fvar"))?;
    let read = |at: usize| {
        let bytes = fvar.get(at..at + 2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    };
    let axes = usize::from(read(4)?);
    let axis_count = usize::from(read(8)?);
    let axis_size = usize::from(read(10)?);
    let size = usize::from(read(14)?);
    // 实例记录依次为：子族名编号、标志、每个轴一个坐标，最后是可选的 PostScript 名编号。
    if size < 6 + 4 * axis_count {
        return None;
    }
    let record = axes + axis_count * axis_size + index * size;
    let id = read(record + 4 + 4 * axis_count)?;
    let name = font
        .localized_strings()
        .find_by_id(StringId::Other(id), None)?;
    Some(name.chars().collect())
}

/// 返回字体自身的 PostScript 名；字体中没有该名字时返回 `None`。
pub(crate) fn postscript_name(font: &FontRef) -> Option<String> {
    let name = font
        .localized_strings()
        .find_by_id(StringId::PostScript, None)?;
    Some(name.chars().collect())
}

/// 字体文件的内容：系统的字体映射进内存，其余的读进内存。
pub(crate) enum FontData {
    /// 映射的文件，见 [`load`]。
    Mapped(memmap2::Mmap),
    /// 读进内存的文件内容。
    ///
    /// Windows 上被映射的文件其他进程无法修改，因此总是映射，不使用该变体。
    #[cfg_attr(not(unix), allow(dead_code))]
    Owned(Vec<u8>),
}

impl std::ops::Deref for FontData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            FontData::Mapped(map) => map,
            FontData::Owned(bytes) => bytes,
        }
    }
}

/// 读取字体文件 `file` 的内容；无法读取时返回 `None`。
///
/// 映射的文件被其他进程截断以后，访问被截掉的页会收到 SIGBUS，整个进程退出。属于 root 的文件
/// （macOS 只读系统卷上的系统字体、Linux 的 `/usr/share/fonts`）普通用户无法修改，映射进来，
/// 多个字号、多个窗口共用系统的页缓存；用户自己的字体（`~/Library/Fonts`、配置里写的路径）
/// 可能在使用中被替换或截断，因此读进内存。
pub(crate) fn load(file: &File) -> Option<FontData> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().ok()?.uid() != 0 {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut &*file, &mut bytes).ok()?;
            return Some(FontData::Owned(bytes));
        }
    }
    map(file).map(FontData::Mapped)
}

/// 把 `file` 以只读方式映射进内存；映射失败时返回 `None`。
///
/// 这是本模块唯一一处 `unsafe`：把文件映射进内存。
#[allow(unsafe_code)]
fn map(file: &File) -> Option<memmap2::Mmap> {
    // SAFETY: `file` 是调用方持有的有效文件句柄，映射为只读。映射期间文件内容必须保持不变：
    // 这里只映射普通用户无法修改的文件（见 `load`），这类文件只有管理员能截断；被截断后访问
    // 被截掉的页会触发 SIGBUS，这是内存映射文件固有的限制。
    unsafe { memmap2::Mmap::map(file) }.ok()
}

/// macOS 系统自带的苹方所在的文件（macOS 15 起）。
const PINGFANG: &str =
    "/System/Library/PrivateFrameworks/FontServices.framework/Resources/Reserved/PingFangUI.ttc";

/// 字体槽位的状态。
enum Slot {
    /// 尚未映射，值是候选字体。
    Pending(Choices),
    /// 已映射。
    Ready(Face),
    /// 候选文件都无法打开。
    Missing,
}

/// 等宽字体所在的槽位；槽位 0 是等宽字体，槽位 1 是界面字体，槽位 2 是等宽字体的斜体，
/// 其后是后备字体。
pub(crate) const MONO: u8 = 0;
/// 界面字体所在的槽位。
const UI: u8 = 1;
/// 等宽字体的斜体所在的槽位。
pub(crate) const MONO_ITALIC: u8 = 2;
/// 数学字体所在的槽位。
const MATH: u8 = 3;
/// 第一个后备字体的槽位。
const FALLBACK: u8 = 4;

/// 字符解析结果缓存的最大项数（项，65536）。
///
/// 缓存项随出现过的字符数增长，满了就整体清空。该值是经验值。
const RESOLVED_ENTRIES: usize = 1 << 16;

/// 全部字体槽位及字符解析缓存。
pub(crate) struct Fonts {
    /// 字体槽位：0 是等宽字体，1 是界面字体，2 是等宽字体的斜体，其后是后备字体。
    slots: Vec<Slot>,
    /// 字符解析结果缓存：（字体种类，字符）→（槽位，字形 id）。
    resolved: FastMap<(FontKind, char), (u8, u16)>,
    /// ASCII 的快速路径：`[字体种类][码点]` →（槽位，字形 id），字体里没有该字符时为 `None`。
    ///
    /// 用于避免每个字符都查哈希表。
    ascii: [[Option<(u8, u16)>; 128]; 4],
}

impl Fonts {
    /// 加载字体；`mono` 和 `ui` 是用户配置里指定的字体，为 `None` 时用内置候选。
    ///
    /// # Errors
    ///
    /// 所有候选中都找不到可用的等宽字体时，返回列出已尝试路径的错误描述。
    ///
    /// # Panics
    ///
    /// 等宽字体成功打开后又立即无法打开时 panic。
    pub(crate) fn load(mono: Option<&FontSpec>, ui: Option<&FontSpec>) -> Result<Fonts, String> {
        // 指定的字体不可用时退回内置候选而不报错：字体配置有误时程序仍可启动。
        let candidates = Candidates::for_platform(mono, ui);
        let (mono, mono_file) = first_opened(&candidates.mono).ok_or_else(|| {
            format!(
                "找不到等宽字体，尝试过：{:?}。可以用 WINC_FONT 指定字体文件，\
                 或在配置里设置 editor.font。",
                candidates.mono.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        })?;

        // 没有界面字体时用等宽字体代替。
        let ui = first_existing(&candidates.ui)
            .or_else(|| first_existing(&candidates.mono))
            .expect("等宽字体刚刚成功打开过");

        let italic = italic_candidates(&mono_file, &mono);
        // 数学字体一开始就映射（只是映射文件，很快）：度量要用到它，`metrics` 不能再去加载。
        let math = first_existing(&candidates.math).map_or(Slot::Missing, Slot::Ready);
        let mut slots = vec![
            Slot::Ready(mono),
            Slot::Ready(ui),
            Slot::Pending(Choices::files(italic)),
            math,
        ];
        slots.extend(candidates.fallbacks.into_iter().map(Slot::Pending));

        let mut fonts = Fonts {
            slots,
            resolved: FastMap::default(),
            ascii: [[None; 128]; 4],
        };

        for kind in [FontKind::Mono, FontKind::Ui, FontKind::Math] {
            fonts.fill_ascii(kind);
        }

        Ok(fonts)
    }

    /// 按 `kind` 主槽位中的字体填写 ASCII 快速路径；槽位中还没有字体时什么也不做。
    ///
    /// 斜体槽位确定没有可用的字体（`Missing`）时，注释里的 ASCII 字符本来就落到正体上（绘制时
    /// 再斜切），按正体填写，同样走快速路径；斜体还没加载（`Pending`）时不填，以免先把正体占上。
    fn fill_ascii(&mut self, kind: FontKind) {
        let primary = self.primary(kind);
        let slot = match (kind, &self.slots[primary as usize]) {
            (FontKind::MonoItalic, Slot::Missing) => MONO,
            (FontKind::Math, Slot::Missing) => UI,
            _ => primary,
        };
        let Some(Slot::Ready(face)) = self.slots.get(slot as usize) else {
            return;
        };
        let charmap = face.font().charmap();

        for (c, entry) in self.ascii[kind as usize].iter_mut().enumerate() {
            let glyph = charmap.map(c as u32);
            *entry = (glyph != 0).then_some((slot, glyph));
        }
    }

    /// 返回字体种类 `kind` 对应的主槽位。
    pub(crate) fn primary(&self, kind: FontKind) -> u8 {
        match kind {
            FontKind::Mono => MONO,
            FontKind::Ui => UI,
            FontKind::MonoItalic => MONO_ITALIC,
            FontKind::Math => MATH,
        }
    }

    /// 字体种类 `kind` 的主字体；数学字体没有时是界面字体。
    pub(crate) fn primary_face(&self, kind: FontKind) -> Option<&Face> {
        self.face(self.primary(kind)).or_else(|| self.face(UI))
    }

    /// 首次用到斜体时映射斜体字体。
    ///
    /// 斜体的字宽与正体不同时不使用斜体（字符必须落在等宽的格子里），改为把正体斜切代替。
    fn load_italic(&mut self) {
        if !matches!(self.slots[MONO_ITALIC as usize], Slot::Pending(_)) {
            return;
        }
        self.ensure_loaded(MONO_ITALIC);

        let width = |face: &Face| {
            let font = face.font();
            font.glyph_metrics(&face.coords)
                .advance_width(font.charmap().map('0'))
                / f32::from(font.metrics(&face.coords).units_per_em.max(1))
        };
        if let (Some(mono), Some(italic)) = (self.face(MONO), self.face(MONO_ITALIC))
            && (width(mono) - width(italic)).abs() > 0.005
        {
            self.slots[MONO_ITALIC as usize] = Slot::Missing;
        }
        self.fill_ascii(FontKind::MonoItalic);
    }

    /// 返回槽位 `slot` 中已映射的字体；尚未映射或不存在时返回 `None`。
    pub(crate) fn face(&self, slot: u8) -> Option<&Face> {
        match self.slots.get(slot as usize) {
            Some(Slot::Ready(face)) => Some(face),
            _ => None,
        }
    }

    /// 返回能显示字符 `c` 的（槽位，字形 id）。
    ///
    /// 依次尝试 `kind` 对应的主字体、另一种主字体和各后备字体，必要时才映射文件。所有字体都没有
    /// 该字符时返回（主槽位，0），即该字体的 `.notdef` 字形。
    pub(crate) fn resolve(&mut self, kind: FontKind, c: char) -> (u8, u16) {
        if kind == FontKind::MonoItalic {
            self.load_italic();
        }
        let primary = self.primary(kind);

        if (c as u32) < 128
            && let Some(hit) = self.ascii[kind as usize][c as usize]
        {
            return hit;
        }

        if let Some(&hit) = self.resolved.get(&(kind, c)) {
            return hit;
        }

        // 先找主字体，再找另外两种主字体（斜体没有的字符用正体），最后是后备字体。斜体不作为
        // 其他字体的后备。
        let mut order: Vec<u8> = match kind {
            FontKind::Mono => vec![MONO, UI],
            FontKind::Ui => vec![UI, MONO],
            FontKind::MonoItalic => vec![MONO_ITALIC, MONO, UI],
            FontKind::Math => vec![MATH, UI, MONO],
        };
        order.extend(FALLBACK..self.slots.len() as u8);

        let mut result = (primary, 0);
        // 缓存项数随出现过的字符数增长（例如打开一个几乎每个字符都不同的文件），满了就整体清空；
        // 之后重新解析只需再查几次字体的字符映射表。
        if self.resolved.len() >= RESOLVED_ENTRIES {
            self.resolved = FastMap::default();
        }

        for slot in order {
            self.ensure_loaded(slot);

            if let Some(face) = self.face(slot) {
                let glyph = face.font().charmap().map(c);

                if glyph != 0 {
                    result = (slot, glyph);
                    break;
                }
            }
        }

        self.resolved.insert((kind, c), result);
        result
    }

    /// 槽位 `slot` 仍是 `Pending` 时，尝试映射其候选文件，并把槽位更新为 `Ready` 或 `Missing`。
    fn ensure_loaded(&mut self, slot: u8) {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };

        if let Slot::Pending(choices) = entry {
            *entry = match choices.open() {
                Some(face) => Slot::Ready(face),
                None => Slot::Missing,
            };
        }
    }
}

/// 返回用户配置的字体解析出的候选；解析不出来时为空列表，调用方会按平台默认字体处理。
fn resolve_first(spec: Option<&FontSpec>) -> Vec<FontFile> {
    spec.and_then(FontSpec::resolve).into_iter().collect()
}
/// 返回 `files` 中第一个能打开的字体；都打不开时返回 `None`。
fn first_existing(files: &[FontFile]) -> Option<Face> {
    files.iter().find_map(Face::open)
}

/// 返回 `files` 中第一个能打开的字体及其文件；都打不开时返回 `None`。
fn first_opened(files: &[FontFile]) -> Option<(Face, FontFile)> {
    files
        .iter()
        .find_map(|file| Face::open(file).map(|face| (face, file.clone())))
}

/// 返回等宽字体 `file`（已打开为 `face`）的斜体候选，按优先级排列。
///
/// 先在同一目录下按常见的命名找文件，只做几次 `stat`：`X-Regular.ttf` → `X-Italic.ttf`，
/// `X[wght].ttf` → `X-Italic[wght].ttf`，`consola.ttf` → `consolai.ttf`，`.ttc` 集合中
/// `Menlo-Regular` → `Menlo-Italic`。都没有时才按字族名向系统查询（macOS 上一次约几毫秒）。
fn italic_candidates(file: &FontFile, face: &Face) -> Vec<FontFile> {
    let mut out = Vec::new();
    let path = &file.path;
    let (Some(stem), Some(ext)) = (
        path.file_stem().and_then(|s| s.to_str()),
        path.extension().and_then(|s| s.to_str()),
    ) else {
        return out;
    };

    if ext.eq_ignore_ascii_case("ttc") {
        let postscript = file
            .postscript
            .clone()
            .or_else(|| postscript_name(&face.font()));
        if let Some(name) = postscript {
            let base = name.strip_suffix("-Regular").unwrap_or(&name);
            out.push(FontFile {
                path: path.clone(),
                postscript: Some(format!("{base}-Italic")),
            });
        }
    } else {
        let mut names = Vec::new();
        if stem.contains("Regular") {
            names.push(stem.replacen("Regular", "Italic", 1));
        }
        if let Some((name, axes)) = stem.split_once('[') {
            names.push(format!("{name}-Italic[{axes}"));
        }
        for suffix in ["Italic", "-Italic", "-Oblique", "i"] {
            names.push(format!("{stem}{suffix}"));
        }
        out.extend(
            names
                .into_iter()
                .map(|name| path.with_file_name(format!("{name}.{ext}")))
                .filter(|p| p.is_file())
                .map(FontFile::first),
        );
    }

    if out.is_empty()
        && let Some(family) = family_name(&face.font())
        && let Some(found) = crate::text::font_path::resolve_face(&family, false, true)
        && found.italic
    {
        out.push(found.file);
    }
    out
}

/// 返回字体的字族名（如 `Menlo`）；字体里没有时返回 `None`。
fn family_name(font: &FontRef) -> Option<String> {
    let name = font
        .localized_strings()
        .find_by_id(StringId::Family, None)?;
    Some(name.to_string())
}

/// 返回 fontconfig 中与 `pattern`（如 `monospace`、`sans-serif:lang=zh-cn`）最匹配的字体。
///
/// 通过 `fc-match` 命令查询，不链接 fontconfig 库。没有 `fc-match`、查询失败或给出的文件不存在
/// 时返回 `None`。
fn fontconfig(pattern: &str) -> Option<FontFile> {
    let output = std::process::Command::new("fc-match")
        .args(["--format", "%{file}\n%{postscriptname}", pattern])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (file, postscript) = text.split_once('\n').unwrap_or((&text, ""));
    let path = PathBuf::from(file);
    path.is_file().then(|| FontFile {
        path,
        postscript: (!postscript.is_empty()).then(|| postscript.to_owned()),
    })
}

/// 返回 fontconfig 为 `pattern` 排序的字体，最匹配的在前，最多 [`SORTED_FONTS`] 个。
///
/// `fc-match -s` 的结果按用户的偏好排序，排在最前的不一定含有查询的语言的文字：用户把不含
/// 汉字的字体强制绑定到 `sans-serif` 时，它在 `lang=zh-cn` 的查询中同样排第一。没有 `fc-match`
/// 或查询失败时返回空列表。
fn fontconfig_sorted(pattern: &str) -> Vec<FontFile> {
    let output = std::process::Command::new("fc-match")
        .args([
            "--sort",
            "--format",
            "%{file}\n%{postscriptname}\n",
            pattern,
        ])
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_sorted_fonts(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

/// 解析 [`fontconfig_sorted`] 的输出：每个字体两行，依次是文件路径和 PostScript 名；只保留存在的
/// 文件。
fn parse_sorted_fonts(text: &str) -> Vec<FontFile> {
    let lines: Vec<&str> = text.split('\n').collect();
    lines
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[path, name]| FontFile {
            path: PathBuf::from(path),
            postscript: (!name.is_empty()).then(|| (*name).to_owned()),
        })
        .filter(|font| font.path.is_file())
        .take(SORTED_FONTS)
        .collect()
}

/// 各字体槽位的候选字体。
struct Candidates {
    /// 等宽字体的候选，按优先级排列。
    mono: Vec<FontFile>,
    /// 界面字体的候选，按优先级排列。
    ui: Vec<FontFile>,
    /// 数学字体的候选，按优先级排列。
    math: Vec<FontFile>,
    /// 每一项是一个后备槽位的候选。
    fallbacks: Vec<Choices>,
}

/// 一个后备槽位的候选：先按序尝试 fontconfig 的查询结果（有查询时），再按优先级尝试固定的字体
/// 文件。
///
/// 后备槽位在第一次用到时才打开，fontconfig 也在那时查询。
struct Choices {
    /// 交给 fontconfig 的查询，如 `sans-serif:lang=zh-cn`。
    query: Option<String>,
    /// 固定的候选字体文件，按优先级排列。
    files: Vec<FontFile>,
    /// 槽位要显示的代表字符；不为 `None` 时选用第一个含有该字符的字体。
    probe: Option<char>,
}

impl Choices {
    /// 只有固定字体文件的候选。
    fn files(files: Vec<FontFile>) -> Choices {
        Choices {
            query: None,
            files,
            probe: None,
        }
    }

    /// 打开第一个含有 [`Self::probe`] 的候选字体；没有 `probe` 时打开第一个能打开的。
    ///
    /// 都不含 `probe` 时返回第一个能打开的字体，都打不开时返回 `None`。
    fn open(&self) -> Option<Face> {
        let asked = self
            .query
            .as_deref()
            .map_or_else(Vec::new, fontconfig_sorted);
        let mut first = None;
        for face in asked.iter().chain(&self.files).filter_map(Face::open) {
            let Some(probe) = self.probe else {
                return Some(face);
            };
            if face.font().charmap().map(probe) != 0 {
                return Some(face);
            }
            first.get_or_insert(face);
        }
        first
    }
}

impl Candidates {
    /// 返回各槽位的候选字体：用户配置的字体排在最前面，其后是平台默认字体。
    fn for_platform(mono: Option<&FontSpec>, ui: Option<&FontSpec>) -> Candidates {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();

        // `WINC_FONT` 环境变量指定的字体排在最前面，其次是用户配置的字体，便于临时试用其他字体。
        let mut mono = resolve_first(mono);
        if let Some(font) = std::env::var_os("WINC_FONT") {
            mono.insert(0, FontFile::first(font));
        }
        // 没有指定编辑器字体时，优先用 Maple Mono NF CN；没安装则继续用后面的平台默认字体。
        if mono.is_empty()
            && let Some(file) = FontSpec::Name(crate::DEFAULT_EDITOR_FONT.to_owned()).resolve()
        {
            mono.push(file);
        }
        let mut ui_paths = resolve_first(ui);

        let user_fonts = |name: &str| -> Vec<FontFile> {
            vec![
                FontFile::first(home.join("Library/Fonts").join(name)),
                FontFile::first(PathBuf::from("/Library/Fonts").join(name)),
            ]
        };
        let paths = |list: &[&str]| -> Vec<FontFile> {
            list.iter().copied().map(FontFile::first).collect()
        };
        if cfg!(target_os = "macos") {
            mono.extend(user_fonts("JetBrainsMono-Regular.ttf"));
            mono.extend(user_fonts("JetBrainsMono[wght].ttf"));
            mono.extend(paths(&[
                "/System/Library/Fonts/SFNSMono.ttf",
                "/System/Library/Fonts/Menlo.ttc",
                "/System/Library/Fonts/Monaco.ttf",
            ]));

            Candidates {
                mono,
                ui: {
                    ui_paths.extend(paths(&[
                        "/System/Library/Fonts/SFNS.ttf",
                        "/System/Library/Fonts/Helvetica.ttc",
                    ]));
                    ui_paths
                },
                math: paths(&["/System/Library/Fonts/Supplemental/STIXTwoMath.otf"]),
                fallbacks: vec![
                    // 中文先用苹方（系统界面的中文字体）。macOS 15 起它的字形只在 `hvgl` 表中，
                    // 由系统提供轮廓；文件不存在时（更早的系统）用冬青黑体。
                    std::iter::once(FontFile {
                        path: PINGFANG.into(),
                        postscript: Some("PingFangSC-Regular".to_owned()),
                    })
                    .chain(paths(&[
                        "/System/Library/Fonts/Hiragino Sans GB.ttc",
                        "/System/Library/Fonts/STHeiti Medium.ttc",
                        "/System/Library/Fonts/STHeiti Light.ttc",
                    ]))
                    .collect(),
                    paths(&["/System/Library/Fonts/Menlo.ttc"]),
                    paths(&["/System/Library/Fonts/Apple Symbols.ttf"]),
                    paths(&["/System/Library/Fonts/Apple Color Emoji.ttc"]),
                    paths(&["/System/Library/Fonts/AppleSDGothicNeo.ttc"]),
                ]
                .into_iter()
                .map(Choices::files)
                .collect(),
            }
        } else if cfg!(target_os = "windows") {
            let fonts = crate::text::font_path::windows_fonts();
            let system = |list: &[&str]| -> Vec<FontFile> {
                list.iter()
                    .map(|name| FontFile::first(fonts.join(name)))
                    .collect()
            };
            mono.extend(system(&["CascadiaMono.ttf", "consola.ttf"]));

            Candidates {
                mono,
                ui: {
                    ui_paths.extend(system(&["segoeui.ttf"]));
                    ui_paths
                },
                math: vec![FontFile {
                    path: fonts.join("cambria.ttc"),
                    postscript: Some("CambriaMath".to_owned()),
                }],
                fallbacks: vec![
                    system(&["msyh.ttc", "simsun.ttc"]),
                    system(&["seguisym.ttf"]),
                    system(&["seguiemj.ttf"]),
                ]
                .into_iter()
                .map(Choices::files)
                .collect(),
            }
        } else {
            // Linux 先查询 fontconfig：它记录了发行版安装字体的位置，也按用户设置的偏好选择；没有
            // fontconfig 的精简系统再试后面这些常见的位置。等宽字体和界面字体启动时就要用到，因此
            // 同时查询（各约几毫秒），中文和 emoji 的后备字体到第一次用到时再查。应用指定了语言时，
            // 等宽字体和界面字体也按该语言选，因为 fontconfig 默认按系统语言选。
            let language = app_language().map_or_else(String::new, |lang| format!(":lang={lang}"));
            let (asked_mono, asked_ui) = std::thread::scope(|scope| {
                let ui = scope.spawn(|| fontconfig(&format!("sans-serif{language}")));
                (
                    fontconfig(&format!("monospace{language}")),
                    ui.join().ok().flatten(),
                )
            });
            mono.extend(asked_mono);
            mono.extend(paths(&[
                "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
            ]));
            ui_paths.extend(asked_ui);
            ui_paths.extend(paths(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            ]));

            Candidates {
                mono,
                ui: ui_paths,
                math: paths(&[
                    "/usr/share/fonts/opentype/stix/STIXTwoMath-Regular.otf",
                    "/usr/share/fonts/opentype/lmodern/latinmodern-math.otf",
                    "/usr/share/fonts/texlive-lm-math/latinmodern-math.otf",
                ]),
                fallbacks: vec![
                    Choices {
                        query: Some(format!("sans-serif:lang={}", cjk_language())),
                        files: paths(&[
                            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                        ]),
                        probe: Some('中'),
                    },
                    Choices {
                        query: Some("emoji".to_owned()),
                        files: paths(&["/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"]),
                        probe: Some('😀'),
                    },
                ],
            }
        }
    }
}

/// 应用指定的、选日韩文字体用的语言；为 `None` 时按系统语言选。见 [`set_cjk_language`]。
static CJK_LANGUAGE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 设置选日韩文字体用的语言（如 `zh-cn`），使汉字按该语言的字形显示；`None` 表示按系统语言选。
///
/// 应用的界面语言是中文时传入对应的语言，使界面文字与字形一致。只有之后加载的字体（新建
/// 渲染器、[`crate::Renderer::set_fonts`]）按新语言选。返回语言是否改变，改变时调用方应重新
/// 加载字体。目前只有 Linux 按语言选字体，其他平台的中文字体是固定的。
pub fn set_cjk_language(language: Option<&str>) -> bool {
    let mut current = CJK_LANGUAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let changed = current.as_deref() != language;
    *current = language.map(str::to_owned);
    changed
}

/// 返回应用指定的、选日韩文字体用的语言；没有指定时返回 `None`。
fn app_language() -> Option<String> {
    CJK_LANGUAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// 返回选日韩文字体用的语言：应用指定了就用它，否则按系统语言（`LC_ALL`、`LC_CTYPE`、
/// `LANG`）选，见 [`cjk_language_of`]。
fn cjk_language() -> String {
    if let Some(language) = app_language() {
        return language;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default();
    cjk_language_of(&locale)
}

/// 返回选日韩文字体用的语言：系统语言 `locale`（如 `zh_TW.UTF-8`）是中文、日文或韩文时用它
/// （`zh-tw`），汉字按该地区的字形显示；其他语言用简体中文 `zh-cn`。
fn cjk_language_of(locale: &str) -> String {
    let tag = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('_', "-");
    let language = tag.split('-').next().unwrap_or_default();
    if matches!(language, "ja" | "ko") || (language == "zh" && tag != "zh") {
        tag
    } else {
        "zh-cn".to_owned()
    }
}

#[cfg(test)]
pub(crate) mod tests;
