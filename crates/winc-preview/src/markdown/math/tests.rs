//! 公式解析的测试：上下标、分式与根号、环境与多行公式排成的表格。

use super::*;

fn sym(text: &str, class: Class, font: Font) -> Node {
    Node::Sym(text.into(), class, font)
}

#[test]
fn letters_operators_and_scripts() {
    let Node::Row(nodes) = parse("x_1^2 + \\alpha' \\le \\mathbb{R}") else {
        panic!();
    };
    assert_eq!(
        nodes[0],
        Node::Scripts(
            Box::new(sym("x", Class::Ord, Font::Italic)),
            Some(Box::new(sym("2", Class::Ord, Font::Roman))),
            Some(Box::new(sym("1", Class::Ord, Font::Roman))),
        )
    );
    assert_eq!(nodes[1], sym("+", Class::Bin, Font::Roman));
    assert!(matches!(&nodes[2], Node::Row(r) if r[1] == sym("′", Class::Ord, Font::Roman)));
    assert_eq!(nodes[3], sym("≤", Class::Rel, Font::Roman));
    assert_eq!(nodes[4], Node::Row(vec![sym("ℝ", Class::Ord, Font::Roman)]));
}

/// amsmath、amssymb、latexsym 中的符号：方框、斜体的大写希腊字母。
#[test]
fn symbols_from_the_common_packages() {
    let Node::Row(nodes) = parse("\\Box \\square \\blacksquare \\varGamma \\varOmega \\boxplus")
    else {
        panic!();
    };
    assert_eq!(nodes[0], sym("□", Class::Ord, Font::Roman));
    assert_eq!(nodes[1], sym("□", Class::Ord, Font::Roman));
    assert_eq!(nodes[2], sym("■", Class::Ord, Font::Roman));
    assert_eq!(nodes[3], sym("Γ", Class::Ord, Font::Italic));
    assert_eq!(nodes[4], sym("Ω", Class::Ord, Font::Italic));
    assert_eq!(nodes[5], sym("⊞", Class::Bin, Font::Roman));
}

#[test]
fn fractions_roots_fences_and_ops() {
    let Node::Row(nodes) =
        parse("\\frac{a}{b} \\sqrt[3]{x} \\left( y \\right] \\sum_{i=1}^n \\int \\lim \\foo")
    else {
        panic!();
    };
    assert!(matches!(&nodes[0], Node::Frac(_, _, true, None)));
    assert!(matches!(&nodes[1], Node::Sqrt(Some(_), _)));
    assert!(matches!(&nodes[2], Node::Fenced(l, _, r) if l == "(" && r == "]"));
    assert!(
        matches!(&nodes[3], Node::Scripts(op, Some(_), Some(_)) if **op == Node::Op("∑".into(), true, None))
    );
    assert_eq!(nodes[4], Node::Op("∫".into(), true, Some(false)));
    assert_eq!(nodes[5], Node::Op("lim".into(), false, None));
    assert_eq!(nodes[6], Node::Error("\\foo".into()));
}

#[test]
fn environments_and_lines_become_grids() {
    let Node::Grid(grid) = parse("\\begin{pmatrix} 1 & 2 \\\\ 3 & 4 \\\\ \\end{pmatrix}") else {
        panic!();
    };
    assert_eq!((grid.left.as_str(), grid.right.as_str()), ("(", ")"));
    assert_eq!(grid.rows.len(), 2);
    assert_eq!(grid.rows[1].len(), 2);
    let Node::Grid(lines) = parse("a &= b \\\\ &= c") else {
        panic!();
    };
    assert_eq!(lines.align, ['r', 'l']);
    assert_eq!(lines.rows.len(), 2);
    let Node::Grid(cases) =
        parse("f(x) = \\begin{cases} 1 & x > 0 \\\\ 0 & \\text{否则} \\end{cases}").rows_last()
    else {
        panic!();
    };
    assert_eq!(cases.left, "{");
    assert_eq!(
        cases.rows[1][1],
        Node::Row(vec![sym("否则", Class::Ord, Font::Text)])
    );
}

impl Node {
    /// 返回一串节点里的最后一个节点；不是一串时返回自身。
    fn rows_last(self) -> Node {
        match self {
            Node::Row(mut nodes) => nodes.pop().unwrap(),
            other => other,
        }
    }
}

/// 在与后台线程相同大小（2 MB）的栈上解析并释放公式，返回最内层无法解析的原文。
///
/// 栈大小取后台解析线程的配置，用于复现深嵌套引起的栈溢出。
fn parse_on_small_stack(tex: String) -> Option<String> {
    fn error(node: &Node) -> Option<String> {
        match node {
            Node::Error(text) => Some(text.clone()),
            Node::Row(nodes) => nodes.iter().find_map(error),
            Node::Frac(a, b, ..) => error(a).or_else(|| error(b)),
            Node::Accent(_, n) | Node::Fenced(_, n, _) | Node::Style(_, n) => error(n),
            _ => None,
        }
    }
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || error(&parse(&tex)))
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deep_nesting_stops_and_shows_the_rest_as_is() {
    // 回归：数万层 `{`、`\hat`、`\left(`、`\frac` 在解析和释放时栈溢出、进程中止。
    // 超过上限后必须返回错误节点，而不是继续下行递归。
    for tex in [
        format!("{}x", "{".repeat(50_000)),
        format!("{} x", "\\hat".repeat(50_000)),
        "\\left(".repeat(50_000),
        "\\frac{".repeat(50_000),
    ] {
        assert!(parse_on_small_stack(tex).is_some());
    }
    // 超过上限的部分原样显示为红色文字，未超过上限的部分正常解析。
    // 断言里的 50 来自 `MAX_DEPTH`：第 51 层起未解析，原文保留第 51 至 150 层的反括号。
    let rest = parse_on_small_stack(format!("{}x{}", "{".repeat(150), "}".repeat(150))).unwrap();
    assert_eq!(rest, format!("{}x{}", "{".repeat(50), "}".repeat(150)));
    assert_eq!(
        parse_on_small_stack(format!("{}x{}", "{".repeat(90), "}".repeat(90))),
        None
    );
}
