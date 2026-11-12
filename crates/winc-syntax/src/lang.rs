//! 语言定义：文件类型识别、关键字、注释与字符串规则，以及 [`Language`] 表。
//!
//! 每种语言的规则都是静态数据，各字段的单位与含义见 [`Language`]；词法分析只读这些字段，
//! 不为具体语言写分支。

mod code;
mod config;
mod data;
mod script;

use code::*;
use config::*;
use data::*;
use script::*;

/// 一种语言的词法规则，全部是静态数据。
#[derive(Debug)]
pub struct Language {
    /// 语言的唯一标识，小写，用于按 id 查找（例如 `rust`、`cpp`）。
    pub id: &'static str,
    /// 状态栏上显示的名字（例如 `Rust`、`C++`）。
    pub name: &'static str,
    /// 文件扩展名（不含点，小写），用于按扩展名识别。
    pub extensions: &'static [&'static str],
    /// 需要按完整文件名识别的文件（例如 `Makefile`、`.gitignore`），区分大小写。
    pub file_names: &'static [&'static str],
    /// 按文件名开头识别的文件（例如 `Dockerfile.` 对应 `Dockerfile.dev`），区分大小写。
    ///
    /// 优先于扩展名：`Dockerfile.dev` 的扩展名 `dev` 不对应任何语言。
    pub name_prefixes: &'static [&'static str],
    /// 首行 `#!` 中的解释器名（例如 `python`、`bash`），用于识别没有扩展名的脚本。
    ///
    /// 解释器名去掉路径和末尾的版本号（`/usr/bin/python3.12` 为 `python`），`env` 取其后的参数。
    pub interpreters: &'static [&'static str],
    /// 关键字表。
    pub keywords: &'static [&'static str],
    /// 内置常量和类型名，按 [`crate::TokenKind::Builtin`] 着色，与关键字颜色不同。
    pub builtins: &'static [&'static str],
    /// 关键字是否不区分大小写（SQL 为真）。
    ///
    /// 为真时，关键字表和内置名表只写小写。
    pub ignore_case: bool,
    /// 行注释符；语言没有行注释时为空。
    ///
    /// 第一个用于切换注释，其余的只用于着色（INI 的 `#` 和 `;`）。
    pub line_comments: &'static [&'static str],
    /// 行注释符之前是否必须是行首或空白。
    ///
    /// Shell 的 `$#` 和 YAML 的 `a#b` 中的 `#` 不是注释。
    pub comment_needs_space: bool,
    /// 块注释的开头与结尾；语言没有块注释时为空。
    ///
    /// 可以有多种写法（Perl 的 `=pod`、`=head1` 都以 `=cut` 结束）。
    pub block_comments: &'static [(&'static str, &'static str)],
    /// 块注释是否可以嵌套（Rust、Swift、Haskell 的 `/* /* */ */`）。
    pub nested_comments: bool,
    /// 块注释的开头与结尾是否只在行首识别（Ruby 的 `=begin`、`=end`，Perl 的 POD）。
    ///
    /// 这类注释的开头与结尾各占一整行。
    pub line_start_comments: bool,
    /// 双引号是否界定字符串。
    ///
    /// 正文为主的格式（纯文本、Markdown、表格）中的双引号是标点：否则一个落单的 `"` 会把行内剩下
    /// 的部分都当作字符串。
    pub double_quote_string: bool,
    /// 是否支持 `"""` 和 `'''` 三引号字符串。
    pub triple_quotes: bool,
    /// 双引号、单引号字符串是否可以跨行（Rust 的双引号字符串、Shell 的单双引号字符串为真）。
    pub multiline_strings: bool,
    /// 单引号是否界定字符串。
    ///
    /// 为假时按字符字面量处理，其余单引号（Rust 的生命周期）当作普通文本。
    pub single_quote_string: bool,
    /// 字符串中的转义字符（多数语言为反斜杠，PowerShell 为反引号，批处理为 `^`）。
    pub escape_char: char,
    /// 字符串中连写两个收尾引号是否表示一个引号（SQL 的 `'it''s'`、C# 的 `@"a""b"`）。
    pub doubled_quotes: bool,
    /// 单引号字符串（`'…'`、`'''…'''`）中反斜杠是否转义下一个字符。
    ///
    /// TOML 的字面字符串、YAML 的单引号字符串没有反斜杠转义：否则 `'C:\dir\'` 末尾的反斜杠会转义
    /// 收尾的单引号，使其后的全部文本都被当作字符串。
    pub single_quote_escapes: bool,
    /// 紧跟在字母、数字或 `_` 之后的单引号是否算撇号（`Don't`、`users'`），不开始字符串。
    ///
    /// 用于正文和代码混在一起的标记语言（HTML、XML）：正文中的撇号否则会把行内剩下的部分都当作
    /// 字符串。Python 的 `f'…'`、`b'…'` 前缀紧跟着引号，不适用。
    pub apostrophes: bool,
    /// 是否支持反引号字符串，可以跨行（JS 模板字符串、Go 原始字符串、Shell 命令替换）。
    pub backtick_string: bool,
    /// 反引号内的反斜杠是否为转义（JS 模板字符串、Shell 命令替换为真）。
    ///
    /// Go 原始字符串、SQL 标识符中的反斜杠不是转义：否则 `` `\` `` 中的反斜杠
    /// 会转义收尾的反引号，使其后的全部文本都被当作字符串。
    pub backtick_escapes: bool,
    /// 反引号字符串中的 `${…}` 是否为代码（JS 模板字符串）。
    ///
    /// 插值内部的代码按普通规则着色，嵌套的模板字符串不会提前结束外层字符串。插值在本行内没有闭合
    /// 时，`${` 按字符串内容处理。
    pub template_interpolation: bool,
    /// 双引号字符串中按代码着色的插值。
    ///
    /// [`Interpolation::single_quotes`] 为真时单引号字符串中也识别，为假时只在双引号字符串中识别。
    pub interpolation: Option<Interpolation>,
    /// 使紧随其后的字符串成为原始字符串的前缀（Python 的 `r`、Dart 的 `r`、Scala 的 `raw`）。
    ///
    /// 原始字符串中不识别转义序列、插值和变量；反斜杠仍使紧随的引号不收尾（Python 的规则）。
    pub raw_prefixes: &'static [&'static str],
    /// 以固定定界符开始和结束、可以跨行、没有转义的字符串（PowerShell 的 `@"…"@`、Nix 的
    /// `''…''`、C# 的 `@"…"`、LaTeX 的 `$…$`）。
    pub block_strings: &'static [BlockString],
    /// 长括号的写法：`[[…]]`、`[==[…]==]` 是字符串，前面是语言的注释符时是注释，收尾的 `=` 个数
    /// 与开头相同；不支持时为 `None`。
    ///
    /// Lua 的长注释是 `--[[…]]`，`CMake` 的括号注释是 `#[[…]]`（文档常用 `#[=======[…]=======]`）。
    pub long_brackets: Option<&'static str>,
    /// 从该记号起到行尾是字符串（Zig 多行字符串的每一行以 `\\` 开头）。
    pub line_string: Option<&'static str>,
    /// 是否识别 `/…/flags` 正则表达式字面量（JS）。
    ///
    /// 否则正则中的引号或反引号会开始字符串，反引号还会把文件剩下的部分都当作字符串。斜杠之前
    /// 是值（标识符、数字、右括号、字符串）时是除号。
    pub regex_literals: bool,
    /// 是否按 Markdown（CommonMark 与 GFM）着色：代码块、行内代码、标题、引用、列表标记、强调、
    /// 链接和行内 HTML 标签。
    ///
    /// 没有配对的反引号和强调标记是普通文本，不开始代码。
    pub markdown: bool,
    /// 是否识别数字字面量、字符字面量和函数名（后面紧跟 `(` 的标识符）。
    ///
    /// 正文为主的格式中这些都是普通文字，不着色。
    pub code_tokens: bool,
    /// 选中文字后键入时包围选区、而不替换选区的字符。
    ///
    /// 括号和引号对所有语言都会包围选区，不列在此字段中。Markdown 为 `*`、`_`、`~`、`$`，
    /// 对应 GFM 的强调、删除线标记与常用的数学公式定界符：输入两个 `*` 得到粗体标记。
    pub surround: &'static [char],
    /// 用定界符决定收尾的原始字符串写法（Rust、Swift 的 `#` 个数，C++ 的 `R"delim(`）。
    ///
    /// 这类字符串内部没有转义，可以跨行。用 `r`、`f` 这类前缀表示的原始字符串见
    /// [`Language::raw_prefixes`]。
    pub raw_strings: RawStrings,
    /// 代码中（字符串之外）反斜杠是否转义下一个字符（Shell 的 `\"`、`\$`）。
    ///
    /// 否则转义的引号会开始字符串。
    pub code_escapes: bool,
    /// 变量引用的写法；不识别变量时为 [`Variables::None`]。
    ///
    /// 变量在双引号字符串和反引号字符串中同样识别（`"$HOME/bin"`）。
    pub variables: Variables,
    /// here 文档的写法；不支持时为 [`Heredocs::None`]。
    pub heredocs: Heredocs,
    /// 是否把以 `@` 开头的标识符识别为装饰器或注解。
    pub decorators: bool,
    /// 是否把 `#[…]`、`#![…]` 识别为属性（Rust、PHP 8），开头与路径按装饰器着色。
    pub hash_attributes: bool,
    /// 是否把以 `#` 开头的行整行着色（C 预处理指令、Markdown 标题）。
    pub hash_lines: bool,
    /// 是否按差异文件（`diff -u` 的输出）整行着色：`+` 开头的是新增，`-` 开头的是删除，`@@` 开头的
    /// 是位置，`+++`、`---` 开头的是文件名。
    pub diff_lines: bool,
    /// 键与值之间的分隔符（YAML、CSS 的 `:`，TOML、INI 的 `=`）；为空时不识别键。
    ///
    /// 只识别行首（缩进和 YAML 列表的 `- ` 之后）的键。分隔符是 `:` 时，后面必须是空白或行尾，
    /// 否则 `a:hover`、`http://` 中的冒号也会被当成分隔符。
    pub key_separators: &'static [char],
    /// 是否把行首的 `[name]`、`[[name]]` 当作节标题（TOML 的表、INI 的节）。
    pub section_headers: bool,
    /// 后面紧跟 `:` 的双引号字符串是否为键（JSON 的 `{"a": 1}`，不限于行首）。
    pub quoted_keys: bool,
    /// 双引号和反引号括起的是否为标识符而非字符串（SQL 的 `"column"`、`` `table` ``）。
    pub quoted_identifiers: bool,
    /// 标记语言的写法：标签、属性、实体，以及 HTML 中内嵌的脚本与样式。
    pub markup: Markup,
    /// 是否按样式表着色：选择器中的类名、id、伪类和元素名，声明中的属性名和十六进制颜色。
    pub stylesheet: bool,
    /// 是否把 `:name` 识别为符号（Ruby、Elixir 的符号，Clojure 的关键字）。
    pub colon_symbols: bool,
    /// 是否识别 YAML 的块标量：值是 `|` 或 `>`（可以带 `-`、`+` 和缩进数字）的键之后，缩进比键
    /// 更深的行都是字符串内容。
    pub block_scalars: bool,
    /// 引号紧跟在普通单词之后（中间可以有空白）时，是否算正文中的标点，不开始字符串。
    ///
    /// 用于 JSX 这类正文与代码混排的标记：`<p>He said "hi</p>` 的引号是正文。代码中的字符串不会
    /// 直接跟在单词之后；`return`、`from`、`case` 关键字之后仍然开始字符串。
    pub prose_quotes: bool,
    /// 除字母、数字和 `_` 外，名字中间还可以有的字符（CSS 类名和 Clojure 中的 `-`、JS 中的
    /// `$`），用于转到定义和查找用法时取光标处的名字。
    pub name_chars: &'static [char],
    /// 只能出现在名字开头的字符，可以连写（PHP 的 `$name`、Ruby 的 `@name`、`@@name`）。
    pub name_sigils: &'static [char],
    /// 只能出现在名字末尾的一个字符（Ruby、Elixir 的 `valid?`、`save!`）。
    pub name_suffixes: &'static [char],
    /// 是否识别 Ruby 的百分号字面量：`%w[…]`、`%i(…)`、`%q{…}`、`%r{…}`、`%(…)`，可以跨行，括号
    /// 定界符可以在内部配对嵌套。
    pub percent_literals: bool,
    /// 是否把 `\name` 识别为命令（LaTeX），`\` 加其他字符是转义。
    pub backslash_commands: bool,
    /// 是否把首字母大写、且含小写字母的标识符（如 `Config`、`HashMap`）当作类型名
    /// （[`crate::TokenKind::Type`]）。全大写的标识符（常量、宏）不算；Go 中大写开头的是导出的
    /// 函数和变量，不适用。
    pub capitalized_types: bool,
    /// 是否把后面紧跟 `!` 和左括号的标识符当作宏调用，与函数同色（Rust 的 `println!(…)`）。
    pub macros: bool,
    /// 回车时，光标之前最近的非空白字符是这些字符则多缩进一级。
    pub indent_after: &'static [char],
    /// 回车时，光标之前以开始标签（如 `<div class="a">`）结尾则多缩进一级（HTML、XML）。
    ///
    /// 结束标签、自闭合标签、空元素（`<br>`）和同一行里闭合的标签不算，缩进规则见
    /// [`winc_core::edit::newline`]。
    pub tag_indent: bool,
    /// 语言的默认一级缩进文本（制表符或若干空格）。
    ///
    /// 优先级低于状态栏中的选择、`.editorconfig` 和从文件内容检测出的缩进。
    pub indent_unit: &'static str,
}

