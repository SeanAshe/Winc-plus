//! 拖动标签调整顺序：被拖动的标签跟随鼠标横向移动，其边缘越过相邻标签的中线时与该标签交换位置，
//! 被换位的标签滑向新位置。
//!
//! 按在多选的标签上拖动时，选中的各个标签先靠拢到它两边，再作为一块
//! 一起移动。
//!
//! 标签栏放不下全部标签时，拖到两端会自动滚动。

use std::time::Instant;

use super::*;
use crate::app::workspace::files::TabGhost;

/// 判定为拖动的横向移动距离（逻辑像素，经验值）：按下后移动不足此距离时仍视为单击。
const THRESHOLD: f32 = 4.0;
/// 触发自动滚动的区域宽度（逻辑像素，经验值）：鼠标距标签栏左右两端不足此距离时开始滚动。
const EDGE: f32 = 32.0;
/// 鼠标贴着标签栏边缘（或在其外）时每帧的滚动量（逻辑像素，经验值）。
///
/// 滚动量随鼠标向 [`EDGE`] 区域内侧移动而线性减小，到区域内侧边界时为 0。
const EDGE_SPEED: f32 = 10.0;

/// 被按住的标签的拖动状态。
#[derive(Debug, Clone, Copy)]
pub struct TabDrag {
    /// 被按住的标签对应的文档。
    id: DocumentId,
    /// 按下点到标签左边的距离（逻辑像素）。
    grab: f32,
    /// 按下时鼠标的横坐标（逻辑像素）。
    start_x: f32,
    /// 横向移动是否已超过 [`THRESHOLD`]，即是否已算作拖动。
    moving: bool,
    /// 按在多选的标签上：拖动时选中的各个标签一起移动；没有拖动就松开时只选这一个。
    grouped: bool,
}

/// 离开原位置的标签：从距新位置 `from` 处滑向新位置。
#[derive(Debug, Clone, Copy)]
pub struct TabSlide {
    /// 滑动的标签对应的文档。
    id: DocumentId,
    /// 起点相对新位置的横向偏移（逻辑像素），负值表示起点在新位置左侧。
    from: f32,
    pub(in crate::app) clock: Clock,
}

/// 关掉第 `index` 个标签以后，右边每个标签（按关掉前的下标）要从多远滑回来：它关掉前、关掉后
/// 在全部标签里的左边之差。
///
/// 关掉以后的宽度去掉这一个，`lock` 给出用鼠标关时锁定的下标和宽度（见 `TabBar::width_lock`），
/// 该下标的宽度改用锁定值。不动的不列出。
fn close_shifts(widths: &[f32], index: usize, lock: Option<(usize, f32)>) -> Vec<(usize, f32)> {
    let slot = |widths: &[f32], i: usize| -> f32 { widths[..i].iter().map(|w| w + TAB_GAP).sum() };
    let mut after = widths.to_vec();
    after.remove(index);
    if let Some((locked, width)) = lock
        && let Some(w) = after.get_mut(locked)
    {
        *w = width;
    }
    (index + 1..widths.len())
        .map(|old| (old, slot(widths, old) - slot(&after, old - 1)))
        .filter(|(_, from)| from.abs() > 0.5)
        .collect()
}

impl App {
    /// 处理在第 `index` 个标签上的按下（不含关闭按钮），`x` 为鼠标的横坐标。
    ///
    /// 切换到该标签并记录按住状态；横向移动超过 [`THRESHOLD`] 后才算拖动。按着 `⌘` 时把它加入
    /// 或移出选区，按着 `⇧` 时选到它（见 `App::toggle_tab_selected`、`App::select_tab_range`），
    /// 这两种不拖动。按在多选的标签上时保留选区，拖动时一起移动，没有拖动就松开时再只选它。
    pub(in crate::app) fn press_tab(&mut self, index: usize, x: f32) {
        if self.command_held() {
            self.toggle_tab_selected(index);
            return;
        }
        if self.modifiers.shift_key() {
            self.select_tab_range(index);
            return;
        }
        let Some(id) = self.editor.documents.get(index).map(|d| d.id) else {
            return;
        };
        let grouped = self.tabs_multi_selected() && self.tab_selection().contains(&id);
        if !grouped {
            self.tabs.marked.clear();
            self.tabs.anchor = Some(id);
        }
        self.activate_tab(index);
        let Some(hit) = self.hits.tabs.iter().find(|h| h.index == index) else {
            return;
        };
        // 清除该标签上次松开后尚未结束的滑动，使本次拖动不再叠加旧的滑动偏移。
        self.tabs.slides.retain(|s| s.id != id);
        self.tabs.drag = Some(TabDrag {
            id,
            grab: x - hit.tab.x,
            start_x: x,
            moving: false,
            grouped,
        });
    }

