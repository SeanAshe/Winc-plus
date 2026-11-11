//! 用户配置和项目配置：读入、校验、写回，以及配置文件的位置。
//!
//! 每一项设置的键名和读写规则由 `fields` 模块给出。

mod fields;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::files::{config_dir, write_atomic};
use crate::json::{self, Json};
use fields::*;
use winc_i18n::{LocaleChoice, t};

/// 用户配置。
///
/// 每一项都是可选的，`None`（集合类型为空）表示用户没写过，使用默认值。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// 编辑区等宽字体，可以是字体名（如 `Menlo`），也可以是字体文件路径。
    pub editor_font: Option<String>,
    /// 编辑区字号（点）。
    ///
    /// 可用范围见 [`Config::EDITOR_SIZE_RANGE`]。
    pub editor_size: Option<f32>,
    /// 界面字体（文件树、标签、状态栏）。
    pub ui_font: Option<String>,
    /// 界面字号（点）。
    ///
    /// 可用范围见 [`Config::UI_SIZE_RANGE`]。
    pub ui_size: Option<f32>,
    /// 界面缩放（百分比）：整个窗口（界面、编辑区和终端）按此比例放大或缩小，叠加在系统的
    /// 显示缩放之上。
    ///
    /// 可用范围见 [`Config::ZOOM_RANGE`]。
    pub ui_zoom: Option<u16>,
    /// 全部文字的字重，默认为 400（各字体自身的字重）；范围见 [`Config::FONT_WEIGHT_RANGE`]。
    pub ui_font_weight: Option<u16>,
    /// 窗口背景图片的路径；没写时没有背景图。
    pub ui_background_image: Option<String>,
    /// 背景图片在底色之上的不透明度（百分比），默认为 15；范围见
    /// [`Config::BACKGROUND_OPACITY_RANGE`]。
    pub ui_background_opacity: Option<f32>,
    /// 背景图片的显示方式，默认为填充。
    pub ui_background_fit: Option<BackgroundFit>,
    /// 是否用系统的标题栏（配置中的 `"titleBarStyle": "native"`）。
    ///
    /// 写成 `"custom"` 或没写时，窗口按钮在 winc 的顶栏中绘制。只影响 macOS 以外的平台。
    pub ui_native_title_bar: Option<bool>,
    /// 是否播放界面的过渡动画（面板、弹出层、列表的变化）。
    ///
    /// 没写时跟随系统的「减弱动态效果」设置；系统没有该设置或未读取时为是。
    pub ui_animations: Option<bool>,
    /// 制表符的显示宽度（空格数）；文件用制表符缩进时，也是每级缩进的宽度。
    ///
    /// 空格缩进的宽度不取此值，按文件内容判断，判断不出时用语言的默认缩进。
    ///
    /// 可用范围见 [`Config::TAB_WIDTH_RANGE`]。
    pub tab_width: Option<usize>,
    /// 是否在切换标签或窗口失去焦点时保存修改，默认为否。
    pub auto_save: Option<bool>,
    /// 热退出：退出时不逐个询问是否保存，未保存的内容在下次启动时恢复，默认为开。
    pub hot_exit: Option<bool>,
    /// 启动时打开上次退出时打开的目录和标签，默认为开。
    ///
    /// 关闭后每次启动都显示开始页；命令行指定了目录或文件时仍然打开。
    pub restore_folder: Option<bool>,
    /// 已经打开了目录的窗口里再打开另一个目录时，`true` 在新窗口打开，`false` 在此窗口打开
    /// （替换当前目录）；没写时每次询问。
    ///
    /// 提示框上勾选「不再询问」后写入当时的选择。
    pub open_folder_in_new_window: Option<bool>,
    /// 滚动时，把已滚出可视区域的函数、类、标题的开头行固定在编辑区顶部，默认为开。
    pub sticky_scroll: Option<bool>,
    /// 自动换行：按编辑区宽度把长行断开显示（不修改文件），默认为否。
    pub word_wrap: Option<bool>,
    /// 超长行显示全部内容，而不是只显示开头的一部分，默认为否。
    pub whole_lines: Option<bool>,
    /// 在编辑区右侧显示缩略图，默认为否。
    pub minimap: Option<bool>,
    /// 在侧栏文件视图的底部显示大纲（当前文件的函数、类型、标题），默认为是。
    pub outline: Option<bool>,
    /// 何时显示空格和制表符，默认为选中时。
    pub render_whitespace: Option<RenderWhitespace>,
    /// 差异单栏显示：新旧内容上下排列，默认为否（左右并排）。
    pub diff_inline: Option<bool>,
    /// 在行号栏显示行号，默认为是；关闭后折叠箭头、书签和改动标记仍然显示。
    pub line_numbers: Option<bool>,
    /// 显示窗口底部的状态栏，默认为是。
    pub status_bar: Option<bool>,
    /// 新窗口沿用上次关闭时的大小和位置，默认为是。
    pub restore_window: Option<bool>,
    /// 配色：`auto` 跟随系统，`dark` 或 `light` 强制指定。
    pub theme: Option<ThemeChoice>,
    /// 浅色外观时用的主题（主题的名字，如 `paper`）；没写时用默认的。
    pub light_theme: Option<String>,
    /// 深色外观时用的主题（主题的名字，如 `ember`）；没写时用默认的。
    pub dark_theme: Option<String>,
    /// 界面语言：`auto` 跟随系统的首选语言，或语言标签（如 `en-US`）指定一种语言。
    pub language: Option<LocaleChoice>,
    /// Markdown 预览是否联网加载网址的图片（README 里的徽章、截图），默认为是。
    pub markdown_remote_images: Option<bool>,
    /// 是否定时检查更新（启动后 5 秒、之后每天一次），默认为是。
    ///
    /// 关闭后仍可从菜单「检查更新…」手动检查。
    pub update_auto_check: Option<bool>,
    /// 在文件树里拖动文件或目录时是否先确认，默认为是。
    ///
    /// 提示框上勾选「不再询问」后写成 `false`。
    pub confirm_drag_and_drop: Option<bool>,
    /// 文件树、Git 改动列表和历史提交的文件列表是否把连续的单子目录合并显示为一行；未设置时
    /// 合并。
    pub compact_folders: Option<bool>,
    /// 在文件树中单击文件时是否在预览标签中打开；未设置时为否。
    pub preview_on_click: Option<bool>,
    /// 「在外部终端中打开」使用的终端程序；未设置时使用系统默认的终端程序。
    pub external_terminal: Option<String>,
    /// 是否开启 Vim 模式，默认为否。
    pub vim_enabled: Option<bool>,
    /// Vim 模式下无名寄存器与系统剪贴板的同步方式；未设置时只有 `"+`、`"*` 使用系统剪贴板。
    pub vim_clipboard: Option<VimClipboard>,
    /// Vim 模式下查找是否忽略大小写，默认为否。
    pub vim_ignore_case: Option<bool>,
    /// Vim 模式下忽略大小写时，模式中有大写字母是否改为区分大小写，默认为否。
    pub vim_smart_case: Option<bool>,
    /// Vim 模式下光标与窗口上下边缘之间保留的行数，默认为 0。
    ///
    /// 可用范围见 [`Config::VIM_SCROLL_OFF_RANGE`]。
    pub vim_scroll_off: Option<usize>,
    /// Vim 模式下按键归属的改写：按键的名字（小写，如 `ctrl+f`）与是否交给 Vim 层处理。
    ///
    /// 按名字排序；没有列出的按键按默认的规则决定归属。
    pub vim_handle_keys: Vec<(String, bool)>,
    /// Vim 模式下是否显示相对行号，默认为否。
    pub vim_relative_number: Option<bool>,
    /// Vim 模式下复制之后高亮被复制范围的毫秒数，默认为 150，0 表示不高亮。
    ///
    /// 可用范围见 [`Config::VIM_YANK_HIGHLIGHT_RANGE`]。
    pub vim_yank_highlight: Option<u32>,
    /// Vim 模式下映射等待后续按键的毫秒数，默认为 1000。
    ///
    /// 可用范围见 [`Config::VIM_TIMEOUT_LEN_RANGE`]。
    pub vim_timeout_len: Option<u32>,
    /// Vim 模式下映射中 `<Leader>` 代表的按键（按键记法），默认为反斜杠。
    pub vim_leader: Option<String>,
    /// Vim 模式下的映射，按配置文件中的顺序。
    pub vim_keys: Vec<VimMapping>,
    /// 是否启用终端模块（编辑区下方的终端面板），默认为否。
    pub terminal_enabled: Option<bool>,
    /// 终端的等宽字体，写法同 [`Config::editor_font`]；未设置时与编辑区相同。
    pub terminal_font: Option<String>,
    /// 终端的字号（点）；未设置时用编辑区的默认字号，不随编辑区字号变化。
    ///
    /// 可用范围同 [`Config::EDITOR_SIZE_RANGE`]。
    pub terminal_size: Option<f32>,
    /// 终端回滚缓冲的行数。
    ///
    /// 可用范围见 [`Config::SCROLLBACK_RANGE`]，默认 10000 行。
    pub terminal_scrollback: Option<u32>,
    /// 终端中视为 Meta（按键前加 `ESC`）的 ⌥ 键，默认为左侧；只在 macOS 上生效。
    pub option_as_meta: Option<OptionAsMeta>,
    /// 终端的对比度保护：字与底色的对比度低于设定值时把字调亮或调暗，0 表示关闭。
    ///
    /// 可用范围见 [`Config::CONTRAST_RANGE`]，默认 3。
    pub terminal_minimum_contrast: Option<f32>,
    /// 终端运行的程序和参数，以空白分隔（如 `wsl.exe -d Ubuntu`）。未设置时运行系统默认的 shell；
    /// 打开的目录位于 WSL 发行版中时进入对应的发行版。
    pub terminal_shell: Option<String>,
    /// 终端是否自动注入 shell 集成（zsh、bash、fish 的命令块），默认为是。
    pub terminal_shell_integration: Option<bool>,
    /// 关闭还有程序在运行的终端标签前是否先确认，默认为是。
    pub terminal_confirm_close: Option<bool>,
    /// 用户自己没设提示符时，终端里换成 winc 的提示符（目录、Git 分支、上一条命令的成败），ls 带
    /// 颜色，zsh 的补全用菜单；默认为是。
    pub terminal_prompt: Option<bool>,
    /// 终端里选中文字（拖选、双击、三击）松开时自动复制，默认为否。
    pub terminal_copy_on_select: Option<bool>,
    /// 终端里把粗体的前 8 种颜色画成对应的亮色，供依赖粗体表示亮色的程序使用，默认为否。
    pub terminal_bold_is_bright: Option<bool>,
    /// 终端面板是否放在编辑区右边（配置里的 `"position": "right"`）；否则（`"bottom"` 或
    /// 没写）放在下面。
    pub terminal_right: Option<bool>,
    /// 是否启用 Git 模块（文件状态、分支、变更视图），默认为否。
    pub git_enabled: Option<bool>,
    /// 发布、推送或同步分支前是否先确认目标远程仓库，默认为是。
    ///
    /// 提示框上勾选「不再询问」后写成 `false`，可在设置中重新打开。
    pub git_confirm_push: Option<bool>,
    /// 在打开的目录下查找仓库的层数，默认见 [`Config::REPOSITORY_SCAN_DEPTH`]，可用范围见
    /// [`Config::REPOSITORY_SCAN_DEPTH_RANGE`]。
    pub repository_scan_depth: Option<usize>,
    /// 新建工作树的存放目录，按仓库分开：`<目录>/<仓库名>/<分支名>`。
    ///
    /// 开头的 `~/` 指仓库所在系统（本机或 WSL 发行版）中用户的主目录，相对路径相对于主工作树。
    /// 未设置，或者该目录与仓库不在同一系统中时，存放在仓库旁边的 `<仓库名>.worktrees` 中。
    pub git_worktree_root: Option<String>,
    /// 新建工作树时复制过去的被忽略的文件（gitignore 写法的 glob，如 `.env*`），默认不复制。
    pub git_worktree_copy: Vec<String>,
    /// 保存时先格式化，默认为否。
    pub format_on_save: Option<bool>,
    /// 保存时不格式化的语言（语言的 id）；手动格式化（`⇧⌥F`）仍然生效。
    pub format_on_save_exclude: Vec<String>,
    /// 各语言的格式化命令，键是语言的 id。
    ///
    /// 没写的语言自动选择工具。
    pub format_commands: BTreeMap<String, String>,
    /// 保存时先修复（在格式化之前），默认为否。
    pub fix_on_save: Option<bool>,
    /// 各语言的修复命令，键是语言的 id。
    ///
    /// 没写的语言自动选择工具。
    pub fix_commands: BTreeMap<String, String>,
}

