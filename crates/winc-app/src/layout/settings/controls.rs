//! 设置各行控件的几何：控件在行中的位置、步进器的各部分、主题缩略图。

use super::*;

/// 返回字体输入框 `control` 右端下拉按钮的位置。
pub(crate) fn font_menu_button(control: Rect) -> Rect {
    const WIDTH: f32 = 26.0;
    Rect::new(control.right() - WIDTH, control.y, WIDTH, control.h)
}

/// 返回模式分段控件的宽度：各段加上段间距和底槽的内边距。
pub(super) fn segmented_width() -> f32 {
    let n = SETTINGS_SEGMENT_W.len() as f32;
    let w: f32 = SETTINGS_SEGMENT_W.iter().sum();
    w + (n - 1.0) * SEGMENT_SPACING + 4.0
}

/// 返回一行中控件的位置：靠右、垂直居中；主题缩略图占满控件区的高度。
pub(super) fn control_rect(setting: Setting, row: Rect, editing: bool) -> Rect {
    let (w, h) = match setting.control() {
        // 整行都可点击。
        SettingControl::Action | SettingControl::Search => return row,
        // 与「外观」页同一组的模式分段控件同宽。
        SettingControl::Font => (segmented_width(), SETTINGS_CONTROL_H),
        SettingControl::Choice if setting == Setting::Language => {
            (segmented_width(), SETTINGS_CONTROL_H)
        }
        SettingControl::Command if editing => (SETTINGS_COMMAND_W, SETTINGS_CONTROL_H),
        SettingControl::Command | SettingControl::Choice | SettingControl::Link => {
            (SETTINGS_CHOICE_W, SETTINGS_CONTROL_H)
        }
        // 产品信息铺满整行。
        SettingControl::Product => return row,
        SettingControl::Button => (SETTINGS_BUTTON_W, SETTINGS_CONTROL_H),
        SettingControl::Switch => (SETTINGS_SWITCH_W, SETTINGS_SWITCH_H),
        SettingControl::Stepper => (SETTINGS_STEP_W * 2.0 + SETTINGS_VALUE_W, SETTINGS_CONTROL_H),
        // 没有名称（组标题就是名称），缩略图排满整行。
        SettingControl::ThemeList => {
            return row.inset(SETTINGS_INSET, THEME_GRID_PAD);
        }
        SettingControl::Themes => (segmented_width(), SETTINGS_CONTROL_H),
    };
    // 控件宽度不超过行宽减去左右内边距，并给左侧名称至少留出 80 像素。
    let w = w.min((row.w - SETTINGS_INSET * 2.0 - 80.0).max(0.0));
    // 可见内容的右缘对齐组内分隔线的右端：文字、图标类控件的底色向外多出内边距。
    let pad = match setting.control() {
        SettingControl::Choice | SettingControl::Link | SettingControl::Stepper => {
            SETTINGS_GLYPH_PAD
        }
        SettingControl::Font | SettingControl::Command if !editing => SETTINGS_GLYPH_PAD,
        _ => 0.0,
    };
    Rect::new(
        row.right() - SETTINGS_INSET + pad - w,
        row.y + (row.h - h) / 2.0,
        w,
        h,
    )
}

/// 加减控件：左右两个按钮，中间是数值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepperParts {
    /// 左侧的减号按钮。
    pub minus: Rect,
    /// 中间显示数值的区域。
    pub value: Rect,
    /// 右侧的加号按钮。
    pub plus: Rect,
}

impl StepperParts {
    /// 根据控件的矩形 `control` 计算减号、数值和加号的位置。
    pub fn new(control: Rect) -> StepperParts {
        let minus = Rect::new(control.x, control.y, SETTINGS_STEP_W, control.h);
        let plus = Rect::new(
            control.right() - SETTINGS_STEP_W,
            control.y,
            SETTINGS_STEP_W,
            control.h,
        );
        let value = Rect::new(
            minus.right(),
            control.y,
            (plus.x - minus.right()).max(0.0),
            control.h,
        );
        StepperParts { minus, value, plus }
    }
}

/// 主题缩略图每排几张。
///
/// 浅色、深色两组用同一个格子，上下两组的缩略图对齐。
pub const THEME_COLUMNS: usize = 4;
/// 主题缩略图之间横竖的间距（逻辑像素，经验值）：留出能分清两张缩略图的缝，同时保证一行
/// 装得下 `THEME_COLUMNS` 张。
pub(super) const THEME_GRID_GAP: f32 = 14.0;
/// 主题缩略图和组上下边的距离（逻辑像素）：与行内控件的 `SETTINGS_INSET` 相仿，组的上沿不贴
/// 住缩略图。
pub(super) const THEME_GRID_PAD: f32 = 14.0;
/// 缩略图下面一行名称的高度（逻辑像素，含与缩略图之间的空）：够放下一行说明文字。
const THEME_NAME_H: f32 = 24.0;

/// 宽 `width` 的格子里一张主题缩略图（连同下方的名称）的宽和高；缩略图是 16∶10。
pub(super) fn theme_tile_size(width: f32) -> (f32, f32) {
    let columns = THEME_COLUMNS as f32;
    let w = ((width - (columns - 1.0) * THEME_GRID_GAP) / columns).floor();
    (w, (w * 0.625).round() + THEME_NAME_H)
}

/// 返回主题格子 `control` 里第 `index` 张缩略图（从 0 起，连同下方一行名称）占的区域。
pub fn theme_tile(control: Rect, index: usize) -> Rect {
    let (w, h) = theme_tile_size(control.w);
    let col = (index % THEME_COLUMNS) as f32;
    let row = (index / THEME_COLUMNS) as f32;
    Rect::new(
        control.x + col * (w + THEME_GRID_GAP),
        control.y + row * (h + THEME_GRID_GAP),
        w,
        h,
    )
}

/// 返回缩略图本身（不含名称）的区域，位于 `tile` 的上部。
pub fn theme_thumb(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y, tile.w, tile.h - THEME_NAME_H)
}

/// 返回「模式」分段控件 `control` 里第 `index` 段的区域；底槽四周留 2 像素，相邻两段隔
/// `SEGMENT_SPACING`。
///
/// # Panics
///
/// `index` 不小于 3（即超出 `SETTINGS_SEGMENT_W` 的长度）时 panic。
pub fn theme_segment(control: Rect, index: usize) -> Rect {
    let before: f32 = SETTINGS_SEGMENT_W[..index].iter().sum();
    Rect::new(
        control.x + 2.0 + before + index as f32 * SEGMENT_SPACING,
        control.y + 2.0,
        SETTINGS_SEGMENT_W[index],
        control.h - 4.0,
    )
}
