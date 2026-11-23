//! 按字族名取字形轮廓，供 SVG 预览中的文字使用。
//!
//! 字族名解析成系统里的字体；该字体没有所需字符时，按顺序尝试后备字体。字体文件按需 mmap
//! 映射，轮廓按（字体、字形、字重）缓存。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use swash::scale::ScaleContext;
use swash::zeno::{Command, PathData, Point};

use crate::text::font::Face;
use crate::text::font_path::{FaceMatch, FontFile, resolve_face};
use crate::text::system_outline::GlyphPath;

/// 一个字形：轮廓和前进宽度。
///
/// 坐标以 em 为单位，原点在基线左端，`y` 轴向下。轮廓放在共享的 `Arc` 里，缓存与交出的
/// 是同一份，复制字形不复制轮廓。
#[derive(Debug, Clone)]
pub struct OutlineGlyph {
    /// 轮廓命令（直线、二次与三次贝塞尔曲线），坐标单位为 em。
    pub outline: Arc<[Command]>,
    /// 前进宽度（em），即排下一个字形时的水平位移。
    pub advance: f32,
}

/// 字体的上升、下降和 x 字高，单位为 em；`ascent` 和 `x_height` 向上为正。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlineMetrics {
    /// 基线以上的高度（em，正数）。
    pub ascent: f32,
    /// 基线以下的高度（em，正数）。
    pub descent: f32,
    /// 小写 `x` 的高度（em）。
    pub x_height: f32,
}

/// 轮廓字体的注册表：缓存字体文件和字形轮廓，所有 SVG 预览共用同一份。
pub struct OutlineFonts {
    /// 内部状态，加锁访问，使多个 SVG 预览可以共用同一份缓存。
    inner: Mutex<Inner>,
}

/// 打开的一个字体，以及取轮廓时需要的属性。
struct OutlineFace {
    /// 字体文件；不固定字重（见 [`Face::open_with`]），按每次请求的字重取轮廓。
    face: Face,
    /// 是否按字重调整轮廓，见 [`Face::has_weight_axis`]。
    has_weight_axis: bool,
    /// 是否合成斜体：字族里没有斜体时，把轮廓向右斜切（斜切系数 0.25，约 14°）。
    fake_italic: bool,
}

/// 轮廓缓存的软上限（字节，8 MiB）。
///
/// 按轮廓占用的字节估算，超过时整体清空，用到的轮廓再重新读取。所有 SVG 预览共用同一份缓存
/// 且一直存活，不设上限时浏览过的字符越多占用越大；该值取自调参。
const GLYPH_CACHE_BYTES: usize = 8 << 20;

/// 记下的字族名个数上限，超过时整体清空（连同打开的字体和轮廓缓存），用到的再重新查找。
///
/// 一个 SVG 用到的字族加上后备字体只有十几个；字族名来自 SVG 文件，不可信，不设上限时浏览的
/// SVG 越多，记下的名字和打开的字体越多，逐个比较也越慢。
const MAX_FAMILIES: usize = 64;

/// 轮廓字体的内部状态：已打开的字体、查找结果和轮廓缓存。
#[derive(Default)]
struct Inner {
    /// 查过的字族名（不分大小写，各一项）和按（粗体，斜体）排的四种查找结果（`faces` 的下标，
    /// 数组下标为 `bold * 2 + italic`）；外层 `None` 表示还没查过，内层 `None` 表示找不到。
    ///
    /// 找不到的结果也记下，避免重复查找；一个 SVG 用到的字族只有十几个，逐个比较比哈希一个
    /// 小写的名字更快。
    lookup: Vec<(String, [Option<Option<usize>>; 4])>,
    faces: Vec<OutlineFace>,
    /// （字体，字形，字重）→ 轮廓，没有字重轴的字体字重记 0；轮廓读不出来时为 `None`。
    glyphs: HashMap<(usize, u16, u16), Option<OutlineGlyph>>,
    /// `glyphs` 估算占用的字节数。
    bytes: usize,
    /// swash 的光栅化上下文，首次使用时创建。
    scale: Option<ScaleContext>,
}

