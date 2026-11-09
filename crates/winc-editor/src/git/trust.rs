//! 仓库配置的信任：找出会让 `git` 运行命令的设置和 hooks，记下用户信任了哪些仓库。
//!
//! 信任按 [`Risks::fingerprint`] 判断：配置或 hooks 变化以后指纹随之改变，需要用户重新确认。

use super::*;

/// 会让 `git` 运行命令的设置项，写成小写，`*` 代表任意一段名字。
///
/// 写法与 `git config` 输出的键名一致（小写，多级名字用 `.` 分隔）。只列可能运行任意命令的项；
/// `core.pager`、`core.editor` 这类只在交互时启动程序、后果较轻的同样列入，使后台进程不会
/// 启动它们。
const RISKY_KEYS: &[&str] = &[
    "core.fsmonitor",
    "core.hookspath",
    "core.sshcommand",
    "core.pager",
    "core.editor",
    "core.askpass",
    "core.gitproxy",
    "core.alternaterefscommand",
    "sequence.editor",
    "diff.external",
    "credential.helper",
    "credential.*.helper",
    "gpg.program",
    "gpg.*.program",
    "filter.*.clean",
    "filter.*.smudge",
    "filter.*.process",
    "diff.*.textconv",
    "diff.*.command",
    "merge.*.driver",
    "remote.*.uploadpack",
    "remote.*.receivepack",
    "remote.*.vcs",
    "uploadpack.packobjectshook",
    "protocol.allow",
    "protocol.*.allow",
];

/// `key`（`git config` 给出的小写名字）是否在 [`RISKY_KEYS`] 里。
fn risky_key(key: &str) -> bool {
    RISKY_KEYS
        .iter()
        .any(|pattern| match pattern.split_once(".*.") {
            Some((section, name)) => key
                .strip_prefix(section)
                .and_then(|k| k.strip_prefix('.'))
                .and_then(|k| k.rsplit_once('.'))
                .is_some_and(|(_, last)| last == name),
            None => key == *pattern,
        })
}

/// 从 `git config --list --show-scope -z` 的输出和 git 实际使用的 hooks 目录里找出 [`Risks`]。
///
/// 只看仓库自己的配置（`local`、`worktree` 范围，含它们 `include` 进来的文件）：系统和用户
/// 范围的配置由用户自己维护，视为用户的选择。`core.fsmonitor` 是布尔值时运行的是
/// `git` 自带的监视进程，不计入。
///
/// 钩子目录由 [`hooks_dir`] 确定，其中可执行的普通文件都计入，`.sample` 示例除外。
/// `root` 是工作树的根目录，`default` 是共用的 git 目录下的 `hooks`，
/// `home` 是 `root` 所在系统中用户的主目录。
pub(super) fn parse_risks(
    stdout: &[u8],
    root: &Path,
    default: Option<&Path>,
    home: Option<&Path>,
) -> Risks {
    let mut risks = Risks::default();
    // 指纹依次写入每项设置（`键\n值`）和每个 hook（名字、内容），各段之后跟一个 0 字节。
    // 记录顺序和分隔符不能改：指纹记在状态文件里，变了就要用户重新确认所有信任过的仓库。
    let mut fingerprint = Fnv::new();
    let mut entries = stdout.split(|&b| b == 0);
    while let (Some(scope), Some(entry)) = (entries.next(), entries.next()) {
        if scope != b"local" && scope != b"worktree" {
            continue;
        }
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, ""));
        if !risky_key(key) {
            continue;
        }
        if key == "core.fsmonitor"
            && matches!(
                value.to_ascii_lowercase().as_str(),
                "" | "true" | "false" | "yes" | "no" | "on" | "off" | "1" | "0"
            )
        {
            continue;
        }
        if let Some(name) = key
            .strip_prefix("filter.")
            .and_then(|k| k.rsplit_once('.'))
            .map(|(name, _)| name.to_owned())
            && !risks.filters.contains(&name)
        {
            risks.filters.push(name);
        }
        risks.items.push(format!("{key} = {value}"));
        fingerprint.write(entry.as_bytes());
        fingerprint.push(0);
    }

    // 先只列出名字排好序，再逐个读内容喂进指纹：同一时刻只有一个 hook 的内容在内存里。
    let (dir, label) = hooks_dir(stdout, root, default, home).unzip();
    let mut hooks: Vec<(String, PathBuf)> = dir
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let metadata = std::fs::metadata(&path).ok()?;
            (!name.ends_with(".sample") && metadata.is_file() && executable(&metadata))
                .then_some((name, path))
        })
        .collect();
    hooks.sort();
    for (name, path) in hooks {
        fingerprint.write(name.as_bytes());
        fingerprint.push(0);
        fingerprint.write(&std::fs::read(&path).unwrap_or_default());
        fingerprint.push(0);
        let label = label.as_deref().unwrap_or("hooks");
        risks.items.push(format!("{label}/{name}"));
    }

    risks.fingerprint = fingerprint.finish();
    risks
}

