//! `` ```mermaid `` 代码块的解析：源码转成图的结构，布局和绘制由 `winc-app` 的预览完成。
//!
//! 支持的图：流程图（`graph`、`flowchart`，含 `subgraph`）、状态图（`stateDiagram`）、时序图
//! （`sequenceDiagram`）、饼图（`pie`）、类图（`classDiagram`）、实体关系图（`erDiagram`）、
//! 思维导图（`mindmap`）、Git 提交图（`gitGraph`）和甘特图（`gantt`）。样式设置与点击事件被
//! 忽略，其余种类的图返回 [`Diagram::Unsupported`]。

use std::collections::HashMap;

mod class;
mod er;
mod gantt;
mod git;
mod mindmap;

pub use class::{Class, ClassBox, End, Relation};
pub use er::{Attribute, Card, Entity, Er, Link};
pub use gantt::{Gantt, Task, format_time, month_start};
pub use git::{Commit, CommitKind, Git};
pub use mindmap::{MindNode, MindShape, Mindmap};

/// 一个 mermaid 图的结构。
#[derive(Debug, Clone, PartialEq)]
pub enum Diagram {
    /// 流程图、状态图。
    Flow(Flow),
    /// 时序图。
    Sequence(Sequence),
    /// 饼图。
    Pie(Pie),
    /// 类图。
    Class(Class),
    /// 实体关系图。
    Er(Er),
    /// 思维导图。
    Mindmap(Mindmap),
    /// Git 提交图。
    Git(Git),
    /// 甘特图。
    Gantt(Gantt),
    /// 识别出种类但尚未支持绘制的图，内容为种类名（如 `classDiagram`）。
    Unsupported(String),
}

/// 流程图从起点向终点绘制时的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// 从上往下（`TD`、`TB`）。
    Down,
    /// 从下往上。
    Up,
    /// 从左往右。
    Right,
    /// 从右往左。
    Left,
}

/// 节点的形状，由源码中文字两侧的括号决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `[文字]`：矩形。
    Rect,
    /// `(文字)`：圆角矩形。
    Round,
    /// `([文字])`：两头是半圆。
    Stadium,
    /// `[[文字]]`：两边多一道竖线。
    Subroutine,
    /// `[(文字)]`：圆柱（数据库）。
    Cylinder,
    /// `((文字))`：圆形。
    Circle,
    /// `(((文字)))`：双层圆形。
    DoubleCircle,
    /// `>文字]`：左边是个缺口的旗子。
    Asym,
    /// `{文字}`：菱形（判断）。
    Diamond,
    /// `{{文字}}`：六边形。
    Hexagon,
    /// `[/文字/]`：平行四边形。
    Lean,
    /// `[\文字\]`。
    LeanLeft,
    /// `[/文字\]`：梯形。
    Trapezoid,
    /// `[\文字/]`：倒梯形。
    TrapezoidDown,
    /// 状态图的开始 `[*]`：实心圆点。
    Start,
    /// 状态图的结束 `[*]`：圆圈里一个实心点。
    End,
    /// 状态图的 `<<fork>>`、`<<join>>`：一道粗横条。
    Bar,
}

/// 流程图的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// 源码中写的节点名。
    pub id: String,
    /// 显示的文字，`\n` 表示换行。
    pub label: String,
    /// 形状。
    pub shape: Shape,
}

/// 连线的线型样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// `---`。
    Solid,
    /// `-.-`。
    Dotted,
    /// `===`。
    Thick,
    /// `~~~`：不绘制，只影响布局。
    Hidden,
}

/// 连线端点的线头形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// 没有线头。
    None,
    /// 实心三角箭头。
    Arrow,
    /// `o`：圆圈。
    Circle,
    /// `x`：叉。
    Cross,
}

/// 流程图的一条连线。
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// 起点在 [`Flow::nodes`] 中的序号。
    pub from: usize,
    /// 终点在 [`Flow::nodes`] 中的序号。
    pub to: usize,
    /// 连线上的文字，可以为空。
    pub label: String,
    /// 线型样式。
    pub line: Line,
    /// 终点的线头。
    pub head: Head,
    /// 起点的线头。
    pub tail: Head,
    /// 连线至少跨越的层数：连字符每多一个增加一层（`--->` 为 2，`-->` 为 1）。
    pub span: usize,
}

