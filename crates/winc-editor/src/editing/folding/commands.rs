//! 文档上的折叠：能折叠的块、折叠和展开的命令、光标避开折起来的行、按搜索结果过滤行。

use super::*;

impl Document {
    /// 是否可以折叠：大文件和无法显示文本的文件不能折叠。
    pub fn can_fold(&self) -> bool {
        !self.is_large() && self.shows_text()
    }

    /// 返回文档当前的折叠状态。
    pub fn folds(&self) -> &Folds {
        &self.folds
    }

    /// 返回第 `line` 行所在的显示行号（从 0 起）。
    pub fn row_of(&self, line: usize) -> usize {
        self.folds.row_of(line)
    }

    /// 返回第 `row` 个显示行对应的行号（从 0 起）；超出时返回最后一行。
    pub fn line_of_row(&self, row: usize) -> usize {
        self.folds
            .line_of_row(row)
            .min(self.text.line_count().saturating_sub(1))
    }

    /// 返回文档的显示行数；差异标签是对齐以后的行数，日志标签是提交数。
    pub fn row_count(&self) -> usize {
        if let Some(view) = &self.log {
            return view.row_count();
        }
        match &self.diff {
            // 末尾删掉一大段时左侧比右侧长，滚动范围取较长的一侧（见 `DiffView::scroll_rows`）。
            Some(view) => view.scroll_rows().max(1),
            None => self.folds.row_count(self.text.line_count()),
        }
    }

    /// 返回整个文件可折叠的块，按（版本号、Tab 宽度）缓存，连续执行多次折叠命令时不必重算；
    /// 返回的是缓存的共享引用，不复制整张列表。
    pub fn fold_ranges(&mut self, tab_width: usize) -> Rc<[FoldRange]> {
        let key = (self.revision, tab_width);
        if let Some((cached, ranges)) = &self.fold_cache
            && *cached == key
        {
            return Rc::clone(ranges);
        }
        // 先把高亮状态推进到文件末尾，才能知道哪些行在多行字符串里。
        let last = self.text.line_count().saturating_sub(1);
        self.highlight_state(last, usize::MAX);
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| crate::document::starts_quoted(highlighter, text, line);
        let ranges: Rc<[FoldRange]> =
            scopes::fold_ranges(text, language, tab_width, &mut quoted).into();
        self.fold_cache = Some((key, Rc::clone(&ranges)));
        ranges
    }

    /// 是否第 `line` 行是一个块的开头。
    ///
    /// 供绘制行号栏的箭头使用，只向下检查几行，不扫描整个文件。
    pub fn starts_block(&mut self, line: usize, tab_width: usize) -> bool {
        if !self.can_fold() || line >= self.text.line_count() {
            return false;
        }
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| crate::document::starts_quoted(highlighter, text, line);
        scopes::starts_block(text, language, line, tab_width, &mut quoted)
    }

    /// 返回 `lines`（升序）中是块的开头的行，结果与逐行调用 [`Self::starts_block`] 相同。
    ///
    /// 供一次询问屏幕上的所有行：Markdown 中省去让每个标题向上统计代码块围栏的开销（见
    /// [`crate::editing::scopes::block_starts`]）。
    pub fn block_starts(
        &mut self,
        lines: impl IntoIterator<Item = usize>,
        tab_width: usize,
    ) -> Vec<usize> {
        if !self.can_fold() {
            return Vec::new();
        }
        let count = self.text.line_count();
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| crate::document::starts_quoted(highlighter, text, line);
        let lines = lines.into_iter().take_while(|&line| line < count);
        scopes::block_starts(text, language, lines, tab_width, &mut quoted)
    }

