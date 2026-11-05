//! 按名字查字体文件：向各平台的字体系统查询，取回字体文件路径。
//!
//! macOS 用 CoreText 查询；Windows 和 Linux 在常见字体目录中按文件名查找。`unsafe` 调用集中在
//! `macos` 模块，每处的 `// SAFETY:` 注释写明了所依赖的前提。

use std::path::PathBuf;

use super::font_list::find_family;

/// 未单独设置编辑器字体时优先使用的字族。系统里没有它时再改用平台自带的等宽字体。
pub const DEFAULT_EDITOR_FONT: &str = "Maple Mono NF CN";

/// 一次字体请求：按名字查系统，或直接使用给定的路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSpec {
    /// 字体名或字族名，例如 `Menlo`、`Helvetica`、`苹方-简`。
    ///
    /// macOS 由 CoreText 查询，识别字族名、PostScript 名和本地化名称；其他平台按文件名前缀匹配，
    /// 忽略空格、连字符、下划线和大小写。这些方式都找不到时，再按同样宽松的写法匹配已安装的字族名。
    Name(String),
    /// 字体文件路径。
    Path(PathBuf),
}

/// 不能使用字体的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontError {
    /// 系统里没有该名字的字体，或路径不是文件。
    NotFound,
    /// 找到了字体文件，但无法打开，或其中的字形格式画不出来。
    Unsupported,
}

/// 解析出的字体：所在的文件和要使用的 PostScript 名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FontFile {
    /// 字体文件路径。
    pub path: PathBuf,
    /// 字体的 PostScript 名；`.ttc` 集合中靠该名字确定是第几个字体。
    ///
    /// 为 `None` 时取第一个。
    pub postscript: Option<String>,
}

impl FontFile {
    /// 创建指向文件中第一个字体的条目。
    pub(crate) fn first(path: impl Into<PathBuf>) -> FontFile {
        FontFile {
            path: path.into(),
            postscript: None,
        }
    }
}

impl FontSpec {
    /// 确认字体存在，并且能用来显示文字。
    ///
    /// 渲染层遇到不能用的字体时静默改用默认字体，调用方据此提前告知用户原因。
    ///
    /// # Errors
    ///
    /// 找不到字体时返回 [`FontError::NotFound`]，找到了但画不出字形时返回
    /// [`FontError::Unsupported`]。
    pub fn check(&self) -> Result<(), FontError> {
        let file = self.resolve().ok_or(FontError::NotFound)?;
        crate::text::font::Face::open(&file)
            .map(drop)
            .ok_or(FontError::Unsupported)
    }

    /// 解析成字体文件；路径不是文件，或系统里没有该名字的字体时返回 `None`。
    ///
    /// 返回 `None` 时调用方退回内置候选。
    pub(crate) fn resolve(&self) -> Option<FontFile> {
        match self {
            FontSpec::Path(path) => path.is_file().then(|| FontFile::first(path.clone())),
            FontSpec::Name(name) => resolve_name(name.trim()),
        }
    }
}

/// 返回字体名 `name` 对应的字体文件；系统里没有该字体时返回 `None`。
///
/// 系统界面字体无法按名字向 CoreText 查询，`SF Mono`、`SF Pro` 直接对应系统字体文件；用户
/// 自行安装了同名字体时优先使用安装的字体。
#[cfg(target_os = "macos")]
fn resolve_name(name: &str) -> Option<FontFile> {
    exact_font(name).or_else(|| find_family(name).map(|family| family.file.clone()))
}

/// 返回按名字 `name` 精确查到的字体文件：先问 CoreText，再查 CoreText 不列出的系统字体。
#[cfg(target_os = "macos")]
pub(super) fn exact_font(name: &str) -> Option<FontFile> {
    macos::resolve(name).or_else(|| {
        let path = match name.to_ascii_lowercase().as_str() {
            "sf mono" => "/System/Library/Fonts/SFNSMono.ttf",
            "sf pro" => "/System/Library/Fonts/SFNS.ttf",
            _ => return None,
        };
        let path = PathBuf::from(path);
        path.is_file().then(|| FontFile::first(path))
    })
}

/// 返回 CoreText 列出的字族名。
#[cfg(target_os = "macos")]
pub(super) fn macos_families() -> Vec<String> {
    macos::families()
}

/// 字族中匹配到的字体，以及实际取到的变体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FaceMatch {
    /// 字体所在的文件；非 macOS 平台不知道 PostScript 名，取文件中的第一个字体。
    pub file: FontFile,
    /// 实际取到的字体是否为粗体；字族里没有粗体时取到常规体，此时为假。
    pub bold: bool,
    /// 实际取到的字体是否为斜体；字族里没有斜体时取到常规体，此时为假。
    pub italic: bool,
}

/// 按字族名和粗体、斜体查找字体文件；字族不存在时返回 `None`。
#[cfg(target_os = "macos")]
pub(crate) fn resolve_face(family: &str, bold: bool, italic: bool) -> Option<FaceMatch> {
    macos::resolve_face(family, bold, italic)
}

