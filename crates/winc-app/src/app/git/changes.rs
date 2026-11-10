//! 编辑区行号栏中的 Git 改动标记：与 `HEAD` 中的版本逐行比较，新增、修改的行在行号右侧显示
//! 细色条，删除处显示小三角；点击标记弹出卡片，提供原内容、回滚和显示差异。
//!
//! 各文件在 `HEAD` 中的内容（基准）在首次使用时于后台读取，按文档编码解码，`HEAD` 变化时全部
//! 作废并重新读取。改动按文档版本号缓存，文本变化时在后台重新计算，完成前继续绘制上一次的结果；
//! 回滚、跳转和打开卡片需要准确的行号，标记不是最新时当场计算。绘制由 `view/git/changes.rs` 完成。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use ropey::Rope;

use winc_core::diff::{self, LineHunk};
use winc_core::{Change, EditKind, Lines, Transaction};
use winc_editor::diff_view::inline_marks;
use winc_editor::encoding::decode_known;
use winc_editor::format::Cancel;
use winc_editor::{DocumentId, LARGE_FILE, looks_binary};
use winc_i18n::t;
use winc_render::Rect;

use crate::layout::{GIT_LANE, Ruler};

use crate::app::{App, UserEvent};

/// 改动标记色条的宽度（逻辑像素），位于行号右侧。
///
/// 取自调参：宽于 1 像素，非整数缩放时不会模糊成两个像素宽的淡色条。
pub(crate) const MARK_W: f32 = 3.0;
/// 改动标记的点击判定宽度（逻辑像素）。
///
/// 取自调参：宽于色条以便点中，且不延伸到右侧的折叠箭头。
const MARK_HIT_W: f32 = 7.0;

/// 返回行号栏中改动标记的左边界（逻辑像素），位于行号与折叠箭头之间的空隙。
pub(crate) fn mark_x(gutter: Rect) -> f32 {
    crate::app::fold_column(gutter).x + 2.0
}

/// 后台计算完成的改动。
struct Computed {
    id: DocumentId,
    /// 计算时的文档版本号（`Document::revision`）。
    revision: u64,
    /// 计算时的基准编号（[`LineChanges::epoch`]）；与当前编号不一致时丢弃。
    epoch: u64,
    hunks: Vec<LineHunk>,
}

/// 一个文件的改动，按文档版本号缓存。
struct Marks {
    /// 计算时的文档版本号（`Document::revision`）。
    revision: u64,
    /// 计算时的基准编号（[`LineChanges::epoch`]）。
    epoch: u64,
    hunks: Vec<LineHunk>,
}

/// 卡片上可点击的控件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTarget {
    /// 上一处改动。
    Previous,
    /// 下一处改动。
    Next,
    /// 回滚该处改动。
    Revert,
    /// 打开该文件的差异。
    ShowDiff,
    /// 关闭卡片。
    Close,
}

/// 已打开的卡片：所属文档与改动的下标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangePopup {
    /// 卡片所属的文档。
    pub doc: DocumentId,
    /// 文档改动的下标（[`LineChanges::hunks`]）。
    pub hunk: usize,
    /// 卡片的锚定行（从 0 起，左闭右开，可以为空）。
    ///
    /// 卡片位于这些行下方，空间不足时位于上方，不遮挡这些行。点击标记打开时为被点击的行，
    /// 改动很长时卡片也不超出屏幕；删除处为删除位置的分界。
    pub anchor: (usize, usize),
}

/// 卡片中绘制的原内容。
///
/// 按（文档版本号、基准、该处改动）缓存，避免每帧重新查找基准、逐词比较和着色。
pub(crate) struct Preview {
    key: PreviewKey,
    /// 着色时是否使用了文档在该处改动开头的高亮状态；尚未推算到时从头着色，下一帧重新计算。
    settled: bool,
    /// 原内容的各行，最多 `PREVIEW_LINES` 行。
    pub lines: Vec<PreviewLine>,
    /// 未显示的行数。
    pub more: usize,
}

