//! 用模型生成提交信息：将要提交的改动和最近提交的首行发送给设置中的模型，回答填入提交信息框。
//!
//! 读取改动时每个文件只保留开头一段，再把 [`PATCH_LIMIT`] 的篇幅在文件之间平均分配。生成期间
//! 再次执行即取消；填入信息框是一次编辑，可以撤销。

use std::fmt::Write;

use winc_editor::ai::{self, AiError, Endpoint};
use winc_editor::git::{FilePatch, Head, PatchLimits, PendingPatch};
use winc_editor::shell::Cancel;

use super::compose::PendingScope;
use super::*;
use crate::app::NoticeKind;
use crate::app::lifecycle::notice::NoticeAction;
use crate::layout::Setting;

/// 发送给模型参照的最近提交的条数。
///
/// 取自调参：10 条已能体现提交信息的语言和前缀，再增加只会加长请求。
const RECENT_SUBJECTS: usize = 10;
/// 读取改动时每个文件最多保留的字节数，也是分配篇幅时一个文件能得到的上限。
///
/// 取自调参：大段改动的开头已足以概括其内容。
const FILE_LIMIT: usize = 16 << 10;
/// 补丁合计最多发送的字节数，在文件之间分配（见 [`shares`]）。
///
/// 取自调参：按每个 token 约 4 字节估算约为 1.6 万 token，兼顾概括所需的内容与请求的耗时和费用。
const PATCH_LIMIT: usize = 64 << 10;
/// 最多读取补丁的文件数（个），读取改动的内存不超过此数乘以 [`FILE_LIMIT`]。
///
/// 有约束：文件更多时每个文件平均分到的篇幅不足 1 KB，统计已能体现改动的范围。
const MAX_FILES: usize = 64;
/// 统计中最多列出的文件数（个）。
///
/// 取自调参：每行约 80 字节，100 行约 8 KB，与补丁的篇幅相比较小。
const STAT_FILES: usize = 100;

/// 给模型的指示。
const INSTRUCTIONS: &str = "You write the Git commit message for the changes below. Reply \
with the message only: a subject line of at most 72 characters, then, only if the change needs \
explaining, a blank line and a short body wrapped at 72 characters. Follow the language and \
conventions of the recent commit subjects, such as a `type: subject` prefix; when there are \
none, write in {language}. Say what changed and why rather than listing files. Do not use \
Markdown, quotes or code fences.";

/// 生成提交信息的进度。
#[derive(Debug, Default)]
pub(crate) struct Generating {
    /// 下一次生成的编号：结果按编号匹配，取消或切换仓库之后到达的结果丢弃。
    next: u64,
    /// 正在进行的生成：编号和用于取消的句柄。
    running: Option<(u64, Cancel)>,
}

impl Generating {
    /// 是否正在生成。
    pub(crate) fn running(&self) -> bool {
        self.running.is_some()
    }

    /// 切换仓库时调用：取消正在进行的生成，保留编号。
    pub(super) fn carried(self) -> Generating {
        if let Some((_, cancel)) = &self.running {
            cancel.cancel();
        }
        Generating {
            next: self.next,
            running: None,
        }
    }
}

impl App {
    /// 是否开启了「AI 生成提交信息」。
    pub(crate) fn commit_generation_on(&self) -> bool {
        self.config.git_generate_commit_message == Some(true)
    }

