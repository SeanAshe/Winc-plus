//! Vim 模式的状态机：模式、按键的处理、命令的执行。
//!
//! [`Vim`] 是一个窗口的状态，[`Shared`] 是各窗口共用的状态（寄存器、上一次修改、上一次行内
//! 查找）。按键经解析器得到命令，命令在 [`Host`] 提供的文本上求值，修改与选区交给宿主应用。
//! 插入模式中的输入由宿主执行，经 [`Vim::typed`] 记录，供次数与点重复重放。

mod block;
mod edit;
mod ex;
mod global;
mod insert;
mod macros;
mod map;
mod marks;
mod multi;
mod number;
mod operate;
mod options;
mod search;
mod sort;
mod substitute;
mod surround;
mod view;

pub use map::{Mapped, Mapping};
pub(crate) use options::option_names;
pub use options::{Options, option_list};
pub use search::Prompt;

use std::collections::VecDeque;

use winc_core::{Change, Lines, Selection, movement};

use crate::command::{Action, Mode, Motion, Operator, TextObject};
use crate::host::{AppCommand, Host, Input, Message};
use crate::key::{Code, Key, Mods, Named};
use crate::motion::{self, Env, Find, Moved, Want, WantUpdate};
use crate::object::{self, At, Becomes};
use crate::parser::{Body, Command, Movement, Outcome, Parser};
use crate::register::{Clipboard, Content, Kind, Registers, Source};
use crate::text;

/// 一次命令（次数、粘贴、重复插入）生成的文本的上限（字节）；超出时命令不执行。
///
/// 取自调参：16 MiB 远大于正常使用时生成的文本，且不会使撤销栈一次占满上限。
pub(crate) const MAX_OUTPUT: usize = 16 << 20;

/// 命令已经失败并提示过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failed;

/// 可视模式的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualKind {
    /// 按字符。
    Char,
    /// 按行。
    Line,
    /// 按块。
    Block,
}

/// 可视模式的选区：两端字符的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visual {
    /// 种类。
    kind: VisualKind,
    /// 起点字符的位置。
    anchor: usize,
    /// 光标所在字符的位置；可以位于行尾。
    cursor: usize,
}

/// 可视模式下的修改覆盖的范围，供点重复在光标处重建同等的选区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    /// 种类。
    kind: VisualKind,
    /// 覆盖的行数，不小于 1。
    lines: usize,
    /// 只有一行时为覆盖的显示列数；多行时为末行上选区终点的显示列；按块时为块的宽度；
    /// `usize::MAX` 表示选区延伸到末行的行尾（含换行符），按块时延伸到各行的行尾。
    cells: usize,
}

/// 上一次修改的记录，供 `.` 重放。
#[derive(Debug, Clone)]
struct Repeat {
    /// 命令。
    command: Command,
    /// 命令进入插入模式之后执行的输入。
    inputs: Vec<Input>,
    /// 命令在可视模式下执行时选区覆盖的范围。
    extent: Option<Extent>,
}

/// 各窗口共用的状态。
#[derive(Debug, Default)]
pub struct Shared {
    /// 寄存器。
    pub registers: Registers,
    /// 系统剪贴板与无名寄存器的同步方式。
    pub clipboard: Clipboard,
    /// 查找时是否忽略大小写。
    pub ignore_case: bool,
    /// 忽略大小写时，模式中有大写字母是否改为区分大小写。
    pub smart_case: bool,
    /// 上一次行内查找。
    last_find: Option<Find>,
    /// 上一次修改。
    repeat: Option<Repeat>,
    /// 上一次查找。
    search: Option<search::Searched>,
    /// 查找历史，从旧到新排列。
    history: Vec<String>,
    /// 多个光标逐个执行命令期间，各光标写入寄存器的内容暂存于此，结束时合成一份。
    capture: Option<Vec<(Content, Source)>>,
    /// 多个光标逐个粘贴期间，当前光标读到的寄存器内容。
    piece: Option<Content>,
    /// 最近一次由多个光标写入寄存器的各段文本；寄存器的内容仍是这些文本时，同样个数的光标
    /// 粘贴时逐个对应。
    pieces: Option<Vec<String>>,
    /// `:set` 可以修改的选项。
    pub options: Options,
    /// 命令行的历史，从旧到新排列。
    commands: Vec<String>,
    /// 上一次执行的命令行。
    last_command: Option<String>,
    /// 上一次替换。
    substitute: Option<substitute::Substitution>,
    /// 上一次回放的寄存器名。
    last_macro: Option<char>,
    /// 用户的映射。
    mappings: Vec<Mapping>,
    /// 最近一次按键中是否有命令失败；回放按键时据此中止。
    failed: bool,
}

