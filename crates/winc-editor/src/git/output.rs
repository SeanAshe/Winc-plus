//! 会改动仓库的 `git` 命令的输出记录。
//!
//! 失败提示只显示一行原因，钩子和远端的完整输出保存在这里。命令一开始就记下，输出随到随记，
//! 开着的输出标签据此实时更新。记录只存在于内存中，进程退出后不再保留；地址中的凭据在写入前
//! 被替换。

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

/// 保留的命令数上限（条）。
///
/// 与 [`TEXT_LIMIT`] 共同限定内存占用，上限约为 12.8 MiB。
const RECORD_LIMIT: usize = 200;
/// 每条命令保留的输出长度（字节）；运行结束后才记录的输出超过时保留末尾，失败原因通常位于输出
/// 的最后；边运行边记录的输出到这个长度为止。
const TEXT_LIMIT: usize = 64 << 10;
/// 全部记录保留的长度（字节）：超过时从最早的一行起丢弃。
const LOG_LIMIT: usize = RECORD_LIMIT * TEXT_LIMIT;

/// 到现在为止的全部记录。
struct Log {
    /// 还保留着的文本。
    text: String,
    /// 已经从开头丢弃的字节数：加上 `text` 的长度就是写入过的总字节数。
    dropped: u64,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    text: String::new(),
    dropped: 0,
});

/// 有新内容写入时要通知的各方；返回 `false` 的不再通知（窗口已经关闭）。
type Listener = Box<dyn Fn() -> bool + Send>;
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// 登记一个在记录有新内容时调用的 `listener`：它在写入的线程上调用，应当只把事情转给界面线程；
/// 返回 `false` 时不再调用。
pub fn on_output(listener: impl Fn() -> bool + Send + 'static) {
    let mut listeners = LISTENERS.lock().unwrap_or_else(PoisonError::into_inner);
    listeners.push(Box::new(listener));
}

/// 把 `text` 接在记录末尾，超出 [`LOG_LIMIT`] 时从最早的整行起丢弃，然后通知各方。
fn append(text: &str) {
    if text.is_empty() {
        return;
    }
    {
        let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
        log.text.push_str(text);
        let cut = trim_front(&mut log.text, LOG_LIMIT);
        log.dropped += cut as u64;
    }
    let mut listeners = LISTENERS.lock().unwrap_or_else(PoisonError::into_inner);
    listeners.retain(|listener| listener());
}

/// `text` 超过 `limit` 字节时从开头起丢弃整行，直到不超过为止，返回丢弃的字节数。
///
/// 按字节找换行：超出的位置可能落在多字节字符中间，不能在那里切字符串；换行之后总是字符边界。
fn trim_front(text: &mut String, limit: usize) -> usize {
    if text.len() <= limit {
        return 0;
    }
    let over = text.len() - limit;
    let cut = (text.as_bytes()[over..].iter())
        .position(|&b| b == b'\n')
        .map_or(text.len(), |at| over + at + 1);
    text.drain(..cut);
    cut
}

/// 两条命令之间的空行：记录不为空时在新的一条前面加一个换行。
fn separator() -> &'static str {
    let log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    if log.text.is_empty() { "" } else { "\n" }
}

/// 记录在仓库 `root` 中运行 `git args` 的结果：`code` 是退出码，进程未能正常结束时为 `None`。
///
/// 用于运行结束后才知道输出的命令；能边运行边取到输出的命令用 [`begin`]。
pub(super) fn record<S: AsRef<OsStr>>(root: &Path, args: &[S], code: Option<i32>, output: &[u8]) {
    let output = String::from_utf8_lossy(output).replace('\0', "");
    append(&format!(
        "{}{}",
        separator(),
        entry(root, args, code, &output)
    ));
}

/// 一条正在运行的命令的记录：输出随到随记，结束时补上退出码。
pub(super) struct Live {
    /// 还没有凑成整行的输出。按整行写入：地址中的凭据不会被两次写入从中间分开而漏过替换。
    pending: Vec<u8>,
    /// 这条命令还可以写入的字节数，见 [`TEXT_LIMIT`]。
    room: usize,
    /// 是否已经写入过输出。
    written: bool,
}

/// 开始记录在仓库 `root` 中运行的 `git args`：立刻写下仓库和命令两行，返回用来接着写输出的记录。
pub(super) fn begin<S: AsRef<OsStr>>(root: &Path, args: &[S]) -> Arc<Mutex<Live>> {
    append(&format!("{}{}", separator(), header(root, args)));
    Arc::new(Mutex::new(Live {
        pending: Vec::new(),
        room: TEXT_LIMIT,
        written: false,
    }))
}