/// `subgraph` 声明的一组节点及其外框。
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// 外框上的标题。
    pub title: String,
    /// 直接或间接属于该框的节点（[`Flow::nodes`] 的序号）。
    pub members: Vec<usize>,
}

/// 按节点与连线排布的流程图，状态图也排成这种结构。
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    /// 绘制方向。
    pub dir: Dir,
    /// 节点，按第一次出现的先后排列。
    pub nodes: Vec<Node>,
    /// 连线，按源码中出现的先后排列。
    pub edges: Vec<Edge>,
    /// `subgraph` 的外框。
    pub groups: Vec<Group>,
}

/// 时序图的参与者。
#[derive(Debug, Clone, PartialEq)]
pub struct Actor {
    /// 源码中写的参与者名。
    pub id: String,
    /// 显示的名字（`participant A as 张三`）。
    pub label: String,
    /// 由 `actor` 声明：绘制成人形图案，而不是方框。
    pub person: bool,
}

/// 时序图消息线端点的箭头样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    /// `->>`：实心箭头。
    Filled,
    /// `->`：没有箭头。
    Plain,
    /// `-x`：叉形。
    Cross,
    /// `-)`：开口箭头，用于异步消息。
    Open,
}

/// 注释相对参与者的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `left of`。
    Left,
    /// `right of`。
    Right,
    /// `over`：覆盖在一个参与者或两个参与者之间。
    Over,
}

/// 时序图中按从上往下顺序排列的事件。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// 消息：发送方与接收方（[`Sequence::actors`] 的序号）、文字、线型、箭头，以及 `+`、`-`
    /// 标记的激活状态变化。
    Message {
        /// 发送方在 [`Sequence::actors`] 中的序号。
        from: usize,
        /// 接收方在 [`Sequence::actors`] 中的序号。
        to: usize,
        /// 消息的文字。
        text: String,
        /// 是否为虚线（`-->>`，通常表示回复）。
        dotted: bool,
        /// 箭头样式。
        arrow: Arrow,
        /// 是否两端都有箭头（`<<->>`）。
        both: bool,
        /// 激活状态变化：`+` 为 `Some(true)`（激活接收方），`-` 为 `Some(false)`
        /// （取消激活发送方）。
        activate: Option<bool>,
    },
    /// 注释：位置、关联的参与者、文字。
    Note {
        /// 注释位于参与者的哪一侧。
        side: Side,
        /// 关联的两个参与者（[`Sequence::actors`] 的序号），`over` 只写一个时两项相同。
        actors: (usize, usize),
        /// 注释的文字。
        text: String,
    },
    /// 分组框的开始：关键字（`loop`、`alt` 等）和框上的说明。
    Begin(String, String),
    /// 分组框内相邻两段之间的分隔：关键字（`else`、`and`、`option`）和下一段的说明。
    Divide(String),
    /// 分组框的结束关键字 `end`。
    End,
    /// 单独激活或取消激活参与者：序号，`true` 为 `activate`。
    Activate(usize, bool),
}

/// 时序图。
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    /// 参与者，按从左到右的顺序排列。
    pub actors: Vec<Actor>,
    /// 事件，按从上到下的顺序排列。
    pub events: Vec<Event>,
    /// 是否为消息显示序号（`autonumber`）。
    pub numbered: bool,
    /// 图的标题，源码没有给出时为 `None`。
    pub title: Option<String>,
}

/// 饼图的结构。
#[derive(Debug, Clone, PartialEq)]
pub struct Pie {
    /// 图的标题，源码没有给出时为 `None`。
    pub title: Option<String>,
    /// 各扇区的名字和数值，单位为源码中数值本身的单位。
    pub slices: Vec<(String, f64)>,
    /// 是否在图例中同时显示数值（`showData`）。
    pub show_data: bool,
}

