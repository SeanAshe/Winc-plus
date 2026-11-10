//! 离屏界面的测试：把界面绘制到内存而不是窗口，各子模块共用的辅助函数也定义在这里。
//!
//! 子模块按被测功能划分；本模块提供目录样本、离屏渲染、鼠标与键盘操作、截图等辅助函数。

use std::path::{Path, PathBuf};

use winc_render::{Color, Frame, Image, Rect, Renderer};
use winit::dpi::PhysicalPosition;
use winit::event::{MouseButton, MouseScrollDelta};

use super::input::context_menu::MenuTarget;
use super::*;
use crate::app::StartAction;
use crate::app::workspace::picker::PickerMotion;
use crate::layout::{
    PanelButton, PickerParts, RowAction, Scrollbar, SearchParts, Setting, SettingsPage,
    SettingsPart, SidebarView, StepperParts, THEME_CHOICES, TREE_NAME_MIN, hscrollbar_thumb,
    row_actions, scrollbar_thumb, theme_tile,
};

mod animation;
mod editor;
#[cfg(unix)]
mod git;
mod input;
#[cfg(target_os = "macos")]
mod memory;
mod preview;
mod settings;
mod shots;
mod status;
mod table;
#[cfg(unix)]
mod terminal;
mod windows;
mod workspace;

/// 离屏渲染的逻辑尺寸（宽、高，逻辑像素）。
///
/// 取新窗口的初始大小，截图的布局与真实窗口一致。
const SIZE: (f32, f32) = (1280.0, 820.0);

/// 离屏渲染的缩放倍数（物理像素每逻辑像素）。
///
/// 取 2：Retina 屏的默认缩放倍数，图片与滚动条的像素断言按这个倍数计算。
const SCALE: f32 = 2.0;

/// 创建测试用的目录并返回其路径。
///
/// 内容固定为一组覆盖常见情形的文件：普通文本、名字很长的文件（文件树里放不下，用于截短与
/// 悬停展开）、几种按名字识别图标的文件、`src` 子目录，以及含 NUL 字节的 `data.bin`（按二进制
/// 打开）。进程号放在上一级目录名中，`name` 对应的目录名不含进程号：该目录名会绘制到项目名和
/// 状态栏里，不含进程号才能让不同次运行的截图逐像素一致。
fn sample_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-snapshot-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let code = "fn main() {\n    println!(\"hello\");\n}\n";
    let files = [
        "a.txt",
        "img_CubLwUlc3Adg-NoCRm7Qz2Kp.txt",
        "img_WNoiTOenBOrb4lcV1xYt8.json",
        "short.rs",
        "Cargo.toml",
        "Dockerfile",
    ];

    for file in files {
        std::fs::write(dir.join(file), code).unwrap();
    }

    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), code).unwrap();

    std::fs::write(dir.join("data.bin"), [1, 0, 2, 3]).unwrap();
    dir
}

/// 打开 `path`（目录或文件），按给定主题准备好一个画进内存的界面。
///
/// `path` 为文件时，按命令行打开文件的流程先打开其所在目录，再打开该文件。
fn app_with(path: &Path, dark: bool) -> App {
    let mut app = blank_app(dark);
    if !path.is_dir()
        && let Some(parent) = path.parent()
    {
        app.open_path(parent);
    }
    app.open_path(path);
    app
}

/// 创建刚启动时的离屏界面：没有打开的目录，没有标签，没有历史记录。
fn blank_app(dark: bool) -> App {
    let palette = Palette::for_dark(dark);
    let mut app = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    app.palette = palette;

    let renderer =
        Renderer::offscreen(SIZE.0, SIZE.1, SCALE, palette.frame_bg, None, None).unwrap();
    app.renderer = Some(renderer);
    app.measure_fonts();
    app.relayout();
    app
}

/// 返回文件树中第一个名字以 `prefix` 开头的行的行号（从 0 起）。
///
/// # Panics
///
/// 没有工作区或没有匹配的行时 panic。
fn row_of(app: &App, prefix: &str) -> usize {
    let rows = app.editor.workspace.as_ref().unwrap().rows();
    rows.iter()
        .position(|e| e.name.starts_with(prefix))
        .unwrap()
}

/// 将鼠标移到文件树第 `index` 行（从 0 起）的垂直中心，`x` 为横坐标（逻辑像素）。
fn hover_tree_row(app: &mut App, index: usize, x: f32) {
    let y = app.layout.tree.y + index as f32 * TREE_ROW + TREE_ROW / 2.0;
    app.on_mouse_move(x, y);
}

