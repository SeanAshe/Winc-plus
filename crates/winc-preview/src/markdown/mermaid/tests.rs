//! mermaid 解析的测试：流程图的形状与连线、状态图、时序图的事件、饼图、类图、实体关系图、
//! 思维导图、Git 提交图、甘特图，以及不受支持的图。

use super::*;

fn flow(source: &str) -> Flow {
    match parse(source) {
        Some(Diagram::Flow(flow)) => flow,
        other => panic!("{other:?}"),
    }
}

#[test]
fn flowchart_nodes_shapes_and_edges() {
    let f = flow(
        "flowchart LR\n  A[开始] --> B{判断?}\n  B -->|是| C([结束])\n  B -- 否 --> D[(库)] & E((圆))\n  C -.-> A\n  D ==> E --- F>旗]\n  %% 注释\n  subgraph 组 [一组]\n    G[/斜/] --x H[\\梯/]\n  end\n",
    );
    assert_eq!(f.dir, Dir::Right);
    let shape = |id: &str| f.nodes.iter().find(|n| n.id == id).unwrap().shape;
    let label = |id: &str| f.nodes.iter().find(|n| n.id == id).unwrap().label.clone();
    assert_eq!(label("A"), "开始");
    assert_eq!(shape("B"), Shape::Diamond);
    assert_eq!(shape("C"), Shape::Stadium);
    assert_eq!(shape("D"), Shape::Cylinder);
    assert_eq!(shape("E"), Shape::Circle);
    assert_eq!(shape("F"), Shape::Asym);
    assert_eq!(shape("G"), Shape::Lean);
    assert_eq!(shape("H"), Shape::TrapezoidDown);
    let edge = |a: &str, b: &str| {
        f.edges
            .iter()
            .find(|e| f.nodes[e.from].id == a && f.nodes[e.to].id == b)
            .unwrap_or_else(|| panic!("{a}->{b}: {:?}", f.edges))
            .clone()
    };
    assert_eq!(edge("B", "C").label, "是");
    assert_eq!(edge("B", "D").label, "否");
    assert_eq!(edge("B", "E").label, "否");
    assert_eq!(edge("C", "A").line, Line::Dotted);
    assert_eq!(edge("D", "E").line, Line::Thick);
    assert_eq!(edge("E", "F").head, Head::None);
    assert_eq!(edge("G", "H").head, Head::Cross);
    assert_eq!(f.groups.len(), 1);
    assert_eq!(f.groups[0].title, "一组");
    assert_eq!(f.groups[0].members.len(), 2);
}

#[test]
fn state_diagrams_become_flows() {
    let f = flow(
        "stateDiagram-v2\n  [*] --> 空闲\n  空闲 --> 运行 : 开始\n  运行 --> [*]\n  state 判断 <<choice>>\n",
    );
    assert_eq!(f.nodes[0].shape, Shape::Start);
    assert_eq!(f.nodes[1].shape, Shape::Round);
    assert_eq!(f.edges[1].label, "开始");
    assert_eq!(f.nodes[3].shape, Shape::End);
    assert_eq!(f.nodes[4].shape, Shape::Diamond);
}

