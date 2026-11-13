//! 逐行词法分析：按 [`Language`] 的规则表把一行切成 [`Token`]。
//!
//! 跨行的结构（块注释、多行字符串、here 文档）用 [`LineState`] 从一行传到下一行，调用方按行
//! 保存状态。本模块处理代码与配置格式的通用规则，Markdown、标记语言和样式表的专门规则分别
//! 在 `markdown`、`markup`、`style` 子模块中。

mod data;
mod markdown;
mod markup;
mod shell;
mod strings;
mod style;

use crate::lang::{Heredocs, Language, Markup, RawStrings, Variables, language_at};

/// 行开头的词法状态：上一行是否停在尚未结束的注释或字符串里，以及是哪种结构。
///
/// 大小不超过 4 字节，可以每行存一项放在 [`crate::Highlighter`] 的状态表里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineState {
    /// 没有跨越行首的结构。
    #[default]
    Normal,
    /// 块注释：`index` 是 [`Language::block_comments`] 中的下标，`depth` 是嵌套层数（从 1 起）。
    BlockComment {
        /// 块注释写法的下标。
        index: u8,
        /// 嵌套层数（从 1 起）。
        depth: u8,
    },
    /// `"""` 三引号字符串。
    TripleDouble,
    /// `'''` 三引号字符串。
    TripleSingle,
    /// 反引号字符串或行内代码。
    Backtick,
    /// 跨行的双引号字符串。
    DoubleQuote,
    /// 跨行的单引号字符串。
    SingleQuote,
    /// `r"…"` 原始字符串，收尾是单个 `"`。
    Raw0,
    /// `r#"…"#`、`#"…"#` 原始字符串，收尾是 `"#`。
    Raw1,
    /// `r##"…"##`、`##"…"##` 原始字符串，收尾是 `"##`。
    Raw2,
    /// Markdown 的代码块（[`Language::markdown`]）。
    Fence {
        /// 是否以 `~` 围起；为假时以反引号围起。
        tilde: bool,
        /// 信息字符串指明的语言在 [`crate::languages`] 中的下标；没有指明或认不出时为
        /// `u8::MAX`，此时内容整块按字符串着色。
        language: u8,
        /// 代码所用语言的行首状态的紧凑编码（见 `compact`）。
        inner: u8,
    },
    /// C++ 原始字符串（[`RawStrings::Cpp`]），值是定界符的散列值（见 `delimiter_hash`）。
    CppRaw(u16),
    /// here 文档的内容（[`Language::heredocs`]）。
    ///
    /// 值的最高位表示结束行可以缩进，次高位表示结束标记带引号，低 14 位是结束标记的散列值。
    Heredoc(u16),
    /// 长括号字符串 `[==[…]==]`（Lua 的长字符串、`CMake` 的括号参数），值是 `=` 的个数。
    LongString(u8),
    /// 长括号注释（Lua 的 `--[==[…]==]`、`CMake` 的 `#[==[…]==]`），值是 `=` 的个数。
    LongComment(u8),
    /// 块字符串，值是 [`Language::block_strings`] 中的下标。
    BlockString(u8),
    /// Ruby 的百分号字面量（[`Language::percent_literals`]）：`open` 是开头的定界符（ASCII），
    /// `depth` 是括号定界符的嵌套层数（从 1 起）。
    Percent {
        /// 开头的定界符。
        open: u8,
        /// 括号定界符的嵌套层数（从 1 起）。
        depth: u8,
    },
    /// 标记语言的标签内部（属性之间）。
    ///
    /// 值表示标签结束后的内容：0 是普通内容，1 是脚本，2 是样式。
    Tag(u8),
    /// HTML 中内嵌的脚本（`script` 为真）或样式；`inner` 是内嵌语言的行首状态，见 `compact`。
    Embedded {
        /// 是否为脚本；为假时是样式。
        script: bool,
        /// 内嵌语言的行首状态的紧凑编码。
        inner: u8,
    },
    /// 样式表中跨行的声明值（`font-family:` 之后以逗号结尾的续行）。
    StyleValue,
    /// YAML 块标量的内容（[`Language::block_scalars`]）。
    ///
    /// 值是所属键的列（字符偏移，从 0 起），缩进比它深的行都是内容；列超过 255 的键不识别块标量。
    BlockScalar(u8),
}

/// 把内嵌代码（HTML 中的脚本与样式、Markdown 代码块）的行首状态编码成一个字节。
///
/// 外层状态只有一个字节存放内层状态，这里只覆盖常见的跨行结构；带散列值或数据超出编码范围
/// 的状态（here 文档、C++ 原始字符串、YAML 块标量、再次内嵌的代码）按 `Normal` 处理，只影响
/// 内嵌代码中的那一处。
///
/// - `0` 到 `9`：不带数据的状态。
/// - `0x10 | i`：块字符串 `i`（`i` 小于 16）。
/// - `0x20 | k`：标签内部（`k` 小于 16）。
/// - `0x40 | 下标 << 3 | 层数`：块注释（下标和层数都小于 8）。
/// - `0x80 | n`、`0xC0 | n`：长括号字符串和注释（`n` 小于 64）。
fn compact(state: LineState) -> u8 {
    match state {
        LineState::Backtick => 1,
        LineState::StyleValue => 2,
        LineState::TripleDouble => 3,
        LineState::TripleSingle => 4,
        LineState::DoubleQuote => 5,
        LineState::SingleQuote => 6,
        LineState::Raw0 => 7,
        LineState::Raw1 => 8,
        LineState::Raw2 => 9,
        LineState::BlockString(index) if index < 0x10 => 0x10 | index,
        LineState::Tag(kind) if kind < 0x10 => 0x20 | kind,
        LineState::BlockComment { index, depth } if index < 8 && depth < 8 => {
            0x40 | index << 3 | depth
        }
        LineState::LongString(level) if level < 0x40 => 0x80 | level,
        LineState::LongComment(level) if level < 0x40 => 0xC0 | level,
        _ => 0,
    }
}

