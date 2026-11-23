//! 公式：把 `$…$`、`$$…$$`、```` ```math ```` 里写的 LaTeX 解析成一棵树，排版和绘制在
//! `winc-app` 的预览排版里。
//!
//! 只识别写公式常用的那部分命令：上下标、分式、根号、大运算符和上下限、`\left(` 与 `\right)`、
//! 字体、重音、矩阵与 `aligned` 这类环境、颜色。识别不了的命令原样显示为红色文字。

mod table;

use table::{Kind, lookup};

/// 公式结构树允许的最大嵌套层数（层）：每个 `{…}`、`\left…\right`、环境，以及作为参数的命令
/// （`\hat\hat x`）各算一层。
///
/// 公式树的解析、排版、绘制和释放都按层递归，嵌套数万层时栈溢出、进程中止。超过上限后，
/// 剩余的原文不再解析，原样显示为红色文字。取值与 Markdown 容器块的上限 `MAX_NESTING` 相同，
/// 远高于实际公式的嵌套深度。
const MAX_DEPTH: usize = 100;

/// 符号的类别：决定符号与左右相邻符号之间留多宽的间距（TeX 的规则，见排版部分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// 普通符号：字母、数字。
    Ord,
    /// 大运算符：`\sum`、`\lim`。
    Op,
    /// 二元运算：`+`、`\times`。
    Bin,
    /// 关系：`=`、`\le`、`\to`。
    Rel,
    /// 左括号。
    Open,
    /// 右括号。
    Close,
    /// 标点：`,`。
    Punct,
}

/// 字体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    /// 斜体：单个拉丁字母的默认样子。
    Italic,
    /// 正体：数字、运算符、函数名、`\mathrm`。
    Roman,
    /// 粗体：`\mathbf`。
    Bold,
    /// 粗斜体：`\boldsymbol` 里的字母。
    BoldItalic,
    /// 等宽：`\mathtt`。
    Mono,
    /// 正文的字体：`\text{}` 里的文字（中文常写在这里）。
    Text,
}

/// 重音和上下的线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Accent {
    /// `\hat`、`\widehat`。
    Hat,
    /// `\check`。
    Check,
    /// `\tilde`、`\widetilde`。
    Tilde,
    /// `\bar`、`\overline`（`wide` 时覆盖整个内容）。
    Bar,
    /// `\underline`。
    Underline,
    /// `\vec`、`\overrightarrow`。
    Vec,
    /// `\overleftarrow`。
    LeftArrow,
    /// `\dot`。
    Dot,
    /// `\ddot`。
    DDot,
    /// `\overbrace`。
    OverBrace,
    /// `\underbrace`。
    UnderBrace,
}

/// 矩阵这类环境的列对齐方式与两侧括号。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Grid {
    /// 各行各格。
    pub rows: Vec<Vec<Node>>,
    /// 各列的对齐：`l`、`c`、`r`；列数超过这个列表时按最后一项对齐，列表为空时居中。
    ///
    /// `aligned` 这类环境按「右左右左」交替。
    pub align: Vec<char>,
    /// 左边的括号（空字符串时不画）。
    pub left: String,
    /// 右边的括号（空字符串时不画）。
    pub right: String,
    /// 是否在各列之间留一个字宽的空白（矩阵、`cases` 为 `true`）。
    ///
    /// `aligned` 这类环境为 `false`，`=` 两边已有空白。
    pub gap: bool,
    /// 是否按正文的文字大小排版单元格（`cases`、`aligned`）；`false` 时与矩阵一样用正文大小。
    pub display: bool,
}

