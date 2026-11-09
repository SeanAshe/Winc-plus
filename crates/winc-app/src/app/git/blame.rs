//! 逐行追溯（`git blame`）：在行号栏左侧增加一栏，显示每一行最后一次修改的作者与时间。
//!
//! 按文档开关（命令「Git：逐行追溯」、编辑区右键菜单）。开启后在后台运行
//! `git blame --porcelain --contents -`，将编辑器中的内容（含未保存的修改）交给 `git`，使行号与
//! 编辑器一致，已修改但未提交的行记为「未提交」。输入时不每次运行 `git`，停止输入
//! [`BLAME_IDLE`] 后或保存后在后台重新计算。该栏由 `view/editor.rs` 在行号栏中绘制。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use ropey::Rope;

use winc_core::Lines;
use winc_editor::DocumentId;
use winc_editor::format::Cancel;
use winc_editor::git::{Blame, BlameCommit};
use winc_i18n::t;
use winc_render::Rect;

use super::log::{display_at, local_text, short};
use super::unusable;
use crate::app::{App, UserEvent};

/// 逐行追溯一栏的宽度（逻辑像素）。
///
/// 取自调参：可容纳 7 位提交号以及常见长度的作者名和日期。
pub(crate) const BLAME_W: f32 = 176.0;

/// 停止输入后重新运行 `git blame` 的等待时长。
///
/// 取自调参：连续输入期间不反复运行 `git`。在此之前按改动原地平移上一次的结果。
const BLAME_IDLE: Duration = Duration::from_secs(1);

/// 一次逐行追溯的结果及其对应的文本。
struct Entry {
    /// 对应的文档版本。
    revision: u64,
    blame: Arc<Blame>,
    /// 计算该结果时的文本，下一次原地平移时与新文本比较。
    text: Rope,
}

/// 后台计算完成的一次逐行追溯。
struct Computed {
    doc: DocumentId,
    revision: u64,
    text: Rope,
    result: Result<Blame, String>,
}

/// 逐行追溯的状态。
pub(crate) struct BlameState {
    /// 开启了逐行追溯的文档。
    on: HashSet<DocumentId>,
    /// 各文档最近一次的结果（后台计算的结果，或按改动原地平移后的结果）。
    data: HashMap<DocumentId, Entry>,
    /// 正在后台计算的文档及其版本号。
    computing: HashMap<DocumentId, u64>,
    /// 已原地平移、需要在对应时刻重新运行 `git blame` 的文档。
    due: HashMap<DocumentId, Instant>,
    results: (Sender<Computed>, Receiver<Computed>),
}

impl Default for BlameState {
    fn default() -> Self {
        BlameState {
            on: HashSet::new(),
            data: HashMap::new(),
            computing: HashMap::new(),
            due: HashMap::new(),
            results: channel(),
        }
    }
}

impl BlameState {
    /// 文档 `id` 是否开启了逐行追溯。
    pub(crate) fn is_on(&self, id: DocumentId) -> bool {
        self.on.contains(&id)
    }

    /// 在文档 `id` 关闭后释放其逐行追溯。
    ///
    /// 正在计算的结果送达时文档已不在 `on` 中，结果被丢弃。
    pub(super) fn forget(&mut self, id: DocumentId) {
        self.on.remove(&id);
        self.data.remove(&id);
        self.computing.remove(&id);
        self.due.remove(&id);
    }

    /// 关闭所有文档的逐行追溯（模块关闭或仓库更换时）。
    pub(super) fn clear(&mut self) {
        self.on.clear();
        self.data.clear();
        self.computing.clear();
        self.due.clear();
    }
}

impl App {
    /// 返回当前文档的逐行追溯一栏的宽度（逻辑像素）；未开启时为 0。
    ///
    /// 行号栏的宽度随之增加。
    pub(crate) fn blame_width(&self) -> f32 {
        match self.editor.active() {
            Some(doc) if self.blame.is_on(doc.id) && doc.shows_text() => BLAME_W,
            _ => 0.0,
        }
    }

    /// 切换当前文档的逐行追溯（命令「Git：逐行追溯」）。
    pub(in crate::app) fn toggle_blame(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        if self.blame.on.remove(&id) {
            self.blame.data.remove(&id);
            self.blame.due.remove(&id);
            self.relayout();
            self.request_redraw();
            return;
        }
        if self.blame_path(id).is_none() {
            self.flash(t!("git-blame-not-in-repository"));
            return;
        }
        self.blame.on.insert(id);
        self.relayout();
        self.update_blame(id);
        self.request_redraw();
    }

