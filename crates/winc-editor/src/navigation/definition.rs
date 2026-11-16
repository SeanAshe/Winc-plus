//! 转到定义：不接入语言服务器、不建索引，按常见的定义写法（`fn name`、`def name`、`class Name`、
//! 行首的 `name = …`）在当前文件和工作区中实时查找，列出找到的全部位置。
//!
//! 每处定义带有类别（[`Kind`]）。带缩进的绑定、赋值和形参是局部变量，用法只在当前文件中查找；
//! 赋值之前已有定义时是重新赋值，不算定义。C 系语言类型在前的声明另有写法。

use std::path::Path;

use regex::Regex;
use ropey::Rope;
use winc_syntax::Language;

use super::usages::token_kinds;
use crate::workspace::project_search::Query;

/// 定义的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 函数、类型、模块、常量等声明（`fn`、`class`、`const`、方法和字段）。
    Declaration,
    /// 用关键字引入的变量（`let`、`var`、`val`、`local`、`my`）。
    Binding,
    /// 没有关键字的赋值（`x = 1`、Go 的 `x := 1`）。
    Assignment,
    /// 函数的形参，只在所在的函数中可见。
    Parameter,
}

/// 一行中的一处定义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Definition {
    /// 名字在该行的列（字符偏移，从 0 起）。
    pub col: usize,
    /// 定义的类别。
    pub kind: Kind,
    /// 该行是否以空白开头（定义位于函数、类或块的内部）。
    pub indented: bool,
}

impl Definition {
    /// 是否是局部变量：带缩进的绑定或赋值，以及形参。
    ///
    /// 局部变量的用法只在当前文件中查找；其他文件中同名的局部变量与之无关。
    pub fn is_local(&self) -> bool {
        self.kind == Kind::Parameter || (self.indented && self.kind != Kind::Declaration)
    }
}

/// 名字之前的单词属于此表时，`word name(…) {` 不是带返回类型的方法定义，而是语句。
///
/// 例如 `return f(x) {`、`new Foo(a) {`。
const NOT_RETURN_TYPES: &[&str] = &[
    "return",
    "new",
    "else",
    "throw",
    "await",
    "yield",
    "case",
    "go",
    "defer",
    "puts",
    "print",
    "echo",
    "not",
    "and",
    "or",
    "in",
    "of",
    "typeof",
    "delete",
    "raise",
    "do",
    "then",
    "when",
    "if",
    "while",
    "for",
    "switch",
    "catch",
    "using",
    "lock",
    "sizeof",
    "default",
    "export",
    "try",
    "co_await",
    "co_return",
    "co_yield",
];

/// 出现在名字之前时表明该行是语句而非声明的单词（不区分大小写），如 `return x;`、SQL 的
/// `FROM users;`。
const NOT_TYPES: &[&str] = &[
    "return", "goto", "break", "continue", "throw", "delete", "new", "else", "case", "default",
    "import", "export", "package", "use", "using", "await", "yield", "typeof", "sizeof", "echo",
    "print", "puts", "from", "into", "table", "join", "as", "in", "of", "and", "or", "not", "do",
    "then", "set", "where", "on", "when", "update", "select", "by",
];

/// 一种定义写法：类别和匹配后的检查。
struct Form {
    kind: Kind,
    check: Check,
}

/// 定义写法匹配后的检查。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    /// 不检查。
    None,
    /// 名字之前的单词不是语句关键字（[`NOT_RETURN_TYPES`]）。
    Statement,
    /// 名字之前是类型：不是语句关键字（[`NOT_TYPES`]），也不是乘法 `a * b` 或按位与 `a & b`。
    CDeclaration,
    /// 多行枚举的成员必须位于 `enum` 块内（[`in_enum`]），以区别逐行书写的实参；只看一行时不检查。
    Enumerator,
}