/// 终端里哪一边的 ⌥ 当 Meta 用；另一边仍然输入 `å`、`∑` 这类字符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionAsMeta {
    /// 左边的 ⌥。
    Left,
    /// 右边的 ⌥。
    Right,
    /// 两边都是。
    Both,
    /// 两边都不当 Meta 用。
    None,
}

impl OptionAsMeta {
    /// 全部选项，顺序与设置中菜单的顺序一致。
    pub const ALL: [OptionAsMeta; 4] = [
        OptionAsMeta::Left,
        OptionAsMeta::Right,
        OptionAsMeta::Both,
        OptionAsMeta::None,
    ];

    /// 返回该变体在配置文件里的写法。
    pub fn name(self) -> &'static str {
        match self {
            OptionAsMeta::Left => "left",
            OptionAsMeta::Right => "right",
            OptionAsMeta::Both => "both",
            OptionAsMeta::None => "none",
        }
    }
}

/// Vim 模式下无名寄存器与系统剪贴板的同步方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimClipboard {
    /// 只有 `"+`、`"*` 使用系统剪贴板。
    #[default]
    Register,
    /// 复制时同时写入系统剪贴板。
    Yank,
    /// 无名寄存器与系统剪贴板同步。
    Always,
}

impl VimClipboard {
    /// 全部取值，按设置菜单中的顺序排列。
    pub const ALL: [Self; 3] = [Self::Register, Self::Yank, Self::Always];
}

