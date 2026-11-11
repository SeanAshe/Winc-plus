//! 命令表的内容，以及按模式建立的前缀树。

use std::sync::OnceLock;

use super::Scrolling;
use super::{Action, Arg, Entry, Focusing, Folding, Force, Kind, Mode, Modes, Motion, Operator};
use super::{Shape, Shorthand, Support, TextObject};
use crate::key::{Key, notation};

/// 创建一条没有参数、状态为 [`Support::Supported`] 的记录。
const fn entry(keys: &'static str, modes: Modes, kind: Kind) -> Entry {
    Entry {
        keys,
        modes,
        kind,
        arg: Arg::None,
        support: Support::Supported,
    }
}

/// 创建在普通、可视模式与等待动作时生效的动作。
const fn motion(keys: &'static str, motion: Motion) -> Entry {
    entry(keys, Modes::NVO, Kind::Motion(motion))
}

/// 创建在普通与可视模式下生效的操作符。
const fn operator(keys: &'static str, operator: Operator) -> Entry {
    entry(keys, Modes::NV, Kind::Operator(operator))
}

/// 创建普通模式下操作符加动作的同义写法。
const fn synonym(keys: &'static str, operator: Operator, target: Shorthand) -> Entry {
    entry(keys, Modes::N, Kind::Synonym(operator, target))
}

/// 创建在可视模式与等待动作时生效的文本对象。
const fn object(keys: &'static str, shape: Shape, around: bool) -> Entry {
    entry(
        keys,
        Modes::VO,
        Kind::TextObject(TextObject { shape, around }),
    )
}

/// 创建普通与可视模式下的滚动命令。
const fn scroll(keys: &'static str, scrolling: Scrolling) -> Entry {
    action(keys, Modes::NV, Action::Scroll(scrolling))
}

/// 创建普通与可视模式下的折叠命令。
const fn fold(keys: &'static str, folding: Folding) -> Entry {
    action(keys, Modes::NV, Action::Fold(folding))
}

/// 创建插入模式下的命令。
const fn typing(keys: &'static str, action: Action) -> Entry {
    entry(keys, Modes::I, Kind::Action(action))
}

/// 创建不带范围的命令。
const fn action(keys: &'static str, modes: Modes, action: Action) -> Entry {
    entry(keys, modes, Kind::Action(action))
}

impl Entry {
    /// 返回带一个字符参数的记录。
    const fn with_char(mut self) -> Self {
        self.arg = Arg::Char;
        self
    }

    /// 返回状态为 [`Support::Unsupported`] 的记录。
    const fn unsupported(mut self) -> Self {
        self.support = Support::Unsupported;
        self
    }
}

