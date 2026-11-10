//! 行内文字的排版：拆成一个个字、量宽度、按规则换行，再按样式合成一段段文字。
//!
//! [`Cx::flow_at`] 排一段行内文字（段落、标题、表格单元格都走这里），图片、行内公式、
//! 行内代码的留白各是一个不可再分的单位；换行规则见 [`break_lines`]。坐标和尺寸都是逻辑像素。

use std::path::PathBuf;
use std::sync::Arc;

use winc_preview::markdown::{Align, IMAGE_MARK, Inline, Length, Style};
use winc_render::{Color, Faux, FontKind, Rect};

use super::math::MathBox;
use super::{Cx, Flow, Item, Para, Span};
use crate::ImageSlot;

/// 行内文字拆出的一个单位（一个字、图片、行内代码两边的留白）。
struct Atom {
    kind: AtomKind,
    /// 宽度（逻辑像素）。
    width: f32,
    /// 在样式表（`atoms` 返回的第二项）中的序号。
    run: usize,
    /// 这一个前面能不能换行。
    brk: bool,
    /// 是否空格：折行时行首的空格去掉，行尾的空格不算进底色和链接范围。
    space: bool,
    /// 在原文中的位置（字节偏移，相对顶层块）；由原文生成的文字为 `None`。
    source: Option<u32>,
}

enum AtomKind {
    /// 一个字符（制表符已换成空格）。
    Char(char),
    /// 行内代码两边的留白。
    Pad,
    /// 行内图片。
    Image {
        /// 图片文件的路径。
        path: PathBuf,
        /// 显示高度（逻辑像素），宽度记在同名的 `width` 字段。
        h: f32,
    },
    /// 行内公式：排好的结果和原文（复制、查找用）。
    Math { b: Arc<MathBox>, tex: Arc<str> },
    /// 原文里的硬换行（`<br>`、行尾两个空格）。
    Newline,
}

/// 一段样式解析好以后的字体、字号和颜色。
#[derive(Clone, Copy)]
struct RunStyle {
    /// 字体（正文用界面字体，行内代码用等宽字体）。
    font: FontKind,
    /// 模拟粗体、斜体。
    faux: Faux,
    /// 字号（逻辑像素）。
    size: f32,
    /// 文字颜色。
    color: Color,
    /// 样式位（`Style` 的位或）：决定底色、下划线、删除线、链接。
    flags: u16,
    /// 链接在 `Inline::links` 中的序号，从 1 起；不是链接时为 0。
    link: u16,
    /// 基线上移多少（脚注编号是上标）。
    rise: f32,
}

impl Cx<'_, '_> {
    fn style_of(&self, style: Style, para: Para) -> RunStyle {
        let code = style.has(Style::CODE);
        let note = style.has(Style::NOTE);
        let (sup, sub) = (style.has(Style::SUP), style.has(Style::SUB));
        let size = if code {
            para.size * 0.85
        } else if note || sup || sub {
            para.size * 0.75
        } else if style.has(Style::SMALL) {
            para.size * 0.85
        } else {
            para.size
        };
        RunStyle {
            font: if code { FontKind::Mono } else { FontKind::Ui },
            faux: Faux {
                bold: !code && (para.bold || style.has(Style::STRONG)),
                italic: style.has(Style::EMPHASIS),
            },
            size,
            color: if style.has(Style::SYNTAX) {
                self.colors.muted
            } else if style.link != 0 || note {
                self.colors.link
            } else {
                para.color
            },
            flags: style.flags,
            link: style.link,
            rise: if note || sup {
                para.size * 0.35
            } else if sub {
                -para.size * 0.15
            } else {
                0.0
            },
        }
    }

