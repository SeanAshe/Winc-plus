//! 导航与键位的测试：命令面板（⇧⌘P）、转到定义（F12）、查找引用（⇧F12）、转到符号、后退与前进、
//! 快捷键、编辑命令，以及粘滞滚动。

use super::*;
use crate::app::editor::definition::PlacesKind;

/// 创建一个小型 Python 项目并返回其目录。
///
/// `main.py` 使用 `helper`，而 `helper` 定义在 `lib/util.py` 中；`load` 在 `lib/util.py`
/// 和 `lib/other.py` 中都有定义。
pub(in crate::app::snapshot) fn navigation_folder(name: &str) -> PathBuf {
    let dir = winc_config::files::canonicalize(sample_folder(name)).unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("main.py"),
        "from lib.util import helper\n\ndef run():\n    total = 1\n    return helper(total)\n\nload()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/util.py"),
        "def helper(x):\n    return x\n\ndef load():\n    pass\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib/other.py"), "def load():\n    pass\n").unwrap();
    dir
}

/// 将光标放在当前文档中 `word` 第 `nth` 次出现（从 0 起）的位置，偏移为词首之后 1 个字符。
fn caret_on(app: &mut App, word: &str, nth: usize) {
    let text = app
        .editor
        .active()
        .unwrap()
        .text_range(0, usize::MAX)
        .unwrap();
    let at = text.match_indices(word).nth(nth).unwrap().0;
    let pos = text[..at].chars().count() + 1;
    app.select(pos, pos);
}

fn selected_text(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    let s = doc.selection;
    doc.text_range(s.from(), s.to()).unwrap()
}

fn active_name(app: &App) -> String {
    app.editor.active().unwrap().name().into_owned()
}

