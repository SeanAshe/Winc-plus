//! Vim 模式的界面测试：开关、模式与光标形状、状态栏、各模式下是否接受文字输入、撤销分组，
//! 以及与鼠标、应用命令、标签切换的配合。

use winc_core::Selection;
use winc_editor::Command;
use winc_vim::{Code, Mode, Named, notation};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::whitespace::changed;
use super::*;
use crate::app::vim::CaretShape;
use crate::app::{Focus, PromptKind};
use crate::keymap::Action;

mod commands;
mod diff;
mod state;

/// 创建打开了内容为 `text` 的文件、并已开启 Vim 模式的界面；返回界面与样本目录。
fn vim_app(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("vim.txt");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    // 直接改配置：经设置开关会开始一段动画，之后各帧的像素随时间变化。
    app.config.vim_enabled = Some(true);
    app.vim_apply_config();
    paint(&mut app);
    (app, dir)
}

/// 依次输入按键记法 `keys`；Vim 层不处理的按键按编辑区原有的方式输入文字。
fn keys(app: &mut App, keys: &str) {
    for key in notation::parse(keys) {
        if app.vim_press(key) {
            continue;
        }
        match key.code() {
            Code::Char(c) if key.as_char().is_some() => app.run(Command::Insert(c.to_string())),
            Code::Named(Named::Enter) => app.run(Command::Newline),
            Code::Named(Named::Backspace) => app.run(Command::Backspace { word: false }),
            _ => {}
        }
    }
}

/// 返回当前文档的文本。
fn text(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    doc.text_range(0, doc.text().end_pos()).unwrap()
}

/// 返回当前文档主选区的端点。
fn head(app: &App) -> usize {
    app.editor.active().unwrap().selection.head
}

/// 绘制一帧并返回全部像素。
fn pixels(app: &mut App) -> Vec<u32> {
    paint(app);
    let renderer = app.renderer.as_ref().unwrap();
    renderer.image().unwrap().pixels.clone()
}

/// 返回光标占用的像素数：光标亮、灭、再亮三帧中，随光标亮灭而变化的像素。
///
/// 后台任务（文件树、Git 状态）的结果可能在两帧之间到达；只在一次切换中变化的像素不计入。
fn caret_pixels(app: &mut App) -> usize {
    app.blink.visible = true;
    let lit = pixels(app);
    app.blink.visible = false;
    let dark = pixels(app);
    app.blink.visible = true;
    let again = pixels(app);
    (lit.iter().zip(&dark).zip(&again))
        .filter(|((lit, dark), again)| lit != dark && lit == again)
        .count()
}

