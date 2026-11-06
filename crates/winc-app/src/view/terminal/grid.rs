//! 网格的绘制：终端颜色换算、按样式分段绘制一行、光标、输入法组字。

use super::*;
use winc_render::Stroke;

/// 把终端的颜色换成界面颜色。
pub(super) struct Colors {
    /// 绘制用的配色。
    pub(super) p: &'static crate::theme::Palette,
    /// 整屏反色（DECSCNM）。
    pub(super) reverse: bool,
    /// 对比度保护的最低对比度（0 表示关）。
    pub(super) min: f32,
    /// 粗体的前 8 种颜色绘制成亮色（`terminal.boldIsBright`）。
    pub(super) bright: bool,
}

impl Colors {
    fn default_fg(&self) -> Color {
        if self.reverse {
            self.p.editor_bg
        } else {
            self.p.syntax_text
        }
    }

    fn default_bg(&self) -> Color {
        if self.reverse {
            self.p.syntax_text
        } else {
            self.p.editor_bg
        }
    }

    fn resolve(&self, color: TermColor, default: Color) -> Color {
        match color {
            TermColor::Default => default,
            TermColor::Indexed(i @ 0..=15) => self.p.ansi[usize::from(i)],
            TermColor::Indexed(i) => {
                let [r, g, b] = winc_pty::protocol::Theme::default().color(i);
                rgb(r, g, b)
            }
            TermColor::Rgb(r, g, b) => rgb(r, g, b),
        }
    }

    /// 一段文字的（字色，底色）：反色、淡色、隐藏都算进去。
    ///
    /// 底色是默认底色时返回 `None`，不必绘制。
    pub(super) fn style(&self, style: &Style) -> (Color, Option<Color>) {
        let attrs = style.attrs;
        let fg = match style.fg {
            TermColor::Indexed(i @ 0..=7) if self.bright && attrs.has(Attrs::BOLD) => {
                TermColor::Indexed(i + 8)
            }
            fg => fg,
        };
        let mut fg = self.resolve(fg, self.default_fg());
        let mut bg = match style.bg {
            TermColor::Default => None,
            color => Some(self.resolve(color, self.default_bg())),
        };
        if attrs.has(Attrs::INVERSE) {
            let back = bg.unwrap_or_else(|| self.default_bg());
            bg = Some(fg);
            fg = back;
        }
        // 浅色终端上，按深色界面画的近黑底（输入框、差异高亮）提成同色相的浅底。
        if !self.p.dark && let Some(color) = bg.as_mut() {
            *color = lighten_dark_fill(*color);
        }
        let under = bg.unwrap_or_else(|| self.default_bg());
        if attrs.has(Attrs::DIM) {
            fg = blend(fg, under, 0.5);
        }
        if attrs.has(Attrs::HIDDEN) {
            fg = under;
        } else if !attrs.has(Attrs::DIM) {
            // 淡色是程序有意压低的亮度，不再提高对比度。
            fg = readable(fg, under, self.min);
        }
        (fg, bg)
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::hex(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
}

/// 浅色底上的近黑填充提亮，色相留着：黑变成浅灰，深色绿变成浅绿。
///
/// 已经够亮的颜色不动，深色终端不走这里。
fn lighten_dark_fill(color: Color) -> Color {
    if crate::theme::luminance(color) >= 0.15 {
        return color;
    }
    blend(Color::hex(0xFFFFFF), color, 0.82)
}

/// `fg` 和 `bg` 按 `t`（`fg` 的比例）调和。
fn blend(fg: Color, bg: Color, t: f32) -> Color {
    let mix = |a: f32, b: f32| a * t + b * (1.0 - t);
    Color {
        r: mix(fg.r, bg.r),
        g: mix(fg.g, bg.g),
        b: mix(fg.b, bg.b),
        a: 1.0,
    }
}

/// 一行里的一段：从第几格到第几格，文字和样式。
pub(super) struct Segment<'a> {
    /// 起始格（从 0 起）。
    pub(super) start: usize,
    /// 结束格（不含）；宽字占两格。
    pub(super) end: usize,
    /// 这一段的文字（`line.text` 中的一段）。
    pub(super) text: &'a str,
    /// 绘制这一段用的颜色与属性。
    style: Style,
}

/// 按样式段把一行切开。
pub(super) fn segments(line: &Line) -> Vec<Segment<'_>> {
    let default = [winc_pty::protocol::Run {
        start: 0,
        style: Style::default(),
    }];
    let runs = if line.runs.is_empty() {
        &default[..]
    } else {
        &line.runs[..]
    };

