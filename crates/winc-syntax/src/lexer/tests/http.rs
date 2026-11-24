//! HTTP 请求文件（`.http`、`.rest`）的词法测试：请求行、请求头、变量、注释与 JSON 正文。

use super::*;

/// 请求行的方法和协议名是关键字，网址中的 `#片段` 不是注释；行首的请求头和 `@变量 =` 按键
/// 着色，`{{变量}}` 按字符串着色；`###` 分隔请求，按注释着色。
#[test]
fn http_requests_headers_and_variables() {
    let cases: [(&str, &[(&str, TokenKind)]); 6] = [
        (
            "GET https://example.com/api#top HTTP/1.1",
            &[
                ("GET", TokenKind::Keyword),
                ("HTTP", TokenKind::Keyword),
                ("1.1", TokenKind::Number),
            ],
        ),
        (
            "POST {{host}}/users",
            &[
                ("POST", TokenKind::Keyword),
                ("{{host}}", TokenKind::String),
            ],
        ),
        (
            "Authorization: Bearer {{token}}",
            &[
                ("Authorization", TokenKind::Property),
                ("{{token}}", TokenKind::String),
            ],
        ),
        ("@host = example.com", &[("@host", TokenKind::Property)]),
        ("### 获取用户", &[("### 获取用户", TokenKind::Comment)]),
        (
            "{\"ok\": true}",
            &[
                ("\"ok\"", TokenKind::Property),
                ("true", TokenKind::Builtin),
            ],
        ),
    ];
    for (line, expected) in cases {
        let (tokens, next) = lex("a.http", line, LineState::Normal);
        let expected: Vec<(String, TokenKind)> =
            expected.iter().map(|&(t, k)| (t.to_owned(), k)).collect();
        assert_eq!(tokens, expected, "{line}");
        assert_eq!(next, LineState::Normal);
    }
}
