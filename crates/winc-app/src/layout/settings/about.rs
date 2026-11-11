//! 「关于」页的几何与内容：产品信息、版本号与各链接。

use winc_i18n::t;

use super::Setting;
use crate::layout::{SETTINGS_GLYPH_PAD, SETTINGS_LOGO};
use winc_render::Rect;

/// 链接文字与其后外链箭头之间的距离，加上箭头的宽度（逻辑像素）。
pub const LINK_ICON_W: f32 = 6.0 + 9.0;

/// 产品名称，与 README 的标题和 macOS 应用包的显示名一致：显示在「关于」页顶部、开始页，
/// 以及没有打开目录时的窗口标题中。命令行命令仍是小写的 `winc`。
pub const PRODUCT_NAME: &str = "Winc";

/// 「关于」页顶部产品信息的各部分：图标、名称所在的行、版本号所在的行，水平居中。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProductParts {
    /// 产品图标，边长 [`SETTINGS_LOGO`]。
    pub logo: Rect,
    /// 名称所在的行。
    pub name: Rect,
    /// 版本号所在的行。
    pub version: Rect,
}

impl ProductParts {
    /// 根据产品信息一行 `row` 计算各部分的位置（见 [`SETTINGS_PRODUCT_H`]）。
    pub fn new(row: Rect) -> ProductParts {
        let x = (row.x + (row.w - SETTINGS_LOGO) / 2.0).round();
        let logo = Rect::new(x, row.y + 20.0, SETTINGS_LOGO, SETTINGS_LOGO);
        let name = Rect::new(row.x, logo.bottom() + 10.0, row.w, 28.0);
        let version = Rect::new(row.x, name.bottom() + 2.0, row.w, 24.0);
        ProductParts {
            logo,
            name,
            version,
        }
    }

    /// 返回版本号实际占的区域：宽为文字宽 `text_w` 加左右各 10 的内边距，水平居中；悬停底色和
    /// 点击范围都取这一块。
    pub fn version_area(&self, text_w: f32) -> Rect {
        let w = (text_w + 20.0).min(self.version.w);
        let x = self.version.x + (self.version.w - w) / 2.0;
        Rect::new(x, self.version.y, w, self.version.h)
    }
}

/// 返回「关于」页顶部版本号一行的文字，例如「版本 0.1.368 (arm64)」。
pub fn version_line() -> String {
    t!("settings-version-line", version = version_label())
}

/// 返回「关于」页中链接的显示名（如「SeanAshe/Winc-plus」）；不是链接时返回 `None`。
///
/// 仓库和许可证取自 `Cargo.toml` 的 `repository` 和 `license`。
pub fn about_value(setting: Setting) -> Option<String> {
    let text = match setting {
        Setting::Repository => env!("CARGO_PKG_REPOSITORY"),
        Setting::License => env!("CARGO_PKG_LICENSE"),
        _ => return None,
    };
    let text = text.trim_start_matches("https://");
    Some(text.strip_prefix("github.com/").unwrap_or(text).to_owned())
}

/// 返回「关于」页中链接的地址；不是链接时返回 `None`。许可证链接到仓库中的许可证全文。
pub fn about_link(setting: Setting) -> Option<String> {
    let repository = env!("CARGO_PKG_REPOSITORY");
    Some(match setting {
        Setting::Repository => repository.to_owned(),
        Setting::License => format!("{repository}/blob/HEAD/LICENSE"),
        _ => return None,
    })
}

/// 返回链接实际占的区域：右缘与控件 `control` 对齐，宽为内容宽 `content` 加左右内边距，不超过
/// 控件；悬停底色和点击范围都取这一块。
pub fn value_area(control: Rect, content: f32) -> Rect {
    let w = (content + 10.0 + SETTINGS_GLYPH_PAD).min(control.w);
    Rect::new(control.right() - w, control.y, w, control.h)
}

/// 「关于」页的版本号：上游版本、这个 fork 的数字和处理器架构，例如 `0.1.502_115`（`x86_64`）。
pub fn version_label() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("{} ({arch})", crate::release_version::display_version())
}
