//! 生成提交信息的测试：按钮随开关显示；未开启或未设置模型时的提示；结果填入信息框并可撤销；
//! 发送给模型的内容包含改动和未跟踪的文件；失败时的提示；密钥不显示明文；超时时间的调整。

use super::*;

/// 开启「AI 生成提交信息」后才显示按钮，位于下拉箭头左侧。未开启或未设置模型时执行命令会提示
/// 原因，「打开设置」分别定位到开关和模型。
#[test]
fn the_button_follows_the_switch() {
    let Some((mut app, root)) = git_app("git-generate-setting", false) else {
        return;
    };
    app.config.ai_model = Some("m".into());
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.commit_parts().generate.is_none());
    app.run_action(Action::GitGenerateCommitMessage);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("请先在设置中开启「AI 生成提交信息」")
    );
    app.run_notice_action();
    assert!(app.settings.open);
    assert_eq!(app.settings.selected, Setting::GitGenerateCommitMessage);
    app.settings_toggle(Setting::GitGenerateCommitMessage);
    assert_eq!(app.config.git_generate_commit_message, Some(true));

    app.config.ai_model = None;
    app.run_action(Action::GitGenerateCommitMessage);
    app.run_notice_action();
    assert_eq!(app.settings.selected, Setting::AiModel);

    app.close_settings();
    paint(&mut app);
    let parts = app.commit_parts();
    let button = parts.generate.expect("开启后出现按钮");
    assert!(Hover::Commit(CommitTarget::Generate).has_tooltip());
    assert!(button.right() < parts.commit_menu.x && button.x > parts.commit.right());
    let _ = std::fs::remove_dir_all(&root);
}

