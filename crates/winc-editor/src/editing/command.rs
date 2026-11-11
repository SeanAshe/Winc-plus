//! 编辑命令 [`Command`] 及其在 [`Document`] 上的执行。
//!
//! 移动类命令直接修改选区；编辑类命令生成事务（[`Transaction`]）并记入撤销历史。
//! 只读文档拒绝编辑类命令，有多个选区时命令在每个选区上各执行一次。

use std::borrow::Cow;

use winc_core::edit::{self, EditKind, EditOptions, Transaction};
use winc_core::text::Aligned;
use winc_core::transform::{self, Case};
use winc_core::{Lines, Restored, Selection, movement};
use winc_syntax::Language;

use crate::Document;
use crate::editing::folding::Tracked;

/// 编辑命令。
///
/// 界面层把按键映射成编辑命令，再交给 [`Document::execute`] 执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 光标左移一个字素，或按词左移（`word`）；`extend` 为真时保留选区起点。
    MoveLeft {
        /// 是否按词移动，而不是按字素。
        word: bool,
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标右移一个字素，或按词右移（`word`）；`extend` 为真时保留选区起点。
    MoveRight {
        /// 是否按词移动，而不是按字素。
        word: bool,
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标上移一行，按显示列对齐。
    MoveUp {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标下移一行，按显示列对齐。
    MoveDown {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标上移一页（一页的行数由界面按可视高度给出）。
    PageUp {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标下移一页（一页的行数由界面按可视高度给出）。
    PageDown {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标移到行首：不在首个非空白字符处时先移到该字符，否则移到第 0 列。
    LineStart {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标移到行末。
    LineEnd {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标移到文档开头。
    DocStart {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 光标移到文档末尾。
    DocEnd {
        /// 是否扩展选区（按住 Shift）。
        extend: bool,
    },
    /// 选中整个文档。
    SelectAll,
    /// 输入文本：替换选区（没有选区时在光标处插入），单个括号或引号补全另一半或包住选区。
    Insert(String),
    /// 粘贴文本；多行代码在满足条件时先调整到光标所在的缩进。
    Paste(String),
    /// 粘贴整行文本，插到当前行的上方。
    InsertLinesAbove(String),
    /// 回车：沿用当前行缩进，必要时多缩进一级。
    Newline,
    /// 退格：有选区时删除选区，否则删除前一个字素或一个词（`word` 为真时按词）。
    Backspace {
        /// 是否按词删除，而不是按字素。
        word: bool,
    },
    /// 向后删除一个字素或一个词（`word` 为真时按词）。
    Delete {
        /// 是否按词删除，而不是按字素。
        word: bool,
    },
    /// 删除光标到行首之间的文本（`⌘⌫`）。
    DeleteToLineStart,
    /// 插入一级缩进；有选区时按整块缩进处理。
    Indent,
    /// 光标所在的行（或选区覆盖的行）整行加一级缩进（`⌘]`）。
    IndentLines,
    /// 选中的每一行去掉最多一级缩进。
    Outdent,
    /// 切换行注释：全部已注释时取消注释，否则加上注释前缀。
    ToggleComment,
    /// 删除当前行（或选区覆盖的行），包括换行符。
    DeleteLine,
    /// 选中的行与上一行（`down` 为真时为下一行）交换位置（`⌥↑`、`⌥↓`）。
    MoveLines {
        /// 是否向下移动。
        down: bool,
    },
    /// 复制整行，放在下方（`down` 为真时）或上方（`⇧⌥↑`、`⇧⌥↓`）。
    CopyLines {
        /// 是否向下复制。
        down: bool,
    },
    /// 改变选区的大小写；没有选区时改变光标所在的词。
    ChangeCase(Case),
    /// 对选中的行排序；选区在一行之内时对整个文件排序。
    SortLines {
        /// 是否按降序排列。
        descending: bool,
    },
    /// 合并行（`⌃J`）。
    JoinLines,
    /// 删除整个文件中每行行尾的空白。
    TrimTrailingWhitespace,
    /// 跳到配对的括号（`⇧⌘\`）。
    JumpToBracket,
    /// 扩大选区（`⌃⇧⌘→`）。
    ExpandSelection,
    /// 缩小选区（`⌃⇧⌘←`）：退回到上一次扩大之前的选区。
    ShrinkSelection,
    /// 撤销上一个撤销单元。
    Undo,
    /// 重做最近撤销的撤销单元。
    Redo,
}

impl Command {
    /// 是否会修改文本。
    ///
    /// 只读文档拒绝返回真值的命令。
    pub fn is_edit(&self) -> bool {
        !matches!(
            self,
            Command::MoveLeft { .. }
                | Command::MoveRight { .. }
                | Command::MoveUp { .. }
                | Command::MoveDown { .. }
                | Command::PageUp { .. }
                | Command::PageDown { .. }
                | Command::LineStart { .. }
                | Command::LineEnd { .. }
                | Command::DocStart { .. }
                | Command::DocEnd { .. }
                | Command::SelectAll
                | Command::JumpToBracket
                | Command::ExpandSelection
                | Command::ShrinkSelection
        )
    }
}

impl Document {
    /// 在文档上执行 `command`。
    ///
    /// `page_lines` 是翻页命令一次移动的行数（行），`tab_width` 是制表符宽度（列）。
    /// 只读文档拒绝编辑类命令，此时不做任何事。有多个选区时，除撤销、重做外的命令在每个
    /// 选区上各执行一次。
    pub fn execute(&mut self, command: Command, page_lines: usize, tab_width: usize) {
        if self.edit_blocked(&command).is_some() {
            return;
        }

        // 移动选区的命令记下移动前的选区，供光标撤销（`⌘U`）退回。
        if !command.is_edit() {
            self.record_cursors();
        }

        if self.has_cursors() && !matches!(command, Command::Undo | Command::Redo) {
            self.execute_each(command, page_lines, tab_width);
        } else {
            self.execute_one(command, page_lines, tab_width);
        }
    }

    /// 只按主选区执行 `command`；参数含义同 [`Self::execute`]，调用方已检查过能否执行。
    pub(crate) fn execute_one(&mut self, command: Command, page_lines: usize, tab_width: usize) {
        let page = page_lines.max(1) as isize;
        let vertical = match command {
            Command::MoveUp { extend } => Some((-1, extend)),
            Command::MoveDown { extend } => Some((1, extend)),
            Command::PageUp { extend } => Some((-page, extend)),
            Command::PageDown { extend } => Some((page, extend)),
            _ => None,
        };

        // 连续上下移动时保持目标显示列，其他命令都会重置它。
        if let Some((delta, extend)) = vertical {
            self.move_vertically(delta, extend, tab_width);
            return;
        }

        self.goal_cell = None;

        match command {
            Command::ExpandSelection => {
                // 光标或选区被其他操作改动过，此前记录的扩大层级作废。
                if self.grown.last().map(|&(_, to)| to) != Some(self.selection) {
                    self.grown.clear();
                }
                if let Some(next) = transform::expand_selection(&self.text, self.selection) {
                    self.grown.push((self.selection, next));
                    self.selection = next;
                }
                return;
            }
            Command::ShrinkSelection => {
                match self.grown.pop() {
                    Some((before, to)) if to == self.selection => self.selection = before,
                    _ => self.grown.clear(),
                }
                return;
            }
            _ => {}
        }

        match command {
            Command::Undo | Command::Redo => {
                let mut text = Tracked {
                    text: &mut self.text,
                    folds: &mut self.folds,
                    bookmarks: &mut self.bookmarks,
                    whole: &mut self.whole,
                };
                let result = if command == Command::Undo {
                    self.history.undo(&mut text)
                } else {
                    self.history.redo(&mut text)
                };
                self.restore(result);
            }
            command if command.is_edit() => {
                if let Some(transaction) = self.transaction(&command, tab_width) {
                    self.apply(transaction);
                }
            }
            command => {
                let before = self.selection.head;
                // 左右移动、移到文件末尾时跳过折叠隐藏的行；跳到配对括号、全选不跳过，
                // 目标在哪里就展开到哪里。
                let steps = matches!(
                    command,
                    Command::MoveLeft { .. }
                        | Command::MoveRight { .. }
                        | Command::DocEnd { extend: false }
                );
                self.selection = self.moved(command);
                if steps {
                    self.skip_hidden(before);
                }
            }
        }
    }

    /// 光标上下移动 `delta` 个显示行，`extend` 为真时保留选区起点；连续移动时保持目标显示列。
    ///
    /// 已折叠隐藏的行不计入；自动换行时按屏幕上的行移动。
    pub(crate) fn move_vertically(&mut self, delta: isize, extend: bool, tab_width: usize) {
        let before = self.selection.head;
        if let Some((pos, goal)) = self.wrapped_vertical(delta) {
            self.goal_cell = Some(goal);
            self.selection = self.selection.moved_to(pos, extend);
            self.skip_hidden(before);
            return;
        }
        let delta = self.line_delta(delta);
        // CSV 按列对齐显示时，按绘制出的列位置上下移动，保持在同一列。
        let (pos, goal) = match self.columns(tab_width) {
            Some(columns) => movement::vertical(
                &self.text,
                self.selection.head,
                delta,
                self.goal_cell,
                Aligned {
                    columns: &columns,
                    tab_width,
                },
            ),
            None => movement::vertical(
                &self.text,
                self.selection.head,
                delta,
                self.goal_cell,
                tab_width,
            ),
        };
        self.goal_cell = Some(goal);
        self.selection = self.selection.moved_to(pos, extend);
        self.skip_hidden(before);
    }

    /// 应用撤销或重做的结果：刷新受影响的缓存，把选区夹取到文档范围内，并恢复多光标。
    fn restore(&mut self, result: Option<Restored>) {
        if let Some(Restored {
            selection,
            extra,
            first,
        }) = result
        {
            self.after_change(first);
            self.redetect_line_ending();
            self.selection = selection.clamped(self.text.end_pos());
            self.restore_extra(extra);
        }
    }

    /// 返回移动类命令 `command` 执行后的选区；其他命令返回当前选区。
    fn moved(&self, command: Command) -> Selection {
        let text = &self.text;
        let sel = self.selection;

        match command {
            Command::MoveLeft { word, extend } => {
                if !extend && !word && !sel.is_empty() {
                    return Selection::point(sel.from());
                }

                let pos = if word {
                    movement::prev_word(text, sel.head)
                } else {
                    movement::prev_grapheme(text, sel.head)
                };
                sel.moved_to(pos, extend)
            }
            Command::MoveRight { word, extend } => {
                if !extend && !word && !sel.is_empty() {
                    return Selection::point(sel.to());
                }

                let pos = if word {
                    movement::next_word(text, sel.head)
                } else {
                    movement::next_grapheme(text, sel.head)
                };
                sel.moved_to(pos, extend)
            }
            Command::LineStart { extend } => {
                let pos = self
                    .wrapped_home_end(false)
                    .unwrap_or_else(|| movement::smart_home(text, sel.head));
                sel.moved_to(pos, extend)
            }
            Command::LineEnd { extend } => {
                let (line, _) = text.pos_to_line_col(sel.head);
                let pos = self
                    .wrapped_home_end(true)
                    .unwrap_or_else(|| text.line_end(line));
                sel.moved_to(pos, extend)
            }
            Command::DocStart { extend } => sel.moved_to(0, extend),
            Command::DocEnd { extend } => sel.moved_to(text.end_pos(), extend),
            Command::SelectAll => Selection::new(0, text.end_pos()),
            Command::JumpToBracket => {
                transform::jump_to_bracket(text, sel.head).map_or(sel, Selection::point)
            }
            _ => sel,
        }
    }

    /// 把编辑类命令转换为事务；命令没有产生修改时返回 `None`。
    pub(crate) fn transaction(&self, command: &Command, tab_width: usize) -> Option<Transaction> {
        let text = &self.text;
        let sel = self.selection;
        let language = self.highlighter.language();
        let line_ending = self.line_ending.as_str();
        let options = EditOptions {
            tab_width,
            indent_unit: self.indent().unit(),
            line_ending,
            indent_after: language.indent_after,
            tag_indent: language.tag_indent,
            line_comment: language.comment_prefix(),
            single_quote_string: language.single_quote_string,
        };

        match command {
            Command::Insert(s) => Some(insert(
                text,
                sel,
                &normalize_newlines(s, line_ending),
                language,
            )),
            Command::Paste(s) => Some(edit::paste(text, sel, &normalize_newlines(s, line_ending))),
            Command::InsertLinesAbove(s) => Some(edit::insert_lines_above(
                text,
                sel,
                &normalize_newlines(s, line_ending),
            )),
            Command::Newline => Some(edit::newline(text, sel, &options)),
            &Command::Backspace { word } => {
                if !word && inside_empty_pair(text, sel, language) {
                    edit::delete_pair(text, sel)
                } else {
                    edit::backspace(text, sel, word, &options)
                }
            }
            &Command::Delete { word } => edit::delete_forward(text, sel, word),
            Command::DeleteToLineStart => edit::delete_to_line_start(text, sel),
            Command::Indent => Some(edit::indent(text, sel, &options)),
            Command::IndentLines => Some(edit::indent_lines(text, sel, &options)),
            Command::Outdent => edit::outdent(text, sel, &options),
            Command::ToggleComment => edit::toggle_comment(text, sel, &options),
            Command::DeleteLine => edit::delete_lines(text, sel),
            &Command::MoveLines { down } => edit::move_lines(text, sel, down),
            &Command::CopyLines { down } => Some(edit::copy_lines(text, sel, down, &options)),
            &Command::ChangeCase(case) => transform::change_case(text, sel, case),
            &Command::SortLines { descending } => transform::sort_lines(text, sel, descending),
            Command::JoinLines => transform::join_lines(text, sel),
            Command::TrimTrailingWhitespace => transform::trim_trailing_whitespace(text, sel),
            _ => None,
        }
    }
}

/// 把外部文本中的换行统一为 `line_ending`；一致时直接借用原文。
fn normalize_newlines<'a>(s: &'a str, line_ending: &str) -> Cow<'a, str> {
    if !s.contains('\r') && line_ending == "\n" {
        return Cow::Borrowed(s);
    }

    let unified = s.replace("\r\n", "\n").replace('\r', "\n");

    Cow::Owned(if line_ending == "\n" {
        unified
    } else {
        unified.replace('\n', line_ending)
    })
}

/// 输入文字：单个括号或引号自动补全另一半，有选区时包住选区；右半边已在光标之后时直接跳过。
fn insert(text: &impl Lines, sel: Selection, s: &str, language: &Language) -> Transaction {
    let typing = sel.is_empty() && s.chars().count() == 1;
    let kind = if typing {
        EditKind::Typing
    } else {
        EditKind::Other
    };

    let mut chars = s.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return edit::replace_selection(text, sel, s, kind);
    };

    let next = text.char_at(sel.head);
    let prev = sel.from().checked_sub(1).and_then(|pos| text.char_at(pos));
    let wrapping = !sel.is_empty();

    if !wrapping && matches!(c, ')' | ']' | '}' | '"' | '\'' | '`') && next == Some(c) {
        return Transaction::new(
            Vec::new(),
            sel,
            Selection::point(sel.head + 1),
            EditKind::Other,
        );
    }

    let close = match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        // 单引号不是字符串定界符的语言（如 Rust），单引号多用于生命周期，不补全。
        '\'' if !language.single_quote_string => None,
        '`' if !language.backtick_string => None,
        // 紧跟在单词之后的引号多用作撇号（如 don't），不补全；紧跟在同种引号之后时通常是在
        // 输入连续引号（```、"""），同样不补全。
        '"' | '\'' | '`' => (wrapping || !follows_word_or_quote(prev, c)).then_some(c),
        _ if wrapping && language.surround.contains(&c) => Some(c),
        _ => None,
    };

    match close {
        Some(close) if wrapping || before_gap(next) => {
            let inner = text.substring(sel.from(), sel.to());
            let mut transaction =
                edit::replace_selection(text, sel, &format!("{c}{inner}{close}"), EditKind::Other);
            let start = sel.from() + 1;
            transaction.after = Selection::new(start, start + inner.chars().count());
            transaction
        }
        _ => edit::replace_selection(text, sel, s, kind),
    }
}

/// 位于字符 `prev` 之后的引号 `quote` 是否不作为字符串的起始引号。
///
/// 紧跟单词的引号是撇号（如 don't），紧跟同种引号的是代码块围栏或三引号字符串的一部分。
fn follows_word_or_quote(prev: Option<char>, quote: char) -> bool {
    prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == quote)
}

/// 光标后的字符 `next` 是否允许自动补全右侧字符：只有文档末尾、空白、右括号和分隔符允许。
fn before_gap(next: Option<char>) -> bool {
    next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';' | ':'))
}

/// 光标是否位于空的括号或引号对之间，此时退格同时删除左右两个字符。
///
/// 引号沿用自动补全的判断规则：`"a"|"b"` 中光标两侧的引号分属两个字符串，不构成引号对。
fn inside_empty_pair(text: &impl Lines, sel: Selection, language: &Language) -> bool {
    if !sel.is_empty() {
        return false;
    }
    let Some(open_at) = sel.head.checked_sub(1) else {
        return false;
    };
    let (Some(open), Some(close)) = (text.char_at(open_at), text.char_at(sel.head)) else {
        return false;
    };
    match (open, close) {
        ('(', ')') | ('[', ']') | ('{', '}') => true,
        ('"', '"') => quotes_pair(text, sel.head, open),
        ('\'', '\'') => language.single_quote_string && quotes_pair(text, sel.head, open),
        ('`', '`') => language.backtick_string && quotes_pair(text, sel.head, open),
        _ => false,
    }
}

/// 位置 `pos` 两侧的引号 `quote` 是否满足自动补全的条件。
fn quotes_pair(text: &impl Lines, pos: usize, quote: char) -> bool {
    let before = pos.checked_sub(2).and_then(|at| text.char_at(at));
    !follows_word_or_quote(before, quote) && before_gap(text.char_at(pos + 1))
}

#[cfg(test)]
mod tests;