/// 一个窗口的 Vim 状态。
#[derive(Debug)]
pub struct Vim {
    /// 当前的模式：普通、可视或插入。
    mode: Mode,
    /// 可视模式的选区；不在可视模式时为 `None`。
    visual: Option<Visual>,
    /// 未完成的命令。
    parser: Parser,
    /// 上下移动时保持的期望列。
    want: Want,
    /// 进行中的插入；不在插入模式时为 `None`。
    insert: Option<insert::Session>,
    /// 进行中的插入是否持有一个未结束的撤销分组。
    insert_group: bool,
    /// 上一次可视模式的选区，供 `gv` 恢复。
    last_visual: Option<Visual>,
    /// 最近一次交给宿主的选区；宿主的选区与之不同时说明选区被其他操作改动过。
    shown: Option<Selection>,
    /// 是否正在重放上一次修改；重放期间不更新记录。
    replaying: bool,
    /// 等待宿主提供的输入。
    prompt: Option<search::Pending>,
    /// 命令执行期间保存的跳转之前的上下文。
    context: Option<marks::Context>,
    /// 修改列表中当前的位置；`None` 表示在列表的末尾之后。
    change_index: Option<usize>,
    /// 半屏滚动的行数；未用次数指定时为 `None`，取窗口高度的一半。
    half: Option<usize>,
    /// 插入模式中执行一条普通模式命令期间保存的状态。
    oneshot: Option<insert::Oneshot>,
    /// 重放按块的修改时块左右边界的显示列；由下一条作用于块的命令取用。
    columns: Option<(usize, usize)>,
    /// 重放可视模式下的修改时所依据的范围；由可视模式的 `I`、`A` 取用。
    replayed: Option<Extent>,
    /// 宿主中主选区之外的选区是否由状态机给出（按块的选区）；下一次设置选区时收回。
    owned: bool,
    /// 是否正在等待宿主输入命令行。
    cmdline: bool,
    /// 等待输入期间由状态机自行处理按键时已输入的一行；等待命令行时开头是预填的范围。
    line: String,
    /// 行内输入的下一个按键是否按字面加入。
    line_literal: bool,
    /// 进行中的宏记录。
    recording: Option<macros::Recording>,
    /// 等待回放的按键，以及各按键是否经过用户的映射。
    typeahead: VecDeque<(Key, bool)>,
    /// 回放的预算；最外层的一次按键结束时清除。
    budget: Option<macros::Budget>,
    /// 回放的上限：按键数与时间。
    limits: (usize, std::time::Duration),
    /// 回放是否已因超过预算而中止。
    aborted: bool,
    /// 最近一次按键中是否有命令失败。
    failed: bool,
    /// 进行中的逐处确认。
    confirm: Option<substitute::Confirm>,
    /// 是否正在逐行执行 `:g` 的命令。
    in_global: bool,
    /// 命令行中正在执行的是否为以 `|` 分隔的最后一条命令。
    ex_last: bool,
    /// 等待映射的后续按键期间保存的按键，以及各按键是否已显示在文本里。
    mapped: Vec<(Key, bool)>,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            visual: None,
            parser: Parser::default(),
            want: Want::Cell(0),
            insert: None,
            insert_group: false,
            last_visual: None,
            shown: None,
            replaying: false,
            prompt: None,
            context: None,
            change_index: None,
            half: None,
            oneshot: None,
            columns: None,
            replayed: None,
            owned: false,
            cmdline: false,
            line: String::new(),
            line_literal: false,
            recording: None,
            typeahead: VecDeque::new(),
            budget: None,
            limits: (macros::MAX_STEPS, macros::MAX_TIME),
            aborted: false,
            failed: false,
            confirm: None,
            in_global: false,
            ex_last: true,
            mapped: Vec::new(),
        }
    }
}

impl Vim {
    /// 返回当前的模式：[`Mode::Normal`]、[`Mode::Visual`] 或 [`Mode::Insert`]。
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 返回可视模式的种类；不在可视模式时返回 `None`。
    pub fn visual_kind(&self) -> Option<VisualKind> {
        self.visual.map(|visual| visual.kind)
    }

    /// 是否已输入操作符、正在等待动作。
    pub fn is_operator_pending(&self) -> bool {
        self.parser.is_operator_pending()
    }

    /// 是否正在插入模式中执行一条普通模式的命令：命令结束后回到插入模式，期间光标可以停在
    /// 行尾之后。
    pub fn is_suspended(&self) -> bool {
        self.oneshot.is_some()
    }

    /// 是否处于替换模式：[`Vim::mode`] 为 [`Mode::Insert`]，键入的文字覆盖已有的字符。
    pub fn is_replace(&self) -> bool {
        self.insert.as_ref().is_some_and(insert::Session::replaces)
    }

    /// 返回未完成命令已输入的按键，用于显示。
    pub fn pending(&self) -> &[Key] {
        self.parser.pending()
    }

    /// 返回光标所在字符的位置：可视模式下是选区中光标一端的字符，其余模式下是选区的端点。
    pub fn caret(&self, host: &impl Host) -> usize {
        match self.visual {
            Some(visual) => visual.cursor.min(host.text().end_pos()),
            None => host.selection().head,
        }
    }