    /// 执行折叠命令 `command`，返回折叠状态是否有变化。
    pub fn run_fold(&mut self, command: FoldCommand, tab_width: usize) -> bool {
        if !self.can_fold() {
            return false;
        }
        if command == FoldCommand::UnfoldAll {
            let changed = !self.folds.is_empty();
            self.folds.clear();
            return changed;
        }

        let ranges = self.fold_ranges(tab_width);
        let line = self.text.line_of(self.selection.head);
        let collapsed = |folds: &Folds, r: &FoldRange| folds.has(r.start);
        // 包住光标所在行的块，从内层到外层。
        let around: Vec<FoldRange> = ranges
            .iter()
            .rev()
            .filter(|r| r.start <= line && line <= r.end)
            .copied()
            .collect();
        let inside = |outer: FoldRange| -> Vec<FoldRange> {
            ranges
                .iter()
                .filter(|r| outer.start <= r.start && r.end <= outer.end)
                .copied()
                .collect()
        };

        let mut fold = Vec::new();
        let mut unfold = Vec::new();
        match command {
            FoldCommand::Fold => fold.extend(around.iter().find(|r| !collapsed(&self.folds, r))),
            FoldCommand::Unfold => unfold.extend(around.iter().find(|r| collapsed(&self.folds, r))),
            FoldCommand::Toggle => {
                if let Some(r) = around.first() {
                    if collapsed(&self.folds, r) {
                        unfold.push(*r);
                    } else {
                        fold.push(*r);
                    }
                }
            }
            FoldCommand::FoldRecursively => {
                if let Some(&r) = around.first() {
                    fold.extend(inside(r));
                }
            }
            FoldCommand::UnfoldRecursively => {
                if let Some(&r) = around.first() {
                    unfold.extend(inside(r));
                }
            }
            FoldCommand::FoldAll => fold.extend_from_slice(&ranges),
            FoldCommand::FoldLevel(level) => fold.extend(
                ranges
                    .iter()
                    .filter(|r| r.level == level && !(r.start <= line && line <= r.end)),
            ),
            FoldCommand::UnfoldAll => {}
        }

        // `unfold` 与 `ranges` 一样按开头行排序。
        let starts: Vec<usize> = unfold.iter().map(|r| r.start).collect();
        let removed = self
            .folds
            .remove(|f| starts.binary_search(&f.start).is_ok());
        let text = &self.text;
        let added = self
            .folds
            .insert(fold.iter().map(|r| Fold::new(text, r.start, r.end)));
        let changed = removed || added;
        if changed {
            self.keep_selection_visible();
        }
        changed
    }

    /// 处理点击行号栏的箭头：第 `line` 行已折叠则展开，是一个块的开头则折叠。
    ///
    /// 返回折叠状态是否有变化。
    pub fn toggle_fold_at(&mut self, line: usize, tab_width: usize) -> bool {
        if !self.can_fold() {
            return false;
        }
        if self.folds.remove(|f| f.start == line) {
            return true;
        }
        let Some(range) = self
            .fold_ranges(tab_width)
            .iter()
            .find(|r| r.start == line)
            .copied()
        else {
            return false;
        };
        self.folds
            .insert([Fold::new(&self.text, range.start, range.end)]);
        self.keep_selection_visible();
        true
    }

    /// 把第 `start + 1..=end` 行折叠到第 `start` 行下面；已有同样的块时不做改动。
    pub fn fold_lines(&mut self, start: usize, end: usize) {
        if self.folds.iter().any(|f| f.start == start && f.end == end) {
            return;
        }
        self.folds.remove(|f| f.start == start);
        self.folds.insert([Fold::new(&self.text, start, end)]);
        self.keep_selection_visible();
    }

    /// 返回已折叠的块（开头行，末尾行），按开头行排序。
    pub fn folded(&self) -> Vec<(usize, usize)> {
        self.folds.iter().map(|f| (f.start, f.end)).collect()
    }

    /// 折叠 `saved` 中仍与文档里可折叠的块一致的块；文件改动后对不上的不折叠。
    pub fn restore_folds(&mut self, saved: &[(usize, usize)], tab_width: usize) {
        if !self.can_fold() {
            return;
        }
        let ranges = self.fold_ranges(tab_width);
        let same = |&&(start, end): &&(usize, usize)| {
            (ranges.iter()).any(|range| (range.start, range.end) == (start, end))
        };
        let folds: Vec<Fold> = (saved.iter().filter(same))
            .map(|&(start, end)| Fold::new(&self.text, start, end))
            .collect();
        self.folds.insert(folds);
        self.keep_selection_visible();
    }

    /// 展开以第 `line` 行为开头的块（点击折叠行后面的 `⋯` 时调用），返回是否有块被展开。
    pub fn unfold_at(&mut self, line: usize) -> bool {
        self.folds.remove(|f| f.start == line)
    }