/// 返回各种定义写法的正则（每条恰好含一个名为 `k<下标>` 的捕获组，括住名字）和对应的写法。
///
/// `local` 为真时额外识别带缩进的赋值（Python 函数内的 `total = 1`）；跨文件查找不使用这一条，
/// 因为其他函数中同名的局部变量会产生大量无关结果。
fn forms(name: &str, local: bool) -> Vec<(String, Form)> {
    let escaped = regex::escape(name);
    // 名字之后不能紧跟名字字符；以标点结尾的名字（`valid?`）之后不能紧跟单词字符。写成 `[^\w]`
    // 而不是等价的大写形式：后者也能匹配换行符，搜索时会被当作跨行查找。
    let after = if name
        .chars()
        .last()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        r"(?:[^\w$'?!-]|$)"
    } else {
        r"(?:[^\w]|$)"
    };
    // 名字之前的限定部分：`function M.helper`、`def self.run`、`defmodule App.Repo`。
    let qualifier = r"(?:[\w$]+(?:\.|::|:))*";
    // Perl 的 `my $x`、`sub &x` 中名字之前的符号。赋值的写法不带这一项：PHP 的名字本身带 `$`，
    // 函数名 `total` 不能匹配变量 `$total = 0`。
    let sigil = r"[$@%&]?";
    let declaration = |pattern: String| {
        (
            pattern,
            Form {
                kind: Kind::Declaration,
                check: Check::None,
            },
        )
    };

    let mut all = vec![
        // 关键字后跟名字：函数、类型、模块和常量的声明。
        declaration(format!(
            r"\b(?:fn|trait|type|mod|def|defp|defmacro|defmacrop|defguard|defprotocol|defimpl|defdelegate|defmodule|defn-|defn|defmulti|defonce|defrecord|deftype|defstruct|class|interface|func|function|fun|sub|object|protocol|extension|module|namespace|record|macro_rules!|macro|data|newtype)\s+(?:(?:mut|rec|mutable|inline|private|abstract|static|async)\s+|\*\s*)*{qualifier}{sigil}(?P<NAME>{escaped}){after}"
        )),
        // C 系语言中这些关键字之后常是类型名（`static uart_ctx_t s;`）：名字之后紧跟名字、`*` 或
        // `&` 时是类型的使用。
        declaration(format!(
            r"\b(?:struct|enum|union|const|static|typedef)\s+(?:(?:mut|inline|static|const)\s+)*{qualifier}{sigil}(?P<NAME>{escaped})\s*(?:[^\w\s$*&'?!-]|$)"
        )),
        // Go 的方法：`func (r *T) name(`。
        declaration(format!(r"\bfunc\s*\([^)]*\)\s*(?P<NAME>{escaped}){after}")),
        // C 的 `#define` 宏。
        declaration(format!(r"#\s*define\s+(?P<NAME>{escaped}){after}")),
        // shell 函数 `name() {`（`function name` 写法由第一条规则覆盖）。
        declaration(format!(r"^(?P<NAME>{escaped})\s*\(\)\s*\{{")),
        // JS 中赋值为函数的写法：`name: function`、`name = (a) =>`、`exports.name = async x =>`。
        declaration(format!(
            r"(?:^|[^\w$])(?P<NAME>{escaped})\s*[:=]\s*(?:async\s*)?(?:function\b|\([^)]*\)\s*=>|[\w$]+\s*=>)"
        )),
        // 类中的方法 `name(a, b) {`，名字前可以有修饰词。
        declaration(format!(
            r"^\s*(?:(?:public|private|protected|static|async|override|get|set)\s+)*(?P<NAME>{escaped})\s*\([^)]*\)\s*\{{"
        )),
        // 至少带一个修饰词的方法（Java、C#、C++）：`public static int name(`。
        declaration(format!(
            r"^\s*(?:(?:public|private|protected|static|final|abstract|synchronized|override|virtual|inline|internal|open|async|extern|unsafe|sealed|partial)\s+)+[\w<>\[\],.?*&:\s]*\b(?P<NAME>{escaped})\s*\("
        )),
        // 带修饰词的字段和属性：`private final Map<K, V> cache = …`、
        // `public string Name { get; }`、TypeScript 的 `readonly size?: number`、Dart 的
        // `final count = 0`。
        declaration(format!(
            r"^\s*(?:(?:public|private|protected|internal|static|final|readonly|const|volatile|transient|late|required|abstract|override|declare)\s+)+(?:[\w<>\[\],.?]+\s+)*?{sigil}(?P<NAME>{escaped})\??\s*(?:[:=;{{]|$)"
        )),
        // 返回类型是基本类型的方法，左花括号可以在下一行（C#、Java、C++）：`void Update()`。
        declaration(format!(
            r"^\s*(?:void|int|bool|boolean|string|float|double|long|char|short|byte|auto|unsigned)\s+[*&]*(?P<NAME>{escaped})\s*\("
        )),
        // C 和 C++ 的函数定义：行首（无缩进）是返回类型，其后是名字（可以带 `Class::`）和左括号，
        // 该行不含分号。JS 顶层的 `await helper()` 形式相同，按名字之前的单词排除。
        (
            format!(r"^[A-Za-z_][\w:<>,*&\s]*(?:[\s*&]|::)(?P<NAME>{escaped})\s*\([^;]*$"),
            Form {
                kind: Kind::Declaration,
                check: Check::Statement,
            },
        ),
        // Ruby 的属性：`attr_reader :name, :size`。
        declaration(format!(
            r"\battr_(?:reader|writer|accessor)\b[^#]*:(?P<NAME>{escaped}){after}"
        )),
        // C 系语言类型在前的声明。
    ];
    all.extend(c_forms(&escaped, local));
    all.extend([
        // Haskell 的类型签名 `name :: Int -> Int`。
        declaration(format!(r"^(?P<NAME>{escaped})\s+::\s")),
        // Haskell 带参数的函数定义 `name x y = …`。
        declaration(format!(
            r"^(?P<NAME>{escaped})(?:\s+[a-z_][\w']*)+\s*=(?:[^=]|$)"
        )),
        // CMake 的 `function(name …)`、`macro(name …)`，命令名不区分大小写。
        declaration(format!(
            r"\b(?i:function|macro)\s*\(\s*(?P<NAME>{escaped}){after}"
        )),
        // Makefile 的目标 `name: deps`、`name::`；`:=` 是赋值，不算。
        declaration(format!(r"^(?P<NAME>{escaped})\s*::?(?:\s|$)")),
        // SQL 的 `CREATE TABLE name`、`CREATE OR REPLACE FUNCTION schema.name`，不区分大小写。
        declaration(format!(
            r#"\b(?i:create)\s+(?:(?i:or\s+replace)\s+)?(?:(?i:temp|temporary|unique|materialized)\s+)?(?i:table|view|function|procedure|index|trigger|type|schema|database|sequence)\s+(?:(?i:if\s+not\s+exists)\s+)?(?:[\w"`]+\.)*["`]?(?P<NAME>{escaped}){after}"#
        )),
        // 用关键字引入的变量：`let mut x`、`var x`、Lua 的 `local x`、Perl 的 `my $x`。
        (
            format!(
                r"\b(?:let|var|val|local|my|our|state)\s+(?:(?:mut|rec|mutable|inline|function)\s+)*{sigil}(?P<NAME>{escaped}){after}"
            ),
            Form {
                kind: Kind::Binding,
                check: Check::None,
            },
        ),
        // 带返回类型、没有修饰词的方法：`int compute(int x) {`、Dart 的 `void build(c) {`。
        (
            format!(
                r"^\s*[\w<>\[\],.?*&:]+\s+[*&]*(?P<NAME>{escaped})\s*\([^;]*\)\s*(?:\{{|=>|throws\b|const\b|noexcept\b|override\b|->)"
            ),
            Form {
                kind: Kind::Declaration,
                check: Check::Statement,
            },
        ),
        // 类型内没有方法体的方法声明（接口、抽象类、C++ 类）：`Token verify(String token);`。
        // 形参为空，或第一个形参是「类型 名字」，以区别带实参的对象构造 `Guard guard(mutex);`。
        (
            format!(
                r"^\s+{ty}\s+[*&]*(?P<NAME>{escaped})\s*\(\s*(?:\)|(?:(?:final|const|@\w+(?:\([^()]*\))?)\s+)*{ty}(?:\.\.\.)?\s+[*&]*\w+\s*[,)])[^;{{}}]*;\s*$",
                ty = r"[A-Za-z_][\w.:]*(?:<[^;(){}=]*>)?(?:\[\])*[?*&]*",
            ),
            Form {
                kind: Kind::Declaration,
                check: Check::Statement,
            },
        ),
        // 行首（无缩进）的赋值：Python 模块的常量、Makefile 和 shell 的变量、Go 的 `x :=`；
        // `==` 比较不算。
        (
            format!(r"^(?P<NAME>{escaped})\s*(?::[^=]*)?=(?:[^=]|$)"),
            Form {
                kind: Kind::Assignment,
                check: Check::None,
            },
        ),
    ]);
    if local {
        all.push((
            format!(r"^\s+(?P<NAME>{escaped})\s*(?::[^=]*)?=(?:[^=]|$)"),
            Form {
                kind: Kind::Assignment,
                check: Check::None,
            },
        ));
    }
    all.push(enumerator_form(&escaped));
    all
}

