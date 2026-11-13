//! 表格的排版：列宽的分配、单元格的折行和对齐。

use super::*;

impl Cx<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn table(
        &mut self,
        aligns: &[Align],
        rows: &[Vec<Inline>],
        head: bool,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Vec<Item>,
        scrolls: &mut Vec<Scroller>,
    ) -> Flow {
        // 表格的字号比正文小一级（正文 16 时为 14），表格内信息密度更高；内边距和行高随之缩放。
        let base = (self.base * TABLE_TEXT).round();
        let first_item = out.len();
        let (pad_x, pad_y) = ((base * 0.8125).round(), (base * 0.5).round());
        let cols = rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(aligns.len());
        if cols == 0 {
            return Flow {
                height: 0.0,
                first_baseline: None,
            };
        }
        let para = |header: bool, col: usize| Para {
            size: base,
            bold: header,
            color: env.color,
            line: 1.5,
            align: aligns.get(col).copied().unwrap_or(Align::None),
        };
        let live = self.caret_cell();
        let mono = base * 0.85;
        let raw_row = |room: f32, cx: &mut Self| {
            let m = cx.frame.metrics(FontKind::Mono, mono);
            Row {
                room: room.max(mono),
                size: mono,
                line_h: (base * 1.5).round(),
                ascent: m.ascent,
                descent: m.descent,
            }
        };
        let (min, max) =
            self.column_bounds(rows, cols, &live, pad_x, |r, c| para(head && r == 0, c));
        let widths = column_widths(&min, &max, width);
        let total: f32 = widths.iter().sum::<f32>() + 1.0;
        let mut top = y;
        let mut first_baseline = None;
        let mut lines = vec![top];
        for (r, row) in rows.iter().enumerate() {
            let start = out.len();
            let mut cell_x = x;
            let mut row_h: f32 = 0.0;
            for (c, w) in widths.iter().enumerate() {
                if let Some(cell) = row.get(c) {
                    let cell = match &live {
                        Some((lr, lc, source, None)) if (*lr, *lc) == (r, c) => Err(*source),
                        Some((lr, lc, _, Some(live))) if (*lr, *lc) == (r, c) => Ok(live),
                        // 空单元格同样保留一段空的原文，使其可点击，光标也有绘制位置。
                        _ if cell.text.is_empty() => Err(cell.source),
                        _ => Ok(cell),
                    };
                    let flow = match cell {
                        Err(source) => {
                            let row = raw_row(w - pad_x * 2.0, self);
                            let source = u32::try_from(source).unwrap_or(0);
                            let mut state = winc_syntax::LineState::default();
                            let piece =
                                self.code_piece("", source, "markdown", None, &mut state, row);
                            self.paste(&piece, cell_x + pad_x, top + pad_y, source, out);
                            Flow {
                                height: piece.height,
                                first_baseline: piece.first_baseline.map(|b| top + pad_y + b),
                            }
                        }
                        Ok(cell) => self.flow(
                            cell,
                            cell_x + pad_x,
                            top + pad_y,
                            w - pad_x * 2.0,
                            para(head && r == 0, c),
                            out,
                        ),
                    };
                    row_h = row_h.max(flow.height);
                    if r == 0 {
                        first_baseline = first_baseline.or(flow.first_baseline);
                    }
                }
                cell_x += w;
            }
            let row_h = row_h.max(base * 1.5) + pad_y * 2.0;
            // 表头铺底色（上方两角与外框同为圆角），与正文各行区分；正文各行之间只有横线。
            if head && r == 0 {
                out.insert(
                    start,
                    Item::Corners {
                        rect: Rect::new(x, top, total, row_h),
                        color: self.colors.table_head,
                        radii: [TABLE_RADIUS, TABLE_RADIUS, 0.0, 0.0],
                    },
                );
            }
            top += row_h;
            lines.push(top);
        }
        // 边框：圆角的外框，行之间一道横线；列之间不画竖线，按单元格的内边距区分各列。
        for &line_y in &lines[1..lines.len() - 1] {
            out.push(Item::Fill {
                rect: Rect::new(x + 1.0, line_y, total - 2.0, 1.0),
                color: self.colors.table_line,
                radius: 0.0,
            });
        }
        out.push(Item::path(
            rounded_frame(Rect::new(x, y, total, top - y + 1.0), TABLE_RADIUS),
            1.0,
            self.colors.table_frame,
        ));
        out.push(Item::Table {
            rect: Rect::new(x, y, total, top - y + 1.0),
            align: live
                .as_ref()
                .map(|(_, c, ..)| aligns.get(*c).copied().unwrap_or_default()),
            cols: widths.into(),
            rows: lines.windows(2).map(|w| w[1] - w[0]).collect(),
        });
        let mut height = top - y + 1.0;
        // 比正文宽：记下表格的各项，绘制时横向平移；下方留出滚动条的位置。
        if total > width + 0.5 {
            height += SCROLL_BAR_ROOM;
            scrolls.push(Scroller {
                items: u32::try_from(first_item).unwrap_or(u32::MAX)
                    ..u32::try_from(out.len()).unwrap_or(u32::MAX),
                rect: Rect::new(x, y, width, height),
                content: total,
            });
        }
        Flow {
            height,
            first_baseline,
        }
    }

    /// 取走并返回光标所在的单元格：（行，列，内容在原文中的起点，按排版显示的内容）。
    ///
    /// 可编辑的预览中该单元格只显示光标接触到的标记，规则同正文；空单元格的内容为 `None`，
    /// 排版时保留一段空的原文，使光标有绘制位置。光标不在这张表中时返回 `None`。
    fn caret_cell(&mut self) -> Option<CaretCell> {
        let (r, c, range) = self.cell.take()?;
        let raw = self.raw.as_ref()?;
        let segment = raw.block.get(range.clone())?;
        let content = segment.trim();
        // 空单元格的原文位置与解析结果一致（左侧 `|` 之后一个空格处），光标进出时不跳动。
        let start = if content.is_empty() {
            (range.start + 1).min(range.end)
        } else {
            range.start + segment.len() - segment.trim_start().len()
        };
        let cell = (!content.is_empty()).then(|| {
            let caret = raw.caret.saturating_sub(start).min(content.len());
            let preedit = raw.preedit.as_deref();
            let mut block = winc_preview::markdown::reveal::reveal(content, Some(caret), preedit);
            let (Block::Paragraph(inline) | Block::Heading(_, inline)) = &mut block else {
                unreachable!("reveal 只排出段落、标题");
            };
            inline.source = start;
            std::mem::take(inline)
        });
        self.preedit |= cell.is_some() && raw.preedit.is_some();
        Some((r, c, start, cell))
    }

    /// 返回 `cols` 列各自最窄的宽度和不换行时的宽度（逻辑像素），都含左右各 `pad_x` 的内边距。
    ///
    /// `live` 所指的单元格按其显示的内容度量；`para` 给出第 `r` 行、第 `c` 列的段落样式。
    fn column_bounds(
        &mut self,
        rows: &[Vec<Inline>],
        cols: usize,
        live: &Option<CaretCell>,
        pad_x: f32,
        para: impl Fn(usize, usize) -> Para,
    ) -> (Vec<f32>, Vec<f32>) {
        let (mut min, mut max) = (vec![0f32; cols], vec![0f32; cols]);
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let cell = match live {
                    Some((lr, lc, _, live)) if (*lr, *lc) == (r, c) => live.as_ref(),
                    _ => Some(cell),
                };
                let (lo, hi) = match cell {
                    Some(cell) => self.measure(cell, para(r, c)),
                    None => (0.0, 0.0),
                };
                min[c] = min[c].max(lo + pad_x * 2.0);
                max[c] = max[c].max(hi + pad_x * 2.0);
            }
        }
        (min, max)
    }
}

