//! 每一项设置在配置文件里的写法（键名、读入、写回），以及读入各类值时的检查。

use std::collections::BTreeMap;

use winc_i18n::t;

use super::*;

/// 一项设置在配置文件里的写法：键名、读入和写回。
///
/// 解析、序列化以及写回时保留不认识的键，都以 [`SECTIONS`] 和 [`TOP_LEVEL`] 两张表为准；新增一项
/// 设置在对应的表里加一行即可。
pub(super) struct Field {
    /// 键名：节内的键（如 `size`），或顶层的键（如 `theme`）。
    pub(super) key: &'static str,
    /// 把 `value` 读入 `config`；类型不符、超出范围等问题记入 `problems`，文案中用 `name`（如
    /// `editor.size`）指代这一项。
    pub(super) read: fn(config: &mut Config, value: &Json, name: &str, problems: &mut Vec<String>),
    /// 返回要写出的值；`None` 表示用户没写过，不写出。
    pub(super) write: fn(config: &Config) -> Option<Json>,
}

/// 布尔设置的 [`Field`]：键名和 [`Config`] 中对应的字段。
macro_rules! bool_field {
    ($key:literal, $field:ident) => {
        Field {
            key: $key,
            read: |config, value, name, problems| config.$field = read_bool(value, name, problems),
            write: |config| config.$field.map(Json::Bool),
        }
    };
}

/// 字符串设置的 [`Field`]：键名和 [`Config`] 中对应的字段，读入规则见 [`read_string`]。
macro_rules! string_field {
    ($key:literal, $field:ident) => {
        Field {
            key: $key,
            read: |config, value, name, problems| {
                config.$field = read_string(value, name, problems)
            },
            write: |config| config.$field.clone().map(Json::String),
        }
    };
}

/// 配置文件中的各节（顶层值为对象的键）和其中的设置，按写出的顺序排列。
pub(super) const SECTIONS: &[(&str, &[Field])] = &[
    ("editor", EDITOR),
    ("ui", UI),
    ("tree", TREE),
    ("markdown", MARKDOWN),
    ("update", UPDATE),
    ("terminal", TERMINAL),
    ("vim", VIM),
    ("git", GIT),
    ("format", FORMAT),
];

/// 顶层的单值设置，写在各节之后。
pub(super) const TOP_LEVEL: &[Field] = &[
    Field {
        key: "theme",
        read: |config, value, name, problems| {
            config.theme = match value.as_str() {
                Some("auto") => Some(ThemeChoice::Auto),
                Some("dark") => Some(ThemeChoice::Dark),
                Some("light") => Some(ThemeChoice::Light),
                _ => {
                    problems.push(t!("config-theme-invalid", name = name));
                    None
                }
            }
        },
        write: |config| {
            let theme = match config.theme? {
                ThemeChoice::Auto => "auto",
                ThemeChoice::Dark => "dark",
                ThemeChoice::Light => "light",
            };
            Some(Json::String(theme.to_owned()))
        },
    },
    string_field!("lightTheme", light_theme),
    string_field!("darkTheme", dark_theme),
    Field {
        key: "language",
        read: |config, value, name, problems| {
            config.language = value.as_str().and_then(LocaleChoice::from_key);
            if config.language.is_none() {
                problems.push(t!("config-language-invalid", name = name));
            }
        },
        write: |config| Some(Json::String(config.language?.key().to_owned())),
    },
];

