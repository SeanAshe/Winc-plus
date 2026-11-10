//! 流程图的分层布局（见上一层的说明）：去环、分层、插虚节点、排每层的顺序、定坐标。
//!
//! 坐标都是逻辑像素，`s` 是排版用的字号。
//!
//! 子图（`subgraph`）：成员在每一层里排在一起（按包含关系一层层排），子图跨过的每一层左右
//! 各插一个「边框点」，框外的点排不进来；边框点按子图对齐成一条竖线，框就不会盖住别的节点。
//! 层与层之间按子图开始、结束的地方多留出框的边距和标题。

use winc_preview::markdown::mermaid::{Dir, Flow};

use super::Label;

/// 流程图排好的位置，坐标在「从上往下」的方向里：`cross` 是横的（同一层里的先后），
/// `along` 是竖的（层）。
pub(crate) struct Plan {
    /// 各节点的中心。
    pub(crate) centers: Vec<(f32, f32)>,
    /// 各连线经过的点（中心到中心）和文字的位置；连到自己的为空的路线。
    pub(crate) routes: Vec<Option<Route>>,
    /// 各子图的框（左、上、右、下），和 [`Flow::groups`] 一一对应。
    pub(crate) boxes: Vec<[f32; 4]>,
    /// 横向总宽（逻辑像素，同一层里各点的跨度）。
    pub(crate) cross: f32,
    /// 纵向总长（逻辑像素，各层叠起来的高度）。
    pub(crate) along: f32,
}

/// 一条连线排好的路线：经过的点和文字的位置（逻辑像素，坐标系同 [`Plan`]）。
pub(crate) struct Route {
    /// 从起点到终点依次经过的点；连到自己的线为空。
    pub(crate) points: Vec<(f32, f32)>,
    /// 连线文字的中心；没有文字时为 `None`。
    pub(crate) label: Option<(f32, f32)>,
    /// 连线文字所在的最内层子图，决定文字底色；没有文字时取两端共同的最内层子图；不在任何子图
    /// 中时为 `None`。
    pub(crate) group: Option<usize>,
}

impl Plan {
    /// 按图的方向换成屏幕坐标。
    pub(crate) fn place(&self, (c, a): (f32, f32), dir: Dir) -> (f32, f32) {
        match dir {
            Dir::Down => (c, a),
            Dir::Up => (c, self.along - a),
            Dir::Right => (a, c),
            Dir::Left => (self.along - a, c),
        }
    }
}

/// 布局里的点是什么。
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// 图里写着的节点。
    Real,
    /// 跨层的连线经过的虚节点。
    Dummy,
    /// 子图在这一层的左、右边框。
    Left(usize),
    Right(usize),
}

/// 分层布局中的一个点。
struct Vertex {
    /// 横向（同层里）占多宽、竖向占多高。
    cross: f32,
    along: f32,
    /// 层号，从 0 起，从上往下（或从左往右）。
    rank: usize,
    /// 上一层、下一层连着的点。
    up: Vec<usize>,
    down: Vec<usize>,
    /// 这个点是真节点、虚节点还是子图的边框点。
    kind: Kind,
    /// 在哪个子图里（最内层）。
    group: Option<usize>,
}

impl Vertex {
    fn border(&self) -> bool {
        matches!(self.kind, Kind::Left(_) | Kind::Right(_))
    }
}

/// 一条边经过的真节点与虚节点；去除环时反转的边按反转后的方向排列。
struct Chain {
    /// 边在 `Flow::edges` 中的下标。
    edge: usize,
    /// 从起点到终点依次经过的点在 `vs` 中的下标。
    points: Vec<usize>,
    /// 中间一层的虚节点在 `vs` 中的下标，连线有文字时放置文字；没有中间层时为 `None`。
    label_at: Option<usize>,
    /// 连线文字所在的最内层子图，同 [`Route::group`]。
    group: Option<usize>,
}

/// 子图之间的包含关系。
struct Tree {
    /// 各子图直接的父级；最外层子图为 `None`。
    parent: Vec<Option<usize>>,
    /// 各子图直接包含的子图；最后一项是不在任何子图里的。
    children: Vec<Vec<usize>>,
    /// 各节点在哪个子图里（最内层）。
    home: Vec<Option<usize>>,
}

