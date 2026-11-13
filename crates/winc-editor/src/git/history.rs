//! 读取历史：日志、提交的内容与文件、逐行追溯、贮藏，以及某个版本中的文件内容。

use super::*;

impl Git {
    /// 返回 `path` 在 `HEAD` 里的内容，见 [`Git::content_at`]。
    ///
    /// `HEAD` 里没有该文件（包括还没有提交过）时返回 `Ok(None)`。
    ///
    /// # Errors
    ///
    /// `git` 运行失败、超时或被取消时返回原因。
    pub fn head_content(
        &self,
        root: &Path,
        path: &Path,
        cancel: &Cancel,
    ) -> Result<Option<Vec<u8>>, String> {
        self.content_at(root, "HEAD", path, cancel)
    }

    /// 日志：按 `query` 筛过的提交，孩子在前，按时间排，用 [`LogStream::next`] 一批一批地读。
    ///
    /// 不指定分支时是所有分支、远程分支、标签（不含储藏）能走到的提交，文件历史只用当前分支。
    /// `git log` 进程一直开着，从它停下的地方继续读；重新起进程时要走过跳过的提交，越翻越慢。
    ///
    /// # Errors
    ///
    /// 文件不在仓库里、`git` 无法启动时返回原因；`git` 报的错在读第一批时返回。
    pub fn log_stream(&self, root: &Path, query: &LogQuery) -> Result<LogStream, String> {
        // 固定的提交优先；否则筛选文字形如提交号时，按提交号找到的提交排在最前面。
        let pinned = match &query.pin {
            Some(oid) => self.commit_by_hash(root, oid),
            None => self.commit_by_hash(root, query.text.trim()),
        };
        let mut command = self.command(root);
        command.args(["log", "--date-order", "--decorate=full", "--no-color"]);
        let text = query.text.trim();
        if !text.is_empty() || query.author.is_some() {
            command.args(["--regexp-ignore-case", "--fixed-strings"]);
        }
        if !text.is_empty() {
            command.arg(format!("--grep={text}"));
        }
        if let Some(author) = &query.author {
            command.arg(format!("--author={author}"));
        }
        command.arg(LOG_DATE).arg(LOG_FORMAT);
        if query.path.is_some() {
            command.arg("--follow");
        }
        // 选项都在前面，最后是从哪些提交往回走、看哪个文件。分支名前面加 `--end-of-options`：
        // 叫 `-x` 的分支也不会被当成选项。
        match (&query.branch, &query.path) {
            (Some(branch), _) => {
                command.arg("--end-of-options").arg(branch);
            }
            (None, Some(_)) => {
                command.arg("HEAD");
            }
            (None, None) => {
                command.args(["--exclude=refs/stash", "--all"]);
            }
        }
        if let Some(path) = &query.path {
            command.arg("--").arg(slash_path(root, path)?);
        }
        let stream = process::Stream::spawn(&mut command).map_err(|e| describe(&e))?;
        Ok(LogStream {
            stream,
            pinned,
            skip: None,
        })
    }

    /// 筛选文字形如提交号（4 位以上的十六进制）且能解析为某个提交时，返回该提交。
    ///
    /// 日志筛选同时匹配说明和提交号，使用户粘贴提交号即可定位提交。`git log` 无法按
    /// 「说明或提交号」筛选，因此该提交单独读取并排在最前面。
    pub(super) fn commit_by_hash(&self, root: &Path, text: &str) -> Option<Box<Commit>> {
        if !is_commit_hash(text) {
            return None;
        }
        let mut command = self.command(root);
        command
            .args([
                "log",
                "--no-walk",
                "--decorate=full",
                "--no-color",
                LOG_DATE,
                LOG_FORMAT,
            ])
            .arg("--end-of-options")
            .arg(format!("{text}^{{commit}}"));
        let stdout = self
            .run(&mut command, STATUS_TIMEOUT, &Cancel::default())
            .ok()?;
        let commit = parse_log(&String::from_utf8_lossy(&stdout))
            .into_iter()
            .next()?;
        commit
            .oid
            .starts_with(&text.to_ascii_lowercase())
            .then(|| Box::new(commit))
    }

