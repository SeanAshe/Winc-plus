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
    assert_eq!(text_width(&app), WIDE_WIDTH.min(area.w - 64.0).floor());
    assert!(app.menu_checked(MenuCommand::TextWidth(1)));
    choose(&mut app, 2);
    assert_eq!(text_width(&app), (area.w - 64.0).floor());
    choose(&mut app, 0);
    assert_eq!(text_width(&app), MAX_WIDTH);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点标题列表右上角的图钉后，鼠标离开列表也一直展开，状态随窗口状态保存；再点一次取消。
/// 左侧留白放不下列表时（正文铺满），钉住的列表仍收成指示条。
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
    app.on_mouse_move(bars.x + 10.0, bars.y + 10.0);
    paint(&mut app);
    let pin = find(&app, TocTarget::Pin).unwrap();
    click(&mut app, (pin.x + pin.w / 2.0, pin.y + pin.h / 2.0));
    assert!(app.state.markdown_toc_pinned);

    // 鼠标移到正文上：列表仍然展开。
    let area = app.preview.markdown.area;
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    paint(&mut app);
    assert!(find(&app, TocTarget::Heading(0)).is_some());

    // 正文铺满，左侧放不下列表：收成指示条。
    app.set_text_width(winc_config::TextWidth::Full);
    settle(&mut app);
    assert!(find(&app, TocTarget::Heading(0)).is_none());
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
/// 没有；段落里连成一段的几行原文只写第一行。代码块里不再另编行号。
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
    assert_eq!(lines(), [0, 2, 4, 8, 9, 10, 13, 15]);
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
