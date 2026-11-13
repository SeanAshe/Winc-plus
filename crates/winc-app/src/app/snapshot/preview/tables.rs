//! 表格预览的测试：工作表、列宽、滚动和单元格的选择。

use super::*;

#[test]
fn clicking_an_empty_table_cell_puts_the_caret_in_it() {
    let (mut app, dir) = live_doc(
        "md-table-empty",
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n|  |  |\n\n后文\n",
    );
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let at = |c: char| shown.chars().position(|s| s == c).unwrap();
    let (a, b, c) = (
        point_of(&app, at('a')),
        point_of(&app, at('b')),
        point_of(&app, at('c')),
    );
    let row = c.1 - a.1;
    let click = |app: &mut App, (x, y): (f32, f32)| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        settle(app);
    };
    // 新增行、列中的空单元格：光标进入被点击的单元格，不进入相邻的非空单元格。
    click(&mut app, (b.0 + 8.0, c.1 + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | | |\n|  |  |\n\n后文\n"
    );
    click(&mut app, (c.0 + 8.0, c.1 + row + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n| | |  |\n\n后文\n"
    );
    click(&mut app, (b.0 + 8.0, c.1 + row + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c |  |\n|  | | |\n\n后文\n"
    );
    // 点击非空单元格中的空白处：光标仍在该单元格。
    click(&mut app, (a.0 - 6.0, a.1 + 4.0));
    assert_eq!(
        with_caret(&app),
        "前文\n\n| |a | b |\n|---|---|\n| c |  |\n|  |  |\n\n后文\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_table_cell_takes_one_click_from_anywhere() {
    use winc_preview::markdown::cells;
    let filler = "前文\n\n".repeat(40);
    let table = "| 名称 | `code` | **粗** |\n|:--|:-:|--:|\n| 中文内容比较长的一格 |  | x |\n\
                 |  |  |  |\n";
    let (mut app, dir) = live_doc(
        "md-table-every",
        &format!("{filler}上面 **粗** 段\n\n{table}\n下面 `码` 段\n\n{filler}"),
    );
    app.preview.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 2);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let a = shown
        .find("名称")
        .map(|b| shown[..b].chars().count())
        .unwrap();
    let top = crate::app::preview::markdown::laid::offset_top(&app, a).unwrap();
    app.preview
        .markdown
        .scroll_to(top - app.preview.markdown.area.h / 2.0);
    settle(&mut app);
    let start = doc_text(&app).find("| 名称").unwrap();
    // 光标所在的行号（不计 `|---|`）和列号。
    let cell_of = |app: &App| {
        let text = doc_text(app);
        let caret = app.live_caret_source()?;
        let mut at = start;
        let mut row = 0;
        for line in text[start..].split('\n').take(4) {
            if (at..=at + line.len()).contains(&caret) {
                let col = cells::cells(line)
                    .iter()
                    .position(|c| caret - at <= c.segment.end)?;
                return Some((row, col));
            }
            if !cells::is_delimiter(line) {
                row += 1;
            }
            at += line.len() + 1;
        }
        None
    };
    let click = |app: &mut App, (x, y): (f32, f32)| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        paint(app);
        app.on_mouse_move(x + 1.0, y + 1.0);
        paint(app);
        app.on_mouse_up(MouseButton::Left);
        settle(app);
    };
    let grid = crate::view::preview::markdown::tests::table_cells(&app);
    assert_eq!((grid.len(), grid[0].len()), (3, 3));
    let mut failures = Vec::new();
    for from in ["上面", "下面", "x"] {
        for r in 0..3 {
            for c in 0..3 {
                put_caret(&mut app, from, true);
                let scroll = app.preview.markdown.scroll();
                let grid = crate::view::preview::markdown::tests::table_cells(&app);
                let cell = grid[r][c];
                // 单元格中部和左侧（空单元格的光标位于左侧）。
                for (x, y) in [
                    (cell.x + cell.w / 2.0, cell.y + cell.h / 2.0),
                    (cell.x + 3.0, cell.y + cell.h / 2.0),
                ] {
                    click(&mut app, (x, y));
                    let got = cell_of(&app);
                    let moved = app.preview.markdown.scroll() - scroll;
                    // 新显示的标记正常淡入；块不应滑动。
                    let motion = app
                        .preview
                        .markdown
                        .motion
                        .as_ref()
                        .is_some_and(|m| m.from.iter().any(|d| d.abs() >= 0.5));
                    if got != Some((r, c)) || moved != 0.0 || motion {
                        failures.push(format!(
                            "从 {from} 点 ({r},{c}) 于 ({x:.0},{y:.0})：进了 {got:?}，\
                             滚了 {moved}，滑动 {motion}"
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tables_are_edited_cell_by_cell() {
    use winit::keyboard::{Key, NamedKey};
    let tab = Key::Named(NamedKey::Tab);

    let (mut app, dir) = live_doc("md-cells", "| a | b |\n|---|---|\n| **粗** | d |\n\n后文\n");
    put_caret(&mut app, "粗", false);
    app.run(winc_editor::Command::Insert("很".into()));
    paint(&mut app);
    // 只有光标所在的那一格显示原文，表格仍按排版显示。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("**很粗**"), "{shown}");
    assert!(
        !shown.contains("|---|") && !shown.contains("| a |"),
        "{shown}"
    );
    let selected = |app: &App| {
        let doc = app.editor.active().unwrap();
        let rope = doc.text().rope().unwrap();
        rope.slice(doc.selection.from()..doc.selection.to())
            .to_string()
    };
    // Tab 选中下一格，在最后一格按 Tab 新增一行；⇧Tab 返回上一格。
    assert!(app.markdown_key(&tab, false));
    assert_eq!(selected(&app), "d");
    settle(&mut app);
    assert!(app.markdown_key(&tab, false));
    assert_eq!(
        with_caret(&app),
        "| a | b |\n|---|---|\n| **很粗** | d |\n| | |  |\n\n后文\n"
    );
    settle(&mut app);
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&tab, false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(selected(&app), "d");
    settle(&mut app);
    // 回车到下一行的同一列。
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    assert_eq!(
        with_caret(&app),
        "| a | b |\n|---|---|\n| **很粗** | d |\n|  | | |\n\n后文\n"
    );
    // 位于表头下方的分隔行上：整张表显示原文，Tab 交给编辑区。
    put_caret(&mut app, "|---", true);
    assert!(
        crate::app::preview::markdown::laid::full_text(&app)
            .unwrap()
            .contains("|---|")
    );
    assert!(!app.markdown_key(&tab, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_table_toolbar_adds_aligns_and_deletes() {
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    let (mut app, dir) = live_doc(
        "md-table-tools",
        "前文\n\n| a | b |\n|---|---|\n| c | d |\n\n后文\n",
    );
    put_caret(&mut app, "c", true);
    // 光标在表格中时上方显示工具栏，单击按钮执行操作，不移动光标。
    let click = |app: &mut App, op: &str| {
        let url = format!("{}{op}", crate::app::preview::markdown::laid::TABLE);
        let (rect, _) = app
            .hits
            .markdown_links
            .iter()
            .find(|(_, u)| *u == url)
            .cloned()
            .unwrap_or_else(|| panic!("工具栏上有 {op}"));
        super::click(app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
        settle(app);
    };
    click(&mut app, "column");
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a |  | b |\n|---|---|---|\n| c | | | d |\n\n后文\n"
    );
    click(&mut app, "center");
    assert!(doc_text(&app).contains("|---|:---:|---|"));
    // 再次点击已选中的对齐方式：取消对齐。
    click(&mut app, "none");
    assert!(doc_text(&app).contains("|---|---|---|"));
    click(&mut app, "delete-column");
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | |d |\n\n后文\n"
    );
    // ⌘↩ 在下方插入一行，⌘⇧⌫ 删除当前行；一次撤销即可恢复。
    app.pointer.modifiers = command_modifiers();
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), true));
    app.pointer.modifiers = ModifiersState::empty();
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | d |\n|  | | |\n\n后文\n"
    );
    app.pointer.modifiers = command_modifiers() | ModifiersState::SHIFT;
    assert!(app.markdown_key(&Key::Named(NamedKey::Backspace), true));
    app.pointer.modifiers = ModifiersState::empty();
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "前文\n\n| a | b |\n|---|---|\n| c | |d |\n\n后文\n"
    );
    click(&mut app, "delete");
    assert_eq!(doc_text(&app), "前文\n\n\n后文\n");
    app.run(winc_editor::Command::Undo);
    assert!(doc_text(&app).contains("| c | d |"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：展开的标题列表覆盖宽表格的水平滚动条时，鼠标在列表内经过该滚动条，列表收起。
#[test]
fn the_heading_list_stays_open_over_a_table_scrollbar() {
    use crate::app::preview::markdown::TocTarget;

    let head = format!("|{}", " 很长的列标题 |".repeat(24));
    let rule = format!("|{}", "---|".repeat(24));
    let row = format!("|{}", " 单元格 |".repeat(24));
    let headings: String = (1..=12)
        .map(|i| format!("## 第 {i} 节\n\n正文\n\n"))
        .collect();
    let (mut app, dir) = live_doc(
        "md-toc-table",
        &format!("# 标题\n\n{head}\n{rule}\n{row}\n\n{headings}"),
    );
    // 表格的滚动条在鼠标经过该表格后开始响应点击，鼠标经过另一个表格后不再响应。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let cell = point_of(&app, shown[..shown.find("单元格").unwrap()].chars().count());
    app.on_mouse_move(cell.0, cell.1);
    paint(&mut app);
    let bars = (app.hits.markdown_toc.iter())
        .find(|(_, t)| *t == TocTarget::Bars)
        .unwrap()
        .0;
    app.on_mouse_move(bars.x + 4.0, bars.y + 10.0);
    settle(&mut app);
    assert!(matches!(app.pointer.hover, Hover::MarkdownToc(_)));

    let bar = app.hits.markdown_table_bar.expect("表格有滚动条");
    // 标题列表展开后有两个 `Bars` 区域：指示条和覆盖其上的卡片，卡片的宽度较大。
    let card = (app.hits.markdown_toc.iter())
        .filter(|(_, t)| *t == TocTarget::Bars)
        .map(|(rect, _)| *rect)
        .max_by(|a, b| a.w.total_cmp(&b.w))
        .unwrap();
    let edge = (bar.track.x + 4.0, bar.track.bottom() - 4.0);
    assert!(card.contains(edge.0, edge.1), "{card:?} {:?}", bar.track);
    app.on_mouse_move(edge.0, edge.1);
    paint(&mut app);
    assert!(
        matches!(app.pointer.hover, Hover::MarkdownToc(_)),
        "{:?}",
        app.pointer.hover
    );
    assert!(
        app.hits
            .markdown_toc
            .iter()
            .any(|(_, t)| *t != TocTarget::Bars)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：列很多的表格不把各列压窄到逐字换行，而是按自然宽度排版、超出正文的部分横向滚动；横向
/// 滚动后点击仍落在对应的字上；编辑表格时光标移到看不见的列，表格横向滚动使光标可见。
#[test]
fn wide_tables_keep_their_columns_and_scroll_sideways() {
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta;

    let head = "| 编号 | 品名 | 类别 | 颜色 | 尺寸 | 重量 | 口感 | 货号 | 条码 | 上架日期 | 批次 \
                | 规格 | 状态 | 单价 | 库存 | 好评率 | 等级 | 备注 |";
    let rule = format!("|{}", "---|".repeat(18));
    let row = "| 1 | 苹果 | 水果 | 红色 | 中号 | 200 克 | 脆甜 | SKU-FRUIT-APPLE-0001 \
               | 6901234567890 | 2024-03-15 | B001 | 每箱十二个 | 在售 | 6.5 | 120 | 98% | 一级 \
               | 需冷藏保存运输 |";
    let (mut app, dir) = live_doc(
        "md-wide-table",
        &format!("前文\n\n{head}\n{rule}\n{row}\n\n后文\n"),
    );
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let at = |c: &str| shown[..shown.find(c).unwrap()].chars().count();
    let offset_at = crate::app::preview::markdown::laid::offset_at;

    // 表头「编号」排在同一行，没有被压成逐字换行。
    let (first, second) = (point_of(&app, at("编")), point_of(&app, at("号")));
    assert!((first.1 - second.1).abs() < 1.0, "{first:?} {second:?}");

    // 在表格上横向滚动：表格左移，同一位置下的字随之改变，点击仍对应移过来的字。
    let probe = (first.0, point_of(&app, at("苹")).1);
    let before = offset_at(&app, probe.0, probe.1);
    app.on_mouse_move(probe.0, probe.1);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        -400.0, 0.0,
    )));
    paint(&mut app);
    let after = offset_at(&app, probe.0, probe.1);
    assert_ne!(before, after);
    // 表格外（「前文」一行）的横向滚动不移动表格。
    let outside = point_of(&app, at("前"));
    app.on_mouse_move(outside.0, outside.1);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        -400.0, 0.0,
    )));
    paint(&mut app);
    assert_eq!(offset_at(&app, probe.0, probe.1), after);

    // 滚动条与其他区域一致：鼠标移到表格底边时可以按住滑块拖动，拖到最右后滚动到尽头。
    let bar = app.hits.markdown_table_bar.expect("表格有滚动条");
    let edge = (bar.track.x + 4.0, bar.track.bottom() - 4.0);
    app.on_mouse_move(edge.0, edge.1);
    paint(&mut app);
    assert_eq!(
        app.pointer.hover,
        Hover::Scrollbar(crate::layout::Scrollbar::MarkdownTable)
    );
    let thumb_x =
        crate::layout::hscrollbar_thumb(bar.track, app.markdown_table_scroll(bar.key), bar.content)
            .unwrap()
            .x;
    app.on_mouse_move(thumb_x + 2.0, edge.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(bar.track.right() + 200.0, edge.1);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert_eq!(
        app.markdown_table_scroll(bar.key),
        bar.content - bar.track.w
    );

    // 滚回最左，光标移到最后一列：表格横向滚动，最后一列的字出现在预览区域内。
    app.on_mouse_move(probe.0, probe.1);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        4000.0, 0.0,
    )));
    paint(&mut app);
    assert_eq!(offset_at(&app, probe.0, probe.1), before);
    let cell = point_of(&app, at("苹"));
    app.on_mouse_move(cell.0, cell.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    put_caret(&mut app, "需冷藏", false);
    let last = point_of(&app, at("需冷藏"));
    assert!(
        app.preview.markdown.area.contains(last.0, last.1),
        "{last:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