/// [`compact`] 的逆变换。
fn expand(inner: u8) -> LineState {
    match inner {
        1 => LineState::Backtick,
        2 => LineState::StyleValue,
        3 => LineState::TripleDouble,
        4 => LineState::TripleSingle,
        5 => LineState::DoubleQuote,
        6 => LineState::SingleQuote,
        7 => LineState::Raw0,
        8 => LineState::Raw1,
        9 => LineState::Raw2,
        0x10..=0x1F => LineState::BlockString(inner & 0x0F),
        0x20..=0x2F => LineState::Tag(inner & 0x0F),
        0x40..=0x7F => LineState::BlockComment {
            index: (inner >> 3) & 7,
            depth: (inner & 7).max(1),
        },
        0x80..=0xBF => LineState::LongString(inner & 0x3F),
        0xC0..=0xFF => LineState::LongComment(inner & 0x3F),
        _ => LineState::Normal,
    }
}

// 状态表每行一项，大文件有上百万行，带数据的变体不能使状态变大。
const _: () = assert!(size_of::<LineState>() <= 4);

/// [`LineState::Heredoc`] 中表示结束行可以缩进的位（`<<-`、`<<~`；PHP 总是可以）。
const HEREDOC_DASH: u16 = 0x8000;
/// [`LineState::Heredoc`] 中表示结束标记带引号的位：内容中不展开变量。
const HEREDOC_QUOTED: u16 = 0x4000;
/// [`LineState::Heredoc`] 中结束标记散列值所占的位。
///
/// 14 位：结束行按整行比较散列值，散列值相同的行会被误当作结束行（概率约为 1/16384）。
const HEREDOC_HASH: u16 = 0x3FFF;

/// 原始字符串的收尾定界符，下标是开头 `#` 的个数。
///
/// `#` 多于 2 个的写法很少见，不识别为原始字符串，按普通字符串处理。
const RAW_CLOSE: [&str; 3] = ["\"", "\"#", "\"##"];
/// 与 [`RAW_CLOSE`] 下标一一对应的行首状态。
const RAW_STATES: [LineState; 3] = [LineState::Raw0, LineState::Raw1, LineState::Raw2];

/// 字符串插值的最大嵌套层数（层）。
///
/// 经验值：实际代码很少超过 3 层。限制层数使递归深度与行长无关；超过时插值的开头按
/// 字符串内容处理。
const MAX_TEMPLATE_DEPTH: u8 = 8;

/// 之后可以紧跟字符串的关键字（见 [`Language::prose_quotes`]）。
const STRING_AFTER_KEYWORDS: &[&str] = &[
    "return",
    "from",
    "import",
    "export",
    "module",
    "default",
    "as",
    "assert",
    "with",
    "case",
    "throw",
    "yield",
    "await",
    "typeof",
    "void",
    "delete",
    "in",
    "of",
    "instanceof",
    "new",
    "else",
    "do",
];

/// 之后的斜杠开始正则表达式、而非除号的关键字（JS、Ruby、Perl）。
const REGEX_AFTER_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "if",
    "unless",
    "while",
    "until",
    "when",
    "and",
    "or",
    "not",
    "split",
    "grep",
];

/// [`Token`] 的类别，决定高亮使用主题中的哪种颜色。
///
/// 类别按词法结构划分，主题可以把几种类别映射到同一种颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// 普通文本：没有被任何 token 覆盖的部分。
    Text,
    /// 语言关键字。
    Keyword,
    /// 内置常量、内置函数和内置类型名（语言表中列出的名字）。
    Builtin,
    /// 按命名习惯识别的类型名（见 [`Language::capitalized_types`]）。
    Type,
    /// 字符串或字符字面量。
    String,
    /// 字符串中的转义序列（`\n`、`\u{1F600}`），以及标记语言的字符实体（`&amp;`）。
    Escape,
    /// 数字字面量，以及样式表中的十六进制颜色。
    Number,
    /// 注释。
    Comment,
    /// 函数名：后面紧跟 `(` 的标识符。
    Function,
    /// 装饰器、注解、属性，以及以 `#` 开头的预处理指令。
    Decorator,
    /// 配置文件中的键、样式表中的属性名（见 [`Language::key_separators`]）。
    Property,
    /// 变量引用（见 [`Language::variables`]），以及符号（见 [`Language::colon_symbols`]）。
    Variable,
    /// 标记语言的标签名、样式表选择器中的元素名。
    Tag,
    /// 标记语言的属性名、样式表选择器中的类名和 id。
    Attribute,
    /// 标题：Markdown 的标题行、TOML 的表头、INI 的节名。
    Heading,
    /// Markdown 的强调（`*a*`）与删除线（`~~a~~`）。
    Emphasis,
    /// Markdown 的加粗（`**a**`）。
    Strong,
    /// Markdown 的链接文字与自动链接。
    Link,
    /// 差异文件中新增的行。
    Inserted,
    /// 差异文件中删除的行。
    Deleted,
}

/// 行内需要着色的一段文本。
///
/// 范围用行内字符偏移表示，没有被任何 token 覆盖的部分按普通文本绘制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// 起点（行内字符偏移，从 0 起，包含）。
    pub start: usize,
    /// 终点（行内字符偏移，从 0 起，不含）。
    pub end: usize,
    /// 着色使用的类别。
    pub kind: TokenKind,
}

