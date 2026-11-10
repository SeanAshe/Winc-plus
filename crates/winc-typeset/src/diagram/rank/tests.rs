//! 流程图分层布局的测试：子图框不与其他节点重叠、嵌套子图互相包含。

use winc_preview::markdown::mermaid::{self, Diagram};

use super::*;

/// 节点宽高取 60 × 24 逻辑像素，子图标题取 12 逻辑像素高，便于计算重叠。
fn plan(source: &str) -> (Flow, Plan) {
    let Some(Diagram::Flow(f)) = mermaid::parse(source) else {
        panic!("不是流程图");
    };
    let sizes = vec![(60.0, 24.0); f.nodes.len()];
    let titles = vec![12.0; f.groups.len()];
    let plan = layout(&f, &sizes, &[], &titles, 14.0, f.dir);
    (f, plan)
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

#[test]
fn subgraph_boxes_keep_other_nodes_out() {
    let (f, plan) = plan(
        "flowchart TB\n U --> S\n U --> A\n N --> S\n A --> P --> I\n I --> M & T & F & D & R & L\n\
             subgraph FE[前端]\n N\n end\n subgraph GO[运行时]\n S\n A\n P\n I\n end\n\
             subgraph EXT[外部]\n M\n T\n F\n end\n subgraph DS[存储]\n D\n R\n L\n end\n",
    );
    assert_eq!(f.groups.len(), 4);
    for (g, b) in plan.boxes.iter().enumerate() {
        for (h, c) in plan.boxes.iter().enumerate().skip(g + 1) {
            assert!(!overlap(*b, *c), "{g} 和 {h} 的框重叠了");
        }
        for (v, &(x, y)) in plan.centers.iter().enumerate() {
            let node = [x - 30.0, y - 12.0, x + 30.0, y + 12.0];
            let member = f.groups[g].members.contains(&v);
            assert_eq!(overlap(*b, node), member, "节点 {v} 和框 {g}");
        }
    }
}

#[test]
fn nested_subgraphs_sit_inside_each_other() {
    let (_, plan) = plan(
        "flowchart LR\n subgraph 外\n subgraph 内\n a --> b\n end\n b --> c\n end\n x --> a\n c --> y\n",
    );
    let (outer, inner) = (plan.boxes[0], plan.boxes[1]);
    assert!(outer[0] < inner[0] && outer[1] < inner[1]);
    assert!(outer[2] > inner[2] && outer[3] > inner[3]);
}