    /// 处理按键 `key`，返回是否已由 Vim 层处理；返回假时按键交给宿主的其他部分。
    ///
    /// 普通与可视模式下，没有定义的可见字符同样算作已处理，不会成为输入的文字。按键先经过
    /// 用户的映射；命令引发的回放在返回之前执行完毕。
    pub fn key(&mut self, shared: &mut Shared, host: &mut impl Host, key: Key) -> bool {
        self.begin_key(shared);
        self.record(key);
        let handled = self.key_mapped(shared, host, key);
        self.finish_key(shared, host);
        handled
    }

    /// 最外层的一次按键开始时清除失败与中止的状态。
    fn begin_key(&mut self, shared: &mut Shared) {
        self.failed = false;
        self.aborted = false;
        shared.failed = false;
    }

    /// 最外层的一次按键结束时回放队列中的按键，并清除预算。
    ///
    /// 一次回放的全部修改属于一个撤销单元。
    fn finish_key(&mut self, shared: &mut Shared, host: &mut impl Host) {
        if !self.typeahead.is_empty() {
            host.begin_group();
            self.drain(shared, host, 0);
            host.end_group();
        }
        self.budget = None;
    }

    /// 提示出错的 `message`，并记下命令已失败。
    fn error(&mut self, host: &mut impl Host, message: Message) {
        self.failed = true;
        host.message(message);
    }

    /// 处理不经过映射的按键 `key`，返回是否已由 Vim 层处理。
    fn key_raw(&mut self, shared: &mut Shared, host: &mut impl Host, key: Key) -> bool {
        if self.confirm.is_some() {
            self.confirm_key(host, key);
            self.resume_if_done(host);
            return true;
        }
        // 等待输入期间按键由宿主的输入框处理；收到按键说明输入已被放弃。
        self.prompt_cancel(host);
        self.sync(host);
        if self.recording.is_some() && self.stop_recording(shared, key) {
            return true;
        }
        if self.mode == Mode::Insert {
            return self.insert_key(shared, host, key);
        }
        let handled = match self.parser.feed(self.mode, key) {
            Outcome::Pending => return true,
            Outcome::Command(command) => {
                self.run(shared, host, command);
                true
            }
            Outcome::Unsupported => {
                self.error(host, Message::Unsupported);
                true
            }
            Outcome::Cancelled => true,
            Outcome::Invalid => {
                self.failed = true;
                true
            }
            Outcome::Unbound => {
                self.failed = true;
                swallowed(key)
            }
        };
        if self.resume_if_done(host) {
            return true;
        }
        handled
    }

    /// 插入模式中执行的一条命令已经结束时回到插入模式并返回真；命令仍在等待输入或逐处确认时
    /// 不结束。
    fn resume_if_done(&mut self, host: &mut impl Host) -> bool {
        let done = self.oneshot.is_some() && self.prompt().is_none() && self.confirm.is_none();
        if done {
            self.resume_insert(host);
        }
        done
    }

    /// 丢弃未完成的命令；焦点离开编辑区时调用。
    pub fn cancel_pending(&mut self) {
        self.parser.reset();
        self.mapped.clear();
    }

    /// 回到普通模式并结束未完成的撤销分组；切换文档之前对原文档调用。
    pub fn reset(&mut self, host: &mut impl Host) {
        self.parser.reset();
        if self.insert_group {
            host.end_group();
            self.insert_group = false;
        }
        if self.confirm.take().is_some() {
            host.end_group();
        }
        self.insert = None;
        self.oneshot = None;
        self.prompt = None;
        self.cmdline = false;
        self.line.clear();
        self.typeahead.clear();
        self.mapped.clear();
        self.in_global = false;
        self.context = None;
        self.change_index = None;
        self.last_visual = None;
        self.mode = Mode::Normal;
        // 可视模式的选区收回为光标，否则下一次同步会重新进入可视模式。
        match self.visual.take() {
            Some(visual) => self.settle(host, visual.cursor),
            None => self.shown = None,
        }
    }

    /// 回到普通模式并丢弃全部状态，不访问宿主；原文档已经关闭时调用。
    pub fn detach(&mut self) {
        self.parser.reset();
        self.insert = None;
        self.insert_group = false;
        self.oneshot = None;
        self.prompt = None;
        self.cmdline = false;
        self.line.clear();
        self.typeahead.clear();
        self.mapped.clear();
        self.confirm = None;
        self.in_global = false;
        self.context = None;
        self.change_index = None;
        self.last_visual = None;
        self.columns = None;
        self.replayed = None;
        self.owned = false;
        self.visual = None;
        self.mode = Mode::Normal;
        self.shown = None;
    }

