//! 调用模型生成文本：按配置的协议构造单轮对话的请求，用 `curl` 发送，从响应中取出文本。
//!
//! 支持 Chat Completions、Responses、Messages、Interactions 四种协议，不使用流式响应。网址、
//! 请求头和正文以 `curl` 配置的形式从标准输入传入，密钥和正文不出现在进程的命令行参数中。

use std::process::{Command, Stdio};
use std::time::Duration;

use winc_config::{AiProtocol, Config, Json};

use super::http::{SendError, parse_response};
use super::shell::Cancel;
use super::shell::process::{self, RunError};

/// 进程超时比 `curl` 自身时限多出的时间：`curl` 先按自身时限退出并给出原因。
const TIMEOUT_GRACE: Duration = Duration::from_secs(5);

/// Messages 协议必填的输出上限（token）。
///
/// 取自调参：为推理留出余量；部分兼容接口不接受大于 8192 的值。
const MAX_TOKENS: f64 = 8192.0;

/// Messages 协议的版本。
const ANTHROPIC_VERSION: &str = "2026-10-21";

/// 调用模型所需的设置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// 接口协议。
    pub protocol: AiProtocol,
    /// 接口地址，包含版本路径，末尾没有 `/`。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// 密钥；为 `None` 时不发送认证请求头。
    pub api_key: Option<String>,
    /// 一次请求最长的等待时间（秒），同时作为 `curl` 的 `--max-time`。
    pub timeout: u64,
}

impl Endpoint {
    /// 由配置 `config` 创建；未设置模型时返回 `None`。
    pub fn from_config(config: &Config) -> Option<Endpoint> {
        let protocol = config.ai_protocol.unwrap_or_default();
        let base_url = (config.ai_base_url.as_deref()).unwrap_or(protocol.default_base_url());
        let base_url = base_url.trim().trim_end_matches('/');
        // 没写协议时按 HTTPS：`curl` 对没有协议的地址默认用 HTTP，密钥会以明文发送。
        let base_url = match base_url.contains("://") {
            true => base_url.to_owned(),
            false => format!("https://{base_url}"),
        };
        Some(Endpoint {
            protocol,
            base_url,
            model: config.ai_model.as_deref()?.trim().to_owned(),
            api_key: (config.ai_api_key.as_deref()).map(|key| key.trim().to_owned()),
            timeout: config.ai_timeout.unwrap_or(Config::AI_TIMEOUT),
        })
    }
}

/// 调用模型失败的原因。
#[derive(Debug)]
pub enum AiError {
    /// 请求未完成：无法运行 `curl`、超时、已取消或连接失败。
    Send(SendError),
    /// 服务器返回错误：状态码，以及响应中的说明（响应中有说明时）。
    Status(u16, Option<String>),
    /// 响应中没有生成的文本。
    Empty,
}

/// 以 `system` 为指示、`input` 为用户消息请求一次回答，返回生成的文本，去掉首尾空白和推理
/// 模型写在正文开头的 `<think>` 段。
///
/// `curl` 是已设好代理等选项的命令。会等待网络，应在后台线程调用；最长等待
/// [`Endpoint::timeout`] 秒，可经 `cancel` 取消。
///
/// # Errors
///
/// 请求未完成、服务器返回错误或响应中没有文本时返回 [`AiError`]。
pub fn complete(
    endpoint: &Endpoint,
    system: &str,
    input: &str,
    mut curl: Command,
    cancel: &Cancel,
) -> Result<String, AiError> {
    let (path, body) = request(endpoint, system, input);
    let mut config = String::new();
    option(&mut config, "url", &format!("{}{path}", endpoint.base_url));
    option(&mut config, "header", "Content-Type: application/json");
    for (name, value) in headers(endpoint) {
        option(&mut config, "header", &format!("{name}: {value}"));
    }
    option(&mut config, "data-binary", &body.compact());

    curl.args(["--silent", "--show-error", "--include"])
        .args(["--max-time", &endpoint.timeout.to_string()])
        .args(["--proto", "=http,https"])
        .args(["--config", "-"])
        .stderr(Stdio::piped());
    let timeout = Duration::from_secs(endpoint.timeout) + TIMEOUT_GRACE;
    let output = process::run(&mut curl, config.into_bytes(), timeout, cancel).map_err(|e| {
        AiError::Send(match e {
            RunError::Spawn(e) => SendError::Spawn(e),
            RunError::Timeout => SendError::Timeout,
            RunError::TooLarge => SendError::TooLarge,
            RunError::Cancelled => SendError::Cancelled,
        })
    })?;
    if !output.status.success() {
        return Err(AiError::Send(SendError::Curl {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }));
    }
    let response = parse_response(&output.stdout).ok_or(AiError::Send(SendError::Invalid))?;
    let reply = Json::parse(&String::from_utf8_lossy(&response.body));
    if !(200..300).contains(&response.status) {
        let message = reply.as_ref().and_then(error_message);
        return Err(AiError::Status(response.status, message));
    }
    let text = reply
        .as_ref()
        .and_then(|r| reply_text(endpoint.protocol, r));
    let text = without_thinking(&text.unwrap_or_default())
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(AiError::Empty);
    }
    Ok(text)
}

