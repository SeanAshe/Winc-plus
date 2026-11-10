//! 标签的显示名称：同名标签的上级目录标注，以及用于选择图标的文件名。

use std::path::Path;

/// 返回同名文件 `path` 的标签后标注的上级目录，用于与 `others`（其他同名文件）区分。
///
/// 从上一级目录起，不足以区分时逐级向上添加，直到与每个同名文件都不同，如 `a/src` 与
/// `b/src`；无法区分时使用全部上级目录。
pub(crate) fn distinct_folder(path: &Path, others: &[&Path]) -> Option<String> {
    let parents = |p: &Path| -> Vec<String> {
        p.parent()
            .map(|d| d.iter().map(|c| c.to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let mine = parents(path);
    let theirs: Vec<_> = others.iter().map(|p| parents(p)).collect();
    let suffix = |dirs: &[String], k: usize| dirs[dirs.len().saturating_sub(k)..].join("/");
    let k = (1..=mine.len())
        .find(|&k| theirs.iter().all(|t| suffix(t, k) != suffix(&mine, k)))
        .unwrap_or(mine.len());
    let folder = suffix(&mine, k).trim_start_matches('/').to_owned();
    (!folder.is_empty()).then_some(folder)
}

/// 返回第 `index` 个标签在名称后标注的上级目录，见 [`distinct_folder`]。
///
/// 仅当存在同名的其他标签时返回 `Some`；标签栏与标签列表菜单共用。
pub(crate) fn tab_folder(docs: &[winc_editor::Document], index: usize) -> Option<String> {
    let doc = docs.get(index)?;
    let name = doc.name();
    let others: Vec<_> = docs
        .iter()
        .filter(|d| d.id != doc.id && d.name() == name)
        .filter_map(|d| d.path.as_deref())
        .collect();
    let path = doc.path.as_deref().filter(|_| !others.is_empty())?;
    distinct_folder(path, &others)
}

/// 返回用于选择标签图标的文件名，标签栏与标签列表菜单共用。
///
/// 差异标签取所比较的文件名；其余标签的标题可能带有「· 差异」「· 提交号」等后缀，
/// 按完整标题无法识别类型，因此取「·」之前的部分。
pub(crate) fn tab_icon_name(doc: &winc_editor::Document) -> String {
    doc.diff
        .as_deref()
        .and_then(winc_editor::diff_view::DiffView::file_path)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            let name = doc.name();
            name.split(" · ").next().unwrap_or(&name).to_owned()
        })
}
