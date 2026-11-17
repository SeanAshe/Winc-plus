//! 文档的缩进和换行风格：取用哪一项、在状态栏里改、转换已有的缩进、保存前的清理。

use super::*;

use winc_i18n::t;

impl Document {
    /// 返回该文件的一级缩进。
    ///
    /// 优先级从高到低：状态栏中的选择、`.editorconfig`、从文件内容检测出的缩进、语言的默认值。
    pub fn indent(&self) -> Indent {
        if let Some(indent) = self.indent_override {
            return indent;
        }
        let found = self
            .detected_indent
            .unwrap_or_else(|| Indent::of_unit(self.highlighter.language().indent_unit));
        self.editorconfig.indent(found).unwrap_or(found)
    }

    /// 返回一个 Tab 的显示宽度（显示列）。
    ///
    /// 优先级从高到低：状态栏中的选择、`.editorconfig`、调用方给出的默认值。
    pub fn tab_width(&self, default: usize) -> usize {
        self.tab_width_override
            .or(self.editorconfig.tab_width())
            .unwrap_or(default)
    }

    /// 返回一级缩进的宽度（显示列）：空格缩进为空格数，Tab 缩进为一个 Tab 的显示宽度。
    pub fn indent_size(&self, default_tab_width: usize) -> usize {
        match self.indent() {
            Indent::Spaces(n) => n,
            Indent::Tabs => self.tab_width(default_tab_width),
        }
    }

    /// 改用空格缩进（`tabs` 为 `true` 时改用 Tab），宽度不变。
    ///
    /// 只影响之后输入的缩进，不改动已有文本。
    pub fn use_indent(&mut self, tabs: bool, default_tab_width: usize) {
        let size = self.indent_size(default_tab_width);
        if tabs {
            self.indent_override = Some(Indent::Tabs);
            self.tab_width_override = Some(size);
        } else {
            self.indent_override = Some(Indent::Spaces(size));
        }
    }

    /// 设置缩进的宽度（显示列）：空格缩进时为空格数（限制在 1 到 8），Tab 缩进时为 Tab 的
    /// 显示宽度（限制在 1 到 16）。
    ///
    /// 两个上限分别与缩进宽度菜单的取值范围、[`FileConfig::tab_width`] 的夹取范围相同：从状态栏
    /// 菜单选择的宽度不会被夹取成菜单之外的数值。
    pub fn set_indent_size(&mut self, size: usize) {
        match self.indent() {
            Indent::Spaces(_) => self.indent_override = Some(Indent::Spaces(size.clamp(1, 8))),
            Indent::Tabs => self.tab_width_override = Some(size.clamp(1, 16)),
        }
    }

    /// 清除状态栏中的缩进选择，重新从文件内容检测缩进。
    pub fn detect_indentation(&mut self) {
        self.indent_override = None;
        self.tab_width_override = None;
        self.detected_indent = indent::detect(&self.text);
    }

    /// 把已有的缩进全部换成空格（`to_tabs` 为 `true` 时换成制表符），作为一次可撤销的修改。
    ///
    /// 之后新建的缩进同样使用这一种。
    ///
    /// # Errors
    ///
    /// 文档只读或是大文件时返回原因（显示给用户的文字）。
    pub fn convert_indentation(
        &mut self,
        to_tabs: bool,
        default_tab_width: usize,
    ) -> Result<(), &'static str> {
        self.check_whole_file_edit()?;
        let size = self.indent_size(default_tab_width);
        if let Some(transaction) =
            transform::convert_indentation(&self.text, self.selection, to_tabs, size)
        {
            self.apply(transaction);
        }
        self.use_indent(to_tabs, default_tab_width);
        Ok(())
    }

    /// 把所有换行符换成 `ending`，并让之后插入的换行使用同一风格，作为一次可撤销的修改。
    ///
    /// # Errors
    ///
    /// 文档只读或是大文件时返回原因（显示给用户的文字）。
    pub fn change_line_ending(&mut self, ending: LineEnding) -> Result<(), &'static str> {
        self.check_whole_file_edit()?;
        if let Some(transaction) =
            transform::change_line_endings(&self.text, self.selection, ending.as_str())
        {
            self.apply(transaction);
        }
        self.line_ending = ending;
        Ok(())
    }

    /// 修改整个文件之前的检查：只读文档和大文件不允许修改。
    fn check_whole_file_edit(&self) -> Result<(), &'static str> {
        if let Some(reason) = self.readonly_reason() {
            return Err(reason);
        }
        if self.is_large() {
            return Err(t!("editor-large-whole-file"));
        }
        Ok(())
    }

    /// 撤销、重做之后按当前文本重新检测换行风格，使撤销换行风格转换后风格回到原值。
    ///
    /// 文本不含换行符时无法检测，保持原值；新文件的换行风格可能来自 `.editorconfig`。
    pub(crate) fn redetect_line_ending(&mut self) {
        if let Content::Rope(rope) = &self.text
            && rope.len_lines() > 1
        {
            self.line_ending = LineEnding::detect(rope);
        }
    }

    /// 保存前按 `.editorconfig` 清理文本，返回是否有改动。
    ///
    /// 清理包括删除行尾空白、统一换行符、末尾补换行，算一次修改，⌘Z 可以还原。`to` 是另存为
    /// 的新路径；`keep_caret` 为 `true` 时每个光标（选区）所在的行都不删除行尾空白，以免
    /// 自动保存时删掉用户正在输入的空格。多光标在清理后保留，撤销时也一并还原。
    pub fn tidy_for_save(&mut self, to: Option<&Path>, keep_caret: bool) -> bool {
        let Some(path) = to.or(self.path.as_deref()) else {
            return false;
        };
        let config = editorconfig::lookup(path);
        if to.is_none() {
            self.editorconfig = config.clone();
        }
        if !config.tidies() || self.readonly_reason().is_some() {
            return false;
        }
        let Content::Rope(rope) = &self.text else {
            return false;
        };

        // 每个光标（选区）所在的行都不删行尾空白；行范围合并成按顺序排列、互不重叠的几段。
        let (all, primary) = self.selections();
        let mut keep: Vec<Range<usize>> = Vec::new();
        if keep_caret {
            for selection in &all {
                let (first, last) = (rope.line_of(selection.from()), rope.line_of(selection.to()));
                match keep.last_mut() {
                    Some(lines) if first <= lines.end => lines.end = lines.end.max(last + 1),
                    _ => keep.push(first..last + 1),
                }
            }
        }
        let old = rope.to_string();
        let new = editorconfig::tidy(&old, &config, self.line_ending, &keep);
        if let Some(eol) = config.end_of_line {
            self.line_ending = eol;
        }
        if new == old {
            return false;
        }

        // 每个选区都按修改映射过去，撤销时也回到原来的各个选区，多光标不会因保存而丢失。
        let changes = diff::line_changes(&old, &new);
        let ends: Vec<usize> = all.iter().flat_map(|s| [s.anchor, s.head]).collect();
        let mapped = diff::map_positions(&changes, &ends)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|ends| Selection::new(ends[0], ends[1]))
            .collect();
        let (mapped, after) = merge(mapped, primary);
        let mut transaction =
            Transaction::new(changes, all[primary], mapped[after], EditKind::Other);
        transaction.extra_before = others(&all, primary);
        transaction.extra_after = others(&mapped, after);
        self.apply(transaction);
        true
    }
}
