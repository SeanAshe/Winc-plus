//! 编辑器到终端：在终端中打开一个目录、把文件拖进终端插入路径、把选中的代码发到终端执行
//! （⌃⇧↩）。

use std::path::Path;

use winc_core::Lines;
use winc_pty::input;
use winc_pty::protocol::Modes;

use super::{App, Focus, os_bytes};

/// 给 shell 用的路径：只有安全的字符时原样，否则用单引号包起来（里面的单引号写成 `'\''`）。
pub(crate) fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    let safe = |c: char| {
        c.is_alphanumeric()
            || matches!(
                c,
                '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@' | '%' | '=' | '~'
            )
    };
    if !text.is_empty() && text.chars().all(safe) {
        return text.into_owned();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        if c == '\'' {
            out.push_str(r"'\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

impl App {
    /// 模块开着时「在终端中打开」用内置的终端：在 `dir` 新建一个标签。
    ///
    /// 返回是否处理了。
    pub(crate) fn open_in_builtin_terminal(&mut self, dir: &Path) -> bool {
        if !self.terminal_enabled() {
            return false;
        }
        self.new_terminal_tab_in(os_bytes(dir.as_os_str()));
        true
    }

    /// 拖进窗口的文件松手在 (`x`, `y`) 处时是否交给终端：落在终端面板里，且有终端标签。
    pub(crate) fn terminal_takes_drop(&self, x: f32, y: f32) -> bool {
        self.layout.terminal.contains(x, y) && self.terminal.tab().is_some()
    }

    /// 文件拖到了终端上（见 [`Self::terminal_takes_drop`]）：在光标处插入加好引号的路径，后面
    /// 跟一个空格。
    pub(crate) fn drop_path_on_terminal(&mut self, path: &Path) {
        let text = format!("{} ", shell_quote(path));
        self.terminal_input(text.into_bytes());
        self.focus = Focus::Terminal;
        self.request_redraw();
    }

    /// ⌃⇧↩：把编辑区选中的代码（没有选区时是光标所在的一行）发到正在看的终端执行。
    ///
    /// 多行的在 shell 开着括号粘贴时整段粘进去再回车，不会一行一行地执行到一半。还没有终端时
    /// 新建一个。焦点留在编辑区。
    pub(in crate::app) fn send_to_terminal(&mut self) {
        if !self.terminal_enabled() {
            self.flash(winc_i18n::t!("terminal-module-disabled"));
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let selection = doc.selection;
        let range = if selection.is_empty() {
            doc.text().line_span(doc.text().line_of(selection.head))
        } else {
            selection.range()
        };
        let Some(text) = doc.text_range(range.start, range.end) else {
            return;
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            return;
        }
        let modes = self.terminal.tab().map_or_else(Modes::default, |t| t.modes);
        let mut bytes = Vec::new();
        if text.contains('\n') {
            input::paste(text, modes, &mut bytes);
        } else {
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes.push(b'\r');

        let focus = self.focus;
        let t = &self.terminal;
        // shell 退出了、伪终端断开了、辅助进程中止了：重新开始，代码等辅助进程启动后再送。
        let dead = t.crashed || t.tab().is_some_and(|tab| tab.ended.is_some());
        if t.tabs.is_empty() {
            self.terminal.pending_input = Some(bytes);
            self.new_terminal_tab();
        } else {
            self.set_terminal_open(true);
            self.relayout();
            if self.terminal_restart_needs_confirm() {
                // 不替用户结束还可能在运行的程序：说明原因，由用户自己重新启动。
                self.flash(winc_i18n::t!("terminal-disconnected"));
            } else if dead {
                self.terminal.pending_input = Some(bytes);
                self.restart_terminal_tab();
            } else {
                self.terminal_input(bytes);
            }
        }
        self.focus = focus;
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests;