/// `editor` 一节。
const EDITOR: &[Field] = &[
    string_field!("font", editor_font),
    Field {
        key: "size",
        read: |config, value, name, problems| {
            let label = t!("config-label-font-size");
            config.editor_size =
                read_number(value, name, Config::EDITOR_SIZE_RANGE, label, problems)
        },
        write: |config| config.editor_size.map(number),
    },
    Field {
        key: "tabWidth",
        read: |config, value, name, problems| {
            let label = t!("config-label-tab-width");
            config.tab_width = read_number(value, name, Config::TAB_WIDTH_RANGE, label, problems)
                .filter(|v| {
                    let whole = v.fract() == 0.0;
                    if !whole {
                        let value = v.to_string();
                        problems.push(t!("config-not-integer", name = name, value = value));
                    }
                    whole
                })
                .map(|v| v as usize)
        },
        write: |config| config.tab_width.map(|width| Json::Number(width as f64)),
    },
    bool_field!("autoSave", auto_save),
    bool_field!("hotExit", hot_exit),
    bool_field!("restoreFolder", restore_folder),
    bool_field!("openFolderInNewWindow", open_folder_in_new_window),
    bool_field!("stickyScroll", sticky_scroll),
    bool_field!("wordWrap", word_wrap),
    bool_field!("wholeLines", whole_lines),
    bool_field!("minimap", minimap),
    bool_field!("outline", outline),
    Field {
        key: "renderWhitespace",
        read: |config, value, name, problems| {
            config.render_whitespace = match value.as_str() {
                Some("none") => Some(RenderWhitespace::None),
                Some("selection") => Some(RenderWhitespace::Selection),
                Some("trailing") => Some(RenderWhitespace::Trailing),
                Some("all") => Some(RenderWhitespace::All),
                _ => {
                    problems.push(t!("config-render-whitespace-invalid", name = name));
                    None
                }
            }
        },
        write: |config| {
            let value = match config.render_whitespace? {
                RenderWhitespace::None => "none",
                RenderWhitespace::Selection => "selection",
                RenderWhitespace::Trailing => "trailing",
                RenderWhitespace::All => "all",
            };
            Some(Json::String(value.to_owned()))
        },
    },
    bool_field!("diffInline", diff_inline),
    bool_field!("lineNumbers", line_numbers),
    bool_field!("statusBar", status_bar),
    bool_field!("restoreWindow", restore_window),
];

/// `ui` 一节。
const UI: &[Field] = &[
    string_field!("font", ui_font),
    bool_field!("animations", ui_animations),
    Field {
        key: "size",
        read: |config, value, name, problems| {
            let label = t!("config-label-font-size");
            config.ui_size = read_number(value, name, Config::UI_SIZE_RANGE, label, problems)
        },
        write: |config| config.ui_size.map(number),
    },
    Field {
        key: "zoom",
        read: |config, value, name, problems| {
            let label = t!("config-label-zoom");
            config.ui_zoom = read_number(value, name, Config::ZOOM_RANGE, label, problems)
                .map(|zoom| zoom.round() as u16);
        },
        write: |config| config.ui_zoom.map(|zoom| number(f32::from(zoom))),
    },
    Field {
        key: "fontWeight",
        read: |config, value, name, problems| {
            let label = t!("config-label-font-weight");
            let range = Config::FONT_WEIGHT_RANGE;
            config.ui_font_weight =
                read_number(value, name, range, label, problems).map(|w| w.round() as u16);
        },
        write: |config| {
            config
                .ui_font_weight
                .map(|weight| number(f32::from(weight)))
        },
    },
    string_field!("backgroundImage", ui_background_image),
    Field {
        key: "backgroundOpacity",
        read: |config, value, name, problems| {
            let label = t!("config-label-background-opacity");
            let range = Config::BACKGROUND_OPACITY_RANGE;
            config.ui_background_opacity = read_number(value, name, range, label, problems);
        },
        write: |config| config.ui_background_opacity.map(number),
    },
    Field {
        key: "backgroundFit",
        read: |config, value, name, problems| {
            config.ui_background_fit = match value.as_str() {
                Some("fill") => Some(BackgroundFit::Fill),
                Some("fit") => Some(BackgroundFit::Fit),
                Some("stretch") => Some(BackgroundFit::Stretch),
                Some("tile") => Some(BackgroundFit::Tile),
                _ => {
                    problems.push(t!("config-background-fit-invalid", name = name));
                    None
                }
            };
        },
        write: |config| {
            let value = match config.ui_background_fit? {
                BackgroundFit::Fill => "fill",
                BackgroundFit::Fit => "fit",
                BackgroundFit::Stretch => "stretch",
                BackgroundFit::Tile => "tile",
            };
            Some(Json::String(value.into()))
        },
    },
    Field {
        // `titleBarStyle` 在配置中是字符串，读入后只保留是否用系统的标题栏。
        key: "titleBarStyle",
        read: |config, value, name, problems| match value.as_str() {
            Some("custom") => config.ui_native_title_bar = Some(false),
            Some("native") => config.ui_native_title_bar = Some(true),
            _ => problems.push(t!("config-title-bar-style-invalid", name = name)),
        },
        write: |config| {
            let native = config.ui_native_title_bar?;
            Some(Json::String(
                if native { "native" } else { "custom" }.into(),
            ))
        },
    },
];

