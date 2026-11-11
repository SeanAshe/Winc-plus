//! 命令解析器：把按键逐个解析成命令（[`Command`]）。
//!
//! 普通与可视模式下的命令依次由寄存器、次数、操作符、次数、动作或文本对象组成，各部分都可以
//! 省略；寄存器与次数可以出现多次，各段次数相乘，寄存器取最后一个。按键序列与记录的对应取自
//! 命令表，解析器只保存尚未完成的命令。

use crate::command::{Action, Arg, Entry, Force, Kind, Lookup, Mode, Motion, Operator};
use crate::command::{ROOT, RegisterName, Shorthand, Support, Table, TextObject, register_name};
use crate::key::{Code, Key, Named};

/// 次数的上限；超过时取该值。取值依据：录制基准 `vim` 9.1 对次数的限制。
pub const MAX_COUNT: u32 = 999_999_999;

/// 未完成命令中保留用于显示的按键数上限；超出的按键仍参与解析，不再记录。
///
/// 寄存器、两段各 9 位的次数、操作符与动作合计不超过 30 个键；重复输入寄存器或次数的序列
/// 才会超过 64 个。
const MAX_PENDING: usize = 64;

/// 带参数的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Movement {
    /// 动作。
    pub motion: Motion,
    /// 字符参数；动作不带参数时为 `None`。`<Tab>` 记为 `\t`，`<CR>` 记为 `\r`。
    pub arg: Option<char>,
}

/// 操作符作用的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 从光标到动作的目标位置。
    Motion(Movement),
    /// 文本对象。
    Object(TextObject),
    /// 叠写形式（`dd`、`g~~`）：从当前行起的若干整行，行数为次数。
    Lines,
    /// 可视模式下的选区。
    Selection,
    /// 可视模式下选区所在的整行；按块时为块本身，`to_end` 为真时块延伸到各行的行尾。
    SelectedLines {
        /// 按块时是否延伸到各行的行尾。
        to_end: bool,
    },
}

/// 命令的主体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// 移动光标；可视模式下延伸选区。
    Motion(Movement),
    /// 可视模式下把选区扩展到文本对象。
    Object(TextObject),
    /// 对 `target` 执行操作符。
    Operator {
        /// 操作符。
        operator: Operator,
        /// 作用的范围。
        target: Target,
        /// 操作符之后强制指定的动作类型，未指定时为 `None`；可视模式下 `Linewise` 表示先把
        /// 选区扩展到整行。
        force: Option<Force>,
    },
    /// 不带范围的命令。
    Action {
        /// 命令。
        action: Action,
        /// 字符参数；命令不带参数时为 `None`。`<Tab>` 记为 `\t`，`<CR>` 记为 `\r`。
        arg: Option<char>,
    },
}

/// 一条解析完成的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// 寄存器名；未指定时为 `None`。大写字母表示追加。
    pub register: Option<char>,
    /// 次数，各段次数之积，范围 1 至 [`MAX_COUNT`]；未输入时为 `None`。
    pub count: Option<u32>,
    /// 主体。
    pub body: Body,
}

/// [`Parser::feed`] 处理一个按键之后的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 命令尚未完成，等待后续按键。
    Pending,
    /// 得到一条完整的命令。
    Command(Command),
    /// 输入的命令不支持或尚未实现；按键序列已完整读取，解析器回到空闲状态。
    Unsupported,
    /// 按键不能接在已输入的部分之后；已输入的部分与该按键一并丢弃。
    Invalid,
    /// `<Esc>` 取消了未完成的命令，或 `<Del>` 删除了仅有的次数。
    Cancelled,
    /// 解析器空闲，且该按键在当前模式下没有定义；按键未被使用。
    Unbound,
}

/// 等待动作或文本对象的操作符。
#[derive(Debug, Clone, Copy)]
struct PendingOperator {
    /// 操作符。
    operator: Operator,
    /// 操作符的记录在命令表中的下标。
    entry: u16,
}

/// 成对符号的命令已输入到哪一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pairing {
    /// `ys` 之后：等待动作或文本对象。
    Add,
    /// 范围已确定：等待要加上的符号。
    Wrap(Target),
    /// `ds` 之后：等待要删除的符号。
    Delete,
    /// `cs` 之后：等待原来的符号。
    From,
    /// `cs{符号}` 之后：等待新的符号。
    To(char),
}