/// 返回 C 系语言类型在前的声明写法，`name` 是转义过的名字。
///
/// 类型是一个或多个单词，可带 `*`、`&`；名字之后可带数组维度。`local` 为真时另外识别带缩进的
/// 初始化、`for` 循环变量和形参，它们只在所在的函数中可见。
fn c_forms(name: &str, local: bool) -> Vec<(String, Form)> {
    let ty = r"(?:[A-Za-z_]\w*[\s*&]+)+";
    let dims = r"(?:\s*\[[^\]]*\])*";
    let form = |pattern: String, kind: Kind, check: Check| (pattern, Form { kind, check });
    let mut all = vec![
        // 全局变量、结构体成员、`extern` 声明和位域：`uint16_t len;`、`unsigned flag : 1;`。
        form(
            format!(r"^\s*{ty}(?P<NAME>{name}){dims}\s*(?:[;,]|:\s*\d)"),
            Kind::Declaration,
            Check::CDeclaration,
        ),
        // 行首带初始值的全局变量：`static const char *names[] = {`。
        form(
            format!(r"^{ty}(?P<NAME>{name}){dims}\s*=(?:[^=]|$)"),
            Kind::Binding,
            Check::CDeclaration,
        ),
        // 右花括号之后的名字：`} app_state_t;`、`} a_t, *a_p;`。
        form(
            format!(r"^\s*\}}\s*(?:[A-Za-z_]\w*\s*,\s*)*[*\s]*(?P<NAME>{name}){dims}\s*[,;=]"),
            Kind::Declaration,
            Check::None,
        ),
        // `typedef` 的别名：`typedef unsigned char u8;`、`typedef struct node node_t;`。
        form(
            format!(r"\btypedef\b[^;]*[\s*](?P<NAME>{name}){dims}\s*;"),
            Kind::Declaration,
            Check::None,
        ),
        // 函数指针：`typedef void (*cb_t)(void);`、`void (*on_rx)(uint8_t);`。
        form(
            format!(r"\w[\w\s*]*\(\s*\*\s*(?P<NAME>{name}){dims}\s*\)\s*\("),
            Kind::Declaration,
            Check::Statement,
        ),
        // 一行写完的枚举：`enum { A, B }`、`typedef enum { IDLE = 0, RUN } state_t;`。
        form(
            format!(r"\benum\b[^{{;]*\{{(?:[^}}]*,)?\s*(?P<NAME>{name})\s*(?:=[^,}}]*)?[,}}]"),
            Kind::Declaration,
            Check::None,
        ),
    ];
    if local {
        all.extend([
            // 带初始值的局部变量：`uart_ctx_t cfg = {0};`。
            form(
                format!(r"^\s+{ty}(?P<NAME>{name}){dims}\s*=(?:[^=]|$)"),
                Kind::Binding,
                Check::CDeclaration,
            ),
            // `for` 循环变量：`for (int k = 0; …)`、`for (auto &item : items)`。
            form(
                format!(r"\bfor\s*\(\s*{ty}(?P<NAME>{name})\s*[=:]"),
                Kind::Binding,
                Check::CDeclaration,
            ),
            // 函数的形参：`(const uint8_t *data, uint16_t size)`。
            form(
                format!(r"[(,]\s*{ty}(?P<NAME>{name}){dims}\s*[,)]"),
                Kind::Parameter,
                Check::CDeclaration,
            ),
        ]);
    }
    all
}