#[test]
fn go_to_definition_tries_this_file_then_the_project() {
    let dir = navigation_folder("definition");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));

    // 定义在当前文件中的局部变量：跳转到其上方的定义处。
    caret_on(&mut app, "total", 1);
    app.go_to_definition();
    assert_eq!(active_name(&app), "main.py");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.from()), (3, 4));
    assert_eq!(selected_text(&app), "total");

    // 定义在其他文件中且只有一处：直接打开该文件，并选中名字。
    caret_on(&mut app, "helper", 1);
    app.go_to_definition();
    assert_eq!(active_name(&app), "util.py");
    assert_eq!(selected_text(&app), "helper");
    assert_eq!(app.focus, Focus::Editor);

    // 光标已在定义处：改为列出用法（全字匹配，不含定义本身），可以筛选，回车跳转并选中名字。
    app.go_to_definition();
    assert!(app.picker_open);
    let places = app.picker_places.clone().unwrap();
    assert_eq!(places.kind, PlacesKind::Usages);
    let listed: Vec<(&str, usize)> = places
        .items
        .iter()
        .map(|p| (p.relative.as_str(), p.line))
        .collect();
    assert_eq!(listed, [("main.py", 0), ("main.py", 4)]);
    app.picker_input.set_text("return");
    app.picker_refresh();
    assert_eq!(app.picker_hits, [1]);
    app.picker_accept();
    assert_eq!(active_name(&app), "main.py");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.from()), (4, 11));
    assert_eq!(selected_text(&app), "helper");

    // 已定义但没有其他用法：在名字上方弹出提示气泡，不写入状态栏。
    caret_on(&mut app, "run", 0);
    app.go_to_definition();
    assert!(!app.picker_open);
    assert_eq!(
        app.name_hint.as_ref().map(|h| h.text.as_str()),
        Some("未找到「run」的其他用法")
    );

    // 有多处定义：列在卡片中，同一目录的排在前面；可以筛选，回车跳转。
    app.open_path(&dir.join("main.py"));
    caret_on(&mut app, "load", 0);
    app.go_to_definition();
    assert!(app.picker_open);
    let places = app.picker_places.clone().unwrap();
    let listed: Vec<&str> = places.items.iter().map(|p| p.relative.as_str()).collect();
    assert_eq!(listed, ["lib/other.py", "lib/util.py"]);
    app.picker_input.set_text("util");
    app.picker_refresh();
    assert_eq!(app.picker_hits, [1]);
    app.picker_accept();
    assert!(!app.picker_open && app.picker_places.is_none());
    assert_eq!(active_name(&app), "util.py");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.from()), (3, 4));

    // 两种失败情形：找不到定义；光标处不是名称。
    app.open_path(&dir.join("main.py"));
    caret_on(&mut app, "import", 0);
    app.go_to_definition();
    assert_eq!(
        app.name_hint.as_ref().map(|h| h.text.as_str()),
        Some("未找到「import」的定义")
    );
    app.select(1, 1);
    app.run(Command::Insert("  ".into()));
    caret_on(&mut app, "  ", 0);
    app.go_to_definition();
    assert_eq!(
        app.name_hint.as_ref().map(|h| h.text.as_str()),
        Some("光标处没有可查找的名称")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：光标在 `struct Item0` 上列出用法时，其他文件中各自定义的同名 `struct Item0` 被当成
/// 用法；注释、字符串和文档中的同名文字也被列出；局部变量在整个项目中查找。
#[test]
fn usages_skip_other_definitions_comments_and_unrelated_files() {
    let dir = winc_config::files::canonicalize(sample_folder("usages")).unwrap();
    std::fs::write(
        dir.join("large.rs"),
        "//! 自动生成的测试文件。\nuse std::collections::HashMap;\n\nstruct Item0 {\n    name: String,\n}\n\
         // Item0 is documented in README\nfn make() -> Item0 { let s = \"Item0\"; todo!() }\n\
         fn run() {\n    let value_13 = 1;\n    let y = value_13 + 1;\n}\n\
         fn second() {\n    let value_13 = 3;\n    let w = value_13;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("huge.rs"),
        "struct Item0 {\n}\nfn other() {\n    let value_13 = 2;\n    let z = value_13;\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("medium.rs"), "struct Item0 {\n}\n").unwrap();
    std::fs::write(dir.join("user.rs"), "fn use_it(x: Item0) {}\n").unwrap();
    std::fs::write(dir.join("README.md"), "Item0 is a struct.\n").unwrap();
    std::fs::write(
        dir.join("again.py"),
        "def f():\n    total = 1\n    total = 2\n    return total\n",
    )
    .unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("large.rs"));

    // 同名定义、注释、字符串和 Markdown 中的 `Item0` 都不是用法。
    caret_on(&mut app, "Item0", 0);
    app.go_to_definition();
    assert!(app.picker_open);
    let places = app.picker_places.clone().unwrap();
    assert_eq!(places.kind, PlacesKind::Usages);
    let listed: Vec<(&str, usize)> = places
        .items
        .iter()
        .map(|p| (p.relative.as_str(), p.line))
        .collect();
    assert_eq!(listed, [("large.rs", 7), ("user.rs", 0)]);
    app.close_picker();

    // 局部变量只在所在的函数中查找：唯一的用法直接跳转；其他文件或同一文件其他函数中的同名
    // 局部变量均不计入。
    caret_on(&mut app, "value_13", 0);
    app.go_to_definition();
    assert!(!app.picker_open);
    assert_eq!(active_name(&app), "large.rs");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.from()), (10, 12));

    // 重新赋值不算定义：跳到之前的定义。
    app.open_path(&dir.join("again.py"));
    caret_on(&mut app, "total", 1);
    app.go_to_definition();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.from()), (1, 4));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_references_is_a_whole_word_project_search() {
    let dir = navigation_folder("references");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));
    caret_on(&mut app, "helper", 1);

    app.find_references();
    assert_eq!(app.global.query.text(), "helper");
    assert!(app.global.whole_word && app.global.case_sensitive && !app.global.regex);
    assert_eq!(app.focus, Focus::Results);
    app.global.engine.wait();
    app.poll_global();
    let files: Vec<&str> = app
        .global
        .engine
        .results
        .iter()
        .map(|f| f.relative.as_str())
        .collect();
    assert_eq!(files, ["lib/util.py", "main.py"]);
    assert_eq!(app.global.engine.total(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_palette_runs_commands_and_remembers_them() {
    let dir = navigation_folder("palette");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));

    // ⇧⌘P：复用快速打开的卡片，输入框预填 `>`，列出的是命令。
    app.open_palette();
    assert!(app.picker_open && app.picker_lists_commands());
    assert_eq!(app.picker_input.text(), ">");
    // 输入的字符接在 `>` 之后。
    app.edit_field(|f| f.insert("撤"));
    assert_eq!(app.picker_input.text(), ">撤");
    assert_eq!(palette()[app.picker_hits[0]].title(), "撤销");
    app.picker_input.set_text(">");
    app.picker_refresh();
    // 模块关闭时（Git）其命令不列出；开发构建没有要安装的 `winc` 命令，同样不列出；Markdown 的
    // 行内格式只在可编辑的 Markdown 预览中列出。
    let installable = |a: &&crate::keymap::Action| {
        use crate::keymap::Action::{
            InstallShellCommand, MarkdownBold, MarkdownCode, MarkdownItalic, MarkdownStrikethrough,
            UninstallShellCommand,
        };
        !matches!(
            a,
            InstallShellCommand
                | UninstallShellCommand
                | MarkdownBold
                | MarkdownItalic
                | MarkdownCode
                | MarkdownStrikethrough
        )
    };
    let always = (palette().iter())
        .filter(|a| a.module().is_none())
        .filter(installable)
        .count();
    assert_eq!(app.picker_hits.len(), always);

    // 中文名和英文名都能搜索；回车执行选中的命令。
    app.picker_input.set_text(">toggle comment");
    app.picker_refresh();
    assert_eq!(palette()[app.picker_hits[0]].title(), "切换行注释");
    app.picker_accept();
    assert!(!app.picker_open);
    assert!(
        app.editor
            .active()
            .unwrap()
            .text_range(0, usize::MAX)
            .unwrap()
            .starts_with("# from lib")
    );

    // 再次打开：最近执行过的命令排在最前面。
    app.open_palette();
    assert_eq!(palette()[app.picker_hits[0]].title(), "切换行注释");
    assert_eq!(app.picker_recent, 1);

    // 删除 `>`：回到文件列表。
    app.picker_input.set_text("util");
    app.picker_refresh();
    assert!(!app.picker_lists_commands());
    let first = app.picker.file(app.picker_hits[0]).unwrap().path.to_owned();
    assert!(first.ends_with("util.py"), "{first}");
    app.close_picker();

    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回当前文档的名称和光标所在的行号（从 0 起）。
fn where_am_i(app: &App) -> (String, usize) {
    let doc = app.editor.active().unwrap();
    (
        active_name(app),
        doc.text().pos_to_line_col(doc.selection.head).0,
    )
}

#[test]
fn back_and_forward_retrace_jumps() {
    let dir = navigation_folder("back-forward");
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("long.txt"), long).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));
    app.track_navigation();

    // 转到定义跳到另一个文件，再跳到较远的第 150 行，之后在附近小幅移动到第 153 行
    // （此处行号从 1 起，与界面一致）。
    caret_on(&mut app, "helper", 1);
    app.track_navigation();
    app.go_to_definition();
    app.track_navigation();
    app.open_path(&dir.join("long.txt"));
    app.track_navigation();
    app.goto_line("150");
    app.track_navigation();
    app.goto_line("153");
    app.track_navigation();

    app.navigate(false);
    assert_eq!(where_am_i(&app), ("long.txt".to_owned(), 0));
    app.navigate(false);
    assert_eq!(where_am_i(&app), ("util.py".to_owned(), 0));
    app.navigate(false);
    // 回到跳离之前光标所在的位置。
    assert_eq!(where_am_i(&app), ("main.py".to_owned(), 4));
    assert_eq!(app.focus, Focus::Editor);
    // 已到历史起点：位置不变。
    app.navigate(false);
    assert_eq!(where_am_i(&app), ("main.py".to_owned(), 4));

    app.navigate(true);
    app.navigate(true);
    app.navigate(true);
    // 附近的小幅移动并入了跳转的那一条记录：前进回到的是最后停留的那一行。
    assert_eq!(where_am_i(&app), ("long.txt".to_owned(), 152));
    let doc = app.editor.active().unwrap();
    let visible = app.visible_lines() as f64;
    assert!((doc.scroll_line..doc.scroll_line + visible).contains(&152.0));

    // 标签已关闭时，后退会重新打开；文件已不存在的记录被跳过。
    app.navigate(false);
    app.navigate(false);
    assert_eq!(active_name(&app), "util.py");
    let util = app.editor.active.unwrap();
    app.close_tab(util);
    app.track_navigation();
    std::fs::remove_file(dir.join("long.txt")).unwrap();
    let long = app
        .editor
        .documents
        .iter()
        .position(|d| d.name() == "long.txt")
        .unwrap();
    app.close_tab(long);
    app.track_navigation();
    app.navigate(false);
    assert_eq!(active_name(&app), "util.py");
    app.navigate(true);
    assert_eq!(where_am_i(&app).0, "main.py");
    assert!(!app.navigation.can_go(true));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 在 `dir` 中创建一个很长的 Python 文件并返回其路径。
