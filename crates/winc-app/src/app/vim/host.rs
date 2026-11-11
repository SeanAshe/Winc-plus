//! 以文档为宿主：Vim 层在其上读取文本与视图、应用修改；需要应用配合的动作留到按键处理之后。

use std::ops::Range;

use winc_core::{Change, Indent, Selection};
use winc_editor::{Command, Content, Document, FoldCommand, ModalPattern};
use winc_vim::Scroll;
use winc_vim::{AppCommand, Fold, Host, HostOption, Indentation, Input, Message, Place, Query};

/// 宿主所需的视图参数，在借用文档之前算好。
pub(super) struct View {
    /// 制表符的宽度（显示列）。
    pub(super) tab_width: usize,
    /// 翻页时移动的行数。
    pub(super) page: usize,
    /// 窗口中完整显示的文本行。
    pub(super) visible: Range<usize>,
    /// 文字区一行可以容纳的显示列数。
    pub(super) text_width: usize,
    /// 文本区可以容纳的显示行数。
    pub(super) rows: usize,
    /// 纵向滚动位置的上限（显示行）。
    pub(super) max_scroll: f64,
    /// 一个显示列的宽度（逻辑像素）。
    pub(super) cell: f32,
    /// 光标与窗口上下边缘之间保留的行数。
    pub(super) scroll_off: usize,
    /// 是否显示行号。
    pub(super) numbers: bool,
    /// 是否显示全部的空白字符。
    pub(super) whitespace: bool,
    /// 是否显示相对行号。
    pub(super) relative: bool,
}

/// Vim 层请求、在按键处理结束之后由应用执行的动作。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Deferred {
    /// 格式化第 `first` 至 `last` 行。
    Format(usize, usize),
    /// 在位置历史中后退（真）或前进若干步。
    Jump(bool, usize),
    /// 下一次记录位置时无论远近都算作跳转。
    MarkJump,
    /// 滚动到某一行位于窗口的指定位置。
    Reveal(usize, Place),
    /// 执行折叠命令。
    Fold(FoldCommand),
    /// 把查找的模式设为当前高亮的查找内容。
    Highlight(Query),
    /// 设置由应用保存的选项。
    Set(HostOption, usize),
    /// 执行文件、标签与窗口的命令。
    App(AppCommand),
    /// 位置区间内的文本刚被复制。
    Yanked(usize, usize),
    /// 大写标记设在了当前文档的某个位置。
    GlobalMark(char, usize),
}

/// 系统剪贴板的访问；测试中以 `App::copied` 代替。
pub(super) struct Clip<'a> {
    #[cfg(not(test))]
    pub(super) system: &'a mut Option<arboard::Clipboard>,
    #[cfg(test)]
    pub(super) copied: &'a mut Option<String>,
    /// 最近一次整行复制的文本：应用的粘贴据此把整行插到当前行的上方。
    pub(super) whole_line: &'a mut Option<String>,
}

impl Clip<'_> {
    fn get(&mut self) -> Option<String> {
        #[cfg(test)]
        {
            self.copied.clone()
        }
        #[cfg(not(test))]
        {
            if self.system.is_none() {
                *self.system = arboard::Clipboard::new().ok();
            }
            self.system.as_mut()?.get_text().ok()
        }
    }

    fn set(&mut self, text: &str) {
        *self.whole_line = text.ends_with('\n').then(|| text.to_owned());
        #[cfg(test)]
        {
            *self.copied = Some(text.to_owned());
        }
        #[cfg(not(test))]
        {
            if self.system.is_none() {
                *self.system = arboard::Clipboard::new().ok();
            }
            if let Some(clipboard) = self.system.as_mut() {
                let _ = clipboard.set_text(text.to_owned());
            }
        }
    }
}