/// `tree` 一节。
const TREE: &[Field] = &[
    bool_field!("confirmDragAndDrop", confirm_drag_and_drop),
    bool_field!("compactFolders", compact_folders),
    bool_field!("previewOnClick", preview_on_click),
];

/// `markdown` 一节。
const MARKDOWN: &[Field] = &[bool_field!("remoteImages", markdown_remote_images)];

/// `update` 一节。
const UPDATE: &[Field] = &[bool_field!("autoCheck", update_auto_check)];

/// `vim` 一节。
const VIM: &[Field] = &[
    bool_field!("enabled", vim_enabled),
    Field {
        key: "clipboard",
        read: |config, value, name, problems| {
            config.vim_clipboard = match value.as_str() {
                Some("register") => Some(VimClipboard::Register),
                Some("yank") => Some(VimClipboard::Yank),
                Some("always") => Some(VimClipboard::Always),
                _ => {
                    problems.push(t!("config-vim-clipboard-invalid", name = name));
                    None
                }
            }
        },
        write: |config| {
            let value = match config.vim_clipboard? {
                VimClipboard::Register => "register",
                VimClipboard::Yank => "yank",
                VimClipboard::Always => "always",
            };
            Some(Json::String(value.to_owned()))
        },
    },
    bool_field!("ignoreCase", vim_ignore_case),
    bool_field!("smartCase", vim_smart_case),
    Field {
        key: "scrollOff",
        read: |config, value, name, problems| {
            let label = t!("config-label-vim-scroll-off");
            let range = Config::VIM_SCROLL_OFF_RANGE;
            config.vim_scroll_off = read_number(value, name, range, label, problems)
                .filter(|v| {
                    let whole = v.fract() == 0.0;
                    if !whole {
                        let value = v.to_string();
                        problems.push(t!("config-not-integer", name = name, value = value));
                    }
                    whole
                })
                .map(|v| v as usize)
        },
        write: |config| {
            config
                .vim_scroll_off
                .map(|lines| Json::Number(lines as f64))
        },
    },
    Field {
        // 对象的每一项是按键的名字与 `vim` 或 `app`；写法有误的项逐个报告并忽略。
        key: "handleKeys",
        read: |config, value, name, problems| {
            let Some(keys) = value.as_object() else {
                problems.push(t!("config-vim-handle-keys-invalid", name = name));
                return;
            };
            config.vim_handle_keys.clear();
            for (key, owner) in keys {
                let key = key.trim().to_lowercase();
                let vim = match owner.as_str() {
                    Some("vim") => true,
                    Some("app") => false,
                    _ => {
                        let name = format!("{name}.{key}");
                        problems.push(t!("config-vim-handle-keys-invalid", name = name));
                        continue;
                    }
                };
                config.vim_handle_keys.push((key, vim));
            }
            config.vim_handle_keys.sort();
        },
        write: |config| {
            if config.vim_handle_keys.is_empty() {
                return None;
            }
            let owner = |vim: &bool| Json::String(if *vim { "vim" } else { "app" }.to_owned());
            let keys = config.vim_handle_keys.iter();
            Some(Json::object(
                keys.map(|(key, vim)| (key.clone(), owner(vim))),
            ))
        },
    },
    bool_field!("relativeNumber", vim_relative_number),
    Field {
        key: "yankHighlight",
        read: |config, value, name, problems| {
            let label = t!("config-label-vim-yank-highlight");
            let range = Config::VIM_YANK_HIGHLIGHT_RANGE;
            config.vim_yank_highlight = read_number(value, name, range, label, problems)
                .filter(|v| {
                    let whole = v.fract() == 0.0;
                    if !whole {
                        let value = v.to_string();
                        problems.push(t!("config-not-integer", name = name, value = value));
                    }
                    whole
                })
                .map(|v| v as u32)
        },
        write: |config| {
            config
                .vim_yank_highlight
                .map(|ms| Json::Number(f64::from(ms)))
        },
    },
    Field {
        key: "timeoutLen",
        read: |config, value, name, problems| {
            let label = t!("config-label-vim-timeout-len");
            let range = Config::VIM_TIMEOUT_LEN_RANGE;
            config.vim_timeout_len = read_number(value, name, range, label, problems)
                .filter(|v| {
                    let whole = v.fract() == 0.0;
                    if !whole {
                        let value = v.to_string();
                        problems.push(t!("config-not-integer", name = name, value = value));
                    }
                    whole
                })
                .map(|v| v as u32)
        },
        write: |config| config.vim_timeout_len.map(|ms| Json::Number(f64::from(ms))),
    },
    Field {
        key: "leader",
        read: |config, value, name, problems| {
            config.vim_leader = match value.as_str() {
                Some(key) if !key.is_empty() => Some(key.to_owned()),
                _ => {
                    problems.push(t!("config-vim-leader-invalid", name = name));
                    None
                }
            }
        },
        write: |config| config.vim_leader.clone().map(Json::String),
    },
    Field {
        // 数组的每一项是一条映射；写法有误的项逐个报告并忽略。
        key: "keys",
        read: |config, value, name, problems| {
            config.vim_keys.clear();
            let Some(items) = value.as_array() else {
                problems.push(t!("config-vim-keys-invalid", name = name));
                return;
            };
            for (index, item) in items.iter().enumerate() {
                let text = |key: &str| {
                    let value = item.as_object()?.iter().find(|(name, _)| *name == key)?.1;
                    value.as_str().filter(|text| !text.is_empty())
                };
                let mode = text("mode").and_then(|mode| {
                    let mut names = VimMapMode::NAMES.iter();
                    names.find(|(_, name)| *name == mode).map(|(mode, _)| *mode)
                });
                match (mode, text("keys"), text("to")) {
                    (Some(mode), Some(keys), Some(to)) => config.vim_keys.push(VimMapping {
                        mode,
                        keys: keys.to_owned(),
                        to: to.to_owned(),
                    }),
                    _ => {
                        let name = format!("{name}[{index}]");
                        problems.push(t!("config-vim-keys-invalid", name = name));
                    }
                }
            }
        },
        write: |config| {
            if config.vim_keys.is_empty() {
                return None;
            }
            let item = |mapping: &VimMapping| {
                let mut names = VimMapMode::NAMES.iter();
                let mode = names
                    .find(|(mode, _)| *mode == mapping.mode)
                    .map_or("normal", |(_, name)| *name);
                Json::object([
                    ("mode".to_owned(), Json::String(mode.to_owned())),
                    ("keys".to_owned(), Json::String(mapping.keys.clone())),
                    ("to".to_owned(), Json::String(mapping.to.clone())),
                ])
            };
            Some(Json::Array(config.vim_keys.iter().map(item).collect()))
        },
    },
];

