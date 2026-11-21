//! 终端按键的归属：管理键留给 Winc，命令历史增强只在明确的 shell 提示符中启用。

use super::*;
use crate::keymap::{self, Action, KeyCode, Lookup, Mods, Stroke};

impl App {
    /// 聚焦显示顺序中从 0 开始的终端；越界时保留当前标签和焦点。
    pub(in crate::app) fn focus_terminal_number(&mut self, index: usize) {
        if index < self.terminal.tabs.len() {
            self.finish_terminal_rename(true);
            self.activate_terminal_tab(index);
            self.focus = Focus::Terminal;
            self.terminal.strip.revealed = None;
            if !self.terminal.open {
                self.set_terminal_open(true);
                self.relayout();
            }
            self.reset_blink();
        }
    }

    /// 是否明确位于 shell 提示符；没有集成标记或程序请求交互模式时按未知处理。
    ///
    /// 只读当前标签的快照，不查询前台进程或访问磁盘。
    pub(in crate::app) fn terminal_at_prompt(&self) -> bool {
        self.config.terminal_shell_integration != Some(false)
            && self.terminal.tab().is_some_and(|tab| {
                tab.ended.is_none()
                    && tab.run.is_none()
                    && tab.keyboard == 0
                    && ![
                        Modes::ALT_SCREEN,
                        Modes::MOUSE_PRESS,
                        Modes::MOUSE_DRAG,
                        Modes::MOUSE_MOTION,
                    ]
                    .into_iter()
                    .any(|mode| tab.modes.has(mode))
                    && tab
                        .blocks
                        .last()
                        .is_some_and(|block| block.output.is_none() && block.end.is_none())
            })
    }

    /// 查找当前窗口中的按键，被终端输入策略排除的命令也不占用组合键第一段。
    ///
    /// macOS 上带 ⌘ 的组合不会发送给程序，让出后按键只会失效，因此不参与让出。
    pub(in crate::app) fn lookup_shortcut(
        &self,
        stroke: Stroke,
        pending: Option<Stroke>,
    ) -> Lookup {
        let terminal = self.focus == Focus::Terminal;
        let application = self.config.terminal_application_priority == Some(true);
        let prompt = !terminal || self.terminal_at_prompt();
        let alternate = self.terminal_alternate_screen();
        keymap::lookup_allowed(
            &keymap::current(),
            stroke,
            pending,
            self.key_context(),
            |binding| {
                let action = binding.action;
                if !terminal {
                    return true;
                }
                // 单按 Ctrl 的复制键只在有选区时生效，否则作为控制字符发给程序。
                if action == Action::TerminalCopy
                    && control_char(binding.first)
                    && !self.terminal_selected()
                {
                    return false;
                }
                if matches!(action, Action::PreviousCommand | Action::NextCommand)
                    && !self.terminal.tab().is_some_and(|tab| {
                        tab.blocks
                            .iter()
                            .any(|block| block.output.is_some() && block.prompt >= tab.history.0)
                    })
                {
                    return false;
                }
                if cfg!(target_os = "macos") && binding.first.mods.has(Mods::META) {
                    return true;
                }
                // 单按 Ctrl 的开关面板键（Ctrl+J）在程序里是换行：终端应用优先时交给程序。
                if application
                    && action == Action::ToggleTerminalPanel
                    && control_char(binding.first)
                {
                    return false;
                }
                (!application || terminal_reserved(action))
                    && (!terminal_content(action) || prompt)
                    && (!matches!(
                        action,
                        Action::ClearTerminal
                            | Action::ScrollTerminalTop
                            | Action::ScrollTerminalBottom
                    ) || !alternate)
                    && (binding.second.is_none() || prompt)
            },
        )
    }

    /// 当前标签是否有选区。
    fn terminal_selected(&self) -> bool {
        (self.terminal.tab()).is_some_and(|tab| tab.selection_range().is_some())
    }

    /// 执行终端中按键 `stroke` 对应的命令 `actions`，返回是否已处理；单按 Ctrl 的复制键复制
    /// 后清除选区，再按一次即发给程序。
    pub(in crate::app) fn run_terminal_shortcut(
        &mut self,
        stroke: Stroke,
        actions: &[Action],
    ) -> bool {
        if control_char(stroke) && actions.first() == Some(&Action::TerminalCopy) {
            self.terminal_copy(true);
            return true;
        }
        false
    }

    /// 当前标签是否处于备用屏；备用屏没有回滚缓冲，屏幕内容由程序管理。
    fn terminal_alternate_screen(&self) -> bool {
        self.terminal
            .tab()
            .is_some_and(|tab| tab.modes.has(Modes::ALT_SCREEN))
    }

    /// 回滚缓冲的键盘翻页是否可用；备用屏和终端应用优先时翻页键交给程序。
    pub(in crate::app) fn terminal_scroll_keys(&self) -> bool {
        self.config.terminal_application_priority != Some(true) && !self.terminal_alternate_screen()
    }
}

/// 是否为单按 Ctrl 加字母：终端里是发给程序的控制字符。
fn control_char(stroke: Stroke) -> bool {
    stroke.mods == Mods::CTRL && matches!(stroke.key, KeyCode::Char(c) if c.is_ascii_alphabetic())
}

/// 标签管理和复制、粘贴在终端应用优先时仍可使用，便于离开程序或管理多个会话。
fn terminal_reserved(action: Action) -> bool {
    matches!(
        action,
        Action::ToggleTerminal
            | Action::ToggleTerminalPanel
            | Action::NewTerminal
            | Action::CloseTerminal
            | Action::NextTerminal
            | Action::PreviousTerminal
            | Action::MaximizeTerminal
            | Action::Copy
            | Action::Paste
            | Action::TerminalCopy
            | Action::TerminalPaste
    ) || action.terminal_index().is_some()
}

/// 这些操作依赖 shell 集成记录的命令块，程序运行时将相应按键交还给程序。
fn terminal_content(action: Action) -> bool {
    matches!(
        action,
        Action::PreviousCommand
            | Action::NextCommand
            | Action::FoldOutput
            | Action::UnfoldOutput
            | Action::NextError
            | Action::PreviousError
    )
}
