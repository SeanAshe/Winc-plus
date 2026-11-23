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
/// 合并方式的拉取接受 `--autostash` 的最低版本；更早的只在变基时接受。
const MERGE_AUTOSTASH: (u32, u32) = (2, 27);

/// 解释 `pull.rebase`、`branch.<分支>.rebase` 的值：变基为 `Some(true)`，合并为
/// `Some(false)`，无法识别时为 `None`。
fn rebase_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" | "merges" | "m" | "interactive" | "i" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

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
    /// 先获取上游的最新状态。只落后时拉取，只领先时推送。两边都有新提交时，上游多出的提交若都
    /// 可由本分支引用日志中的某条记录到达，`force` 时用 `--force-with-lease`（`git` 够新时再加
    /// `--force-if-includes`）推送本地版本，否则返回 [`Synced::NeedsForce`]，不改动本地和上游。
    /// 上游有本地从未有过的提交时先拉取再推送，拉取的方式 `rebase` 同 [`Git::pull`]。
    ///
    /// # Errors
    ///
    /// 没有上游、拉取出现冲突、推送被拒绝（包括上游在本地最后一次获取之后又有了新提交）时返回
    /// `git` 给出的原因。
    pub fn sync(&self, root: &Path, force: bool, rebase: Option<bool>) -> Result<Synced, String> {
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
            let mut args = vec!["push".to_owned(), "--force-with-lease".to_owned()];
            if self.version(root, &cancel) >= Some(FORCE_IF_INCLUDES) {
                args.push("--force-if-includes".to_owned());
            }
            args.extend(self.push_target(root, &cancel));
            return self.operate(root, &args).map(Synced::Done);
        }
        let mut summary = String::new();
        if behind > 0 {
            summary = self.pull(root, rebase)?;
        }
        // 拉取时合并产生的提交同样需要推送。
        if self.count(root, "@{upstream}..HEAD", &cancel)? > 0 {
            let mut args = vec!["push".to_owned()];
            args.extend(self.push_target(root, &cancel));
            summary = self.operate(root, &args)?;
        }
        Ok(Synced::Done(summary))
    }

    /// 从上游拉取：`rebase` 为真时变基，为假时合并，为 `None` 时按配置（[`Git::pull_rebases`]）。
    ///
    /// 未配置 `pull.rebase` 时 `git` 拒绝拉取分叉的分支，因此这时显式按合并方式拉取。未提交的更改
    /// 由 `git` 先储藏、拉取完成后恢复（`--autostash`）；拉取产生冲突时，更改在合并或变基结束后
    /// 恢复，结果见 [`Git::carried`]。2.27 以前的 `git` 合并时不接受 `--autostash`，这时不带：更改
    /// 会被拉取的内容覆盖时 `git` 拒绝拉取，更改不会丢失。
    ///
    /// # Errors
    ///
    /// 没有上游、出现冲突、未跟踪的文件会被拉取到的文件覆盖时返回 `git` 给出的原因。
    pub fn pull(&self, root: &Path, rebase: Option<bool>) -> Result<String, String> {
        let configured = rebase.is_none().then(|| self.pull_rebases(root)).flatten();
        let mut args = vec!["pull"];
        match (rebase, configured) {
            (Some(true), _) => args.push("--rebase"),
            (Some(false), _) | (None, None) => args.push("--no-rebase"),
            (None, Some(_)) => {}
        }
        if rebase.or(configured) == Some(true) || self.merge_autostash(root) {
            args.push("--autostash");
        }
        self.operate(root, &args)
    }

    /// 合并方式的拉取能否带 `--autostash`（见 [`MERGE_AUTOSTASH`]）。
    fn merge_autostash(&self, root: &Path) -> bool {
        self.version(root, &Cancel::default()) >= Some(MERGE_AUTOSTASH)
    }

    /// 从远程 `remote` 的分支 `branch` 拉取到当前分支，`rebase` 为真时变基、否则合并，返回
    /// `git` 的结论；未提交的更改的处理同 [`Git::pull`]。
    ///
    /// # Errors
    ///
    /// `remote`、`branch` 以 `-` 开头，或出现冲突、远程没有该分支时返回原因。
    pub fn pull_from(
        &self,
        root: &Path,
        remote: &str,
        branch: &str,
        rebase: bool,
    ) -> Result<String, String> {
        if remote.starts_with('-') || branch.starts_with('-') {
            return Err(winc_i18n::t!("git-push-no-remote").to_owned());
        }
        let mode = if rebase { "--rebase" } else { "--no-rebase" };
        let mut args = vec!["pull", mode];
        if rebase || self.merge_autostash(root) {
            args.push("--autostash");
        }
        args.extend(["--end-of-options", remote, branch]);
        self.operate(root, &args)
    }

    /// 返回最新一条储藏的提交号；没有储藏时为 `None`。
    ///
    /// 拉取前记下，拉取后交给 [`Git::carried`]，判断未提交的更改是否留在了新的储藏里。
    pub fn stash_tip(&self, root: &Path) -> Option<String> {
        let mut command = self.command(root);
        command.args(["rev-parse", "-q", "--verify", "refs/stash"]);
        let stdout = self
            .run(&mut command, STATUS_TIMEOUT, &Cancel::default())
            .ok()?;
        Some(String::from_utf8_lossy(&stdout).trim().to_owned())
    }

    /// 是否工作区中有未合并（冲突）的文件；`git` 运行失败时按没有处理。
    ///
    /// 只看标准输出：`git` 在标准错误上的提示（例如换行符将被转换）不是文件名。不进入子模块：
    /// 其配置不在信任检查的范围内。
    pub fn has_unmerged(&self, root: &Path) -> bool {
        let mut command = self.command(root);
        command.args(["diff", "--name-only", "--diff-filter=U", "--no-ext-diff"]);
        command.arg("--ignore-submodules=dirty");
        self.run(&mut command, STATUS_TIMEOUT, &Cancel::default())
            .is_ok_and(|names| !names.trim_ascii().is_empty())
    }

    /// 返回拉取完成后未提交的更改的去向；`before` 是拉取前的 [`Git::stash_tip`]。
    ///
    /// `git` 恢复更改失败时把更改存为一条新的储藏：有未合并的文件时是冲突，否则是未能恢复。
    pub fn carried(&self, root: &Path, before: Option<&str>) -> Carried {
        if self.has_unmerged(root) {
            return Carried::Conflicted;
        }
        if self.stash_tip(root).as_deref() != before {
            return Carried::Kept;
        }
        Carried::Restored
    }

    /// 返回把 `head`（`HEAD` 或一个提交号）推送到 `target` 会送出的提交：最新的在前、最多
    /// `limit` 个提交的标题，以及这样的提交一共多少个。
    ///
    /// # Errors
    ///
    /// 远程名、分支名、`head` 以 `-` 开头，或 `git` 运行失败时返回原因。
    pub fn outgoing(
        &self,
        root: &Path,
        target: Outgoing<'_>,
        head: &str,
        limit: usize,
    ) -> Result<(Vec<String>, usize), String> {
        if head.starts_with('-') {
            return Err(winc_i18n::t!("git-rebase-not-on-branch").to_owned());
        }
        let cancel = Cancel::default();
        let range: Vec<String> = match target {
            Outgoing::Upstream => vec![format!("@{{upstream}}..{head}")],
            Outgoing::Publish => [head, "--not", "--remotes"].map(String::from).into(),
            Outgoing::To { remote, branch } => {
                if remote.starts_with('-') || branch.starts_with('-') {
                    return Err(winc_i18n::t!("git-push-no-remote").to_owned());
                }
                let tracking = format!("refs/remotes/{remote}/{branch}");
                let mut command = self.command(root);
                command.args(["rev-parse", "--verify", "--quiet", &tracking]);
                match self.run(&mut command, STATUS_TIMEOUT, &cancel) {
                    Ok(_) => vec![format!("{tracking}..{head}")],
                    // `--remotes=<名字>` 匹配 `refs/remotes/<名字>/*`。
                    Err(_) => vec![head.into(), "--not".into(), format!("--remotes={remote}")],
                }
            }
        };
        self.subjects(root, &range, limit)
    }

    /// 返回拉取远程 `remote` 的分支 `branch` 时会带来的提交：最新的至多 `limit` 个提交的标题
    /// （最新的在前）和提交总数，按本地记下的远程分支计算。
    ///
    /// # Errors
    ///
    /// `remote`、`branch` 以 `-` 开头，本地没有该远程分支，或 `git` 运行失败时返回原因。
    pub fn incoming(
        &self,
        root: &Path,
        remote: &str,
        branch: &str,
        limit: usize,
    ) -> Result<(Vec<String>, usize), String> {
        if remote.starts_with('-') || branch.starts_with('-') {
            return Err(winc_i18n::t!("git-push-no-remote").to_owned());
        }
        let tracking = format!("refs/remotes/{remote}/{branch}");
        let mut command = self.command(root);
        command.args(["rev-parse", "--verify", "--quiet", &tracking]);
        self.run(&mut command, STATUS_TIMEOUT, &Cancel::default())?;
        self.subjects(root, &[format!("HEAD..{tracking}")], limit)
    }

    /// 返回 `range` 中最新的至多 `limit` 个提交的标题（最新的在前）和提交总数。
    fn subjects(
        &self,
        root: &Path,
        range: &[String],
        limit: usize,
    ) -> Result<(Vec<String>, usize), String> {
        let cancel = Cancel::default();
        let mut command = self.command(root);
        command.args(["rev-list", "--count"]).args(range);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        let total = String::from_utf8_lossy(&stdout).trim().parse().unwrap_or(0);
        let mut command = self.command(root);
        let count = format!("--max-count={limit}");
        // `log.showSignature` 等配置会在标题之间插入签名校验的输出，这里关掉。
        let args = ["log", "--no-show-signature", "--format=%s", &count];
        command.args(args).args(range);
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

    /// 返回拉取当前分支时配置的方式：`branch.<分支>.rebase` 优先，其次 `pull.rebase`；为
    /// `true`、`merges`、`interactive` 时是变基（`Some(true)`），为 `false` 时是合并，都没有
    /// 配置时为 `None`。
    pub fn pull_rebases(&self, root: &Path) -> Option<bool> {
        let cancel = Cancel::default();
        let branch = self.current_branch(root, &cancel);
        let keys = (branch.map(|b| format!("branch.{b}.rebase")).into_iter())
            .chain(["pull.rebase".to_owned()]);
        let value = keys
            .into_iter()
            .find_map(|key| self.config(root, &key, &cancel))?;
        rebase_value(&value)
    }

    /// 返回当前分支的名字；分离头指针时为 `None`。
    pub(super) fn current_branch(&self, root: &Path, cancel: &Cancel) -> Option<String> {
        let mut command = self.command(root);
        command.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let branch = self.run(&mut command, STATUS_TIMEOUT, cancel).ok()?;
        Some(String::from_utf8_lossy(&branch).trim().to_owned())
    }

    /// 推送当前分支时写在 `git push` 后面的目标：上游与当前分支不同名时是上游所在的远程和
    /// `HEAD:` 上游分支，其余情况为空，由 `git` 按配置决定。
    ///
    /// `push.default` 默认的 `simple` 拒绝推到不同名的上游，从别人的合并请求签出的分支常常这样；
    /// 这时同其他 Git 客户端一样推到上游。用户设置了 `push.default`，或推送去另一个远程时按其
    /// 设置（见 [`upstream_target`]）。
    pub(super) fn push_target(&self, root: &Path, cancel: &Cancel) -> Vec<String> {
        let Some(branch) = self.current_branch(root, cancel) else {
            return Vec::new();
        };
        let pattern =
            r"^(push\.default|remote\.pushdefault|branch\..*\.(remote|merge|pushremote))$";
        let mut command = self.command(root);
        command.args(["config", "-z", "--get-regexp", pattern]);
        // 一项都没有配置时 `git` 以 1 退出。
        let Ok(stdout) = self.run(&mut command, STATUS_TIMEOUT, cancel) else {
            return Vec::new();
        };
        let Some((remote, merge)) = upstream_target(&String::from_utf8_lossy(&stdout), &branch)
        else {
            return Vec::new();
        };
        vec![
            "--end-of-options".to_owned(),
            remote,
            format!("HEAD:{merge}"),
        ]
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

/// 从 `git config -z --get-regexp` 的输出 `config` 中找出当前分支 `branch` 推送时要写明的
/// （远程，上游的引用）：上游与当前分支同名、设置了 `push.default`，或推送去的远程
/// （`branch.<名字>.pushRemote`、`remote.pushDefault`）不是上游所在的远程时为 `None`，交给
/// `git` 按配置推送：后一种是从一个仓库拉取、推到另一个仓库，`git` 推到同名的分支。
///
/// 上游在本仓库（远程为 `.`）、不是分支，或远程名以 `-` 开头时同样为 `None`。
pub(super) fn upstream_target(config: &str, branch: &str) -> Option<(String, String)> {
    let key = |name: &str| format!("branch.{branch}.{name}");
    let (mut remote, mut merge, mut push_remote, mut push_default) = (None, None, None, None);
    // 同一项写了几次时以最后一次为准，与 `git` 相同。
    for (name, value) in config.split('\0').filter_map(|e| e.split_once('\n')) {
        match name {
            "push.default" => return None,
            "remote.pushdefault" => push_default = Some(value),
            _ if name == key("pushremote") => push_remote = Some(value),
            _ if name == key("remote") => remote = Some(value),
            _ if name == key("merge") => merge = Some(value),
            _ => {}
        }
    }
    let (remote, merge) = (remote?, merge?);
    let upstream = merge.strip_prefix("refs/heads/")?;
    let pushes_to = push_remote.or(push_default).unwrap_or(remote);
    (upstream != branch && pushes_to == remote && remote != "." && !remote.starts_with('-'))
        .then(|| (remote.to_owned(), merge.to_owned()))
}
