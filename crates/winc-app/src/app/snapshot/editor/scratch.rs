//! 临时文件的测试：启动时新建文件、新建文件的默认语言、粘贴时识别语言、最近选择的语言。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};

/// 当前文档的语言标识。
fn language(app: &App) -> &'static str {
    app.editor.active().unwrap().highlighter.language().id
}

/// 新建一个未命名文件并粘贴 `text`，返回粘贴后的语言。
fn pasted(app: &mut App, text: &str) -> &'static str {
    app.new_file();
    app.set_clipboard(text.to_owned(), false);
    app.paste();
    language(app)
}

#[test]
fn pasting_into_a_blank_untitled_file_detects_the_language() {
    let mut app = blank_app(false);
    assert_eq!(pasted(&mut app, "{\"status\":\"ok\",\"count\":2}"), "json");
    assert_eq!(pasted(&mut app, "<?xml version=\"1.0\"?>\n<a/>\n"), "xml");
    assert_eq!(pasted(&mut app, "<!DOCTYPE html>\n<html></html>"), "html");
    assert_eq!(pasted(&mut app, "#!/bin/sh\necho hi\n"), "shell");
    // 没有把握的内容保持纯文本：以方括号开头和结尾但不是 JSON 的文字。
    assert_eq!(pasted(&mut app, "[1] 见注释 [2]"), "text");
    assert_eq!(pasted(&mut app, "普通的一句话"), "text");

    // 已有内容、已选了语言的文档不识别。
    assert_eq!(pasted(&mut app, "{\"a\":1}"), "json");
    app.set_clipboard("<?xml version=\"1.0\"?>".to_owned(), false);
    app.paste();
    assert_eq!(language(&app), "json");
    app.new_file();
    app.use_language(winc_syntax::detect("a.md"));
    app.set_clipboard("{\"a\":1}".to_owned(), false);
    app.paste();
    assert_eq!(language(&app), "markdown");

    // 关闭设置后不识别。
    app.config.detect_language_on_paste = Some(false);
    assert_eq!(pasted(&mut app, "{\"a\":1}"), "text");
}

#[test]
fn new_files_use_the_default_language() {
    let mut app = blank_app(false);
    app.new_file();
    assert_eq!(language(&app), "text");
    app.config.default_language = Some("markdown".to_owned());
    app.new_file();
    assert_eq!(language(&app), "markdown");
    // 设置的默认语言优先于粘贴时的识别。
    assert_eq!(pasted(&mut app, "{\"a\":1}"), "markdown");
    // 没有这种语言时为纯文本。
    app.config.default_language = Some("no-such-language".to_owned());
    app.new_file();
    assert_eq!(language(&app), "text");
}