#[derive(PartialEq, Eq)]
struct PreviewKey {
    doc: DocumentId,
    /// 文档的版本号（`Document::revision`）。
    revision: u64,
    /// 缓存时的基准编号（[`LineChanges::epoch`]）。
    epoch: u64,
    hunk: LineHunk,
    language: *const winc_syntax::Language,
}

/// 卡片中原内容的一行。
pub(crate) struct PreviewLine {
    /// 该行的字符（不含换行符）。
    pub chars: Vec<char>,
    /// 该行的高亮记号。
    pub tokens: Vec<winc_syntax::Token>,
    /// 相对当前内容修改过的字符区间（按词比较）。
    pub words: Vec<std::ops::Range<usize>>,
}

/// 卡片中最多显示的原内容行数（行）。
///
/// 取自调参：保持卡片高度适中；超出部分显示为「还有 N 行」。
const PREVIEW_LINES: usize = 14;

/// 后台读取的基准。
struct Fetched {
    path: PathBuf,
    /// 读取时的基准编号（[`LineChanges::epoch`]）；与当前编号不一致时丢弃。
    epoch: u64,
    /// 文件在 `HEAD` 中的内容；不在 `HEAD` 中、为二进制、过大或读取失败时为 `None`，不显示标记。
    text: Option<Arc<str>>,
}

/// 行号栏改动标记的状态。
pub(crate) struct LineChanges {
    /// 各文件的基准；`None` 表示该文件不显示标记（不在 `HEAD` 中、二进制、过大）。
    bases: HashMap<PathBuf, Option<Arc<str>>>,
    /// 正在后台读取基准的文件。
    pending: HashSet<PathBuf>,
    /// 基准对应的 `HEAD` 提交号；变化时基准作废。
    oid: Option<String>,
    /// 基准编号，每作废一次加 1；作废前发出的请求送达时据此丢弃。
    epoch: u64,
    marks: HashMap<DocumentId, Marks>,
    /// 正在后台重新计算的文档及其版本号；同一文档同一时刻只计算一次。
    computing: HashMap<DocumentId, u64>,
    computed: (Sender<Computed>, Receiver<Computed>),
    /// 已打开的卡片。
    pub popup: Option<ChangePopup>,
    /// 卡片中的原内容。
    preview: Option<Rc<Preview>>,
    results: (Sender<Fetched>, Receiver<Fetched>),
}

impl Default for LineChanges {
    fn default() -> Self {
        LineChanges {
            bases: HashMap::new(),
            pending: HashSet::new(),
            oid: None,
            epoch: 0,
            marks: HashMap::new(),
            computing: HashMap::new(),
            computed: channel(),
            popup: None,
            preview: None,
            results: channel(),
        }
    }
}

impl LineChanges {
    /// 作废所有基准和改动（`HEAD` 变化、仓库更换或模块关闭时）。
    pub(super) fn forget(&mut self) {
        self.bases.clear();
        self.pending.clear();
        self.marks.clear();
        self.computing.clear();
        self.popup = None;
        self.preview = None;
        self.epoch += 1;
    }

    /// 在文档 `id` 关闭后释放其改动；卡片属于该文档时关闭卡片。
    pub(super) fn forget_document(&mut self, id: DocumentId) {
        self.marks.remove(&id);
        self.computing.remove(&id);
        if self.popup.is_some_and(|p| p.doc == id) {
            self.popup = None;
        }
        if self.preview.as_ref().is_some_and(|p| p.key.doc == id) {
            self.preview = None;
        }
    }

    /// 在文件 `path` 不再有打开的文档时释放其基准，再次打开时重新读取。
    ///
    /// 正在读取的基准送达后同样丢弃。
    pub(super) fn forget_base(&mut self, path: &std::path::Path) {
        self.bases.remove(path);
        self.pending.remove(path);
    }