/// 解析一段 mermaid 源码，源码中第一个非空、非注释的行决定图的种类。
///
/// 无法识别的图返回 `None`，包括连线数超过上限而拒绝绘制的流程图。
pub fn parse(source: &str) -> Option<Diagram> {
    // 保留行首空白：思维导图按缩进确定层级，其余种类的图使用去掉首尾空白的行。
    let raw: Vec<&str> = source
        .lines()
        .map(strip_comment)
        .filter(|l| !l.trim().is_empty())
        .collect();
    // 开头的 `---` 之间是 front matter，可设置图的标题，内容不参与后续解析。
    let mut at = 0;
    let mut title = None;
    if raw.first()?.trim() == "---" {
        at = 1;
        while let Some(line) = raw.get(at).map(|l| l.trim()) {
            at += 1;
            if line == "---" {
                break;
            }
            if let Some(t) = line.strip_prefix("title:") {
                title = Some(unquote(t.trim()).to_owned());
            }
        }
    }
    let first = raw.get(at)?.trim();
    let raw = &raw[at + 1..];
    let rest: Vec<&str> = raw.iter().map(|l| l.trim()).collect();
    let (kind, args) = first.split_once(char::is_whitespace).unwrap_or((first, ""));
    Some(match kind {
        "gitGraph" | "gitGraph:" => Diagram::Git(git::parse(args, &rest)?),
        "graph" | "flowchart" | "flowchart-elk" => {
            Diagram::Flow(flowchart(direction(args.trim()), &rest)?)
        }
        "stateDiagram" | "stateDiagram-v2" => Diagram::Flow(state(&rest)?),
        "sequenceDiagram" => {
            let mut seq = sequence(&rest);
            seq.title = seq.title.or(title);
            Diagram::Sequence(seq)
        }
        "pie" => {
            let mut pie = pie(args, &rest);
            pie.title = pie.title.or(title);
            Diagram::Pie(pie)
        }
        "classDiagram" | "classDiagram-v2" => Diagram::Class(class::parse(&rest)?),
        "erDiagram" => Diagram::Er(er::parse(&rest)?),
        "mindmap" => Diagram::Mindmap(mindmap::parse(raw)?),
        "gantt" => {
            let mut gantt = gantt::parse(&rest)?;
            gantt.title = gantt.title.or(title);
            Diagram::Gantt(gantt)
        }
        "journey" | "timeline" | "quadrantChart" | "requirementDiagram" | "C4Context"
        | "sankey-beta" | "xychart-beta" | "block-beta" | "packet-beta" | "kanban"
        | "architecture-beta" | "radar-beta" => Diagram::Unsupported(kind.to_owned()),
        _ => return None,
    })
}

/// 返回去掉开头 `%%` 注释后的行内容。
fn strip_comment(line: &str) -> &str {
    if line.trim_start().starts_with("%%") {
        ""
    } else {
        line
    }
}

/// 返回去掉首尾一对双引号后的字符串。
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// 返回节点或连线上的显示文字：去掉引号和 Markdown 字符串的反引号，把 `<br>` 与实体转成文字。
fn label(raw: &str) -> String {
    let s = unquote(raw.trim());
    let s = s
        .strip_prefix('`')
        .and_then(|s| s.strip_suffix('`'))
        .unwrap_or(s);
    let mut out = s
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .replace("\\n", "\n");
    for (entity, c) in [
        ("#quot;", "\""),
        ("#amp;", "&"),
        ("#lt;", "<"),
        ("#gt;", ">"),
    ] {
        out = out.replace(entity, c);
    }
    // 去掉了首尾反引号的 Markdown 字符串里，粗体和斜体记号不参与显示。
    if s.len() != unquote(raw.trim()).len() {
        out = out.replace("**", "").replace('*', "");
    }
    out.trim().to_owned()
}

fn direction(s: &str) -> Dir {
    match s.split_whitespace().next().unwrap_or("") {
        "LR" => Dir::Right,
        "RL" => Dir::Left,
        "BT" => Dir::Up,
        _ => Dir::Down,
    }
}