    let mut out = Vec::with_capacity(runs.len());
    let (mut col, mut byte) = (0, 0);
    let mut chars = line.text.char_indices().peekable();
    for (i, run) in runs.iter().enumerate() {
        let until = runs.get(i + 1).map_or(usize::MAX, |r| usize::from(r.start));
        let (start, from) = (col, byte);
        while let Some(&(at, c)) = chars.peek() {
            let w = char_width(c);
            // 组合字符跟着前一个字，留在这一段里。
            if w > 0 && col >= until {
                break;
            }
            col += w;
            byte = at + c.len_utf8();
            chars.next();
        }
        out.push(Segment {
            start,
            end: col,
            text: &line.text[from..byte],
            style: run.style,
        });
    }
    out
}

/// 绘制一行。
///
/// `marks` 是要铺的底色（选区、查找的匹配）：（起始格，结束格，颜色）。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_line(
    app: &App,
    frame: &mut Frame<'_>,
    colors: &Colors,
    line: &Line,
    row: Rect,
    baseline: f32,
    marks: &[(usize, usize, Color)],
    clip: Rect,
) {
    let (cell, size) = (app.cell, app.fonts.code);
    let parts = segments(line);
    let styles: Vec<(Color, Option<Color>)> =
        parts.iter().map(|p| colors.style(&p.style)).collect();
    let x_of = |col: usize| row.x + col as f32 * cell;

    for (part, &(_, bg)) in parts.iter().zip(&styles) {
        if let Some(bg) = bg
            && part.end > part.start
        {
            let rect = Rect::new(
                x_of(part.start),
                row.y,
                (part.end - part.start) as f32 * cell,
                row.h,
            );
            frame.rect_clipped(rect, bg, 0.0, clip);
        }
    }
    for &(start, end, color) in marks {
        // 面板刚变窄、重排好的屏幕还没回来时，选区的起点可能在新的宽度外面：底色最多铺 10000 格，
        // 屏幕宽度远小于这个上限，异常的值不会画到屏幕外。
        let end = end.min(start + 10_000).max(start);
        let rect = Rect::new(x_of(start), row.y, (end - start) as f32 * cell, row.h);
        frame.rect_clipped(rect, color, 0.0, clip);
    }

    let nudge = 1.0 / frame.scale().max(1.0);
    for (part, &(fg, _)) in parts.iter().zip(&styles) {
        let attrs = part.style.attrs;
        let bold = attrs.has(Attrs::BOLD);
        let kind = if attrs.has(Attrs::ITALIC) {
            FontKind::MonoItalic
        } else {
            FontKind::Mono
        };
        let mut col = part.start;
        // 前一个占格子的字：所在列和占几格，组合字符画在它上面。
        let mut base: Option<(usize, usize)> = None;
        if !attrs.has(Attrs::HIDDEN) {
            for c in part.text.chars() {
                let w = char_width(c);
                if w == 0 {
                    // 组合字符（重音符号、变体选择符）不占格子。字形不占宽度时（多数字体），原点
                    // 放在所附的字结束处、不做居中，与编辑区的画法相同：字形自带向左的偏移，落在
                    // 前一个字上。有的等宽字体给组合字符一整格的宽度、字形画在这一格里：那就和
                    // 前一个字画在同一格里。
                    let mut buf = [0; 4];
                    let advance = frame.measure(kind, size, c.encode_utf8(&mut buf));
                    match base {
                        Some((at, cells)) if advance > 0.0 => {
                            frame.cell_char(
                                kind,
                                size,
                                x_of(at),
                                baseline,
                                cell,
                                cells,
                                c,
                                fg,
                                clip,
                            );
                        }
                        _ => frame.cell_char(kind, size, x_of(col), baseline, cell, 0, c, fg, clip),
                    }
                    continue;
                }
                base = Some((col, w));
                if c != ' ' {
                    frame.cell_char(kind, size, x_of(col), baseline, cell, w, c, fg, clip);
                    // 粗体：往右错开一个物理像素再绘制一次，笔画加粗、字宽不变（等宽的格子不乱）。
                    if bold {
                        let x = x_of(col) + nudge;
                        frame.cell_char(kind, size, x, baseline, cell, w, c, fg, clip);
                    }
                }
                col += w;
            }
        }
        let span = |y: f32, h: f32| {
            Rect::new(
                x_of(part.start),
                y,
                (part.end - part.start) as f32 * cell,
                h,
            )
        };
        let underline = attrs.underline();
        if underline != Underline::None && part.end > part.start {
            let color = match part.style.underline_color {
                TermColor::Default => fg,
                c => colors.resolve(c, fg),
            };
            let (x0, x1) = (x_of(part.start), x_of(part.end));
            draw_underline(frame, underline, (x0, x1), baseline + 2.0, color, clip);
        }
        if attrs.has(Attrs::STRIKE) && part.end > part.start {
            let y = (baseline - app.mono_metrics.x_height / 2.0).round();
            frame.rect_clipped(span(y, 1.0), fg, 0.0, clip);
        }
    }
}