/// 全部记录。
///
/// 同一模式下任何记录的按键序列都不是另一条记录的前缀，按键序列写成规范的按键记法。
pub static ENTRIES: &[Entry] = &[
    // 前缀。
    entry("\"", Modes::NV, Kind::Register).with_char(),
    entry("v", Modes::O, Kind::Force(Force::Charwise)),
    entry("V", Modes::O, Kind::Force(Force::Linewise)),
    entry("<C-v>", Modes::O, Kind::Force(Force::Blockwise)),
    // 左右移动。
    motion("h", Motion::Left),
    motion("<Left>", Motion::Left),
    motion("l", Motion::Right),
    motion("<Right>", Motion::Right),
    entry("<BS>", Modes::NO, Kind::Motion(Motion::Backspace)),
    entry("<C-h>", Modes::NO, Kind::Motion(Motion::Backspace)),
    motion("<Space>", Motion::Space),
    motion("0", Motion::LineStart),
    motion("<Home>", Motion::LineStart),
    motion("^", Motion::FirstNonBlank),
    motion("$", Motion::LineEnd),
    motion("<End>", Motion::LineEnd),
    motion("g_", Motion::LastNonBlank),
    motion("g0", Motion::DisplayStart),
    motion("g<Home>", Motion::DisplayStart),
    motion("g^", Motion::DisplayFirstNonBlank),
    motion("g$", Motion::DisplayEnd),
    motion("g<End>", Motion::DisplayLastNonBlank),
    motion("gm", Motion::DisplayMiddle),
    motion("|", Motion::Column),
    motion("f", Motion::FindForward).with_char(),
    motion("F", Motion::FindBackward).with_char(),
    motion("t", Motion::TillForward).with_char(),
    motion("T", Motion::TillBackward).with_char(),
    motion(";", Motion::RepeatFind),
    motion(",", Motion::RepeatFindReverse),
    // 上下移动。
    motion("k", Motion::Up),
    motion("<Up>", Motion::Up),
    motion("j", Motion::Down),
    motion("<C-j>", Motion::Down),
    motion("<C-n>", Motion::Down),
    motion("<C-p>", Motion::Up),
    motion("<Down>", Motion::Down),
    motion("gk", Motion::DisplayUp),
    motion("g<Up>", Motion::DisplayUp),
    motion("gj", Motion::DisplayDown),
    motion("g<Down>", Motion::DisplayDown),
    motion("-", Motion::LineUp),
    motion("+", Motion::LineDown),
    motion("<CR>", Motion::LineDown),
    motion("_", Motion::LineHere),
    motion("gg", Motion::FirstLine),
    motion("<C-Home>", Motion::FirstLine),
    motion("G", Motion::LastLine),
    motion("<C-End>", Motion::LastLineEnd),
    motion("%", Motion::Percent),
    motion("H", Motion::WindowTop),
    motion("M", Motion::WindowMiddle),
    motion("L", Motion::WindowBottom),
    // 按词与文本移动。
    motion("w", Motion::WordForward),
    motion("<S-Right>", Motion::WordForward),
    motion("W", Motion::BigWordForward),
    motion("<C-Right>", Motion::BigWordForward),
    motion("b", Motion::WordBackward),
    motion("<S-Left>", Motion::WordBackward),
    motion("B", Motion::BigWordBackward),
    motion("<C-Left>", Motion::BigWordBackward),
    motion("e", Motion::WordEnd),
    motion("E", Motion::BigWordEnd),
    motion("ge", Motion::WordEndBackward),
    motion("gE", Motion::BigWordEndBackward),
    motion("(", Motion::SentenceBackward),
    motion(")", Motion::SentenceForward),
    motion("{", Motion::ParagraphBackward),
    motion("}", Motion::ParagraphForward),
    motion("[[", Motion::SectionBackward),
    motion("]]", Motion::SectionForward),
    motion("[]", Motion::SectionEndBackward),
    motion("][", Motion::SectionEndForward),
    motion("[(", Motion::UnmatchedParenBackward),
    motion("[{", Motion::UnmatchedBraceBackward),
    motion("])", Motion::UnmatchedParenForward),
    motion("]}", Motion::UnmatchedBraceForward),
    // 操作符。
    operator("d", Operator::Delete),
    operator("c", Operator::Change),
    operator("y", Operator::Yank),
    operator(">", Operator::Indent),
    operator("<lt>", Operator::Outdent),
    operator("g~", Operator::ToggleCase),
    operator("gu", Operator::Lowercase),
    operator("gU", Operator::Uppercase),
    operator("g?", Operator::Rot13),
    operator("gc", Operator::Comment),
    operator("=", Operator::Reindent),
    operator("gq", Operator::Format).unsupported(),
    operator("gw", Operator::Format).unsupported(),
    operator("zf", Operator::Fold).unsupported(),
    operator("!", Operator::Filter).unsupported(),
    // 可视模式下直接作用于选区的写法。
    entry("<BS>", Modes::V, Kind::SmallDelete),
    entry("<C-h>", Modes::V, Kind::SmallDelete),
    entry("x", Modes::V, Kind::Operator(Operator::Delete)),
    entry("<Del>", Modes::V, Kind::Operator(Operator::Delete)),
    entry("s", Modes::V, Kind::Operator(Operator::Change)),
    entry("u", Modes::V, Kind::Operator(Operator::Lowercase)),
    entry("U", Modes::V, Kind::Operator(Operator::Uppercase)),
    entry("~", Modes::V, Kind::Operator(Operator::ToggleCase)),
    entry("J", Modes::V, Kind::Operator(Operator::Join)),
    entry("gJ", Modes::V, Kind::Operator(Operator::JoinRaw)),
    entry("X", Modes::V, Kind::WideOperator(Operator::Delete, false)),
    entry("D", Modes::V, Kind::WideOperator(Operator::Delete, true)),
    entry("Y", Modes::V, Kind::WideOperator(Operator::Yank, false)),
    entry("S", Modes::V, Kind::Surround).with_char(),
    entry("s", Modes::O, Kind::Surround),
    entry("C", Modes::V, Kind::WideOperator(Operator::Change, true)),
    entry("R", Modes::V, Kind::LineOperator(Operator::Change)),
    // 操作符加动作的同义写法。
    synonym("x", Operator::Delete, Shorthand::Motion(Motion::Right)),
    synonym("<Del>", Operator::Delete, Shorthand::Motion(Motion::Right)),
    synonym("X", Operator::Delete, Shorthand::Motion(Motion::Left)),
    synonym("D", Operator::Delete, Shorthand::Motion(Motion::LineEnd)),
    synonym("C", Operator::Change, Shorthand::Motion(Motion::LineEnd)),
    synonym("s", Operator::Change, Shorthand::Motion(Motion::Right)),
    synonym("S", Operator::Change, Shorthand::Lines),
    synonym("Y", Operator::Yank, Shorthand::Lines),
    // 文本对象。
    object("ii", Shape::Indent, false),
    object("ai", Shape::Indent, true),
    object("iw", Shape::Word, false),
    object("aw", Shape::Word, true),
    object("iW", Shape::BigWord, false),
    object("aW", Shape::BigWord, true),
    object("is", Shape::Sentence, false),
    object("as", Shape::Sentence, true),
    object("ip", Shape::Paragraph, false),
    object("ap", Shape::Paragraph, true),
    object("i(", Shape::Paren, false),
    object("i)", Shape::Paren, false),
    object("ib", Shape::Paren, false),
    object("a(", Shape::Paren, true),
    object("a)", Shape::Paren, true),
    object("ab", Shape::Paren, true),
    object("i[", Shape::Bracket, false),
    object("i]", Shape::Bracket, false),
    object("a[", Shape::Bracket, true),
    object("a]", Shape::Bracket, true),
    object("i{", Shape::Brace, false),
    object("i}", Shape::Brace, false),
    object("iB", Shape::Brace, false),
    object("a{", Shape::Brace, true),
    object("a}", Shape::Brace, true),
    object("aB", Shape::Brace, true),
    object("i<lt>", Shape::Angle, false),
    object("i>", Shape::Angle, false),
    object("a<lt>", Shape::Angle, true),
    object("a>", Shape::Angle, true),
    object("it", Shape::Tag, false),
    object("at", Shape::Tag, true),
    object("i\"", Shape::DoubleQuote, false),
    object("a\"", Shape::DoubleQuote, true),
    object("i'", Shape::SingleQuote, false),
    object("a'", Shape::SingleQuote, true),
    object("i`", Shape::Backtick, false),
    object("a`", Shape::Backtick, true),
    object("gn", Shape::NextMatch, false),
    object("gN", Shape::PreviousMatch, false),
    // 进入插入模式。
    action("i", Modes::N, Action::Insert),
    action("<Insert>", Modes::N, Action::Insert),
    action("a", Modes::N, Action::Append),
    action("I", Modes::N, Action::InsertFirstNonBlank),
    action("gI", Modes::N, Action::InsertLineStart),
    action("A", Modes::N, Action::AppendLineEnd),
    action("gi", Modes::N, Action::InsertAtLast),
    action("o", Modes::N, Action::OpenBelow),
    action("O", Modes::N, Action::OpenAbove),
    // 编辑。
    action("r", Modes::NV, Action::ReplaceChar).with_char(),
    action("~", Modes::N, Action::ToggleCaseChar),
    action("p", Modes::NV, Action::PasteAfter),
    action("P", Modes::NV, Action::PasteBefore),
    action("gp", Modes::N, Action::PasteAfterMove),
    action("gP", Modes::N, Action::PasteBeforeMove),
    action("]p", Modes::N, Action::PasteIndented),
    action("J", Modes::N, Action::JoinLines),
    action("gJ", Modes::N, Action::JoinLinesRaw),
    action("u", Modes::N, Action::Undo),
    action("<C-r>", Modes::N, Action::Redo),
    action(".", Modes::N, Action::Repeat),
    // 可视模式。
    action("v", Modes::NV, Action::Visual),
    action("V", Modes::NV, Action::VisualLine),
    action("gv", Modes::NV, Action::Reselect),
    action("o", Modes::V, Action::SwapAnchor),
    action("O", Modes::V, Action::SwapColumn),
    action("<Esc>", Modes::V.with(Modes::I), Action::Escape),
    action("<C-c>", Modes::NV, Action::Interrupt),
    action("<C-v>", Modes::NV, Action::VisualBlock),
    action("<C-q>", Modes::NV, Action::VisualBlock),
    action("I", Modes::V, Action::InsertSelection),
    action("A", Modes::V, Action::AppendSelection),
    // 查找。
    motion("/", Motion::SearchForward),
    motion("?", Motion::SearchBackward),
    motion("n", Motion::SearchNext),
    motion("N", Motion::SearchPrevious),
    motion("*", Motion::SearchWordForward),
    motion("#", Motion::SearchWordBackward),
    motion("g*", Motion::SearchPartForward),
    motion("g#", Motion::SearchPartBackward),
    action("gn", Modes::N, Action::SelectNext),
    action("gN", Modes::N, Action::SelectPrevious),
    // 标记、跳转与修改列表。
    action("m", Modes::NV, Action::SetMark).with_char(),
    motion("`", Motion::Mark).with_char(),
    motion("'", Motion::MarkLine).with_char(),
    action("<C-o>", Modes::N, Action::JumpBack),
    action("<Tab>", Modes::N, Action::JumpForward),
    action("g;", Modes::N, Action::ChangeOlder),
    action("g,", Modes::N, Action::ChangeNewer),
    // 替换模式与数的增减。
    action("R", Modes::N, Action::Replace),
    action("<C-a>", Modes::N, Action::Increment),
    action("<C-x>", Modes::N, Action::Decrement),
    // 滚动。
    scroll("<C-e>", Scrolling::LineDown),
    scroll("<C-y>", Scrolling::LineUp),
    scroll("<C-d>", Scrolling::HalfDown),
    scroll("<C-u>", Scrolling::HalfUp),
    scroll("<C-f>", Scrolling::PageDown),
    scroll("<PageDown>", Scrolling::PageDown),
    scroll("<S-Down>", Scrolling::PageDown),
    scroll("<C-b>", Scrolling::PageUp),
    scroll("<PageUp>", Scrolling::PageUp),
    scroll("<S-Up>", Scrolling::PageUp),
    scroll("zt", Scrolling::Top),
    scroll("z<CR>", Scrolling::TopStart),
    scroll("zz", Scrolling::Center),
    scroll("z.", Scrolling::CenterStart),
    scroll("zb", Scrolling::Bottom),
    scroll("z-", Scrolling::BottomStart),
    scroll("zh", Scrolling::Left),
    scroll("z<Left>", Scrolling::Left),
    scroll("zl", Scrolling::Right),
    scroll("z<Right>", Scrolling::Right),
    scroll("zH", Scrolling::HalfLeft),
    scroll("zL", Scrolling::HalfRight),
    scroll("zs", Scrolling::CaretStart),
    scroll("ze", Scrolling::CaretEnd),
    // 折叠。
    fold("zo", Folding::Open),
    fold("zc", Folding::Close),
    fold("za", Folding::Toggle),
    fold("zO", Folding::OpenTree),
    fold("zC", Folding::CloseTree),
    fold("zA", Folding::ToggleTree),
    fold("zR", Folding::OpenAll),
    fold("zM", Folding::CloseAll),
    fold("zv", Folding::Reveal),
    fold("zj", Folding::Next),
    fold("zk", Folding::Previous),
    // 插入模式。
    typing("<C-c>", Action::Interrupt),
    typing("<C-h>", Action::InsertBackspace),
    typing("<C-w>", Action::InsertDeleteWord),
    typing("<C-u>", Action::InsertDeleteLine),
    typing("<C-t>", Action::InsertIndent),
    typing("<C-d>", Action::InsertOutdent),
    typing("<C-r>", Action::InsertRegister).with_char(),
    typing("<C-o>", Action::InsertNormal),
    typing("<C-e>", Action::InsertBelow),
    typing("<C-y>", Action::InsertAbove),
    // 命令行、宏与交给宿主的命令。
    action(":", Modes::NV, Action::CommandLine),
    action("q", Modes::NV, Action::Record).with_char(),
    action("@", Modes::NV, Action::Play).with_char(),
    action("&", Modes::N, Action::RepeatSubstitute),
    action("g&", Modes::N, Action::RepeatSubstituteAll),
    action("gd", Modes::N, Action::Definition),
    action("gf", Modes::NV, Action::OpenFile),
    action("gt", Modes::NV, Action::TabNext),
    action("gT", Modes::NV, Action::TabPrevious),
    action("ga", Modes::NV, Action::CharInfo),
    action("ZZ", Modes::N, Action::WriteQuit),
    action("ZQ", Modes::N, Action::QuitForce),
    action("<C-^>", Modes::N, Action::Alternate),
    action("<C-w>h", Modes::NV, Action::Focus(Focusing::Left)),
    action("<C-w>l", Modes::NV, Action::Focus(Focusing::Right)),
    action("<C-w>j", Modes::NV, Action::Focus(Focusing::Down)),
    action("<C-w>k", Modes::NV, Action::Focus(Focusing::Up)),
    action("<C-w>w", Modes::NV, Action::Focus(Focusing::Next)),
    action("<C-w><C-h>", Modes::NV, Action::Focus(Focusing::Left)),
    action("<C-w><C-l>", Modes::NV, Action::Focus(Focusing::Right)),
    action("<C-w><C-j>", Modes::NV, Action::Focus(Focusing::Down)),
    action("<C-w><C-k>", Modes::NV, Action::Focus(Focusing::Up)),
    action("<C-w><C-w>", Modes::NV, Action::Focus(Focusing::Next)),
    // 不支持的命令。
    action("Q", Modes::NV, Action::ExMode).unsupported(),
    action("K", Modes::NV, Action::Keyword).unsupported(),
    action("U", Modes::N, Action::UndoLine).unsupported(),
    action("zd", Modes::NV, Action::Later).unsupported(),
];

