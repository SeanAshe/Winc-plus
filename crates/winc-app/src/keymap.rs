//! 键位表：每个命令在三个系统上的快捷键。
//!
//! 按键处理、命令面板、右键菜单、欢迎页和设置中的「快捷键」页
//! 都从同一张键位表读取快捷键。
//!
//! 一个命令可以对应多个键，每个键在表中占一条 [`Binding`]。
//! 两段组合键（如 `⌘K ⌘S`）在按下第一段后返回 [`Lookup::Chord`]。

use winc_i18n::t;
use winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey, PhysicalKey};

mod action;
mod page;
mod table;
#[cfg(test)]
mod tests;

pub use action::*;
pub use page::*;
pub use table::*;

/// 操作系统，决定快捷键的默认键位和显示写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// macOS。
    Mac,
    /// Windows。
    Windows,
    /// Linux 及其他类 Unix 系统。
    Linux,
}

impl Os {
    /// 当前编译目标对应的系统。
    pub const CURRENT: Os = if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };
}

/// 修饰键的位集合。
///
/// `META` 在 macOS 上是 ⌘，在其他系统上是 Win / Super 键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods(u8);

impl Mods {
    /// 没有修饰键。
    pub const NONE: Mods = Mods(0);
    /// Ctrl 键（macOS 上的 ⌃）。
    pub const CTRL: Mods = Mods(1);
    /// Shift 键。
    pub const SHIFT: Mods = Mods(2);
    /// Alt 键（macOS 上的 ⌥）。
    pub const ALT: Mods = Mods(4);
    /// macOS 上的 ⌘，其他系统上的 Win / Super 键。
    ///
    /// 置位为 `8`（`1 << 3`），与其余修饰键的位互不重叠。
    pub const META: Mods = Mods(8);

    /// 将 `winit` 的修饰键状态转换为 [`Mods`]。
    pub fn from_winit(m: ModifiersState) -> Mods {
        let mut mods = Mods::NONE;
        for (on, bit) in [
            (m.control_key(), Mods::CTRL),
            (m.shift_key(), Mods::SHIFT),
            (m.alt_key(), Mods::ALT),
            (m.super_key(), Mods::META),
        ] {
            if on {
                mods = mods | bit;
            }
        }
        mods
    }

    /// 是否包含 `other` 中的全部修饰键。
    pub(crate) fn has(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }
}

/// 不含修饰键的键。
///
/// 字面字母为小写，符号取不按 Shift 时的字符
/// （如 `=` 表示 Shift 加 `=`，而不是 `+`）。
///
/// 没有参数的变体对应同名的非字符键（回车、退格、方向键等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    /// 字符键，字母为小写。
    Char(char),
    /// 小键盘上的键。
    Numpad(char),
    /// 功能键，参数为 1 到 12。
    F(u8),
    Enter,
    Backspace,
    Delete,
    Tab,
    Space,
    Escape,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
}

/// 返回物理键 `key` 在美式键盘上的字符（字母为小写）；不是字母、数字或标点键时返回 `None`。
pub fn us_char(key: PhysicalKey) -> Option<char> {
    use winit::keyboard::KeyCode as K;
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    const LETTERS: [K; 26] = [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
    ];
    const DIGITS: [K; 10] = [
        K::Digit0,
        K::Digit1,
        K::Digit2,
        K::Digit3,
        K::Digit4,
        K::Digit5,
        K::Digit6,
        K::Digit7,
        K::Digit8,
        K::Digit9,
    ];
    if let Some(i) = LETTERS.iter().position(|&k| k == code) {
        return Some(char::from(b'a' + i as u8));
    }
    if let Some(i) = DIGITS.iter().position(|&k| k == code) {
        return Some(char::from(b'0' + i as u8));
    }
    Some(match code {
        K::Minus => '-',
        K::Equal => '=',
        K::BracketLeft => '[',
        K::BracketRight => ']',
        K::Backslash => '\\',
        K::Semicolon => ';',
        K::Quote => '\'',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        K::Backquote => '`',
        _ => return None,
    })
}

/// 一次按键：一个键加上修饰键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stroke {
    /// 不含修饰键的键。
    pub key: KeyCode,
    /// 按下的修饰键。
    pub mods: Mods,
}