    /// 返回图片 `url` 的本地路径（网址的图片是下载到的缓存文件）和显示大小（逻辑像素）。
    ///
    /// 无法显示（读不了、下载不了、不让联网）时返回 `None`；还在读时大小为 0。
    fn image(
        &mut self,
        image: &winc_preview::markdown::Image,
        room: f32,
    ) -> Option<(PathBuf, f32, f32)> {
        // `<picture>` 给深色外观配了图时，深色主题用它。
        let url = match &image.dark {
            Some(dark) if self.dark => dark,
            _ => &image.url,
        };
        let path = (self.image_path)(url)?;
        let picture = match self.images.get(&path) {
            Some(ImageSlot::Ready { picture, .. }) => picture,
            Some(ImageSlot::Failed) => return None,
            Some(ImageSlot::Loading) => return Some((path, 0.0, 0.0)),
            None => {
                self.wanted.push((path.clone(), url.clone()));
                return Some((path, 0.0, 0.0));
            }
        };
        let (w, h) = (picture.width.max(1) as f32, picture.height.max(1) as f32);
        // 百分比按一行的宽度算（README 里 `width="49.45%"` 的几张图并排）。
        // 只量宽度、不限行宽时（`room` 无穷大）按原图大小。
        let width = image.width.and_then(|width| match width {
            Length::Px(px) => Some(px),
            Length::Percent(p) if room.is_finite() => {
                Some((room * p as f32 / 10000.0).floor().max(1.0) as u32)
            }
            Length::Percent(_) => None,
        });
        // HTML 写了宽或高时按它，只写其中一项时另一项按比例计算。
        let (w, h) = match (width, image.height) {
            (Some(ww), Some(hh)) => (ww as f32, hh as f32),
            (Some(ww), None) => (ww as f32, h * ww as f32 / w),
            (None, Some(hh)) => (w * hh as f32 / h, hh as f32),
            (None, None) => (w, h),
        };
        // 不超过一行的宽度，按比例缩小。
        let fit = (room / w).min(1.0);
        Some((path, (w * fit).floor().max(1.0), (h * fit).floor().max(1.0)))
    }

    /// 把行内文字拆成一个个字，量好宽度、标好哪里能换行。
    fn atoms(&mut self, inline: &Inline, para: Para, room: f32) -> (Vec<Atom>, Vec<RunStyle>) {
        let mut atoms: Vec<Atom> = Vec::with_capacity(inline.text.len());
        let mut runs = Vec::new();
        let mut images = inline.images.iter();
        for (range, style) in inline.runs() {
            let run = runs.len();
            let st = self.style_of(style, para);
            runs.push(st);
            if style.has(Style::MATH) {
                let tex = &inline.text[range.clone()];
                let b = self.math(tex, st.size, false, st.color);
                let width = b.w;
                atoms.push(Atom {
                    kind: AtomKind::Math {
                        b: Arc::new(b),
                        tex: tex.into(),
                    },
                    width,
                    run,
                    brk: true,
                    space: false,
                    source: source_of(inline, range.start),
                });
                continue;
            }
            let code = style.has(Style::CODE);
            let pad = st.size * 0.4;
            if code {
                atoms.push(Atom {
                    kind: AtomKind::Pad,
                    width: pad,
                    run,
                    brk: true,
                    space: false,
                    source: None,
                });
            }
            let start = range.start;
            for (i, c) in inline.text[range].char_indices() {
                let source = source_of(inline, start + i);
                if c == IMAGE_MARK {
                    let Some(image) = images.next() else { continue };
                    match self.image(image, room) {
                        Some((path, w, h)) => atoms.push(Atom {
                            kind: AtomKind::Image { path, h },
                            width: w,
                            run,
                            brk: true,
                            space: false,
                            source,
                        }),
                        // 无法显示时用替代文字。
                        None => {
                            let alt = if image.alt.is_empty() {
                                winc_i18n::t!("preview-markdown-image-alt")
                            } else {
                                &image.alt
                            };
                            for c in alt.chars() {
                                self.push_char(&mut atoms, c, run, st, source);
                            }
                        }
                    }
                    continue;
                }
                if c == '\n' {
                    atoms.push(Atom {
                        kind: AtomKind::Newline,
                        width: 0.0,
                        run,
                        brk: true,
                        space: false,
                        source,
                    });
                    continue;
                }
                self.push_char(&mut atoms, c, run, st, source);
            }
            if code {
                atoms.push(Atom {
                    kind: AtomKind::Pad,
                    width: pad,
                    run,
                    brk: false,
                    space: false,
                    source: None,
                });
            }
        }
        // 图片后面可以换行。
        for i in 1..atoms.len() {
            if matches!(
                atoms[i - 1].kind,
                AtomKind::Image { .. } | AtomKind::Math { .. }
            ) {
                atoms[i].brk = true;
            }
        }
        (atoms, runs)
    }

    fn push_char(
        &mut self,
        atoms: &mut Vec<Atom>,
        c: char,
        run: usize,
        st: RunStyle,
        source: Option<u32>,
    ) {
        let c = if c == '\t' { ' ' } else { c };
        let width = self.frame.advance_faux(st.font, st.faux, st.size, c);
        let prev = atoms.last().and_then(|a| match a.kind {
            AtomKind::Char(p) => Some(p),
            _ => None,
        });
        let brk = match prev {
            Some(p) => can_break(p, c),
            None => !atoms.is_empty(),
        };
        atoms.push(Atom {
            kind: AtomKind::Char(c),
            width,
            run,
            brk,
            space: is_space(c),
            source,
        });
    }

