//! 实体关系图（`erDiagram`）的解析：实体及其属性、实体之间带基数的关系。
//!
//! 关系只识别符号写法（`||--o{`），样式设置被忽略。

use std::collections::HashMap;

use super::{Dir, MAX_EDGES, direction, label, unquote};

/// 实体关系图。
#[derive(Debug, Clone, PartialEq)]
pub struct Er {
    /// 绘制方向，默认从上往下。
    pub dir: Dir,
    /// 实体，按第一次出现的先后排列。
    pub entities: Vec<Entity>,
    /// 关系，按源码中出现的先后排列。
    pub links: Vec<Link>,
}

/// 一个实体。
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// 源码中写的实体名。
    pub id: String,
    /// 显示的名字（`A["名字"]` 中的文字，没有时同实体名）。
    pub label: String,
    /// 属性，按源码中的先后排列。
    pub attributes: Vec<Attribute>,
}

/// 实体的一条属性：`类型 名字 PK, FK "说明"`。
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// 类型。
    pub kind: String,
    /// 名字。
    pub name: String,
    /// 键（`PK`、`FK`、`UK`），多个时以 `, ` 连接，可以为空。
    pub keys: String,
    /// 说明，可以为空。
    pub comment: String,
}

/// 关系一端的基数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    /// `|o`、`o|`：零或一个。
    ZeroOrOne,
    /// `||`：恰好一个。
    ExactlyOne,
    /// `}o`、`o{`：零或多个。
    ZeroOrMore,
    /// `}|`、`|{`：一或多个。
    OneOrMore,
}

/// 两个实体之间的关系。
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// 左侧的实体在 [`Er::entities`] 中的序号。
    pub from: usize,
    /// 右侧的实体在 [`Er::entities`] 中的序号。
    pub to: usize,
    /// 左端（靠近左侧实体）的基数。
    pub from_card: Card,
    /// 右端（靠近右侧实体）的基数。
    pub to_card: Card,
    /// 是否为标识关系（实线 `--`）；非标识关系（`..`）绘制为虚线。
    pub identifying: bool,
    /// 关系上的文字，可以为空。
    pub label: String,
}

/// 解析实体关系图的正文；没有任何实体或关系数超过上限时返回 `None`。
pub(super) fn parse(lines: &[&str]) -> Option<Er> {
    let mut er = Er {
        dir: Dir::Down,
        entities: Vec::new(),
        links: Vec::new(),
    };
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut entity = |er: &mut Er, head: &str| -> usize {
        let (id, shown) = match head.split_once('[') {
            Some((id, rest)) => (id.trim(), rest.strip_suffix(']').map(label)),
            None => (head.trim(), None),
        };
        let i = *ids.entry(id.to_owned()).or_insert_with(|| {
            er.entities.push(Entity {
                id: id.to_owned(),
                label: unquote(id).to_owned(),
                attributes: Vec::new(),
            });
            er.entities.len() - 1
        });
        if let Some(shown) = shown {
            er.entities[i].label = shown;
        }
        i
    };
    // 正在读取属性的实体（`A {` 之后、`}` 之前）。
    let mut body: Option<usize> = None;
    for &line in lines {
        if let Some(i) = body {
            if line == "}" {
                body = None;
            } else if let Some(a) = attribute(line) {
                er.entities[i].attributes.push(a);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("direction ") {
            er.dir = direction(rest.trim());
        } else if let Some(head) = line.strip_suffix('{') {
            body = Some(entity(&mut er, head.trim()));
        } else if let Some((left, from_card, identifying, to_card, right)) = link(line) {
            let (right, text) = match right.split_once(':') {
                Some((name, text)) => (name, label(text)),
                None => (right, String::new()),
            };
            let (left, right) = (left.trim(), right.trim());
            if left.is_empty() || right.is_empty() {
                continue;
            }
            let from = entity(&mut er, left);
            let to = entity(&mut er, right);
            er.links.push(Link {
                from,
                to,
                from_card,
                to_card,
                identifying,
                label: text,
            });
        } else if !line.contains(char::is_whitespace) && !line.starts_with(['%', '}']) {
            // 单独一行的实体名。
            entity(&mut er, line);
        }
    }
    (!er.entities.is_empty() && er.links.len() <= MAX_EDGES).then_some(er)
}

/// 找出关系的符号：返回左侧部分、左端基数、是否为实线、右端基数和右侧部分。
fn link(line: &str) -> Option<(&str, Card, bool, Card, &str)> {
    let b = line.as_bytes();
    for i in 2..b.len().saturating_sub(3) {
        let solid = &b[i..i + 2] == b"--";
        if !solid && &b[i..i + 2] != b".." {
            continue;
        }
        let from = match &b[i - 2..i] {
            b"|o" => Card::ZeroOrOne,
            b"||" => Card::ExactlyOne,
            b"}o" => Card::ZeroOrMore,
            b"}|" => Card::OneOrMore,
            _ => continue,
        };
        let to = match &b[i + 2..i + 4] {
            b"o|" => Card::ZeroOrOne,
            b"||" => Card::ExactlyOne,
            b"o{" => Card::ZeroOrMore,
            b"|{" => Card::OneOrMore,
            _ => continue,
        };
        return Some((&line[..i - 2], from, solid, to, &line[i + 4..]));
    }
    None
}

/// 解析一行属性：`类型 名字 [键[, 键]] ["说明"]`。
fn attribute(line: &str) -> Option<Attribute> {
    let (main, comment) = match line.find('"') {
        Some(i) => (&line[..i], unquote(line[i..].trim()).to_owned()),
        None => (line, String::new()),
    };
    let mut words = main.split_whitespace();
    let kind = words.next()?.to_owned();
    let name = words.next()?.to_owned();
    let keys: Vec<&str> = words
        .flat_map(|w| w.split(','))
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    Some(Attribute {
        kind,
        name,
        keys: keys.join(", "),
        comment,
    })
}