    /// 记录 `HEAD` 当前指向 `oid`；与基准所取自的提交不同时作废基准。
    pub(super) fn head_is(&mut self, oid: Option<&str>) {
        if self.oid.as_deref() != oid {
            self.oid = oid.map(str::to_owned);
            self.forget();
        }
    }

    /// 返回基准对应的 `HEAD` 提交号的前 7 位；不在仓库中或尚无提交时返回 `None`。
    pub(crate) fn short_head(&self) -> Option<&str> {
        self.oid.as_deref().map(|oid| &oid[..oid.len().min(7)])
    }

    /// 返回文件 `path` 的基准（在 `HEAD` 中的内容）；尚未读取或不显示标记时返回 `None`。
    pub(crate) fn base(&self, path: &std::path::Path) -> Option<&str> {
        self.bases.get(path)?.as_deref()
    }

    /// 返回文档 `id` 已计算的改动；没有基准或不显示标记时为空。
    pub(crate) fn hunks(&self, id: DocumentId) -> &[LineHunk] {
        self.marks.get(&id).map_or(&[], |m| m.hunks.as_slice())
    }
}

impl App {
    /// 返回第 `index` 个文档的 `id` 及其在仓库中的路径；该文档不显示标记时返回 `None`。
    fn line_change_inputs(&mut self, index: usize) -> Option<(DocumentId, PathBuf)> {
        let root = self.git.root.as_deref()?;
        let doc = &self.editor.documents[index];
        let path = doc.path.clone().filter(|p| p.starts_with(root))?;
        if !doc.shows_text() || doc.is_large() {
            return None;
        }
        Some((doc.id, path))
    }

    /// 在绘制前更新第 `index` 个文档的改动：没有基准时在后台读取；文本变化时在后台重新计算，
    /// 完成前继续使用上一次的结果。
    ///
    /// 同一文档同一时刻只计算一次。计算期间又有输入时，完成后的下一帧按最新文本再计算一次，
    /// 连续输入时不会排队执行多次全文比较。
    pub(crate) fn update_line_changes(&mut self, index: usize) {
        let Some((id, path)) = self.line_change_inputs(index) else {
            return;
        };
        let doc = &self.editor.documents[index];
        let (revision, epoch) = (doc.revision, self.changes.epoch);
        let base = match self.changes.bases.get(&path) {
            None => {
                if self.changes.pending.insert(path.clone()) {
                    self.fetch_base(path, doc.encoding);
                }
                return;
            }
            Some(None) => {
                self.changes.marks.remove(&id);
                return;
            }
            Some(Some(base)) => base.clone(),
        };
        let fresh = self
            .changes
            .marks
            .get(&id)
            .is_some_and(|m| m.revision == revision && m.epoch == epoch);
        if fresh || self.changes.computing.contains_key(&id) {
            return;
        }
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        self.changes.computing.insert(id, revision);
        let sender = self.changes.computed.0.clone();
        self.spawn_then(UserEvent::GitBase, move || {
            let hunks = compare(&base, &rope);
            let _ = sender.send(Computed {
                id,
                revision,
                epoch,
                hunks,
            });
        });
    }

