//! 交互式变基：读取要变基的提交，按用户的安排生成 `git rebase -i` 的待办列表并执行。
//!
//! 待办列表通过 `GIT_SEQUENCE_EDITOR` 写入。改写的提交信息写在待办的 `exec` 行里，不依赖临时文件，
//! 变基因冲突暂停后继续时仍然有效；待办文件在 `git rebase` 返回后删除。

use super::*;

/// 对一个提交的处理方式，对应 `git rebase -i` 待办列表的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebaseAction {
    /// 保留提交。
    Pick,
    /// 保留提交并改写信息。
    Reword,
    /// 应用提交后暂停，供用户修改。
    Edit,
    /// 并入上一个提交，合并两者的信息。
    Squash,
    /// 并入上一个提交，丢弃此提交的信息。
    Fixup,
    /// 丢弃提交。
    Drop,
}

/// 要变基的一个提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseCommit {
    /// 完整的提交号。
    pub oid: String,
    /// 信息的第一行。
    pub subject: String,
    /// 完整的信息。
    pub message: String,
}

/// 一次交互式变基的范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebasePlan {
    /// 变基的基点；从根提交开始时为 `None`。
    pub base: Option<String>,
    /// 范围内的提交，最早的在前。
    pub commits: Vec<RebaseCommit>,
    /// 范围内已存在于远程分支的提交数；改写这些提交后需要强制推送。
    pub pushed: usize,
}

/// 待办列表中的一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseStep {
    /// 完整的提交号。
    pub oid: String,
    /// 处理方式。
    pub action: RebaseAction,
    /// [`RebaseAction::Reword`] 的新信息。
    pub message: Option<String>,
}

/// 把 `message` 写成 `exec` 行的 shell 参数：由 `printf '%b'` 还原。
///
/// 待办列表按行解析，信息中的换行符必须转义；单引号内只有单引号本身需要处理。
fn shell_message(message: &str) -> String {
    let escaped = message
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// 返回 `steps` 对应的待办列表。
///
/// 改写用 `pick` 加 `exec` 实现：`reword` 需要打开编辑器，而 `exec` 把新信息写在列表里。
pub fn rebase_todo(steps: &[RebaseStep]) -> String {
    let mut todo = String::new();
    for step in steps {
        let command = match step.action {
            RebaseAction::Pick | RebaseAction::Reword => "pick",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        todo.push_str(&format!("{command} {}\n", step.oid));
        if let (RebaseAction::Reword, Some(message)) = (step.action, &step.message) {
            todo.push_str(&format!(
                "exec printf '%b' {} | git commit --amend --cleanup=whitespace --file=-\n",
                shell_message(message)
            ));
        }
    }
    todo
}

/// 解析 `git log --format=%H%x1f%P%x1f%B%x1e` 的输出，返回提交和其中是否有合并提交。
fn parse_commits(text: &str) -> (Vec<RebaseCommit>, bool) {
    let mut merges = false;
    let commits = text
        .split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_start().splitn(3, '\x1f');
            let (oid, parents, message) = (fields.next()?, fields.next()?, fields.next()?);
            merges |= parents.split_whitespace().count() > 1;
            let message = message.trim_end().to_owned();
            Some(RebaseCommit {
                oid: oid.to_owned(),
                subject: message.lines().next().unwrap_or_default().to_owned(),
                message,
            })
        })
        .collect();
    (commits, merges)
}

impl Git {
    /// 返回从提交 `oid`（包含）到 `HEAD` 的交互式变基范围。
    ///
    /// # Errors
    ///
    /// `oid` 不在当前分支上、范围内有合并提交或 `git` 运行失败时返回原因。
    pub fn rebase_plan(&self, root: &Path, oid: &str) -> Result<RebasePlan, String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        let mut command = self.command(root);
        command.args([
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            oid,
            "HEAD",
        ]);
        if self.run(&mut command, STATUS_TIMEOUT, &cancel).is_err() {
            return Err(winc_i18n::t!("git-rebase-not-on-branch").to_owned());
        }
        let parent = format!("{oid}^");
        let mut command = self.command(root);
        command.args([
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &parent,
        ]);
        let base = self
            .run(&mut command, STATUS_TIMEOUT, &cancel)
            .ok()
            .map(|out| String::from_utf8_lossy(&out).trim().to_owned());
        let range = base
            .as_ref()
            .map_or_else(|| "HEAD".to_owned(), |base| format!("{base}..HEAD"));

        let mut command = self.command(root);
        command.args(["log", "--reverse", "--format=%H%x1f%P%x1f%B%x1e", &range]);
        let stdout = self.run(&mut command, LOG_TIMEOUT, &cancel)?;
        let (commits, merges) = parse_commits(&String::from_utf8_lossy(&stdout));
        if merges {
            return Err(winc_i18n::t!("git-rebase-has-merges").to_owned());
        }

        let mut command = self.command(root);
        command.args(["rev-list", "--count", &range, "--not", "--remotes"]);
        let local = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        let local: usize = String::from_utf8_lossy(&local).trim().parse().unwrap_or(0);
        Ok(RebasePlan {
            base,
            pushed: commits.len().saturating_sub(local),
            commits,
        })
    }

    /// 按 `steps` 执行交互式变基，返回变基是否暂停（冲突以外的原因，如 [`RebaseAction::Edit`]）。
    ///
    /// `base` 为 `None` 时从根提交开始。未提交的更改由 `--autostash` 暂存并在结束后恢复。
    ///
    /// # Errors
    ///
    /// 仓库未获信任、出现冲突或 `git` 运行失败时返回原因；出现冲突时变基停在冲突处。
    pub fn interactive_rebase(
        &self,
        root: &Path,
        base: Option<&str>,
        steps: &[RebaseStep],
    ) -> Result<bool, String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        // `--git-path` 给出 `git` 看到的路径：写入时换算到本机，写进 shell 命令时原样使用。
        let path_of = |name: &str| -> Result<String, String> {
            let mut command = self.command(root);
            command.args(["rev-parse", "--git-path", name]);
            let out = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            Ok(String::from_utf8_lossy(&out).trim().to_owned())
        };
        let todo_path = path_of("winc-rebase-todo")?;
        let local = crate::host::resolve(root, &todo_path);
        winc_backend::fs::write(&local, rebase_todo(steps)).map_err(|e| e.to_string())?;
        let editor = format!("cat '{}' >", todo_path.replace('\'', "'\\''"));
        let mut args = vec![
            "rebase",
            "--interactive",
            "--autostash",
            "--no-autosquash",
            "--no-rebase-merges",
        ];
        args.push(base.unwrap_or("--root"));
        let result = self.run_prepared(root, &args, OPERATION_TIMEOUT, |command| {
            command.env("GIT_SEQUENCE_EDITOR", &editor);
        });
        let _ = winc_backend::fs::remove_file(&local);
        result?;
        let state = path_of("rebase-merge")?;
        Ok(winc_backend::fs::exists(crate::host::resolve(root, &state)))
    }
}

#[cfg(test)]
mod tests;
