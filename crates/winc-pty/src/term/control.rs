//! 光标与状态：移动光标、制表位、保存和恢复光标、备用屏幕、修改尺寸、复位。

use super::*;

impl Emulator {
    /// 返回光标上下移动的行号边界，光标在滚动区域内时以区域的上下边为界。
    fn vertical_bounds(&self) -> (usize, usize) {
        let row = self.cursor().row;
        if (self.top..self.bottom).contains(&row) {
            (self.top, self.bottom - 1)
        } else {
            (0, self.rows - 1)
        }
    }

    /// 把光标向上移动 `n` 行，不越过滚动区域顶边。
    pub(super) fn move_up(&mut self, n: usize) {
        let cursor = self.cursor();
        let (top, _) = self.vertical_bounds();
        let row = cursor.row.saturating_sub(n).max(top.min(cursor.row));
        self.set_cursor(row, cursor.col);
    }

    /// 把光标向下移动 `n` 行，不越过滚动区域底边。
    pub(super) fn move_down(&mut self, n: usize) {
        let cursor = self.cursor();
        let (_, bottom) = self.vertical_bounds();
        let row = (cursor.row + n).min(bottom.max(cursor.row));
        self.set_cursor(row, cursor.col);
    }

    /// 把光标移到第 `row` 行第 `col` 格（均从 0 起，CUP）。
    ///
    /// 原点模式下行号从滚动区域顶边算起。
    pub(super) fn goto(&mut self, row: usize, col: usize) {
        if self.origin {
            let row = (self.top + row).min(self.bottom - 1);
            self.set_cursor(row, col);
        } else {
            self.set_cursor(row, col);
        }
    }

    /// 把光标移到第 `row` 行（从 0 起），列不变。
    pub(super) fn goto_row(&mut self, row: usize) {
        let col = self.cursor().col;
        self.goto(row, col);
    }

    /// 把光标移到后面第 `n` 个制表位，没有更多制表位时移到最后一列。
    pub(super) fn tab(&mut self, n: usize) {
        let cursor = self.cursor();
        let mut col = cursor.col;
        // `n` 来自程序的参数（最大 65535），光标到达最后一列后不再有制表位，提前结束循环。
        for _ in 0..n {
            if col + 1 >= self.cols {
                break;
            }
            col = (col + 1..self.cols)
                .find(|&c| self.tabs[c])
                .unwrap_or(self.cols - 1);
        }
        self.set_cursor(cursor.row, col);
    }

