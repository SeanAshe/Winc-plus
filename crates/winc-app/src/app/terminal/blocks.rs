//! 命令块在界面上能做的事：⌘↑ ⌘↓ 在命令之间跳、折起输出、长命令结束时提醒、F8 在最后一条
//! 失败命令的报错之间跳。
//!
//! 块本身（哪几行是哪条命令）由辅助进程从 shell 集成的标记里切好发来，
//! 见 `winc_pty::protocol::Block`。没有 shell 集成时没有块，这些都不出现：跳转和折叠什么也
//! 不做，F8 仍然发给程序。

use std::path::PathBuf;

use winc_i18n::t;
use winc_pty::input::{self, Key as TermKey, Mods};
use winc_pty::protocol::{Block, Line, Modes};

use super::locate::locations;
use super::tab::{Point, Selection, Unit};
use super::{App, Focus, Request};

/// 运行超过该时长的命令结束时提醒（毫秒，经验值）：更短的命令提醒过于频繁，反而打断操作。
const LONG_COMMAND: u32 = 10_000;
/// F8 最多查看失败命令输出的行数（行，经验值）：只认输出开头这段里的报错位置，1 万行输出
/// 里的报错通常都在靠前的部分。
const ERROR_LINES: u64 = 20_000;

/// 重新运行 `text` 要发给 shell 的字节：⌃U 清掉提示符后面已经敲的字，命令按粘贴的方式送进去
/// （开了括号粘贴时多行命令不会一行一行地跑），再回车。
fn rerun_bytes(text: &str, modes: Modes) -> Vec<u8> {
    let mut bytes = vec![0x15];
    input::paste(text, modes, &mut bytes);
    bytes.push(b'\r');
    bytes
}

/// F8 在报错之间跳的状态。
pub(crate) struct ErrorNav {
    /// 哪个标签。
    pub tab: u16,
    /// 哪个块（块的编号）。
    pub block: u32,
    /// 输出取回来、认完了没有。
    loaded: bool,
    /// 取回来以后先往哪个方向跳。
    forward: bool,
    /// 认出来的、文件存在的位置，按出现的顺序。
    pub spots: Vec<Spot>,
    /// 当前停在第几处；还没选中时为 `None`。
    pub current: Option<usize>,
    /// 分段要的输出已经回来的各段（起始行，行），都回来了才一起认。
    pub(super) got: Vec<(u64, Vec<winc_pty::protocol::Line>)>,
}

/// 报错里的一处文件位置。
#[derive(Debug, Clone)]
pub(crate) struct Spot {
    /// 报错文字在终端里的位置（被选中的那一处）。
    pub start: Point,
    /// 报错文字的结束位置（不含）。
    pub end: Point,
    /// 文件路径。
    pub path: PathBuf,
    /// 行号，从 0 起。
    pub line: usize,
    /// 列号，从 0 起。
    pub col: usize,
}

/// 失败了、而且有输出。
///
/// `lsof`、`grep`、`pgrep` 没找到东西时也退出 1，什么都不输出：色带仍按失败
/// 画成红色，但不算 F8 要找的报错，后台标签上也不亮红点。
fn failed_with_output(block: &Block) -> bool {
    block.exit.is_some_and(|e| e != 0) && block.output.is_some() && block.end > block.output
}

/// 命令行在提醒里怎么写：太长的截断，shell 没报命令行时写「命令」。
fn command_name(block: &Block) -> String {
    let text = block.text.trim();
    if text.is_empty() {
        return t!("terminal-command-fallback").into();
    }
    let short: String = text.chars().take(40).collect();
    if short.len() < text.len() {
        t!("terminal-quoted-command", command = format!("{short}…"))
    } else {
        t!("terminal-quoted-command", command = short)
    }
}

/// 把时长（毫秒）写成「0.8 s」「12.4 s」「3 min 5 s」「1 h 2 min」这样的文字。
pub(crate) fn format_duration(ms: u32) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{:.1} s", f64::from(ms) / 1000.0)
    } else if s < 3600 {
        format!("{} min {} s", s / 60, s % 60)
    } else {
        format!("{} h {} min", s / 3600, s / 60 % 60)
    }
}