    /// 生成提交信息；正在生成时取消。未开启、未设置模型或未选择更改时提示原因，前两种提示附带
    /// 「打开设置」，定位到对应的设置项。
    pub(crate) fn generate_commit_message(&mut self) {
        if self.git.root.is_none() {
            return;
        }
        if let Some((_, cancel)) = self.git.commit.generating.running.take() {
            cancel.cancel();
            self.request_redraw();
            return;
        }
        if !self.commit_generation_on() {
            let action = NoticeAction::ShowSetting(Setting::GitGenerateCommitMessage);
            self.notify_with(NoticeKind::Warning, t!("git-generate-off"), action);
            return;
        }
        let Some(endpoint) = Endpoint::from_config(&self.config) else {
            let action = NoticeAction::ShowSetting(Setting::AiModel);
            self.notify_with(NoticeKind::Warning, t!("git-generate-unset"), action);
            return;
        };
        let Some(scopes) = self.commit_scopes() else {
            self.notify(NoticeKind::Info, t!("git-generate-nothing"));
            return;
        };
        let branch = match self.git.status.as_ref().and_then(|s| s.head.as_ref()) {
            Some(Head::Branch(name)) => Some(name.clone()),
            _ => None,
        };
        let generating = &mut self.git.commit.generating;
        let id = generating.next;
        generating.next += 1;
        let cancel = Cancel::default();
        generating.running = Some((id, cancel.clone()));
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let result = git.and_then(|git| {
                let changes = changes(git, root, &scopes, &cancel)?;
                let subjects = git.recent_subjects(root, RECENT_SUBJECTS, &cancel)?;
                let input = prompt(branch.as_deref(), &subjects, &changes);
                let curl = crate::web::curl();
                let reply = ai::complete(&endpoint, &instructions(), &input, curl, &cancel);
                reply.map_err(describe)
            });
            let _ = sender.send(Outcome::Generated(id, result.map(|r| message_from(&r))));
        });
        self.request_redraw();
    }

    /// 处理第 `id` 次生成的结果：成功时替换提交信息（可以撤销），失败时提示原因。已取消或已切换
    /// 仓库时丢弃。
    pub(super) fn generated(&mut self, id: u64, result: Result<String, String>) {
        let generating = &mut self.git.commit.generating;
        if generating.running.as_ref().map(|(running, _)| *running) != Some(id) {
            return;
        }
        generating.running = None;
        match result {
            // 只换掉提交信息、不移动焦点：生成期间可能已经在别处输入。
            Ok(message) => {
                self.git.commit.message.set_text(&message);
                self.git.commit.message.set_caret(message.len(), false);
                self.reset_blink();
            }
            Err(e) => self.notify(NoticeKind::Error, t!("git-generate-failed", error = e)),
        }
        self.request_redraw();
    }
}

/// 返回给模型的指示：没有可参照的最近提交时，要求使用界面的语言。
fn instructions() -> String {
    let language = match winc_i18n::locale() {
        winc_i18n::Locale::ZhCn => "Simplified Chinese",
        winc_i18n::Locale::EnUs => "English",
    };
    INSTRUCTIONS.replace("{language}", language)
}

/// 返回发送给模型的内容：分支、最近提交的首行和将要提交的改动。
fn prompt(branch: Option<&str>, subjects: &[String], changes: &str) -> String {
    let mut input = String::new();
    if let Some(branch) = branch {
        let _ = writeln!(input, "Branch: {branch}\n");
    }
    if !subjects.is_empty() {
        input.push_str("Recent commit subjects:\n");
        for subject in subjects {
            let _ = writeln!(input, "- {subject}");
        }
        input.push('\n');
    }
    input.push_str("Changes to commit:\n");
    input.push_str(changes);
    input
}

/// 返回将要提交的改动：各仓库的统计和补丁，未跟踪的文件写为新增整个文件的补丁；同时提交多个
/// 仓库时先写仓库名。补丁的篇幅见 [`render`]。
fn changes(
    git: &Git,
    root: &Path,
    scopes: &[PendingScope],
    cancel: &Cancel,
) -> Result<String, String> {
    let mut parts = Vec::new();
    let mut files = 0;
    for scope in scopes {
        let repo = scope.repo.as_deref().unwrap_or(root);
        let limits = PatchLimits {
            stat_files: STAT_FILES,
            keep: FILE_LIMIT,
            max_files: MAX_FILES - files,
        };
        let mut patch = match scope.staged || !scope.tracked.is_empty() {
            true => git.pending_patch(repo, scope.staged, &scope.tracked, limits, cancel)?,
            false => PendingPatch::default(),
        };
        for path in &scope.untracked {
            if files + patch.files.len() >= MAX_FILES {
                patch.partial = true;
                break;
            }
            patch.files.push(new_file(repo, path));
        }
        files += patch.files.len();
        let name = (scopes.len() > 1).then(|| winc_editor::display_name(repo).into_owned());
        parts.push((name, patch));
    }
    Ok(render(&parts))
}

