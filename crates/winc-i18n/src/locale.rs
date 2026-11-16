//! 支持的界面语言，以及用户在设置中的语言选择。

use unic_langid::LanguageIdentifier;

/// 一种界面语言，每种语言对应 `locales/` 下的一个目录。
// TODO: 新增语言时同步修改 `COUNT`、`ALL`、`tag` 和 `native_name`；常量与变体的一致性由测试保证。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    /// 简体中文。
    ZhCn,
    /// 英文（美国）。
    EnUs,
}

impl Locale {
    /// 支持的语言数量（种），等于 [`Locale::ALL`] 的长度。
    pub const COUNT: usize = 2;

    /// 全部语言，顺序与设置中列出的顺序一致。
    pub const ALL: [Locale; Locale::COUNT] = [Locale::ZhCn, Locale::EnUs];

    /// 源语言：文案首先以这种语言写成，其他语言缺少某条消息时使用这种语言的文本。
    pub const SOURCE: Locale = Locale::ZhCn;

    /// 系统的首选语言都不受支持时使用的语言。
    pub const FALLBACK: Locale = Locale::EnUs;

    /// 返回在 [`Locale::ALL`] 中的序号。
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 返回 BCP 47 语言标签，同时是 `locales/` 下的目录名和配置文件中的写法。
    pub const fn tag(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::EnUs => "en-US",
        }
    }

    /// 返回以该语言自身书写的名称，在语言选项中显示，不随界面语言变化。
    pub const fn native_name(self) -> &'static str {
        match self {
            Locale::ZhCn => "简体中文",
            Locale::EnUs => "English",
        }
    }

    /// 返回解析后的语言标识，供 Fluent 选择复数规则。
    pub(crate) fn langid(self) -> LanguageIdentifier {
        self.tag().parse().expect("内置的语言标签合法")
    }

    /// 返回与 BCP 47 语言标签 `tag` 最接近的受支持语言；只比较语言子标签。
    ///
    /// 例如 `zh-Hans-CN`、`zh-TW` 都对应简体中文，`en-GB` 对应英文。繁体中文没有语言包，
    /// 简体中文是其中最接近的一种。
    pub fn matching(tag: &str) -> Option<Locale> {
        // macOS 返回的标签可能带 `_` 或 `@` 后缀（如 `zh_CN`、`en_US@rg=cnzzzz`）。
        let tag = tag.split('@').next().unwrap_or(tag).replace('_', "-");
        let langid: LanguageIdentifier = tag.parse().ok()?;
        match langid.language.as_str() {
            "zh" => Some(Locale::ZhCn),
            "en" => Some(Locale::EnUs),
            _ => None,
        }
    }

    /// 返回系统首选语言的 POSIX 区域名（如 `zh_CN`）；系统给出的标签不含地区时返回 `None`。
    pub fn system_posix() -> Option<String> {
        let tag = sys_locale::get_locales().next()?;
        let tag = tag.split('@').next().unwrap_or(&tag).replace('_', "-");
        let langid: LanguageIdentifier = tag.parse().ok()?;
        Some(format!("{}_{}", langid.language, langid.region?))
    }

    /// 按系统的首选语言顺序，返回第一种受支持的语言；都不受支持时返回 [`Locale::FALLBACK`]。
    pub fn system() -> Locale {
        sys_locale::get_locales()
            .find_map(|tag| Locale::matching(&tag))
            .unwrap_or(Locale::FALLBACK)
    }
}

/// 用户在设置中选择的界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LocaleChoice {
    /// 跟随系统的首选语言，见 [`Locale::system`]。
    #[default]
    Auto,
    /// 固定使用一种语言。
    Fixed(Locale),
}

impl LocaleChoice {
    /// 全部选项，顺序与设置中列出的顺序一致。
    pub const ALL: [LocaleChoice; Locale::COUNT + 1] = [
        LocaleChoice::Auto,
        LocaleChoice::Fixed(Locale::ZhCn),
        LocaleChoice::Fixed(Locale::EnUs),
    ];

    /// 返回实际使用的语言。
    pub fn resolve(self) -> Locale {
        match self {
            LocaleChoice::Auto => Locale::system(),
            LocaleChoice::Fixed(locale) => locale,
        }
    }

    /// 返回配置文件中的写法：`auto` 或语言标签。
    pub const fn key(self) -> &'static str {
        match self {
            LocaleChoice::Auto => "auto",
            LocaleChoice::Fixed(locale) => locale.tag(),
        }
    }

    /// 按配置文件中的写法 `key` 返回选项；`key` 不是 `auto` 或受支持的语言标签时返回 `None`。
    pub fn from_key(key: &str) -> Option<LocaleChoice> {
        if key == "auto" {
            return Some(LocaleChoice::Auto);
        }
        Locale::ALL
            .into_iter()
            .find(|locale| locale.tag() == key)
            .map(LocaleChoice::Fixed)
    }
}