/// 命令解析器：逐个接收按键，保存尚未完成的命令。
#[derive(Debug)]
pub struct Parser {
    /// 未完成命令所属的模式；与传入的模式不同时丢弃未完成的命令。
    mode: Mode,
    /// 未完成命令已输入的按键，用于显示；最多 [`MAX_PENDING`] 个。
    keys: Vec<Key>,
    /// 已输入的寄存器名。
    register: Option<char>,
    /// 已结束的各段次数之积；寄存器、操作符与强制类型的前缀结束一段次数。
    total: Option<u32>,
    /// 正在输入的一段次数。
    count: Option<u32>,
    /// 已输入、等待范围的操作符。
    operator: Option<PendingOperator>,
    /// 操作符之后输入的强制类型。
    force: Option<Force>,
    /// 多键序列在前缀树中到达的节点；不在多键序列中时为根。
    node: u16,
    /// 按键序列已匹配、等待字符参数的记录在命令表中的下标。
    arg: Option<u16>,
    /// 已输入的操作符是否不受支持。
    unsupported: bool,
    /// 成对符号的命令已输入到哪一步，以及该命令的记录在命令表中的下标。
    pairing: Option<(Pairing, u16)>,
}

impl Default for Parser {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            keys: Vec::new(),
            register: None,
            total: None,
            count: None,
            operator: None,
            force: None,
            node: ROOT,
            arg: None,
            unsupported: false,
            pairing: None,
        }
    }
}

impl Parser {
    /// 处理 `mode` 模式下输入的按键 `key`。
    ///
    /// `mode` 是窗口所在的模式；等待动作的状态由解析器自身保存，传入
    /// [`Mode::OperatorPending`] 时按普通模式处理。`mode` 与未完成命令所属的模式不同时，
    /// 先丢弃未完成的命令。插入模式下没有寄存器与次数，未定义的按键返回 [`Outcome::Unbound`]。
    pub fn feed(&mut self, mode: Mode, key: Key) -> Outcome {
        let mode = match mode {
            Mode::OperatorPending => Mode::Normal,
            mode => mode,
        };
        if self.mode != mode {
            self.reset();
            self.mode = mode;
        }
        let table = Table::get();
        // 普通与可视模式下有未完成的命令时，`Ctrl-C` 与 `<Esc>` 相同。
        let pending = !self.is_idle() || self.pairing.is_some();
        let key = match key {
            key if pending && mode != Mode::Insert && key == Key::ctrl('c') => {
                Key::named(Named::Esc)
            }
            key => key,
        };

        if let Some(index) = self.arg.take() {
            return self.argument(table, index, key);
        }
        let idle = self.is_idle();
        if self.node == ROOT
            && mode != Mode::Insert
            && let Some(outcome) = self.count_key(key)
        {
            return outcome;
        }
        if key == Key::named(Named::Esc) && !idle {
            // 可视模式下只输入了寄存器或次数时，`<Esc>` 仍是离开可视模式的命令。
            let leaves = mode == Mode::Visual && self.operator.is_none() && self.node == ROOT;
            if !leaves {
                self.reset();
                return Outcome::Cancelled;
            }
        }
        // 按键序列多于一个键的操作符，重复最后一个键也是叠写形式（`g~~`、`gUU`）。
        if self.node == ROOT
            && let Some(pending) = self.operator
            && let [_, .., last] = table.keys(pending.entry)
            && *last == key
        {
            self.note(key);
            let body = self.operate(pending.operator, Target::Lines);
            return self.finish(Support::Supported, body);
        }

        let lookup = match self.operator {
            Some(_) => Mode::OperatorPending,
            None => mode,
        };
        match table.step(lookup, self.node, key) {
            Lookup::None => {
                self.reset();
                if idle {
                    Outcome::Unbound
                } else {
                    Outcome::Invalid
                }
            }
            Lookup::Prefix(node) => {
                self.node = node;
                self.note(key);
                Outcome::Pending
            }
            Lookup::Entry(index) => {
                self.node = ROOT;
                self.note(key);
                match table.entry(index) {
                    Some(entry) if entry.arg == Arg::Char => {
                        self.arg = Some(index);
                        Outcome::Pending
                    }
                    Some(entry) => self.matched(index, entry, None),
                    None => self.invalid(),
                }
            }
        }
    }

    /// 返回未完成命令已输入的按键，用于显示；解析器空闲时为空。
    pub fn pending(&self) -> &[Key] {
        &self.keys
    }

    /// 下一个按键是否为一条命令、动作或文本对象的第一个键：不在多键序列与字符参数之中。
    pub(crate) fn at_start(&self) -> bool {
        self.node == ROOT && self.arg.is_none()
    }