/// 原始字符串的写法，见 [`Language::raw_strings`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStrings {
    /// 不支持原始字符串。
    None,
    /// Rust 的 `r"…"`、`r#"…"#`、`br"…"`。
    ///
    /// 收尾由开头 `#` 的个数决定，最多 2 个；更多的 `#` 按普通字符串处理。
    Rust,
    /// C++11 的 `R"delim(…)delim"`，前面可以有编码前缀（`u8R`、`LR`）：收尾是 `)`、同样的定界符
    /// 和 `"`。依据 C++ 标准，定界符最长 16 个字符，不含空白、括号、反斜杠和双引号。
    Cpp,
    /// Swift 的 `#"…"#`、`##"…"##`：收尾由开头 `#` 的个数决定，最多 2 个；更多的 `#` 按普通
    /// 字符串处理。
    Swift,
}

/// 字符串插值的写法，见 [`Language::interpolation`]。
#[derive(Debug, Clone, Copy)]
pub struct Interpolation {
    /// 插值的开头（`${`、`#{`、`$(`、`\(`、`{`）。
    pub open: &'static str,
    /// 插值的收尾字符，与之配对的括号在插值内部计数。
    pub close: char,
    /// 使字符串支持插值的前缀（C# 的 `$`、Python 的 `f`）；为空时所有字符串都支持插值。
    ///
    /// 开头是 `{` 时，连写的 `{{` 表示一个花括号，不开始插值。
    pub prefixes: &'static [&'static str],
    /// 单引号字符串是否同样支持插值（Dart、Elixir）。
    pub single_quotes: bool,
}