/// 绘制一帧。
///
/// 第一次绘制会记录各可点击区域的位置，据此重新计算鼠标悬停状态；因此再绘制一次，
/// 使第二帧带有悬停效果。与 `App::redraw` 相同，绘制前先声明底色矩形。
pub(super) fn paint(app: &mut App) {
    app.relayout();
    let mut renderer = app.renderer.take().unwrap();
    app.declare_base(&mut renderer);
    renderer.draw(|frame| view::draw(app, frame));
    app.hover = app.hover_at(app.mouse.0, app.mouse.1);
    app.declare_base(&mut renderer);
    renderer.draw(|frame| view::draw(app, frame));
    app.renderer = Some(renderer);
}

/// 直接删除 `path`（文件或目录），不移入废纸篓，避免测试在用户的废纸篓中留下内容。
fn delete_for_test(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// 在 `(x, y)` 处按下并释放右键。
///
/// 测试中没有窗口，系统菜单不会真正弹出；菜单内容保存在 `app.context_menu` 中，
/// 可用 [`menu_commands`] 读取。
fn right_click(app: &mut App, (x, y): (f32, f32)) {
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Right);
    app.on_mouse_up(MouseButton::Right);
}

/// 返回当前菜单中的全部命令及其是否可用，按显示顺序排列，包含子菜单中的命令。
///
/// # Panics
///
/// 当前没有菜单时 panic。
fn menu_commands(app: &App) -> Vec<(MenuCommand, bool)> {
    fn flatten(
        entries: &[crate::app::input::context_menu::MenuEntry],
        out: &mut Vec<(MenuCommand, bool)>,
    ) {
        for entry in entries {
            match entry {
                crate::app::input::context_menu::MenuEntry::Item(c, enabled)
                | crate::app::input::context_menu::MenuEntry::Alternate(c, enabled) => {
                    out.push((*c, *enabled))
                }
                crate::app::input::context_menu::MenuEntry::Submenu(_, items) => {
                    flatten(items, out)
                }
                crate::app::input::context_menu::MenuEntry::Separator => {}
            }
        }
    }
    let mut out = Vec::new();
    flatten(&app.context_menu.as_ref().unwrap().entries, &mut out);
    out
}

/// 返回文件树第 `index` 行（从 0 起）的点击位置：垂直居中，横向距树区域左边缘 80 逻辑像素。
fn tree_row_center(app: &App, index: usize) -> (f32, f32) {
    let tree = app.layout.tree;
    (
        tree.x + 80.0,
        tree.y + index as f32 * TREE_ROW + TREE_ROW / 2.0,
    )
}

