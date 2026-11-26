//! 调用模型的测试：各协议的请求和响应、错误说明，以及经 `curl` 发送的请求内容不变。

use super::*;

fn endpoint(protocol: AiProtocol) -> Endpoint {
    Endpoint {
        protocol,
        base_url: "https://example.com/v1".into(),
        model: "m".into(),
        api_key: Some("k".into()),
        timeout: 30,
    }
}

#[test]
fn the_endpoint_falls_back_to_the_official_address_and_needs_a_model() {
    let mut config = Config::default();
    assert_eq!(Endpoint::from_config(&config), None);
    config.ai_model = Some("m".into());
    config.ai_protocol = Some(AiProtocol::Messages);
    let endpoint = Endpoint::from_config(&config).unwrap();
    assert_eq!(endpoint.base_url, "https://api.anthropic.com/v1");
    config.ai_base_url = Some("http://localhost:11434/v1/".into());
    let endpoint = Endpoint::from_config(&config).unwrap();
    assert_eq!(endpoint.base_url, "http://localhost:11434/v1");
    config.ai_base_url = Some("api.example.com/v1".into());
    let endpoint = Endpoint::from_config(&config).unwrap();
    assert_eq!(endpoint.base_url, "https://api.example.com/v1");
    config.ai_base_url = None;
    assert_eq!(endpoint.timeout, Config::AI_TIMEOUT);
    config.ai_timeout = Some(90);
    assert_eq!(Endpoint::from_config(&config).unwrap().timeout, 90);
}

#[test]
fn each_protocol_has_its_path_body_and_key_header() {
    let body = |p| request(&endpoint(p), "S", "U").1.compact();
    assert_eq!(
        request(&endpoint(AiProtocol::ChatCompletions), "S", "U").0,
        "/chat/completions"
    );
    assert_eq!(
        body(AiProtocol::ChatCompletions),
        concat!(
            r#"{"messages":[{"content":"S","role":"system"},"#,
            r#"{"content":"U","role":"user"}],"model":"m"}"#
        )
    );
    assert_eq!(
        body(AiProtocol::Responses),
        r#"{"input":"U","instructions":"S","model":"m","store":false}"#
    );
    assert_eq!(
        body(AiProtocol::Messages),
        r#"{"max_tokens":8192,"messages":[{"content":"U","role":"user"}],"model":"m","system":"S"}"#
    );
    assert_eq!(
        body(AiProtocol::Interactions),
        r#"{"input":"U","model":"m","system_instruction":"S"}"#
    );

    let names = |p| {
        headers(&endpoint(p))
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(AiProtocol::Responses), ["Authorization"]);
    assert_eq!(
        names(AiProtocol::Messages),
        ["x-api-key", "anthropic-version"]
    );
    assert_eq!(names(AiProtocol::Interactions), ["x-goog-api-key"]);
    let keyless = Endpoint {
        api_key: None,
        ..endpoint(AiProtocol::ChatCompletions)
    };
    assert!(headers(&keyless).is_empty());
}

#[test]
fn the_text_is_read_from_each_protocol_reply() {
    let text = |p, json: &str| reply_text(p, &Json::parse(json).unwrap());
    let chat = r#"{"choices":[{"message":{"role":"assistant","content":"fix: a"}}]}"#;
    assert_eq!(
        text(AiProtocol::ChatCompletions, chat).as_deref(),
        Some("fix: a")
    );
    let parts = r#"{"choices":[{"message":{"content":[{"type":"text","text":"fix: "},
        {"type":"text","text":"a"}]}}]}"#;
    assert_eq!(
        text(AiProtocol::ChatCompletions, parts).as_deref(),
        Some("fix: a")
    );
    let responses = r#"{"output":[{"type":"reasoning","summary":[]},
        {"type":"message","content":[{"type":"output_text","text":"fix: b"}]}]}"#;
    assert_eq!(
        text(AiProtocol::Responses, responses).as_deref(),
        Some("fix: b")
    );
    let messages = r#"{"content":[{"type":"thinking","thinking":""},
        {"type":"text","text":"fix: "},{"type":"text","text":"c"}]}"#;
    assert_eq!(
        text(AiProtocol::Messages, messages).as_deref(),
        Some("fix: c")
    );
    let steps =
        r#"{"steps":[{"type":"model_output","content":[{"type":"text","text":"fix: d"}]}]}"#;
    assert_eq!(
        text(AiProtocol::Interactions, steps).as_deref(),
        Some("fix: d")
    );
    let outputs = r#"{"outputs":[{"type":"thought"},{"type":"text","text":"fix: e"}]}"#;
    assert_eq!(
        text(AiProtocol::Interactions, outputs).as_deref(),
        Some("fix: e")
    );
    assert_eq!(text(AiProtocol::ChatCompletions, "{}"), None);
}

