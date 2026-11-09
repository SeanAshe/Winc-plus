//! 会改动仓库的 `git` 命令的输出记录。
//!
//! 失败提示只显示一行原因，钩子和远端的完整输出保存在这里。记录只存在于内存中，进程退出后不再保留；
//! 地址中的凭据在写入前被替换。

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// 保留的命令数上限（条）。
///
/// 与 [`TEXT_LIMIT`] 共同限定内存占用，上限约为 12.8 MiB。
const RECORD_LIMIT: usize = 200;
/// 每条命令保留的输出长度（字节）；超过时保留末尾，失败原因通常位于输出的最后。
const TEXT_LIMIT: usize = 64 << 10;

static RECORDS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// 记录在仓库 `root` 中运行 `git args` 的结果：`code` 是退出码，进程未能正常结束时为 `None`。
pub(super) fn record<S: AsRef<OsStr>>(root: &Path, args: &[S], code: Option<i32>, output: &[u8]) {
    let output = String::from_utf8_lossy(output).replace('\0', "");
    let entry = entry(root, args, code, &output);
    let mut records = RECORDS.lock().unwrap_or_else(PoisonError::into_inner);
    if records.len() == RECORD_LIMIT {
        records.pop_front();
    }
    records.push_back(entry);
}

/// 返回全部记录，最早的在前；没有记录时返回空字符串。
pub fn output_text() -> String {
    let records = RECORDS.lock().unwrap_or_else(PoisonError::into_inner);
    records
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 返回一条命令结果的文本；参数中不是 UTF-8 的部分按有损转换显示。
fn entry<S: AsRef<OsStr>>(root: &Path, args: &[S], code: Option<i32>, output: &str) -> String {
    let args: Vec<_> = args.iter().map(|a| a.as_ref().to_string_lossy()).collect();
    let command = redact(&args.join(" "));
    // 先替换再截断：截断点落在地址中间时，剩下的凭据前面已经没有 `://`，认不出来。
    let output = redact(output.trim_end());
    let output = tail(&output, TEXT_LIMIT);
    let status = match code {
        Some(code) => winc_i18n::t!("git-output-exit-code", code = code.to_string()),
        None => winc_i18n::t!("git-output-no-exit-code").to_owned(),
    };
    let mut entry = format!("{}\n$ git {command}\n", root.display());
    if !output.is_empty() {
        entry.push_str(output);
        entry.push('\n');
    }
    entry.push_str(&status);
    entry.push('\n');
    entry
}

/// 返回 `text` 最后不超过 `limit` 字节的部分，从字符边界开始。
fn tail(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// 把 `text` 中地址的用户信息（`scheme://用户:密码@主机` 的 `用户:密码`）替换为 `***`。
///
/// 只处理 `://` 之后、下一个空白或 `/` 之前以 `@` 结尾的部分；`git@host:path` 不含密码，不作替换。
fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail.find(|c: char| c.is_whitespace() || c == '/');
        let authority = &tail[..end.unwrap_or(tail.len())];
        match authority.rfind('@') {
            Some(user) => {
                out.push_str("***");
                rest = &tail[user..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests;
