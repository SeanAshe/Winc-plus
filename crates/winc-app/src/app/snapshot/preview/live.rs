//! Markdown 预览中实时编辑的测试：光标、按键和围栏代码块。

use super::*;

#[test]
fn split_preview_follows_edits_and_scrolls() {
    let (mut app, dir) = readme("md-edit", false, PreviewMode::Split);
    let before = app.preview.markdown.height;
    // 在文末追加两段：预览变高。
    app.run(winc_editor::Command::DocEnd { extend: false });
    app.run(winc_editor::Command::Insert(
        "\n\n多一段\n\n再多一段\n".into(),
    ));
    settle(&mut app);
    assert!(app.preview.markdown.height > before);

    // 滚轮滚动预览。
    let area = app.preview.markdown.area;
    app.pointer.mouse = (area.x + 50.0, area.y + 50.0);
    app.preview.markdown.scroll_to(0.0);
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -120.0,
    )));
    assert!(app.preview.markdown.scroll() > 0.0);
    // 不会滚动超过底部。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -100_000.0,
    )));
    paint(&mut app);
    let max = app.preview.markdown.height - area.h;
    assert!((app.preview.markdown.scroll() - max).abs() < 0.5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_full_preview_edits_the_document() {
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = readme("md-live", false, PreviewMode::Preview);
    // 刚打开时整篇按排版显示。
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(!shown.contains("**Rust**"));
    // 点击「启动快」：光标移到原文中对应的位置，被点击的文字位置不变。该段落仍按排版显示，
    // 只显示光标接触到的标记，即「启动快」两侧的 `*`；未接触的 `**Rust**` 不显示标记。
    let at = shown[..shown.find("启动快").unwrap()].chars().count();
    let (x, y) = point_of(&app, at);
    super::click(&mut app, (x, y));
    assert_eq!(after_caret(&app, 3), "启动快");
    let before = caret_screen_y(&app);
    paint(&mut app);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(
        shown.contains("一个用 Rust 写的") && shown.contains("*启动快*"),
        "{shown}"
    );
    assert!(!shown.contains("# winc"));
    assert!((caret_screen_y(&app) - before).abs() < 1.0);

    // 输入修改的是文档；重新解析之前，预览已显示刚输入的文字。
    app.run(winc_editor::Command::Insert("很".into()));
    paint(&mut app);
    assert!(
        crate::app::preview::markdown::laid::full_text(&app)
            .unwrap()
            .contains("很启动快")
    );
    settle(&mut app);
    assert!(
        app.editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .to_string()
            .contains("*很启动快*")
    );

    // ↓ 按显示行向下移动，移出该段后到达下一段；↑ 返回。
    let start = app.live_caret_source().unwrap();
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    paint(&mut app);
    let down = app.live_caret_source().unwrap();
    assert!(down > start);
    // 继续向下：经过提示块的标题（没有对应的原文，取块的开头），到达其正文的最后一行；保持
    // 原来的横坐标，落在该行末尾。一段折成几行取决于字体，因此逐行移动到该处，每一步都向前。
    let mut last = down;
    for _ in 0..8 {
        if after_caret(&app, 12).starts_with("\n\n## 安装") {
            break;
        }
        assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
        paint(&mut app);
        let now = app.live_caret_source().unwrap();
        assert!(now > last);
        last = now;
    }
    assert!(
        after_caret(&app, 12).starts_with("\n\n## 安装"),
        "{}",
        after_caret(&app, 12)
    );
    // 继续向下：越过块间留白到达下一个标题（保持横坐标，落在该行末尾），
    // 不因留白高于一行而停在原处或跳到文末。
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowDown), false));
    paint(&mut app);
    assert!(
        after_caret(&app, 5).starts_with("\n\n1. "),
        "{:?}",
        after_caret(&app, 8)
    );
    // ⇧↑ 扩展选区。
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&Key::Named(NamedKey::ArrowUp), false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    paint(&mut app);
    assert!(!app.editor.active().unwrap().selection.is_empty());
    // 其他键交给编辑区。
    assert!(!app.markdown_key(&Key::Character("x".into()), false));
    assert!(!app.markdown_key(&Key::Named(NamedKey::ArrowLeft), false));

    // 光标到达文末：预览滚动到光标可见。
    app.run(winc_editor::Command::DocEnd { extend: false });
    paint(&mut app);
    paint(&mut app);
    let y = caret_screen_y(&app);
    let area = app.preview.markdown.area;
    assert!(y >= area.y && y < area.bottom(), "{y} {area:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_paragraph_keeps_its_look() {
    use crate::app::preview::markdown::laid::full_text;
    use winit::keyboard::{Key, NamedKey};

    let (mut app, dir) = live_doc(
        "md-reveal",
        "第一段 **粗** 和 [链接](https://a.dev)。\n\n第二段\n",
    );
    let laid = full_text(&app).unwrap();
    // 光标未接触标记时保持排版后的外观，不显示原文。
    put_caret(&mut app, "第一段", true);
    assert_eq!(full_text(&app).unwrap(), laid);
    // 只显示光标所接触元素的标记。
    put_caret(&mut app, "粗", false);
    let shown = full_text(&app).unwrap();
    assert!(
        shown.contains("**粗**") && !shown.contains("](https"),
        "{shown}"
    );
    put_caret(&mut app, "链接", true);
    let shown = full_text(&app).unwrap();
    assert!(
        shown.contains("[链接](https://a.dev)") && !shown.contains("**"),
        "{shown}"
    );
    // 段落末尾回车：光标落在下方单独的一行上。
    put_caret(&mut app, "第二段", true);
    let line = caret_screen_y(&app);
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    settle(&mut app);
    assert!(caret_screen_y(&app) > line + 10.0);
    // 输入法组字排入光标所在的行，其后的文字向后移动，不被组字覆盖。
    put_caret(&mut app, "粗", true);
    app.ime.preedit = "中文".into();
    paint(&mut app);
    let shown = full_text(&app).unwrap();
    assert!(shown.contains("粗中文** 和"), "{shown}");
    // 焦点移到终端后组字文字属于终端：预览不再显示，也不绘制光标、不占用候选窗的位置。
    app.focus = Focus::Terminal;
    app.ime.cursor = None;
    paint(&mut app);
    let shown = full_text(&app).unwrap();
    assert!(!shown.contains("中文"), "{shown}");
    assert!(app.ime.cursor.is_none());
    app.focus = Focus::Editor;
    paint(&mut app);
    assert!(full_text(&app).unwrap().contains("粗中文** 和"));
    assert!(app.ime.cursor.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：输入之后、重新解析之前，光标所在块的范围来自上一版文档。文档变短或范围落在多字节
/// 字符中间时，绘制或按回车不应 panic（发布版中 panic 即进程中止）。
#[test]
fn edits_before_the_next_parse_keep_the_live_block_inside_the_text() {
    use winit::keyboard::{Key, NamedKey};

    let text =
        "第一段中文\n\n第二段中文内容\n\n| 表头 | 中 |\n| - | - |\n| 一 | 二 |\n\n最后一段\n";
    for keep in ["", "中", "中文\n\n表", "第一段中文\n\n第"] {
        let (mut app, dir) = live_doc("md-stale", text);
        // 按后台解析处理：输入之后的排版与按键仍依据上一版的解析结果。
        app.preview.markdown.cost = Some(std::time::Duration::MAX);
        let doc = app.editor.active_mut().unwrap();
        let count = doc.text().rope().unwrap().len_chars();
        doc.selection = winc_core::Selection::new(0, count);
        // 整篇替换为更短的文字，不等待重新解析：后台的解析结果仍属于上一版。
        app.run(winc_editor::Command::Insert(keep.into()));
        let count = app
            .editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .len_chars();
        for at in 0..=count {
            app.editor.active_mut().unwrap().selection = winc_core::Selection::point(at);
            let rope = app.editor.active().unwrap().text().rope().unwrap().clone();
            if let Some(live) = app.live_leaf() {
                let _ = rope.byte_slice(live.range.clone());
                let _ = rope.byte_slice(live.leaf.clone());
            }
            app.markdown_key(&Key::Named(NamedKey::Enter), false);
            paint(&mut app);
            app.run(winc_editor::Command::Undo);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn enter_after_a_fence_closes_the_block() {
    use winit::keyboard::{Key, NamedKey};
    let enter = Key::Named(NamedKey::Enter);

    let (mut app, dir) = live_doc("md-fence", "前文\n\n后文\n");
    put_caret(&mut app, "前文", true);
    for text in ["\n\n", "```rust"] {
        app.run(winc_editor::Command::Insert(text.into()));
    }
    // 尚未解析（仍为正文段落）时同样补全闭围栏，光标位于两行围栏之间；后文不会变为代码。
    assert!(app.markdown_key(&enter, false));
    assert_eq!(with_caret(&app), "前文\n\n```rust\n|\n```\n\n后文\n");
    settle(&mut app);
    app.run(winc_editor::Command::Insert("fn x() {}".into()));
    settle(&mut app);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(
        shown.contains("fn x() {}") && shown.contains("后文"),
        "{shown}"
    );
    // 在已闭合的围栏上按回车：按普通换行处理。
    put_caret(&mut app, "```rust", true);
    let _ = app.markdown_key(&enter, false);
    assert_eq!(doc_text(&app).matches("```").count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn backspace_after_a_marker_removes_it() {
    use winit::keyboard::{Key, NamedKey};
    let backspace = Key::Named(NamedKey::Backspace);

    let (mut app, dir) = live_doc(
        "md-backspace",
        "- [ ] 甲\n- 乙\n\n> 引\n\n```\n- x\n```\n\n## 题\n",
    );
    put_caret(&mut app, "甲", false);
    // 逐层移除任务框和列表记号；「乙」成为上一项中的另一段。
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).starts_with("- |甲\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    put_caret(&mut app, "乙", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).contains("\n  |乙\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    put_caret(&mut app, "引", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(with_caret(&app).contains("\n|引\n"), "{}", with_caret(&app));
    settle(&mut app);
    // 标题的 `#` 全部移除，成为段落。
    put_caret(&mut app, "题", false);
    assert!(app.markdown_key(&backspace, false));
    assert!(
        with_caret(&app).ends_with("\n|题\n"),
        "{}",
        with_caret(&app)
    );
    settle(&mut app);
    // 光标前还有文字或位于代码块中时，按普通删除处理，删除一个字符。
    put_caret(&mut app, "引", true);
    assert!(!app.markdown_key(&backspace, false));
    put_caret(&mut app, "x", false);
    assert!(!app.markdown_key(&backspace, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn enter_in_the_preview_continues_lists_and_splits_paragraphs() {
    use winit::keyboard::{Key, NamedKey};
    let enter = Key::Named(NamedKey::Enter);

    let (mut app, dir) = live_doc("md-enter", "- 一\n- 二\n\n段落一\n\n```\ncode\n```\n");
    put_caret(&mut app, "二", true);
    // 列表里回车续一项，空项再回车结束列表、另起一段。
    assert!(app.markdown_key(&enter, false));
    assert_eq!(
        with_caret(&app),
        "- 一\n- 二\n- |\n\n段落一\n\n```\ncode\n```\n"
    );
    settle(&mut app);
    assert!(app.markdown_key(&enter, false));
    app.run(winc_editor::Command::Insert("新".into()));
    assert_eq!(
        with_caret(&app),
        "- 一\n- 二\n\n新|\n\n段落一\n\n```\ncode\n```\n"
    );
    // 段落中回车另起一段，⇧ 回车段内换行。
    put_caret(&mut app, "段落", true);
    assert!(app.markdown_key(&enter, false));
    assert!(with_caret(&app).contains("段落\n\n|一"));
    settle(&mut app);
    app.pointer.modifiers = winit::keyboard::ModifiersState::SHIFT;
    assert!(app.markdown_key(&enter, false));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert!(
        with_caret(&app).contains("段落\n\n\n|一"),
        "{}",
        with_caret(&app)
    );
    // 代码块中交给编辑区按普通方式换行。
    put_caret(&mut app, "code", true);
    assert!(!app.markdown_key(&enter, false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn code_fences_show_only_with_the_caret_on_them() {
    let (mut app, dir) = live_doc("md-fences", "前\n\n```rust\nfn x() {}\n```\n");
    put_caret(&mut app, "fn x", false);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("fn x") && !shown.contains("```"), "{shown}");
    // 按 ← 移到围栏行：显示围栏，可以修改语言标识。
    app.run(winc_editor::Command::MoveLeft {
        extend: false,
        word: false,
    });
    settle(&mut app);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("```rust\nfn x"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edits_slide_the_blocks_below_and_fade_in_markers() {
    let half = crate::app::animation::MARKDOWN_MOTION / 2;
    let (mut app, dir) = live_doc(
        "md-motion",
        "正文 **粗** 字\n\n| a |\n|---|\n| b |\n| c |\n\n后文\n",
    );
    app.preview.markdown.frozen = Some(half);
    // 光标移入粗体（未修改文字）：两侧的 `**` 淡入，块高度不变，不滑动。
    put_caret(&mut app, "字", false);
    put_caret(&mut app, "粗", false);
    let motion = app.preview.markdown.motion.as_ref().expect("标记淡入");
    assert!(!motion.moving());
    assert_eq!(motion.fading.len(), 1);
    assert_eq!(motion.fading[0].1.len(), 2);
    // 输入文字时不淡入。
    app.run(winc_editor::Command::Insert("很".into()));
    settle(&mut app);
    assert!(
        app.preview
            .markdown
            .motion
            .as_ref()
            .is_none_or(|m| m.fading.is_empty())
    );
    // 删除表格的一行：表格变矮，后文从原位置向上滑动，停在过渡的中点。
    put_caret(&mut app, "c", false);
    assert!(app.live_table_tool("delete-row"));
    settle(&mut app);
    let motion = app.preview.markdown.motion.as_ref().expect("后文往上滑");
    let last = motion.from.len() - 1;
    assert!(motion.from[last] > 10.0, "{:?}", motion.from);
    let offset = motion.offset(last);
    assert!(offset > 0.0 && offset < motion.from[last], "{offset}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_live_preview_hides_its_caret_until_clicked_and_ignores_the_margins() {
    let (mut app, dir) = readme("md-live-caret", false, PreviewMode::Preview);
    paint(&mut app);
    // 刚打开、仅阅读时：不显示光标。
    assert!(!app.live_caret_shown());
    let before = app.editor.active().unwrap().selection;

    // 点在内容一栏左侧的空白：不放置光标、不开始选择。
    let area = app.preview.markdown.area;
    super::click(&mut app, (area.x + 2.0, area.y + area.h / 2.0));
    paint(&mut app);
    assert!(!app.live_caret_shown());
    assert_eq!(app.editor.active().unwrap().selection, before);

    // 点在文字上：显示光标。
    let at = point_of(&app, 2);
    super::click(&mut app, at);
    paint(&mut app);
    assert!(app.live_caret_shown());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：单独成行的公式中没有可定位的文字，点击后光标仍应进入公式的原文，公式随之显示原文、
/// 可以编辑（`$$…$$` 和语言为 math 的围栏代码块都是如此）。
#[test]
fn clicking_a_block_formula_puts_the_caret_in_its_source() {
    for (name, formula) in [
        ("md-math-dollars", "$$\nx^2 + y^2 = z^2\n$$"),
        ("md-math-fence", "```math\nx^2 + y^2 = z^2\n```"),
    ] {
        let text = format!("前文\n\n{formula}\n\n后文\n");
        let (mut app, dir) = live_doc(name, &text);
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        let at = |c: &str| shown[..shown.find(c).unwrap()].chars().count();
        let (above, below) = (point_of(&app, at("前文")), point_of(&app, at("后文")));
        // 公式在两段之间居中显示：从上往下找第一处落在公式上的点（正文的字不会落到公式的原文处）。
        let area = app.preview.markdown.area;
        let start = text.find(formula).unwrap();
        let x = area.x + area.w / 2.0;
        let middle = (above.1 as usize..below.1 as usize)
            .map(|y| (x, y as f32))
            .find(|&(x, y)| {
                crate::app::preview::markdown::laid::source_at(&app, x, y) == Some(start)
            })
            .expect("点得到公式");
        app.on_mouse_move(middle.0, middle.1);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        let caret = app.live_caret_source().unwrap();
        assert!(
            (start..start + formula.len()).contains(&caret),
            "{name}: {}",
            with_caret(&app)
        );
        // 光标在公式里：公式显示原文，可以直接编辑。
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        assert!(shown.contains("x^2 + y^2 = z^2"), "{name}: {shown}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 回归：行内公式（如希腊字母、运算符）中没有可定位的字，点击公式时光标仍应进入公式的原文。
#[test]
fn clicking_inline_math_puts_the_caret_in_its_source() {
    let text = "前文\n\n$\\alpha\\beta\\gamma \\nabla\\partial\\infty$\n\n后文\n";
    let (mut app, dir) = live_doc("md-inline-math", text);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let at = |c: &str| shown[..shown.find(c).unwrap()].chars().count();
    let (above, below) = (point_of(&app, at("前文")), point_of(&app, at("后文")));
    let tex = text.find("\\alpha").unwrap();
    let area = app.preview.markdown.area;
    // 从左到右、从上到下找第一处落在公式原文上的点（正文的字不会落到公式的原文处）。
    let spot = (above.1 as usize..below.1 as usize)
        .flat_map(|y| {
            (area.x as usize..area.right() as usize)
                .step_by(4)
                .map(move |x| (x, y))
        })
        .map(|(x, y)| (x as f32, y as f32))
        .find(|&(x, y)| crate::app::preview::markdown::laid::source_at(&app, x, y) == Some(tex))
        .expect("点得到公式");
    app.on_mouse_move(spot.0, spot.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let caret = app.live_caret_source().unwrap();
    let end = text.find("$\n\n后文").unwrap();
    assert!((tex..=end).contains(&caret), "{}", with_caret(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 回车续出的空项：光标停在新项的记号之后，不归到下一项；下一项照常按排版显示。
#[test]
fn enter_in_a_list_puts_the_caret_in_the_new_empty_item() {
    use winit::keyboard::{Key, NamedKey};
    let (mut app, dir) = live_doc("md-empty-item", "1. 一\n2. 二\n3. 三\n\n后文\n");
    put_caret(&mut app, "二", true);
    let before = caret_screen_y(&app);
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "1. 一\n2. 二\n3. |\n4. 三\n\n后文\n");
    assert!(caret_screen_y(&app) > before + 10.0);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("\n三\n") && !shown.contains("3."), "{shown}");
    // 在空项上输入的文字排在这一项里。
    app.run(winc_editor::Command::Insert("新".into()));
    settle(&mut app);
    let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(shown.contains("二\n新\n三"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 输入多字节的字之后、重新解析之前，光标所在项之后的项照常显示：页面高度不变，下方内容不
/// 上下跳动。
#[test]
fn typing_wide_characters_keeps_the_following_blocks_in_place() {
    let text = "前文\n\n1. 第一\n2. 第二\n3. 第三\n\n1. 第四\n\n后文\n";
    let (mut app, dir) = live_doc("md-typing-wide", text);
    // 按后台解析处理：输入之后的第一帧仍依据上一版的解析结果。
    app.preview.markdown.cost = Some(std::time::Duration::MAX);
    put_caret(&mut app, "第二", true);
    let height = app.preview.markdown.height;
    for c in ["a", "项", "，", "b"] {
        app.run(winc_editor::Command::Insert(c.into()));
        paint(&mut app);
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        assert!(
            shown.contains("第三") && shown.contains("第四"),
            "{c}：{shown}"
        );
        assert!((app.preview.markdown.height - height).abs() < 0.5, "{c}");
        settle(&mut app);
        assert!((app.preview.markdown.height - height).abs() < 0.5, "{c}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 删除键：空项上先移除记号，再删除留下的缩进并回到上一行末尾。
#[test]
fn backspace_on_an_empty_item_removes_it_in_two_steps() {
    use winit::keyboard::{Key, NamedKey};
    let back = Key::Named(NamedKey::Backspace);
    let (mut app, dir) = live_doc("md-back-item", "- 甲\n  - 乙\n  - \n\n后文\n");
    put_caret(&mut app, "  - \n", true);
    let doc = app.editor.active_mut().unwrap();
    doc.selection = winc_core::Selection::point(doc.selection.head - 1);
    settle(&mut app);
    assert!(app.markdown_key(&back, false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "- 甲\n  - 乙\n    |\n\n后文\n");
    assert!(app.markdown_key(&back, false));
    settle(&mut app);
    assert_eq!(with_caret(&app), "- 甲\n  - 乙|\n\n后文\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点击分隔线：光标移到它的原文处，分隔线显示原文；列表里的分隔线同样可以点击。
#[test]
fn clicking_a_rule_puts_the_caret_on_it() {
    for (text, expect) in [
        ("前文\n\n---\n\n后文\n", "前文\n\n|---\n\n后文\n"),
        (
            "前文\n\n1. 一\n2. ***\n\n后文\n",
            "前文\n\n1. 一\n2. |***\n\n后文\n",
        ),
    ] {
        let (mut app, dir) = live_doc("md-rule-click", text);
        put_caret(&mut app, "前文", true);
        let area = app.preview.markdown.area;
        let middle = |app: &App, s: &str| {
            let laid = app.preview.markdown.layout.as_ref().unwrap();
            let at = doc_text(app).find(s).unwrap();
            let (_, y, h) = laid.caret_point(at).unwrap();
            area.y + y - app.preview.markdown.scroll() + h / 2.0
        };
        // 分隔线在上一行文字与「后文」之间，取两者正中。
        let above = if text.contains("一") {
            "一"
        } else {
            "前文"
        };
        let y = (middle(&app, above) + middle(&app, "后文")) / 2.0;
        super::click(&mut app, (area.x + area.w / 2.0, y));
        settle(&mut app);
        assert_eq!(with_caret(&app), expect);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 输入之后的第一帧即按当前文字的结构排版：与等待后台解析之后的排版相同，没有按上一版结构
/// 推算的中间帧；输入引起的重排不滑动。
#[test]
fn typing_shows_the_final_layout_in_the_same_frame() {
    use winit::keyboard::{Key, NamedKey};
    let enter = Key::Named(NamedKey::Enter);
    let (mut app, dir) = live_doc("md-same-frame", "前文\n\n- 一\n- 二\n\n后文\n");
    app.preview.markdown.frozen = Some(crate::app::animation::MARKDOWN_MOTION / 2);
    let shown = |app: &App| {
        let text = crate::app::preview::markdown::laid::full_text(app).unwrap();
        (text, app.preview.markdown.height, caret_screen_y(app))
    };
    let step = |app: &mut App, name: &str, act: &dyn Fn(&mut App)| {
        act(app);
        paint(app);
        let first = shown(app);
        assert!(
            app.preview
                .markdown
                .motion
                .as_ref()
                .is_none_or(|m| !m.moving()),
            "{name}"
        );
        settle(app);
        assert_eq!(first, shown(app), "{name}");
        first
    };
    put_caret(&mut app, "前文", true);
    // 新的一段：空行的位置与输入第一个字之后段落的位置相同。
    let empty = step(&mut app, "回车", &|app| {
        assert!(app.markdown_key(&enter, false));
    });
    let typed = step(&mut app, "输入", &|app| {
        app.run(winc_editor::Command::Insert("新".into()));
    });
    assert!(
        (empty.1 - typed.1).abs() < 1.5 && (empty.2 - typed.2).abs() < 1.5,
        "{empty:?} {typed:?}"
    );
    // 列表里续一项、把一段文字改成标题：结构变化同样在第一帧显示。
    put_caret(&mut app, "二", true);
    let item = step(&mut app, "续项", &|app| {
        assert!(app.markdown_key(&enter, false));
    });
    put_caret(&mut app, "后文", false);
    let heading = step(&mut app, "标题", &|app| {
        app.run(winc_editor::Command::Insert("# ".into()));
    });
    // 标题的字号、上下间距大于段落：第一帧的总高度已经增加。
    assert!(heading.1 > item.1 + 10.0, "{heading:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 有序列表中间回车续出一项：其后各项的序号顺延，光标停在新的一项里，撤销一步恢复原文。
#[test]
fn enter_in_an_ordered_list_renumbers_the_items_below() {
    use winit::keyboard::{Key, NamedKey};
    let text = "1. 一\n2. 二\n   - 子项\n3. 三\n\n后文\n";
    let (mut app, dir) = live_doc("md-renumber", text);
    put_caret(&mut app, "一", true);
    assert!(app.markdown_key(&Key::Named(NamedKey::Enter), false));
    settle(&mut app);
    assert_eq!(
        with_caret(&app),
        "1. 一\n2. |\n3. 二\n   - 子项\n4. 三\n\n后文\n"
    );
    app.run(winc_editor::Command::Undo);
    assert_eq!(doc_text(&app), text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 行内格式的命令只在可编辑的预览中可用：选中的文字加上、去掉标记，没有选区时插入一对标记；
/// 代码块中不修改。
#[test]
fn format_commands_toggle_inline_marks_in_the_live_preview() {
    use crate::keymap::Action;
    let (mut app, dir) = live_doc("md-format", "正文 粗体 字\n\n```\ncode\n```\n");
    let select = |app: &mut App, needle: &str| {
        let at = doc_text(app).find(needle).unwrap();
        let doc = app.editor.active_mut().unwrap();
        let rope = doc.text().rope().unwrap();
        let (a, b) = (rope.byte_to_char(at), rope.byte_to_char(at + needle.len()));
        doc.selection = winc_core::Selection::new(a, b);
        settle(app);
    };
    select(&mut app, "粗体");
    assert!(app.run_action(Action::MarkdownBold));
    settle(&mut app);
    assert!(doc_text(&app).starts_with("正文 **粗体** 字\n"));
    let sel = app.editor.active().unwrap().selection;
    assert_eq!(sel.to() - sel.from(), 2);
    assert!(app.run_action(Action::MarkdownBold));
    settle(&mut app);
    assert!(doc_text(&app).starts_with("正文 粗体 字\n"));
    put_caret(&mut app, "字", true);
    assert!(app.run_action(Action::MarkdownCode));
    assert_eq!(with_caret(&app), "正文 粗体 字`|`\n\n```\ncode\n```\n");
    app.run(winc_editor::Command::Undo);
    put_caret(&mut app, "code", false);
    let before = doc_text(&app);
    assert!(app.run_action(Action::MarkdownItalic));
    assert_eq!(doc_text(&app), before);
    // 换成源代码后这些命令不可用，同一个键执行其他命令。
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert!(!app.action_available(Action::MarkdownBold));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 任务列表可以逐字输入：`- [` 之后括号配对补上 `]`，再输入空格即成为任务项；`[]`、`-[]`
/// 加空格是同样的简写。
#[test]
fn task_items_can_be_typed() {
    use winit::keyboard::Key;
    let space = Key::Character(" ".into());
    for typed in [&["-", " ", "["][..], &["[", "]"], &["-", "[", "]"]] {
        let (mut app, dir) = live_doc("md-typed-task", "前文\n\n\n\n后文\n");
        let doc = app.editor.active_mut().unwrap();
        doc.selection = winc_core::Selection::point(4);
        settle(&mut app);
        for key in typed {
            app.run(winc_editor::Command::Insert((*key).into()));
            paint(&mut app);
        }
        assert!(app.markdown_key(&space, false), "{typed:?}");
        paint(&mut app);
        assert_eq!(with_caret(&app), "前文\n\n- [ ] |\n\n后文\n", "{typed:?}");
        app.run(winc_editor::Command::Insert("任务".into()));
        settle(&mut app);
        let parsed = app.preview.markdown.parsed.clone().unwrap();
        let task = parsed.blocks.iter().find_map(|b| match b {
            winc_preview::markdown::Block::List(_, items) => items[0].task,
            _ => None,
        });
        assert_eq!(task, Some(false), "{typed:?}");
        let shown = crate::app::preview::markdown::laid::full_text(&app).unwrap();
        assert!(shown.contains("前文\n任务\n后文"), "{shown}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