/// 绘制一帧并保存为 `{name}.png`，并将文件路径输出到标准错误。
///
/// 目录取环境变量 `WINC_SNAPSHOT_DIR`；未设置时使用系统临时目录下的 `winc-snapshots`。
fn shoot(app: &mut App, name: &str) {
    paint(app);
    let renderer = app.renderer.take().unwrap();

    let dir = std::env::var_os("WINC_SNAPSHOT_DIR").map_or_else(
        || std::env::temp_dir().join("winc-snapshots"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    write_png(&path, renderer.image().unwrap()).unwrap();
    app.renderer = Some(renderer);
    eprintln!("{}", path.display());
}

/// 创建全局搜索测试用的目录：在 [`sample_folder`] 的基础上，`src/main.rs`、`src/config.rs`、
/// `Cargo.toml` 都含有 `config`（`Config` 在不区分大小写时匹配）。
fn search_folder(name: &str) -> PathBuf {
    let dir = sample_folder(name);
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    let config = load_config();\n    println!(\"{config:?}\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/config.rs"),
        "pub struct Config;\n\npub fn load_config() -> Config {\n    Config\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"config-demo\"\n",
    )
    .unwrap();
    dir
}

/// 等待后台搜索完成，接收结果并绘制一帧。
fn finish_search(app: &mut App) {
    app.global.engine.wait();
    app.poll_global();
    paint(app);
}

/// 返回设置浮层中 `part` 的中心点。
///
/// 几何数据来自 `settings_layout`，与绘制、点击使用同一份布局。
fn settings_point(app: &App, part: SettingsPart) -> (f32, f32) {
    let layout = app.settings_layout();
    let rect = match part {
        SettingsPart::Close => layout.close,
        SettingsPart::SearchField => layout.search,
        SettingsPart::Page(page) => layout.pages.iter().find(|(p, _)| *p == page).unwrap().1,
        SettingsPart::Field(s) | SettingsPart::Switch(s) | SettingsPart::Action(s) => {
            layout.row(s).unwrap().control
        }
        SettingsPart::Copy(s) | SettingsPart::Link(s) => {
            app.settings_value_area(&layout, s).unwrap()
        }
        SettingsPart::FontMenu(s) => {
            crate::layout::font_menu_button(layout.row(s).unwrap().control)
        }
        SettingsPart::Decrease(s) => StepperParts::new(layout.row(s).unwrap().control).minus,
        SettingsPart::Increase(s) => StepperParts::new(layout.row(s).unwrap().control).plus,
        SettingsPart::Theme(choice) => {
            let control = layout.row(Setting::Theme).unwrap().control;
            let i = THEME_CHOICES
                .iter()
                .position(|(c, _)| *c == choice)
                .unwrap();
            crate::layout::theme_segment(control, i)
        }
        SettingsPart::NamedTheme(dark, i) => {
            let setting = if dark {
                Setting::DarkTheme
            } else {
                Setting::LightTheme
            };
            theme_tile(layout.row(setting).unwrap().control, i)
        }
        SettingsPart::KeySearch => app.keys_layout().search,
        SettingsPart::Record => app.keys_layout().record,
        // 链接的位置在绘制时才确定，调用前需要先绘制一帧。
        SettingsPart::Install(id) => {
            let links = &app.hits.install_links;
            links.iter().find(|(l, _)| *l == id).unwrap().1
        }
    };
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

fn click_part(app: &mut App, part: SettingsPart) {
    let point = settings_point(app, part);
    click(app, point);
}

/// 主修饰键（macOS 上是 ⌘，其他系统上是 Ctrl）按下时的修饰键状态：⌘ 单击、⌘ 悬停用。
fn command_modifiers() -> winit::keyboard::ModifiersState {
    if cfg!(target_os = "macos") {
        winit::keyboard::ModifiersState::SUPER
    } else {
        winit::keyboard::ModifiersState::CONTROL
    }
}

/// 测试中的界面文字中回收站的叫法（中文）：macOS 上叫「废纸篓」，其他系统叫「回收站」。
fn trash_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "废纸篓"
    } else {
        "回收站"
    }
}

/// 在 `(x, y)` 处按下并释放左键。
pub(super) fn click(app: &mut App, (x, y): (f32, f32)) {
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
}

/// 将 `image` 写成未压缩的 PNG 文件。
///
/// zlib 流只使用存储块（stored block，每块最多 `0xffff` 字节），无需压缩库；截图文件体积因此较大，
/// 但测试场景可以接受。
fn write_png(path: &Path, image: &Image) -> std::io::Result<()> {
    let (w, h) = (image.width as usize, image.height as usize);
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);

    for row in image.pixels.chunks(w).take(h) {
        // 每行扫描线以滤波类型 0（不滤波）开头。
        raw.push(0);

        for &px in row {
            raw.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8, 0xff]);
        }
    }

    let mut zlib = vec![0x78, 0x01];

    for (i, block) in raw.chunks(0xffff).enumerate() {
        let last = (i + 1) * 0xffff >= raw.len();
        let len = block.len() as u16;
        zlib.push(u8::from(last));
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }

    let (mut a, mut b) = (1u32, 0u32);

    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }

    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut header = Vec::new();
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    // IHDR 的后五个字节：位深 8、颜色类型 6（RGBA）、压缩方法 0、滤波方法 0、不隔行扫描（0）。
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();

    for (kind, data) in [(b"IHDR", &header), (b"IDAT", &zlib), (b"IEND", &Vec::new())] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }

    std::fs::write(path, png)
}

/// 返回 `data` 的 CRC-32，用于 PNG 数据块的校验（多项式 `0xEDB88320`，反射形式）。
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;

    for &byte in data {
        crc ^= u32::from(byte);

        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }

    !crc
}

/// 「提交并推送」进行期间切换了目录：提交完成后不推送（否则会推送到当前打开的仓库），
/// 只提示提交发生在哪个仓库。
#[test]
fn a_commit_finishing_after_a_folder_switch_does_not_push_the_new_repo() {
    let dir = sample_folder("commit-elsewhere");
    let mut app = app_with(&dir, false);
    let elsewhere = dir.join("other-repo");
    app.commit.busy = true;
    app.committed_in(&elsewhere);
    assert!(app.branch.running.is_none());
    assert!(app.commit.busy, "这边正在提交的不受影响");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已在「other-repo」中提交 abc1234")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
