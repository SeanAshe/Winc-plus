//! 转到定义的测试：各语言常见定义写法与类别、调用不算定义、注释和字符串中的写法、跨行搜索、
//! 结果排序。

use super::*;

fn finds(name: &str, line: &str) -> Option<usize> {
    Finder::new(name, false, false).unwrap().in_line(line)
}

#[test]
fn definitions_in_common_languages() {
    for (line, col) in [
        ("pub fn run(x: u8) {", 7),
        ("    let mut run = 1;", 12),
        ("struct run;", 7),
        ("def run(self):", 4),
        ("    async def run():", 14),
        ("class run:", 6),
        ("run = 3", 0),
        ("run: int = 3", 0),
        ("func run() {}", 5),
        ("func (s *Server) run() {", 17),
        ("function run(a) {", 9),
        ("export const run = () => 1;", 13),
        ("  run: function () {", 2),
        ("  async run(a, b) {", 8),
        ("  public static int run(String a) {", 20),
        ("static int run(void)", 11),
        ("int *run(int a) {", 5),
        ("#define run 1", 8),
        ("run() {", 0),
        ("int run(int running)", 4),
        ("  async run(a, run) {", 8),
    ] {
        assert_eq!(finds("run", line), Some(col), "{line}");
    }
}

#[test]
fn uses_are_not_definitions() {
    for line in [
        "run();",
        "    return run(x);",
        "x = run(y)",
        "if run == 3:",
        "running = 1",
        "    run(a) + 1",
        "int x = run(3);",
        "self.run()",
        "run()",
    ] {
        assert_eq!(finds("run", line), None, "{line}");
    }
}

#[test]
fn whole_text_is_searched_line_by_line() {
    let python = winc_syntax::detect("a.py");
    let text = Rope::from_str("x = run()\r\ndef run():\r\n    run = 2\r\nrun = 1\r\n");
    let lines = |finder: &Finder| -> Vec<(usize, usize)> {
        finder
            .in_text(&text, python)
            .into_iter()
            .map(|(line, d)| (line, d.col))
            .collect()
    };
    assert_eq!(
        lines(&Finder::new("run", false, false).unwrap()),
        [(1, 4), (3, 0)]
    );
    // 在当前文件内查找（`local` 为 `true`）时，带缩进的赋值也算定义。
    let local = Finder::new("run", true, false).unwrap();
    assert_eq!(lines(&local), [(1, 4), (2, 4), (3, 0)]);
    assert_eq!(local.in_line("    run == 2"), None);
    assert!(Finder::new("a b", false, false).is_none() && Finder::new("", false, false).is_none());
}

#[test]
fn same_language_and_folder_come_first() {
    let dir = Path::new("/p/src");
    let mut paths = vec![
        Path::new("/p/a.py"),
        Path::new("/p/lib/b.rs"),
        Path::new("/p/src/c.rs"),
    ];
    paths.sort_by_key(|p| rank(p, winc_syntax::detect("a.rs"), dir));
    assert_eq!(
        paths,
        [
            Path::new("/p/src/c.rs"),
            Path::new("/p/lib/b.rs"),
            Path::new("/p/a.py")
        ]
    );
}