/// 返回多行枚举成员的写法：一行只有名字，可带 `= 值`、逗号和行末注释。
///
/// 该写法与赋值 `x = 1` 重叠，排在最后：同一行符合多种写法时取前面的。
fn enumerator_form(name: &str) -> (String, Form) {
    (
        format!(r"^\s*(?P<NAME>{name})\s*(?:=[^=,;{{}}][^,;{{}}]*?)?\s*,?\s*(?://.*|/\*.*)?$"),
        Form {
            kind: Kind::Declaration,
            check: Check::Enumerator,
        },
    )
}

/// 返回在项目中查找 `name` 的定义所用的搜索条件，每次匹配一行。
///
/// `ignore_case` 为真时不区分大小写（SQL、PHP 这类语言中的名字）。各语言的定义写法合并为一个
/// 正则，不按语言过滤，由调用方优先列出相关语言中的结果（见 [`rank`]）。每种写法都含名字本身，
/// 搜索因此只需逐行匹配含名字的行。
pub fn query(name: &str, ignore_case: bool) -> Query {
    Query {
        text: combine(&forms(name, false), ignore_case),
        case_sensitive: !ignore_case,
        regex: true,
        required: name.to_owned(),
        ..Query::default()
    }
}

/// 把各写法的正则合并为一个，把第 `i` 条中的捕获组命名为 `k<i>`。
fn combine(forms: &[(String, Form)], ignore_case: bool) -> String {
    let body = forms
        .iter()
        .enumerate()
        .map(|(i, (pattern, _))| pattern.replace("(?P<NAME>", &format!("(?P<k{i}>")))
        .collect::<Vec<_>>()
        .join("|");
    if ignore_case {
        format!("(?i){body}")
    } else {
        body
    }
}

