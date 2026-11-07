//! 编辑区行号栏里的 Git 改动标记：和 `HEAD` 里的版本逐行比较，新增、修改的行在行号右边一道
//! 细色条，删掉的地方一个小三角；点标记弹出一张卡片，有原内容、回滚、显示差异。
//!
//! 每个文件在 `HEAD` 里的内容用到时在后台取，按文档编码解码，`HEAD` 变了就整体作废重取。改动按
//! 文档的版本缓存，文字没变不重算，变了在后台重算，算好之前继续画上一次的；回滚、跳转、打开卡片
//! 要准确行号，标记不是最新时就当场算一次。绘制在 `view/git/changes.rs`。

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

/// 标记那一条的宽度（逻辑像素），贴在行号右边。
///
/// 3 是经验值：比 1 像素宽，非整数缩放时不会糊成两像素。
pub(crate) const MARK_W: f32 = 3.0;
/// 点标记时认的宽度（逻辑像素）：比色条宽一些，好点中；不伸到右边折叠箭头上（经验值 7）。
const MARK_HIT_W: f32 = 7.0;

/// 行号栏里改动标记的左边（逻辑像素）：行号右边、折叠箭头左边那一小条空当。
pub(crate) fn mark_x(gutter: Rect) -> f32 {
    crate::app::fold_column(gutter).x + 2.0
}

/// 后台算好的改动。
struct Computed {
    id: DocumentId,
    /// 算的是哪个版本号（`Document::revision`）。
    revision: u64,
    /// 算的时候的基准编号（[`LineChanges::epoch`]）：对不上就丢弃。
    epoch: u64,
    hunks: Vec<LineHunk>,
}

/// 一个文件的改动，按文档版本缓存。
struct Marks {
    /// 算的是哪个版本号（`Document::revision`）。
    revision: u64,
    /// 算的时候的基准编号（[`LineChanges::epoch`]）。
    epoch: u64,
    hunks: Vec<LineHunk>,
}

/// 卡片上可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTarget {
    /// 上一处改动。
    Previous,
    /// 下一处改动。
    Next,
    /// 回滚这一块。
    Revert,
    /// 打开该文件的差异。
    ShowDiff,
    /// 关闭卡片。
    Close,
}

/// 打开着的卡片：哪个文档的第几处改动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangePopup {
    /// 卡片开在哪个文档上。
    pub doc: DocumentId,
    /// 文档改动的下标（[`LineChanges::hunks`]）。
    pub hunk: usize,
    /// 卡片跟着的行（左闭右开，可以为空）：卡片贴在它们下面，放不下贴在上面，不盖住它们。
    ///
    /// 点标记打开时是点的那一行，改动很长时卡片也不会超出屏幕；删掉的地方是那道分界。
    pub anchor: (usize, usize),
}

/// 卡片里要画的原来的内容：按（文档版本、基准、这一处改动）缓存，每帧不再从头翻基准、逐词
/// 比较、着色。
pub(crate) struct Preview {
    key: PreviewKey,
    /// 着色时用上了文档在这一块开头的状态；还没推算到时先从头着色，下一帧再算一次。
    settled: bool,
    /// 原来的行，最多 `PREVIEW_LINES` 行。
    pub lines: Vec<PreviewLine>,
    /// 没显示出来的行数。
    pub more: usize,
}

#[derive(PartialEq, Eq)]
struct PreviewKey {
    doc: DocumentId,
    /// 文档的版本号（`Document::revision`）。
    revision: u64,
    /// 当时的基准编号（[`LineChanges::epoch`]）。
    epoch: u64,
    hunk: LineHunk,
    language: *const winc_syntax::Language,
}

/// 卡片里原来的一行。
pub(crate) struct PreviewLine {
    /// 这一行的字符（不含换行符）。
    pub chars: Vec<char>,
    /// 这一行的高亮记号。
    pub tokens: Vec<winc_syntax::Token>,
    /// 和现在的内容相比改了的字符区间（按词比较）。
    pub words: Vec<std::ops::Range<usize>>,
}

