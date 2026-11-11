//! 命令表：每条命令一条记录（[`Entry`]），包含模式、按键序列、种类与支持状态。
//!
//! [`ENTRIES`] 是全部记录，首次使用时按模式建成前缀树，供解析器逐键查找。操作符的叠写形式
//! （`dd`、`g~~`）不单独列出，由解析器按操作符的按键序列识别。

mod table;

pub use table::ENTRIES;
pub(crate) use table::{Lookup, ROOT, Table};

/// 命令表区分的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 普通模式。
    Normal,
    /// 可视模式，包括按字符、按行、按块三种。
    Visual,
    /// 等待动作：普通模式下输入操作符之后，到动作或文本对象输入完成之前。
    OperatorPending,
    /// 插入模式。
    Insert,
}

impl Mode {
    /// 全部模式。
    pub const ALL: [Self; 4] = [
        Self::Normal,
        Self::Visual,
        Self::OperatorPending,
        Self::Insert,
    ];

    /// 返回模式在 [`Mode::ALL`] 中的下标。
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// 一组模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modes(u8);

impl Modes {
    /// 普通模式。
    pub const N: Self = Self::of(Mode::Normal);
    /// 可视模式。
    pub const V: Self = Self::of(Mode::Visual);
    /// 等待动作。
    pub const O: Self = Self::of(Mode::OperatorPending);
    /// 插入模式。
    pub const I: Self = Self::of(Mode::Insert);
    /// 普通与可视模式。
    pub const NV: Self = Self::N.with(Self::V);
    /// 可视模式与等待动作。
    pub const VO: Self = Self::V.with(Self::O);
    /// 普通模式与等待动作。
    pub const NO: Self = Self::N.with(Self::O);
    /// 普通、可视模式与等待动作。
    pub const NVO: Self = Self::NV.with(Self::O);

    /// 返回只含 `mode` 的一组模式。
    pub const fn of(mode: Mode) -> Self {
        Self(1 << mode.index())
    }