impl Tree {
    /// 按成员关系建出子图的包含树。
    fn new(f: &Flow) -> Tree {
        let g = f.groups.len();
        let len = |h: usize| f.groups[h].members.len();
        let inside = |a: usize, b: usize| {
            f.groups[a]
                .members
                .iter()
                .all(|m| f.groups[b].members.binary_search(m).is_ok())
        };
        // 一样大、成员相同的两个子图，先写的算外层。
        let outer = |b: usize, a: usize| len(b) > len(a) || (len(b) == len(a) && b < a);
        let parent: Vec<Option<usize>> = (0..g)
            .map(|a| {
                (0..g)
                    .filter(|&b| b != a && outer(b, a) && inside(a, b))
                    .min_by_key(|&b| (len(b), std::cmp::Reverse(b)))
            })
            .collect();
        let mut children = vec![Vec::new(); g + 1];
        for (a, p) in parent.iter().enumerate() {
            children[p.unwrap_or(g)].push(a);
        }
        let home = (0..f.nodes.len())
            .map(|v| {
                (0..g)
                    .filter(|&h| f.groups[h].members.binary_search(&v).is_ok())
                    .min_by_key(|&h| (len(h), std::cmp::Reverse(h)))
            })
            .collect();
        Tree {
            parent,
            children,
            home,
        }
    }

    /// 从最外面到 `g` 的一串子图。
    fn path(&self, g: Option<usize>) -> Vec<usize> {
        let mut path: Vec<usize> = std::iter::successors(g, |&h| self.parent[h]).collect();
        path.reverse();
        path
    }

    /// 包含 `a`、`b` 的最内层共同子图。
    fn common(&self, a: Option<usize>, b: Option<usize>) -> Option<usize> {
        let (pa, pb) = (self.path(a), self.path(b));
        pa.iter()
            .zip(&pb)
            .take_while(|(x, y)| x == y)
            .last()
            .map(|(x, _)| *x)
    }
}

/// 一层的顺序：按 `key` 排，同一个子图里的排在一起，左右是它的边框点。
fn arrange(layer: &[usize], key: &[f32], vs: &[Vertex], tree: &Tree) -> Vec<usize> {
    let root = tree.children.len() - 1;
    let mut direct = vec![Vec::new(); root + 1];
    let mut borders = vec![(None, None); root];
    for &v in layer {
        match vs[v].kind {
            Kind::Left(g) => borders[g].0 = Some(v),
            Kind::Right(g) => borders[g].1 = Some(v),
            _ => direct[vs[v].group.unwrap_or(root)].push(v),
        }
    }
    fn block(
        g: usize,
        key: &[f32],
        tree: &Tree,
        direct: &[Vec<usize>],
        borders: &[(Option<usize>, Option<usize>)],
    ) -> Option<(f32, Vec<usize>)> {
        let mut items: Vec<(f32, Vec<usize>)> =
            direct[g].iter().map(|&v| (key[v], vec![v])).collect();
        for &h in &tree.children[g] {
            items.extend(block(h, key, tree, direct, borders));
        }
        let (left, right) = borders.get(g).copied().unwrap_or((None, None));
        if items.is_empty() && left.is_none() {
            return None;
        }
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        let k = if items.is_empty() {
            left.map_or(0.0, |v| key[v])
        } else {
            items.iter().map(|i| i.0).sum::<f32>() / items.len() as f32
        };
        let mut out: Vec<usize> = left.into_iter().collect();
        out.extend(items.into_iter().flat_map(|i| i.1));
        out.extend(right);
        Some((k, out))
    }
    block(root, key, tree, &direct, &borders).map_or_else(Vec::new, |b| b.1)
}

