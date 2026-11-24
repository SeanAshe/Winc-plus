//! `.http` 请求的界面部分的测试：响应标签页的内容、未定义变量的检查。

use std::time::Duration;

use super::*;

fn response(body: &[u8]) -> Response {
    Response {
        status_line: "HTTP/1.1 200 OK".into(),
        status: 200,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.to_vec(),
        elapsed: Duration::from_millis(12),
    }
}

/// 状态行、响应头、空行之后是正文；正文不是 UTF-8 时只注明长度。
#[test]
fn the_response_tab_shows_status_headers_and_body() {
    assert_eq!(
        response_text(&response(b"{\"ok\": true}")),
        "HTTP/1.1 200 OK\nContent-Type: application/json\n\n{\"ok\": true}"
    );
    let binary = response_text(&response(&[0xff, 0xfe, 0x00]));
    assert!(binary.ends_with("（二进制内容，3 字节）"), "{binary}");
}

/// 超长的正文只显示开头，截断处落在多字节字符中间时去掉不完整的字符。
#[test]
fn a_long_body_is_cut_at_a_character_boundary() {
    let mut body = vec![b'a'; SHOWN_BODY - 1];
    body.extend("中文".as_bytes());
    let text = response_text(&response(&body));
    assert!(!text.contains('\u{fffd}'));
    assert!(text.ends_with(&format!(
        "（仅显示前 {SHOWN_BODY} 字节，共 {} 字节）",
        body.len()
    )));
}

#[test]
fn undefined_variables_are_found_in_any_part() {
    let mut request = Request {
        line: 0,
        method: "GET".into(),
        url: "https://example.com".into(),
        headers: vec![("Authorization".into(), "Bearer {{ token }}".into())],
        body: String::new(),
    };
    assert_eq!(undefined_variable(&request).as_deref(), Some("token"));
    request.headers.clear();
    assert_eq!(undefined_variable(&request), None);
}
