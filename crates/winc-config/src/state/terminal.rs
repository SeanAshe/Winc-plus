//! 现场里终端的分栏、编辑器标签位置，以及在状态文件中的读写。

use std::collections::BTreeMap;

use crate::json::Json;

/// 现场里终端的一栏；编辑器中的一个终端标签也占一栏。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalGroup {
    /// 这一栏有几个 shell（拆分的终端分别计数）。
    pub tabs: usize,
    /// 这一栏占面板的份额，各栏按份额的比例分面板。
    pub share: f32,
    /// 这一栏在编辑器中的位置；没有时停靠在终端面板里。
    pub editor: Option<TerminalEditor>,
}

/// 一个终端标签在现场的编辑器窗格和混合标签顺序中的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalEditor {
    /// 在现场窗格树的叶子顺序中的序号；未拆分时为 0。
    pub pane: usize,
    /// 窗格内的位置，只计算会保存的文件、备份和终端标签。
    pub tab: usize,
    /// 是否为所在窗格的当前标签。
    pub active: bool,
}

/// 读出一栏在编辑区中的位置；窗格或标签的序号不是非负整数时返回 `None`，这一栏停靠在面板里。
fn read_editor(json: &Json) -> Option<TerminalEditor> {
    let editor = json.as_object()?;
    let index = |key| {
        editor
            .get(key)?
            .as_number()
            .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < usize::MAX as f64)
            .map(|n| n as usize)
    };
    Some(TerminalEditor {
        pane: index("pane")?,
        tab: index("tab")?,
        active: match editor.get("active") {
            Some(active) => active.as_bool()?,
            None => false,
        },
    })
}

/// 读出终端面板 `t` 中的各栏；共有 `tabs` 个标签。
///
/// 只有一栏且不在编辑器中、各栏的 shell 数加起来与标签数对不上时返回空的。
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
                editor: g.get("editor").and_then(read_editor),
            })
        })
        .collect();
    let counted = groups.iter().map(|g| g.tabs).sum::<usize>() == tabs;
    if counted && (groups.len() > 1 || groups.iter().any(|g| g.editor.is_some())) {
        groups
    } else {
        Vec::new()
    }
}

/// 写进状态文件的各栏；只有一个面板栏时为 `None`，不写。
pub(super) fn terminal_groups_json(groups: &[TerminalGroup]) -> Option<Json> {
    (groups.len() > 1 || groups.iter().any(|g| g.editor.is_some())).then(|| {
        let groups = groups.iter().map(|g| {
            let mut fields = vec![
                ("tabs", Json::Number(g.tabs as f64)),
                ("share", Json::Number(f64::from(g.share))),
            ];
            if let Some(editor) = g.editor {
                fields.push((
                    "editor",
                    Json::object(vec![
                        ("pane", Json::Number(editor.pane as f64)),
                        ("tab", Json::Number(editor.tab as f64)),
                        ("active", Json::Bool(editor.active)),
                    ]),
                ));
            }
            Json::object(fields)
        });
        Json::Array(groups.collect())
    })
}