/// 一张图允许的最大连线数（条），超过时不绘制，按代码块显示原文。
///
/// 依据 mermaid 的 `maxEdges` 默认值。`A & B --> C & D` 会生成两组节点数之积条连线，
/// 数百个节点即可产生数万条，而排版在界面线程上进行。
const MAX_EDGES: usize = 500;

/// 解析流程图的正文；连线超过 `MAX_EDGES` 条时返回 `None`。
fn flowchart(dir: Dir, lines: &[&str]) -> Option<Flow> {
    let mut b = FlowBuilder {
        flow: Flow {
            dir,
            nodes: Vec::new(),
            edges: Vec::new(),
            groups: Vec::new(),
        },
        open: Vec::new(),
        ids: HashMap::new(),
    };
    for line in lines {
        for statement in line.split(';') {
            b.statement(statement.trim());
        }
    }
    b.finish()
}

struct FlowBuilder {
    flow: Flow,
    /// 尚未遇到 `end` 的 `subgraph`，内容为 [`Flow::groups`] 的序号，用于确定嵌套层级。
    open: Vec<usize>,
    /// 节点名到 [`Flow::nodes`] 中序号的对应关系。
    ///
    ids: HashMap<String, usize>,
}

impl FlowBuilder {
    /// 整理子图成员并返回流程图；连线超过 `MAX_EDGES` 条时返回 `None`。
    fn finish(mut self) -> Option<Flow> {
        if self.flow.edges.len() > MAX_EDGES {
            return None;
        }
        // 子图的成员也算进外面的子图。
        let groups = &mut self.flow.groups;
        for g in groups.iter_mut() {
            g.members.sort_unstable();
            g.members.dedup();
        }
        self.flow.groups.retain(|g| !g.members.is_empty());
        Some(self.flow)
    }

    /// 名字是 `id` 的节点，没有时新建一个（形状、文字先按名字）。
    fn node(&mut self, id: &str) -> usize {
        let i = match self.ids.get(id) {
            Some(&i) => i,
            None => {
                self.flow.nodes.push(Node {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    shape: Shape::Rect,
                });
                let i = self.flow.nodes.len() - 1;
                self.ids.insert(id.to_owned(), i);
                i
            }
        };
        for &g in &self.open {
            self.flow.groups[g].members.push(i);
        }
        i
    }

    fn statement(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let word = s.split_whitespace().next().unwrap_or("");
        match word {
            "subgraph" => {
                let rest = s["subgraph".len()..].trim();
                // `subgraph id [标题]`、`subgraph id["标题"]`、`subgraph 标题`。
                let title = match rest.find('[') {
                    Some(i) if rest.ends_with(']') => label(&rest[i + 1..rest.len() - 1]),
                    _ => label(rest),
                };
                self.flow.groups.push(Group {
                    title,
                    members: Vec::new(),
                });
                self.open.push(self.flow.groups.len() - 1);
                return;
            }
            "end" => {
                self.open.pop();
                return;
            }
            "classDef" | "class" | "style" | "linkStyle" | "click" | "direction" | "accTitle"
            | "accDescr" => return,
            _ => {}
        }
        // 一串：节点组 (连线 节点组)*。
        let mut rest = s;
        let Some((mut prev, after)) = self.nodes(rest) else {
            return;
        };
        rest = after;
        while let Some((spec, after)) = edge(rest) {
            let Some((next, after)) = self.nodes(after) else {
                break;
            };
            for &from in &prev {
                for &to in &next {
                    // 已经超过上限，整张图不画（见 [`FlowBuilder::finish`]），不必再生成。
                    if self.flow.edges.len() > MAX_EDGES {
                        return;
                    }
                    self.flow.edges.push(Edge {
                        from,
                        to,
                        ..spec.clone()
                    });
                }
            }
            prev = next;
            rest = after;
        }
    }

