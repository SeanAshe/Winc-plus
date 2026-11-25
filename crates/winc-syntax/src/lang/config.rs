//! 系统脚本、构建与接口描述格式的规则：PowerShell、批处理、`CMake`、HCL（Terraform）、
//! Protocol Buffers、GraphQL、Nix。

use super::*;

/// 支持 `${…}` 插值、只在双引号字符串中的写法（HCL、Nix）。
const DOLLAR_BRACE: Interpolation = Interpolation {
    open: "${",
    close: '}',
    prefixes: &[],
    single_quotes: false,
    doubled: true,
};

/// PowerShell 的规则。
///
/// 依据 PowerShell 语言规范：关键字不区分大小写；转义字符是反引号；`$name` 是变量，双引号字符串
/// 中展开变量和 `$(…)` 子表达式，单引号字符串中 `''` 表示一个单引号；`@"…"@`、`@'…'@` 是 here
/// 字符串；`<# … #>` 是块注释。
pub(super) static POWERSHELL: Language = Language {
    id: "powershell",
    name: "PowerShell",
    name_chars: &['-'],
    name_sigils: &['$'],
    extensions: &["ps1", "psm1", "psd1"],
    interpreters: &["pwsh", "powershell"],
    keywords: &[
        "begin",
        "break",
        "catch",
        "class",
        "continue",
        "data",
        "define",
        "do",
        "dynamicparam",
        "else",
        "elseif",
        "end",
        "enum",
        "exit",
        "filter",
        "finally",
        "for",
        "foreach",
        "from",
        "function",
        "hidden",
        "if",
        "in",
        "param",
        "process",
        "return",
        "static",
        "switch",
        "throw",
        "trap",
        "try",
        "until",
        "using",
        "var",
        "while",
        "workflow",
    ],
    ignore_case: true,
    line_comments: &["#"],
    block_comments: &[("<#", "#>")],
    single_quote_string: true,
    single_quote_escapes: false,
    escape_char: '`',
    doubled_quotes: true,
    code_escapes: true,
    interpolation: Some(Interpolation {
        open: "$(",
        close: ')',
        prefixes: &[],
        single_quotes: false,
        doubled: false,
    }),
    block_strings: &[
        BlockString {
            open: "@\"",
            close: "\"@",
            interpolate: true,
            escapes: BlockEscapes::None,
        },
        BlockString {
            open: "@'",
            close: "'@",
            interpolate: false,
            escapes: BlockEscapes::None,
        },
    ],
    variables: Variables::Dollar,
    indent_after: BRACKETS,
    ..CODE
};

/// Windows 批处理的规则。
///
/// 命令不区分大小写；`REM` 和 `:` 开始注释；转义字符是 `^`；变量写作 `%name%`、`!name!` 等形式，
/// 包括 `%1` 和 `%%i`。
pub(super) static BATCH: Language = Language {
    id: "batch",
    name: "Batch",
    extensions: &["bat", "cmd"],
    keywords: &[
        "if",
        "else",
        "for",
        "in",
        "do",
        "goto",
        "call",
        "exit",
        "set",
        "setlocal",
        "endlocal",
        "not",
        "exist",
        "defined",
        "errorlevel",
        "equ",
        "neq",
        "lss",
        "leq",
        "gtr",
        "geq",
        "shift",
        "pause",
    ],
    builtins: &[
        "echo", "cd", "chdir", "pushd", "popd", "cls", "copy", "xcopy", "del", "erase", "dir",
        "mkdir", "md", "rd", "rmdir", "move", "ren", "rename", "type", "start", "title", "color",
        "choice", "timeout", "where",
    ],
    ignore_case: true,
    line_comments: &["::", "REM ", "rem ", "Rem ", "@REM ", "@rem "],
    comment_needs_space: true,
    escape_char: '^',
    code_escapes: true,
    variables: Variables::Batch,
    ..CODE
};

/// `CMake` 的规则。
///
/// 命令名不区分大小写；`[[…]]` 形式的括号参数与引号参数可以跨行，`#[[ … ]]` 是括号注释；变量是
/// `${VAR}` 和 `$ENV{VAR}`。
pub(super) static CMAKE: Language = Language {
    id: "cmake",
    name: "CMake",
    extensions: &["cmake"],
    file_names: &["CMakeLists.txt", "CMakeCache.txt"],
    keywords: &[
        "if",
        "elseif",
        "else",
        "endif",
        "foreach",
        "endforeach",
        "while",
        "endwhile",
        "function",
        "endfunction",
        "macro",
        "endmacro",
        "return",
        "break",
        "continue",
        "block",
        "endblock",
        "and",
        "or",
        "not",
        "defined",
        "in",
        "matches",
        "strequal",
        "equal",
        "less",
        "greater",
        "version_less",
        "version_greater",
        "version_equal",
    ],
    builtins: &["true", "false", "on", "off", "yes", "no", "notfound"],
    ignore_case: true,
    line_comments: &["#"],
    multiline_strings: true,
    // 不带引号的参数中 `\"` 是转义；`[[…]]`、`[=[…]=]` 是括号参数，`#[[…]]` 是括号注释。
    code_escapes: true,
    long_brackets: Some("#"),
    variables: Variables::Dollar,
    indent_after: &['('],
    indent_unit: "  ",
    ..CODE
};

