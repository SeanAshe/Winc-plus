//! 终端里的鼠标：选字、滚轮翻回滚缓冲，以及程序要了鼠标时上报给它。
//!
//! 程序开了鼠标上报（全屏程序）时，按键、拖动、滚轮都编码后发给程序；按住 ⇧ 时仍然由
//! 终端自己选字。没开时：单击拖动按字选、双击选词、三击选行，⇧ 单击扩展选区，⌥ 单击移动 shell
//! 的光标；滚轮在主屏上滚动回滚缓冲，在备用屏上（分页器）换成 ↑ ↓。

use winit::event::MouseButton;

use winc_pty::input::{self, Button, MouseAction};
use winc_pty::protocol::Modes;

use super::tab::{Point, Selection, Unit, ViewRow};
use super::{App, Focus};
use crate::app::Drag;

/// 命令块上可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockHit {
    /// 吸顶的命令行。
    Sticky,
    /// 第几个块的色带（或者它折起的那一行）。
    Band(usize),
}

impl App {
    /// 窗口坐标落在第几个显示行（超出内容区域的夹到边上）。
    fn terminal_display_row(&self, y: f32) -> u64 {
        let view = self.terminal_view();
        let (first, origin) = self.terminal_origin();
        let y = y.clamp(view.y, view.bottom() - 0.5);
        first
            + ((y - origin) / self.fonts.terminal_line_height)
                .floor()
                .max(0.0) as u64
    }

    /// 窗口坐标 `y` 是否落在吸顶行上。
    fn terminal_on_sticky(&self, y: f32) -> bool {
        let band = self.terminal_sticky_band();
        let sticky = self.terminal.tab().is_some_and(|t| t.sticky().is_some());
        sticky && y < band.bottom()
    }

    /// 窗口坐标 `y` 上显示的是不是终端里的一行字：吸顶的命令行和折起的那一行不是。
    pub(in crate::app) fn terminal_shows_line(&self, y: f32) -> bool {
        let Some(tab) = self.terminal.tab() else {
            return false;
        };
        if self.terminal_on_sticky(y) {
            return false;
        }
        matches!(
            tab.row_at(self.terminal_display_row(y)),
            Some(ViewRow::Line(_))
        )
    }

    /// 窗口坐标落在屏幕的哪一格：（屏幕上的行，格），超出网格的夹到边上，上报给程序用。
    ///
    /// 往上翻着、或者落在回滚缓冲的行上时没有对应的屏幕行，返回 `None`。
    fn terminal_cell(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let tab = self.terminal.tab()?;
        let grid = self.terminal_grid();
        let col = ((x - grid.x) / self.metrics.terminal_cell).floor().max(0.0) as usize;
        let cols = usize::from(tab.size.cols).max(1);
        let line = match tab.row_at(self.terminal_display_row(y))? {
            ViewRow::Line(line) => line,
            ViewRow::Fold(..) => return None,
        };
        let row = line.checked_sub(tab.history.1)?;
        Some((row as u16, col.min(cols - 1) as u16))
    }

    /// 窗口坐标对应的选区位置。
    ///
    /// `between` 为真时取两格之间的那条缝（按字选时用，鼠标过了一格的
    /// 一半才算选上它），否则取所在的那一格。网格下面没有内容的地方算最后一行；吸顶的命令行
    /// 盖着的那一行看不见，算它下面一行。
    pub(super) fn terminal_point(&self, x: f32, y: f32, between: bool) -> Option<Point> {
        let tab = self.terminal.tab()?;
        let grid = self.terminal_grid();
        let y = if self.terminal_on_sticky(y) {
            self.terminal_sticky_band().bottom()
        } else {
            y
        };
        let cells = (x - grid.x) / self.metrics.terminal_cell;
        let col = if between {
            cells.round()
        } else {
            cells.floor()
        };
        let col = (col.max(0.0) as usize).min(usize::from(tab.size.cols));
        let last = tab.display_len().checked_sub(1)?;
        let row = tab.row_at(self.terminal_display_row(y).min(last))?;
        Some(Point {
            line: row.line(tab),
            col,
        })
    }

    /// 窗口坐标落在命令块的哪个可以点的地方：吸顶的命令行、色带（网格左边）、折起的那一行。
    pub(in crate::app) fn terminal_block_hit(&self, x: f32, y: f32) -> Option<BlockHit> {
        let tab = self.terminal.tab()?;
        let view = self.terminal_view();
        if y < view.y || y >= view.bottom() || x >= view.right() {
            return None;
        }
        if self.terminal_on_sticky(y) {
            return Some(BlockHit::Sticky);
        }
        let block = match tab.row_at(self.terminal_display_row(y))? {
            ViewRow::Fold(block, _) => block,
            ViewRow::Line(line) if x < view.x => tab.block_at(line)?,
            ViewRow::Line(_) => return None,
        };
        tab.blocks[block]
            .output
            .is_some()
            .then_some(BlockHit::Band(block))
    }

