//! 标记、跳转与修改列表：`m`、`` ` ``、`'`、`Ctrl-O`、`Ctrl-I`、`g;`、`g,`。
//!
//! 标记与修改列表的位置由宿主保存并随编辑移动；状态机只决定何时设置、如何跳转。

use winc_core::Lines;

use super::Vim;
use crate::command::Motion;
use crate::host::{Host, Message};
use crate::motion::{Moved, Span};
use crate::parser::Movement;
use crate::text;

/// 跳转之前保存的上下文，用于在光标没有移动时还原。
#[derive(Debug, Clone, Copy)]
pub(super) struct Context {
    /// 跳转之前「上一个位置」标记的值。
    previous: Option<usize>,
    /// 跳转之前光标的位置。
    from: usize,
}

/// 是否为可以用 `m` 设置的标记名。
fn settable(name: char) -> bool {
    name.is_ascii_alphabetic() || matches!(name, '\'' | '`' | '[' | ']' | '<' | '>')
}

/// 返回标记名在宿主中保存时所用的名字：`` ` `` 与 `'` 是同一个标记。
fn stored(name: char) -> char {
    if name == '`' { '\'' } else { name }
}

impl Vim {
    /// 记下跳转之前的位置 `from`：设为「上一个位置」标记，并记入跳转历史。
    pub(super) fn set_context(&mut self, host: &mut impl Host, from: usize) {
        if self.context.is_none() {
            self.context = Some(Context {
                previous: host.mark('\''),
                from,
            });
        }
        host.set_mark('\'', from);
        host.record_jump(from);
    }

    /// 命令结束时调用：光标没有离开跳转之前的位置时还原「上一个位置」标记。
    pub(super) fn check_context(&mut self, host: &mut impl Host) {
        let Some(context) = self.context.take() else {
            return;
        };
        if self.caret(host) == context.from {
            // 没有设置过时是文本的开头。
            host.set_mark('\'', context.previous.unwrap_or(0));
        }
    }

    /// `m{标记}`：把光标的位置记为标记 `name`。
    pub(super) fn set_mark(&mut self, host: &mut impl Host, name: Option<char>) {
        let Some(name) = name.filter(|name| settable(*name)) else {
            return;
        };
        let cursor = self.caret(host);
        host.set_mark(stored(name), cursor);
    }

    /// `` ` ``{标记}、`'`{标记}：求值到标记的动作；标记没有设置时提示并返回 `None`。
    pub(super) fn mark_motion(
        &mut self,
        host: &mut impl Host,
        movement: Movement,
    ) -> Option<Moved> {
        let name = movement.arg?;
        // 「上一个位置」没有设置过时是文本的开头。
        let found = match stored(name) {
            '\'' => Some(host.mark('\'').unwrap_or(0)),
            name => host.mark(name),
        };
        let Some(pos) = found else {
            self.error(host, Message::NoMark(name));
            return None;
        };
        let text = host.text();
        let pos = pos.min(text.end_pos());
        let line = text::line_of(text, pos);
        if movement.motion == Motion::MarkLine {
            return Some(Moved::new(
                text::first_non_blank(text, line),
                Span::Linewise,
            ));
        }
        let pos = pos.clamp(text.line_start(line), text.line_end(line));
        Some(Moved {
            numbered: true,
            ..Moved::new(pos, Span::Exclusive)
        })
    }

    /// `Ctrl-O`、`Ctrl-I`：在跳转历史中后退或前进 `count` 步。
    pub(super) fn jump(&mut self, host: &mut impl Host, back: bool, count: Option<u32>) {
        let count = count.map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
        if host.jump(back, count) {
            self.shown = None;
            let pos = host.selection().head.min(host.text().end_pos());
            self.settle(host, pos);
        }
    }

    /// `g;`、`g,`：到修改列表中更早（`older` 为真）或更新的位置。
    pub(super) fn change_jump(&mut self, host: &mut impl Host, older: bool, count: Option<u32>) {
        let changes = host.changes();
        if changes.is_empty() {
            return;
        }
        let count = count.map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX));
        let last = changes.len() - 1;
        // 没有记录位置时位于列表的末尾之后。
        let index = self
            .change_index
            .unwrap_or(changes.len())
            .min(changes.len());
        let target = if older {
            if index == 0 {
                return;
            }
            index.saturating_sub(count)
        } else {
            if index == last {
                return;
            }
            index.saturating_add(count).min(last)
        };
        self.change_index = Some(target);
        let pos = changes[target].min(host.text().end_pos());
        self.settle(host, pos);
    }

    /// 记下一次修改：`from` 是被修改文本的起点，`to` 是修改之后文本的最后一个字符。
    pub(super) fn note_change(&mut self, host: &mut impl Host, from: usize, to: usize) {
        host.set_mark('[', from);
        host.set_mark(']', to);
        host.set_mark('.', from);
        host.record_change(from);
        self.change_index = None;
    }
}