/// 公式树的一个节点。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Node {
    /// 一个符号（或者 `\sin` 这类一串字母）。
    Sym(String, Class, Font),
    /// 一串节点，`{}` 包着的一组。
    Row(Vec<Node>),
    /// 分式：分子、分母、是否有分数线（`\binom` 没有，两边加圆括号）、强制的大小
    /// （`\dfrac` 为 `Some(true)`，`\tfrac` 为 `Some(false)`）。
    Frac(Box<Node>, Box<Node>, bool, Option<bool>),
    /// 根号：开几次方、被开方的。
    Sqrt(Option<Box<Node>>, Box<Node>),
    /// 上下标：底、上标、下标。
    Scripts(Box<Node>, Option<Box<Node>>, Option<Box<Node>>),
    /// 大运算符：符号或名字、是否为放大的符号（`\sum` 为 `true`，`\lim` 为 `false`）、
    /// 上下限放在正上下方（`Some(true)`）还是右边（`Some(false)`）。
    ///
    /// `None` 是 `\sum`、`\lim` 的默认：单独成行的公式里放正上下方，行内的放右边。
    /// 积分、`\sin` 是 `Some(false)`。
    Op(String, bool, Option<bool>),
    /// `\left( … \right)`：左右括号随内容变高（`.` 为空）。
    Fenced(String, Box<Node>, String),
    /// `\big(` 这类：括号、放大级数（1 至 4）、类别。
    Big(String, u8, Class),
    /// 重音、上下的线和括号。
    Accent(Accent, Box<Node>),
    /// `\overset{上}{下}`、`\underset{下}{上}`：底、上面的、下面的。
    Stack(Box<Node>, Option<Box<Node>>, Option<Box<Node>>),
    /// 空白，单位为字号的倍数。
    Space(u16),
    /// 矩阵、`cases`、`aligned` 这类。
    Grid(Grid),
    /// `\displaystyle`（`true`）、`\textstyle`（`false`）之后的部分。
    Style(bool, Box<Node>),
    /// `\color{red}{…}`：颜色（名字或 `#rrggbb`）。
    Color(String, Box<Node>),
    /// 改了类别的（`\mathrel{…}`、`\mathbin{…}`）。
    Class(Class, Box<Node>),
    /// `\boxed{…}`。
    Boxed(Box<Node>),
    /// `\phantom{…}`：只占位置，不绘制。
    Phantom(Box<Node>),
    /// 认不出的命令、没有配对的括号：原样显示为红色文字。
    Error(String),
}

impl Node {
    /// 空的一组。
    fn empty() -> Node {
        Node::Row(Vec::new())
    }
}

/// 把一段 LaTeX 源码解析成一棵公式树。
///
/// 空白只决定命令在哪里结束，不显示（`\text` 里的除外）。不在环境里写的 `\\`、`&` 也分行、
/// 分列（按行排成一张表），这时整个公式是一个 [`Grid`]。
pub fn parse(tex: &str) -> Node {
    let mut parser = Parser {
        chars: tex.chars().collect(),
        at: 0,
        right: String::new(),
        depth: 0,
    };
    let rows = parser.rows();
    match <[Vec<Node>; 1]>::try_from(rows) {
        Ok([row]) if row.len() == 1 => match row.into_iter().next() {
            // 只有一个节点（比如整个公式是一个环境）：不用再包一层。
            Some(Node::Row(mut nodes)) if nodes.len() == 1 => {
                nodes.pop().unwrap_or_else(Node::empty)
            }
            node => node.unwrap_or_else(Node::empty),
        },
        Ok([row]) => grid(vec![row], vec!['r', 'l']),
        Err(rows) => {
            let columns = rows.iter().any(|r| r.len() > 1);
            grid(rows, if columns { vec!['r', 'l'] } else { vec!['c'] })
        }
    }
}

/// 不在环境里写的多行公式。
fn grid(rows: Vec<Vec<Node>>, align: Vec<char>) -> Node {
    Node::Grid(Grid {
        rows,
        align,
        left: String::new(),
        right: String::new(),
        gap: false,
        display: true,
    })
}

/// 一串节点解析到哪里为止。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// 到 `}`。
    Brace,
    /// 到 `]`（根号的次数）。
    Bracket,
    /// 到 `\right`。
    Right,
    /// 到 `\end`、`&`、`\\`（环境里的一格）。
    Cell,
}