#[test]
fn startup_opens_a_new_file_only_when_nothing_is_restored() {
    let mut app = blank_app(false);
    app.start_with_new_file();
    assert_eq!(app.editor.tab_count(), 0);
    app.config.new_file_on_startup = Some(true);
    app.start_with_new_file();
    assert_eq!(app.editor.tab_count(), 1);
    assert!(!app.editor.active().unwrap().is_dirty());
    assert!(app.is_blank());
    // 已有标签或打开了目录时不新建。
    app.start_with_new_file();
    assert_eq!(app.editor.tab_count(), 1);
    let dir = sample_folder("scratch-startup");
    let mut app = app_with(&dir, false);
    app.config.new_file_on_startup = Some(true);
    app.start_with_new_file();
    assert_eq!(app.editor.tab_count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_first_opened_file_or_folder_replaces_the_startup_file() {
    let dir = sample_folder("scratch-replace");
    let started = || {
        let mut app = blank_app(false);
        app.config.new_file_on_startup = Some(true);
        app.start_with_new_file();
        app
    };
    let names = |app: &App| -> Vec<String> {
        let docs = app.editor.documents.iter();
        docs.map(|doc| doc.name().into_owned()).collect()
    };
    let mut app = started();
    app.open_path(&dir.join("a.txt"));
    assert_eq!(names(&app), ["a.txt"]);
    let mut app = started();
    app.open_folder_now(&dir);
    assert!(!names(&app).contains(&"untitled".to_owned()));

    // 修改过的不替换；用户自己新建的文件也不替换。
    let mut app = started();
    app.set_clipboard("x".to_owned(), false);
    app.paste();
    assert!(!app.is_blank());
    app.open_path(&dir.join("a.txt"));
    assert_eq!(names(&app), ["untitled", "a.txt"]);
    let mut app = blank_app(false);
    app.new_file();
    app.open_path(&dir.join("a.txt"));
    assert_eq!(names(&app), ["untitled", "a.txt"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recently_chosen_languages_lead_the_language_menu() {
    let mut app = blank_app(false);
    app.new_file();
    let id = app.editor.active().unwrap().id;
    let index = |name: &str| {
        let wanted = winc_syntax::detect(name).id;
        let all = winc_syntax::languages();
        all.iter().position(|l| l.id == wanted).unwrap()
    };
    let leading = |app: &mut App| {
        app.open_context_menu(MenuTarget::Language(id), (0.0, 0.0));
        let commands = menu_commands(app);
        app.overlays.context_menu = None;
        commands
            .into_iter()
            .map(|(c, _)| c)
            .take(3)
            .collect::<Vec<_>>()
    };
    assert_eq!(leading(&mut app)[0], MenuCommand::UsePlainText);
    // 选过的语言排在纯文本之后，最近的在前，后面不再重复；菜单和面板中选择的都记下。
    app.run_status_command(MenuCommand::UseLanguage(index("a.json")), id);
    app.run_action(crate::keymap::Action::ChangeLanguageMode);
    app.overlays.picker.input.set_text("markdown");
    app.picker_refresh();
    app.language_picker_accept();
    let recent = [
        MenuCommand::UsePlainText,
        MenuCommand::UseLanguage(index("a.md")),
        MenuCommand::UseLanguage(index("a.json")),
    ];
    assert_eq!(leading(&mut app), recent);
    assert_eq!(app.state.recent_languages, ["markdown", "json"]);
    app.open_context_menu(MenuTarget::Language(id), (0.0, 0.0));
    let listed = menu_commands(&app);
    let markdown = MenuCommand::UseLanguage(index("a.md"));
    assert_eq!(listed.iter().filter(|(c, _)| *c == markdown).count(), 1);
}

/// 未命名文件另存时默认的名字带所用语言的扩展名，保存后语言不变；已有名字的沿用。
#[test]
fn saving_an_untitled_file_suggests_the_extension_of_its_language() {
    let dir = sample_folder("scratch-save-name");
    let mut app = blank_app(false);
    app.new_file();
    let id = app.editor.active().unwrap().id;
    assert_eq!(app.save_name(id), "untitled.txt");
    app.use_language(winc_syntax::detect("a.md"));
    assert_eq!(app.save_name(id), "untitled.md");
    app.use_language(winc_syntax::detect("Makefile"));
    assert_eq!(app.save_name(id), "untitled.mk");
    assert_eq!(pasted(&mut app, "{\"a\":1}"), "json");
    let id = app.editor.active().unwrap().id;
    let name = app.save_name(id);
    assert_eq!(name, "untitled.json");
    app.on_picked(Pick::SaveAs(id), Some(dir.join(&name)));
    assert_eq!(language(&app), "json");
    // 已有名字的文件沿用原名，不随所选的语言改。
    app.use_language(winc_syntax::detect("a.md"));
    assert_eq!(app.save_name(id), "untitled.json");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 语言菜单和语言面板的每一行开头是这种语言的文件的图标。
#[test]
fn languages_are_listed_with_the_icon_of_their_files() {
    use crate::app::input::context_menu::MenuIcon;
    use crate::icons::FileGlyph;
    let app = blank_app(false);
    let glyph = |name: &str| {
        let wanted = winc_syntax::detect(name).id;
        let all = winc_syntax::languages();
        let at = all.iter().position(|l| l.id == wanted).unwrap();
        match app.menu_icon_kind(MenuCommand::UseLanguage(at)) {
            Some(MenuIcon::File(glyph, _)) => glyph,
            other => panic!("{name}: {other:?}"),
        }
    };
    assert_eq!(glyph("a.json"), FileGlyph::Json);
    assert_eq!(glyph("a.md"), FileGlyph::Markdown);
    assert_eq!(glyph("a.sh"), FileGlyph::Shell);
    assert_eq!(glyph("a.rs"), FileGlyph::Letters("RS"));
    assert_eq!(glyph("Dockerfile"), FileGlyph::Docker);
    assert!(app.menu_icon_kind(MenuCommand::UsePlainText).is_some());
    assert!(app.menu_icon_kind(MenuCommand::DetectLanguage).is_none());
    // 系统菜单按图标自己的尺寸显示，不按目录图标的。
    assert_eq!(app.menu_icon_size(MenuCommand::UsePlainText), (16.0, 16.0));
    assert_eq!(
        app.menu_icon_size(MenuCommand::DetectLanguage),
        (15.0, 12.0)
    );
    // 面板中的行按同一种语言取图标，「自动检测」没有。
    use crate::app::LanguageRow;
    assert_eq!(LanguageRow::Plain.language().map(|l| l.id), Some("text"));
    assert!(LanguageRow::Detect.language().is_none());
}