/// 字符串的结束规则。
struct Span {
    /// 结束定界符。
    close: &'static str,
    /// 转义字符是否使下一个字符不收尾（见 [`Language::escape_char`]）。
    escapes: bool,
    /// 转义序列是否单独着色（[`TokenKind::Escape`]）；原始字符串为假。
    escape_tokens: bool,
    /// 连写两个结束定界符是否表示字面的定界符（[`Language::doubled_quotes`]）。
    doubled: bool,
    /// 整段着色使用的类别。
    kind: TokenKind,
    /// 到行尾还没结束时带到下一行的状态；不能跨行的结构为 `Normal`。
    carry: LineState,
    /// 其中按代码着色的插值的开头和收尾字符，规则见 [`Language::interpolation`] 与
    /// [`Language::template_interpolation`]。
    interpolation: Option<(&'static str, char)>,
    /// 其中的变量引用是否单独着色（[`Language::variables`]）。
    variables: bool,
}

/// 字符串前缀（`r"…"`、`f"…"`、`$"…"`）的效果。
#[derive(Debug, Clone, Copy, Default)]
struct Prefix {
    /// 是否为原始字符串（[`Language::raw_prefixes`]）。
    raw: bool,
    /// 是否支持插值（[`crate::Interpolation::prefixes`]）。
    interpolated: bool,
}

impl Prefix {
    /// 返回不带前缀的字符串的效果：插值不要求前缀时支持插值。
    fn plain(language: &Language) -> Self {
        Prefix {
            raw: false,
            interpolated: language
                .interpolation
                .is_some_and(|it| it.prefixes.is_empty()),
        }
    }
}

/// 返回以 `close` 结束的引号字符串的规则。
///
/// `single` 表示单引号字符串；`carry` 是到行尾仍未结束时带到下一行的状态。
fn quoted_span(
    language: &Language,
    close: &'static str,
    single: bool,
    prefix: Prefix,
    carry: LineState,
) -> Span {
    let escapes = !single || language.single_quote_escapes;
    // 前缀可以同时表示原始字符串和插值（Python 的 `rf"…"`），见 `Lexer::prefix`。
    let interpolation = language
        .interpolation
        .filter(|it| prefix.interpolated && (!single || it.single_quotes));
    let variables = match language.variables {
        Variables::None | Variables::Ruby => false,
        Variables::InStrings => interpolation.is_some(),
        _ => !single && !prefix.raw,
    };
    Span {
        close,
        escapes,
        escape_tokens: escapes && !prefix.raw,
        doubled: language.doubled_quotes,
        kind: TokenKind::String,
        carry,
        interpolation: interpolation.map(|it| (it.open, it.close)),
        variables,
    }
}

/// 返回没有转义、整段按 `kind` 着色的规则（原始字符串、带引号的标识符）。
fn literal_span(close: &'static str, kind: TokenKind, carry: LineState) -> Span {
    Span {
        close,
        escapes: false,
        escape_tokens: false,
        doubled: false,
        kind,
        carry,
        interpolation: None,
        variables: false,
    }
}

impl LineState {
    /// 返回处于该状态时继续扫描所用的字符串规则。
    ///
    /// 不是字符串的状态，以及收尾不是固定字符串的状态（C++ 原始字符串、here 文档）返回 `None`。
    fn span(self, language: &Language) -> Option<Span> {
        let plain = Prefix::plain(language);
        match self {
            LineState::TripleDouble => Some(quoted_span(language, "\"\"\"", false, plain, self)),
            LineState::TripleSingle => Some(quoted_span(language, "'''", true, plain, self)),
            LineState::DoubleQuote => Some(quoted_span(language, "\"", false, plain, self)),
            LineState::SingleQuote => Some(quoted_span(language, "'", true, plain, self)),
            LineState::Backtick => Some(backtick_span(language)),
            LineState::Raw0 | LineState::Raw1 | LineState::Raw2 => {
                let hashes = RAW_STATES.iter().position(|&s| s == self)?;
                Some(literal_span(RAW_CLOSE[hashes], TokenKind::String, self))
            }
            LineState::BlockString(index) => block_string_span(language, index),
            _ => None,
        }
    }
}

/// 返回反引号字符串的规则。
///
/// JS 的模板字符串用反引号，支持 `${…}` 插值；SQL 的反引号是标识符。
fn backtick_span(language: &Language) -> Span {
    if language.quoted_identifiers {
        return literal_span("`", TokenKind::Property, LineState::Normal);
    }
    Span {
        close: "`",
        escapes: language.backtick_escapes,
        escape_tokens: language.backtick_escapes,
        doubled: false,
        kind: TokenKind::String,
        carry: LineState::Backtick,
        interpolation: language.template_interpolation.then_some(("${", '}')),
        variables: !matches!(
            language.variables,
            Variables::None | Variables::InStrings | Variables::Ruby
        ),
    }
}

/// 返回第 `index` 种块字符串的规则；下标超出范围时返回 `None`。
fn block_string_span(language: &Language, index: u8) -> Option<Span> {
    let block = language.block_strings.get(usize::from(index))?;
    let interpolation = language.interpolation.filter(|_| block.interpolate);
    Some(Span {
        close: block.close,
        escapes: false,
        escape_tokens: false,
        doubled: language.doubled_quotes,
        kind: TokenKind::String,
        carry: LineState::BlockString(index),
        interpolation: interpolation.map(|it| (it.open, it.close)),
        variables: block.interpolate && language.variables != Variables::None,
    })
}

/// 对一行做词法分析，生成 token 并返回下一行开头的状态。
///
/// `chars` 是这一行的字符（不含换行符），`state` 是这一行开头的状态；`tokens` 先被清空，
/// 之后按位置从小到大存放本行的 token。
pub fn highlight_line(
    language: &Language,
    chars: &[char],
    state: LineState,
    tokens: &mut Vec<Token>,
) -> LineState {
    tokens.clear();
    Lexer::new(language, chars, Some(tokens), 0).run(state)
}

/// 只推算下一行开头的状态，不生成 token，也不查关键字表。
///
/// 调用方不需要 token 时用它代替 [`highlight_line`]，省去 token 的生成与内存分配。
pub(crate) fn scan_line(language: &Language, chars: &[char], state: LineState) -> LineState {
    Lexer::new(language, chars, None, 0).run(state)
}

/// 对一行字符做词法分析的临时状态。
struct Lexer<'a> {
    /// 这一行使用的语言规则。
    language: &'a Language,
    /// 要分析的字符（不含换行符）；内嵌代码是所在行的一段。
    chars: &'a [char],
    /// 接收 token 的缓冲；为 `None` 时只推算行末状态。
    tokens: Option<&'a mut Vec<Token>>,
    /// `chars[0]` 在所在行中的位置（字符偏移，从 0 起），记录 token 时加上；内嵌代码之外为 0。
    base: usize,
    /// 当前所在的字符串插值的嵌套层数（层）；0 表示不在插值中。
    depth: u8,
    /// 是否已确认当前插值及其内层插值都在本行闭合。
    ///
    /// 为真时不再预先试扫，见 `Lexer::interpolation`。
    closes_known: bool,
    /// 本行中第一个 here 文档的状态，内容从下一行开始；没有时为 `None`。
    heredoc: Option<LineState>,
    /// 样式表：当前是否处于选择器中（`{` 之前）。
    selector: bool,
    /// 样式表：下一个标识符是否可能是声明的属性名（行首、`{` 或 `;` 之后）。
    expect_property: bool,
    /// Markdown：当前行是否是以 `|` 开头的表格行，其中的 `|` 是列分隔符。
    table_row: bool,
    /// 样式表：当前是否处于声明的值中（属性名之后、`;` 或 `}` 之前）。
    ///
    /// 到行尾仍为真时值延续到下一行。
    in_value: bool,
}