/// 块字符串的写法，见 [`Language::block_strings`]。
#[derive(Debug, Clone, Copy)]
pub struct BlockString {
    /// 开头的定界符。
    pub open: &'static str,
    /// 收尾的定界符。
    pub close: &'static str,
    /// 是否支持插值和变量（PowerShell 的 `@"…"@` 支持，`@'…'@` 不支持）。
    pub interpolate: bool,
}

/// here 文档的写法，见 [`Language::heredocs`]。
///
/// 内容从下一行开始，到只含结束标记的一行为止；结束标记带引号时内容中不展开变量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heredocs {
    /// 不支持 here 文档。
    None,
    /// POSIX shell：`<<EOF`、`<< EOF`，`<<-` 允许结束行以制表符缩进。`<<<` 是 here 字符串，
    /// `<<` 之后是数字或 `$` 时是算术左移。
    Shell,
    /// Ruby、Perl、HCL：`<<EOF`，`<<-`、`<<~` 允许结束行缩进；`<<` 与结束标记之间不能有空白，
    /// `a << b` 是运算符。
    Compact,
    /// PHP：`<<<EOT`、`<<<'EOT'`；结束行可以缩进，结束标记之后可以紧跟 `;`、`,`、`)`。
    Php,
}

/// 标记语言的写法，见 [`Language::markup`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    /// 不是标记语言。
    None,
    /// XML：标签、属性、实体与 CDATA。
    Xml,
    /// HTML、SVG 与单文件组件（Vue、Svelte）：在 XML 之上，`<script>` 的内容按 TypeScript、
    /// `<style>` 的内容按 SCSS 着色，标签名不区分大小写。
    Html,
}