/// 某个名字的定义查找器。
pub struct Finder {
    regex: Regex,
    name: String,
    ignore_case: bool,
    /// 各写法的捕获组名（`k<下标>`）和写法，与正则中的顺序一致。
    forms: Vec<(String, Form)>,
}

impl Finder {
    /// 创建查找定义名为 `name` 的查找器；`name` 为空、含空白或正则编译失败时返回 `None`。
    ///
    /// `local` 为 `true` 表示只在当前文件内查找，此时额外识别带缩进的赋值；`ignore_case` 为
    /// `true` 时名字不区分大小写。
    pub fn new(name: &str, local: bool, ignore_case: bool) -> Option<Finder> {
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return None;
        }
        let forms = forms(name, local);
        let regex = Regex::new(&combine(&forms, ignore_case)).ok()?;
        let forms = forms
            .into_iter()
            .enumerate()
            .map(|(i, (_, form))| (format!("k{i}"), form))
            .collect();
        Some(Finder {
            regex,
            name: name.to_owned(),
            ignore_case,
            forms,
        })
    }

    /// 返回正在查找的定义名。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 返回 `line` 中定义的名字所在的列（字符偏移，从 0 起）；`line` 不含定义时返回 `None`。
    pub fn in_line(&self, line: &str) -> Option<usize> {
        self.definition(line).map(|d| d.col)
    }

    /// 返回 `line` 中的定义；`line` 不含定义时返回 `None`。
    ///
    /// 只看这一行，不检查多行枚举成员的上下文，见 [`Self::definition_at`]。
    pub fn definition(&self, line: &str) -> Option<Definition> {
        self.matched(line).map(|(definition, _)| definition)
    }

    /// 返回 `text` 第 `index` 行（内容为 `line`）中的定义。
    ///
    /// 多行枚举的成员必须位于 `enum` 块内；块内的 `NAME = 值,` 算声明而非赋值。
    pub fn definition_at(&self, text: &Rope, index: usize, line: &str) -> Option<Definition> {
        let (mut definition, check) = self.matched(line)?;
        match (check, definition.kind) {
            (Check::Enumerator, _) => in_enum(text, index).then_some(definition),
            (_, Kind::Assignment) if definition.indented && in_enum(text, index) => {
                definition.kind = Kind::Declaration;
                Some(definition)
            }
            _ => Some(definition),
        }
    }

    /// 返回 `line` 中的定义和该写法的检查。
    fn matched(&self, line: &str) -> Option<(Definition, Check)> {
        let line = line.trim_end_matches(['\n', '\r']);
        let captures = self.regex.captures(line)?;
        // 合并后的正则中只有匹配上的那种写法的捕获组参与了匹配。
        let (name, form) = self
            .forms
            .iter()
            .find_map(|(group, form)| Some((captures.name(group)?, form)))?;
        let before = &line[..name.start()];
        let rejected = match form.check {
            Check::Statement => returns_nothing(before),
            Check::CDeclaration => not_a_type(before),
            Check::None | Check::Enumerator => false,
        };
        if rejected {
            return None;
        }
        let definition = Definition {
            col: before.chars().count(),
            kind: form.kind,
            indented: line.starts_with(char::is_whitespace),
        };
        Some((definition, form.check))
    }

    /// 返回 `text` 中所有定义的位置（行号，从 0 起）和定义，按行号从小到大排列。
    ///
    /// `text` 按 `language` 做词法分析，注释和字符串中的「定义」（`// fn helper`）不算。
    pub fn in_text(&self, text: &Rope, language: &'static Language) -> Vec<(usize, Definition)> {
        let mut line = String::new();
        let mut found = Vec::new();
        let lowered = self.name.to_lowercase();
        for (i, slice) in text.lines().enumerate() {
            // 先做字符串包含检查：不含该名字的行无需运行正则。
            line.clear();
            line.extend(slice.chunks());
            let contains = if self.ignore_case {
                line.to_lowercase().contains(&lowered)
            } else {
                line.contains(&self.name)
            };
            if contains && let Some(definition) = self.definition_at(text, i, &line) {
                found.push((i, definition));
            }
        }
        let positions: Vec<(usize, usize)> = found.iter().map(|(l, d)| (*l, d.col)).collect();
        let kinds = token_kinds(text, language, &positions);
        found
            .into_iter()
            .zip(kinds)
            .filter(|(_, kind)| !kind.is_some_and(is_text))
            .map(|(found, _)| found)
            .collect()
    }
}

