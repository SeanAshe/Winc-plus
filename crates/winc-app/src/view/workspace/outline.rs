//! 文件视图底部的大纲：标题行（上沿一条分隔线、箭头、「大纲」、「全部展开」「全部折叠」）和展开时
//! 当前文档的符号树。
//!
//! 标题行与提交视图的储藏一组相同：小一号的次要色字，箭头随展开的比例旋转。行的样式与文件树
//! 相同：每深一层缩进 `TREE_INDENT` 并画一条参考线，有下级的符号行首是箭头，后面是表示种类的
//! 线条图标和符号名；下级展开、收起的动画也与文件树相同。光标所在（或键盘选中）的符号铺选中的
//! 底色。可点击的位置在绘制时记入 `OutlineState::hits`。

use std::borrow::Cow;
use std::f32::consts::FRAC_PI_2;

use winc_editor::symbols::SymbolKind;
use winc_i18n::t;

use super::*;
use crate::app::{Focus, OUTLINE_GAP, OutlineEmpty, OutlineTarget, outline_has_children};
use crate::icons::SymbolGlyph;
use crate::layout::tree_row_slot;
use crate::theme::{BUTTON_SMALL, TREE_INDENT};

/// 第一级箭头的中心离标题行底色左边的距离（逻辑像素），与文件树第一级目录的箭头同一列。
const ARROW: f32 = 10.0;
/// 标题文字离底色左边的距离（逻辑像素）：箭头右边再隔 10，与储藏一组的标题相同。
const TITLE: f32 = ARROW + 10.0;
/// 第一级缩进起点离列表左边的距离（逻辑像素），与文件树相同：箭头在其右 4，图标在其右 14。
const INDENT: f32 = 14.0;
/// 第一级符号名离列表左边的距离（逻辑像素），与文件树的名字同一列。
const LABEL: f32 = 48.0;

/// 绘制文件视图底部的大纲；不在文件视图时不绘制。
pub(in crate::view) fn draw_outline(app: &mut App, frame: &mut Frame<'_>) {
    let section = app.layout.outline;
    if section.h <= 0.0 {
        return;
    }
    app.sync_outline(std::time::Instant::now());
    app.reveal_outline_current();
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(section);
    // 文件树滚到底时最后几行延伸到大纲下方：先铺侧栏底色遮住它们，再画分隔线。
    frame.rect_clipped(section, p.sidebar_bg, 0.0, clip);
    frame.rect_clipped(
        Rect::new(section.x, section.y, section.w, 1.0),
        p.separator,
        0.0,
        clip,
    );
    let mut hits = Vec::new();

    let header = app.outline_header();
    let band = Rect::new(
        header.x + 8.0,
        header.y + OUTLINE_GAP,
        (header.w - 16.0).max(0.0),
        (header.h - 2.0 * OUTLINE_GAP).max(0.0),
    );
    let fold = (app.outline.open && !app.outline.items.is_empty()).then(|| {
        let y = band.y + (band.h - BUTTON_SMALL) / 2.0;
        Rect::new(band.right() - BUTTON_SMALL, y, BUTTON_SMALL, BUTTON_SMALL)
    });
    if let Some(button) = fold {
        draw_fold_all(app, frame, button, clip);
        hits.push((button, OutlineTarget::FoldAll));
    }
    let hovered = app.hover == Hover::Outline(OutlineTarget::Header);
    if hovered {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, p.hover, RADIUS_ROW, clip);
    }
    let cy = band.y + band.h / 2.0;
    // 箭头随展开的进度从朝右转到朝下。
    let open = f32::from(u8::from(app.outline.open));
    let share = app.outline.motion.map_or(open, |(_, clock)| {
        let progress = clock.progress();
        if app.outline.open {
            progress
        } else {
            1.0 - progress
        }
    });
    let turned = share * FRAC_PI_2;
    icons::chevron_turned(frame, (band.x + ARROW, cy), 6.0, turned, p.text_dim, clip);
    let title = t!("workspace-outline-title");
    let size = app.fonts.ui_small;
    label(
        frame,
        app,
        size,
        (band.x + TITLE, band),
        title,
        p.text_muted,
        clip,
    );
    hits.push((band, OutlineTarget::Header));

    let list = app.outline_list();
    if list.h > 0.0 {
        if let Some(empty) = app.outline.empty {
            let title = match empty {
                OutlineEmpty::NoFile => t!("workspace-outline-no-file"),
                OutlineEmpty::NotText => t!("workspace-outline-unsupported"),
                OutlineEmpty::TooLarge => t!("workspace-outline-too-large"),
                OutlineEmpty::NoSymbols => t!("workspace-outline-no-symbols"),
            };
            // 说明在列表里上下、左右居中：大纲高度可以拖动，居中参照列表本身。
            draw_empty_state(app, frame, (list, list), title, None);
        } else {
            draw_rows(app, frame, list, &mut hits);
        }
    }
    app.hits.outline = hits;
}

/// 绘制标题行右端的「全部折叠」（有展开着的符号时）或「全部展开」按钮。
fn draw_fold_all(app: &App, frame: &mut Frame<'_>, button: Rect, clip: Rect) {
    let p = app.palette;
    let hovered = app.hover == Hover::Outline(OutlineTarget::FoldAll);
    draw_button(frame, p, button, clip, hovered, false);
    let color = if hovered { p.text } else { p.text_muted };
    if app.outline_any_expanded() {
        icons::collapse_all(frame, center(button), color, clip);
    } else {
        icons::expand_all(frame, center(button), color, clip);
    }
}