/// 卡片里最多显示的原来的行数（行，经验值）：超出部分写「还有 N 行」。
const PREVIEW_LINES: usize = 14;

/// 后台取到的基准。
struct Fetched {
    path: PathBuf,
    /// 取的时候的基准编号（[`LineChanges::epoch`]）：对不上就丢弃。
    epoch: u64,
    /// 文件在 `HEAD` 里的内容；不在 `HEAD` 里、是二进制、太大或取不到时为 `None`（不标）。
    text: Option<Arc<str>>,
}

/// 行号栏改动标记的状态。
pub(crate) struct LineChanges {
    /// 各文件的基准；`None` 表示该文件不标（不在 `HEAD` 里、二进制、太大）。
    bases: HashMap<PathBuf, Option<Arc<str>>>,
    /// 正在后台取基准的文件。
    pending: HashSet<PathBuf>,
    /// 基准对应的 `HEAD` 提交号；变了就作废。
    oid: Option<String>,
    /// 基准每作废一次加 1；作废之前发出的请求交回来时据此丢弃。
    epoch: u64,
    marks: HashMap<DocumentId, Marks>,
    /// 正在后台重算的文档和它们的版本；同一个文档同一时刻只算一次。
    computing: HashMap<DocumentId, u64>,
    computed: (Sender<Computed>, Receiver<Computed>),
    /// 打开着的卡片。
    pub popup: Option<ChangePopup>,
    /// 卡片上可以点的地方，绘制时更新。
    pub hits: Vec<(Rect, ChangeTarget)>,
    /// 卡片的位置，绘制时更新；点在卡片外面时关闭卡片。
    pub card: Option<Rect>,
    /// 卡片里原来的内容。
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
            hits: Vec::new(),
            card: None,
            preview: None,
            results: channel(),
        }
    }
}

impl LineChanges {
    /// 作废所有基准和改动（`HEAD` 变了、仓库换了、模块关了）。
    pub(super) fn forget(&mut self) {
        self.bases.clear();
        self.pending.clear();
        self.marks.clear();
        self.computing.clear();
        self.popup = None;
        self.preview = None;
        self.epoch += 1;
    }

    /// 文档 `id` 关闭了：放下它的改动；卡片开在它上面时关闭。
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

    /// 文件 `path` 不再有打开着的文档：放下它的基准（正在取的，取回来也不要了），再打开时
    /// 重新取一次。
    pub(super) fn forget_base(&mut self, path: &std::path::Path) {
        self.bases.remove(path);
        self.pending.remove(path);
    }

    /// `HEAD` 现在指向 `oid`：和基准取自的不一样时作废。
    pub(super) fn head_is(&mut self, oid: Option<&str>) {
        if self.oid.as_deref() != oid {
            self.oid = oid.map(str::to_owned);
            self.forget();
        }
    }

    /// 基准版本对应的 `HEAD` 提交号的前 7 位；不在仓库里、还没有提交时为 `None`。
    pub(crate) fn short_head(&self) -> Option<&str> {
        self.oid.as_deref().map(|oid| &oid[..oid.len().min(7)])
    }

    /// 文件 `path` 的基准（在 `HEAD` 里的内容）；还没取到、不标时为 `None`。
    pub(crate) fn base(&self, path: &std::path::Path) -> Option<&str> {
        self.bases.get(path)?.as_deref()
    }

    /// 文档 `id` 已经算好的改动；没有基准、不标时为空。
    pub(crate) fn hunks(&self, id: DocumentId) -> &[LineHunk] {
        self.marks.get(&id).map_or(&[], |m| m.hunks.as_slice())
    }
}

impl App {
    /// 第 `index` 个文档的 `id` 和它在仓库里的路径；不标该文档时返回 `None`。
    fn line_change_inputs(&mut self, index: usize) -> Option<(DocumentId, PathBuf)> {
        let root = self.git.root.as_deref()?;
        let doc = &self.editor.documents[index];
        let path = doc.path.clone().filter(|p| p.starts_with(root))?;
        if !doc.shows_text() || doc.is_large() {
            return None;
        }
        Some((doc.id, path))
    }

