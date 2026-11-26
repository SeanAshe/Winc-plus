//! 排版的测试：折行、标题、表格、任务列表、代码块和链接排出的项。

use super::*;
use winc_render::Renderer;

/// 正文颜色。
const TEXT: Color = Color::hex(0x202020);
/// 语法标记的颜色：与正文不同，用于确认代码已着色。
const TOKEN: Color = Color::hex(0x0000ff);

/// 以宽 `width`（逻辑像素）、正文字号 16 排版 `markdown`，返回各顶层块排出的内容和高度。
fn lay(markdown: &str, width: f32) -> Vec<(Out, f32)> {
    lay_with(markdown, width, &HashMap::new())
}

/// 同 [`lay`]，图片按网址在 `images` 中查找。
fn lay_with(markdown: &str, width: f32, images: &HashMap<PathBuf, ImageSlot>) -> Vec<(Out, f32)> {
    lay_in(markdown, width, images, (0.0, 0.0))
}

/// 同 [`lay`]，比一栏宽的表格可以占用两侧的留白 `bleed`（见 [`Cx::bleed`]）。
fn lay_bled(markdown: &str, width: f32, bleed: (f32, f32)) -> Vec<(Out, f32)> {
    lay_in(markdown, width, &HashMap::new(), bleed)
}