    /// 返回已打开的卡片中绘制的原内容；缓存仍然有效时直接使用缓存。
    pub(crate) fn change_preview(&mut self) -> Option<Rc<Preview>> {
        let popup = self.changes.popup?;
        let index = self
            .editor
            .documents
            .iter()
            .position(|d| d.id == popup.doc)?;
        let doc = &self.editor.documents[index];
        let hunk = self.changes.hunks(doc.id).get(popup.hunk)?.clone();
        let key = PreviewKey {
            doc: doc.id,
            revision: doc.revision,
            epoch: self.changes.epoch,
            hunk: hunk.clone(),
            language: doc.highlighter.language(),
        };
        if let Some(preview) = &self.changes.preview
            && preview.key == key
            && preview.settled
        {
            return Some(preview.clone());
        }
        let base = self.changes.base(doc.path.as_deref()?)?;
        let old = lines_of(base, hunk.old.clone());
        let new = current_lines(doc, &hunk);
        let language = doc.highlighter.language();
        // 按词标记修改过的字符。
        let marks = (!hunk.old.is_empty() && !hunk.new.is_empty())
            .then(|| {
                let local = LineHunk {
                    old: 0..hunk.old.len(),
                    new: 0..hunk.new.len(),
                };
                inline_marks(&Rope::from_str(&old), &Rope::from_str(&new), &local)
            })
            .flatten();
        // 着色沿用文档在该处改动开头的高亮状态，使块注释、多行字符串中的行颜色正确。
        let state = self.editor.documents[index].highlight_state(hunk.new.start, 20_000);
        let settled = state.is_some();
        let mut state = state.unwrap_or_default();
        let mut lines = Vec::new();
        for (i, line) in old.split_inclusive('\n').take(PREVIEW_LINES).enumerate() {
            let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
            let mut tokens = Vec::new();
            state = winc_syntax::highlight_line(language, &chars, state, &mut tokens);
            let words = marks
                .as_ref()
                .and_then(|m| m.old.get(i).cloned())
                .unwrap_or_default();
            lines.push(PreviewLine {
                chars,
                tokens,
                words,
            });
        }
        let preview = Rc::new(Preview {
            key,
            settled,
            more: hunk.old.len().saturating_sub(lines.len()),
            lines,
        });
        self.changes.preview = Some(preview.clone());
        Some(preview)
    }

    /// 在回滚、跳转或打开卡片之前确保当前文档的改动是最新的，必要时当场计算，以保证行号准确。
    fn fresh_line_changes(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        let Some((id, path)) = self.line_change_inputs(index) else {
            return;
        };
        let Some(Some(base)) = self.changes.bases.get(&path).cloned() else {
            return;
        };
        let doc = &self.editor.documents[index];
        let (revision, epoch) = (doc.revision, self.changes.epoch);
        if self
            .changes
            .marks
            .get(&id)
            .is_some_and(|m| m.revision == revision && m.epoch == epoch)
        {
            return;
        }
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let hunks = compare(&base, rope);
        self.changes.marks.insert(
            id,
            Marks {
                revision,
                epoch,
                hunks,
            },
        );
    }