/// 按字族名和粗体、斜体查找字体文件；字族不存在时返回 `None`。
///
/// macOS 以外的平台先按「字族 Bold Italic」这样的名字查找文件，没有则使用字族本身，此时
/// `bold`、`italic` 为假。
#[cfg(not(target_os = "macos"))]
pub(crate) fn resolve_face(family: &str, bold: bool, italic: bool) -> Option<FaceMatch> {
    let styled = match (bold, italic) {
        (true, true) => Some(format!("{family} Bold Italic")),
        (true, false) => Some(format!("{family} Bold")),
        (false, true) => Some(format!("{family} Italic")),
        (false, false) => None,
    };
    if let Some(file) = styled.and_then(|name| resolve_file_name(&name)) {
        return Some(FaceMatch { file, bold, italic });
    }
    resolve_file_name(family).map(|file| FaceMatch {
        file,
        bold: false,
        italic: false,
    })
}

/// 返回字体名 `name` 对应的字体文件：先按文件名查找，再按字族名查找（文件名与字族名无关的
/// 字体，例如 `msyh.ttc`），找不到时返回 `None`。
#[cfg(not(target_os = "macos"))]
fn resolve_name(name: &str) -> Option<FontFile> {
    resolve_file_name(name).or_else(|| find_family(name).map(|family| family.file.clone()))
}

/// 返回字体目录下的全部文件，每项是（规范化文件名，路径）。
///
/// 字体目录只在第一次调用时遍历一遍并建立索引，之后都使用这份索引。程序运行期间新安装的字体
/// 需要重启后才能找到。
#[cfg(not(target_os = "macos"))]
pub(super) fn font_index() -> &'static [(String, PathBuf)] {
    static INDEX: std::sync::OnceLock<Vec<(String, PathBuf)>> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut files = Vec::new();
        for dir in font_dirs() {
            walk(&dir, &mut files, 0);
        }
        // 同一名字命中多个文件时，按路径排序取第一个，使结果稳定。
        files.sort_by(|a, b| a.1.cmp(&b.1));
        files
    })
}

/// 返回文件名以字体名 `name` 开头的字体文件；找不到时返回 `None`。
#[cfg(not(target_os = "macos"))]
fn resolve_file_name(name: &str) -> Option<FontFile> {
    let target = normalize(name);
    font_index()
        .iter()
        .find(|(stem, _)| stem.starts_with(&target))
        .map(|(_, path)| FontFile::first(path.clone()))
}

/// 返回 Windows 的系统字体目录，即系统目录（`%SystemRoot%`，不一定在 C 盘）下的 `Fonts`。
pub(crate) fn windows_fonts() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from)
        .join("Fonts")
}

/// 返回要查找的字体目录，包括系统的和当前用户的。
#[cfg(not(target_os = "macos"))]
fn font_dirs() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        let mut dirs = vec![windows_fonts()];
        // 仅为当前用户安装的字体。
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
        return dirs;
    }
    // `/usr/share/fonts` 已经包含 `truetype` 等子目录，不再单独列出。
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    dirs.extend(data.map(|data| data.join("fonts")));
    dirs.extend(home.map(|home| home.join(".fonts")));
    dirs
}

/// 返回规范化后的字体名：去掉空格、连字符和下划线并转为小写。
///
/// 字体名中的空格、连字符和下划线不一定出现在文件名里，比较前要统一去掉。
pub(super) fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace([' ', '-', '_'], "")
}

/// 递归遍历 `dir`，把每个文件的（规范化文件名，路径）收集到 `out`。
///
/// 目录深度超过 4 层时停止遍历，以限制开销；4 层为经验值，覆盖常见字体目录的层级。
#[cfg(not(target_os = "macos"))]
fn walk(dir: &std::path::Path, out: &mut Vec<(String, PathBuf)>, depth: usize) {
    if depth > 4 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, depth + 1);
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        out.push((normalize(stem), path));
    }
}

/// CoreText 调用的封装。
///
/// CoreText 是外部函数与外部类型，使用约束和字体对象的生命周期都由系统管理。本模块只把字体
/// 对象当作句柄传递，不复制其内部数据；每个 `unsafe` 块上方的 `// SAFETY:` 注释写明前提。
#[cfg(target_os = "macos")]
// 本模块的 `unsafe` 仅用于调用 CoreText，不扩散到其他模块。
#[allow(unsafe_code)]
mod macos {
    use objc2_core_foundation::{CFArray, CFRetained, CFString, CFURL};
    use objc2_core_text::{
        CTFont, CTFontDescriptor, CTFontManagerCopyAvailableFontFamilyNames, CTFontSymbolicTraits,
        kCTFontNameAttribute, kCTFontURLAttribute,
    };

    use super::{FaceMatch, FontFile};

