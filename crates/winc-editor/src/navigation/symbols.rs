//! 提取文件中的符号，供「转到符号」使用（在 ⌘P 面板中输入 `@`）。
//!
//! 与 [`crate::navigation::definition`] 相同，本模块逐行用正则识别常见写法，不接入语言服务器。
//! 识别的符号包括函数、方法、类型、模块、顶层的常量和变量、Markdown 标题、配置文件的小节。
//!
//! 符号的归属按缩进（Markdown 为标题级别）判断：位于其前、层级更浅且距离最近的类型、函数、
//! 模块或标题是它的上级；直接位于类型或模块之下的函数记为方法。

use std::sync::LazyLock;

use regex::Regex;
use ropey::Rope;
use winc_i18n::t;
use winc_syntax::Language;

/// 符号的种类，决定列表中显示的图标和种类名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// 函数。
    Function,
    /// 类型的方法（按缩进判定归属；Go 按接收者类型判定）。
    Method,
    /// 类型、结构体、接口等。
    Type,
    /// 模块、命名空间。
    Module,
    /// 顶层的常量或变量。
    Variable,
    /// Markdown 标题。
    Heading,
    /// 配置文件的小节。
    Section,
}

impl SymbolKind {
    /// 返回列表中显示的种类名（「函数」「方法」等）。
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Function => t!("editor-symbol-function"),
            SymbolKind::Method => t!("editor-symbol-method"),
            SymbolKind::Type => t!("editor-symbol-type"),
            SymbolKind::Module => t!("editor-symbol-module"),
            SymbolKind::Variable => t!("editor-symbol-variable"),
            SymbolKind::Heading => t!("editor-symbol-heading"),
            SymbolKind::Section => t!("editor-symbol-section"),
        }
    }

    /// 是否可以作为后续符号的上级。
    fn contains(self) -> bool {
        matches!(
            self,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Type
                | SymbolKind::Module
                | SymbolKind::Heading
        )
    }
}

/// 文件里的一个符号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// 符号名。
    pub name: String,
    /// 符号种类。
    pub kind: SymbolKind,
    /// 所在行号（从 0 起）。
    pub line: usize,
    /// 名字在所在行内的列号（字符偏移，从 0 起）。
    pub col: usize,
    /// 上级符号的名字；没有上级时为 `None`。
    pub container: Option<String>,
    /// 嵌套层级：没有上级时为 0，每多一层上级加 1。
    ///
    /// 按缩进（Markdown 为标题级别）判定；行内写明上级的写法（Go 方法的接收者类型）不计入。
    pub depth: usize,
    /// Markdown 标题的级别（行首 `#` 的个数，1 到 6）；其他符号为 0。
    pub heading: u8,
}

/// 标识符的正则：字母、数字、下划线、`$`，不以数字开头。
///
/// 下面各条正则中的 `IDENT` 先替换为这段正则，[`regex`] 负责替换。
const IDENT: &str = r"[\w$--\d][\w$]*";

/// 编译 `pattern`，其中的 `ANNOTATIONS`、`IDENT` 先替换为 [`ANNOTATIONS`]、[`IDENT`] 的正则。
fn regex(pattern: &str) -> Regex {
    let pattern = pattern.replace("ANNOTATIONS", ANNOTATIONS);
    Regex::new(&pattern.replace("IDENT", IDENT)).expect("符号的正则写错了")
}

/// 写在声明同一行、位于声明之前的注解或装饰器：`@Name`、`@Name(参数)`，可以连写多个。
const ANNOTATIONS: &str = r"(?:@IDENT(?:\.IDENT)*(?:\([^()]*\))?\s+)*";

/// 声明语句：关键字（`fn`、`def`、`class`、`struct` 等）后跟名字，关键字前可以有注解和修饰词。
static DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"^\s*ANNOTATIONS(?:(?:pub(?:\s*\([^)]*\))?|export|default|declare|abstract|final|sealed|open|data|inline|value|private|public|protected|internal|static|async|unsafe|extern(?:\s+"[^"]*")?|const|override|suspend|partial|enum|annotation|companion)\s+)*(?P<kw>fn|def|func|function\*?|fun|class|struct|enum|trait|interface|type|union|mod|module|namespace|protocol|extension|object|record|macro_rules!)\s+(?:<[^>]*>\s*)?(?:IDENT\.)*(?P<name>IDENT)"#,
    )
});