#[test]
fn errors_and_thinking_are_read_from_the_reply() {
    let message = |json: &str| error_message(&Json::parse(json).unwrap());
    let nested = r#"{"type":"error","error":{"type":"not_found_error","message":"model: x"}}"#;
    assert_eq!(message(nested).as_deref(), Some("model: x"));
    assert_eq!(
        message(r#"{"message":"Unauthorized"}"#).as_deref(),
        Some("Unauthorized")
    );
    assert_eq!(message(r#"{"error":"bad"}"#), None);

    assert_eq!(
        without_thinking("<think>\nhmm\n</think>\n\nfix: a"),
        "\n\nfix: a"
    );
    assert_eq!(without_thinking("<think>unfinished"), "");
    assert_eq!(without_thinking("fix: <think>"), "fix: <think>");
}

/// 经 `curl` 向本机的临时服务端发送请求：密钥位于请求头中，正文中的引号、反斜杠、换行和汉字
/// 到达时不变，回答取自响应。没有 `curl` 时跳过。
#[test]
fn a_request_is_sent_with_curl() {
    use std::io::{BufRead, BufReader, Read, Write};

    if Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = Vec::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = n.trim().parse().unwrap();
            }
            if line.trim().is_empty() {
                break;
            }
            head.push(line.trim().to_owned());
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let json = r#"{"content":[{"type":"text","text":"feat: 加入 \"AI\""}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        );
        (&stream).write_all(reply.as_bytes()).unwrap();
        (head, String::from_utf8(body).unwrap())
    });

    let endpoint = Endpoint {
        protocol: AiProtocol::Messages,
        base_url: format!("http://127.0.0.1:{port}/v1"),
        model: "m".into(),
        api_key: Some("sk-secret".into()),
        timeout: 30,
    };
    let mut curl = Command::new("curl");
    curl.args(["--noproxy", "*"]);
    let input = "diff \"a\\b\"\n+提交";
    let text = complete(&endpoint, "S", input, curl, &Cancel::default()).unwrap();
    assert_eq!(text, "feat: 加入 \"AI\"");

    let (head, body) = server.join().unwrap();
    assert_eq!(head[0], "POST /v1/messages HTTP/1.1");
    assert!(head.contains(&"x-api-key: sk-secret".to_owned()));
    assert!(head.contains(&"Content-Type: application/json".to_owned()));
    let sent = Json::parse(&body).unwrap();
    let content = sent.get("messages").unwrap().as_array().unwrap()[0].get("content");
    assert_eq!(content.and_then(Json::as_str), Some(input));
}

/// 服务器返回错误时，结果包含状态码和说明。没有 `curl` 时跳过。
#[test]
fn an_error_reply_gives_the_status_and_message() {
    use std::io::{BufRead, BufReader, Write};

    if Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 2 {
            line.clear();
        }
        let json = r#"{"error":{"message":"Incorrect API key"}}"#;
        let reply = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        );
        (&stream).write_all(reply.as_bytes()).unwrap();
    });
    let endpoint = Endpoint {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        ..endpoint(AiProtocol::ChatCompletions)
    };
    let mut curl = Command::new("curl");
    curl.args(["--noproxy", "*"]);
    let error = complete(&endpoint, "S", "U", curl, &Cancel::default()).unwrap_err();
    assert!(
        matches!(error, AiError::Status(401, Some(ref m)) if m == "Incorrect API key"),
        "{error:?}"
    );
    server.join().unwrap();
}

/// 发送前已取消时不发送请求，结果为已取消。没有 `curl` 时跳过。
#[test]
fn a_cancelled_request_is_not_sent() {
    if Command::new("curl").arg("--version").output().is_err() {
        return;
    }
    let cancel = Cancel::default();
    cancel.cancel();
    let curl = Command::new("curl");
    let error = complete(&endpoint(AiProtocol::Messages), "S", "U", curl, &cancel).unwrap_err();
    assert!(
        matches!(error, AiError::Send(SendError::Cancelled)),
        "{error:?}"
    );
}