    /// 返回文档 `id` 所在的仓库与文件路径；不是仓库中的文本文件时返回 `None`。
    fn blame_path(&self, id: DocumentId) -> Option<(PathBuf, PathBuf)> {
        let root = self.git.root.clone()?;
        let doc = self.editor.document(id)?;
        if !doc.shows_text() || doc.is_large() {
            return None;
        }
        let path = doc.path.clone()?;
        let path = if path.starts_with(&root) {
            path
        } else {
            winc_config::files::canonicalize(path)
                .ok()
                .filter(|p| p.starts_with(&root))?
        };
        Some((root, path))
    }

    /// 在绘制前更新文档 `id` 的逐行追溯；未开启或结果已是最新时不做处理。
    ///
    /// 尚无结果或到达重新计算的时刻时，在后台运行 `git blame`（同一文档同一时刻只计算一次）。
    /// 文本刚变化时先按改动平移上一次的结果，停止输入 [`BLAME_IDLE`] 后再重新计算。
    pub(crate) fn update_blame(&mut self, id: DocumentId) {
        if !self.blame.is_on(id) {
            return;
        }
        let Some(doc) = self.editor.document(id) else {
            self.blame.on.remove(&id);
            return;
        };
        let revision = doc.revision;
        let Some(rope) = doc.text().rope() else {
            return;
        };
        if let Some(entry) = self.blame.data.get_mut(&id)
            && entry.revision != revision
        {
            entry.blame = Arc::new(shifted(&entry.blame, &entry.text, rope));
            entry.text = rope.clone();
            entry.revision = revision;
            self.blame.due.insert(id, Instant::now() + BLAME_IDLE);
        }
        let due = match self.blame.due.get(&id) {
            Some(at) => Instant::now() >= *at,
            None => !self.blame.data.contains_key(&id),
        };
        if !due || self.blame.computing.contains_key(&id) {
            return;
        }
        let rope = rope.clone();
        let Some((root, path)) = self.blame_path(id) else {
            return;
        };
        self.blame.due.remove(&id);
        self.blame.computing.insert(id, revision);
        let sender = self.blame.results.0.clone();
        self.spawn_git(UserEvent::GitBlame, move |tool| {
            let contents = rope.to_string();
            let result = match tool {
                Ok(git) => git.blame(&root, &path, contents.into_bytes(), &Cancel::default()),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Computed {
                doc: id,
                revision,
                text: rope,
                result,
            });
        });
    }

    /// 在文档 `id` 保存后立即重新计算逐行追溯，不等待停止输入；未开启时不做处理。
    pub(in crate::app) fn blame_saved(&mut self, id: DocumentId) {
        if self.blame.is_on(id) {
            self.blame.due.insert(id, Instant::now());
            self.update_blame(id);
        }
    }

    /// 在后台重新计算已到时刻的文档，返回下一个重新计算的时刻；事件循环每一轮调用。
    pub(in crate::app) fn blame_tick(&mut self, now: Instant) -> Option<Instant> {
        let ready: Vec<_> = (self.blame.due.iter())
            .filter(|&(_, &at)| now >= at)
            .map(|(&id, _)| id)
            .collect();
        for id in ready {
            self.update_blame(id);
        }
        self.blame.due.values().min().copied()
    }

    /// 接收后台计算完成的逐行追溯。
    ///
    /// 失败时（文件尚未被跟踪）提示原因并关闭逐行追溯。
    pub(in crate::app) fn poll_blame(&mut self) {
        while let Ok(done) = self.blame.results.1.try_recv() {
            self.blame.computing.remove(&done.doc);
            if !self.blame.is_on(done.doc) {
                continue;
            }
            match done.result {
                Ok(blame) => {
                    let entry = Entry {
                        revision: done.revision,
                        blame: Arc::new(blame),
                        text: done.text,
                    };
                    self.blame.data.insert(done.doc, entry);
                }
                Err(e) => {
                    self.blame.forget(done.doc);
                    self.relayout();
                    self.flash(t!("git-blame-failed", error = e));
                }
            }
        }
        self.request_redraw();
    }

    /// 返回文档 `id` 最近一次计算的逐行追溯；可能落后于当前文本。
    pub(crate) fn blame_of(&self, id: DocumentId) -> Option<Arc<Blame>> {
        self.blame.data.get(&id).map(|e| e.blame.clone())
    }

