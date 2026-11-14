//! PDF 的目录（书签，`/Outlines`）：各条的标题、层级和指向的页。
//!
//! 直接读文件里的对象（见 [`super::objects`]），不经过系统的 PDF 渲染器：Windows 的渲染器不提供
//! 目录，各平台用同一份实现。文件损坏、加密或没有目录时返回空的目录，预览照常进行。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::objects::{File, Obj};

/// 目录最多列出的条数：再多的目录在侧栏里也翻不过来，防止伪造的文件让列表无限长。
const ITEM_LIMIT: usize = 10_000;
/// 目录最多几层：正常的书最多六七层，防止伪造的文件嵌套过深。
const DEPTH_LIMIT: usize = 32;
/// 按名字查找目标位置时名字树最多下钻几层。
const TREE_LIMIT: usize = 32;

/// 目录的一条。
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    /// 标题。
    pub title: String,
    /// 层级：顶层为 0。
    pub level: usize,
    /// 指向的页（从 0 起）；指向别的文件、网址，或目标读不出来时为 `None`。
    pub page: Option<usize>,
    /// 指向页内的哪里：目标位置到页面上沿的距离与页高之比（0 到 1）；只指向整页时为 `None`。
    pub top: Option<f32>,
}

/// 读出 PDF 文件 `path` 的目录，按出现的顺序排列；读不出来时为空。
pub fn read_outline(path: &Path) -> Vec<OutlineItem> {
    winc_backend::fs::read(path).map_or_else(|_| Vec::new(), |bytes| outline(&bytes))
}

/// 读出 PDF 文件内容 `bytes` 的目录。
pub fn outline(bytes: &[u8]) -> Vec<OutlineItem> {
    let Some(file) = File::open(bytes) else {
        return Vec::new();
    };
    // 加密的文件里标题也是加密的：不读。
    if file.trailer.get(b"Encrypt").is_some() {
        return Vec::new();
    }
    let Some(catalog) = file.resolve(file.trailer.get(b"Root")) else {
        return Vec::new();
    };
    let reader = Reader {
        pages: page_numbers(&file, &catalog),
        file: &file,
        catalog: &catalog,
    };
    let mut items = Vec::new();
    let first = file
        .resolve(catalog.get(b"Outlines"))
        .and_then(|outlines| outlines.get(b"First").cloned());
    reader.siblings(first, 0, &mut items, &mut HashSet::new());
    items
}

/// 读目录时用到的文件、根对象和各页的对象号。
struct Reader<'a> {
    file: &'a File<'a>,
    catalog: &'a Obj,
    /// 页对象的对象号到页序号（从 0 起）和页框。
    pages: HashMap<u32, Page>,
}

/// 页树里的一页：序号（从 0 起），显示的页框（`/CropBox`，没有时为 `/MediaBox`）的上下沿和
/// 旋转的角度；页框读不出来时上下沿为 `None`。
#[derive(Debug, Clone, Copy)]
struct Page {
    index: usize,
    edges: Option<(f64, f64)>,
    rotate: i64,
}

impl Page {
    /// 页面坐标里高 `y` 处到显示出来的上沿的距离与页高之比；横着放的页不按高度对应，为 `None`。
    fn top(&self, y: f64) -> Option<f32> {
        let (bottom, top) = self.edges?;
        let height = top - bottom;
        let from_top = match self.rotate {
            0 => top - y,
            180 => y - bottom,
            _ => return None,
        };
        (height > 0.0).then(|| (from_top / height).clamp(0.0, 1.0) as f32)
    }
}

