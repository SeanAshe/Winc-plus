//! 内置的几套配色，以及设置里按深色、浅色列出的主题。

use winc_i18n::t;

use super::*;

pub(super) const fn hex(rgb: u32) -> Color {
    Color::hex(rgb)
}

/// 「墨蓝」：深色主题，文件树和编辑区共用近黑的蓝灰底。
///
/// 橙色关键字、蓝色函数、绿色字符串、紫色的内建名和属性，当前标签带一点蓝。语法色按色相分类
/// （关键字暖、函数冷），再靠明度拉开：色弱时红绿轴最先丢失，冷暖这一轴仍然可分。
pub static DARK: Palette = Palette::build(Tokens {
    dark: true,
    frame: hex(0x101114),
    card: hex(0x181A1E),
    popup: hex(0x212328),
    text: hex(0xDFE1E5),
    text_muted: hex(0x8C9098),
    text_dim: hex(0x5F636B),
    accent: hex(0x548AF7),
    accent_fill: hex(0x3574F0),
    selection: hex(0x214283),
    gutter: [hex(0x4B5059), hex(0xA1A3AB)],
    error: hex(0xF75464),
    warning: hex(0xF2C55C),
    git: [hex(0x6C9FE8), hex(0x73BD79), hex(0x868A91), hex(0xE06C75)],
    tinted_tab: true,
    syntax: [
        hex(0xBCBEC4),
        hex(0xCF8E6D),
        hex(0xC77DBB),
        hex(0x6AAB73),
        hex(0x2AACB8),
        hex(0x7A7E85),
        hex(0x56A8F5),
        hex(0xB3AE60),
        hex(0xC77DBB),
    ],
    ansi: [
        hex(0x2B2D30),
        hex(0xF75464),
        hex(0x5FB865),
        hex(0xD6AE58),
        hex(0x548AF7),
        hex(0xC77DBB),
        hex(0x2AACB8),
        hex(0xDFE1E5),
        hex(0x6F737A),
        hex(0xFF7A86),
        hex(0x73BD79),
        hex(0xF2C55C),
        hex(0x6BA0FF),
        hex(0xD896CB),
        hex(0x4CC3CE),
        hex(0xFFFFFF),
    ],
});

/// 「晴空」：浅色主题，白底、浅灰窗框。
///
/// 藏青关键字、绿色字符串、蓝绿色函数、紫色的内建名和属性，当前标签带一点蓝。语法色按白底
/// 压暗，在卡片底色上的对比度至少 6:1，否则细笔画的彩色字会发虚。
pub static LIGHT: Palette = Palette {
    current_line: hex(0xF5F8FE),
    occurrence: hex(0xEDEBFC),
    ..Palette::build(Tokens {
        dark: false,
        frame: hex(0xEBECF0),
        card: hex(0xFFFFFF),
        popup: hex(0xFFFFFF),
        text: hex(0x1E1F22),
        text_muted: hex(0x6C707E),
        text_dim: hex(0xA8ADBD),
        accent: hex(0x3574F0),
        accent_fill: hex(0x3574F0),
        selection: hex(0xA6D2FF),
        gutter: [hex(0xAEB3C2), hex(0x767A8A)],
        error: hex(0xE55765),
        warning: hex(0xC27D04),
        git: [hex(0x3A6FD8), hex(0x3E8F48), hex(0x8C8F99), hex(0xD5441B)],
        tinted_tab: true,
        syntax: [
            hex(0x080808),
            hex(0x0033B3),
            hex(0x871094),
            hex(0x057414),
            hex(0x1750EB),
            hex(0x767676),
            hex(0x00627A),
            hex(0x736207),
            hex(0x871094),
        ],
        ansi: [
            hex(0x000000),
            hex(0xC7222D),
            hex(0x067D17),
            hex(0x9E7B0A),
            hex(0x1750EB),
            hex(0x871094),
            hex(0x00627A),
            hex(0x6C707E),
            hex(0x5E6066),
            hex(0xE55765),
            hex(0x3E8F48),
            hex(0xC27D04),
            hex(0x3574F0),
            hex(0xA33EB5),
            hex(0x0E8C9E),
            hex(0xA8ADBD),
        ],
    })
};