#[test]
fn notes_keep_their_actors_names() {
    // 回归：`note` 后面多一个空格且参与者名是中文时，按小写副本的长度切分原文会切在字符中间，
    // 导致 panic。
    let Some(Diagram::Sequence(s)) = parse(
        "sequenceDiagram\n  note  over 中文: x\n  NOTE Left Of Bob: y\n  Note over 中文,Bob: z\n",
    ) else {
        panic!();
    };
    let names: Vec<&str> = s.actors.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(names, ["中文", "Bob"]);
    let notes: Vec<_> = s
        .events
        .iter()
        .map(|e| match e {
            Event::Note { side, actors, text } => (*side, *actors, text.as_str()),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        notes,
        [
            (Side::Over, (0, 0), "x"),
            (Side::Left, (1, 1), "y"),
            (Side::Over, (0, 1), "z"),
        ]
    );
}

#[test]
fn sequence_messages_notes_and_blocks() {
    let Some(Diagram::Sequence(s)) = parse(
        "sequenceDiagram\n  autonumber\n  participant A as 张三\n  actor B\n  A->>+B: 你好\n  B-->>-A: 好\n  Note right of B: 想一想\n  loop 每分钟\n    A-)B: 心跳\n  end\n  A-xC: 断了\n",
    ) else {
        panic!();
    };
    assert!(s.numbered);
    assert_eq!(s.actors.len(), 3);
    assert_eq!(s.actors[0].label, "张三");
    assert!(s.actors[1].person);
    assert!(matches!(
        &s.events[0],
        Event::Message { from: 0, to: 1, text, dotted: false, arrow: Arrow::Filled, activate: Some(true), .. } if text == "你好"
    ));
    assert!(matches!(
        &s.events[1],
        Event::Message {
            dotted: true,
            activate: Some(false),
            ..
        }
    ));
    assert!(matches!(
        &s.events[2],
        Event::Note {
            side: Side::Right,
            actors: (1, 1),
            ..
        }
    ));
    assert!(
        matches!(&s.events[3], Event::Begin(kind, label) if kind == "loop" && label == "每分钟")
    );
    assert!(matches!(
        &s.events[4],
        Event::Message {
            arrow: Arrow::Open,
            ..
        }
    ));
    assert_eq!(s.events[5], Event::End);
    assert!(matches!(
        &s.events[6],
        Event::Message {
            arrow: Arrow::Cross,
            to: 2,
            ..
        }
    ));
}

#[test]
fn pies_and_unsupported() {
    let Some(Diagram::Pie(p)) = parse("pie showData title 宠物\n  \"狗\" : 386\n  \"猫\" : 85.5\n")
    else {
        panic!();
    };
    assert_eq!(p.title.as_deref(), Some("宠物"));
    assert!(p.show_data);
    assert_eq!(
        p.slices,
        [("狗".to_owned(), 386.0), ("猫".to_owned(), 85.5)]
    );
    assert_eq!(
        parse("journey\n  title 步骤\n"),
        Some(Diagram::Unsupported("journey".into()))
    );
    assert_eq!(parse("不是图"), None);
}

#[test]
fn too_many_edges_are_left_as_source() {
    // `A & B --> C & D` 生成两组节点数之积条连线：20 × 20 = 400 条低于上限，正常绘制；
    // 30 × 30 = 900 条超过上限，不绘制，按代码块显示原文。
    let chain = |n: usize| {
        let group = |side: &str| {
            (0..n)
                .map(|i| format!("{side}{i}"))
                .collect::<Vec<_>>()
                .join(" & ")
        };
        format!("graph LR\n{} --> {}", group("a"), group("b"))
    };
    let Some(Diagram::Flow(flow)) = parse(&chain(20)) else {
        panic!("400 条连线应当照常画");
    };
    assert_eq!((flow.nodes.len(), flow.edges.len()), (40, 400));
    assert_eq!(parse(&chain(30)), None);
}

#[test]
fn class_members_relations_and_namespaces() {
    let Some(Diagram::Class(c)) = parse(
        "classDiagram\n  direction LR\n  class Shape {\n    <<interface>>\n    +area() double$\n    \
         -String name\n  }\n  class Box~T~\n  Box : +List~T~ items\n  Shape <|.. Circle\n  \
         Box \"1\" *-- \"many\" Item : 包含\n  Foo--Bar\n  Pool o-- Item\n  namespace 形状 {\n    \
         class Square\n  }\n",
    ) else {
        panic!();
    };
    assert_eq!(c.dir, Dir::Right);
    let class = |id: &str| c.classes.iter().find(|k| k.id == id).unwrap();
    let shape = class("Shape");
    assert_eq!(shape.annotation.as_deref(), Some("interface"));
    assert_eq!(shape.methods, ["+area() double"]);
    assert_eq!(shape.attributes, ["-String name"]);
    assert_eq!(class("Box").label, "Box<T>");
    assert_eq!(class("Box").attributes, ["+List<T> items"]);
    let rel = |a: &str, b: &str| {
        c.relations
            .iter()
            .find(|r| c.classes[r.from].id == a && c.classes[r.to].id == b)
            .unwrap_or_else(|| panic!("{a}-{b}: {:?}", c.relations))
    };
    let r = rel("Shape", "Circle");
    assert_eq!(
        (r.from_end, r.to_end, r.dotted),
        (End::Triangle, End::None, true)
    );
    let r = rel("Box", "Item");
    assert_eq!((r.from_end, r.from_card.as_str()), (End::Diamond, "1"));
    assert_eq!((r.to_card.as_str(), r.label.as_str()), ("many", "包含"));
    // 紧贴连线的 `o` 是类名的一部分，隔开时才是聚合标记。
    assert_eq!(rel("Foo", "Bar").from_end, End::None);
    assert_eq!(rel("Pool", "Item").from_end, End::OpenDiamond);
    let square = c.classes.iter().position(|k| k.id == "Square").unwrap();
    assert_eq!(c.groups.len(), 1);
    assert_eq!(
        (c.groups[0].title.as_str(), &c.groups[0].members[..]),
        ("形状", &[square][..])
    );
}

#[test]
fn er_entities_attributes_and_cardinalities() {
    let Some(Diagram::Er(er)) = parse(
        "erDiagram\n  SHELF ||--o{ BOOK : holds\n  BOOK }|..|| LINE-ITEM : \"listed in\"\n  \
         BOOK {\n    string isbn PK \"编号\"\n    int shelf_id FK, UK\n  }\n",
    ) else {
        panic!();
    };
    let ids: Vec<&str> = er.entities.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["SHELF", "BOOK", "LINE-ITEM"]);
    let l = &er.links[0];
    assert_eq!(
        (l.from_card, l.to_card, l.identifying),
        (Card::ExactlyOne, Card::ZeroOrMore, true)
    );
    assert_eq!(l.label, "holds");
    let l = &er.links[1];
    assert_eq!(
        (l.from_card, l.to_card, l.identifying),
        (Card::OneOrMore, Card::ExactlyOne, false)
    );
    assert_eq!(l.label, "listed in");
    let book = &er.entities[1].attributes;
    assert_eq!(
        (book[0].kind.as_str(), book[0].name.as_str()),
        ("string", "isbn")
    );
    assert_eq!(
        (book[0].keys.as_str(), book[0].comment.as_str()),
        ("PK", "编号")
    );
    assert_eq!(book[1].keys, "FK, UK");
}

