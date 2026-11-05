//! 构建脚本：把应用图标和各文件类型的图标嵌入 Windows 的 `winc.exe`。
//!
//! 编号 1 是应用图标，窗口、任务栏和「打开方式」用它。后面每一种文件类型一张
//! （见 `shell_icons`），资源管理器和属性窗口按编号取。

#[path = "build_icon.rs"]
mod build_icon;
#[path = "src/shell_icons.rs"]
mod shell_icons;

use std::env::var;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_icon.rs");
    println!("cargo:rerun-if-changed=assets/winc.ico");
    println!("cargo:rerun-if-changed=src/shell_icons.rs");

    let msvc = var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !msvc {
        return;
    }
    let app = std::fs::read("assets/winc.ico").expect("读取应用图标");
    let mut owned = vec![(shell_icons::APP_ICON_ID, app)];
    for (index, icon) in shell_icons::FILE_ICONS.iter().enumerate() {
        let id = shell_icons::APP_ICON_ID + 1 + index as u16;
        owned.push((id, file_ico(icon.label, icon.color)));
    }
    let groups: Vec<(u16, &[u8])> = owned
        .iter()
        .map(|(id, bytes)| (*id, bytes.as_slice()))
        .collect();
    let res = build_icon::icon_groups(&groups).expect("解析图标");
    let out = Path::new(&var("OUT_DIR").expect("Cargo 总是设置 OUT_DIR")).join("winc.res");
    std::fs::write(&out, res).expect("写入资源文件");
    println!("cargo:rustc-link-arg-bin=winc={}", out.display());
}

/// 把一种文件图标画成含 16、32、48、256 四档的 ICO。
fn file_ico(label: &str, color: u32) -> Vec<u8> {
    let mut images = Vec::new();
    for size in [16u32, 32, 48, 256] {
        let rgba = build_icon::draw_file_icon(size, label, color);
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&rgba))
            .expect("编码文件图标");
        images.push((size, bytes));
    }
    build_icon::pngs_to_ico(&images)
}
