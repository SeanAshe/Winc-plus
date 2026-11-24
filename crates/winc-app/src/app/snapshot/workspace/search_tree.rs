//! 侧栏搜索结果以树形显示：标题行的切换按钮、按目录排成的各行、目录的折叠和按键、目录行上的
//! 移除、替换和菜单。

use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuTarget};
use crate::layout::{PanelButton, PanelTool};

/// 一个有几级目录的文件夹，「config」出现在：`Cargo.toml`、`src/main.rs`、`src/config.rs`、
/// `src/app/deep/settings.rs`（`app` 只有一个子目录，与 `deep` 合成一行）、`docs/guide.md`。
fn tree_folder(name: &str) -> PathBuf {
    let dir = search_folder(name);
    std::fs::create_dir_all(dir.join("src/app/deep")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(
        dir.join("src/app/deep/settings.rs"),
        "use crate::config;\nfn read() { config::load_config(); }\n",
    )
    .unwrap();
    std::fs::write(dir.join("docs/guide.md"), "Edit the config file.\n").unwrap();
    dir
}

/// 打开 `dir`，在侧栏搜索「config」并等搜完。
fn searched(dir: &Path) -> App {
    let mut app = app_with(dir, false);
    paint(&mut app);
    app.open_global_search(false);
    app.edit_field(|f| f.insert("config"));
    app.run_global_search();
    finish_search(&mut app);
    app
}

/// 各结果行写成文字：目录是 `名字/`，文件是文件名，匹配是 `·`；前面按层数缩进两格。
fn outline(app: &App) -> Vec<String> {
    let g = &app.panels.global;
    (g.rows().iter())
        .map(|&row| {
            let indent = "  ".repeat(g.row_depth(row));
            let text = match row {
                ResultRow::Dir(d) => format!("{}/", g.dir(d).unwrap().label),
                ResultRow::File(f) => {
                    let path = &g.engine.results[f].path;
                    path.file_name().unwrap().to_string_lossy().into_owned()
                }
                ResultRow::Match(..) => "·".to_owned(),
            };
            format!("{indent}{text}")
        })
        .collect()
}

/// 结果行里写着 `text`（见 [`outline`]，不含缩进）的第一行。
fn row_named(app: &App, text: &str) -> usize {
    outline(app)
        .iter()
        .position(|t| t.trim_start() == text)
        .unwrap_or_else(|| panic!("没有 {text}：{:?}", outline(app)))
}

/// 标题行的按钮在列表和树形之间切换：树形时先是目录、再是文件，各按名字排，只有一个子目录的
/// 几级合成一行，文件后面不再写目录；切换记进状态文件，选中的行保持选中。
#[test]
fn the_layout_button_switches_the_results_between_a_list_and_a_tree() {
    let dir = tree_folder("search-tree-switch");
    let mut app = searched(&dir);
    assert!(!app.search_tree());
    assert!(outline(&app).iter().all(|t| !t.ends_with('/')));
    let main = row_named(&app, "main.rs");
    app.panels.global.select_row(main);

    let (rect, _) = (app.panel_parts().tools.into_iter())
        .find(|(_, t)| *t == PanelTool::Layout)
        .unwrap();
    click(&mut app, (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0));
    paint(&mut app);
    assert!(app.search_tree());
    assert!(app.state.search_tree);
    let shape: Vec<String> = (outline(&app).into_iter())
        .filter(|t| !t.ends_with('·'))
        .collect();
    assert_eq!(
        shape,
        [
            "docs/",
            "  guide.md",
            "src/",
            "  app/deep/",
            "    settings.rs",
            "  config.rs",
            "  main.rs",
            "Cargo.toml",
        ]
    );
    // 匹配比所在的文件深一层。
    let first = row_named(&app, "guide.md") + 1;
    assert_eq!(outline(&app)[first], "    ·");
    let selected = app.panels.global.selected_row();
    assert_eq!(selected, Some(row_named(&app, "main.rs")));

    // 目录一行右端是其中各级文件的匹配总数。
    let g = &app.panels.global;
    let ResultRow::Dir(src) = g.rows()[row_named(&app, "src/")] else {
        panic!();
    };
    let under_src = (g.engine.results.iter())
        .filter(|f| f.relative.starts_with("src/"))
        .map(|f| f.matches.len())
        .sum::<usize>();
    assert_eq!(g.dir(src).unwrap().matches, under_src);
    assert_eq!(g.dir(src).unwrap().files.len(), 3);

    app.press_panel_button(PanelButton::Tool(PanelTool::Layout));
    assert!(!app.search_tree());
    assert!(outline(&app).iter().all(|t| !t.ends_with('/')));
    assert_eq!(
        app.panels.global.selected_row(),
        Some(row_named(&app, "main.rs"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 点目录行折叠、展开它；按住 ⌥ 点时里面的各级一起折叠或展开。「全部折叠」把目录也折叠起来，
/// F4 跳过目录行停在下一处匹配上（先全部展开）。
#[test]
fn directories_fold_like_files() {
    let dir = tree_folder("search-tree-fold");
    let mut app = searched(&dir);
    app.toggle_search_tree();
    let src = row_named(&app, "src/");
    app.click_result(src, false);
    assert!(!outline(&app).iter().any(|t| t.trim_start() == "main.rs"));
    app.click_result(src, false);
    assert!(outline(&app).iter().any(|t| t.trim_start() == "main.rs"));

    // ⌥ 点：连同里面的目录和文件一起折起来；再普通地点开 `src`，里面的仍然折着。
    let all = outline(&app).len();
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    app.click_result(src, false);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    app.click_result(src, false);
    assert!(app.result_row_collapsed(row_named(&app, "app/deep/")));
    assert!(app.result_row_collapsed(row_named(&app, "main.rs")));
    // ⌥ 点两次（折起、再连同里面的一起展开）：恢复全部展开。
    app.pointer.modifiers = winit::keyboard::ModifiersState::ALT;
    app.click_result(src, false);
    app.click_result(src, false);
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(outline(&app).len(), all);

    // 选中的是深处的一处匹配：全部折叠以后改选它最上一级的 `src`。
    app.panels
        .global
        .select_row(row_named(&app, "settings.rs") + 1);
    app.press_panel_button(PanelButton::Tool(PanelTool::CollapseAll));
    assert_eq!(outline(&app), ["docs/", "src/", "Cargo.toml"]);
    assert_eq!(app.panels.global.selected_row(), Some(1));
    assert!(app.panels.global.all_collapsed());
    app.focus = Focus::Results;
    app.panels.global.select_row(0);
    app.next_result(false);
    let selected = app.panels.global.selected_row().unwrap();
    assert!(matches!(
        app.panels.global.rows()[selected],
        ResultRow::Match(..)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 按键同文件树：`←` 折叠展开着的目录、文件，已经折叠或在匹配上时回到上一级；`→` 展开折叠着的，
/// 已经展开时进到第一行。
#[test]
fn arrow_keys_walk_the_result_tree() {
    let dir = tree_folder("search-tree-keys");
    let mut app = searched(&dir);
    app.toggle_search_tree();
    app.focus = Focus::Results;
    let selected = |app: &App| outline(app)[app.panels.global.selected_row().unwrap()].clone();
    let key = |app: &mut App, key: NamedKey| app.results_key(&Key::Named(key));

    app.panels
        .global
        .select_row(row_named(&app, "settings.rs") + 1);
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(selected(&app).trim_start(), "settings.rs");
    key(&mut app, NamedKey::ArrowLeft);
    assert!(app.result_row_collapsed(row_named(&app, "settings.rs")));
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(selected(&app).trim_start(), "app/deep/");
    key(&mut app, NamedKey::ArrowLeft);
    assert!(app.result_row_collapsed(row_named(&app, "app/deep/")));
    key(&mut app, NamedKey::ArrowLeft);
    assert_eq!(selected(&app).trim_start(), "src/");

    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(selected(&app).trim_start(), "app/deep/");
    key(&mut app, NamedKey::ArrowRight);
    assert!(!app.result_row_collapsed(row_named(&app, "app/deep/")));
    key(&mut app, NamedKey::ArrowRight);
    assert_eq!(selected(&app).trim_start(), "settings.rs");
    key(&mut app, NamedKey::Enter);
    assert!(!app.result_row_collapsed(row_named(&app, "settings.rs")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目录行上的「从结果中移除」移除其中的各个文件，「全部替换」替换其中的各个文件；右键菜单没有
/// 「打开」，其余同文件行。
#[test]
fn directory_rows_dismiss_replace_and_offer_a_menu() {
    let dir = tree_folder("search-tree-rows");
    let mut app = searched(&dir);
    app.toggle_search_tree();
    let root = winc_config::files::canonicalize(&dir).unwrap();

    let docs = row_named(&app, "docs/");
    let rect = app.result_row_rect(docs);
    right_click(&mut app, (rect.x + 60.0, rect.y + rect.h / 2.0));
    assert_eq!(
        app.overlays.context_menu.as_ref().map(|m| m.target.clone()),
        Some(MenuTarget::SearchResult(root.join("docs"), None))
    );
    let commands: Vec<MenuCommand> = menu_commands(&app).into_iter().map(|(c, _)| c).collect();
    assert_eq!(
        commands,
        [
            MenuCommand::CopyPath,
            MenuCommand::CopyRelativePath,
            MenuCommand::RevealInTree,
            MenuCommand::DismissResult,
        ]
    );
    app.overlays.context_menu = None;

    app.dismiss_row(docs);
    assert!(!outline(&app).iter().any(|t| t.trim_start() == "guide.md"));

    app.panels.global.replace_open = true;
    app.panels.global.replace.set_text("setting");
    let deep = row_named(&app, "app/deep/");
    app.press_row_action(deep, crate::layout::RowAction::Replace);
    let text = std::fs::read_to_string(root.join("src/app/deep/settings.rs")).unwrap();
    assert!(
        !text.contains("config") && text.contains("setting::load_setting"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 树形结果的样子：浅色和深色，选中一个目录、鼠标停在一个文件上。
///
/// 运行：`cargo test -p winc-app shoot_search_tree -- --ignored`。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_search_tree() {
    for dark in [false, true] {
        let dir = tree_folder(if dark {
            "search-tree-dark"
        } else {
            "search-tree-light"
        });
        let mut app = app_with(&dir, dark);
        paint(&mut app);
        app.open_global_search(false);
        app.edit_field(|f| f.insert("config"));
        app.run_global_search();
        finish_search(&mut app);
        app.toggle_search_tree();
        app.focus = Focus::Results;
        app.panels.global.select_row(row_named(&app, "src/"));
        let row = app.result_row_rect(row_named(&app, "settings.rs"));
        app.on_mouse_move(row.x + 80.0, row.y + row.h / 2.0);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("search-tree-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
