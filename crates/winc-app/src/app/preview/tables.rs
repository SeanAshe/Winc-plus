//! 表格预览：xlsx 工作簿和切换到预览的 csv、tsv 在后台线程读成表格，读好后显示。
//!
//! 与图片相同，只读当前标签；不在屏幕上的标签释放读好的表格（见 `release_hidden_pictures`）。

use std::collections::HashSet;
use std::sync::mpsc;

use winc_editor::DocumentId;
use winc_preview::table::{self, Area, Table};
use winc_render::Rect;
use winit::keyboard::{Key, NamedKey};

use crate::app::{App, Drag, Focus, UserEvent};

/// 后台读好的表格：文档、读取依据的文档版本号、读取结果。
type Done = (DocumentId, u64, Result<Table, String>);

/// 表格的后台任务状态。
pub(crate) struct TableJobs {
    sender: mpsc::Sender<Done>,
    receiver: mpsc::Receiver<Done>,
    /// 已提交后台、尚未返回的任务，键为（文档，文档版本号）。
    pending: HashSet<(DocumentId, u64)>,
}

impl Default for TableJobs {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            pending: HashSet::new(),
        }
    }
}

impl App {
    /// 当前标签的表格需要读取时，提交后台任务；每次绘制前调用。
    ///
    /// 工作簿按路径读磁盘上的文件；csv、tsv 读文档当前的文字，包含未保存的修改。同一文档同时
    /// 只读一版：打字快过读取时，上一版回来以后直接读最新的，中间的版本跳过。
    pub(crate) fn advance_tables(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(revision) = doc.table_to_load() else {
            return;
        };
        let key = (doc.id, revision);
        if self.tables.pending.iter().any(|&(id, _)| id == key.0) {
            return;
        }
        let path = doc.path.clone();
        let job: Box<dyn FnOnce() -> Result<Table, String> + Send> = if doc.unsupported.is_some() {
            let Some(path) = path else {
                return;
            };
            Box::new(move || table::load_workbook(&path))
        } else {
            let Some(rope) = doc.text().rope() else {
                return;
            };
            let text = rope.to_string();
            Box::new(move || {
                let sheet = table::read_delimited(&text, path.as_deref());
                Ok(Table {
                    sheets: vec![sheet],
                })
            })
        };
        self.tables.pending.insert(key);
        let (sender, proxy) = (self.tables.sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let result = job();
            if sender.send((key.0, key.1, result)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 接收后台读好的表格，放入对应的文档。
    pub(crate) fn poll_tables(&mut self) {
        while let Ok((id, revision, result)) = self.tables.receiver.try_recv() {
            self.tables.pending.remove(&(id, revision));
            let doc = self.editor.documents.iter_mut().find(|d| d.id == id);
            if doc.is_some_and(|d| d.set_table(revision, result)) {
                self.request_redraw();
            }
        }
    }

    /// 当前标签是否显示着读好的表格，且 (`x`, `y`) 在表格上（编辑区，或分栏时的右边）。
    pub(crate) fn table_at(&self, x: f32, y: f32) -> bool {
        let shown = self
            .editor
            .active()
            .and_then(|d| d.table.as_deref())
            .is_some_and(|v| v.table().is_some());
        shown && self.table_layout.area.contains(x, y)
    }

    /// 按滚轮位移滚动当前标签的表格。
    ///
    /// `dx`、`dy` 为滚轮的横向、纵向位移（逻辑像素），(`x`, `y`) 为鼠标位置。在当前格的卡片
    /// 上时滚动卡片中的文字；只取主方向，触控板竖直滑动时不会同时左右移动。
    pub(crate) fn scroll_table(&mut self, dx: f32, dy: f32, (x, y): (f32, f32)) {
        let layout = &self.table_layout;
        let (max_x, max_y) = layout.max_scroll;
        let (in_card, card_max) = (layout.card.contains(x, y), layout.card_max);
        let Some(view) = self.active_table_mut() else {
            return;
        };
        if in_card {
            view.card_scroll = (view.card_scroll - dy).clamp(0.0, card_max);
        } else if dx.abs() > dy.abs() {
            view.scroll.0 = (view.scroll.0 - dx).clamp(0.0, max_x);
        } else {
            view.scroll.1 = (view.scroll.1 - dy).clamp(0.0, max_y);
        }
        self.request_redraw();
    }

    /// 当前标签的表格（工作簿，或只看预览、分栏时的 csv）。
    fn active_table_mut(&mut self) -> Option<&mut winc_editor::TableView> {
        self.editor.active_mut()?.table.as_deref_mut()
    }

    /// 当前标签读好的表格中显示的那张工作表。
    fn active_sheet(&self) -> Option<(std::sync::Arc<Table>, usize)> {
        let view = self.editor.active()?.table.as_deref()?;
        let table = view.table()?.clone();
        (view.sheet < table.sheets.len()).then_some((table, view.sheet))
    }

    /// 编辑区显示的是读好的表格、焦点在编辑区：方向键、复制、全选作用于表格。
    pub(crate) fn table_focused(&self) -> bool {
        self.focus == Focus::Editor
            && self.editor.active().is_some_and(|d| {
                !d.shows_text() && d.table.as_deref().is_some_and(|v| v.table().is_some())
            })
    }

    /// 是否正在表格中拖着选单元格。
    pub(crate) fn table_dragging(&self) -> bool {
        self.drag == Drag::TableCells
    }

    /// 处理在表格上的单击，返回是否点中了表格。
    ///
    /// 底部的工作表标签换工作表；单元格、行号、列名选中该格、整行、整列，按着 `⇧` 时从原来
    /// 的起点扩大到该处；按在单元格上可以拖着选。点在当前格的卡片上不改选区。
    pub(crate) fn click_table(&mut self, x: f32, y: f32) -> bool {
        if self.table_layout.card.contains(x, y) {
            return true;
        }
        if let Some(sheet) = self
            .table_layout
            .sheets
            .iter()
            .position(|r| r.contains(x, y))
        {
            let Some(view) = self.active_table_mut() else {
                return false;
            };
            if view.sheet != sheet {
                view.sheet = sheet;
                view.scroll = (0.0, 0.0);
                view.selection = ((0, 0), (0, 0));
                view.card_scroll = 0.0;
            }
            self.request_redraw();
            return true;
        }
        let Some(view) = self.editor.active().and_then(|d| d.table.as_deref()) else {
            return false;
        };
        let Some(hit) = self.table_layout.hit(x, y, view.scroll, false) else {
            return false;
        };
        let (rows, columns) = self.table_layout.size;
        let (last_row, last_column) = (rows - 1, columns - 1);
        let extend = self.modifiers.shift_key();
        let (anchor, head) = match hit {
            TableHit::Cell(row, column) => ((row, column), (row, column)),
            TableHit::Row(row) => ((row, 0), (row, last_column)),
            TableHit::Column(column) => ((0, column), (last_row, column)),
            TableHit::Corner => ((0, 0), (last_row, last_column)),
        };
        let start = view.selection.0;
        self.set_table_selection(if extend { start } else { anchor }, head, false);
        if matches!(hit, TableHit::Cell(..)) {
            self.drag = Drag::TableCells;
        }
        true
    }

    /// 拖着选单元格时鼠标移到 (`x`, `y`)：当前格跟到鼠标下的那一格（移出表格时取最近的一格）。
    pub(crate) fn drag_table(&mut self, x: f32, y: f32) {
        let Some(view) = self.editor.active().and_then(|d| d.table.as_deref()) else {
            return;
        };
        let anchor = view.selection.0;
        if let Some(TableHit::Cell(row, column)) = self.table_layout.hit(x, y, view.scroll, true) {
            self.set_table_selection(anchor, (row, column), false);
        }
    }

    /// 把选区设为以 `anchor`、`head` 为对角的区域；`reveal` 时滚动到当前格完整可见。
    ///
    /// 只选一格且落在合并区域中时，选中的是区域的左上角（卡片显示该格的文字）。
    fn set_table_selection(&mut self, anchor: (usize, usize), head: (usize, usize), reveal: bool) {
        let merged = self
            .active_sheet()
            .and_then(|(table, sheet)| table.sheets[sheet].merge_at(head.0, head.1));
        let (anchor, head) = match merged {
            Some(m) if anchor == head => ((m.top, m.left), (m.top, m.left)),
            _ => (anchor, head),
        };
        let Some(scroll) = self
            .editor
            .active()
            .and_then(|d| d.table.as_deref())
            .map(|v| v.scroll)
        else {
            return;
        };
        let scroll = if reveal {
            self.table_layout.reveal(head, scroll)
        } else {
            scroll
        };
        let Some(view) = self.active_table_mut() else {
            return;
        };
        if view.selection.1 != head {
            view.card_scroll = 0.0;
            view.card_closed = false;
        }
        view.selection = (anchor, head);
        view.scroll = scroll;
        self.request_redraw();
    }

    /// 处理焦点在表格上时的按键，返回是否处理了：方向键移动当前格（按着 `⇧` 扩大选区，按着
    /// `⌘`（其他平台 `Ctrl`）到表格边缘），Tab、回车向右、向下（按着 `⇧` 向左、向上），翻页键
    /// 按一屏移动，Home、End 到行首、行尾，Esc 只留当前格（只有一格时收起它的卡片）。
    pub(crate) fn table_key(&mut self, key: &Key, cmd: bool) -> bool {
        if !self.table_focused() {
            return false;
        }
        let Some((table, sheet)) = self.active_sheet() else {
            return false;
        };
        let sheet = &table.sheets[sheet];
        let (rows, columns) = (sheet.rows.len(), sheet.columns);
        if rows == 0 || columns == 0 {
            return false;
        }
        let Some(view) = self.editor.active().and_then(|d| d.table.as_deref()) else {
            return false;
        };
        let (anchor, head) = view.selection;
        let shift = self.modifiers.shift_key();
        // 从合并区域中移出时从区域的边上走：向右从最右一列的下一列开始，依此类推。
        let merge = sheet
            .merge_at(head.0, head.1)
            .unwrap_or(Area::spanning(head, head));
        let page = ((self.table_layout.body.h / self.table_layout.row_h.max(1.0)) as usize)
            .saturating_sub(1)
            .max(1);
        let (row, column) = head;
        let up = (merge.top.saturating_sub(1), column);
        let down = ((merge.bottom + 1).min(rows - 1), column);
        let left = (row, merge.left.saturating_sub(1));
        let right = (row, (merge.right + 1).min(columns - 1));
        let next = match key {
            Key::Named(NamedKey::ArrowUp) if cmd => (0, column),
            Key::Named(NamedKey::ArrowDown) if cmd => (rows - 1, column),
            Key::Named(NamedKey::ArrowLeft) if cmd => (row, 0),
            Key::Named(NamedKey::ArrowRight) if cmd => (row, columns - 1),
            Key::Named(NamedKey::ArrowUp) => up,
            Key::Named(NamedKey::ArrowDown) => down,
            Key::Named(NamedKey::ArrowLeft) => left,
            Key::Named(NamedKey::ArrowRight) => right,
            Key::Named(NamedKey::Enter) => {
                if shift {
                    up
                } else {
                    down
                }
            }
            Key::Named(NamedKey::Tab) => {
                if shift {
                    left
                } else {
                    right
                }
            }
            Key::Named(NamedKey::PageUp) => (row.saturating_sub(page), column),
            Key::Named(NamedKey::PageDown) => ((row + page).min(rows - 1), column),
            Key::Named(NamedKey::Home) => (row, 0),
            Key::Named(NamedKey::End) => (row, columns - 1),
            // 选了多格时只留当前格；只有一格时收起它的卡片。
            Key::Named(NamedKey::Escape) => {
                if anchor == head {
                    if let Some(view) = self.active_table_mut() {
                        view.card_closed = true;
                    }
                    self.request_redraw();
                } else {
                    self.set_table_selection(head, head, true);
                }
                return true;
            }
            _ => return false,
        };
        let extends = shift && !matches!(key, Key::Named(NamedKey::Tab | NamedKey::Enter));
        self.set_table_selection(if extends { anchor } else { next }, next, true);
        true
    }

    /// 选中整张表。
    pub(crate) fn table_select_all(&mut self) {
        let (rows, columns) = self
            .active_sheet()
            .map(|(table, sheet)| (table.sheets[sheet].rows.len(), table.sheets[sheet].columns))
            .unwrap_or_default();
        if rows > 0 && columns > 0 {
            self.set_table_selection((0, 0), (rows - 1, columns - 1), false);
        }
    }

    /// 复制选中的单元格：一格时是它的文字，多格时一行一行、格与格之间用制表符分开（粘贴到
    /// 表格软件中仍是这些格）。
    pub(crate) fn copy_table(&mut self) {
        let Some((table, sheet)) = self.active_sheet() else {
            return;
        };
        let Some(view) = self.editor.active().and_then(|d| d.table.as_deref()) else {
            return;
        };
        let sheet = &table.sheets[sheet];
        if sheet.rows.is_empty() || sheet.columns == 0 {
            return;
        }
        let area = sheet.cover_merges(Area::spanning(view.selection.0, view.selection.1));
        let text = sheet.copy_text(area);
        self.set_clipboard(text, false);
        self.flash(winc_i18n::t!("editor-selection-copied"));
    }
}

/// 绘制表格时留给输入处理的几何。
///
/// 每帧由 [`draw`] 写入，鼠标和键盘处理按它判断点在哪一格、滚动到哪里。
#[derive(Debug, Default)]
pub(crate) struct TableLayout {
    /// 底部各张工作表标签的矩形，按工作表的顺序。
    pub(crate) sheets: Vec<Rect>,
    /// 横向、纵向的最大滚动距离（逻辑像素）。
    pub(crate) max_scroll: (f32, f32),
    /// 表格所在的区域：编辑区，或分栏时的右边。
    pub(crate) area: Rect,
    /// 当前格的卡片；没有显示时宽高为 0。
    pub(crate) card: Rect,
    /// 当前格卡片的最大滚动距离（逻辑像素）。
    pub(crate) card_max: f32,
    /// 单元格所在的区域（不含列名一行和行号一列）。
    pub(crate) body: Rect,
    /// 列名一行。
    pub(crate) header: Rect,
    /// 行号一列。
    pub(crate) numbers: Rect,
    /// 行高（逻辑像素）。
    pub(crate) row_h: f32,
    /// 各列左边在表格内容中的横坐标，最后多一项是最右边：第 `i` 列占 `edges[i]..edges[i + 1]`。
    pub(crate) edges: Vec<f32>,
    /// 绘制时的行数、列数。
    pub(crate) size: (usize, usize),
    /// 列宽缓存：（文档，表格的 `Arc` 指针地址，工作表序号）以及各列的宽度。
    pub(crate) widths: Option<((DocumentId, usize, usize), Vec<f32>)>,
    /// 卡片文字的换行结果缓存：[`CardKey`] 以及换好的各行。
    pub(crate) card_lines: Option<(CardKey, Vec<String>)>,
}

/// 表格上一点对应的位置，见 [`TableLayout::hit`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableHit {
    /// 单元格（行号，列号）。
    Cell(usize, usize),
    /// 行号一列中的一行。
    Row(usize),
    /// 列名一行中的一列。
    Column(usize),
    /// 左上角：选中整张表。
    Corner,
}

/// 卡片换行结果的缓存键：文档、表格的 `Arc` 指针地址、工作表序号、单元格（行，列）、
/// 文字宽度和字号（`f32` 的位）。
pub(crate) type CardKey = (DocumentId, usize, usize, (usize, usize), u32, u32);

impl TableLayout {
    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处的单元格、行号或列名。
    ///
    /// `scroll` 是表格的滚动位置（逻辑像素）；点不在表格上时返回 `None`，`clamp` 时表格之外的
    /// 点算作最近的一格（拖选时鼠标移出表格）。
    pub(crate) fn hit(&self, x: f32, y: f32, scroll: (f32, f32), clamp: bool) -> Option<TableHit> {
        let (rows, columns) = self.size;
        if rows == 0 || columns == 0 || self.row_h <= 0.0 {
            return None;
        }
        let in_header = self.header.contains(x, y);
        let in_numbers = self.numbers.contains(x, y);
        let corner = Rect::new(self.numbers.x, self.header.y, self.numbers.w, self.header.h);
        if !clamp && corner.contains(x, y) {
            return Some(TableHit::Corner);
        }
        if !clamp && !in_header && !in_numbers && !self.body.contains(x, y) {
            return None;
        }
        let cx = (x - self.body.x + scroll.0).max(0.0);
        let column = self
            .edges
            .partition_point(|&edge| edge <= cx)
            .saturating_sub(1)
            .min(columns - 1);
        let row = (((y - self.body.y + scroll.1) / self.row_h).max(0.0) as usize).min(rows - 1);
        Some(match () {
            () if clamp => TableHit::Cell(row, column),
            () if in_header => TableHit::Column(column),
            () if in_numbers => TableHit::Row(row),
            () => TableHit::Cell(row, column),
        })
    }

    /// 让第 `row` 行第 `column` 列完整显示时的滚动位置（从 `scroll` 出发，移动得最少）。
    pub(crate) fn reveal(&self, (row, column): (usize, usize), scroll: (f32, f32)) -> (f32, f32) {
        let (Some(&left), Some(&right)) = (self.edges.get(column), self.edges.get(column + 1))
        else {
            return scroll;
        };
        let top = row as f32 * self.row_h;
        let bottom = top + self.row_h;
        let fit = |start: f32, end: f32, at: f32, span: f32| {
            if start < at {
                start
            } else if end > at + span {
                (end - span).min(start)
            } else {
                at
            }
        };
        (
            fit(left, right, scroll.0, self.body.w).clamp(0.0, self.max_scroll.0),
            fit(top, bottom, scroll.1, self.body.h).clamp(0.0, self.max_scroll.1),
        )
    }
}

#[cfg(test)]
mod tests;