/// 一格（[`Stop::Cell`]）因为什么结束。
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// 到了要停的地方（或者结尾）。
    Done,
    /// `&`：下一格。
    Column,
    /// `\\`：下一行。
    Line,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    /// 刚读到的 `\right` 后面的括号，`\left` 那边取走。
    right: String,
    /// 当前的嵌套层数（层，见 [`MAX_DEPTH`]）。
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    /// 一格一格、逐行读到 `\end` 或者结尾。
    ///
    /// 末尾 `\\` 留下的空行不保留。
    fn rows(&mut self) -> Vec<Vec<Node>> {
        let mut rows: Vec<Vec<Node>> = vec![Vec::new()];
        loop {
            let (cell, end) = self.row(Stop::Cell);
            if let Some(row) = rows.last_mut() {
                row.push(cell);
            }
            match end {
                End::Column => {}
                End::Line => rows.push(Vec::new()),
                End::Done => break,
            }
        }
        if rows.len() > 1
            && rows
                .last()
                .is_some_and(|r| r.len() == 1 && r[0] == Node::empty())
        {
            rows.pop();
        }
        rows
    }

    /// 读一个命令名（`\` 已经读过）：一串字母，或者一个别的字。
    fn command(&mut self) -> String {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.at += 1;
        }
        if self.at == start && self.at < self.chars.len() {
            self.at += 1;
        }
        let name: String = self.chars[start..self.at].iter().collect();
        // `\operatorname*` 这类：星号算在名字里。
        if self.peek() == Some('*') && name.chars().all(|c| c.is_ascii_alphabetic()) {
            self.at += 1;
            return name + "*";
        }
        name
    }

    /// 解析一串，到 `stop` 为止，并把结束的记号读掉。
    fn row(&mut self, stop: Stop) -> (Node, End) {
        if self.depth >= MAX_DEPTH {
            return (self.rest(), End::Done);
        }
        self.depth += 1;
        let row = self.nodes(stop);
        self.depth -= 1;
        row
    }

    /// 解析 [`Parser::row`] 的内容；调用方已经计入这一层嵌套。
    fn nodes(&mut self, stop: Stop) -> (Node, End) {
        let mut nodes: Vec<Node> = Vec::new();
        loop {
            self.skip_space();
            let Some(c) = self.peek() else {
                return (Node::Row(nodes), End::Done);
            };
            self.at += 1;
            match c {
                '}' if stop == Stop::Brace => break,
                ']' if stop == Stop::Bracket => break,
                '&' if stop == Stop::Cell => return (Node::Row(nodes), End::Column),
                '{' => {
                    let (group, _) = self.row(Stop::Brace);
                    nodes.push(group);
                }
                '^' | '_' => {
                    let arg = self.argument();
                    attach(&mut nodes, c == '^', arg);
                }
                '\'' => {
                    // 撇号 ′ 本身画在高处，不再当上标抬高；几个连着的合成一个，接在
                    // 前一个节点后面（`f'(x)`、`f''^2`）。
                    let mut primes = String::from('′');
                    while self.peek() == Some('\'') {
                        self.at += 1;
                        primes.push('′');
                    }
                    let base = nodes.pop().unwrap_or_else(Node::empty);
                    nodes.push(Node::Row(vec![
                        base,
                        Node::Sym(primes, Class::Ord, Font::Roman),
                    ]));
                }
                '\\' => {
                    let name = self.command();
                    match name.as_str() {
                        "\\" if stop == Stop::Cell => {
                            // `\\[2pt]` 这类行距：忽略。
                            if self.peek() == Some('[') {
                                self.skip_to(']');
                            }
                            return (Node::Row(nodes), End::Line);
                        }
                        "end" if stop == Stop::Cell => {
                            self.group_text();
                            return (Node::Row(nodes), End::Done);
                        }
                        "right" if stop == Stop::Right => {
                            self.right = self.delimiter();
                            return (Node::Row(nodes), End::Done);
                        }
                        "displaystyle" | "textstyle" | "scriptstyle" => {
                            let (rest, end) = self.row(stop);
                            nodes.push(Node::Style(name == "displaystyle", Box::new(rest)));
                            return (Node::Row(nodes), end);
                        }
                        "color" => {
                            let color = self.group_text();
                            // 两种写法：`\color{red}{x}`，或者 `\color{red}` 之后的全部内容。
                            self.skip_space();
                            if self.peek() == Some('{') {
                                let arg = self.argument();
                                nodes.push(Node::Color(color, Box::new(arg)));
                            } else {
                                let (rest, end) = self.row(stop);
                                nodes.push(Node::Color(color, Box::new(rest)));
                                return (Node::Row(nodes), end);
                            }
                        }
                        "limits" | "nolimits" => {
                            if let Some(Node::Op(_, _, limits)) = nodes.last_mut() {
                                *limits = Some(name == "limits");
                            }
                        }
                        _ => {
                            let node = self.command_node(&name);
                            nodes.push(node);
                        }
                    }
                }
                c => nodes.push(symbol(c)),
            }
        }
        (Node::Row(nodes), End::Done)
    }

    /// 嵌套超过 [`MAX_DEPTH`] 层时调用：剩余的原文不再解析，原样显示为红色文字。
    fn rest(&mut self) -> Node {
        let text = self.chars[self.at..].iter().collect();
        self.at = self.chars.len();
        Node::Error(text)
    }

    /// 跳到 `close` 之后。
    fn skip_to(&mut self, close: char) {
        while let Some(c) = self.peek() {
            self.at += 1;
            if c == close {
                break;
            }
        }
    }

    /// `{…}` 里的原文（不解析）；不是 `{` 开头时取一个字。
    fn group_text(&mut self) -> String {
        self.skip_space();
        if self.peek() != Some('{') {
            return self
                .peek()
                .map(|c| {
                    self.at += 1;
                    c.to_string()
                })
                .unwrap_or_default();
        }
        self.at += 1;
        let start = self.at;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '\\' => self.at += 1,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return self.chars[start..self.at - 1].iter().collect();
                    }
                }
                _ => {}
            }
        }
        self.chars[start..].iter().collect()
    }

    /// 一个参数：`{…}` 一组，或者一个符号、一个命令。
    fn argument(&mut self) -> Node {
        self.skip_space();
        let Some(c) = self.peek() else {
            return Node::empty();
        };
        // 命令的参数也是命令（`\hat\hat x`）时不经过 `row`，因此在这里计入一层。
        if c == '\\' && self.depth >= MAX_DEPTH {
            return self.rest();
        }
        self.at += 1;
        match c {
            '{' => self.row(Stop::Brace).0,
            '\\' => {
                let name = self.command();
                self.depth += 1;
                let node = self.command_node(&name);
                self.depth -= 1;
                node
            }
            c => symbol(c),
        }
    }

    /// 可选参数 `[…]`。
    fn optional(&mut self) -> Option<Node> {
        self.skip_space();
        (self.peek() == Some('[')).then(|| {
            self.at += 1;
            self.row(Stop::Bracket).0
        })
    }

    /// `\left`、`\big` 后面的括号。
    fn delimiter(&mut self) -> String {
        self.skip_space();
        let Some(c) = self.peek() else {
            return String::new();
        };
        self.at += 1;
        if c != '\\' {
            return if c == '.' {
                String::new()
            } else {
                c.to_string()
            };
        }
        let name = self.command();
        match name.as_str() {
            "{" | "lbrace" => "{".into(),
            "}" | "rbrace" => "}".into(),
            "|" | "Vert" => "‖".into(),
            "vert" => "|".into(),
            "langle" => "⟨".into(),
            "rangle" => "⟩".into(),
            "lfloor" => "⌊".into(),
            "rfloor" => "⌋".into(),
            "lceil" => "⌈".into(),
            "rceil" => "⌉".into(),
            "backslash" => "\\".into(),
            _ => lookup(&name).map(|(s, _)| s.to_owned()).unwrap_or_default(),
        }
    }

    /// 读完名字的命令 `name`，生成一个节点。
    fn command_node(&mut self, name: &str) -> Node {
        let boxed = |n: Node| Box::new(n);
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" => {
                let num = self.argument();
                let den = self.argument();
                let size = match name {
                    "dfrac" | "cfrac" | "dbinom" => Some(true),
                    "tfrac" | "tbinom" => Some(false),
                    _ => None,
                };
                Node::Frac(boxed(num), boxed(den), !name.ends_with("binom"), size)
            }
            "sqrt" => {
                let index = self.optional().map(Box::new);
                Node::Sqrt(index, boxed(self.argument()))
            }
            "left" => {
                let left = self.delimiter();
                let (body, _) = self.row(Stop::Right);
                Node::Fenced(left, boxed(body), std::mem::take(&mut self.right))
            }
            "right" => Node::Error("\\right".into()),
            "middle" => Node::Big(self.delimiter(), 2, Class::Rel),
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "Bigl" | "biggl" | "Biggl" | "bigr"
            | "Bigr" | "biggr" | "Biggr" | "bigm" | "Bigm" | "biggm" | "Biggm" => {
                let level = match name.trim_end_matches(['l', 'r', 'm']) {
                    "big" => 1,
                    "Big" => 2,
                    "bigg" => 3,
                    _ => 4,
                };
                let class = match name.chars().last() {
                    Some('l') => Class::Open,
                    Some('r') => Class::Close,
                    Some('m') => Class::Rel,
                    _ => Class::Ord,
                };
                Node::Big(self.delimiter(), level, class)
            }
            "text" | "textrm" | "textnormal" | "mbox" | "textup" | "hbox" => {
                Node::Sym(self.group_text().replace('~', " "), Class::Ord, Font::Text)
            }
            "textbf" => Node::Sym(self.group_text(), Class::Ord, Font::Bold),
            "textit" | "emph" => Node::Sym(self.group_text(), Class::Ord, Font::Italic),
            "texttt" => Node::Sym(self.group_text(), Class::Ord, Font::Mono),
            "operatorname" | "operatorname*" => {
                let text = self.group_text();
                // 不带星号的上下标写在右边，带星号的和 `\lim` 一样。
                Node::Op(text, false, (!name.ends_with('*')).then_some(false))
            }
            "mathop" => {
                let arg = self.argument();
                Node::Class(Class::Op, boxed(arg))
            }
            "mathrel" | "mathbin" | "mathord" | "mathopen" | "mathclose" | "mathpunct" => {
                let class = match name {
                    "mathrel" => Class::Rel,
                    "mathbin" => Class::Bin,
                    "mathopen" => Class::Open,
                    "mathclose" => Class::Close,
                    "mathpunct" => Class::Punct,
                    _ => Class::Ord,
                };
                Node::Class(class, boxed(self.argument()))
            }
            "mathrm" | "mathup" | "rm" => restyle(self.argument(), Font::Roman),
            "mathit" | "it" => restyle(self.argument(), Font::Italic),
            "mathbf" | "bf" | "textbf*" => restyle(self.argument(), Font::Bold),
            "boldsymbol" | "bm" | "pmb" | "mathbfit" => restyle(self.argument(), Font::BoldItalic),
            "mathtt" => restyle(self.argument(), Font::Mono),
            "mathsf" => restyle(self.argument(), Font::Text),
            "mathbb" | "Bbb" => alphabet(self.argument(), double_struck),
            "mathcal" | "mathscr" => alphabet(self.argument(), script),
            "mathfrak" => alphabet(self.argument(), fraktur),
            "hat" | "widehat" => Node::Accent(Accent::Hat, boxed(self.argument())),
            "check" | "widecheck" => Node::Accent(Accent::Check, boxed(self.argument())),
            "tilde" | "widetilde" => Node::Accent(Accent::Tilde, boxed(self.argument())),
            "bar" | "overline" => Node::Accent(Accent::Bar, boxed(self.argument())),
            "underline" => Node::Accent(Accent::Underline, boxed(self.argument())),
            "vec" | "overrightarrow" => Node::Accent(Accent::Vec, boxed(self.argument())),
            "overleftarrow" => Node::Accent(Accent::LeftArrow, boxed(self.argument())),
            "dot" => Node::Accent(Accent::Dot, boxed(self.argument())),
            "ddot" => Node::Accent(Accent::DDot, boxed(self.argument())),
            "overbrace" => Node::Accent(Accent::OverBrace, boxed(self.argument())),
            "underbrace" => Node::Accent(Accent::UnderBrace, boxed(self.argument())),
            "overset" | "stackrel" => {
                let over = self.argument();
                let base = self.argument();
                Node::Class(
                    Class::Rel,
                    boxed(Node::Stack(boxed(base), Some(boxed(over)), None)),
                )
            }
            "underset" => {
                let under = self.argument();
                let base = self.argument();
                Node::Stack(boxed(base), None, Some(boxed(under)))
            }
            "boxed" | "fbox" => Node::Boxed(boxed(self.argument())),
            "phantom" | "hphantom" => Node::Phantom(boxed(self.argument())),
            "vphantom" | "label" | "nonumber" | "notag" => {
                if name == "vphantom" || name == "label" {
                    self.argument();
                }
                Node::empty()
            }
            "cancel" | "bcancel" | "xcancel" | "smash" | "mathstrut" | "underleftarrow" => {
                if name == "mathstrut" {
                    Node::empty()
                } else {
                    self.argument()
                }
            }
            "textcolor" => {
                let color = self.group_text();
                Node::Color(color, boxed(self.argument()))
            }
            "tag" | "tag*" => {
                let text = self.group_text();
                let shown = if name == "tag" {
                    format!("({text})")
                } else {
                    text
                };
                Node::Row(vec![
                    Node::Space(36),
                    Node::Sym(shown, Class::Ord, Font::Text),
                ])
            }
            "hspace" | "hspace*" | "kern" | "mkern" | "mskip" | "hskip" => {
                self.group_text();
                Node::Space(6)
            }
            "not" => {
                let next = self.argument();
                match next {
                    Node::Sym(s, class, font) => Node::Sym(negate(&s), class, font),
                    other => Node::Row(vec![
                        other,
                        Node::Sym("\u{338}".into(), Class::Rel, Font::Roman),
                    ]),
                }
            }
            "begin" => self.environment(),
            "end" => {
                self.group_text();
                Node::empty()
            }
            "," | "thinspace" => Node::Space(3),
            ":" | ">" | "medspace" => Node::Space(4),
            ";" | "thickspace" => Node::Space(5),
            "!" | "negthinspace" => Node::Space(0),
            " " | "space" | "nobreakspace" => Node::Space(4),
            "quad" => Node::Space(18),
            "qquad" => Node::Space(36),
            "enspace" => Node::Space(9),
            _ => match lookup(name) {
                Some((text, kind)) => match kind {
                    Kind::Sym(class, font) => Node::Sym(text.to_owned(), class, font),
                    Kind::BigOp(limits) => {
                        Node::Op(text.to_owned(), true, (!limits).then_some(false))
                    }
                    Kind::Func(limits) => {
                        Node::Op(text.to_owned(), false, (!limits).then_some(false))
                    }
                },
                None => Node::Error(format!("\\{name}")),
            },
        }
    }

    /// `\begin{…}` 之后：环境的名字、内容，直到 `\end{…}` 为止。
    fn environment(&mut self) -> Node {
        let name = self.group_text();
        let name = name.trim_end_matches('*');
        // `array` 写了列格式 `{lcr}`；`alignat` 写了列数。
        let mut align: Vec<char> = Vec::new();
        if name == "array" || name == "alignat" || name == "subarray" {
            let spec = self.group_text();
            align = spec
                .chars()
                .filter(|c| matches!(c, 'l' | 'c' | 'r'))
                .collect();
        }
        let rows = self.rows();
        let (left, right) = match name {
            "pmatrix" => ("(", ")"),
            "bmatrix" => ("[", "]"),
            "Bmatrix" => ("{", "}"),
            "vmatrix" => ("|", "|"),
            "Vmatrix" => ("‖", "‖"),
            "cases" | "dcases" => ("{", ""),
            "rcases" => ("", "}"),
            _ => ("", ""),
        };
        let aligned = matches!(
            name,
            "aligned" | "align" | "alignat" | "alignedat" | "split" | "eqnarray" | "flalign"
        );
        if align.is_empty() {
            align = match name {
                "cases" | "dcases" | "rcases" => vec!['l', 'l'],
                _ if aligned => vec!['r', 'l'],
                _ => vec!['c'],
            };
        }
        Node::Grid(Grid {
            rows,
            align,
            left: left.into(),
            right: right.into(),
            gap: !aligned,
            display: aligned || matches!(name, "gather" | "gathered" | "equation" | "dcases"),
        })
    }
}