    fn fetch_base(&mut self, path: PathBuf, encoding: winc_editor::Encoding) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let sender = self.changes.results.0.clone();
        let epoch = self.changes.epoch;
        self.spawn_git(UserEvent::GitBase, move |tool| {
            let text = tool
                .ok()
                .and_then(|git| git.head_content(&root, &path, &Cancel::default()).ok())
                .flatten()
                .filter(|bytes| bytes.len() as u64 <= LARGE_FILE && !looks_binary(bytes))
                .map(|bytes| Arc::from(decode_known(&bytes, encoding)));
            let _ = sender.send(Fetched { path, epoch, text });
        });
    }

    /// 接收后台读取的基准与计算完成的改动。
    pub(in crate::app) fn poll_bases(&mut self) {
        while let Ok(fetched) = self.changes.results.1.try_recv() {
            if fetched.epoch != self.changes.epoch {
                continue;
            }
            if self.changes.pending.remove(&fetched.path) {
                self.changes.bases.insert(fetched.path, fetched.text);
            }
        }
        while let Ok(computed) = self.changes.computed.1.try_recv() {
            // 作废前发出的结果直接丢弃，且不从 `computing` 中移除：作废时 `computing` 已清空，
            // 其中再出现的该文档属于作废后新发出的请求。
            if computed.epoch != self.changes.epoch {
                continue;
            }
            self.changes.computing.remove(&computed.id);
            // 等待结果期间文档已关闭。
            if self.editor.document(computed.id).is_none() {
                continue;
            }
            // 只丢弃比已有结果更旧的结果；比文档当前版本旧的结果仍先使用，下一帧继续重新计算。
            let newer = self
                .changes
                .marks
                .get(&computed.id)
                .is_none_or(|m| m.epoch != computed.epoch || m.revision <= computed.revision);
            if newer {
                self.changes.marks.insert(
                    computed.id,
                    Marks {
                        revision: computed.revision,
                        epoch: computed.epoch,
                        hunks: computed.hunks,
                    },
                );
            }
            // 卡片指向的改动可能已不存在。
            if let Some(popup) = self.changes.popup
                && popup.doc == computed.id
                && popup.hunk >= self.changes.hunks(computed.id).len()
            {
                self.changes.popup = None;
            }
        }
        self.request_redraw();
    }

    /// 返回窗口坐标 `(x, y)`（逻辑像素）处改动标记所属的改动下标。
    pub(in crate::app) fn change_mark_at(&self, x: f32, y: f32) -> Option<usize> {
        self.change_mark_hit(x, y).map(|(hunk, _)| hunk)
    }

    /// 返回窗口坐标 `(x, y)`（逻辑像素）处的改动标记：（改动下标，所在行号）。
    fn change_mark_hit(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        // 三栏合并不绘制这些标记。
        if self.merge_panes_placed() {
            return None;
        }
        let l = self.layout;
        let left = mark_x(l.gutter) - 2.0;
        if !(left..left + MARK_HIT_W).contains(&x) || y < l.text.y || y > l.text.bottom() {
            return None;
        }
        let doc = self.editor.active()?;
        let hunks = self.changes.hunks(doc.id);
        if hunks.is_empty() {
            return None;
        }
        let row =
            (f64::from((y - l.text.y) / self.fonts.line_height) + doc.scroll_line).floor() as usize;
        let line = doc.line_of_row(row);
        let hunk = hunks.iter().position(|h| {
            if h.new.is_empty() {
                // 删除处的标记绘制在下一行顶边，上一行和下一行都可命中。
                line == h.new.start || line + 1 == h.new.start
            } else {
                h.new.contains(&line)
            }
        })?;
        Some((hunk, line))
    }

    /// 返回窗口坐标 `(x, y)` 处概览标尺上的改动下标及其标记的位置。
    ///
    /// 判定范围宽于色条，上下各扩展 2 像素以便点中，不延伸到右侧滑块所在的列。
    /// 多个标记相邻时取距离最近的一个。
    pub(crate) fn ruler_change_at(&self, x: f32, y: f32) -> Option<(usize, Rect)> {
        // 三栏合并不绘制这些标记。
        if self.merge_panes_placed() {
            return None;
        }
        let doc = self.editor.active().filter(|d| d.shows_text())?;
        let hunks = self.changes.hunks(doc.id);
        let track = self.layout.editor;
        let left = track.right() - GIT_LANE.0;
        if hunks.is_empty()
            || !(left - 2.0..left + GIT_LANE.1).contains(&x)
            || !track.contains(x, y)
        {
            return None;
        }
        let ruler = Ruler::new(track, doc.row_count());
        let distance = |r: &Rect| (y - y.clamp(r.y, r.bottom())).abs();
        hunks
            .iter()
            .enumerate()
            .map(|(i, h)| (i, ruler.rect(GIT_LANE, ruler_rows(doc, h))))
            .filter(|(_, r)| distance(r) <= 2.0)
            .min_by(|a, b| distance(&a.1).total_cmp(&distance(&b.1)))
    }

    /// 返回鼠标悬停在概览标尺第 `hunk` 处改动上时的提示。
    ///
    /// 返回锚点矩形（标记上鼠标所在的高度）与「修改 · 第 3–4 行」形式的说明（按当前行号）。
    pub(crate) fn ruler_tip(&self, hunk: usize) -> Option<(Rect, String)> {
        let doc = self.editor.active()?;
        let h = self.changes.hunks(doc.id).get(hunk)?;
        let ruler = Ruler::new(self.layout.editor, doc.row_count());
        let mark = ruler.rect(GIT_LANE, ruler_rows(doc, h));
        // 提示跟随鼠标高度：数十行的改动在标尺上较长，锚定在下沿时会远离鼠标。
        let y = self.mouse.1.clamp(mark.y, mark.bottom());
        let rect = Rect::new(mark.x, y, mark.w, 10.0);
        let lines = |r: &std::ops::Range<usize>| match r.len() {
            1 => t!("git-change-line", line = r.start + 1),
            _ => t!("git-change-lines", start = r.start + 1, end = r.end),
        };
        let label = match (h.old.is_empty(), h.new.is_empty()) {
            (true, _) => t!("git-change-added", lines = lines(&h.new)),
            (_, true) if h.new.start == 0 => {
                t!("git-change-deleted-at-start", count = h.old.len())
            }
            (_, true) => {
                let (count, line) = (h.old.len(), h.new.start);
                t!("git-change-deleted-after", count = count, line = line)
            }
            _ => t!("git-change-modified", lines = lines(&h.new)),
        };
        Some((rect, label))
    }

    /// 处理对概览标尺上第 `hunk` 处改动的点击：光标移到该改动的第一行。
    ///
    /// 改动未完全可见时滚动到距顶部三行处。不打开卡片：标尺只负责定位，卡片由行号栏的标记打开。
    pub(in crate::app) fn go_to_change(&mut self, hunk: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(h) = self.changes.hunks(doc.id).get(hunk).cloned() else {
            return;
        };
        let line = h.new.start.min(doc.text().line_count() - 1);
        let pos = doc.text().line_start(line);
        let visible = self.visible_lines() as f64;
        self.changes.popup = None;
        self.focus = crate::app::Focus::Editor;
        if let Some(doc) = self.editor.active_mut() {
            let rows = ruler_rows(doc, &h);
            let (top, bottom) = (rows.start as f64, rows.end.max(rows.start + 1) as f64);
            if top < doc.scroll_line || bottom > doc.scroll_line + visible {
                doc.scroll_line = (top - 3.0).max(0.0);
            }
        }
        self.clamp_scroll();
        // 滚动完成后再设置光标，此时光标已可见，不再触发滚动。
        self.select(pos, pos);
    }

    /// 处理对 `(x, y)` 处改动标记的点击：打开该处改动的卡片，锚定在被点击的行。
    ///
    /// 未点中标记时返回 `false`。
    pub(in crate::app) fn click_change_mark(&mut self, x: f32, y: f32) -> bool {
        if self.change_mark_hit(x, y).is_none() {
            return false;
        }
        // 标记不是最新时先重新计算，以保证行号准确。
        self.fresh_line_changes();
        let Some((hunk, line)) = self.change_mark_hit(x, y) else {
            return true;
        };
        let Some(doc) = self.editor.active() else {
            return true;
        };
        let id = doc.id;
        let h = &self.changes.hunks(id)[hunk];
        let anchor = if h.new.is_empty() {
            (h.new.start, h.new.start)
        } else {
            (line, line + 1)
        };
        self.changes.popup = Some(ChangePopup {
            doc: id,
            hunk,
            anchor,
        });
        self.request_redraw();
        true
    }

    /// 打开第 `hunk` 处改动的卡片（用于上一处、下一处），并使其可见。
    ///
    /// 卡片位于该处改动下方；改动过长、最后一行不可见时位于第一行下方。
    pub(in crate::app) fn open_change_popup(&mut self, hunk: usize) {
        self.fresh_line_changes();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        let Some(h) = self.changes.hunks(id).get(hunk).cloned() else {
            return;
        };
        self.reveal_line(h.new.start);
        let anchor = if h.new.is_empty() {
            (h.new.start, h.new.start)
        } else {
            let visible = self.visible_lines() as f64;
            let doc = self
                .editor
                .active()
                .expect("开头确认过有当前文档，滚动不会关掉它");
            let last = doc.row_of(h.new.end - 1) as f64;
            if last + 1.0 <= doc.scroll_line + visible {
                (h.new.start, h.new.end)
            } else {
                (h.new.start, h.new.start + 1)
            }
        };
        self.changes.popup = Some(ChangePopup {
            doc: id,
            hunk,
            anchor,
        });
        self.request_redraw();
    }

    /// 处理对卡片上 `target` 的点击。
    pub(in crate::app) fn press_change(&mut self, target: ChangeTarget) {
        let Some(popup) = self.changes.popup else {
            return;
        };
        let count = self.changes.hunks(popup.doc).len();
        match target {
            ChangeTarget::Previous if popup.hunk > 0 => self.open_change_popup(popup.hunk - 1),
            ChangeTarget::Next if popup.hunk + 1 < count => self.open_change_popup(popup.hunk + 1),
            ChangeTarget::Previous | ChangeTarget::Next => {}
            ChangeTarget::Revert => self.revert_change(popup.hunk),
            ChangeTarget::ShowDiff => {
                self.changes.popup = None;
                self.git_show_changes();
            }
            ChangeTarget::Close => self.changes.popup = None,
        }
        self.request_redraw();
    }

    /// 回滚当前文档的第 `hunk` 处改动，将这些行恢复为 `HEAD` 中的内容。
    ///
    /// 回滚是一次普通的编辑，可以用 `⌘Z` 撤销。
    pub(super) fn revert_change(&mut self, hunk: usize) {
        self.fresh_line_changes();
        let Some(index) = self.editor.active else {
            return;
        };
        let doc = &self.editor.documents[index];
        let Some(h) = self.changes.hunks(doc.id).get(hunk).cloned() else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            return;
        };
        let Some(Some(base)) = self.changes.bases.get(&path).cloned() else {
            return;
        };
        let inserted = lines_of(&base, h.old.clone());
        if let Err(reason) = replace_lines(&mut self.editor.documents[index], h.new, inserted) {
            self.flash(reason);
            return;
        }
        self.changes.popup = None;
        self.flash(t!("git-change-reverted"));
        self.request_redraw();
    }

    /// 跳转到当前文档中下一处（`forward`）或上一处改动的第一行，返回当前文档是否有改动标记。
    ///
    /// 没有更多改动时在状态栏提示；返回 `false` 时不做任何操作。
    pub(in crate::app) fn jump_change(&mut self, forward: bool) -> bool {
        self.fresh_line_changes();
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let hunks = self.changes.hunks(doc.id);
        if hunks.is_empty() {
            return false;
        }
        let caret = doc.text().line_of(doc.selection.head);
        let target = if forward {
            hunks.iter().find(|h| h.new.start > caret)
        } else {
            hunks.iter().rev().find(|h| h.new.start < caret)
        };
        match target.map(|h| h.new.start) {
            Some(line) => {
                let pos = doc.text().line_start(line.min(doc.text().line_count() - 1));
                self.changes.popup = None;
                self.select(pos, pos);
            }
            None if forward => self.flash(t!("git-change-no-next")),
            None => self.flash(t!("git-change-no-previous")),
        }
        true
    }

    /// 回滚光标所在的那一处改动（命令「Git：回滚光标处的改动」）。
    pub(in crate::app) fn revert_change_at_caret(&mut self) {
        self.fresh_line_changes();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let caret = doc.text().line_of(doc.selection.head);
        let hit = self
            .changes
            .hunks(doc.id)
            .iter()
            .position(|h| h.new.contains(&caret) || (h.new.is_empty() && h.new.start == caret));
        match hit {
            Some(hunk) => self.revert_change(hunk),
            None => self.flash(t!("git-change-none-at-caret")),
        }
    }

    /// 滚动编辑区使第 `line` 行可见，光标不变。
    fn reveal_line(&mut self, line: usize) {
        let visible = self.visible_lines() as f64;
        if let Some(doc) = self.editor.active_mut() {
            let row = doc.row_of(line) as f64;
            if row < doc.scroll_line + 1.0 || row > doc.scroll_line + visible - 6.0 {
                doc.scroll_line = (row - 3.0).max(0.0);
            }
        }
        self.clamp_scroll();
    }
}

