//! 空白符的测试：四个选项在编辑区的绘制结果、设置菜单与切换命令。

use winc_config::RenderWhitespace;

use super::*;
use crate::keymap::Action;

/// 用选项 `mode` 绘制一帧，返回全部像素。
pub(in crate::app::snapshot) fn pixels_with(app: &mut App, mode: RenderWhitespace) -> Vec<u32> {
    app.config.render_whitespace = Some(mode);
    paint(app);
    app.frame
        .renderer
        .as_ref()
        .unwrap()
        .image()
        .unwrap()
        .pixels
        .clone()
}

/// 返回两帧中不同的像素数。
pub(in crate::app::snapshot) fn changed(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).filter(|(a, b)| a != b).count()
}

#[test]
fn each_option_marks_its_own_whitespace() {
    let dir = sample_folder("whitespace-editor");
    let path = dir.join("spaces.txt");
    std::fs::write(&path, "a b\nc d  \n\te\n").unwrap();
    let mut app = app_with(&path, false);

    let none = pixels_with(&mut app, RenderWhitespace::None);
    // 没有选区时，「选中时」和「不显示」一样。
    assert_eq!(none, pixels_with(&mut app, RenderWhitespace::Selection));
    let trailing = pixels_with(&mut app, RenderWhitespace::Trailing);
    let all = pixels_with(&mut app, RenderWhitespace::All);
    assert!(changed(&none, &trailing) > 0);
    // 「全部」比「行尾」多出单词之间的空格和缩进的制表符。
    assert!(changed(&none, &all) > changed(&none, &trailing));

    app.run(winc_editor::Command::SelectAll);
    let selected_none = pixels_with(&mut app, RenderWhitespace::None);
    let selected = pixels_with(&mut app, RenderWhitespace::Selection);
    assert!(changed(&selected_none, &selected) > 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_setting_menu_and_the_command_change_the_option() {
    let dir = search_folder("whitespace-setting");
    let mut app = app_with(&dir, false);
    app.open_settings();
    app.settings_show_page(Setting::RenderWhitespace.page());
    assert_eq!(app.config.render_whitespace, None);

    app.settings_open_menu(Setting::RenderWhitespace);
    let commands: Vec<_> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        (0..4).map(MenuCommand::Whitespace).collect::<Vec<_>>()
    );
    app.run_menu_command(MenuCommand::Whitespace(2));
    assert_eq!(
        app.config.render_whitespace,
        Some(RenderWhitespace::Trailing)
    );

    // 命令在「全部」和「不显示」之间切换，其他选项先切到「全部」。
    app.run_action(Action::ToggleWhitespace);
    assert_eq!(app.config.render_whitespace, Some(RenderWhitespace::All));
    app.run_action(Action::ToggleWhitespace);
    assert_eq!(app.config.render_whitespace, Some(RenderWhitespace::None));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：编辑区显示全部空白符和选区内的空白符，深浅两套配色。
#[test]
#[ignore]
fn shoot_render_whitespace() {
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        let dir = search_folder(&format!("whitespace-shot-{theme}"));
        let path = dir.join("spaces.rs");
        let text = "fn main() {\n    let a = 1;  \n\tlet tabbed\t= 2;\n}\n";
        std::fs::write(&path, text).unwrap();
        let mut app = app_with(&path, dark);
        app.config.render_whitespace = Some(RenderWhitespace::All);
        shoot(&mut app, &format!("whitespace-all-{theme}"));
        app.config.render_whitespace = Some(RenderWhitespace::Selection);
        app.run(winc_editor::Command::SelectAll);
        shoot(&mut app, &format!("whitespace-selection-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