    /// 窗口坐标所在的、已经运行过的命令块的编号：吸顶的那一行算它那个块，折起的那一行算被折起
    /// 的块，其余按所在的行。
    ///
    /// 右键菜单用。
    pub(in crate::app) fn terminal_block_under(&self, x: f32, y: f32) -> Option<u32> {
        let tab = self.terminal.tab()?;
        let view = self.terminal_view();
        if y < view.y || y >= view.bottom() || x >= view.right() {
            return None;
        }
        let block = match tab.sticky().filter(|_| self.terminal_on_sticky(y)) {
            Some(block) => block,
            None => match tab.row_at(self.terminal_display_row(y))? {
                ViewRow::Fold(block, _) => block,
                ViewRow::Line(line) => tab.block_at(line)?,
            },
        };
        let block = &tab.blocks[block];
        block.output.is_some().then_some(block.id)
    }

    /// 点在命令块上：吸顶的命令行跳回那条命令，色带和折起的那一行折起、展开。
    ///
    /// 处理了返回真。
    fn terminal_block_click(&mut self, x: f32, y: f32) -> bool {
        match self.terminal_block_hit(x, y) {
            Some(BlockHit::Sticky) => {
                if let Some(tab) = self.terminal.tab_mut()
                    && let Some(block) = tab.sticky()
                {
                    let prompt = tab.blocks[block].prompt;
                    tab.jump = Some(tab.blocks[block].id);
                    tab.scroll_to(prompt, 0);
                }
                self.fetch_terminal_history();
                self.request_redraw();
                true
            }
            Some(BlockHit::Band(block)) => {
                self.click_terminal_band(block);
                true
            }
            None => false,
        }
    }

    /// 程序当前是否需要鼠标：开了鼠标上报、没按住 ⇧。
    fn terminal_reports_mouse(&self) -> bool {
        !self.pointer.modifiers.shift_key()
            && self
                .terminal
                .tab()
                .is_some_and(|t| t.ended.is_none() && input::wants_mouse(t.modes))
    }

    /// 把一次鼠标事件上报给程序。
    fn report_terminal_mouse(&mut self, button: Button, action: MouseAction, (x, y): (f32, f32)) {
        let Some(cell) = self.terminal_cell(x, y) else {
            return;
        };
        let Some(modes) = self.terminal.tab().map(|t| t.modes) else {
            return;
        };
        let mods = self.terminal_mods();
        let mut bytes = Vec::new();
        if input::mouse(button, action, (cell.1, cell.0), mods, modes, &mut bytes) {
            self.terminal_input(bytes);
        }
        self.terminal.last_mouse_cell = Some(cell);
    }

    /// 在终端的网格上按下鼠标键。
    pub(in crate::app) fn terminal_mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        self.focus = Focus::Terminal;
        self.reset_blink();
        self.request_redraw();
        // ⌘ 单击链接（程序要了鼠标时也是）。
        if button == MouseButton::Left
            && self.open_terminal_link(self.pointer.modifiers.shift_key())
        {
            return;
        }
        if self.terminal_reports_mouse() {
            let button = match button {
                MouseButton::Left => Button::Left,
                MouseButton::Middle => Button::Middle,
                MouseButton::Right => Button::Right,
                _ => return,
            };
            self.report_terminal_mouse(button, MouseAction::Press, (x, y));
            self.pointer.drag = Drag::TerminalMouse(button);
            return;
        }
        if button != MouseButton::Left || self.terminal_block_click(x, y) {
            return;
        }
        if self.pointer.modifiers.alt_key() && self.move_shell_cursor(x, y) {
            return;
        }
        // ⇧ 单击：保留选区起点，把终点移到点击处，之后可以继续拖动。
        if self.pointer.modifiers.shift_key()
            && let Some(head) = self.terminal_point(x, y, true)
            && let Some(selection) = self.terminal.tab_mut().and_then(|t| t.selection.as_mut())
        {
            selection.head = head;
            self.pointer.drag = Drag::TerminalSelect;
            return;
        }
        let unit = match self.click_count(x, y) {
            1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let Some(anchor) = self.terminal_point(x, y, unit == Unit::Char) else {
            return;
        };
        if let Some(tab) = self.terminal.tab_mut() {
            tab.selection = Some(Selection {
                anchor,
                head: anchor,
                unit,
            });
        }
        self.pointer.drag = Drag::TerminalSelect;
    }

