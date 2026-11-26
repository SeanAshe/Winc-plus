//! 将要提交的改动与最近提交的首行，用于生成提交信息。
//!
//! 补丁边读边截断：每个文件只保留开头一段，读满规定的文件数后终止 `git`。占用的内存只取决于
//! [`PatchLimits`]，与改动的大小无关。

use std::time::Instant;

use super::*;

/// 每次从 `git diff` 的输出中读取的行数。
///
/// 取自调参：每次读取之间检查是否已取消，几百行的间隔既能及时响应取消，也不频繁唤醒读取线程。
const CHUNK_LINES: usize = 256;

/// 读取补丁时的上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchLimits {
    /// 统计中最多列出的文件数（个），其余文件合并为一行。
    pub stat_files: usize,
    /// 每个文件的补丁最多保留的字节数；补丁的首行总是保留。
    pub keep: usize,
    /// 最多读取补丁的文件数（个）；达到后停止读取。
    pub max_files: usize,
}

/// 一个文件的补丁：保留的开头部分和完整补丁的大小。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// 保留的开头部分，由完整的行组成，首行是 `diff --git` 行。
    pub text: String,
    /// 完整补丁的字节数；大于 `text` 的长度时表示有省略。
    pub size: usize,
}

/// 将要提交的改动：统计和各文件的补丁。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingPatch {
    /// 各文件的增删统计（`git diff --stat`），含末尾的合计行。
    pub stat: String,
    /// 各文件的补丁，顺序与 `git diff` 的输出相同。
    pub files: Vec<FilePatch>,
    /// 是否因文件数达到 [`PatchLimits::max_files`] 而未读完。
    pub partial: bool,
}

impl Git {
    /// 读取将要提交的改动，按 `limits` 截断。`staged`、`paths` 同 [`Git::added_lines`]，同样
    /// 不含未跟踪的文件；仓库尚无提交时读取暂存区的内容。最多等待 30 秒，可经
    /// `cancel` 取消。
    ///
    /// # Errors
    ///
    /// `git` 报错、超时或已取消时返回说明。
    pub fn pending_patch(
        &self,
        root: &Path,
        staged: bool,
        paths: &[PathBuf],
        limits: PatchLimits,
        cancel: &Cancel,
    ) -> Result<PendingPatch, String> {
        let base = if staged || !self.born(root, cancel) {
            "--cached"
        } else {
            "HEAD"
        };
        let deadline = Instant::now() + STATUS_TIMEOUT;
        let mut patch = PendingPatch::default();
        for batch in path_batches(root, paths) {
            let mut command = self.command(root);
            command.args(["diff", "--no-color", "--no-ext-diff", "--find-renames"]);
            command.args(["--no-textconv", "--ignore-submodules=dirty", "--stat"]);
            command.arg(format!("--stat-count={}", limits.stat_files));
            command.args(["--patch", base, "--"]).args(batch);
            let mut stream = process::Stream::spawn(&mut command).map_err(|e| describe(&e))?;
            if read_patch(&mut stream, &mut patch, limits, deadline, cancel)? {
                // 释放 `stream` 时终止仍在输出的 `git`。
                patch.partial = true;
                break;
            }
        }
        Ok(patch)
    }

    /// 返回最近 `count` 个非合并提交的首行，最近的在前；仓库尚无提交时为空。
    ///
    /// # Errors
    ///
    /// `git` 报错、已取消时返回说明。
    pub fn recent_subjects(
        &self,
        root: &Path,
        count: usize,
        cancel: &Cancel,
    ) -> Result<Vec<String>, String> {
        if !self.born(root, cancel) {
            return Ok(Vec::new());
        }
        let mut command = self.command(root);
        command.args(["log", "--no-merges", "--format=%s"]);
        command.arg(format!("--max-count={count}"));
        let stdout = self.run(&mut command, STATUS_TIMEOUT, cancel)?;
        let text = String::from_utf8_lossy(&stdout);
        Ok(text.lines().map(str::to_owned).collect())
    }
}

/// 把 `stream` 中一次 `git diff --stat --patch` 的输出读入 `patch`；返回是否因文件数达到上限而
/// 提前停止。
///
/// # Errors
///
/// `git` 报错、在 `deadline` 前未读完或已取消时返回说明。
fn read_patch(
    stream: &mut process::Stream,
    patch: &mut PendingPatch,
    limits: PatchLimits,
    deadline: Instant,
    cancel: &Cancel,
) -> Result<bool, String> {
    // 输出先是统计，第一个 `diff --git` 行之后是各文件的补丁。
    let mut in_stat = true;
    loop {
        if cancel.is_cancelled() {
            return Err(describe(&RunError::Cancelled));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let (bytes, done) =
            (stream.read_records(b'\n', CHUNK_LINES, left)).map_err(|e| match e {
                process::StreamError::Run(e) => describe(&e),
                process::StreamError::Exited(stderr) => failure(&stderr),
            })?;
        for line in String::from_utf8_lossy(&bytes).split_inclusive('\n') {
            if line.starts_with("diff --git ") {
                if patch.files.len() >= limits.max_files {
                    return Ok(true);
                }
                in_stat = false;
                patch.files.push(FilePatch {
                    text: String::new(),
                    size: 0,
                });
            }
            match patch.files.last_mut() {
                Some(file) if !in_stat => keep_line(file, line, limits.keep),
                _ => patch.stat.push_str(line),
            }
        }
        if done {
            return Ok(false);
        }
    }
}

/// 把补丁的一行 `line` 计入 `file`：首行总是保留；其后的行在保留的部分不超过 `keep` 字节、且之前
/// 没有省略过时保留，保留的部分总是补丁的连续开头。
fn keep_line(file: &mut FilePatch, line: &str, keep: usize) {
    let whole = file.text.len() == file.size;
    file.size += line.len();
    if whole && (file.text.is_empty() || file.text.len() + line.len() <= keep) {
        file.text.push_str(line);
    }
}
