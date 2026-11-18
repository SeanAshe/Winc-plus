//! 设置中「快捷键」页的几何。

use std::sync::Arc;

use super::*;

/// 「快捷键」页固定的部分：标题行右侧的搜索框，以及页面顶部的「键位方案」一行。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KeysHeader {
    /// 「键位方案」一行，样式与设置页中带说明的一行相同。
    pub keymap: Rect,
    /// 「键位方案」一行右端点击后弹出菜单的值。
    pub value: Rect,
    /// 标题行右侧的搜索框。
    pub search: Rect,
    /// 搜索框右端的「录制按键」按钮。
    pub record: Rect,
}

/// 「快捷键」页中展开的行：正在修改的一行 `open`、同时收起的一行 `closing`，`t` 是展开、收起的
/// 进度（0 到 1）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KeyRows {
    /// 正在修改快捷键的命令。
    pub open: Option<keymap::Action>,
    /// 正在收起的一行的命令。
    pub closing: Option<keymap::Action>,
    /// 过渡的进度，1 表示已结束。
    pub t: f32,
}

impl KeyRows {
    /// 返回命令 `action` 一行的高度：展开的行由 [`KEYS_ROW`] 过渡到 [`KEYS_EDIT_H`]，收起的行相反。
    pub fn height(self, action: keymap::Action) -> f32 {
        let extra = KEYS_EDIT_H - KEYS_ROW;
        if self.open == Some(action) {
            KEYS_ROW + extra * self.t
        } else if self.closing == Some(action) {
            KEYS_ROW + extra * (1.0 - self.t)
        } else {
            KEYS_ROW
        }
    }
}

/// 「快捷键」页的几何：标题行中的搜索框（右端是「录制按键」）、页面顶部固定的「键位方案」，
/// 下方按分组列出命令和键位，可以滚动。
///
/// 页面布局相同：组上方是小标题，组内每行一个命令，行与行之间有一条细线。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeysLayout {
    /// 顶部固定的部分。
    pub header: KeysHeader,
    /// 「键位方案」下方可以滚动的区域。
    pub list: Rect,
    /// 各组的标题：标题所在行的位置和标题文字。
    pub headings: Vec<(Rect, &'static str)>,
    /// 各组的底色区域。
    pub groups: Vec<Rect>,
    /// 组内行与行之间的细线。
    pub separators: Vec<Rect>,
    /// 各命令行的位置，以及对应的键位行在 `source` 中的序号（组，组内的行）。
    pub rows: Vec<(Rect, (usize, usize))>,
    /// 筛选前的全部键位行。
    pub source: Arc<[(keymap::Group, Vec<keymap::Row>)]>,
    /// 内容的总高度（像素），用于计算最大滚动量。
    pub content_h: f32,
}

impl KeysLayout {
    /// 计算「快捷键」页的几何。
    ///
    /// `body` 是设置标题下方的区域，`source` 是全部键位行，其中 `keep` 为真的行列出；各行的高度
    /// 见 `edit`；`scroll` 是列表的滚动量（像素）。
    pub fn compute(
        body: Rect,
        source: Arc<[(keymap::Group, Vec<keymap::Row>)]>,
        keep: impl Fn(&keymap::Row) -> bool,
        edit: KeyRows,
        scroll: f32,
    ) -> Self {
        let x = body.x + SETTINGS_PAD;
        let w = (body.w - SETTINGS_PAD * 2.0).max(0.0);
        let header = Self::header(body);
        let top = header.keymap.bottom() + SETTINGS_GROUP_GAP;
        let list = Rect::new(body.x, top, body.w, (body.bottom() - top).max(0.0));

        let mut layout = KeysLayout {
            header,
            list,
            source: Arc::clone(&source),
            ..KeysLayout::default()
        };
        let mut y = list.y - scroll;
        let mut shown = 0;
        for (g, (group, rows)) in source.iter().enumerate() {
            let kept: Vec<usize> = (0..rows.len()).filter(|&r| keep(&rows[r])).collect();
            if kept.is_empty() {
                continue;
            }
            if shown > 0 {
                y += SETTINGS_GROUP_GAP;
            }
            shown += 1;
            let title = Rect::new(x + SETTINGS_INSET, y, w, SETTINGS_TITLE);
            layout.headings.push((title, group.title()));
            y += SETTINGS_TITLE;
            let block = y;
            for (n, r) in kept.into_iter().enumerate() {
                if n > 0 {
                    let inner = (w - SETTINGS_INSET * 2.0).max(0.0);
                    layout
                        .separators
                        .push(Rect::new(x + SETTINGS_INSET, y, inner, 1.0));
                }
                let h = edit.height(rows[r].action);
                layout.rows.push((Rect::new(x, y, w, h), (g, r)));
                y += h;
            }
            layout.groups.push(Rect::new(x, block, w, y - block));
        }
        layout.content_h = y + scroll - list.y + SETTINGS_BOTTOM;
        layout
    }

    /// 返回 `body` 中顶部固定部分的位置。
    ///
    /// 这部分与列表内容和滚动无关；悬停、点击只需这几个矩形，不必筛选整张键位表。
    pub fn header(body: Rect) -> KeysHeader {
        let x = body.x + SETTINGS_PAD;
        let w = (body.w - SETTINGS_PAD * 2.0).max(0.0);
        let keymap = Rect::new(x, body.y + 4.0, w, SETTINGS_ROW_TALL);
        let value = Rect::new(
            keymap.right() - SETTINGS_INSET + SETTINGS_GLYPH_PAD - KEYS_KEYMAP_W,
            (keymap.y + (keymap.h - SETTINGS_CONTROL_H) / 2.0).round(),
            KEYS_KEYMAP_W,
            SETTINGS_CONTROL_H,
        );
        // 与分类栏顶部的搜索框同高、在同一行上，右端在关闭按钮左边。
        let close = body.right() - 12.0 - BUTTON;
        let search_w = KEYS_SEARCH_W.min((body.w / 2.0).max(0.0));
        let search = Rect::new(
            close - 8.0 - search_w,
            body.y - SETTINGS_HEADER + (SETTINGS_HEADER - SETTINGS_CONTROL_H) / 2.0,
            search_w,
            SETTINGS_CONTROL_H,
        );
        let record = Rect::new(
            search.right() - 4.0 - KEYS_RECORD,
            search.y + (search.h - KEYS_RECORD) / 2.0,
            KEYS_RECORD,
            KEYS_RECORD,
        );
        KeysHeader {
            keymap,
            value,
            search,
            record,
        }
    }

    /// 返回各命令行的位置和对应的键位行。
    pub fn rows(&self) -> impl Iterator<Item = (Rect, &keymap::Row)> {
        (self.rows.iter()).map(|&(rect, (g, r))| (rect, &self.source[g].1[r]))
    }

    /// 返回列表的最大滚动量（像素）。
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.list.h).max(0.0)
    }
}