/// 把 `arg` 接成前一个节点的上标（`sup`）或下标。
///
/// 前面没有东西时接在一个空的底上。
fn attach(nodes: &mut Vec<Node>, sup: bool, arg: Node) {
    let base = nodes.pop().unwrap_or_else(Node::empty);
    let node = match base {
        // 已经有上下标的：补上另一个（`x_1^2`）；同一边写两次的（`x''^2`）合在一起。
        Node::Scripts(base, up, down) => {
            let (up, down) = if sup {
                (Some(merge(up, arg)), down)
            } else {
                (up, Some(merge(down, arg)))
            };
            Node::Scripts(base, up, down)
        }
        base => {
            let arg = Some(Box::new(arg));
            if sup {
                Node::Scripts(Box::new(base), arg, None)
            } else {
                Node::Scripts(Box::new(base), None, arg)
            }
        }
    };
    nodes.push(node);
}

fn merge(old: Option<Box<Node>>, new: Node) -> Box<Node> {
    Box::new(match old {
        Some(old) => Node::Row(vec![*old, new]),
        None => new,
    })
}

/// 把一个字转成符号节点。
fn symbol(c: char) -> Node {
    let (text, class, font) = match c {
        'a'..='z' | 'A'..='Z' => (c.to_string(), Class::Ord, Font::Italic),
        '+' => ("+".into(), Class::Bin, Font::Roman),
        '-' => ("−".into(), Class::Bin, Font::Roman),
        '*' => ("∗".into(), Class::Bin, Font::Roman),
        '=' | '<' | '>' | ':' => (c.to_string(), Class::Rel, Font::Roman),
        '(' | '[' => (c.to_string(), Class::Open, Font::Roman),
        ')' | ']' => (c.to_string(), Class::Close, Font::Roman),
        ',' | ';' => (c.to_string(), Class::Punct, Font::Roman),
        '~' => (" ".into(), Class::Ord, Font::Roman),
        _ => (c.to_string(), Class::Ord, Font::Roman),
    };
    Node::Sym(text, class, font)
}

