//! 搜索结果各行的高亮状态：列出的行属于没有打开的文件时，在后台读出文件、从开头推算这些行行首
//! 的状态，送回后各行按跨行的结构（块注释、Vue 的 `<script>`）着色。
//!
//! 只推算列表中看得到的行所在的文件；每个文件推算一次，换了查询后重新推算。已打开的文件由文档
//! 推算并缓存，每帧最多推算一次预算内的行数。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use winc_syntax::LineState;

use super::CONTEXT_STATE_BUDGET;
use crate::app::{App, PICKER_ROW, UserEvent};

/// 后台推算好的一个文件：（查询的代次，文件，各行的行号与行首状态）。
type Done = (u64, PathBuf, Vec<(usize, LineState)>);

/// 搜索结果各行的高亮状态及其后台任务。
pub(crate) struct RowStates {
    /// 各文件推算好的各行行首状态。
    states: HashMap<PathBuf, HashMap<usize, LineState>>,
    /// 已交给后台、尚未送回的文件。
    pending: HashSet<PathBuf>,
    /// 查询的代次：换了查询后加 1，之前交给后台的结果送回时丢弃。
    generation: u64,
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
}

impl Default for RowStates {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            states: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            sender,
            receiver,
        }
    }
}

impl RowStates {
    /// 换了查询：丢掉推算好的状态，之后送回的旧结果不再采用。
    pub(super) fn reset(&mut self) {
        self.states.clear();
        self.pending.clear();
        self.generation += 1;
    }

    /// 返回推算好的 `path` 第 `line` 行行首的状态；还没有推算时返回 `None`。
    pub(crate) fn state(&self, path: &Path, line: usize) -> Option<LineState> {
        self.states.get(path)?.get(&line).copied()
    }
}

impl App {
    /// 每帧调用：收下后台推算好的状态；列表中看得到的行属于没有打开、也还没推算过的文件时，交给
    /// 后台推算该文件中列出的各行。属于已打开的文件、文档尚未推算到时，由文档接着推算。
    pub(super) fn advance_row_states(&mut self, list_h: f32) {
        let rows = &mut self.search_dialog.rows;
        let mut received = false;
        while let Ok((generation, path, lines)) = rows.receiver.try_recv() {
            if generation == rows.generation {
                rows.pending.remove(&path);
                rows.states.insert(path, lines.into_iter().collect());
                received = true;
            }
        }
        if received {
            self.request_redraw();
        }

        let d = &self.search_dialog;
        let Some(places) = &d.places else {
            return;
        };
        let first = (d.scroll / PICKER_ROW).max(0.0) as usize;
        let shown = (list_h / PICKER_ROW).ceil() as usize + 1;
        let (mut wanted, mut opened): (Vec<PathBuf>, Vec<(PathBuf, usize)>) = Default::default();
        for &index in d.hits.iter().skip(first).take(shown) {
            let Some(place) = places.items.get(index) else {
                continue;
            };
            let path = &place.path;
            let open = (self.editor.documents.iter()).find(|doc| doc.path.as_ref() == Some(path));
            if let Some(doc) = open {
                if !doc.is_large() && doc.known_highlight_state(place.line).is_none() {
                    opened.push((path.clone(), place.line));
                }
                continue;
            }
            let known = d.rows.states.contains_key(path) || d.rows.pending.contains(path);
            if !known && !wanted.contains(path) {
                wanted.push(path.clone());
            }
        }
        let jobs: Vec<(PathBuf, Vec<usize>)> = (wanted.into_iter())
            .map(|path| {
                let mut lines: Vec<usize> = (places.items.iter())
                    .filter(|place| place.path == path)
                    .map(|place| place.line)
                    .collect();
                lines.sort_unstable();
                lines.dedup();
                (path, lines)
            })
            .collect();
        // 文档从推算到的位置接着推算并缓存；一次用完预算时留到下一帧，每帧的耗时有上限。
        for (path, line) in opened {
            let open =
                (self.editor.documents.iter_mut()).find(|doc| doc.path.as_ref() == Some(&path));
            if open.is_some_and(|doc| doc.highlight_state(line, CONTEXT_STATE_BUDGET).is_none()) {
                self.request_redraw();
                break;
            }
        }
        for (path, lines) in jobs {
            let rows = &mut self.search_dialog.rows;
            rows.pending.insert(path.clone());
            let (sender, generation) = (rows.sender.clone(), rows.generation);
            let proxy = self.link.proxy.clone();
            winc_editor::background::spawn(move || {
                let states = row_states(&path, &lines);
                if sender.send((generation, path, states)).is_ok()
                    && let Some(proxy) = proxy
                {
                    let _ = proxy.send_event(UserEvent::Searched);
                }
            });
        }
    }
}

/// 读出文件 `path`，推算各行 `lines`（从小到大）行首的状态；过大或读不出的文件按 `Normal` 算。
///
/// 应在后台线程调用：耗时与文件大小成正比。
fn row_states(path: &Path, lines: &[usize]) -> Vec<(usize, LineState)> {
    let small = |m: winc_backend::fs::Metadata| m.len <= winc_editor::LARGE_FILE;
    let bytes = (winc_backend::fs::metadata(path).is_ok_and(small))
        .then(|| winc_backend::fs::read(path).ok())
        .flatten();
    let Some(bytes) = bytes else {
        return lines
            .iter()
            .map(|&line| (line, LineState::Normal))
            .collect();
    };
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(&bytes));
    let language = winc_syntax::detect(&winc_editor::display_name(path));
    let states = winc_editor::line_highlight_states(&rope, language, lines, CONTEXT_STATE_BUDGET);
    lines.iter().copied().zip(states).collect()
}
