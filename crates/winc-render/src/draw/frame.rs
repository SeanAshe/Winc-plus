//! 一帧的绘制：矩形、圆角矩形、描边、文字、图片、阴影，以及裁剪和整体不透明度。
//!
//! [`Frame`] 持有本帧的画布与各个缓存，把逻辑像素坐标换算成物理像素后交给 [`Canvas`] 绘制。

use super::*;
use crate::draw::canvas::{Tinted, Wall};

/// 一帧的绘制上下文：画布、字形缓存、覆盖率表和裁剪区域。
///
/// 绘制命令按调用顺序直接写入缓冲，后绘制的内容覆盖先绘制的内容；
/// 所有入口都受 `limit` 与 `opacity` 约束。
pub struct Frame<'a> {
    pub(crate) canvas: Canvas<'a>,
    /// 字形缓存，与 [`Renderer`] 共用。
    pub(crate) glyphs: &'a mut GlyphCache,
    /// 描边蒙版缓存，与 [`Renderer`] 共用。
    pub(crate) strokes: &'a mut StrokeCache,
    /// 覆盖率映射表，来自 [`Renderer`]。
    pub(crate) gamma: &'a [u8; 256],
    /// `gamma` 是否为深色背景的映射表，用作字形缓存键的一部分。
    pub(crate) dark: bool,
    /// 屏幕缩放系数（每个逻辑像素对应的物理像素数）。
    pub(crate) scale: f32,
    /// 本帧画布的范围（逻辑像素）。
    pub(crate) bounds: Rect,
    /// 裁剪上限：不论各个调用传入的裁剪框是什么，绘制都不会超出该范围。
    ///
    /// 由 [`Frame::within`] 临时缩小。
    pub(crate) limit: Rect,
    /// 整体不透明度（0–1），所有绘制的颜色都乘以它。
    ///
    /// 由 [`Frame::with_opacity`] 临时相乘。
    pub(crate) opacity: f32,
    /// 背景色，`0xAARRGGBB`。
    pub(crate) background: u32,
    /// 已随背景色绘制的底色矩形（物理像素），见 [`Frame::base`]。
    pub(crate) based: &'a [(IRect, u32)],
    /// 窗口背景图的图层；没有背景图时为 `None`。
    pub(crate) wall: Option<Wall<'a>>,
    /// 透出背景图的底色（`0xAARRGGBB`）及其加在图层像素上的值，见 `Renderer::set_wallpaper`。
    pub(crate) surfaces: &'a [(u32, u32)],
}

impl<'a> Frame<'a> {
    /// 颜色 `argb` 是透出背景图的底色时，返回用它填充时写入的内容。
    fn tint(&self, argb: u32) -> Option<Tinted<'a>> {
        let wall = self.wall?;
        let &(_, k) = self.surfaces.iter().find(|(surface, _)| *surface == argb)?;
        let plain = argb;
        Some(Tinted { wall, k, plain })
    }

    /// 此后绘制的底色不再透出背景图：用于浮在内容之上的面板和菜单。
    pub fn end_wallpaper(&mut self) {
        self.wall = None;
    }
}