impl Reader<'_> {
    /// 从 `first` 起沿 `/Next` 读完同一层的各条，有下级的接着读下级；`seen` 防止循环引用。
    fn siblings(
        &self,
        first: Option<Obj>,
        level: usize,
        items: &mut Vec<OutlineItem>,
        seen: &mut HashSet<u32>,
    ) {
        let mut next = first;
        while let Some(Obj::Ref(n, _)) = next {
            if level > DEPTH_LIMIT || items.len() >= ITEM_LIMIT || !seen.insert(n) {
                return;
            }
            let Some(item) = self.file.object(n) else {
                return;
            };
            let title = item.get(b"Title").map_or_else(String::new, |t| match t {
                Obj::Str(bytes) => text(bytes),
                other => self
                    .file
                    .resolve(Some(other))
                    .map_or_else(String::new, |t| match t {
                        Obj::Str(bytes) => text(&bytes),
                        _ => String::new(),
                    }),
            });
            let (page, top) = self.target(&item).unzip();
            items.push(OutlineItem {
                title,
                level,
                page,
                top: top.flatten(),
            });
            self.siblings(item.get(b"First").cloned(), level + 1, items, seen);
            next = item.get(b"Next").cloned();
        }
    }

    /// 一条目录指向的页和页内位置（见 [`OutlineItem::top`]）：`/Dest`，或者 `/A` 里的跳转
    /// （`/S /GoTo`）。
    fn target(&self, item: &Obj) -> Option<(usize, Option<f32>)> {
        if let Some(dest) = self.file.resolve(item.get(b"Dest")) {
            return self.destination(&dest, 0);
        }
        let action = self.file.resolve(item.get(b"A"))?;
        match action.get(b"S") {
            Some(Obj::Name(kind)) if kind == b"GoTo" => {
                let dest = self.file.resolve(action.get(b"D"))?;
                self.destination(&dest, 0)
            }
            _ => None,
        }
    }

    /// 目标位置指向的页和页内位置：`[页 /XYZ …]` 这样的数组、带 `/D` 的字典，或者按名字查到的
    /// 这两种。
    fn destination(&self, dest: &Obj, depth: usize) -> Option<(usize, Option<f32>)> {
        if depth > 2 {
            return None;
        }
        match dest {
            Obj::Array(parts) => {
                let page = match parts.first()? {
                    Obj::Ref(n, _) => *self.pages.get(n)?,
                    // 少数文件直接写页序号：不知道页框，只到整页。
                    Obj::Int(n) => {
                        let index = usize::try_from(*n).ok().filter(|&n| n < self.pages.len())?;
                        return Some((index, None));
                    }
                    _ => return None,
                };
                // 各种方式里表示上沿的那一项：`/XYZ 左 上 缩放`、`/FitH 上`、`/FitBH 上`、
                // `/FitR 左 下 右 上`；`/Fit`、`/FitV` 只到整页，上沿为 null 时也是。
                let at = match parts.get(1) {
                    Some(Obj::Name(kind)) => match kind.as_slice() {
                        b"XYZ" => 3,
                        b"FitH" | b"FitBH" => 2,
                        b"FitR" => 5,
                        _ => 0,
                    },
                    _ => 0,
                };
                let y = (at > 0).then(|| parts.get(at).and_then(number)).flatten();
                Some((page.index, y.and_then(|y| page.top(y))))
            }
            Obj::Dict(_) => {
                let inner = self.file.resolve(dest.get(b"D"))?;
                self.destination(&inner, depth + 1)
            }
            Obj::Name(name) | Obj::Str(name) => {
                let found = self.named(name)?;
                self.destination(&found, depth + 1)
            }
            _ => None,
        }
    }

    /// 按名字查目标位置：先查根对象的 `/Names /Dests` 名字树，再查旧式的 `/Dests` 字典。
    fn named(&self, name: &[u8]) -> Option<Obj> {
        let names = self.file.resolve(self.catalog.get(b"Names"));
        let tree = names.and_then(|names| self.file.resolve(names.get(b"Dests")));
        if let Some(found) = tree.and_then(|tree| self.lookup(&tree, name, 0)) {
            return Some(found);
        }
        let dests = self.file.resolve(self.catalog.get(b"Dests"))?;
        self.file.resolve(dests.get(name))
    }

    /// 在名字树的节点 `node` 下查 `name`：叶子的 `/Names` 是 `[名字 值 …]`，中间节点的 `/Kids`
    /// 按 `/Limits` 下钻。
    fn lookup(&self, node: &Obj, name: &[u8], depth: usize) -> Option<Obj> {
        if depth > TREE_LIMIT {
            return None;
        }
        if let Some(Obj::Array(pairs)) = self.file.resolve(node.get(b"Names")) {
            for [key, value] in pairs.as_chunks::<2>().0 {
                if matches!(key, Obj::Str(key) if key == name) {
                    return self.file.resolve(Some(value));
                }
            }
        }
        let Some(Obj::Array(kids)) = self.file.resolve(node.get(b"Kids")) else {
            return None;
        };
        for kid in &kids {
            let Some(kid) = self.file.resolve(Some(kid)) else {
                continue;
            };
            // 不在这个节点的名字范围里：不下钻（没有 `/Limits` 时照样找）。
            if let Some(Obj::Array(limits)) = self.file.resolve(kid.get(b"Limits"))
                && let [Obj::Str(low), Obj::Str(high)] = &limits[..]
                && (name < low.as_slice() || name > high.as_slice())
            {
                continue;
            }
            if let Some(found) = self.lookup(&kid, name, depth + 1) {
                return Some(found);
            }
        }
        None
    }
}