    /// 处理按住标签时的鼠标移动，`x` 为鼠标的横坐标。
    ///
    /// 横向移动超过 [`THRESHOLD`] 后进入拖动状态，并按鼠标位置调整标签顺序。
    pub(in crate::app) fn drag_tab(&mut self, x: f32) {
        let Some(drag) = &mut self.tabs.drag else {
            return;
        };
        if !drag.moving && (x - drag.start_x).abs() < THRESHOLD {
            return;
        }
        let gather = (!drag.moving && drag.grouped).then_some(drag.id);
        drag.moving = true;
        if let Some(id) = gather {
            self.gather_tab_group(id);
        }
        self.reorder_dragged_tab();
        self.request_redraw();
    }

    /// 处理鼠标松开：拖动中的标签从松开处滑回它在标签栏中的位置。
    ///
    /// 没有形成拖动（仍是单击）时不移动；按在多选的标签上时只选它。
    pub(in crate::app) fn release_tab(&mut self) {
        let Some(drag) = self.tabs.drag.take() else {
            return;
        };
        if !drag.moving {
            if drag.grouped {
                self.tabs.marked.clear();
                self.tabs.anchor = Some(drag.id);
                self.request_redraw();
            }
            return;
        }
        if let Some(index) = self.tab_index(drag.id) {
            let left = self.dragged_tab_left(index, drag.grab);
            let from = left - self.tab_slot(index);
            self.slide_tab(drag.id, from);
        }
        self.request_redraw();
    }

    /// 返回正在拖动的标签及其左边的横坐标；没有拖动时返回 `None`。
    ///
    /// 横坐标单位为逻辑像素，从第一个标签的左边起算。
    pub(crate) fn dragged_tab(&self) -> Option<(DocumentId, f32)> {
        let drag = self.tabs.drag.filter(|d| d.moving)?;
        let index = self.tab_index(drag.id)?;
        Some((drag.id, self.dragged_tab_left(index, drag.grab)))
    }

    /// 返回标签 `id` 当前相对目标位置的横向偏移（逻辑像素）；没有在滑动时为 0。
    pub(crate) fn tab_slide(&self, id: DocumentId) -> f32 {
        self.tabs
            .slides
            .iter()
            .find(|s| s.id == id)
            .map_or(0.0, |s| s.clock.between(s.from, 0.0))
    }

    /// 拖动到标签栏两端时自动滚动标签栏，返回下一次需要检查的时刻。
    ///
    /// 没有在拖动，或无需继续滚动时返回 `None`。
    pub(in crate::app) fn tab_drag_tick(&mut self, now: Instant) -> Option<Instant> {
        self.tabs.drag.filter(|d| d.moving)?;
        let view = self.tabs.view;
        let x = self.mouse.0;
        let push = if x < view.x + EDGE {
            x - (view.x + EDGE)
        } else if x > view.right() - EDGE {
            x - (view.right() - EDGE)
        } else {
            0.0
        };
        let max = self.tabs.max_scroll();
        let scroll =
            (self.tabs.scroll + push.clamp(-EDGE, EDGE) / EDGE * EDGE_SPEED).clamp(0.0, max);
        if scroll == self.tabs.scroll {
            return None;
        }
        self.tabs.scroll = scroll;
        // 标签栏滚动后，鼠标位置对应的内容位置改变，拖动的标签可能越过其他标签，需要重新排序。
        self.reorder_dragged_tab();
        self.request_redraw();
        Some(now + self.frame_interval)
    }

    fn tab_index(&self, id: DocumentId) -> Option<usize> {
        self.editor.documents.iter().position(|d| d.id == id)
    }

    /// 返回第 `index` 个标签左边在内容中的横坐标（逻辑像素）。
    fn tab_slot(&self, index: usize) -> f32 {
        self.tabs.widths[..index].iter().map(|w| w + TAB_GAP).sum()
    }

    /// 返回拖动中的第 `index` 个标签左边在内容中的横坐标（逻辑像素）。
    ///
    /// 横坐标随鼠标移动，但限制在全部标签占据的范围和标签栏的可见范围之内。
    fn dragged_tab_left(&self, index: usize, grab: f32) -> f32 {
        let tabs = &self.tabs;
        let width = tabs.widths.get(index).copied().unwrap_or(0.0);
        let left = self.mouse.0 - grab - tabs.view.x + tabs.scroll;
        let low = tabs.scroll;
        let high = (tabs.content - TAB_GAP - width).min(tabs.scroll + tabs.view.w - width);
        left.min(high).max(low)
    }