    /// 返回窗口坐标 `(x, y)`（逻辑像素）处逐行追溯一栏对应的行号（从 0 起）。
    pub(crate) fn blame_at(&self, x: f32, y: f32) -> Option<usize> {
        let width = self.blame_width();
        let l = self.layout;
        if width == 0.0 || !(l.gutter.x..l.gutter.x + width).contains(&x) {
            return None;
        }
        if y < l.text.y || y > l.text.bottom() {
            return None;
        }
        let doc = self.editor.active()?;
        let row =
            (f64::from((y - l.text.y) / self.fonts.line_height) + doc.scroll_line).floor() as usize;
        let line = doc.line_of_row(row);
        (line < doc.text().line_count()).then_some(line)
    }

    /// 返回最后修改第 `line` 行的提交。
    pub(crate) fn blame_commit(&self, line: usize) -> Option<BlameCommit> {
        let doc = self.editor.active()?;
        let blame = self.blame_of(doc.id)?;
        let index = *blame.lines.get(line)?;
        blame.commits.get(index as usize).cloned()
    }

    /// 返回鼠标悬停在逐行追溯第 `line` 行时的提示：锚点矩形，以及「a1b2c3d 说明 · 作者 · 时间」。
    pub(crate) fn blame_tip(&self, line: usize) -> Option<(Rect, String)> {
        let commit = self.blame_commit(line)?;
        let doc = self.editor.active()?;
        let l = self.layout;
        let lh = self.fonts.line_height;
        let y = l.text.y + (doc.row_of(line) as f64 - doc.scroll_line) as f32 * lh;
        let rect = Rect::new(l.gutter.x + 8.0, y, BLAME_W - 16.0, lh);
        let label = if commit.uncommitted {
            t!("git-blame-uncommitted").to_owned()
        } else {
            format!(
                "{} {} · {} · {}",
                short(&commit.oid),
                commit.summary,
                commit.author,
                local_text(commit.time, commit.offset)
            )
        };
        Some((rect, label))
    }

    /// 处理对逐行追溯第 `line` 行的点击：在日志标签中选中对应的提交。
    pub(in crate::app) fn click_blame(&mut self, line: usize) {
        let Some(commit) = self.blame_commit(line) else {
            return;
        };
        if commit.uncommitted {
            self.flash(t!("git-blame-line-uncommitted"));
            return;
        }
        self.git_show_log_at(&commit.oid);
    }
}

/// 返回逐行追溯一栏显示的时间：今天、昨天、若干分钟前，或日期（按作者所在时区）。
pub(crate) fn blame_time(commit: &BlameCommit, now: i64) -> String {
    display_at(commit.time + commit.offset, commit.time, now)
}

/// 返回文本从 `old` 变为 `new` 后平移得到的逐行追溯。
///
/// 首尾未变的行沿用 `blame` 中的提交，中间修改过的行记为「未提交」。只按整行比较首尾相同的
/// 部分，不运行 `git`；行数随改动增减。`git` 不追溯末尾换行符之后的空行，此处同样不补。
fn shifted(blame: &Blame, old: &Rope, new: &Rope) -> Blame {
    let head = old
        .lines()
        .zip(new.lines())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_n, new_n) = (old.len_lines(), new.len_lines());
    let mut tail = 0;
    let (mut a, mut b) = (old.lines_at(old_n), new.lines_at(new_n));
    while tail < old_n.min(new_n) - head {
        match (a.prev(), b.prev()) {
            (Some(x), Some(y)) if x == y => tail += 1,
            _ => break,
        }
    }
    let mut commits = blame.commits.clone();
    let uncommitted = match commits.iter().position(|c| c.uncommitted) {
        Some(i) => i,
        None => {
            commits.push(BlameCommit {
                uncommitted: true,
                ..BlameCommit::default()
            });
            commits.len() - 1
        }
    } as u32;
    let row = |line: usize| blame.lines.get(line).copied().unwrap_or(uncommitted);
    let lines = (0..head)
        .map(row)
        .chain(std::iter::repeat_n(uncommitted, new_n - head - tail))
        .chain((old_n - tail..old_n).map(row))
        .take((blame.lines.len() + new_n).saturating_sub(old_n))
        .collect();
    Blame { commits, lines }
}

#[cfg(test)]
mod tests;
