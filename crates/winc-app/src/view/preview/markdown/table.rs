//! 预览中比正文宽的表格：两侧的渐隐、横向滚动条，以及可编辑的预览中表格上方的工具栏。

use super::*;

/// 画可横向滚动的表格被裁掉一侧的渐隐，提示可以横向滚动：`strip` 是显示区域（窗口坐标），
/// `off` 是滚动位置。渐隐的强度随离该侧尽头的距离在 [`TABLE_FADE`] 内线性变化，裁剪到 `area` 内。
pub(super) fn draw_table_fades(
    frame: &mut Frame<'_>,
    p: &Palette,
    scroller: &winc_typeset::Scroller,
    strip: Rect,
    off: f32,
    area: Rect,
) {
    let max = scroller.max();
    if max <= 0.0 {
        return;
    }
    let table = Rect::new(
        strip.x,
        strip.y,
        strip.w,
        strip.h - winc_typeset::SCROLL_BAR_ROOM,
    );
    let left = (off / TABLE_FADE).min(1.0);
    let right = ((max - off) / TABLE_FADE).min(1.0);
    frame.within(table.intersect(area), |frame| {
        if left > 0.0 {
            let edge = Rect::new(table.x, table.y, TABLE_FADE, table.h);
            frame.with_opacity(left, |f| crate::view::fade(f, edge, p.editor_bg, true));
        }
        if right > 0.0 {
            let edge = Rect::new(table.right() - TABLE_FADE, table.y, TABLE_FADE, table.h);
            frame.with_opacity(right, |f| crate::view::fade(f, edge, p.editor_bg, false));
        }
    });
}

/// 画表格 `chosen`（表格、滚动位置、是否因鼠标所在而选中）的水平滚动条，与其他区域的滚动条
/// 一致：平时隐藏，滚动、鼠标移到底边或拖动时显示（见 `App::scrollbar_look`），可以拖动。
pub(super) fn draw_table_scrollbar(
    app: &mut App,
    frame: &mut Frame<'_>,
    chosen: Option<(TableBar, f32, bool)>,
    area: Rect,
) {
    app.hits.markdown_table_bar = chosen.map(|(bar, ..)| bar);
    let Some((bar, off, hovered)) = chosen else {
        return;
    };
    if hovered {
        app.preview.markdown.last_table = Some(bar.key);
    }
    let thumb = crate::layout::hscrollbar_thumb(bar.track, off, bar.content);
    // 滚动条状态的所属对象按表格区分：换到另一个表格时立即隐藏上一个表格的滚动条。
    let owner = ((bar.key.0 as u64) << 32) | bar.key.1 as u64;
    let look = app.scrollbar_look(
        ScrollArea::MarkdownTable,
        owner,
        off,
        thumb.map(|_| bar.track),
    );
    if let Some(thumb) = thumb {
        let p = app.palette;
        frame.within(area, |frame| crate::view::draw_thumb(frame, p, thumb, look));
    }
}

/// 表格工具栏的按钮：链接中 [`TABLE`] 之后的操作名和按钮文字（三个对齐按钮绘制为图标）。
fn table_tools() -> [(&'static str, &'static str); 8] {
    [
        ("left", ""),
        ("center", ""),
        ("right", ""),
        ("row", t!("preview-table-insert-row")),
        ("column", t!("preview-table-insert-column")),
        ("delete-row", t!("preview-table-delete-row")),
        ("delete-column", t!("preview-table-delete-column")),
        ("delete", t!("preview-table-delete")),
    ]
}

/// 在光标所在的表格 `table`（窗口坐标）上方绘制工具栏。
///
/// 按钮包括光标所在列的对齐方式（`align` 对应的按钮显示为按下）、插入行和列、删除当前行和列、
/// 删除整张表，并作为链接加入 `links`。上方空间不足时工具栏贴紧预览上边缘，覆盖在表格上。
#[allow(clippy::too_many_arguments)]
pub(super) fn table_toolbar(
    frame: &mut Frame<'_>,
    p: &Palette,
    table: Rect,
    align: winc_preview::markdown::Align,
    area: Rect,
    mouse: (f32, f32),
    links: &mut Vec<(Rect, String)>,
) {
    use winc_preview::markdown::Align;
    // 工具栏高 26、内边距 3、按钮字号 12（逻辑像素，经验值）。
    let (h, pad, size) = (26.0, 3.0, 12.0);
    let tools = table_tools();
    let widths: Vec<f32> = tools
        .iter()
        .map(|(_, label)| {
            if label.is_empty() {
                h - pad * 2.0
            } else {
                frame.measure(winc_render::FontKind::Ui, size, label) + 16.0
            }
        })
        .collect();
    // 对齐与插入之间、插入与删除之间、删除整张表之前各绘制一条竖线分隔。
    let gaps = [3, 5, 7];
    let w = widths.iter().sum::<f32>() + pad * 2.0 + gaps.len() as f32 * 9.0;
    let y = (table.y - h - 6.0).max(area.y + 2.0);
    let bar = Rect::new(table.x, y, w, h);
    frame.shadow(bar, 6.0, 8.0, 2.0, p.shadow, None);
    frame.rect_clipped(bar, p.popup_bg, 6.0, area);
    let mut x = bar.x + pad;
    for (i, ((op, label), bw)) in tools.iter().zip(&widths).enumerate() {
        if gaps.contains(&i) {
            let line = Rect::new(x + 4.0, y + 7.0, 1.0, h - 14.0);
            frame.rect_clipped(line, p.border, 0.0, area);
            x += 9.0;
        }
        let button = Rect::new(x, y + pad, *bw, h - pad * 2.0);
        let on = match *op {
            "left" => align == Align::Left,
            "center" => align == Align::Center,
            "right" => align == Align::Right,
            _ => false,
        };
        if on {
            frame.rect_clipped(button, p.accent.alpha(0.18), 4.0, area);
        } else if button.contains(mouse.0, mouse.1) {
            frame.rect_clipped(button, p.text.alpha(0.08), 4.0, area);
        }
        let color = if on { p.accent } else { p.text_muted };
        if label.is_empty() {
            // 对齐图标：四条长短交错的横线，按对齐方式靠左、居中或靠右排列。
            let (cx, cy) = (button.x + button.w / 2.0, button.y + button.h / 2.0);
            for (k, len) in [12.0, 8.0, 12.0, 8.0].into_iter().enumerate() {
                let lx = match *op {
                    "left" => cx - 6.0,
                    "center" => cx - len / 2.0,
                    _ => cx + 6.0 - len,
                };
                let ly = cy - 5.5 + k as f32 * 3.5;
                frame.rect_clipped(Rect::new(lx, ly, len, 1.5), color, 0.0, area);
            }
        } else {
            let baseline = button.y + button.h / 2.0 + size * 0.35;
            frame.text(
                winc_render::FontKind::Ui,
                size,
                x + 8.0,
                baseline,
                label,
                color,
                area,
            );
        }
        // 再次点击已选中的对齐方式时取消对齐，使对齐按钮可以切换开关。
        let op = if on { "none" } else { op };
        links.push((button, format!("{TABLE}{op}")));
        x += bw;
    }
}