/// 「炭火」：深色主题，炭灰底、比底色浅一档的窗框。
///
/// 橙色关键字、黄色函数、暗绿字符串、紫色的内建名；查找结果是在卡片上叠出的墨绿色，括号匹配
/// 是暗青色，两者都不与选区的蓝色混淆。
pub static EMBER: Palette = Palette {
    search_match: hex(0x32593D),
    search_current: hex(0x3F7F4E),
    match_brace: hex(0x3B514D),
    occurrence: hex(0x344134),
    ..Palette::build(Tokens {
        dark: true,
        frame: hex(0x313335),
        card: hex(0x2B2B2B),
        popup: hex(0x3C3F41),
        text: hex(0xBBBBBB),
        text_muted: hex(0x8C8C8C),
        text_dim: hex(0x666666),
        accent: hex(0x589DF6),
        accent_fill: hex(0x365880),
        selection: hex(0x214283),
        gutter: [hex(0x606366), hex(0xA4A3A3)],
        error: hex(0xFF6B68),
        warning: hex(0xBBB529),
        git: [hex(0x6897BB), hex(0x629755), hex(0x6C6C6C), hex(0xD5756C)],
        tinted_tab: true,
        syntax: [
            hex(0xA9B7C6),
            hex(0xCC7832),
            hex(0x9876AA),
            hex(0x6A8759),
            hex(0x6897BB),
            hex(0x808080),
            hex(0xFFC66D),
            hex(0xBBB529),
            hex(0x9876AA),
        ],
        ansi: [
            hex(0x3C3F41),
            hex(0xFF6B68),
            hex(0xA8C023),
            hex(0xD6BF55),
            hex(0x5394EC),
            hex(0xAE8ABE),
            hex(0x299999),
            hex(0xBBBBBB),
            hex(0x787878),
            hex(0xFF8785),
            hex(0xB8D433),
            hex(0xE8D06B),
            hex(0x7EAEF1),
            hex(0xC69DD6),
            hex(0x6CDADA),
            hex(0xFFFFFF),
        ],
    })
};

/// 「深夜」：深色主题，编辑区比窗框暗一档（近黑）。
///
/// 青蓝色的按钮和链接，蓝色关键字、青绿色类型、橙色字符串、淡黄色函数；语法色为深色底调亮，
/// 注释仍比正文暗一档，不抢正文。
pub static MIDNIGHT: Palette = Palette::build(Tokens {
    dark: true,
    frame: hex(0x181A1C),
    card: hex(0x121514),
    popup: hex(0x1E2022),
    text: hex(0xC0C2C1),
    text_muted: hex(0x8C8E90),
    text_dim: hex(0x5E6163),
    accent: hex(0x3FA2CC),
    accent_fill: hex(0x2A7BA3),
    selection: hex(0x264F78),
    gutter: [hex(0x6E7681), hex(0xCCCCCC)],
    error: hex(0xF48771),
    warning: hex(0xCCA700),
    git: [hex(0x4F9FD6), hex(0x73C991), hex(0xE5534B), hex(0xE4676B)],
    tinted_tab: false,
    syntax: [
        hex(0xCCCCCC),
        hex(0x569CD6),
        hex(0x4EC9B0),
        hex(0xCE9178),
        hex(0xB5CEA8),
        hex(0x6A9955),
        hex(0xDCDCAA),
        hex(0xC586C0),
        hex(0x9CDCFE),
    ],
    ansi: [
        hex(0x000000),
        hex(0xCD3131),
        hex(0x0DBC79),
        hex(0xE5E510),
        hex(0x2472C8),
        hex(0xBC3FBC),
        hex(0x11A8CD),
        hex(0xE5E5E5),
        hex(0x666666),
        hex(0xF14C4C),
        hex(0x23D18B),
        hex(0xF5F543),
        hex(0x3B8EEA),
        hex(0xD670D6),
        hex(0x29B8DB),
        hex(0xFFFFFF),
    ],
});

/// 「素纸」：浅色主题，纯白底、浅灰窗框。
///
/// 蓝色关键字、暗红字符串、绿色注释、棕色函数，全部按白底压暗到正文附近的明度。
pub static PAPER: Palette = Palette::build(Tokens {
    dark: false,
    frame: hex(0xF0F0F0),
    card: hex(0xFFFFFF),
    popup: hex(0xFFFFFF),
    text: hex(0x3B3B3B),
    text_muted: hex(0x6E6E6E),
    text_dim: hex(0xA0A0A0),
    accent: hex(0x005FB8),
    accent_fill: hex(0x005FB8),
    selection: hex(0xADD6FF),
    gutter: [hex(0x6E7681), hex(0x171184)],
    error: hex(0xC72E0F),
    warning: hex(0xBF8803),
    git: [hex(0x1F7ACC), hex(0x388A34), hex(0xD1402E), hex(0xAD0707)],
    tinted_tab: false,
    syntax: [
        hex(0x1F1F1F),
        hex(0x0000FF),
        hex(0x1F6B82),
        hex(0xA31515),
        hex(0x06724A),
        hex(0x008000),
        hex(0x795E26),
        hex(0xA500CE),
        hex(0x001080),
    ],
    ansi: [
        hex(0x000000),
        hex(0xCD3131),
        hex(0x1A7F1A),
        hex(0x856A00),
        hex(0x0451A5),
        hex(0xBC05BC),
        hex(0x0598BC),
        hex(0x555555),
        hex(0x666666),
        hex(0xCD3131),
        hex(0x14A314),
        hex(0xB08600),
        hex(0x0451A5),
        hex(0xBC05BC),
        hex(0x0598BC),
        hex(0xA5A5A5),
    ],
});