/// Go 的方法：`func (s *Server) Name(`。
static GO_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"^func\s*\(\s*(?:IDENT\s+)?\*?\s*(?P<recv>IDENT)(?:\[[^\]]*\])?\s*\)\s*(?P<name>IDENT)")
});

/// Rust 的 `impl Trait for Type`、`impl<T> Type<T>`：符号名取 `impl`（及其泛型参数）之后、
/// `where` 或 `{` 之前的部分。
static RUST_IMPL: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^\s*(?:unsafe\s+)?impl\b\s*(?:<[^{]*?>)?\s*(?P<name>[^{;<\s][^{;]*?)\s*(?:\bwhere\b.*|\{.*)?$",
    )
});

/// JS、TS 中以函数或箭头函数赋值的声明：`const run = async (a) => …`、`export let f = function`。
static ARROW: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^\s*(?:export\s+)?(?:const|let|var)\s+(?P<name>IDENT)\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\b|\([^()]*\)\s*(?::[^=]+)?=>|IDENT\s*=>)",
    )
});

/// 类中的方法（JS、TS、Java、C++ 等）：带缩进的 `name(a, b) {`，名字前可以有修饰词和返回类型。
///
/// 参数中不能出现引号，以排除 `it("works", function () {` 这类调用。
static METHOD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"^\s+ANNOTATIONS(?:(?:public|private|protected|static|async|override|abstract|final|synchronized|virtual|internal|open|readonly|get|set)\s+)*(?:[\w<>\[\],.?*&:]+\s+)?(?P<name>IDENT)\s*(?:<[^>]*>)?\s*\([^;()"'`]*\)\s*(?::\s*[^{;=]+)?(?:throws\s+[\w., ]+)?(?:const\s*)?\{\s*$"#,
    )
});

/// C、C++ 的函数定义：行首（无缩进）是返回类型，其后是名字和左括号，该行不含分号。
static C_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"^(?P<head>[A-Za-z_][\w:<>,*&\s]*[\s*&])(?P<name>[A-Za-z_~][\w:~]*)\s*\([^;]*$")
});

/// C 的 `#define` 宏。
static C_DEFINE: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*#\s*define\s+(?P<name>IDENT)"));

/// 顶格的常量和变量：`const X`、`static mut Y`、`let z`、`var w`。
static TOP_VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^(?:export\s+)?(?:pub(?:\s*\([^)]*\))?\s+)?(?:const|static|let|var|val)\s+(?:mut\s+)?(?P<name>IDENT)",
    )
});

/// Python 模块顶层（行首无缩进）的赋值；`==` 比较不算。
static PY_ASSIGN: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^(?P<name>IDENT)\s*(?::[^=]*)?=(?:[^=]|$)"));

/// shell 函数：`name() {` 或 `function name {`。
static SHELL_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^\s*(?:function\s+(?P<name>[\w.:-]+)|(?P<bare>[\w.:-]+)\s*\(\))"));

/// Makefile 的目标，如 `build: deps`；`:=` 属于赋值，不匹配。
static MAKE_TARGET: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^(?P<name>[\w./%$(){}-]+)\s*::?(?:[^=]|$)"));

/// Markdown 标题：行首的 `#` 到 `######` 加标题文字，末尾的 `#` 不属于标题。
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^(?P<hashes>#{1,6})\s+(?P<name>.*?)\s*#*\s*$"));

/// INI 风格的小节：`[名字]` 或 `[[名字]]`。
static SECTION: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*\[\[?\s*(?P<name>[^\]]+?)\s*\]\]?"));

/// YAML 的键：行首的 `键:`，键可以用引号包住。
static YAML_KEY: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"^(?P<name>[\w.-]+|"[^"]+"|'[^']+')\s*:(?:\s|$)"#));

/// SQL 的 `CREATE TABLE`、`CREATE VIEW` 等语句；`IF NOT EXISTS` 不算名字的一部分。
static SQL_CREATE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?i)^\s*create\s+(?:or\s+replace\s+)?(?:temp(?:orary)?\s+)?(?P<kw>table|view|function|procedure|index|trigger|type|schema|sequence)\s+(?:if\s+not\s+exists\s+)?(?P<name>[\w."`\[\]]+)"#,
    )
});

/// 不能作为方法名的关键字。
///
/// 它们后接括号和 `{` 的写法是控制语句或调用，如 `if (x) {`、`while (y) {`，不是方法定义。
const NOT_NAMES: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "with",
    "return",
    "else",
    "do",
    "try",
    "new",
    "function",
    "synchronized",
    "foreach",
    "using",
    "lock",
    "fixed",
    "when",
    "sizeof",
    "typeof",
];