    /// 使 Vim 层的状态与宿主的选区一致；选区被 Vim 层之外的操作（鼠标、应用的命令）改动之后
    /// 调用。
    ///
    /// 普通模式下选区非空时进入可视模式，可视模式下选区变空时回到普通模式；普通模式的光标
    /// 收拢到字符上。插入模式不受影响。
    pub fn sync(&mut self, host: &mut impl Host) {
        let selection = host.selection();
        if self.mode == Mode::Insert || self.shown == Some(selection) {
            return;
        }
        // 选区被其他操作改动：逐处确认到此结束。
        if self.confirm.take().is_some() {
            host.end_group();
        }
        let text = host.text();
        let end = text.end_pos();
        let (anchor, head) = (selection.anchor.min(end), selection.head.min(end));
        if anchor == head {
            self.visual = None;
            self.mode = Mode::Normal;
            self.settle(host, head);
            return;
        }
        // 选区是字符之间的位置，可视模式记录两端的字符。
        let (anchor, cursor) = if head > anchor {
            (anchor, movement::prev_grapheme(text, head))
        } else {
            (movement::prev_grapheme(text, anchor), head)
        };
        self.visual = Some(Visual {
            kind: VisualKind::Char,
            anchor,
            cursor,
        });
        self.mode = Mode::Visual;
        self.want = Want::Cell(text::want_cell(text, cursor, host.tab_width()));
        self.shown = Some(selection);
    }

    /// 执行一条命令；命令的全部修改属于一个撤销单元，进入插入模式时延续到离开为止。
    fn run(&mut self, shared: &mut Shared, host: &mut impl Host, command: Command) {
        if self.ask(host, command) {
            return;
        }
        host.begin_group();
        match self.cursors(host, command) {
            Some((all, primary)) => self.execute_each(shared, host, command, &all, primary),
            None => self.execute(shared, host, command),
        }
        self.check_context(host);
        if self.mode == Mode::Insert {
            self.insert_group = true;
        } else {
            host.end_group();
        }
    }

    /// 执行命令 `command`，不处理撤销分组。
    fn execute(&mut self, shared: &mut Shared, host: &mut impl Host, command: Command) {
        match command.body {
            Body::Motion(movement) => self.motion(shared, host, movement, command.count),
            Body::Operator {
                operator,
                target,
                force,
            } => self.operate(shared, host, command, operator, target, force),
            Body::Action { action, arg } => self.act(shared, host, command, action, arg),
            Body::Object(object) => self.select_object(shared, host, object, command.count),
        }
    }

    /// 求值动作 `movement`：查找与标记由状态机求值，其余只依赖文本与视图。
    ///
    /// 属于跳转的动作在求值成功时记下跳转之前的位置。
    fn moved(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        movement: Movement,
        count: Option<u32>,
        operator: Option<Operator>,
    ) -> Option<Moved> {
        let from = self.caret(host);
        let moved = match movement.motion {
            // 模式由宿主输入之后改为重复上一次查找；没有模式时不执行。
            Motion::SearchForward | Motion::SearchBackward => None,
            Motion::SearchNext | Motion::SearchPrevious => {
                let reverse = movement.motion == Motion::SearchPrevious;
                self.search_again(shared, host, from, count, reverse)
            }
            Motion::SearchWordForward
            | Motion::SearchWordBackward
            | Motion::SearchPartForward
            | Motion::SearchPartBackward => {
                self.search_word(shared, host, from, count, movement.motion)
            }
            Motion::Mark | Motion::MarkLine => self.mark_motion(host, movement),
            _ => {
                let env = Env {
                    host: &*host,
                    from,
                    want: self.want,
                    count,
                    operator,
                    visual: self.visual.is_some() && operator.is_none(),
                };
                motion::evaluate(&env, movement, &mut shared.last_find)
            }
        }?;
        if movement.motion.is_jump() {
            self.set_context(host, from);
        }
        Some(moved)
    }

    /// 执行不带操作符的动作：移动光标，可视模式下延伸选区。
    fn motion(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        movement: Movement,
        count: Option<u32>,
    ) {
        // 不在当前文档中的大写标记由宿主跳转到所在的文档。
        if let (Motion::Mark | Motion::MarkLine, Some(name)) = (movement.motion, movement.arg)
            && name.is_ascii_uppercase()
            && host.mark(name).is_none()
        {
            let line = movement.motion == Motion::MarkLine;
            if !host.app(AppCommand::GlobalMark { name, line }) {
                self.error(host, Message::NoMark(name));
            }
            return;
        }
        match self.moved(shared, host, movement, count, None) {
            Some(moved) => {
                self.failed |= moved.failed;
                self.goto(host, moved.pos, moved.want);
            }
            None => self.failed = true,
        }
    }

    /// 可视模式下把选区扩展到文本对象 `object`。
    fn select_object(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        object: TextObject,
        count: Option<u32>,
    ) {
        let Some(visual) = self.visual else {
            return;
        };
        let count = count.map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let forward = match object.shape {
            crate::command::Shape::NextMatch => Some(true),
            crate::command::Shape::PreviousMatch => Some(false),
            _ => None,
        };
        if let Some(forward) = forward {
            self.select_match(shared, host, forward, count);
            return;
        }
        let at = At {
            cursor: visual.cursor,
            anchor: Some(visual.anchor),
            linewise: visual.kind == VisualKind::Line,
            count: count.min(motion::MAX_STEPS),
            tab_width: host.tab_width(),
        };
        let Some(selected) = object::select(host.text(), object, at) else {
            self.failed = true;
            return;
        };
        let kind = match selected.becomes {
            Becomes::Same => visual.kind,
            Becomes::CharUnlessBlock if visual.kind == VisualKind::Line => VisualKind::Char,
            Becomes::CharUnlessBlock => visual.kind,
            Becomes::Char => VisualKind::Char,
            Becomes::Line => VisualKind::Line,
        };
        self.visual = Some(Visual {
            kind,
            anchor: selected.start,
            cursor: selected.end,
        });
        self.goto(host, selected.end, WantUpdate::Recompute);
    }

