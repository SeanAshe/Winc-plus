//! 与当前分支的上游同步：拉取上游新增的提交，推送本地新增的提交。
//!
//! 两边都有对方没有的提交时，上游多出的提交若都是本分支以前的版本（修改上一次提交、变基等
//! 改写了已推送的提交），确认后以本地为准推送；上游有本地从未有过的提交时，先拉取再推送。
//!
//! 拉取时未提交的更改先由 `git` 储藏（`--autostash`），拉取完成后再恢复，见 [`Carried`]。

use super::*;

/// [`Git::sync`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Synced {
    /// 同步完成，内容是最后一步 `git` 的结论。
    Done(String),
    /// 需要强制推送才能同步，尚未推送；内容是上游将被替换的提交数。
    NeedsForce(usize),
}

/// 拉取前未提交的更改在拉取完成后的去向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carried {
    /// 已恢复到工作区，或者拉取前没有未提交的更改。
    Restored,
    /// 已恢复，但部分更改与拉取到的内容冲突，冲突已在工作区中标记；原更改仍保留在储藏中。
    Conflicted,
    /// 未能恢复，更改保留在储藏中。
    Kept,
}

impl Git {
    /// 与当前分支的上游同步。
    ///
    /// 先获取上游的最新状态。只落后时拉取，只领先时推送。两边都有新提交时，上游多出的提交
    /// 若都可由本分支引用日志中的某条记录到达，`force` 时用 `--force-with-lease` 推送本地版本，
    /// 否则返回 [`Synced::NeedsForce`]，不改动本地和上游。上游有本地从未有过的提交时先拉取再
    /// 推送，未配置 `pull.rebase` 时按合并拉取。
    ///
    /// # Errors
    ///
    /// 没有上游、拉取出现冲突、推送被拒绝（包括上游在本地最后一次获取之后又有了新提交）时返回
    /// `git` 给出的原因。
    pub fn sync(&self, root: &Path, force: bool) -> Result<Synced, String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        // 先获取：上游在上次获取之后的新提交也计入，判断分叉时依据的是上游当前的状态。
        self.operate(root, &["fetch"])?;
        let behind = self.count(root, "HEAD..@{upstream}", &cancel)?;
        let ahead = self.count(root, "@{upstream}..HEAD", &cancel)?;
        if ahead > 0 && behind > 0 && self.rewritten(root, &cancel)? {
            if !force {
                return Ok(Synced::NeedsForce(behind));
            }
            return self
                .operate(root, &["push", "--force-with-lease"])
                .map(Synced::Done);
        }
        let mut summary = String::new();
        if behind > 0 {
            summary = self.pull(root)?;
        }
        // 拉取时合并产生的提交同样需要推送。
        if self.count(root, "@{upstream}..HEAD", &cancel)? > 0 {
            summary = self.operate(root, &["push"])?;
        }
        Ok(Synced::Done(summary))
    }

    /// 从上游拉取，返回 `git` 的结论。
    ///
    /// 未配置 `pull.rebase` 时 `git` 拒绝拉取分叉的分支，因此显式按合并方式拉取。未提交的更改
    /// 由 `git` 先储藏、拉取完成后恢复（`--autostash`）：工作区有改动时拉取不会因此被拒绝；拉取
    /// 产生冲突时，更改在合并或变基结束（继续或放弃）后恢复。恢复的结果见 [`Git::carried`]。
    ///
    /// # Errors
    ///
    /// 没有上游、出现冲突、未跟踪的文件会被拉取到的文件覆盖时返回 `git` 给出的原因。
    pub fn pull(&self, root: &Path) -> Result<String, String> {
        let configured = (self.config(root, "pull.rebase", &Cancel::default())).is_some();
        let pull: &[&str] = if configured {
            &["pull", "--autostash"]
        } else {
            &["pull", "--no-rebase", "--autostash"]
        };
        self.operate(root, pull)
    }

    /// 从上游拉取并把本地提交变基到上游之上，返回 `git` 的结论；未提交的更改的处理同
    /// [`Git::pull`]。
    ///
    /// # Errors
    ///
    /// 没有上游、变基出现冲突时返回 `git` 给出的原因。
    pub fn pull_rebase(&self, root: &Path) -> Result<String, String> {
        self.operate(root, &["pull", "--rebase", "--autostash"])
    }

    /// 返回最新一条储藏的提交号；没有储藏时为 `None`。
    ///
    /// 拉取前记下，拉取后交给 [`Git::carried`]，判断未提交的更改是否留在了新的储藏里。
    pub fn stash_tip(&self, root: &Path) -> Option<String> {
        self.operate(root, &["rev-parse", "-q", "--verify", "refs/stash"])
            .ok()
    }

    /// 返回拉取完成后未提交的更改的去向；`before` 是拉取前的 [`Git::stash_tip`]。
    ///
    /// `git` 恢复更改失败时把更改存为一条新的储藏：有未合并的文件时是冲突，否则是未能恢复。
    pub fn carried(&self, root: &Path, before: Option<&str>) -> Carried {
        let unmerged = ["diff", "--name-only", "--diff-filter=U"];
        if self.operate(root, &unmerged).is_ok_and(|f| !f.is_empty()) {
            return Carried::Conflicted;
        }
        if self.stash_tip(root).as_deref() != before {
            return Carried::Kept;
        }
        Carried::Restored
    }

    /// 返回 `range` 中的提交数（`git rev-list --count`）。
    fn count(&self, root: &Path, range: &str, cancel: &Cancel) -> Result<usize, String> {
        let mut command = self.command(root);
        command.args(["rev-list", "--count", range]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(String::from_utf8_lossy(&stdout).trim().parse().unwrap_or(0))
    }

    /// 返回配置项 `key` 的值；未配置时为 `None`。
    pub(super) fn config(&self, root: &Path, key: &str, cancel: &Cancel) -> Option<String> {
        let mut command = self.command(root);
        command.args(["config", "--get", key]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel).ok()?;
        Some(String::from_utf8_lossy(&stdout).trim().to_owned())
    }

    /// 上游多出的提交是否都可由当前分支引用日志中的某条记录到达，即都是本分支以前的版本。
    ///
    /// 引用日志中的提交号经标准输入以 `^` 前缀传给 `git rev-list`，记录较多时不受命令行长度
    /// 限制。
    fn rewritten(&self, root: &Path, cancel: &Cancel) -> Result<bool, String> {
        let mut command = self.command(root);
        command.args(["symbolic-ref", "--quiet", "HEAD"]);
        let branch = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let branch = String::from_utf8_lossy(&branch).trim().to_owned();

        let mut command = self.command(root);
        command
            .args(["log", "--walk-reflogs", "--format=%H", "--end-of-options"])
            .arg(&branch);
        let history = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let input: Vec<u8> = String::from_utf8_lossy(&history)
            .lines()
            .filter(|oid| !oid.is_empty())
            .flat_map(|oid| format!("^{oid}\n").into_bytes())
            .collect();

        let mut command = self.command(root);
        command
            .args(["rev-list", "--count", "@{upstream}", "^HEAD", "--stdin"])
            .stderr(Stdio::piped());
        match process::run(&mut command, input, STATUS_TIMEOUT, cancel) {
            Ok(output) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim() == "0")
            }
            Ok(output) => Err(failure(&output.stderr)),
            Err(e) => Err(describe(&e)),
        }
    }
}

#[cfg(test)]
mod tests;
