//! 类图（`classDiagram`）的解析：类及其成员、类之间的关系、`namespace` 分组。
//!
//! 注释（`note`）、样式与点击事件被忽略。

use std::collections::HashMap;

use super::{Dir, Group, MAX_EDGES, direction, label, unquote};

/// 类图。
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    /// 绘制方向，默认从上往下。
    pub dir: Dir,
    /// 类，按第一次出现的先后排列。
    pub classes: Vec<ClassBox>,
    /// 关系，按源码中出现的先后排列。
    pub relations: Vec<Relation>,
    /// `namespace` 的外框，成员为 [`Class::classes`] 的序号。
    pub groups: Vec<Group>,
}

/// 类图中的一个类。
#[derive(Debug, Clone, PartialEq)]
pub struct ClassBox {
    /// 源码中写的类名。
    pub id: String,
    /// 显示的类名：泛型参数 `~T~` 显示为 `<T>`，`class A["文字"]` 显示为其中的文字。
    pub label: String,
    /// 类名上方的标注（`<<interface>>` 中的 `interface`）。
    pub annotation: Option<String>,
    /// 属性，按源码中的写法逐条排列。
    pub attributes: Vec<String>,
    /// 方法（含括号的成员），按源码中的写法逐条排列。
    pub methods: Vec<String>,
}

/// 关系线一端的标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// 没有标记。
    None,
    /// `<|`、`|>`：空心三角，表示继承或实现。
    Triangle,
    /// `*`：实心菱形，表示组合。
    Diamond,
    /// `o`：空心菱形，表示聚合。
    OpenDiamond,
    /// `<`、`>`：箭头，表示关联或依赖。
    Arrow,
    /// `()`：圆圈，表示提供的接口。
    Lollipop,
}

/// 两个类之间的关系。
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    /// 左侧的类在 [`Class::classes`] 中的序号。
    pub from: usize,
    /// 右侧的类在 [`Class::classes`] 中的序号。
    pub to: usize,
    /// 左端的标记。
    pub from_end: End,
    /// 右端的标记。
    pub to_end: End,
    /// 是否为虚线（`..`）。
    pub dotted: bool,
    /// 关系上的文字，可以为空。
    pub label: String,
    /// 左端的多重性（`"1"`），可以为空。
    pub from_card: String,
    /// 右端的多重性（`"*"`），可以为空。
    pub to_card: String,
}

