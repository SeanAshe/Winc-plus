//! `.http` 请求文件的测试：行号栏中发送请求的按钮，以及发送后显示响应的标签页。

use std::io::{BufRead, BufReader, Write};

use super::*;
use crate::app::editor::http::RunMark;
use crate::keymap::Action;

/// 在本机启动只应答一次的服务端，返回端口和取回请求行的线程。
fn serve_once() -> (u16, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
        }
        let reply = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: 12\r\nConnection: close\r\n\r\n{\"ok\": true}";
        (&stream).write_all(reply.as_bytes()).unwrap();
        first.trim().to_owned()
    });
    (port, server)
}

/// 请求行的行号栏中有发送按钮；点击后在后台发送，响应显示在只读的「响应」标签页中，按 HTTP
/// 着色。不是 `.http` 文件时命令不可用。没有 `curl` 时跳过。
#[test]
fn the_gutter_button_sends_the_request_and_shows_the_response() {
    if crate::web::command("curl")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (port, server) = serve_once();
    let dir = sample_folder("http-send");
    let file = dir.join("api.http");
    let text = format!("@host = http://127.0.0.1:{port}\n\n### 列出\nGET {{{{host}}}}/items\n");
    std::fs::write(&file, text).unwrap();
    let mut app = app_with(&file, false);
    assert!(app.action_available(Action::SendHttpRequest));
    paint(&mut app);

    let &(button, line) = app.hits.http_runs.first().expect("请求行有发送按钮");
    assert_eq!(line, 3);
    let (x, y) = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.hover_at(x, y), Hover::HttpRun(3));
    app.on_mouse_down(MouseButton::Left);
    // 发送期间按钮转动，悬停提示改为取消。
    let id = app.editor.active().unwrap().id;
    assert_eq!(app.http_mark(id), Some((3, RunMark::Running)));
    assert!(app.http_sending(3));

    // 测试中没有事件循环：反复取结果，直到响应标签页出现。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while app.http_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.poll_http();
    }
    assert_eq!(server.join().unwrap(), "GET /items HTTP/1.1");
    let doc = app.editor.active().unwrap();
    let title = format!("响应：GET http://127.0.0.1:{port}/items");
    assert_eq!(doc.title.as_deref(), Some(title.as_str()));
    assert_eq!(doc.highlighter.language().id, "http");
    let shown = doc.text().rope().unwrap().to_string();
    assert!(shown.starts_with("HTTP/1.1 200 OK\n"), "{shown}");
    assert!(shown.ends_with("\n\n{\"ok\": true}"), "{shown}");

    // 响应标签页不是 `.http` 文件，发送请求的命令不可用，按键留给其他命令。
    assert!(!app.action_available(Action::SendHttpRequest));
    assert!(!app.send_http_request(None));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 再点一次正在发送的请求即取消：结束 `curl`，不显示响应。没有 `curl` 时跳过。
#[test]
fn clicking_a_sending_request_again_cancels_it() {
    if crate::web::command("curl")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    // 只接受连接、不应答的服务端：请求一直停在等待响应。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = sample_folder("http-cancel");
    let file = dir.join("api.http");
    std::fs::write(&file, format!("GET http://127.0.0.1:{port}/slow\n")).unwrap();
    let mut app = app_with(&file, false);
    let tabs = app.editor.tab_count();

    assert!(app.send_http_request(Some(0)));
    assert!(app.http_sending(0));
    let _held = listener.accept().unwrap();
    assert!(app.send_http_request(Some(0)));
    assert!(!app.http_sending(0));
    assert_eq!(app.status.message.as_deref(), Some("已取消请求"));
    std::thread::sleep(std::time::Duration::from_millis(100));
    app.poll_http();
    assert_eq!(app.editor.tab_count(), tabs);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：`.http` 文件的着色和行号栏中的发送按钮（第二个请求的按钮处于悬停状态），浅色、深色
/// 各一张。运行方式：`cargo test -p winc-app shoot_http_requests -- --ignored`，图片写到
/// `WINC_SNAPSHOT_DIR`（默认是系统临时目录下的 `winc-snapshots`）。
#[test]
#[ignore = "截图"]
fn shoot_http_requests() {
    let text = "@host = https://api.example.com\n\n### 列出用户\nGET {{host}}/users?page=1\n\
                Accept: application/json\n\n### 新建用户\nPOST {{host}}/users\n\
                Content-Type: application/json\n\n\
                {\n  \"name\": \"winc\",\n  \"admin\": false\n}\n";
    for (tag, dark) in [("light", false), ("dark", true)] {
        let dir = sample_folder(&format!("http-shot-{tag}"));
        let file = dir.join("api.http");
        std::fs::write(&file, text).unwrap();
        let mut app = app_with(&file, dark);
        paint(&mut app);
        let &(button, _) = app.hits.http_runs.get(1).unwrap();
        app.on_mouse_move(button.x + button.w / 2.0, button.y + button.h / 2.0);
        shoot(&mut app, &format!("http-requests-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