impl Default for OutlineFonts {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlineFonts {
    /// 创建空的轮廓字体注册表。
    pub fn new() -> OutlineFonts {
        OutlineFonts {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// 返回字符 `c` 的轮廓；都找不到时返回 `None`。
    ///
    /// 按顺序尝试 `families`，再尝试系统的后备字体，取第一个同时有该字符且轮廓可读的字体。
    /// `weight` 是字重数值（如 400、700），不小于 600 时请求粗体；`italic` 为真时请求斜体。
    pub fn glyph(
        &self,
        families: &[String],
        weight: u16,
        italic: bool,
        c: char,
    ) -> Option<OutlineGlyph> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let bold = weight >= 600;
        for family in candidates(families) {
            let Some(index) = inner.face(family, bold, italic) else {
                continue;
            };
            let id = inner.faces[index].face.font().charmap().map(c);
            if id == 0 {
                continue;
            }
            // 字体有该字符但轮廓读不出来（例如轮廓格式不被 swash 支持）时，继续尝试下一个字体。
            if let Some(glyph) = inner.outline(index, id, weight) {
                return Some(glyph);
            }
        }
        None
    }

    /// 返回第一个能打开的字体对应的竖向尺寸（单位 em）。
    ///
    /// 都找不到时返回默认值：上升 0.8、下降 0.2、x 字高 0.5；取值与常见西文字体的比例
    /// 接近。
    pub fn metrics(&self, families: &[String], weight: u16, italic: bool) -> OutlineMetrics {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        for family in candidates(families) {
            if let Some(index) = inner.face(family, weight >= 600, italic) {
                let m = inner.faces[index].face.font().metrics(&[]);
                let upem = f32::from(m.units_per_em.max(1));
                return OutlineMetrics {
                    ascent: m.ascent / upem,
                    descent: m.descent / upem,
                    x_height: if m.x_height > 0.0 {
                        m.x_height / upem
                    } else {
                        0.5
                    },
                };
            }
        }
        OutlineMetrics {
            ascent: 0.8,
            descent: 0.2,
            x_height: 0.5,
        }
    }
}

impl Inner {
    /// 返回（字族名，粗体，斜体）对应的字体在 `faces` 中的下标；找不到时返回 `None`。
    ///
    /// 首次查找时打开字体。
    fn face(&mut self, family: &str, bold: bool, italic: bool) -> Option<usize> {
        let variant = usize::from(bold) * 2 + usize::from(italic);
        // 字族名不分大小写，逐个比较，不必每次查找都另外分配一个小写的名字。
        let entry = match self
            .lookup
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            Some(i) => i,
            None => {
                if self.lookup.len() >= MAX_FAMILIES {
                    // 轮廓缓存的键中包含 `faces` 的下标，因此一并清空。
                    self.lookup.clear();
                    self.faces.clear();
                    self.glyphs.clear();
                    self.bytes = 0;
                }
                self.lookup.push((family.to_owned(), [None; 4]));
                self.lookup.len() - 1
            }
        };
        if let Some(found) = self.lookup[entry].1[variant] {
            return found;
        }
        let found = open_face(family, bold, italic).map(|face| {
            self.faces.push(face);
            self.faces.len() - 1
        });
        self.lookup[entry].1[variant] = Some(found);
        found
    }

    /// 返回字体 `index` 中字形 `id` 的轮廓（单位 em，`y` 向下），结果带缓存。
    ///
    /// 轮廓无法读取时返回 `None`。
    fn outline(&mut self, index: usize, id: u16, weight: u16) -> Option<OutlineGlyph> {
        let OutlineFace {
            face,
            has_weight_axis,
            fake_italic,
        } = &self.faces[index];
        // 没有字重轴的字体：字重只影响选择哪个字体（已体现在 `index` 中），轮廓相同，
        // 因此缓存键中的字重记 0。
        let key = (index, id, if *has_weight_axis { weight } else { 0 });
        if let Some(cached) = self.glyphs.get(&key) {
            return cached.clone();
        }
        let font = face.font();
        let upem = f32::from(font.metrics(&[]).units_per_em.max(1));
        let weighted: Vec<i16>;
        let coords: &[i16] = if *has_weight_axis {
            weighted = font
                .variations()
                .normalized_coords([("wght", f32::from(weight))])
                .collect();
            &weighted
        } else {
            &face.coords
        };
        let advance = font.glyph_metrics(coords).advance_width(id) / upem;
        // 字体的 `y` 轴向上：转换为 `y` 向下并以 em 为单位。需要合成斜体时，按斜切系数 0.25
        // （约 14°）向右斜切。
        let skew = if *fake_italic { 0.25 } else { 0.0 };
        let map = |p: Point| Point::new((p.x + p.y * skew) / upem, -p.y / upem);
        let transform = |c: Command| match c {
            Command::MoveTo(p) => Command::MoveTo(map(p)),
            Command::LineTo(p) => Command::LineTo(map(p)),
            Command::QuadTo(a, p) => Command::QuadTo(map(a), map(p)),
            Command::CurveTo(a, b, p) => Command::CurveTo(map(a), map(b), map(p)),
            Command::Close => Command::Close,
        };
        // 轮廓坐标以字体单位计（1 em = `upem`），`y` 向上。
        let outline: Option<Arc<[Command]>> = match &face.system {
            Some(system) => {
                let mut path = GlyphPath::default();
                system.outline(id, upem, &mut path);
                Some(path.data().commands().map(transform).collect())
            }
            None => {
                let context = self.scale.get_or_insert_with(ScaleContext::new);
                let mut scaler = context
                    .builder(font)
                    .size(upem)
                    .normalized_coords(coords)
                    .hint(false)
                    .build();
                scaler
                    .scale_outline(id)
                    .map(|outline| outline.path().commands().map(transform).collect())
            }
        };
        let glyph = outline.map(|outline| OutlineGlyph { outline, advance });
        let size = std::mem::size_of::<Option<OutlineGlyph>>()
            + glyph
                .as_ref()
                .map_or(0, |g| g.outline.len() * std::mem::size_of::<Command>());
        if self.bytes + size > GLYPH_CACHE_BYTES {
            self.glyphs = HashMap::new();
            self.bytes = 0;
        }
        self.bytes += size;
        self.glyphs.insert(key, glyph.clone());
        glyph
    }
}

/// 返回字族 `family`（按 `bold`、`italic` 请求变体）对应的字体；找不到时返回 `None`。
fn open_face(family: &str, bold: bool, italic: bool) -> Option<OutlineFace> {
    let found = match family {
        // 系统界面字体：这四个通用名无法通过 CoreText 按名字查到文件，直接使用系统字体文件。
        "system-ui" | "-apple-system" | "BlinkMacSystemFont" | "ui-sans-serif"
            if cfg!(target_os = "macos") =>
        {
            Some(FaceMatch {
                file: FontFile::first("/System/Library/Fonts/SFNS.ttf"),
                bold: false,
                italic: false,
            })
        }
        name => resolve_face(name, bold, italic),
    }?;
    let face = Face::open_with(&found.file, false)?;
    Some(OutlineFace {
        has_weight_axis: face.has_weight_axis(),
        fake_italic: italic && !found.italic,
        face,
    })
}

/// 返回要尝试的字族：先是 `families`（通用字族名替换成系统字体，并去重），再是后备字体。
fn candidates(families: &[String]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut push = |name| {
        if !out.contains(&name) {
            out.push(name);
        }
    };
    for family in families {
        match generic(family) {
            Some(names) => names.iter().for_each(|name| push(name)),
            None => push(family),
        }
    }
    FALLBACKS.iter().for_each(|name| push(name));
    out
}

/// 返回 CSS 通用字族名（`serif`、`sans-serif`、`monospace` 等）在当前平台上对应的字体列表。
///
/// 其他名字返回 `None`，调用方原样使用。
fn generic(family: &str) -> Option<&'static [&'static str]> {
    let is = |name: &str| family.eq_ignore_ascii_case(name);
    Some(if is("serif") {
        SERIF
    } else if is("sans-serif") || is("ui-sans-serif") {
        SANS
    } else if is("monospace") || is("ui-monospace") {
        MONO
    } else if is("cursive") {
        CURSIVE
    } else if is("fantasy") {
        FANTASY
    } else {
        return None;
    })
}