    /// 把光标移到 `pos` 并按 `want` 更新期望列；可视模式下移动选区的光标一端。
    fn goto(&mut self, host: &mut impl Host, pos: usize, want: WantUpdate) {
        let text = host.text();
        let pos = match &mut self.visual {
            Some(visual) => {
                let line = text::line_of(text, pos);
                let pos = pos.clamp(text.line_start(line), text.line_end(line));
                visual.cursor = movement::snap_to_grapheme(text, pos);
                visual.cursor
            }
            // 插入模式中执行一条命令期间，光标可以停在行尾之后。
            None if self.oneshot.is_some() => {
                let line = text::line_of(text, pos);
                let pos = pos.clamp(text.line_start(line), text.line_end(line));
                movement::snap_to_grapheme(text, pos)
            }
            None => text::on_char(text, pos),
        };
        match want {
            WantUpdate::Keep => {}
            WantUpdate::Set(want) => self.want = want,
            WantUpdate::Recompute => {
                self.want = Want::Cell(text::want_cell(text, pos, host.tab_width()));
            }
        }
        self.show(host, pos);
    }

    /// 把普通模式的光标放到 `pos` 所在的字符上，并重新计算期望列。
    fn settle(&mut self, host: &mut impl Host, pos: usize) {
        self.goto(host, pos, WantUpdate::Recompute);
    }

    /// 把选区交给宿主：可视模式下是包含两端字符的选区，否则是位于 `pos` 的光标。
    fn show(&mut self, host: &mut impl Host, pos: usize) {
        let text = host.text();
        let selection = match self.visual {
            None => Selection::point(pos),
            Some(visual) if visual.kind == VisualKind::Block => {
                // 按块的选区每行一个，主选区在光标所在的行。
                let to_end = self.want == Want::End;
                let (selections, primary) = block::selections(host, visual, to_end);
                let selection = selections.get(primary).copied();
                host.set_selections(selections, primary);
                self.owned = true;
                self.shown = selection;
                return;
            }
            Some(visual) => {
                let (low, high) = (
                    visual.anchor.min(visual.cursor),
                    visual.anchor.max(visual.cursor),
                );
                let (from, to) = match visual.kind {
                    VisualKind::Char => (low, text::after(text, high)),
                    _ => {
                        let first = text::line_of(text, low);
                        let last = text::line_of(text, high);
                        (text.line_start(first), text.line_span(last).end)
                    }
                };
                if visual.cursor >= visual.anchor {
                    Selection::new(from, to)
                } else {
                    Selection::new(to, from)
                }
            }
        };
        self.place(host, selection);
    }

    /// 把主选区设为 `selection`；其余选区由状态机给出时一并收回。
    fn place(&mut self, host: &mut impl Host, selection: Selection) {
        if std::mem::take(&mut self.owned) {
            host.set_selections(vec![selection], 0);
        } else {
            host.set_selection(selection);
        }
        self.shown = Some(selection);
    }

    /// 应用一组修改；`at` 是修改之前光标所在的位置，`first_line` 是第一处变化所在的行。
    ///
    /// 撤销之后的光标按以下规则填写：`at` 在第一处变化所在的行或其上一行时回到 `at`，否则位于
    /// 该行的第一个非空白字符。返回宿主是否接受了修改。
    fn edit(
        &mut self,
        host: &mut impl Host,
        changes: Vec<Change>,
        at: usize,
        first_line: usize,
    ) -> bool {
        if changes.is_empty() {
            return true;
        }
        let text = host.text();
        let at_line = text::line_of(text, at);
        let line = if at_line + 1 == first_line {
            at_line
        } else {
            first_line
        };
        let before = if line > text::last_line(text) {
            text.line_start(text::last_line(text))
        } else if line == at_line {
            at
        } else {
            text::first_non_blank(text, line)
        };
        // 修改按位置从后往前排列：最后一项是最靠前的一处，第一项是最靠后的一处。
        let from = changes
            .iter()
            .map(|change| change.start)
            .min()
            .unwrap_or(at);
        let grown: isize = (changes.iter().skip(1))
            .map(|change| {
                let inserted = isize::try_from(change.inserted.chars().count()).unwrap_or(0);
                inserted - isize::try_from(change.removed.chars().count()).unwrap_or(0)
            })
            .sum();
        let to = changes.first().map_or(from, |change| {
            let end = change.start + change.inserted.chars().count().saturating_sub(1);
            end.saturating_add_signed(grown)
        });
        let applied = host.apply(changes, before, before);
        if applied {
            self.note_change(host, from, to.max(from));
        } else {
            self.error(host, Message::TooLarge);
        }
        self.shown = None;
        applied
    }

