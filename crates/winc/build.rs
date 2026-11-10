//! 构建脚本：把应用图标和各文件类型的图标嵌入 Windows 的 `winc.exe`。
//!
//! 编号 1 是应用图标，窗口、任务栏和「打开方式」用它。后面每一种文件类型一张
//! （见 `shell_icons`），资源管理器和属性窗口按编号取。

#[path = "build_icon.rs"]
mod build_icon;
#[path = "../winc-app/src/shell_icons.rs"]
mod shell_icons;

use std::env::var;
use std::path::Path;

/// 应用图标的路径，相对于本 crate 的目录。
const ICO: &str = "../winc-app/assets/winc.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_icon.rs");
    println!("cargo:rerun-if-changed={ICO}");
    println!("cargo:rerun-if-changed=../winc-app/src/shell_icons.rs");
    println!("cargo:rerun-if-changed=../winc-app/assets/file-icons");

    let msvc = var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !msvc {
        return;
    }
    let app = std::fs::read(ICO).expect("读取应用图标");
    let mut owned = vec![(shell_icons::APP_ICON_ID, app)];
    for (index, icon) in shell_icons::FILE_ICONS.iter().enumerate() {
        let id = shell_icons::APP_ICON_ID + 1 + index as u16;
        owned.push((id, file_ico(icon.asset)));
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

/// 把 Notepads 的四档原图包成 ICO：16、32、48 用位图，512 用 PNG。
///
/// 资源管理器按视图挑选尺寸。四档都是同一张 PNG 时，它只认最大的那张再缩小，
/// 列表里的「MD」就会糊成一块。
fn file_ico(asset: &str) -> Vec<u8> {
    let mut images = Vec::new();
    for size in [16u32, 32, 48, 512] {
        let path = format!("../winc-app/assets/file-icons/{asset}.targetsize-{size}.png");
        let png = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        images.push((size, build_icon::icon_payload(&png, size)));
    }
    build_icon::pngs_to_ico(&images)
}