/// 排流程图的分层布局，返回各点的位置、连线的路线和子图的框。
///
/// `sizes` 是各节点在屏幕上的宽高，`labels` 是各连线上排好的文字，`titles` 是各子图标题的
/// 高度（都是逻辑像素）；`s` 是字号，`dir` 是图的方向。
pub(crate) fn layout(
    f: &Flow,
    sizes: &[(f32, f32)],
    labels: &[Label],
    titles: &[f32],
    s: f32,
    dir: Dir,
) -> Plan {
    let n = f.nodes.len();
    let vertical = matches!(dir, Dir::Down | Dir::Up);
    let flip = |(w, h): (f32, f32)| if vertical { (w, h) } else { (h, w) };
    let tree = Tree::new(f);
    let groups = f.groups.len();
    // 1. 去掉环：深度优先，回头的边反过来。
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, e) in f.edges.iter().enumerate() {
        if e.from != e.to {
            out_edges[e.from].push(k);
        }
    }
    let mut reversed = vec![false; f.edges.len()];
    let mut state = vec![0u8; n];
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        state[root] = 1;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if let Some(&k) = out_edges[v].get(*next) {
                *next += 1;
                let w = f.edges[k].to;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[k] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let dag: Vec<(usize, usize, usize, usize)> = f
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from != e.to)
        .map(|(k, e)| {
            let (a, b) = if reversed[k] {
                (e.to, e.from)
            } else {
                (e.from, e.to)
            };
            (k, a, b, e.span.max(1) * 2)
        })
        .collect();
    // 2. 分层：最长路径；只有出边的节点再往下挪，挨着它连到的节点。
    let mut indegree = vec![0usize; n];
    for &(_, _, b, _) in &dag {
        indegree[b] += 1;
    }
    let mut order: Vec<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
    let mut i = 0;
    let mut left = indegree.clone();
    while i < order.len() {
        let v = order[i];
        for &(_, a, b, _) in &dag {
            if a == v {
                left[b] -= 1;
                if left[b] == 0 {
                    order.push(b);
                }
            }
        }
        i += 1;
    }
    let mut rank = vec![0usize; n];
    for &v in &order {
        for &(_, a, b, len) in &dag {
            if a == v {
                rank[b] = rank[b].max(rank[v] + len);
            }
        }
    }
    for &v in order.iter().rev() {
        if indegree[v] == 0 {
            let lowest = dag
                .iter()
                .filter(|e| e.1 == v)
                .map(|&(_, _, b, len)| rank[b].saturating_sub(len))
                .min();
            if let Some(lowest) = lowest {
                rank[v] = rank[v].max(lowest);
            }
        }
    }
    // 3. 跨层的边插入虚节点，位于中间一层的虚节点放置连线文字。子图跨越其成员所在的各层。
    let span: Vec<Option<(usize, usize)>> = f
        .groups
        .iter()
        .map(|g| {
            let ranks = g.members.iter().map(|&m| rank[m]);
            Some((ranks.clone().min()?, ranks.max()?))
        })
        .collect();
    // 第 `r` 层的虚节点所属的子图：从一端所在的子图逐级向外，取第一个跨越该层的子图；两端各得到
    // 一个时取内层的一个。连线因此在子图跨越的各层中位于框内，离开这些层后才穿过边框，不需要
    // 绕到框外。两个结果互不包含时取二者共同的外层子图。
    let around = |home: Option<usize>, r: usize| {
        std::iter::successors(home, |&g| tree.parent[g])
            .find(|&g| span[g].is_some_and(|(top, bottom)| (top..=bottom).contains(&r)))
    };
    let mut vs: Vec<Vertex> = (0..n)
        .map(|v| {
            let (cross, along) = flip(sizes[v]);
            Vertex {
                cross,
                along,
                rank: rank[v],
                up: Vec::new(),
                down: Vec::new(),
                kind: Kind::Real,
                group: tree.home[v],
            }
        })
        .collect();
    let mut chains: Vec<Chain> = Vec::new();
    for &(k, a, b, _) in &dag {
        let (ra, rb) = (rank[a], rank[b]);
        let mut group = tree.common(tree.home[a], tree.home[b]);
        let mut chain = vec![a];
        let mid = ra + (rb - ra) / 2;
        let mut label_at = None;
        for r in ra + 1..rb {
            let has_label = r == mid && !f.edges[k].label.is_empty();
            let (cross, along) = if has_label {
                flip((labels[k].w + s * 0.6, labels[k].h + s * 0.4))
            } else {
                (s * 0.3, 0.0)
            };
            let (ga, gb) = (around(tree.home[a], r), around(tree.home[b], r));
            let common = tree.common(ga, gb);
            let inner = if common == ga {
                gb
            } else if common == gb {
                ga
            } else {
                common
            };
            vs.push(Vertex {
                cross,
                along,
                rank: r,
                up: Vec::new(),
                down: Vec::new(),
                kind: Kind::Dummy,
                group: inner,
            });
            if r == mid {
                label_at = Some(vs.len() - 1);
                group = inner;
            }
            chain.push(vs.len() - 1);
        }
        chain.push(b);
        for w in chain.windows(2) {
            vs[w[0]].down.push(w[1]);
            vs[w[1]].up.push(w[0]);
        }
        chains.push(Chain {
            edge: k,
            points: chain,
            label_at,
            group,
        });
    }
    // 子图在每层的左右边框点：同一侧的边框点上下相连，确定横坐标时互相对齐。
    let mut sides: Vec<[Vec<usize>; 2]> = vec![[Vec::new(), Vec::new()]; groups];
    for (g, sp) in span.iter().enumerate() {
        let Some((top, bottom)) = *sp else { continue };
        for (side, kind) in [Kind::Left(g), Kind::Right(g)].into_iter().enumerate() {
            for r in top..=bottom {
                let v = vs.len();
                vs.push(Vertex {
                    cross: 0.0,
                    along: 0.0,
                    rank: r,
                    up: Vec::new(),
                    down: Vec::new(),
                    kind,
                    group: Some(g),
                });
                if let Some(&above) = sides[g][side].last() {
                    vs[above].down.push(v);
                    vs[v].up.push(above);
                }
                sides[g][side].push(v);
            }
        }
    }
    // 框的边距：标题那边多留一条标题的高度。上下走向的图标题在层的方向上（自下往上的图
    // 在末尾那头），左右走向的在同一层的方向上（左边框那侧，转成屏幕坐标是上边）。
    let pad = s * 0.8;
    let band: Vec<f32> = titles
        .iter()
        .map(|&h| if h > 0.0 { h + s * 0.4 } else { 0.0 })
        .collect();
    let lead_left: Vec<f32> = (0..groups)
        .map(|g| pad + if vertical { 0.0 } else { band[g] })
        .collect();
    let lead_start: Vec<f32> = (0..groups)
        .map(|g| pad + if dir == Dir::Down { band[g] } else { 0.0 })
        .collect();
    let lead_end: Vec<f32> = (0..groups)
        .map(|g| pad + if dir == Dir::Up { band[g] } else { 0.0 })
        .collect();

    // 4. 每层的顺序：按邻居的平均位置排，来回多轮，留交叉最少的。
    let ranks = vs.iter().map(|v| v.rank).max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); ranks];
    for (i, v) in vs.iter().enumerate() {
        layers[v.rank].push(i);
    }
    let mut pos: Vec<f32> = (0..vs.len()).map(|v| v as f32).collect();
    for layer in &mut layers {
        *layer = arrange(layer, &pos, &vs, &tree);
        for (k, &v) in layer.iter().enumerate() {
            pos[v] = k as f32;
        }
    }
    let mut best = layers.clone();
    let mut best_crossings = crossings(&layers, &vs, &pos);
    // 24 轮排序（经验值，往下、往上交替）：再多交叉数不再下降。
    for round in 0..24 {
        let downward = round % 2 == 0;
        let range: Vec<usize> = if downward {
            (1..ranks).collect()
        } else {
            (0..ranks.saturating_sub(1)).rev().collect()
        };
        for r in range {
            let mut key = pos.clone();
            for &v in &layers[r] {
                let near = if downward { &vs[v].up } else { &vs[v].down };
                if !near.is_empty() && !vs[v].border() {
                    key[v] = near.iter().map(|&u| pos[u]).sum::<f32>() / near.len() as f32;
                }
            }
            layers[r] = arrange(&layers[r], &key, &vs, &tree);
            for (k, &v) in layers[r].iter().enumerate() {
                pos[v] = k as f32;
            }
        }
        transpose(&mut layers, &mut pos, &vs, round % 4 == 1);
        let c = crossings(&layers, &vs, &pos);
        if c < best_crossings {
            best_crossings = c;
            best = layers.clone();
        }
    }
    let layers = best;

    // 5. 横坐标：先挨着排，再按邻居的平均位置迭代几轮，保持顺序和间距；最后把每个子图的
    // 边框点拉成一条竖线。
    let gap = |a: &Vertex, b: &Vertex| {
        let space = match (a.kind, b.kind) {
            (Kind::Real, Kind::Real) => s * 2.5,
            (Kind::Left(g), _) => lead_left[g],
            (_, Kind::Right(_)) => pad,
            _ => s * 0.8,
        };
        (a.cross + b.cross) / 2.0 + space
    };
    let mut x = vec![0f32; vs.len()];
    for layer in &layers {
        let mut at = 0.0;
        for (k, &v) in layer.iter().enumerate() {
            if k > 0 {
                at += gap(&vs[layer[k - 1]], &vs[v]);
            }
            x[v] = at;
        }
    }
    // 16 轮拉坐标（经验值）：最后 4 轮同时参考上下两边的邻居。
    for round in 0..16 {
        let both = round >= 12;
        let downward = round % 2 == 0;
        let order: Vec<usize> = if downward {
            (0..ranks).collect()
        } else {
            (0..ranks).rev().collect()
        };
        for r in order {
            let layer = &layers[r];
            if layer.is_empty() {
                continue;
            }
            let want: Vec<f32> = layer
                .iter()
                .enumerate()
                .map(|(k, &v)| {
                    // 边框点贴着框里挨着它的点，不向其他方向偏移。
                    match vs[v].kind {
                        Kind::Left(_) if k + 1 < layer.len() => {
                            return x[layer[k + 1]] - gap(&vs[v], &vs[layer[k + 1]]);
                        }
                        Kind::Right(_) if k > 0 => {
                            return x[layer[k - 1]] + gap(&vs[layer[k - 1]], &vs[v]);
                        }
                        _ => {}
                    }
                    let mut near: Vec<usize> = Vec::new();
                    if downward || both {
                        near.extend(&vs[v].up);
                    }
                    if !downward || both {
                        near.extend(&vs[v].down);
                    }
                    if near.is_empty() {
                        x[v]
                    } else {
                        near.iter().map(|&u| x[u]).sum::<f32>() / near.len() as f32
                    }
                })
                .collect();
            let m = layer.len();
            let mut l = want.clone();
            for k in 1..m {
                l[k] = l[k].max(l[k - 1] + gap(&vs[layer[k - 1]], &vs[layer[k]]));
            }
            let mut rr = want;
            for k in (0..m.saturating_sub(1)).rev() {
                rr[k] = rr[k].min(rr[k + 1] - gap(&vs[layer[k]], &vs[layer[k + 1]]));
            }
            for k in 0..m {
                x[layer[k]] = (l[k] + rr[k]) / 2.0;
            }
        }
    }
    if groups > 0 {
        // 12 轮（经验值）：拉齐边框与推开邻居互相影响，多迭代几次才能收敛。
        for _ in 0..12 {
            for [lefts, rights] in &sides {
                let l = lefts.iter().map(|&v| x[v]).fold(f32::MAX, f32::min);
                let r = rights.iter().map(|&v| x[v]).fold(f32::MIN, f32::max);
                lefts.iter().for_each(|&v| x[v] = l);
                rights.iter().for_each(|&v| x[v] = r);
            }
            // 对齐后的边框可能与相邻的点重叠。保持顺序，先从右向左把重叠的点向左推开，
            // 再从左向右向右推开；推开时不移动对侧的边框：框只向外扩大，框外的点留在框外。
            // 若同时移动对侧边框，各层的边框不再对齐，框外的点可能落入框内。
            for layer in &layers {
                for k in (1..layer.len()).rev() {
                    let (a, b) = (layer[k - 1], layer[k]);
                    if !matches!(vs[a].kind, Kind::Right(_)) {
                        x[a] = x[a].min(x[b] - gap(&vs[a], &vs[b]));
                    }
                }
                for k in 1..layer.len() {
                    let (a, b) = (layer[k - 1], layer[k]);
                    if !matches!(vs[b].kind, Kind::Left(_)) {
                        x[b] = x[b].max(x[a] + gap(&vs[a], &vs[b]));
                    }
                }
            }
        }
    }

    // 6. 竖坐标：每层按最高的项，层之间留相同的间距，子图开始、结束的地方再加上框的边距。
    let thick: Vec<f32> = layers
        .iter()
        .map(|l| l.iter().map(|&v| vs[v].along).fold(0.0, f32::max))
        .collect();
    let chain = |g: usize, lead: &[f32], end: bool| {
        let edge = |h: usize| span[h].map(|(a, b)| if end { b } else { a });
        let mut total = 0.0;
        let mut at = Some(g);
        while let Some(h) = at.filter(|&h| edge(h) == edge(g)) {
            total += lead[h];
            at = tree.parent[h];
        }
        total
    };
    let mut before = vec![0f32; ranks];
    let mut after = vec![0f32; ranks];
    for (g, sp) in span.iter().enumerate() {
        if let Some((top, bottom)) = *sp {
            before[top] = before[top].max(chain(g, &lead_start, false));
            after[bottom] = after[bottom].max(chain(g, &lead_end, true));
        }
    }
    let rank_gap = s * 1.4;
    let mut y = vec![0f32; ranks];
    let mut at = thick.first().copied().unwrap_or(0.0) / 2.0 + before[0];
    for r in 0..ranks {
        if r > 0 {
            at += thick[r - 1] / 2.0 + after[r - 1] + rank_gap + before[r] + thick[r] / 2.0;
        }
        y[r] = at;
    }
    let along = at + thick.last().copied().unwrap_or(0.0) / 2.0 + after[ranks - 1];

    // 子图的框：里面的点（真节点、连线经过的点）的外框加上边距，从最里面的子图往外算。
    let mut boxes = vec![[f32::MAX, f32::MAX, f32::MIN, f32::MIN]; groups];
    let grow = |b: &mut [f32; 4], r: [f32; 4]| {
        *b = [
            b[0].min(r[0]),
            b[1].min(r[1]),
            b[2].max(r[2]),
            b[3].max(r[3]),
        ];
    };
    for (v, vertex) in vs.iter().enumerate() {
        if vertex.border() {
            continue;
        }
        let (cx, cy) = (x[v], y[vertex.rank]);
        let r = [
            cx - vertex.cross / 2.0,
            cy - vertex.along / 2.0,
            cx + vertex.cross / 2.0,
            cy + vertex.along / 2.0,
        ];
        for g in tree.path(vertex.group) {
            grow(&mut boxes[g], r);
        }
    }
    let mut inner_first: Vec<usize> = (0..groups).collect();
    inner_first.sort_by_key(|&g| std::cmp::Reverse(tree.path(Some(g)).len()));
    for g in inner_first {
        let b = boxes[g];
        if b[0] > b[2] {
            continue;
        }
        let b = [
            b[0] - lead_left[g],
            b[1] - lead_start[g],
            b[2] + pad,
            b[3] + lead_end[g],
        ];
        boxes[g] = b;
        if let Some(p) = tree.parent[g] {
            grow(&mut boxes[p], b);
        }
    }

    let min = vs
        .iter()
        .enumerate()
        .map(|(i, v)| x[i] - v.cross / 2.0)
        .chain(boxes.iter().filter(|b| b[0] <= b[2]).map(|b| b[0]))
        .fold(f32::MAX, f32::min);
    let max = vs
        .iter()
        .enumerate()
        .map(|(i, v)| x[i] + v.cross / 2.0)
        .chain(boxes.iter().filter(|b| b[0] <= b[2]).map(|b| b[2]))
        .fold(f32::MIN, f32::max);
    for b in &mut boxes {
        b[0] -= min;
        b[2] -= min;
    }
    let center = |v: usize| (x[v] - min, y[vs[v].rank]);
    let mut routes: Vec<Option<Route>> = f.edges.iter().map(|_| None).collect();
    for Chain {
        edge: k,
        points: chain,
        label_at,
        group,
    } in chains
    {
        let mut points: Vec<(f32, f32)> = chain.iter().map(|&v| center(v)).collect();
        if reversed[k] {
            points.reverse();
        }
        routes[k] = Some(Route {
            points,
            label: label_at.map(center),
            group,
        });
    }
    for (k, e) in f.edges.iter().enumerate() {
        if e.from == e.to {
            routes[k] = Some(Route {
                points: Vec::new(),
                label: None,
                group: None,
            });
        }
    }
    Plan {
        centers: (0..n).map(center).collect(),
        routes,
        boxes,
        cross: (max - min).max(0.0),
        along,
    }
}

