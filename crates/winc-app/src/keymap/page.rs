//! 设置里「快捷键」一页的行：同一个命令的几个键合成一行，按名字和按键搜索。

use super::*;

/// 「快捷键」页的一行：一个命令在一个生效范围内的全部键。
///
/// 同一命令在不同范围内的键不同时，分成多行（例如复制路径在编辑区与非编辑区的键不同）；键
/// 相同时合并为一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// 命令。
    pub action: Action,
    /// 这一行的生效范围。
    pub when: When,
    /// 该命令在此范围内的键的显示文字；没有键时为空。
    pub keys: Vec<String>,
}

/// 把 `os` 上的绑定表 `table` 按分组列出全部命令的快捷键，没有键的命令也列出。
pub(super) fn rows(table: &[Binding], os: Os) -> Vec<(Group, Vec<Row>)> {
    Group::ALL
        .into_iter()
        .map(|group| {
            let mut rows: Vec<Row> = Vec::new();
            for &action in ACTIONS.iter().filter(|a| a.group() == group) {
                let mut mine: Vec<Row> = Vec::new();
                for b in table.iter().filter(|b| b.action == action) {
                    let label = b.label(os);
                    match mine.iter_mut().find(|r| r.when == b.when) {
                        Some(row) => {
                            if !row.keys.contains(&label) {
                                row.keys.push(label);
                            }
                        }
                        None => mine.push(Row {
                            action,
                            when: b.when,
                            keys: vec![label],
                        }),
                    }
                }
                // 多个范围内的键相同（查找条和搜索面板的开关）时，只列一行。
                if mine.len() > 1 && mine.iter().all(|r| r.keys == mine[0].keys) {
                    let keys = mine[0].keys.clone();
                    mine = vec![Row {
                        action,
                        when: When::Global,
                        keys,
                    }];
                }
                if mine.is_empty() {
                    mine.push(Row {
                        action,
                        when: When::Global,
                        keys: Vec::new(),
                    });
                }
                if mine.len() == 1 && group.implies_scope() {
                    mine[0].when = When::Global;
                }
                rows.extend(mine);
            }
            (group, rows)
        })
        .collect()
}

/// 返回 `row` 是否匹配「快捷键」页的搜索词 `query`。
///
/// 普通搜索匹配命令的中英文名或键（如 `cmd k`、`ctrl+shift+p`），不区分大小写，忽略空格和加号。
/// `recorded` 为 `true` 时，`query` 是录制的键（`⌘K`、`⌘K ⌘S`），只匹配以所录各段开头的键，
/// 不匹配名字。
pub fn row_matches(row: &Row, query: &str, os: Os, recorded: bool) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    if recorded {
        let wanted: Vec<&str> = query.split(' ').collect();
        return row.keys.iter().any(|k| {
            let strokes: Vec<&str> = k.split(' ').collect();
            strokes.len() >= wanted.len() && strokes[..wanted.len()] == wanted[..]
        });
    }
    let lower = query.to_lowercase();
    if row.action.title().to_lowercase().contains(&lower)
        || row.action.english().to_lowercase().contains(&lower)
    {
        return true;
    }
    let wanted = normalize_keys(query, os);
    !wanted.is_empty()
        && row
            .keys
            .iter()
            .any(|k| normalize_keys(k, os).contains(&wanted))
}

/// 返回当前键位表中名称或键与 `query` 相符的命令个数（见 [`row_matches`]）；`query` 为空时为 0。
pub fn count_matches(query: &str) -> usize {
    if query.trim().is_empty() {
        return 0;
    }
    let rows = super::current_rows();
    let all = rows.iter().flat_map(|(_, rows)| rows);
    all.filter(|row| row_matches(row, query, Os::CURRENT, false))
        .count()
}

/// 把键规范化后再比较：转小写，去掉空格和加号，macOS 上把 cmd、shift 等词换成对应符号。
fn normalize_keys(text: &str, os: Os) -> String {
    let mut out = String::new();
    for word in text
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '+')
    {
        let word = match (os, word) {
            (Os::Mac, "cmd" | "command" | "meta") => "⌘",
            (Os::Mac, "ctrl" | "control") => "⌃",
            (Os::Mac, "alt" | "option" | "opt") => "⌥",
            (Os::Mac, "shift") => "⇧",
            (_, "control") => "ctrl",
            (_, w) => w,
        };
        out.push_str(word);
    }
    out
}
