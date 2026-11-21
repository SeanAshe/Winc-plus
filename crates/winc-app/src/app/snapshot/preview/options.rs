//! Markdown 预览的显示选项：正文宽度、钉住标题列表、可编辑预览的原文行号和代码块的行号。

use winc_editor::PreviewMode;

use crate::app::input::context_menu::MenuCommand;
use crate::app::preview::markdown::TocTarget;
use crate::app::preview::markdown::laid::{MAX_WIDTH, WIDE_WIDTH, full_text, source_lines};
use crate::layout::Setting;

use super::*;

/// 三个标题、一段两行的段落和一个三行的代码块；原文第 0、2、4、5、7、11、13 行有文字。
const DOC: &str = "# 第一节\n\n正文第一行\n\n第二段第一行\n第二段第二行\n\n```rust\nfn a() {}\nlet b = 1;\nlet c = 2;\n```\n\n## 第二节\n\n## 第三节\n";

/// 打开写有 [`DOC`] 的 `NOTES.md`，以 `mode` 显示预览。
fn notes(name: &str, mode: PreviewMode) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("NOTES.md");
    std::fs::write(&path, DOC).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(mode);
    settle(&mut app);
    (app, dir)
}

/// 正文宽度：最宽的一块（标题下的横线与正文同宽）。
fn text_width(app: &App) -> f32 {
    app.preview.markdown.layout.as_ref().unwrap().key.width
}

