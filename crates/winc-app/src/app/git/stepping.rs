//! 在工作区文件的差异标签中依次查看提交视图列出的各个文件。

use super::*;

impl App {
    /// 标签 `id` 是否为工作区文件的 Git 差异。
    pub(in crate::app) fn steps_changed_files(&self, id: DocumentId) -> bool {
        self.git.diff_tabs.contains_key(&id)
    }

    /// 返回提交视图列出的下一个（`forward`）或上一个文件：（路径，序号（从 0 起），文件总数）。
    ///
    /// 当前标签不是工作区文件的差异，或没有更多文件时返回 `None`。从最近一次切换到的文件起算：
    /// 该文件无法打开时（二进制、没有差异）标签保留原内容，再次切换即可跳过该文件。
    pub(in crate::app) fn next_changed_file(
        &self,
        forward: bool,
    ) -> Option<(PathBuf, usize, usize)> {
        let id = self.editor.active()?.id;
        let tab = self.git.diff_tabs.get(&id)?;
        let current = match &self.git.stepped {
            Some((doc, path)) if *doc == id => path,
            _ => &tab.path,
        };
        // 列表中已折叠目录内的文件不在 `commit_files` 中，此时按状态中的顺序。
        let mut files = self.commit_files();
        if !files.contains(current) {
            let entries = &self.git.status.as_ref()?.entries;
            files = entries.iter().map(|e| e.path.clone()).collect();
        }
        let at = files.iter().position(|p| p == current)?;
        let next = if forward { at + 1 } else { at.checked_sub(1)? };
        let path = files.get(next)?.clone();
        Some((path, next, files.len()))
    }

    /// 在工作区文件的差异标签中打开提交视图列出的下一个（`forward`）或上一个文件，返回是否已打开。
    ///
    /// 新文件在当前标签中打开，连续查看时只占用一个标签。
    pub(in crate::app) fn step_changed_file(&mut self, forward: bool) -> bool {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return false;
        };
        let Some((path, next, total)) = self.next_changed_file(forward) else {
            return false;
        };
        let name = winc_editor::display_name(&path).into_owned();
        self.git.commit.preview = Some(id);
        self.git.stepped = Some((id, path.clone()));
        self.select_commit_file(path);
        let (index, name) = (next + 1, name.as_str());
        self.flash(if forward {
            t!("git-next-file", name = name, index = index, total = total)
        } else {
            t!(
                "git-previous-file",
                name = name,
                index = index,
                total = total
            )
        });
        true
    }
}
