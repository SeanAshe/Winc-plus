//! 写提交信息时的辅助：提交过的信息、首行的格式提示、提交前检查新增的 TODO、共同作者。
//!
//! 提交过的信息记在状态文件里，提交信息框里按 `↑` 翻阅，提交按钮的菜单里列出最近的几条。
//! 首行的格式提示不阻止提交，写在提交按钮一行的右侧。共同作者从仓库最近的提交中读出，选中后在
//! 信息末尾加一行 `Co-authored-by`。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuLabel};

/// 首行的建议长度（显示列）：`git` 的惯例，也是常见编辑器提交信息检查的默认值。按终端里占的
/// 列数计，汉字等全角字符占两列：这条惯例是为了在终端里一行写得下。
const SUBJECT_LIMIT: usize = 72;
/// 提交按钮的菜单里列出的最近的提交信息条数：子菜单一屏放得下。
const MENU_MESSAGES: usize = 10;
/// 提交按钮的菜单里列出的共同作者人数，理由同 [`MENU_MESSAGES`]。
const MENU_AUTHORS: usize = 10;
/// 读取共同作者时查看的最近提交数：覆盖近期参与的人，不在大仓库里读完整个历史。
pub(crate) const AUTHOR_COMMITS: usize = 1000;
/// 从仓库里读出的自己写的提交信息的条数：与本程序记住的条数上限相同。
pub(crate) const OWN_MESSAGES: usize = winc_config::MAX_COMMIT_MESSAGES;
/// 共同作者一行的写法，`git` 和各代码托管平台都认这个名字。
const CO_AUTHOR: &str = "Co-authored-by: ";

impl CommitState {
    /// 可以翻阅的提交信息变了（读到了仓库里自己写的提交信息）：下次按 `↑` 时重新放进输入历史。
    pub(in crate::app) fn message_history_changed(&mut self) {
        self.history_seeded = false;
    }
}

impl App {
    /// 可以翻阅的提交信息，最近的在前：本程序提交时记住的，后面补上当前仓库里自己写的（在终端
    /// 等别处提交的也在其中），不重复。
    fn recent_messages(&self) -> Vec<&str> {
        let remembered = self.state.commit_messages.iter();
        let mut messages: Vec<&str> = Vec::new();
        for message in remembered.chain(&self.git.own_messages) {
            if !messages.contains(&message.as_str()) {
                messages.push(message);
            }
        }
        messages
    }

    /// 把可以翻阅的提交信息（[`App::recent_messages`]）放进提交信息框的输入历史，`↑` 翻阅时能
    /// 翻到上次运行时和在别处的提交。放过一次后，翻阅的来源变了才重新放。
    pub(super) fn seed_message_history(&mut self) {
        if std::mem::replace(&mut self.git.commit.history_seeded, true) {
            return;
        }
        let messages: Vec<String> = self
            .recent_messages()
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.git
            .commit
            .message
            .set_history(messages.into_iter().rev());
    }

    /// 提交成功后记下这次的信息：所有窗口共用，下次启动沿用。
    pub(super) fn remember_commit_message(&mut self, message: &str) {
        self.update_state(|state| state.remember_commit_message(message));
    }

    /// 首行的格式提示：首行过长（见 [`SUBJECT_LIMIT`]），或者首行后面紧接着正文、没有空一行；
    /// 都没有问题时为 `None`。
    pub(crate) fn commit_message_hint(&self) -> Option<String> {
        let message = self.git.commit.message.text();
        let mut lines = message.lines();
        let subject = lines.next()?;
        let mut length = 0;
        for c in subject.chars() {
            length += winc_core::text::char_cells(c, length, 4);
        }
        if length > SUBJECT_LIMIT {
            let (length, limit) = (length.to_string(), SUBJECT_LIMIT.to_string());
            return Some(t!("git-commit-hint-subject", count = length, limit = limit));
        }
        let second = lines.next().filter(|line| !line.trim().is_empty());
        second.map(|_| t!("git-commit-hint-blank-line").to_owned())
    }