/// 把命令刚输出的 `bytes` 记下：凑成整行的部分立刻写入，剩下的等后面的输出。
pub(super) fn feed(live: &Mutex<Live>, bytes: &[u8]) {
    let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
    live.pending.extend_from_slice(bytes);
    // 进度信息用回车覆盖同一行：这里每次更新各占一行。
    let Some(end) = (live.pending.iter()).rposition(|b| matches!(b, b'\n' | b'\r')) else {
        return;
    };
    let lines: Vec<u8> = live.pending.drain(..=end).collect();
    let text = live_text(&mut live, &lines);
    append(&text);
}

/// 返回 `bytes` 作为这条命令的输出要写入记录的文本：凭据已经替换，不超过它剩下的长度。
fn live_text(live: &mut Live, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).replace('\0', "");
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = redact(&text);
    if text.is_empty() || live.room == 0 {
        return String::new();
    }
    let mut end = text.len().min(live.room);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    live.room -= end;
    live.written = true;
    let mut shown = text[..end].to_owned();
    if end < text.len() {
        shown.push_str("…\n");
    }
    shown
}

/// 结束 `live` 这条命令的记录：补上还没写入的输出和退出码，一次写入。`output` 是命令的全部
/// 输出，运行途中一点都没有取到（在远端运行的命令）时写它的末尾。
pub(super) fn finish(live: &Mutex<Live>, code: Option<i32>, output: &[u8]) {
    let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
    let rest = std::mem::take(&mut live.pending);
    let mut text = if live.written || !rest.is_empty() {
        live_text(&mut live, &rest)
    } else {
        let output = String::from_utf8_lossy(output).replace('\0', "");
        let output = redact(output.trim_end());
        tail(&output, TEXT_LIMIT).to_owned()
    };
    // 之前写入的都以换行结尾：这里只需要给这一段补上。
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&status(code));
    text.push('\n');
    append(&text);
}

/// 返回全部记录，最早的在前；没有记录时返回空字符串。
pub fn output_text() -> String {
    output_snapshot().0
}

/// 返回全部记录和到现在为止写入过的总字节数，两者取自同一时刻。
pub fn output_snapshot() -> (String, u64) {
    let log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    (log.text.clone(), log.dropped + log.text.len() as u64)
}

/// 返回第 `written` 个字节以后写入的内容和现在的总字节数，可以直接接在已经显示的文本后面。
///
/// `written` 为 0（之前什么都没有显示）或者那以后的内容已有一部分被丢弃时返回 `None`，调用方
/// 应当换上 [`output_snapshot`] 的全文。
pub fn output_since(written: u64) -> Option<(String, u64)> {
    let log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    let start = usize::try_from(written.checked_sub(log.dropped)?).ok()?;
    if written == 0 || start > log.text.len() || !log.text.is_char_boundary(start) {
        return None;
    }
    Some((
        log.text[start..].to_owned(),
        log.dropped + log.text.len() as u64,
    ))
}

/// 仓库和命令两行：`root`，然后是 `$ git args`，地址中的凭据已经替换。
fn header<S: AsRef<OsStr>>(root: &Path, args: &[S]) -> String {
    let args: Vec<_> = args.iter().map(|a| a.as_ref().to_string_lossy()).collect();
    let command = redact(&args.join(" "));
    format!("{}\n$ git {command}\n", root.display())
}

/// 退出码一行的文字。
fn status(code: Option<i32>) -> String {
    match code {
        Some(code) => winc_i18n::t!("git-output-exit-code", code = code.to_string()),
        None => winc_i18n::t!("git-output-no-exit-code").to_owned(),
    }
}

/// 返回一条命令结果的文本；参数中不是 UTF-8 的部分按有损转换显示。
fn entry<S: AsRef<OsStr>>(root: &Path, args: &[S], code: Option<i32>, output: &str) -> String {
    // 先替换再截断：截断点落在地址中间时，剩下的凭据前面已经没有 `://`，认不出来。
    let output = redact(output.trim_end());
    let output = tail(&output, TEXT_LIMIT);
    let mut entry = header(root, args);
    if !output.is_empty() {
        entry.push_str(output);
        entry.push('\n');
    }
    entry.push_str(&status(code));
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
