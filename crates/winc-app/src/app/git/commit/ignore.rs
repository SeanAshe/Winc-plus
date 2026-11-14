//! 把未跟踪的文件加入 `.gitignore`，以及储藏选中的文件。

use winc_editor::git::Change;

use super::*;

/// 加入 `.gitignore` 的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IgnoreKind {
    /// 这一个文件（或这一个未跟踪的目录）。
    File,
    /// 与该文件扩展名相同的全部文件。
    Extension,
    /// 该文件所在的目录。
    Folder,
}

impl IgnoreKind {
    /// 返回菜单项上的文字。
    pub fn label(self) -> &'static str {
        match self {
            IgnoreKind::File => t!("menu-git-ignore-file"),
            IgnoreKind::Extension => t!("menu-git-ignore-extension"),
            IgnoreKind::Folder => t!("menu-git-ignore-folder"),
        }
    }
}

impl App {
    /// 返回文件 `path` 可以加入 `.gitignore` 的各种范围；不是未跟踪的文件时为空。
    ///
    /// 没有扩展名的文件没有「此扩展名」，仓库根目录下的文件没有「所在目录」；未跟踪的目录只能
    /// 整个忽略。
    pub(in crate::app) fn ignore_kinds(&self, path: &Path) -> Vec<IgnoreKind> {
        let mut entries = self.git.status.iter().flat_map(|s| &s.entries);
        let (Some(entry), Some(root)) = (entries.find(|e| e.path == path), &self.git.root) else {
            return Vec::new();
        };
        if !entry.untracked() {
            return Vec::new();
        }
        let mut kinds = vec![IgnoreKind::File];
        if !entry.is_dir {
            if path.extension().is_some() {
                kinds.push(IgnoreKind::Extension);
            }
            if path.parent().is_some_and(|dir| dir != root) {
                kinds.push(IgnoreKind::Folder);
            }
        }
        kinds
    }

    /// 把文件 `path` 按范围 `kind` 写入仓库根目录的 `.gitignore`，然后重新读取状态。
    ///
    /// 已有相同的一行时不重复写入。写入失败时提示原因。
    pub(in crate::app) fn git_ignore(&mut self, path: &Path, kind: IgnoreKind) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&root) else {
            return;
        };
        let is_dir =
            (self.git.status.iter().flat_map(|s| &s.entries)).any(|e| e.path == path && e.is_dir);
        let Some(line) = ignore_line(relative, is_dir, kind) else {
            return;
        };
        let file = root.join(".gitignore");
        let mut text = winc_backend::fs::read_to_string(&file).unwrap_or_default();
        if !text.lines().any(|existing| existing == line) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&line);
            text.push('\n');
            if let Err(e) = winc_backend::fs::write(&file, text) {
                let error = e.to_string();
                self.notify(t!("git-ignore-failed", error = error));
                return;
            }
        }
        self.flash(t!("git-ignore-done", pattern = line));
        self.check_disk();
    }

    /// 打开储藏的输入条，储藏 `paths` 中的文件（连同改名文件原来的路径）；说明可以为空。
    pub(in crate::app) fn ask_stash_files(&mut self, paths: Vec<PathBuf>) {
        let entries = self.git.status.iter().flat_map(|s| &s.entries);
        let mut all = Vec::new();
        for entry in entries.filter(|e| paths.contains(&e.path)) {
            all.push(entry.path.clone());
            // 新加入暂存区的文件没有原来的路径。
            all.extend(
                entry
                    .from
                    .clone()
                    .filter(|_| entry.staged != Some(Change::Added)),
            );
        }
        if all.is_empty() {
            return;
        }
        let field = crate::field::TextField::default();
        self.open_prompt(crate::app::PromptKind::StashFiles(all), field);
    }

    /// 储藏 `paths` 中的文件（含未跟踪的文件），其余的更改留在工作区；说明是 `message`。
    pub(in crate::app) fn git_stash_files(&mut self, message: &str, paths: Vec<PathBuf>) {
        let message = message.trim().to_owned();
        self.git_operate(t!("git-op-stash").into(), move |git, root| {
            git.stash_paths(root, &message, &paths)
        });
    }
}

/// 返回把仓库内相对路径 `relative` 按范围 `kind` 写入 `.gitignore` 的一行；无法表示时为 `None`。
///
/// 文件和目录的路径以 `/` 开头，只匹配这一个位置；目录以 `/` 结尾。路径中在 `.gitignore` 里有
/// 特殊含义的字符前加反斜杠，按字面匹配。
fn ignore_line(relative: &Path, is_dir: bool, kind: IgnoreKind) -> Option<String> {
    let slashed = |path: &Path| -> Option<String> {
        let parts = path
            .components()
            .map(|c| c.as_os_str().to_str().map(escaped));
        Some(parts.collect::<Option<Vec<_>>>()?.join("/"))
    };
    match kind {
        IgnoreKind::File if is_dir => Some(format!("/{}/", slashed(relative)?)),
        IgnoreKind::File => Some(format!("/{}", slashed(relative)?)),
        IgnoreKind::Extension => {
            let extension = relative.extension()?.to_str()?;
            Some(format!("*.{}", escaped(extension)))
        }
        IgnoreKind::Folder => {
            let dir = relative.parent().filter(|d| !d.as_os_str().is_empty())?;
            Some(format!("/{}/", slashed(dir)?))
        }
    }
}

/// 返回 `name` 在 `.gitignore` 中按字面匹配的写法：通配符、反斜杠和末尾的空格前加反斜杠。
fn escaped(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let last = name.len().saturating_sub(1);
    for (i, c) in name.char_indices() {
        let special = matches!(c, '\\' | '*' | '?' | '[' | ']') || (c == ' ' && i == last);
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_anchor_paths_and_escape_wildcards() {
        let line = |path: &str, is_dir, kind| ignore_line(Path::new(path), is_dir, kind);
        let file = |path: &str, kind| line(path, false, kind);
        assert_eq!(file("src/a b.rs", IgnoreKind::File).unwrap(), "/src/a b.rs");
        assert_eq!(file("src/a.rs", IgnoreKind::Extension).unwrap(), "*.rs");
        assert_eq!(file("src/x/a.rs", IgnoreKind::Folder).unwrap(), "/src/x/");
        assert_eq!(line("out", true, IgnoreKind::File).unwrap(), "/out/");
        // 根目录下的文件没有所在目录，没有扩展名的文件没有扩展名的写法。
        assert_eq!(file("a.rs", IgnoreKind::Folder), None);
        assert_eq!(file("Makefile", IgnoreKind::Extension), None);
        // 通配符按字面匹配；末尾的空格不被忽略。
        assert_eq!(file("[x]*.md", IgnoreKind::File).unwrap(), "/\\[x\\]\\*.md");
        assert_eq!(file("note ", IgnoreKind::File).unwrap(), "/note\\ ");
    }
}
