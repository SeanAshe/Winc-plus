//! 创建仓库（初始化、克隆）和管理远程仓库。

use super::*;

/// 克隆的时长上限（秒）：大型仓库经普通网络下载可能需要数十分钟，超时只用于结束失去响应的进程。
const CLONE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// 删除标签的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTarget {
    /// 本地仓库。
    Local,
    /// 默认的远程仓库（[`Git::default_remote`]）。
    Remote,
}

/// 配置的一个远程仓库。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// 名称。
    pub name: String,
    /// 获取地址。
    pub url: String,
}

/// 返回克隆 `url` 时默认的目录名：地址最后一段去掉 `.git`；无法得到合法目录名时返回 `None`。
///
/// 支持 `scheme://host/a/b.git`、`git@host:a/b.git` 和本地路径。
pub fn repository_dir_name(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let last = url.rsplit(['/', '\\', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    let valid = !name.is_empty() && name != "." && name != ".." && !name.starts_with('-');
    valid.then(|| name.to_owned())
}

/// 解析 `git remote -v` 的输出，每个远程只取获取地址。
fn parse_remotes(text: &str) -> Vec<Remote> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let url = rest.strip_suffix(" (fetch)")?;
            Some(Remote {
                name: name.to_owned(),
                url: url.to_owned(),
            })
        })
        .collect()
}

impl Git {
    /// 在目录 `dir` 中初始化仓库，返回 `git` 的结论。
    ///
    /// 目录还不是仓库，没有需要信任的配置，因此不做信任检查。
    ///
    /// # Errors
    ///
    /// `git` 运行失败时返回原因。
    pub fn init(&self, dir: &Path) -> Result<String, String> {
        self.run_operation(dir, &["init"], OPERATION_TIMEOUT)
    }

    /// 在刚初始化的目录 `dir` 里生成一份 `.gitignore`；已经有这个文件时不动它。
    ///
    /// 内容按目录里的项目文件定（见 `starter_ignore`）。返回是否写了文件。
    pub fn write_starter_ignore(&self, dir: &Path) -> bool {
        let file = dir.join(".gitignore");
        let paths: Vec<_> = (std::iter::once(".gitignore").chain(IGNORE_MARKERS))
            .map(|name| dir.join(name))
            .collect();
        let found = winc_backend::fs::metadata_many(&paths);
        if found.first().is_some_and(Result::is_ok) {
            return false;
        }
        let present = (IGNORE_MARKERS.into_iter().zip(found.iter().skip(1)))
            .filter(|(_, metadata)| metadata.is_ok())
            .map(|(name, _)| name);
        let text = starter_ignore(&present.collect::<Vec<_>>());
        winc_backend::fs::write(&file, text).is_ok()
    }

    /// 把 `url` 克隆到目录 `parent` 下的 `dir`（相对路径），返回 `git` 的结论。
    ///
    /// 新仓库的配置由 `git clone` 生成，不含远端内容，因此不做信任检查；`ext` 协议能运行任意命令，
    /// 不允许使用。
    ///
    /// # Errors
    ///
    /// 地址不可用、认证失败、目标目录不为空时返回 `git` 给出的原因。
    pub fn clone_repository(&self, parent: &Path, url: &str, dir: &str) -> Result<String, String> {
        let args = ["-c", "protocol.ext.allow=never", "clone", "--", url, dir];
        self.run_operation(parent, &args, CLONE_TIMEOUT)
    }

    /// 返回仓库 `root` 配置的远程仓库。
    ///
    /// # Errors
    ///
    /// `git` 运行失败或超时时返回原因。
    pub fn remote_list(&self, root: &Path, cancel: &Cancel) -> Result<Vec<Remote>, String> {
        let mut command = self.command(root);
        command.args(["remote", "--verbose"]);
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        Ok(parse_remotes(&String::from_utf8_lossy(&stdout)))
    }

    /// 把标签 `name` 推送到默认的远程仓库，返回 `git` 的结论。
    ///
    /// 远程名来自仓库配置，可能以 `-` 开头，因此放在 `--end-of-options` 之后，不能被解析成选项。
    ///
    /// # Errors
    ///
    /// 没有配置远程仓库或 `git` 运行失败时返回原因。
    pub fn push_tag(&self, root: &Path, name: &str) -> Result<String, String> {
        let remote = self.default_remote(root)?;
        let spec = format!("refs/tags/{name}");
        self.operate(root, &["push", "--end-of-options", &remote, &spec])
    }

    /// 从默认的远程仓库删除标签 `name`，返回 `git` 的结论；远程名的处理同 [`Git::push_tag`]。
    ///
    /// # Errors
    ///
    /// 没有配置远程仓库或 `git` 运行失败时返回原因。
    pub fn delete_remote_tag(&self, root: &Path, name: &str) -> Result<String, String> {
        let remote = self.default_remote(root)?;
        let spec = format!("refs/tags/{name}");
        let args = ["push", "--delete", "--end-of-options", &remote, &spec];
        self.operate(root, &args)
    }

    /// 返回推送和删除标签使用的远程仓库：优先 `origin`，否则第一个。
    ///
    /// # Errors
    ///
    /// 没有配置远程仓库或 `git` 运行失败时返回原因。
    pub fn default_remote(&self, root: &Path) -> Result<String, String> {
        let remotes = self.remotes(root, &Cancel::default())?;
        let origin = remotes.iter().find(|r| *r == "origin").or(remotes.first());
        origin
            .cloned()
            .ok_or_else(|| winc_i18n::t!("git-no-remote").to_owned())
    }
}

#[cfg(test)]
mod tests;

/// 用来判断项目类型的文件，见 `starter_ignore`。
const IGNORE_MARKERS: [&str; 7] = [
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
];

/// 返回新仓库的 `.gitignore` 的内容：系统生成的杂项文件，加上 `markers`（目录里存在的项目
/// 文件）对应的构建产物和依赖目录。
///
/// 只列各类项目里公认不入库的目录，不替用户决定其余的文件。
pub fn starter_ignore(markers: &[&str]) -> String {
    let has = |name: &str| markers.contains(&name);
    let mut lines = vec![".DS_Store", "Thumbs.db"];
    if has("Cargo.toml") {
        lines.push("/target/");
    }
    if has("package.json") {
        lines.push("node_modules/");
    }
    if has("pyproject.toml") || has("requirements.txt") {
        lines.extend(["__pycache__/", "*.pyc", ".venv/"]);
    }
    if has("pom.xml") {
        lines.push("target/");
    }
    if has("build.gradle") {
        lines.extend([".gradle/", "build/"]);
    }
    lines.dedup();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}
