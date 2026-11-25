//! 日志文件（`.log`）的词法测试：时间戳、级别、地址、链接、路径、异常与调用栈。

use super::*;

#[test]
fn log_lines_color_timestamps_levels_and_locations() {
    use TokenKind::*;
    let tokens = |line: &str| {
        let (tokens, state) = lex("server.log", line, LineState::Normal);
        assert_eq!(state, LineState::Normal);
        tokens
    };
    let own = |items: &[(&str, TokenKind)]| -> Vec<(std::string::String, TokenKind)> {
        items
            .iter()
            .map(|(text, kind)| ((*text).to_owned(), *kind))
            .collect()
    };
    assert_eq!(
        tokens("2026-11-16T10:42:31.123Z INFO  server: listening on 127.0.0.1:8080"),
        own(&[
            ("2026-11-16T10:42:31.123Z", Comment),
            ("INFO", Inserted),
            ("127.0.0.1:8080", Number),
        ])
    );
    assert_eq!(
        tokens("2026-11-16 10:42:32 WARN  request took 1532ms path=/api/v1/users"),
        own(&[
            ("2026-11-16 10:42:32", Comment),
            ("WARN", Warning),
            ("1532ms", Number),
            ("path", Property),
            ("/api/v1/users", Link),
        ])
    );
    assert_eq!(
        tokens("2026-11-16 10:42:33 ERROR worker failed: connection refused"),
        own(&[("2026-11-16 10:42:33", Comment), ("ERROR", Deleted)])
    );
    assert_eq!(
        tokens("Traceback (most recent call last):"),
        own(&[("Traceback", Keyword)])
    );
    assert_eq!(
        tokens("  File \"/app/main.py\", line 42, in run"),
        own(&[("\"/app/main.py\"", String), ("42", Number)])
    );
    assert_eq!(
        tokens("Caused by: java.net.ConnectException: Connection refused"),
        own(&[("Caused by", Keyword), ("java.net.ConnectException", Type)])
    );
    assert_eq!(
        tokens("\tat com.example.Main$Worker.run(Main.java:42)"),
        own(&[
            ("at", Keyword),
            ("com.example.Main$Worker.run", Function),
            ("42", Number)
        ])
    );
    // 其他时间戳：系统日志、访问日志、只有时间。
    assert_eq!(
        tokens("Nov  6 10:42:33 host sshd[812]: Accepted from 10.0.0.7 port 22"),
        own(&[
            ("Nov  6 10:42:33", Comment),
            ("812", Number),
            ("10.0.0.7", Number),
            ("22", Number),
        ])
    );
    assert_eq!(
        tokens("[16/Nov/2026:10:42:33 +0000] \"GET /x HTTP/1.1\" 200 512"),
        own(&[
            ("16/Nov/2026:10:42:33 +0000", Comment),
            ("\"GET /x HTTP/1.1\"", String),
            ("200", Number),
            ("512", Number),
        ])
    );
    assert_eq!(
        tokens("10:42:31,123+08:00 [debug] level=warn id=3f2a9c1e-7b4d-4c1a-9e2f-0a1b2c3d4e5f"),
        own(&[
            ("10:42:31,123+08:00", Comment),
            ("debug", Builtin),
            ("level", Property),
            ("warn", Warning),
            ("id", Property),
            ("3f2a9c1e-7b4d-4c1a-9e2f-0a1b2c3d4e5f", Number),
        ])
    );
    // 链接、盘符路径、带行列的路径；末尾的句号不计入。
    assert_eq!(
        tokens("see https://example.com/a?b=1. at C:\\logs\\app.txt and ./src/main.rs:12:5."),
        own(&[
            ("https://example.com/a?b=1", Link),
            ("C:\\logs\\app.txt", Link),
            ("./src/main.rs:12:5", Link),
        ])
    );
    assert_eq!(
        tokens("error: thread 'main' panicked at src/main.rs:3: ValueError 99% 0x1F 1.5s"),
        own(&[
            ("error", Deleted),
            ("'main'", String),
            ("panicked", Keyword),
            ("3", Number),
            ("ValueError", Type),
            ("99%", Number),
            ("0x1F", Number),
            ("1.5s", Number),
        ])
    );
    // 行在任意位置截断或从任意位置开始时都能分析，token 按位置排列且互不重叠。
    let sample = "Nov  6 10:42:31,123+08:00 [E] 16/Nov/2026:10:42:33 +0000 a=/x:1:2 C:\\a ~/b \
                  10.0.0.7:80 3f2a9c1e-7b4d-4c1a-9e2f-0a1b2c3d4e5f 0x1F 1.5ms 'q' \"s\" x://y. at a.B$C(";
    let chars: Vec<char> = sample.chars().collect();
    let mut found = Vec::new();
    for cut in 0..=chars.len() {
        for part in [&chars[..cut], &chars[cut..]] {
            highlight_line(detect("a.log"), part, LineState::Normal, &mut found);
            assert!(found.windows(2).all(|w| w[0].end <= w[1].start));
            assert!(found.iter().all(|t| t.start < t.end && t.end <= part.len()));
        }
    }
    // 正文不着色：小写的级别词、撇号、落单的引号、单词中的数字和斜杠、版本号、标签。
    for line in [
        "an error occurred, see the info page and/or the warning",
        "it doesn't work and users' data isn't \"saved",
        "build abc123 of v1.2 used 3f2a for item_7 </div> +/- N/A",
        "连接已关闭，重试中",
    ] {
        assert_eq!(tokens(line), [], "{line}");
    }
}