    /// `A & B[文字] & C` 这样的一组节点，返回序号和后面剩下的。
    fn nodes<'s>(&mut self, s: &'s str) -> Option<(Vec<usize>, &'s str)> {
        let mut out = Vec::new();
        let mut rest = s.trim_start();
        loop {
            let (i, after) = self.one_node(rest)?;
            out.push(i);
            rest = after.trim_start();
            match rest.strip_prefix('&') {
                Some(after) => rest = after.trim_start(),
                None => return Some((out, rest)),
            }
        }
    }

    /// 一个节点：名字，后面可以跟形状括号和文字、`:::类名`。
    fn one_node<'s>(&mut self, s: &'s str) -> Option<(usize, &'s str)> {
        let end = id_end(s);
        if end == 0 {
            return None;
        }
        let id = &s[..end];
        let mut rest = &s[end..];
        let i = self.node(id);
        if let Some((shape, text, after)) = shape(rest) {
            let node = &mut self.flow.nodes[i];
            node.shape = shape;
            node.label = label(text);
            rest = after;
        }
        if let Some(after) = rest.strip_prefix(":::") {
            rest = &after[id_end(after)..];
        }
        Some((i, rest))
    }
}

/// 节点名字到哪里为止：字母、数字、`_`、非 ASCII 的字；`-` 后面接字母数字时也算（`my-node`），
/// 接 `-`、`.`、`>` 是连线。
fn id_end(s: &str) -> usize {
    let mut end = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let ok = c.is_alphanumeric()
            || c == '_'
            || (c == '-' && i > 0 && next.is_some_and(|n| n.is_alphanumeric() || n == '_'))
            || (c == '.' && i > 0 && next.is_some_and(char::is_alphanumeric));
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    end
}

/// 节点名字后面的形状括号：形状、里面的文字、后面剩下的。
fn shape(s: &str) -> Option<(Shape, &str, &str)> {
    // 长的开头先认。
    const SHAPES: &[(&str, &str, Shape)] = &[
        ("(((", ")))", Shape::DoubleCircle),
        ("((", "))", Shape::Circle),
        ("([", "])", Shape::Stadium),
        ("[[", "]]", Shape::Subroutine),
        ("[(", ")]", Shape::Cylinder),
        ("{{", "}}", Shape::Hexagon),
        ("[/", "/]", Shape::Lean),
        ("[/", "\\]", Shape::Trapezoid),
        ("[\\", "\\]", Shape::LeanLeft),
        ("[\\", "/]", Shape::TrapezoidDown),
        ("{", "}", Shape::Diamond),
        ("(", ")", Shape::Round),
        ("[", "]", Shape::Rect),
        (">", "]", Shape::Asym),
    ];
    for &(open, close, shape) in SHAPES {
        let Some(after) = s.strip_prefix(open) else {
            continue;
        };
        // 文字里有引号时，引号里的括号不算。
        let text_end = if after.trim_start().starts_with('"') {
            let start = after.find('"')? + 1;
            start + after[start..].find('"')? + 1
        } else {
            0
        };
        let Some(i) = after[text_end..].find(close).map(|i| i + text_end) else {
            continue;
        };
        // `[/文字/]` 和 `[/文字\]` 开头一样：找到的收尾里面不能再有另一种收尾。
        if (open == "[/" || open == "[\\") && after[..i].contains(['/', '\\']) && text_end == 0 {
            let other = if close.starts_with('/') { "\\]" } else { "/]" };
            if after.find(other).is_some_and(|j| j < i) {
                continue;
            }
        }
        return Some((shape, &after[..i], &after[i + close.len()..]));
    }
    None
}