    /// 在字族存在的前提下，按粗体、斜体请求变体；字族不存在时返回 `None`。
    ///
    /// 同时要求粗体和斜体而找不到时，依次退为只要粗体、只要斜体。
    pub(super) fn resolve_face(family: &str, bold: bool, italic: bool) -> Option<FaceMatch> {
        let matched = lookup(family)?;
        // SAFETY: `matched` 是 `lookup` 返回的有效描述符，调用期间保持存活；矩阵参数传 null
        // 表示使用单位矩阵，`CTFontCreateWithFontDescriptor` 接受该值。
        let base = unsafe { CTFont::with_font_descriptor(&matched, 13.0, std::ptr::null()) };
        let want = |b: bool, i: bool| {
            let mut traits = CTFontSymbolicTraits::empty();
            if b {
                traits |= CTFontSymbolicTraits::TraitBold;
            }
            if i {
                traits |= CTFontSymbolicTraits::TraitItalic;
            }
            traits
        };
        let mut font = base.clone();
        for (b, i) in [(bold, italic), (bold, false), (false, italic)] {
            let traits = want(b, i);
            if traits.is_empty() {
                continue;
            }
            // SAFETY: `base` 是上文创建的有效 `CTFont`，整个调用期间保持存活；矩阵为 null 时
            // 使用单位矩阵。找不到变体时返回 `None`，返回值判空后才使用。
            let styled =
                unsafe { base.copy_with_symbolic_traits(0.0, std::ptr::null(), traits, traits) };
            if let Some(styled) = styled {
                font = styled;
                break;
            }
        }
        // SAFETY: 只读查询。`font` 在上面的循环里得到，在本函数内保持存活。
        let traits = unsafe { font.symbolic_traits() };
        // SAFETY: 只读查询。`font` 在本函数内保持存活；返回的描述符由调用方持有。
        let descriptor = unsafe { font.font_descriptor() };
        Some(FaceMatch {
            file: font_file(&descriptor)?,
            bold: traits.contains(CTFontSymbolicTraits::TraitBold),
            italic: traits.contains(CTFontSymbolicTraits::TraitItalic),
        })
    }

    /// 返回系统中已安装的字族名，名称可能是本地化的。
    pub(super) fn families() -> Vec<String> {
        // SAFETY: 该函数没有前提条件，返回的数组由调用方持有。文档写明元素都是 `CFString`，
        // 因此可以按 `CFArray<CFString>` 使用。
        let names: CFRetained<CFArray<CFString>> =
            unsafe { CFRetained::cast_unchecked(CTFontManagerCopyAvailableFontFamilyNames()) };
        names.iter().map(|name| name.to_string()).collect()
    }

    /// 用 CoreText 按名字查找字体文件；系统里没有该名字的字体时返回 `None`。
    pub(super) fn resolve(name: &str) -> Option<FontFile> {
        lookup(name).and_then(|descriptor| font_file(&descriptor))
    }

    /// 返回 CoreText 按名字选中的字体描述符；系统里没有该名字时返回 `None`。
    ///
    /// 名字可以是字族名（取常规体）、PostScript 名或本地化名称（如 `苹方-简`）。按名字创建字体
    /// （`CTFontCreateWithName`）在找不到时静默换成 Helvetica，匹配描述符则直接返回空，因此用
    /// 后者判断字体是否存在。以 `.` 开头的名字为系统保留，直接视为不存在。
    fn lookup(name: &str) -> Option<CFRetained<CTFontDescriptor>> {
        if name.is_empty() || name.starts_with('.') {
            return None;
        }
        let requested = CFString::from_str(name);
        // SAFETY: `requested` 是有效的 `CFString`，调用期间保持存活；字号为 0 时描述符中不含
        // 字号属性，`CTFontDescriptorCreateWithNameAndSize` 接受该值。
        let descriptor = unsafe { CTFontDescriptor::with_name_and_size(&requested, 0.0) };
        // SAFETY: `descriptor` 是上一行创建的有效描述符；不指定必须匹配的属性时传 null，
        // `CTFontDescriptorCreateMatchingFontDescriptor` 接受该值。
        unsafe { descriptor.matching_font_descriptor(None) }
    }

    /// 返回描述符对应的字体文件和 PostScript 名；文件不存在或没有该属性时返回 `None`。
    fn font_file(descriptor: &CTFontDescriptor) -> Option<FontFile> {
        // SAFETY: 只读查询；`descriptor` 在调用期间保持存活，属性缺失时返回 `None`。
        let url = unsafe { descriptor.attribute(kCTFontURLAttribute) }?;
        // 取文件系统路径，不取 URL 的路径部分：后者带百分号转义，含空格的文件名（例如
        // `Hiragino Sans GB.ttc`）会变成 `Hiragino%20Sans%20GB.ttc`，找不到文件。
        let path = url.downcast::<CFURL>().ok()?.to_file_path()?;
        // SAFETY: 只读查询；`descriptor` 在调用期间保持存活，属性缺失时 `attribute` 返回 `None`，
        // 返回的 `CFType` 由本函数持有并立即转成 `CFString`。
        let postscript = unsafe { descriptor.attribute(kCTFontNameAttribute) }
            .and_then(|name| name.downcast::<CFString>().ok())
            .map(|name| name.to_string());
        path.is_file().then_some(FontFile { path, postscript })
    }
}

#[cfg(test)]
mod tests;
