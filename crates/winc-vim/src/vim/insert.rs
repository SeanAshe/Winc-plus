//! 插入与替换模式：进入、按键、输入的记录、次数的重复与离开。

use winc_core::text::char_cells;
use winc_core::{Change, Lines, Selection, movement};

use super::{Extent, Repeat, Shared, Vim};
use crate::command::{Action, Mode};
use crate::host::{Host, Input, Message};
use crate::key::{Key, Named};
use crate::motion::{Want, WantUpdate};
use crate::parser::{Body, Command, Outcome};
use crate::register::Kind;
use crate::text;

/// 一次插入记录的输入占用的上限（字节）；超出之后的输入不再记录。
///
/// 取自调参：1 MiB 远多于一次连续输入的文字，重复插入时生成的文本另有上限。
const MAX_RECORD: usize = 1 << 20;

/// 离开插入模式之后光标的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum After {
    /// 每个光标各左移一个字符。
    Left,
    /// 只保留第一个光标，并左移一个字符。
    First,
    /// 只保留一个光标，放在第 `line` 行的显示列 `cell` 上。
    At {
        /// 行号。
        line: usize,
        /// 显示列。
        cell: usize,
    },
}

/// 一次进行中的插入。
#[derive(Debug)]
pub(super) struct Session {
    /// 引起这次插入的命令，离开时连同输入记为上一次修改。
    command: Command,
    /// 离开时输入重复的次数，不小于 1。
    count: usize,
    /// 已执行的输入。
    inputs: Vec<Input>,
    /// 已记录的输入占用的字节数。
    bytes: usize,
    /// 由 `o`（真）或 `O`（假）进入时，每次重复之前先新开一行。
    open: Option<bool>,
    /// 命令在可视模式下执行时选区覆盖的范围。
    pub(super) extent: Option<Extent>,
    /// 替换模式下被覆盖的字符，按键入的顺序排列；`None` 表示键入时没有覆盖字符。不在替换
    /// 模式时为 `None`。
    replaced: Option<Vec<Option<String>>>,
    /// 离开时光标的位置。
    after: After,
    /// 插入开始的位置。
    start: usize,
    /// 按字面插入的输入在 `inputs` 中的下标：从上一行或下一行复制的字符。
    literal: Vec<usize>,
}

impl Session {
    /// 是否为替换模式。
    pub(super) fn replaces(&self) -> bool {
        self.replaced.is_some()
    }

    /// 改为不重复的插入，离开时光标按 `after` 放置；`extent` 的含义见同名字段。
    pub(super) fn single(&mut self, after: After, extent: Option<Extent>) {
        self.count = 1;
        self.after = after;
        self.extent = extent;
    }
}

/// 插入模式中执行一条普通模式命令期间保存的状态。
#[derive(Debug, Clone, Copy)]
pub(super) struct Oneshot {
    /// 是否回到替换模式。
    replace: bool,
}

/// 返回输入 `input` 占用的字节数。
fn size(input: &Input) -> usize {
    match input {
        Input::Text(text) | Input::Paste(text) => text.len().max(1),
        _ => 1,
    }
}

/// 返回由不带范围的命令 `action` 组成的命令。
fn plain(action: Action) -> Command {
    Command {
        register: None,
        count: None,
        body: Body::Action { action, arg: None },
    }
}

