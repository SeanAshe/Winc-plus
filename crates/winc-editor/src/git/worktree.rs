//! 工作树（`git worktree`）：同一个仓库在几个目录里各签出一个分支。
//!
//! 列出工作树、算出新建工作树的目录、把被忽略的文件复制到新工作树，并在移除之前检查里面
//! 未提交的改动。主要类型是 [`Worktree`] 与 [`WorktreeCheck`]。

use ignore::overrides::OverrideBuilder;

use super::*;

/// 仓库的一个工作树及其签出状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// 工作树的目录；能解析时是解析过符号链接的真实路径，与打开目录时记下的路径相同。
    pub path: PathBuf,
    /// 签出的分支或游离的提交；裸仓库的主工作树没有签出，为 `None`。
    pub head: Option<Head>,
    /// 是否被锁定（`git worktree lock`）：不会被移除、移动、清理。
    pub locked: bool,
    /// 目录是否已经不存在，`git worktree prune` 会清掉这条记录。
    pub prunable: bool,
}

impl Worktree {
    /// 签出的分支名；游离的 `HEAD`、裸仓库时返回 `None`。
    pub fn branch(&self) -> Option<&str> {
        match &self.head {
            Some(Head::Branch(name)) => Some(name),
            _ => None,
        }
    }
}

/// 移除工作树之前的检查结果（[`Git::check_worktree`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorktreeCheck {
    /// 修改过（含已暂存）的文件数。
    pub changed: usize,
    /// 未跟踪的文件数，不含被忽略的文件。
    pub untracked: usize,
    /// 签出的分支能否用 `git branch -d` 删除：有上游时看是否已合并到上游，没有上游时看是否已
    /// 合并到当前仓库的 `HEAD`。
    pub merged: bool,
    /// 游离的 `HEAD` 上不在任何分支、标签、远程分支上的提交数。
    ///
    /// 移除工作树时连同它的引用日志一起清掉，这些提交将无法找回。
    pub unreachable: usize,
}

/// 返回仓库的名字：主工作树的目录名，裸仓库去掉 `.git` 后缀（`winc.git` → `winc`）。
pub fn repo_name(main: &Path) -> String {
    let name = crate::display_name(main);
    name.strip_suffix(".git").unwrap_or(&name).to_owned()
}

/// 返回新建工作树的目录：`folder` 下的 `<仓库名>/<分支名>`；没有 `folder` 时放在主工作树
/// `main` 旁边的 `<仓库名>.worktrees/<分支名>`，同名目录已存在时加后缀。
///
/// `folder` 对应配置项 `git.worktreeRoot`，开头的 `~/` 展开为 `home`，相对路径相对于 `main`。
/// `home` 是 `main` 所在系统中用户的主目录（[`crate::wsl::home_dir`]）。
pub fn worktree_dir(
    main: &Path,
    folder: Option<&str>,
    home: Option<&Path>,
    branch: &str,
) -> PathBuf {
    unique_dir(&worktree_parent(main, folder, home), branch)
}

/// 返回新建工作树所在的上层目录：`folder` 下的 `<仓库名>`；没有 `folder` 时是主工作树
/// `main` 旁边的 `<仓库名>.worktrees`。
///
/// 以下情况同样使用 `main` 旁边的目录：
///
/// - `folder` 以 `~/` 开头，而 `home` 未知；
/// - `folder` 与 `main` 不在同一系统中（[`crate::wsl::same_system`]）。
pub fn worktree_parent(main: &Path, folder: Option<&str>, home: Option<&Path>) -> PathBuf {
    let repo = repo_name(main);
    let configured = folder.and_then(|folder| match folder.strip_prefix("~/") {
        Some(rest) => home.map(|home| home.join(rest)),
        None => Some(main.join(folder)),
    });
    match configured.filter(|dir| crate::wsl::same_system(main, dir)) {
        Some(dir) => dir.join(repo),
        None => main
            .parent()
            .unwrap_or(main)
            .join(format!("{repo}.worktrees")),
    }
}

/// 返回 `parent` 下给分支 `branch` 用的目录：分支名里的 `/` 换成 `-`，同名的文件或目录已存在时
/// 依次加 `-2`、`-3` 等后缀。
pub fn unique_dir(parent: &Path, branch: &str) -> PathBuf {
    let name = branch.replace('/', "-");
    let mut dir = parent.join(&name);
    let mut n = 2;
    while dir.symlink_metadata().is_ok() {
        dir = parent.join(format!("{name}-{n}"));
        n += 1;
    }
    dir
}

