//! 符号提取的测试：各语言的声明与方法、注释和字符串中的内容不误判、标题与小节的层次。

use super::*;

fn outline(language: &str, text: &str) -> Vec<(String, &'static str, Option<String>)> {
    let language = winc_syntax::detect(&format!("x.{language}"));
    symbols(&Rope::from_str(text), language)
        .into_iter()
        .map(|s| (s.name, s.kind.label(), s.container))
        .collect()
}

fn names(language: &str, text: &str) -> Vec<String> {
    outline(language, text)
        .into_iter()
        .map(|(n, ..)| n)
        .collect()
}

#[test]
fn rust_items_and_impls() {
    // 样本的 `//!` 注释和块注释中各含有一个 `fn`，二者都不应被识别为符号。
    let text = "\
//! fn not_this
use std::fmt;

pub const LIMIT: usize = 3;

pub(crate) struct Server {
    port: u16,
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = 1;
        Ok(())
    }
}

/* fn hidden() {
} */
pub async fn run() {}
macro_rules! log { () => {} }
";
    assert_eq!(
        outline("rs", text),
        [
            ("LIMIT".into(), "变量", None),
            ("Server".into(), "类型", None),
            ("fmt::Display for Server".into(), "模块", None),
            ("fmt".into(), "方法", Some("fmt::Display for Server".into())),
            ("run".into(), "函数", None),
            ("log".into(), "函数", None),
        ]
    );
}

#[test]
fn python_classes_methods_and_module_names() {
    // 样本的三引号字符串和 `#` 注释中各含有一个 `def`，二者都不应被识别为符号。
    let text = "\
\"\"\"
def not_this():
\"\"\"
VERSION = \"1\"

class Shop:
    tax = 0.1

    def total(self):
        count = 1
        def inner():
            pass

# def commented():
async def main():
    pass
";
    assert_eq!(
        outline("py", text),
        [
            ("VERSION".into(), "变量", None),
            ("Shop".into(), "类型", None),
            ("total".into(), "方法", Some("Shop".into())),
            ("inner".into(), "函数", Some("total".into())),
            ("main".into(), "函数", None),
        ]
    );
}

#[test]
fn javascript_functions_classes_and_methods() {
    let text = "\
export default class Cart {
  constructor(items) {
    this.items = items;
  }
  async total(tax) {
    if (tax) {
      return 1;
    }
  }
}
export const sum = (a, b) => a + b;
const label = 'x';
function render() {}
describe('cart', () => {
  it(\"adds\", function () {
  });
});
";
    assert_eq!(
        outline("js", text),
        [
            ("Cart".into(), "类型", None),
            ("constructor".into(), "方法", Some("Cart".into())),
            ("total".into(), "方法", Some("Cart".into())),
            ("sum".into(), "函数", None),
            ("label".into(), "变量", None),
            ("render".into(), "函数", None),
        ]
    );
}

#[test]
fn go_c_and_java() {
    let go = "type Server struct {}\n\nfunc (s *Server) Start() error {\n}\n\nfunc main() {\n}\n";
    assert_eq!(
        outline("go", go),
        [
            ("Server".into(), "类型", None),
            ("Start".into(), "方法", Some("Server".into())),
            ("main".into(), "函数", None),
        ]
    );

    let c = "#define MAX 3\nstatic int count(const char *s)\n{\n    return strlen(s);\n}\nint x = foo(3);\n";
    assert_eq!(names("c", c), ["MAX", "count", "x"]);

    let java = "public class App {\n    public static void main(String[] args) {\n        while (true) {\n        }\n    }\n}\n";
    assert_eq!(
        outline("java", java),
        [
            ("App".into(), "类型", None),
            ("main".into(), "方法", Some("App".into())),
        ]
    );
}

#[test]
fn headings_sections_and_others() {
    let md = "# Title\n\n```sh\n# not a heading\n```\n## Install ##\n### Mac\n## Usage\n";
    assert_eq!(
        outline("md", md),
        [
            ("Title".into(), "标题", None),
            ("Install".into(), "标题", Some("Title".into())),
            ("Mac".into(), "标题", Some("Install".into())),
            ("Usage".into(), "标题", Some("Title".into())),
        ]
    );

    assert_eq!(
        names("toml", "[package]\nname = \"x\"\n[[bin]]\n"),
        ["package", "bin"]
    );
    assert_eq!(
        names("sh", "# deploy() {\ndeploy() {\n}\nfunction clean {\n}\n"),
        ["deploy", "clean"]
    );
    assert_eq!(
        names(
            "sql",
            "CREATE TABLE IF NOT EXISTS users (\n);\ncreate view v as select 1;\n"
        ),
        ["users", "v"]
    );
    assert!(names("json", "{\"a\": 1}").is_empty());
}

#[test]
fn arkts_components_list_their_members_but_not_the_components_they_build() {
    let text = "\
@Entry
@Component
struct Index {
  @State message: string = 'Hello';

  aboutToAppear(): void {
    this.load();
  }

  @Builder header(title: string) {
    Text(title)
  }

  build() {
    Column({ space: 8 }) {
      Row() {
        Text(this.message)
          .fontSize(50)
      }
      if (this.message) {
        this.header('x')
      }
    }
  }
}

@Component export struct Badge {
  build() {
  }
}

@Builder function footer() {
  Row() {
  }
}
class Store {
  constructor(name: string) {
  }
}
";
    assert_eq!(
        outline("ets", text),
        [
            ("Index".into(), "类型", None),
            ("aboutToAppear".into(), "方法", Some("Index".into())),
            ("header".into(), "方法", Some("Index".into())),
            ("build".into(), "方法", Some("Index".into())),
            ("Badge".into(), "类型", None),
            ("build".into(), "方法", Some("Badge".into())),
            ("footer".into(), "函数", None),
            ("Store".into(), "类型", None),
            ("constructor".into(), "方法", Some("Store".into())),
        ]
    );
    // 其他语言中写在声明同一行的注解同样不妨碍识别。
    assert_eq!(
        names(
            "java",
            "class A {\n  @Override public void run() {\n  }\n}\n"
        ),
        ["A", "run"]
    );
    assert_eq!(
        names("kt", "@Serializable data class User(val id: Int)\n"),
        ["User"]
    );
}