/// `terminal` 一节。
const TERMINAL: &[Field] = &[
    Field {
        key: "external",
        // 只有空白视为没写，使用系统默认的终端程序，不报告。
        read: |config, value, name, problems| match value.as_str() {
            Some(app) => {
                let app = app.trim();
                config.external_terminal = (!app.is_empty()).then(|| app.to_owned());
            }
            None => problems.push(t!("config-not-string", name = name)),
        },
        write: |config| config.external_terminal.clone().map(Json::String),
    },
    bool_field!("enabled", terminal_enabled),
    string_field!("font", terminal_font),
    Field {
        key: "size",
        read: |config, value, name, problems| {
            let label = t!("config-label-font-size");
            config.terminal_size =
                read_number(value, name, Config::EDITOR_SIZE_RANGE, label, problems)
        },
        write: |config| config.terminal_size.map(number),
    },
    string_field!("shell", terminal_shell),
    bool_field!("shellIntegration", terminal_shell_integration),
    bool_field!("confirmClose", terminal_confirm_close),
    bool_field!("prompt", terminal_prompt),
    bool_field!("copyOnSelect", terminal_copy_on_select),
    bool_field!("boldIsBright", terminal_bold_is_bright),
    Field {
        // `position` 在配置里是字符串，读入后只保留是否在右边。
        key: "position",
        read: |config, value, name, problems| match value.as_str() {
            Some("bottom") => config.terminal_right = Some(false),
            Some("right") => config.terminal_right = Some(true),
            _ => problems.push(t!("config-terminal-position-invalid", name = name)),
        },
        write: |config| {
            let right = config.terminal_right?;
            Some(Json::String(if right { "right" } else { "bottom" }.into()))
        },
    },
    Field {
        key: "scrollback",
        read: |config, value, name, problems| {
            let label = t!("config-label-lines");
            config.terminal_scrollback =
                read_number(value, name, Config::SCROLLBACK_RANGE, label, problems)
                    .map(|n| n.round() as u32);
        },
        write: |config| config.terminal_scrollback.map(|n| number(n as f32)),
    },
    Field {
        key: "minimumContrast",
        read: |config, value, name, problems| {
            let label = t!("config-label-contrast");
            config.terminal_minimum_contrast =
                read_number(value, name, Config::CONTRAST_RANGE, label, problems);
        },
        write: |config| config.terminal_minimum_contrast.map(number),
    },
    Field {
        key: "optionAsMeta",
        read: |config, value, name, problems| {
            config.option_as_meta = match value.as_str() {
                Some("left") => Some(OptionAsMeta::Left),
                Some("right") => Some(OptionAsMeta::Right),
                Some("both") => Some(OptionAsMeta::Both),
                Some("none") => Some(OptionAsMeta::None),
                _ => {
                    problems.push(t!("config-option-as-meta-invalid", name = name));
                    None
                }
            }
        },
        write: |config| {
            config
                .option_as_meta
                .map(|o| Json::String(o.name().to_owned()))
        },
    },
];