impl Vim {
    /// 执行进入插入模式的命令 `action`。
    pub(super) fn start_insert(&mut self, host: &mut impl Host, command: Command, action: Action) {
        if self.blocked(host) {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let (_, last) = host
            .fold(line)
            .filter(|_| host.has_folds())
            .unwrap_or((line, line));
        let pos = match action {
            Action::Append => text::next_in_line(text, cursor).unwrap_or(cursor),
            Action::InsertFirstNonBlank => text.line_start(line) + text::indent_chars(text, line),
            Action::InsertLineStart => text.line_start(line),
            Action::AppendLineEnd => text.line_end(line),
            Action::InsertAtLast => match host.mark('^') {
                Some(pos) => {
                    let pos = pos.min(text.end_pos());
                    let line = text::line_of(text, pos);
                    pos.clamp(text.line_start(line), text.line_end(line))
                }
                None => cursor,
            },
            Action::OpenBelow => {
                self.open_line(host, last, true);
                self.begin_insert(host, command, host.selection().head, Some(true));
                return;
            }
            Action::OpenAbove => {
                self.open_line(host, line, false);
                self.begin_insert(host, command, host.selection().head, Some(false));
                return;
            }
            _ => cursor,
        };
        self.begin_insert(host, command, pos, None);
    }

    /// `R`：进入替换模式。
    pub(super) fn start_replace(&mut self, host: &mut impl Host, command: Command) {
        if self.blocked(host) {
            return;
        }
        let cursor = self.caret(host);
        self.begin_insert(host, command, cursor, None);
        if let Some(session) = &mut self.insert {
            session.replaced = Some(Vec::new());
        }
    }

    /// 在第 `line` 行的下方（`below` 为真）或上方新开一行，光标放在新行上。
    fn open_line(&mut self, host: &mut impl Host, line: usize, below: bool) {
        let text = host.text();
        if below {
            host.open_line(text.line_end(line));
            self.shown = None;
            return;
        }
        let cursor = self.caret(host);
        let start = text.line_start(line);
        let indent = if host.keeps_indent() {
            text.substring(start, start + text::indent_chars(text, line))
        } else {
            String::new()
        };
        let pos = start + indent.chars().count();
        let change = Change {
            start,
            removed: String::new(),
            inserted: format!("{indent}{}", host.line_ending()),
        };
        self.edit(host, vec![change], cursor, line);
        host.set_selection(Selection::point(pos));
    }

    /// 进入插入模式，光标位于 `pos`；`open` 的含义见 [`Session`]。
    pub(super) fn begin_insert(
        &mut self,
        host: &mut impl Host,
        command: Command,
        pos: usize,
        open: Option<bool>,
    ) {
        // 修改操作符的次数属于动作，只有直接进入插入模式的命令才重复输入。
        let direct = matches!(command.body, Body::Action { .. });
        let count = match command.count {
            Some(count) if direct => usize::try_from(count).unwrap_or(usize::MAX),
            _ => 1,
        };
        if self.visual.is_some() {
            self.end_visual(host);
        }
        self.mode = Mode::Insert;
        self.parser.reset();
        self.insert = Some(Session {
            command,
            count,
            inputs: Vec::new(),
            bytes: 0,
            open,
            extent: None,
            replaced: None,
            after: After::Left,
            start: pos,
            literal: Vec::new(),
        });
        self.place(host, Selection::point(pos));
    }

    /// 处理插入模式下的按键，返回是否已处理。
    pub(super) fn insert_key(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        key: Key,
    ) -> bool {
        // 替换模式的退格还原被覆盖的字符。
        if self.is_replace()
            && key == Key::named(Named::Backspace)
            && self.parser.pending().is_empty()
        {
            self.play(host, &Input::Backspace { word: false });
            return true;
        }
        match self.parser.feed(Mode::Insert, key) {
            Outcome::Command(command) => {
                if let Body::Action { action, arg } = command.body {
                    self.insert_action(shared, host, action, arg);
                }
                true
            }
            Outcome::Pending | Outcome::Cancelled | Outcome::Invalid => true,
            Outcome::Unsupported => {
                self.error(host, Message::Unsupported);
                true
            }
            Outcome::Unbound => false,
        }
    }

    /// 执行插入模式下的命令 `action`。
    fn insert_action(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        action: Action,
        arg: Option<char>,
    ) {
        match action {
            Action::Escape | Action::Interrupt => self.leave_insert(shared, host),
            Action::InsertBackspace => self.play(host, &Input::Backspace { word: false }),
            Action::InsertDeleteWord => self.play(host, &Input::Backspace { word: true }),
            Action::InsertDeleteLine => self.play(host, &Input::DeleteToLineStart),
            Action::InsertIndent => self.play(host, &Input::IndentLine),
            Action::InsertOutdent => self.play(host, &Input::Outdent),
            Action::InsertRegister => {
                let Some(content) = Self::read_register(shared, host, arg) else {
                    self.error(host, Message::EmptyRegister(arg.unwrap_or('"')));
                    return;
                };
                let mut text = content.text.replace('\n', host.line_ending());
                if content.kind == Kind::Linewise {
                    text.push_str(host.line_ending());
                }
                self.play(host, &Input::Paste(text));
            }
            Action::InsertBelow | Action::InsertAbove => {
                let text = host.text();
                let pos = host.selection().head.min(text.end_pos());
                let line = text::line_of(text, pos);
                let other = if action == Action::InsertBelow {
                    (line < text::last_line(text)).then(|| line + 1)
                } else {
                    line.checked_sub(1)
                };
                let tab_width = host.tab_width();
                let (_, col) = text.pos_to_line_col(pos);
                let cell = text::cell_of(text, line, col, tab_width);
                // 取另一行上覆盖同一显示列的字符。
                let copied = other.and_then(|other| {
                    let content = text.line_content(other);
                    let mut at = 0;
                    content.slice().chars().find(|c| {
                        at += char_cells(*c, at, tab_width.max(1));
                        at > cell
                    })
                });
                if let Some(c) = copied {
                    self.play(host, &Input::Text(c.to_string()));
                    // 字母与数字之外的字符按字面插入。
                    if let Some(session) = &mut self.insert
                        && !c.is_alphanumeric()
                    {
                        session.literal.push(session.inputs.len().saturating_sub(1));
                    }
                }
            }
            Action::InsertNormal => self.suspend_insert(shared, host),
            _ => {}
        }
    }

    /// 执行插入模式中的一次输入并记录；替换模式下文字覆盖已有的字符。
    pub(super) fn play(&mut self, host: &mut impl Host, input: &Input) {
        match (input, self.is_replace()) {
            (Input::Text(text) | Input::Paste(text), true) => self.overwrite(host, text),
            (Input::Backspace { word: false }, true) => self.restore_one(host),
            (Input::Newline, true) => {
                host.input(input);
                self.note_replaced(None);
            }
            _ => host.input(input),
        }
        self.shown = Some(host.selection());
        self.typed(input.clone());
    }

    /// 替换模式下键入文字 `text`：每个字符覆盖光标处的一个字符，行尾之后改为插入。
    ///
    /// 宿主在替换模式（[`Vim::is_replace`]）下收到键入的文字时调用，不自行插入。
    pub fn overtype(&mut self, host: &mut impl Host, text: &str) {
        self.play(host, &Input::Text(text.to_owned()));
    }

    /// 记下替换模式中一次键入覆盖的字符。
    fn note_replaced(&mut self, old: Option<String>) {
        if let Some(replaced) = self.insert.as_mut().and_then(|s| s.replaced.as_mut()) {
            replaced.push(old);
        }
    }

    /// 以 `typed` 的各个字符依次覆盖光标处的字符。
    fn overwrite(&mut self, host: &mut impl Host, typed: &str) {
        for c in typed.chars() {
            let text = host.text();
            let pos = host.selection().head.min(text.end_pos());
            if c == '\n' || c == '\r' {
                host.input(&Input::Newline);
                self.note_replaced(None);
                continue;
            }
            let next = text::next_in_line(text, pos);
            let old = next.map(|next| text.substring(pos, next));
            let change = Change {
                start: pos,
                removed: old.clone().unwrap_or_default(),
                inserted: c.to_string(),
            };
            if !host.apply(vec![change], pos, pos + 1) {
                self.error(host, Message::TooLarge);
                return;
            }
            host.set_selection(Selection::point(pos + 1));
            self.note_replaced(old);
        }
    }

    /// 替换模式的退格：还原上一次键入覆盖的字符；没有可还原的内容时只把光标左移。
    fn restore_one(&mut self, host: &mut impl Host) {
        let text = host.text();
        let pos = host.selection().head.min(text.end_pos());
        let entry = (self.insert.as_mut())
            .and_then(|session| session.replaced.as_mut())
            .and_then(Vec::pop);
        let Some(old) = entry else {
            // 没有可还原的内容：光标左移，在行首时移到上一行的行尾之后。
            let left = text::prev_in_line(text, pos).unwrap_or_else(|| {
                let line = text::line_of(text, pos);
                line.checked_sub(1)
                    .map_or(pos, |above| text.line_end(above))
            });
            host.set_selection(Selection::point(left));
            return;
        };
        let start = movement::prev_grapheme(text, pos);
        let change = Change {
            start,
            removed: text.substring(start, pos),
            inserted: old.unwrap_or_default(),
        };
        if host.apply(vec![change], pos, start) {
            host.set_selection(Selection::point(start));
        }
    }

    /// 记录插入模式中宿主已经执行的一次输入。
    pub fn typed(&mut self, input: Input) {
        if let Some(session) = &mut self.insert {
            let size = size(&input);
            if session.bytes + size <= MAX_RECORD {
                session.bytes += size;
                session.inputs.push(input);
            }
        }
    }

    /// 插入模式中光标被移动（方向键、鼠标）或执行了输入之外的编辑时调用。
    ///
    /// 此前的输入结束一个撤销单元；之后的输入另起一个，点重复只重放之后的输入。
    pub fn insert_moved(&mut self, host: &mut impl Host) {
        let Some(session) = &mut self.insert else {
            return;
        };
        session.inputs.clear();
        session.literal.clear();
        session.bytes = 0;
        session.count = 1;
        session.open = None;
        session.extent = None;
        session.after = After::Left;
        session.start = host.selection().head;
        let action = match &mut session.replaced {
            Some(replaced) => {
                replaced.clear();
                Action::Replace
            }
            None => Action::Insert,
        };
        session.command = plain(action);
        if self.insert_group {
            host.end_group();
            host.begin_group();
        }
    }

    /// 插入模式的 `Ctrl-O`：暂时回到普通模式，执行一条命令之后再回到插入模式。
    fn suspend_insert(&mut self, shared: &mut Shared, host: &mut impl Host) {
        let text = host.text();
        let pos = host.selection().head.min(text.end_pos());
        let (line, col) = text.pos_to_line_col(pos);
        let cell = text::cell_of(text, line, col, host.tab_width());
        let oneshot = Oneshot {
            replace: self.is_replace(),
        };
        // 输入不重复，光标留在原处；行尾之后的光标退到最后一个字符上，期望列仍在其后。
        if let Some(session) = &mut self.insert {
            session.count = 1;
        }
        self.finish_insert(shared, host, false);
        if self.want != Want::End {
            self.want = Want::Cell(cell);
        }
        self.oneshot = Some(oneshot);
    }

    /// 插入模式中执行的一条命令结束之后回到插入模式。
    pub(super) fn resume_insert(&mut self, host: &mut impl Host) {
        let Some(oneshot) = self.oneshot else {
            return;
        };
        if self.prompt.is_some() {
            return;
        }
        self.oneshot = None;
        if self.mode != Mode::Normal || self.visual.is_some() {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        // 期望列在光标之后（原来在行尾之后而命令没有改变期望列，或命令把光标移到了行尾）且
        // 光标在最后一个字符上：回到最后一个字符之后。
        let to_end = match self.want {
            Want::End => true,
            Want::Cell(cell) => cell > text::want_cell(text, cursor, host.tab_width()),
        };
        let pos = if to_end && cursor == text::last_char(text, line) && text.line_len(line) > 0 {
            text.line_end(line)
        } else {
            cursor
        };
        let action = if oneshot.replace {
            Action::Replace
        } else {
            Action::Insert
        };
        host.begin_group();
        self.begin_insert(host, plain(action), pos, None);
        if oneshot.replace
            && let Some(session) = &mut self.insert
        {
            session.replaced = Some(Vec::new());
        }
        self.insert_group = true;
    }

    /// 离开插入模式：按次数重复输入，光标左移一个字符，并把这次插入记为上一次修改。
    pub(super) fn leave_insert(&mut self, shared: &mut Shared, host: &mut impl Host) {
        self.finish_insert(shared, host, true);
    }

    /// 结束进行中的插入；`left` 为真时光标左移一个字符，否则只从行尾之后退到字符上。
    fn finish_insert(&mut self, shared: &mut Shared, host: &mut impl Host, left: bool) {
        let Some(mut session) = self.insert.take() else {
            return;
        };
        self.parser.reset();

        // 次数大于 1 时重复输入；生成的文本超过上限时只执行一次。新开的行按一个字节计。
        let repeats = session.count - 1;
        let each = session.bytes + usize::from(session.open.is_some());
        if repeats > 0 && each > 0 && each.saturating_mul(repeats) <= super::MAX_OUTPUT {
            let inputs = std::mem::take(&mut session.inputs);
            let replace = session.replaced.is_some();
            // 重复期间仍按原来的方式执行输入。
            self.insert = Some(Session {
                inputs: Vec::new(),
                bytes: 0,
                ..session
            });
            let started = std::time::Instant::now();
            for _ in 0..repeats {
                // 重复的耗时超过回放的时间上限时中止，已经输入的部分保留。
                if started.elapsed() > self.limits.1 {
                    self.error(host, Message::Interrupted);
                    break;
                }
                // 重复时新行都开在刚输入的一行的下方。
                if self.insert.as_ref().is_some_and(|s| s.open.is_some()) {
                    let line = text::line_of(host.text(), host.selection().head);
                    self.open_line(host, line, true);
                }
                for input in &inputs {
                    if replace {
                        self.play(host, input);
                    } else {
                        host.input(input);
                    }
                }
            }
            let Some(restored) = self.insert.take() else {
                return;
            };
            session = Session { inputs, ..restored };
        }

        let pos = host.selection().head.min(host.text().end_pos());
        host.set_mark('^', pos);
        let typed = session.inputs.iter().any(|input| {
            matches!(
                input,
                Input::Text(_) | Input::Paste(_) | Input::Newline | Input::Indent
            )
        });
        if typed {
            let last = text::prev_in_line(host.text(), pos).unwrap_or(pos);
            let start = session.start.min(pos);
            host.set_mark('[', start);
            host.set_mark(']', pos);
            host.set_mark('.', last);
            host.record_change(last);
            self.change_index = None;
        }
        self.mode = Mode::Normal;
        let all = host.selections();
        let text = host.text();
        // 没有输入时光标按通常的方式左移。
        let after = match session.after {
            After::At { .. } if !typed => After::First,
            after => after,
        };
        match after {
            After::Left if all.len() > 1 => {
                // 多个光标：各自左移一个字符。
                let text = host.text();
                let primary = host.selection();
                let index = all.iter().position(|s| *s == primary).unwrap_or(0);
                let moved: Vec<Selection> = (all.iter())
                    .map(|s| {
                        let head = s.head.min(text.end_pos());
                        let target = if left {
                            text::prev_in_line(text, head).unwrap_or(head)
                        } else {
                            head
                        };
                        Selection::point(text::on_char(text, target))
                    })
                    .collect();
                self.shown = moved.get(index).copied();
                self.want = Want::Cell(text::want_cell(
                    text,
                    self.shown.map_or(pos, |s| s.head),
                    host.tab_width(),
                ));
                host.set_selections(moved, index);
            }
            After::At { line, cell } => {
                // 只保留一个光标。
                self.owned = all.len() > 1;
                let line = line.min(text::last_line(text));
                let target = text::pos_at_cell(text, line, cell, host.tab_width());
                self.goto(host, target, WantUpdate::Recompute);
            }
            After::Left | After::First => {
                let pos = match after {
                    After::First => all.first().map_or(pos, |s| s.head.min(text.end_pos())),
                    _ => pos,
                };
                let target = if left {
                    text::prev_in_line(text, pos).unwrap_or(pos)
                } else {
                    pos
                };
                self.owned = after == After::First && all.len() > 1;
                self.goto(host, target, WantUpdate::Recompute);
            }
        }
        if self.insert_group {
            host.end_group();
            self.insert_group = false;
        }

        // 最近插入的文本：删除与缩进记作对应的控制字符，按字面插入的字符带前导的控制字符。
        let mut typed = String::new();
        for (index, input) in session.inputs.iter().enumerate() {
            if session.literal.contains(&index) {
                typed.push('\u{16}');
            }
            typed.push_str(match input {
                Input::Text(text) | Input::Paste(text) => text.as_str(),
                Input::Newline => "\n",
                Input::Backspace { word: false } => "\u{8}",
                Input::Backspace { word: true } => "\u{17}",
                Input::DeleteToLineStart => "\u{15}",
                Input::IndentLine => "\u{14}",
                Input::Outdent => "\u{4}",
                Input::Delete { .. } | Input::Indent => "",
            });
        }
        shared.registers.set_special('.', typed);
        if !self.replaying {
            shared.repeat = Some(Repeat {
                command: session.command,
                inputs: session.inputs,
                extent: session.extent,
            });
        }
    }
}