#[test]
fn mindmap_levels_follow_indentation() {
    let Some(Diagram::Mindmap(m)) = parse(
        "mindmap\n  root((中心))\n    A[方形]\n      A1\n      A2(圆角)\n    B))爆炸((\n      \
         ::icon(fa fa-book)\n      B1{{六边}}\n",
    ) else {
        panic!();
    };
    let shown: Vec<(&str, MindShape, Option<usize>)> = m
        .nodes
        .iter()
        .map(|n| (n.label.as_str(), n.shape, n.parent))
        .collect();
    assert_eq!(
        shown,
        [
            ("中心", MindShape::Circle, None),
            ("方形", MindShape::Rect, Some(0)),
            ("A1", MindShape::Plain, Some(1)),
            ("圆角", MindShape::Round, Some(1)),
            ("爆炸", MindShape::Bang, Some(0)),
            ("六边", MindShape::Hexagon, Some(4)),
        ]
    );
}

#[test]
fn git_graph_branches_merges_and_order() {
    let Some(Diagram::Git(g)) = parse(
        "gitGraph TB:\n  commit id: \"a\"\n  branch dev order: 2\n  branch fix order: 1\n  \
         commit tag: \"v1\"\n  checkout dev\n  commit type: HIGHLIGHT\n  checkout main\n  \
         merge fix\n  cherry-pick id: \"a\"\n",
    ) else {
        panic!();
    };
    assert_eq!(g.dir, Dir::Down);
    assert_eq!(g.branches, ["main", "fix", "dev"]);
    let c = &g.commits;
    assert_eq!((c[0].id.as_deref(), c[0].branch), (Some("a"), 0));
    // `fix` 从 `main` 的最新提交分出。
    assert_eq!(
        (c[1].branch, &c[1].parents[..], c[1].tag.as_deref()),
        (1, &[0][..], Some("v1"))
    );
    assert_eq!((c[2].branch, c[2].kind), (2, CommitKind::Highlight));
    assert_eq!(
        (c[3].kind, &c[3].parents[..]),
        (CommitKind::Merge, &[0, 1][..])
    );
    assert_eq!((c[4].kind, c[4].picked), (CommitKind::CherryPick, Some(0)));
}

#[test]
fn gantt_tasks_resolve_dates_durations_and_dependencies() {
    let Some(Diagram::Gantt(g)) = parse(
        "gantt\n  title 计划\n  dateFormat YYYY-MM-DD\n  section 准备\n  调研 :done, a1, \
         2024-01-01, 3d\n  设计 :active, a2, after a1, 1w\n  section 实现\n  编码 :crit, a3, \
         2024-01-15, 2024-01-20\n  验收 :milestone, after a2, 0d\n  联调 :2024-01-12, until a3\n  \
         收尾 :2d\n",
    ) else {
        panic!();
    };
    assert_eq!(g.title.as_deref(), Some("计划"));
    assert_eq!(g.sections, ["准备", "实现"]);
    let day = |t: i64| format_time(t, "%Y-%m-%d");
    let spans: Vec<(String, String, Option<usize>)> = g
        .tasks
        .iter()
        .map(|t| (day(t.start), day(t.end), t.section))
        .collect();
    let at = |a: &str, b: &str, s: usize| (a.to_owned(), b.to_owned(), Some(s));
    assert_eq!(
        spans,
        [
            at("2024-01-01", "2024-01-04", 0),
            at("2024-01-04", "2024-01-11", 0),
            at("2024-01-15", "2024-01-20", 1),
            at("2024-01-11", "2024-01-11", 1),
            // `until a3` 结束于 `a3` 的开始时间。
            at("2024-01-12", "2024-01-15", 1),
            // 没有写开始时间时接在上一项任务之后。
            at("2024-01-15", "2024-01-17", 1),
        ]
    );
    assert!(g.tasks[0].done && g.tasks[1].active && g.tasks[2].crit && g.tasks[3].milestone);
    assert_eq!(
        format_time(month_start(g.tasks[2].start, 1), "%b %e"),
        "Feb 1"
    );
}