    /// 文档只读时提示并返回真。
    fn blocked(&mut self, host: &mut impl Host) -> bool {
        let blocked = host.read_only();
        if blocked {
            self.error(host, Message::ReadOnly);
        }
        blocked
    }

    /// 离开可视模式，记下选区供 `gv` 与 `<`、`>` 标记使用；光标的位置由调用方设置。
    fn end_visual(&mut self, host: &mut impl Host) {
        if let Some(visual) = self.visual.take() {
            self.last_visual = Some(visual);
            let text = host.text();
            let (mut low, mut high) = (
                visual.anchor.min(visual.cursor),
                visual.anchor.max(visual.cursor),
            );
            if visual.kind == VisualKind::Line {
                low = text.line_start(text::line_of(text, low));
                high = text.line_end(text::line_of(text, high));
            }
            host.set_mark('<', low);
            host.set_mark('>', high);
        }
        self.mode = Mode::Normal;
    }

    /// 读取寄存器 `name` 的内容；`None` 表示未指定。
    fn read_register(
        shared: &mut Shared,
        host: &mut impl Host,
        name: Option<char>,
    ) -> Option<Content> {
        let read_clipboard = |shared: &Shared, host: &mut dyn FnMut() -> Option<String>| {
            host().map(|text| shared.registers.read_clipboard(&text))
        };
        if let Some(piece) = &shared.piece {
            return Some(piece.clone());
        }
        match name {
            Some('_') => None,
            Some('+' | '*') => read_clipboard(shared, &mut || host.clipboard()),
            Some('%') => host.file_name().map(|text| Content {
                kind: Kind::Charwise,
                text,
            }),
            None | Some('"') if shared.clipboard == Clipboard::Always => {
                read_clipboard(shared, &mut || host.clipboard())
                    .or_else(|| shared.registers.get(None).cloned())
            }
            name => shared.registers.get(name).cloned(),
        }
    }

    /// 把 `content` 写入寄存器 `name`；寄存器只读时提示并返回假。
    fn write_register(
        shared: &mut Shared,
        host: &mut impl Host,
        name: Option<char>,
        content: Content,
        source: Source,
    ) -> bool {
        if let Some(name @ ('.' | ':' | '/' | '%')) = name {
            shared.failed = true;
            host.message(Message::ReadOnlyRegister(name));
            return false;
        }
        if let Some(capture) = &mut shared.capture {
            capture.push((content, source));
            return true;
        }
        match name {
            Some('_') => {}
            Some('+' | '*') => {
                let text = shared.registers.write_clipboard(&content);
                host.set_clipboard(&text);
            }
            name => {
                let unnamed = name.is_none_or(|name| name == '"');
                let synced = match shared.clipboard {
                    Clipboard::Register => false,
                    Clipboard::Yank => source == Source::Yank,
                    Clipboard::Always => true,
                };
                if unnamed && synced {
                    let text = shared.registers.write_clipboard(&content);
                    host.set_clipboard(&text);
                }
                shared.registers.write(name, content, source);
            }
        }
        true
    }

    /// 记录上一次修改；重放期间不记录。
    fn remember(&mut self, shared: &mut Shared, command: Command, extent: Option<Extent>) {
        if !self.replaying {
            shared.repeat = Some(Repeat {
                command,
                inputs: Vec::new(),
                extent,
            });
        }
    }