#[test]
fn definitions_in_more_languages() {
    for (name, line, col) in [
        ("bar", "int Foo::bar(int x) {", 9),
        ("compute", "    int compute(int x) {", 8),
        (
            "cache",
            "    private final Map<String, Integer> cache = new HashMap<>();",
            39,
        ),
        ("Name", "    public string Name { get; set; }", 18),
        ("build", "  void build(BuildContext c) {", 7),
        ("Update", "    void Update()", 9),
        ("count", "final count = 0;", 6),
        ("size", "  readonly size?: number;", 11),
        ("helper", "function M.helper(a)", 11),
        ("count", "local count = 0", 6),
        ("valid?", "  def valid?", 6),
        ("name", "  attr_reader :id, :name", 20),
        ("helper", "  defp helper(x) do", 7),
        ("Repo", "defmodule MyApp.Repo do", 16),
        ("helper", "helper :: Int -> Int", 0),
        ("helper", "helper x y = x + y", 0),
        ("helper", "let rec helper x =", 8),
        ("helper", "sub helper {", 4),
        ("x", "my $x = 1;", 4),
        ("add_tests", "FUNCTION(add_tests name)", 9),
        ("build", "build: deps", 0),
        ("users", "create table if not exists app.users (", 31),
        ("helper", "(defn helper [x]", 6),
        ("$total", "$total = 0;", 0),
    ] {
        assert_eq!(finds(name, line), Some(col), "{line}");
    }
    // 语句不是带返回类型的方法；函数名不匹配变量；`valid` 不是 `valid?`。
    for (name, line) in [
        ("save", "    return save(x) {"),
        ("Foo", "    new Foo(a) {"),
        ("total", "$total = 0;"),
        ("valid", "  def valid?"),
        ("helper", "await helper()"),
        ("helper", "export default helper(config)"),
    ] {
        assert_eq!(finds(name, line), None, "{line}");
    }
}

#[test]
fn definition_kinds() {
    let kind = |name: &str, line: &str| {
        let d = Finder::new(name, true, false)
            .unwrap()
            .definition(line)
            .unwrap();
        (d.kind, d.is_local())
    };
    assert_eq!(kind("run", "fn run() {}"), (Kind::Declaration, false));
    assert_eq!(kind("x", "    let x = 1;"), (Kind::Binding, true));
    assert_eq!(kind("x", "let x = 1;"), (Kind::Binding, false));
    assert_eq!(kind("total", "    total = 1"), (Kind::Assignment, true));
    assert_eq!(kind("x", "    x := compute()"), (Kind::Assignment, true));
    // 方法缩进在类中，但不是局部变量。
    assert_eq!(
        kind("save", "    def save(self):"),
        (Kind::Declaration, false)
    );
}

#[test]
fn definitions_in_comments_and_strings_do_not_count() {
    let rust = winc_syntax::detect("a.rs");
    let text = Rope::from_str(
        "// fn helper\n/*\nfn helper() {}\n*/\nlet s = \"fn helper\";\nfn helper() {}\n",
    );
    let found: Vec<usize> = Finder::new("helper", false, false)
        .unwrap()
        .in_text(&text, rust)
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(found, [5]);
    // 不区分大小写的语言：SQL 的关键字与名字写成大写时同样能找出定义。
    let sql = winc_syntax::detect("a.sql");
    let text = Rope::from_str("CREATE TABLE Users (id int);\n");
    assert_eq!(
        Finder::new("users", false, true)
            .unwrap()
            .in_text(&text, sql)
            .len(),
        1
    );
}

/// 以标点结尾的名字也逐行查找：跨行查找要把整个文件读入内存，也不搜索大文件。
#[test]
fn names_ending_in_punctuation_are_searched_by_line() {
    use crate::editing::search::{Matcher, Options};

    let query = query("valid?", false);
    let options = Options {
        case_sensitive: true,
        whole_word: false,
        regex: true,
    };
    assert!(!Matcher::new(&query.text, options).unwrap().is_multiline());
    assert_eq!(finds("valid?", "def valid?"), Some(4));
    assert_eq!(finds("valid?", "def valid? x"), Some(4));
    assert_eq!(finds("valid?", "def valid?x"), None);
}

/// 嵌入式 C 工程的常见声明：枚举成员、`typedef` 类型、结构体成员、函数指针、全局变量、形参与
/// 局部变量。
const EMBEDDED_C: &str = "\
typedef enum
{
    STATE_IDLE = 0,
    STATE_RUN,          // 运行
    STATE_MAX
} app_state_t;

typedef struct {
    uint8_t  buf[64];
    uint16_t len;
    void (*on_rx)(uint8_t byte);
    unsigned ready : 1;
} uart_ctx_t;