/// Vim 模式下映射生效的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMapMode {
    /// 普通模式。
    Normal,
    /// 可视模式。
    Visual,
    /// 插入模式。
    Insert,
    /// 已输入操作符、等待动作时。
    Operator,
}

impl VimMapMode {
    /// 各模式及其在配置文件中的写法。
    pub const NAMES: [(Self, &'static str); 4] = [
        (Self::Normal, "normal"),
        (Self::Visual, "visual"),
        (Self::Insert, "insert"),
        (Self::Operator, "operator"),
    ];
}

/// Vim 模式下的一条映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VimMapping {
    /// 生效的模式。
    pub mode: VimMapMode,
    /// 左侧的按键，按键记法。
    pub keys: String,
    /// 右侧：按键记法，或命令的英文名。
    pub to: String,
}

/// 何时显示空格和制表符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderWhitespace {
    /// 不显示。
    None,
    /// 只显示选中文字里的。
    #[default]
    Selection,
    /// 只显示行尾的。
    Trailing,
    /// 全部显示。
    All,
}

impl RenderWhitespace {
    /// 全部选项，顺序与设置中菜单的顺序一致。
    pub const ALL: [RenderWhitespace; 4] = [
        RenderWhitespace::None,
        RenderWhitespace::Selection,
        RenderWhitespace::Trailing,
        RenderWhitespace::All,
    ];
}

