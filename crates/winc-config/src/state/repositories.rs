//! 现场里用户对仓库列表的改动：手动添加的仓库和从列表中移除的仓库，以及在状态文件中的读写。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::json::Json;

/// 一组目录的仓库列表上，用户在自动查找的结果之外做的改动。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionRepositories {
    /// 手动添加的仓库的根目录，按添加的顺序：查找不到（层数之外、依赖目录里）时仍然列出。
    pub added: Vec<PathBuf>,
    /// 从列表中移除的仓库的根目录：查找、打开其中的文件时不再加入，直到再次手动添加。
    pub removed: Vec<PathBuf>,
}

impl SessionRepositories {
    /// 是否没有任何改动。
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// 读出现场 `s` 中的仓库改动；没有记录时为空。
pub(super) fn read_repositories(s: &BTreeMap<String, Json>) -> SessionRepositories {
    let Some(r) = s.get("repositories").and_then(Json::as_object) else {
        return SessionRepositories::default();
    };
    let paths = |key: &str| -> Vec<PathBuf> {
        let items = r.get(key).and_then(Json::as_array).into_iter().flatten();
        items.filter_map(Json::as_str).map(PathBuf::from).collect()
    };
    SessionRepositories {
        added: paths("added"),
        removed: paths("removed"),
    }
}

/// 返回仓库改动 `repositories` 在状态文件中的写法；没有改动时返回 `None`，不写这一项。
pub(super) fn repositories_json(repositories: &SessionRepositories) -> Option<Json> {
    if repositories.is_empty() {
        return None;
    }
    let paths = |list: &[PathBuf]| {
        let items = list
            .iter()
            .map(|p| Json::String(p.to_string_lossy().into_owned()));
        Json::Array(items.collect())
    };
    let mut fields = Vec::new();
    if !repositories.added.is_empty() {
        fields.push(("added", paths(&repositories.added)));
    }
    if !repositories.removed.is_empty() {
        fields.push(("removed", paths(&repositories.removed)));
    }
    Some(Json::object(fields))
}