/// 解析类图的正文；没有任何类或关系数超过上限时返回 `None`。
pub(super) fn parse(lines: &[&str]) -> Option<Class> {
    let mut b = Builder {
        class: Class {
            dir: Dir::Down,
            classes: Vec::new(),
            relations: Vec::new(),
            groups: Vec::new(),
        },
        ids: HashMap::new(),
        open: Vec::new(),
    };
    // 正在读取成员的类（`class A {` 之后、`}` 之前）。
    let mut body: Option<usize> = None;
    for &line in lines {
        if let Some(i) = body {
            if line == "}" {
                body = None;
            } else {
                b.member(i, line);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("direction ") {
            b.class.dir = direction(rest.trim());
        } else if let Some(rest) = line.strip_prefix("namespace ") {
            let title = rest.trim_end_matches('{').trim();
            b.class.groups.push(Group {
                title: label(title),
                members: Vec::new(),
            });
            b.open.push(b.class.groups.len() - 1);
        } else if line == "}" {
            b.open.pop();
        } else if let Some(rest) = line.strip_prefix("class ") {
            let (head, opens) = match rest.trim_end().strip_suffix('{') {
                Some(head) => (head.trim(), true),
                None => (rest.trim(), false),
            };
            let i = b.declare(head);
            if opens {
                body = Some(i);
            }
        } else if let Some(rest) = line.strip_prefix("<<") {
            // `<<interface>> 类名`。
            if let Some((note, name)) = rest.split_once(">>") {
                let i = b.declare(name.trim());
                b.class.classes[i].annotation = Some(note.trim().to_owned());
            }
        } else if !b.relation(line) {
            b.single_member(line);
        }
    }
    if b.class.classes.is_empty() || b.class.relations.len() > MAX_EDGES {
        return None;
    }
    for g in &mut b.class.groups {
        g.members.sort_unstable();
        g.members.dedup();
    }
    b.class.groups.retain(|g| !g.members.is_empty());
    Some(b.class)
}

struct Builder {
    class: Class,
    /// 类名到 [`Class::classes`] 中序号的对应关系。
    ids: HashMap<String, usize>,
    /// 尚未遇到 `}` 的 `namespace`，内容为 [`Class::groups`] 的序号。
    open: Vec<usize>,
}

impl Builder {
    /// 名字是 `id` 的类，没有时新建一个；所在的 `namespace` 记入成员。
    fn class(&mut self, id: &str) -> usize {
        let i = match self.ids.get(id) {
            Some(&i) => i,
            None => {
                self.class.classes.push(ClassBox {
                    id: id.to_owned(),
                    label: generic(id),
                    annotation: None,
                    attributes: Vec::new(),
                    methods: Vec::new(),
                });
                self.ids.insert(id.to_owned(), self.class.classes.len() - 1);
                self.class.classes.len() - 1
            }
        };
        for &g in &self.open {
            self.class.groups[g].members.push(i);
        }
        i
    }

    /// `class` 声明后的类名部分：`A`、`A~T~`、`A["文字"]`，可带 `:::样式`。
    fn declare(&mut self, head: &str) -> usize {
        let head = head.split(":::").next().unwrap_or(head).trim();
        let (name, shown) = match head.split_once('[') {
            Some((name, rest)) => (name.trim(), rest.strip_suffix(']').map(label)),
            None => (head, None),
        };
        let id = name.split('~').next().unwrap_or(name);
        let i = self.class(id);
        let class = &mut self.class.classes[i];
        if let Some(shown) = shown {
            class.label = shown;
        } else if name.contains('~') {
            class.label = generic(name);
        }
        i
    }

    /// 类体中的一行：标注或成员。
    fn member(&mut self, i: usize, line: &str) {
        if let Some(note) = line.strip_prefix("<<").and_then(|r| r.strip_suffix(">>")) {
            self.class.classes[i].annotation = Some(note.trim().to_owned());
            return;
        }
        let text = member(line);
        if text.is_empty() {
            return;
        }
        let class = &mut self.class.classes[i];
        if text.contains('(') {
            class.methods.push(text);
        } else {
            class.attributes.push(text);
        }
    }

    /// `类名 : 成员` 这样单独一行声明的成员。
    fn single_member(&mut self, line: &str) {
        let Some((name, rest)) = line.split_once(':') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return;
        }
        let i = self.class(name.split('~').next().unwrap_or(name));
        self.member(i, rest.trim());
    }

    /// 解析一行关系：`A "1" *-- "n" B : 文字`；不是关系时返回 `false`。
    fn relation(&mut self, line: &str) -> bool {
        let (main, text) = match line.split_once(':') {
            Some((main, text)) => (main, label(text)),
            None => (line, String::new()),
        };
        let Some(at) = link_at(main) else {
            return false;
        };
        let dotted = &main[at..at + 2] == "..";
        let (left, from_end) = left_end(&main[..at]);
        let (right, to_end) = right_end(&main[at + 2..]);
        let (from_name, from_card) = split_card(left.trim(), true);
        let (to_name, to_card) = split_card(right.trim(), false);
        if from_name.is_empty() || to_name.is_empty() {
            return false;
        }
        let from = self.class(from_name.split('~').next().unwrap_or(from_name));
        let to = self.class(to_name.split('~').next().unwrap_or(to_name));
        self.class.relations.push(Relation {
            from,
            to,
            from_end,
            to_end,
            dotted,
            label: text,
            from_card,
            to_card,
        });
        true
    }
}

/// 关系线（`--` 或 `..`）在 `s` 中的位置；引号内的不算。
fn link_at(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut quoted = false;
    for i in 0..bytes.len().saturating_sub(1) {
        match bytes[i] {
            b'"' => quoted = !quoted,
            b'-' | b'.' if !quoted && bytes[i + 1] == bytes[i] => return Some(i),
            _ => {}
        }
    }
    None
}

/// 关系线左边：去掉紧贴线的标记，返回剩下的部分和标记。
fn left_end(s: &str) -> (&str, End) {
    for (mark, end) in [
        ("<|", End::Triangle),
        ("()", End::Lollipop),
        ("*", End::Diamond),
        ("<", End::Arrow),
    ] {
        if let Some(rest) = s.strip_suffix(mark) {
            return (rest, end);
        }
    }
    // `o` 只有与前面的类名或多重性隔开时才是标记，否则是类名的最后一个字母。
    match s.strip_suffix('o') {
        Some(rest) if rest.ends_with([' ', '"']) => (rest, End::OpenDiamond),
        _ => (s, End::None),
    }
}

/// 关系线右边：去掉紧贴线的标记，返回剩下的部分和标记。
fn right_end(s: &str) -> (&str, End) {
    for (mark, end) in [
        ("|>", End::Triangle),
        ("()", End::Lollipop),
        ("*", End::Diamond),
        (">", End::Arrow),
    ] {
        if let Some(rest) = s.strip_prefix(mark) {
            return (rest, end);
        }
    }
    match s.strip_prefix('o') {
        Some(rest) if rest.starts_with([' ', '"']) => (rest, End::OpenDiamond),
        _ => (s, End::None),
    }
}

/// 把 `A "1"`（`card_after` 为真）或 `"1" B` 拆成类名和多重性。
fn split_card(s: &str, card_after: bool) -> (&str, String) {
    let quoted = if card_after {
        s.strip_suffix('"')
            .and_then(|r| r.rfind('"').map(|i| (&r[..i], &r[i + 1..])))
    } else {
        s.strip_prefix('"')
            .and_then(|r| r.find('"').map(|i| (&r[i + 1..], &r[..i])))
    };
    match quoted {
        Some((name, card)) => (name.trim(), card.trim().to_owned()),
        None => (s, String::new()),
    }
}

/// 成员的显示文字：去掉静态（`$`）与抽象（`*`）标记，泛型参数 `~T~` 显示为 `<T>`。
fn member(s: &str) -> String {
    let s = unquote(s.trim());
    let s = s.strip_suffix(['$', '*']).unwrap_or(s);
    generic(s.trim())
}

/// 把成对的 `~` 换成 `<`、`>`：`List~int~` 显示为 `List<int>`。
fn generic(s: &str) -> String {
    let mut open = true;
    s.chars()
        .map(|c| {
            if c == '~' {
                open = !open;
                if open { '>' } else { '<' }
            } else {
                c
            }
        })
        .collect()
}