/// 返回一处改动在概览标尺上占用的显示行范围（行）。
///
/// 范围为当前文档中该改动的各行；删除处为删除位置的分界，文件末尾的删除位于最后一行之下。
/// 绘制与点击判定共用该函数。
pub(crate) fn ruler_rows(doc: &winc_editor::Document, h: &LineHunk) -> std::ops::Range<usize> {
    let count = doc.text().line_count();
    if h.new.is_empty() {
        let at = if h.new.start >= count {
            doc.row_count()
        } else {
            doc.row_of(h.new.start)
        };
        return at..at;
    }
    doc.row_of(h.new.start)..doc.row_of(h.new.end - 1) + doc.rows_of(h.new.end - 1)
}

/// 返回文档中该处改动当前的各行（含换行符），用于与原内容逐词比较。
fn current_lines(doc: &winc_editor::Document, hunk: &LineHunk) -> String {
    let text = doc.text();
    let start = |line: usize| {
        if line >= text.line_count() {
            text.end_pos()
        } else {
            text.line_start(line)
        }
    };
    doc.text_range(start(hunk.new.start), start(hunk.new.end))
        .unwrap_or_default()
}

fn compare(base: &str, rope: &Rope) -> Vec<LineHunk> {
    diff::line_hunks(base, &rope.to_string())
}

