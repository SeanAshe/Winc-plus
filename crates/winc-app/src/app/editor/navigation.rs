//! 后退、前进：记录光标去过的位置。
//!
//! 记录的内容是一串位置和当前所在的一项。光标在同一个文件内移动不远（打字、上下移动几行）时，
//! 只把当前项更新为新位置；换了文件，或一次跳出超过 [`FAR`] 行（转到定义、转到行、点击很远的位置、
//! 查找跳到下一处）时，才在当前项之后添加新项，并丢弃其后可前进的项。
//! 编辑引起的光标移动（粘贴一大段、格式化）不算跳转。

use std::path::{Path, PathBuf};

use winc_core::text::Lines;
use winc_editor::DocumentId;

use super::*;

/// 最多记录的位置个数（个，经验值）；超出时丢弃最早的一个。
const LIMIT: usize = 50;

/// 同一个文件内，光标一次移动超过多少行才算跳转（行，经验值）。
const FAR: usize = 10;

/// 去过的一个位置，按行列而不是字符偏移记录：前面的行被修改后，回来时仍大致在原处。
#[derive(Debug, Clone, PartialEq)]
struct Spot {
    /// 文档的编号。
    id: DocumentId,
    /// 标签关闭后据此重新打开；未命名的文档关闭后无法回到。
    path: Option<PathBuf>,
    /// 行号（从 0 起）。
    line: usize,
    /// 列号（从 0 起）。
    col: usize,
}

/// 后退、前进使用的位置历史。
#[derive(Debug, Default)]
pub struct Navigation {
    /// 已记录的位置，按先后顺序排列。
    spots: Vec<Spot>,
    /// 当前所在项在 `spots` 中的下标。
    current: usize,
    /// 上一次观察时的当前文档及其版本号。
    ///
    /// 版本号变化说明光标是随编辑移动的。
    seen: Option<(DocumentId, u64)>,
    /// 上一次观察时的当前文档、光标位置（字符偏移）和版本号。
    ///
    /// 每处理完一批事件（包括鼠标移动）都观察一次，三者都没变时不再换算行列、复制路径。
    last: Option<(DocumentId, usize, u64)>,
    /// 下一次观察到光标移动时，无论远近都算作跳转。
    forced: bool,
}

impl Navigation {
    /// 是否还能向前（`forward` 为 `true`）或向后移动一项。
    pub(in crate::app) fn can_go(&self, forward: bool) -> bool {
        if forward {
            self.current + 1 < self.spots.len()
        } else {
            self.current > 0
        }
    }

    /// 下一次观察到光标移动时，无论远近都添加新项；模态编辑层的跳转命令执行之前调用。
    pub(in crate::app) fn mark_jump(&mut self) {
        self.forced = true;
    }

    /// 清空全部历史。
    pub(in crate::app) fn clear(&mut self) {
        *self = Navigation::default();
    }

    /// 文件或目录从 `from` 移到 `to` 后，同步更新已记录的路径。
    pub(in crate::app) fn relocate(&mut self, from: &Path, to: &Path) {
        for spot in &mut self.spots {
            if let Some(path) = &mut spot.path
                && let Ok(rest) = path.strip_prefix(from)
            {
                *path = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
            }
        }
    }

    /// 记录光标当前位于 `here`，文档版本号为 `revision`。
    ///
    /// 没有发生跳转时只更新当前项；发生跳转时在当前项之后添加新项，并丢弃其后的项。
    fn observe(&mut self, here: Spot, revision: u64) {
        let edited = self
            .seen
            .is_some_and(|(id, seen)| id == here.id && seen != revision);
        self.seen = Some((here.id, revision));

        let Some(current) = self.spots.get(self.current) else {
            self.spots.push(here);
            self.current = 0;
            return;
        };
        let moved = (current.line, current.col) != (here.line, here.col);
        let forced = std::mem::take(&mut self.forced) && moved;
        let jumped =
            current.id != here.id || forced || (!edited && current.line.abs_diff(here.line) > FAR);
        if !jumped {
            self.spots[self.current] = here;
            return;
        }

        self.spots.truncate(self.current + 1);
        self.spots.push(here);
        if self.spots.len() > LIMIT {
            self.spots.remove(0);
        }
        self.current = self.spots.len() - 1;
    }
}

impl App {
    /// 把光标当前位置记入后退、前进的历史，每处理完一批事件调用一次。
    pub(in crate::app) fn track_navigation(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        if !doc.shows_text() {
            return;
        }
        let key = (doc.id, doc.selection.head, doc.revision);
        let nav = &self.navigation;
        // 当前项仍是该文档、路径也没变（另存为不改版本号）：记下的就是现在的位置。
        let current = nav.spots.get(nav.current);
        if nav.last == Some(key) && current.is_some_and(|s| s.id == doc.id && s.path == doc.path) {
            // 光标没有移动：已标记的跳转没有发生。
            self.navigation.forced = false;
            return;
        }
        self.navigation.last = Some(key);
        let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
        let here = Spot {
            id: doc.id,
            path: doc.path.clone(),
            line,
            col,
        };
        let revision = doc.revision;
        self.navigation.observe(here, revision);
    }

    /// 后退或前进一项（`forward` 为 `true` 时前进；macOS 上后退为 `⌃-`，前进为 `⌃⇧-`）。
    ///
    /// 目标标签已关闭时按路径重新打开；文件已不存在时跳过该项，继续向同一方向查找。
    pub(in crate::app) fn navigate(&mut self, forward: bool) {
        self.track_navigation();

        while self.navigation.can_go(forward) {
            let target = if forward {
                self.navigation.current + 1
            } else {
                self.navigation.current - 1
            };
            let spot = self.navigation.spots[target].clone();

            if !self.show_spot(&spot) {
                self.navigation.spots.remove(target);
                if target < self.navigation.current {
                    self.navigation.current -= 1;
                }
                continue;
            }

            self.navigation.current = target;
            if let Some(doc) = self.editor.active() {
                self.navigation.spots[target].id = doc.id;
                self.navigation.seen = Some((doc.id, doc.revision));
            }
            return;
        }
    }

    /// 是否已回到 `spot`：切换到对应标签（已关闭则按路径重新打开），并把光标放回该位置。
    ///
    /// 目标行不在可见范围内时，滚动到该行位于可见区域顶部起约 1/3 处。无法回到时返回 `false`。
    fn show_spot(&mut self, spot: &Spot) -> bool {
        let index = self.editor.index_of(spot.id).or_else(|| {
            let path = spot.path.as_ref()?;
            self.editor
                .documents
                .iter()
                .position(|d| d.path.as_ref() == Some(path))
        });

        match (index, &spot.path) {
            (Some(index), _) => self.activate_tab(index),
            (None, Some(path)) if path.is_file() => self.open_path(path),
            _ => return false,
        }
        if self.editor.active().is_none() {
            return false;
        }
        self.focus = Focus::Editor;

        // 标签还没读（刚重新打开、恢复现场后没去过）时，读好再放回光标（`loading.rs`）。
        let (line, col) = (spot.line, spot.col);
        self.when_loaded(move |app| {
            let Some(doc) = app.editor.active() else {
                return;
            };
            let pos = doc.text().line_col_to_pos(line, col);
            let line = doc.text().line_of(pos);
            app.scroll_to_line(line, 1.0 / 3.0, true);
            app.select(pos, pos);
        });
        true
    }
}

#[cfg(test)]
mod tests;