/// 变量引用的写法，见 [`Language::variables`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variables {
    /// 不识别变量。
    None,
    /// Shell：`$name`、`${…}`、位置参数 `$1`、特殊参数 `$@`、`$?`、`$#` 等。
    ///
    /// `$(…)` 是命令替换，`$'…'` 和 `$"…"` 是字符串，都不是变量。双引号字符串中的命令替换按代码
    /// 着色，嵌套的引号不会结束外层字符串（`"$(cat "$f")"`）；`${…}` 中嵌套的引号和花括号同样不会
    /// 提前结束变量（`"${x#"$p"/}"`）。
    Shell,
    /// Makefile：`$(name)`、`${name}`、单字符变量 `$@`、`$<`、`$x`，以及传给 shell 的
    /// `$$name`。
    ///
    /// `$(shell …)` 这类函数调用只把 `$(` 和函数名着色为函数。
    Make,
    /// PHP、Perl、PowerShell、`CMake`、GraphQL、SCSS：`$name`、`${…}`，PowerShell 的
    /// `$env:PATH`，`CMake` 的 `$ENV{X}`。代码和双引号字符串中都识别。
    Dollar,
    /// Kotlin、Dart、Groovy、Scala、Julia：只在支持插值的字符串中识别 `$name`，代码中的 `$` 不是
    /// 变量。
    InStrings,
    /// Ruby：实例变量 `@name`、类变量 `@@name`、全局变量 `$name`。
    Ruby,
    /// 批处理：`%name%`、`!name!`、参数 `%1`、`%~dp0`，以及 `for` 的循环变量 `%%i`。
    Batch,
}

