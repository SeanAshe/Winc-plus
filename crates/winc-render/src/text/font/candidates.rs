//! 各字体槽位的候选字体：用户配置的字体在前，其后是各平台的默认字体与后备字体；以及选日韩文
//! 字体用的语言。

use std::path::PathBuf;

use super::*;

/// 各字体槽位的候选字体。
pub(super) struct Candidates {
    /// 等宽字体的候选，按优先级排列。
    pub(super) mono: Vec<FontFile>,
    /// 界面字体的候选，按优先级排列。
    pub(super) ui: Vec<FontFile>,
    /// 数学字体的候选，按优先级排列。
    pub(super) math: Vec<FontFile>,
    /// 没有配置等宽字体时，系统为等宽字族配置的其余字体（Linux 上为 fontconfig 的回退顺序，
    /// 见 [`fontconfig::chain`]），主字体缺字时依次尝试；配置了时为空。
    pub(super) mono_rest: Vec<FontFile>,
    /// 没有配置界面字体时，系统为无衬线字族配置的其余字体，含义同 `mono_rest`。
    pub(super) ui_rest: Vec<FontFile>,
    /// 每一项是一个后备槽位的候选。
    pub(super) fallbacks: Vec<Choices>,
    /// 彩色 emoji 字体在 `fallbacks` 中的下标（从 0 起）。
    pub(super) emoji: usize,
}

impl Candidates {
    /// 返回各槽位的候选字体：用户配置的字体排在最前面，其后是平台默认字体。
    pub(super) fn for_platform(mono: &[FontFile], ui: &[FontFile]) -> Candidates {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();

        // `WINC_FONT` 环境变量指定的字体排在最前面，其次是用户配置的字体，便于临时试用其他字体。
        let mut mono = mono.to_vec();
        if let Some(font) = std::env::var_os("WINC_FONT") {
            mono.insert(0, FontFile::first(font));
        }
        // 没有指定编辑器字体时，优先用 Maple Mono NF CN；没安装则继续用后面的平台默认字体。
        // 用户写了逗号分隔的字体列表时，`mono` 已是解析出的文件，这里不再插入默认字体。
        if mono.is_empty()
            && let Some(file) =
                crate::FontSpec::Name(crate::DEFAULT_EDITOR_FONT.to_owned()).resolve()
        {
            mono.push(file);
        }
        let mut ui_paths = ui.to_vec();

        let user_fonts = |name: &str| -> Vec<FontFile> {
            vec![
                FontFile::first(home.join("Library/Fonts").join(name)),
                FontFile::first(PathBuf::from("/Library/Fonts").join(name)),
            ]
        };
        let paths = |list: &[&str]| -> Vec<FontFile> {
            list.iter().copied().map(FontFile::first).collect()
        };
        if cfg!(target_os = "macos") {
            mono.extend(user_fonts("JetBrainsMono-Regular.ttf"));
            mono.extend(user_fonts("JetBrainsMono[wght].ttf"));
            mono.extend(paths(&[
                "/System/Library/Fonts/SFNSMono.ttf",
                "/System/Library/Fonts/Menlo.ttc",
                "/System/Library/Fonts/Monaco.ttf",
            ]));

            Candidates {
                mono,
                ui: {
                    ui_paths.extend(paths(&[
                        "/System/Library/Fonts/SFNS.ttf",
                        "/System/Library/Fonts/Helvetica.ttc",
                    ]));
                    ui_paths
                },
                math: paths(&["/System/Library/Fonts/Supplemental/STIXTwoMath.otf"]),
                fallbacks: vec![
                    // 中文先用苹方（系统界面的中文字体）。macOS 15 起它的字形只在 `hvgl` 表中，
                    // 由系统提供轮廓；文件不存在时（更早的系统）用冬青黑体。
                    std::iter::once(FontFile {
                        path: PINGFANG.into(),
                        postscript: Some("PingFangSC-Regular".to_owned()),
                        index: None,
                    })
                    .chain(paths(&[
                        "/System/Library/Fonts/Hiragino Sans GB.ttc",
                        "/System/Library/Fonts/STHeiti Medium.ttc",
                        "/System/Library/Fonts/STHeiti Light.ttc",
                    ]))
                    .collect(),
                    paths(&["/System/Library/Fonts/Menlo.ttc"]),
                    paths(&["/System/Library/Fonts/Apple Symbols.ttf"]),
                    paths(&["/System/Library/Fonts/Apple Color Emoji.ttc"]),
                    paths(&["/System/Library/Fonts/AppleSDGothicNeo.ttc"]),
                ]
                .into_iter()
                .map(Choices::files)
                .collect(),
                emoji: 3,
                mono_rest: Vec::new(),
                ui_rest: Vec::new(),
            }
        } else if cfg!(target_os = "windows") {
            let fonts = crate::text::font_path::windows_fonts();
            let system = |list: &[&str]| -> Vec<FontFile> {
                list.iter()
                    .map(|name| FontFile::first(fonts.join(name)))
                    .collect()
            };
            mono.extend(system(&["CascadiaMono.ttf", "consola.ttf"]));

            Candidates {
                mono,
                ui: {
                    ui_paths.extend(system(&["segoeui.ttf"]));
                    ui_paths
                },
                math: vec![FontFile {
                    path: fonts.join("cambria.ttc"),
                    postscript: Some("CambriaMath".to_owned()),
                    index: None,
                }],
                fallbacks: vec![
                    system(&["msyh.ttc", "simsun.ttc"]),
                    system(&["seguisym.ttf"]),
                    system(&["seguiemj.ttf"]),
                ]
                .into_iter()
                .map(Choices::files)
                .collect(),
                emoji: 2,
                mono_rest: Vec::new(),
                ui_rest: Vec::new(),
            }
        } else {
            // Linux 向 fontconfig 查询：它记录发行版安装字体的位置，并按用户的配置选择。等宽字体和
            // 界面字体启动时就要用到，因此同时查询；应用指定了语言时按该语言选，否则按系统语言选。
            // 没有配置字体时同时取该字族的回退顺序，主字体缺字时依次尝试。
            let language = app_language().map_or_else(String::new, |lang| format!(":lang={lang}"));
            let pattern = |generic: &str| format!("{generic}{language}");
            let best = |generic: &str| fontconfig::best(&pattern(generic)).map(|found| found.file);
            let chain = |generic: &str, configured: bool| {
                if configured {
                    Vec::new()
                } else {
                    fontconfig::chain(&pattern(generic))
                }
            };
            let configured = (!mono.is_empty(), !ui_paths.is_empty());
            let (asked_mono, asked_ui, mono_chain, ui_chain) = std::thread::scope(|scope| {
                let ui = scope.spawn(|| best("sans-serif"));
                let mono_chain = scope.spawn(|| chain("monospace", configured.0));
                let ui_chain = scope.spawn(|| chain("sans-serif", configured.1));
                let asked_mono = best("monospace");
                let asked_ui = ui.join().ok().flatten();
                let mono_chain = mono_chain.join().unwrap_or_default();
                (
                    asked_mono,
                    asked_ui,
                    mono_chain,
                    ui_chain.join().unwrap_or_default(),
                )
            });
            // 主字体仍按最佳匹配选：得分相同的字体在排序中的先后不确定，回退顺序的第一个不一定是
            // 它。回退顺序中略去主字体。
            let rest = |chain: Vec<FontFile>, first: &Option<FontFile>| -> Vec<FontFile> {
                chain
                    .into_iter()
                    .filter(|file| Some(file) != first.as_ref())
                    .collect()
            };
            let (mono_rest, ui_rest) = (rest(mono_chain, &asked_mono), rest(ui_chain, &asked_ui));
            mono.extend(asked_mono);
            ui_paths.extend(asked_ui);
            // 固定路径只用于没有 fontconfig 的系统；fontconfig 可用时只使用它列出的字体，否则会
            // 加回配置中排除的字体。
            let common = !fontconfig::available();
            let fixed = |list: &[&str]| if common { paths(list) } else { Vec::new() };
            mono.extend(fixed(&[
                "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
            ]));
            ui_paths.extend(fixed(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            ]));
            let mut math: Vec<FontFile> = ["STIX Two Math", "Latin Modern Math"]
                .iter()
                .filter_map(|family| fontconfig::lookup(family, false, false))
                .map(|found| found.file)
                .collect();
            math.extend(fixed(&[
                "/usr/share/fonts/opentype/stix/STIXTwoMath-Regular.otf",
                "/usr/share/fonts/opentype/lmodern/latinmodern-math.otf",
                "/usr/share/fonts/texlive-lm-math/latinmodern-math.otf",
            ]));

            Candidates {
                mono,
                ui: ui_paths,
                math,
                fallbacks: vec![
                    Choices {
                        query: Some(format!("sans-serif:lang={}", cjk_language())),
                        files: fixed(&[
                            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                        ]),
                        probe: Some('中'),
                    },
                    Choices {
                        query: Some("emoji".to_owned()),
                        files: fixed(&["/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"]),
                        probe: Some('😀'),
                    },
                ],
                emoji: 1,
                mono_rest,
                ui_rest,
            }
        }
    }
}

