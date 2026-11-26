//! 字重：为字体找字重不同的同族字体和命名实例，以及按需派生的粗体槽位。
//!
//! 字重设置和粗体都经 [`Face::open_at`] 调整：有 `wght` 轴时沿轴调整，否则改用同目录中的同族
//! 字体（[`sibling`]），由系统提供轮廓的字体改用同族的命名实例（[`system_instance`]），达不到的
//! 部分加宽笔画。粗体槽位由常规槽位按需派生，见 [`Fonts::resolve_bold`]。

use super::*;

/// 在 `file` 所在的目录中找与 `font` 同族、同样式的字体：字重介于自身与「自身加 `shift`」
/// 之间，且最接近后者。返回该字体和实际得到的字重差。
///
/// 只打开文件名以 [`name_prefix`] 开头的字体文件，包括 `file` 自身；首次调用会列出该目录，
/// 耗时与目录中的文件数成正比。
pub(super) fn sibling(file: &FontFile, font: &FontRef, shift: f32) -> Option<(FontFile, f32)> {
    let family = weight_family(font)?;
    let (stretch, own, style) = font.attributes().parts();
    let own = f32::from(own.0);
    let wanted = own + shift;
    let prefix = name_prefix(&file.path)?;
    let mut best: Option<(FontFile, f32)> = None;
    for entry in std::fs::read_dir(file.path.parent()?).ok()?.flatten() {
        let path = entry.path();
        let is_font = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| e.eq_ignore_ascii_case(known))
        });
        let named = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().to_lowercase().starts_with(&prefix));
        if !is_font || !named {
            continue;
        }
        let Some(data) = load(&path) else {
            continue;
        };
        let count = swash::FontDataRef::new(&data).map_or(1, |d| d.len());
        for (index, other) in
            (0..count).filter_map(|i| FontRef::from_index(&data, i).map(|font| (i, font)))
        {
            let (other_stretch, weight, other_style) = other.attributes().parts();
            let weight = f32::from(weight.0);
            let between = weight != own && weight >= own.min(wanted) && weight <= own.max(wanted);
            let closer = best
                .as_ref()
                .is_none_or(|(_, reached)| (wanted - weight).abs() < (shift - reached).abs());
            if between
                && closer
                && (other_stretch, other_style) == (stretch, style)
                && weight_range(&other).is_none()
                && weight_family(&other).as_deref() == Some(family.as_str())
            {
                let found = FontFile {
                    path: path.clone(),
                    postscript: postscript_name(&other),
                    index: (count > 1).then_some(index as u32),
                };
                best = Some((found, weight - own));
            }
        }
    }
    best
}

/// 查找同族字体时比较的文件名前缀的最短长度（字符）。
///
/// 同族各字重的文件名开头相同（如 `segoeui.ttf` 与 `seguisb.ttf`），据此不必打开目录中的
/// 每个字体文件。
const SIBLING_PREFIX: usize = 3;

/// 返回同族字体的文件名共有的前缀（小写）；文件名不足 [`SIBLING_PREFIX`] 个字符时返回 `None`。
///
/// 文件名含 `-` 时取到第一个 `-` 为止（`JetBrainsMono-Regular.ttf` 取 `jetbrainsmono-`），不包括
/// 同一目录中前缀更长的其他字族；否则取前 [`SIBLING_PREFIX`] 个字符。
pub(super) fn name_prefix(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let short: String = name.chars().take(SIBLING_PREFIX).collect();
    if short.chars().count() < SIBLING_PREFIX {
        return None;
    }
    let family = name.find('-').map(|at| &name[..=at]);
    Some(
        family
            .filter(|family| family.len() > short.len())
            .map_or(short, str::to_owned),
    )
}

/// 返回字族名，优先取把各字重归为一族的排版字族名。
fn weight_family(font: &FontRef) -> Option<String> {
    let strings = font.localized_strings();
    let name = strings
        .find_by_id(StringId::TypographicFamily, None)
        .or_else(|| strings.find_by_id(StringId::Family, None))?;
    Some(name.to_string())
}

/// 由系统提供轮廓的字体中的一个命名实例，见 [`system_instance`]。
pub(super) struct SystemInstance<'a> {
    /// 实例所在的字体。
    pub(super) font: FontRef<'a>,
    /// 实例的 PostScript 名，系统按该名字提供轮廓。
    pub(super) postscript: String,
    /// 实例的归一化坐标。
    pub(super) coords: Vec<i16>,
    /// 实例相对原字体的字重差（字重单位）。
    pub(super) gained: f32,
}