    /// 提交前检查新增的 TODO：开着检查、这次还没检查过时在后台检查，返回 `true`，提交等结果
    /// 出来再继续（[`App::todo_checked`]）。检查过一次后放行这一次提交。
    pub(super) fn check_todo_first(&mut self) -> bool {
        if std::mem::take(&mut self.git.commit.todo_passed) || !self.state.commit_check_todo {
            return false;
        }
        let Some(status) = self.git.status.as_ref() else {
            return false;
        };
        let staged = self.git.commit.staging();
        let (mut tracked, mut untracked) = (Vec::new(), Vec::new());
        if !staged {
            for entry in self.git.commit.picked_entries(status) {
                match entry.untracked() {
                    true => untracked.push(entry.path.clone()),
                    false => tracked.push(entry.path.clone()),
                }
            }
        }
        // 当前仓库，然后是一并提交的其他仓库：各自取新增的行。
        let current = TodoScope {
            repo: None,
            staged,
            tracked,
            untracked,
        };
        let mut scopes = vec![current];
        scopes.extend(self.other_todo_scopes());
        let empty = |s: &TodoScope| !s.staged && s.tracked.is_empty() && s.untracked.is_empty();
        if scopes.iter().all(empty) {
            return false;
        }
        let sender = self.git.commit.results.0.clone();
        self.git.commit.busy = true;
        self.run_git_job(move |git, root| {
            let mut lines = Vec::new();
            for scope in &scopes {
                let repo = scope.repo.as_deref().unwrap_or(root);
                if scope.staged || !scope.tracked.is_empty() {
                    let added = (git.as_ref().ok())
                        .and_then(|git| git.added_lines(repo, scope.staged, &scope.tracked).ok());
                    lines.extend(added.unwrap_or_default());
                }
                lines.extend(
                    scope
                        .untracked
                        .iter()
                        .filter_map(|p| text_lines(p))
                        .flatten(),
                );
            }
            let count = lines.iter().filter(|line| has_todo(line)).count();
            let _ = sender.send(Outcome::TodoChecked(count, root.to_path_buf()));
        });
        true
    }

    /// 收到提交前检查的结果：没有新增的 TODO 时接着提交；有时先确认。
    pub(super) fn todo_checked(&mut self, count: usize, root: &Path) {
        self.git.commit.busy = false;
        if self.git.root.as_deref() != Some(root) {
            self.git.commit.push_after = false;
            return;
        }
        if count == 0 {
            self.git.commit.todo_passed = true;
            self.git_commit();
            return;
        }
        let message = t!("git-commit-todo-confirm", count = count);
        let detail = t!("git-commit-todo-confirm-detail");
        let button = t!("git-commit-todo-confirm-button");
        let shown = self.show_confirm(&message, detail, button, |ok| {
            Some(UserEvent::GitCommitTodo(ok))
        });
        if !shown {
            self.git.commit.push_after = false;
        }
    }

    /// 新增 TODO 的确认框选了 `ok`（仍然提交）或取消。
    pub(in crate::app) fn todo_confirmed(&mut self, ok: bool) {
        if ok {
            self.git.commit.todo_passed = true;
            self.git_commit();
        } else {
            self.git.commit.push_after = false;
        }
    }

    /// 打开或关闭提交前检查新增的 TODO（提交按钮的菜单）。
    pub(super) fn toggle_check_todo(&mut self) {
        self.update_state(|state| state.commit_check_todo = !state.commit_check_todo);
    }

    /// 提交按钮菜单的最后几项：「提交检查」子菜单（格式化代码、检查 TODO 两个开关），以及最近的
    /// 提交信息、共同作者两个子菜单（有内容时）。
    pub(super) fn compose_menu_entries(&self) -> Vec<MenuEntry> {
        // 提交前的检查放在一个子菜单里逐项勾选，与签署、跳过钩子分开。
        let checks = vec![
            MenuEntry::Item(MenuCommand::CommitReformat, true),
            MenuEntry::Item(MenuCommand::CommitCheckTodo, true),
        ];
        let mut entries = vec![MenuEntry::Submenu(t!("menu-commit-checks").into(), checks)];
        let messages = self.recent_messages().len().min(MENU_MESSAGES);
        let authors = self.git.authors.len().min(MENU_AUTHORS);
        if messages + authors > 0 {
            entries.push(MenuEntry::Separator);
        }
        if messages > 0 {
            let items = (0..messages).map(|i| MenuEntry::Item(MenuCommand::CommitMessage(i), true));
            let title = t!("menu-commit-recent-messages").into();
            entries.push(MenuEntry::Submenu(title, items.collect()));
        }
        if authors > 0 {
            let items = (0..authors).map(|i| MenuEntry::Item(MenuCommand::CommitCoAuthor(i), true));
            let title = t!("menu-commit-co-author").into();
            entries.push(MenuEntry::Submenu(title, items.collect()));
        }
        entries
    }