    /// 按着键拖动：选字时扩选区（拖出网格上下时一次翻一行），上报时报拖动。
    pub(in crate::app) fn terminal_mouse_drag(&mut self, x: f32, y: f32) {
        match self.pointer.drag {
            Drag::TerminalMouse(button) => {
                if self.terminal_cell(x, y) != self.terminal.last_mouse_cell {
                    self.report_terminal_mouse(button, MouseAction::Motion, (x, y));
                }
            }
            Drag::TerminalSelect => {
                self.terminal_drag_scroll(y);
                let between = self
                    .terminal
                    .tab()
                    .and_then(|t| t.selection)
                    .is_some_and(|s| s.unit == Unit::Char);
                if let Some(head) = self.terminal_point(x, y, between)
                    && let Some(selection) =
                        self.terminal.tab_mut().and_then(|t| t.selection.as_mut())
                {
                    selection.head = head;
                }
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// 拖选时鼠标位于内容区上下之外：按超出的距离滚动回滚缓冲，滚动量按经过的时间计算。
    ///
    /// 鼠标静止时仍逐帧滚动（[`App::drag_autoscrolling`]），速度不受鼠标事件频率影响。
    fn terminal_drag_scroll(&mut self, y: f32) {
        let view = self.terminal_view();
        let outside = if y < view.y {
            view.y - y
        } else if y >= view.bottom() {
            view.bottom() - y
        } else {
            0.0
        };
        if outside == 0.0 {
            self.pointer.drag_scrolled_at = None;
            return;
        }
        let lines = self.drag_scroll_lines(outside.abs() / self.fonts.terminal_line_height);
        if let Some(tab) = self.terminal.tab_mut() {
            let (position, _) = tab.scroll_position();
            tab.set_scroll_position(position + f64::from(lines.copysign(outside)));
        }
        self.fetch_terminal_history();
    }

    /// ⌥ 单击：发送相应次数的方向键，把 shell 的光标移到点击处；已处理时返回真。
    ///
    /// 仅在光标确定位于输入行时生效：有 shell 集成、最后一个块尚未运行、点击处与光标同行。软换行的
    /// 输入和提示符不做特殊处理，方向键越界时由 shell 忽略。
    fn move_shell_cursor(&mut self, x: f32, y: f32) -> bool {
        let Some(point) = self.terminal_point(x, y, true) else {
            return false;
        };
        let Some(tab) = self.terminal.tab() else {
            return false;
        };
        let at_prompt = tab
            .blocks
            .last()
            .is_some_and(|block| block.output.is_none());
        let line = tab.history.1 + u64::from(tab.cursor.row);
        if !at_prompt
            || tab.modes.has(Modes::ALT_SCREEN)
            || tab.ended.is_some()
            || point.line != line
        {
            return false;
        }
        let Some(row) = tab.rows.get(usize::from(tab.cursor.row)) else {
            return false;
        };
        let (key, count) = input::cursor_steps(&row.text, usize::from(tab.cursor.col), point.col);
        let (modes, keyboard) = (tab.modes, tab.keyboard);
        let mut bytes = Vec::new();
        for _ in 0..count {
            input::encode(key, input::Mods::default(), modes, keyboard, &mut bytes);
        }
        self.terminal_input(bytes);
        true
    }

    /// 没按键时在网格上移动：程序要了所有移动（1003）时上报，同一格里不重复报。
    pub(in crate::app) fn terminal_mouse_hover(&mut self, x: f32, y: f32) {
        let wants = self
            .terminal
            .tab()
            .is_some_and(|t| t.modes.has(Modes::MOUSE_MOTION));
        if wants
            && self.terminal_reports_mouse()
            && self.terminal_cell(x, y) != self.terminal.last_mouse_cell
        {
            self.report_terminal_mouse(Button::None, MouseAction::Motion, (x, y));
        }
    }

    /// 松开鼠标键：上报松开；选字时什么也没选上（单击）就清掉选区。
    pub(in crate::app) fn terminal_mouse_up(&mut self) {
        match self.pointer.drag {
            Drag::TerminalMouse(button) => {
                self.report_terminal_mouse(button, MouseAction::Release, self.pointer.mouse);
            }
            Drag::TerminalSelect => {
                if let Some(tab) = self.terminal.tab_mut()
                    && tab.selection_range().is_none()
                {
                    tab.selection = None;
                } else if self.config.terminal_copy_on_select == Some(true) {
                    self.terminal_copy();
                }
                self.request_redraw();
            }
            _ => {}
        }
    }

    /// 滚轮在终端上（`dy` 是逻辑像素，正数往上）。
    ///
    /// 翻回滚缓冲时按像素移动；上报给程序、改发方向键时攒够一行才发一次。
    pub(in crate::app) fn terminal_wheel(&mut self, dy: f32) {
        let line_height = self.fonts.terminal_line_height;
        let reports = self.terminal_reports_mouse();
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
        if !reports && !tab.modes.has(Modes::ALT_SCREEN) {
            let (position, _) = tab.scroll_position();
            tab.set_scroll_position(position + f64::from(dy / line_height));
            self.fetch_terminal_history();
            self.request_redraw();
            return;
        }
        tab.scroll_rest += dy / line_height;
        let lines = tab.scroll_rest.trunc();
        tab.scroll_rest -= lines;
        let lines = lines as isize;
        if lines == 0 {
            return;
        }
        let modes = tab.modes;
        if reports {
            let button = if lines > 0 {
                Button::WheelUp
            } else {
                Button::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.report_terminal_mouse(button, MouseAction::Press, self.pointer.mouse);
            }
        } else if modes.has(Modes::ALT_SCROLL) {
            // 备用屏上没有回滚缓冲：分页器里滚轮换成方向键。
            let key = if lines > 0 {
                input::Key::Up
            } else {
                input::Key::Down
            };
            let mut bytes = Vec::new();
            for _ in 0..lines.unsigned_abs().min(10) {
                input::encode(key, input::Mods::default(), modes, 0, &mut bytes);
            }
            self.terminal_input(bytes);
        }
    }
}