/// 把 `node` 里的字母换成字体 `font`（`\mathbf{x}`、`\mathrm{d}`）。
fn restyle(node: Node, font: Font) -> Node {
    match node {
        Node::Sym(text, class, old) => {
            // 粗体只管字母、数字：`\mathbf{+}` 还是普通的加号。
            let keep = old == Font::Roman && matches!(class, Class::Bin | Class::Rel);
            Node::Sym(text, class, if keep { old } else { font })
        }
        Node::Row(nodes) => Node::Row(nodes.into_iter().map(|n| restyle(n, font)).collect()),
        Node::Scripts(base, up, down) => Node::Scripts(Box::new(restyle(*base, font)), up, down),
        other => other,
    }
}

/// `\mathbb` 这类：字母换成 Unicode 数学字母区里对应的字（ℝ、𝒜、𝔤）。
fn alphabet(node: Node, table: fn(char) -> Option<char>) -> Node {
    match node {
        Node::Sym(text, class, font) => {
            let mapped: String = text.chars().map(|c| table(c).unwrap_or(c)).collect();
            let font = if mapped == text { font } else { Font::Roman };
            Node::Sym(mapped, class, font)
        }
        Node::Row(nodes) => Node::Row(nodes.into_iter().map(|n| alphabet(n, table)).collect()),
        other => other,
    }
}