// 每个平台各有一组常量：`SERIF`、`SANS`、`MONO`、`CURSIVE`、`FANTASY` 是 CSS 通用字族名对应的
// 字体列表，`FALLBACKS` 是所请求的字体缺字时依次尝试的后备字体；都按优先级排列。
#[cfg(target_os = "macos")]
const SERIF: &[&str] = &["Times", "Songti SC"];
#[cfg(target_os = "macos")]
const SANS: &[&str] = &[
    "Helvetica",
    "PingFang SC",
    "STHeitiSC-Light",
    "HiraginoSansGB-W3",
];
#[cfg(target_os = "macos")]
const MONO: &[&str] = &["Courier", "Menlo"];
#[cfg(target_os = "macos")]
const CURSIVE: &[&str] = &["Apple Chancery"];
#[cfg(target_os = "macos")]
const FANTASY: &[&str] = &["Papyrus"];
#[cfg(target_os = "macos")]
const FALLBACKS: &[&str] = &[
    "Helvetica",
    "PingFang SC",
    "STHeitiSC-Light",
    "HiraginoSansGB-W3",
    "Apple SD Gothic Neo",
    "Apple Symbols",
    "Arial Unicode MS",
];

#[cfg(target_os = "windows")]
const SERIF: &[&str] = &["Times New Roman", "SimSun"];
#[cfg(target_os = "windows")]
const SANS: &[&str] = &["Arial", "Microsoft YaHei"];
#[cfg(target_os = "windows")]
const MONO: &[&str] = &["Courier New", "Consolas"];
#[cfg(target_os = "windows")]
const CURSIVE: &[&str] = &["Comic Sans MS"];
#[cfg(target_os = "windows")]
const FANTASY: &[&str] = &["Impact"];
#[cfg(target_os = "windows")]
const FALLBACKS: &[&str] = &["Segoe UI", "Microsoft YaHei", "Segoe UI Symbol"];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SERIF: &[&str] = &["DejaVu Serif", "Noto Serif", "Noto Serif CJK SC"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SANS: &[&str] = &["DejaVu Sans", "Noto Sans", "Noto Sans CJK SC"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &[&str] = &["DejaVu Sans Mono", "Noto Sans Mono"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const CURSIVE: &[&str] = &["DejaVu Sans"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const FANTASY: &[&str] = &["DejaVu Sans"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const FALLBACKS: &[&str] = &[
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans CJK SC",
    "WenQuanYi Micro Hei",
];

#[cfg(test)]
mod tests;
