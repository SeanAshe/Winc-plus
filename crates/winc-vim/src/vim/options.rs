//! `:set`：选项的读取与设置。
//!
//! 行号、换行、缩进等选项由宿主保存；查找、滚动、映射等待时间等是 Vim 层自己的选项，保存在
//! [`Shared`] 中，只在本次运行中生效。

use super::{Failed, Shared, Vim};
use crate::host::{Host, HostOption, Message};
use crate::register::Clipboard;

/// Vim 层自己的选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// 是否高亮查找的匹配。
    pub hlsearch: bool,
    /// 输入查找的模式时是否即时跳转。
    pub incsearch: bool,
    /// 查找到达文本的一端时是否从另一端继续。
    pub wrapscan: bool,
    /// 替换是否默认作用于一行中的每一处匹配。
    pub gdefault: bool,
    /// 光标与窗口上下边缘之间保留的行数；`None` 表示按宿主的配置。
    pub scrolloff: Option<usize>,
    /// 映射等待后续按键的毫秒数；`None` 表示按宿主的配置。
    pub timeoutlen: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hlsearch: true,
            incsearch: true,
            wrapscan: true,
            gdefault: false,
            scrolloff: None,
            timeoutlen: None,
        }
    }
}

/// 选项保存的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// 由宿主保存。
    Host(HostOption),
    /// `ignorecase`。
    IgnoreCase,
    /// `smartcase`。
    SmartCase,
    /// `hlsearch`。
    HlSearch,
    /// `incsearch`。
    IncSearch,
    /// `wrapscan`。
    WrapScan,
    /// `gdefault`。
    GDefault,
    /// `scrolloff`。
    ScrollOff,
    /// `timeoutlen`。
    TimeoutLen,
    /// `clipboard`。
    Clipboard,
}

/// 选项的全名、缩写、保存的位置，以及是否为开关。
const OPTIONS: &[(&str, &str, Target, bool)] = &[
    ("number", "nu", Target::Host(HostOption::Number), true),
    (
        "relativenumber",
        "rnu",
        Target::Host(HostOption::RelativeNumber),
        true,
    ),
    ("wrap", "wrap", Target::Host(HostOption::Wrap), true),
    ("list", "list", Target::Host(HostOption::List), true),
    ("tabstop", "ts", Target::Host(HostOption::TabStop), false),
    (
        "shiftwidth",
        "sw",
        Target::Host(HostOption::ShiftWidth),
        false,
    ),
    ("expandtab", "et", Target::Host(HostOption::ExpandTab), true),
    ("ignorecase", "ic", Target::IgnoreCase, true),
    ("smartcase", "scs", Target::SmartCase, true),
    ("hlsearch", "hls", Target::HlSearch, true),
    ("incsearch", "is", Target::IncSearch, true),
    ("wrapscan", "ws", Target::WrapScan, true),
    ("gdefault", "gd", Target::GDefault, true),
    ("scrolloff", "so", Target::ScrollOff, false),
    ("timeoutlen", "tm", Target::TimeoutLen, false),
    ("clipboard", "cb", Target::Clipboard, false),
];

/// 返回各选项的全名与缩写，供帮助页列出。
pub fn option_list() -> impl Iterator<Item = (&'static str, &'static str)> {
    OPTIONS.iter().map(|(name, short, _, _)| (*name, *short))
}

/// 返回各选项的全名，供补全使用。
pub(crate) fn option_names() -> impl Iterator<Item = &'static str> {
    OPTIONS.iter().map(|(name, _, _, _)| *name)
}

/// 返回名为 `name`（全名或缩写）的选项。
fn lookup(name: &str) -> Option<(&'static str, Target, bool)> {
    OPTIONS
        .iter()
        .find(|(full, short, _, _)| *full == name || *short == name)
        .map(|(full, _, target, switch)| (*full, *target, *switch))
}