    /// 执行不带范围的命令。
    fn act(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        action: Action,
        arg: Option<char>,
    ) {
        match action {
            Action::Insert
            | Action::Append
            | Action::InsertFirstNonBlank
            | Action::InsertLineStart
            | Action::AppendLineEnd
            | Action::InsertAtLast
            | Action::OpenBelow
            | Action::OpenAbove => self.start_insert(host, command, action),
            Action::ReplaceChar => self.replace_char(shared, host, command, arg),
            Action::ToggleCaseChar => self.toggle_case_char(shared, host, command),
            Action::PasteAfter
            | Action::PasteBefore
            | Action::PasteAfterMove
            | Action::PasteBeforeMove
            | Action::PasteIndented => self.paste(shared, host, command, action),
            Action::JoinLines | Action::JoinLinesRaw => {
                self.join_lines(shared, host, command, action == Action::JoinLines);
            }
            Action::Undo | Action::Redo => {
                let times = command.count.unwrap_or(1);
                // 重做之后光标回到修改之前所在的行与列，而不是修改之后的位置；该行已不存在时
                // 放在最后一行的行首。
                let mut origin = None;
                for _ in 0..times {
                    if action == Action::Undo {
                        if !host.undo() {
                            break;
                        }
                    } else {
                        let text = host.text();
                        let before = host.redo_origin().map(|pos| text.pos_to_line_col(pos));
                        if !host.redo() {
                            break;
                        }
                        origin = before;
                    }
                }
                self.shown = None;
                let text = host.text();
                let pos = match origin {
                    Some((line, _)) if line > text::last_line(text) => {
                        text.line_start(text::last_line(text))
                    }
                    Some((line, col)) => text.line_col_to_pos(line, col),
                    None => host.selection().head.min(text.end_pos()),
                };
                self.settle(host, pos);
            }
            Action::Repeat => self.repeat(shared, host, command.count),
            Action::Visual | Action::VisualLine | Action::VisualBlock => {
                let kind = match action {
                    Action::Visual => VisualKind::Char,
                    Action::VisualLine => VisualKind::Line,
                    _ => VisualKind::Block,
                };
                self.toggle_visual(host, kind);
            }
            Action::Reselect => self.reselect(host),
            Action::SwapAnchor | Action::SwapColumn => self.swap_ends(host, action),
            Action::Escape | Action::Interrupt => {
                if let Some(visual) = self.visual {
                    self.end_visual(host);
                    self.settle(host, visual.cursor);
                }
            }
            Action::InsertSelection | Action::AppendSelection => {
                self.insert_selection(host, command, action == Action::AppendSelection);
            }
            Action::Replace => self.start_replace(host, command),
            Action::Increment | Action::Decrement => {
                self.add(shared, host, command, action == Action::Decrement);
            }
            Action::SetMark => self.set_mark(host, arg),
            Action::JumpBack | Action::JumpForward => {
                self.jump(host, action == Action::JumpBack, command.count);
            }
            Action::ChangeOlder | Action::ChangeNewer => {
                self.change_jump(host, action == Action::ChangeOlder, command.count);
            }
            Action::SelectNext | Action::SelectPrevious => {
                let count = command
                    .count
                    .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
                self.select_match(shared, host, action == Action::SelectNext, count);
            }
            Action::Scroll(scrolling) => self.scroll(host, scrolling, command.count),
            Action::Fold(folding) => self.fold(host, folding, command.count),
            // 插入模式的命令不经此处执行。
            Action::InsertBackspace
            | Action::InsertDeleteWord
            | Action::InsertDeleteLine
            | Action::InsertIndent
            | Action::InsertOutdent
            | Action::InsertRegister
            | Action::InsertNormal
            | Action::InsertBelow
            | Action::InsertAbove => {}
            Action::DeleteSurround => self.unwrap(shared, host, command, arg, None),
            Action::ChangeSurround(old) => self.unwrap(shared, host, command, Some(old), arg),
            Action::Focus(_) | Action::Alternate => self.app_action(host, action, command.count),
            // 由 `ask` 处理。
            Action::CommandLine => {}
            Action::Record => self.start_recording(arg),
            Action::Play => self.play_register(shared, host, command.count, arg),
            Action::RepeatSubstitute | Action::RepeatSubstituteAll => {
                self.repeat_substitute(shared, host, action == Action::RepeatSubstituteAll);
            }
            Action::Definition
            | Action::OpenFile
            | Action::TabNext
            | Action::TabPrevious
            | Action::WriteQuit
            | Action::QuitForce => self.app_action(host, action, command.count),
            Action::CharInfo => {
                let c = host.text().char_at(self.caret(host)).filter(|c| *c != '\n');
                match c {
                    Some(c) => host.message(Message::Char(c)),
                    None => self.failed = true,
                }
            }
            Action::Later | Action::ExMode | Action::Keyword | Action::UndoLine => {
                self.error(host, Message::Unsupported);
            }
        }
    }

    /// 交给宿主执行的按键命令：转到定义、打开路径、切换标签、保存并关闭。
    fn app_action(&mut self, host: &mut impl Host, action: Action, count: Option<u32>) {
        let number = count.map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        let step = isize::try_from(number.unwrap_or(1)).unwrap_or(isize::MAX);
        let command = match action {
            Action::Definition => AppCommand::Definition,
            Action::OpenFile => AppCommand::OpenFile,
            Action::Focus(focusing) => AppCommand::Focus(focusing),
            Action::Alternate => AppCommand::Alternate,
            Action::TabNext => match number {
                Some(number) => AppCommand::BufferNumber(number),
                None => AppCommand::BufferStep(1),
            },
            Action::TabPrevious => AppCommand::BufferStep(-step),
            Action::WriteQuit => AppCommand::Quit {
                write: true,
                force: false,
            },
            _ => AppCommand::Quit {
                write: false,
                force: true,
            },
        };
        if let Some(visual) = self.visual {
            self.end_visual(host);
            self.settle(host, visual.cursor);
        }
        if !host.app(command) {
            self.error(host, Message::Unsupported);
        }
    }