/// 微信小程序：WXS 按 JavaScript 列出函数和变量；WXSS 和样式表一样不列出符号。
#[test]
fn wechat_mini_program_files() {
    let wxs = "var format = function (n) {\n  return n;\n};\nfunction pad(n) {}\nmodule.exports = { format: format };\n";
    assert_eq!(names("wxs", wxs), ["format", "pad"]);
    assert!(names("wxss", ".title {\n  font-size: 32rpx;\n}\n").is_empty());
}

/// 列出 `typedef` 别名、`} name_t;`、全局变量和各种写法的函数；不列出成员、局部变量和前置声明。
#[test]
fn c_typedefs_globals_and_functions() {
    let c = "\
typedef enum
{
    STATE_IDLE = 0,
    STATE_RUN,
} app_state_t;

typedef struct {
    uint8_t  buf[64];
    struct node *next;
} uart_ctx_t;

static struct {
    int count;
} s_stats;

struct node;
struct node {
    struct node *next;
};
typedef void (*uart_cb_t)(uint8_t);
typedef unsigned char u8;
uint32_t g_tick;
extern volatile uint32_t g_ticks;
uint8_t *p_buf;
static const char *names[] = { \"a\" };
__attribute__((weak)) void SysTick_Handler(void)
{
}
static void
uart_send(const uint8_t *data, uint16_t size)
{
    uart_ctx_t cfg = {0};
    return;
}
void prototype(void);
";
    assert_eq!(
        outline("c", c),
        [
            ("app_state_t".into(), "类型", None),
            ("uart_ctx_t".into(), "类型", None),
            ("s_stats".into(), "变量", None),
            ("node".into(), "类型", None),
            ("uart_cb_t".into(), "类型", None),
            ("u8".into(), "类型", None),
            ("g_tick".into(), "变量", None),
            ("g_ticks".into(), "变量", None),
            ("p_buf".into(), "变量", None),
            ("names".into(), "变量", None),
            ("SysTick_Handler".into(), "函数", None),
            ("uart_send".into(), "函数", None),
        ]
    );
}

#[test]
fn cpp_header_members_with_wrapped_signatures_and_inline_bodies() {
    let text = r"namespace example {
class Conversion {
public:
    void set_destination(int id) { destination = id; }
    void add(const std::vector<Location>& rows,
             const std::vector<unsigned>& limits) {
        record(rows, limits);
    }
    const std::pair<int, int>& destination_at(
            unsigned row)
            const {
        return lookup(row);
    }
    int count() const
    {
        return destination;
    }
    void declaration(const std::vector<Location>& rows,
                     unsigned limit);
};
}
";
    for extension in ["h", "hpp"] {
        assert_eq!(
            outline(extension, text),
            [
                ("example".into(), "模块", None),
                ("Conversion".into(), "类型", None),
                ("set_destination".into(), "方法", Some("Conversion".into())),
                ("add".into(), "方法", Some("Conversion".into())),
                ("destination_at".into(), "方法", Some("Conversion".into())),
                ("count".into(), "方法", Some("Conversion".into())),
            ]
        );
    }
    let found = symbols(&Rope::from_str(text), winc_syntax::detect("x.h"));
    assert_eq!((found[3].line, found[3].col, found[3].depth), (4, 9, 1));
}

#[test]
fn cpp_members_with_constructors_comments_defaults_and_trailing_parts() {
    let text = r#"class Store {
public:
    explicit Store(int size,
                   int limit) : size_(size), limit_{limit} {
        Guard guard(size,
                    limit);
        if (size &&
            limit) {
        }
    }
    Store(const Store& other,
          int limit) {
        copy(other);
    }
    Store(Store&&) = delete;
    Store(int size);
    ~Store() { release(); }
    void commented(int first,  // the first; isn't checked
                   int second /* (unbalanced */) {
    }
    void defaulted(const std::string& name = "x;{",
                   int limit = compute(1)) {
    }
    [[nodiscard]] int attributed(int first,
                                 int second) const {
    }
    auto trailing(int first,
                  int second) -> std::vector<int> {
    }
    void guarded(int first,
                 int second) noexcept(true) override {
    }
    void optional(std::vector<int> values = {});
    int field(1), other(2);
};
namespace tools {
    void indented(int first,
                  int second) {
    }
    helper(int first,
           int second) {
    }
}
"#;
    for extension in ["h", "hpp"] {
        assert_eq!(
            names(extension, text),
            [
                "Store",
                "Store",
                "Store",
                "~Store",
                "commented",
                "defaulted",
                "attributed",
                "trailing",
                "guarded",
                "tools",
                "indented",
            ]
        );
    }
    let found = symbols(&Rope::from_str(text), winc_syntax::detect("x.hpp"));
    assert!(found[1..9].iter().all(|s| s.kind == SymbolKind::Method));
    assert_eq!((found[1].line, found[1].col), (2, 13));
    assert_eq!((found[3].line, found[3].col), (16, 4));

    // 超过行数上限的签名不列出。
    let long = format!(
        "class A {{\n    void f(int a,{}\n           int b) {{\n    }}\n}};\n",
        "\n".repeat(20)
    );
    assert_eq!(names("hpp", &long), ["A"]);
}