fn lay_in(
    markdown: &str,
    width: f32,
    images: &HashMap<PathBuf, ImageSlot>,
    bleed: (f32, f32),
) -> Vec<(Out, f32)> {
    let parsed = winc_preview::markdown::parse(markdown);
    let mut renderer =
        Renderer::offscreen(800.0, 600.0, 1.0, Color::hex(0xffffff), None, None).unwrap();
    let toggled = HashSet::new();
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
            images,
            image_path: &|url| (!images.is_empty()).then(|| PathBuf::from(url)),
            wanted: Vec::new(),
            memo: &mut memo,
            toggled: &toggled,
            text: String::new(),
            chars: 0,
            raw: None,
            cell: None,
            preedit: false,
            line_numbers: false,
            tall: f32::INFINITY,
            repeat: 0,
            toggles: HashMap::new(),
            bleed,
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

/// 返回各块中表格的外框（块里的坐标）。
fn tables(laid: &[(Out, f32)]) -> Vec<Rect> {
    (laid.iter().flat_map(|(out, _)| &out.items))
        .filter_map(|item| match item {
            Item::Table { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect()
}

/// 比一栏宽的表格占用两侧留白：放得下时以一栏为中心展开，一侧不够时移到另一侧；放不下时
/// 占满留白横向滚动；缩进的表格只向右展开。
#[test]
fn wide_tables_spread_into_the_margins() {
    let cell = "a-long-unbreakable-cell-value";
    let table = |cols: usize| {
        let row = format!("| {} |\n", vec![cell; cols].join(" | "));
        format!("{row}|{}\n{row}", "---|".repeat(cols))
    };
    let width = tables(&lay(&table(3), 10_000.0))[0].w;
    let column = (width - 200.0).round();

    let laid = lay_bled(&table(3), column, (300.0, 300.0));
    let rect = tables(&laid)[0];
    assert!(laid[0].0.scrolls.is_empty());
    assert!(
        (rect.x + ((rect.w - column) / 2.0).round()).abs() <= 1.0,
        "{rect:?}"
    );
    let laid = lay_bled(&table(3), column, (300.0, 40.0));
    let rect = tables(&laid)[0];
    assert!(laid[0].0.scrolls.is_empty());
    assert!((rect.right() - (column + 40.0)).abs() <= 1.0, "{rect:?}");
    // 超出的宽度只比两侧留白之和多不到半个像素：仍然放下，不横向滚动。
    let total = lay_bled(&table(3), column, (0.0, 0.0))[0].0.scrolls[0].content;
    let laid = lay_bled(&table(3), column, (100.0, total - column - 100.3));
    assert!(laid[0].0.scrolls.is_empty());

    let laid = lay_bled(&table(8), column, (100.0, 50.0));
    let strip = laid[0].0.scrolls[0].rect;
    assert_eq!((strip.x, strip.w), (-100.0, column + 150.0));

    let nested = format!("- item\n\n  {}", table(3).replace('\n', "\n  "));
    let rect = tables(&lay_bled(&nested, column, (300.0, 300.0)))[0];
    assert!(rect.x > 0.0, "{rect:?}");
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
fn the_code_of_a_fenced_block_is_the_text_between_its_fences() {
    assert_eq!(
        fenced_code("```rust\nfn a() {}\n\nfn b() {}\n```"),
        "fn a() {}\n\nfn b() {}"
    );
    // 还没有结尾围栏：到末尾为止。
    assert_eq!(fenced_code("```\nlet x = 1;"), "let x = 1;");
    for empty in ["```", "```\n```", "```\n"] {
        assert_eq!(fenced_code(empty), "", "{empty:?}");
    }
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

#[test]
fn emoji_sequences_are_laid_out_as_one_piece() {
    let spans = |out: &Out| -> Vec<(String, Vec<f32>)> {
        out.items
            .iter()
            .filter_map(|item| match item {
                Item::Text { text, span, .. } => {
                    Some((text.to_string(), span.as_ref()?.advances.to_vec()))
                }
                _ => None,
            })
            .collect()
    };
    let family = "👨\u{200d}👩\u{200d}👧";
    for (markdown, unit) in [
        (format!("a {family} b"), family),
        ("a 👍\u{1f3fd} b".to_owned(), "👍\u{1f3fd}"),
        ("a ⚠\u{fe0f} b".to_owned(), "⚠\u{fe0f}"),
        (format!("```\na {family} b\n```"), family),
    ] {
        let laid = lay(&markdown, 600.0);
        let found = spans(&laid[0].0);
        let (text, advances) = found.iter().find(|(text, _)| text.contains(unit)).unwrap();
        assert_eq!(advances.len(), text.chars().count(), "{markdown:?}");
        // 序列的宽度记在第一个字符上，其余字符为 0；没有 emoji 字体时各字符仍有自己的宽度，
        // 不做比较。
        let at = text[..text.find(unit).unwrap()].chars().count();
        let members = &advances[at + 1..at + unit.chars().count()];
        if members.iter().all(|&w| w == 0.0) {
            assert!(advances[at] > 0.0, "{markdown:?}");
        }
    }
    // 很窄时在序列前后折行，序列中间不折：每一行的文字要么含整个序列，要么不含其中的字符。
    let laid = lay(&format!("word {family} word {family} word"), 40.0);
    for (_, _, _, text) in texts(&laid[0].0) {
        let inside = text.chars().filter(|c| family.contains(*c)).count();
        assert!(inside % family.chars().count() == 0, "{text:?}");
    }
}

/// 状态图中往返和并排的连线上的文字不重叠。
#[test]
fn edge_labels_do_not_overlap() {
    let source = "```mermaid\nstateDiagram-v2\n\
        [*] --> queued: 提交通过校验\n\
        [*] --> succeeded: 输入与通过验收的旧任务完全相同（复用结果）\n\
        queued --> running: worker 领取（没有租约）\n\
        running --> queued: 租约过期（worker 失联），重试次数未用完\n\
        queued --> canceled: 取消\n\
        running --> canceled: 心跳发现取消，终止 pi 容器\n\
        queued --> failed: 排队超过截止时间（expired）\n\
        running --> succeeded: 有结果（passed 或 degraded）\n\
        running --> failed: 拿不出结果（input_error / refused / system_error）\n\
        canceled --> [*]\nsucceeded --> [*]\nfailed --> [*]\n```\n";
    let laid = lay(source, 900.0);
    // 按字号估算文字宽度：汉字为一个字号，其余字符为半个字号。
    let boxes: Vec<(String, Rect)> = laid[0]
        .0
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text {
                x,
                baseline,
                text,
                size,
                ..
            } => {
                let w: f32 = text
                    .chars()
                    .map(|c| if c.is_ascii() { size * 0.5 } else { *size })
                    .sum();
                Some((
                    text.to_string(),
                    Rect::new(*x, baseline - size * 0.8, w, *size),
                ))
            }
            _ => None,
        })
        .collect();
    assert!(boxes.iter().any(|(t, _)| t.contains("领取")), "{boxes:?}");
    for (i, (a, ra)) in boxes.iter().enumerate() {
        for (b, rb) in &boxes[i + 1..] {
            let r = ra.inset(1.0, 1.0).intersect(rb.inset(1.0, 1.0));
            assert!(r.w <= 0.0 || r.h <= 0.0, "「{a}」和「{b}」重叠");
        }
    }
}

#[test]
fn list_markers_align_with_the_top_of_an_image_line() {
    let slot = |width| ImageSlot::Ready {
        width,
        height: 400,
        density: 1,
        picture: None,
        pixels: Vec::new(),
        drawing: Vec::new(),
    };
    let images = HashMap::from([
        (PathBuf::from("wide.png"), slot(2000)),
        (PathBuf::from("dot.png"), slot(400)),
    ]);
    let marker_and_image = |markdown: &str| {
        let laid = lay_with(markdown, 600.0, &images);
        let out = &laid[0].0;
        let image = out
            .items
            .iter()
            .find_map(|item| match item {
                Item::Image { rect, .. } => Some(*rect),
                _ => None,
            })
            .unwrap();
        let texts = texts(out);
        let marker = texts.iter().find(|t| t.3 == "1.").unwrap().0;
        let text = texts.iter().find(|t| t.3.contains("文字")).unwrap().0;
        (marker, image, text)
    };
    // 图片独占首行：记号的顶端与图片的顶端对齐。
    let (marker, image, text) = marker_and_image("1. ![a](wide.png) 文字\n");
    assert!(
        marker > image.y && marker < image.y + 16.0,
        "{marker} {image:?}"
    );
    assert!(text > image.y + image.h);
    // 图片与文字同行：记号与文字同一基线。
    let (marker, _, text) = marker_and_image("1. ![a](dot.png) 文字\n");
    assert_eq!(marker, text);
}

/// 回归：行内代码排在行尾、整段换到下一行时，开头的留白留在上一行，画成一块空的底色。
#[test]
fn inline_code_moves_to_the_next_line_with_its_padding() {
    let markdown =
        "修复分支：`fix/review-findings`（自 `perf/large-file-loading` 拉出，十个提交，未推送）";
    for width in (160..520).step_by(4) {
        let laid = lay(markdown, width as f32);
        let fills = laid[0].0.items.iter().filter_map(|item| match item {
            Item::Fill { rect, .. } => Some(*rect),
            _ => None,
        });
        for rect in fills {
            assert!(rect.w > 8.0, "宽 {width} 时有一块空的代码底色：{rect:?}");
        }
    }
}

#[test]
fn drift_moves_positions_after_the_first_edit() {
    // 在第 10 个字节处插入 3 个字节。
    let drift = Drift { from: 10, delta: 3 };
    assert_eq!(drift.moved(9), 9);
    assert_eq!(drift.moved(10), 13);
    assert_eq!(drift.moved_start(10), 10);
    assert_eq!(drift.moved_start(11), 14);
    // 删去第 10 至 15 个字节：其中的位置落在改动处。
    let drift = Drift {
        from: 10,
        delta: -5,
    };
    assert_eq!(drift.moved(12), 10);
    assert_eq!(drift.moved(20), 15);
    // 顶层块的起点在改动之后时整块移动，块内的相对位置不变。
    assert_eq!(drift.within(4), Drift { from: 6, delta: -5 });
    assert_eq!(drift.within(11), Drift::NONE);
    assert_eq!(Drift::NONE.moved(usize::MAX - 1), usize::MAX - 1);
}

#[test]
fn a_live_cell_hides_its_padding_and_shows_typed_spaces() {
    fn shown(segment: &str, caret: usize) -> Option<&str> {
        table::cell_content(segment, caret).map(|range| &segment[range])
    }
    // 光标在边距中或文字中：只显示文字。
    assert_eq!(shown(" 甲 ", 0), Some("甲"));
    assert_eq!(shown(" 甲 ", 1), Some("甲"));
    assert_eq!(shown(" 甲 ", 5), Some("甲"));
    // 在末尾输入的空格显示到光标为止。
    assert_eq!(shown(" 甲   ", 5), Some("甲 "));
    assert_eq!(shown(" 甲   ", 4), Some("甲"));
    // 在开头输入的空格在光标位于开头的空白中时显示。
    assert_eq!(shown("   甲 ", 3), Some("  甲"));
    assert_eq!(shown("   甲 ", 6), Some("甲"));
    // 没有边距、全为空白。
    assert_eq!(shown("甲", 0), Some("甲"));
    assert_eq!(shown("   ", 1), None);
}