impl<'a> Lexer<'a> {
    /// 创建分析 `chars` 的临时状态；`base` 的含义见 [`Lexer::base`]。
    fn new(
        language: &'a Language,
        chars: &'a [char],
        tokens: Option<&'a mut Vec<Token>>,
        base: usize,
    ) -> Self {
        Lexer {
            language,
            chars,
            tokens,
            base,
            depth: 0,
            closes_known: false,
            heredoc: None,
            selector: false,
            expect_property: false,
            in_value: false,
            table_row: false,
        }
    }
}

impl Lexer<'_> {
    /// 把 `start..end` 记成一个 token；区间为空或只推算状态时不记录。
    fn push(&mut self, start: usize, end: usize, kind: TokenKind) {
        if end > start
            && let Some(tokens) = self.tokens.as_deref_mut()
        {
            tokens.push(Token {
                start: self.base + start,
                end: self.base + end,
                kind,
            });
        }
    }

    /// 已记录的 token 个数；只推算状态时为 0。
    fn token_count(&self) -> usize {
        self.tokens.as_deref().map_or(0, Vec::len)
    }

    /// 把第 `from` 个起记录的 token 改为 `kind`。
    fn retag(&mut self, from: usize, kind: TokenKind) {
        if let Some(tokens) = self.tokens.as_deref_mut() {
            for token in &mut tokens[from..] {
                token.kind = kind;
            }
        }
    }

    /// 返回本行结束后下一行开头的状态。
    ///
    /// `state` 是行尾仍未结束的结构；为 `Normal` 且本行打开了 here 文档时，下一行是
    /// here 文档的内容。
    fn line_end(&self, carry: LineState) -> LineState {
        match (carry, self.heredoc) {
            (LineState::Normal, Some(heredoc)) => heredoc,
            (LineState::Normal, None) if self.in_value => LineState::StyleValue,
            (LineState::Normal, None) if self.language.block_scalars => self
                .block_scalar_start()
                .map_or(LineState::Normal, LineState::BlockScalar),
            _ => carry,
        }
    }

    /// 从 `state` 起分析整行，返回下一行开头的状态。
    fn run(mut self, state: LineState) -> LineState {
        if self.language.markup != Markup::None {
            return self.markup_line(state);
        }

        let n = self.chars.len();

        if self.language.markdown
            && let Some(next) = self.fence(state)
        {
            return next;
        }

        let mut i = match self.resume(state) {
            Ok(i) => i,
            Err(next) => return next,
        };

        if i == 0 {
            if self.language.diff_lines {
                return self.diff_line();
            }

            if self.language.hash_lines
                && let Some(first) = self.chars.iter().position(|c| !c.is_whitespace())
                && self.chars[first] == '#'
                && !matches!(self.chars.get(first + 1), Some('"' | '#' | '['))
            {
                self.push(first, n, TokenKind::Decorator);
                return LineState::Normal;
            }

            if self.language.markdown {
                match self.markdown_block() {
                    Ok(end) => i = end,
                    Err(next) => return next,
                }
            }

            if self.language.section_headers
                && let Some(end) = self.section_header()
            {
                i = end;
            }

            if self.language.stylesheet {
                self.style_line_start(state);
            }

            // 跨行声明值的续行（`  Helvetica, sans-serif;`）不会以属性名开头。
            if i == 0
                && state != LineState::StyleValue
                && !self.language.key_separators.is_empty()
                && let Some(end) = self.key()
            {
                i = end;
                self.selector = false;
                self.in_value = self.language.stylesheet;
            }
        }

        while i < n {
            match self.step(i) {
                Ok(next) => i = next,
                Err(next) => return next,
            }
        }

        self.line_end(LineState::Normal)
    }

    /// 继续上一行未结束的结构，返回结构之后的位置；到行尾仍未结束时返回下一行开头的状态。
    fn resume(&mut self, state: LineState) -> Result<usize, LineState> {
        match state {
            LineState::Normal => Ok(0),
            LineState::Heredoc(tag) => Err(self.heredoc_line(tag)),
            LineState::CppRaw(hash) => self.cpp_raw_close(0, 0, hash).ok_or(state),
            LineState::BlockComment { index, depth } => self.comment_close(0, 0, index, depth),
            LineState::LongString(level) => self.long_close(0, 0, level, false),
            LineState::LongComment(level) => self.long_close(0, 0, level, true),
            LineState::Percent { open, depth } => self.percent_close(0, 0, open, depth),
            LineState::BlockScalar(key) => self.block_scalar_line(key),
            _ => match state.span(self.language) {
                Some(span) => self.close(0, 0, &span).ok_or(span.carry),
                None => Ok(0),
            },
        }
    }

    /// 分析 `i` 处的一个结构（注释、字符串或单词），返回结构之后的位置；结构到行尾仍未结束时
    /// 返回下一行开头的状态。
    fn step(&mut self, i: usize) -> Result<usize, LineState> {
        let language = self.language;
        let n = self.chars.len();

        if language.hash_attributes
            && let Some(end) = self.hash_attribute(i)
        {
            return Ok(end);
        }

        if let Some(comment) = language.long_brackets
            && let Some((end, level, comment)) = self.long_opening(i, comment)
        {
            return self.long_close(i, end, level, comment);
        }

        if let Some((index, len)) = self.comment_opening(i) {
            return self.comment_close(i, i + len, index, 1);
        }

        if language.percent_literals
            && let Some((end, open)) = self.percent_opening(i)
        {
            return self.percent_close(i, end, open, 1);
        }

        if self.at_line_comment(i) {
            self.push(i, n, TokenKind::Comment);
            return Err(self.line_end(LineState::Normal));
        }

        if let Some(marker) = language.line_string
            && starts_with(self.chars, i, marker)
        {
            self.push(i, n, TokenKind::String);
            return Err(self.line_end(LineState::Normal));
        }

        if language.raw_strings == RawStrings::Cpp
            && let Some((open_end, hash)) = self.cpp_raw_opening(i)
        {
            return self
                .cpp_raw_close(i, open_end, hash)
                .ok_or(LineState::CppRaw(hash));
        }

        if let Some((open, span)) = self.opening(i) {
            let before = self.token_count();
            let end = self
                .close(i, i + open, &span)
                .ok_or_else(|| self.line_end(span.carry))?;
            if language.quoted_keys && self.chars[i] == '"' && self.followed_by_colon(end) {
                self.retag(before, TokenKind::Property);
            }
            return Ok(end);
        }

        Ok(self.token(i))
    }

    /// 按行首字符为差异文件的一行整行着色，返回下一行开头的状态。
    fn diff_line(&mut self) -> LineState {
        let line = self.chars;
        let kind =
            if line.starts_with(&['+', '+', '+', ' ']) || line.starts_with(&['-', '-', '-', ' ']) {
                Some(TokenKind::Keyword)
            } else if line.starts_with(&['@', '@']) {
                Some(TokenKind::Function)
            } else {
                match line.first() {
                    Some('+') => Some(TokenKind::Inserted),
                    Some('-') => Some(TokenKind::Deleted),
                    Some('\\') => Some(TokenKind::Comment),
                    _ => None,
                }
            };
        if let Some(kind) = kind {
            self.push(0, line.len(), kind);
        }
        LineState::Normal
    }

    /// `end` 之后（隔着空白）是否紧跟 `:`。
    fn followed_by_colon(&self, end: usize) -> bool {
        self.chars[end..].iter().find(|c| !c.is_whitespace()) == Some(&':')
    }

    /// 从 `from` 开始查找 `span` 的结束符，把 `start` 到结束符记成 token，返回结束位置。
    ///
    /// 转义序列、插值和变量单独记成 token。到行尾仍未结束时，剩下的部分整个记成 token，
    /// 返回 `None`。
    fn close(&mut self, start: usize, from: usize, span: &Span) -> Option<usize> {
        let n = self.chars.len();
        let escape = self.language.escape_char;
        let close_len = span.close.chars().count();
        let mut start = start;
        let mut j = from;

        while j < n {
            let c = self.chars[j];
            if let Some((open, closer)) = span.interpolation
                && starts_with(self.chars, j, open)
            {
                // `{{` 是字面的花括号（C#、Python 的插值字符串）。
                if open == "{" && self.chars.get(j + 1) == Some(&'{') {
                    j += 2;
                    continue;
                }
                if let Some(end) = self.interpolation(start, j + open.chars().count(), closer) {
                    // 收尾的括号与紧随其后的字符串内容一起着色。
                    start = end - 1;
                    j = end;
                    continue;
                }
            }

            if span.escapes && c == escape {
                let len = escape_len(self.chars, j);
                if span.escape_tokens {
                    self.push(start, j, span.kind);
                    self.push(j, (j + len).min(n), TokenKind::Escape);
                    start = (j + len).min(n);
                }
                j += len.max(2);
            } else if span.variables
                && matches!(c, '$' | '%' | '!')
                && let Some((end, kind)) = self.variable(j, true)
            {
                self.push(start, j, span.kind);
                self.push(j, end, kind);
                start = end;
                j = end;
            } else if starts_with(self.chars, j, span.close) {
                if span.doubled && starts_with(self.chars, j + close_len, span.close) {
                    j += 2 * close_len;
                    continue;
                }
                let end = j + close_len;
                self.push(start, end, span.kind);
                return Some(end);
            } else {
                j += 1;
            }
        }

        self.push(start, n.max(start), span.kind);
        None
    }

    /// 返回 `i` 处开始的块注释写法的下标和开头定界符的长度（字符）。
    ///
    /// `i` 处不是块注释开头，或块注释写法的下标超出 `u8` 的范围时返回 `None`。
    fn comment_opening(&self, i: usize) -> Option<(u8, usize)> {
        let language = self.language;
        if language.line_start_comments && i != 0 {
            return None;
        }
        let index = language
            .block_comments
            .iter()
            .position(|(open, _)| starts_with(self.chars, i, open))?;
        let len = language.block_comments[index].0.chars().count();
        Some((u8::try_from(index).ok()?, len))
    }

    /// 从 `from` 开始查找第 `index` 种块注释的收尾，把 `start` 到收尾记成注释，返回收尾之后的
    /// 位置；到行尾仍未结束时返回下一行开头的状态。
    ///
    /// `depth` 是进入时的嵌套层数。语言允许嵌套时（[`Language::nested_comments`]），内部的开头
    /// 使层数加一；只在行首识别时（[`Language::line_start_comments`]），收尾行整行是注释。
    fn comment_close(
        &mut self,
        start: usize,
        from: usize,
        index: u8,
        depth: u8,
    ) -> Result<usize, LineState> {
        let language = self.language;
        let n = self.chars.len();
        let Some(&(open, close)) = language.block_comments.get(usize::from(index)) else {
            return Ok(from);
        };

        if language.line_start_comments {
            self.push(start, n, TokenKind::Comment);
            return if from == 0 && starts_with(self.chars, 0, close) {
                Ok(n)
            } else {
                Err(LineState::BlockComment { index, depth })
            };
        }

        let mut depth = depth;
        let mut j = from;
        while j < n {
            if language.nested_comments && starts_with(self.chars, j, open) {
                depth = depth.saturating_add(1);
                j += open.chars().count();
            } else if starts_with(self.chars, j, close) {
                j += close.chars().count();
                depth -= 1;
                if depth == 0 {
                    self.push(start, j, TokenKind::Comment);
                    return Ok(j);
                }
            } else {
                j += 1;
            }
        }
        self.push(start, n, TokenKind::Comment);
        Err(LineState::BlockComment { index, depth })
    }

    /// `i` 处是否开始行注释。
    ///
    /// 语言要求注释符之前是空白时（`comment_needs_space`），`i` 必须在行首，或前一个字符是空白。
    fn at_line_comment(&self, i: usize) -> bool {
        self.language
            .line_comments
            .iter()
            .any(|prefix| starts_with(self.chars, i, prefix))
            && (!self.language.comment_needs_space || i == 0 || self.chars[i - 1].is_whitespace())
    }

    /// 识别 `i` 处的 `#[`、`#![` 属性，把开头连同路径记成装饰器，返回其后的位置。
    ///
    /// `i` 处不是属性时返回 `None`。
    fn hash_attribute(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        if chars[i] != '#' {
            return None;
        }
        let open = if chars.get(i + 1) == Some(&'[') {
            i + 2
        } else if starts_with(chars, i + 1, "![") {
            i + 3
        } else {
            return None;
        };
        let end = scan(chars, open, |c| is_name(c) || c == ':' || c == '\\');
        self.push(i, end, TokenKind::Decorator);
        Some(end)
    }

    /// 返回 `i` 处字符串之前紧邻的前缀的效果（见 [`Prefix`]）。
    fn prefix(&self, i: usize) -> Prefix {
        let language = self.language;
        let start = self.chars[..i]
            .iter()
            .rposition(|&c| !(is_ident(c) || c == '@'))
            .map_or(0, |s| s + 1);
        if start == i {
            return Prefix::plain(language);
        }
        let mut buf = [0u8; 24];
        // 前缀都是短的 ASCII 单词；更长或含其他字符的单词不是前缀。
        let word = ascii_word(&self.chars[start..i], &mut buf, false).unwrap_or("");
        let raw = language.raw_prefixes.contains(&word);
        let interpolated = language.interpolation.is_some_and(|it| {
            if it.prefixes.is_empty() {
                !raw
            } else {
                it.prefixes.contains(&word)
            }
        });
        Prefix { raw, interpolated }
    }

    /// 返回 `i` 处开始的字符串的开头定界符长度（字符）和结束规则。
    ///
    /// `i` 处不是字符串开头时返回 `None`。
    fn opening(&self, i: usize) -> Option<(usize, Span)> {
        let language = self.language;
        let c = self.chars[i];

        if let Some(index) = language
            .block_strings
            .iter()
            .position(|b| starts_with(self.chars, i, b.open))
        {
            let len = language.block_strings[index].open.chars().count();
            return Some((len, block_string_span(language, u8::try_from(index).ok()?)?));
        }

        if language.triple_quotes {
            for (open, single, state) in [
                ("\"\"\"", false, LineState::TripleDouble),
                ("'''", true, LineState::TripleSingle),
            ] {
                if starts_with(self.chars, i, open) {
                    let span = quoted_span(language, open, single, self.prefix(i), state);
                    return Some((3, span));
                }
            }
        }

        if matches!(language.raw_strings, RawStrings::Rust | RawStrings::Swift)
            && let Some((open, state)) = self.raw_opening(i)
        {
            return Some((open, state.span(language)?));
        }

        match c {
            '"' if language.quoted_identifiers => Some((
                1,
                Span {
                    doubled: true,
                    ..literal_span("\"", TokenKind::Property, LineState::Normal)
                },
            )),
            '"' if !language.double_quote_string => None,
            '"' | '\'' if language.prose_quotes && self.after_word(i) => None,
            '"' => {
                let carry = if language.multiline_strings {
                    LineState::DoubleQuote
                } else {
                    LineState::Normal
                };
                Some((1, quoted_span(language, "\"", false, self.prefix(i), carry)))
            }
            '\'' if language.single_quote_string && !self.apostrophe(i) => {
                let carry = if language.multiline_strings {
                    LineState::SingleQuote
                } else {
                    LineState::Normal
                };
                Some((1, quoted_span(language, "'", true, self.prefix(i), carry)))
            }
            '`' if language.backtick_string => Some((1, backtick_span(language))),
            _ => None,
        }
    }

    /// `i` 处的引号之前（隔着空白）是否是普通单词，而不是之后可以跟字符串的关键字（见
    /// [`Language::prose_quotes`]）。
    fn after_word(&self, i: usize) -> bool {
        let chars = self.chars;
        let Some(end) = chars[..i].iter().rposition(|c| !c.is_whitespace()) else {
            return false;
        };
        if !is_ident(chars[end]) {
            return false;
        }
        let start = chars[..end]
            .iter()
            .rposition(|&c| !is_ident(c))
            .map_or(0, |k| k + 1);
        let mut buf = [0u8; 24];
        !ascii_word(&chars[start..=end], &mut buf, false)
            .is_some_and(|w| STRING_AFTER_KEYWORDS.contains(&w))
    }

    /// 识别 `i` 处的一个单词、数字、字符字面量或装饰器，返回其末尾之后的位置。
    ///
    /// 不产生 token 时同样推进到下一个字符，保证调用方总能前进。
    fn token(&mut self, i: usize) -> usize {
        let language = self.language;
        let chars = self.chars;
        let n = chars.len();
        let c = chars[i];

        if language.markdown
            && let Some(end) = self.markdown_inline(i)
        {
            return end;
        }

        if language.stylesheet
            && let Some(end) = self.style_token(i)
        {
            return end;
        }

        if c == '\\' && language.backslash_commands {
            let end = scan(chars, i + 1, char::is_alphabetic);
            if end > i + 1 {
                self.push(i, end, TokenKind::Keyword);
                return end;
            }
            return i + 2;
        }

        if c == language.escape_char && language.code_escapes {
            return i + 2;
        }

        if matches!(c, '$' | '@' | '%' | '!')
            && let Some((end, kind)) = self.variable(i, false)
        {
            self.push(i, end, kind);
            return end;
        }

        if c == '<'
            && language.heredocs != Heredocs::None
            && let Some((end, tag)) = self.heredoc_opening(i)
        {
            self.push(i, end, TokenKind::Keyword);
            self.heredoc.get_or_insert(LineState::Heredoc(tag));
            return end;
        }

        if c == '/'
            && language.regex_literals
            && let Some(end) = self.regex(i)
        {
            self.push(i, end, TokenKind::String);
            return end;
        }

        if c == ':'
            && language.colon_symbols
            && let Some(end) = self.symbol(i)
        {
            self.push(i, end, TokenKind::Variable);
            return end;
        }

        // 单引号不界定字符串的语言中，`'a'`、`'\n'` 是字符字面量，其余情况（例如
        // Rust 的生命周期）按普通文本处理。
        if c == '\'' && !language.single_quote_string && language.code_tokens {
            // 反斜杠之后的字符被转义，不能作为收尾（如 `'\''`），因此从 `i + 3` 起查找收尾的
            // 单引号，最远查到 `i + 12`（经验值，足以覆盖 `'\u{10FFFF}'` 这类较长的转义）。
            let end = if chars.get(i + 1) == Some(&'\\') {
                (i + 3..n.min(i + 12))
                    .find(|&j| chars[j] == '\'')
                    .map(|j| j + 1)
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 3)
            } else {
                None
            };

            if let Some(end) = end {
                self.push(i, end, TokenKind::String);
                return end;
            }

            return i + 1;
        }

        // 装饰器的 `@` 之后紧跟名字；两侧有空格的 `@` 是运算符（Python 的矩阵乘法 `a @ b`）。
        if c == '@' && language.decorators && chars.get(i + 1).is_some_and(|&c| is_ident_start(c)) {
            let end = scan(chars, i + 1, |c| is_ident(c) || c == '.');
            self.push(i, end, TokenKind::Decorator);
            return end;
        }

        if c.is_ascii_digit() && language.code_tokens && (i == 0 || !is_ident(chars[i - 1])) {
            return self.number(i);
        }

        if is_ident_start(c) {
            let end = scan(chars, i + 1, is_ident);

            if self.tokens.is_some()
                && let Some(kind) = self.classify(i, end)
            {
                self.push(i, end, kind);
            }

            return end;
        }

        i + 1
    }

    /// 识别 `i` 处开始的数字，返回其末尾之后的位置。
    fn number(&mut self, i: usize) -> usize {
        let chars = self.chars;
        // 整数部分连同进制前缀、类型后缀和指数（`0x1F`、`10u8`、`1e9`）。
        let digits = |from| scan(chars, from, |c| c.is_ascii_alphanumeric() || c == '_');
        // 小数点之后紧跟数字才是小数部分：`0..10`、`2.pow(3)` 中的点不属于数字。
        let dot_digit = |at: usize| {
            chars.get(at) == Some(&'.') && chars.get(at + 1).is_some_and(char::is_ascii_digit)
        };
        let mut end = digits(i + 1);
        if dot_digit(end) {
            end = digits(end + 1);
        }
        // 小数部分之后还有「点 + 数字」的是版本号或地址（`1.4.1`、`127.0.0.1`），`@` 之后的
        // 是包的版本（`react@19`），都不是数字，按普通文本显示，不会一段数字色一段正文色。
        let mut version = i > 0 && chars[i - 1] == '@';
        while dot_digit(end) {
            version = true;
            end = digits(end + 1);
        }
        if !version {
            self.push(i, end, TokenKind::Number);
        }
        end
    }

    /// 返回 `start..end` 处标识符的类别，都不是时返回 `None`。
    ///
    /// 类别依次为关键字、内置名、类型名，或后面紧跟 `(` 的函数名。
    fn classify(&self, start: usize, end: usize) -> Option<TokenKind> {
        let language = self.language;
        let mut buf = [0u8; 24];

        if let Some(word) = ascii_word(&self.chars[start..end], &mut buf, language.ignore_case) {
            if language.keywords.contains(&word) {
                return Some(TokenKind::Keyword);
            }

            if language.builtins.contains(&word) {
                return Some(TokenKind::Builtin);
            }
        }

        let name = &self.chars[start..end];
        if language.capitalized_types
            && name[0].is_uppercase()
            && name.iter().any(|c| c.is_lowercase())
        {
            return Some(TokenKind::Type);
        }

        // 命令替换 `$(…)` 中的 `$` 不是函数名。
        if !language.code_tokens || name == ['$'] {
            return None;
        }

        if language.macros
            && self.chars.get(end) == Some(&'!')
            && matches!(self.chars.get(end + 1), Some('(' | '[' | '{'))
        {
            return Some(TokenKind::Function);
        }

        let next = self.chars[end..].iter().find(|&&c| c != ' ' && c != '\t');
        (next == Some(&'(')).then_some(TokenKind::Function)
    }

    /// 返回 `i` 处 `:` 开始的符号（`:name`、`:name?`）的末尾之后的位置；不是符号时返回 `None`。
    ///
    /// `a::b` 中的 `::` 和 `a ? b : c` 中的 `:` 不是符号：`:` 之前不能是标识符字符或 `:`，之后
    /// 必须紧跟标识符。
    fn symbol(&self, i: usize) -> Option<usize> {
        let chars = self.chars;
        if i > 0 && (is_ident(chars[i - 1]) || chars[i - 1] == ':') {
            return None;
        }
        // 运算符方法的符号：`` :` ``、`:+`、`:<=>`、`:[]=`。
        let operator = scan(chars, i + 1, |c| "+-*/%<>=!~`[]^&|".contains(c));
        if operator > i + 1 && !matches!(chars.get(i + 1), Some('=' | '>' | '[')) {
            return Some(operator);
        }
        if !chars
            .get(i + 1)
            .is_some_and(|&c| c.is_alphabetic() || c == '_')
        {
            return None;
        }
        let end = scan(chars, i + 1, |c| is_name(c) || c == '-');
        Some(end + usize::from(matches!(chars.get(end), Some('?' | '!'))))
    }
}