    /// 排一段行内文字：左上角在 `(x, y)`，最宽 `width`。
    ///
    /// 不经过缓存，见 [`Cx::flow`]。
    pub(crate) fn flow_at(
        &mut self,
        inline: &Inline,
        x: f32,
        y: f32,
        width: f32,
        para: Para,
        out: &mut Vec<Item>,
    ) -> Flow {
        let (atoms, runs) = self.atoms(inline, para, width);
        let lines = break_lines(&atoms, width);
        // 没有文字的段落（只有记号的列表项）仍记下原文位置，可编辑的预览中光标落在该行。
        // 光标所在的空段落按原文重新生成，没有对照，取段落的起点。
        let anchor = atoms
            .is_empty()
            .then(|| source_of(inline, 0).or_else(|| u32::try_from(inline.source).ok()))
            .flatten();
        let mut top = y;
        let mut first_baseline = None;
        let main = self.frame.metrics(FontKind::Ui, para.size);
        let mut prev_end = 0;
        for line in lines {
            // 块文字里：`<br>`、硬换行处换行，自动折行处不插入字符。
            if atoms[prev_end..line.start]
                .iter()
                .any(|a| matches!(a.kind, AtomKind::Newline))
            {
                self.text.push('\n');
                self.chars += 1;
            }
            prev_end = line.end;
            let atoms = &atoms[line.clone()];
            // 行高取正文的行距，图片较高时增加该行的高度；基线以下保留正文的下行高度。
            let (mut ascent, mut descent) = (main.ascent, main.descent);
            let mut image_h: f32 = 0.0;
            let mut math_below: f32 = 0.0;
            let mut line_h = para.size * para.line;
            for atom in atoms {
                match &atom.kind {
                    AtomKind::Image { h, .. } => image_h = image_h.max(*h),
                    // 公式较高时增加该行的高度，上下各保留少量间距。
                    AtomKind::Math { b, .. } => {
                        image_h = image_h.max(b.asc + para.size * 0.15);
                        math_below = math_below.max(b.desc + para.size * 0.15);
                    }
                    AtomKind::Char(_) => {
                        let st = runs[atom.run];
                        if st.size > para.size {
                            let m = self.frame.metrics(st.font, st.size);
                            ascent = ascent.max(m.ascent);
                            descent = descent.max(m.descent);
                            line_h = line_h.max(st.size * para.line);
                        }
                    }
                    _ => {}
                }
            }
            let half = ((line_h - ascent - descent) / 2.0).max(0.0);
            let above = (ascent + half).max(image_h);
            let below = (descent + half).max(math_below);
            let baseline = top + above;
            first_baseline.get_or_insert(baseline);
            let height = above + below;

            let used: f32 = atoms
                .iter()
                .rev()
                .skip_while(|a| a.space)
                .map(|a| a.width)
                .sum();
            let align = match para.align {
                Align::None => inline.align,
                align => align,
            };
            let mut pen = x + match align {
                Align::Center => ((width - used) / 2.0).max(0.0),
                Align::Right => (width - used).max(0.0),
                Align::None | Align::Left => 0.0,
            };
            self.emit_line(atoms, &runs, &mut pen, baseline, (top, height), inline, out);
            if let Some(source) = anchor {
                out.push(Item::Text {
                    x: pen,
                    baseline,
                    text: "".into(),
                    font: FontKind::Ui,
                    faux: Faux::default(),
                    size: para.size,
                    color: para.color,
                    span: Some(Span {
                        start: self.chars,
                        advances: Arc::from([]),
                        source: Some(source),
                        jumps: None,
                    }),
                });
            }
            top += height;
        }
        self.end_paragraph();
        Flow {
            height: top - y,
            first_baseline,
        }
    }