/// 页树（`/Pages`）里各页对象的对象号到页，按阅读顺序编号；页框和旋转角度可以写在上级节点
/// 上，由下级继承。
fn page_numbers(file: &File<'_>, catalog: &Obj) -> HashMap<u32, Page> {
    let mut pages = HashMap::new();
    let mut seen = HashSet::new();
    let root = Inherited {
        media: None,
        crop: None,
        rotate: 0,
    };
    let mut stack = vec![(catalog.get(b"Pages").cloned(), 0usize, root)];
    // 先序遍历：子节点倒序入栈，出栈时恰好按 `/Kids` 的顺序。
    while let Some((node, depth, above)) = stack.pop() {
        let Some(Obj::Ref(n, _)) = node else {
            continue;
        };
        if depth > DEPTH_LIMIT || !seen.insert(n) {
            continue;
        }
        let Some(object) = file.object(n) else {
            continue;
        };
        let here = Inherited {
            media: page_box(file, object.get(b"MediaBox")).or(above.media),
            crop: page_box(file, object.get(b"CropBox")).or(above.crop),
            rotate: (file.resolve(object.get(b"Rotate")).and_then(|r| r.int()))
                .map_or(above.rotate, |r| r.rem_euclid(360)),
        };
        match file.resolve(object.get(b"Kids")) {
            Some(Obj::Array(kids)) => {
                stack.extend((kids.into_iter().rev()).map(|kid| (Some(kid), depth + 1, here)));
            }
            _ => {
                let page = Page {
                    index: pages.len(),
                    edges: here.crop.or(here.media),
                    rotate: here.rotate,
                };
                pages.insert(n, page);
            }
        }
    }
    pages
}

/// 页树节点传给下级的页框（各自的下沿、上沿）和旋转角度。
#[derive(Debug, Clone, Copy)]
struct Inherited {
    media: Option<(f64, f64)>,
    crop: Option<(f64, f64)>,
    rotate: i64,
}

/// 页框 `[左 下 右 上]` 的下沿和上沿；读不出来时为 `None`。
fn page_box(file: &File<'_>, value: Option<&Obj>) -> Option<(f64, f64)> {
    let Some(Obj::Array(corners)) = file.resolve(value) else {
        return None;
    };
    let [_, low, _, high] = &corners[..] else {
        return None;
    };
    let (low, high) = (number(low)?, number(high)?);
    Some((low.min(high), low.max(high)))
}

/// 数值（整数或实数）。
fn number(value: &Obj) -> Option<f64> {
    match value {
        Obj::Int(n) => Some(*n as f64),
        Obj::Real(n) => Some(*n),
        _ => None,
    }
}

/// PDF 的文本字符串：开头是 `FE FF` 时为 UTF-16BE，`EF BB BF` 时为 UTF-8，否则按
/// `PDFDocEncoding`；换行等控制字符换成空格，首尾空白去掉。
pub(super) fn text(bytes: &[u8]) -> String {
    let decoded = if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| u16::from_be_bytes(p));
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| u16::from_le_bytes(p));
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Ok(utf8) = std::str::from_utf8(bytes) {
        // 不少生成工具直接写 UTF-8 而不加标记；纯 ASCII 两种编码相同。
        utf8.to_owned()
    } else {
        bytes.iter().map(|&b| doc_char(b)).collect()
    };
    let spaced: String = (decoded.chars())
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    spaced.trim().to_owned()
}

/// `PDFDocEncoding` 的一个字节：0x80 到 0xA0 是专门的符号，其余与 Latin-1 相同。
fn doc_char(b: u8) -> char {
    const HIGH: [char; 33] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}',
        '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}',
        '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}',
        '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}',
        '\u{0153}', '\u{0161}', '\u{017E}', '\u{FFFD}', '\u{20AC}',
    ];
    match b {
        0x80..=0xA0 => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests;