/// 「朱砂」：浅色主题，白底、浅灰窗框。
///
/// 绿色主按钮，红色关键字、深蓝字符串、紫色函数。强调色与主按钮色分开：细线和图标用蓝，
/// 整块铺满的按钮用绿。
pub static VERMILION: Palette = Palette::build(Tokens {
    dark: false,
    frame: hex(0xEFF2F5),
    card: hex(0xFFFFFF),
    popup: hex(0xFFFFFF),
    text: hex(0x1F2328),
    text_muted: hex(0x656D76),
    text_dim: hex(0x8C959F),
    accent: hex(0x0969DA),
    accent_fill: hex(0x1F883D),
    selection: hex(0xB6DCFF),
    gutter: [hex(0x8C959F), hex(0x1F2328)],
    error: hex(0xCF222E),
    warning: hex(0x9A6700),
    git: [hex(0x0969DA), hex(0x1A7F37), hex(0xCF222E), hex(0xBC4C00)],
    tinted_tab: false,
    syntax: [
        hex(0x1F2328),
        hex(0xC11F2A),
        hex(0x953800),
        hex(0x0A3069),
        hex(0x0550AE),
        hex(0x6E7781),
        hex(0x7547C9),
        hex(0x116329),
        hex(0x0550AE),
    ],
    ansi: [
        hex(0x24292F),
        hex(0xCF222E),
        hex(0x116329),
        hex(0x4D2D00),
        hex(0x0969DA),
        hex(0x8250DF),
        hex(0x1B7C83),
        hex(0x6E7781),
        hex(0x57606A),
        hex(0xA40E26),
        hex(0x1A7F37),
        hex(0x633C01),
        hex(0x218BFF),
        hex(0xA475F9),
        hex(0x3192AA),
        hex(0x8C959F),
    ],
});

/// 「暮紫」：深色主题，蓝灰底、比底色深一档的窗框。
///
/// 紫色关键字、绿色字符串、蓝色函数、黄色类型；语法色明度都在正文附近，靠色相区分。
pub static DUSK: Palette = Palette::build(Tokens {
    dark: true,
    frame: hex(0x1E2227),
    card: hex(0x282C34),
    popup: hex(0x2F343E),
    text: hex(0xABB2BF),
    text_muted: hex(0x7F848E),
    text_dim: hex(0x5C6370),
    accent: hex(0x61AFEF),
    accent_fill: hex(0x4D78CC),
    selection: hex(0x3E4451),
    gutter: [hex(0x495162), hex(0xABB2BF)],
    error: hex(0xE06C75),
    warning: hex(0xE5C07B),
    git: [hex(0x61AFEF), hex(0x98C379), hex(0xE06C75), hex(0xD19A66)],
    tinted_tab: false,
    syntax: [
        hex(0xABB2BF),
        hex(0xC678DD),
        hex(0xE5C07B),
        hex(0x98C379),
        hex(0xD19A66),
        hex(0x7F848E),
        hex(0x61AFEF),
        hex(0x56B6C2),
        hex(0xE06C75),
    ],
    ansi: [
        hex(0x3F4451),
        hex(0xE06C75),
        hex(0x98C379),
        hex(0xE5C07B),
        hex(0x61AFEF),
        hex(0xC678DD),
        hex(0x56B6C2),
        hex(0xABB2BF),
        hex(0x7F848E),
        hex(0xEF7B84),
        hex(0xA9D18D),
        hex(0xF0CD8E),
        hex(0x7ABEF5),
        hex(0xD394E5),
        hex(0x6CC5D0),
        hex(0xD7DAE0),
    ],
});