    /// 绘制一行：同一样式连着的字合成一段文字，行内代码铺底色，删除线、链接范围一起记下。
    #[allow(clippy::too_many_arguments)]
    fn emit_line(
        &mut self,
        atoms: &[Atom],
        runs: &[RunStyle],
        pen: &mut f32,
        baseline: f32,
        (top, height): (f32, f32),
        inline: &Inline,
        out: &mut Vec<Item>,
    ) {
        let mut i = 0;
        while i < atoms.len() {
            let run = atoms[i].run;
            let st = runs[run];
            let group = out.len();
            let start = *pen;
            let at = baseline - st.rise;
            let mut text = Pending::default();
            let mut j = i;
            while j < atoms.len() && atoms[j].run == run {
                let atom = &atoms[j];
                match &atom.kind {
                    AtomKind::Char(c) => {
                        if text.text.is_empty() {
                            text.x = *pen;
                            text.start = self.chars;
                            text.source = atom.source;
                        } else if let (Some(at), Some(source)) = (text.source, atom.source)
                            && source != text.next
                        {
                            let k = text.advances.len() as u32;
                            text.jumps.push((k, source.saturating_sub(at)));
                        }
                        text.next = atom.source.map_or(text.next, |s| s + c.len_utf8() as u32);
                        text.text.push(*c);
                        text.advances.push(atom.width);
                        self.text.push(*c);
                        self.chars += 1;
                    }
                    AtomKind::Image { path, h } => {
                        flush_text(out, &mut text, at, st);
                        out.push(Item::Image {
                            rect: Rect::new(*pen, baseline - h, atom.width, *h),
                            path: path.clone(),
                            at: atom.source.map(|s| (self.chars, s)),
                        });
                    }
                    AtomKind::Math { b, tex } => {
                        flush_text(out, &mut text, at, st);
                        let x = *pen;
                        out.extend(b.items.iter().map(|item| item.moved(x, baseline, 0)));
                        // 公式中没有可定位的字：点击公式时光标移到公式的原文处。
                        if let Some(source) = atom.source {
                            out.push(Item::Spot {
                                rect: Rect::new(x, baseline - b.asc, atom.width, b.asc + b.desc),
                                at: (self.chars, source),
                            });
                        }
                        self.text.push_str(tex);
                        self.chars += tex.chars().count();
                    }
                    AtomKind::Pad | AtomKind::Newline => flush_text(out, &mut text, at, st),
                }
                *pen += atom.width;
                j += 1;
            }
            flush_text(out, &mut text, at, st);
            // 末尾的空格不算进底色、删除线和链接的范围。
            let trailing: f32 = atoms[i..j]
                .iter()
                .rev()
                .take_while(|a| a.space)
                .map(|a| a.width)
                .sum();
            let end = *pen - trailing;
            if st.flags & Style::CODE != 0 && end > start {
                let m = self.frame.metrics(st.font, st.size);
                let pad = st.size * 0.2;
                let rect = Rect::new(
                    start,
                    baseline - m.ascent - pad,
                    end - start,
                    m.ascent + m.descent + pad * 2.0,
                );
                // 底色画在这一段的文字下面。
                let fill = Item::Fill {
                    rect,
                    color: self.colors.code_inline,
                    radius: 4.0,
                };
                out.insert(group, fill);
            }
            if st.flags & Style::MARK != 0 && end > start {
                let m = self.frame.metrics(st.font, st.size);
                let pad = st.size * 0.1;
                let rect = Rect::new(
                    start,
                    baseline - m.ascent - pad,
                    end - start,
                    m.ascent + m.descent + pad * 2.0,
                );
                // 高亮底色（经验值）：深色主题用 30% 不透明的暗黄，浅色主题用浅黄。
                let color = if self.dark {
                    Color::hex(0xbb8009).alpha(0.3)
                } else {
                    Color::hex(0xfff8c5)
                };
                out.insert(
                    group,
                    Item::Fill {
                        rect,
                        color,
                        radius: 2.0,
                    },
                );
            }
            if st.flags & Style::UNDERLINE != 0 && end > start {
                let thick = (st.size / 16.0).max(1.0);
                out.push(Item::Fill {
                    rect: Rect::new(start, baseline + st.size * 0.12, end - start, thick),
                    color: st.color,
                    radius: 0.0,
                });
            }
            if st.flags & Style::STRIKE != 0 && end > start {
                let thick = (st.size / 16.0).max(1.0);
                out.push(Item::Fill {
                    rect: Rect::new(start, baseline - st.size * 0.3, end - start, thick),
                    color: st.color,
                    radius: 0.0,
                });
            }
            if st.link != 0
                && let Some(url) = inline.links.get(st.link as usize - 1)
            {
                out.push(Item::Link {
                    rect: Rect::new(start, top, (end - start).max(1.0), height),
                    url: url.as_str().into(),
                });
            }
            i = j;
        }
    }

    /// 返回一段行内文字的测量结果：最长的不能断开的一段有多宽、整段不换行有多宽（逻辑像素），
    /// 表格按它分列宽。
    pub(crate) fn measure_raw(&mut self, inline: &Inline, para: Para) -> (f32, f32) {
        let (atoms, _) = self.atoms(inline, para, f32::INFINITY);
        let (mut longest_word, mut word, mut line, mut longest_line) = (0f32, 0f32, 0f32, 0f32);
        for atom in &atoms {
            if matches!(atom.kind, AtomKind::Newline) {
                longest_line = longest_line.max(line);
                line = 0.0;
                word = 0.0;
                continue;
            }
            if atom.brk {
                word = 0.0;
            }
            if !atom.space {
                word += atom.width;
            }
            longest_word = longest_word.max(word);
            line += atom.width;
        }
        (longest_word, longest_line.max(line))
    }
}

