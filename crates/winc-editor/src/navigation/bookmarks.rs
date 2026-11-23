//! 书签：标记文档中的若干行，并在这些行之间来回跳转。
//!
//! 书签按文件保存：关闭标签、退出程序后再次打开该文件，书签仍然存在。一个工作区目录的书签
//! 随该目录的会话状态保存，未打开文件的书签保存在 [`crate::Editor::bookmarks`]。

use std::collections::BTreeMap;
use std::path::PathBuf;

use winc_core::Lines;

use crate::document::Document;

/// 一个文档的书签集合：行号从小到大排列且不重复，编辑文本时随之移动。
///
/// 行号从 0 起，与文档的行号相同。编辑落在书签所在行内时，书签保留在该行；该行被整行删除时，
/// 书签一并删除。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bookmarks {
    lines: Vec<usize>,
}

impl Bookmarks {
    /// 是否没有任何书签。
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// 返回书签所在的行号，从小到大排列。
    pub fn lines(&self) -> &[usize] {
        &self.lines
    }

    /// 第 `line` 行（从 0 起）是否有书签。
    pub fn contains(&self, line: usize) -> bool {
        self.lines.binary_search(&line).is_ok()
    }

    /// 在第 `line` 行（从 0 起）切换书签：没有则添加，已有则移除，返回切换后该行是否有书签。
    pub fn toggle(&mut self, line: usize) -> bool {
        match self.lines.binary_search(&line) {
            Ok(at) => {
                self.lines.remove(at);
                false
            }
            Err(at) => {
                self.lines.insert(at, line);
                true
            }
        }
    }

    /// 清空全部书签。
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// 返回 `line` 之后的第一个书签的行号；`line` 之后没有书签时回绕到第一个书签，没有任何
    /// 书签时返回 `None`。
    pub fn next(&self, line: usize) -> Option<usize> {
        self.lines
            .iter()
            .copied()
            .find(|&l| l > line)
            .or_else(|| self.lines.first().copied())
    }

    /// 返回 `line` 之前的最后一个书签的行号；`line` 之前没有书签时回绕到最后一个书签，没有
    /// 任何书签时返回 `None`。
    pub fn previous(&self, line: usize) -> Option<usize> {
        self.lines
            .iter()
            .copied()
            .rev()
            .find(|&l| l < line)
            .or_else(|| self.lines.last().copied())
    }

    /// 文本编辑后调整书签的行号。
    ///
    /// 被删除的行上的书签一并删除，编辑点之后的书签整体上移或下移，编辑落在其上的行保留书签。
    /// 行号均取编辑之前的值，从 0 起：`start_line`、`end_line` 是被替换范围的起止行，`delta`
    /// 是文档总行数的变化，`whole_lines` 表示删除范围从行首到下一行行首（例如 ⇧⌘K 删除行）。
    pub(crate) fn edit(
        &mut self,
        start_line: usize,
        end_line: usize,
        delta: isize,
        whole_lines: bool,
    ) {
        if self.lines.is_empty() {
            return;
        }
        // `first..=last` 是被删除的行。通常从 `start_line` 的下一行起；整行删除时从 `start_line`
        // 起，因为 `start_line` 整行消失。
        let first = if whole_lines {
            start_line
        } else {
            start_line + 1
        };
        // 整行删除时 `end_line` 是范围之后的那一行（范围止于行首），该行本身仍然存在，其上的
        // 书签必须保留，因此 `last` 取 `end_line - 1`。
        let last = if whole_lines {
            end_line.saturating_sub(1)
        } else {
            end_line
        };
        self.lines.retain(|&line| line < first || line > last);
        for line in &mut self.lines {
            if *line > last {
                *line = line.saturating_add_signed(delta);
            }
        }
        self.lines.dedup();
    }

    /// 由一组行号创建书签集合，行号排序并去重。
    pub fn from_lines(mut lines: Vec<usize>) -> Bookmarks {
        lines.sort_unstable();
        lines.dedup();
        Bookmarks { lines }
    }

    /// 移除行号不小于 `line_count` 的书签，用于文档行数减少之后。
    pub fn clamp(&mut self, line_count: usize) {
        self.lines.retain(|&l| l < line_count);
    }
}

impl Document {
    /// 返回文档的书签集合。
    pub fn bookmarks(&self) -> &Bookmarks {
        &self.bookmarks
    }

    /// 切换光标所在行的书签：没有则添加，已有则移除，返回切换后该行是否有书签。
    pub fn toggle_bookmark(&mut self) -> bool {
        let line = self.text.line_of(self.selection.head);
        self.bookmarks.toggle(line)
    }

    /// 移除文档的所有书签。
    pub fn clear_bookmarks(&mut self) {
        self.bookmarks.clear();
    }

    /// 用 `marks` 替换文档的书签（打开文件时恢复），行号超出文档行数的书签被丢弃。
    pub fn set_bookmarks(&mut self, mut marks: Bookmarks) {
        marks.clamp(self.text.line_count());
        self.bookmarks = marks;
    }

    /// 返回有书签的行号（从 0 起），从小到大排列。
    pub fn bookmark_lines(&self) -> &[usize] {
        self.bookmarks.lines()
    }

    /// 返回第 `line` 行（从 0 起）用于书签列表的预览文本。
    ///
    /// 预览去掉行首空白，最多取 160 个字符（足够填满列表宽度），被截断时末尾加 `…`。
    /// `line` 超出文档行数时返回空字符串。
    pub fn bookmark_preview(&self, line: usize) -> String {
        if line >= self.text.line_count() {
            return String::new();
        }
        // 只读取预览所需的开头部分：单行可能很长（例如压缩后的文件），不先复制整行再截断。
        let content = self.text.line_content(line);
        let mut chars = content.slice().chars().skip_while(|c| c.is_whitespace());
        let mut preview: String = chars.by_ref().take(160).collect();
        if chars.next().is_some() {
            preview.push('…');
        }
        preview
    }
}

/// 各文件的书签；键为文件的规范路径。
pub type BookmarkMap = BTreeMap<PathBuf, Bookmarks>;

#[cfg(test)]
mod tests;