impl Stroke {
    /// 将一次按键事件转换为 `Stroke`。
    ///
    /// `base` 是按下该键时输入的字符。单独按修饰键、`base` 含多个字符
    /// 或键无法识别时返回 `None`。
    ///
    /// 输入法接管的按键不携带字符，按住 Ctrl 时按物理键 `physical` 取字符，使快捷键仍然生效；
    /// 没按 Ctrl 时是在打字，返回 `None`。
    pub fn from_event(
        key: &Key,
        base: &str,
        physical: PhysicalKey,
        location: KeyLocation,
        m: ModifiersState,
    ) -> Option<Stroke> {
        let code = match key {
            Key::Named(named) => match named {
                NamedKey::Enter => KeyCode::Enter,
                NamedKey::Backspace => KeyCode::Backspace,
                NamedKey::Delete => KeyCode::Delete,
                NamedKey::Tab => KeyCode::Tab,
                NamedKey::Space => KeyCode::Space,
                NamedKey::Escape => KeyCode::Escape,
                NamedKey::ArrowUp => KeyCode::Up,
                NamedKey::ArrowDown => KeyCode::Down,
                NamedKey::ArrowLeft => KeyCode::Left,
                NamedKey::ArrowRight => KeyCode::Right,
                NamedKey::PageUp => KeyCode::PageUp,
                NamedKey::PageDown => KeyCode::PageDown,
                NamedKey::Process if m.control_key() => KeyCode::Char(us_char(physical)?),
                named => {
                    let n = function_key(named)?;
                    KeyCode::F(n)
                }
            },
            _ => {
                let mut chars = base.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                match c {
                    ' ' => KeyCode::Space,
                    c if location == KeyLocation::Numpad => KeyCode::Numpad(c),
                    c => KeyCode::Char(c.to_ascii_lowercase()),
                }
            }
        };
        Some(Stroke {
            key: code,
            mods: Mods::from_winit(m),
        })
    }

    /// 返回按键的显示文字。
    ///
    /// macOS 上是 `⌥⇧⌘F`，其他系统上是 `Shift+Alt+F`。
    pub fn label(self, os: Os) -> String {
        let key = match self.key {
            KeyCode::Char(c) => c.to_uppercase().collect(),
            KeyCode::Numpad(c) => format!("NumPad{c}"),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Enter if os == Os::Mac => "↩".into(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Backspace if os == Os::Mac => "⌫".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete if os == Os::Mac => "⌦".into(),
            KeyCode::Delete => "Delete".into(),
            KeyCode::Tab if os == Os::Mac => "⇥".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::Space => "Space".into(),
            KeyCode::Escape if os == Os::Mac => "⎋".into(),
            KeyCode::Escape => "Esc".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::PageUp => "PageUp".into(),
            KeyCode::PageDown => "PageDown".into(),
        };
        let m = self.mods;
        if os == Os::Mac {
            // macOS 上修饰键的显示顺序：⌃⌥⇧⌘。
            let mut text = String::new();
            for (bit, symbol) in [
                (Mods::CTRL, '⌃'),
                (Mods::ALT, '⌥'),
                (Mods::SHIFT, '⇧'),
                (Mods::META, '⌘'),
            ] {
                if m.has(bit) {
                    text.push(symbol);
                }
            }
            text + &key
        } else {
            let meta = if os == Os::Windows { "Win" } else { "Super" };
            let mut parts: Vec<&str> = [
                (Mods::CTRL, "Ctrl"),
                (Mods::SHIFT, "Shift"),
                (Mods::ALT, "Alt"),
                (Mods::META, meta),
            ]
            .into_iter()
            .filter(|&(bit, _)| m.has(bit))
            .map(|(_, name)| name)
            .collect();
            parts.push(&key);
            parts.join("+")
        }
    }
}

fn function_key(named: &NamedKey) -> Option<u8> {
    use NamedKey::*;
    let keys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    keys.iter().position(|k| k == named).map(|i| i as u8 + 1)
}

/// 焦点所在的区域，绑定的生效范围按区域划分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// 编辑区。
    Editor,
    /// 查找条的查找输入。
    Find,
    /// 查找条的替换输入。
    Replace,
    /// 侧栏的搜索面板（输入和结果）。
    Panel,
    /// 文件树。
    Tree,
    /// 快速打开、跳转条、设置等其他位置。
    Other,
    /// 设置中「快捷键」页的搜索框。
    Keys,
    /// 终端面板。除了带 ⌘ 的快捷键，按键都交给终端里的程序。
    Terminal,
    /// 可编辑的 Markdown 预览。编辑区的绑定在这里同样生效，另有行内格式的绑定。
    Markdown,
}

