//! 排版的测试：折行、标题、表格、任务列表、代码块和链接排出的项。

use super::*;
use winc_render::Renderer;

/// 正文颜色。
const TEXT: Color = Color::hex(0x202020);
/// 语法标记的颜色：与正文不同，用于确认代码已着色。
const TOKEN: Color = Color::hex(0x0000ff);

/// 以宽 `width`（逻辑像素）、正文字号 16 排版 `markdown`，返回各顶层块排出的内容和高度。
fn lay(markdown: &str, width: f32) -> Vec<(Out, f32)> {
    let parsed = winc_preview::markdown::parse(markdown);
    let mut renderer =
        Renderer::offscreen(800.0, 600.0, 1.0, Color::hex(0xffffff), None, None).unwrap();
    let (images, toggled) = (HashMap::new(), HashSet::new());
    let mut memo = Memo::default();
    let mut laid = Vec::new();
    renderer.draw(|frame| {
        let accent = Color::hex(0x3574f0);
        let mut cx = Cx {
            frame,
            base: 16.0,
            token: &|_| TOKEN,
            surface: Color::hex(0xffffff),
            accent,
            colors: Colors::new(TEXT, Color::hex(0x808080), accent, false),
            dark: false,
            images: &images,
            image_path: &|_| None,
            wanted: Vec::new(),
            memo: &mut memo,
            toggled: &toggled,
            text: String::new(),
            chars: 0,
            raw: None,
            cell: None,
            preedit: false,
        };
        let env = Env {
            color: TEXT,
            depth: 0,
        };
        for block in &parsed.blocks {
            let mut out = Out::default();
            let flow = cx.block(block, 0.0, 0.0, width, env, &mut out);
            laid.push((out, flow.height));
        }
    });
    laid
}

/// 返回 `out` 中文字项的（基线，字号，颜色，文字）。
fn texts(out: &Out) -> Vec<(f32, f32, Color, String)> {
    out.items
        .iter()
        .filter_map(|item| match item {
            Item::Text {
                baseline,
                size,
                color,
                text,
                ..
            } => Some((*baseline, *size, *color, text.to_string())),
            _ => None,
        })
        .collect()
}

/// 返回不同基线的个数，即文字的行数。
fn lines(out: &Out) -> usize {
    let mut baselines: Vec<i32> = texts(out).iter().map(|t| t.0.round() as i32).collect();
    baselines.sort_unstable();
    baselines.dedup();
    baselines.len()
}

#[test]
fn paragraphs_wrap_to_the_width() {
    let text = "word ".repeat(60);
    let wide = lay(&text, 4000.0);
    let narrow = lay(&text, 200.0);
    assert_eq!(lines(&wide[0].0), 1);
    assert!(lines(&narrow[0].0) > 3, "{}", lines(&narrow[0].0));
    // 行数增加时高度随之增加。
    assert!(narrow[0].1 > wide[0].1 * 3.0);
}

#[test]
fn headings_are_larger_than_body_text_and_record_anchors() {
    let laid = lay("# Getting Started\n\nbody\n", 600.0);
    let (heading, body) = (&laid[0].0, &laid[1].0);
    assert!(texts(heading)[0].1 > texts(body)[0].1);
    assert_eq!(heading.anchors.len(), 1);
    assert_eq!(heading.anchors[0].0, "getting-started");
    assert!(body.anchors.is_empty());
}

#[test]
fn tables_report_their_columns_and_rows() {
    let laid = lay(
        "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n",
        600.0,
    );
    let table = laid[0].0.items.iter().find_map(|item| match item {
        Item::Table {
            rect, cols, rows, ..
        } => Some((*rect, cols.clone(), rows.clone())),
        _ => None,
    });
    let (rect, cols, rows) = table.expect("表格排出一个 `Item::Table`");
    assert_eq!((cols.len(), rows.len()), (3, 3));
    // 各列都有宽度，合计不超过表格的宽度；表格不超过可用宽度。
    assert!(cols.iter().all(|&w| w > 0.0));
    assert!(cols.iter().sum::<f32>() <= rect.w + 1.0);
    assert!(rect.w <= 600.0);
    assert!(laid[0].0.scrolls.is_empty());
}

#[test]
fn tables_wider_than_the_text_scroll_sideways() {
    let cell = "a-long-unbreakable-cell-value";
    let row = format!("| {} |\n", [cell; 8].join(" | "));
    let rule = format!("|{}\n", "---|".repeat(8));
    let laid = lay(&format!("{row}{rule}{row}"), 300.0);
    assert_eq!(laid[0].0.scrolls.len(), 1);
    assert!(laid[0].0.scrolls[0].max() > 0.0);
}

#[test]
fn task_list_items_have_checkboxes() {
    let laid = lay("- [x] done\n- [ ] todo\n", 600.0);
    let checks: Vec<bool> = laid[0]
        .0
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Check { checked, .. } => Some(*checked),
            _ => None,
        })
        .collect();
    assert_eq!(checks, [true, false]);
}

#[test]
fn code_blocks_keep_their_text_and_are_highlighted() {
    let laid = lay("```rust\nfn main() {}\n```\n", 600.0);
    let out = &laid[0].0;
    let code = out.items.iter().find_map(|item| match item {
        Item::Code { text, diagram, .. } => Some((text.to_string(), *diagram)),
        _ => None,
    });
    let (text, diagram) = code.expect("代码块排出一个 `Item::Code`");
    assert!(text.contains("fn main() {}"), "{text:?}");
    assert!(!diagram);
    // 关键字按语法标记的颜色绘制。
    assert!(texts(out).iter().any(|t| t.2 == TOKEN));
}

#[test]
fn links_cover_their_text() {
    let laid = lay("see [the site](https://example.com) for more\n", 600.0);
    let link = laid[0].0.items.iter().find_map(|item| match item {
        Item::Link { rect, url } => Some((*rect, url.to_string())),
        _ => None,
    });
    let (rect, url) = link.expect("链接排出一个 `Item::Link`");
    assert_eq!(url, "https://example.com");
    assert!(rect.w > 0.0 && rect.h > 0.0);
}