    /// 把提交号（可以缩写）、分支名或标签名 `revision` 解析成完整的提交号。
    ///
    /// 提交号也可以是任何分支都走不到的提交（例如修改上次提交之前的那一个），只要对象仍在
    /// 仓库中。
    ///
    /// # Errors
    ///
    /// `revision` 为空、含有空白或控制字符、无法唯一解析为一个提交，或 Git 查询失败时返回
    /// 原因。
    pub fn resolve_revision(
        &self,
        root: &Path,
        revision: &str,
        cancel: &Cancel,
    ) -> Result<String, String> {
        let unresolved = || winc_i18n::t!("git-go-to-unresolved").to_owned();
        // 以 `-` 开头的文字会被当成选项；较旧的 Git 的 `rev-parse` 不支持 `--end-of-options`，
        // 因此直接拒绝。
        let valid = !revision.is_empty()
            && !revision.starts_with('-')
            && !revision
                .chars()
                .any(|c| c.is_whitespace() || c.is_control());
        if !valid {
            return Err(unresolved());
        }
        let mut command = self.command(root);
        command.args(["rev-parse", "--verify", "--quiet"]);
        command.arg(format!("{revision}^{{commit}}"));
        let stdout = self
            .run(&mut command, SHOW_TIMEOUT, cancel)
            .map_err(|_| unresolved())?;
        let oid = String::from_utf8_lossy(&stdout).trim().to_owned();
        if is_commit_hash(&oid) {
            Ok(oid)
        } else {
            Err(unresolved())
        }
    }

    /// 提交 `oid` 是否在不筛选的日志中：能从某个分支、远程分支、标签（不含储藏）或 `HEAD`
    /// 走到，与 [`Git::log_stream`] 列出的范围相同。
    ///
    /// # Errors
    ///
    /// Git 查询失败时返回原因。
    pub fn in_log(&self, root: &Path, oid: &str, cancel: &Cancel) -> Result<bool, String> {
        let mut command = self.command(root);
        command.args(["for-each-ref", "--format=%(refname)", "--contains"]);
        command.arg(oid);
        let stdout = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        let listed = String::from_utf8_lossy(&stdout)
            .lines()
            .any(|name| !name.is_empty() && name != "refs/stash");
        if listed {
            return Ok(true);
        }
        // 分离的 `HEAD` 不是引用，单独判断。
        let mut command = self.command(root);
        // 完整的十六进制提交号不以 `-` 开头，不会被当成选项。
        command.args(["merge-base", "--is-ancestor", oid, "HEAD"]);
        Ok(self.run(&mut command, SHOW_TIMEOUT, cancel).is_ok())
    }