/// 以一个文档为宿主：Vim 层在其上读取文本、应用修改。
pub(super) struct DocHost<'a> {
    pub(super) doc: &'a mut Document,
    pub(super) view: View,
    pub(super) clip: Clip<'a>,
    /// 本次按键产生的提示，处理结束后显示。
    pub(super) messages: Vec<Message>,
    /// 最近一次查找的模式及其编译结果。
    pub(super) pattern: &'a mut Option<(Query, ModalPattern)>,
    /// 处理结束后由应用执行的动作。
    pub(super) deferred: Vec<Deferred>,
}

/// 返回文档 `doc` 在可以容纳 `rows` 个显示行的窗口中，光标可以停留的文本行。
///
/// 窗口的第一行与最后一行留作边距，与滚动到光标时保留的边距一致；窗口在文本开头时第一行
/// 不留。
pub(super) fn visible_lines(doc: &Document, rows: usize) -> Range<usize> {
    let first = doc.scroll_line.max(0.0).ceil() as usize;
    let (above, below) = if rows > 3 {
        (usize::from(first > 0), 1)
    } else {
        (0, 0)
    };
    let top = doc.line_of_row(first + above);
    let bottom = doc.line_of_row(first + rows.saturating_sub(1 + below));
    top..bottom.max(top) + 1
}

impl DocHost<'_> {
    /// 按文档当前的滚动位置重新计算窗口中显示的行。
    fn refresh_visible(&mut self) {
        self.view.visible = visible_lines(self.doc, self.view.rows);
    }

    /// 返回 `query` 的编译结果，与上一次的模式相同时沿用；无法编译时返回 `None`。
    fn compiled(&mut self, query: &Query) -> Option<&ModalPattern> {
        if (self.pattern.as_ref()).is_none_or(|(compiled, _)| compiled != query) {
            let pattern = ModalPattern::new(&query.regex, query.ignore_case).ok()?;
            *self.pattern = Some((query.clone(), pattern));
        }
        self.pattern.as_ref().map(|(_, pattern)| pattern)
    }
}

