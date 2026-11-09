//! 日志标签的键盘操作：在提交列表与改动文件列表之间移动，以及后退、前进。

use super::*;

impl App {
    /// 处理日志标签有焦点时的按键。
    ///
    /// `↑` `↓` 选择提交，`PageUp` `PageDown` 翻页，`Home` `End` 移到首尾，回车打开第一个改动文件
    /// 的差异，`→` 进入详情中的文件列表（见 [`App::log_file_key`]）。
    pub(in crate::app) fn log_key(&mut self, key: &Key) {
        if self.log.file.is_some() {
            self.log_file_key(key);
            return;
        }
        if *key == Key::Named(NamedKey::ArrowRight) {
            if self.log_files_len() > 0 {
                self.log.file = Some(0);
                self.reveal_log_file(0);
                self.request_redraw();
            }
            return;
        }
        let Some(view) = self.editor.active().and_then(|d| d.log.as_deref()) else {
            return;
        };
        let count = view.commits.len();
        if count == 0 {
            return;
        }
        let page = ((self.log_parts().list.h - self.log_list_top()) / self.fonts.line_height)
            .floor()
            .max(2.0) as usize
            - 1;
        let at = view.selected.unwrap_or(0);
        let row = match key {
            Key::Named(NamedKey::ArrowUp) => at.saturating_sub(1),
            Key::Named(NamedKey::ArrowDown) => (at + 1).min(count - 1),
            Key::Named(NamedKey::PageUp) => at.saturating_sub(page),
            Key::Named(NamedKey::PageDown) => (at + page).min(count - 1),
            Key::Named(NamedKey::Home) => 0,
            // `Esc` 退出比较，详情区域恢复显示选中的提交。
            Key::Named(NamedKey::Escape) => {
                self.clear_log_compare();
                return;
            }
            Key::Named(NamedKey::End) => count - 1,
            Key::Named(NamedKey::Enter) => {
                self.open_log_file(0);
                return;
            }
            _ => return,
        };
        self.select_log_row(row);
    }

    /// 处理焦点位于详情文件列表时的按键。
    ///
    /// `↑` `↓` `Home` `End` 选择文件，回车打开该文件的差异，`←`、`Esc` 返回提交列表。
    fn log_file_key(&mut self, key: &Key) {
        let count = self.log_files_len();
        let Some(at) = self.log.file.filter(|_| count > 0) else {
            self.log.file = None;
            return;
        };
        let at = at.min(count - 1);
        let file = match key {
            Key::Named(NamedKey::ArrowUp) => at.saturating_sub(1),
            Key::Named(NamedKey::ArrowDown) => (at + 1).min(count - 1),
            Key::Named(NamedKey::Home) => 0,
            Key::Named(NamedKey::End) => count - 1,
            Key::Named(NamedKey::Enter) => {
                self.open_log_file(at);
                return;
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::Escape) => {
                self.log.file = None;
                self.request_redraw();
                return;
            }
            _ => return,
        };
        self.log.file = Some(file);
        self.reveal_log_file(file);
        self.request_redraw();
    }

    /// 跳转到第 `row` 个提交（沿提交图的连线或点击详情中的父提交时使用）并选中。
    ///
    /// 跳转前选中的提交记入后退历史（见 [`App::log_navigate`]）。目标行不可见时滚动到距列表
    /// 顶部约三分之一处，使目标前后的提交都可见。
    pub(super) fn jump_log_row(&mut self, row: usize) {
        let visible = ((self.log_parts().list.h - self.log_list_top()) / self.fonts.line_height)
            .floor()
            .max(1.0) as f64;
        let Some((from, shown)) = self.editor.active().and_then(|d| {
            let view = d.log.as_deref()?;
            let r = row as f64;
            let shown = r >= d.scroll_line && r + 1.0 <= d.scroll_line + visible;
            Some((view.selected_oid().map(str::to_owned), shown))
        }) else {
            return;
        };
        self.select_log_row(row);
        let to = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref()?.selected_oid().map(str::to_owned));
        if let Some(from) = from
            && to.as_deref() != Some(from.as_str())
        {
            self.log.back.push(from);
            if self.log.back.len() > LOG_HISTORY {
                self.log.back.remove(0);
            }
            self.log.forward.clear();
        }
        if !shown && let Some(doc) = self.editor.active_mut() {
            doc.scroll_line = (row as f64 - (visible / 3.0).floor()).max(0.0);
            self.clamp_scroll();
        }
        self.request_redraw();
    }

    /// 在日志中后退（`forward` 为 `false`）或前进到跳转前、后选中的提交。
    ///
    /// 日志标签不是当前标签，或者没有可用的历史时返回 `false`，由编辑区处理后退、前进。
    /// 历史中的提交已不在列表中时（更换了筛选或刷新后消失）跳过该提交。
    pub(in crate::app) fn log_navigate(&mut self, forward: bool) -> bool {
        if !self.log_active() {
            return false;
        }
        loop {
            let popped = if forward {
                self.log.forward.pop()
            } else {
                self.log.back.pop()
            };
            let Some(oid) = popped else {
                return false;
            };
            let Some((row, current)) = self.editor.active().and_then(|d| {
                let view = d.log.as_deref()?;
                let row = view.commits.iter().position(|c| c.oid == oid)?;
                Some((row, view.selected_oid().map(str::to_owned)))
            }) else {
                continue;
            };
            if let Some(current) = current {
                if forward {
                    self.log.back.push(current);
                } else {
                    self.log.forward.push(current);
                }
            }
            self.clear_log_compare();
            self.focus = Focus::Editor;
            self.select_log_row(row);
            return true;
        }
    }
}