    /// 储藏列表，最新的在前。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn stashes(&self, root: &Path, cancel: &Cancel) -> Result<Vec<Stash>, String> {
        let mut command = self.command(root);
        // 时间用 `%ci`（提交者的本地时间）的前 16 个字符：`--date` 会把 `%gd` 换成按日期的写法
        // （`stash@{2026-…}`），不能用来格式化 `%cd`。
        command.args([
            "stash",
            "list",
            "--format=%gd%x1f%H%x1f%P%x1f%ct%x1f%ci%x1f%gs",
        ]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(String::from_utf8_lossy(&stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(6, '\u{1f}');
                let (name, oid) = (fields.next()?, fields.next()?);
                let parents = fields
                    .next()?
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                let time = fields.next()?.parse().unwrap_or(0);
                let local = fields.next()?.chars().take(16).collect();
                Some(Stash {
                    name: name.to_owned(),
                    oid: oid.to_owned(),
                    parents,
                    time,
                    local,
                    message: fields.next()?.to_owned(),
                })
            })
            .collect())
    }

    /// 对提交号为 `oid` 的储藏执行 `git stash <verb>`，返回 `git` 的结论。
    ///
    /// `verb` 是 `apply`、`pop` 或 `drop`。`git stash pop`、`drop` 只接受 `stash@{n}` 形式的名字，
    /// 编号会随储藏的增删改变，因此执行前按提交号查出当前的名字。
    ///
    /// # Errors
    ///
    /// 储藏已不存在时返回说明这一点的文案；其余情况同 [`Git::operate`]。
    pub fn stash(&self, root: &Path, verb: &str, oid: &str) -> Result<String, String> {
        let stashes = self.stashes(root, &Cancel::default())?;
        let Some(stash) = stashes.iter().find(|s| s.oid == oid) else {
            return Err(winc_i18n::t!("git-stash-gone").to_owned());
        };
        self.operate(root, &["stash", verb, "--end-of-options", &stash.name])
    }

    /// 逐行追溯：返回每一行最后是哪次提交改的。
    ///
    /// `contents` 是编辑器里该文件当前的内容（含未保存的修改），改过还没提交的行算「未提交」。
    /// 它整个移交给写标准输入的线程，不再复制，因此行号和编辑器里的行号一一对应。
    ///
    /// # Errors
    ///
    /// `git` 运行失败（文件没被跟踪、仓库还没有提交）时返回原因。
    pub fn blame(
        &self,
        root: &Path,
        path: &Path,
        contents: Vec<u8>,
        cancel: &Cancel,
    ) -> Result<Blame, String> {
        let relative = slash_path(root, path)?;
        let mut command = self.command(root);
        command
            // 不使用 textconv：转换后的行与编辑器中的行不对应，转换命令也来自仓库的配置。
            .args([
                "blame",
                "--no-textconv",
                "--porcelain",
                "--contents",
                "-",
                "--",
            ])
            .arg(relative)
            .stderr(Stdio::piped());
        match process::run(&mut command, contents, LOG_TIMEOUT, cancel) {
            Ok(output) if output.status.success() => {
                Ok(parse_blame(&String::from_utf8_lossy(&output.stdout)))
            }
            Ok(output) => Err(failure(&output.stderr)),
            Err(e) => Err(describe(&e)),
        }
    }

    /// 提交 `oid` 的完整说明。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn commit_message(
        &self,
        root: &Path,
        oid: &str,
        cancel: &Cancel,
    ) -> Result<String, String> {
        let mut command = self.command(root);
        command.args(["show", "-s", "--format=%B", "--end-of-options", oid]);
        let stdout = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        Ok(String::from_utf8_lossy(&stdout).trim_end().to_owned())
    }

    /// 返回提交 `oid` 相对于 `parent`（根提交传 `None`）改动的文件及各文件的增删行数，按路径
    /// 排序。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn commit_files(
        &self,
        root: &Path,
        parent: Option<&str>,
        oid: &str,
        cancel: &Cancel,
    ) -> Result<Vec<CommitFile>, String> {
        // 文件和行数一次取齐：`--raw` 的各条在前，`--numstat` 的各条按同样的顺序跟在后面。
        let mut command = self.command(root);
        command.args([
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--no-commit-id",
            "--raw",
            "--numstat",
        ]);
        // 提交号来自日志，分支名来自仓库：加 `--end-of-options`，以 `-` 开头的参数不会被当作选项。
        match parent {
            Some(parent) => command.args(["--end-of-options", parent, oid]),
            // 根提交和空树比；不写空树的对象号：SHA-1 和 SHA-256 的仓库里该对象号不同。
            None => command.args(["--root", "--end-of-options", oid]),
        };
        let stdout = self.run(&mut command, SHOW_TIMEOUT, cancel)?;
        let mut files = parse_diff_tree(&stdout, root);
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// 文件 `path` 在版本 `rev`（提交号、分支名……；空字符串是暂存区）里的内容，套用检出时
    /// 的转换；该版本里没有该文件时为 `None`。
    ///
    /// # Errors
    ///
    /// `git` 运行失败（版本不存在、对象损坏、必需的过滤器失败……）、超时或被取消时返回原因。
    pub fn content_at(
        &self,
        root: &Path,
        rev: &str,
        path: &Path,
        cancel: &Cancel,
    ) -> Result<Option<Vec<u8>>, String> {
        let relative = slash_path(root, path)?;
        let mut command = self.command(root);
        command
            // `--filters` 套用 `git` 检出时的转换（autocrlf 的换行、LFS、textconv 以外的
            // 过滤器），取到的内容和检出到工作区的文件相同，才能和工作区逐行比较。
            .args(["cat-file", "--filters", "--end-of-options"])
            .arg(joined_arg(&[OsStr::new(rev), OsStr::new(":"), &relative]))
            // 要按 `git` 的报错认出「没有该文件」，报错不能被翻译成用户的语言。
            .env("LC_ALL", "C")
            .stderr(Stdio::piped());
        match process::run(&mut command, Vec::new(), SHOW_TIMEOUT, cancel) {
            Ok(output) if output.status.success() => Ok(Some(output.stdout)),
            Ok(output) if output.status.code() == Some(128) && absent(&output.stderr, rev) => {
                Ok(None)
            }
            Ok(output) => Err(failure(&output.stderr)),
            Err(e) => Err(describe(&e)),
        }
    }
}