/// 前缀树的根节点。
pub(crate) const ROOT: u16 = 0;

/// 前缀树的一个节点。
#[derive(Debug, Default)]
struct Node {
    /// 子节点：按键与节点下标，按按键排序。
    children: Vec<(Key, u16)>,
    /// 按键序列在此结束的记录在 [`ENTRIES`] 中的下标；有记录的节点没有子节点。
    entry: Option<u16>,
}

/// 一步查找的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// 按键序列与一条记录完全匹配；值为该记录在 [`ENTRIES`] 中的下标。
    Entry(u16),
    /// 按键序列是若干记录的前缀；值为到达的节点，用于继续查找。
    Prefix(u16),
    /// 没有记录以该按键序列开头。
    None,
}

/// 按模式建立的前缀树。首次使用时由 [`ENTRIES`] 建立，之后只读。
#[derive(Debug)]
pub(crate) struct Table {
    /// 各模式的前缀树，下标为 [`Mode::index`]；每棵树的 0 号节点是根。
    tries: [Vec<Node>; Mode::ALL.len()],
    /// 各记录解析后的按键序列，下标与 [`ENTRIES`] 相同。
    keys: Vec<Vec<Key>>,
}

impl Table {
    /// 返回进程内唯一的命令表，首次调用时建立。
    pub(crate) fn get() -> &'static Self {
        static TABLE: OnceLock<Table> = OnceLock::new();
        TABLE.get_or_init(|| Self::build().0)
    }

    /// 由 [`ENTRIES`] 建立前缀树；同时返回未能加入的记录（模式与下标），正确的命令表中为空。
    ///
    /// 普通模式的操作符同时加入等待动作的前缀树，用于识别叠写形式。
    pub(super) fn build() -> (Self, Vec<(Mode, usize)>) {
        let keys: Vec<Vec<Key>> = ENTRIES.iter().map(|e| notation::parse(e.keys)).collect();
        let mut tries = Mode::ALL.map(|_| vec![Node::default()]);
        let mut rejected = Vec::new();
        for (index, (entry, sequence)) in ENTRIES.iter().zip(&keys).enumerate() {
            for mode in Mode::ALL {
                let doubled = mode == Mode::OperatorPending
                    && matches!(entry.kind, Kind::Operator(_))
                    && entry.modes.contains(Mode::Normal);
                if !entry.modes.contains(mode) && !doubled {
                    continue;
                }
                if !insert(&mut tries[mode.index()], sequence, index) {
                    rejected.push((mode, index));
                }
            }
        }
        (Self { tries, keys }, rejected)
    }

    /// 从 `mode` 的前缀树的节点 `node` 出发，按 `key` 前进一步。
    pub(crate) fn step(&self, mode: Mode, node: u16, key: Key) -> Lookup {
        let trie = &self.tries[mode.index()];
        let child = trie.get(usize::from(node)).and_then(|node| {
            let at = node.children.binary_search_by(|(k, _)| k.cmp(&key)).ok()?;
            node.children.get(at).map(|(_, child)| *child)
        });
        match child {
            None => Lookup::None,
            Some(child) => match trie.get(usize::from(child)).and_then(|n| n.entry) {
                Some(entry) => Lookup::Entry(entry),
                None => Lookup::Prefix(child),
            },
        }
    }

    /// 返回下标为 `index` 的记录；下标越界时返回 `None`。
    pub(crate) fn entry(&self, index: u16) -> Option<&'static Entry> {
        ENTRIES.get(usize::from(index))
    }

    /// 返回下标为 `index` 的记录的按键序列；下标越界时为空。
    pub(crate) fn keys(&self, index: u16) -> &[Key] {
        self.keys.get(usize::from(index)).map_or(&[], Vec::as_slice)
    }
}

/// 把按键序列 `keys` 对应的记录 `entry` 加入前缀树 `trie`，返回是否成功。
///
/// 按键序列为空、与已有记录相同，或与已有记录互为前缀时不加入。
fn insert(trie: &mut Vec<Node>, keys: &[Key], entry: usize) -> bool {
    let Ok(entry) = u16::try_from(entry) else {
        return false;
    };
    let mut at = usize::from(ROOT);
    for key in keys {
        if trie[at].entry.is_some() {
            return false;
        }
        at = match trie[at].children.binary_search_by(|(k, _)| k.cmp(key)) {
            Ok(i) => usize::from(trie[at].children[i].1),
            Err(i) => {
                let Ok(child) = u16::try_from(trie.len()) else {
                    return false;
                };
                trie.push(Node::default());
                trie[at].children.insert(i, (*key, child));
                usize::from(child)
            }
        };
    }
    let node = &mut trie[at];
    if keys.is_empty() || node.entry.is_some() || !node.children.is_empty() {
        return false;
    }
    node.entry = Some(entry);
    true
}
