//! `.http` 请求文件：按 `###` 拆成多个请求，解析请求行、请求头和正文，用 `curl` 发送并解析响应。
//!
//! 文件中任意位置的 `@名字 = 值` 定义变量，请求中的 `{{名字}}` 替换为它的值；没有定义的变量
//! 保留原样。请求行是 `方法 网址 [协议版本]`，只写网址时为 `GET`。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ropey::Rope;

use super::shell::Cancel;
use super::shell::process::{self, RunError};

/// 识别的请求方法；请求行的第一个词是其中之一时，第二个词是网址。
const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

/// 一次请求最长的等待时间（秒），同时作为 `curl` 的 `--max-time`。
///
/// 取自调参：长于一般接口的响应时间，又不至于让无响应的请求长时间占用后台线程。
pub const TIMEOUT_SECS: u64 = 60;

/// 进程超时比 `curl` 自身的时限多等的时间：`curl` 先按自己的时限退出并给出原因。
const TIMEOUT_GRACE: Duration = Duration::from_secs(5);

/// 文件中的一个请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// 请求行所在的行号（从 0 起）。
    pub line: usize,
    /// 请求方法，大写。
    pub method: String,
    /// 网址，变量已替换。
    pub url: String,
    /// 请求头（名字，值），变量已替换。
    pub headers: Vec<(String, String)>,
    /// 正文，变量已替换；没有正文时为空。
    pub body: String,
}

/// 服务器的响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// 状态行，例如 `HTTP/1.1 200 OK`。
    pub status_line: String,
    /// 状态码；状态行中没有可识别的状态码时为 0。
    pub status: u16,
    /// 响应头（名字，值），按收到的顺序。
    pub headers: Vec<(String, String)>,
    /// 正文的原始字节。
    pub body: Vec<u8>,
    /// 从启动 `curl` 到收到全部响应的时长。
    pub elapsed: Duration,
}

/// 发送请求失败的原因。
#[derive(Debug)]
pub enum SendError {
    /// 无法运行 `curl`（例如没有安装）。
    Spawn(std::io::Error),
    /// 超过等待时间。
    Timeout,
    /// 响应超过了能收集的大小。
    TooLarge,
    /// 已取消。
    Cancelled,
    /// `curl` 以非零状态退出：退出码和错误输出。
    Curl {
        /// `curl` 的退出码；被信号终止时为 `None`。
        code: Option<i32>,
        /// `curl` 的错误输出。
        stderr: String,
    },
    /// `curl` 的输出中找不到 HTTP 响应。
    Invalid,
}

/// 返回各请求的请求行所在的行号（从 0 起），按从上到下的顺序。
///
/// 耗时与文件的行数成正比。
pub fn request_lines(text: &Rope) -> Vec<usize> {
    blocks(text)
        .into_iter()
        .filter_map(|block| request_line_in(text, block))
        .collect()
}

/// 返回第 `line` 行（从 0 起）所在的请求；该行所在的一段中没有请求行时返回 `None`。
///
/// 耗时与文件的行数成正比。
pub fn request_at(text: &Rope, line: usize) -> Option<Request> {
    let block = blocks(text)
        .into_iter()
        .find(|block| block.contains(&line))?;
    let start = request_line_in(text, block.clone())?;
    let (method, url) = parse_request_line(&line_text(text, start))?;
    let variables = variables(text);
    let fill = |s: &str| substitute(s, &variables);

    let mut url = url;
    let mut headers = Vec::new();
    let mut i = start + 1;
    // 请求行之后、第一个空行之前：续写的查询参数（以 `?`、`&` 开头的缩进行）和请求头。
    while i < block.end {
        let raw = line_text(text, i);
        let trimmed = raw.trim();
        i += 1;
        if trimmed.is_empty() {
            break;
        }
        if is_comment(trimmed) {
            continue;
        }
        if raw.starts_with(char::is_whitespace) && trimmed.starts_with(['?', '&']) {
            url.push_str(trimmed);
        } else if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((fill(name.trim()), fill(value.trim())));
        }
    }
    let mut body: Vec<String> = (i..block.end).map(|n| line_text(text, n)).collect();
    while body.last().is_some_and(|l| l.trim().is_empty()) {
        body.pop();
    }
    Some(Request {
        line: start,
        method,
        url: fill(&url),
        headers,
        body: fill(&body.join("\n")),
    })
}

