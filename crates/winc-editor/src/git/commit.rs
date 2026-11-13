//! 提交和回滚：整个文件或只提交一部分改动，提交说明。
//!
//! [`Git::rollback`] 把文件恢复成 `HEAD` 里的内容，[`Git::commit`] 只提交指定的文件，其余
//! 改动留在暂存区和工作区。只提交一部分改动时，先在临时索引里拼出要提交的内容，再提交。

use super::*;

impl Git {
    /// 回滚已跟踪的 `paths`：暂存区和工作区都恢复成 `HEAD` 里的内容（`git restore`）。
    ///
    /// `added` 是 `HEAD` 里没有、新加入版本管理的文件，只从暂存区移除，工作区里的文件由调用方处理。
    ///
    /// # Errors
    ///
    /// `git` 报错时返回它的说明。
    pub fn rollback(
        &self,
        root: &Path,
        paths: &[PathBuf],
        added: &[PathBuf],
    ) -> Result<(), String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        if !added.is_empty() {
            let mut command = self.command(root);
            command
                .args(["rm", "--cached", "--quiet", "--"])
                .args(relative_all(root, added));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        if !paths.is_empty() {
            let mut command = self.command(root);
            command
                .args(["restore", "--source=HEAD", "--staged", "--worktree", "--"])
                .args(relative_all(root, paths));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        Ok(())
    }

    /// 提交：只提交 `paths` 指定的文件，暂存区里其他文件的改动原样留着。
    ///
    /// `amend` 为 `true` 时修改上一次提交，`paths` 可以为空，只改提交信息。返回新提交号的前 7 位。
    /// `add` 中未跟踪的文件先 `git add`；`partial` 非空时在临时索引里拼出要提交的内容。
    ///
    /// # Errors
    ///
    /// `git` 报错（hooks 拒绝、没有可提交的内容、身份没配置……）时返回它的说明。
    pub fn commit(
        &self,
        root: &Path,
        message: &str,
        paths: &[PathBuf],
        add: &[PathBuf],
        partial: &[Partial],
        amend: bool,
    ) -> Result<String, String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        // 合并、拣选、还原进行中时不能用 `--only`：勾上的文件先加入暂存区，再提交整个暂存区，
        // 这样才是一个合并提交。
        let operation = git_dir(root).and_then(|d| Operation::detect(&d));
        if let Some(operation @ (Operation::Merge | Operation::CherryPick | Operation::Revert)) =
            operation
        {
            if !partial.is_empty() {
                let op = operation.label();
                let error = winc_i18n::t!("git-commit-partial-during-operation", operation = op);
                return Err(error);
            }
            if !paths.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["add", "--all", "--"])
                    .args(relative_all(root, paths));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            let mut command = self.command(root);
            command.args(["commit", "--file=-", "--cleanup=whitespace"]);
            self.run_commit(root, &mut command, message)?;
            return self.short_head(root);
        }
        if !partial.is_empty() {
            return self.commit_partial(root, message, paths, partial, amend);
        }
        if !add.is_empty() {
            let mut command = self.command(root);
            command.args(["add", "--"]).args(relative_all(root, add));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        let mut command = self.command(root);
        // `whitespace` 只去掉首尾空行和行尾空白；`strip` 还会删掉 `#` 开头的行，
        // 「#42 修复登录」这样的信息会被整段删除。
        command.args(["commit", "--file=-", "--cleanup=whitespace"]);
        if amend {
            command.arg("--amend");
        }
        command
            .arg("--only")
            .arg("--")
            .args(relative_all(root, paths));
        if let Err(e) = self.run_commit(root, &mut command, message) {
            // 提交被拒绝（hooks、身份没配置……）时把刚加入暂存区的未跟踪文件移出，暂存区回到提交
            // 之前的状态。`rm --cached` 在还没有提交过的仓库里也能用；回退失败仍报提交本身的原因。
            if !add.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["rm", "--cached", "--quiet", "--ignore-unmatch", "--"])
                    .args(relative_all(root, add));
                let _ = self.run(&mut command, STATUS_TIMEOUT, &cancel);
            }
            return Err(e);
        }
        self.short_head(root)
    }

    /// 有只提交一部分的文件时执行的提交，见 [`Git::commit`]。
    fn commit_partial(
        &self,
        root: &Path,
        message: &str,
        paths: &[PathBuf],
        partial: &[Partial],
        amend: bool,
    ) -> Result<String, String> {
        let cancel = Cancel::default();
        let index = crate::wsl::temp_dir(root).join(format!(
            "winc-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        // 发行版内的 `git` 只接受发行版内的路径。
        let index_arg = crate::wsl::locate_here(&index)
            .map_or_else(|| index.clone().into_os_string(), |place| place.path.into());
        let with_index = |command: &mut Command| {
            command.env("GIT_INDEX_FILE", &index_arg);
        };
        let result = (|| {
            // 从 `HEAD` 开始；还没有提交过时是空的暂存区。其他失败（超时、对象损坏、写不了临时
            // 文件）不能当作空的暂存区：否则提交出的树只含 `paths` 中的文件，其余文件都成了删除。
            let mut command = self.command(root);
            with_index(&mut command);
            command.args(["read-tree", "HEAD"]);
            if let Err(e) = self.run(&mut command, STATUS_TIMEOUT, &cancel) {
                let mut command = self.command(root);
                command.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
                if self.run(&mut command, SHOW_TIMEOUT, &cancel).is_ok() {
                    return Err(e);
                }
            }

            let whole: Vec<PathBuf> = paths
                .iter()
                .filter(|p| !partial.iter().any(|q| &q.path == *p))
                .cloned()
                .collect();
            if !whole.is_empty() {
                let mut command = self.command(root);
                with_index(&mut command);
                command
                    .args(["add", "--all", "--"])
                    .args(relative_all(root, &whole));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            for part in partial {
                let relative = slash_path(root, &part.path)?;
                // 按工作区的形式写进去：`--path` 套用该路径的过滤器（autocrlf 等）。
                let mut command = self.command(root);
                command
                    .args(["hash-object", "-w", "--stdin"])
                    .arg(joined_arg(&[OsStr::new("--path="), &relative]))
                    .stderr(Stdio::piped());
                let content = part.content.clone();
                let oid = match process::run(&mut command, content, STATUS_TIMEOUT, &cancel) {
                    Ok(output) if output.status.success() => {
                        String::from_utf8_lossy(&output.stdout).trim().to_owned()
                    }
                    Ok(output) => return Err(failure(&output.stderr)),
                    Err(e) => return Err(describe(&e)),
                };
                let mode = self.file_mode(&part.path, &cancel)?;
                let mut command = self.command(root);
                with_index(&mut command);
                command
                    .args(["update-index", "--add", "--cacheinfo"])
                    .arg(joined_arg(&[
                        OsStr::new(&format!("{mode},{oid},")),
                        &relative,
                    ]));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }

            let mut command = self.command(root);
            with_index(&mut command);
            command.args(["commit", "--file=-", "--cleanup=whitespace"]);
            if amend {
                command.arg("--amend");
            }
            self.run_commit(root, &mut command, message)?;

            // 实际的暂存区里，`paths` 换成刚提交的内容。
            if !paths.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["reset", "--quiet", "--"])
                    .args(relative_all(root, paths));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            self.short_head(root)
        })();
        let _ = winc_backend::fs::remove_file(&index);
        result
    }

    /// 运行 `git commit`，提交信息由标准输入传入。
    fn run_commit(&self, root: &Path, command: &mut Command, message: &str) -> Result<(), String> {
        command.stderr(Stdio::piped());
        match process::run(
            command,
            message.as_bytes().to_vec(),
            COMMIT_TIMEOUT,
            &Cancel::default(),
        ) {
            Ok(output) => {
                // hooks 的输出可能在标准输出里。
                let mut text = output.stderr;
                text.extend_from_slice(&output.stdout);
                output::record(root, &["commit"], output.status.code(), &text);
                if output.status.success() {
                    Ok(())
                } else {
                    Err(failure(&text))
                }
            }
            Err(e) => {
                let reason = describe(&e);
                output::record(root, &["commit"], None, reason.as_bytes());
                Err(reason)
            }
        }
    }

    /// 返回 `HEAD` 的提交号（前 7 位）。
    fn short_head(&self, root: &Path) -> Result<String, String> {
        let mut command = self.command(root);
        command.args(["rev-parse", "--short", "HEAD"]);
        let oid = self.run(&mut command, SHOW_TIMEOUT, &Cancel::default())?;
        Ok(String::from_utf8_lossy(&oid).trim().to_owned())
    }

    /// 返回 `HEAD` 指向的提交；还没有提交过、`git` 运行失败时为 `None`。
    pub fn last_commit(&self, root: &Path) -> Option<LastCommit> {
        let mut command = self.command(root);
        command.args([
            "log",
            "-1",
            "--date=format-local:%Y-%m-%d %H:%M",
            "--format=%H%x00%ct%x00%ad%x00%B",
        ]);
        let stdout = self
            .run(&mut command, SHOW_TIMEOUT, &Cancel::default())
            .ok()?;
        let text = String::from_utf8_lossy(&stdout);
        let mut fields = text.splitn(4, '\0');
        let oid = fields.next()?.trim().to_owned();
        let time = fields.next()?.trim().parse().ok()?;
        let local = fields.next()?.to_owned();
        let message = fields.next()?.trim_end().to_owned();
        let subject = message.lines().next().unwrap_or_default().to_owned();
        (!oid.is_empty()).then_some(LastCommit {
            oid,
            subject,
            message,
            time,
            local,
        })
    }

    /// 撤销 `HEAD` 指向的提交 `oid`：分支退回上一个提交，这次提交的改动留在暂存区，工作区不动
    /// （同 `git reset --soft HEAD~1`）；`oid` 是第一个提交时删掉分支的引用，改动同样留在暂存区。
    ///
    /// 只在分支仍指向 `oid` 时改动（`update-ref` 带旧值），之后又提交过、换了分支时不撤销。
    ///
    /// # Errors
    ///
    /// 仓库不受信任、`HEAD` 已经不是 `oid`、`git` 报错时返回说明。
    pub fn undo_commit(&self, root: &Path, oid: &str) -> Result<(), String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        let mut command = self.command(root);
        command.args(["rev-parse", "--verify", "--quiet", &format!("{oid}^")]);
        let parent = self.run(&mut command, SHOW_TIMEOUT, &cancel).ok();
        let parent = parent.map(|p| String::from_utf8_lossy(&p).trim().to_owned());
        let mut command = self.command(root);
        command.args(["update-ref", "-m", "reset: undo commit"]);
        match parent.filter(|p| !p.is_empty()) {
            Some(parent) => command.args(["HEAD", &parent, oid]),
            None => command.args(["-d", "HEAD", oid]),
        };
        self.run(&mut command, SHOW_TIMEOUT, &cancel)?;
        Ok(())
    }

    /// 返回上一次提交的完整提交信息；还没有提交过、`git` 运行失败时为空字符串。
    pub fn last_message(&self, root: &Path) -> String {
        let mut command = self.command(root);
        command.args(["log", "-1", "--format=%B"]);
        match self.run(&mut command, SHOW_TIMEOUT, &Cancel::default()) {
            Ok(stdout) => String::from_utf8_lossy(&stdout).trim_end().to_owned(),
            Err(_) => String::new(),
        }
    }
}