/// 正文宽度默认为「标准」，不超过 [`MAX_WIDTH`]；在预览右键菜单的「正文宽度」中改为「较宽」
/// 时不超过 [`WIDE_WIDTH`]，「铺满」时铺满预览，选中的一项打勾，并写回配置。
#[test]
fn the_text_width_has_three_levels() {
    use winc_config::TextWidth;
    let (mut app, dir) = notes("md-width", PreviewMode::Preview);
    app.toggle_sidebar();
    settle(&mut app);
    assert_eq!(text_width(&app), MAX_WIDTH);
    assert!(app.menu_checked(MenuCommand::TextWidth(0)));

    let area = app.preview.markdown.area;
    let choose = |app: &mut App, i: usize| {
        right_click(app, (area.x + area.w / 2.0, area.y + 40.0));
        app.run_menu_command(MenuCommand::TextWidth(i));
        settle(app);
    };
    choose(&mut app, 1);
    assert_eq!(app.config.markdown_text_width, Some(TextWidth::Wide));
    // 文档有标题列表：正文左侧为标题指示条和原文行号留出位置，右侧留白仍为 32 逻辑像素。
    let room = |app: &App| {
        let left = app.preview.markdown.layout.as_ref().unwrap().left;
        (area.right() - 32.0 - left).floor()
    };
    assert_eq!(text_width(&app), WIDE_WIDTH.min(room(&app)));
    assert!(app.menu_checked(MenuCommand::TextWidth(1)));
    choose(&mut app, 2);
    assert_eq!(text_width(&app), room(&app));
    assert!(room(&app) < (area.w - 64.0).floor());
    choose(&mut app, 0);
    assert_eq!(text_width(&app), MAX_WIDTH);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点标题列表右上角的图钉后，鼠标离开列表也一直展开，状态随窗口状态保存；再点一次取消。
/// 左侧留白不足以容纳钉住的列表时，正文右移并变窄，让出一列；
/// 预览宽度不足以让出时（分栏），列表显示为指示条。
#[test]
fn the_heading_list_can_be_pinned_open() {
    let (mut app, dir) = notes("md-pin", PreviewMode::Preview);
    app.toggle_sidebar();
    settle(&mut app);
    let find = |app: &App, target| {
        (app.hits.markdown_toc.iter())
            .find(|(_, t)| *t == target)
            .map(|h| h.0)
    };
    let bars = find(&app, TocTarget::Bars).unwrap();
    rest_on_toc(&mut app, bars.x + 10.0, bars.y + 10.0);
    paint(&mut app);
    let pin = find(&app, TocTarget::Pin).unwrap();
    click(&mut app, (pin.x + pin.w / 2.0, pin.y + pin.h / 2.0));
    assert!(app.state.markdown_toc_pinned);

    // 鼠标移到正文上：列表仍然展开。
    let area = app.preview.markdown.area;
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    paint(&mut app);
    assert!(find(&app, TocTarget::Heading(0)).is_some());

    // 正文较宽或铺满时左侧留白不足：正文让出一列，列表保持展开，二者不重叠，正文右边缘不变。
    for width in [winc_config::TextWidth::Wide, winc_config::TextWidth::Full] {
        app.set_text_width(width);
        settle(&mut app);
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let (left, body) = (laid.left, laid.key.width);
        let row = find(&app, TocTarget::Heading(0)).expect("钉住的列表仍然展开");
        assert!(row.right() <= left, "{width:?}: {row:?} {left}");
        assert!(left - area.x >= 180.0 + 12.0, "{width:?}: {left}");
        assert!(left + body <= area.right() - 32.0 + 0.5, "{width:?}");
        // 取消钉住后正文恢复居中，宽于让出一列时。
        app.state.markdown_toc_pinned = false;
        settle(&mut app);
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        assert!(laid.left < left && laid.key.width > body, "{width:?}");
        app.state.markdown_toc_pinned = true;
        settle(&mut app);
    }
    // 分栏时让出一列后正文宽度不足：不让出，列表显示为指示条。
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert!(find(&app, TocTarget::Heading(0)).is_none());
    app.set_preview_mode(PreviewMode::Preview);
    app.set_text_width(winc_config::TextWidth::Standard);
    settle(&mut app);

    let pin = find(&app, TocTarget::Pin).unwrap();
    click(&mut app, (pin.x + pin.w / 2.0, pin.y + pin.h / 2.0));
    assert!(!app.state.markdown_toc_pinned);
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    paint(&mut app);
    assert!(find(&app, TocTarget::Heading(0)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 可编辑的预览按原文编行号：从原文一行的开头排起的显示行写该行的行号，空行和代码块的围栏
/// 没有。代码块里不再另编行号。
#[test]
fn the_editable_preview_numbers_source_lines() {
    let (app, dir) = notes("md-numbers", PreviewMode::Preview);
    assert!(app.markdown_live());
    let lines = || {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let rope = app.editor.active().unwrap().text().rope().unwrap();
        let found = source_lines(laid, rope, 0.0..laid.height);
        found.into_iter().map(|(line, _)| line).collect::<Vec<_>>()
    };
    assert_eq!(lines(), [0, 2, 4, 5, 8, 9, 10, 13, 15]);
    // 可编辑的预览在左侧编行号，代码块里不再另编。
    assert!(!app.preview.markdown.layout.as_ref().unwrap().key.numbers);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 只读的预览（分栏的右侧）中代码块从 1 起编行号；行号不是正文的一部分，全选复制时不带出。
/// 行号由单独的设置控制：编辑区的行号设置不影响预览；在预览的右键菜单中关闭后不再编号，菜单项
/// 随之取消打勾。
#[test]
fn code_blocks_are_numbered_in_the_read_only_preview() {
    let (mut app, dir) = notes("md-code-numbers", PreviewMode::Split);
    assert!(!app.markdown_live());
    let key = |app: &App| app.preview.markdown.layout.as_ref().unwrap().key.numbers;
    assert!(key(&app));
    let text = full_text(&app).unwrap();
    assert!(text.contains("fn a() {}\nlet b = 1;\nlet c = 2;"), "{text}");
    assert!(!text.contains("1fn") && !text.contains("2let"));

    app.settings_toggle(Setting::LineNumbers);
    settle(&mut app);
    assert!(key(&app));
    assert!(app.menu_checked(MenuCommand::MarkdownLineNumbers));
    let area = app.preview.markdown.area;
    right_click(&mut app, (area.x + area.w / 2.0, area.y + 40.0));
    assert!(menu_commands(&app).contains(&(MenuCommand::MarkdownLineNumbers, true)));
    app.run_menu_command(MenuCommand::MarkdownLineNumbers);
    settle(&mut app);
    assert_eq!(app.config.markdown_line_numbers, Some(false));
    assert!(!key(&app) && !app.menu_checked(MenuCommand::MarkdownLineNumbers));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：可编辑预览的原文行号与钉住的标题列表；分栏右侧代码块的行号；正文铺满。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_preview_options() {
    let (mut app, dir) = notes("md-options-shot", PreviewMode::Preview);
    app.toggle_sidebar();
    app.state.markdown_toc_pinned = true;
    settle(&mut app);
    shoot(&mut app, "md-options-live");
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    shoot(&mut app, "md-options-split");
    app.set_preview_mode(PreviewMode::Preview);
    app.set_text_width(winc_config::TextWidth::Full);
    settle(&mut app);
    shoot(&mut app, "md-options-full");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 能否预览取决于文档当前的语言：未保存的新文件和扩展名不符的文件在状态栏中选择语言后可以预览；
/// 预览中换成不可预览的语言时回到源代码。
#[test]
fn the_preview_follows_the_chosen_language() {
    use winc_editor::PreviewKind;
    let language = |id: &str| winc_syntax::find(id).unwrap();
    let dir = sample_folder("preview-by-language");
    let mut app = app_with(&dir.join("short.rs"), false);
    let kind = |app: &App| app.editor.active().unwrap().preview_kind();
    let mode = |app: &App| app.editor.active().unwrap().preview_mode();

    // 新建的文件没有路径，语言为纯文本，不能预览；选择 Markdown 后可以预览并完成排版。
    app.new_file();
    assert!(app.editor.active().unwrap().path.is_none());
    assert_eq!(kind(&app), None);
    app.use_language(language("markdown"));
    assert_eq!(kind(&app), Some(PreviewKind::Markdown));
    app.run(Command::Insert(
        "# 标题\n\n正文 ![图](a.png)\n\n## 小节\n".into(),
    ));
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    assert_eq!(mode(&app), PreviewMode::Preview);
    let text = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    assert!(text.contains("标题") && text.contains("小节"), "{text}");

    // 预览中换成不可预览的语言：回到源代码，文字正常显示。
    app.use_language(winc_syntax::detect("plain.unknown"));
    settle(&mut app);
    assert_eq!((kind(&app), mode(&app)), (None, PreviewMode::Source));
    assert!(app.editor.active().unwrap().shows_text());

    // 分栏时换成另一种可预览的语言：先回到源代码，之后按新语言预览。
    app.use_language(language("markdown"));
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    app.use_language(language("csv"));
    assert_eq!(
        (kind(&app), mode(&app)),
        (Some(PreviewKind::Table), PreviewMode::Source)
    );
    app.use_language(language("svg"));
    assert_eq!(kind(&app), Some(PreviewKind::Svg));

    // 已保存的文件同理：`.txt` 选择 Markdown 后可以预览，`.md` 默认可以预览。
    app.open_path(&dir.join("a.txt"));
    assert_eq!(kind(&app), None);
    app.use_language(language("markdown"));
    assert_eq!(kind(&app), Some(PreviewKind::Markdown));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 标题多到列表放不下时只列出一段：滚轮在列表上时逐行移动列表，不滚动正文；正文滚到另一节后
/// 列表回到当前所在的一节。
#[test]
fn a_heading_list_too_long_for_the_view_scrolls_by_the_wheel() {
    let dir = sample_folder("md-toc-scroll");
    let path = dir.join("book.md");
    let text: String = (0..120)
        .map(|i| format!("## Chapter {i}\n\nText of chapter {i}.\n\n"))
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    let find = |app: &App, target| {
        (app.hits.markdown_toc.iter())
            .find(|(_, t)| *t == target)
            .map(|h| h.0)
    };
    let listed = |app: &App| {
        let rows = app.hits.markdown_toc.iter().filter_map(|(_, t)| match t {
            TocTarget::Heading(i) => Some(*i),
            _ => None,
        });
        rows.collect::<Vec<_>>()
    };
    let bars = find(&app, TocTarget::Bars).unwrap();
    rest_on_toc(&mut app, bars.x + 10.0, bars.y + 10.0);
    paint(&mut app);
    let first = listed(&app);
    assert!(first.len() < 120 && first[0] == 0, "{first:?}");

    // 滚轮在列表上：列表下移三行，正文不动。
    let row = find(&app, TocTarget::Heading(2)).unwrap();
    app.on_mouse_move(row.x + 30.0, row.y + row.h / 2.0);
    // 滚轮位移是物理像素。
    let scale = app.frame.renderer.as_ref().unwrap().scale_factor();
    let wheel = |app: &mut App, dy: f64| {
        let dy = dy * f64::from(scale);
        app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, dy)));
        paint(app);
    };
    wheel(&mut app, -3.0 * f64::from(row.h));
    assert_eq!(listed(&app)[0], 3);
    assert_eq!(app.preview.markdown.scroll(), 0.0);
    shoot(&mut app, "md-toc-scroll");

    // 滚到尽头为止：最后一行是最后一个标题。
    wheel(&mut app, -1.0e5);
    assert_eq!(listed(&app).last(), Some(&119));
    wheel(&mut app, 1.0e5);
    assert_eq!(listed(&app), first);

    // 滚动条和其他区域的一样：鼠标在列表右边缘时显示，按住滑块拖到底列到最后一个标题，
    // 拖动期间列表不收起。
    let track = app.preview.markdown.toc_list.track;
    let x = track.right() - 4.0;
    app.on_mouse_move(x, track.y + track.h / 2.0);
    paint(&mut app);
    assert_eq!(
        app.scrollbar_shown(crate::app::ScrollArea::MarkdownToc),
        1.0
    );
    app.on_mouse_move(x, track.y + track.h * 0.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, track.bottom() + 40.0);
    paint(&mut app);
    assert_eq!(listed(&app).last(), Some(&119));
    app.on_mouse_move(x, track.bottom() - 4.0);
    app.on_mouse_up(MouseButton::Left);
    app.on_mouse_move(row.x + 30.0, row.y + row.h / 2.0);
    paint(&mut app);
    assert_eq!(listed(&app).last(), Some(&119));
    wheel(&mut app, 1.0e5);

    // 点列表里的标题跳到那一节，列表回到以它为中心的位置。
    wheel(&mut app, -40.0 * f64::from(row.h));
    let far = listed(&app)[5];
    let at = find(&app, TocTarget::Heading(far)).unwrap();
    click(&mut app, (at.x + 30.0, at.y + at.h / 2.0));
    settle(&mut app);
    assert!(app.preview.markdown.scroll() > 0.0);
    assert_eq!(app.preview.markdown.toc_list.shift, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 很长的文档改变宽度时分帧排版：这一帧只排看得见的块，其余的留到以后的帧里补排；补排期间
/// 预览顶边所在的块不动，排完后与一次排完的结果相同。
#[test]
fn a_long_document_is_laid_out_over_several_frames_after_its_width_changes() {
    let dir = sample_folder("md-catch-up");
    let path = dir.join("long.md");
    let text: String = (0..400)
        .map(|i| {
            format!(
                "Paragraph {i} {}\n\n",
                "with enough words to wrap. ".repeat(6)
            )
        })
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    app.set_text_width(winc_config::TextWidth::Full);
    settle(&mut app);
    let laid = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        (
            laid.key.width,
            laid.height,
            laid.stale.iter().filter(|s| **s).count(),
        )
    };
    let (wide, wide_height, waiting) = laid(&app);
    assert_eq!(waiting, 0);

    // 滚到中间，记下顶边所在的块。
    app.preview.markdown.scroll_to(wide_height / 2.0);
    paint(&mut app);
    let top = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        laid.place_at(app.preview.markdown.scroll() - laid.pad)
            .unwrap()
    };
    let (held, _) = top(&app);

    // 不给补排的时间：收起侧栏使正文变宽，这一帧只排了看得见的块。
    app.preview.markdown.catch_up.budget = Some(std::time::Duration::ZERO);
    app.toggle_sidebar();
    paint(&mut app);
    let (width, _, waiting) = laid(&app);
    assert!(width > wide && waiting > 300, "{width} {waiting}");
    assert!(app.preview.markdown.catch_up.pending && app.animating());
    assert_eq!(top(&app).0, held);
    let area = app.preview.markdown.area;
    let shown = {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let y = app.preview.markdown.scroll() - laid.pad;
        laid.blocks_in(y, y + area.h)
    };
    let laid_now = app.preview.markdown.layout.as_ref().unwrap();
    assert!(shown.clone().all(|i| !laid_now.stale[i]), "{shown:?}");

    // 给足时间：下一帧排完，顶边所在的块仍是那一块。
    app.preview.markdown.catch_up.budget = None;
    paint(&mut app);
    let (_, height, waiting) = laid(&app);
    assert_eq!(waiting, 0);
    assert!(!app.preview.markdown.catch_up.pending);
    assert_eq!(top(&app).0, held);

    // 与一次排完的结果相同：换回原来的宽度再换回来，高度一致。
    app.toggle_sidebar();
    paint(&mut app);
    assert_eq!(laid(&app), (wide, wide_height, 0));
    app.toggle_sidebar();
    paint(&mut app);
    assert_eq!(laid(&app), (width, height, 0));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 很长的文档只保留视口附近的块的排版结果：其余的块释放要画的项，滚动、查找、移动光标到
/// 那里时重新排版，文档的总高度不变。
#[test]
fn a_long_document_keeps_layout_only_near_the_viewport() {
    let dir = sample_folder("md-shed");
    let path = dir.join("long.md");
    let text: String = (0..1500)
        .map(|i| {
            format!(
                "Paragraph {i} [link](#p{i}) {}\n\n",
                "with some more words. ".repeat(5)
            )
        })
        .collect();
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    settle(&mut app);
    // （释放了要画的项的块数，视口内是否都排好了，总高度）
    let state = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        let y = app.preview.markdown.scroll() - laid.pad;
        let seen = laid.blocks_in(y, y + app.preview.markdown.area.h);
        let shed = laid.blocks.iter().filter(|b| b.1.is_shed()).count();
        let drawn = seen.clone().all(|i| !laid.blocks[i].1.is_shed());
        (shed, drawn, laid.height, seen)
    };
    let (shed, drawn, height, seen) = state(&app);
    assert!(shed > 1000 && drawn && seen.start == 0, "{shed} {seen:?}");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert!(laid.blocks[1400].1.is_shed() && laid.blocks[1400].1.links_to("#p1400"));
    assert!(app.preview.markdown.memo.len() < 600);

    // 滚到中间：视口内的块重新排好，开头的块释放，总高度不变。
    app.preview.markdown.scroll_to(height / 2.0);
    paint(&mut app);
    let (shed, drawn, now, seen) = state(&app);
    assert!(shed > 1000 && drawn && seen.start > 500, "{shed} {seen:?}");
    assert_eq!(now, height);
    assert!(
        app.preview.markdown.layout.as_ref().unwrap().blocks[0]
            .1
            .is_shed()
    );

    // 查找跳到靠后的一处：选中的文字所在的块排好，滚动到它附近。
    let full = crate::app::preview::markdown::laid::full_text(&app).unwrap();
    let at = full
        .find("Paragraph 1450 ")
        .map(|byte| full[..byte].chars().count())
        .unwrap();
    app.select_in_markdown(at, at + 14);
    settle(&mut app);
    let (_, drawn, now, seen) = state(&app);
    assert!(drawn && seen.contains(&1450), "{seen:?}");
    assert_eq!(now, height);
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert!(!laid.blocks[1450].1.is_shed());

    // 改变宽度后分帧重排，排完后离视口远的块仍然释放。
    app.set_text_width(winc_config::TextWidth::Full);
    settle(&mut app);
    let (shed, drawn, ..) = state(&app);
    assert!(shed > 1000 && drawn, "{shed}");
    assert!(!app.preview.markdown.catch_up.pending);

    // 可编辑的预览里光标跳到文末：光标所在的块排好，预览滚动到那里；回到开头同样如此。
    assert!(app.markdown_live());
    app.focus = Focus::Editor;
    app.run(winc_editor::Command::DocEnd { extend: false });
    settle(&mut app);
    let (_, drawn, _, seen) = state(&app);
    assert!(drawn && seen.end >= 1499, "{seen:?}");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert!(!laid.blocks[1499].1.is_shed() && laid.blocks[0].1.is_shed());
    app.run(winc_editor::Command::DocStart { extend: false });
    settle(&mut app);
    let (shed, drawn, _, seen) = state(&app);
    assert!(shed > 1000 && drawn && seen.start == 0, "{shed} {seen:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 启动时恢复的标签和命令行打开的文件按记下的显示方式打开。
#[test]
fn files_opened_at_launch_use_the_remembered_mode() {
    let dir = sample_folder("preview-at-launch");
    std::fs::write(dir.join("a.md"), "# 标题\n").unwrap();
    let mut state = winc_config::State::default();
    state
        .preview_modes
        .insert("markdown".into(), "preview".into());
    let mut editor = winc_editor::Editor::default();
    editor.preview_defaults = crate::app::preview::images::preview_defaults(&state);
    let index = editor.open(&dir.join("a.md"));
    assert_eq!(editor.documents[index].preview_mode(), PreviewMode::Preview);
    let index = editor.open(&dir.join("short.rs"));
    assert_eq!(editor.documents[index].preview_mode(), PreviewMode::Source);
    let _ = std::fs::remove_dir_all(&dir);
}