/// 将 `doc` 的第 `lines` 行（含换行符）替换为 `inserted`，作为一步可撤销的编辑；光标不变。
///
/// # Errors
///
/// 文档不可编辑（只读、大文件）时返回原因。
pub(crate) fn replace_lines(
    doc: &mut winc_editor::Document,
    lines: std::ops::Range<usize>,
    inserted: String,
) -> Result<(), &'static str> {
    if let Some(reason) = doc.edit_blocked(&winc_editor::Command::Insert(String::new())) {
        return Err(reason);
    }
    let text = doc.text();
    let line_start = |line: usize| {
        if line >= text.line_count() {
            text.end_pos()
        } else {
            text.line_start(line)
        }
    };
    let start = line_start(lines.start);
    let end = if lines.is_empty() {
        start
    } else {
        line_start(lines.end)
    };
    let removed = doc.text_range(start, end).unwrap_or_default();
    let selection = doc.selection;
    let change = Change {
        start,
        removed,
        inserted,
    };
    doc.apply(Transaction::new(
        vec![change],
        selection,
        selection,
        EditKind::Other,
    ));
    Ok(())
}

/// 返回文本中第 `lines` 行范围（从 0 起，左闭右开）的内容，保留各行的换行符。
///
/// 换行符包括 `\n`、`\r\n` 和单独的 `\r`，分行规则同 `diff::line_hunks`。按 `\n` 分段查找，
/// 段内含 `\r` 时才细分：每次打开卡片都要从基准开头计数到该处改动，
/// 数万行的文件逐字节计数需要 1–2 ms。
pub(crate) fn lines_of(text: &str, lines: std::ops::Range<usize>) -> String {
    let mut out = String::new();
    let mut line = 0;
    let mut rest = text;
    while line < lines.end && !rest.is_empty() {
        let segment = rest.find('\n').map_or(rest, |i| &rest[..=i]);
        rest = &rest[segment.len()..];
        // 段内单独的 `\r` 同样是换行符（`\r\n` 中的 `\r` 除外）。
        let body = segment.strip_suffix('\n').unwrap_or(segment);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let ends = body
            .match_indices('\r')
            .map(|(i, _)| i + 1)
            .chain(std::iter::once(segment.len()));
        let mut start = 0;
        for end in ends {
            if lines.contains(&line) {
                out.push_str(&segment[start..end]);
            }
            line += 1;
            start = end;
        }
    }
    out
}

#[cfg(test)]
mod tests;