/// 背景图在窗口中的显示方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundFit {
    /// 等比缩放并裁剪，铺满窗口。
    #[default]
    Fill,
    /// 等比缩放到整张可见。
    Fit,
    /// 拉伸到窗口的大小。
    Stretch,
    /// 按原始大小平铺。
    Tile,
}

impl BackgroundFit {
    /// 全部选项，顺序与设置中菜单的顺序一致。
    pub const ALL: [BackgroundFit; 4] = [
        BackgroundFit::Fill,
        BackgroundFit::Fit,
        BackgroundFit::Stretch,
        BackgroundFit::Tile,
    ];
}

/// 配色方案的选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    /// 跟随系统的浅色或深色外观。
    Auto,
    /// 强制使用深色配色。
    Dark,
    /// 强制使用浅色配色。
    Light,
}

impl Config {
    /// 编辑区字号的合法范围（点）。
    ///
    /// 超出范围的值在读入时忽略并报告；上下限为经验值。
    pub const EDITOR_SIZE_RANGE: std::ops::RangeInclusive<f32> = 8.0..=32.0;
    /// 界面字号的合法范围（点）。
    ///
    /// 超出范围的值在读入时忽略并报告；上下限为经验值。
    pub const UI_SIZE_RANGE: std::ops::RangeInclusive<f32> = 8.0..=24.0;
    /// 界面缩放的合法范围（百分比）。
    ///
    /// 下限时 13 点的界面文字约为 9 点，仍可辨认；上限时 4K 屏幕在系统缩放为 100% 的情况下，
    /// 界面与 1080p 屏幕上的原始大小相同。超出范围的值在读入时忽略并报告，小数取整。
    pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 70.0..=200.0;
    /// 字重的合法范围；超出的值在读入时忽略并报告，小数取整。
    pub const FONT_WEIGHT_RANGE: std::ops::RangeInclusive<f32> = 100.0..=900.0;
    /// 默认的字重。
    pub const FONT_WEIGHT: u16 = 400;
    /// 背景图片不透明度的合法范围（百分比）：0 时不显示图片，100 时图片完全盖住底色。
    ///
    /// 超出范围的值在读入时忽略并报告。
    pub const BACKGROUND_OPACITY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=100.0;
    /// 背景图片不透明度的默认值（百分比）。
    pub const BACKGROUND_OPACITY: f32 = 15.0;
    /// 制表符长度的合法范围（空格数）。
    ///
    /// 超出范围的值在读入时忽略并报告；上下限为经验值。
    pub const TAB_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 1.0..=16.0;
    /// Vim 模式下光标与窗口上下边缘之间保留行数的可用范围（行）。
    ///
    /// 超出范围的值在读入时忽略并报告；上限取自调参，超过窗口高度一半的值效果相同。
    pub const VIM_SCROLL_OFF_RANGE: std::ops::RangeInclusive<f32> = 0.0..=999.0;
    /// Vim 模式下复制之后高亮时间的可用范围（毫秒）。
    ///
    /// 超出范围的值在读入时忽略并报告；上限取自调参，更长的高亮妨碍阅读。
    pub const VIM_YANK_HIGHLIGHT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=5_000.0;
    /// Vim 模式下映射等待后续按键的时间的可用范围（毫秒）。
    ///
    /// 超出范围的值在读入时忽略并报告；上限取自调参，更长的等待与不超时无异。
    pub const VIM_TIMEOUT_LEN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=60_000.0;
    /// 查找仓库的默认层数：覆盖「目录 / 分组 / 项目」这样的常见结构；扫描在后台进行，有目录项总数
    /// 的上限，较深的层数不会拖慢界面。
    pub const REPOSITORY_SCAN_DEPTH: usize = 3;
    /// 查找仓库层数的合法范围。
    ///
    /// 超出范围的值在读入时忽略并报告。
    pub const REPOSITORY_SCAN_DEPTH_RANGE: std::ops::RangeInclusive<f32> = 1.0..=8.0;
    /// 终端回滚缓冲行数的合法范围。
    ///
    /// 超出范围的值在读入时忽略并报告；上限 1000000 行为经验值，避免回滚缓冲占用过多内存。
    pub const SCROLLBACK_RANGE: std::ops::RangeInclusive<f32> = 0.0..=1_000_000.0;
    /// 终端最低对比度的合法范围（WCAG 对比度，1 和 0 都表示不保护）。
    ///
    /// 上限 21 是 WCAG 对比度的最大值，即黑底白字之比。
    pub const CONTRAST_RANGE: std::ops::RangeInclusive<f32> = 0.0..=21.0;