/// `git` 一节。
const GIT: &[Field] = &[
    bool_field!("enabled", git_enabled),
    bool_field!("confirmPush", git_confirm_push),
    Field {
        key: "repositoryScanDepth",
        read: |config, value, name, problems| {
            let label = t!("config-label-repository-scan-depth");
            let range = Config::REPOSITORY_SCAN_DEPTH_RANGE;
            config.repository_scan_depth = read_number(value, name, range, label, problems)
                .filter(|v| {
                    let whole = v.fract() == 0.0;
                    if !whole {
                        let value = v.to_string();
                        problems.push(t!("config-not-integer", name = name, value = value));
                    }
                    whole
                })
                .map(|v| v as usize)
        },
        write: |config| {
            config
                .repository_scan_depth
                .map(|depth| Json::Number(depth as f64))
        },
    },
    string_field!("worktreeRoot", git_worktree_root),
    Field {
        key: "worktreeCopy",
        read: |config, value, name, problems| {
            let Some(items) = value.as_array() else {
                problems.push(t!("config-not-array", name = name));
                return;
            };
            for item in items {
                match item.as_str() {
                    Some(glob) if !glob.trim().is_empty() => {
                        config.git_worktree_copy.push(glob.to_owned());
                    }
                    _ => problems.push(t!("config-item-not-non-empty-string", name = name)),
                }
            }
        },
        write: |config| {
            let globs = &config.git_worktree_copy;
            (!globs.is_empty())
                .then(|| Json::Array(globs.iter().cloned().map(Json::String).collect()))
        },
    },
];

/// `format` 一节，用户配置和项目配置共用。
///
/// 语言可以写 id 或状态栏上显示的名字（如 `json`、`JSON`），统一存成 id。
pub(super) const FORMAT: &[Field] = &[
    bool_field!("onSave", format_on_save),
    Field {
        key: "onSaveExclude",
        read: |config, value, name, problems| {
            let Some(items) = value.as_array() else {
                problems.push(t!("config-not-array", name = name));
                return;
            };
            for item in items {
                let Some(language) = item.as_str() else {
                    problems.push(t!("config-item-not-string", name = name));
                    continue;
                };
                if let Some(id) = language_id(language, name, problems)
                    && !config.format_on_save_exclude.contains(&id)
                {
                    config.format_on_save_exclude.push(id);
                }
            }
        },
        write: |config| {
            let ids = &config.format_on_save_exclude;
            (!ids.is_empty()).then(|| Json::Array(ids.iter().cloned().map(Json::String).collect()))
        },
    },
    Field {
        key: "commands",
        read: |config, value, name, problems| {
            read_commands(value, name, &mut config.format_commands, problems)
        },
        write: |config| commands_json(&config.format_commands),
    },
    bool_field!("fixOnSave", fix_on_save),
    Field {
        key: "fixCommands",
        read: |config, value, name, problems| {
            read_commands(value, name, &mut config.fix_commands, problems)
        },
        write: |config| commands_json(&config.fix_commands),
    },
];

/// 在 `fields` 中找键名为 `key` 的一项。
pub(super) fn find<'a>(fields: &'a [Field], key: &str) -> Option<&'a Field> {
    fields.iter().find(|field| field.key == key)
}

/// 顶层的 `key` 是否是认识的设置：[`SECTIONS`] 中某一节的名字，或 [`TOP_LEVEL`] 中的一项。
pub(super) fn is_known(key: &str) -> bool {
    SECTIONS.iter().any(|&(name, _)| name == key) || find(TOP_LEVEL, key).is_some()
}

