//! 修改屏幕内容：写入字符、换行、反向换行、滚动、插入行、删除行和擦除。

use super::width::{Cluster, Step};
use super::*;

impl Emulator {
    /// 在光标处写入一个字符，处理组合字符、宽字符、插入模式和延迟换行。
    pub(super) fn write_char(&mut self, c: char) {
        // 最常见的情形：可打印 ASCII、不在行尾、没有插入模式和制图字符集，直接写一格。
        if (' '..='~').contains(&c) && !self.insert && !self.charsets[usize::from(self.shift_out)] {
            let cols = self.cols;
            let pen = self.pen;
            let screen = if self.alt {
                &mut self.alternate
            } else {
                &mut self.primary
            };
            let cursor = &mut screen.cursor;
            if !cursor.pending_wrap && cursor.col + 1 < cols {
                let (row, col) = (cursor.row, cursor.col);
                cursor.col += 1;
                screen.rows[row].write(col, c, &pen, false);
                self.damage.row(row);
                self.last_char = Some(c);
                self.cluster = Cluster::Narrow(c);
                return;
            }
        }

        let c = if self.charsets[usize::from(self.shift_out)] {
            dec_graphics(c)
        } else {
            c
        };
        let width = match self.cluster.next(c) {
            Step::Advance(width) => width,
            Step::Combine => 0,
            Step::Widen => return self.widen_previous(c),
        };

        if width == 0 {
            // 组合字符并入前一个字符；在行首没有前一个字符时丢弃。
            let cursor = self.cursor();
            let col = if cursor.pending_wrap {
                cursor.col
            } else if cursor.col > 0 {
                cursor.col - 1
            } else {
                return;
            };
            self.screen_mut().rows[cursor.row].push_combining(col, c);
            self.damage.row(cursor.row);
            return;
        }
        if width > self.cols {
            return;
        }

        let mut cursor = self.cursor();
        if cursor.pending_wrap && self.autowrap {
            self.screen_mut().rows[cursor.row].wrapped = true;
            self.damage.row(cursor.row);
            self.linefeed();
            cursor = self.cursor();
            cursor.col = 0;
        }
        if cursor.col + width > self.cols {
            // 宽字符放不下最后一格时留一个占位空格，并换到下一行。
            if self.autowrap {
                let cols = self.cols;
                let row = &mut self.screen_mut().rows[cursor.row];
                row.erase(cursor.col, cols, Color::Default);
                row.cells[cursor.col]
                    .attrs
                    .set(crate::protocol::Attrs::FILLER, true);
                row.wrapped = true;
                self.damage.row(cursor.row);
                self.linefeed();
                cursor = self.cursor();
                cursor.col = 0;
            } else {
                cursor.col = self.cols - width;
            }
        }

        let pen = self.pen;
        let cols = self.cols;
        let insert = self.insert;
        let row = &mut self.screen_mut().rows[cursor.row];
        if insert {
            row.insert_blanks(cursor.col, width, pen.bg);
        }
        row.write(cursor.col, c, &pen, width == 2);
        self.damage.row(cursor.row);

        cursor.col += width;
        if cursor.col >= cols {
            // 不自动换行时同样记为停在行尾：下一个字符写回最后一格，组合字符并入该格。
            cursor.col = cols - 1;
            cursor.pending_wrap = true;
        } else {
            cursor.pending_wrap = false;
        }
        self.screen_mut().cursor = cursor;
        self.last_char = Some(c);
    }

    /// 前一个字后面来了 VS16：把它加宽到两格，并入 `selector`。
    ///
    /// 前一个字在最后一格时和宽字符的折行相同：留一个占位空格，移到下一行；不自动换行时放不下，
    /// 丢弃 `selector`，该字仍占一格。
    fn widen_previous(&mut self, selector: char) {
        let cursor = self.cursor();
        let col = if cursor.pending_wrap {
            cursor.col
        } else if cursor.col > 0 {
            cursor.col - 1
        } else {
            return;
        };
        let cols = self.cols;
        if col + 2 <= cols {
            let row = &mut self.screen_mut().rows[cursor.row];
            row.widen(col);
            row.push_combining(col, selector);
            self.damage.row(cursor.row);
            let screen = self.screen_mut();
            screen.cursor.col = (col + 2).min(cols - 1);
            screen.cursor.pending_wrap = col + 2 >= cols;
            return;
        }
        if !self.autowrap || cols < 2 {
            return;
        }
        let base = self.screen_mut().rows[cursor.row].cells[col].ch;
        let row = &mut self.screen_mut().rows[cursor.row];
        row.erase(col, cols, Color::Default);
        row.cells[col]
            .attrs
            .set(crate::protocol::Attrs::FILLER, true);
        row.wrapped = true;
        self.damage.row(cursor.row);
        self.linefeed();
        let pen = self.pen;
        let next = self.cursor().row;
        let row = &mut self.screen_mut().rows[next];
        row.write(0, base, &pen, true);
        row.push_combining(0, selector);
        self.damage.row(next);
        let screen = self.screen_mut();
        screen.cursor.col = 2.min(cols - 1);
        screen.cursor.pending_wrap = cols <= 2;
    }