/// 在 `x0..x1`（逻辑像素）、上沿为 `y` 处绘制样式为 `underline` 的下划线，线宽 1 逻辑像素。
///
/// 点线每 2 像素一点；虚线画 3 像素、空 2 像素；波浪线是周期 4 像素、高 2 像素的折线。
fn draw_underline(
    frame: &mut Frame<'_>,
    underline: Underline,
    (x0, x1): (f32, f32),
    y: f32,
    color: Color,
    clip: Rect,
) {
    let line = |frame: &mut Frame<'_>, from: f32, to: f32, y: f32| {
        frame.rect_clipped(Rect::new(from, y, to.min(x1) - from, 1.0), color, 0.0, clip);
    };
    // 只画落在裁剪范围内的部分：一行可能很宽，点线、虚线逐段绘制。
    let start = x0.max(clip.x);
    let end = x1.min(clip.right());
    let steps = |period: f32| {
        let first = x0 + ((start - x0) / period).floor() * period;
        (0..)
            .map(move |i| first + i as f32 * period)
            .take_while(move |&x| x < end)
    };
    match underline {
        Underline::None => {}
        Underline::Single => line(frame, x0, x1, y),
        Underline::Double => {
            line(frame, x0, x1, y);
            line(frame, x0, x1, y + 2.0);
        }
        Underline::Dotted => steps(2.0).for_each(|x| line(frame, x, x + 1.0, y)),
        Underline::Dashed => steps(5.0).for_each(|x| line(frame, x, x + 3.0, y)),
        Underline::Curly => {
            let wave: Vec<Stroke> = steps(2.0)
                .map(|x| {
                    let up = ((x - x0) / 2.0).round() as i64 % 2 == 0;
                    let (a, b) = if up {
                        (y + 1.5, y - 0.5)
                    } else {
                        (y - 0.5, y + 1.5)
                    };
                    Stroke::Line(x, a, (x + 2.0).min(x1), b)
                })
                .collect();
            frame.stroke(&wave, 1.0, color, clip);
        }
    }
}

