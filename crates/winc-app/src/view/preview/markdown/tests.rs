//! `markdown` 的测试辅助：读取排版结果中图形超出内容的宽度和表格单元格的位置，供预览的
//! 界面测试检查和点击。

use super::*;

pub(crate) use winc_preview::markdown::slug;

/// 绘制的框和线超出内容宽度的量（逻辑像素，未超出时不大于 0），供测试检查图是否完整显示。
pub(crate) fn overflow(app: &App) -> f32 {
    let Some(laid) = app.markdown.layout.as_ref() else {
        return 0.0;
    };
    let right = laid
        .blocks
        .iter()
        .flat_map(|block| block.1.items.iter())
        .filter_map(|item| match item {
            Item::Fill { rect, .. } | Item::Path { rect, .. } | Item::Polygon { rect, .. } => {
                Some(rect.right())
            }
            _ => None,
        })
        .fold(0.0, f32::max);
    right - laid.key.width
}

/// 第一张表格各单元格在窗口中的范围，按行排列，供测试点击单元格。
pub(crate) fn table_cells(app: &App) -> Vec<Vec<Rect>> {
    let Some(laid) = app.markdown.layout.as_ref() else {
        return Vec::new();
    };
    let origin = app.markdown.area.y + laid.pad - app.markdown.scroll();
    for (block, top) in laid.blocks.iter().zip(&laid.tops) {
        for item in block.1.items.iter() {
            if let Item::Table {
                rect, cols, rows, ..
            } = item
            {
                let mut y = origin + top + rect.y;
                return rows
                    .iter()
                    .map(|h| {
                        let mut x = laid.left + rect.x;
                        let row = cols
                            .iter()
                            .map(|w| {
                                let cell = Rect::new(x, y, *w, *h);
                                x += w;
                                cell
                            })
                            .collect();
                        y += h;
                        row
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}
