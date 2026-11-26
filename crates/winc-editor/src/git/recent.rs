//! 提交视图中的提交历史：当前分支和它的上游最近的提交、提交图，以及每个提交只在本地、只在
//! 上游还是两边都有。
//!
//! 只读取最近的 [`RECENT_COMMITS`] 个提交，更早的提交在日志中查看。

use std::collections::HashMap;
use std::path::Path;

use winc_core::graph::{Graph, GraphRow};

use super::{Commit, Git, LogQuery, Ref};

/// 提交历史最多读取的提交数（个）。
///
/// 一组展开时露出约 8 行，滚动查看最近的一百个提交已足够，更早的提交在日志中查看；
/// 一条记录约几百字节，一次读取约几十 KB。
pub const RECENT_COMMITS: usize = 100;

/// 一个提交相对当前分支与上游的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// 当前分支和上游都包含；没有上游时所有提交都是这一种。
    Both,
    /// 只在当前分支上，尚未推送。
    Outgoing,
    /// 只在上游，尚未拉取。
    Incoming,
}

/// 当前分支和上游最近的提交。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recent {
    /// 提交，子提交在前（`git log --date-order`）。
    pub commits: Vec<Commit>,
    /// 每个提交一行的提交图，与 `commits` 一一对应。
    pub graph: Vec<GraphRow>,
    /// 每个提交的位置，与 `commits` 一一对应。
    pub sides: Vec<Side>,
    /// 是否还有更早的提交没有读取。
    pub more: bool,
}

impl Recent {
    /// 由按 `--date-order` 排好的提交 `commits` 算出提交图和各提交的位置。
    ///
    /// `upstream` 是上游的远程分支名（`origin/main`）；`more` 表示还有更早的提交。
    pub fn new(commits: Vec<Commit>, upstream: Option<&str>, more: bool) -> Recent {
        let mut lanes = Graph::new();
        let graph = (commits.iter())
            .map(|c| lanes.push(&c.oid, &c.parents))
            .collect();
        let sides = sides(&commits, upstream);
        Recent {
            commits,
            graph,
            sides,
            more,
        }
    }
}

impl Git {
    /// 读取 `HEAD` 与上游 `upstream`（`origin/main`；没有上游时为 `None`）最近的
    /// [`RECENT_COMMITS`] 个提交。应在后台线程调用。
    ///
    /// # Errors
    ///
    /// `git` 无法启动、报错（还没有提交、上游不存在）或超时时返回原因。
    pub fn recent(&self, root: &Path, upstream: Option<&str>) -> Result<Recent, String> {
        let mut branches = vec!["HEAD".to_owned()];
        branches.extend(upstream.map(str::to_owned));
        let query = LogQuery {
            branches,
            ..LogQuery::default()
        };
        // 多读一个，据此判断是否还有更早的提交。
        let mut commits = self.log_stream(root, &query)?.next(RECENT_COMMITS + 1)?;
        let more = commits.len() > RECENT_COMMITS;
        commits.truncate(RECENT_COMMITS);
        Ok(Recent::new(commits, upstream, more))
    }
}

/// 返回 `commits` 中每个提交相对 `HEAD` 与上游 `upstream` 的位置。
///
/// `commits` 按 `--date-order` 排列，子提交都在父提交之前，因此按顺序沿父提交向下标记即可得到
/// 从 `HEAD`、从上游各自能到达的提交。某一端不在列表中时，列出的提交都不能从该端到达。
fn sides(commits: &[Commit], upstream: Option<&str>) -> Vec<Side> {
    let Some(upstream) = upstream else {
        return vec![Side::Both; commits.len()];
    };
    let head = commits
        .iter()
        .position(|c| (c.refs.iter()).any(|r| matches!(r, Ref::Head(_) | Ref::Detached)));
    let remote = commits
        .iter()
        .position(|c| (c.refs.iter()).any(|r| matches!(r, Ref::Remote(name) if name == upstream)));
    let index: HashMap<&str, usize> = (commits.iter().enumerate())
        .map(|(i, c)| (c.oid.as_str(), i))
        .collect();
    let reach = |start: Option<usize>| {
        let mut reached = vec![false; commits.len()];
        let Some(start) = start else {
            return reached;
        };
        reached[start] = true;
        for i in start..commits.len() {
            if !reached[i] {
                continue;
            }
            for parent in &commits[i].parents {
                if let Some(&j) = index.get(parent.as_str()) {
                    reached[j] = true;
                }
            }
        }
        reached
    };
    let (local, remote) = (reach(head), reach(remote));
    (local.iter().zip(&remote))
        .map(|pair| match pair {
            (true, false) => Side::Outgoing,
            (false, true) => Side::Incoming,
            _ => Side::Both,
        })
        .collect()
}

#[cfg(test)]
mod tests;