/// 用 `curl` 发送请求 `request`，跟随重定向，返回最终的响应。
///
/// `curl` 是已设好代理等选项的命令（见 `winc_app::web::curl`）。会等待网络，应在后台线程调用；
/// 最长等待 [`TIMEOUT_SECS`] 秒，可经 `cancel` 取消。
///
/// # Errors
///
/// 无法运行 `curl`、超时、取消、`curl` 报错或输出中没有响应时返回 [`SendError`]。
pub fn send(request: &Request, mut curl: Command, cancel: &Cancel) -> Result<Response, SendError> {
    curl.args(["--silent", "--show-error", "--include", "--location"])
        .args(["--max-time", &TIMEOUT_SECS.to_string()]);
    match request.method.as_str() {
        // `-X HEAD` 会让 `curl` 等待不会到来的正文。
        "HEAD" => {
            curl.arg("--head");
        }
        "GET" => {}
        method => {
            curl.args(["--request", method]);
        }
    }
    // 网址经 `--url` 传入：以 `-` 开头的网址不会被当作选项。
    curl.arg("--url").arg(&request.url);
    for (name, value) in &request.headers {
        curl.arg("--header").arg(format!("{name}: {value}"));
    }
    let input = if request.body.is_empty() {
        Vec::new()
    } else {
        curl.args(["--data-binary", "@-"]);
        request.body.clone().into_bytes()
    };
    curl.stderr(Stdio::piped());

    let started = Instant::now();
    let timeout = Duration::from_secs(TIMEOUT_SECS) + TIMEOUT_GRACE;
    let output = process::run(&mut curl, input, timeout, cancel).map_err(|e| match e {
        RunError::Spawn(e) => SendError::Spawn(e),
        RunError::Timeout => SendError::Timeout,
        RunError::TooLarge => SendError::TooLarge,
        RunError::Cancelled => SendError::Cancelled,
    })?;
    if !output.status.success() {
        return Err(SendError::Curl {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let mut response = parse_response(&output.stdout).ok_or(SendError::Invalid)?;
    response.elapsed = started.elapsed();
    Ok(response)
}

/// 从 `curl --include` 的输出中取出最终的响应：跳过 `1xx` 临时响应、重定向和代理的
/// `Connection established`，它们的响应头之后紧跟着下一段响应。
pub(crate) fn parse_response(out: &[u8]) -> Option<Response> {
    let mut rest = out;
    loop {
        if !rest.starts_with(b"HTTP/") {
            return None;
        }
        let (head, body) = split_head(rest)?;
        let head = String::from_utf8_lossy(head);
        let mut lines = head.lines();
        let status_line = lines.next()?.trim().to_owned();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .collect();
        let established = status_line
            .to_ascii_lowercase()
            .contains("connection established");
        let interim = (100..200).contains(&status) || (300..400).contains(&status) || established;
        if interim && body.starts_with(b"HTTP/") {
            rest = body;
            continue;
        }
        return Some(Response {
            status_line,
            status,
            headers,
            body: body.to_vec(),
            elapsed: Duration::ZERO,
        });
    }
}

/// 把一段响应拆成（响应头，其后的内容）；响应头以空行结束，换行可以是 `\r\n` 或 `\n`。
fn split_head(out: &[u8]) -> Option<(&[u8], &[u8])> {
    let crlf = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    let lf = out.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let (at, len) = match (crlf, lf) {
        (Some(a), Some(b)) => a.min(b),
        (a, b) => a.or(b)?,
    };
    Some((&out[..at], &out[at + len..]))
}

/// 按 `###` 分隔的各段的行范围（不含分隔行）。
fn blocks(text: &Rope) -> Vec<std::ops::Range<usize>> {
    let lines = text.len_lines();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..lines {
        if line_text(text, i).trim_start().starts_with("###") {
            out.push(start..i);
            start = i + 1;
        }
    }
    out.push(start..lines);
    out
}

/// 一段中的请求行：跳过开头的空行、注释和变量定义后的第一行，是请求行时返回其行号。
fn request_line_in(text: &Rope, block: std::ops::Range<usize>) -> Option<usize> {
    let first = block.clone().find(|&i| {
        let line = line_text(text, i);
        let trimmed = line.trim();
        !trimmed.is_empty() && !is_comment(trimmed) && variable(trimmed).is_none()
    })?;
    parse_request_line(&line_text(text, first)).map(|_| first)
}

/// 解析请求行，返回（方法，网址）；不是请求行时返回 `None`。
fn parse_request_line(line: &str) -> Option<(String, String)> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    if METHODS.contains(&first) {
        return Some((first.to_owned(), words.next()?.to_owned()));
    }
    let url = ["http://", "https://", "{{"]
        .iter()
        .any(|prefix| first.starts_with(prefix));
    url.then(|| ("GET".to_owned(), first.to_owned()))
}

/// 是否为注释行（`#` 或 `//` 开头）。
fn is_comment(trimmed: &str) -> bool {
    trimmed.starts_with('#') || trimmed.starts_with("//")
}

/// 解析变量定义 `@名字 = 值`，返回（名字，值）。
fn variable(trimmed: &str) -> Option<(&str, &str)> {
    let (name, value) = trimmed.strip_prefix('@')?.split_once('=')?;
    let name = name.trim();
    let valid = !name.is_empty() && !name.contains(char::is_whitespace);
    valid.then(|| (name, value.trim()))
}

/// 文件中全部的变量定义；同名的以后定义的为准。
fn variables(text: &Rope) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for i in 0..text.len_lines() {
        let line = line_text(text, i);
        if let Some((name, value)) = variable(line.trim()) {
            out.retain(|(known, _)| known != name);
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    out
}

/// 把 `text` 中的 `{{名字}}` 替换为变量的值；没有定义的保留原样。
fn substitute(text: &str, variables: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open + 2..].find("}}") else {
            break;
        };
        let name = rest[open + 2..open + 2 + close].trim();
        out.push_str(&rest[..open]);
        match variables.iter().find(|(known, _)| known == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[open..open + 4 + close]),
        }
        rest = &rest[open + 4 + close..];
    }
    out.push_str(rest);
    out
}

/// 第 `line` 行的文字，不含行尾的换行符。
fn line_text(text: &Rope, line: usize) -> String {
    let mut s = text.line(line).to_string();
    while s.ends_with(['\n', '\r']) {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests;