/// 返回从 `chars[j]` 的转义字符起算的转义序列长度（字符）。
///
/// `\x41` 取 2 位十六进制数，`\u0041` 取 4 位，`\u{1F600}` 到 `}` 为止，`\U0001F600` 取 8 位，
/// `\101` 取至多 3 位八进制数，其余取转义字符和紧随的一个字符。
fn escape_len(chars: &[char], j: usize) -> usize {
    let hex =
        |from: usize, max: usize| scan(chars, from, |c| c.is_ascii_hexdigit()).min(from + max) - j;
    match chars.get(j + 1) {
        None => 1,
        Some('x') => hex(j + 2, 2),
        Some('u') if chars.get(j + 2) == Some(&'{') => chars[j + 2..]
            .iter()
            .position(|&c| c == '}')
            .map_or(2, |k| k + 3),
        Some('u') => hex(j + 2, 4),
        Some('U') => hex(j + 2, 8),
        Some(c) if c.is_digit(8) => scan(chars, j + 1, |c| c.is_digit(8)).min(j + 4) - j,
        Some(_) => 2,
    }
}

/// `chars` 从位置 `i` 起是否以 `pattern` 开头。
fn starts_with(chars: &[char], i: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(k, p)| chars.get(i + k) == Some(&p))
}

/// `chars` 从位置 `i` 起是否以 `pattern` 开头，不区分 ASCII 大小写。
fn starts_with_ignore_case(chars: &[char], i: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(k, p)| chars.get(i + k).is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

/// 从 `from` 开始跳过满足条件的字符，返回第一个不满足的位置。
fn scan(chars: &[char], from: usize, accept: impl Fn(char) -> bool) -> usize {
    from + chars[from.min(chars.len())..]
        .iter()
        .take_while(|&&c| accept(c))
        .count()
}

/// 把短的纯 ASCII 标识符复制到栈上缓冲区并返回其字符串视图，避免每个单词分配一次。
///
/// `lowercase` 为真时转成小写。关键字都是 ASCII，含其他字符的标识符不可能是关键字，返回
/// `None`。缓冲区长 24 字节，大于现有关键字表中最长的词（`reinterpret_cast`，16 字节），超过
/// 缓冲区的标识符同样返回 `None`。
fn ascii_word<'b>(chars: &[char], buf: &'b mut [u8; 24], lowercase: bool) -> Option<&'b str> {
    if chars.len() > buf.len() {
        return None;
    }

    for (b, &c) in buf.iter_mut().zip(chars) {
        if !c.is_ascii() {
            return None;
        }

        *b = if lowercase { c.to_ascii_lowercase() } else { c } as u8;
    }

    std::str::from_utf8(&buf[..chars.len()]).ok()
}

/// 返回定界符（C++ 原始字符串的定界符、here 文档的结束标记）的散列值（FNV-1a，折叠到 16 位）。
///
/// [`LineState`] 只能存放定长的数据，因此用散列值代替定界符本身。
fn delimiter_hash(word: &[char]) -> u16 {
    let mut hash: u32 = 0x811c_9dc5;
    for &c in word {
        hash ^= c as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (hash ^ (hash >> 16)) as u16
}

/// 字符是否可以出现在变量名中（字母、数字或 `_`）。
fn is_name(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 字符是否可以作为标识符的开头（字母、`_` 或 `$`）。
fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

/// 字符是否可以出现在标识符中（字母、数字、`_` 或 `$`）。
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

#[cfg(test)]
mod tests;