/// 双线体（`\mathbb`）。
///
/// 双线体、花体、哥特体有几个常用字母位于 Unicode 的「字母式符号」区，码位与数学字母区不连续。
fn double_struck(c: char) -> Option<char> {
    let special = match c {
        'C' => 'ℂ',
        'H' => 'ℍ',
        'N' => 'ℕ',
        'P' => 'ℙ',
        'Q' => 'ℚ',
        'R' => 'ℝ',
        'Z' => 'ℤ',
        _ => '\0',
    };
    if special != '\0' {
        return Some(special);
    }
    math_letter(c, 0x1D538, 0x1D552, Some(0x1D7D8))
}

/// 花体（`\mathcal`、`\mathscr`）。
fn script(c: char) -> Option<char> {
    let special = match c {
        'B' => 'ℬ',
        'E' => 'ℰ',
        'F' => 'ℱ',
        'H' => 'ℋ',
        'I' => 'ℐ',
        'L' => 'ℒ',
        'M' => 'ℳ',
        'R' => 'ℛ',
        'e' => 'ℯ',
        'g' => 'ℊ',
        'o' => 'ℴ',
        _ => '\0',
    };
    if special != '\0' {
        return Some(special);
    }
    math_letter(c, 0x1D49C, 0x1D4B6, None)
}

