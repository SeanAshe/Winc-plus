//! Shell、构建文件、配置格式、标记语言、样式表、SQL 和数据格式的规则。

use super::*;

/// Shell 的关键字（保留字，以及 `local`、`export`、`declare` 这类声明变量的内置命令）。
///
/// Dockerfile 的 `RUN` 行和 Makefile 的命令行也是 shell 语句，两者的关键字以这张表为基础。
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "select", "function", "return", "break", "continue", "local", "export", "declare",
    "readonly", "typeset", "time", "coproc",
];

/// Dockerfile 的指令，只属于 Dockerfile，Shell 和 Makefile 不使用这些指令。
///
/// `AS` 是 `FROM image AS name` 中给构建阶段命名的关键字。
const DOCKERFILE_DIRECTIVES: &[&str] = &[
    "FROM",
    "AS",
    "RUN",
    "COPY",
    "ADD",
    "CMD",
    "ENTRYPOINT",
    "ENV",
    "ARG",
    "WORKDIR",
    "USER",
    "EXPOSE",
    "VOLUME",
    "LABEL",
    "HEALTHCHECK",
    "SHELL",
    "ONBUILD",
    "STOPSIGNAL",
    "MAINTAINER",
];

/// Makefile 的指令（GNU make），只属于 Makefile。
///
/// `-include` 开头的 `-` 不是标识符字符，行首的 `-include` 按其中的 `include` 着色。
const MAKE_DIRECTIVES: &[&str] = &[
    "ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "sinclude", "define", "endef",
    "export", "unexport", "override", "undefine", "private", "vpath",
];

/// Dockerfile 的关键字：Shell 关键字加上 Dockerfile 指令。
static DOCKERFILE_KEYWORDS: [&str; SHELL_KEYWORDS.len() + DOCKERFILE_DIRECTIVES.len()] =
    concat_keywords(SHELL_KEYWORDS, DOCKERFILE_DIRECTIVES);

/// Makefile 的关键字：Shell 关键字加上 Makefile 指令。
static MAKE_KEYWORDS: [&str; SHELL_KEYWORDS.len() + MAKE_DIRECTIVES.len()] =
    concat_keywords(SHELL_KEYWORDS, MAKE_DIRECTIVES);

/// Shell 的规则。
pub(super) static SHELL: Language = Language {
    id: "shell",
    name: "Shell",
    extensions: &["sh", "bash", "zsh", "ksh", "ash"],
    file_names: &[
        ".bashrc",
        ".bash_profile",
        ".bash_aliases",
        ".bash_logout",
        ".zshrc",
        ".zprofile",
        ".zshenv",
        ".zlogin",
        ".zlogout",
        ".profile",
        ".envrc",
        "PKGBUILD",
        "APKBUILD",
    ],
    interpreters: &["sh", "bash", "zsh", "ksh", "dash", "ash", "mksh"],
    keywords: SHELL_KEYWORDS,
    builtins: &[
        "echo", "printf", "read", "cd", "pwd", "set", "unset", "source", "eval", "exec", "exit",
        "shift", "trap", "test", "alias", "unalias", "wait", "getopts", "umask", "ulimit", "type",
        "hash", "let", "true", "false",
    ],
    line_comments: &["#"],
    comment_needs_space: true,
    // 单双引号字符串都可以跨行（`awk '…'` 中的多行脚本）。
    multiline_strings: true,
    single_quote_string: true,
    // POSIX：单引号内的字符全部按字面处理，反斜杠不转义，`'C:\dir\'` 在第二个单引号处结束。
    single_quote_escapes: false,
    backtick_string: true,
    backtick_escapes: true,
    code_escapes: true,
    variables: Variables::Shell,
    // 双引号字符串中的命令替换按代码着色，其中嵌套的引号不会结束外层的字符串
    // （`"$(cat "$f")"`）。
    interpolation: Some(Interpolation {
        open: "$(",
        close: ')',
        prefixes: &[],
        single_quotes: false,
    }),
    heredocs: Heredocs::Shell,
    indent_after: &['{'],
    ..CODE
};