    /// 让第 `index` 个文档的改动尽量是最新的（绘制前调用）：没有基准时去后台取；文字变了时在
    /// 后台重算，算好之前继续用上一次的。
    ///
    /// 同一个文档同一时刻只算一次：算的期间又打了字，算完下一帧再按最新的文字算一次，连续输入
    /// 时不排一串整个文件的比较。
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

    /// 打开着的卡片里要画的原来的内容；缓存还对得上就直接用。
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
        // 按词标出改了的字，和差异视图一样。
        let marks = (!hunk.old.is_empty() && !hunk.new.is_empty())
            .then(|| {
                let local = LineHunk {
                    old: 0..hunk.old.len(),
                    new: 0..hunk.new.len(),
                };
                inline_marks(&Rope::from_str(&old), &Rope::from_str(&new), &local)
            })
            .flatten();
        // 着色沿用文档在这一块开头的状态，块注释、多行字符串里的几行也着对颜色。
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

    /// 回滚、跳转、打开卡片之前：当前文档的改动不是最新的就当场算一次，行号才准。
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

    /// 收下后台取到的基准和算好的改动。
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
            // 作废之前发出的结果：`computing` 在作废时已经清空，这里再出现该文档说明它是之后
            // 新发出的那一次。
            if computed.epoch != self.changes.epoch {
                continue;
            }
            self.changes.computing.remove(&computed.id);
            // 等结果的时候文档关闭了。
            if self.editor.document(computed.id).is_none() {
                continue;
            }
            // 比手上的旧才丢；比文档当前的旧也先用着（下一帧会继续重算）。
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
            // 卡片指向的那一处可能已经没有了。
            if let Some(popup) = self.changes.popup
                && popup.doc == computed.id
                && popup.hunk >= self.changes.hunks(computed.id).len()
            {
                self.changes.popup = None;
            }
        }
        self.request_redraw();
    }

    /// 鼠标在 `(x, y)` 时，下面是当前文档的第几处改动的标记。
    pub(in crate::app) fn change_mark_at(&self, x: f32, y: f32) -> Option<usize> {
        self.change_mark_hit(x, y).map(|(hunk, _)| hunk)
    }

    /// 鼠标在 `(x, y)` 时，下面的改动标记：第几处改动、指着的是哪一行。
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
        let row = ((y - l.text.y) / self.fonts.line_height + doc.scroll_line).floor() as usize;
        let line = doc.line_of_row(row);
        let hunk = hunks.iter().position(|h| {
            if h.new.is_empty() {
                // 删掉的地方：标记画在下一行的顶上，上一行和下一行都认。
                line == h.new.start || line + 1 == h.new.start
            } else {
                h.new.contains(&line)
            }
        })?;
        Some((hunk, line))
    }

    /// 鼠标在 `(x, y)` 时指着滚动条（概览标尺）上的第几处改动，和那道标记的位置。
    ///
    /// 认的范围比色条宽一些、上下各多 2 像素，便于点中；不伸到右边滑块那一列。
    /// 挨得近时取离得最近的。
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

    /// 鼠标停在概览标尺第 `hunk` 处改动上时的提示：提示挂在哪里（标记上鼠标所在的高度），和
    /// 「修改 · 第 3–4 行」这样的说明（行号是现在的）。
    pub(crate) fn ruler_tip(&self, hunk: usize) -> Option<(Rect, String)> {
        let doc = self.editor.active()?;
        let h = self.changes.hunks(doc.id).get(hunk)?;
        let ruler = Ruler::new(self.layout.editor, doc.row_count());
        let mark = ruler.rect(GIT_LANE, ruler_rows(doc, h));
        // 提示跟着鼠标的高度：几十行的一处改动在标尺上是一长条，贴着整条的下沿会离鼠标很远。
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

    /// 点了概览标尺上的第 `hunk` 处改动：光标移到该处改动的第一行；整处改动不是全在屏幕上
    /// 时滚过去，停在离顶上三行的地方。
    ///
    /// 不打开卡片：标尺只管定位，卡片从行号栏的标记打开。
    pub(in crate::app) fn go_to_change(&mut self, hunk: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(h) = self.changes.hunks(doc.id).get(hunk).cloned() else {
            return;
        };
        let line = h.new.start.min(doc.text().line_count() - 1);
        let pos = doc.text().line_start(line);
        let visible = self.visible_lines() as f32;
        self.changes.popup = None;
        self.focus = crate::app::Focus::Editor;
        if let Some(doc) = self.editor.active_mut() {
            let rows = ruler_rows(doc, &h);
            let (top, bottom) = (rows.start as f32, rows.end.max(rows.start + 1) as f32);
            if top < doc.scroll_line || bottom > doc.scroll_line + visible {
                doc.scroll_line = (top - 3.0).max(0.0);
            }
        }
        self.clamp_scroll();
        // 滚好了再放光标：光标已经在屏幕上，不会再滚。
        self.select(pos, pos);
    }

    /// 点了 `(x, y)` 处的改动标记：打开那一处的卡片，贴在点的那一行旁边。
    ///
    /// 没点在标记上时返回 `false`。
    pub(in crate::app) fn click_change_mark(&mut self, x: f32, y: f32) -> bool {
        if self.change_mark_hit(x, y).is_none() {
            return false;
        }
        // 标记不是最新的就先重算，行号才准。
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

    /// 打开第 `hunk` 处改动的卡片（上一处、下一处），并让它露在屏幕上。
    ///
    /// 卡片贴在这一块下面；这一块长到最后一行不在屏幕上时贴在第一行下面。
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
            let visible = self.visible_lines() as f32;
            let doc = self
                .editor
                .active()
                .expect("开头确认过有当前文档，滚动不会关掉它");
            let last = doc.row_of(h.new.end - 1) as f32;
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

    /// 点了卡片上的 `target`。
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

    /// 回滚当前文档的第 `hunk` 处改动：这几行换回 `HEAD` 里的样子。
    ///
    /// 是一次普通的修改，⌘Z 能撤销。
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

    /// 在当前文档里跳到下一处（`forward`）或上一处改动的第一行，返回当前文档是否有改动标记。
    ///
    /// 没有了就在状态栏说一声；返回 `false` 时什么都没做。
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

    /// 让第 `line` 行露在编辑区里（不动光标）。
    fn reveal_line(&mut self, line: usize) {
        let visible = self.visible_lines() as f32;
        if let Some(doc) = self.editor.active_mut() {
            let row = doc.row_of(line) as f32;
            if row < doc.scroll_line + 1.0 || row > doc.scroll_line + visible - 6.0 {
                doc.scroll_line = (row - 3.0).max(0.0);
            }
        }
        self.clamp_scroll();
    }
}

/// 一处改动在概览标尺上占的显示行（行）：当前文档中该改动的各行；删除处是那道分界，文件末尾
/// 删掉的在最后一行底下。
///
/// 绘制和点击共用。
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

/// 文档里这一块现在的几行（连同换行符），用来和原来的内容逐词比较。
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

/// 把 `doc` 的第 `lines` 行（含换行符）换成 `inserted`，作为一步可以撤销的编辑；光标不动。
///
/// # Errors
///
/// 文档不能改（只读、大文件）时返回原因。
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
/// 换行符认 `\n`、`\r\n` 和单独的 `\r`，和 `diff::line_hunks` 分行一致。按 `\n` 跳着找，
/// 一段里有 `\r` 时才细分：卡片每次打开都要从基准开头数到这一块，几万行的文件逐字节数要
/// 一两毫秒。
pub(crate) fn lines_of(text: &str, lines: std::ops::Range<usize>) -> String {
    let mut out = String::new();
    let mut line = 0;
    let mut rest = text;
    while line < lines.end && !rest.is_empty() {
        let segment = rest.find('\n').map_or(rest, |i| &rest[..=i]);
        rest = &rest[segment.len()..];
        // 这一段里单独的 `\r` 也是换行（`\r\n` 的 `\r` 不算）。
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
