//! 现场里终端面板的分栏：各栏的标签数和份额，以及在状态文件中的读写。

use std::collections::BTreeMap;

use crate::json::Json;

/// 现场里终端面板的一栏。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalGroup {
    /// 这一栏有几个标签。
    pub tabs: usize,
    /// 这一栏占面板的份额，各栏按份额的比例分面板。
    pub share: f32,
}

/// 读出终端面板 `t` 中的各栏；共有 `tabs` 个标签。
///
/// 只有一栏、各栏的标签数加起来与标签数对不上（文件被改过）时返回空的，即不分栏。
pub(super) fn read_terminal_groups(t: &BTreeMap<String, Json>, tabs: usize) -> Vec<TerminalGroup> {
    let groups: Vec<TerminalGroup> = (t.get("groups").and_then(Json::as_array))
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let g = g.as_object()?;
            let count = g.get("tabs")?.as_number().filter(|n| *n >= 1.0)?;
            let share = (g.get("share").and_then(Json::as_number))
                .filter(|n| n.is_finite() && *n > 0.0)
                .unwrap_or(1.0);
            Some(TerminalGroup {
                tabs: count as usize,
                share: share as f32,
            })
        })
        .collect();
    let counted = groups.iter().map(|g| g.tabs).sum::<usize>() == tabs;
    if counted && groups.len() > 1 {
        groups
    } else {
        Vec::new()
    }
}

/// 写进状态文件的各栏；没有分栏时为 `None`，不写。
pub(super) fn terminal_groups_json(groups: &[TerminalGroup]) -> Option<Json> {
    (groups.len() > 1).then(|| {
        let groups = groups.iter().map(|g| {
            Json::object(vec![
                ("tabs", Json::Number(g.tabs as f64)),
                ("share", Json::Number(f64::from(g.share))),
            ])
        });
        Json::Array(groups.collect())
    })
}
