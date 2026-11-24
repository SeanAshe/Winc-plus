//! `.http` 请求文件的测试：拆分请求、请求头与正文、变量替换，以及 `curl` 输出的解析。

use super::*;

/// 样本：开头的变量定义、两个请求（第二个带续写的查询参数、请求头和 JSON 正文），以及一段
/// 只有注释的内容。
const SAMPLE: &str = "@host = https://api.example.com
@token = abc

### 列出用户
GET {{host}}/users HTTP/1.1
Accept: application/json

###
# 新建用户
POST {{host}}/users
    ?notify=1
    &dry=0
Authorization: Bearer {{token}}
Content-Type: application/json

{\"name\": \"{{missing}}\"}


### 只有注释
# 没有请求
";

#[test]
fn request_lines_skip_comments_and_variables() {
    assert_eq!(request_lines(&Rope::from_str(SAMPLE)), [4, 9]);
}

#[test]
fn a_request_has_its_headers_body_and_variables_filled_in() {
    let text = Rope::from_str(SAMPLE);
    let get = request_at(&text, 5).unwrap();
    assert_eq!(get.line, 4);
    assert_eq!(
        (get.method.as_str(), get.url.as_str()),
        ("GET", "https://api.example.com/users")
    );
    assert_eq!(get.headers, [("Accept".into(), "application/json".into())]);
    assert!(get.body.is_empty());

    // 光标在正文中也属于这个请求；没有定义的变量保留原样，正文末尾的空行去掉。
    let post = request_at(&text, 16).unwrap();
    assert_eq!(post.url, "https://api.example.com/users?notify=1&dry=0");
    assert_eq!(
        post.headers[0],
        ("Authorization".into(), "Bearer abc".into())
    );
    assert_eq!(post.body, "{\"name\": \"{{missing}}\"}");

    // 分隔行之后只有注释的一段没有请求；只写网址的一行是 GET。
    assert_eq!(request_at(&text, 21), None);
    let bare = request_at(&Rope::from_str("https://example.com\n"), 0).unwrap();
    assert_eq!(bare.method, "GET");
}

/// `curl --include --location` 的输出：先是临时响应和重定向，最后一段才是要显示的响应。
#[test]
fn the_last_response_after_interim_ones_is_kept() {
    let out = b"HTTP/1.1 100 Continue\r\n\r\n\
HTTP/1.1 302 Found\r\nLocation: /b\r\n\r\n\
HTTP/2 200\r\ncontent-type: application/json\r\n\r\n{\"ok\":true}";
    let response = parse_response(out).unwrap();
    assert_eq!(response.status_line, "HTTP/2 200");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers,
        [("content-type".into(), "application/json".into())]
    );
    assert_eq!(response.body, b"{\"ok\":true}");

    // 正文本身以 `HTTP/` 开头时不当作下一段响应。
    let out = b"HTTP/1.1 200 OK\n\nHTTP/1.1 is a protocol";
    assert_eq!(parse_response(out).unwrap().body, b"HTTP/1.1 is a protocol");
    assert_eq!(parse_response(b"curl: (6) no host"), None);
}

/// 经 `curl` 向本机的临时服务端发送请求：方法、请求头和正文原样送达，响应解析为状态、响应头
/// 和正文。没有 `curl` 时跳过。
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
        let reply = "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\
                     Content-Length: 11\r\nConnection: close\r\n\r\n{\"id\": 42}\n";
        (&stream).write_all(reply.as_bytes()).unwrap();
        (head, String::from_utf8(body).unwrap())
    });

    let request = Request {
        line: 0,
        method: "POST".into(),
        url: format!("http://127.0.0.1:{port}/users"),
        headers: vec![("X-Test".into(), "yes".into())],
        body: "{\"name\": \"a\"}".into(),
    };
    let mut curl = Command::new("curl");
    curl.args(["--noproxy", "*"]);
    let response = send(&request, curl, &Cancel::default()).unwrap();
    assert_eq!(response.status, 201);
    assert!(
        response
            .headers
            .contains(&("Content-Type".into(), "application/json".into()))
    );
    assert_eq!(response.body, b"{\"id\": 42}\n");

    let (head, body) = server.join().unwrap();
    assert_eq!(head[0], "POST /users HTTP/1.1");
    assert!(head.contains(&"X-Test: yes".to_owned()));
    assert_eq!(body, "{\"name\": \"a\"}");
}