typedef void (*uart_cb_t)(uint8_t);
typedef unsigned char u8;
uint32_t g_tick;
extern volatile uint32_t g_ticks;
uint8_t *p_buf;
char **pp;
static uart_ctx_t s_ctx;
static const char *names[] = { \"a\" };
struct node *next;

static void uart_send(const uint8_t *data, uint16_t size)
{
    uart_ctx_t cfg = {0};
    for (int k = 0; k < size; k++) {
        cfg.buf[k] = data[k];
    }
    if (cfg.len > STATE_RUN) { }
    configure(
        STATE_IDLE,
        size
    );
    return cfg;
}
";

/// 返回 `name` 在 [`EMBEDDED_C`] 中各处定义的（行号，列号，类别）；`local` 见 [`Finder::new`]。
fn c_definitions(name: &str, local: bool) -> Vec<(usize, usize, Kind)> {
    let text = Rope::from_str(EMBEDDED_C);
    let finder = Finder::new(name, local, false).unwrap();
    finder
        .in_text(&text, winc_syntax::detect("a.c"))
        .into_iter()
        .map(|(line, d)| (line, d.col, d.kind))
        .collect()
}

#[test]
fn c_declarations_are_definitions() {
    use Kind::{Binding, Declaration, Parameter};
    for (name, line, col, kind) in [
        ("STATE_IDLE", 2, 4, Declaration),
        ("STATE_RUN", 3, 4, Declaration),
        ("STATE_MAX", 4, 4, Declaration),
        ("app_state_t", 5, 2, Declaration),
        ("buf", 8, 13, Declaration),
        ("len", 9, 13, Declaration),
        ("on_rx", 10, 11, Declaration),
        ("ready", 11, 13, Declaration),
        ("uart_ctx_t", 12, 2, Declaration),
        ("uart_cb_t", 14, 15, Declaration),
        ("u8", 15, 22, Declaration),
        ("g_tick", 16, 9, Declaration),
        ("g_ticks", 17, 25, Declaration),
        ("p_buf", 18, 9, Declaration),
        ("pp", 19, 7, Declaration),
        ("s_ctx", 20, 18, Declaration),
        ("names", 21, 19, Binding),
        ("next", 22, 13, Declaration),
    ] {
        assert_eq!(c_definitions(name, false), [(line, col, kind)], "{name}");
    }
    // 只在当前文件内查找时，形参和局部变量也算定义，且是局部的。
    for (name, line, col, kind) in [
        ("data", 24, 37, Parameter),
        ("size", 24, 52, Parameter),
        ("cfg", 26, 15, Binding),
        ("k", 27, 13, Binding),
    ] {
        let found = c_definitions(name, true);
        assert_eq!(found, [(line, col, kind)], "{name}");
        let definition = Definition {
            col,
            kind,
            indented: line != 24,
        };
        assert!(definition.is_local(), "{name}");
        assert!(c_definitions(name, false).is_empty(), "{name}");
    }
}

#[test]
fn c_type_uses_and_statements_are_not_definitions() {
    // `static uart_ctx_t s_ctx;` 中的 `uart_ctx_t`、`struct node *next;` 中的 `node`。
    assert_eq!(c_definitions("uart_ctx_t", true).len(), 1);
    assert!(c_definitions("node", true).is_empty());
    assert!(c_definitions("uint8_t", true).is_empty());
    // `configure(` 之后逐行书写的实参不在 `enum` 块内。
    assert_eq!(
        c_definitions("STATE_IDLE", true),
        [(2, 4, Kind::Declaration)]
    );
    for line in [
        "    return cfg;",
        "    goto out;",
        "    a * b;",
        "    x & y;",
        "import foo;",
    ] {
        for name in ["cfg", "out", "b", "y", "foo"] {
            assert_eq!(finds(name, line), None, "{line}");
        }
    }
    // 只看一行时不检查上下文，单独一个名字的一行按枚举成员算。
    assert_eq!(finds("STATE_RUN", "    STATE_RUN,"), Some(4));
    assert_eq!(finds("A", "enum { A, B };"), Some(7));
    assert_eq!(finds("B", "typedef enum { A = 1, B } ab_t;"), Some(22));
}