/// 在编译期把两张关键字表接成一张。
///
/// `N` 必须等于两张表的长度之和：小于时越界写入在编译期报错，大于时末尾多出空字符串。
const fn concat_keywords<const N: usize>(
    first: &[&'static str],
    second: &[&'static str],
) -> [&'static str; N] {
    let mut out = [""; N];
    let mut i = 0;
    while i < first.len() {
        out[i] = first[i];
        i += 1;
    }
    let mut j = 0;
    while j < second.len() {
        out[first.len() + j] = second[j];
        j += 1;
    }
    out
}

/// 彼此之间可以引用名字的语言：同一组中，一种语言定义的名字可能在另一种语言的文件中使用。
///
/// 查找用法时只看与当前文件相关的语言，其他语言（文档、数据文件）中同名的文字不算用法。
const RELATED: &[&[&str]] = &[
    &["c", "cpp", "objective-c", "objective-cpp"],
    &["javascript", "typescript", "arkts", "html", "vue", "svelte"],
    &["css", "scss", "sass", "less", "html", "vue", "svelte"],
    &["java", "kotlin", "scala", "groovy"],
    &["shell", "dockerfile", "makefile"],
    &["elixir", "erlang"],
    &["csharp", "fsharp"],
];

impl Language {
    /// 是否与 `other` 是同一种语言，或者同属一组相关的语言（`RELATED`）。
    pub fn is_related(&self, other: &Language) -> bool {
        self.id == other.id
            || RELATED
                .iter()
                .any(|group| group.contains(&self.id) && group.contains(&other.id))
    }

    /// `c` 是否可以作为名字中的字符（字母、数字、`_` 和 [`Language::name_chars`]）。
    pub fn is_name_char(&self, c: char) -> bool {
        c.is_alphanumeric() || c == '_' || self.name_chars.contains(&c)
    }

    /// `word` 是否是一个名字。
    ///
    /// 名字以可选的前缀字符开头，主体是字母、`_` 或 [`Language::name_chars`]，末尾可以有一个后缀
    /// 字符。
    pub fn is_name(&self, word: &str) -> bool {
        let body = word.trim_start_matches(|c| self.name_sigils.contains(&c));
        let body = match body.chars().last() {
            Some(c) if self.name_suffixes.contains(&c) => &body[..body.len() - c.len_utf8()],
            _ => body,
        };
        let mut chars = body.chars();
        chars
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || self.name_chars.contains(&c))
            && chars.all(|c| self.is_name_char(c))
    }

    /// 返回 `chars` 中第 `col` 个字符处（或紧挨在它之前）的名字的字符范围。
    ///
    /// 该位置没有名字时返回 `None`。
    ///
    /// 范围包含开头的前缀字符（[`Language::name_sigils`]）和末尾的后缀字符（见
    /// [`Language::is_name`]）。
    pub fn name_range(&self, chars: &[char], col: usize) -> Option<std::ops::Range<usize>> {
        let inside = |k: usize| chars.get(k).is_some_and(|&c| self.is_name_char(c));
        let at = if inside(col) {
            col
        } else if col > 0 && inside(col - 1) {
            col - 1
        } else {
            return None;
        };
        let mut start = at;
        while start > 0 && self.is_name_char(chars[start - 1]) {
            start -= 1;
        }
        while start > 0 && self.name_sigils.contains(&chars[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < chars.len() && self.is_name_char(chars[end]) {
            end += 1;
        }
        if chars
            .get(end)
            .is_some_and(|c| self.name_suffixes.contains(c))
        {
            end += 1;
        }
        let word: String = chars[start..end].iter().collect();
        self.is_name(&word).then_some(start..end)
    }

    /// 返回切换注释时插入的注释前缀；语言没有行注释时返回 `None`。
    ///
    /// 行注释符之后加一个空格（`# `、`; `、`:: `）；已经以空格结尾的（批处理的 `REM `）原样返回。
    pub fn comment_prefix(&self) -> Option<&'static str> {
        self.line_comments.first().map(|&c| match c {
            "#" => "# ",
            "//" => "// ",
            "--" => "-- ",
            ";" => "; ",
            "%" => "% ",
            "::" => ":: ",
            other => other,
        })
    }
}

/// 各语言共用的默认规则：全部关闭，各语言用 `..BASE` 覆盖需要的字段。
const BASE: Language = Language {
    id: "text",
    name: "Plain Text",
    extensions: &[],
    file_names: &[],
    name_prefixes: &[],
    interpreters: &[],
    keywords: &[],
    builtins: &[],
    ignore_case: false,
    line_comments: &[],
    comment_needs_space: false,
    block_comments: &[],
    nested_comments: false,
    line_start_comments: false,
    triple_quotes: false,
    multiline_strings: false,
    double_quote_string: false,
    single_quote_string: false,
    single_quote_escapes: true,
    escape_char: '\\',
    doubled_quotes: false,
    apostrophes: false,
    backtick_string: false,
    backtick_escapes: false,
    template_interpolation: false,
    interpolation: None,
    raw_prefixes: &[],
    block_strings: &[],
    long_brackets: None,
    line_string: None,
    regex_literals: false,
    markdown: false,
    code_tokens: false,
    surround: &[],
    raw_strings: RawStrings::None,
    code_escapes: false,
    variables: Variables::None,
    heredocs: Heredocs::None,
    decorators: false,
    hash_attributes: false,
    hash_lines: false,
    diff_lines: false,
    key_separators: &[],
    section_headers: false,
    quoted_keys: false,
    quoted_identifiers: false,
    markup: Markup::None,
    stylesheet: false,
    colon_symbols: false,
    percent_literals: false,
    name_chars: &[],
    name_sigils: &[],
    name_suffixes: &[],
    block_scalars: false,
    prose_quotes: false,
    backslash_commands: false,
    capitalized_types: false,
    macros: false,
    indent_after: &[],
    tag_indent: false,
    indent_unit: "    ",
};

/// 代码和配置格式共用的默认规则：在 [`BASE`] 之上识别双引号字符串、数字、字符字面量和函数名。
///
/// 正文为主的格式（纯文本、Markdown、表格、忽略规则、差异文件）直接沿用 [`BASE`]。
const CODE: Language = Language {
    double_quote_string: true,
    code_tokens: true,
    ..BASE
};

/// C 风格的块注释：以 `/` 加 `*` 开始，以 `*` 加 `/` 结束。
const C_COMMENTS: &[(&str, &str)] = &[("/*", "*/")];
/// 回车时触发多缩进一级的行末括号字符。
const BRACKETS: &[char] = &['{', '(', '['];

/// 纯文本的规则：没有任何高亮规则，识别不出语言时使用。
static PLAIN: Language = BASE;

/// 全部支持高亮的语言（不含纯文本）；顺序决定识别时的先后。
static ALL: &[&Language] = &[
    &PYTHON,
    &RUST,
    &TYPESCRIPT,
    &JAVASCRIPT,
    &ARKTS,
    &GO,
    &C,
    &CPP,
    &OBJECTIVE_C,
    &OBJECTIVE_CPP,
    &JAVA,
    &KOTLIN,
    &SWIFT,
    &SCALA,
    &GROOVY,
    &CSHARP,
    &PHP,
    &RUBY,
    &LUA,
    &PERL,
    &R,
    &DART,
    &ZIG,
    &HASKELL,
    &ELIXIR,
    &ERLANG,
    &JULIA,
    &CLOJURE,
    &OCAML,
    &FSHARP,
    &SHELL,
    &FISH,
    &POWERSHELL,
    &BATCH,
    &DOCKERFILE,
    &MAKEFILE,
    &CMAKE,
    &HCL,
    &PROTOBUF,
    &GRAPHQL,
    &NIX,
    &TOML,
    &YAML,
    &INI,
    &PROPERTIES,
    &DOTENV,
    &IGNORE,
    &JSON,
    &HTML,
    &XML,
    &SVG,
    &VUE,
    &SVELTE,
    &CSS,
    &SCSS,
    &SASS,
    &LESS,
    &SQL,
    &MARKDOWN,
    &LATEX,
    &DIFF,
    &CSV,
    &TSV,
];

/// 返回支持高亮的全部语言（不含纯文本），顺序与识别时一致。
pub fn languages() -> &'static [&'static Language] {
    ALL
}

/// 按 id 或状态栏上显示的名字查找语言，不区分大小写（例如 `json`、`JSON`、`c++`）。
///
/// 纯文本不在查找范围内；找不到时返回 `None`。
pub fn find(key: &str) -> Option<&'static Language> {
    ALL.iter()
        .find(|l| l.id.eq_ignore_ascii_case(key) || l.name.eq_ignore_ascii_case(key))
        .copied()
}