///
/// 开头是一个类，中间隔着 200 行赋值语句，最后是两个函数。
pub(in crate::app::snapshot) fn long_python(dir: &Path) -> PathBuf {
    let mut text = String::from("class Shop:\n    def total(self):\n        pass\n");
    for i in 0..200 {
        text.push_str(&format!("x{i} = {i}\n"));
    }
    text.push_str("def render():\n    pass\n\ndef main():\n    pass\n");
    let path = dir.join("shop.py");
    std::fs::write(&path, text).unwrap();
    path
}

fn symbol_names(app: &App) -> Vec<String> {
    let list = app.picker_symbols.as_ref().unwrap();
    app.picker_hits
        .iter()
        .map(|&i| list.items[i].name.clone())
        .collect()
}

#[test]
fn go_to_symbol_previews_filters_and_jumps() {
    let dir = navigation_folder("symbols");
    let path = long_python(&dir);
    // 没有打开目录时同样可用。
    let mut app = blank_app(false);
    app.editor.open(&path);
    app.relayout();
    caret_on(&mut app, "pass", 0);

    app.open_symbols();
    assert!(app.picker_open && app.picker_lists_symbols());
    assert_eq!(app.picker_input.text(), "@");
    let names = symbol_names(&app);
    assert_eq!(&names[..3], ["Shop", "total", "x0"]);
    assert_eq!(&names[names.len() - 2..], ["render", "main"]);
    // 查询为空时选中光标所在的符号。
    assert_eq!(names[app.picker_selected], "total");

    // 筛选：选中靠后的函数时，编辑区同步滚动到该函数，光标位置不变。
    app.picker_input.set_text("@ren");
    app.picker_refresh();
    assert_eq!(symbol_names(&app), ["render"]);
    let scrolled = app.editor.active().unwrap().scroll_line;
    assert!(scrolled > 100.0, "{scrolled}");
    assert_eq!(where_am_i(&app).1, 2);

    // Esc：滚动位置恢复为打开前的值，光标位置不变。
    app.close_picker();
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    assert_eq!(where_am_i(&app).1, 2);
    assert!(app.picker_symbols.is_none());

    // 在快速打开中手动输入 `@` 同样进入符号列表；回车跳转并选中名字。
    app.open_symbols();
    app.picker_input.set_text("@main");
    app.picker_refresh();
    app.picker_accept();
    assert!(!app.picker_open);
    assert_eq!(where_am_i(&app), ("shop.py".to_owned(), 206));
    assert_eq!(selected_text(&app), "main");
    let doc = app.editor.active().unwrap();
    assert!(doc.scroll_line > 100.0);

    // 删除 `@` 回到文件列表，滚动位置同样恢复。
    app.select(0, 0);
    app.editor.active_mut().unwrap().scroll_line = 0.0;
    app.open_symbols();
    app.picker_input.set_text("@ren");
    app.picker_refresh();
    app.picker_input.set_text("ren");
    app.picker_refresh();
    assert!(app.picker_symbols.is_none());
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 模拟一次按键，返回该按键是否被当作快捷键处理。
///
/// `mods` 为修饰键名称：`"cmd"`、`"ctrl"`、`"shift"`、`"alt"`，以及终端自己的命令的 `"term"`。
/// `"cmd"` 在 macOS 上是 `⌘`；`"term"` 在 macOS 上是 `⌘`，在其他系统上是 `Ctrl+Shift`。
///
/// # Panics
///
/// `mods` 中出现其他名称时 panic。
pub(in crate::app::snapshot) fn press(
    app: &mut App,
    mods: &[&str],
    key: crate::keymap::KeyCode,
) -> bool {
    use crate::keymap::{Mods, Os, Stroke};
    let mut m = Mods::NONE;
    for name in mods {
        m = m | match *name {
            "cmd" | "term" if Os::CURRENT == Os::Mac => Mods::META,
            "term" => Mods::CTRL | Mods::SHIFT,
            "cmd" | "ctrl" => Mods::CTRL,
            "shift" => Mods::SHIFT,
            "alt" => Mods::ALT,
            other => panic!("{other}"),
        };
    }
    app.on_shortcut(Stroke { key, mods: m })
}

#[test]
fn shortcuts_come_from_the_keymap() {
    use crate::keymap::KeyCode::{self, Char};

    let dir = navigation_folder("keymap");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));

    // 两段组合键：按下第一段后在状态栏提示等待第二段，`⌘K ⌘S` 打开设置中的「快捷键」页。
    assert!(press(&mut app, &["cmd"], Char('k')));
    assert!(app.pending_chord.is_some());
    assert!(
        app.status_message
            .as_deref()
            .unwrap()
            .contains("请按第二个键")
    );
    assert!(press(&mut app, &["cmd"], Char('s')));
    assert!(app.pending_chord.is_none());
    assert!(app.settings.open);
    assert_eq!(
        app.settings.selected.page(),
        crate::layout::SettingsPage::Keys
    );
    app.close_settings();
    app.focus = Focus::Editor;

    // 第二个键不匹配：只在状态栏提示，不执行其他操作；Esc 放弃组合键，且不给提示。
    press(&mut app, &["cmd"], Char('k'));
    press(&mut app, &[], Char('x'));
    assert!(
        app.status_message
            .as_deref()
            .unwrap()
            .ends_with("未分配任何命令")
    );
    press(&mut app, &["cmd"], Char('k'));
    press(&mut app, &[], KeyCode::Escape);
    assert_eq!(app.status_message, None);

    // 不是快捷键的按键不拦截：字符仍然输入编辑区。
    assert!(!press(&mut app, &[], Char('g')));

    // 编辑区命令：⇧⌘K 删除行。
    let lines = |app: &App| app.editor.active().unwrap().text().line_count();
    let before = lines(&app);
    assert!(press(&mut app, &["cmd", "shift"], Char('k')));
    assert_eq!(lines(&app), before - 1);

    // 文件树中：空格打开文件，焦点仍留在文件树。
    app.open_path(&dir.join("lib/util.py"));
    app.show_files();
    let main = row_of(&app, "main.py");
    app.editor.workspace.as_mut().unwrap().selected = Some(main);
    assert!(press(&mut app, &[], KeyCode::Space));
    assert_eq!(active_name(&app), "main.py");
    assert_eq!(app.focus, Focus::Tree);

    // 同一个键在编辑区和文件树中的含义不同（macOS 上 ⌘⌫：编辑区删除到行首，文件树删除文件）。
    if cfg!(target_os = "macos") {
        app.focus = Focus::Editor;
        let end = app.editor.active().unwrap().text().line_len(0);
        app.select(end, end);
        assert!(press(&mut app, &["cmd"], KeyCode::Backspace));
        assert_eq!(app.editor.active().unwrap().text().line_len(0), 0);
        // 文件树中 ↩ 触发重命名。
        app.focus = Focus::Tree;
        assert!(press(&mut app, &[], KeyCode::Enter));
        assert!(app.name_input.as_ref().is_some_and(|n| n.rename.is_some()));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn editing_commands_from_keys_and_the_palette() {
    use crate::keymap::KeyCode::{Char, Right};

    let dir = navigation_folder("edit-commands");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));
    let text = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .text_range(0, usize::MAX)
            .unwrap()
    };
    let selected = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text_range(doc.selection.from(), doc.selection.to())
            .unwrap()
    };

    // 光标在 `helper(total)` 的 `total` 上：扩大选区先得到该词；括号内的内容恰好是同一范围，
    // 因此下一层连同括号一起选中；再往外是整行。
    let at = text(&app).find("total)").unwrap();
    app.select(at + 1, at + 1);
    let grow = |app: &mut App| {
        if cfg!(target_os = "macos") {
            press(app, &["ctrl", "shift", "cmd"], Right)
        } else {
            press(app, &["shift", "alt"], Right)
        }
    };
    assert!(grow(&mut app));
    assert_eq!(selected(&app), "total");
    grow(&mut app);
    assert_eq!(selected(&app), "(total)");
    grow(&mut app);
    assert_eq!(selected(&app), "return helper(total)");
    let shrink = |app: &mut App| {
        if cfg!(target_os = "macos") {
            press(app, &["ctrl", "shift", "cmd"], crate::keymap::KeyCode::Left)
        } else {
            press(app, &["shift", "alt"], crate::keymap::KeyCode::Left)
        }
    };
    shrink(&mut app);
    shrink(&mut app);
    assert_eq!(selected(&app), "total");

    // 转换大小写只能通过命令面板触发，没有快捷键。
    app.run_action(crate::keymap::Action::TransformToUppercase);
    assert!(text(&app).contains("helper(TOTAL)"));
    // ⌘K ⌘X 删除行尾空白；一次撤销即可还原。
    app.run(Command::LineEnd { extend: false });
    app.run(Command::Insert("   ".into()));
    assert!(press(&mut app, &["cmd"], Char('k')));
    assert!(press(&mut app, &["cmd"], Char('x')));
    assert!(text(&app).contains("helper(TOTAL)\n"));
    app.run(Command::Undo);
    assert!(text(&app).contains("helper(TOTAL)   \n"));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 在 `dir` 中创建一个较长的 Python 文件并返回其路径。