    /// 返回两组模式的并集。
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 是否包含 `mode`。
    pub const fn contains(self, mode: Mode) -> bool {
        self.0 & Self::of(mode).0 != 0
    }
}

/// 动作：把光标移到另一个位置的命令，可以作为操作符的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// `h`、`<Left>`：向左一个字符，不越过行首。
    Left,
    /// `l`、`<Right>`：向右一个字符，不越过行尾。
    Right,
    /// `<BS>`：向左一个字符，在行首时到上一行行尾。
    Backspace,
    /// `<Space>`：向右一个字符，在行尾时到下一行行首。
    Space,
    /// `0`、`<Home>`：行首。
    LineStart,
    /// `^`：行内第一个非空白字符。
    FirstNonBlank,
    /// `$`、`<End>`：行尾；带次数时为向下若干行的行尾。
    LineEnd,
    /// `g_`：行内最后一个非空白字符。
    LastNonBlank,
    /// `g0`：显示行的开头。
    DisplayStart,
    /// `g^`：显示行内第一个非空白字符。
    DisplayFirstNonBlank,
    /// `g$`：显示行的末尾。
    DisplayEnd,
    /// `g<End>`：显示行内最后一个非空白字符。
    DisplayLastNonBlank,
    /// `gm`：显示行的中间。
    DisplayMiddle,
    /// `|`：次数指定的显示列。
    Column,
    /// `f{字符}`：行内向右查找字符。
    FindForward,
    /// `F{字符}`：行内向左查找字符。
    FindBackward,
    /// `t{字符}`：行内向右查找字符，停在该字符之前。
    TillForward,
    /// `T{字符}`：行内向左查找字符，停在该字符之后。
    TillBackward,
    /// `;`：重复上一次行内查找。
    RepeatFind,
    /// `,`：反方向重复上一次行内查找。
    RepeatFindReverse,
    /// `k`、`<Up>`：向上一行。
    Up,
    /// `j`、`<Down>`：向下一行。
    Down,
    /// `gk`：向上一个显示行。
    DisplayUp,
    /// `gj`：向下一个显示行。
    DisplayDown,
    /// `-`：向上若干行的第一个非空白字符。
    LineUp,
    /// `+`、`<CR>`：向下若干行的第一个非空白字符。
    LineDown,
    /// `_`：向下「次数减一」行的第一个非空白字符。
    LineHere,
    /// `gg`、`<C-Home>`：第一行，带次数时为指定的行。
    FirstLine,
    /// `G`：最后一行，带次数时为指定的行。
    LastLine,
    /// `<C-End>`：最后一行的行尾，带次数时为指定行的行尾。
    LastLineEnd,
    /// `%`：配对的括号；带次数时为文件中该百分比处的行。
    Percent,
    /// `H`：窗口中的第一行。
    WindowTop,
    /// `M`：窗口中间的一行。
    WindowMiddle,
    /// `L`：窗口中的最后一行。
    WindowBottom,
    /// `w`、`<S-Right>`：下一个词的开头。
    WordForward,
    /// `W`、`<C-Right>`：下一个以空白分隔的词的开头。
    BigWordForward,
    /// `b`、`<S-Left>`：上一个词的开头。
    WordBackward,
    /// `B`、`<C-Left>`：上一个以空白分隔的词的开头。
    BigWordBackward,
    /// `e`：词尾。
    WordEnd,
    /// `E`：以空白分隔的词的词尾。
    BigWordEnd,
    /// `ge`：上一个词的词尾。
    WordEndBackward,
    /// `gE`：上一个以空白分隔的词的词尾。
    BigWordEndBackward,
    /// `(`：上一个句子的开头。
    SentenceBackward,
    /// `)`：下一个句子的开头。
    SentenceForward,
    /// `{`：上一个段落的开头。
    ParagraphBackward,
    /// `}`：下一个段落的开头。
    ParagraphForward,
    /// `[[`：向上找到第一列是 `{` 的行。
    SectionBackward,
    /// `]]`：向下找到第一列是 `{` 的行。
    SectionForward,
    /// `[]`：向上找到第一列是 `}` 的行。
    SectionEndBackward,
    /// `][`：向下找到第一列是 `}` 的行。
    SectionEndForward,
    /// `[(`：向左找到未配对的 `(`。
    UnmatchedParenBackward,
    /// `[{`：向左找到未配对的 `{`。
    UnmatchedBraceBackward,
    /// `])`：向右找到未配对的 `)`。
    UnmatchedParenForward,
    /// `]}`：向右找到未配对的 `}`。
    UnmatchedBraceForward,
    /// `/`：向后文查找输入的模式。
    SearchForward,
    /// `?`：向前文查找输入的模式。
    SearchBackward,
    /// `n`：沿上一次查找的方向查找下一处。
    SearchNext,
    /// `N`：沿上一次查找的反方向查找下一处。
    SearchPrevious,
    /// `*`：向后文查找光标处的词，按整词匹配。
    SearchWordForward,
    /// `#`：向前文查找光标处的词，按整词匹配。
    SearchWordBackward,
    /// `g*`：向后文查找光标处的词，不要求整词。
    SearchPartForward,
    /// `g#`：向前文查找光标处的词，不要求整词。
    SearchPartBackward,
    /// `` ` ``{标记}：标记的位置。
    Mark,
    /// `'`{标记}：标记所在行的第一个非空白字符。
    MarkLine,
}

impl Motion {
    /// 是否属于跳转：执行之前把光标的位置记入跳转历史。
    pub(crate) fn is_jump(self) -> bool {
        matches!(
            self,
            Self::FirstLine
                | Self::LastLine
                | Self::LastLineEnd
                | Self::Percent
                | Self::WindowTop
                | Self::WindowMiddle
                | Self::WindowBottom
                | Self::SentenceBackward
                | Self::SentenceForward
                | Self::ParagraphBackward
                | Self::ParagraphForward
                | Self::SectionBackward
                | Self::SectionForward
                | Self::SectionEndBackward
                | Self::SectionEndForward
                | Self::SearchForward
                | Self::SearchBackward
                | Self::SearchNext
                | Self::SearchPrevious
                | Self::SearchWordForward
                | Self::SearchWordBackward
                | Self::SearchPartForward
                | Self::SearchPartBackward
                | Self::Mark
                | Self::MarkLine
        )
    }
}

/// 操作符：作用于一个动作、文本对象或选区所覆盖的文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// `d`：删除。
    Delete,
    /// `c`：删除并进入插入模式。
    Change,
    /// `y`：复制。
    Yank,
    /// `>`：增加缩进。
    Indent,
    /// `<`：减少缩进。
    Outdent,
    /// `g~`：切换大小写。
    ToggleCase,
    /// `gu`：转为小写。
    Lowercase,
    /// `gU`：转为大写。
    Uppercase,
    /// `g?`：ROT13 编码。
    Rot13,
    /// 可视模式的 `J`：合并所选的行，行间留一个空格。
    Join,
    /// 可视模式的 `gJ`：合并所选的行，不调整空白。
    JoinRaw,
    /// `gc`：切换行注释。
    Comment,
    /// `=`：格式化所覆盖的行。
    Reindent,
    /// `gq`、`gw`：重排文本。
    Format,
    /// `zf`：创建折叠。
    Fold,
    /// `!`：用外部程序过滤文本。
    Filter,
    /// `ys`、可视模式的 `S`：在范围的两端加上成对的符号；值为输入的符号。
    Surround(char),
}