/// 把名为 `name` 的一节读入 `config`：不是对象，或其中有 `fields` 以外的键时，记入 `problems`。
pub(super) fn read_section(
    config: &mut Config,
    name: &str,
    fields: &[Field],
    section: &Json,
    problems: &mut Vec<String>,
) {
    let Some(values) = section.as_object() else {
        problems.push(t!("config-not-object", name = name));
        return;
    };
    for (key, value) in values {
        match find(fields, key) {
            Some(field) => (field.read)(config, value, &format!("{name}.{key}"), problems),
            None => problems.push(t!("config-unknown-key", key = format!("{name}.{key}"))),
        }
    }
}

/// 把「语言 → 命令」的对象（`format.commands`、`format.fixCommands`）读入 `out`，无法理解的地方
/// 记入 `problems`，文案中用 `name` 指代这一项。
///
/// 命令去掉首尾空白，空字符串视为没写，自动选择工具。两个键指向同一种语言时（如 `JSON` 和
/// `json`），按键名排序后靠后的一条生效。
fn read_commands(
    value: &Json,
    name: &str,
    out: &mut BTreeMap<String, String>,
    problems: &mut Vec<String>,
) {
    let Some(commands) = value.as_object() else {
        problems.push(t!("config-not-object", name = name));
        return;
    };
    for (language, command) in commands {
        let Some(id) = language_id(language, name, problems) else {
            continue;
        };
        let command = match command.as_str().map(str::trim) {
            Some("") => continue,
            Some(command) => command.to_owned(),
            None => {
                problems.push(t!("config-not-string", name = format!("{name}.{language}")));
                continue;
            }
        };
        if out.insert(id, command).is_some() {
            let language = language.as_str();
            problems.push(t!(
                "config-duplicate-language",
                name = name,
                language = language
            ));
        }
    }
}

/// 把「语言 → 命令」写成 JSON 对象，没有命令时返回 `None`，不写出。
fn commands_json(commands: &BTreeMap<String, String>) -> Option<Json> {
    let pairs = commands
        .iter()
        .map(|(id, command)| (id.as_str(), Json::String(command.clone())));
    (!commands.is_empty()).then(|| Json::object(pairs))
}

/// 把用户写的语言（id 或名字）转成语言的 id，无法识别时记入 `problems` 并返回 `None`。
///
/// 文案中用 `name` 指代所在的设置。
fn language_id(language: &str, name: &str, problems: &mut Vec<String>) -> Option<String> {
    let id = winc_syntax::find(language).map(|l| l.id.to_owned());
    if id.is_none() {
        problems.push(t!(
            "config-unknown-language",
            name = name,
            language = language
        ));
    }
    id
}

/// 把 `f32` 转成 JSON 数字，按 `f32` 的最短十进制写法输出：`12.3` 不写成 `12.300000190734863`。
fn number(value: f32) -> Json {
    Json::Number(value.to_string().parse().unwrap_or(f64::from(value)))
}

/// 读取字符串设置，类型不符或只有空白时向 `problems` 记录问题并返回 `None`。
fn read_string(value: &Json, key: &str, problems: &mut Vec<String>) -> Option<String> {
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => Some(text.to_string()),
        Some(_) => {
            problems.push(t!("config-empty", name = key));
            None
        }
        None => {
            problems.push(t!("config-not-string", name = key));
            None
        }
    }
}

/// 读取布尔设置，类型不符时向 `problems` 记录问题并返回 `None`。
fn read_bool(value: &Json, key: &str, problems: &mut Vec<String>) -> Option<bool> {
    let value = value.as_bool();
    if value.is_none() {
        problems.push(t!("config-not-bool", name = key));
    }
    value
}

/// 读取数字设置并检查范围，类型不符或超出 `range` 时向 `problems` 记录问题并返回 `None`。
///
/// 整数写法同样接受（如 `"size": 14`）；`label` 用于问题文案中的名称。
fn read_number(
    value: &Json,
    key: &str,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    problems: &mut Vec<String>,
) -> Option<f32> {
    let Some(number) = value.as_number() else {
        problems.push(t!("config-not-number", name = key));
        return None;
    };

    if !number.is_finite() || !range.contains(&(number as f32)) {
        let setting = format!("{key} {number}");
        let range = format!("{}–{}", range.start(), range.end());
        problems.push(t!(
            "config-out-of-range",
            setting = setting,
            label = label,
            range = range
        ));
        return None;
    }

    Some(number as f32)
}