/// 按邻居的平均位置排完以后再挨个看：同一个子图里挨着的两项（一个点，或者一整个子图的块）
/// 换个位置交叉更少时就换，直到换不动。
///
/// `explore` 时交叉一样多也换：平均位置相同的两项排不出先后，换一换可能让别的层排得更
/// 好；最后留交叉最少的那次，不会变差。
fn transpose(layers: &mut [Vec<usize>], pos: &mut [f32], vs: &[Vertex], explore: bool) {
    // `u` 在 `v` 左边时，两个点连出去的线交叉几次（上下两边都算）。
    let cost = |u: usize, v: usize, pos: &[f32]| {
        let mut total = 0;
        for (a, b) in [(&vs[u].up, &vs[v].up), (&vs[u].down, &vs[v].down)] {
            for &x in a {
                total += b.iter().filter(|&&y| pos[x] > pos[y]).count();
            }
        }
        total
    };
    let block_cost = |a: &[usize], b: &[usize], pos: &[f32]| {
        a.iter()
            .flat_map(|&u| b.iter().map(move |&v| (u, v)))
            .filter(|&(u, v)| !vs[u].border() && !vs[v].border())
            .map(|(u, v)| cost(u, v, pos))
            .sum::<usize>()
    };
    // 8 轮（经验值）：直到一轮下来没有改进就停。
    for _ in 0..8 {
        let mut improved = false;
        for layer in layers.iter_mut() {
            let mut start = 0;
            while let Some((a, b)) = next_pair(layer, vs, start) {
                let (left, right) = (&layer[a.clone()], &layer[b.clone()]);
                let (now, swapped) = (block_cost(left, right, pos), block_cost(right, left, pos));
                if swapped < now || (explore && swapped == now && swapped > 0) {
                    let moved: Vec<usize> = right.iter().chain(left).copied().collect();
                    layer.splice(a.start..b.end, moved);
                    for k in a.start..b.end {
                        pos[layer[k]] = k as f32;
                    }
                    improved |= swapped < now;
                }
                start = a.start + 1;
            }
        }
        if !improved {
            break;
        }
    }
}