    /// 是否已输入操作符、正在等待动作或文本对象。
    pub fn is_operator_pending(&self) -> bool {
        self.operator.is_some()
    }

    /// 丢弃未完成的命令，回到空闲状态。
    pub fn reset(&mut self) {
        self.keys.clear();
        self.register = None;
        self.total = None;
        self.count = None;
        self.operator = None;
        self.force = None;
        self.node = ROOT;
        self.arg = None;
        self.unsupported = false;
        self.pairing = None;
    }

    /// 是否没有未完成的命令。
    fn is_idle(&self) -> bool {
        self.register.is_none()
            && self.total.is_none()
            && self.count.is_none()
            && self.operator.is_none()
            && self.force.is_none()
            && self.node == ROOT
            && self.arg.is_none()
    }

    /// 把 `key` 记入未完成命令的按键。
    fn note(&mut self, key: Key) {
        if self.keys.len() < MAX_PENDING {
            self.keys.push(key);
        }
    }

    /// 丢弃未完成的命令并返回 [`Outcome::Invalid`]。
    fn invalid(&mut self) -> Outcome {
        self.reset();
        Outcome::Invalid
    }

    /// 处理次数的输入：数字键追加一位，`<Del>` 删除末位；`key` 不属于次数时返回 `None`。
    ///
    /// `0` 只在已经输入次数时算作数字，否则是移到行首的动作。
    fn count_key(&mut self, key: Key) -> Option<Outcome> {
        if let Code::Char(c) = key.code()
            && key.mods().is_empty()
            && let Some(digit) = c.to_digit(10)
            && (digit != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0);
            let next = if count > MAX_COUNT / 10 {
                MAX_COUNT
            } else {
                count * 10 + digit
            };
            if self.count != Some(next) {
                self.note(key);
            }
            self.count = Some(next);
            return Some(Outcome::Pending);
        }
        if key == Key::named(Named::Delete)
            && let Some(count) = self.count
        {
            self.count = (count >= 10).then_some(count / 10);
            if (self.keys.last()).is_some_and(|k| k.as_char().is_some_and(|c| c.is_ascii_digit())) {
                self.keys.pop();
            }
            if self.is_idle() {
                self.reset();
                return Some(Outcome::Cancelled);
            }
            return Some(Outcome::Pending);
        }
        None
    }

    /// 把正在输入的一段次数并入已结束的各段之积。
    fn fold_count(&mut self) {
        self.total = product(self.total, self.count.take());
    }

    /// 处理等待字符参数的记录 `index` 收到的按键。
    fn argument(&mut self, table: &Table, index: u16, key: Key) -> Outcome {
        self.note(key);
        if key == Key::named(Named::Esc) {
            self.reset();
            return Outcome::Cancelled;
        }
        let arg = match key.code() {
            Code::Char(c) if key.as_char().is_some() => c,
            Code::Named(Named::Tab) if key.mods().is_empty() => '\t',
            Code::Named(Named::Enter) if key.mods().is_empty() => '\r',
            _ => return self.invalid(),
        };
        match table.entry(index) {
            Some(entry) => self.matched(index, entry, Some(arg)),
            None => self.invalid(),
        }
    }

    /// 处理按键序列与参数都已读取的记录。
    fn matched(&mut self, index: u16, entry: &Entry, arg: Option<char>) -> Outcome {
        let body = match entry.kind {
            Kind::Register => {
                return match arg.map(register_name) {
                    Some(RegisterName::Valid) => {
                        self.register = arg;
                        self.fold_count();
                        Outcome::Pending
                    }
                    Some(RegisterName::Unsupported) => {
                        self.reset();
                        Outcome::Unsupported
                    }
                    Some(RegisterName::Invalid) | None => self.invalid(),
                };
            }
            Kind::Force(force) => {
                self.force = Some(force);
                self.unsupported |= entry.support != Support::Supported;
                self.fold_count();
                return Outcome::Pending;
            }
            Kind::Operator(_) if self.pairing.is_some() => return self.invalid(),
            Kind::Operator(operator) => match self.operator {
                Some(pending) if pending.operator == operator => {
                    self.operate(operator, Target::Lines)
                }
                Some(_) => return self.invalid(),
                None if self.mode == Mode::Visual => self.operate(operator, Target::Selection),
                None => {
                    self.operator = Some(PendingOperator {
                        operator,
                        entry: index,
                    });
                    self.unsupported = entry.support != Support::Supported;
                    self.fold_count();
                    return Outcome::Pending;
                }
            },
            Kind::SmallDelete => {
                self.register = self.register.or(Some('-'));
                self.operate(Operator::Delete, Target::Selection)
            }
            Kind::LineOperator(operator) => Body::Operator {
                operator,
                target: Target::Selection,
                force: Some(Force::Linewise),
            },
            Kind::WideOperator(operator, to_end) => Body::Operator {
                operator,
                target: Target::SelectedLines { to_end },
                force: None,
            },
            Kind::Synonym(operator, shorthand) => {
                let target = match shorthand {
                    Shorthand::Motion(motion) => Target::Motion(Movement { motion, arg: None }),
                    Shorthand::Lines => Target::Lines,
                };
                self.operate(operator, target)
            }
            Kind::Motion(motion) => {
                let movement = Movement { motion, arg };
                match (self.operator, self.pairing) {
                    (Some(_), Some((Pairing::Add, at))) => {
                        return self.await_pair(at, Pairing::Wrap(Target::Motion(movement)));
                    }
                    (Some(pending), _) => self.operate(pending.operator, Target::Motion(movement)),
                    (None, _) => Body::Motion(movement),
                }
            }
            Kind::TextObject(object) => match (self.operator, self.pairing) {
                (Some(_), Some((Pairing::Add, at))) => {
                    return self.await_pair(at, Pairing::Wrap(Target::Object(object)));
                }
                (Some(pending), _) => self.operate(pending.operator, Target::Object(object)),
                (None, _) => Body::Object(object),
            },
            Kind::Surround => {
                let stage = self.pairing.map(|(stage, _)| stage);
                let operator = self.operator.map(|pending| pending.operator);
                match (arg, stage, operator) {
                    // 可视模式的 `S{符号}`。
                    (Some(with), None, None) => {
                        self.operate(Operator::Surround(with), Target::Selection)
                    }
                    (None, None, Some(Operator::Yank)) => {
                        self.pairing = Some((Pairing::Add, index));
                        return Outcome::Pending;
                    }
                    // `yss`：当前行。
                    (None, Some(Pairing::Add), _) => {
                        return self.await_pair(index, Pairing::Wrap(Target::Lines));
                    }
                    (None, None, Some(Operator::Delete)) => {
                        return self.await_pair(index, Pairing::Delete);
                    }
                    (None, None, Some(Operator::Change)) => {
                        return self.await_pair(index, Pairing::From);
                    }
                    (Some(with), Some(Pairing::Wrap(target)), _) => {
                        self.operate(Operator::Surround(with), target)
                    }
                    (Some(old), Some(Pairing::Delete), _) => Body::Action {
                        action: Action::DeleteSurround,
                        arg: Some(old),
                    },
                    (Some(old), Some(Pairing::From), _) => {
                        return self.await_pair(index, Pairing::To(old));
                    }
                    (Some(new), Some(Pairing::To(old)), _) => Body::Action {
                        action: Action::ChangeSurround(old),
                        arg: Some(new),
                    },
                    _ => return self.invalid(),
                }
            }
            Kind::Action(action) => Body::Action { action, arg },
        };
        self.finish(entry.support, body)
    }

    /// 成对符号的命令进入 `stage` 这一步，等待一个符号；`index` 是该命令的记录的下标。
    fn await_pair(&mut self, index: u16, stage: Pairing) -> Outcome {
        self.pairing = Some((stage, index));
        self.arg = Some(index);
        Outcome::Pending
    }

    /// 返回以已输入的强制类型对 `target` 执行 `operator` 的命令主体。
    fn operate(&self, operator: Operator, target: Target) -> Body {
        Body::Operator {
            operator,
            target,
            force: self.force,
        }
    }

    /// 结束当前命令并回到空闲状态；`support` 是最后匹配的记录的支持状态，不是
    /// [`Support::Supported`] 时返回 [`Outcome::Unsupported`]。
    fn finish(&mut self, support: Support, body: Body) -> Outcome {
        let unsupported = self.unsupported || support != Support::Supported;
        let command = Command {
            register: self.register,
            count: product(self.total, self.count),
            body,
        };
        self.reset();
        if unsupported {
            Outcome::Unsupported
        } else {
            Outcome::Command(command)
        }
    }
}

/// 返回两段次数之积，不超过 [`MAX_COUNT`]；只有一段时返回该段。
fn product(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let product = (u64::from(a) * u64::from(b)).min(u64::from(MAX_COUNT));
            Some(u32::try_from(product).unwrap_or(MAX_COUNT))
        }
        (a, None) => a,
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests;