/// 返回 `text` 中的符号，按行号从小到大排列。
///
/// `language` 决定按哪种语言的写法识别。
pub fn symbols(text: &Rope, language: &Language) -> Vec<Symbol> {
    let mut found = Vec::new();
    // 上级栈，元素为（层级，名字，种类）；层级是缩进列数或 Markdown 标题级别。
    let mut parents: Vec<(usize, String, SymbolKind)> = Vec::new();
    let mut skip = Skip::default();
    let mut line = String::new();

    for (i, slice) in text.lines().enumerate() {
        line.clear();
        line.extend(slice.chunks());
        let content = line.trim_end_matches(['\n', '\r']);

        if skip.line(content, language) {
            continue;
        }
        let Some((name, kind, col, level, container)) = recognize(content, language) else {
            continue;
        };

        while parents.last().is_some_and(|(l, ..)| *l >= level) {
            parents.pop();
        }
        let parent = parents.last();
        // ArkTS 的函数体内，行末带 `{` 的调用是带尾随闭包的组件（`Column() {`），不是声明。
        if language.id == "arkts"
            && kind == SymbolKind::Function
            && matches!(
                parent,
                Some((_, _, SymbolKind::Function | SymbolKind::Method))
            )
            && !DECLARATION.is_match(content)
            && !ARROW.is_match(content)
        {
            continue;
        }
        let kind = match (kind, parent) {
            (SymbolKind::Function, Some((_, _, SymbolKind::Type | SymbolKind::Module))) => {
                SymbolKind::Method
            }
            _ => kind,
        };
        let container = container.or_else(|| parent.map(|(_, name, _)| name.clone()));
        let depth = parents.len();
        if kind.contains() {
            parents.push((level, name.clone(), kind));
        }
        found.push(Symbol {
            name,
            kind,
            line: i,
            col,
            container,
            depth,
            heading: if kind == SymbolKind::Heading {
                level as u8
            } else {
                0
            },
        });
    }
    found
}

/// 逐行跟踪注释和多行字符串的状态，用于跳过其中的行：块注释、Python 的三引号字符串、
/// Markdown 的围栏代码块。
#[derive(Default)]
struct Skip {
    /// 位于块注释内。
    block_comment: bool,
    /// 位于三引号字符串内。
    docstring: bool,
    /// 位于 Markdown 的围栏代码块内。
    fence: bool,
}

impl Skip {
    /// 是否应跳过 `content` 这一行（位于注释、多行字符串或代码块内）；同时更新跨行状态。
    fn line(&mut self, content: &str, language: &Language) -> bool {
        let trimmed = content.trim_start();

        if language.id == "markdown" {
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                self.fence = !self.fence;
                return true;
            }
            return self.fence;
        }

        // 只跟踪语言的第一种块注释写法，其余写法很少出现在符号所在的行附近。
        if let Some(&(open, close)) = language.block_comments.first() {
            if self.block_comment {
                self.block_comment = !content.contains(close);
                return true;
            }
            if let Some(rest) = trimmed.strip_prefix(open) {
                self.block_comment = !rest.contains(close);
                return true;
            }
        }

        if language.triple_quotes {
            let quotes = content.matches("\"\"\"").count() + content.matches("'''").count();
            let inside = self.docstring;
            if quotes % 2 == 1 {
                self.docstring = !self.docstring;
            }
            if inside || (quotes > 0 && trimmed.starts_with(['"', '\''])) {
                return true;
            }
        }

        language
            .line_comments
            .iter()
            .any(|comment| trimmed.starts_with(comment) && !trimmed.starts_with("#define"))
    }
}