impl Shared {
    /// 返回选项的值；开关以 0、1 表示，`clipboard` 以同步的方式是否为「全部」表示。
    fn option(&self, host: &impl Host, target: Target) -> Option<usize> {
        Some(match target {
            Target::Host(option) => return host.option(option),
            Target::IgnoreCase => usize::from(self.ignore_case),
            Target::SmartCase => usize::from(self.smart_case),
            Target::HlSearch => usize::from(self.options.hlsearch),
            Target::IncSearch => usize::from(self.options.incsearch),
            Target::WrapScan => usize::from(self.options.wrapscan),
            Target::GDefault => usize::from(self.options.gdefault),
            Target::ScrollOff => (self.options.scrolloff).unwrap_or_else(|| host.scroll_margin()),
            Target::TimeoutLen => self.options.timeoutlen.map_or(1000, |ms| ms as usize),
            Target::Clipboard => usize::from(self.clipboard == Clipboard::Always),
        })
    }

    /// 设置选项的值；返回是否已设置。
    fn set_option(&mut self, host: &mut impl Host, target: Target, value: usize) -> bool {
        let on = value != 0;
        match target {
            Target::Host(option) => return host.set_option(option, value),
            Target::IgnoreCase => self.ignore_case = on,
            Target::SmartCase => self.smart_case = on,
            Target::HlSearch => self.options.hlsearch = on,
            Target::IncSearch => self.options.incsearch = on,
            Target::WrapScan => self.options.wrapscan = on,
            Target::GDefault => self.options.gdefault = on,
            Target::ScrollOff => self.options.scrolloff = Some(value),
            Target::TimeoutLen => self.options.timeoutlen = u32::try_from(value).ok(),
            Target::Clipboard => {
                self.clipboard = if on {
                    Clipboard::Always
                } else {
                    Clipboard::Register
                };
            }
        }
        true
    }
}

impl Vim {
    /// `:set`：依次处理参数中以空白分隔的各项。
    pub(super) fn ex_set(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        args: &str,
    ) -> Result<(), Failed> {
        for item in args.split_whitespace() {
            // 各项的写法：`名`、`no名`、`inv名`、`名!`、`名?`、`名=值`。
            let (name, value) = match item.split_once(['=', ':']) {
                Some((name, value)) => (name, Some(value)),
                None => (item, None),
            };
            let (name, query) = match name.strip_suffix('?') {
                Some(name) => (name, true),
                None => (name, false),
            };
            let (name, mut toggle) = match name.strip_suffix('!') {
                Some(name) => (name, true),
                None => (name, false),
            };
            let mut off = false;
            let found = lookup(name).or_else(|| {
                let (rest, inverse) = match name.strip_prefix("no") {
                    Some(rest) => (rest, false),
                    None => (name.strip_prefix("inv")?, true),
                };
                off = !inverse;
                toggle |= inverse;
                lookup(rest).filter(|(_, _, switch)| *switch)
            });
            let Some((full, target, switch)) = found else {
                return self.fail(host, Message::UnknownOption(name.to_owned()));
            };
            let current = shared.option(host, target);
            let wanted = match value {
                // 只写了名字：开关设为开，其余的选项显示当前的值。
                None if query || !(switch || off || toggle) => None,
                None if toggle => current.map(|value| usize::from(value == 0)),
                None => Some(usize::from(!off)),
                Some(_) if switch => return self.fail(host, Message::BadArgument),
                Some(value) if target == Target::Clipboard => {
                    Some(usize::from(value.contains("unnamed")))
                }
                Some(value) => match value.parse() {
                    Ok(value) => Some(value),
                    Err(_) => return self.fail(host, Message::BadArgument),
                },
            };
            match wanted {
                Some(value) => {
                    if !shared.set_option(host, target, value) {
                        return self.fail(host, Message::BadArgument);
                    }
                }
                None => {
                    let Some(current) = current else {
                        return self.fail(host, Message::UnknownOption(name.to_owned()));
                    };
                    let shown = match (switch, target) {
                        (true, _) if current == 0 => format!("no{full}"),
                        (true, _) => full.to_owned(),
                        (_, Target::Clipboard) if current == 0 => format!("{full}="),
                        (_, Target::Clipboard) => format!("{full}=unnamed"),
                        _ => format!("{full}={current}"),
                    };
                    host.message(Message::Option(shown));
                }
            }
        }
        Ok(())
    }
}