#[test]
fn vim_mode_is_off_by_default_and_keeps_no_state() {
    let dir = sample_folder("vim-off");
    let mut app = app_with(&dir.join("a.txt"), false);
    assert!(app.vim.is_none());
    assert_eq!(
        (app.vim_mode(), app.vim_caret(), app.vim_status()),
        (None, None, None)
    );
    // 关闭时按键不经 Vim 层，编辑区仍然接受文字。
    assert!(!app.vim_press(winc_vim::Key::char('x')));
    assert!(app.takes_text());

    // 设置页的开关与命令都能开启和关闭，并写入配置。
    app.settings_toggle(Setting::VimMode);
    assert!(app.vim.is_some());
    assert_eq!(app.config.vim_enabled, Some(true));
    assert!(app.run_action(Action::ToggleVimMode));
    assert!(app.vim.is_none());
    assert_eq!(app.config.vim_enabled, Some(false));
    assert!(app.takes_text());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn modes_decide_text_input_the_status_bar_and_the_caret() {
    let (mut app, dir) = vim_app("vim-modes", "one two\nthree\n");

    // 普通模式：不接受文字输入（输入法停用），块状光标，状态栏不显示模式。
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert!(!app.takes_text());
    assert_eq!(app.vim_caret(), Some((0, CaretShape::Block)));
    assert_eq!(app.vim_status(), None);

    // 等待动作：下划线光标，状态栏显示已输入的按键。
    keys(&mut app, "2d");
    assert_eq!(app.vim_caret(), Some((0, CaretShape::Underline)));
    assert_eq!(app.vim_status().as_deref(), Some("2d"));
    assert!(!app.takes_text());
    keys(&mut app, "<Esc>");
    assert_eq!(app.vim_status(), None);

    // 插入模式：接受文字输入，使用竖线光标。
    keys(&mut app, "i");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    assert!(app.takes_text());
    assert_eq!(app.vim_caret(), None);
    assert_eq!(app.vim_status().as_deref(), Some("-- 插入 --"));
    keys(&mut app, "<Esc>");
    assert!(!app.takes_text());

    // 可视模式：光标在选区的光标一端所含的字符上。
    keys(&mut app, "vll");
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    assert!(!app.takes_text());
    assert_eq!(app.editor.active().unwrap().selection, Selection::new(0, 3));
    assert_eq!(app.vim_caret(), Some((2, CaretShape::Block)));
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 --"));
    keys(&mut app, "V\"a");
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 行 --  \"a"));
    keys(&mut app, "<Esc><Esc>");
    assert_eq!(
        (app.vim_mode(), app.vim_status()),
        (Some(Mode::Normal), None)
    );

    // 焦点离开编辑区：空心的块状光标，其他输入框仍然接受文字。
    app.focus = Focus::Tree;
    assert_eq!(
        app.vim_caret().map(|caret| caret.1),
        Some(CaretShape::Hollow)
    );
    assert_eq!(app.vim_mode(), None);
    app.focus = Focus::Search;
    assert!(app.takes_text());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_is_drawn_as_a_block_a_bar_or_an_underline() {
    let (mut app, dir) = vim_app("vim-caret", "one two\nthree\n");
    // 窗口获得焦点后先绘制一帧，之后两帧之间只有光标不同。
    app.window.focused = true;
    paint(&mut app);
    let block = caret_pixels(&mut app);
    keys(&mut app, "d");
    let underline = caret_pixels(&mut app);
    keys(&mut app, "<Esc>i");
    let bar = caret_pixels(&mut app);
    keys(&mut app, "<Esc>");

    // 块覆盖一个字符格，竖线宽 2 逻辑像素，横线高 2 逻辑像素、宽一个字符格。
    let scale = (SCALE * SCALE) as usize;
    let cell = app.cell.round() as usize;
    let line = app.fonts.line_height as usize;
    assert_eq!(bar, 2 * (line - 2) * scale);
    assert_eq!(underline, cell * 2 * scale);
    assert!(block > bar * 2 && block <= cell * line * scale, "{block}");

    // 空心块不闪烁，只画四条边：宽一个字符格、高一行的边框。
    app.focus = Focus::Tree;
    assert_eq!(caret_pixels(&mut app), 0);
    let hollow = pixels(&mut app);
    app.config.vim_enabled = Some(false);
    app.vim_apply_config();
    let none = pixels(&mut app);
    let edges = changed(&hollow, &none);
    assert!(edges > 0 && edges < block, "{edges}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_change_and_its_typed_text_undo_together_and_repeat() {
    let (mut app, dir) = vim_app("vim-undo", "one two three\nfour\n");
    keys(&mut app, "cwab cd<Esc>");
    assert_eq!(text(&app), "ab cd two three\nfour\n");
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 4));
    // 点重复重放命令与键入的文字。
    keys(&mut app, "ww.");
    assert_eq!(text(&app), "ab cd two ab cd\nfour\n");
    // 每次修改连同键入的文字是一个撤销单元。
    keys(&mut app, "u");
    assert_eq!(text(&app), "ab cd two three\nfour\n");
    keys(&mut app, "u");
    assert_eq!(
        (text(&app).as_str(), head(&app)),
        ("one two three\nfour\n", 0)
    );
    keys(&mut app, "<C-r>");
    assert_eq!(text(&app), "ab cd two three\nfour\n");

    // 新开的行沿用所在行的缩进；带次数时重复输入，整体是一个撤销单元。
    keys(&mut app, "uI    <Esc>");
    keys(&mut app, "2ox<Esc>");
    assert_eq!(text(&app), "    one two three\n    x\n    x\nfour\n");
    keys(&mut app, "jOy<Esc>");
    assert_eq!(text(&app), "    one two three\n    x\n    x\ny\nfour\n");
    keys(&mut app, "uu");
    assert_eq!(text(&app), "    one two three\nfour\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn alt_shortcuts_of_the_application_stay_with_it() {
    use crate::keymap::{KeyCode, Mods, Stroke};
    let (app, dir) = vim_app("vim-alt", "abc\n");
    // `Alt`+上（上移一行）是应用的快捷键；`Alt`+`J` 没有绑定，仍交给 Vim 层。
    let (up, j) = (KeyCode::Up, KeyCode::Char('j'));
    assert!(app.binds(Stroke {
        key: up,
        mods: Mods::ALT
    }));
    assert!(!app.binds(Stroke {
        key: j,
        mods: Mods::ALT
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_without_a_meaning_never_become_text() {
    let (mut app, dir) = vim_app("vim-keys", "abc\n");
    // 普通模式下没有定义的可见字符被吞掉，功能键与 `Esc` 交给应用。
    assert!(app.vim_press(winc_vim::Key::char('\\')));
    assert!(app.vim_press(notation::parse("<Tab>")[0]));
    assert!(!app.vim_press(notation::parse("<Esc>")[0]));
    assert!(!app.vim_press(notation::parse("<F5>")[0]));
    assert_eq!(text(&app), "abc\n");

    // 不支持的命令只在状态栏提示。
    keys(&mut app, "Q");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Vim 模式尚不支持此命令")
    );
    assert_eq!(
        (text(&app).as_str(), app.vim_mode()),
        ("abc\n", Some(Mode::Normal))
    );

    // 只读文档：修改类命令提示原因，不改变模式；移动与复制仍然可用。
    app.editor.active_mut().unwrap().readonly = true;
    app.status.message = None;
    keys(&mut app, "x");
    assert!(app.status.message.is_some());
    keys(&mut app, "i");
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    keys(&mut app, "l\"+yl");
    assert_eq!((head(&app), app.copied.as_deref()), (1, Some("b")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn selections_and_commands_from_outside_keep_the_mode_in_step() {
    let (mut app, dir) = vim_app("vim-outside", "one two\nthree\n");

    // 鼠标拖出选区：普通模式进入可视模式，之后的命令作用于选区。
    app.editor.active_mut().unwrap().selection = Selection::new(4, 7);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    assert_eq!(app.vim_caret(), Some((6, CaretShape::Block)));
    // 应用的复制等同于复制到系统剪贴板，随后回到普通模式。
    assert!(app.run_action(Action::Copy));
    assert_eq!(app.copied.as_deref(), Some("two"));
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 4));

    // 点击行尾之后：光标退到最后一个字符。
    app.editor.active_mut().unwrap().selection = Selection::point(7);
    paint(&mut app);
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 6));

    // 应用的剪切、粘贴、撤销、重做由 Vim 层执行。
    assert!(app.run_action(Action::Cut));
    assert_eq!(
        (text(&app).as_str(), app.copied.as_deref()),
        ("three\n", Some("one two\n"))
    );
    assert!(app.run_action(Action::Paste));
    assert_eq!((text(&app).as_str(), head(&app)), ("one two\nthree\n", 8));
    assert!(app.run_action(Action::Undo));
    assert_eq!(text(&app), "three\n");
    assert!(app.run_action(Action::Redo));
    assert_eq!(text(&app), "one two\nthree\n");

    // 插入模式下选区不改变模式，应用的命令照原样执行；光标被移动之后另起一个撤销单元。
    keys(&mut app, "ggiab");
    app.editor.active_mut().unwrap().selection = Selection::new(3, 6);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    keys(&mut app, "X<Esc>");
    assert_eq!(text(&app), "aboXtwo\nthree\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "abone two\nthree\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one two\nthree\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_tabs_returns_to_normal_mode_and_ends_the_insert() {
    let (mut app, dir) = vim_app("vim-tabs", "first\n");
    let first = dir.join("vim.txt");
    let second = dir.join("a.txt");
    keys(&mut app, "ix");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));

    app.open_path(&second);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert!(!app.takes_text());

    // 原文档的插入已经结束：回去之后的两次修改各是一个撤销单元。
    app.open_path(&first);
    paint(&mut app);
    assert_eq!(text(&app), "xfirst\n");
    keys(&mut app, "$xx");
    assert_eq!(text(&app), "xfir\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "xfirs\n");
    keys(&mut app, "uu");
    assert_eq!(text(&app), "first\n");

    // 关闭 Vim 模式时同样结束进行中的插入。
    keys(&mut app, "iy");
    app.settings_toggle(Setting::VimMode);
    assert!(app.vim.is_none() && app.takes_text());
    app.run(Command::Insert("z".into()));
    app.run(Command::Undo);
    assert_eq!(text(&app), "yfirst\n");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 返回光标所在的行号。
fn line(app: &App) -> usize {
    let doc = app.editor.active().unwrap();
    doc.text().line_of(doc.selection.head)
}

#[test]
fn searching_uses_the_prompt_previews_the_match_and_keeps_a_history() {
    let (mut app, dir) = vim_app("vim-search", "one two\nthree two\nfour\n");
    keys(&mut app, "/");
    assert_eq!(app.focus, Focus::Prompt);
    assert_eq!(app.prompt_kind, PromptKind::VimSearch { forward: true });
    assert_eq!(app.vim_mode(), None);

    // 输入过程中光标即时移到第一处匹配，匹配被高亮；回车之后停在该处。
    app.edit_field(|field| field.insert("two"));
    paint(&mut app);
    assert_eq!(head(&app), 4);
    assert!(app.vim_highlights());
    app.submit_prompt();
    assert_eq!((app.focus, head(&app)), (Focus::Editor, 4));
    keys(&mut app, "n");
    assert_eq!(head(&app), 14);
    keys(&mut app, "N");
    assert_eq!(head(&app), 4);

    // 再次打开时可以翻出上一次的模式；放弃输入时光标回到原处，高亮取消。
    keys(&mut app, "?");
    assert_eq!(app.prompt_kind, PromptKind::VimSearch { forward: false });
    assert!(app.vim_prompt_key(&Key::Named(NamedKey::ArrowUp), ""));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "two");
    paint(&mut app);
    assert_eq!(head(&app), 14);
    app.close_prompt();
    app.focus = Focus::Editor;
    app.vim_prompt_cancel();
    assert_eq!(head(&app), 4);
    assert!(!app.vim_highlights());

    // 查找可以作为操作符的范围；找不到与无法转换的模式只提示。
    keys(&mut app, "d/");
    app.edit_field(|field| field.insert("four"));
    app.submit_prompt();
    assert_eq!(text(&app), "one \nfour\n");
    for (pattern, message) in [
        ("zzz", "找不到模式"),
        ("a\\%Vb", "模式的第 2 个字符处有无法识别的写法"),
    ] {
        keys(&mut app, "/");
        app.edit_field(|field| field.insert(pattern));
        app.submit_prompt();
        assert_eq!(app.status.message.as_deref(), Some(message), "{pattern}");
    }
    assert_eq!(text(&app), "one \nfour\n");

    // 查找条打开之后以查找条的内容为准。
    keys(&mut app, "0*");
    assert!(app.vim_highlights());
    app.open_search(false);
    assert!(!app.vim_highlights());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn marks_follow_edits_and_jumps_use_the_position_history() {
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let (mut app, dir) = vim_app("vim-marks", &format!("first\n\nsecond\n{body}"));
    app.track_navigation();

    // 标记随其上方插入的行移动。
    keys(&mut app, "jjma");
    let marked = head(&app);
    keys(&mut app, "ggOnew<Esc>");
    keys(&mut app, "`a");
    assert_eq!(head(&app), marked + 4);
    keys(&mut app, "`z");
    assert_eq!(app.status.message.as_deref(), Some("标记 z 没有设置"));

    // 跳转之前的位置记入位置历史：`Ctrl-O` 后退，`Tab` 前进；不远的跳转同样记入。每批事件
    // 处理完之后记录一次位置。
    let step = |app: &mut App, typed: &str| {
        keys(app, typed);
        app.track_navigation();
    };
    step(&mut app, "gg");
    step(&mut app, "G");
    let last = line(&app);
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 0);
    step(&mut app, "<Tab>");
    assert_eq!(line(&app), last);
    step(&mut app, "gg");
    step(&mut app, "}");
    assert_eq!(line(&app), 2);
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 0);
    // 没有移动光标的跳转命令不留下记录。
    step(&mut app, "gg");
    step(&mut app, "j");
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 43);

    // 修改列表：回到最近修改的位置。
    keys(&mut app, "Gx");
    let changed = head(&app);
    keys(&mut app, "ggg;");
    assert_eq!(head(&app), changed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scrolling_commands_move_the_window_and_keep_the_caret_inside() {
    let body: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    let (mut app, dir) = vim_app("vim-scroll", &body);
    let rows = app.visible_lines() as f64;
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;

    // 所在行移到窗口的顶部、中间、底部；顶部留出一行。
    keys(&mut app, "100G");
    keys(&mut app, "zt");
    assert_eq!(scroll(&app), 98.0);
    keys(&mut app, "zz");
    assert_eq!(scroll(&app), 99.0 - (rows / 2.0).floor());
    keys(&mut app, "zb");
    assert_eq!(scroll(&app), 99.0 - rows + 2.0);
    assert_eq!(line(&app), 99);

    // 按行滚动：光标仍在窗口之内时不动，滚出窗口时跟随。
    keys(&mut app, "zz");
    let before = scroll(&app);
    keys(&mut app, "<C-e>");
    assert_eq!((scroll(&app), line(&app)), (before + 1.0, 99));
    keys(&mut app, "zt");
    keys(&mut app, "3<C-e>");
    assert!(line(&app) > 99, "{}", line(&app));

    // 半屏与整屏：光标随窗口移动。
    keys(&mut app, "100Gzz");
    let half = (app.visible_lines() / 2).max(1);
    keys(&mut app, "<C-d>");
    assert_eq!(line(&app), 99 + half);
    keys(&mut app, "<C-u>");
    assert_eq!(line(&app), 99);
    let before = scroll(&app);
    keys(&mut app, "<C-f>");
    assert_eq!(scroll(&app), before + rows - 2.0);
    keys(&mut app, "<PageUp>");
    assert_eq!(scroll(&app), before);

    // 光标与窗口上下边缘之间保留配置的行数。
    app.config.vim_scroll_off = Some(5);
    keys(&mut app, "150G");
    for _ in 0..40 {
        keys(&mut app, "k");
    }
    let row = app.editor.active().unwrap().row_at(head(&app)) as f64;
    assert!(row - scroll(&app) >= 5.0, "{row} {}", scroll(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fold_commands_use_the_blocks_of_the_document() {
    let dir = sample_folder("vim-folds");
    let path = dir.join("fold.rs");
    std::fs::write(
        &path,
        "fn a() {\n    x;\n    if y {\n        z;\n    }\n}\nfn b() {\n    w;\n}\n",
    )
    .unwrap();
    let mut app = app_with(&path, false);
    app.config.vim_enabled = Some(true);
    app.vim_apply_config();
    paint(&mut app);
    let folded = |app: &App| app.editor.active().unwrap().folded();

    // 收起光标所在的块，光标移到块的首行；展开后恢复。
    keys(&mut app, "jzc");
    assert_eq!(folded(&app), [(0, 4)]);
    assert_eq!(line(&app), 0);
    keys(&mut app, "za");
    assert!(folded(&app).is_empty());
    keys(&mut app, "zM");
    assert_eq!(folded(&app).len(), 3);
    keys(&mut app, "zR");
    assert!(folded(&app).is_empty());

    // 在可折叠的块之间移动；收起的块在上下移动中算一行。
    keys(&mut app, "ggzj");
    assert_eq!(line(&app), 2);
    keys(&mut app, "zj");
    assert_eq!(line(&app), 6);
    keys(&mut app, "zk");
    assert_eq!(line(&app), 4);
    keys(&mut app, "ggzcj");
    assert_eq!(line(&app), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_mode_overwrites_and_insert_mode_handles_control_keys() {
    let (mut app, dir) = vim_app("vim-replace", "abcdef\nxyz\n");
    keys(&mut app, "R");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    assert_eq!(app.vim_status().as_deref(), Some("-- 替换 --"));
    assert_eq!(app.vim_caret().map(|c| c.1), Some(CaretShape::Underline));
    assert!(app.takes_text());
    keys(&mut app, "XY");
    assert_eq!(text(&app), "XYcdef\nxyz\n");
    // 退格还原被覆盖的字符。
    keys(&mut app, "<BS>");
    assert_eq!(text(&app), "Xbcdef\nxyz\n");
    keys(&mut app, "<Esc>j.");
    assert_eq!(text(&app), "Xbcdef\nXyz\n");

    // 插入模式的 `Ctrl` 键：删除前一个词、插入寄存器、执行一条普通模式的命令。
    keys(&mut app, "ggyiwA<Space>one<Space>two<C-w><C-r>0<Esc>");
    assert_eq!(text(&app), "Xbcdef one Xbcdef\nXyz\n");
    keys(&mut app, "A!<C-o>");
    assert_eq!(app.vim_status().as_deref(), Some("-- (插入) --"));
    keys(&mut app, "0?<Esc>");
    assert_eq!(text(&app), "?Xbcdef one Xbcdef!\nXyz\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn block_selections_and_multiple_cursors_edit_every_line() {
    let (mut app, dir) = vim_app("vim-block", "abcd\nefgh\nijkl\n");
    let cursors = |app: &App| app.editor.active().unwrap().selections().0;

    // 按块的选区每行一个；插入对各行同时生效，离开后只留一个光标。
    keys(&mut app, "l<C-v>jl");
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 块 --"));
    assert_eq!(cursors(&app), [Selection::new(1, 3), Selection::new(6, 8)]);
    keys(&mut app, "IX<Esc>");
    assert_eq!(text(&app), "aXbcd\neXfgh\nijkl\n");
    assert_eq!(cursors(&app), [Selection::point(1)]);
    keys(&mut app, "l<C-v>jld");
    assert_eq!(text(&app), "aXd\neXh\nijkl\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "aXbcd\neXfgh\nijkl\n");

    // 多个光标：命令在每个光标上各执行一次，粘贴时逐个对应。
    let doc = app.editor.active_mut().unwrap();
    doc.set_selections(vec![Selection::point(0), Selection::point(6)], 0);
    keys(&mut app, "x");
    assert_eq!(text(&app), "Xbcd\nXfgh\nijkl\n");
    assert_eq!(cursors(&app).len(), 2);
    keys(&mut app, "$p");
    assert_eq!(text(&app), "Xbcda\nXfghe\nijkl\n");
    keys(&mut app, "iZ<Esc>");
    assert_eq!(text(&app), "XbcdZa\nXfghZe\nijkl\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_clipboard_setting_decides_what_reaches_the_system_clipboard() {
    use crate::layout::{Setting, SettingControl};
    let (mut app, dir) = vim_app("vim-clipboard", "one\ntwo\n");
    assert_eq!(Setting::VimClipboard.control(), SettingControl::Choice);
    keys(&mut app, "yy");
    assert_eq!(app.copied, None);
    app.set_vim_clipboard(winc_config::VimClipboard::Yank);
    keys(&mut app, "yy");
    assert_eq!(app.copied.as_deref(), Some("one\n"));
    keys(&mut app, "jdd");
    assert_eq!(app.copied.as_deref(), Some("one\n"));
    app.set_vim_clipboard(winc_config::VimClipboard::Always);
    keys(&mut app, "x");
    assert_eq!(app.copied.as_deref(), Some("o"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 在命令行的输入条中输入 `line` 并回车。
fn ex(app: &mut App, line: &str) {
    keys(app, ":");
    app.edit_field(|field| field.insert(line));
    app.submit_prompt();
}

#[test]
fn the_command_line_sits_in_the_status_row_with_history_and_completion() {
    let (mut app, dir) = vim_app("vim-command", "one two\nthree\nfour\nfive\n");
    keys(&mut app, ":");
    assert_eq!(
        (app.focus, app.prompt_kind.clone()),
        (Focus::Prompt, PromptKind::VimCommand)
    );
    // 输入条占状态栏所在的一行；状态栏隐藏时临时显示。
    assert_eq!(app.prompt_bar(), app.layout.status);
    let shown = app.layout.status.h;
    app.close_prompt();
    app.focus = Focus::Editor;
    app.vim_prompt_cancel();
    app.config.status_bar = Some(false);
    app.relayout();
    assert!(app.layout.status.h < shown);
    keys(&mut app, ":");
    assert_eq!(app.layout.status.h, shown);
    paint(&mut app);
    app.edit_field(|field| field.insert("2d"));
    app.submit_prompt();
    assert_eq!(
        (app.focus, text(&app).as_str()),
        (Focus::Editor, "one two\nfour\nfive\n")
    );
    assert!(app.layout.status.h < shown);
    app.config.status_bar = None;
    app.relayout();

    // 带次数与可视模式下预填范围。
    keys(&mut app, "gg2:");
    assert_eq!(app.goto_input.as_ref().unwrap().text(), ".,.+1");
    app.edit_field(|field| field.insert(">"));
    app.submit_prompt();
    assert_eq!(text(&app), "    one two\n    four\nfive\n");

    // 上下键按已输入的前缀翻阅历史。
    ex(&mut app, "set ic");
    keys(&mut app, ":");
    let up = Key::Named(NamedKey::ArrowUp);
    let down = Key::Named(NamedKey::ArrowDown);
    let line = |app: &App| app.goto_input.as_ref().unwrap().text().to_owned();
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(line(&app), "set ic");
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(line(&app), ".,.+1>");
    assert!(app.vim_prompt_key(&down, "") && app.vim_prompt_key(&down, ""));
    assert_eq!(line(&app), "");
    app.edit_field(|field| field.insert("2"));
    assert!(!app.vim_prompt_key(&Key::Character("2".into()), "2"));
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(line(&app), "2d");
    assert!(app.vim_prompt_key(&up, ""));
    assert_eq!(line(&app), "2d");

    // `Tab` 补全命令名与选项名，连续按下时轮换。
    let tab = Key::Named(NamedKey::Tab);
    app.goto_input.as_mut().unwrap().set_text("");
    assert!(!app.vim_prompt_key(&Key::Character("x".into()), "x"));
    app.edit_field(|field| field.insert("tabn"));
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(line(&app), "tabnew");
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(line(&app), "tabnext");
    app.goto_input.as_mut().unwrap().set_text("");
    assert!(!app.vim_prompt_key(&Key::Character("x".into()), "x"));
    app.edit_field(|field| field.insert("set sm"));
    assert!(app.vim_prompt_key(&tab, ""));
    assert_eq!(line(&app), "set smartcase");

    // `Ctrl-R` 之后是寄存器名，`Ctrl-R Ctrl-W` 是光标处的词。
    app.goto_input.as_mut().unwrap().set_text("");
    assert!(!app.vim_prompt_key(&Key::Character("x".into()), "x"));
    app.modifiers = ModifiersState::CONTROL;
    assert!(app.vim_prompt_key(&Key::Character("r".into()), "r"));
    assert!(app.vim_prompt_key(&Key::Character("w".into()), "w"));
    app.modifiers = ModifiersState::empty();
    assert_eq!(line(&app), "four");
    app.close_prompt();
    app.focus = Focus::Editor;
    app.vim_prompt_cancel();
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_and_tab_commands_run_in_the_application() {
    let (mut app, dir) = vim_app("vim-files", "one\ntwo\n");
    let other = dir.join("other.txt");
    std::fs::write(&other, "other\n").unwrap();
    keys(&mut app, "x");
    ex(&mut app, "w");
    assert_eq!(
        std::fs::read_to_string(dir.join("vim.txt")).unwrap(),
        "ne\ntwo\n"
    );

    ex(&mut app, "e other.txt");
    assert_eq!(app.editor.documents.len(), 2);
    assert_eq!(text(&app), "other\n");
    ex(&mut app, "bn");
    assert_eq!(text(&app), "ne\ntwo\n");
    keys(&mut app, "gt");
    assert_eq!(text(&app), "other\n");
    ex(&mut app, "b 1");
    assert_eq!(text(&app), "ne\ntwo\n");
    ex(&mut app, "b 9");
    assert_eq!(app.status.message.as_deref(), Some("没有这个文档"));

    // `gf` 打开光标处的路径；`:q!` 丢弃未保存的修改，`ZZ` 保存并关闭。
    keys(&mut app, "ccsee other.txt<Esc>$gf");
    assert_eq!(text(&app), "other\n");
    keys(&mut app, "gT");
    ex(&mut app, "q!");
    assert_eq!(
        (app.editor.documents.len(), text(&app).as_str()),
        (1, "other\n")
    );
    keys(&mut app, "x");
    keys(&mut app, "ZZ");
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "ther\n");
    assert_eq!(app.editor.documents.len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lists_options_and_highlighting_follow_the_command_line() {
    let (mut app, dir) = vim_app("vim-lists", "one two\nthree two\n");
    keys(&mut app, "yiwjma");
    ex(&mut app, "reg");
    assert!(app.picker.open && app.picker.vim_list.is_some());
    paint(&mut app);
    // 输入的文字筛选各行；回车粘贴选中的寄存器。
    app.picker.input.set_text("\"0");
    app.picker_refresh();
    assert_eq!(app.picker.hits.len(), 1);
    app.picker_accept();
    assert!(!app.picker.open && app.picker.vim_list.is_none());
    assert_eq!(text(&app), "one two\ntonehree two\n");
    keys(&mut app, "ugg");
    ex(&mut app, "marks");
    app.picker.input.set_text("'a");
    app.picker_refresh();
    app.picker_accept();
    assert_eq!(line(&app), 1);
    ex(&mut app, "ls");
    assert_eq!(
        app.picker.vim_list.as_ref().map(|list| list.rows.len()),
        Some(1)
    );
    app.close_picker();

    // 选项：行号、制表符宽度与缩进由应用保存，其余由 Vim 层保存。
    ex(&mut app, "set nonu ts=3 et sw=2");
    assert_eq!(app.config.line_numbers, Some(false));
    let doc = app.editor.active().unwrap();
    assert_eq!(
        (doc.tab_width(4), doc.indent()),
        (3, winc_core::Indent::Spaces(2))
    );
    ex(&mut app, "set ts? nu?");
    assert_eq!(app.status.message.as_deref(), Some("nonumber"));
    ex(&mut app, "set ts=99");
    assert_eq!(app.status.message.as_deref(), Some("命令的参数有误"));

    // 查找的高亮可以用 `:noh` 取消，也可以整体关闭。
    keys(&mut app, "gg*");
    assert!(app.vim_highlights());
    ex(&mut app, "noh");
    assert!(!app.vim_highlights());
    keys(&mut app, "n");
    assert!(app.vim_highlights());
    ex(&mut app, "set nohls");
    assert!(!app.vim_highlights());
    ex(&mut app, "set hls");

    // 逐处确认与记录宏显示在状态栏上。
    ex(&mut app, "%s/two/2/gc");
    assert_eq!(app.vim_status().as_deref(), Some("替换为 2？（y/n/a/q/l）"));
    keys(&mut app, "yq");
    assert_eq!(text(&app), "one 2\nthree two\n");
    keys(&mut app, "qa");
    assert_eq!(app.vim_status().as_deref(), Some("记录 @a"));
    keys(&mut app, "q");
    assert_eq!(app.vim_status(), None);
    ex(&mut app, "zzz");
    assert_eq!(app.status.message.as_deref(), Some("没有这条命令"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mappings_come_from_the_configuration_and_time_out() {
    use winc_config::{VimMapMode, VimMapping};
    let (mut app, dir) = vim_app("vim-maps", "one two\nthree\n");
    let mapping = |mode, keys: &str, to: &str| VimMapping {
        mode,
        keys: keys.to_owned(),
        to: to.to_owned(),
    };
    app.config.vim_keys = vec![
        mapping(VimMapMode::Insert, "jk", "<Esc>"),
        mapping(VimMapMode::Normal, "<Leader>d", "dd"),
        mapping(VimMapMode::Normal, "<Leader>", "j"),
        mapping(VimMapMode::Normal, "gs", "Toggle Word Wrap"),
    ];
    app.config.vim_leader = Some(",".to_owned());
    app.config.vim_timeout_len = Some(0);
    app.vim_apply_config();

    keys(&mut app, "ijk");
    assert_eq!(
        (app.vim_mode(), text(&app).as_str()),
        (Some(Mode::Normal), "one two\nthree\n")
    );
    // 等待后续按键超时之后执行较短的一条映射。
    keys(&mut app, ",");
    assert_eq!(line(&app), 0);
    let wake = app.tick();
    assert!(wake.is_none() || line(&app) == 1);
    assert_eq!(line(&app), 1);
    keys(&mut app, ",d");
    assert_eq!(text(&app), "one two\n");
    // 右侧可以是命令的英文名。
    let wrap = app.config.word_wrap == Some(true);
    keys(&mut app, "gs");
    assert_eq!(app.config.word_wrap == Some(true), !wrap);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn relative_numbers_the_yank_flash_and_set_last_for_this_run() {
    let (mut app, dir) = vim_app("vim-view", "one\ntwo\nthree\n");
    let id = app.editor.active().unwrap().id;
    // 相对行号：配置给出默认值，`:set` 的值优先。
    assert!(!app.vim_relative());
    ex(&mut app, "set rnu");
    assert!(app.vim_relative());
    paint(&mut app);
    ex(&mut app, "set nornu");
    app.config.vim_relative_number = Some(true);
    assert!(!app.vim_relative());
    app.settings_toggle(crate::layout::Setting::VimRelativeNumber);
    assert_eq!(app.config.vim_relative_number, Some(false));

    // `:set` 改动的行号开关只在本次运行中生效：保存配置时换回文件中的值。
    ex(&mut app, "set nonu nowrap list");
    assert_eq!(app.config.line_numbers, Some(false));
    app.vim_swap_options();
    assert_eq!(app.config.line_numbers, None);
    app.vim_swap_options();
    assert_eq!(app.config.line_numbers, Some(false));
    // 在设置页改动之后以设置页的值为准；关闭 Vim 模式时其余各项还原。
    app.settings_toggle(crate::layout::Setting::LineNumbers);
    app.vim_swap_options();
    assert_eq!(app.config.line_numbers, Some(true));
    app.vim_swap_options();
    let whitespace = app.config.render_whitespace;
    assert_eq!(whitespace, Some(winc_config::RenderWhitespace::All));

    // 复制之后被复制的范围高亮片刻；时长为 0 时不高亮。
    keys(&mut app, "ggyy");
    assert_eq!(app.vim_yanked(id), Some(Selection::new(0, 4)));
    paint(&mut app);
    let later = std::time::Instant::now() + std::time::Duration::from_secs(1);
    app.vim_tick(later);
    assert_eq!(app.vim_yanked(id), None);
    app.config.vim_yank_highlight = Some(0);
    keys(&mut app, "yy");
    assert_eq!(app.vim_yanked(id), None);

    app.config.vim_enabled = Some(false);
    app.vim_apply_config();
    assert_eq!(app.config.render_whitespace, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uppercase_marks_reach_other_files_and_are_kept_in_the_state() {
    let (mut app, dir) = vim_app("vim-global-marks", "one\n  two\nthree\n");
    let other = dir.join("other.txt");
    std::fs::write(&other, "other\n").unwrap();
    let first = app.editor.active().unwrap().path.clone().unwrap();
    keys(&mut app, "jwmAgg");
    assert_eq!(app.state.vim_marks, [('A', first.clone(), 1, 2)]);

    ex(&mut app, "e other.txt");
    assert_eq!(text(&app), "other\n");
    ex(&mut app, "marks");
    let rows = &app.picker.vim_list.as_ref().unwrap().rows;
    assert!(
        rows.iter()
            .any(|row| row.label == "'A" && row.text.ends_with("vim.txt:2"))
    );
    app.close_picker();
    // `` ` `` 回到标记所在的文件与位置，`'` 落在该行的第一个非空白字符上。
    keys(&mut app, "`A");
    assert_eq!(
        (text(&app).as_str(), head(&app)),
        ("one\n  two\nthree\n", 6)
    );
    keys(&mut app, "gg'A");
    assert_eq!(head(&app), 6);
    // 标记随编辑移动，离开文档时把新的位置写入状态。
    keys(&mut app, "ggOzero<Esc>");
    keys(&mut app, "<C-^>");
    paint(&mut app);
    assert_eq!(text(&app), "other\n");
    assert_eq!(app.state.vim_marks, [('A', first, 2, 2)]);
    keys(&mut app, "<C-^>`Z");
    assert_eq!(app.status.message.as_deref(), Some("标记 Z 没有设置"));
    assert_eq!(line(&app), 0);

    // `:b` 的参数按文档名补全。
    keys(&mut app, ":");
    app.edit_field(|field| field.insert("b ot"));
    assert!(app.vim_prompt_key(&Key::Named(NamedKey::Tab), ""));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "b other.txt");
    app.submit_prompt();
    assert_eq!(text(&app), "other\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lists_move_with_vim_keys_and_the_focus_follows_window_commands() {
    let (mut app, dir) = vim_app("vim-lists-keys", "one\ntwo\n");
    let key = |c: &str| Key::Character(c.into());
    // 编辑区里的按键不经列表处理。
    assert!(!app.vim_list_key(&key("j"), "j"));
    keys(&mut app, "<C-w>h");
    assert_ne!(app.focus, Focus::Editor);
    app.focus = Focus::Tree;
    for typed in ["j", "k", "G", "h", "l"] {
        assert!(
            app.vim_list_key(&key(typed), &typed.to_lowercase()),
            "{typed}"
        );
    }
    // `gg` 要按两次；其余的字符仍交给列表自己处理。
    assert!(app.vim_list_key(&key("g"), "g") && app.vim_list_key(&key("g"), "g"));
    assert!(!app.vim_list_key(&key("x"), "x"));
    assert!(!app.vim_list_key(&Key::Named(NamedKey::ArrowDown), ""));
    for focus in [Focus::Results, Focus::Outline, Focus::CommitList] {
        app.focus = focus;
        assert!(app.vim_list_key(&key("j"), "j"), "{focus:?}");
    }
    // `Ctrl-W` 之后的方向把焦点移回编辑区。
    app.focus = Focus::Tree;
    app.modifiers = ModifiersState::CONTROL;
    assert!(app.vim_list_key(&key("w"), "w"));
    app.modifiers = ModifiersState::empty();
    assert!(app.vim_list_key(&key("l"), "l"));
    assert_eq!(app.focus, Focus::Editor);

    // 被 Vim 层占用的应用快捷键在「快捷键」页标注。
    assert!(app.vim_takes_keys(&["⌃R".to_owned()]) || app.vim_takes_keys(&["Ctrl+R".to_owned()]));
    assert!(!app.vim_takes_keys(&["⌘S".to_owned(), "F5".to_owned()]));

    // 关闭 Vim 模式之后列表中的字符不再换成方向键。
    app.config.vim_enabled = Some(false);
    app.vim_apply_config();
    app.focus = Focus::Tree;
    assert!(!app.vim_list_key(&key("j"), "j"));
    assert!(!app.vim_takes_keys(&["⌃R".to_owned()]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_lists_the_command_table() {
    let (mut app, dir) = vim_app("vim-help", "one\n");
    ex(&mut app, "h");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.name(), "Vim 模式：命令一览");
    let page = text(&app);
    for expected in [
        "ys ds cs",
        ":s[ubstitute]",
        "relativenumber  (rnu)",
        "<C-w>h",
        "不支持",
    ] {
        assert!(page.contains(expected), "{expected}");
    }
    // 帮助页只读，移动与复制可用。
    keys(&mut app, "ggjyyx");
    assert_eq!(text(&app), page);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_editable_side_of_a_diff_takes_vim_keys() {
    use winc_editor::diff_view::{DiffSide, DiffView};

    let (mut app, dir) = vim_app("vim-diff", "one\ntwo\nthree\n");
    let path = app.editor.active().unwrap().path.clone();
    let source = app.editor.active().unwrap().id;
    let language = winc_syntax::detect("vim.txt");
    let mut view = DiffView::new(
        DiffSide::new("old", "one\nthree\n", None, language),
        DiffSide::new("worktree", "one\ntwo\nthree\n", path, language),
    );
    view.editing = true;
    app.editor.show_diff_in(None, "vim.txt (worktree)", view);
    app.focus = Focus::Editor;
    paint(&mut app);
    let source_text = |app: &App| {
        let doc = app
            .editor
            .documents
            .iter()
            .find(|doc| doc.id == source)
            .unwrap();
        doc.text_range(0, doc.text().end_pos()).unwrap()
    };
    // 光标在可编辑的一侧：按键作用于该侧对应的文档，差异随之更新。
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    keys(&mut app, "ggjdd");
    assert_eq!(source_text(&app), "one\nthree\n");
    assert!(app.editor.active().unwrap().diff.is_some());
    keys(&mut app, "u");
    assert_eq!(source_text(&app), "one\ntwo\nthree\n");
    keys(&mut app, "cwTWO");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    keys(&mut app, "<Esc>");
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    // 离开可编辑的一侧之后不再处理。
    let view = app
        .editor
        .active_mut()
        .unwrap()
        .diff
        .as_deref_mut()
        .unwrap();
    view.editing = false;
    assert_eq!(app.vim_mode(), None);
    let _ = std::fs::remove_dir_all(&dir);
}