/// 一条连线（及上面的文字）：样子和后面剩下的。
fn edge(s: &str) -> Option<(Edge, &str)> {
    let s = s.trim_start();
    let mut rest = s;
    let mut tail = Head::None;
    for (mark, head) in [('<', Head::Arrow), ('o', Head::Circle), ('x', Head::Cross)] {
        if let Some(after) = rest.strip_prefix(mark)
            && after.starts_with(['-', '=', '.'])
        {
            tail = head;
            rest = after;
            break;
        }
    }
    let body_len = rest
        .find(|c: char| !matches!(c, '-' | '=' | '.' | '~'))
        .unwrap_or(rest.len());
    if body_len < 2 && !rest.starts_with("-.") {
        return None;
    }
    let mut body = rest[..body_len].to_owned();
    rest = &rest[body_len..];
    let mut head = arrow_head(&mut rest);
    let mut text = String::new();
    // `-- 文字 -->`、`== 文字 ==>`、`-. 文字 .->`：开头没有箭头，后面是空白和文字。
    if head == Head::None
        && matches!(body.as_str(), "--" | "==" | "-.")
        && rest.starts_with(char::is_whitespace)
        && let Some((label_end, close_start, close_len)) = closing(rest)
    {
        text = label(&rest[..label_end]);
        body.push_str(&rest[close_start..close_start + close_len]);
        rest = &rest[close_start + close_len..];
        head = arrow_head(&mut rest);
    }
    // `-->|文字|`。
    let trimmed = rest.trim_start();
    if let Some(after) = trimmed.strip_prefix('|')
        && let Some(end) = after.find('|')
    {
        text = label(&after[..end]);
        rest = &after[end + 1..];
    }
    let line = if body.contains('~') {
        Line::Hidden
    } else if body.contains('.') {
        Line::Dotted
    } else if body.contains('=') {
        Line::Thick
    } else {
        Line::Solid
    };
    let dashes = body.chars().filter(|&c| c == '-' || c == '=').count();
    Some((
        Edge {
            from: 0,
            to: 0,
            label: text,
            line,
            head,
            tail,
            span: dashes.saturating_sub(2).max(1),
        },
        rest,
    ))
}

/// 连线末尾的线头：`>`、`o`、`x`（后面是空白或节点时才算）。
fn arrow_head(rest: &mut &str) -> Head {
    if let Some(after) = rest.strip_prefix('>') {
        *rest = after;
        return Head::Arrow;
    }
    for (mark, head) in [('o', Head::Circle), ('x', Head::Cross)] {
        if let Some(after) = rest.strip_prefix(mark)
            && after.starts_with(char::is_whitespace)
        {
            *rest = after;
            return head;
        }
    }
    Head::None
}

/// `-- 文字 -->` 里文字后面的收尾：返回（文字的长度，收尾的开头、长度）。
fn closing(s: &str) -> Option<(usize, usize, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if matches!(c, b'-' | b'=' | b'.') && matches!(bytes[i + 1], b'-' | b'=' | b'.' | b'>') {
            let len = s[i..]
                .find(|c: char| !matches!(c, '-' | '=' | '.'))
                .unwrap_or(s.len() - i);
            return Some((i, i, len));
        }
        i += 1;
    }
    None
}

