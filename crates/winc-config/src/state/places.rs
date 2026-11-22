//! 各文件上次的位置：记录、淘汰，以及在状态文件中的读写。

use std::path::{Path, PathBuf};

use super::State;
use crate::json::Json;

/// 记下位置的文件数上限（个），取 VS Code 保存编辑器视图状态的上限；超出时丢掉最久没有用到的。
pub const MAX_PLACES: usize = 100;

/// 一个文件上次的位置：文本的光标和视口，预览的阅读位置。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FilePlace {
    /// 光标所在的行和列（字符），以及视口顶边所在的行；均从 0 起。
    pub text: Option<(usize, usize, usize)>,
    /// 阅读位置：视口顶边所在的页（PDF）或顶层块（EPUB），从 0 起，以及视口顶边到它上边缘的
    /// 距离与它的高度之比。
    pub view: Option<(usize, f32)>,
}

impl FilePlace {
    /// 是否什么都没有记：这样的记录不保存。
    fn is_empty(&self) -> bool {
        self.text.is_none() && self.view.is_none()
    }
}

impl State {
    /// 返回文件 `path` 上次的位置；没有记录时为 `None`。
    pub fn place_of(&self, path: &Path) -> Option<FilePlace> {
        let found = self.places.iter().find(|(file, _)| file == path);
        found.map(|&(_, place)| place)
    }

    /// 用 `change` 修改文件 `path` 的位置，并把它记为最近用到的一条；改完什么都没有记的去掉，
    /// 超过 [`MAX_PLACES`] 条时丢掉最久没有用到的。
    pub fn set_place(&mut self, path: &Path, change: impl FnOnce(&mut FilePlace)) {
        let at = self.places.iter().position(|(file, _)| file == path);
        let mut place = at.map(|at| self.places.remove(at).1).unwrap_or_default();
        change(&mut place);
        if !place.is_empty() {
            self.places.insert(0, (path.to_path_buf(), place));
            self.places.truncate(MAX_PLACES);
        }
    }

    /// 文件或目录 `from` 移到了 `to`：其下各文件的位置改记在新路径上。
    pub fn move_places(&mut self, from: &Path, to: &Path) {
        for (path, _) in &mut self.places {
            if let Ok(rest) = path.strip_prefix(from) {
                *path = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
            }
        }
    }
}

/// 读出一个阅读位置 `[序号, 比例]`：序号为负数或小数、比例不是有限的数、缺少一项时为 `None`。
fn read_view(value: &Json) -> Option<(usize, f32)> {
    match value.as_array()? {
        [index, at] => {
            let index = index.as_number()?;
            let at = at.as_number().filter(|n| n.is_finite())?;
            (index >= 0.0 && index.fract() == 0.0).then_some((index as usize, at as f32))
        }
        _ => None,
    }
}

/// 读出早先记在现场里的阅读位置（`pdfPlaces`、`bookPlaces`）：文件路径到 [`read_view`] 的位置，
/// 读不出的记录跳过。
pub(super) fn read_views(value: Option<&Json>) -> Vec<(PathBuf, (usize, f32))> {
    let files = value.and_then(Json::as_object).into_iter().flatten();
    files
        .filter(|(path, _)| !path.is_empty())
        .filter_map(|(path, at)| Some((PathBuf::from(path), read_view(at)?)))
        .collect()
}

/// 读出各文件的位置（`places`）：每项有 `path`，以及 `caret`（`[行, 列]`）和 `top`（行）、
/// `view`（见 [`read_view`]）中的至少一样；读不出的部分当作没有，什么都没有的、路径重复的和
/// 超出 [`MAX_PLACES`] 的记录跳过。
pub(super) fn read_file_places(value: Option<&Json>) -> Vec<(PathBuf, FilePlace)> {
    let whole = |n: &Json| {
        let n = n.as_number().filter(|n| *n >= 0.0 && n.fract() == 0.0)?;
        Some(n as usize)
    };
    let mut places: Vec<(PathBuf, FilePlace)> = Vec::new();
    for item in value.and_then(Json::as_array).unwrap_or_default() {
        let Some(item) = item.as_object() else {
            continue;
        };
        let path = item.get("path").and_then(Json::as_str).unwrap_or_default();
        let text = match item.get("caret").and_then(Json::as_array) {
            Some([line, col]) => whole(line).zip(whole(col)).map(|(line, col)| {
                let top = item.get("top").and_then(whole).unwrap_or(line);
                (line, col, top)
            }),
            _ => None,
        };
        let place = FilePlace {
            text,
            view: item.get("view").and_then(read_view),
        };
        let known = places.iter().any(|(file, _)| file.as_os_str() == path);
        if !path.is_empty() && !place.is_empty() && !known && places.len() < MAX_PLACES {
            places.push((PathBuf::from(path), place));
        }
    }
    places
}

/// 写出各文件的位置；比例保留四位小数：一页（一块）高 10000 逻辑像素时误差为 1 逻辑像素。
pub(super) fn write_file_places(places: &[(PathBuf, FilePlace)]) -> Json {
    let number = |n: usize| Json::Number(n as f64);
    let items = places.iter().map(|(file, place)| {
        let mut fields = vec![("path", Json::String(file.to_string_lossy().into_owned()))];
        if let Some((line, col, top)) = place.text {
            fields.push(("caret", Json::Array(vec![number(line), number(col)])));
            fields.push(("top", number(top)));
        }
        if let Some((index, at)) = place.view {
            let at = (f64::from(at) * 1e4).round() / 1e4;
            fields.push(("view", Json::Array(vec![number(index), Json::Number(at)])));
        }
        let fields = fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value));
        Json::Object(fields.collect())
    });
    Json::Array(items.collect())
}
