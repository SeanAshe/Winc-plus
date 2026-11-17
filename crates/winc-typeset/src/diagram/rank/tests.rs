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
    let plan = layout(&f, &sizes, &[], &titles, 14.0, f.dir, &[]);
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

/// 并列的子图各有成员在不同的层，且有连线从一个子图连回另一个：两个子图的框不相交，各节点
/// 只在自己的子图的框内。
#[test]
fn sibling_subgraphs_spanning_different_ranks_stay_apart() {
    let (f, plan) = plan(
        "flowchart TB\n s --> g\n g --> r0\n r0 --> r1\n r1 --> w0\n r1 --> v0\n r1 --> w1\n\
             r1 --> v1\n g --> q\n q --> w0\n subgraph BIG[host]\n s\n g\n r0\n r1\n q\n end\n\
             subgraph WA[client A]\n w0\n w1\n end\n subgraph WB[client B]\n v0\n v1\n end\n\
             v0 --> w0\n",
    );
    assert_eq!(f.groups.len(), 3);
    for (g, b) in plan.boxes.iter().enumerate() {
        for (h, c) in plan.boxes.iter().enumerate().skip(g + 1) {
            assert!(!overlap(*b, *c), "{g} 和 {h} 的框重叠了：{b:?} {c:?}");
        }
        for (v, &(x, y)) in plan.centers.iter().enumerate() {
            let node = [x - 30.0, y - 12.0, x + 30.0, y + 12.0];
            let member = f.groups[g].members.contains(&v);
            assert_eq!(overlap(*b, node), member, "节点 {v} 和框 {g}");
        }
    }
}

/// 随机生成的流程图：子图的框要么互相包含，要么不相交，各节点只在自己所属的子图的框内。
///
/// 每张图有 5 到 12 个节点、2 到 4 个子图，一半的图把前两个子图再套进一个外层子图。随机数用
/// 固定种子的 xorshift 产生，结果可重复。
#[test]
fn random_flowcharts_keep_subgraph_boxes_apart() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let inside =
        |a: [f32; 4], b: [f32; 4]| a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3];
    for _ in 0..600 {
        let (nodes, groups) = (5 + next(8), 2 + next(3));
        let mut source = String::from("flowchart TB\n");
        for _ in 0..nodes + next(6) {
            let (a, b) = (next(nodes), next(nodes));
            if a != b {
                source += &format!(" n{a} --> n{b}\n");
            }
        }
        let home: Vec<usize> = (0..nodes).map(|_| next(groups + 1)).collect();
        let nested = next(2) == 0;
        for g in 0..groups {
            let members: Vec<String> = (0..nodes)
                .filter(|&v| home[v] == g)
                .map(|v| format!(" n{v}\n"))
                .collect();
            if g == 0 && nested {
                source += " subgraph OUTER\n";
            }
            if !members.is_empty() {
                source += &format!(" subgraph G{g}\n{} end\n", members.concat());
            }
            if g == 1 && nested {
                source += " end\n";
            }
        }
        let (f, plan) = plan(&source);
        for (g, b) in plan.boxes.iter().enumerate() {
            for c in plan.boxes.iter().skip(g + 1) {
                let apart = !overlap(*b, *c) || inside(*b, *c) || inside(*c, *b);
                assert!(apart, "子图的框相交：{b:?} {c:?}\n{source}");
            }
            for (v, &(x, y)) in plan.centers.iter().enumerate() {
                let node = [x - 30.0, y - 12.0, x + 30.0, y + 12.0];
                let member = f.groups[g].members.contains(&v);
                assert_eq!(overlap(*b, node), member, "节点 {v} 和框 {g}\n{source}");
            }
        }
    }
}