/// 第 `col` 格上的字占几格（光标盖住宽字符时绘制成两格宽）。
pub(super) fn width_at(line: &Line, col: usize) -> usize {
    let mut at = 0;
    for c in line.text.chars() {
        let w = char_width(c);
        if w == 0 {
            continue;
        }
        if at == col {
            return w;
        }
        at += w;
        if at > col {
            break;
        }
    }
    1
}

/// 光标：焦点在终端时按程序要的形状绘制实心的（方块里的字反色重绘），不在时绘制空心方块。
pub(super) fn draw_cursor(
    app: &App,
    frame: &mut Frame<'_>,
    tab: &TermTab,
    at: Rect,
    focused: bool,
    clip: Rect,
) {
    use winc_pty::protocol::CursorShape;

    let p = app.palette;
    if !focused {
        let mut edge = |r: Rect| frame.rect_clipped(r, p.caret, 0.0, clip);
        edge(Rect::new(at.x, at.y, at.w, 1.0));
        edge(Rect::new(at.x, at.bottom() - 1.0, at.w, 1.0));
        edge(Rect::new(at.x, at.y, 1.0, at.h));
        edge(Rect::new(at.right() - 1.0, at.y, 1.0, at.h));
        return;
    }
    if !app.caret_visible {
        return;
    }
    match tab.cursor.shape {
        CursorShape::Bar => {
            frame.rect_clipped(Rect::new(at.x, at.y, 2.0, at.h), p.caret, 0.0, clip)
        }
        CursorShape::Underline => frame.rect_clipped(
            Rect::new(at.x, at.bottom() - 2.0, at.w, 2.0),
            p.caret,
            0.0,
            clip,
        ),
        CursorShape::Block => {
            frame.rect_clipped(at, p.caret, 0.0, clip);
            // 方块里的字用底色重绘，盖住了也看得见。
            let row = usize::from(tab.cursor.row);
            let col = usize::from(tab.cursor.col);
            if let Some(c) = tab.rows.get(row).and_then(|line| char_at(line, col))
                && c != ' '
            {
                let baseline = baseline_in(at, app.mono_metrics.ascent, app.mono_metrics.descent);
                frame.cell_char(
                    FontKind::Mono,
                    app.fonts.code,
                    at.x,
                    baseline,
                    app.cell,
                    char_width(c).max(1),
                    c,
                    p.editor_bg,
                    clip,
                );
            }
        }
    }
}

/// 返回第 `col` 格上的字符；落在宽字符的后半格或空白处时返回 `None`。
pub(super) fn char_at(line: &Line, col: usize) -> Option<char> {
    let mut at = 0;
    for c in line.text.chars() {
        let w = char_width(c);
        if w == 0 {
            continue;
        }
        if at == col {
            return Some(c);
        }
        at += w;
        if at > col {
            return None;
        }
    }
    None
}

/// 输入法组字中的文字：绘制在终端光标处，带强调色的下划线，盖住下面的格子。
pub(super) fn draw_preedit(
    app: &App,
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    baseline: f32,
    clip: Rect,
) {
    let p = app.palette;
    let cell = app.cell;
    let cells = |c: char| char_width(c).max(1);
    let width = app.preedit.chars().map(cells).sum::<usize>() as f32 * cell;
    frame.rect_clipped(
        Rect::new(x, y, width, app.fonts.line_height),
        p.editor_bg,
        0.0,
        clip,
    );
    let mut px = x;
    for c in app.preedit.chars() {
        frame.cell_char(
            FontKind::Mono,
            app.fonts.code,
            px,
            baseline,
            cell,
            cells(c),
            c,
            p.text,
            clip,
        );
        px += cells(c) as f32 * cell;
    }
    frame.rect_clipped(
        Rect::new(x, baseline + 3.0, width, 1.0),
        p.accent,
        0.0,
        clip,
    );
}