/// 返回请求的路径（接在接口地址之后）和正文。
fn request(endpoint: &Endpoint, system: &str, input: &str) -> (&'static str, Json) {
    let text = |s: &str| Json::String(s.to_owned());
    let model = ("model", text(&endpoint.model));
    let message = |role: &str, content: &str| {
        Json::object([("role", text(role)), ("content", text(content))])
    };
    match endpoint.protocol {
        AiProtocol::ChatCompletions => {
            let messages = Json::Array(vec![message("system", system), message("user", input)]);
            (
                "/chat/completions",
                Json::object([model, ("messages", messages)]),
            )
        }
        AiProtocol::Responses => {
            // 不要求服务器保存对话：只请求一次，保存的内容只有仓库的改动。
            let fields = [
                model,
                ("instructions", text(system)),
                ("input", text(input)),
                ("store", Json::Bool(false)),
            ];
            ("/responses", Json::object(fields))
        }
        AiProtocol::Messages => {
            let fields = [
                model,
                ("max_tokens", Json::Number(MAX_TOKENS)),
                ("system", text(system)),
                ("messages", Json::Array(vec![message("user", input)])),
            ];
            ("/messages", Json::object(fields))
        }
        AiProtocol::Interactions => {
            let fields = [
                model,
                ("system_instruction", text(system)),
                ("input", text(input)),
            ];
            ("/interactions", Json::object(fields))
        }
    }
}

/// 返回随协议而定的请求头；没有密钥时不包含认证请求头。
fn headers(endpoint: &Endpoint) -> Vec<(&'static str, String)> {
    let key = endpoint.api_key.clone().filter(|key| !key.is_empty());
    match endpoint.protocol {
        AiProtocol::ChatCompletions | AiProtocol::Responses => key
            .map(|key| ("Authorization", format!("Bearer {key}")))
            .into_iter()
            .collect(),
        AiProtocol::Messages => {
            let version = ("anthropic-version", ANTHROPIC_VERSION.to_owned());
            key.map(|key| ("x-api-key", key))
                .into_iter()
                .chain([version])
                .collect()
        }
        AiProtocol::Interactions => key.map(|key| ("x-goog-api-key", key)).into_iter().collect(),
    }
}

/// 从成功的响应中取出生成的文本；分为多段时按顺序拼接。
fn reply_text(protocol: AiProtocol, reply: &Json) -> Option<String> {
    let parts: Vec<&str> = match protocol {
        // 内容通常是字符串，部分兼容接口以分段的数组返回。
        AiProtocol::ChatCompletions => {
            let choice = reply.get("choices")?.as_array()?.first()?;
            let content = choice.get("message")?.get("content")?;
            match content.as_str() {
                Some(text) => vec![text],
                None => items(choice.get("message")?, "content", "text")
                    .filter_map(|part| part.get("text")?.as_str())
                    .collect(),
            }
        }
        AiProtocol::Responses => items(reply, "output", "message")
            .flat_map(|message| items(message, "content", "output_text"))
            .filter_map(|part| part.get("text")?.as_str())
            .collect(),
        AiProtocol::Messages => items(reply, "content", "text")
            .filter_map(|part| part.get("text")?.as_str())
            .collect(),
        // 文本在 `model_output` 步骤的内容中；早期版本的响应直接放在 `outputs` 中。
        AiProtocol::Interactions => items(reply, "steps", "model_output")
            .flat_map(|step| items(step, "content", "text"))
            .chain(items(reply, "outputs", "text"))
            .filter_map(|part| part.get("text")?.as_str())
            .collect(),
    };
    Some(parts.concat())
}

/// 返回 `json` 的数组 `key` 中 `type` 为 `kind` 的各项。
fn items<'a>(json: &'a Json, key: &str, kind: &'a str) -> impl Iterator<Item = &'a Json> {
    let list = json.get(key).and_then(Json::as_array).unwrap_or_default();
    list.iter()
        .filter(move |item| item.get("type").and_then(Json::as_str) == Some(kind))
}

/// 返回错误响应中的说明：四种协议都写在 `error.message`，部分兼容接口写在顶层的 `message`。
fn error_message(reply: &Json) -> Option<String> {
    let message = (reply.get("error").and_then(|e| e.get("message")))
        .or_else(|| reply.get("message"))?
        .as_str()?
        .trim();
    (!message.is_empty()).then(|| message.to_owned())
}

/// 去掉推理模型写在正文开头的 `<think>…</think>` 段。
fn without_thinking(text: &str) -> &str {
    let trimmed = text.trim_start();
    match trimmed.strip_prefix("<think>") {
        Some(rest) => rest
            .find("</think>")
            .map_or("", |end| &rest[end + "</think>".len()..]),
        None => text,
    }
}

/// 在 `curl` 配置 `out` 的末尾写入一行选项 `name = "value"`：值中的 `\`、`"` 和换行按配置文件
/// 的规则转义。
fn option(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(" = \"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push_str("\"\n");
}

#[cfg(test)]
mod tests;