impl Git {
    /// 返回仓库 `root` 的所有工作树，主工作树在最前面。
    ///
    /// 只有一个工作树时（共用的 `git` 目录里没有 `worktrees/`，绝大多数仓库都是这样）不运行
    /// `git`，返回空列表，调用方按只有主工作树处理。
    ///
    /// `git` 2.36 以前不支持 `-z`，此时改用以换行分隔的输出；路径中含换行时该格式无法解析。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn worktrees(&self, root: &Path, cancel: &Cancel) -> Result<Vec<Worktree>, String> {
        if !git_common_dir(root).is_some_and(|d| d.join("worktrees").is_dir()) {
            return Ok(Vec::new());
        }
        let mut command = self.command(root);
        command
            .args(["worktree", "list", "--porcelain", "-z"])
            .stderr(Stdio::piped());
        let output = process::run(&mut command, Vec::new(), STATUS_TIMEOUT, cancel)
            .map_err(|e| describe(&e))?;
        let mut worktrees = match output.status.code() {
            Some(0) => parse_worktrees(&output.stdout, b'\0'),
            // `git` 遇到不支持的选项时按用法错误退出，退出码为 129。
            Some(129) => {
                let mut command = self.command(root);
                command.args(["worktree", "list", "--porcelain"]);
                parse_worktrees(&self.run(&mut command, STATUS_TIMEOUT, cancel)?, b'\n')
            }
            _ => return Err(failure(&output.stderr)),
        };
        // `git` 给出的路径没有解析符号链接（`/tmp` 和 `/private/tmp`），打开目录时记下的是
        // 解析过的路径。
        for worktree in &mut worktrees {
            worktree.path = crate::wsl::resolve(root, &worktree.path.to_string_lossy());
            if let Ok(real) = winc_config::files::canonicalize(&worktree.path) {
                worktree.path = real;
            }
        }
        Ok(worktrees)
    }

    /// 把仓库 `from` 里被忽略、匹配 `globs` 的文件复制到新建的工作树 `to` 的同一位置。
    ///
    /// 返回复制了几项，`to` 里已经有的不覆盖。被忽略的整个目录（`git` 列出时合成一项）也整个
    /// 复制；复制用 `fs::copy`，macOS 上同一磁盘内是 `clonefile`，几乎不占额外空间。
    /// WSL 发行版中的仓库在发行版内复制，以保留可执行位和符号链接。
    ///
    /// # Errors
    ///
    /// glob 有误、`git` 运行失败或复制失败时返回原因；失败之前已复制的文件保留。
    pub fn copy_ignored(
        &self,
        from: &Path,
        to: &Path,
        globs: &[String],
        cancel: &Cancel,
    ) -> Result<usize, String> {
        if globs.is_empty() {
            return Ok(0);
        }
        let mut builder = OverrideBuilder::new(from);
        for glob in globs {
            builder.add(glob).map_err(|e| {
                let (glob, error) = (glob.as_str(), e.to_string());
                winc_i18n::t!("git-worktree-copy-invalid-glob", glob = glob, error = error)
            })?;
        }
        let globs = builder.build().map_err(|e| e.to_string())?;
        let mut command = self.command(from);
        command.args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
        ]);
        command.arg("--directory");
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let mut entries = Vec::new();
        for entry in stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
            let entry = String::from_utf8_lossy(entry);
            let (relative, is_dir) = match entry.strip_suffix('/') {
                Some(dir) => (dir, true),
                None => (&*entry, false),
            };
            let source = from.join(relative);
            let target = to.join(relative);
            // 新工作树放在仓库里面（`git.worktreeRoot` 写了相对路径）时，被忽略的那个目录里就有
            // 新工作树自身，复制它会无休止地嵌套下去。
            if !globs.matched(&source, is_dir).is_whitelist()
                || target.symlink_metadata().is_ok()
                || to.starts_with(&source)
            {
                continue;
            }
            entries.push(relative.to_owned());
        }
        let failed = |path: &str, error: String| {
            winc_i18n::t!("git-worktree-copy-failed", path = path, error = error)
        };
        if let (Some(source), Some(target)) =
            (crate::wsl::locate_here(from), crate::wsl::locate_here(to))
            && source.distro.eq_ignore_ascii_case(&target.distro)
        {
            return crate::wsl::copy_entries(&source, &target, &entries, cancel)
                .map_err(|(path, error)| failed(&path, error));
        }
        let mut copied = 0;
        for relative in &entries {
            let (source, target) = (from.join(relative), to.join(relative));
            let copy = || {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                crate::editing::paste::copy_entry(&source, &target)
            };
            copy().map_err(|e| failed(relative, e.to_string()))?;
            copied += 1;
        }
        Ok(copied)
    }

    /// 移除工作树 `worktree` 之前检查：里面有多少未提交的改动，签出的分支是否已合并到仓库
    /// `root` 的 `HEAD`。
    ///
    /// # Errors
    ///
    /// `root` 的配置不可信（移除算改仓库的操作），或 `git` 运行失败时返回原因。
    pub fn check_worktree(
        &self,
        root: &Path,
        worktree: &Worktree,
        cancel: &Cancel,
    ) -> Result<WorktreeCheck, String> {
        self.require_trust(root)?;
        let mut command = self.command(&worktree.path);
        command.args(["status", "--porcelain", "-z", "--no-renames"]);
        command.arg("--untracked-files=all");
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let mut check = WorktreeCheck::default();
        for entry in stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
            if entry.starts_with(b"??") {
                check.untracked += 1;
            } else {
                check.changed += 1;
            }
        }
        match worktree.branch() {
            Some(branch) => check.merged = self.deletable(root, branch, cancel)?,
            None => {
                let mut command = self.command(&worktree.path);
                command.args(["rev-list", "--count", "HEAD", "--not"]);
                command.args(["--branches", "--tags", "--remotes"]);
                let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
                check.unreachable = String::from_utf8_lossy(&stdout).trim().parse().unwrap_or(0);
            }
        }
        Ok(check)
    }

    /// 返回本地分支 `branch` 能否用 `git branch -d` 删除：有上游时看是否已合并到上游，没有
    /// 上游或上游引用已不存在时看是否已合并到仓库 `root` 的 `HEAD`。
    fn deletable(&self, root: &Path, branch: &str, cancel: &Cancel) -> Result<bool, String> {
        // 模式 `refs/heads/feat` 也匹配 `refs/heads/feat/x`，因此按完整的引用名比较。
        let full = format!("refs/heads/{branch}");
        let mut command = self.command(root);
        command.args([
            "for-each-ref",
            "--format=%(refname)%00%(upstream)%00%(upstream:track)",
        ]);
        command.arg(&full);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let text = String::from_utf8_lossy(&stdout);
        let upstream = text.lines().find_map(|line| {
            let mut fields = line.split('\0');
            let (name, upstream, track) = (fields.next()?, fields.next()?, fields.next()?);
            (name == full && !upstream.is_empty() && track != "[gone]").then(|| upstream.to_owned())
        });
        let target = upstream.unwrap_or_else(|| "HEAD".to_owned());
        let mut command = self.command(root);
        command
            .arg("for-each-ref")
            .arg(format!("--merged={target}"));
        command.arg("--format=%(refname)").arg(&full);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(String::from_utf8_lossy(&stdout).lines().any(|r| r == full))
    }
}