/// fish 的规则，其余字段沿用 Shell。
///
/// fish 不兼容 POSIX：语句以 `end` 结束，没有 here 文档；单引号内 `\'` 和 `\\` 是转义。
pub(super) static FISH: Language = Language {
    id: "fish",
    name: "Fish",
    extensions: &["fish"],
    file_names: &[],
    interpreters: &["fish"],
    keywords: &[
        "if", "else", "end", "for", "in", "while", "break", "continue", "function", "return",
        "switch", "case", "begin", "and", "or", "not",
    ],
    builtins: &[
        "set",
        "echo",
        "printf",
        "read",
        "cd",
        "pwd",
        "source",
        "exit",
        "test",
        "string",
        "math",
        "count",
        "contains",
        "status",
        "functions",
        "abbr",
        "alias",
        "type",
        "command",
        "builtin",
        "set_color",
        "true",
        "false",
    ],
    single_quote_escapes: true,
    heredocs: Heredocs::None,
    ..SHELL
};

/// Dockerfile 的规则，其余字段沿用 Shell。
pub(super) static DOCKERFILE: Language = Language {
    id: "dockerfile",
    name: "Dockerfile",
    extensions: &["dockerfile"],
    file_names: &["Dockerfile", "Containerfile"],
    name_prefixes: &["Dockerfile.", "Containerfile."],
    interpreters: &[],
    keywords: &DOCKERFILE_KEYWORDS,
    ..SHELL
};

/// Makefile 的规则，其余字段沿用 Shell。
pub(super) static MAKEFILE: Language = Language {
    id: "makefile",
    name: "Makefile",
    name_chars: &['-'],
    extensions: &["mk"],
    file_names: &["Makefile", "makefile", "GNUmakefile"],
    name_prefixes: &["Makefile."],
    interpreters: &["make"],
    keywords: &MAKE_KEYWORDS,
    // 每个命令行由单独的 shell 执行，引号和 here 文档不会延续到下一行。
    multiline_strings: false,
    heredocs: Heredocs::None,
    variables: Variables::Make,
    // `"$(CC)"` 中的 `$(…)` 是 make 的变量引用，不是命令替换。
    interpolation: None,
    // 行首的 `目标: 依赖` 中的目标按键着色。冒号后面必须是空白或行尾，`A := 1` 中的 `A` 不算。
    key_separators: &[':'],
    // 规则下的命令行必须以制表符开头，以空格开头时 make 报「missing separator」。
    indent_unit: "\t",
    ..SHELL
};

/// TOML 的规则。
///
/// 依据 TOML 1.0：布尔值只有 `true`、`false`，特殊浮点数是 `inf`、`nan`；单引号括起不支持转义的
/// 字面字符串；值中紧跟在字母、数字之后的单引号是撇号（`msg = it's`），不开始字符串。
pub(super) static TOML: Language = Language {
    id: "toml",
    name: "TOML",
    extensions: &["toml"],
    file_names: &[
        "Cargo.lock",
        "Pipfile",
        "poetry.lock",
        "uv.lock",
        "pdm.lock",
    ],
    builtins: &["true", "false", "inf", "nan"],
    line_comments: &["#"],
    comment_needs_space: true,
    triple_quotes: true,
    single_quote_string: true,
    single_quote_escapes: false,
    apostrophes: true,
    key_separators: &['='],
    section_headers: true,
    indent_after: &['[', '{'],
    indent_unit: "  ",
    ..CODE
};

/// YAML 的规则。
///
/// 布尔值和空值包括 YAML 1.2 核心模式的大小写写法，以及 YAML 1.1 的 `yes`、`no`、`on`、`off`
/// （Ansible、Kubernetes 等工具仍按 1.1 解析）。单引号字符串没有反斜杠转义，`''` 表示一个单引号。
pub(super) static YAML: Language = Language {
    id: "yaml",
    name: "YAML",
    extensions: &["yaml", "yml"],
    file_names: &["yarn.lock", ".clang-format", ".clang-tidy", ".yamllint"],
    builtins: &[
        "true", "True", "TRUE", "false", "False", "FALSE", "null", "Null", "NULL", "yes", "Yes",
        "YES", "no", "No", "NO", "on", "On", "ON", "off", "Off", "OFF",
    ],
    line_comments: &["#"],
    comment_needs_space: true,
    single_quote_string: true,
    single_quote_escapes: false,
    apostrophes: true,
    key_separators: &[':'],
    block_scalars: true,
    indent_after: &[':'],
    indent_unit: "  ",
    ..CODE
};