/// 返回 `git` 实际运行钩子的目录，以及列出钩子时写在名字前面的路径。
///
/// 目录取最后一个 `core.hooksPath`（各范围都算，后出现的覆盖先出现的），列出时写配置里的原文；
/// 没有设置时是 `default`，写作 `hooks`。相对路径从工作树的根目录 `root` 算起；`~/` 开头的路径
/// 从 `home` 算起，`git` 同样按仓库所在系统中用户的主目录展开。系统和用户自己的配置里写的
/// 绝对路径是用户自己的钩子目录，不属于仓库，返回 `None`。
fn hooks_dir(
    stdout: &[u8],
    root: &Path,
    default: Option<&Path>,
    home: Option<&Path>,
) -> Option<(PathBuf, String)> {
    let mut configured = None;
    let mut entries = stdout.split(|&b| b == 0);
    while let (Some(scope), Some(entry)) = (entries.next(), entries.next()) {
        let entry = String::from_utf8_lossy(entry);
        if let Some(value) = entry.strip_prefix("core.hookspath\n") {
            configured = Some((scope, value.to_owned()));
        }
    }
    let Some((scope, value)) = configured.filter(|(_, value)| !value.is_empty()) else {
        return default.map(|dir| (dir.to_path_buf(), "hooks".to_owned()));
    };
    let home = value
        .strip_prefix("~/")
        .and_then(|rest| Some(home?.join(rest)));
    let own = scope != b"local" && scope != b"worktree";
    let dir = match home {
        Some(_) if own => return None,
        Some(dir) => dir,
        None if Path::new(&value).is_absolute() && own => return None,
        None => root.join(&value),
    };
    let label = value.trim_end_matches('/').to_owned();
    Some((dir, label))
}

#[cfg(unix)]
fn executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

/// Windows 上的 `git` 不检查可执行位，钩子目录中的文件都会运行。
///
/// WSL 发行版中的仓库由发行版内的 `git` 运行钩子，不可执行的文件不会运行。Windows 无法读取
/// 这类文件的可执行位，因此同样计入：结果最多多出一次信任确认，不会遗漏实际运行的钩子。
#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    true
}

/// 仓库自己的配置里会让 `git` 运行命令的设置项和钩子。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Risks {
    /// 给用户看的每一项（`core.fsmonitor = …`、`hooks/pre-commit`）。
    pub items: Vec<String>,
    /// 仓库里的配置和 hooks 内容的哈希（FNV-1a，由 [`Fnv`] 计算）。
    ///
    /// 信任时记下该值；之后内容有变化时新算出的值与记录的不同，需要用户重新确认。
    pub fingerprint: u64,
    /// 仓库里定义的过滤器名（`filter.<名字>.*`）；仓库的配置不信任时清空这些过滤器的命令。
    pub(super) filters: Vec<String>,
}

impl Risks {
    /// 是否没有任何会运行命令的设置项和钩子；这样的仓库不需要询问用户。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 各仓库的配置检查结果和信任状态，[`Git`] 的各个副本共用。
#[derive(Debug, Default)]
pub(super) struct Trust {
    /// 最近一次检查出的 [`Risks`]，按仓库根目录。
    risks: HashMap<PathBuf, Arc<Risks>>,
    /// 用户信任的仓库和当时记下的 [`Risks::fingerprint`]。
    trusted: HashMap<PathBuf, u64>,
}

impl Git {
    /// 检查仓库 `root` 自己的配置（本地和工作树范围的设置、hooks 目录）里有没有会运行命令的
    /// 设置，记下结果供之后的命令使用。
    ///
    /// [`Git::status`] 每次都会先检查一遍。
    ///
    /// # Errors
    ///
    /// `git config` 运行失败时返回原因。
    pub fn inspect(&self, root: &Path) -> Result<Arc<Risks>, String> {
        let mut command = self.at(root);
        command.args(["-c", "core.fsmonitor=false"]).args([
            "config",
            "--list",
            "--show-scope",
            "-z",
        ]);
        let stdout = self.run(&mut command, SHOW_TIMEOUT, &Cancel::default())?;
        let hooks = git_common_dir(root).map(|d| d.join("hooks"));
        let home = crate::wsl::home_dir(root);
        let risks = Arc::new(parse_risks(
            &stdout,
            root,
            hooks.as_deref(),
            home.as_deref(),
        ));
        self.lock().risks.insert(root.to_path_buf(), risks.clone());
        Ok(risks)
    }

    /// 记下用户信任了仓库 `root` 的 `fingerprint` 所对应的配置。
    ///
    /// 之后的命令在检查出的指纹与它相同时照常运行；配置变化后指纹不同，需要重新确认。
    pub fn trust(&self, root: &Path, fingerprint: u64) {
        self.lock().trusted.insert(root.to_path_buf(), fingerprint);
    }

    /// 仓库 `root` 的配置是否可以照常使用：没有会运行命令的设置，或者用户信任了现在的设置。
    ///
    /// 还没检查过时先检查；检查失败时按不可信处理。
    ///
    /// 返回不可信时检查出的 [`Risks`]，可以照常使用时返回 `None`。
    pub fn untrusted(&self, root: &Path) -> Option<Arc<Risks>> {
        let cached = self.lock().risks.get(root).cloned();
        let risks = match cached {
            Some(risks) => risks,
            None => self.inspect(root).unwrap_or_else(|_| {
                Arc::new(Risks {
                    items: vec![winc_i18n::t!("git-trust-config-unreadable").to_owned()],
                    ..Risks::default()
                })
            }),
        };
        let trusted = self.lock().trusted.get(root) == Some(&risks.fingerprint);
        (!risks.is_empty() && !trusted).then_some(risks)
    }

    /// 在会改动仓库的操作之前调用，仓库的配置不可信时拒绝。
    ///
    /// # Errors
    ///
    /// 仓库的配置不可信时返回要求用户确认的说明，其中列出会运行命令的设置项和 hooks。
    pub(super) fn require_trust(&self, root: &Path) -> Result<(), String> {
        match self.untrusted(root) {
            None => Ok(()),
            Some(risks) => {
                let commands = risks.items.join(winc_i18n::t!("git-list-separator"));
                Err(winc_i18n::t!("git-trust-required", commands = commands))
            }
        }
    }

    /// 取锁；持锁的线程 panic 以后锁仍然可用，取回内层数据继续。
    fn lock(&self) -> std::sync::MutexGuard<'_, Trust> {
        self.trust.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