/// `kind` 是否是注释或字符串：其中的名字只是文字，不是代码。
pub(crate) fn is_text(kind: winc_syntax::TokenKind) -> bool {
    matches!(
        kind,
        winc_syntax::TokenKind::Comment | winc_syntax::TokenKind::String
    )
}

/// `before`（名字之前的部分）的最后一个单词是否是语句关键字（见 [`NOT_RETURN_TYPES`]）。
///
/// 是时 `word name(…) {` 不是方法定义。
fn returns_nothing(before: &str) -> bool {
    let word = before
        .trim_end_matches(|c: char| c.is_whitespace() || c == '*' || c == '&')
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    NOT_RETURN_TYPES.contains(&word)
}

/// 名字之前的部分 `before` 是否表明不是声明：最后一个单词是语句关键字（[`NOT_TYPES`]），或只有
/// 一个单词且后接两侧带空格的 `*`、`&`（乘法、按位与）。
fn not_a_type(before: &str) -> bool {
    let words = before.trim_end_matches(|c: char| c.is_whitespace() || c == '*' || c == '&');
    let last = words
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    if NOT_TYPES.iter().any(|w| w.eq_ignore_ascii_case(last)) {
        return true;
    }
    let operator = &before[words.len()..];
    let single = words
        .trim_start_matches(|c: char| !(c.is_alphanumeric() || c == '_'))
        .split_whitespace()
        .count()
        == 1;
    single
        && [" * ", " & "]
            .iter()
            .any(|op| operator.starts_with(op) && operator.trim() == op.trim())
}

/// 判断是否位于 `enum` 块内时往上查看的最多行数（行）：使单次检查的开销有上限，同时容纳芯片
/// 头文件中上千个成员的中断号、外设枚举。
const ENUM_SCAN_LINES: usize = 4096;

/// `text` 第 `line` 行是否位于 `enum` 块内：往上第一个未配对的 `{` 之前（同一行或上一行）含
/// `enum`。
///
/// 只跳过 `//` 行末注释，字符串和块注释中的花括号也计入。
pub(crate) fn in_enum(text: &Rope, line: usize) -> bool {
    let mut depth = 0usize;
    let first = line.saturating_sub(ENUM_SCAN_LINES);
    for index in (first..line).rev() {
        let content: String = text.line(index).chars().collect();
        let code = content.split("//").next().unwrap_or_default();
        for (at, c) in code.char_indices().rev() {
            match c {
                '}' => depth += 1,
                '{' if depth > 0 => depth -= 1,
                '{' => {
                    let head = code[..at].trim();
                    let head = if head.is_empty() && index > 0 {
                        text.line(index - 1).to_string()
                    } else {
                        head.to_owned()
                    };
                    return head
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .any(|word| word == "enum");
                }
                _ => {}
            }
        }
    }
    false
}