/// 绘制列表中露出的各行，并记下每行和行首箭头的位置。
///
/// 每行的几何与文件树相同（缩进、参考线、箭头、图标和名字各列对齐，见 `draw_tree_entry`）；
/// 下级正在展开或收起时按 [`tree_row_slot`] 排布：那一段只露出一部分，下面的行随之移动。
fn draw_rows(
    app: &mut App,
    frame: &mut Frame<'_>,
    list: Rect,
    hits: &mut Vec<(Rect, OutlineTarget)>,
) {
    let p = app.palette;
    let layout = app.outline_layout();
    let rows = &layout.rows;
    let content = (rows.len() + 1) as f32 * TREE_ROW - layout.hidden;
    let scroll = app.outline.scroll.clamp(0.0, (content - list.h).max(0.0));
    app.outline.scroll = scroll;
    let highlight = app.outline_highlight(&app.outline_rows());
    let focused = app.focus == Focus::Outline;
    let items = &app.outline.items;
    let first = (scroll / TREE_ROW).floor().max(0.0) as usize;
    let shown =
        (list.h / TREE_ROW).ceil() as usize + 1 + (layout.hidden / TREE_ROW).ceil() as usize;
    for (v, &i) in rows.iter().enumerate().skip(first).take(shown) {
        let (row, clip) = tree_row_slot(list, scroll, v, layout.block, layout.hidden);
        if row.bottom() <= clip.y || row.y >= clip.bottom() {
            continue;
        }
        let symbol = &items[i];
        let hovered = matches!(
            app.hover,
            Hover::Outline(OutlineTarget::Row(h) | OutlineTarget::Arrow(h)) if h == i
        );
        let pill = Rect::new(
            row.x + 8.0,
            row.y + 1.0,
            (row.w - 16.0).max(0.0),
            row.h - 2.0,
        );
        if let Some(color) = row_color(p, highlight == Some(i), focused, hovered) {
            frame.rect_clipped(pill, color, RADIUS_ROW, clip);
        }

        let indent = row.x + INDENT;
        let cy = icon_center_y(app, row, app.fonts.ui);
        // 缩进参考线：每级一条，位于上级箭头的正下方，相邻行的参考线连成竖线。
        for level in 0..symbol.depth {
            let x = (indent + level as f32 * TREE_INDENT + 4.0).round();
            frame.rect_clipped(Rect::new(x, row.y, 1.0, row.h), p.indent_guide, 0.0, clip);
        }
        let x = indent + symbol.depth as f32 * TREE_INDENT;
        // 箭头在行内，先于整行记下：命中时按记下的先后取第一个。
        let band = pill.intersect(clip);
        if outline_has_children(items, i) {
            let open = match layout.turning {
                Some((turning, share)) if turning == i => share,
                _ if app.outline_collapsed(i) => 0.0,
                _ => 1.0,
            };
            let arrow_hovered = app.hover == Hover::Outline(OutlineTarget::Arrow(i));
            let color = if arrow_hovered { p.text } else { p.text_dim };
            icons::chevron_turned(frame, (x + 4.0, cy), 6.0, open * FRAC_PI_2, color, clip);
            let arrow = Rect::new(x - 4.0, band.y, 16.0, band.h).intersect(band);
            hits.push((arrow, OutlineTarget::Arrow(i)));
        }
        let (glyph, color) = symbol_look(p, symbol.kind);
        icons::symbol(frame, (x + 14.0 + 1.5, cy - 7.5), glyph, color, clip);

        let label_x = row.x + LABEL + symbol.depth as f32 * TREE_INDENT;
        let room = (row.right() - 12.0 - label_x).max(0.0);
        // Markdown 标题照原文写出 `#`，一眼看出级别。
        let marks = "#".repeat(usize::from(symbol.heading));
        let label = if marks.is_empty() {
            Cow::Borrowed(symbol.name.as_str())
        } else {
            Cow::Owned(format!("{marks} {}", symbol.name))
        };
        let name = ellipsize(frame, &label, app.fonts.ui, room);
        ui_text(frame, app, label_x, row, &name, p.text, clip);
        hits.push((band, OutlineTarget::Row(i)));
    }
    let scroll = app.outline.scroll;
    draw_scrollbar(app, frame, ScrollArea::Outline, 0, list, scroll, content);
}

/// 返回符号种类的图标和颜色。
///
/// 颜色取语法高亮的颜色（函数用函数色、类型用内建类型色……），与编辑区里同类名字的颜色一致；
/// 标题和小节不是代码里的名字，用次要色。
fn symbol_look(p: &Palette, kind: SymbolKind) -> (SymbolGlyph, Color) {
    match kind {
        SymbolKind::Function | SymbolKind::Method => (SymbolGlyph::Cube, p.syntax_function),
        SymbolKind::Type => (SymbolGlyph::Card, p.syntax_builtin),
        SymbolKind::Module => (SymbolGlyph::Braces, p.syntax_keyword),
        SymbolKind::Variable => (SymbolGlyph::Field, p.syntax_property),
        SymbolKind::Heading => (SymbolGlyph::Text, p.text_muted),
        SymbolKind::Section => (SymbolGlyph::Brackets, p.text_muted),
    }
}
