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