impl Context {
    /// 该区域在 [`When`] 的生效范围掩码中占用的位。
    ///
    /// 各区域占用的位互不重叠，按位与可判断覆盖关系。
    const fn bit(self) -> u16 {
        match self {
            Context::Editor => 1,
            Context::Find => 2,
            Context::Panel => 4,
            Context::Tree => 8,
            Context::Other => 16,
            Context::Replace => 32,
            Context::Keys => 64,
            Context::Terminal => 128,
            Context::Markdown => 256,
        }
    }
}

/// 一条绑定的生效范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum When {
    /// 所有区域；终端里只认带 ⌘ 且 [`Action::reaches_terminal`] 为真的命令。
    Global,
    /// 编辑区。
    Editor,
    /// 可编辑的 Markdown 预览：范围小于 [`When::Editor`]，同一个键优先执行这里的绑定。
    Markdown,
    /// 编辑区和查找条（含替换输入）。
    EditorOrFind,
    /// 查找条的替换输入里。
    ReplaceInput,
    /// 侧栏的搜索面板。
    Panel,
    /// 文件树。
    Tree,
    /// 「快捷键」页的搜索框里。
    KeysPage,
    /// 焦点既不在编辑区也不在查找条的区域（文件树、搜索面板等）。
    ///
    /// 查找条与编辑区属于同一范围。
    NotEditor,
    /// 终端面板里。
    Terminal,
    /// 所有区域。
    ///
    /// 终端里的 ⌃ 组合也优先按这一条处理（显示或隐藏终端）。
    Everywhere,
}

impl When {
    fn mask(self) -> u16 {
        const MARKDOWN: u16 = Context::Markdown.bit();
        // 可编辑的 Markdown 预览属于编辑区：覆盖编辑区的范围都覆盖该预览。
        const EDITOR: u16 = Context::Editor.bit() | MARKDOWN;
        const FIND: u16 = Context::Find.bit();
        const PANEL: u16 = Context::Panel.bit();
        const TREE: u16 = Context::Tree.bit();
        const OTHER: u16 = Context::Other.bit();
        const REPLACE: u16 = Context::Replace.bit();
        const KEYS: u16 = Context::Keys.bit();
        const TERMINAL: u16 = Context::Terminal.bit();
        match self {
            // 终端里只认带 ⌘ 的全局快捷键，其余按键交给终端里的程序；
            // 判断条件在 `lookup` 中。
            When::Global => EDITOR | FIND | REPLACE | PANEL | TREE | OTHER | TERMINAL,
            When::Editor => EDITOR,
            When::Markdown => MARKDOWN,
            When::EditorOrFind => EDITOR | FIND | REPLACE,
            When::ReplaceInput => REPLACE,
            When::Panel => PANEL,
            When::Tree => TREE,
            When::KeysPage => KEYS,
            When::NotEditor => PANEL | TREE | OTHER,
            When::Terminal => TERMINAL,
            When::Everywhere => EDITOR | FIND | REPLACE | PANEL | TREE | OTHER | TERMINAL,
        }
    }

    /// 是否覆盖焦点所在的区域 `context`。
    pub fn covers(self, context: Context) -> bool {
        self.mask() & context.bit() != 0
    }

    /// 返回「快捷键」页中命令名后标注的生效范围。
    ///
    /// 在所有区域都生效的范围不标注，返回 `None`。
    pub fn note(self) -> Option<&'static str> {
        match self {
            When::Global => None,
            When::Editor => Some(t!("cmd-when-editor")),
            When::Markdown => Some(t!("cmd-when-markdown")),
            When::EditorOrFind => Some(t!("cmd-when-editor-or-find")),
            When::ReplaceInput => Some(t!("cmd-when-replace-input")),
            When::Panel => Some(t!("cmd-when-panel")),
            When::Tree => Some(t!("cmd-when-tree")),
            When::KeysPage => Some(t!("cmd-when-keys-page")),
            When::NotEditor => Some(t!("cmd-when-not-editor")),
            When::Terminal => Some(t!("cmd-when-terminal")),
            When::Everywhere => None,
        }
    }
}