/// 文本对象的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// `w`：词。
    Word,
    /// `W`：以空白分隔的词。
    BigWord,
    /// `s`：句子。
    Sentence,
    /// `p`：段落。
    Paragraph,
    /// `(`、`)`、`b`：圆括号。
    Paren,
    /// `[`、`]`：方括号。
    Bracket,
    /// `{`、`}`、`B`：花括号。
    Brace,
    /// `<`、`>`：尖括号。
    Angle,
    /// `t`：标签。
    Tag,
    /// `"`：双引号。
    DoubleQuote,
    /// `'`：单引号。
    SingleQuote,
    /// `` ` ``：反引号。
    Backtick,
    /// `gn`：上一次查找的模式在光标处或之后的一处匹配。
    NextMatch,
    /// `gN`：上一次查找的模式在光标处或之前的一处匹配。
    PreviousMatch,
    /// `i`：缩进不小于光标所在行的连续各行；`ai` 连同其上缩进更小的一行。
    Indent,
}

/// 文本对象：以 `i` 或 `a` 开头，选中光标处的一段文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextObject {
    /// 形状。
    pub shape: Shape,
    /// 是否包含两侧的空白或定界符：`a` 开头为真，`i` 开头为假。
    pub around: bool,
}

/// 不带范围的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// `i`：在光标之前进入插入模式。
    Insert,
    /// `a`：在光标之后进入插入模式。
    Append,
    /// `I`：在行内第一个非空白字符之前进入插入模式。
    InsertFirstNonBlank,
    /// `gI`：在行首进入插入模式。
    InsertLineStart,
    /// `A`：在行尾进入插入模式。
    AppendLineEnd,
    /// `gi`：在上一次离开插入模式的位置进入插入模式。
    InsertAtLast,
    /// `o`：在下方新开一行并进入插入模式。
    OpenBelow,
    /// `O`：在上方新开一行并进入插入模式。
    OpenAbove,
    /// `r{字符}`：把光标处的字符替换为给定的字符；可视模式下替换选区中的每个字符。
    ReplaceChar,
    /// `~`：切换光标处字符的大小写并右移。
    ToggleCaseChar,
    /// `p`：粘贴到光标之后；可视模式下替换选区。
    PasteAfter,
    /// `P`：粘贴到光标之前。
    PasteBefore,
    /// `gp`：粘贴到光标之后，光标停在粘贴内容之后。
    PasteAfterMove,
    /// `gP`：粘贴到光标之前，光标停在粘贴内容之后。
    PasteBeforeMove,
    /// `]p`：粘贴到光标之后，按行的内容调整到当前行的缩进。
    PasteIndented,
    /// `J`：合并行，行间留一个空格。
    JoinLines,
    /// `gJ`：合并行，不调整空白。
    JoinLinesRaw,
    /// `u`：撤销。
    Undo,
    /// `Ctrl-R`：重做。
    Redo,
    /// `.`：重复上一次修改。
    Repeat,
    /// `v`：进入或离开按字符的可视模式。
    Visual,
    /// `V`：进入或离开按行的可视模式。
    VisualLine,
    /// `gv`：恢复上一次的可视选区。
    Reselect,
    /// 可视模式的 `o`、`O`：交换选区的起点与光标。
    SwapAnchor,
    /// `<Esc>`：离开可视模式或插入模式。
    Escape,
    /// `Ctrl-V`、`Ctrl-Q`：进入或离开按块的可视模式。
    VisualBlock,
    /// 可视模式的 `I`：在选区的起点进入插入模式；按块时在各行的左边界插入。
    InsertSelection,
    /// 可视模式的 `A`：在选区的终点之后进入插入模式；按块时在各行的右边界之后插入。
    AppendSelection,
    /// 可视模式的 `O`：按块时把光标移到块的另一侧，其余与 `o` 相同。
    SwapColumn,
    /// `R`：进入替换模式。
    Replace,
    /// `Ctrl-A`：光标处或其后的数加上次数。
    Increment,
    /// `Ctrl-X`：光标处或其后的数减去次数。
    Decrement,
    /// `m{标记}`：把光标的位置记为标记。
    SetMark,
    /// `Ctrl-O`：回到跳转历史中的上一个位置。
    JumpBack,
    /// `Ctrl-I`、`<Tab>`：前进到跳转历史中的下一个位置。
    JumpForward,
    /// `g;`：到修改列表中更早的位置。
    ChangeOlder,
    /// `g,`：到修改列表中更新的位置。
    ChangeNewer,
    /// 普通模式的 `gn`：选中上一次查找的模式的下一处匹配。
    SelectNext,
    /// 普通模式的 `gN`：选中上一次查找的模式的上一处匹配。
    SelectPrevious,
    /// 滚动窗口。
    Scroll(Scrolling),
    /// 折叠命令。
    Fold(Folding),
    /// `Ctrl-C`：离开插入模式或可视模式；普通模式下没有效果。
    Interrupt,
    /// 插入模式的 `Ctrl-H`：删除光标之前的一个字符。
    InsertBackspace,
    /// 插入模式的 `Ctrl-W`：删除光标之前的一个词。
    InsertDeleteWord,
    /// 插入模式的 `Ctrl-U`：删除光标到行首之间的文本。
    InsertDeleteLine,
    /// 插入模式的 `Ctrl-T`：当前行增加一级缩进。
    InsertIndent,
    /// 插入模式的 `Ctrl-D`：当前行减少一级缩进。
    InsertOutdent,
    /// 插入模式的 `Ctrl-R {寄存器}`：插入寄存器的内容。
    InsertRegister,
    /// 插入模式的 `Ctrl-O`：执行一条普通模式的命令之后回到插入模式。
    InsertNormal,
    /// 插入模式的 `Ctrl-E`：插入下一行同一显示列上的字符。
    InsertBelow,
    /// 插入模式的 `Ctrl-Y`：插入上一行同一显示列上的字符。
    InsertAbove,
    /// `:`：输入命令行。
    CommandLine,
    /// `q{寄存器}`：开始记录宏。
    Record,
    /// `@{寄存器}`：回放寄存器中的按键。
    Play,
    /// `&`：在当前行上重复上一次替换。
    RepeatSubstitute,
    /// `g&`：在全部的行上重复上一次替换，沿用标志。
    RepeatSubstituteAll,
    /// `gd`：转到光标处名字的定义。
    Definition,
    /// `gf`：打开光标处的路径。
    OpenFile,
    /// `gt`：切换到下一个标签，带次数时切换到该序号的标签。
    TabNext,
    /// `gT`：切换到上一个标签。
    TabPrevious,
    /// `ga`：显示光标处字符的编码。
    CharInfo,
    /// `ZZ`：保存并关闭当前文档。
    WriteQuit,
    /// `ZQ`：不保存，关闭当前文档。
    QuitForce,
    /// `ds{符号}`：删除光标两侧成对的符号。
    DeleteSurround,
    /// `cs{符号}{符号}`：把光标两侧成对的符号换成另一对；值为原来的符号。
    ChangeSurround(char),
    /// `Ctrl-W` 加方向：把焦点移到侧栏、编辑区或终端。
    Focus(Focusing),
    /// `Ctrl-^`：切换到上一个活动的文档。
    Alternate,
    /// 之后的阶段实现的命令。
    Later,
    /// `Q`：进入 Ex 模式。
    ExMode,
    /// `K`：查询光标处的关键字。
    Keyword,
    /// `U`：撤销最近对一行的全部修改。
    UndoLine,
}