/// INI 的规则，其余字段沿用 TOML。
pub(super) static INI: Language = Language {
    id: "ini",
    name: "INI",
    extensions: &["ini", "cfg", "conf", "desktop", "service", "inf"],
    file_names: &[
        ".editorconfig",
        ".gitconfig",
        ".gitmodules",
        ".npmrc",
        ".pylintrc",
        ".flake8",
        ".coveragerc",
    ],
    builtins: &["true", "false", "yes", "no", "on", "off"],
    // `;` 是 INI 的传统注释符，`.editorconfig`、`.gitconfig`、`.npmrc` 都同时接受 `#` 和 `;`。
    line_comments: &["#", ";"],
    triple_quotes: false,
    indent_after: &[],
    ..TOML
};

/// Properties 的规则，其余字段沿用 TOML。
///
/// 依据 `java.util.Properties`：注释以 `#` 或 `!` 开头，键与值之间是 `=` 或 `:`。
pub(super) static PROPERTIES: Language = Language {
    id: "properties",
    name: "Properties",
    extensions: &["properties"],
    builtins: &["true", "false"],
    line_comments: &["#", "!"],
    key_separators: &['=', ':'],
    triple_quotes: false,
    section_headers: false,
    indent_after: &[],
    ..TOML
};

/// Dotenv 的规则，其余字段沿用 TOML。
pub(super) static DOTENV: Language = Language {
    id: "dotenv",
    name: "Dotenv",
    extensions: &["env"],
    name_prefixes: &[".env."],
    builtins: &["true", "false"],
    triple_quotes: false,
    section_headers: false,
    indent_after: &[],
    variables: Variables::Dollar,
    ..TOML
};

/// 忽略规则文件的规则：每行是一个路径模式，只有 `#` 注释。
///
/// 不沿用 TOML 的单引号字符串和内置名，否则模式里的撇号会把行内剩下的部分都当作字符串。
///
/// `.gitattributes` 和 `CODEOWNERS` 每行同样以路径模式开头，规则相同。
pub(super) static IGNORE: Language = Language {
    id: "ignore",
    name: "Ignore",
    file_names: &[
        ".gitignore",
        ".dockerignore",
        ".npmignore",
        ".prettierignore",
        ".eslintignore",
        ".stylelintignore",
        ".hgignore",
        ".containerignore",
        ".gitattributes",
        "CODEOWNERS",
    ],
    line_comments: &["#"],
    comment_needs_space: true,
    ..BASE
};

/// JSON、JSONC 与 JSON5 的规则。
///
/// 后面紧跟 `:` 的字符串是键，不限于行首（`[{"a": 1}]`）。
pub(super) static JSON: Language = Language {
    id: "json",
    name: "JSON",
    extensions: &[
        "json",
        "jsonc",
        "json5",
        "jsonl",
        "ndjson",
        "geojson",
        "webmanifest",
        "code-workspace",
        "winc-project",
        "sublime-project",
    ],
    file_names: &[
        ".babelrc",
        ".eslintrc",
        ".prettierrc",
        ".jshintrc",
        ".swcrc",
        "composer.lock",
        "flake.lock",
    ],
    builtins: &["true", "false", "null"],
    line_comments: &["//"],
    block_comments: C_COMMENTS,
    key_separators: &[':'],
    quoted_keys: true,
    indent_after: &['{', '['],
    indent_unit: "  ",
    ..CODE
};

/// HTML 的规则：标签、属性与字符实体，`<script>`、`<style>` 的内容按内嵌语言着色。
pub(super) static HTML: Language = Language {
    id: "html",
    name: "HTML",
    // 自定义元素（`my-widget`）、`data-` 属性和 id 中的 `-`。
    name_chars: &['-'],
    extensions: &["html", "htm", "xhtml"],
    block_comments: &[("<!--", "-->")],
    block_strings: &[BlockString {
        open: "<![CDATA[",
        close: "]]>",
        interpolate: false,
    }],
    markup: Markup::Html,
    // 几乎每行都以 `>` 结尾，不能按行末字符多缩进，只在开始标签之后多缩进；`{` 和未闭合的括号
    // 用于内嵌的脚本与样式（包括 Vue 的 `<script>`）。
    indent_after: &['{'],
    tag_indent: true,
    indent_unit: "  ",
    ..BASE
};