impl Host for DocHost<'_> {
    type Text = Content;

    fn text(&self) -> &Content {
        self.doc.text()
    }

    fn selection(&self) -> Selection {
        self.doc.selection
    }

    fn set_selection(&mut self, selection: Selection) {
        if self.doc.has_cursors() {
            self.doc.set_primary(selection);
        } else {
            self.doc.selection = selection;
        }
        self.doc.goal_cell = None;
    }

    fn selections(&self) -> Vec<Selection> {
        if self.doc.has_cursors() {
            self.doc.selections().0
        } else {
            vec![self.doc.selection]
        }
    }

    fn set_selections(&mut self, selections: Vec<Selection>, primary: usize) {
        match selections.as_slice() {
            [only] => {
                self.doc.collapse_selections();
                self.doc.selection = *only;
            }
            _ => self.doc.set_selections(selections, primary),
        }
        self.doc.goal_cell = None;
    }

    fn apply(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        self.doc.apply_changes(changes, before, after)
    }

    fn input(&mut self, input: &Input) {
        let command = match input {
            Input::Text(text) => Command::Insert(text.clone()),
            Input::Paste(text) => Command::Paste(text.clone()),
            Input::Newline => Command::Newline,
            Input::Backspace { word } => Command::Backspace { word: *word },
            Input::Delete { word } => Command::Delete { word: *word },
            Input::DeleteToLineStart => Command::DeleteToLineStart,
            Input::Indent => Command::Indent,
            Input::Outdent => Command::Outdent,
            Input::IndentLine => Command::IndentLines,
        };
        self.doc
            .execute(command, self.view.page, self.view.tab_width);
    }

    fn open_line(&mut self, at: usize) {
        self.doc.open_line_at(at, self.view.tab_width);
    }

    fn undo(&mut self) -> bool {
        let revision = self.doc.revision;
        (self.doc).execute(Command::Undo, self.view.page, self.view.tab_width);
        self.doc.revision != revision
    }

    fn redo(&mut self) -> bool {
        let revision = self.doc.revision;
        (self.doc).execute(Command::Redo, self.view.page, self.view.tab_width);
        self.doc.revision != revision
    }

    fn redo_origin(&self) -> Option<usize> {
        self.doc.redo_origin()
    }

    fn begin_group(&mut self) {
        self.doc.begin_undo_group();
    }

    fn end_group(&mut self) {
        self.doc.end_undo_group();
    }

    fn read_only(&self) -> bool {
        self.doc.readonly
    }

    fn fits(&self, from: usize, to: usize) -> bool {
        self.doc.fits_edit(from, to)
    }

    fn tab_width(&self) -> usize {
        self.view.tab_width.max(1)
    }

    fn indentation(&self) -> Indentation {
        match self.doc.indent() {
            Indent::Tabs => Indentation {
                width: self.view.tab_width.max(1),
                tabs: true,
            },
            Indent::Spaces(width) => Indentation {
                width: width.max(1),
                tabs: false,
            },
        }
    }

    fn keeps_indent(&self) -> bool {
        true
    }

    fn line_ending(&self) -> &str {
        self.doc.line_ending.as_str()
    }

    fn visible_lines(&self) -> Range<usize> {
        self.view.visible.clone()
    }

    fn window_height(&self) -> usize {
        self.view.rows.max(1)
    }

    fn text_width(&self) -> usize {
        self.view.text_width
    }

    fn display_row(&self, pos: usize) -> Range<usize> {
        self.doc.display_row_span(pos)
    }

    fn display_vertical(&self, pos: usize, delta: isize, _cell: usize) -> Option<usize> {
        self.doc.display_vertical(pos, delta)
    }

    fn find(&mut self, query: &Query, from: usize, forward: bool) -> Option<(usize, usize)> {
        self.compiled(query)?;
        let highlight = Deferred::Highlight(query.clone());
        if !self.deferred.contains(&highlight) {
            self.deferred.push(highlight);
        }
        let (_, pattern) = self.pattern.as_ref()?;
        self.doc.find_modal(pattern, from, forward)
    }

    fn mark(&self, name: char) -> Option<usize> {
        self.doc.modal_mark(name)
    }

    fn set_mark(&mut self, name: char, pos: usize) {
        self.doc.set_modal_mark(name, pos);
        // 大写标记同时记录路径与位置。
        if name.is_ascii_uppercase() {
            self.deferred.push(Deferred::GlobalMark(name, pos));
        }
    }

    fn changes(&self) -> Vec<usize> {
        self.doc.modal_changes().to_vec()
    }

    fn record_change(&mut self, pos: usize) {
        self.doc.record_modal_change(pos);
    }

    fn record_jump(&mut self, _pos: usize) {
        if !self.deferred.contains(&Deferred::MarkJump) {
            self.deferred.push(Deferred::MarkJump);
        }
    }

    fn jump(&mut self, back: bool, count: usize) -> bool {
        // 目标可以在另一个文档中：按键处理结束之后由应用执行。
        self.deferred.push(Deferred::Jump(back, count));
        false
    }

    fn scroll(&mut self, scroll: Scroll) -> bool {
        match scroll {
            Scroll::Lines(lines) => {
                let before = self.doc.scroll_line;
                let target = (before + lines as f64).clamp(0.0, self.view.max_scroll.max(0.0));
                self.doc.scroll_line = target;
                self.refresh_visible();
                target != before
            }
            Scroll::Reveal { line, place } => {
                self.deferred.push(Deferred::Reveal(line, place));
                true
            }
            Scroll::Columns(columns) => {
                if self.doc.is_wrapping() {
                    return false;
                }
                let before = self.doc.scroll_x;
                self.doc.scroll_x = (before + columns as f32 * self.view.cell).max(0.0);
                self.doc.scroll_x != before
            }
        }
    }

    fn visible_cells(&self) -> Option<Range<usize>> {
        if self.doc.is_wrapping() {
            return None;
        }
        let first = (self.doc.scroll_x / self.view.cell.max(1.0)).ceil() as usize;
        Some(first..first + self.view.text_width)
    }

    fn scroll_margin(&self) -> usize {
        self.view.scroll_off
    }

    fn fold_at(&mut self, line: usize, fold: Fold) -> bool {
        let collapsed = self.doc.collapsed_fold_at(line).is_some();
        let command = match fold {
            Fold::Open => FoldCommand::Unfold,
            Fold::Close => FoldCommand::Fold,
            Fold::Toggle => FoldCommand::Toggle,
            Fold::OpenTree => FoldCommand::UnfoldRecursively,
            Fold::CloseTree => FoldCommand::FoldRecursively,
            Fold::ToggleTree if collapsed => FoldCommand::UnfoldRecursively,
            Fold::ToggleTree => FoldCommand::FoldRecursively,
            Fold::OpenAll => FoldCommand::UnfoldAll,
            Fold::CloseAll => FoldCommand::FoldAll,
            Fold::Reveal => {
                let pos = self.doc.selection.head;
                return self.doc.reveal(pos);
            }
        };
        // 折叠的变化连同滚动位置的保持与动画由应用执行。
        self.deferred.push(Deferred::Fold(command));
        false
    }

    fn fold_edge(&mut self, line: usize, forward: bool) -> Option<usize> {
        self.doc.fold_edge(line, forward, self.view.tab_width)
    }

    fn format_lines(&mut self, first: usize, last: usize) -> bool {
        self.deferred.push(Deferred::Format(first, last));
        true
    }

    fn has_folds(&self) -> bool {
        self.doc.has_collapsed_folds()
    }

    fn fold(&self, line: usize) -> Option<(usize, usize)> {
        self.doc.collapsed_fold_at(line)
    }

    fn clipboard(&mut self) -> Option<String> {
        self.clip.get()
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clip.set(text);
    }

    fn toggle_comment(&mut self, first: usize, last: usize) -> bool {
        (self.doc).toggle_comment_lines(first, last, self.view.tab_width)
    }

    fn file_name(&self) -> Option<String> {
        self.doc.path.as_ref().map(|_| self.doc.name().into_owned())
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }

    fn find_groups(&mut self, query: &Query, from: usize) -> Option<Vec<Option<(usize, usize)>>> {
        self.compiled(query)?;
        let (_, pattern) = self.pattern.as_ref()?;
        self.doc.match_modal(pattern, from)
    }

    fn track(&mut self, positions: Vec<usize>) {
        self.doc.track_modal(positions);
    }

    fn next_tracked(&mut self) -> Option<usize> {
        self.doc.next_tracked_modal()
    }

    fn option(&self, option: HostOption) -> Option<usize> {
        Some(match option {
            HostOption::Number => usize::from(self.view.numbers),
            HostOption::RelativeNumber => usize::from(self.view.relative),
            HostOption::Wrap => usize::from(self.doc.is_wrapping()),
            HostOption::List => usize::from(self.view.whitespace),
            HostOption::TabStop => self.view.tab_width.max(1),
            HostOption::ShiftWidth => self.indentation().width,
            HostOption::ExpandTab => usize::from(!self.indentation().tabs),
        })
    }

    fn set_option(&mut self, option: HostOption, value: usize) -> bool {
        let valid = match option {
            HostOption::TabStop | HostOption::ShiftWidth => (1..=16).contains(&value),
            _ => true,
        };
        if valid {
            self.deferred.push(Deferred::Set(option, value));
        }
        valid
    }

    fn yanked(&mut self, from: usize, to: usize) {
        self.deferred.push(Deferred::Yanked(from, to));
    }

    fn app(&mut self, command: AppCommand) -> bool {
        self.deferred.push(Deferred::App(command));
        true
    }
}