/// 解析状态图的正文，排成流程图：状态是圆角框，`[*]` 是开始、结束的圆点，`state X { … }`
/// 是外面的框；连线超过 [`MAX_EDGES`] 条时返回 `None`。
fn state(lines: &[&str]) -> Option<Flow> {
    let mut b = FlowBuilder {
        flow: Flow {
            dir: Dir::Down,
            nodes: Vec::new(),
            edges: Vec::new(),
            groups: Vec::new(),
        },
        open: Vec::new(),
        ids: HashMap::new(),
    };
    // `[*]` 在每层框里各是一对开始、结束。
    let mut scope = String::new();
    let state_node = |b: &mut FlowBuilder, name: &str, scope: &str, start: bool| -> usize {
        if name == "[*]" {
            let id = format!("[*]{}{scope}", if start { "start" } else { "end" });
            let i = b.node(&id);
            let node = &mut b.flow.nodes[i];
            node.shape = if start { Shape::Start } else { Shape::End };
            node.label = String::new();
            i
        } else {
            let i = b.node(name);
            let node = &mut b.flow.nodes[i];
            if node.shape == Shape::Rect {
                node.shape = Shape::Round;
            }
            i
        }
    };
    let mut scopes: Vec<String> = Vec::new();
    for line in lines {
        let line = line.trim();
        if let Some(dir) = line.strip_prefix("direction ") {
            if scopes.is_empty() {
                b.flow.dir = direction(dir);
            }
            continue;
        }
        if line == "}" {
            b.open.pop();
            scope = scopes.pop().unwrap_or_default();
            continue;
        }
        if line == "--"
            || line.starts_with("note ")
            || line.starts_with("classDef ")
            || line.starts_with("class ")
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("state ") {
            let rest = rest.trim();
            // `state "说明" as S`、`state S <<choice>>`、`state S {`。
            if let Some(name) = rest.strip_suffix('{') {
                let name = name.trim();
                let (title, id) = match name.split_once(" as ") {
                    Some((t, id)) => (label(t), id.trim().to_owned()),
                    None => (label(name), name.to_owned()),
                };
                let i = state_node(&mut b, &id, &scope, false);
                b.flow.groups.push(Group {
                    title,
                    members: vec![i],
                });
                b.open.push(b.flow.groups.len() - 1);
                scopes.push(std::mem::replace(&mut scope, format!("@{id}")));
                continue;
            }
            if let Some((t, id)) = rest.split_once(" as ") {
                let i = state_node(&mut b, id.trim(), &scope, false);
                b.flow.nodes[i].label = label(t);
                continue;
            }
            let (id, kind) = rest.split_once("<<").unwrap_or((rest, ""));
            let i = state_node(&mut b, id.trim(), &scope, false);
            let node = &mut b.flow.nodes[i];
            if kind.starts_with("choice") {
                node.shape = Shape::Diamond;
                node.label = String::new();
            } else if kind.starts_with("fork") || kind.starts_with("join") {
                node.shape = Shape::Bar;
                node.label = String::new();
            }
            continue;
        }
        // `A --> B : 文字`。
        if let Some((from, rest)) = line.split_once("-->") {
            let (to, text) = rest.split_once(':').unwrap_or((rest, ""));
            let from = state_node(&mut b, from.trim(), &scope, true);
            let to = state_node(&mut b, to.trim(), &scope, false);
            b.flow.edges.push(Edge {
                from,
                to,
                label: label(text),
                line: Line::Solid,
                head: Head::Arrow,
                tail: Head::None,
                span: 1,
            });
            continue;
        }
        // `S : 说明`：说明另起一行接在名字后面。
        if let Some((id, text)) = line.split_once(':') {
            let i = state_node(&mut b, id.trim(), &scope, false);
            let node = &mut b.flow.nodes[i];
            let text = label(text);
            if node.label == node.id {
                node.label = text;
            } else {
                node.label = format!("{}\n{text}", node.label);
            }
            continue;
        }
        if id_end(line) == line.len() {
            state_node(&mut b, line, &scope, false);
        }
    }
    b.finish()
}