/// 「晨雾」：浅色主题，近白底、浅灰窗框。
///
/// 紫色关键字、绿色字符串、蓝色函数、土黄色类型；语法色按底色压暗，与正文区分靠色相与明度
/// 两方面。
pub static MIST: Palette = Palette::build(Tokens {
    dark: false,
    frame: hex(0xE8E8E9),
    card: hex(0xFAFAFA),
    popup: hex(0xFFFFFF),
    text: hex(0x383A42),
    text_muted: hex(0x696C77),
    text_dim: hex(0xA0A1A7),
    accent: hex(0x4078F2),
    accent_fill: hex(0x4078F2),
    selection: hex(0xE5E5E6),
    gutter: [hex(0x9D9D9F), hex(0x383A42)],
    error: hex(0xE45649),
    warning: hex(0xC18401),
    git: [hex(0x4078F2), hex(0x50A14F), hex(0xE45649), hex(0xC18401)],
    tinted_tab: false,
    syntax: [
        hex(0x383A42),
        hex(0xA325A1),
        hex(0x825800),
        hex(0x336C33),
        hex(0x9A4900),
        hex(0x737378),
        hex(0x2F5BBB),
        hex(0x016693),
        hex(0xA73D33),
    ],
    ansi: [
        hex(0x383A42),
        hex(0xE45649),
        hex(0x50A14F),
        hex(0xC18401),
        hex(0x4078F2),
        hex(0xA626A4),
        hex(0x0184BC),
        hex(0x696C77),
        hex(0x80838C),
        hex(0xCA1243),
        hex(0x3E953A),
        hex(0x986801),
        hex(0x2F5BBB),
        hex(0x8B2A8A),
        hex(0x016693),
        hex(0xC8C8CA),
    ],
});

/// 一个可以选的主题：配置文件中的 id 与对应的配色；设置里显示的名字见 [`ThemeEntry::name`]。
///
/// `id` 是配置文件中的稳定标识，改名时保持不变；配色由本项目定义，并按需要的对比度调整过。
pub struct ThemeEntry {
    /// 配置文件中的主题 id（如 `winc-night`）。
    pub id: &'static str,
    /// 主题的配色。
    pub palette: &'static Palette,
}

impl ThemeEntry {
    /// 返回设置里显示的主题名。
    pub fn name(&self) -> &'static str {
        match self.id {
            "winc-day" => t!("settings-theme-name-winc-day"),
            "winc-paper" => t!("settings-theme-name-winc-paper"),
            "sky" => t!("settings-theme-name-sky"),
            "paper" => t!("settings-theme-name-paper"),
            "vermilion" => t!("settings-theme-name-vermilion"),
            "mist" => t!("settings-theme-name-mist"),
            "veil-snow" => t!("settings-theme-name-veil-snow"),
            "veil-linen" => t!("settings-theme-name-veil-linen"),
            "winc-night" => t!("settings-theme-name-winc-night"),
            "winc-slate" => t!("settings-theme-name-winc-slate"),
            "ink" => t!("settings-theme-name-ink"),
            "ember" => t!("settings-theme-name-ember"),
            "midnight" => t!("settings-theme-name-midnight"),
            "dusk" => t!("settings-theme-name-dusk"),
            "veil-night" => t!("settings-theme-name-veil-night"),
            "veil-umber" => t!("settings-theme-name-veil-umber"),
            id => id,
        }
    }
}

const fn entry(id: &'static str, palette: &'static Palette) -> ThemeEntry {
    ThemeEntry { id, palette }
}

/// 可选的浅色主题，第一个是默认的：Winc 系列排在前面，为背景图设计的「幕」系列排在最后。
pub static LIGHT_THEMES: [ThemeEntry; 8] = [
    entry("winc-day", &WINC_DAY),
    entry("winc-paper", &WINC_PAPER),
    entry("sky", &LIGHT),
    entry("paper", &PAPER),
    entry("vermilion", &VERMILION),
    entry("mist", &MIST),
    entry("veil-snow", &VEIL_SNOW),
    entry("veil-linen", &VEIL_LINEN),
];

/// 可选的深色主题，第一个是默认的：Winc 系列排在前面，为背景图设计的「幕」系列排在最后。
pub static DARK_THEMES: [ThemeEntry; 8] = [
    entry("winc-night", &WINC_NIGHT),
    entry("winc-slate", &WINC_SLATE),
    entry("ink", &DARK),
    entry("ember", &EMBER),
    entry("midnight", &MIDNIGHT),
    entry("dusk", &DUSK),
    entry("veil-night", &VEIL_NIGHT),
    entry("veil-umber", &VEIL_UMBER),
];

/// 深色（`dark`）或浅色主题的列表。
pub fn themes(dark: bool) -> &'static [ThemeEntry] {
    if dark { &DARK_THEMES } else { &LIGHT_THEMES }
}

/// 名为 `id` 的深色（`dark`）或浅色主题在列表里的位置；没写或不认识时是默认的（0）。
pub fn theme_index(dark: bool, id: Option<&str>) -> usize {
    id.and_then(|id| themes(dark).iter().position(|t| t.id == id))
        .unwrap_or(0)
}
