//! 截图的测试：将界面绘制到内存并保存为 PNG，用于人工检查界面改动的效果。
//!
//! 这些测试都带有 `#[ignore]`，需要手动运行；输出目录见 [`shoot`]。

mod icons;
mod settings;
mod sidebar;

use super::*;
#[cfg(unix)]
use winc_editor::format::Task;

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_placeholder() {
    let dir = sample_folder("placeholder");

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };

        let mut app = app_with(&dir.join("data.bin"), dark);
        shoot(&mut app, &format!("placeholder-binary-{theme}"));

        let mut app = app_with(&dir.join("missing.txt"), dark);
        shoot(&mut app, &format!("placeholder-missing-{theme}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_long_names() {
    let dir = sample_folder("long-names");

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let mut app = app_with(&dir.join("img_CubLwUlc3Adg-NoCRm7Qz2Kp.txt"), dark);
        app.sidebar.width = 230.0;
        app.relayout();

        // 选中的行放不下完整名称，鼠标不在文件树内。
        app.on_mouse_move(700.0, 400.0);
        shoot(&mut app, &format!("tree-truncated-{theme}"));

        // 鼠标位于另一个放不下完整名称的行上。
        let other = row_of(&app, "img_WNoi");
        hover_tree_row(&mut app, other, 120.0);
        shoot(&mut app, &format!("tree-hover-{theme}"));

        // 文件树有焦点，鼠标位于选中的行上。
        app.focus = Focus::Tree;
        let selected = row_of(&app, "img_CubL");
        hover_tree_row(&mut app, selected, 120.0);
        shoot(&mut app, &format!("tree-hover-selected-{theme}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 将线条图标并排绘制并放大，用于检查图标风格是否一致。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_line_icons() {
    type Draw = fn(&mut Frame<'_>, (f32, f32), Color, Rect);
    let icons: [Draw; 10] = [
        crate::icons::add_file,
        crate::icons::add_folder,
        crate::icons::refresh,
        crate::icons::collapse_all,
        crate::icons::locate,
        crate::icons::filter,
        crate::icons::clear_results,
        crate::icons::more,
        crate::icons::replace_one,
        crate::icons::replace_all,
    ];

    for (tag, dark) in [("light", false), ("dark", true)] {
        let p = Palette::for_dark(dark);
        let mut renderer =
            Renderer::offscreen(10.0 * 32.0, 32.0, 4.0, p.sidebar_bg, None, None).unwrap();
        renderer.draw(|frame| {
            let all = Rect::new(0.0, 0.0, 10.0 * 32.0, 32.0);
            frame.rect(all, p.sidebar_bg);

            for (i, draw) in icons.iter().enumerate() {
                let cell = Rect::new(i as f32 * 32.0 + 4.0, 4.0, 24.0, 24.0);
                draw(frame, (cell.x + 12.0, cell.y + 12.0), p.text_muted, cell);
            }
        });
        let dir = std::env::var_os("WINC_SNAPSHOT_DIR").map_or_else(
            || std::env::temp_dir().join("winc-snapshots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&dir).unwrap();
        write_png(
            &dir.join(format!("icons-{tag}.png")),
            renderer.image().unwrap(),
        )
        .unwrap();
    }
}

/// 顶栏的标签：长短不一的文件名、没有扩展名的长名字，以及两个同名文件（名称后显示上级目录）。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_tabs() {
    let dir = sample_folder("tabs-shot");
    let files = [
        ".gitignore",
        "AGENTS.md",
        "00d8067972d4c1e0a6b3f2e987157b5e-a",
        "patch.js",
        "src/editor/mod.rs",
        "src/render/mod.rs",
    ];
    for file in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x\n").unwrap();
    }

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir.join(files[0]), dark);
        for file in &files[1..] {
            app.open_path(&dir.join(file));
        }
        app.activate_tab(1);
        shoot(&mut app, &format!("tabs-{tag}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 顶栏左端放大 4 倍：侧栏收起、展开两种状态，用于检查红绿灯与右侧图标的间距和垂直对齐。
///
/// 红绿灯由系统绘制，离屏截图里没有，因此按 `TRAFFIC_LIGHTS_END` 的默认位置（按钮边长 14、
/// 间距 23 逻辑像素）画上同样大小的圆代替。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_top_left() {
    const ZOOM: f32 = 4.0;
    const CROP: (f32, f32) = (260.0, TOP_BAR_END + ISLAND_MARGIN);
    let dir = search_folder("top-left");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        // 一个改过的未命名标签：看侧栏开关、标签图标、修改圆点、加号是否在一条线上。
        app.new_file();
        app.run(Command::Insert("123".into()));
        for (state, visible) in [("open", true), ("closed", false)] {
            app.set_sidebar(visible);
            app.renderer = Some(
                Renderer::offscreen(SIZE.0, SIZE.1, ZOOM, app.palette.frame_bg, None, None)
                    .unwrap(),
            );
            paint(&mut app);
            let mut renderer = app.renderer.take().unwrap();
            renderer.draw(|frame| {
                view::draw(&mut app, frame);
                let lights = [0xFF5F57, 0xFEBC2E, 0x28C840];
                for (i, color) in lights.into_iter().enumerate() {
                    let x = TRAFFIC_LIGHT_LEFT + i as f32 * 23.0;
                    let y = ISLAND_MARGIN + (TOP_BAR - 14.0) / 2.0;
                    let circle = Rect::new(x, y, 14.0, 14.0);
                    frame.rect_clipped(circle, Color::hex(color), 7.0, circle);
                }
            });
            let image = renderer.image().unwrap();
            let (w, h) = ((CROP.0 * ZOOM) as usize, (CROP.1 * ZOOM) as usize);
            let pixels = image
                .pixels
                .chunks(image.width as usize)
                .take(h)
                .flat_map(|row| row[..w].iter().copied())
                .collect();
            let crop = Image {
                pixels,
                width: w as u32,
                height: h as u32,
            };
            let out = std::env::var_os("WINC_SNAPSHOT_DIR").map_or_else(
                || std::env::temp_dir().join("winc-snapshots"),
                PathBuf::from,
            );
            std::fs::create_dir_all(&out).unwrap();
            write_png(&out.join(format!("top-left-{state}-{tag}.png")), &crop).unwrap();
            app.renderer = Some(renderer);
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_welcome() {
    let dir = search_folder("welcome");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        shoot(&mut app, &format!("welcome-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_occurrences() {
    let dir = search_folder("occurrence-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/main.rs"));
        // 光标位于第二行的 `config` 上。
        let doc = app.editor.active_mut().unwrap();
        let at = doc.text().line_start(1) + 9;
        doc.selection = winc_core::Selection::point(at);
        shoot(&mut app, &format!("occurrences-{tag}"));

        // 鼠标停在状态栏路径的 `src` 上：该段铺上按钮底色，不覆盖两侧的箭头。
        let (src, _) = app.hits.crumbs[1];
        app.on_mouse_move(src.x + 4.0, src.y + src.h / 2.0);
        shoot(&mut app, &format!("status-crumb-hover-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_rename() {
    let dir = search_folder("rename-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/main.rs"));
        let root = app.editor.workspace.as_ref().unwrap().root.clone();

        app.start_rename(&root.join("src/main.rs"));
        shoot(&mut app, &format!("rename-file-{tag}"));
        app.close_name_input();

        app.start_rename(&root.join("src"));
        app.edit_field(|f| {
            f.select_all();
            f.insert("Cargo.toml")
        });
        app.submit_prompt();
        shoot(&mut app, &format!("rename-error-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_start_page() {
    let dir = search_folder("start-shot");
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();

    for (tag, dark) in [("light", false), ("dark", true)] {
        // 首次启动，没有任何历史。
        let mut app = blank_app(dark);
        shoot(&mut app, &format!("start-empty-{tag}"));

        // 没有打开目录时手动展开侧栏：不绘制项目徽章，避免看起来像已打开了一个名为 `winc` 的项目。
        app.toggle_sidebar();
        app.sidebar.motion = None;
        shoot(&mut app, &format!("start-sidebar-{tag}"));
        app.toggle_sidebar();
        app.sidebar.motion = None;

        // 有四个最近目录，鼠标停在第二个上。
        app.state.recent = vec![
            home.join("project/winc"),
            home.join("work/backend-service"),
            home.join("Downloads/一个中文名字的项目"),
            PathBuf::from("/Volumes/外接盘/archive/very/deep/path/legacy-monorepo"),
        ];
        paint(&mut app);
        let row = app
            .hits
            .start
            .iter()
            .find(|(_, a)| *a == StartAction::Recent(1))
            .unwrap()
            .0;
        app.on_mouse_move(row.x + 60.0, row.y + 10.0);
        shoot(&mut app, &format!("start-recent-{tag}"));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_picker() {
    let dir = search_folder("picker-shot");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/config.rs"));
        app.open_path(&dir.join("Cargo.toml"));
        app.open_path(&dir.join("src/main.rs"));
        app.open_picker();
        app.picker.input.set_text("cfg");
        app.picker_refresh();
        app.on_mouse_move(400.0, 500.0);
        shoot(&mut app, &format!("picker-{tag}"));

        // 查询为空时，最近打开的文件在前，其后是按路径排序的文件列表。
        app.picker.input.set_text("");
        app.picker_refresh();
        shoot(&mut app, &format!("picker-empty-{tag}"));
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_palette_and_definitions() {
    let dir = super::editor::navigation::navigation_folder("palette-shot");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("main.py"));
        app.on_mouse_move(400.0, 500.0);

        // ⇧⌘P：列出命令，右侧显示快捷键。
        app.open_palette();
        app.picker.input.set_text(">格式");
        app.picker_refresh();
        shoot(&mut app, &format!("palette-{tag}"));
        app.close_picker();

        // F12 找到两处 `load` 的定义：列出供选择。
        let text = app
            .editor
            .active()
            .unwrap()
            .text_range(0, usize::MAX)
            .unwrap();
        let at = text.find("load").unwrap() + 1;
        app.select(at, at);
        app.go_to_definition();
        shoot(&mut app, &format!("definitions-{tag}"));
        app.close_picker();

        // `⇧⌘O`：列出当前文档中的符号，选中某项后编辑区随之滚动。截图后删除该文件，
        // 使下一轮循环中前两张截图的文件树不多出该文件。
        let shop = super::editor::navigation::long_python(&dir);
        app.open_path(&shop);
        app.open_symbols();
        app.picker_move(true);
        shoot(&mut app, &format!("symbols-{tag}"));
        app.close_picker();
        std::fs::remove_file(&shop).unwrap();
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_global_search() {
    let dir = search_folder("global-shot");

    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/main.rs"));
        app.on_mouse_move(900.0, 500.0);
        app.open_global_search(false);
        app.global.query.set_text("config");
        app.run_global_search();
        app.global.engine.wait();
        app.poll_global();
        shoot(&mut app, &format!("global-{tag}"));

        // 展开替换和过滤条件，鼠标停在一处匹配上。
        app.global.replace_open = true;
        app.global.details_open = true;
        app.global.replace.set_text("settings");
        app.global.include.set_text("src");
        app.run_global_search();
        app.global.engine.wait();
        app.poll_global();
        paint(&mut app);
        let row = app.result_row_rect(2);
        app.on_mouse_move(row.x + 80.0, row.y + row.h / 2.0);
        shoot(&mut app, &format!("global-replace-{tag}"));

        // 顶部：当前视图是搜索，鼠标停在「文件」上。
        let (files_tab, _) = app.sidebar_tabs()[0];
        app.on_mouse_move(files_tab.x + 4.0, files_tab.y + 4.0);
        app.tooltip.since = None;
        shoot(&mut app, &format!("global-top-{tag}"));

        // 鼠标停在展开替换的箭头上：底色与输入框等高，二者之间留有间隔。
        let expand = app.panel_parts().expand;
        app.on_mouse_move(expand.x + 4.0, expand.y + 4.0);
        app.tooltip.since = Some(Instant::now() - TOOLTIP_DELAY);
        shoot(&mut app, &format!("global-expand-{tag}"));

        // 鼠标停在「全部替换」图标上，提示已显示。
        let all = app.panel_parts().replace_all;
        app.on_mouse_move(all.x + 4.0, all.y + 4.0);
        app.tooltip.since = Some(Instant::now() - TOOLTIP_DELAY);
        shoot(&mut app, &format!("global-tip-{tag}"));

        // 文件内查找条展开替换：「替换」「全部替换」是图标按钮，鼠标停在「替换」上。
        app.open_path(&dir.join("src/config.rs"));
        app.open_search(true);
        app.search.query = "Config".into();
        app.find.input.set_text("Config");
        app.find.replace_input.set_text("Settings");
        paint(&mut app);
        let one = SearchParts::new(app.layout.search).replace_one;
        app.on_mouse_move(one.x + 4.0, one.y + 4.0);
        app.tooltip.since = Some(Instant::now() - TOOLTIP_DELAY);
        shoot(&mut app, &format!("find-replace-{tag}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_status_bar() {
    let dir = search_folder("status-shot");
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/main.rs"));
        paint(&mut app);

        // 鼠标停在状态栏的缩进上：铺一层按钮底色。
        let (rect, _) = *app
            .hits
            .status
            .iter()
            .find(|(_, item)| *item == StatusItem::Indent)
            .unwrap();
        app.on_mouse_move(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        shoot(&mut app, &format!("status-hover-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_editor_sticky_scroll() {
    let dir = search_folder("editor-sticky-shot");
    let mut text = String::from(
        "use std::io;\n\nimpl Store {\n    pub fn load(&self) -> io::Result<()> {\n        for item in self.items() {\n",
    );
    for i in 0..40 {
        text.push_str(&format!("            let value_{i} = item.get({i})?;\n"));
    }
    text.push_str("        }\n        Ok(())\n    }\n\n    pub fn save(&self) {}\n}\n");
    std::fs::write(dir.join("src/store.rs"), text).unwrap();
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/store.rs"));
        app.editor.active_mut().unwrap().scroll_line = 18.0;
        shoot(&mut app, &format!("editor-sticky-{tag}"));
        // `for` 循环接近结束：其置顶行被推上去一半。
        app.editor.active_mut().unwrap().scroll_line = 42.5;
        shoot(&mut app, &format!("editor-sticky-pushed-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_editor_folding() {
    let dir = search_folder("editor-folding-shot");
    let text = "use std::io;\n\nimpl Store {\n    pub fn load(&self) -> io::Result<()> {\n        for item in self.items() {\n            item.check()?;\n            self.cache.insert(item.id, item);\n        }\n        Ok(())\n    }\n\n    pub fn save(&self) -> io::Result<()> {\n        let file = self.open()?;\n        file.write_all(&self.bytes())\n    }\n}\n";
    std::fs::write(dir.join("src/store.rs"), text).unwrap();
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/store.rs"));
        app.editor.active_mut().unwrap().toggle_fold_at(3, 4);
        // 平时只有已折叠的行显示箭头。
        shoot(&mut app, &format!("editor-folding-{tag}"));
        // 鼠标位于行号栏上：所有可折叠的行都显示箭头；之后再悬停在 `⋯` 上。
        let gutter = app.layout.gutter;
        app.on_mouse_move(gutter.x + 6.0, app.layout.text.y + 30.0);
        shoot(&mut app, &format!("editor-folding-gutter-{tag}"));
        let (chip, _) = app.hits.folded[0];
        app.on_mouse_move(chip.x + 4.0, chip.y + 4.0);
        shoot(&mut app, &format!("editor-folding-dots-{tag}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_editor_fold_motion() {
    use std::time::Duration;

    let dir = search_folder("editor-fold-motion-shot");
    let text = "use std::io;\n\nimpl Store {\n    pub fn load(&self) -> io::Result<()> {\n        for item in self.items() {\n            item.check()?;\n            self.cache.insert(item.id, item);\n        }\n        Ok(())\n    }\n\n    pub fn save(&self) -> io::Result<()> {\n        let file = self.open()?;\n        file.write_all(&self.bytes())\n    }\n}\n";
    std::fs::write(dir.join("src/store.rs"), text).unwrap();
    for (tag, dark) in [("light", false), ("dark", true)] {
        let mut app = app_with(&dir, dark);
        app.open_path(&dir.join("src/store.rs"));
        let doc = app.editor.active().unwrap();
        let before = (doc.folds().clone(), doc.scroll_line);
        app.editor.active_mut().unwrap().toggle_fold_at(3, 4);
        let motion = app.fold_motion(before, Clock::new(FOLD_ANIMATION)).unwrap();
        // 收起 `load`：截取动画开始后 20、50、90 毫秒的画面（`FOLD_ANIMATION` 为 160 毫秒）。
        for ms in [20, 50, 90] {
            app.fold_motion = Some(crate::app::FoldMotion {
                clock: Clock::frozen(Duration::from_millis(ms), FOLD_ANIMATION),
                ..motion.clone()
            });
            shoot(&mut app, &format!("editor-fold-motion-{ms}-{tag}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_disk_conflict() {
    let dir = sample_folder("disk-conflict");
    let dir = winc_config::files::canonicalize(&dir).unwrap();

    // 未保存的修改与别的程序写入的内容冲突：右上角是冲突卡片，鼠标停在「使用磁盘版本」上；
    // 再比较两边，得到差异标签。
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let path = dir.join("short.rs");
        std::fs::write(&path, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
        let mut app = app_with(&path, dark);
        app.run(Command::Insert("// mine\n".into()));
        std::fs::write(&path, "fn main() {\n    println!(\"hello, world\");\n}\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(1))
            .unwrap();
        app.sync_documents(|_| true);
        paint(&mut app);
        let (_, buttons) = app.hits.conflict_card.clone().unwrap();
        let take = buttons[1].0;
        app.on_mouse_move(take.x + 6.0, take.y + 6.0);
        shoot(&mut app, &format!("conflict-{theme}"));

        app.resolve_conflict(ConflictAction::Compare);
        app.on_mouse_move(900.0, 600.0);
        shoot(&mut app, &format!("conflict-diff-{theme}"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}