/// 光标所在的单元格：（行，列，内容在原文中的起点，按排版显示的内容）。
type CaretCell = (usize, usize, usize, Option<Inline>);

/// 把宽度 `width` 分给各列，返回各列的宽度（逻辑像素）。
///
/// `min`、`max` 是各列最窄的宽度和不换行时的宽度。放得下时各列取不换行的宽度；放不下时每列
/// 最宽为 `width`，更长的内容在列内换行，表格超出的部分横向滚动，不把各列压窄到逐字换行。
/// 按该规则排出的表格比 `width` 窄时按比例放宽各列。
fn column_widths(min: &[f32], max: &[f32], width: f32) -> Vec<f32> {
    let (sum_min, sum_max): (f32, f32) = (min.iter().sum(), max.iter().sum());
    // 向上取整：向下取整会少零点几像素，最后一个字放不下而换行。
    let capped: Vec<f32> = (min.iter().zip(max))
        .map(|(lo, hi)| hi.min(width.max(*lo)).ceil())
        .collect();
    if sum_max <= width {
        max.to_vec()
    } else if capped.iter().sum::<f32>() > width || sum_min >= width {
        capped
    } else {
        let t = (width - sum_min) / (sum_max - sum_min);
        (min.iter().zip(max))
            .map(|(lo, hi)| (lo + (hi - lo) * t).floor())
            .collect()
    }
}