/// 返回未跟踪的文件 `path`（在仓库 `repo` 中）写为新增整个文件的补丁，只读取开头的
/// [`FILE_LIMIT`] 字节；二进制文件只写一行说明。
fn new_file(repo: &Path, path: &Path) -> FilePatch {
    let name = path.strip_prefix(repo).unwrap_or(path).display();
    let mut text = format!("diff --git a/{name} b/{name}\nnew file\n");
    let size = winc_backend::fs::metadata(path).map_or(0, |m| m.len as usize);
    let Ok((bytes, end)) = winc_backend::fs::read_range(path, 0, FILE_LIMIT as u32) else {
        text.push_str("Unreadable file\n");
        let size = text.len();
        return FilePatch { text, size };
    };
    if winc_editor::looks_binary(&bytes) {
        text.push_str("Binary file\n");
        let size = text.len();
        return FilePatch { text, size };
    }
    let content = String::from_utf8_lossy(&bytes);
    // 没有读到文件末尾时，最后一行可能不完整，不保留。
    let read = match end {
        true => &content[..],
        false => content.rfind('\n').map_or("", |at| &content[..=at]),
    };
    let _ = writeln!(text, "--- /dev/null\n+++ b/{name}");
    for line in read.lines() {
        let _ = writeln!(text, "+{line}");
    }
    // 未读的部分按原大小计入，每行开头的 `+` 忽略不计。
    let size = text.len() + size.saturating_sub(read.len());
    FilePatch { text, size }
}

/// 写出各仓库的统计和补丁：[`PATCH_LIMIT`] 的篇幅按 [`shares`] 分给各文件，每个文件写到分得的
/// 篇幅为止（至少写首行），省略的字节数写在其后。
fn render(parts: &[(Option<String>, PendingPatch)]) -> String {
    let sizes: Vec<usize> = (parts.iter())
        .flat_map(|(_, patch)| &patch.files)
        .map(|file| file.text.len())
        .collect();
    let mut shares = shares(&sizes, PATCH_LIMIT).into_iter();
    let mut out = String::new();
    for (name, patch) in parts {
        if let Some(name) = name {
            let _ = writeln!(out, "Repository {name}:");
        }
        let stat = patch.stat.trim_end();
        if !stat.is_empty() {
            let _ = writeln!(out, "{stat}\n");
        }
        for file in &patch.files {
            let share = shares.next().unwrap_or(0);
            let mut used = 0;
            for line in file.text.split_inclusive('\n') {
                if used > 0 && used + line.len() > share {
                    break;
                }
                used += line.len();
                out.push_str(line);
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
            let omitted = file.size.saturating_sub(used);
            if omitted > 0 {
                let _ = writeln!(out, "[{omitted} more bytes omitted]");
            }
        }
        if patch.partial {
            out.push_str("[Patches of the remaining files omitted]\n");
        }
    }
    out
}

/// 把 `budget` 字节的篇幅分给大小为 `sizes` 的各段，返回各段分得的字节数，顺序与 `sizes` 相同。
///
/// 从小到大依次分配，每段分得剩余篇幅的平均值与自身大小中较小的一个：小段完整保留，用不完的
/// 篇幅留给更大的段。
fn shares(sizes: &[usize], budget: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| sizes[i]);
    let mut shares = vec![0; sizes.len()];
    let mut left = budget;
    for (k, &i) in order.iter().enumerate() {
        let share = sizes[i].min(left / (sizes.len() - k));
        shares[i] = share;
        left -= share;
    }
    shares
}

/// 从模型的回答中取出提交信息：去掉外层的代码块标记和各行末尾的空白。
fn message_from(reply: &str) -> String {
    let mut text = reply.trim();
    if let Some(rest) = text.strip_prefix("```") {
        // 代码块的首行可能带有语言名，如 ```text。
        let body = rest.split_once('\n').map_or("", |(_, body)| body);
        text = body.trim_end().strip_suffix("```").unwrap_or(body).trim();
    }
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n")
}

/// 返回调用模型失败的原因，用于提示。
fn describe(error: AiError) -> String {
    match error {
        AiError::Send(e) => crate::app::editor::http::send_error(&e),
        AiError::Status(status, Some(message)) => format!("HTTP {status}: {message}"),
        AiError::Status(status, None) => format!("HTTP {status}"),
        AiError::Empty => t!("git-generate-empty").to_owned(),
    }
}

#[cfg(test)]
mod tests;
