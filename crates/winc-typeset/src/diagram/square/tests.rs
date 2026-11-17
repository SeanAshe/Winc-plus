//! 直角走线的测试：连线只有横线和竖线，上下对着的节点之间是一条直线。

use winc_preview::markdown::mermaid::{self, Diagram};

use super::super::rank::layout;
use super::*;

/// 排好 `source` 的布局并走线，返回各条连线经过的拐角之间的线段（屏幕坐标）。节点宽高取
/// 60 × 24 逻辑像素，字号 14。
fn lines(source: &str, ports: Ports) -> Vec<Vec<Point>> {
    let Some(Diagram::Flow(flow)) = mermaid::parse(source) else {
        panic!("不是流程图");
    };
    let sizes = vec![(60.0, 24.0); flow.nodes.len()];
    let titles = vec![12.0; flow.groups.len()];
    let mut plan = layout(&flow, &sizes, &[], &titles, 14.0, flow.dir, &[]);
    let (mut squared, room) = square_routes(&plan, &flow, &sizes, 14.0, ports);
    if room.iter().any(|&more| more > 0.5) {
        plan = layout(&flow, &sizes, &[], &titles, 14.0, flow.dir, &room);
        (squared, _) = square_routes(&plan, &flow, &sizes, 14.0, ports);
    }
    squared.into_iter().flatten().map(|line| line.0).collect()
}

/// 折线是否只在圆角处改变方向：出发和到达的一段沿 `vertical` 指的方向，其余各点要么与前一点
/// 同列或同行，要么在一段圆角上（相邻两点相距不超过圆角的半径）。
fn square(line: &[Point], vertical: bool) -> bool {
    let along = |a: Point, b: Point| {
        if vertical {
            (a.0 - b.0).abs()
        } else {
            (a.1 - b.1).abs()
        }
    };
    let n = line.len();
    let ends = along(line[0], line[1]) < 1.5 && along(line[n - 1], line[n - 2]) < 1.5;
    let steps = line.windows(2).all(|w| {
        let (dx, dy) = ((w[0].0 - w[1].0).abs(), (w[0].1 - w[1].1).abs());
        dx < 1.5 || dy < 1.5 || dx.hypot(dy) <= 14.0 * RADIUS * 2.0
    });
    ends && steps
}

#[test]
fn edges_run_along_the_axes_in_every_direction() {
    for dir in ["TB", "BT", "LR", "RL"] {
        let source = format!(
            "flowchart {dir}\n a --> b\n a --> c\n a --> d\n b --> e\n c --> e\n d --> e\n\
             e --> a\n a --> f\n subgraph G\n b\n c\n end\n"
        );
        let vertical = matches!(dir, "TB" | "BT");
        for ports in [Ports::Shared, Ports::Spread] {
            let lines = lines(&source, ports);
            assert_eq!(lines.len(), 8, "{dir}");
            for line in &lines {
                assert!(square(line, vertical), "{dir}: {line:?}");
            }
        }
    }
}

#[test]
fn a_node_directly_below_another_is_reached_by_a_straight_line() {
    // b 比 a 宽，二者的中心不在同一列：接点移到对方的正上方，连线不拐弯。
    let lines = lines(
        "flowchart TB\n a --> b\n b --> c\n b --> d\n",
        Ports::Shared,
    );
    assert_eq!(lines[0].len(), 2, "{:?}", lines[0]);
    assert!((lines[0][0].0 - lines[0][1].0).abs() < 1.5);
}

#[test]
fn spread_ports_give_each_edge_its_own_place_on_a_side() {
    let lines = lines(
        "flowchart TB\n a --> b\n a --> c\n a --> d\n",
        Ports::Spread,
    );
    let mut starts: Vec<f32> = lines.iter().map(|line| line[0].0).collect();
    starts.sort_by(f32::total_cmp);
    assert!(starts.windows(2).all(|w| w[1] - w[0] > 4.0), "{starts:?}");
    // 共用接点时三条线从同一点出发。
    let shared = self::lines(
        "flowchart TB\n a --> b\n a --> c\n a --> d\n",
        Ports::Shared,
    );
    assert!(
        shared
            .iter()
            .all(|line| (line[0].0 - shared[0][0].0).abs() < 0.5)
    );
}