/// 「快捷键」页的分组，顺序与页面中从上到下一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// 常用。
    General,
    /// 文件。
    File,
    /// 编辑。
    Edit,
    /// 查找。
    Find,
    /// 文件树。
    Tree,
    /// 视图。
    View,
}

impl Group {
    /// 全部分组，顺序与页面中从上到下一致。
    pub const ALL: [Group; 6] = [
        Group::General,
        Group::File,
        Group::Edit,
        Group::Find,
        Group::Tree,
        Group::View,
    ];

    /// 组名是否已说明生效区域（编辑、查找、文件树）。
    ///
    /// 组名已说明生效区域时，组内命令不再标注生效范围。
    fn implies_scope(self) -> bool {
        matches!(self, Group::Edit | Group::Find | Group::Tree)
    }

    /// 返回分组的标题。
    pub fn title(self) -> &'static str {
        match self {
            Group::General => t!("cmd-group-general"),
            Group::File => t!("cmd-group-file"),
            Group::Edit => t!("cmd-group-edit"),
            Group::Find => t!("cmd-group-find"),
            Group::Tree => t!("cmd-group-tree"),
            Group::View => t!("cmd-group-view"),
        }
    }
}

/// 返回当前编译目标系统的绑定表，首次调用时构建并缓存。
pub fn current() -> &'static [Binding] {
    static TABLE: std::sync::OnceLock<Vec<Binding>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| bindings(Os::CURRENT))
}

/// 一次按键查找的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// 可执行的命令，按优先级排序。
    ///
    /// 前一个当前无法执行（例如查找条未打开）时尝试下一个。
    Run(Vec<Action>),
    /// 两段组合键的第一段，正在等待第二段。
    Chord,
    /// 不是快捷键，交回焦点所在区域处理。
    None,
}

/// 返回在 `context` 中按下 `stroke` 的查找结果。
///
/// `pending` 是此前已按下的组合键第一段。
pub fn lookup(
    table: &[Binding],
    stroke: Stroke,
    pending: Option<Stroke>,
    context: Context,
) -> Lookup {
    // 焦点在终端时，全局快捷键中只有带 ⌘ 的（macOS）或带 Ctrl+Shift 的
    // （其他系统）不交给终端程序，其余组合留给终端程序使用；
    // 单按 Ctrl 的组合由终端程序处理。
    let reaches = |mods: Mods| {
        mods.has(Mods::META) || !cfg!(target_os = "macos") && mods.has(Mods::CTRL | Mods::SHIFT)
    };
    let applies = |b: &&Binding| {
        b.when.covers(context)
            && (context != Context::Terminal
                || b.when != When::Global
                || reaches(b.first.mods) && b.action.reaches_terminal())
    };
    match pending {
        Some(first) => Lookup::Run(
            table
                .iter()
                .filter(applies)
                .filter(|b| b.first == first && b.second == Some(stroke))
                .map(|b| b.action)
                .collect(),
        ),
        None => {
            let run: Vec<Action> = table
                .iter()
                .filter(applies)
                .filter(|b| b.first == stroke && b.second.is_none())
                .map(|b| b.action)
                .collect();
            if !run.is_empty() {
                Lookup::Run(run)
            } else if table
                .iter()
                .filter(applies)
                .any(|b| b.first == stroke && b.second.is_some())
            {
                Lookup::Chord
            } else {
                Lookup::None
            }
        }
    }
}

/// 返回 `action` 在当前系统上的第一个键的显示文字。
///
/// 显示文字供菜单、欢迎页和命令面板使用。
///
/// `action` 没有绑定时返回 `None`。
pub fn shortcut(action: Action) -> Option<String> {
    current()
        .iter()
        .find(|b| b.action == action)
        .map(|b| b.label(Os::CURRENT))
}

/// 返回 `action` 在 `context` 中可用的单段按键（不含组合键）。
///
/// 结果供右键菜单显示。
pub fn single_stroke(action: Action, context: Context) -> Option<Stroke> {
    current()
        .iter()
        .find(|b| b.action == action && b.second.is_none() && b.when.covers(context))
        .map(|b| b.first)
}
