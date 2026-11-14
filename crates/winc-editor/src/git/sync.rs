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

/// 推送的目标，用来算出会送出哪些提交（见 [`Git::outgoing`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outgoing<'a> {
    /// 当前分支的上游。
    Upstream,
    /// 还没有上游的分支第一次推送：任何远程分支都没有的提交。
    Publish,
    /// 远程 `remote` 上的分支 `branch`；那里还没有这个分支时，是该远程的任何分支都没有的提交。
    To {
        /// 远程名。
        remote: &'a str,
        /// 远程上的分支名。
        branch: &'a str,
    },
}

/// 各项操作所需的最低 `git` 版本：`switch`、`restore` 与 `--end-of-options` 自该版本起齐备。
pub const MINIMUM_VERSION: (u32, u32) = (2, 24);

/// 支持 `push --force-if-includes` 的最低 `git` 版本。
const FORCE_IF_INCLUDES: (u32, u32) = (2, 30);

/// 从 `git version` 的输出（`git version 2.54.0 (Apple Git-157)`、
/// `git version 2.47.1.windows.2`）读出主、次版本号。
pub(super) fn parse_version(output: &str) -> Option<(u32, u32)> {
    let number = output.trim().strip_prefix("git version ")?;
    let mut parts = number.split(['.', ' ']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

impl Git {
    /// 与当前分支的上游同步。
    ///
    /// 先获取上游的最新状态。只落后时拉取，只领先时推送。两边都有新提交时，上游多出的提交
    /// 若都可由本分支引用日志中的某条记录到达，`force` 时用 `--force-with-lease`（`git` 够新时再加
    /// `--force-if-includes`）推送本地版本，
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
            // 上面已经按引用日志确认过上游多出的提交本地都见过；`--force-if-includes` 让 `git` 在
            // 推送的那一刻再查一遍，两次检查之间上游又变了也会被拒绝。该选项 2.30 才有。
            let mut args = vec!["push", "--force-with-lease"];
            if self.version(root, &cancel) >= Some(FORCE_IF_INCLUDES) {
                args.push("--force-if-includes");
            }
            return self.operate(root, &args).map(Synced::Done);
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

    /// 返回推送到 `target` 会送出的提交：最新的在前、最多 `limit` 个提交的标题，以及这样的提交
    /// 一共多少个。
    ///
    /// # Errors
    ///
    /// 远程名、分支名以 `-` 开头，或 `git` 运行失败时返回原因。
    pub fn outgoing(
        &self,
        root: &Path,
        target: Outgoing<'_>,
        limit: usize,
    ) -> Result<(Vec<String>, usize), String> {
        let cancel = Cancel::default();
        let range: Vec<String> = match target {
            Outgoing::Upstream => vec!["@{upstream}..HEAD".into()],
            Outgoing::Publish => ["HEAD", "--not", "--remotes"].map(String::from).into(),
            Outgoing::To { remote, branch } => {
                if remote.starts_with('-') || branch.starts_with('-') {
                    return Err(winc_i18n::t!("git-push-no-remote").to_owned());
                }
                let tracking = format!("refs/remotes/{remote}/{branch}");
                let mut command = self.command(root);
                command.args(["rev-parse", "--verify", "--quiet", &tracking]);
                match self.run(&mut command, STATUS_TIMEOUT, &cancel) {
                    Ok(_) => vec![format!("{tracking}..HEAD")],
                    // `--remotes=<名字>` 匹配 `refs/remotes/<名字>/*`。
                    Err(_) => vec!["HEAD".into(), "--not".into(), format!("--remotes={remote}")],
                }
            }
        };
        let mut command = self.command(root);
        command.args(["rev-list", "--count"]).args(&range);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        let total = String::from_utf8_lossy(&stdout).trim().parse().unwrap_or(0);
        let mut command = self.command(root);
        let count = format!("--max-count={limit}");
        // `log.showSignature` 等配置会在标题之间插入签名校验的输出，这里关掉。
        let args = ["log", "--no-show-signature", "--format=%s", &count];
        command.args(args).args(&range);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        let subjects = String::from_utf8_lossy(&stdout);
        Ok((subjects.lines().map(str::to_owned).collect(), total))
    }

    /// 返回 `root` 所在系统中 `git` 的主、次版本号；读不出时为 `None`。
    pub fn version(&self, root: &Path, cancel: &Cancel) -> Option<(u32, u32)> {
        let mut command = self.command(root);
        command.arg("version");
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel).ok()?;
        parse_version(&String::from_utf8_lossy(&stdout))
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