/// 一行里攒着的一段同样式的文字：文字、每个字的宽度、在块文字里的起点、横坐标。
#[derive(Default)]
struct Pending {
    text: String,
    advances: Vec<f32>,
    start: usize,
    x: f32,
    /// 第一个字符在原文中的位置、原文不连续之处（见 [`Span`]）。
    source: Option<u32>,
    jumps: Vec<(u32, u32)>,
    /// 下一个字符在原文中紧跟其后的位置（字节偏移）。
    next: u32,
}

/// 攒着的一段文字（同一样式、中间没有图片）推成一项；只有空格时不画（块文字里仍然有）。
fn flush_text(out: &mut Vec<Item>, text: &mut Pending, baseline: f32, st: RunStyle) {
    let pending = std::mem::take(text);
    if pending.text.trim_end().is_empty() {
        return;
    }
    out.push(Item::Text {
        x: pending.x,
        baseline,
        text: pending.text.into(),
        font: st.font,
        faux: st.faux,
        size: st.size,
        color: st.color,
        span: Some(Span {
            start: pending.start,
            advances: pending.advances.into(),
            source: pending.source,
            jumps: (!pending.jumps.is_empty()).then(|| pending.jumps.into()),
        }),
    });
}

/// `inline` 中第 `at` 个字节在原文中的位置（相对顶层块）。
fn source_of(inline: &Inline, at: usize) -> Option<u32> {
    inline.source_of(at).and_then(|s| u32::try_from(s).ok())
}

/// 返回各行的范围：按 `width`（逻辑像素）把 `atoms` 分成行。
fn break_lines(atoms: &[Atom], width: f32) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut x = 0.0;
    let mut last_break: Option<usize> = None;
    while i < atoms.len() {
        let atom = &atoms[i];
        if matches!(atom.kind, AtomKind::Newline) {
            lines.push(start..i);
            start = i + 1;
            i = start;
            x = 0.0;
            last_break = None;
            continue;
        }
        if atom.brk && i > start {
            last_break = Some(i);
        }
        // 保留 0.01 逻辑像素的余量：测得的宽度加上边距再减去（表格的列宽）会产生浮点误差，
        // 恰好放得下的最后一个字符不应折到下一行。
        if x + atom.width > width + 0.01 && i > start && !atom.space {
            // 没有可断处、只能在字与字之间强制断行时，若下一行将以不能出现在行首的标点开头，
            // 断点向前移一个字，使该标点与前一个字一起换行。
            let forced = |mut at: usize| {
                while at > start + 1
                    && matches!(atoms[at].kind, AtomKind::Char(c) if no_break_before(c))
                {
                    at -= 1;
                }
                at
            };
            let at = last_break
                .filter(|&b| b > start)
                .unwrap_or_else(|| forced(i));
            lines.push(start..at);
            start = at;
            while start < atoms.len() && atoms[start].space {
                start += 1;
            }
            i = start;
            x = 0.0;
            last_break = None;
            continue;
        }
        x += atom.width;
        i += 1;
    }
    if start < atoms.len() || lines.is_empty() {
        lines.push(start..atoms.len());
    }
    lines
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// 中日韩文字和全角标点：每个字前后都可以换行。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F)
}

/// 是否可以在字 `p` 与紧随其后的 `c` 之间换行。
///
/// 空格之后、中日韩文字前后可以换行；`c` 不能出现在行首时不换行；`p` 不能出现在行尾时，除非
/// `p` 是空格，否则不换行。
fn can_break(p: char, c: char) -> bool {
    (is_space(p) || is_cjk(p) || is_cjk(c))
        && !no_break_before(c)
        && (is_space(p) || !no_break_after(p))
}

/// 标点 `c` 是否不能出现在行首。
///
/// 包括中点 `·`、`・`：用空格隔开的中点作为分隔符时，留在前一个词所在行的末尾。
fn no_break_before(c: char) -> bool {
    "，。、；：？！）」』】》〉…—”’,.;:?!)]}%·・".contains(c)
}

/// 标点 `c` 是否不能出现在行尾。
fn no_break_after(c: char) -> bool {
    "（「『【《〈“‘([{".contains(c)
}

#[cfg(test)]
mod tests;