/// 指代当前对象或当前类型的接收者：其后的成员定义在当前文件或其父类型中。
const OWN_RECEIVERS: &[&str] = &["this", "self", "Self", "super", "base", "cls", "me"];

/// 返回第 `line` 行 `col` 列（字符偏移，均从 0 起）处的名字 `name` 所属成员访问（`service.name`、
/// `Type::name`、`ptr->name`）的接收者类型名；不是成员访问或无法推断时返回 `None`。
///
/// 接收者以大写字母开头时即为类型；否则取 `text` 中光标之前最近的声明或赋值（`Type service`、
/// `service: Type`、`service = new Type(`），没有时取之后的第一处，只识别以大写字母开头的类型名。
/// 接收者指代当前对象（`this`、`self` 等）时返回 `None`。
pub fn receiver_type(text: &Rope, line: usize, col: usize, name: &str) -> Option<String> {
    let content: Vec<char> = text.get_line(line)?.chars().collect();
    let len = name.chars().count();
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    // 光标处名字的起点。
    let start = (col.saturating_sub(len)..=col.min(content.len())).find(|&at| {
        content
            .get(at..at + len)
            .is_some_and(|w| w.iter().copied().eq(name.chars()))
            && !at.checked_sub(1).is_some_and(|k| is_name(content[k]))
    })?;
    let before = &content[..start];
    let access = [".", "::", "->"].into_iter().find_map(|mark| {
        let at = before.len().checked_sub(mark.len())?;
        before[at..].iter().copied().eq(mark.chars()).then_some(at)
    })?;
    // `?.`、`!.` 中的 `?`、`!` 不属于接收者。
    let end = match before[..access].last() {
        Some('?' | '!') => access - 1,
        _ => access,
    };
    let from = end
        - before[..end]
            .iter()
            .rev()
            .take_while(|&&c| is_name(c))
            .count();
    let receiver: String = before[from..end].iter().collect();
    if receiver.is_empty() || OWN_RECEIVERS.contains(&receiver.as_str()) {
        return None;
    }
    if receiver.starts_with(char::is_uppercase) {
        return Some(receiver);
    }
    let escaped = regex::escape(&receiver);
    let declared = Regex::new(&format!(
        r"(?:\b(?P<a>[A-Z]\w*)(?:<[^;(){{}}=]*>)?(?:\[\])?[*&\s]+{escaped}\s*(?:[;=,)]|$)|\b{escaped}\s*:\s*(?:&\s*(?:mut\s+)?|\*)?(?:\w+(?:::|\.))*(?P<b>[A-Z]\w*)|\b{escaped}\s*:?=\s*(?:new\s+)?(?:\w+(?:::|\.))*(?P<c>[A-Z]\w*)\s*[(:{{<])"
    ))
    .ok()?;
    let mut found = None;
    for (index, slice) in text.lines().enumerate() {
        if index > line && found.is_some() {
            break;
        }
        let content = std::borrow::Cow::<str>::from(slice);
        if let Some(caps) = declared.captures(&content) {
            let kind = ["a", "b", "c"]
                .into_iter()
                .find_map(|group| caps.name(group));
            found = kind.map(|m| m.as_str().to_owned()).or(found);
        }
    }
    found
}

/// 判断路径 `path` 的文件名（不含扩展名）是否以类型 `type_name` 命名：相同时返回 `Some(true)`，
/// 包含类型名（`LoginServiceImpl`、`DefaultLoginService`）时返回 `Some(false)`，无关时返回 `None`。
pub fn named_after(path: &Path, type_name: &str) -> Option<bool> {
    let stem = path.file_stem()?.to_string_lossy();
    stem.contains(type_name).then(|| stem == type_name)
}

/// 返回各处定义的排序键，键越小越靠前。
///
/// `language` 是当前文件的语言，`dir` 是当前文件所在的目录。相关语言由 `Language`
/// 的 `is_related` 判定，其中直接位于 `dir` 中的又排在前面。`path` 的语言按文件名检测。
pub fn rank(path: &Path, language: &Language, dir: &Path) -> (bool, bool) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let related = language.is_related(winc_syntax::detect(&name));
    let same_dir = path.parent() == Some(dir);
    (!related, !same_dir)
}

#[cfg(test)]
mod tests;
