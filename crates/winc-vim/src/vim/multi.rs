//! 多个光标：命令在每个光标上各执行一次，寄存器保存各光标的文本。
//!
//! 各光标从后往前依次执行，前面的修改不影响后面已经得到的结果以外的位置；每执行一次，已得到
//! 的结果按文本长度的变化平移。

use winc_core::{Lines, Selection};

use super::{Shared, Vim, VisualKind};
use crate::command::{Action, Mode};
use crate::host::Host;
use crate::motion::Want;
use crate::parser::{Body, Command};
use crate::register::{Content, Kind};
use crate::text;

/// 命令 `command` 是否在每个光标上各执行一次；其余命令只在主选区上执行。
fn per_cursor(command: Command) -> bool {
    match command.body {
        Body::Motion(_) | Body::Object(_) | Body::Operator { .. } => true,
        Body::Action { action, .. } => matches!(
            action,
            Action::Insert
                | Action::Append
                | Action::InsertFirstNonBlank
                | Action::InsertLineStart
                | Action::AppendLineEnd
                | Action::OpenBelow
                | Action::OpenAbove
                | Action::ReplaceChar
                | Action::ToggleCaseChar
                | Action::PasteAfter
                | Action::PasteBefore
                | Action::PasteAfterMove
                | Action::PasteBeforeMove
                | Action::PasteIndented
                | Action::JoinLines
                | Action::JoinLinesRaw
                | Action::Increment
                | Action::Decrement
                | Action::Repeat
                | Action::InsertSelection
                | Action::AppendSelection
                | Action::Visual
                | Action::VisualLine
                | Action::Escape
        ),
    }
}

impl Vim {
    /// 宿主有多个选区且命令逐个光标执行时返回全部选区与主选区的下标。
    ///
    /// 按块的可视模式下，选区仍是状态机给出的块时不算：命令作用于块。
    pub(super) fn cursors(
        &self,
        host: &impl Host,
        command: Command,
    ) -> Option<(Vec<Selection>, usize)> {
        let all = host.selections();
        if all.len() < 2 || !per_cursor(command) {
            return None;
        }
        if let Some(visual) = self.visual
            && visual.kind == VisualKind::Block
        {
            let to_end = self.want == Want::End;
            if super::block::selections(host, visual, to_end).0 == all {
                return None;
            }
        }
        let primary = host.selection();
        let index = all.iter().position(|s| *s == primary).unwrap_or(0);
        Some((all, index))
    }

    /// 在 `all` 的每个选区上各执行一次命令 `command`；`primary` 是主选区的下标。
    pub(super) fn execute_each(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        all: &[Selection],
        primary: usize,
    ) {
        let linewise = self.visual.is_some_and(|v| v.kind == VisualKind::Line);
        let visual = self.visual.is_some();
        // 寄存器的内容由同样个数的光标写入时，粘贴逐个对应。
        let pieces: Option<Vec<String>> = match command.body {
            Body::Action {
                action:
                    Action::PasteAfter
                    | Action::PasteBefore
                    | Action::PasteAfterMove
                    | Action::PasteBeforeMove,
                ..
            } => Self::read_register(shared, host, command.register).and_then(|content| {
                (shared.pieces.clone())
                    .filter(|pieces| pieces.len() == all.len() && pieces.join("\n") == content.text)
            }),
            _ => None,
        };

        shared.capture = Some(Vec::new());
        let mut results: Vec<Selection> = Vec::with_capacity(all.len());
        let mut session = None;
        for (index, selection) in all.iter().enumerate().rev() {
            self.mode = Mode::Normal;
            self.visual = None;
            self.insert = None;
            self.shown = None;
            host.set_selections(vec![*selection], 0);
            if visual {
                self.sync(host);
                if linewise && let Some(visual) = &mut self.visual {
                    visual.kind = VisualKind::Line;
                }
            } else {
                let pos = text::on_char(host.text(), selection.head);
                self.settle(host, pos);
            }
            shared.piece = pieces.as_ref().map(|pieces| Content {
                kind: Kind::Charwise,
                text: pieces[index].clone(),
            });
            let before = host.text().end_pos();
            self.execute(shared, host, command);
            let after = host.text().end_pos();
            // 已得到的结果都在这次修改之后。
            let moved = |pos: usize| {
                if after >= before {
                    pos + (after - before)
                } else {
                    pos.saturating_sub(before - after)
                }
            };
            for result in &mut results {
                *result = Selection::new(moved(result.anchor), moved(result.head));
            }
            results.push(host.selection());
            if self.mode == Mode::Insert {
                session = self.insert.take().or(session);
            }
        }
        shared.piece = None;
        results.reverse();

        // 各光标写入寄存器的文本合成一份。
        let captured = shared.capture.take().unwrap_or_default();
        if let Some((_, source)) = captured.first().cloned() {
            let texts: Vec<Content> = captured.into_iter().rev().map(|(c, _)| c).collect();
            let all_lines = texts.iter().all(|c| c.kind == Kind::Linewise);
            let kind = if all_lines {
                Kind::Linewise
            } else {
                Kind::Charwise
            };
            let texts: Vec<String> = texts.into_iter().map(|c| c.text).collect();
            let text = texts.join("\n");
            Self::write_register(
                shared,
                host,
                command.register,
                Content { kind, text },
                source,
            );
            shared.pieces = Some(texts);
        }

        let inserting = session.is_some();
        self.insert = session;
        let still_visual = self.mode == Mode::Visual;
        // 前面的光标的修改可能改变后面的光标所在的行：普通模式下各光标重新收拢到字符上。
        let text = host.text();
        let end = text.end_pos();
        let results: Vec<Selection> = (results.iter())
            .map(|s| s.clamped(end))
            .map(|s| {
                if inserting || still_visual {
                    Selection::new(
                        winc_core::movement::snap_to_grapheme(text, s.anchor),
                        winc_core::movement::snap_to_grapheme(text, s.head),
                    )
                } else {
                    Selection::point(text::on_char(text, s.head))
                }
            })
            .collect();
        self.visual = None;
        self.mode = if inserting {
            Mode::Insert
        } else {
            Mode::Normal
        };
        host.set_selections(results.clone(), primary);
        let shown = results.get(primary).copied();
        self.shown = if inserting || !still_visual {
            shown
        } else {
            None
        };
        if still_visual && !inserting {
            self.sync(host);
            if linewise && let Some(visual) = &mut self.visual {
                visual.kind = VisualKind::Line;
            }
        }
    }
}