/// XML 的规则，其余字段沿用 HTML：没有内嵌的脚本与样式。
pub(super) static XML: Language = Language {
    id: "xml",
    name: "XML",
    extensions: &[
        "xml",
        "plist",
        "xsd",
        "xsl",
        "xslt",
        "xaml",
        "csproj",
        "fsproj",
        "vbproj",
        "vcxproj",
        "props",
        "targets",
        "nuspec",
        "resx",
        "storyboard",
        "xib",
        "xcscheme",
        "wsdl",
        "rss",
        "atom",
        "kml",
    ],
    markup: Markup::Xml,
    ..HTML
};

/// SVG 的规则，其余字段沿用 HTML：SVG 中同样可以内嵌 `<script>` 和 `<style>`。
pub(super) static SVG: Language = Language {
    id: "svg",
    name: "SVG",
    extensions: &["svg"],
    ..HTML
};

/// Vue 单文件组件的规则，其余字段沿用 HTML。
pub(super) static VUE: Language = Language {
    id: "vue",
    name: "Vue",
    extensions: &["vue"],
    ..HTML
};

/// Svelte 组件的规则，其余字段沿用 HTML；属性中的 `{…}` 是表达式。
pub(super) static SVELTE: Language = Language {
    id: "svelte",
    name: "Svelte",
    extensions: &["svelte"],
    ..HTML
};

/// 微信小程序的 WXML 的规则，其余字段沿用 HTML；`<wxs>` 中的 WXS 按脚本着色。
pub(super) static WXML: Language = Language {
    id: "wxml",
    name: "WXML",
    extensions: &["wxml"],
    script_tags: &["script", "wxs"],
    ..HTML
};

/// CSS 的规则。
///
/// CSS 没有 `//` 行注释：识别 `//` 会让 `url(http://…)` 中 `//` 之后的部分被当作注释。
pub(super) static CSS: Language = Language {
    id: "css",
    name: "CSS",
    // 类名、属性名和自定义属性（`--main-color`）中的 `-`。
    name_chars: &['-'],
    name_sigils: &['-'],
    extensions: &["css"],
    keywords: &["important", "from", "to", "and", "not", "only", "or"],
    block_comments: C_COMMENTS,
    single_quote_string: true,
    decorators: true,
    key_separators: &[':'],
    stylesheet: true,
    indent_after: &['{'],
    indent_unit: "  ",
    ..CODE
};

/// SCSS 的规则，其余字段沿用 CSS；`$name` 是变量。
pub(super) static SCSS: Language = Language {
    id: "scss",
    name: "SCSS",
    name_sigils: &['-', '$'],
    extensions: &["scss"],
    line_comments: &["//"],
    comment_needs_space: true,
    variables: Variables::Dollar,
    ..CSS
};

/// Sass 的规则，其余字段沿用 SCSS。
pub(super) static SASS: Language = Language {
    id: "sass",
    name: "Sass",
    extensions: &["sass"],
    ..SCSS
};

/// Less 的规则，其余字段沿用 SCSS；`@name` 是变量，按装饰器着色。
pub(super) static LESS: Language = Language {
    id: "less",
    name: "Less",
    name_sigils: &['-', '@'],
    extensions: &["less"],
    variables: Variables::None,
    ..SCSS
};

/// 微信小程序的 WXSS 的规则，其余字段沿用 CSS：在 CSS 之上多了 `rpx` 等写法，不影响着色。
pub(super) static WXSS: Language = Language {
    id: "wxss",
    name: "WXSS",
    extensions: &["wxss"],
    ..CSS
};