/// 应用指定的、选日韩文字体用的语言；为 `None` 时按系统语言选。见 [`set_cjk_language`]。
static CJK_LANGUAGE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 设置选日韩文字体用的语言（如 `zh-cn`），使汉字按该语言的字形显示；`None` 表示按系统语言选。
///
/// 应用的界面语言是中文时传入对应的语言，使界面文字与字形一致。只有之后加载的字体（新建
/// 渲染器、[`crate::Renderer::set_fonts`]）按新语言选。返回语言是否改变，改变时调用方应重新
/// 加载字体。目前只有 Linux 按语言选字体，其他平台的中文字体是固定的。
pub fn set_cjk_language(language: Option<&str>) -> bool {
    let mut current = CJK_LANGUAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let changed = current.as_deref() != language;
    *current = language.map(str::to_owned);
    changed
}

/// 返回应用指定的、选日韩文字体用的语言；没有指定时返回 `None`。
fn app_language() -> Option<String> {
    CJK_LANGUAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// 返回选日韩文字体用的语言：应用指定了就用它，否则按系统语言（`LC_ALL`、`LC_CTYPE`、
/// `LANG`）选，见 [`cjk_language_of`]。
fn cjk_language() -> String {
    if let Some(language) = app_language() {
        return language;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default();
    cjk_language_of(&locale)
}

/// 返回选日韩文字体用的语言：系统语言 `locale`（如 `zh_TW.UTF-8`）是中文、日文或韩文时用它
/// （`zh-tw`），汉字按该地区的字形显示；其他语言用简体中文 `zh-cn`。
fn cjk_language_of(locale: &str) -> String {
    let tag = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('_', "-");
    let language = tag.split('-').next().unwrap_or_default();
    if matches!(language, "ja" | "ko") || (language == "zh" && tag != "zh") {
        tag
    } else {
        "zh-cn".to_owned()
    }
}

#[cfg(test)]
mod tests;
