//! 同一个目录换了写法（例如改为经符号链接打开）时，把它的现场和最近列表中的记录改到新写法下。

use std::path::{Path, PathBuf};

use super::{Session, State, Tab};

/// `path` 是 `from` 或在其下时，改为 `to` 下的同一位置。
fn rebase(path: &mut PathBuf, from: &Path, to: &Path) {
    if let Ok(rest) = path.strip_prefix(from) {
        *path = match rest.as_os_str().is_empty() {
            true => to.to_path_buf(),
            false => to.join(rest),
        };
    }
}

impl Session {
    /// 把现场中位于 `from` 下的路径（目录本身、标签、书签、附加的目录、仓库等）改到 `to` 下。
    fn rebase(&mut self, from: &Path, to: &Path) {
        let mut paths: Vec<&mut PathBuf> = vec![&mut self.folder];
        paths.extend(self.tabs.iter_mut().filter_map(|tab| match tab {
            Tab::File(path) => Some(path),
            Tab::Untitled(_) => None,
        }));
        paths.extend(self.bookmarks.iter_mut().map(|(path, _)| path));
        paths.extend(self.outline.iter_mut().map(|(path, _)| path));
        paths.extend(self.folds.iter_mut().map(|(path, _)| path));
        // 终端标签的空路径表示打开的目录，不必改。
        paths.extend(self.terminal.iter_mut().flat_map(|t| t.tabs.iter_mut()));
        paths.extend(self.folders.iter_mut());
        paths.extend(self.project.iter_mut());
        paths.extend(self.readonly.iter_mut());
        paths.extend(self.repositories.added.iter_mut());
        paths.extend(self.repositories.removed.iter_mut());
        for path in paths {
            rebase(path, from, to);
        }
    }
}

impl State {
    /// 把目录 `from` 的记录改到同一目录的另一种写法 `to` 下：现场（`to` 还没有现场时）、最近
    /// 列表中的位置和徽章颜色、上次退出时的目录。
    ///
    /// `from` 没有记录时不做改动。
    pub fn rename_folder(&mut self, from: &Path, to: &Path) {
        if self.session_for(to).is_none()
            && let Some(session) = self.sessions.iter_mut().find(|s| s.folder == from)
        {
            session.rebase(from, to);
        }
        if let Some(at) = self.recent.iter().position(|p| p == from) {
            match self.recent.iter().any(|p| p == to) {
                true => _ = self.recent.remove(at),
                false => self.recent[at] = to.to_path_buf(),
            }
        }
        if !self.badges.iter().any(|(p, _)| p == to)
            && let Some((path, _)) = self.badges.iter_mut().find(|(p, _)| p == from)
        {
            *path = to.to_path_buf();
        }
        if self.last.as_deref() == Some(from) {
            self.last = Some(to.to_path_buf());
        }
        // 两种写法都在列表中时去掉了 `from`：它的徽章颜色随之释放。
        self.assign_badges();
    }
}