/// 块的输出是否值得折叠：只有一行时折起后的提示和原内容一样高，反而遮住内容。
fn has_foldable_output(block: &Block) -> bool {
    matches!((block.output, block.end), (Some(output), Some(end)) if end > output + 1)
}

impl App {
    /// 辅助进程发来了新的块：刚结束的命令按需要提醒。
    pub(in crate::app) fn terminal_blocks(&mut self, id: u16, blocks: Vec<Block>, early: bool) {
        let t = &mut self.terminal;
        let Some(index) = t.index_of(id) else {
            return;
        };
        let tab = &mut t.tabs[index];
        // 在运行的一般只有最后那一条。比的是最新知道的块，先到、还没换上的也算。
        let known = tab.early_blocks.as_ref().unwrap_or(&tab.blocks);
        let running: Vec<u32> = known.iter().filter(|b| b.running()).map(|b| b.id).collect();
        let finished: Vec<Block> = blocks
            .iter()
            .filter(|b| b.end.is_some() && running.contains(&b.id))
            .cloned()
            .collect();
        if early {
            // 命令结束仍然提示（后台标签上要亮一个点），块等那一帧到了再换上。
            tab.early_blocks = Some(blocks);
        } else {
            tab.early_blocks = None;
            tab.set_blocks(blocks);
        }
        if t.errors.as_ref().is_some_and(|e| e.tab == id) && !finished.is_empty() {
            // 又有命令结束了：「最后一条失败的命令」可能换了。
            t.errors = None;
        }
        for block in finished {
            self.terminal_command_finished(index, &block);
        }
        self.fetch_terminal_history();
    }

