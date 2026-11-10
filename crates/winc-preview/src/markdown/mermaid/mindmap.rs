//! 思维导图（`mindmap`）的解析：按缩进确定层级，括号决定节点形状。
//!
//! 图标（`::icon(…)`）与样式类（`:::类名`）被忽略。

use super::label;

/// 思维导图节点的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MindShape {
    /// 没有括号：只有文字，下方一道横线。
    Plain,
    /// `[文字]`：矩形。
    Rect,
    /// `(文字)`：圆角矩形。
    Round,
    /// `((文字))`：圆形。
    Circle,
    /// `))文字((`：爆炸形。
    Bang,
    /// `)文字(`：云朵形。
    Cloud,
    /// `{{文字}}`：六边形。
    Hexagon,
}

/// 思维导图的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct MindNode {
    /// 显示的文字，`\n` 表示换行。
    pub label: String,
    /// 形状。
    pub shape: MindShape,
    /// 父节点在 [`Mindmap::nodes`] 中的序号；根节点为 `None`。
    pub parent: Option<usize>,
}

/// 思维导图：节点按源码中的先后排列，第一个是根节点，父节点总在子节点之前。
#[derive(Debug, Clone, PartialEq)]
pub struct Mindmap {
    /// 所有节点。
    pub nodes: Vec<MindNode>,
}

/// 解析思维导图的正文（保留行首空白）；没有任何节点时返回 `None`。
pub(super) fn parse(lines: &[&str]) -> Option<Mindmap> {
    let mut nodes: Vec<MindNode> = Vec::new();
    // 从根到当前节点的一串祖先：缩进和序号。
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for line in lines {
        let text = line.trim();
        if text.starts_with("::icon(") || text.starts_with(":::") {
            continue;
        }
        let indent: usize = line
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum();
        while stack.len() > 1 && stack.last().is_some_and(|&(at, _)| at >= indent) {
            stack.pop();
        }
        // 缩进不超过根节点的行也挂在根节点下：只有一个根。
        let parent = stack.last().map(|&(_, i)| i);
        let (shape, text) = node(text.split(":::").next().unwrap_or(text).trim());
        nodes.push(MindNode {
            label: label(text),
            shape,
            parent,
        });
        stack.push((indent, nodes.len() - 1));
    }
    (!nodes.is_empty()).then_some(Mindmap { nodes })
}

/// 节点的形状和文字：`id(文字)` 这类写法去掉前面的名字和括号，没有括号时整行是文字。
fn node(s: &str) -> (MindShape, &str) {
    let Some(at) = s.find(['(', '[', '{', ')']) else {
        return (MindShape::Plain, s);
    };
    let body = &s[at..];
    for (open, close, shape) in [
        ("((", "))", MindShape::Circle),
        ("))", "((", MindShape::Bang),
        ("{{", "}}", MindShape::Hexagon),
        ("(", ")", MindShape::Round),
        (")", "(", MindShape::Cloud),
        ("[", "]", MindShape::Rect),
    ] {
        if let Some(inner) = body.strip_prefix(open).and_then(|b| b.strip_suffix(close)) {
            return (shape, inner);
        }
    }
    (MindShape::Plain, s)
}
