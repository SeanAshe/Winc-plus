//! 表格预览的测试：xlsx 以表格打开、切换工作表、滚动、磁盘上改动后重新读、损坏的文件，
//! 以及 csv 的预览和分栏。

use winc_editor::{PreviewMode, TableState};

use super::*;

/// 把 `files`（包中路径，内容）原样存成 zip 文件并返回字节，不压缩、CRC 写 0。
///
/// 手写最小的 zip 结构，测试因此不依赖压缩库与外部样本文件；读取端不核对 CRC，写 0 即可。
fn stored_zip(files: &[(&str, String)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, content) in files {
        let (len, name_len) = (content.len() as u32, name.len() as u16);
        let offset = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend(len.to_le_bytes());
        out.extend(len.to_le_bytes());
        out.extend(name_len.to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(content.as_bytes());
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend(len.to_le_bytes());
        central.extend(len.to_le_bytes());
        central.extend(name_len.to_le_bytes());
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend([0, 0]);
    out
}

/// 构造一个工作簿的 `.xlsx` 字节内容：工作表「销售」有 `rows` 行（第一行是表头），
/// C2:D3 合并成一格，写着几行长文字；工作表「备注」只有一格 `note`。
///
/// 行数由调用方决定：需要滚动的用例给 300 行，需要完整选中一列时给 30 行，最后一行的行号
/// 因此可以写成断言里的期望值。合并格与带换行的长文字用于卡片与复制的用例。
fn workbook(rows: usize, note: &str) -> Vec<u8> {
    const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let rels = |items: &str| {
        format!(
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             {items}</Relationships>"
        )
    };
    let mut data = String::from(
        "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>城市</t></is></c>\
         <c r=\"B1\" t=\"inlineStr\"><is><t>销量</t></is></c></row>",
    );
    // 合并格里的长文字：同一段话重复 60 遍再加两行，必然换行，卡片高度才能超过几行。
    let long = "第一段很长的说明文字，".repeat(60) + "\n第二段\n第三段";
    for row in 2..=rows {
        let extra = if row == 2 {
            format!("<c r=\"C2\" t=\"inlineStr\"><is><t>{long}</t></is></c>")
        } else {
            String::new()
        };
        data.push_str(&format!(
            "<row r=\"{row}\"><c r=\"A{row}\" t=\"inlineStr\"><is><t>城市 {row}</t></is></c>\
             <c r=\"B{row}\"><v>{}</v></c>{extra}</row>",
            row * 10
        ));
    }
    stored_zip(&[
        (
            "_rels/.rels",
            rels(&format!(
                "<Relationship Id=\"r\" Type=\"{REL}/officeDocument\" Target=\"xl/workbook.xml\"/>"
            )),
        ),
        (
            "xl/workbook.xml",
            format!(
                "<workbook xmlns=\"{NS}\" xmlns:r=\"{REL}\"><sheets>\
                 <sheet name=\"销售\" sheetId=\"1\" r:id=\"rId1\"/>\
                 <sheet name=\"备注\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&format!(
                "<Relationship Id=\"rId1\" Type=\"{REL}/worksheet\" Target=\"worksheets/a.xml\"/>\
                 <Relationship Id=\"rId2\" Type=\"{REL}/worksheet\" Target=\"worksheets/b.xml\"/>"
            )),
        ),
        (
            "xl/worksheets/a.xml",
            format!(
                "<worksheet xmlns=\"{NS}\"><sheetData>{data}</sheetData>\
                 <mergeCells><mergeCell ref=\"C2:D3\"/></mergeCells></worksheet>"
            ),
        ),
        (
            "xl/worksheets/b.xml",
            format!(
                "<worksheet xmlns=\"{NS}\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\">\
                 <is><t>{note}</t></is></c></row></sheetData></worksheet>"
            ),
        ),
    ])
}

/// 当前标签读好的表格。
fn shown_table(app: &App) -> std::sync::Arc<winc_preview::table::Table> {
    let view = app.editor.active().unwrap().table.as_deref().unwrap();
    view.table().unwrap().clone()
}

#[test]
fn workbooks_open_as_tables_with_sheet_tabs() {
    let dir = search_folder("table-workbook");
    let path = dir.join("销售.xlsx");
    // 300 行远高于一屏：滚轮与滚动到底的断言才有意义。
    std::fs::write(&path, workbook(300, "第一版")).unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_tables();

    let doc = app.editor.active().unwrap();
    assert!(doc.readonly && !doc.shows_text());
    let table = shown_table(&app);
    assert_eq!(table.sheets.len(), 2);
    assert_eq!(table.sheets[0].rows[1][..2], ["城市 2", "20"]);
    // 两张工作表：底部有两个标签，表格画在编辑区中。
    assert_eq!(app.preview.table_layout.sheets.len(), 2);
    assert_eq!(app.preview.table_layout.area, app.layout.editor);
    shoot(&mut app, "table-workbook");

    // 滚轮往下滚；不会滚过底部。
    let area = app.layout.editor;
    app.pointer.mouse = (area.x + 200.0, area.y + 200.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -120.0,
    )));
    let scroll = |app: &App| app.editor.active().unwrap().table.as_ref().unwrap().scroll;
    assert!(scroll(&app).1 > 0.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0,
        -1_000_000.0,
    )));
    assert_eq!(scroll(&app).1, app.preview.table_layout.max_scroll.1);
    assert!(app.preview.table_layout.max_scroll.1 > 0.0);

    // 点第二个标签：换到「备注」，回到顶上。
    let tab = app.preview.table_layout.sheets[1];
    click(&mut app, (tab.x + tab.w / 2.0, tab.y + tab.h / 2.0));
    let view = app.editor.active().unwrap().table.as_deref().unwrap();
    assert_eq!((view.sheet, view.scroll), (1, (0.0, 0.0)));

    // 其他程序改了文件：重新读，仍停在第二张工作表。
    std::fs::write(&path, workbook(3, "第二版")).unwrap();
    let modified = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    app.editor.active_mut().unwrap().check_disk();
    app.wait_for_tables();
    assert_eq!(shown_table(&app).sheets[1].rows[0], ["第二版"]);
    assert_eq!(
        app.editor.active().unwrap().table.as_ref().unwrap().sheet,
        1
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn damaged_workbooks_say_why() {
    let dir = search_folder("table-damaged");
    let path = dir.join("坏的.xlsx");
    std::fs::write(&path, b"not a workbook").unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_tables();
    let view = app.editor.active().unwrap().table.as_deref().unwrap();
    assert!(matches!(&view.state, TableState::Failed(reason) if !reason.is_empty()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn csv_files_switch_between_text_and_table() {
    let dir = search_folder("table-csv");
    let path = dir.join("data.csv");
    std::fs::write(&path, "name,count\napple,3\n\"pear, green\",5\n").unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.shows_text() && doc.table.is_none());
    assert_eq!(doc.preview_kind(), Some(winc_editor::PreviewKind::Table));

    // 只看预览：编辑区是表格，文档暂时只读。
    app.set_preview_mode(PreviewMode::Preview);
    app.wait_for_tables();
    let doc = app.editor.active().unwrap();
    assert!(doc.readonly && !doc.shows_text());
    assert_eq!(shown_table(&app).sheets[0].rows[2], ["pear, green", "5"]);
    // 只有一张、也没截断：没有底部一栏。
    assert!(app.preview.table_layout.sheets.is_empty());

    // 分栏：左边仍可编辑，右边的表格跟着文字更新。
    app.set_preview_mode(PreviewMode::Split);
    app.wait_for_tables();
    let doc = app.editor.active().unwrap();
    assert!(!doc.readonly && doc.shows_text());
    assert_eq!(app.preview.table_layout.area, app.layout.split);
    app.run(winc_editor::Command::Insert("plum,7\n".into()));
    app.wait_for_tables();
    assert_eq!(shown_table(&app).sheets[0].rows[0], ["plum", "7"]);
    shoot(&mut app, "table-csv-split");

    // 回到代码：不再有表格。
    app.set_preview_mode(PreviewMode::Source);
    let doc = app.editor.active().unwrap();
    assert!(doc.table.is_none() && doc.shows_text() && !doc.readonly);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cards_show_cut_off_cells_and_follow_the_selected_cell() {
    use winit::keyboard::NamedKey::*;

    let dir = search_folder("table-card");
    let path = dir.join("notes.csv");
    // 五个格分别验证：换行后只剩空白、换行后还有文字、短文字、两种长度的长文字（行数不同，
    // 卡片高度也不同）。
    let (short, long) = ("很长".repeat(40), "很长".repeat(120));
    std::fs::write(&path, format!("a,\"x\n\",\"x\ny\",{short},{long}\n")).unwrap();
    let mut app = app_with(&path, false);
    app.set_preview_mode(PreviewMode::Preview);
    app.wait_for_tables();
    paint(&mut app);
    let card = |app: &App| app.preview.table_layout.card.h;

    // 换行之后只剩空白：显示得全，没有卡片；换行之后还有文字：有卡片。
    table_key(&mut app, ArrowRight, false);
    assert_eq!(card(&app), 0.0);
    table_key(&mut app, ArrowRight, false);
    assert!(card(&app) > 0.0);
    // 一行放不下的文字：卡片高度随所选格的文字变化，回到原来的格时也一样。
    table_key(&mut app, ArrowRight, false);
    let first = card(&app);
    assert!(first > 0.0);
    table_key(&mut app, ArrowRight, false);
    assert!(card(&app) > first, "{first} {}", card(&app));
    table_key(&mut app, ArrowLeft, false);
    assert_eq!(card(&app), first);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 按一个键（交给表格；`shift` 时按着 `⇧`），然后绘制一帧。
///
/// # Panics
///
/// 表格不处理该键时 panic，用于断言按键被表格接住。
fn table_key(app: &mut App, key: winit::keyboard::NamedKey, shift: bool) {
    app.pointer.modifiers = if shift {
        winit::keyboard::ModifiersState::SHIFT
    } else {
        winit::keyboard::ModifiersState::empty()
    };
    assert!(app.table_key(&winit::keyboard::Key::Named(key), false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    paint(app);
}

/// 当前标签表格的选区。
fn selection(app: &App) -> ((usize, usize), (usize, usize)) {
    app.editor
        .active()
        .unwrap()
        .table
        .as_ref()
        .unwrap()
        .selection
}

/// 窗口中第 `row` 行第 `column` 列的中心。
fn cell_center(app: &App, row: usize, column: usize) -> (f32, f32) {
    let layout = &app.preview.table_layout;
    let x = layout.body.x + (layout.edges[column] + layout.edges[column + 1]) / 2.0;
    let y = layout.body.y + (row as f32 + 0.5) * layout.row_h;
    (x, y)
}

#[test]
fn cells_are_selected_copied_and_merged_cells_read_in_full() {
    use winit::keyboard::NamedKey::*;

    let dir = search_folder("table-select");
    let path = dir.join("销售.xlsx");
    // 30 行：最后一行是第 29 行，点列名选整列、全选、⌘↑ 到第一行等断言直接写行号。
    std::fs::write(&path, workbook(30, "备注")).unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_tables();
    assert_eq!(selection(&app), ((0, 0), (0, 0)));
    // A1 的文字很短，放得下：没有卡片。
    assert_eq!(app.preview.table_layout.card.h, 0.0);
    let body = app.preview.table_layout.body;

    // 点 B3，往右进入合并区域 C2:D3：选中区域左上角 C2，文字放不下，格子下方浮出卡片换行显示
    // 完整的长文字；表格本身不动。
    let at = cell_center(&app, 2, 1);
    click(&mut app, at);
    assert_eq!(selection(&app), ((2, 1), (2, 1)));
    table_key(&mut app, ArrowRight, false);
    assert_eq!(selection(&app), ((1, 2), (1, 2)));
    let card = app.preview.table_layout.card;
    let merged = Rect::new(
        body.x + app.preview.table_layout.edges[2],
        body.y + app.preview.table_layout.row_h,
        1.0,
        1.0,
    );
    assert!(card.h > app.preview.table_layout.row_h * 3.0, "{card:?}");
    assert!(
        card.y > merged.y + app.preview.table_layout.row_h && card.x >= merged.x - 1.0,
        "{card:?}"
    );
    assert_eq!(app.preview.table_layout.body, body);
    // 点在卡片上不改选区；滚轮在卡片上滚动卡片中的文字，不滚表格。
    click(&mut app, (card.x + 20.0, card.y + 20.0));
    assert_eq!(selection(&app), ((1, 2), (1, 2)));
    let table_scroll = app.editor.active().unwrap().table.as_ref().unwrap().scroll;
    app.pointer.mouse = (card.x + 20.0, card.y + 20.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -40.0,
    )));
    let view = app.editor.active().unwrap().table.as_ref().unwrap();
    assert!(app.preview.table_layout.card_max > 0.0 && view.card_scroll > 0.0);
    assert_eq!(view.scroll, table_scroll);
    // 按在格子上拖着选的时候不显示卡片，松开后再显示。
    let at = cell_center(&app, 1, 2);
    app.on_mouse_move(at.0, at.1);
    app.on_mouse_down(MouseButton::Left);
    paint(&mut app);
    assert_eq!(app.preview.table_layout.card.h, 0.0);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!(app.preview.table_layout.card.h > 0.0);
    // Esc 收起卡片，选区不变；换到另一格再回来时重新显示。
    table_key(&mut app, Escape, false);
    assert_eq!(app.preview.table_layout.card.h, 0.0);
    assert_eq!(selection(&app), ((1, 2), (1, 2)));
    table_key(&mut app, ArrowLeft, false);
    table_key(&mut app, ArrowRight, false);
    assert!(app.preview.table_layout.card.h > 0.0);
    shoot(&mut app, "table-merged");
    // 复制合并区域：左上角那一格的文字。
    app.copy(false);
    let copied = app.clipboard.copied.clone().unwrap();
    assert!(
        copied.starts_with("第一段很长的说明文字") && copied.ends_with("第三段"),
        "{copied}"
    );

    // 从合并区域往下：从区域的下边出去，到 C4。
    table_key(&mut app, ArrowDown, false);
    assert_eq!(selection(&app), ((3, 2), (3, 2)));

    // ⇧ 点 A2：选区 A2:C4 碰到合并区域，扩大到 A2:D4；复制得到制表符分隔的三行四列，带换行
    // 的格加引号，合并区域中其余格为空。
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    let at = cell_center(&app, 1, 0);
    click(&mut app, at);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(selection(&app), ((3, 2), (1, 0)));
    app.copy(false);
    let copied = app.clipboard.copied.clone().unwrap();
    assert!(copied.starts_with("城市 2\t20\t\"第一段"), "{copied}");
    assert!(
        copied.ends_with("第三段\"\t\n城市 3\t30\t\t\n城市 4\t40\t\t\n"),
        "{copied}"
    );

    // 拖着选：从 A10 拖到 B12。
    let at = cell_center(&app, 9, 0);
    click(&mut app, at);
    app.on_mouse_down(MouseButton::Left);
    let (x, y) = cell_center(&app, 11, 1);
    app.on_mouse_move(x, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(selection(&app), ((9, 0), (11, 1)));

    // 点行号选整行，点列名选整列，全选。
    let numbers = app.preview.table_layout.numbers;
    let at = (numbers.x + 5.0, cell_center(&app, 4, 0).1);
    click(&mut app, at);
    assert_eq!(selection(&app), ((4, 0), (4, 3)));
    let header = app.preview.table_layout.header;
    let at = (cell_center(&app, 0, 1).0, header.y + 5.0);
    click(&mut app, at);
    assert_eq!(selection(&app), ((0, 1), (29, 1)));
    app.focus = Focus::Editor;
    app.run_action(crate::keymap::Action::SelectAll);
    assert_eq!(selection(&app), ((0, 0), (29, 3)));
    table_key(&mut app, Escape, false);
    assert_eq!(selection(&app), ((29, 3), (29, 3)));

    // ⌘↑ 到第一行，Home 到行首；回车往下、⇧回车往上；翻页往下滚到当前格可见。
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert!(app.table_key(&winit::keyboard::Key::Named(ArrowUp), true));
    assert_eq!(selection(&app).1, (0, 3));
    table_key(&mut app, Home, false);
    table_key(&mut app, Enter, false);
    assert_eq!(selection(&app).1, (1, 0));
    table_key(&mut app, Enter, true);
    assert_eq!(selection(&app).1, (0, 0));
    for _ in 0..3 {
        table_key(&mut app, PageDown, false);
    }
    let head = selection(&app).1;
    let view = app.editor.active().unwrap().table.as_ref().unwrap();
    assert!(head.0 > 10 && view.scroll.1 > 0.0);
    let (_, y) = cell_center(&app, head.0, head.1);
    let body = app.preview.table_layout.body;
    assert!(y - view.scroll.1 < body.bottom() && y - view.scroll.1 > body.y);

    std::fs::remove_dir_all(&dir).unwrap();
}