/// 哥特体（`\mathfrak`）。
fn fraktur(c: char) -> Option<char> {
    let special = match c {
        'C' => 'ℭ',
        'H' => 'ℌ',
        'I' => 'ℑ',
        'R' => 'ℜ',
        'Z' => 'ℨ',
        _ => '\0',
    };
    if special != '\0' {
        return Some(special);
    }
    math_letter(c, 0x1D504, 0x1D51E, None)
}

/// 数学字母区里的字：大写从 `upper` 起，小写从 `lower` 起，数字从 `digit` 起。
fn math_letter(c: char, upper: u32, lower: u32, digit: Option<u32>) -> Option<char> {
    let code = match c {
        'A'..='Z' => upper + (c as u32 - 'A' as u32),
        'a'..='z' => lower + (c as u32 - 'a' as u32),
        '0'..='9' => digit? + (c as u32 - '0' as u32),
        _ => return None,
    };
    char::from_u32(code)
}

/// `\not` 后面的符号：有现成的否定符号时用它，否则叠一道斜线。
fn negate(s: &str) -> String {
    let negated = match s {
        "=" => "≠",
        "<" => "≮",
        ">" => "≯",
        "≤" => "≰",
        "≥" => "≱",
        "∈" => "∉",
        "∋" => "∌",
        "⊂" => "⊄",
        "⊃" => "⊅",
        "⊆" => "⊈",
        "⊇" => "⊉",
        "≡" => "≢",
        "∼" => "≁",
        "≈" => "≉",
        "≅" => "≇",
        "∣" | "|" => "∤",
        "∥" => "∦",
        _ => return format!("{s}\u{338}"),
    };
    negated.to_owned()
}

#[cfg(test)]
mod tests;