/// 解析 `git worktree list --porcelain` 的输出，返回各工作树及其签出状态。
///
/// 每个工作树若干个字段：`worktree 路径`、`HEAD 提交号`、`branch refs/heads/x`、`detached`、
/// `bare`、`locked [原因]`、`prunable [原因]`。各字段以 `terminator` 结尾，带 `-z` 时为 NUL，
/// 否则为换行；工作树之间多一个 `terminator`。
pub(super) fn parse_worktrees(output: &[u8], terminator: u8) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<(Worktree, Option<String>)> = None;
    for field in output.split(|&b| b == terminator) {
        let field = String::from_utf8_lossy(field);
        let (key, value) = field.split_once(' ').unwrap_or((&field, ""));
        match key {
            "worktree" => {
                worktrees.extend(current.take().map(finish));
                let worktree = Worktree {
                    path: PathBuf::from(value),
                    head: None,
                    locked: false,
                    prunable: false,
                };
                current = Some((worktree, None));
            }
            _ => {
                let Some((worktree, oid)) = current.as_mut() else {
                    continue;
                };
                match key {
                    "HEAD" => *oid = Some(value.to_owned()),
                    "branch" => {
                        let name = value.strip_prefix("refs/heads/").unwrap_or(value);
                        worktree.head = Some(Head::Branch(name.to_owned()));
                    }
                    "locked" => worktree.locked = true,
                    "prunable" => worktree.prunable = true,
                    _ => {}
                }
            }
        }
    }
    worktrees.extend(current.map(finish));
    worktrees
}

/// 读完一个工作树的字段后补全它：没有分支、有提交号的是游离的 `HEAD`（裸仓库两者都没有）。
fn finish((mut worktree, oid): (Worktree, Option<String>)) -> Worktree {
    if worktree.head.is_none()
        && let Some(oid) = oid.filter(|o| !o.is_empty())
    {
        worktree.head = Some(Head::Detached(oid.chars().take(7).collect()));
    }
    worktree
}

#[cfg(test)]
mod tests;