/// 解析时序图的正文。
fn sequence(lines: &[&str]) -> Sequence {
    let mut seq = Sequence {
        actors: Vec::new(),
        events: Vec::new(),
        numbered: false,
        title: None,
    };
    fn actor(seq: &mut Sequence, id: &str) -> usize {
        let id = id.trim();
        match seq.actors.iter().position(|a| a.id == id) {
            Some(i) => i,
            None => {
                seq.actors.push(Actor {
                    id: id.to_owned(),
                    label: id.to_owned(),
                    person: false,
                });
                seq.actors.len() - 1
            }
        }
    }
    for line in lines {
        let line = line.trim();
        let (word, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(w, r)| (w, r.trim()));
        match word {
            "participant" | "actor" => {
                let (id, name) = rest.split_once(" as ").unwrap_or((rest, rest));
                let i = actor(&mut seq, id);
                seq.actors[i].label = label(name);
                seq.actors[i].person = word == "actor";
                continue;
            }
            "autonumber" => {
                seq.numbered = true;
                continue;
            }
            "title" => {
                seq.title = Some(label(rest.trim_start_matches(':')));
                continue;
            }
            "activate" | "deactivate" => {
                let i = actor(&mut seq, rest);
                seq.events.push(Event::Activate(i, word == "activate"));
                continue;
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                seq.events.push(Event::Begin(word.to_owned(), label(rest)));
                continue;
            }
            "else" | "and" | "option" => {
                seq.events.push(Event::Divide(label(rest)));
                continue;
            }
            "end" => {
                seq.events.push(Event::End);
                continue;
            }
            "box" | "create" | "destroy" | "links" | "link" | "properties" | "details" => continue,
            _ => {}
        }
        if word.eq_ignore_ascii_case("note") {
            let (place, text) = rest.split_once(':').unwrap_or((rest, ""));
            let place = place.trim();
            // 方位关键字不区分大小写，参与者的名字保持原样。
            let after = |prefix: &str| {
                place
                    .get(..prefix.len())
                    .filter(|p| p.eq_ignore_ascii_case(prefix))
                    .map(|_| &place[prefix.len()..])
            };
            let (side, who) = if let Some(w) = after("left of") {
                (Side::Left, w)
            } else if let Some(w) = after("right of") {
                (Side::Right, w)
            } else {
                (Side::Over, after("over").unwrap_or(place))
            };
            let (a, b) = who.split_once(',').unwrap_or((who, who));
            let a = actor(&mut seq, a);
            let b = actor(&mut seq, b);
            seq.events.push(Event::Note {
                side,
                actors: (a, b),
                text: label(text),
            });
            continue;
        }
        if let Some(event) = message(&mut seq, line, actor) {
            seq.events.push(event);
        }
    }
    seq
}

/// 一条消息 `A->>+B: 文字`。
fn message(
    seq: &mut Sequence,
    line: &str,
    actor: fn(&mut Sequence, &str) -> usize,
) -> Option<Event> {
    // 长的箭头先认。
    const ARROWS: &[(&str, bool, Arrow, bool)] = &[
        ("<<-->>", true, Arrow::Filled, true),
        ("<<->>", false, Arrow::Filled, true),
        ("-->>", true, Arrow::Filled, false),
        ("->>", false, Arrow::Filled, false),
        ("--x", true, Arrow::Cross, false),
        ("-x", false, Arrow::Cross, false),
        ("--)", true, Arrow::Open, false),
        ("-)", false, Arrow::Open, false),
        ("-->", true, Arrow::Plain, false),
        ("->", false, Arrow::Plain, false),
    ];
    let (head, text) = line.split_once(':').unwrap_or((line, ""));
    let (at, &(mark, dotted, arrow, both)) = ARROWS
        .iter()
        .filter_map(|a| head.find(a.0).map(|i| (i, a)))
        .min_by_key(|&(i, a)| (i, std::cmp::Reverse(a.0.len())))?;
    let from = head[..at].trim();
    let mut to = head[at + mark.len()..].trim();
    let mut activate = None;
    if let Some(t) = to.strip_prefix('+') {
        activate = Some(true);
        to = t.trim();
    } else if let Some(t) = to.strip_prefix('-') {
        activate = Some(false);
        to = t.trim();
    }
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let from = actor(seq, from);
    let to = actor(seq, to);
    Some(Event::Message {
        from,
        to,
        text: label(text),
        dotted,
        arrow,
        both,
        activate,
    })
}

/// 解析饼图：`pie showData`、`title …`、`"名字" : 数`。
fn pie(args: &str, lines: &[&str]) -> Pie {
    let mut pie = Pie {
        title: None,
        slices: Vec::new(),
        show_data: args.contains("showData"),
    };
    if let Some(t) = args
        .trim()
        .strip_prefix("showData")
        .unwrap_or(args)
        .trim()
        .strip_prefix("title")
    {
        pie.title = Some(label(t));
    }
    for line in lines {
        if let Some(t) = line.strip_prefix("title") {
            pie.title = Some(label(t));
            continue;
        }
        if *line == "showData" {
            pie.show_data = true;
            continue;
        }
        if let Some((name, value)) = line.rsplit_once(':')
            && let Ok(value) = value.trim().parse::<f64>()
            && value >= 0.0
        {
            pie.slices.push((label(name), value));
        }
    }
    pie
}

#[cfg(test)]
mod tests;