/// 焦点移动的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focusing {
    /// 左侧：侧栏。
    Left,
    /// 右侧：编辑区。
    Right,
    /// 下方：终端。
    Down,
    /// 上方：编辑区。
    Up,
    /// 下一个区域，依次轮换。
    Next,
}

/// 滚动命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scrolling {
    /// `Ctrl-E`：窗口向下滚动若干行。
    LineDown,
    /// `Ctrl-Y`：窗口向上滚动若干行。
    LineUp,
    /// `Ctrl-D`：窗口与光标向下移动半屏。
    HalfDown,
    /// `Ctrl-U`：窗口与光标向上移动半屏。
    HalfUp,
    /// `Ctrl-F`、`<PageDown>`、`<S-Down>`：向下翻页。
    PageDown,
    /// `Ctrl-B`、`<PageUp>`、`<S-Up>`：向上翻页。
    PageUp,
    /// `zt`：光标所在行移到窗口顶部。
    Top,
    /// `z<CR>`：光标所在行移到窗口顶部，光标移到第一个非空白字符。
    TopStart,
    /// `zz`：光标所在行移到窗口中间。
    Center,
    /// `z.`：光标所在行移到窗口中间，光标移到第一个非空白字符。
    CenterStart,
    /// `zb`：光标所在行移到窗口底部。
    Bottom,
    /// `z-`：光标所在行移到窗口底部，光标移到第一个非空白字符。
    BottomStart,
    /// `zh`：窗口向左滚动若干列。
    Left,
    /// `zl`：窗口向右滚动若干列。
    Right,
    /// `zH`：窗口向左滚动半屏。
    HalfLeft,
    /// `zL`：窗口向右滚动半屏。
    HalfRight,
    /// `zs`：横向滚动到光标位于窗口的左边界。
    CaretStart,
    /// `ze`：横向滚动到光标位于窗口的右边界。
    CaretEnd,
}

