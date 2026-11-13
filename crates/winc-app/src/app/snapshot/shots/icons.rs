//! 截图的测试：图标一览、文件图标、各套配色的语法着色。

use super::*;

/// 把所有界面图标排成一张表放大（`icon-sheet-light`、`icon-sheet-dark`）。
///
/// 第一行是工具栏、顶栏、侧栏的图标，第二行是目录和各类文件图标，用于对比笔宽、大小与轻重是否
/// 统一。每格 24×24，格内画一道淡色十字线标出中心，用于检查图标是否居中。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_icon_sheet() {
    use crate::icons::{self, Direction, FileGlyph};
    type Draw = Box<dyn Fn(&mut Frame<'_>, (f32, f32), Color, Rect)>;
    let row1: Vec<Draw> = vec![
        Box::new(|f, c, k, r| icons::sidebar(f, c, true, k, r)),
        Box::new(|f, c, k, r| icons::files(f, c, 1.0, k, r)),
        Box::new(|f, c, k, r| icons::search(f, c, 1.0, k, r)),
        Box::new(|f, c, k, r| icons::git(f, c, 1.0, k, r)),
        Box::new(icons::branch_small),
        Box::new(icons::add_file),
        Box::new(icons::add_folder),
        Box::new(icons::refresh),
        Box::new(icons::collapse_all),
        Box::new(icons::locate),
        Box::new(icons::history),
        Box::new(icons::more),
        Box::new(icons::gear),
        Box::new(icons::lock),
        Box::new(icons::compare),
        Box::new(icons::rollback),
        Box::new(|f, c, k, r| icons::chevron(f, c, 8.0, Direction::Down, k, r)),
        Box::new(|f, c, k, r| icons::close(f, c, 9.0, k, r)),
        Box::new(|f, c, k, r| icons::plus(f, c, 10.0, k, r)),
    ];
    let mut row2: Vec<Draw> = vec![
        Box::new(|f, (x, y), k, r| icons::folder(f, (x - 7.5, y - 6.0), false, k, r)),
        Box::new(|f, (x, y), k, r| icons::folder(f, (x - 7.5, y - 6.0), true, k, r)),
        Box::new(|f, (x, y), k, r| icons::new_file(f, (x - 6.0, y - 7.5), k, r)),
    ];
    for glyph in FileGlyph::ALL {
        row2.push(Box::new(move |f, (x, y), k, r| {
            icons::file(f, (x - 6.0, y - 7.5), glyph, k, r)
        }));
    }
    let cols = row1.len().max(row2.len()) as f32;
    let (w, h) = (cols * 32.0, 64.0);
    for (tag, dark) in [("light", false), ("dark", true)] {
        let p = Palette::for_dark(dark);
        let mut renderer = Renderer::offscreen(w, h, 4.0, p.sidebar_bg, None, None).unwrap();
        renderer.draw(|frame| {
            frame.rect(Rect::new(0.0, 0.0, w, h), p.sidebar_bg);
            for (y, row) in [(0.0, &row1), (32.0, &row2)] {
                for (i, draw) in row.iter().enumerate() {
                    let cell = Rect::new(i as f32 * 32.0 + 4.0, y + 4.0, 24.0, 24.0);
                    frame.rect(cell, p.hover);
                    let c = (cell.x + 12.0, cell.y + 12.0);
                    frame.rect(Rect::new(c.0 - 0.25, cell.y, 0.5, 24.0), p.separator);
                    frame.rect(Rect::new(cell.x, c.1 - 0.25, 24.0, 0.5), p.separator);
                    draw(frame, c, p.text_muted, cell);
                }
            }
        });
        let dir = std::env::var_os("WINC_SNAPSHOT_DIR").map_or_else(
            || std::env::temp_dir().join("winc-snapshots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&dir).unwrap();
        write_png(
            &dir.join(format!("icon-sheet-{tag}.png")),
            renderer.image().unwrap(),
        )
        .unwrap();
    }
}

/// 截图：文件树里各类文件的图标（`file-icons-light`、`file-icons-dark`），用于检查细分后的
/// 记号在真实的一列文件中是否容易辨认、轻重是否统一。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_file_icons() {
    let names = [
        ".env",
        ".gitignore",
        "Cargo.toml",
        "Dockerfile",
        "LICENSE",
        "Makefile",
        "README.md",
        "app.ts",
        "config.yaml",
        "docker-compose.yml",
        "main.rs",
        "notes.md",
        "package.json",
        "pnpm-lock.yaml",
        "ruff.toml",
        "schema.sql",
        "turbo.json",
        "users.csv",
        "NOTICE.txt",
        "server.log",
        "settings.ini",
        "logo.png",
        "demo.mp4",
        "VERSION",
        "index.js",
        "App.jsx",
        "App.tsx",
        "main.py",
        "main.go",
        "main.c",
        "engine.cpp",
        "Main.java",
        "Program.cs",
        "Build.kt",
        "app.rb",
        "View.swift",
    ];
    for (tag, dark) in [("light", false), ("dark", true)] {
        let dir = std::env::temp_dir()
            .join(format!("winc-snapshot-{}", std::process::id()))
            .join(format!("file-icons-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        for name in names {
            std::fs::write(dir.join(name), "x\n").unwrap();
        }
        let mut app = app_with(&dir.join("README.md"), dark);
        shoot(&mut app, &format!("file-icons-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 每个主题下的一段代码，用于检查语法色在各自底色上是否清楚。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_syntax_themes() {
    let dir = sample_folder("syntax-themes");
    let code = r#"use std::collections::HashMap;

/// 按名字统计出现的次数。
#[derive(Debug, Default)]
pub struct Counter {
    counts: HashMap<String, usize>,
    limit: Option<u32>,
}

impl Counter {
    pub fn add(&mut self, name: &str) -> usize {
        // 超过上限就不再计数
        let entry = self.counts.entry(name.to_owned()).or_insert(0);
        *entry += 1;
        if *entry > 42 && self.limit.is_some() {
            println!("too many: {name}");
        }
        *entry
    }
}
"#;
    let file = dir.join("counter.rs");
    std::fs::write(&file, code).unwrap();

    for dark in [false, true] {
        for theme in crate::theme::themes(dark) {
            let mut app = app_with(&file, dark);
            app.palette = theme.palette;
            app.frame
                .renderer
                .as_mut()
                .unwrap()
                .set_background(theme.palette.frame_bg);
            shoot(&mut app, &format!("syntax-{}", theme.id));
        }
    }

    std::fs::remove_dir_all(&dir).unwrap();
}