    /// 一条命令结束了。
    ///
    /// 不在看这个标签时标签上亮一个点；运行得久、窗口又不在前台时让程序坞
    /// 图标跳动一次，回到窗口时提示结果。
    fn terminal_command_finished(&mut self, index: usize, block: &Block) {
        let failed = failed_with_output(block);
        let t = &mut self.terminal;
        // 窗口不在前台但面板上就是这个标签时不用点：回来时看到的就是它。
        if !(t.open && t.active == index) {
            t.tabs[index].unseen = Some(failed);
        }
        if self.window.focused || block.duration.is_none_or(|d| d < LONG_COMMAND) {
            return;
        }
        let name = command_name(block);
        let duration = format_duration(block.duration.unwrap_or(0));
        let message = match block.exit {
            Some(code) if code != 0 => t!(
                "terminal-command-failed",
                name = name,
                code = code,
                duration = duration
            ),
            _ => t!(
                "terminal-command-finished",
                name = name,
                duration = duration
            ),
        };
        t.pending_notice = Some(message);
        if let Some(window) = &self.window.handle {
            window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
    }

    /// 程序响铃（`message` 为 `None`）或发了通知（OSC 9、777）。
    ///
    /// 不在看这个标签时标签上亮一个点；
    /// 窗口不在前台时程序坞图标跳一次，通知等回到窗口时提示。窗口在前台时通知直接提示，正在
    /// 看的标签响铃不做什么（没有声音，程序常拿响铃当「到头了」）。
    pub(in crate::app) fn terminal_alert(&mut self, id: u16, message: Option<String>) {
        let t = &mut self.terminal;
        let Some(index) = t.index_of(id) else {
            return;
        };
        let seen = t.open && t.active == index && self.window.focused;
        if seen && message.is_none() {
            return;
        }
        if !(t.open && t.active == index) {
            let tab = &mut t.tabs[index];
            tab.unseen = Some(tab.unseen == Some(true));
        }
        if self.window.focused {
            if let Some(message) = message {
                self.notify(message);
            }
        } else {
            if message.is_some() {
                t.pending_notice = message;
            }
            if let Some(window) = &self.window.handle {
                window
                    .request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
        }
        self.request_redraw();
    }

    /// 窗口回到前台：把不在时结束的长命令的结果提示出来。
    pub(crate) fn terminal_window_focused(&mut self) {
        if let Some(message) = self.terminal.pending_notice.take() {
            self.notify(message);
        }
        let t = &mut self.terminal;
        if t.open
            && let Some(tab) = t.tabs.get_mut(t.active)
        {
            tab.unseen = None;
        }
    }

    /// ⌘↑、⌘↓：跳到上一条、下一条命令，命令的提示符放在网格第一行。
    ///
    /// 跳过的命令行铺一层底色，
    /// 继续按时仍从它向上、向下数；输入文字以后从头数。往下跳过了最后一条回到最新的输出。
    pub(in crate::app) fn jump_terminal_command(&mut self, down: bool) {
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        if tab.modes.has(Modes::ALT_SCREEN) {
            return;
        }
        let commands: Vec<&Block> = tab
            .blocks
            .iter()
            .filter(|b| b.output.is_some() && b.prompt >= tab.history.0)
            .collect();
        let at = tab
            .jump
            .and_then(|id| commands.iter().position(|b| b.id == id));
        let target = match (at, down) {
            (Some(i), false) => i.checked_sub(1).or(Some(i)),
            (Some(i), true) => (i + 1 < commands.len()).then_some(i + 1),
            (None, false) => commands.len().checked_sub(1),
            (None, true) => None,
        };
        match target.map(|i| (commands[i].id, commands[i].prompt)) {
            Some((id, prompt)) => {
                tab.jump = Some(id);
                tab.scroll_to(prompt, 0);
            }
            None => {
                tab.jump = None;
                tab.scroll = 0;
                tab.scroll_frac = 0.0;
            }
        }
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// ⌥⌘[、⌥⌘]：折起、展开一条命令的输出。
    ///
    /// 是哪一条：刚用 ⌘↑ ⌘↓ 跳到的那条；往上翻着时网格
    /// 第一行所在的那条；看着最新输出时是最后一条结束了的（展开时是最后一条折着的）。
    pub(in crate::app) fn fold_terminal_output(&mut self, fold: bool) {
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        let folded = tab.folded.clone();
        let foldable = |b: &Block| has_foldable_output(b) || folded.contains(&b.id);
        let target = if let Some(id) = tab.jump {
            Some(id)
        } else if tab.scroll > 0 {
            tab.block_at(tab.top()).map(|i| tab.blocks[i].id)
        } else {
            tab.blocks
                .iter()
                .rev()
                .find(|b| foldable(b) && tab.folded.contains(&b.id) != fold)
                .map(|b| b.id)
        };
        let Some(id) = target.filter(|id| tab.blocks.iter().any(|b| b.id == *id && foldable(b)))
        else {
            return;
        };
        tab.set_folded(id, fold);
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// 点了第 `block` 个块的色带：折起或展开它的输出；按住 ⇧ 时选中整块输出，之后可以 ⌘C 复制。
    pub(in crate::app) fn click_terminal_band(&mut self, block: usize) {
        let shift = self.modifiers.shift_key();
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        let Some(b) = tab.blocks.get(block) else {
            return;
        };
        if b.output.is_none() {
            return;
        }
        if shift {
            tab.select_output(block);
        } else {
            let id = b.id;
            let folded = tab.folded.contains(&id);
            // 已折起的块总是可以展开：重排后输出可能变成一行。
            if folded || has_foldable_output(b) {
                tab.set_folded(id, !folded);
            }
        }
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// 右键菜单「复制命令」：编号为 `id` 的块的命令行（shell 报的那一行）。
    pub(in crate::app) fn copy_terminal_command(&mut self, id: u32) {
        let Some(text) = self
            .terminal
            .tab()
            .and_then(|t| t.blocks.iter().find(|b| b.id == id))
            .map(|b| b.text.clone())
            .filter(|t| !t.is_empty())
        else {
            return;
        };
        self.set_clipboard(text, false);
        self.flash(t!("terminal-command-copied"));
    }

    /// 右键菜单「复制输出」：选中整段输出再复制，和 ⇧ 点色带再 ⌘C 一样（有没取回的历史行时
    /// 取回来再复制）。
    pub(in crate::app) fn copy_terminal_output(&mut self, id: u32) {
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        let Some(index) = tab.blocks.iter().position(|b| b.id == id) else {
            return;
        };
        if tab.select_output(index) {
            self.fetch_terminal_history();
            self.terminal_copy();
            self.request_redraw();
        }
    }

    /// 右键菜单「重新运行」：把那条命令再送给 shell（[`rerun_bytes`]）。
    ///
    /// 只在 shell 停在提示符上
    /// 时可用，见 [`App::terminal_can_rerun`]。
    pub(in crate::app) fn rerun_terminal_command(&mut self, id: u32) {
        if !self.terminal_can_rerun(id) {
            return;
        }
        let Some(tab) = self.terminal.tab() else {
            return;
        };
        let Some(block) = tab.blocks.iter().find(|b| b.id == id) else {
            return;
        };
        let bytes = rerun_bytes(&block.text, tab.modes);
        self.terminal_input(bytes);
    }

    /// 编号为 `id` 的块能不能重新运行：报了命令行，终端还活着，不在备用屏上，也没有命令在跑。
    pub(in crate::app) fn terminal_can_rerun(&self, id: u32) -> bool {
        let Some(tab) = self.terminal.tab() else {
            return false;
        };
        tab.ended.is_none()
            && !self.terminal.crashed
            && !tab.modes.has(Modes::ALT_SCREEN)
            && !tab.blocks.iter().any(Block::running)
            && tab.blocks.iter().any(|b| b.id == id && !b.text.is_empty())
    }

    /// F8、⇧F8：在最后一条失败命令的输出里，按出现的顺序在认得出的文件位置之间跳，选中它；↩
    /// 打开那个文件的那一行。
    ///
    /// 备用屏上、没有 shell 集成时仍然把键发给程序。
    pub(in crate::app) fn step_terminal_error(&mut self, forward: bool) {
        let t = &mut self.terminal;
        let Some(tab) = t.tabs.get(t.active) else {
            return;
        };
        if tab.modes.has(Modes::ALT_SCREEN) || tab.blocks.is_empty() {
            let mods = Mods {
                shift: !forward,
                ..Mods::default()
            };
            let mut bytes = Vec::new();
            input::encode(TermKey::F(8), mods, tab.modes, tab.keyboard, &mut bytes);
            self.terminal_input(bytes);
            return;
        }
        let failed = tab.blocks.iter().rev().find(|b| failed_with_output(b));
        let Some(block) = failed else {
            self.flash(t!("terminal-no-failed-commands"));
            return;
        };
        let (tab_id, block_id) = (tab.id, block.id);
        if let Some(nav) = &mut t.errors
            && nav.tab == tab_id
            && nav.block == block_id
        {
            if nav.loaded {
                let n = nav.spots.len();
                nav.current = Some(match nav.current {
                    Some(i) if forward => (i + 1) % n,
                    Some(i) => (i + n - 1) % n,
                    None if forward => 0,
                    None => n - 1,
                });
                self.reveal_terminal_error();
            }
            return;
        }
        // 先把输出取回来：屏幕上的现成，回滚缓冲里的向辅助进程要。
        let output = block.output.unwrap_or_default();
        let end = block.end.unwrap_or_default().min(output + ERROR_LINES);
        // 上一次还没取完的不要了：回来的会拼进这一次的输出里，行号全错开。
        t.requests.retain(|r| r.2 != Request::Errors);
        t.errors = Some(ErrorNav {
            tab: tab_id,
            block: block_id,
            loaded: false,
            forward,
            spots: Vec::new(),
            current: None,
            got: Vec::new(),
        });
        let history_end = end.min(tab.history.1);
        if output < history_end {
            t.request(tab_id, (output, history_end), Request::Errors);
        } else {
            self.terminal_errors_loaded(tab_id, output, Vec::new());
        }
    }

    /// F8 要的输出取回来了（`lines` 是回滚缓冲里的那部分，从 `start` 起）：认出文件位置。
    pub(in crate::app) fn terminal_errors_loaded(&mut self, id: u16, start: u64, lines: Vec<Line>) {
        let root = self.editor.root().map(PathBuf::from);
        let t = &mut self.terminal;
        let Some(block_id) = t
            .errors
            .as_ref()
            .filter(|n| n.tab == id && !n.loaded)
            .map(|n| n.block)
        else {
            return;
        };
        let Some(tab) = t.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let Some(block) = tab.blocks.iter().find(|b| b.id == block_id) else {
            t.errors = None;
            return;
        };
        // 打开的目录位于 WSL 发行版中时，命令报告的绝对路径属于同一发行版。
        let local = |path| match &root {
            Some(root) => winc_editor::wsl::from_terminal(root, path),
            None => path,
        };
        let cwd = if block.cwd.is_empty() {
            root.clone()
        } else {
            Some(local(path_from_bytes(&block.cwd)))
        };
        let (output, end) = (
            block.output.unwrap_or_default(),
            block.end.unwrap_or_default(),
        );
        let history = lines
            .iter()
            .enumerate()
            .map(|(i, line)| (start + i as u64, line));
        let screen = tab
            .rows
            .iter()
            .enumerate()
            .map(|(r, line)| (tab.history.1 + r as u64, line));
        let mut spots: Vec<Spot> = Vec::new();
        for (number, line) in history.chain(screen) {
            if number < output || number >= end {
                continue;
            }
            for found in locations(line, false) {
                let Some(line_no) = found.line else { continue };
                let path = local(PathBuf::from(&found.path));
                let path = match (&cwd, path.is_absolute()) {
                    (Some(cwd), false) => cwd.join(path),
                    _ => path,
                };
                // 同一个位置报多次（rustc 的 `-->` 加上下面的注释）只算一处。
                if spots
                    .iter()
                    .any(|s| s.path == path && s.line + 1 == line_no)
                    || !path.is_file()
                {
                    continue;
                }
                let point = |col| Point { line: number, col };
                spots.push(Spot {
                    start: point(found.start),
                    end: point(found.end),
                    path,
                    line: line_no - 1,
                    col: found.col.unwrap_or(1) - 1,
                });
            }
        }
        if spots.is_empty() {
            t.errors = None;
            self.flash(t!("terminal-no-error-location"));
            return;
        }
        if let Some(nav) = &mut t.errors {
            nav.current = Some(if nav.forward { 0 } else { spots.len() - 1 });
            nav.spots = spots;
            nav.loaded = true;
        }
        self.reveal_terminal_error();
    }

    /// 选中当前的报错位置，翻到能看见的地方。
    fn reveal_terminal_error(&mut self) {
        let t = &mut self.terminal;
        let Some(nav) = &t.errors else { return };
        let Some(spot) = nav.current.and_then(|i| nav.spots.get(i)) else {
            return;
        };
        let (start, end) = (spot.start, spot.end);
        let label = t!(
            "terminal-error-position",
            index = nav.current.unwrap_or_default() + 1,
            total = nav.spots.len()
        );
        let Some(tab) = t.tabs.iter_mut().find(|tab| tab.id == nav.tab) else {
            return;
        };
        tab.reveal(start.line);
        tab.selection = Some(Selection {
            anchor: start,
            head: end,
            unit: Unit::Char,
        });
        self.flash(label);
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// 焦点在终端时按了键（快捷键之外的）：F8 选中了一处报错、选区还在时 ↩ 打开它，返回真；
    /// 别的键结束 F8 的跳转，仍然交给 shell。
    pub(in crate::app) fn terminal_error_key(&mut self, enter: bool) -> bool {
        let t = &mut self.terminal;
        let Some(nav) = &t.errors else {
            return false;
        };
        let spot = nav.current.and_then(|i| nav.spots.get(i)).cloned();
        let selected = t
            .tabs
            .iter()
            .find(|tab| tab.id == nav.tab)
            .and_then(|tab| tab.selection)
            .zip(spot.as_ref())
            .is_some_and(|(s, spot)| s.anchor == spot.start && s.head == spot.end);
        if !(enter && selected) {
            t.errors = None;
            return false;
        }
        let Some(spot) = spot else { return false };
        self.open_path(&spot.path);
        self.jump_to(spot.line, spot.col, 0);
        if self.editor.active().is_none() {
            self.focus = Focus::Terminal;
        }
        true
    }
}

/// 把辅助进程报的字节串换成路径（路径不一定是 UTF-8）。
#[cfg(unix)]
pub(super) fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// 把辅助进程报的字节串换成路径；非 Unix 平台只能按 UTF-8 有损转换。
#[cfg(not(unix))]
pub(super) fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests;