/// 折叠命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Folding {
    /// `zo`：展开光标处的折叠。
    Open,
    /// `zc`：收起光标处的折叠。
    Close,
    /// `za`：切换光标处的折叠。
    Toggle,
    /// `zO`：展开光标处的各层折叠。
    OpenTree,
    /// `zC`：收起光标处的各层折叠。
    CloseTree,
    /// `zA`：切换光标处的各层折叠。
    ToggleTree,
    /// `zR`：展开全部折叠。
    OpenAll,
    /// `zM`：收起全部折叠。
    CloseAll,
    /// `zv`：展开到光标所在的行可见。
    Reveal,
    /// `zj`：移到下一个可折叠区域的首行。
    Next,
    /// `zk`：移到上一个可折叠区域的末行。
    Previous,
}

/// 操作符之后强制指定的动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Force {
    /// `v`：按字符。
    Charwise,
    /// `V`：按行。
    Linewise,
    /// `Ctrl-V`：按块。
    Blockwise,
}

/// 同义写法展开后操作符的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shorthand {
    /// 一个不带参数的动作，例如 `x` 等同 `dl`。
    Motion(Motion),
    /// 叠写形式，例如 `S` 等同 `cc`。
    Lines,
}

/// 记录的种类，决定解析器在按键序列匹配之后的处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 动作。
    Motion(Motion),
    /// 操作符；可视模式下直接作用于选区。
    Operator(Operator),
    /// 可视模式下把选区扩展到整行之后执行的操作符（`S`、`R`）。
    LineOperator(Operator),
    /// 可视模式下作用于整行的操作符（`X`、`Y`、`D`、`C`）；按块时作用于块，第二项为真时块
    /// 延伸到各行的行尾。
    WideOperator(Operator, bool),
    /// 可视模式的 `<BS>`：删除选区；未指定寄存器时写入 `"-`。
    SmallDelete,
    /// 操作符加动作的同义写法（`x`、`D`、`Y` 等），解析后展开成操作符命令。
    Synonym(Operator, Shorthand),
    /// 文本对象。
    TextObject(TextObject),
    /// 成对符号：`y`、`d`、`c` 之后的 `s`，以及可视模式的 `S`。
    Surround,
    /// 不带范围的命令。
    Action(Action),
    /// 寄存器前缀 `"`：其后的字符是寄存器名，之后继续输入命令。
    Register,
    /// 操作符之后强制动作类型的前缀，之后继续输入动作。
    Force(Force),
}

/// 按键序列之后的参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arg {
    /// 没有参数。
    None,
    /// 一个字符，例如 `f` 查找的字符、`m` 的标记名。
    Char,
}

/// 记录的支持状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Support {
    /// 已实现，且有录制用例核对。
    Supported,
    /// 在兼容目标之内，尚未实现；输入后提示不支持。
    Planned,
    /// 不支持：输入后提示不支持，不执行任何操作。
    Unsupported,
}

/// 命令表中的一条记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// 按键序列（按键记法），不含次数、寄存器与参数。
    pub keys: &'static str,
    /// 记录生效的模式。
    pub modes: Modes,
    /// 种类。
    pub kind: Kind,
    /// 按键序列之后的参数。
    pub arg: Arg,
    /// 支持状态。
    pub support: Support,
}

/// 寄存器名的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterName {
    /// 可以使用的寄存器。
    Valid,
    /// 寄存器存在，但不在支持范围内（表达式寄存器 `=`、轮换文件名 `#`）。
    Unsupported,
    /// 不是寄存器名。
    Invalid,
}

/// 返回字符 `c` 作为寄存器名的类别。
pub(crate) fn register_name(c: char) -> RegisterName {
    match c {
        '"' | '-' | '_' | '+' | '*' | '/' | ':' | '.' | '%' => RegisterName::Valid,
        c if c.is_ascii_alphanumeric() => RegisterName::Valid,
        '=' | '#' => RegisterName::Unsupported,
        _ => RegisterName::Invalid,
    }
}

#[cfg(test)]
mod tests;