    /// 最近的提交信息、共同作者两个子菜单里各项的文字：信息写首行，后面还有正文时写在下一行小字
    /// 里；作者写名字，邮箱写在下一行小字里。
    pub(in crate::app) fn compose_menu_label(&self, command: MenuCommand) -> Option<MenuLabel> {
        match command {
            MenuCommand::CommitMessage(i) => {
                let message = *self.recent_messages().get(i)?;
                let mut lines = message.lines();
                let subject = lines.next().unwrap_or_default();
                let body = lines.find(|line| !line.trim().is_empty());
                Some(MenuLabel {
                    subtitle: body.map(str::to_owned),
                    ..MenuLabel::plain(subject)
                })
            }
            MenuCommand::CommitCoAuthor(i) => {
                let author = self.git.authors.get(i)?;
                let (name, email) = author.split_once(" <").unwrap_or((author, ""));
                Some(MenuLabel {
                    subtitle: Some(email.trim_end_matches('>').to_owned()),
                    ..MenuLabel::plain(name)
                })
            }
            _ => None,
        }
    }

    /// 选了最近的第 `i` 条提交信息：换掉信息框里的文字（可以撤销），光标放在末尾。
    pub(super) fn use_commit_message(&mut self, i: usize) {
        let Some(message) = self.recent_messages().get(i).map(|m| (*m).to_owned()) else {
            return;
        };
        self.git.commit.message.set_text(&message);
        self.git.commit.message.set_caret(message.len(), false);
        self.focus = Focus::CommitMessage;
    }

    /// 选了第 `i` 个共同作者：在信息末尾加一行 `Co-authored-by`，已经有这一行时不重复加。
    pub(super) fn add_co_author(&mut self, i: usize) {
        let Some(author) = self.git.authors.get(i).cloned() else {
            return;
        };
        let message = with_co_author(self.git.commit.message.text(), &author);
        let subject_end = message.find('\n').unwrap_or(message.len());
        self.git.commit.message.set_text(&message);
        // 还没写首行时光标留在首行，接着写。
        let caret = if subject_end == 0 { 0 } else { message.len() };
        self.git.commit.message.set_caret(caret, false);
        self.focus = Focus::CommitMessage;
    }
}

/// 在提交信息 `message` 末尾加上共同作者 `author`（`名字 <邮箱>`）。
///
/// 末尾已经是一段 `键: 值` 形式的附注（签署行、其他共同作者）时接在这一段里，否则先空一行；
/// 已经有这一行时原样返回。
pub(super) fn with_co_author(message: &str, author: &str) -> String {
    let line = format!("{CO_AUTHOR}{author}");
    if message.lines().any(|l| l.trim() == line) {
        return message.to_owned();
    }
    let body = message.trim_end();
    let last = body.rsplit("\n\n").next().unwrap_or_default();
    let trailers = body.contains("\n\n") && last.lines().all(is_trailer);
    match trailers {
        true => format!("{body}\n{line}"),
        false => format!("{body}\n\n{line}"),
    }
}