    /// 展开隐藏位置 `pos`（字符偏移）的块（查找、转到行跳到折叠的位置时调用），返回是否有块被展开。
    pub fn reveal(&mut self, pos: usize) -> bool {
        let line = self.text.line_of(pos.min(self.text.end_pos()));
        self.folds.remove(|f| f.start < line && line <= f.end)
    }

    /// 折叠的块里含有光标或选区的一端时，把这一端移到块开头那一行的行尾。
    fn keep_selection_visible(&mut self) {
        let visible = |pos: usize| {
            let line = self.text.line_of(pos);
            match self.folds.hidden_span(line) {
                Some((a, _)) if a > 0 => self.text.line_end(a - 1),
                _ => pos,
            }
        };
        self.selection =
            Selection::new(visible(self.selection.anchor), visible(self.selection.head));
    }

    /// 光标移进折叠的行时修正位置：向后移动则跳到块之后一行的开头，向前移动则停在开头那一行的行尾。
    ///
    /// `before` 是移动前的光标位置（字符偏移）。
    pub(crate) fn skip_hidden(&mut self, before: usize) {
        let head = self.selection.head;
        let Some((a, b)) = self.folds.hidden_span(self.text.line_of(head)) else {
            return;
        };
        let after = (b + 1 < self.text.line_count()).then(|| self.text.line_start(b + 1));
        let before_span = a.checked_sub(1).map(|line| self.text.line_end(line));
        // 向后移动，或前面没有显示着的行（筛选时第 0 行被隐藏）：移到后面那一行。
        let pos = match (after, before_span) {
            (Some(after), _) if head > before => after,
            (_, Some(end)) => end,
            (Some(after), None) => after,
            (None, None) => return,
        };
        self.selection = self.selection.moved_to(pos, !self.selection.is_empty());
    }

    /// 返回上下移动 `delta` 个显示行相当于移动的行数（负数表示向上）。
    pub(crate) fn line_delta(&self, delta: isize) -> isize {
        if self.folds.is_empty() {
            return delta;
        }
        let line = self.text.line_of(self.selection.head);
        let row = self.folds.row_of(line) as isize + delta;
        if row < 0 {
            return delta;
        }
        let rows = self.row_count() as isize;
        if row >= rows {
            return self.text.line_count() as isize;
        }
        self.folds.line_of_row(row as usize) as isize - line as isize
    }

    /// 文本修改后按新位置重新计算折叠块的行号，并重新排列被断开的行。
    pub(crate) fn refresh_folds(&mut self) {
        if let Some(wrap) = self.folds.wrap_mut() {
            wrap.revision = self.revision;
        }
        self.folds.refresh();
    }

    /// 末尾追加了一段（追加前的最后一行是第 `first` 行）：自动换行只重算追加的行。
    ///
    /// 否则日志每追加一行、标准输入每读到一段，下一帧都要把整个文件重新断行。
    pub(crate) fn appended(&mut self, first: usize) {
        let last = self.text.line_count() - 1;
        self.folds.edit_wrap(&self.text, first, first, last);
        self.refresh_folds();
    }

    /// 与 `folds` 交换折叠状态。
    ///
    /// 绘制折叠动画时临时换上展开状态的折叠，画完再换回来。
    pub fn swap_folds(&mut self, folds: &mut Folds) {
        std::mem::swap(&mut self.folds, folds);
    }

    /// 只显示 `shown` 列出的行（「只显示匹配的行」），`None` 表示都显示；返回是否应用了筛选。
    ///
    /// `searched` 是已搜索到的行号（从 0 起），其后尚未搜索的行仍然显示。不能折叠的文档（大文件等）
    /// 不能筛选，返回 `false`。
    pub fn filter_lines(&mut self, shown: Option<&[usize]>, searched: usize) -> bool {
        if !self.can_fold() {
            return false;
        }
        let line_count = self.text.line_count();
        self.folds.set_filter(shown, line_count, searched);
        let line = self.text.line_of(self.selection.head);
        if let Some((a, b)) = self.folds.hidden_span(line) {
            let shown = if b + 1 < line_count {
                b + 1
            } else {
                a.saturating_sub(1)
            };
            self.selection = Selection::point(self.text.line_start(shown));
        }
        true
    }

    /// 重新读取文件后原有的块不再对应，断行也要在下一次绘制之前重算。
    pub(crate) fn clear_folds(&mut self) {
        self.folds.wrap = None;
        self.folds.clear();
    }
}
