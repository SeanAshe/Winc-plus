//! 已安装的字体：设置中的字体菜单从这里取字族，按名字找不到字体时也在这里按宽松的写法再找一次。
//!
//! macOS 向 CoreText 查询字族；其他平台读取字体目录下每个字体文件的名字表。

use std::sync::OnceLock;

use swash::FontRef;
#[cfg(any(not(target_os = "macos"), test))]
use swash::{Style, Weight};

use super::font::{load, select};
use super::font_path::{FontFile, normalize};

/// 一个已安装的字族。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamily {
    /// 字族名，可以直接作为字体设置的值。
    pub name: String,
    /// 是否为等宽字体。
    pub monospace: bool,
    /// 字族中常规体所在的文件。
    pub(crate) file: FontFile,
}

/// 返回已安装的字族，按名字排序（不区分大小写）。
///
/// 第一次调用时读取字体文件，字体多时耗时明显，应先在后台线程调用一次；之后直接返回结果。程序
/// 运行期间新安装的字体需要重启后才能列出。
pub fn installed_fonts() -> &'static [FontFamily] {
    static FONTS: OnceLock<Vec<FontFamily>> = OnceLock::new();
    FONTS.get_or_init(|| sorted(list()))
}

/// 返回规范化后的名字（见 [`normalize`]）与 `name` 相同的字族；没有时返回 `None`。
pub(crate) fn find_family(name: &str) -> Option<&'static FontFamily> {
    let target = normalize(name);
    if target.is_empty() {
        return None;
    }
    installed_fonts()
        .iter()
        .find(|family| normalize(&family.name) == target)
}

/// 按名字排序；规范化后相同的名字只留一个。
fn sorted(mut families: Vec<FontFamily>) -> Vec<FontFamily> {
    families.sort_by_cached_key(|family| (family.name.to_lowercase(), family.name.clone()));
    families.dedup_by_key(|family| normalize(&family.name));
    families
}

/// 返回 CoreText 列出的字族，以及它不列出但可以按名字使用的系统字体；以 `.` 开头的名字是系统保留
/// 的，不列出。
#[cfg(target_os = "macos")]
fn list() -> Vec<FontFamily> {
    use super::font_path::{exact_font, macos_families};

    macos_families()
        .into_iter()
        .chain(["SF Mono".to_owned(), "SF Pro".to_owned()])
        .filter(|name| !name.starts_with('.'))
        .filter_map(|name| {
            let file = exact_font(&name)?;
            let monospace = file_is_monospace(&file);
            Some(FontFamily {
                name,
                monospace,
                file,
            })
        })
        .collect()
}

/// 返回 `file` 是否为等宽字体；无法读取时返回 `false`。
#[cfg(target_os = "macos")]
fn file_is_monospace(file: &FontFile) -> bool {
    let Some(data) = load(&file.path) else {
        return false;
    };
    select(&data, file.postscript.as_deref()).is_some_and(|selected| is_monospace(&selected.font))
}

/// 返回字体目录下各字体文件中的字族。
#[cfg(not(target_os = "macos"))]
fn list() -> Vec<FontFamily> {
    let paths = super::font_path::font_index().iter().map(|(_, path)| path);
    scan(paths)
}

/// 读取 `paths` 中各字体文件的名字表，返回其中的字族；字族中有多个字体时取最接近常规体的一个。
///
/// 不是字体或画不出字形的文件跳过。
#[cfg(any(not(target_os = "macos"), test))]
fn scan<'a>(paths: impl Iterator<Item = &'a std::path::PathBuf>) -> Vec<FontFamily> {
    use std::collections::HashMap;

    let mut found: HashMap<String, (u32, FontFamily)> = HashMap::new();
    for path in paths {
        let is_font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| e.eq_ignore_ascii_case(known))
        });
        if !is_font {
            continue;
        }
        let Some(data) = load(path) else {
            continue;
        };
        let count = swash::FontDataRef::new(&data).map_or(0, |d| d.len());
        for font in (0..count).filter_map(|i| FontRef::from_index(&data, i)) {
            let Some(name) = family_name(&font).filter(|n| !n.starts_with('.')) else {
                continue;
            };
            let postscript = super::font::postscript_name(&font);
            let usable = select(&data, postscript.as_deref()).is_some();
            if !usable {
                continue;
            }
            let distance = distance_from_regular(&font);
            let family = FontFamily {
                name: name.clone(),
                monospace: is_monospace(&font),
                file: FontFile {
                    path: path.clone(),
                    postscript,
                },
            };
            let key = normalize(&name);
            let closer = found.get(&key).is_none_or(|(best, _)| distance < *best);
            if closer {
                found.insert(key, (distance, family));
            }
        }
    }
    found.into_values().map(|(_, family)| family).collect()
}

/// 返回字体所属的字族名：优先取把各字重归为一族的排版字族名。
#[cfg(any(not(target_os = "macos"), test))]
fn family_name(font: &FontRef) -> Option<String> {
    let strings = font.localized_strings();
    let name = strings
        .find_by_id(swash::StringId::TypographicFamily, None)
        .or_else(|| strings.find_by_id(swash::StringId::Family, None))?;
    let name: String = name.chars().collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// 返回字体与常规体的差距，越小越接近：斜体的差距大于任何字重的差距。
#[cfg(any(not(target_os = "macos"), test))]
fn distance_from_regular(font: &FontRef) -> u32 {
    let (_, weight, style) = font.attributes().parts();
    let weight = u32::from(weight.0.abs_diff(Weight::NORMAL.0));
    match style {
        Style::Normal => weight,
        _ => weight + 1000,
    }
}

/// 返回字体是否为等宽字体。
///
/// 字体声明的等宽标志不可靠（含中文字形的等宽字体常常不设置），因此同时比较窄字母和宽字母的
/// 字宽。
fn is_monospace(font: &FontRef) -> bool {
    if font.metrics(&[]).is_monospace {
        return true;
    }
    let charmap = font.charmap();
    let glyphs = font.glyph_metrics(&[]);
    let [narrow, wide] = ['i', 'W'].map(|c| glyphs.advance_width(charmap.map(c)));
    charmap.map('i') != 0 && charmap.map('W') != 0 && narrow > 0.0 && narrow == wide
}

#[cfg(test)]
mod tests;