/// SQL 的规则。
///
/// 依据 SQL 标准：字符串用单引号，`''` 表示一个单引号，反斜杠不转义；双引号与反引号括起的是
/// 标识符。关键字取 SQL 标准与常见方言共有的部分；`first`、`last`、`row`、`range` 这类常用作列名
/// 的非保留字不列入。
pub(super) static SQL: Language = Language {
    id: "sql",
    name: "SQL",
    extensions: &["sql"],
    keywords: &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "drop",
        "alter",
        "add",
        "column",
        "rename",
        "to",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "full",
        "cross",
        "natural",
        "on",
        "using",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "fetch",
        "and",
        "or",
        "not",
        "null",
        "as",
        "distinct",
        "all",
        "any",
        "some",
        "exists",
        "in",
        "is",
        "like",
        "ilike",
        "between",
        "case",
        "when",
        "then",
        "else",
        "end",
        "union",
        "intersect",
        "except",
        "with",
        "recursive",
        "returning",
        "default",
        "primary",
        "key",
        "foreign",
        "references",
        "unique",
        "check",
        "constraint",
        "index",
        "view",
        "database",
        "schema",
        "if",
        "replace",
        "truncate",
        "grant",
        "revoke",
        "begin",
        "commit",
        "rollback",
        "transaction",
        "savepoint",
        "asc",
        "desc",
        "nulls",
        "over",
        "partition",
        "window",
        "rows",
        "unbounded",
        "preceding",
        "following",
        "lateral",
        "cascade",
        "restrict",
        "trigger",
        "procedure",
        "function",
        "return",
        "returns",
        "declare",
        "do",
        "nothing",
        "conflict",
        "merge",
        "matched",
        "temporary",
        "sequence",
        "top",
        "escape",
        "collate",
        "cast",
        "explain",
        "analyze",
    ],
    builtins: &[
        "true",
        "false",
        "unknown",
        "count",
        "sum",
        "avg",
        "min",
        "max",
        "coalesce",
        "nullif",
        "int",
        "integer",
        "smallint",
        "bigint",
        "tinyint",
        "decimal",
        "numeric",
        "real",
        "float",
        "double",
        "precision",
        "char",
        "varchar",
        "text",
        "boolean",
        "bool",
        "date",
        "time",
        "timestamp",
        "timestamptz",
        "interval",
        "uuid",
        "json",
        "jsonb",
        "blob",
        "bytea",
        "serial",
        "bigserial",
    ],
    ignore_case: true,
    line_comments: &["--"],
    block_comments: C_COMMENTS,
    single_quote_string: true,
    single_quote_escapes: false,
    doubled_quotes: true,
    backtick_string: true,
    quoted_identifiers: true,
    indent_after: &['('],
    ..CODE
};

/// Markdown（GFM）的规则；HTML 注释可以跨行。
pub(super) static MARKDOWN: Language = Language {
    id: "markdown",
    name: "Markdown",
    extensions: &["md", "markdown", "mdown", "mkd", "mkdn"],
    block_comments: &[("<!--", "-->")],
    markdown: true,
    surround: &['*', '_', '~', '$'],
    indent_unit: "  ",
    ..BASE
};

/// LaTeX 的规则：`\name` 是命令，`%` 开始注释（`\%` 是转义），`$…$`、`$$…$$`、`\[…\]`、
/// `\(…\)` 是数学公式，按字符串着色。
pub(super) static LATEX: Language = Language {
    id: "latex",
    name: "LaTeX",
    extensions: &["tex", "ltx", "sty", "cls", "dtx"],
    line_comments: &["%"],
    block_strings: &[
        BlockString {
            open: "$$",
            close: "$$",
            interpolate: false,
        },
        BlockString {
            open: "\\[",
            close: "\\]",
            interpolate: false,
        },
        BlockString {
            open: "\\(",
            close: "\\)",
            interpolate: false,
        },
        BlockString {
            open: "$",
            close: "$",
            interpolate: false,
        },
    ],
    backslash_commands: true,
    surround: &['$'],
    indent_unit: "  ",
    ..BASE
};

/// 差异文件（`diff -u`、`git diff` 的输出）。
pub(super) static DIFF: Language = Language {
    id: "diff",
    name: "Diff",
    extensions: &["diff", "patch"],
    diff_lines: true,
    ..BASE
};

/// 日志文件：只为时间戳、级别、地址等可识别的写法着色，其余文字不着色。
pub(super) static LOG: Language = Language {
    id: "log",
    name: "Log",
    extensions: &["log"],
    log_lines: true,
    ..BASE
};

/// 逗号分隔的表格。
///
/// 不做语法着色；按列着色和对齐由编辑器层处理。
pub(super) static CSV: Language = Language {
    id: "csv",
    name: "CSV",
    extensions: &["csv"],
    ..BASE
};

/// 制表符分隔的表格。
///
/// 不做语法着色；按列对齐由编辑器层处理。
pub(super) static TSV: Language = Language {
    id: "tsv",
    name: "TSV",
    extensions: &["tsv", "tab"],
    ..BASE
};