/// 识别一行中的符号，返回（名字，种类，列，层级，行内写明的上级）。
///
/// 列是名字的字符偏移（从 0 起）；层级是缩进列数（Tab 按 4 列计）或 Markdown 标题级别。
/// 行内写明的上级只有 Go 方法的接收者类型。
fn recognize(
    content: &str,
    language: &Language,
) -> Option<(String, SymbolKind, usize, usize, Option<String>)> {
    let indent = indent_width(content);
    let found = |caps: &regex::Captures<'_>, kind: SymbolKind| {
        let name = caps.name("name")?;
        let col = content[..name.start()].chars().count();
        Some((name.as_str().to_owned(), kind, col, indent, None))
    };

    match language.id {
        "markdown" => {
            let caps = HEADING.captures(content)?;
            let level = caps["hashes"].len();
            let (name, kind, col, _, _) = found(&caps, SymbolKind::Heading)?;
            return (!name.is_empty()).then_some((name, kind, col, level, None));
        }
        "toml" | "ini" | "properties" => {
            return found(&SECTION.captures(content)?, SymbolKind::Section);
        }
        "yaml" => return found(&YAML_KEY.captures(content)?, SymbolKind::Section),
        "sql" => {
            let caps = SQL_CREATE.captures(content)?;
            let kind = match caps["kw"].to_lowercase().as_str() {
                "function" | "procedure" | "trigger" => SymbolKind::Function,
                _ => SymbolKind::Type,
            };
            return found(&caps, kind);
        }
        "makefile" => {
            if let Some(caps) = MAKE_TARGET.captures(content)
                && !caps["name"].starts_with('.')
            {
                return found(&caps, SymbolKind::Function);
            }
            return None;
        }
        "shell" | "fish" => {
            let caps = SHELL_FUNCTION.captures(content)?;
            let name = caps.name("name").or_else(|| caps.name("bare"))?;
            let col = content[..name.start()].chars().count();
            return Some((
                name.as_str().to_owned(),
                SymbolKind::Function,
                col,
                indent,
                None,
            ));
        }
        "text" | "json" | "html" | "xml" | "svg" | "css" | "scss" | "sass" | "less"
        | "dockerfile" | "dotenv" | "ignore" => return None,
        _ => {}
    }

    if language.id == "go"
        && let Some(caps) = GO_METHOD.captures(content)
    {
        let (name, _, col, level, _) = found(&caps, SymbolKind::Method)?;
        return Some((
            name,
            SymbolKind::Method,
            col,
            level,
            Some(caps["recv"].to_owned()),
        ));
    }
    if language.id == "rust"
        && let Some(caps) = RUST_IMPL.captures(content)
    {
        let name = caps["name"].trim_end().to_owned();
        let col = content[..caps.name("name")?.start()].chars().count();
        return Some((name, SymbolKind::Module, col, indent, None));
    }
    if let Some(caps) = DECLARATION.captures(content) {
        let kind = match &caps["kw"] {
            "mod" | "module" | "namespace" | "extension" => SymbolKind::Module,
            "class" | "struct" | "enum" | "trait" | "interface" | "type" | "union" | "protocol"
            | "object" | "record" => SymbolKind::Type,
            _ => SymbolKind::Function,
        };
        return found(&caps, kind);
    }
    if let Some(caps) = ARROW.captures(content) {
        return found(&caps, SymbolKind::Function);
    }

    let c_family = matches!(language.id, "c" | "cpp" | "objective-c" | "objective-cpp");
    let braces = c_family
        || matches!(
            language.id,
            "javascript" | "typescript" | "arkts" | "vue" | "java" | "groovy" | "scala" | "kotlin"
        );
    if braces
        && let Some(caps) = METHOD.captures(content)
        && !NOT_NAMES.contains(&&caps["name"])
    {
        return found(&caps, SymbolKind::Function);
    }
    if c_family {
        if let Some(caps) = C_FUNCTION.captures(content) {
            let first = caps["head"].split_whitespace().next().unwrap_or_default();
            let statement = matches!(
                first,
                "return" | "else" | "if" | "while" | "for" | "switch" | "case" | "typedef" | "goto"
            );
            if !statement && !NOT_NAMES.contains(&&caps["name"]) {
                return found(&caps, SymbolKind::Function);
            }
        }
        if let Some(caps) = C_DEFINE.captures(content) {
            return found(&caps, SymbolKind::Variable);
        }
    }
    // C 系语言的 `static int x` 把类型写在名字之前，上面的规则无法识别；这里直接返回，
    // 避免 `TOP_VARIABLE` 把该声明误判为名为 `int` 的变量。
    if c_family {
        return None;
    }
    if let Some(caps) = TOP_VARIABLE.captures(content) {
        return found(&caps, SymbolKind::Variable);
    }
    if language.id == "python"
        && let Some(caps) = PY_ASSIGN.captures(content)
    {
        return found(&caps, SymbolKind::Variable);
    }
    None
}

/// 返回行首空白的宽度（列），Tab 按 4 列计，其他空白字符按 1 列计。
fn indent_width(content: &str) -> usize {
    content
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

#[cfg(test)]
mod tests;