    /// 把光标移到前面第 `n` 个制表位，没有更多制表位时移到第 0 列。
    pub(super) fn back_tab(&mut self, n: usize) {
        let cursor = self.cursor();
        let mut col = cursor.col;
        // 同 [`Self::tab`]：到了第 0 列提前停下。
        for _ in 0..n {
            if col == 0 {
                break;
            }
            col = (0..col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
        self.set_cursor(cursor.row, col);
    }

    /// 保存光标、画笔、原点模式、字符集和 SO 状态（DECSC）。
    pub(super) fn save_cursor(&mut self) {
        let saved = Saved {
            pos: self.cursor(),
            pen: self.pen,
            origin: self.origin,
            charsets: self.charsets,
            shift_out: self.shift_out,
        };
        self.screen_mut().saved = Some(saved);
    }

    /// 恢复 DECSC 保存的状态，没有保存过时使用默认值（DECRC）。
    pub(super) fn restore_cursor(&mut self) {
        let saved = self.screen().saved.unwrap_or_default();
        self.pen = saved.pen;
        self.origin = saved.origin;
        self.charsets = saved.charsets;
        self.shift_out = saved.shift_out;
        let pos = Pos {
            row: saved.pos.row.min(self.rows - 1),
            col: saved.pos.col.min(self.cols - 1),
            pending_wrap: saved.pos.pending_wrap,
        };
        self.screen_mut().cursor = pos;
    }

    /// 切换到备用屏或切回主屏。
    ///
    /// `save` 为真时保存光标并在切回时恢复，`clear` 为真时清空备用屏：模式 1047 只清空，
    /// 模式 1049 两者都做。
    pub(super) fn set_alternate(&mut self, on: bool, save: bool, clear: bool) {
        if on == self.alt {
            // 已在备用屏上又收到切换请求（程序未切回主屏即退出，随后启动新程序）：
            // `clear` 为真时仍然清空备用屏。
            if on && clear {
                let cols = self.cols;
                for row in &mut self.alternate.rows {
                    row.reset(cols, Color::Default);
                }
                self.damage.all();
            }
            return;
        }
        if on {
            if save {
                self.save_cursor();
            }
            self.alt = true;
            if clear {
                let cols = self.cols;
                for row in &mut self.alternate.rows {
                    row.reset(cols, Color::Default);
                }
            }
            self.alternate.cursor = self.primary.cursor;
        } else {
            self.alt = false;
            if save {
                self.restore_cursor();
            }
        }
        self.damage.all();
    }

    /// 修改尺寸：主屏按新宽度重排，备用屏只截断或补齐，光标跟随所在的字符移动。
    pub(super) fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        // 尺寸变化后屏幕和回滚缓冲之间可能移动过行，上一次移入的内容不能再用于比较。
        self.last_kept = None;
        let old_cols = self.cols;
        if cols != old_cols {
            self.clear_prompt(cols);
        }

        // 重排主屏的行。
        let primary = &mut self.primary;
        let cursor = primary.cursor;
        let used = primary
            .rows
            .iter()
            .rposition(|r| r.content_len() > 0 || r.wrapped)
            .map_or(0, |i| i + 1)
            .max(cursor.row + 1);
        let mut screen = std::mem::take(&mut primary.rows);
        screen.truncate(used);
        // 等待折行时把光标算在最后一个字符之后的一格，重排后随该字符移动。
        let at = (cursor.row, cursor.col + usize::from(cursor.pending_wrap));
        let (screen, moved) = if cols == old_cols {
            reflow::resize_rows(&mut self.history, screen, at, cols, rows)
        } else {
            let history = self.history.take_all();
            let result = reflow::reflow(
                history,
                screen,
                at,
                cursor.pending_wrap,
                old_cols,
                cols,
                rows,
            );
            self.history.extend(result.history);
            (result.rows, result.cursor)
        };
        self.primary.rows = screen;
        self.blocks.dirty = true;
        let pending_wrap = cursor.pending_wrap && moved.1 >= cols;
        let new = Pos {
            row: moved.0,
            col: moved.1.min(cols - 1),
            pending_wrap,
        };
        self.primary.cursor = new;
        // 保存的光标（DECSC 与进入备用屏时保存的）随内容移动：与当前光标位置相同的直接取新
        // 位置，否则按光标移动的行数平移，切回主屏时提示符仍落在原来的内容上。
        if let Some(saved) = &mut self.primary.saved {
            saved.pos = if saved.pos == cursor {
                new
            } else {
                let row = (saved.pos.row + new.row).saturating_sub(cursor.row);
                Pos {
                    row: row.min(rows - 1),
                    col: saved.pos.col.min(cols - 1),
                    pending_wrap: false,
                }
            };
        }

        // 备用屏不重排：程序收到 SIGWINCH 后自行重画。
        let alternate = &mut self.alternate;
        for row in &mut alternate.rows {
            row.resize(cols);
        }
        alternate
            .rows
            .resize_with(rows, || Row::new(cols, Color::Default));
        alternate.cursor.row = alternate.cursor.row.min(rows - 1);
        alternate.cursor.col = alternate.cursor.col.min(cols - 1);
        alternate.cursor.pending_wrap = false;

        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows;
        let old_tabs = std::mem::take(&mut self.tabs);
        self.reset_tabs();
        for (c, &on) in old_tabs.iter().enumerate().take(cols) {
            self.tabs[c] = on;
        }
        self.damage.clear(rows);
        // 只改行数时回滚缓冲的行与行号都不变，主程序的缓存不必丢弃：拖高面板时上方露出的历史
        // 行无需重新请求，不会先显示空白。
        if cols != old_cols {
            self.damage.reset = true;
        }
        self.damage.all();
    }

    /// 复位全部状态，回滚缓冲保留（RIS）。
    pub(super) fn hard_reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        self.primary = Screen::new(cols, rows);
        self.alternate = Screen::new(cols, rows);
        self.alt = false;
        self.last_kept = None;
        self.scheme_reports = false;
        self.soft_reset();
        self.modes = Modes(Modes::ALT_SCROLL);
        self.shape = CursorShape::Block;
        self.blink = true;
        self.sync = false;
        self.reset_tabs();
        self.damage.all();
    }

    /// 复位模式和画笔，屏幕内容不变（DECSTR）。
    pub(super) fn soft_reset(&mut self) {
        self.pen = Style::default();
        self.top = 0;
        self.bottom = self.rows;
        self.origin = false;
        self.autowrap = true;
        self.insert = false;
        self.cursor_visible = true;
        self.charsets = [false; 2];
        self.shift_out = false;
        self.modes.set(Modes::APP_CURSOR | Modes::APP_KEYPAD, false);
        self.screen_mut().saved = None;
    }

    /// 改宽度前擦掉从提示符第一行往下的内容，光标所在行和总行数不变。
    ///
    /// 仅在 shell 停在会自行重画的提示符上时执行（[`Blocks::redraws_prompt`]）。不擦除时重排会把
    /// 用点填满的提示符第一行折成两行，shell 收到 SIGWINCH 后无法退回顶部，旧提示符留在屏幕上。
    fn clear_prompt(&mut self, cols: usize) {
        if self.alt || !self.blocks.at_prompt || !self.blocks.redraws_prompt {
            return;
        }
        let (new_cols, cols) = (cols, self.cols);
        let primary = &mut self.primary;
        let cursor = primary.cursor.row;
        let Some(start) = primary.rows[..=cursor]
            .iter()
            .rposition(|r| r.marks.flags & Marks::PROMPT != 0)
        else {
            return;
        };
        // 标记保留在提示符所在行：重画时不再发送 `A`，命令块仍依赖该标记。
        let marks = primary.rows[start].marks;
        for row in &mut primary.rows[start..] {
            row.reset(cols, Color::Default);
        }
        primary.rows[start].marks = marks;
        // 上一行（zsh 的 `PROMPT_SP` 写出的半行标记）不再与提示符相连，重排时分别处理。
        if let Some(above) = start.checked_sub(1) {
            primary.rows[above].wrapped = false;
        }
        primary.cursor.pending_wrap = false;
        primary.cursor.col = primary.cursor.col.min(new_cols.saturating_sub(1));
        self.blocks.dirty = true;
    }
}
