//! Git 提交图（`gitGraph`）的解析：`commit`、`branch`、`checkout`（`switch`）、`merge`、
//! `cherry-pick`。

use super::{Dir, unquote};

/// 提交的种类，决定提交点的画法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// 普通提交：实心圆点。
    Normal,
    /// `type: REVERSE`：撤销提交，圆点中画叉。
    Reverse,
    /// `type: HIGHLIGHT`：需要突出的提交，画成方块。
    Highlight,
    /// `merge` 生成的合并提交：圆点中套一个小圆。
    Merge,
    /// `cherry-pick` 生成的提交：空心圆点。
    CherryPick,
}

/// 一次提交。
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    /// 源码中指定的提交名（`id: "…"`）；没有指定时为 `None`，不显示。
    pub id: Option<String>,
    /// 所在分支在 [`Git::branches`] 中的序号。
    pub branch: usize,
    /// 父提交在 [`Git::commits`] 中的序号：普通提交至多一个，合并提交为两个。
    pub parents: Vec<usize>,
    /// `cherry-pick` 来源提交的序号。
    pub picked: Option<usize>,
    /// 标签（`tag: "v1.0"`）。
    pub tag: Option<String>,
    /// 种类。
    pub kind: CommitKind,
}

/// Git 提交图。
#[derive(Debug, Clone, PartialEq)]
pub struct Git {
    /// 时间推进的方向：从左往右（`LR`）、从上往下（`TB`）或从下往上（`BT`）。
    pub dir: Dir,
    /// 分支名，按绘制时从上到下（或从左到右）的顺序排列：`order` 小的在前，相同时按声明顺序。
    pub branches: Vec<String>,
    /// 提交，按时间先后排列。
    pub commits: Vec<Commit>,
}

/// 解析 Git 提交图；`args` 是 `gitGraph` 后面的方向（`LR:`、`TB:`、`BT:`）。没有任何提交时
/// 返回 `None`。
pub(super) fn parse(args: &str, lines: &[&str]) -> Option<Git> {
    let dir = match args.trim().trim_end_matches(':') {
        "TB" | "TD" => Dir::Down,
        "BT" => Dir::Up,
        _ => Dir::Right,
    };
    // 分支名、`order`、最新的提交。
    let mut branches: Vec<(String, Option<f32>, Option<usize>)> =
        vec![("main".to_owned(), None, None)];
    let mut commits: Vec<Commit> = Vec::new();
    let mut current = 0;
    let find = |branches: &[(String, Option<f32>, Option<usize>)], name: &str| {
        branches.iter().position(|b| b.0 == name)
    };
    for &line in lines {
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match word {
            "commit" => {
                let parents = branches[current].2.into_iter().collect();
                commits.push(commit(rest, current, parents, CommitKind::Normal));
                branches[current].2 = Some(commits.len() - 1);
            }
            "branch" => {
                let name = name_of(rest);
                if name.is_empty() || find(&branches, name).is_some() {
                    continue;
                }
                let order = attr(rest, "order").and_then(|o| o.parse().ok());
                branches.push((name.to_owned(), order, branches[current].2));
                current = branches.len() - 1;
            }
            "checkout" | "switch" => {
                if let Some(b) = find(&branches, name_of(rest)) {
                    current = b;
                }
            }
            "merge" => {
                let Some(other) = find(&branches, name_of(rest)) else {
                    continue;
                };
                let Some(head) = branches[other].2 else {
                    continue;
                };
                let parents = branches[current].2.into_iter().chain([head]).collect();
                commits.push(commit(rest, current, parents, CommitKind::Merge));
                branches[current].2 = Some(commits.len() - 1);
            }
            "cherry-pick" => {
                let source = attr(rest, "id")
                    .and_then(|id| commits.iter().position(|c| c.id.as_deref() == Some(id)));
                let Some(source) = source else { continue };
                let parents = branches[current].2.into_iter().collect();
                let mut c = commit("", current, parents, CommitKind::CherryPick);
                c.picked = Some(source);
                c.tag = attr(rest, "tag").map(str::to_owned);
                commits.push(c);
                branches[current].2 = Some(commits.len() - 1);
            }
            _ => {}
        }
    }
    if commits.is_empty() {
        return None;
    }
    // 按 `order` 排列分支（没写的视为声明顺序），再把提交中的分支序号换成排好后的。
    let mut order: Vec<usize> = (0..branches.len()).collect();
    order.sort_by(|&a, &b| {
        let key = |i: usize| branches[i].1.unwrap_or(i as f32);
        key(a).total_cmp(&key(b)).then(a.cmp(&b))
    });
    let mut rank = vec![0; branches.len()];
    for (at, &b) in order.iter().enumerate() {
        rank[b] = at;
    }
    for c in &mut commits {
        c.branch = rank[c.branch];
    }
    Some(Git {
        dir,
        branches: order.iter().map(|&b| branches[b].0.clone()).collect(),
        commits,
    })
}

/// 解析 `commit`、`merge` 后面的 `id`、`tag`、`type`。
fn commit(rest: &str, branch: usize, parents: Vec<usize>, kind: CommitKind) -> Commit {
    let kind = match attr(rest, "type") {
        Some("REVERSE") => CommitKind::Reverse,
        Some("HIGHLIGHT") => CommitKind::Highlight,
        _ => kind,
    };
    Commit {
        id: attr(rest, "id").map(str::to_owned),
        branch,
        parents,
        picked: None,
        tag: attr(rest, "tag").map(str::to_owned),
        kind,
    }
}

/// `branch`、`checkout`、`merge` 后面的分支名（可加引号），不含后面的属性。
fn name_of(rest: &str) -> &str {
    let rest = rest.trim();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.split('"').next().unwrap_or("");
    }
    rest.split_whitespace().next().unwrap_or("")
}

/// 属性 `key: 值` 的值：引号中的字符串，或到空白为止的一个词。
fn attr<'s>(s: &'s str, key: &str) -> Option<&'s str> {
    let mut from = 0;
    while let Some(at) = s[from..].find(key) {
        let start = from + at;
        from = start + key.len();
        // 属性名前面必须是行首或空白，避免 `id` 匹配到 `myid`。
        if start > 0 && !s[..start].ends_with(char::is_whitespace) {
            continue;
        }
        let Some(value) = s[from..].trim_start().strip_prefix(':') else {
            continue;
        };
        let value = value.trim_start();
        return Some(match value.strip_prefix('"') {
            Some(q) => q.split('"').next().unwrap_or(""),
            None => unquote(value.split_whitespace().next().unwrap_or("")),
        });
    }
    None
}