    /// 可视模式的 `o`、`O`：交换选区的起点与光标；按块时 `O` 只交换两者所在的显示列。
    fn swap_ends(&mut self, host: &mut impl Host, action: Action) {
        let Some(visual) = self.visual else {
            return;
        };
        let (anchor, cursor) = if visual.kind == VisualKind::Block && action == Action::SwapColumn {
            // 起点移到块的左边界、光标移到右边界；光标因此没有移动时两者对调。
            let text = host.text();
            let tab_width = host.tab_width();
            let block = block::block_of(host, visual, false, None);
            let at = |pos: usize, cell: usize| {
                text::pos_at_cell(text, text::line_of(text, pos), cell, tab_width)
            };
            let cursor = at(visual.cursor, block.right);
            if cursor == visual.cursor {
                (
                    at(visual.anchor, block.right),
                    at(visual.cursor, block.left),
                )
            } else {
                (at(visual.anchor, block.left), cursor)
            }
        } else {
            (visual.cursor, visual.anchor)
        };
        self.visual = Some(Visual {
            anchor,
            cursor,
            ..visual
        });
        self.goto(host, cursor, WantUpdate::Recompute);
    }

    /// 进入、切换或离开可视模式。
    fn toggle_visual(&mut self, host: &mut impl Host, kind: VisualKind) {
        let cursor = self.caret(host);
        match &mut self.visual {
            Some(visual) if visual.kind == kind => {
                self.end_visual(host);
                self.settle(host, cursor);
            }
            Some(visual) => {
                visual.kind = kind;
                self.show(host, cursor);
            }
            None => {
                self.visual = Some(Visual {
                    kind,
                    anchor: cursor,
                    cursor,
                });
                self.mode = Mode::Visual;
                self.show(host, cursor);
            }
        }
    }

    /// `gv`：恢复上一次的可视选区；已在可视模式时与当前的选区交换。
    fn reselect(&mut self, host: &mut impl Host) {
        let Some(mut last) = self.last_visual else {
            return;
        };
        let text = host.text();
        let clamp = |pos: usize| {
            let line = text::line_of(text, pos.min(text.end_pos()));
            pos.clamp(text.line_start(line), text.line_end(line))
        };
        last.anchor = clamp(last.anchor);
        last.cursor = clamp(last.cursor);
        if let Some(current) = self.visual {
            self.last_visual = Some(current);
        }
        self.visual = Some(last);
        self.mode = Mode::Visual;
        self.goto(host, last.cursor, WantUpdate::Recompute);
    }

    /// `.`：重放上一次修改；`count` 给出时替换原来的次数。
    fn repeat(&mut self, shared: &mut Shared, host: &mut impl Host, count: Option<u32>) {
        let Some(mut repeat) = shared.repeat.clone() else {
            return;
        };
        if count.is_some() {
            repeat.command.count = count;
        }
        // 重复从编号寄存器粘贴时改用下一个编号，依次取出更早删除的文本。
        if let (Some(digit @ '1'..='8'), Body::Action { .. }) =
            (repeat.command.register, repeat.command.body)
        {
            repeat.command.register = char::from_digit(digit.to_digit(10).unwrap_or(0) + 1, 10);
        }
        if let Some(stored) = &mut shared.repeat {
            stored.command = repeat.command;
        }
        if self.visual.is_some() {
            return;
        }
        if let Some(extent) = repeat.extent {
            self.select_extent(host, extent);
        }
        self.replayed = repeat.extent;
        self.replaying = true;
        self.execute(shared, host, repeat.command);
        self.columns = None;
        self.replayed = None;
        if self.mode == Mode::Insert {
            for input in &repeat.inputs {
                self.play(host, input);
            }
            self.leave_insert(shared, host);
        }
        self.replaying = false;
    }

    /// 在光标处建立与 `extent` 同等范围的可视选区。
    fn select_extent(&mut self, host: &mut impl Host, extent: Extent) {
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let last = (line + extent.lines - 1).min(text::last_line(text));
        let tab_width = host.tab_width();
        let end = if extent.kind == VisualKind::Block {
            // 按块：从光标所在的显示列起同样宽度的块，不受末行长度的影响。
            let (_, col) = text.pos_to_line_col(cursor);
            let left = text::cell_of(text, line, col, tab_width);
            if extent.cells == usize::MAX {
                self.want = Want::End;
                text.line_end(last)
            } else {
                let right = left + extent.cells - 1;
                self.columns = Some((left, right));
                text::pos_at_cell(text, last, right, tab_width).min(text::last_char(text, last))
            }
        } else if extent.cells == usize::MAX {
            text.line_end(last)
        } else if extent.lines == 1 {
            // 行尾算作一列：选区可以延伸到行尾，把换行符包括在内。
            let cell = text::want_cell(text, cursor, tab_width) + extent.cells - 1;
            text::pos_at_cell(text, line, cell, tab_width)
        } else {
            text::pos_at_cell(text, last, extent.cells, tab_width)
        };
        self.visual = Some(Visual {
            kind: extent.kind,
            anchor: cursor,
            cursor: end,
        });
        self.mode = Mode::Visual;
    }
}

/// 普通与可视模式下没有定义的按键是否由 Vim 层吞掉：可见字符不交给宿主，其余按键交给宿主。
fn swallowed(key: Key) -> bool {
    let plain = key.mods().without(Mods::SHIFT).is_empty();
    match key.code() {
        Code::Char(_) => plain,
        Code::Named(named) => {
            plain
                && matches!(
                    named,
                    Named::Tab | Named::Enter | Named::Backspace | Named::Delete
                )
        }
    }
}