    /// 把光标下移一行；光标在滚动区域底边时整个区域向上滚动。
    pub(super) fn linefeed(&mut self) {
        let cursor = self.cursor();
        if cursor.row + 1 == self.bottom {
            self.scroll_up(1);
        } else if cursor.row + 1 < self.rows {
            self.screen_mut().cursor.row += 1;
        }
        self.screen_mut().cursor.pending_wrap = false;
    }

    /// 把光标上移一行；光标在滚动区域顶边时整个区域向下滚动（RI）。
    pub(super) fn reverse_index(&mut self) {
        let cursor = self.cursor();
        if cursor.row == self.top {
            self.scroll_down(1);
        } else if cursor.row > 0 {
            self.screen_mut().cursor.row -= 1;
        }
        self.screen_mut().cursor.pending_wrap = false;
    }

    /// 把滚动区域向上滚动 `n` 行。
    ///
    /// 主屏上区域从第一行开始时，滚出屏幕的行写入回滚缓冲。
    pub(super) fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top);
        if n == 0 {
            return;
        }
        let to_history = top == 0 && !self.alt;
        let full = top == 0 && bottom == self.rows;
        let (cols, bg) = (self.cols, self.pen.bg);

        for _ in 0..n {
            let screen = if self.alt {
                &mut self.alternate
            } else {
                &mut self.primary
            };
            let mut row = screen.rows.remove(top);
            if to_history {
                self.history.push_row(&row);
                self.last_kept = None;
            }
            row.reset(cols, bg);
            screen.rows.insert(bottom - 1, row);
        }

        if full {
            self.damage.scroll_up(n);
        } else {
            self.damage.range(top, bottom);
        }
    }

    /// 把滚动区域向下滚动 `n` 行，顶部补空行。
    pub(super) fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top);
        let (cols, bg) = (self.cols, self.pen.bg);
        let rows = &mut self.screen_mut().rows;
        for _ in 0..n {
            let mut row = rows.remove(bottom - 1);
            row.reset(cols, bg);
            rows.insert(top, row);
        }
        self.damage.range(top, bottom);
    }

    /// 在光标所在行插入 `n` 个空行，下方的行下移（IL）。
    ///
    /// 光标不在滚动区域内时不做任何改动。
    pub(super) fn insert_lines(&mut self, n: usize) {
        let cursor = self.cursor();
        if cursor.row < self.top || cursor.row >= self.bottom {
            return;
        }
        let top = self.top;
        self.top = cursor.row;
        self.scroll_down(n);
        self.top = top;
        self.set_cursor(cursor.row, 0);
    }

    /// 删除光标所在行起的 `n` 行，下方的行上移（DL）。
    pub(super) fn delete_lines(&mut self, n: usize) {
        let cursor = self.cursor();
        if cursor.row < self.top || cursor.row >= self.bottom {
            return;
        }
        let bottom = self.bottom;
        let n = n.min(bottom - cursor.row);
        let (cols, bg) = (self.cols, self.pen.bg);
        let rows = &mut self.screen_mut().rows;
        for _ in 0..n {
            let mut row = rows.remove(cursor.row);
            row.reset(cols, bg);
            rows.insert(bottom - 1, row);
        }
        self.damage.range(cursor.row, bottom);
        self.set_cursor(cursor.row, 0);
    }

    /// 擦掉第 `row` 行的 `start..end` 格。
    pub(super) fn erase_cells(&mut self, row: usize, start: usize, end: usize) {
        let bg = self.pen.bg;
        let line = &mut self.screen_mut().rows[row];
        line.erase(start, end, bg);
        if end >= line.cells.len() {
            line.wrapped = false;
        }
        self.damage.row(row);
    }

    /// 擦除第 `row` 行的整行内容，并清除行上的标记。
    fn erase_row(&mut self, row: usize) {
        let (cols, bg) = (self.cols, self.pen.bg);
        self.screen_mut().rows[row].reset(cols, bg);
        self.damage.row(row);
    }

    /// 擦除屏幕内容（ED）：0 从光标到末尾，1 从开头到光标，2 整屏，3 回滚缓冲。
    ///
    /// 主屏上擦除整屏（2，或光标在左上角时的 0）之前，先把有内容的行移入回滚缓冲，用清屏刷新输出的
    /// 程序之后仍可翻看之前的内容，条件见 `keep_screen_in_history`。
    pub(super) fn erase_display(&mut self, mode: u16) {
        let cursor = self.cursor();
        match mode {
            0 => {
                if (cursor.row, cursor.col) == (0, 0) {
                    self.keep_screen_in_history();
                }
                self.erase_cells(cursor.row, cursor.col, self.cols);
                for row in cursor.row + 1..self.rows {
                    self.erase_row(row);
                }
            }
            1 => {
                for row in 0..cursor.row {
                    self.erase_row(row);
                }
                self.erase_cells(cursor.row, 0, cursor.col + 1);
            }
            2 => {
                self.keep_screen_in_history();
                let marks = self.prompt_marks();
                for row in 0..self.rows {
                    self.erase_row(row);
                }
                // shell 重画提示符时不再发送 `A`，命令块仍依赖该标记。
                if let Some(marks) = marks {
                    self.primary.rows[self.primary.cursor.row].marks = marks;
                    self.blocks.dirty = true;
                }
            }
            3 => {
                self.history_erased = true;
                self.last_kept = None;
                self.history.clear();
                self.blocks.dirty = true;
                self.damage.reset = true;
                self.damage.all();
            }
            _ => {}
        }
        self.screen_mut().cursor.pending_wrap = false;
    }

    /// 把主屏第一行到最后一个有内容的行移入回滚缓冲，不受滚动区域限制。
    ///
    /// 以下情况不移入：
    /// - 备用屏，或屏幕为空。
    /// - 回滚缓冲刚被程序清空（`history_erased`）：`clear` 命令可能先清回滚缓冲再清屏。
    /// - 内容与上一次移入的完全相同（`last_kept`）：程序反复重绘同样的画面。
    /// - 当前提示符及其下方的行：提示符由 shell 重画，不属于历史内容。
    fn keep_screen_in_history(&mut self) {
        if self.alt || self.history_erased {
            return;
        }
        let end = self.prompt_row().unwrap_or(self.rows);
        let rows = &self.primary.rows[..end];
        let Some(last) = rows.iter().rposition(|row| !row.is_blank()) else {
            return;
        };
        let screen = &rows[..=last];
        if self.last_kept.as_deref() == Some(screen) {
            return;
        }
        let kept = screen.to_vec();
        let region = (self.top, self.bottom);
        (self.top, self.bottom) = (0, self.rows);
        self.scroll_up(last + 1);
        (self.top, self.bottom) = region;
        self.last_kept = Some(kept);
    }

    /// 返回 shell 停在提示符上时，主屏上提示符第一行的行号。
    fn prompt_row(&self) -> Option<usize> {
        if !self.blocks.at_prompt {
            return None;
        }
        (self.primary.rows.iter()).rposition(|row| row.marks.flags & Marks::PROMPT != 0)
    }

    /// 返回擦除整屏后要保留的提示符标记；程序同时清空了回滚缓冲或位于备用屏时为 `None`。
    fn prompt_marks(&self) -> Option<Marks> {
        if self.alt || self.history_erased {
            return None;
        }
        self.prompt_row().map(|row| self.primary.rows[row].marks)
    }

    /// 擦除行内容（EL）：0 从光标到行尾，1 从行首到光标，2 整行。
    pub(super) fn erase_line(&mut self, mode: u16) {
        let cursor = self.cursor();
        match mode {
            0 => self.erase_cells(cursor.row, cursor.col, self.cols),
            1 => self.erase_cells(cursor.row, 0, cursor.col + 1),
            2 => self.erase_cells(cursor.row, 0, self.cols),
            _ => {}
        }
        self.screen_mut().cursor.pending_wrap = false;
    }

    /// 清空回滚缓冲和屏幕，光标所在行移到第一行（⌘K）。
    pub(super) fn clear_all(&mut self) {
        // 备用屏由全屏程序自行绘制，只清空回滚缓冲。
        if !self.alt {
            let cursor = self.cursor();
            // 从提示符第一行保留到光标所在行：只保留最后一行时，多行提示符的其余行要到下一个提示符
            // 才恢复。
            let start = (self.prompt_row())
                .filter(|&row| row <= cursor.row)
                .unwrap_or(cursor.row);
            let (cols, rows) = (self.cols, self.rows);
            let screen = &mut self.primary;
            let kept: Vec<Row> = screen.rows.drain(start..=cursor.row).collect();
            screen.rows = kept;
            screen
                .rows
                .resize_with(rows, || Row::new(cols, Color::Default));
            screen.cursor.row = cursor.row - start;
        }
        self.last_kept = None;
        self.history.clear();
        self.blocks.dirty = true;
        self.damage.reset = true;
        self.damage.all();
    }
}