/// `line` 是不是一行附注：`键: 值`，键由字母、数字和 `-` 组成。
fn is_trailer(line: &str) -> bool {
    line.split_once(": ").is_some_and(|(key, _)| {
        !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// `line` 里有没有作为单词出现的 TODO 或 FIXME（不分大小写）。
pub(super) fn has_todo(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["todo", "fixme"].iter().any(|word| {
        lower.match_indices(word).any(|(at, _)| {
            let before = lower[..at].chars().next_back();
            let after = lower[at + word.len()..].chars().next();
            let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
            edge(before) && edge(after)
        })
    })
}

/// 提交前的 TODO 检查在一个仓库里查的范围。
struct TodoScope {
    /// 仓库的根目录；`None` 是当前仓库。
    repo: Option<PathBuf>,
    /// 查暂存区相对 `HEAD` 新增的行（暂存区视图）；否则查 `tracked` 在工作区里新增的行。
    staged: bool,
    /// 勾上的已跟踪文件。
    tracked: Vec<PathBuf>,
    /// 勾上的未跟踪文件：整个文件都是新增的。
    untracked: Vec<PathBuf>,
}

impl App {
    /// 一并提交的其他仓库各自的 TODO 检查范围：勾选式下勾上的文件，暂存区视图中暂存区里的内容。
    /// 修改上一次提交、有合并或变基进行中的仓库不一并提交，也不查。
    fn other_todo_scopes(&self) -> Vec<TodoScope> {
        if !self.commit_shows_others() || self.git.commit.amend {
            return Vec::new();
        }
        if self.git.commit.staging() {
            let roots = self.other_staged_roots().into_iter();
            return roots
                .map(|(repo, _)| TodoScope {
                    repo: Some(repo),
                    staged: true,
                    tracked: Vec::new(),
                    untracked: Vec::new(),
                })
                .collect();
        }
        let root = self.git.root.as_deref();
        let summaries = self.git.summaries.lock();
        let summaries = summaries.unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut scopes = Vec::new();
        for repo in (self.git.repositories.iter()).filter(|r| Some(r.as_path()) != root) {
            let Some(status) = summaries.get(repo).filter(|s| s.operation.is_none()) else {
                continue;
            };
            let picked = status
                .entries
                .iter()
                .filter(|e| self.other_checked(&e.path));
            let (untracked, tracked): (Vec<&Entry>, Vec<&Entry>) =
                picked.partition(|e| e.untracked());
            if tracked.is_empty() && untracked.is_empty() {
                continue;
            }
            let paths = |list: Vec<&Entry>| list.into_iter().map(|e| e.path.clone()).collect();
            scopes.push(TodoScope {
                repo: Some(repo.clone()),
                staged: false,
                tracked: paths(tracked),
                untracked: paths(untracked),
            });
        }
        scopes
    }
}

/// 未跟踪文件 `path` 的各行：大于 [`winc_editor::LARGE_FILE`] 或是二进制的文件不读，与差异视图的
/// 规则相同，勾上很大的数据文件时提交不会卡在检查上。
fn text_lines(path: &Path) -> Option<Vec<String>> {
    let size = winc_backend::fs::metadata(path).ok()?.len;
    if size > winc_editor::LARGE_FILE {
        return None;
    }
    let bytes = winc_backend::fs::read(path).ok()?;
    if winc_editor::looks_binary(&bytes) {
        return None;
    }
    Some(
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::{has_todo, with_co_author};

    /// 附注接在已有的附注段里；正文后面没有附注时先空一行；只有首行时也空一行。
    #[test]
    fn co_author_trailers_join_the_trailer_block() {
        let alice = "Alice <a@x>";
        let line = "Co-authored-by: Alice <a@x>";
        assert_eq!(with_co_author("修复", alice), format!("修复\n\n{line}"));
        let signed = "修复\n\n正文\n\nSigned-off-by: Me <m@x>";
        assert_eq!(with_co_author(signed, alice), format!("{signed}\n{line}"));
        assert_eq!(
            with_co_author("修复\n\n正文", alice),
            format!("修复\n\n正文\n\n{line}")
        );
    }

    /// TODO、FIXME 按单词认，不分大小写；词中间的不算。
    #[test]
    fn todo_words_are_matched_as_words() {
        assert!(has_todo("// TODO: 加日志"));
        assert!(has_todo("# fixme later"));
        assert!(has_todo("todo()"));
        assert!(!has_todo("let todos = 1;"));
        assert!(!has_todo("autodoc"));
    }
}