    /// 和拖动的标签一起移动的各个标签（含它自己），按标签顺序：按在多选的标签上时是选中的各个，
    /// 否则只有它。
    fn dragged_block(&self, drag: TabDrag) -> Vec<DocumentId> {
        if drag.grouped {
            let selection = self.tab_selection();
            if selection.contains(&drag.id) {
                return selection;
            }
        }
        vec![drag.id]
    }

    /// 开始拖动多选的标签：选中的各个标签靠拢到被拖的标签 `id` 两边（原来在它左边的靠在左边，
    /// 右边的靠在右边，次序不变），挪了位置的从原处滑过去。
    fn gather_tab_group(&mut self, id: DocumentId) {
        if self.tabs.widths.len() != self.editor.documents.len() {
            return;
        }
        let group = self.tab_selection();
        let before: Vec<(DocumentId, f32)> = (0..self.editor.documents.len())
            .map(|i| (self.editor.documents[i].id, self.tab_slot(i)))
            .collect();
        let mut order: Vec<DocumentId> = before
            .iter()
            .map(|&(d, _)| d)
            .filter(|d| *d == id || !group.contains(d))
            .collect();
        let Some(at) = order.iter().position(|d| *d == id) else {
            return;
        };
        order.splice(at..=at, group.iter().copied());

        for (target, want) in order.iter().enumerate() {
            let Some(current) = self.tab_index(*want) else {
                continue;
            };
            if current != target {
                self.editor.move_document(current, target);
                let width = self.tabs.widths.remove(current);
                self.tabs.widths.insert(target, width);
            }
        }
        for (other, slot) in before {
            if other == id {
                continue;
            }
            if let Some(index) = self.tab_index(other) {
                let from = slot - self.tab_slot(index);
                if from.abs() > 0.5 {
                    self.slide_tab(other, from);
                }
            }
        }
    }

    /// 按拖动标签的位置调整标签顺序。
    ///
    /// 一起拖动的各个标签（见 [`App::dragged_block`]）算作一块：向右拖动时，这一块的右边越过
    /// 右侧相邻标签的中线；向左拖动时，左边越过左侧相邻标签的中线。越过后该标签换到这一块的
    /// 另一边，一次调用可以连续换多个。换过之后需要反向越过新相邻标签的中线才会换回，因此在
    /// 中线附近不会来回跳动。
    fn reorder_dragged_tab(&mut self) {
        let Some(drag) = self.tabs.drag else {
            return;
        };
        let Some(index) = self.tab_index(drag.id) else {
            // 拖动过程中该标签已被关闭。
            self.tabs.drag = None;
            return;
        };
        // `widths` 尚未按当前标签重新生成（刚打开或关闭了其他标签），待下次绘制之后再计算。
        if self.tabs.widths.len() != self.editor.documents.len() {
            return;
        }

        let block = self.dragged_block(drag);
        let indices: Vec<usize> = block.iter().filter_map(|&d| self.tab_index(d)).collect();
        let (Some(&lo), Some(&hi)) = (indices.iter().min(), indices.iter().max()) else {
            return;
        };
        // 一块中的标签必须在次序上连续（靠拢之后才满足），否则只拖动被按住的标签自己。
        let (mut lo, mut hi) = if hi - lo + 1 == indices.len() {
            (lo, hi)
        } else {
            (index, index)
        };
        let span = |tabs: &TabBar, from: usize, to: usize| -> f32 {
            tabs.widths[from..to].iter().map(|w| w + TAB_GAP).sum()
        };
        let left = self.dragged_tab_left(index, drag.grab);
        let block_left = left - span(&self.tabs, lo, index);
        let block_right = left + span(&self.tabs, index, hi + 1) - TAB_GAP;
        let shift = span(&self.tabs, lo, hi + 1);
        loop {
            let (from, to, offset) = if lo > 0 && block_left < self.tab_center(lo - 1) {
                (lo - 1, hi, -shift)
            } else if hi + 1 < self.tabs.widths.len() && block_right > self.tab_center(hi + 1) {
                (hi + 1, lo, shift)
            } else {
                break;
            };
            // 被换位的标签从这一块原来的另一边滑向新位置，起点偏移为这一块的宽度。
            let other = self.editor.documents[from].id;
            self.editor.move_document(from, to);
            let width = self.tabs.widths.remove(from);
            self.tabs.widths.insert(to, width);
            self.slide_tab(other, offset);
            if from < lo {
                (lo, hi) = (lo - 1, hi - 1);
            } else {
                (lo, hi) = (lo + 1, hi + 1);
            }
        }
    }