///
/// 类中有两个方法，第一个方法有 60 行语句。
pub(super) fn sticky_file(dir: &Path) -> PathBuf {
    let mut text = String::from("import os\n\nclass Store:\n    def load(self):\n");
    for i in 0..60 {
        text.push_str(&format!("        x{i} = {i}\n"));
    }
    text.push_str("\n    def save(self):\n        pass\n");
    let path = dir.join("store.py");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn sticky_scroll_pins_the_enclosing_blocks() {
    let dir = navigation_folder("sticky-scroll");
    let mut app = app_with(&dir, false);
    app.open_path(&sticky_file(&dir));
    paint(&mut app);
    // 首行位于顶部：没有需要固定的行。
    assert!(app.sticky.is_empty());

    // `class` 所在行向上滚出一部分（0.25 到 0.75 行）：立即固定在顶部，而不是等整行滚出后再跳回；
    // `def` 行同时固定在第二个位置。
    let lh = app.fonts.line_height;
    for scroll in [2.25, 2.5, 2.75] {
        app.editor.active_mut().unwrap().scroll_line = scroll;
        paint(&mut app);
        let pinned: Vec<_> = app.sticky.iter().map(|r| (r.line, r.y)).collect();
        assert_eq!(pinned, [(2, 0.0), (3, lh)], "滚到 {scroll}");
    }
    // 整行对齐时 `class` 自己就位于顶部，无需固定。
    app.editor.active_mut().unwrap().scroll_line = 2.0;
    paint(&mut app);
    assert!(app.sticky.is_empty());

    // 滚动到方法中间：固定 `class` 和 `def` 两行，各占一个行高。
    app.editor.active_mut().unwrap().scroll_line = 20.0;
    paint(&mut app);
    let pinned: Vec<_> = app.sticky.iter().map(|r| (r.line, r.depth)).collect();
    assert_eq!(pinned, [(2, 0), (3, 1)]);
    assert_eq!(app.sticky[1].y, app.fonts.line_height);

    // 方法的最后一行滚到固定的 `def` 行下方时，`def` 上移半个行高，`class` 不动；
    // 滚过整个方法后只剩 `class`。
    app.editor.active_mut().unwrap().scroll_line = 62.5;
    paint(&mut app);
    assert_eq!(app.sticky[0].y, 0.0);
    assert_eq!(app.sticky[1].y, app.fonts.line_height / 2.0);
    app.editor.active_mut().unwrap().scroll_line = 64.0;
    paint(&mut app);
    let pinned: Vec<_> = app.sticky.iter().map(|r| r.line).collect();
    assert_eq!(pinned, [2]);

    // 悬停在固定的 `def` 上；点击后光标跳到该行，并滚动到该行恰好位于 `class` 下方。
    app.editor.active_mut().unwrap().scroll_line = 20.0;
    paint(&mut app);
    let t = app.layout.text;
    let (x, y) = (t.x + 60.0, t.y + app.fonts.line_height * 1.5);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover, Hover::Sticky(1));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().line_of(doc.selection.head), 3);
    assert_eq!(doc.scroll_line, 2.0);

    // 光标上移到被固定行覆盖的区域时，多滚动一行，避免光标藏在固定行下方。
    app.editor.active_mut().unwrap().scroll_line = 30.0;
    let pos = app.editor.active().unwrap().text().line_start(31);
    app.select(pos, pos);
    app.ensure_caret_visible();
    assert_eq!(app.editor.active().unwrap().scroll_line, 29.0);

    // 关闭「粘滞滚动」后不再固定任何行。
    app.run_action(crate::keymap::Action::ToggleStickyScroll);
    assert_eq!(app.config.sticky_scroll, Some(false));
    paint(&mut app);
    assert!(app.sticky.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sticky_scroll_ignores_lines_inside_strings() {
    let dir = navigation_folder("sticky-strings");
    let mut text = String::from("def query():\n    sql = \"\"\"\nSELECT *\nFROM t\n\"\"\"\n");
    for i in 0..60 {
        text.push_str(&format!("    x{i} = {i}\n"));
    }
    let path = dir.join("q.py");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    app.editor.active_mut().unwrap().scroll_line = 20.0;
    paint(&mut app);
    // 字符串内顶格的 SQL 不是代码块的开头：固定的是 `def`。
    let pinned: Vec<_> = app.sticky.iter().map(|r| r.line).collect();
    assert_eq!(pinned, [0]);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 以每次 1/8 行（`STEP`）的步长从头滚到尾，检查固定在顶部的行不发生跳变。
///
/// 对每一行计算露出的位置（像素，相对编辑区顶部）：被固定时取固定位置，否则取原本的位置；若被绘制在
/// 其上方的行覆盖（对被固定的行是外层固定行，否则是全部固定行），视作紧贴在这些行的下方。相邻两帧
/// 之间，该位置只能向上移动，且移动量不超过文字滚动的距离。`name` 用于在断言失败信息中标识文件。
fn assert_sticky_scrolls_smoothly(app: &mut App, name: &str) {
    const STEP: f64 = 0.125;
    let lh = app.fonts.line_height;
    let shown = |app: &App, line: usize, pinned: &[StickyRow]| {
        let bottom = |depth: usize| {
            pinned
                .iter()
                .filter(|r| r.depth < depth)
                .map(|r| r.y + lh)
                .fold(0.0, f32::max)
        };
        let (y, cover) = match pinned.iter().find(|r| r.line == line) {
            Some(r) => (r.y, bottom(r.depth)),
            None => {
                let doc = app.editor.active().unwrap();
                (
                    (doc.row_of(line) as f64 - doc.scroll_line) as f32 * lh,
                    bottom(usize::MAX),
                )
            }
        };
        y.max(cover - lh)
    };

    let max = app.max_scroll_line();
    let mut scroll = 0.0;
    app.editor.active_mut().unwrap().scroll_line = scroll;
    let mut before = app.sticky_rows();
    let mut pinned_frames = 0;
    while scroll + STEP <= max {
        let next = scroll + STEP;
        app.editor.active_mut().unwrap().scroll_line = next;
        let after = app.sticky_rows();
        pinned_frames += usize::from(!after.is_empty());

        let mut lines: Vec<usize> = before.iter().chain(&after).map(|r| r.line).collect();
        lines.sort_unstable();
        lines.dedup();
        for line in lines {
            app.editor.active_mut().unwrap().scroll_line = scroll;
            let y0 = shown(app, line, &before);
            app.editor.active_mut().unwrap().scroll_line = next;
            let y1 = shown(app, line, &after);
            assert!(
                y1 <= y0 + 0.01 && y0 - y1 <= STEP as f32 * lh + 0.01,
                "{name}：滚到 {next} 时第 {} 行从 {y0} 转到 {y1}（{before:?} → {after:?}）",
                line + 1
            );
        }
        before = after;
        scroll = next;
    }
    assert!(pinned_frames > 0, "{name}：一直没有钉住的行");
}

#[test]
fn sticky_scroll_never_jumps_while_scrolling() {
    let dir = navigation_folder("sticky-smooth");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // 取本仓库中的两个文件作样本：嵌套很深的 Rust 源文件，以及标题很多的 Markdown 样本
    // （scopes 的测试同样使用该文件）。
    std::fs::copy(
        repo.join("crates/winc-app/src/view/editor.rs"),
        dir.join("editor.rs"),
    )
    .unwrap();
    std::fs::copy(
        repo.join("crates/winc-editor/src/editing/scopes/sections.md"),
        dir.join("sections.md"),
    )
    .unwrap();
    let mut docker = String::from("FROM golang:1.22 AS build\nWORKDIR /src\n");
    for i in 0..12 {
        docker.push_str(&format!(
            "RUN --mount=type=cache,id=mod{i},target=/go/pkg/mod \\\n    go build ./cmd/{i}\n\nCOPY backend/{i} ./{i}\n"
        ));
    }
    std::fs::write(dir.join("Dockerfile"), docker).unwrap();
    let python = sticky_file(&dir);

    for file in [
        python.clone(),
        dir.join("Dockerfile"),
        dir.join("editor.rs"),
        dir.join("sections.md"),
    ] {
        let mut app = app_with(&dir, false);
        app.open_path(&file);
        assert_sticky_scrolls_smoothly(&mut app, &file.display().to_string());
    }

    // 存在折叠块时同样不发生跳变。
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("editor.rs"));
    app.run_action(crate::keymap::Action::FoldLevel3);
    assert!(!app.editor.active().unwrap().folds().is_empty());
    assert_sticky_scrolls_smoothly(&mut app, "editor.rs（折叠第三层）");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn places_popup_hangs_next_to_the_name() {
    let dir = navigation_folder("places-popup");
    // 在 `main.py` 末尾追加 200 个空行，再在最后放一处 `load()`：滚动到底后该调用位于编辑区最下方。
    let mut main = std::fs::read_to_string(dir.join("main.py")).unwrap();
    main.push_str(&"\n".repeat(200));
    main.push_str("load()\n");
    std::fs::write(dir.join("main.py"), main).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));
    paint(&mut app);

    // 名字位于上方：浮窗在该行下方，左边缘比名字靠左 12 逻辑像素。
    caret_on(&mut app, "load", 0);
    app.go_to_definition();
    paint(&mut app);
    assert!(app.picker_places.is_some());
    let line_h = app.fonts.line_height;
    let doc = app.editor.active().unwrap();
    let (line, _) = doc.text().pos_to_line_col(doc.selection.head);
    let top = app.layout.text.y + (line as f64 - doc.scroll_line) as f32 * line_h;
    let card = app.layout.picker;
    assert_eq!(card.y, (top + line_h + 4.0).round());
    let name_x = app.layout.text.x + TEXT_PAD - doc.scroll_x;
    assert_eq!(card.x, (name_x - 12.0).round());

    // 鼠标悬停在浮窗的第二行上：这一行有悬停效果；移到浮窗外面即消失，也不会穿透到下方。
    let row = crate::view::picker_parts(&app, card).row(1, app.picker_scroll);
    app.on_mouse_move(row.x + 100.0, row.y + row.h / 2.0);
    assert_eq!(app.hover, Hover::PickerRow(1));
    app.on_mouse_move(card.x + 100.0, card.y + 4.0);
    assert_eq!(app.hover, Hover::None);
    app.close_picker();

    // 名字位于编辑区底部：下方空间不足，浮窗改放在上方，底边与该行相距 4 逻辑像素。
    caret_on(&mut app, "load", 1);
    paint(&mut app);
    app.go_to_definition();
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let (line, _) = doc.text().pos_to_line_col(doc.selection.head);
    let top = app.layout.text.y + (line as f64 - doc.scroll_line) as f32 * line_h;
    assert!(
        top + line_h > app.layout.editor.bottom() - 200.0,
        "名字应在编辑区下部"
    );
    let card = app.layout.picker;
    assert_eq!(card.bottom(), top - 4.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn places_popup_scrolls_by_whole_rows() {
    let dir = navigation_folder("places-scroll");
    let uses: String = (0..30).map(|i| format!("v{i} = helper({i})\n")).collect();
    std::fs::write(
        dir.join("many.py"),
        format!("from lib.util import helper\n{uses}"),
    )
    .unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("lib/util.py"));
    app.select(5, 5);
    app.go_to_definition();
    paint(&mut app);
    let card = app.layout.picker;
    app.on_mouse_move(card.x + 100.0, card.y + card.h / 2.0);

    // 触控板的小幅滑动（每次 5 逻辑像素，即 2 倍屏上的 10 个物理像素）：累计不足一行时不滚动，
    // 累计满一行时恰好滚动一行。
    let wheel = |app: &mut App, dy: f64| {
        app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, dy)));
    };
    for _ in 0..4 {
        wheel(&mut app, -10.0);
    }
    assert_eq!(app.picker_scroll, 0.0);
    wheel(&mut app, -10.0);
    assert_eq!(app.picker_scroll, PICKER_ROW);

    // 滚动到底部：滚动量始终是整行，最后一行完整可见。
    for _ in 0..100 {
        wheel(&mut app, -37.0);
        assert_eq!(app.picker_scroll % PICKER_ROW, 0.0);
    }
    let span = crate::view::picker_span(&app);
    let rows = app.picker_hits.len() as f32;
    assert_eq!(app.picker_scroll + span, rows * PICKER_ROW);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hints_hang_on_the_name_and_go_away() {
    let dir = navigation_folder("name-hint");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("main.py"));

    // 没有其他用法：提示显示在名字旁边并对齐名字的开头，不写入状态栏。
    caret_on(&mut app, "run", 0);
    app.go_to_definition();
    let hint = app.name_hint.as_ref().unwrap();
    assert_eq!(hint.text, "未找到「run」的其他用法");
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.text().pos_to_line_col(hint.anchor),
        (2, 4),
        "挂在 `run` 的开头"
    );
    assert_eq!(app.status_message, None);
    paint(&mut app);

    // 点击、滚动时消失（测试中没有动画，直接清除）。
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.name_hint.is_none());
    caret_on(&mut app, "run", 0);
    app.go_to_definition();
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    assert!(app.name_hint.is_none());

    // 停留时间结束后自动消失；未到时间时返回下次检查的时刻。
    caret_on(&mut app, "run", 0);
    app.go_to_definition();
    let now = std::time::Instant::now();
    assert!(app.hint_tick(now).is_some());
    assert!(app.name_hint.is_some());
    app.hint_tick(now + std::time::Duration::from_secs(10));
    assert!(app.name_hint.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_tabs_closes_lists_that_belong_to_the_old_file() {
    let dir = navigation_folder("switch-closes");
    let path = long_python(&dir);
    let other = dir.join("other.py");
    std::fs::write(&other, "x = 1\n").unwrap();
    let mut app = blank_app(false);
    app.editor.open(&path);
    app.editor.open(&other);
    app.activate_tab(0);

    // 符号列表属于 `shop.py`：切到其他标签后关闭，回车不会在 `other.py` 中跳到 `shop.py` 的行号。
    app.open_symbols();
    assert!(app.picker_open);
    app.activate_tab(1);
    assert!(!app.picker_open);
    assert_eq!(app.focus, Focus::Editor);

    // 由面板自身打开文件的操作（⌘P 选中文件）仍然生效：先关闭面板，再切换标签。
    app.open_symbols();
    app.picker_accept();
    assert!(!app.picker_open);
}

#[test]
fn hover_is_checked_again_after_keyboard_changes() {
    let dir = sample_folder("hover-refresh");
    let mut app = app_with(&dir.join("a.txt"), false);
    app.cursor_inside = true;

    // 鼠标停在 ⌘P 的第一行上。
    app.open_picker();
    paint(&mut app);
    let list = view::picker_parts(&app, app.layout.picker).list;
    app.on_mouse_move(list.x + 40.0, list.y + 6.0);
    assert_eq!(app.hover, Hover::PickerRow(0));

    // 鼠标不动，按 Esc 关闭面板：重绘一帧后按鼠标当前位置重新判定悬停，下次打开面板时
    // 打开时不会立即高亮第一行。
    app.close_picker();
    app.relayout();
    app.dirty = false;
    let mut renderer = app.renderer.take().unwrap();
    renderer.draw(|frame| view::draw(&mut app, frame));
    app.renderer = Some(renderer);
    app.refresh_hover();
    assert_ne!(app.hover, Hover::PickerRow(0));
    assert_eq!(app.hover, app.hover_at(app.mouse.0, app.mouse.1));
    // 悬停项已改变，新的高亮需要在下一帧绘制：`redraw` 不能清除该重绘请求。
    assert!(app.dirty);

    std::fs::remove_dir_all(&dir).unwrap();
}