/// HCL（Terraform、Packer、Nomad）的规则：`#`、`//` 和 `/* */` 注释，`"${…}"` 插值（`$${` 是
/// 字面的 `${`），`<<EOT` here 文档，行首的 `name =` 是属性。
pub(super) static HCL: Language = Language {
    id: "hcl",
    name: "HCL",
    extensions: &["tf", "tfvars", "hcl", "nomad", "tftest"],
    file_names: &[".terraformrc", "terraform.rc"],
    keywords: &[
        "resource",
        "data",
        "variable",
        "output",
        "module",
        "provider",
        "locals",
        "terraform",
        "moved",
        "import",
        "check",
        "for",
        "in",
        "if",
        "else",
        "endif",
        "endfor",
        "dynamic",
    ],
    builtins: &[
        "true", "false", "null", "string", "number", "bool", "list", "map", "set", "object",
        "tuple", "any",
    ],
    line_comments: &["#", "//"],
    block_comments: C_COMMENTS,
    interpolation: Some(DOLLAR_BRACE),
    heredocs: Heredocs::Compact,
    key_separators: &['='],
    indent_after: BRACKETS,
    indent_unit: "  ",
    ..CODE
};

/// Protocol Buffers 的规则。
pub(super) static PROTOBUF: Language = Language {
    id: "protobuf",
    name: "Protocol Buffers",
    extensions: &["proto"],
    keywords: &[
        "syntax",
        "edition",
        "package",
        "import",
        "public",
        "weak",
        "option",
        "message",
        "enum",
        "service",
        "rpc",
        "returns",
        "stream",
        "repeated",
        "optional",
        "required",
        "oneof",
        "map",
        "reserved",
        "extend",
        "extensions",
        "to",
        "max",
        "group",
    ],
    builtins: &[
        "double", "float", "int32", "int64", "uint32", "uint64", "sint32", "sint64", "fixed32",
        "fixed64", "sfixed32", "sfixed64", "bool", "string", "bytes", "true", "false",
    ],
    line_comments: &["//"],
    block_comments: C_COMMENTS,
    single_quote_string: true,
    capitalized_types: true,
    indent_after: BRACKETS,
    indent_unit: "  ",
    ..CODE
};

/// GraphQL 的规则：`#` 开始注释，`"""` 是块字符串，`$name` 是变量，`@include` 这类指令按装饰器
/// 着色成 [`crate::TokenKind::Decorator`]。
pub(super) static GRAPHQL: Language = Language {
    id: "graphql",
    name: "GraphQL",
    extensions: &["graphql", "gql", "graphqls"],
    keywords: &[
        "query",
        "mutation",
        "subscription",
        "fragment",
        "on",
        "type",
        "interface",
        "union",
        "enum",
        "input",
        "scalar",
        "schema",
        "extend",
        "directive",
        "implements",
        "repeatable",
    ],
    builtins: &[
        "true", "false", "null", "Int", "Float", "String", "Boolean", "ID",
    ],
    line_comments: &["#"],
    triple_quotes: true,
    variables: Variables::Dollar,
    decorators: true,
    capitalized_types: true,
    indent_after: BRACKETS,
    indent_unit: "  ",
    ..CODE
};

/// Nix 的规则：`#` 和 `/* */` 注释，`"…"` 与 `''…''`（缩进字符串）都可以跨行，其中的 `${…}`
/// 是插值，`$$` 是两个字面的 `$`。
///
/// 依据 Nix 语言参考：缩进字符串以 `'''`、`''$`、`''\` 转义；名字中可以有 `'` 和 `-`。
pub(super) static NIX: Language = Language {
    id: "nix",
    name: "Nix",
    extensions: &["nix"],
    keywords: &[
        "let", "in", "with", "rec", "inherit", "if", "then", "else", "assert", "or",
    ],
    builtins: &[
        "true",
        "false",
        "null",
        "import",
        "builtins",
        "throw",
        "abort",
        "map",
        "toString",
        "derivation",
    ],
    line_comments: &["#"],
    block_comments: C_COMMENTS,
    multiline_strings: true,
    interpolation: Some(DOLLAR_BRACE),
    multiline_interpolation: true,
    path_literals: true,
    block_strings: &[BlockString {
        open: "''",
        close: "''",
        interpolate: true,
        escapes: BlockEscapes::Nix,
    }],
    name_chars: &['\'', '-'],
    indent_after: BRACKETS,
    indent_unit: "  ",
    ..CODE
};