    fn tab_center(&self, index: usize) -> f32 {
        self.tab_slot(index) + self.tabs.widths[index] / 2.0
    }

    /// 即将关闭第 `index` 个标签：标签内容先在原位淡出（`TabGhost`），之后宽度收起，右侧的标签
    /// 从原位置滑到关闭后的位置（与拖动时让位使用同一种滑动）。
    ///
    /// 在真正从文档列表里删掉之前调用。
    ///
    /// 新位置按关掉以后的宽度计算：用鼠标关闭时，紧邻的标签暂用被关标签的宽度（`width_lock`），
    /// 该标签不动，再往右的标签照常滑过来。没有窗口、动态效果关闭时不做动画。
    pub(in crate::app) fn animate_tab_close(&mut self, index: usize) {
        if let Some(fade) = self.clock(EXIT_FADE) {
            self.animate_tab_close_with(index, fade);
        }
    }

    /// 同 [`App::animate_tab_close`]，内容淡出按给定的时钟 `fade` 计时，宽度收起与右侧标签的滑动
    /// 在 `fade` 结束后开始。测试中使用固定时刻的时钟。
    pub(in crate::app) fn animate_tab_close_with(&mut self, index: usize, fade: Clock) {
        let clock = fade.then(TAB_SLIDE);
        let widths = self.tabs.widths.clone();
        if index >= widths.len() || widths.len() != self.editor.documents.len() {
            return;
        }
        let doc = &self.editor.documents[index];
        let slot =
            |widths: &[f32], i: usize| -> f32 { widths[..i].iter().map(|w| w + TAB_GAP).sum() };
        let name = doc.name().into_owned();
        let icon = doc
            .diff
            .as_deref()
            .and_then(|v| v.new.path.as_deref().or(v.old.path.as_deref()))
            .and_then(|p| p.file_name())
            .map_or_else(
                || name.split(" · ").next().unwrap_or(&name).to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
        self.tabs.closing.push(TabGhost {
            left: slot(&widths, index),
            width: widths[index],
            name,
            icon,
            log: doc.log.is_some(),
            active: self.editor.active == Some(index),
            fade,
            clock,
        });
        let moved: Vec<(DocumentId, f32)> = close_shifts(&widths, index, self.tabs.width_lock)
            .into_iter()
            .map(|(old, from)| (self.editor.documents[old].id, from))
            .collect();
        for (id, from) in moved {
            self.tabs.slides.retain(|s| s.id != id);
            self.tabs.slides.push(TabSlide {
                id,
                from: from + self.tab_slide(id),
                clock,
            });
        }
    }

    /// 绘制标签栏前调用：和上一帧比，正好多出一个标签（新建、打开文件、打开差异、日志……）时，
    /// 它从零宽过渡到原来的宽度并淡入，右边的标签和新建按钮跟着让开（`tab_grow`）。
    ///
    /// 一次多出多个标签（启动时恢复现场、切换目录）、第一次画、动态效果关闭时不做，直接到位。
    pub(crate) fn note_opened_tabs(&mut self) {
        let ids: Vec<DocumentId> = self.editor.documents.iter().map(|d| d.id).collect();
        if let Some(known) = &self.tabs.known {
            let new: Vec<DocumentId> = ids
                .iter()
                .filter(|id| !known.contains(id))
                .copied()
                .collect();
            if let [id] = new[..]
                && let Some(clock) = self.clock(TAB_SLIDE)
            {
                self.tabs.opening.retain(|(o, _)| *o != id);
                self.tabs.opening.push((id, clock));
            }
        }
        self.tabs.opening.retain(|(id, _)| ids.contains(id));
        self.tabs.known = Some(ids);
    }

    /// 标签 `id` 展开的程度（0 到 1）：刚打开时从 0 过渡到 1，其余的标签一直是 1。
    pub(crate) fn tab_grow(&self, id: DocumentId) -> f32 {
        self.tabs
            .opening
            .iter()
            .find(|(o, _)| *o == id)
            .map_or(1.0, |(_, c)| c.progress())
    }

    /// 让标签 `id` 从距新位置 `from`（逻辑像素）处滑向新位置。
    ///
    /// 已有未结束的滑动时，从该滑动当前的偏移处继续，位置不会突变。
    fn slide_tab(&mut self, id: DocumentId, from: f32) {
        let from = from + self.tab_slide(id);
        self.tabs.slides.retain(|s| s.id != id);
        if let Some(clock) = self.clock(TAB_SLIDE) {
            self.tabs.slides.push(TabSlide { id, from, clock });
        }
    }
}

#[cfg(test)]
mod tests;