/// 返回 HTML 中内嵌代码使用的语言：`script` 为真时是 `<script>` 的内容（TypeScript，兼容
/// JavaScript），为假时是 `<style>` 的内容（SCSS，兼容 CSS）。
pub(crate) fn embedded(script: bool) -> &'static Language {
    if script { &TYPESCRIPT } else { &SCSS }
}

/// 代码块信息字符串中常用、但既不是语言 id 也不是扩展名的写法，以及对应的语言 id。
const FENCE_ALIASES: &[(&str, &str)] = &[
    ("make", "makefile"),
    ("objc", "objective-c"),
    ("golang", "go"),
    ("docker", "dockerfile"),
    ("pwsh", "powershell"),
    ("terraform", "hcl"),
    ("console", "shell"),
];

/// 返回 Markdown 代码块信息字符串（```` ```rust ````、```` ```js {1,3} ````）指明的语言在
/// [`languages`] 中的下标；认不出时返回 `None`。
///
/// 取信息字符串的第一个单词，依次按语言 id 或名字、[`FENCE_ALIASES`] 中的常用写法、扩展名查找。
pub(crate) fn fence_language(info: &str) -> Option<u8> {
    let word = info
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '{' | '}'))
        .find(|w| !w.is_empty())?
        .trim_start_matches('.');
    let language = find(word)
        .or_else(|| {
            FENCE_ALIASES
                .iter()
                .find(|(alias, _)| alias.eq_ignore_ascii_case(word))
                .and_then(|(_, id)| find(id))
        })
        .or_else(|| {
            let language = detect(&format!("x.{word}"));
            (!std::ptr::eq(language, &PLAIN)).then_some(language)
        })?;
    let index = ALL.iter().position(|l| std::ptr::eq(*l, language))?;
    u8::try_from(index).ok()
}

