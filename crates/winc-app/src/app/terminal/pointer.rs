//! 终端里的鼠标：选字、滚轮翻回滚缓冲，以及程序要了鼠标时上报给它。
//!
//! 程序开了鼠标上报（全屏程序）时，按键、拖动、滚轮都编码后发给程序；按住 ⇧ 时仍然由
//! 终端自己选字。没开时：单击拖动按字选、双击选词、三击选行；滚轮在主屏上翻回滚缓冲，在备用屏
//! 上（分页器）换成 ↑ ↓。

use winit::event::MouseButton;

use winc_pty::input::{self, Button, MouseAction};
use winc_pty::protocol::Modes;

use super::tab::{Point, Selection, Unit, ViewRow};
use super::{App, Focus, grid_rect};
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
    /// 窗口坐标落在网格的哪一行（网格上的第几行，超出网格的夹到边上）。
    fn terminal_view_row(&self, y: f32) -> Option<usize> {
        let tab = self.terminal.tab()?;
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        let row = ((y - grid.y) / self.fonts.line_height).floor().max(0.0) as usize;
        Some(row.min(tab.grid_rows().max(1) - 1))
    }

    /// 网格第 `row` 行上显示的是不是终端里的一行字：吸顶的命令行和折起的那一行不是。
    pub(in crate::app) fn terminal_shows_line(&self, row: usize) -> bool {
        let Some(tab) = self.terminal.tab() else {
            return false;
        };
        if row == 0 && tab.sticky().is_some() {
            return false;
        }
        matches!(
            tab.row_at(tab.view_top() + row as u64),
            Some(ViewRow::Line(_))
        )
    }

    /// 窗口坐标落在屏幕的哪一格：（屏幕上的行，格），超出网格的夹到边上，上报给程序用。
    ///
    /// 往上翻着、或者落在回滚缓冲的行上时没有对应的屏幕行，返回 `None`。
    fn terminal_cell(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let tab = self.terminal.tab()?;
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        let col = ((x - grid.x) / self.cell).floor().max(0.0) as usize;
        let cols = usize::from(tab.size.cols).max(1);
        let row = self.terminal_view_row(y)?;
        let line = match tab.row_at(tab.view_top() + row as u64)? {
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
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        let mut row = self.terminal_view_row(y)? as u64;
        if row == 0 && tab.sticky().is_some() {
            row = 1;
        }
        let cells = (x - grid.x) / self.cell;
        let col = if between {
            cells.round()
        } else {
            cells.floor()
        };
        let col = (col.max(0.0) as usize).min(usize::from(tab.size.cols));
        let top = tab.view_top();
        let last = tab.display_len().checked_sub(1)?;
        let row = tab.row_at((top + row).min(last))?;
        Some(Point {
            line: row.line(tab),
            col,
        })
    }

    /// 窗口坐标落在命令块的哪个可以点的地方：吸顶的命令行、色带（网格左边）、折起的那一行。
    pub(in crate::app) fn terminal_block_hit(&self, x: f32, y: f32) -> Option<BlockHit> {
        let tab = self.terminal.tab()?;
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        if y < grid.y || y >= grid.bottom() || x >= grid.right() {
            return None;
        }
        let row = self.terminal_view_row(y)?;
        if row == 0 && tab.sticky().is_some() {
            return Some(BlockHit::Sticky);
        }
        let block = match tab.row_at(tab.view_top() + row as u64)? {
            ViewRow::Fold(block, _) => block,
            ViewRow::Line(line) if x < grid.x => tab.block_at(line)?,
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
        let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
        if y < grid.y || y >= grid.bottom() || x >= grid.right() {
            return None;
        }
        let row = self.terminal_view_row(y)?;
        let block = match (row, tab.sticky()) {
            (0, Some(block)) => block,
            _ => match tab.row_at(tab.view_top() + row as u64)? {
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
        !self.modifiers.shift_key()
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
        if button == MouseButton::Left && self.open_terminal_link(self.modifiers.shift_key()) {
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
            self.drag = Drag::TerminalMouse(button);
            return;
        }
        if button != MouseButton::Left || self.terminal_block_click(x, y) {
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
        self.drag = Drag::TerminalSelect;
    }

    /// 按着键拖动：选字时扩选区（拖出网格上下时一次翻一行），上报时报拖动。
    pub(in crate::app) fn terminal_mouse_drag(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::TerminalMouse(button) => {
                if self.terminal_cell(x, y) != self.terminal.last_mouse_cell {
                    self.report_terminal_mouse(button, MouseAction::Motion, (x, y));
                }
            }
            Drag::TerminalSelect => {
                let grid = grid_rect(self.layout.terminal, self.fonts.line_height);
                if y < grid.y {
                    self.scroll_terminal(1);
                } else if y >= grid.bottom() {
                    self.scroll_terminal(-1);
                }
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
        match self.drag {
            Drag::TerminalMouse(button) => {
                self.report_terminal_mouse(button, MouseAction::Release, self.mouse);
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
    /// 攒够一行才动一行。
    pub(in crate::app) fn terminal_wheel(&mut self, dy: f32) {
        let line_height = self.fonts.line_height;
        let reports = self.terminal_reports_mouse();
        let Some(tab) = self.terminal.tab_mut() else {
            return;
        };
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
                self.report_terminal_mouse(button, MouseAction::Press, self.mouse);
            }
        } else if modes.has(Modes::ALT_SCREEN) {
            // 备用屏上没有回滚缓冲：分页器里滚轮换成方向键。
            if modes.has(Modes::ALT_SCROLL) {
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
        } else {
            self.scroll_terminal(lines);
        }
    }
}