impl Frame<'_> {
    /// 返回本帧画布的范围（逻辑像素），原点在左上角。
    ///
    /// 窗口绘制时即整个窗口。
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 在 `clip` 之内执行 `paint`，结束后恢复调用前的裁剪区域，返回 `paint` 的结果。
    ///
    /// `paint` 中的所有绘制都不会超出 `clip`，不论各个调用传入的裁剪框是什么。滚动区域（设置页、
    /// 列表）以此兜底：区域内任一调用传入的裁剪框只覆盖其自身范围时，滚动到边缘后该调用会绘制到
    /// 区域之外。可以嵌套，内层的有效范围不会超出外层。
    pub fn within<R>(&mut self, clip: Rect, paint: impl FnOnce(&mut Self) -> R) -> R {
        let outer = self.limit;
        self.limit = outer.intersect(clip);
        let result = paint(self);
        self.limit = outer;
        result
    }

    /// 以整体不透明度 `opacity`（0–1，与外层的相乘）执行 `paint`，用于浮层的淡入淡出。
    ///
    /// 其中绘制的矩形、描边、阴影、文字和图片都按比例变淡。各部分分别变淡后再叠加，重叠处会比
    /// 单独一层略深；淡入淡出只持续一百多毫秒，这种加深难以察觉。
    pub fn with_opacity<R>(&mut self, opacity: f32, paint: impl FnOnce(&mut Self) -> R) -> R {
        let outer = self.opacity;
        self.opacity = outer * opacity.clamp(0.0, 1.0);
        let result = paint(self);
        self.opacity = outer;
        result
    }

    /// 保存画面上 `rect`（逻辑像素，先裁剪到画面之内）中当前的像素。
    ///
    /// 之后可用 [`Self::restore_tinted`] 写回该位置，或用 [`Self::blend_snapshot`] 作为图层叠加。
    /// 用于动画：动画开始时绘制并保存一次，动画过程中每帧直接复用，无需每帧重绘不变的内容。
    pub fn snapshot(&self, rect: Rect) -> Snapshot {
        let (area, px) = self.canvas.copy(rect.to_pixels(self.scale));
        Snapshot {
            area,
            px,
            scale: self.scale,
        }
    }

    /// 与 [`Self::snapshot`] 相同，但保存到已有的 `snapshot` 并复用其像素缓冲。
    ///
    /// 用于每帧都要保存的场景。每帧新分配数 MB 的副本再释放时，macOS 的分配器会保留释放的大块
    /// 内存而不归还系统，打开 `⌘P` 面板时物理内存会多占用三十多 MB。
    pub fn snapshot_into(&self, rect: Rect, snapshot: &mut Snapshot) {
        snapshot.area = self
            .canvas
            .copy_into(rect.to_pixels(self.scale), &mut snapshot.px);
        snapshot.scale = self.scale;
    }

    /// 把快照 `snapshot` 写回其保存时的位置，同时叠加一层颜色 `tint`（乘以整体不透明度）。
    ///
    /// 用于恢复下方的窗口内容并叠加遮罩，一次完成。`tint` 全透明时直接写回；缩放系数改变
    /// （切换显示器）时不写回。
    pub fn restore_tinted(&mut self, snapshot: &Snapshot, tint: Color) {
        if snapshot.scale == self.scale {
            let tint = self.faded(tint);
            self.canvas
                .paste_tinted(snapshot.area, &snapshot.px, tint.argb());
        }
    }

    /// 把快照 `snapshot` 作为圆角半径为 `radius`（逻辑像素）的卡片叠加到当前画面上。
    ///
    /// 以卡片中心缩放 `scale`，整体按 `opacity` 叠加，圆角之外保持不变。[`Self::with_opacity`]
    /// 逐个图形应用不透明度，本方法则使整张卡片一起淡入淡出、一起缩放。缩放系数改变时不绘制。
    pub fn blend_snapshot(&mut self, snapshot: &Snapshot, radius: f32, opacity: f32, scale: f32) {
        if snapshot.scale == self.scale {
            let radius = radius * self.scale;
            self.canvas
                .blend_layer(snapshot.area, &snapshot.px, opacity, scale, radius);
        }
    }

    /// 返回乘以整体不透明度之后的颜色。
    fn faded(&self, color: Color) -> Color {
        color.alpha(color.a * self.opacity)
    }

    /// 返回屏幕缩放系数（每个逻辑像素对应的物理像素数，Retina 屏幕上为 2）。
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 绘制一张非预乘的 RGBA 图像，与屏幕像素一一对应，左上角在 `(x, y)`。
    ///
    /// `rgba` 含 `width` × `height` 个物理像素，`(x, y)` 是逻辑像素，四舍五入到物理像素。图像
    /// 不缩放，需要缩放时由调用方先处理。超出 `clip` 的部分不绘制；`rgba` 不足
    /// `width * height * 4` 字节时不绘制。
    pub fn pixels(&mut self, (x, y): (f32, f32), width: u32, height: u32, rgba: &[u8], clip: Rect) {
        let opacity = (self.opacity * 255.0).round() as u32;
        if opacity == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
            return;
        }
        let area = IRect::new(
            (x * self.scale).round() as i32,
            (y * self.scale).round() as i32,
            width as i32,
            height as i32,
        );
        let clip = clip.intersect(self.limit).to_pixels_outer(self.scale);
        self.canvas.rgba(area, rgba, opacity, clip);
    }

    /// 把一张非预乘的 RGBA 图像缩放到 `rect`（逻辑像素）绘制，取最近的像素，不做插值。
    ///
    /// 用于暂时显示尺寸不符的图像（例如缩放过渡期间）。超出 `clip` 的部分不绘制；`rgba` 不足
    /// `width * height * 4` 字节时不绘制。
    pub fn pixels_scaled(&mut self, rect: Rect, width: u32, height: u32, rgba: &[u8], clip: Rect) {
        let opacity = (self.opacity * 255.0).round() as u32;
        let size = (width as usize, height as usize);
        if opacity == 0 || rgba.len() < size.0 * size.1 * 4 {
            return;
        }
        let area = rect.to_pixels(self.scale);
        let clip = clip.intersect(self.limit).to_pixels_outer(self.scale);
        self.canvas.rgba_scaled(area, size, rgba, opacity, clip);
    }

    /// 在 `rect` 中绘制棋盘格（透明图片的底色），超出 `clip` 的部分不绘制。
    ///
    /// 方格边长 `cell`（逻辑像素），左上角的方格为 `first`，与其相邻的为 `second`。两种颜色都
    /// 不透明时按行一次填色；有半透明（淡入淡出）时逐格绘制。两种情况都只处理 `clip` 内的
    /// 方格，耗时与 `rect` 超出 `clip` 的部分无关。
    pub fn checker(&mut self, rect: Rect, cell: f32, (first, second): (Color, Color), clip: Rect) {
        if self.opacity >= 1.0 && first.a >= 1.0 && second.a >= 1.0 {
            let area = rect.to_pixels(self.scale);
            let cell = (cell * self.scale).round().max(1.0) as usize;
            let clip = clip.intersect(self.limit).to_pixels_outer(self.scale);
            self.canvas
                .checker(area, cell, (first.argb(), second.argb()), clip);
            return;
        }
        let seen = rect.intersect(clip).intersect(self.limit);
        if seen.w <= 0.0 || seen.h <= 0.0 || cell <= 0.0 {
            return;
        }
        self.rect_clipped(rect, second, 0.0, clip);
        // 与 `seen` 相交的方格的行、列范围；行号与列号之和为偶数的方格为 `first`。
        let span = |from: f32, to: f32| {
            (
                (from / cell).floor().max(0.0) as usize,
                (to / cell).ceil() as usize,
            )
        };
        let (left, right) = span(seen.x - rect.x, seen.right() - rect.x);
        let (top, bottom) = span(seen.y - rect.y, seen.bottom() - rect.y);
        for row in top..bottom {
            for col in (left + (left + row) % 2..right).step_by(2) {
                let x = rect.x + col as f32 * cell;
                let square = Rect::new(x, rect.y + row as f32 * cell, cell, cell);
                self.rect_clipped(square.intersect(rect), first, 0.0, clip);
            }
        }
    }

    /// 绘制本帧的底色矩形 `rects`，后者覆盖前者；须为本帧的第一次绘制。
    ///
    /// `rects` 与 [`Renderer::base_next_frame`] 声明的一致时已由渲染器绘制，直接返回；未声明时
    /// 逐个绘制；不一致时重新填充背景色后绘制。三种情况的画面相同。
    pub fn base(&mut self, rects: &[(Rect, Color)]) {
        let scale = self.scale;
        let painted = self.based.len() == rects.len()
            && (self.based.iter().zip(rects)).all(|(&(area, argb), &(rect, color))| {
                area == rect.to_pixels(scale) && argb == color.argb()
            });
        if painted {
            return;
        }
        if !self.based.is_empty() {
            let all = IRect::new(0, 0, self.canvas.width as i32, self.canvas.height as i32);
            let background = self.background | 0xff00_0000;
            let tint = self.tint(background);
            self.canvas.rect(all, background, all, None, tint);
        }
        for &(rect, color) in rects {
            self.rect(rect, color);
        }
    }

    /// 绘制一个直角矩形，超出 [`Self::bounds`] 的部分被裁剪。
    pub fn rect(&mut self, rect: Rect, color: Color) {
        self.rect_clipped(rect, color, 0.0, self.bounds);
    }

    /// 绘制一个圆角矩形，四个角的半径都是 `radius`（逻辑像素）。
    pub fn rounded(&mut self, rect: Rect, color: Color, radius: f32) {
        self.rect_clipped(rect, color, radius, self.bounds);
    }

    /// 绘制矩形，`radius` 大于 0 时为圆角矩形，超出 `clip` 的部分不绘制。
    pub fn rect_clipped(&mut self, rect: Rect, color: Color, radius: f32, clip: Rect) {
        self.rect_corners(rect, color, [radius; 4], clip);
    }

    /// 绘制四个角半径各不相同的圆角矩形，`radii` 依次是左上、右上、右下、左下（逻辑像素）。
    pub fn rect_corners(&mut self, rect: Rect, color: Color, radii: [f32; 4], clip: Rect) {
        let color = self.faded(color);
        if rect.w <= 0.0 || rect.h <= 0.0 || color.a <= 0.0 {
            return;
        }

        let area = rect.to_pixels(self.scale);
        let clip = clip.intersect(self.limit).to_pixels_outer(self.scale);

        let tint = self.tint(color.argb());
        if radii.iter().any(|&r| r > 0.0) {
            let radii = radii.map(|r| r * self.scale);
            self.canvas
                .rounded_rect(area, radii, color.argb(), clip, None, tint);
        } else {
            self.canvas.rect(area, color.argb(), clip, None, tint);
        }
    }

    /// 返回不透明的圆角矩形 `rect`（圆角半径 `radius`，逻辑像素）必然覆盖的像素。
    ///
    /// 绘制位于该矩形下方的阴影、边框和遮罩时传入该值，可跳过这些随后会被覆盖的像素。
    pub fn covered(&self, rect: Rect, radius: f32) -> Covered {
        Covered::new(rect.to_pixels(self.scale), radius * self.scale)
    }

    /// 绘制圆角矩形并跳过 `covered` 中的像素；用于绘制不透明卡片下方的边框和遮罩。
    pub fn rect_except(&mut self, rect: Rect, color: Color, radius: f32, covered: Covered) {
        let color = self.faded(color);
        if rect.w <= 0.0 || rect.h <= 0.0 || color.a <= 0.0 {
            return;
        }
        let area = rect.to_pixels(self.scale);
        let clip = self.limit.to_pixels_outer(self.scale);
        let radii = [radius * self.scale; 4];
        let tint = self.tint(color.argb());
        self.canvas
            .rounded_rect(area, radii, color.argb(), clip, Some(covered), tint);
    }

    /// 把矩形 `rect` 四角半径为 `radius`（逻辑像素）的圆弧外侧填充为 `color`，使其显示为圆角矩形。
    ///
    /// 先绘制 `rect` 内的内容，最后用外侧的底色修整四角。
    pub fn round_corners(&mut self, rect: Rect, radius: f32, color: Color) {
        let color = self.faded(color);
        if rect.w <= 0.0 || rect.h <= 0.0 || color.a <= 0.0 {
            return;
        }
        let area = rect.to_pixels(self.scale);
        let clip = self.limit.to_pixels_outer(self.scale);
        let tint = self.tint(color.argb());
        self.canvas
            .round_corners(area, radius * self.scale, color.argb(), clip, tint);
    }

    /// 用宽 `width`（逻辑像素）的笔绘制一组线段、圆周和圆弧，用于绘制图标。
    ///
    /// 图标大小的一组形状只在第一次绘制时逐像素计算覆盖率并存为蒙版，之后按颜色叠加（见
    /// `StrokeCache`），结果与直接描边相同。
    pub fn stroke(&mut self, shapes: &[Stroke], width: f32, color: Color, clip: Rect) {
        let color = self.faded(color);
        if color.a <= 0.0 {
            return;
        }

        let clip = clip.intersect(self.limit).to_pixels_outer(self.scale);
        match self.strokes.get(shapes, width, self.scale) {
            // 描边的覆盖率已是线性值，不经过文字使用的覆盖率映射。
            Some((area, mask)) => {
                self.canvas
                    .mask(area, mask, color.argb(), &LINEAR_COVERAGE, clip)
            }
            None => self
                .canvas
                .stroke(shapes, width, self.scale, color.argb(), clip),
        }
    }

    /// 填充凸多边形 `points`（逻辑像素），边缘抗锯齿。
    ///
    /// 用于 Markdown 预览中的图表（菱形、箭头）。
    pub fn polygon(&mut self, points: &[(f32, f32)], color: Color, clip: Rect) {
        let color = self.faded(color);
        if color.a <= 0.0 {
            return;
        }
        let s = self.scale;
        let scaled: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x * s, y * s)).collect();
        let clip = clip.intersect(self.limit).to_pixels_outer(s);
        self.canvas.polygon(&scaled, color.argb(), clip);
    }

    /// 返回字符 `c` 的墨迹在基线上方、下方的高度（逻辑像素，下方为正）。
    ///
    /// 空白等没有墨迹的字符返回 `None`。排版公式时据此确定根号、分数线、括号的高度。
    pub fn ink(&mut self, kind: FontKind, faux: Faux, size: f32, c: char) -> Option<(f32, f32)> {
        let Resolved { slot, glyph, .. } = self.glyphs.lookup_faux(kind, faux, size, c);
        let s = self.scale;
        let key = GlyphKey {
            slot,
            glyph,
            size: (size * s * 4.0) as u32,
            subpixel: 0,
            oblique: (kind.italic() || faux.italic) && !text::font::italic_slot(slot),
            bold: faux.bold,
            dark: self.dark,
        };
        let glyph = self.glyphs.get(key, size * s, self.gamma)?;
        (glyph.height > 0).then(|| {
            let top = f32::from(glyph.top) / s;
            (top, f32::from(glyph.height) / s - top)
        })
    }

    /// 绘制圆角矩形下方的柔和阴影，向下偏移 `offset`，向外扩散 `blur`（均为逻辑像素）。
    ///
    /// 调用方先绘制阴影，再绘制卡片。
    ///
    /// `covered` 是随后绘制在阴影上方的不透明卡片覆盖的像素（[`Frame::covered`]），这些像素不绘制。
    pub fn shadow(
        &mut self,
        rect: Rect,
        radius: f32,
        blur: f32,
        offset: f32,
        color: Color,
        covered: Option<Covered>,
    ) {
        let color = self.faded(color);
        if color.a <= 0.0 {
            return;
        }
        let area = Rect::new(rect.x, rect.y + offset, rect.w, rect.h).to_pixels(self.scale);
        let clip = self.limit.to_pixels_outer(self.scale);
        self.canvas.shadow(
            area,
            radius * self.scale,
            blur * self.scale,
            color.argb(),
            clip,
            covered,
        );
    }

    /// 绘制一段文字，返回文字末尾的横坐标（逻辑像素）。
    ///
    /// `x` 是起点横坐标，`baseline` 是基线的纵坐标，字号 `size` 也是逻辑像素。
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        kind: FontKind,
        size: f32,
        x: f32,
        baseline: f32,
        text: &str,
        color: Color,
        clip: Rect,
    ) -> f32 {
        self.text_faux(kind, Faux::default(), size, x, baseline, text, color, clip)
    }

    /// 与 [`Self::text`] 相同，但按 `faux` 模拟粗体、斜体。
    #[allow(clippy::too_many_arguments)]
    pub fn text_faux(
        &mut self,
        kind: FontKind,
        faux: Faux,
        size: f32,
        x: f32,
        baseline: f32,
        text: &str,
        color: Color,
        clip: Rect,
    ) -> f32 {
        let mut pen = x;

        for c in text.chars() {
            let resolved = self.glyphs.lookup_faux(kind, faux, size, c);
            // 超出裁剪框右边的部分只累加宽度，不光栅化。
            if pen <= clip.right().min(self.limit.right()) {
                self.draw_glyph(kind, faux, size, (pen, baseline), c, resolved, color, clip);
            }
            pen += resolved.advance;
        }

        pen
    }

    /// 在等宽网格上用等宽字体 `kind`（编辑区或终端的正体、斜体）绘制字符 `c`。
    ///
    /// 该字符占 `cells` 个格子（每格宽 `cell`，逻辑像素）。
    /// 字形比占用的格子窄时（如宽字符）水平居中。
    #[allow(clippy::too_many_arguments)]
    pub fn cell_char(
        &mut self,
        kind: FontKind,
        size: f32,
        x: f32,
        baseline: f32,
        cell: f32,
        cells: usize,
        c: char,
        color: Color,
        clip: Rect,
    ) {
        let resolved = self.glyphs.lookup(kind, size, c);
        let offset = ((cell * cells as f32 - resolved.advance) / 2.0).max(0.0);
        let faux = Faux::default();
        self.draw_glyph(
            kind,
            faux,
            size,
            (x + offset, baseline),
            c,
            resolved,
            color,
            clip,
        );
    }

    /// 返回一段文字的宽度（逻辑像素）。
    pub fn measure(&mut self, kind: FontKind, size: f32, text: &str) -> f32 {
        text.chars()
            .map(|c| self.glyphs.advance(kind, size, c))
            .sum()
    }

    /// 返回字符 `c` 按 `faux` 绘制时的前进宽度（逻辑像素）。
    pub fn advance_faux(&mut self, kind: FontKind, faux: Faux, size: f32, c: char) -> f32 {
        self.glyphs.lookup_faux(kind, faux, size, c).advance
    }

    /// 返回字体 `kind` 在字号 `size`（逻辑像素）下的度量，见 [`Renderer::metrics`]。
    pub fn metrics(&self, kind: FontKind, size: f32) -> Metrics {
        self.glyphs.metrics(kind, size)
    }

    /// 在笔位置 `(x, baseline)` 绘制已经解析好（[`GlyphCache::lookup`]）的字符 `c`。
    ///
    /// 空白、控制字符和完全透明的颜色不光栅化。
    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &mut self,
        kind: FontKind,
        faux: Faux,
        size: f32,
        (x, baseline): (f32, f32),
        c: char,
        Resolved { slot, glyph, .. }: Resolved,
        color: Color,
        clip: Rect,
    ) {
        if c.is_whitespace() || c.is_control() {
            return;
        }
        self.paint_glyph(kind, faux, size, x, baseline, slot, glyph, color, clip);
    }

    /// 字符 `c` 是否由等宽字体 `kind` 自己绘制（不是后备字体）。
    ///
    /// 连字只能发生在同一主字体里。
    pub fn in_mono_font(&mut self, kind: FontKind, c: char) -> bool {
        self.glyphs.in_shape_slot(kind, c)
    }

    /// 把 `chars` 按等宽字体 `kind` 整形（连字、上下文替换），结果留到下一次调用之前。
    ///
    /// 返回字形个数。为 0 时这段文字应逐字绘制。
    pub fn shape_ligatures(&mut self, kind: FontKind, size: f32, chars: &[char]) -> usize {
        self.glyphs.shape_run(kind, size, chars);
        self.glyphs.shaped.len()
    }

    /// 返回第 `index` 个连字字形覆盖的字符范围（这段文字内的下标，含起点、不含终点）。
    pub fn ligature_span(&self, index: usize) -> (usize, usize) {
        let glyph = &self.glyphs.shaped[index];
        (glyph.start, glyph.end)
    }

    /// 绘制 [`Self::shape_ligatures`] 的第 `index` 个字形。
    ///
    /// `x` 是这一簇第一个字符的左边缘，`span` 是整簇所占的宽度（逻辑像素）。单个字形在
    /// `span` 里居中；一簇多个字形时从左边缘按整形结果排开。
    #[allow(clippy::too_many_arguments)]
    pub fn draw_ligature(
        &mut self,
        index: usize,
        kind: FontKind,
        size: f32,
        x: f32,
        baseline: f32,
        span: f32,
        color: Color,
        clip: Rect,
    ) {
        let glyph = self.glyphs.shaped[index];
        let advance = self.glyphs.glyph_advance(glyph.slot, glyph.id, size);
        // 前进宽度和所占格子差不多时居中（普通字符、以及正好占满这几格的连字）。
        // 字形明显更窄时靠左，避免一个单格字形浮在两格正中。
        let gap = span - advance;
        let dx = if glyph.placed {
            glyph.pen + glyph.x
        } else if gap.abs() <= span * 0.15 {
            (gap / 2.0).max(0.0)
        } else {
            0.0
        };
        self.paint_glyph(
            kind,
            Faux::default(),
            size,
            x + dx,
            baseline - glyph.y,
            glyph.slot,
            glyph.id,
            color,
            clip,
        );
    }

    /// 在笔位置 `(x, baseline)` 绘制已知槽位和字形 id 的一个字形。
    #[allow(clippy::too_many_arguments)]
    fn paint_glyph(
        &mut self,
        kind: FontKind,
        faux: Faux,
        size: f32,
        x: f32,
        baseline: f32,
        slot: u8,
        glyph: u16,
        color: Color,
        clip: Rect,
    ) {
        let color = self.faded(color);
        if color.a <= 0.0 {
            return;
        }
        let s = self.scale;
        // 先取整到 ¼ 像素，再拆成整像素和档位（0 至 3）；取整后小数部分达到 1 时，
        // 进位到下一个像素的第 0 档。
        let quarters = (x * s * 4.0).round();
        let snapped = (quarters / 4.0).floor();
        let subpixel = (quarters - snapped * 4.0) as u8;
        let baseline_px = (baseline * s).round() as i32;
        let clip = clip.intersect(self.limit).to_pixels_outer(s);

        let key = GlyphKey {
            slot,
            glyph,
            size: (size * s * 4.0) as u32,
            subpixel,
            // 需要斜体但实际使用的字体不是斜体：字体没有可用的斜体，或斜体中没有该字符
            // （例如注释中的中文使用后备字体）时均做斜切，使同一行注释中的文字倾斜一致。
            oblique: (kind.italic() || faux.italic) && !text::font::italic_slot(slot),
            bold: faux.bold,
            dark: self.dark,
        };
        let Some(glyph) = self.glyphs.get(key, size * s, self.gamma) else {
            return;
        };

        let area = IRect::new(
            snapped as i32 + glyph.left as i32,
            baseline_px - glyph.top as i32,
            glyph.width as i32,
            glyph.height as i32,
        );

        if glyph.color {
            let opacity = (color.a.clamp(0.0, 1.0) * 255.0) as u32;
            self.canvas.rgba(area, &glyph.data, opacity, clip);
        } else {
            self.canvas.coverage(area, &glyph.data, color.argb(), clip);
        }
    }
}