/// 改动、未跟踪的文件和最近的提交发送给模型；回答填入信息框、焦点不动，撤销后恢复原有内容。
/// 没有 `curl` 或配置了代理（请求无法到达本机的临时服务端）时跳过。
#[test]
fn the_generated_message_fills_the_box_and_can_be_undone() {
    use std::io::{BufRead, BufReader, Read, Write};

    let curl = crate::web::curl();
    if Command::new("curl").arg("--version").output().is_err()
        || winc_editor::proxy::configured(&curl)
    {
        return;
    }
    let Some((mut app, root)) = git_app("git-generate-fill", false) else {
        return;
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
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
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let json = r#"{"choices":[{"message":{"content":"```\nfeat: 加入入口\n\n说明\n```"}}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        );
        (&stream).write_all(reply.as_bytes()).unwrap();
        String::from_utf8(body).unwrap()
    });
    app.config.ai_base_url = Some(format!("http://127.0.0.1:{port}/v1"));
    app.config.ai_model = Some("m".into());
    app.config.git_generate_commit_message = Some(true);
    app.run_action(Action::ShowCommit);
    let paths: Vec<PathBuf> = (app.git.status.iter())
        .flat_map(|s| &s.entries)
        .map(|e| e.path.clone())
        .collect();
    app.set_commit_checked(&paths, true);
    app.git.commit.message.set_text("草稿");
    app.focus = Focus::Editor;

    app.run_action(Action::GitGenerateCommitMessage);
    app.poll_commit();
    assert!(!app.git.commit.generating.running());
    assert_eq!(app.git.commit.message.text(), "feat: 加入入口\n\n说明");
    // 结果到达时不移动焦点：生成期间可能已经在别处输入。
    assert_eq!(app.focus, Focus::Editor);
    assert!(app.git.commit.message.undo());
    assert_eq!(app.git.commit.message.text(), "草稿");

    let body = winc_config::Json::parse(&server.join().unwrap()).unwrap();
    let messages = body.get("messages").and_then(winc_config::Json::as_array);
    let input = messages.unwrap()[1].get("content").and_then(|c| c.as_str());
    let input = input.unwrap();
    assert!(input.contains("- first"), "{input}");
    assert!(input.contains("+++ b/src/main.rs"), "{input}");
    assert!(
        input.contains("diff --git a/src/new.rs b/src/new.rs\nnew file"),
        "{input}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 未选择任何更改时提示并且不发送请求；请求失败时提示原因，信息框的内容不变。
#[test]
fn failures_and_empty_commits_are_reported() {
    let Some((mut app, root)) = git_app("git-generate-failure", false) else {
        return;
    };
    // 1 号端口没有服务：连接被拒绝；配置了代理时由代理返回错误。
    app.config.ai_base_url = Some("http://127.0.0.1:1/v1".into());
    app.config.ai_model = Some("m".into());
    app.config.git_generate_commit_message = Some(true);
    app.run_action(Action::ShowCommit);
    let paths: Vec<PathBuf> = (app.git.status.iter())
        .flat_map(|s| &s.entries)
        .map(|e| e.path.clone())
        .collect();
    app.set_commit_checked(&paths, false);
    app.git.commit.message.set_text("草稿");
    app.run_action(Action::GitGenerateCommitMessage);
    assert_eq!(app.notice.text.as_deref(), Some("请先选择要提交的更改"));
    assert!(!app.git.commit.generating.running());

    app.set_commit_checked(&paths, true);
    app.run_action(Action::GitGenerateCommitMessage);
    app.poll_commit();
    let notice = app.notice.text.clone().unwrap_or_default();
    assert!(notice.starts_with("无法生成提交信息"), "{notice}");
    assert_eq!(app.git.commit.message.text(), "草稿");
    assert!(!app.git.commit.generating.running());
    let _ = std::fs::remove_dir_all(&root);
}

/// 设置中的密钥：输入框不显示明文，确认后写入配置。
#[test]
fn the_api_key_is_masked_in_settings() {
    let mut app = blank_app(false);
    app.settings_start_editing(Setting::AiApiKey);
    let masked = matches!(&app.settings.editing, Some((_, field)) if field.is_masked());
    assert!(masked);
    app.edit_field(|f| f.insert("sk-1234567890abcd"));
    app.settings_commit_field();
    assert_eq!(app.config.ai_api_key.as_deref(), Some("sk-1234567890abcd"));
    assert!(app.settings.editing.is_none());
}

/// 等待模型响应的时间默认 30 秒，每档调整 10 秒，不超出合法范围。
#[test]
fn the_timeout_steps_by_ten_seconds_within_range() {
    let mut app = blank_app(false);
    assert_eq!(app.setting_value(Setting::AiTimeout).0, "30");
    app.settings_adjust(Setting::AiTimeout, true);
    assert_eq!(app.config.ai_timeout, Some(40));
    for _ in 0..10 {
        app.settings_adjust(Setting::AiTimeout, false);
    }
    assert_eq!(app.config.ai_timeout, Some(10));
    let (_, at_min, _) = app.setting_value(Setting::AiTimeout);
    assert!(at_min);
}

/// 截图：提交卡片底行的「生成提交信息」（`commit-generate`）、设置的 AI 页（`settings-ai`），
/// 以及 Git 页的开关（`settings-git-generate`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_generate() {
    let Some((mut app, root)) = git_app("git-generate-shot", false) else {
        return;
    };
    app.config.ai_model = Some("gpt-5".into());
    app.config.git_generate_commit_message = Some(true);
    app.config.ai_api_key = Some("sk-proj-1234567890abcd".into());
    app.run_action(Action::ShowCommit);
    app.layout.sidebar.w = 320.0;
    app.relayout();
    app.git.commit.message.set_text("修复");
    paint(&mut app);
    let button = app.commit_parts().generate.unwrap();
    app.on_mouse_move(button.x + 4.0, button.y + 4.0);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(700));
    paint(&mut app);
    shoot(&mut app, "commit-generate");
    app.show_setting(Setting::AiModel);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(500));
    shoot(&mut app, "settings-ai");
    app.show_setting(Setting::GitGenerateCommitMessage);
    paint(&mut app);
    std::thread::sleep(std::time::Duration::from_millis(500));
    shoot(&mut app, "settings-git-generate");
    let _ = std::fs::remove_dir_all(&root);
}