/// 在由系统提供轮廓的字体文件 `data` 中，找与 PostScript 名为 `postscript` 的字体同族的命名
/// 实例：字重介于 `own` 与「`own` 加 `shift`」之间，且最接近后者。
///
/// 同族指 PostScript 名中第一个 `-` 之前的部分相同（`PingFangSC-Regular` 与
/// `PingFangSC-Semibold`）；没有 `wght` 轴的字体不参与比较。
pub(super) fn system_instance<'a>(
    data: &'a [u8],
    postscript: &str,
    own: f32,
    shift: f32,
) -> Option<SystemInstance<'a>> {
    let family = |name: &str| name.split('-').next().unwrap_or_default().to_owned();
    let wanted_family = family(postscript);
    let wanted = own + shift;
    let count = swash::FontDataRef::new(data).map_or(1, |d| d.len());
    let mut best: Option<SystemInstance<'a>> = None;
    for font in (0..count).filter_map(|i| FontRef::from_index(data, i)) {
        let Some(axis) = font.variations().position(|v| v.tag() == WGHT) else {
            continue;
        };
        for instance in font.instances() {
            let Some(name) = instance_postscript(&font, instance.index()) else {
                continue;
            };
            let Some(weight) = instance.values().nth(axis) else {
                continue;
            };
            let between = weight != own && weight >= own.min(wanted) && weight <= own.max(wanted);
            let closer = best
                .as_ref()
                .is_none_or(|best| (wanted - weight).abs() < (shift - best.gained).abs());
            if between && closer && family(&name) == wanted_family {
                best = Some(SystemInstance {
                    font,
                    postscript: name,
                    coords: instance.normalized_coords().collect(),
                    gained: weight - own,
                });
            }
        }
    }
    best
}

impl Fonts {
    /// 返回粗体字符 `c` 的（槽位，字形 id，是否加宽笔画模拟粗体）。
    ///
    /// 字符所在槽位的字体能变粗时使用派生的粗体（见 [`Self::bold_of`]）。字体无法变粗或粗体中
    /// 没有该字符时仍用原槽位并加宽笔画；彩色 emoji 和缺字（字形 id 为 0）不加粗。
    pub(crate) fn resolve_bold(&mut self, kind: FontKind, c: char) -> (u8, u16, bool) {
        let kind = self.effective(kind);
        if (c as u32) < 128
            && let Some(hit) = self.ascii_bold[kind as usize][c as usize]
        {
            return hit;
        }
        if let Some(&hit) = self.resolved_bold.get(&(kind, c)) {
            return hit;
        }
        let (slot, glyph) = self.resolve(kind, c);
        let result = if glyph == 0 || self.is_emoji_slot(slot) {
            (slot, glyph, false)
        } else if let Some(bold) = self.bold_of(slot)
            && let Some(face) = self.face(bold)
            && let bold_glyph = face.font().charmap().map(c)
            && bold_glyph != 0
        {
            (bold, bold_glyph, false)
        } else {
            (slot, glyph, true)
        };
        if (c as u32) < 128 {
            self.ascii_bold[kind as usize][c as usize] = Some(result);
        } else {
            // 与 `resolved` 相同，满了就整体清空。
            if self.resolved_bold.len() >= RESOLVED_ENTRIES {
                self.resolved_bold = FastMap::default();
            }
            self.resolved_bold.insert((kind, c), result);
        }
        result
    }

    /// 返回槽位 `slot` 派生的粗体所在的槽位，首次调用时打开；字体自身无法变粗时返回 `None`。
    ///
    /// 粗体按槽位的字重偏移再加 [`BOLD_STEP`] 打开（见 [`Face::open_at`]），字体自身提供的字重比
    /// 原槽位多时才采用，达不到的部分加宽笔画。首次调用可能列出字体所在的目录，见 [`sibling`]。
    fn bold_of(&mut self, slot: u8) -> Option<u8> {
        let Slot::Ready(loaded) = self.slots.get(slot as usize)? else {
            return None;
        };
        match loaded.bold {
            Bold::Slot(bold) => return Some(bold),
            Bold::Unavailable => return None,
            Bold::Untried => {}
        }
        let slanted = loaded.slanted || italic_slot(slot);
        // 槽位编号是 `u8`：编号用完时不再派生。
        let room = self.slots.len() <= usize::from(u8::MAX);
        let face = loaded
            .origin
            .as_ref()
            .filter(|_| room)
            .and_then(|(file, shift)| Face::open_at(file, shift.saturating_add(BOLD_STEP as i16)))
            .filter(|face| face.gained > loaded.face.gained);
        let bold = match face {
            Some(face) => {
                let index = self.slots.len() as u8;
                self.slots.push(Slot::Ready(Loaded {
                    face,
                    origin: None,
                    bold: Bold::Unavailable,
                    slanted,
                }));
                Bold::Slot(index)
            }
            None => Bold::Unavailable,
        };
        if let Some(Slot::Ready(loaded)) = self.slots.get_mut(slot as usize) {
            loaded.bold = bold;
        }
        match bold {
            Bold::Slot(index) => Some(index),
            _ => None,
        }
    }
}
