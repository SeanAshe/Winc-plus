//! 列出远端主机上一个目录中的文件：在那台主机上运行一条命令，一次往返。
//!
//! 快速打开的文件表和全局搜索都要知道目录中有哪些文件。逐个目录列出要每个目录一次往返，
//! 忽略规则的文件也都在远端，本机的遍历用不上。是 Git 仓库时用 `git`（已跟踪的和未被忽略的
//! 未跟踪文件，规则与本机的遍历相同）；不是仓库或远端没有 `git` 时用 `find`，只跳过
//! [`HIDDEN`] 中的名字。

use std::path::Path;

use winc_backend::remote::{Exec, exec};

use super::tree::HIDDEN;

/// 命令输出的字节数上限（64 MiB）：二十万个文件、每个路径数百字节以内。
const MAX_LISTING: usize = 64 << 20;

/// 只列出含有某段文字的文件。
pub(crate) struct Containing<'a> {
    /// 要含有的文字：单行，按字面比较。
    pub text: &'a str,
    /// 是否区分大小写。
    pub case_sensitive: bool,
}

/// 返回远端目录 `root` 中各文件相对于它的路径（以 `/` 分隔），顺序不定。
///
/// `ignored` 为真时也列出被忽略规则排除的文件。`containing` 给出时只列出含有那段文字的
/// 文件：由远端的 `grep` 判断，结果可能多列（例如二进制文件），不会少列。`stop` 返回真时结束
/// 命令并返回空表。名字不是有效 UTF-8、或含 [`HIDDEN`] 中的名字的路径不列出。
pub(crate) fn list(
    root: &Path,
    ignored: bool,
    containing: Option<&Containing<'_>>,
    stop: &dyn Fn() -> bool,
) -> Vec<String> {
    let run = |command: Exec| exec(root, &command.limit(MAX_LISTING), stop).ok();
    // `git grep` 没有匹配时以 1 退出，输出为空；其余非零的退出码是没能运行（不是仓库等）。
    let from_git = |command: Exec| {
        let output = run(command)?;
        let found = output.code == Some(0) || containing.is_some() && output.code == Some(1);
        found.then_some(output.stdout)
    };
    let git = match containing {
        None => {
            let command = Exec::new("git").arg("ls-files").arg("-z");
            let command = command.arg("--cached").arg("--others");
            if ignored {
                command
            } else {
                command.arg("--exclude-standard")
            }
        }
        Some(wanted) => {
            let mut command = Exec::new("git").arg("grep").arg("-lzF").arg("--untracked");
            if !wanted.case_sensitive {
                command = command.arg("-i");
            }
            if ignored {
                command = command.arg("--no-exclude-standard");
            }
            command.arg("-e").arg(wanted.text)
        }
    };
    let (listing, separator) = match from_git(git) {
        Some(listing) => (listing, 0),
        None => {
            // `find . ( -name A -o -name B … ) -prune -o -type f …`
            let mut find = Exec::new("find").arg(".").arg("(");
            for (index, name) in HIDDEN.iter().enumerate() {
                if index > 0 {
                    find = find.arg("-o");
                }
                find = find.arg("-name").arg(name);
            }
            find = find.arg(")").arg("-prune").arg("-o").arg("-type").arg("f");
            match containing {
                None => {
                    let listing = run(find.arg("-print0")).map(|output| output.stdout);
                    (listing.unwrap_or_default(), 0)
                }
                // `grep -l` 逐行输出文件名：名字中含换行的文件无法分辨，不会被搜索到。`find`
                // 在某一批没有匹配时以非零退出，因此不看它的退出码。
                Some(wanted) => {
                    let mut find = find.arg("-exec").arg("grep").arg("-lF");
                    if !wanted.case_sensitive {
                        find = find.arg("-i");
                    }
                    find = find.arg("-e").arg(wanted.text).arg("{}").arg("+");
                    let listing = run(find).map(|output| output.stdout);
                    (listing.unwrap_or_default(), b'\n')
                }
            }
        }
    };
    if stop() {
        return Vec::new();
    }
    (listing.split(|&byte| byte == separator))
        .filter_map(|path| std::str::from_utf8(path).ok())
        .map(|path| path.strip_prefix("./").unwrap_or(path))
        .filter(|path| !path.is_empty() && !path.split('/').any(|part| HIDDEN.contains(&part)))
        .map(str::to_owned)
        .collect()
}