    /// 读取用户级配置，返回配置和读取过程中发现的问题（可能有多条）。
    ///
    /// 配置文件不存在或无法确定配置路径时返回默认值；内容无法理解的部分同样退回默认值，
    /// 原因写入问题列表，由调用方提示用户。
    pub fn load() -> (Config, Vec<String>) {
        match config_path() {
            Some(path) => Config::load_from(&path),
            None => (Config::default(), Vec::new()),
        }
    }

    /// 读取指定文件中的配置，返回值含义同 [`Config::load`]。
    ///
    /// 文件不存在时返回默认值，不报告问题。
    pub fn load_from(path: &Path) -> (Config, Vec<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
            Err(e) => (
                Config::default(),
                vec![t!("config-read-failed", error = e.to_string())],
            ),
        }
    }

    /// 解析配置文件内容，返回设置和所有无法理解的地方；任何输入都不会 panic。
    ///
    /// 内容按 JSONC 解析：允许行注释、块注释、开头的 BOM，以及最后一项后面的逗号。
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut config = Config::default();
        let mut problems = Vec::new();

        let Some((root, _)) = Json::parse_jsonc(text) else {
            problems.push(t!("config-invalid-json").to_owned());
            return (config, problems);
        };

        let Some(object) = root.as_object() else {
            problems.push(t!("config-root-not-object").to_owned());
            return (config, problems);
        };

        for &(name, fields) in SECTIONS {
            if let Some(section) = object.get(name) {
                read_section(&mut config, name, fields, section, &mut problems);
            }
        }
        for field in TOP_LEVEL {
            if let Some(value) = object.get(field.key) {
                (field.read)(&mut config, value, field.key, &mut problems);
            }
        }
        for key in object.keys().filter(|key| !is_known(key)) {
            problems.push(t!("config-unknown-key", key = key.as_str()));
        }

        (config, problems)
    }

    /// 序列化为缩进 2 个空格的 JSON，只写出有值的项。
    ///
    /// 各节和各项按固定顺序写出，按严格 JSON 编码，不写注释。
    pub fn to_json(&self) -> String {
        self.render(None)
    }

    /// 序列化为缩进 2 个空格的 JSON：各节和各项按 [`SECTIONS`]、[`TOP_LEVEL`] 的顺序写出，只写出
    /// 有值的项。
    ///
    /// `original` 是磁盘上原有配置的顶层对象和上次读入或写出该文件时的 `base`。给出时：
    ///
    /// - 相对 `base` 修改过的项写 `self` 中的值，`self` 中没有值的项不写出。
    /// - 没有修改的项和节按原文件写出，包括读入时报告并忽略的值（例如超出范围的值）。
    /// - 不认识的键原样保留：节内的键写在该节已知的项之后，顶层的键写在最后。
    fn render(&self, original: Option<(&BTreeMap<String, Json>, &Config)>) -> String {
        /// 顶层的一项：按固定顺序写出的一节，或单个值。
        enum Entry<'a> {
            Section(Vec<(&'a str, Json)>),
            Value(Json),
        }

        let changed = |field: &Field| {
            original.is_none_or(|(_, base)| (field.write)(self) != (field.write)(base))
        };
        // 一项要写出的值：修改过的项取 `self` 中的值，没有修改的项取原文件同一对象（`old`）中的值。
        let value = |field: &Field, old: Option<&BTreeMap<String, Json>>| {
            if changed(field) {
                (field.write)(self)
            } else {
                old?.get(field.key).cloned()
            }
        };
        let root = original.map(|(root, _)| root);
        let mut entries = Vec::new();
        for &(name, fields) in SECTIONS {
            let old = root.and_then(|root| root.get(name));
            if let Some(old) = old.filter(|old| old.as_object().is_none())
                && !fields.iter().any(changed)
            {
                // 原文件中这一节不是对象（读入时已报告并忽略），节内的项也没有修改：原样保留。
                entries.push((name, Entry::Value(old.clone())));
                continue;
            }
            let old = old.and_then(Json::as_object);
            let mut items: Vec<(&str, Json)> = fields
                .iter()
                .filter_map(|field| Some((field.key, value(field, old)?)))
                .collect();
            if let Some(old) = old {
                let unknown = old.iter().filter(|(key, _)| find(fields, key).is_none());
                items.extend(unknown.map(|(key, value)| (key.as_str(), value.clone())));
            }
            if !items.is_empty() {
                entries.push((name, Entry::Section(items)));
            }
        }
        for field in TOP_LEVEL {
            if let Some(value) = value(field, root) {
                entries.push((field.key, Entry::Value(value)));
            }
        }
        if let Some(original) = root {
            let unknown = original.iter().filter(|(key, _)| !is_known(key));
            entries.extend(unknown.map(|(key, value)| (key.as_str(), Entry::Value(value.clone()))));
        }

        let mut out = String::new();
        json::write_object(&mut out, 0, entries, |out, entry, indent| match entry {
            Entry::Section(items) => {
                let pairs = items.iter().map(|(key, value)| (*key, value));
                json::write_object(out, indent, pairs, |out, v, indent| {
                    v.write_into(out, indent)
                });
            }
            Entry::Value(value) => value.write_into(out, indent),
        });
        out.push('\n');
        out
    }

    /// 读取项目配置（`root` 下的 `.winc/config.json`），只认 `format` 一节，其他内容报告后忽略。
    ///
    /// 文件不存在时返回 `None`；无法读取或无法理解的部分与用户配置相同，退回默认值并报告。
    pub fn load_project(root: &Path) -> Option<(Config, Vec<String>)> {
        let text = match std::fs::read_to_string(project_config_path(root)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                let problem = t!("config-project-read-failed", error = e.to_string());
                return Some((Config::default(), vec![problem]));
            }
        };
        Some(Config::parse_project(&text))
    }

    /// 解析项目配置的内容，规则见 [`Config::load_project`]。
    pub(crate) fn parse_project(text: &str) -> (Config, Vec<String>) {
        let mut config = Config::default();
        let mut problems = Vec::new();
        let root = Json::parse_jsonc(text);
        let Some(object) = root.as_ref().and_then(|(root, _)| root.as_object()) else {
            problems.push(t!("config-project-invalid-json").to_owned());
            return (config, problems);
        };
        for (key, value) in object {
            if key == "format" {
                read_section(&mut config, "format", FORMAT, value, &mut problems);
            } else {
                problems.push(t!("config-project-unsupported-key", key = key.as_str()));
            }
        }
        (config, problems)
    }

    /// 把相对 `base`（上次读入或写出该文件时的配置）修改过的项写入指定文件，目录不存在时先创建。
    ///
    /// 其余内容按文件当前的内容保留，其他位置在程序运行期间手动修改的设置不会被覆盖。原文件无法
    /// 解析或含有注释时，先原样另存为同目录下的 `config.json.bak`，再写出 `self` 的全部设置。
    ///
    /// # Errors
    ///
    /// 原文件无法读取、备份失败、目录创建失败、没有写权限或磁盘错误时返回 `std::io::Error`。
    pub fn save_to(&self, path: &Path, base: &Config) -> std::io::Result<Saved> {
        let old = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let root = if old.trim_ascii().is_empty() {
            Some((Json::Object(BTreeMap::new()), false))
        } else {
            std::str::from_utf8(&old).ok().and_then(Json::parse_jsonc)
        };
        let original = root.as_ref().and_then(|(root, _)| root.as_object());
        let comments = root.as_ref().is_some_and(|&(_, comments)| comments);

        let mut backup = None;
        if original.is_none() || comments {
            let path = path.with_extension("json.bak");
            std::fs::write(&path, &old)?;
            backup = Some(Backup { path, comments });
        }
        let text = self.render(original.map(|root| (root, base)));
        write_atomic(path, &text)?;
        Ok(Saved {
            config: Config::parse(&text).0,
            backup,
        })
    }
}