/// 返回 [`languages`] 中下标为 `index` 的语言；超出范围时返回 `None`。
pub(crate) fn language_at(index: u8) -> Option<&'static Language> {
    ALL.get(usize::from(index)).copied()
}

/// 按文件名识别语言，依次匹配完整文件名、文件名开头和扩展名（不区分大小写）。
///
/// 都认不出时返回纯文本的规则。
pub fn detect(file_name: &str) -> &'static Language {
    if let Some(language) = ALL.iter().find(|l| l.file_names.contains(&file_name)) {
        return language;
    }

    if let Some(language) = ALL
        .iter()
        .find(|l| l.name_prefixes.iter().any(|p| file_name.starts_with(p)))
    {
        return language;
    }

    let extension = file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());

    if let Some(extension) = extension
        && let Some(language) = ALL
            .iter()
            .find(|l| l.extensions.contains(&extension.as_str()))
    {
        return language;
    }

    &PLAIN
}

/// 按文件名识别语言，认不出时再按首行 `first_line` 的 `#!` 行识别解释器（见
/// [`Language::interpreters`]）。
pub fn detect_with_first_line(file_name: &str, first_line: &str) -> &'static Language {
    let language = detect(file_name);
    if !std::ptr::eq(language, &PLAIN) {
        return language;
    }
    shebang(first_line).unwrap_or(language)
}

/// 按 `#!` 行识别语言；行首不是 `#!` 或解释器不认识时返回 `None`。
fn shebang(line: &str) -> Option<&'static Language> {
    let mut words = line.strip_prefix("#!")?.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    // `#!/usr/bin/env -S python3 -u`：`env` 之后第一个不是选项的参数才是解释器。
    if program == "env" {
        program = words.find(|w| !w.starts_with('-'))?;
    }
    let name = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    ALL.iter().find(|l| l.interpreters.contains(&name)).copied()
}

#[cfg(test)]
mod tests;