/// 这一层从 `from` 往后第一对挨着的同级项（同一个子图里的一个点或者一整个子图的块），
/// 返回它们各自的范围。
fn next_pair(
    layer: &[usize],
    vs: &[Vertex],
    from: usize,
) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    // 从 `k` 开始的一项到哪里为止：子图的块到它的右边框为止；碰到右边框说明这一级完了。
    let item = |k: usize| -> Option<std::ops::Range<usize>> {
        match vs[*layer.get(k)?].kind {
            Kind::Right(_) => None,
            Kind::Left(g) => {
                let end = (k..layer.len()).find(|&e| vs[layer[e]].kind == Kind::Right(g))?;
                Some(k..end + 1)
            }
            _ => Some(k..k + 1),
        }
    };
    for k in from..layer.len() {
        if let Some(a) = item(k)
            && let Some(b) = item(a.end)
        {
            return Some((a, b));
        }
    }
    None
}

/// 相邻两层之间连线的交叉数（不算边框点之间的线）。
fn crossings(layers: &[Vec<usize>], vs: &[Vertex], pos: &[f32]) -> usize {
    let mut total = 0;
    for layer in layers {
        let segments: Vec<(f32, f32)> = layer
            .iter()
            .filter(|&&v| !vs[v].border())
            .flat_map(|&v| vs[v].down.iter().map(move |&w| (pos[v], pos[w])))
            .collect();
        for (i, a) in segments.iter().enumerate() {
            for b in &segments[i + 1..] {
                if (a.0 - b.0) * (a.1 - b.1) < 0.0 {
                    total += 1;
                }
            }
        }
    }
    total
}

#[cfg(test)]
mod tests;