/// [`Config::save_to`] 的结果。
#[derive(Debug, PartialEq)]
pub struct Saved {
    /// 写入后文件中的配置：修改过的项是新值，其余项取自文件，可能刚在其他位置被手动修改过。
    ///
    /// 调用方改用写入后的配置，并以它作为下次写回时的 `base`。
    pub config: Config,
    /// 写入前另存的原文件：位置和原因；没有另存时为 `None`。
    pub backup: Option<Backup>,
}

/// 写回之前另存的原文件（同目录下的 `config.json.bak`）。
#[derive(Debug, PartialEq)]
pub struct Backup {
    /// 另存的位置。
    pub path: PathBuf,
    /// 原文件可以解析但含有注释（写出的文件不保留注释）时为真；原文件无法解析时为假。
    pub comments: bool,
}

/// 返回用户级配置文件的路径；无法确定用户目录时返回 `None`。
///
/// - macOS：`~/Library/Application Support/winc/config.json`
/// - Linux：`$XDG_CONFIG_HOME/winc/config.json`，未设置时为 `~/.config/winc/config.json`
/// - Windows：`%APPDATA%\winc\config.json`，未设置时使用 `HOME` 环境变量指向的目录；便携版放在
///   便携数据目录中（见 [`crate::files::portable_dir`]）
///
/// 不读取系统级配置（如 `/etc/winc/config.json`）：只支持用户级配置。
pub fn config_path() -> Option<PathBuf> {
    let dir = config_dir()?;
    Some(dir.join("config.json"))
}

/// 返回项目配置文件的路径：`root`（打开的目录）下的 `.winc/config.json`。
pub(crate) fn project_config_path(root: &Path) -> PathBuf {
    root.join(".winc").join("config.json")
}

#[cfg(test)]
mod tests;
