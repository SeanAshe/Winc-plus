//! 逐行追溯的测试：行号栏标注、未提交的行、悬停提示与跳到日志中的提交。

use super::*;

/// 在 `root` 中以作者 `who`（姓名、邮箱）提交 `file`，提交时间设为 `days` 天前。
fn dated_commit(root: &Path, file: &str, text: &str, message: &str, who: (&str, &str), days: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let date = format!("{} +0800", now - days * 86_400);
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    git_in(root, &["add", "."]);
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", &format!("user.name={}", who.0), "-c"])
        .arg(format!("user.email={}", who.1))
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "commit {message}");
}

/// 为 `src/server.rs` 生成三次提交：两个作者分别修改监听地址与连接处理，供逐行追溯使用。
fn blame_repo(root: &Path) {
    let zhang = ("张伟", "zhang@example.com");
    let alice = ("Alice Chen", "alice@example.com");
    let v1 = "use std::net::TcpListener;\n\nfn main() {\n    let listener = TcpListener::bind(\"127.0.0.1:8080\").unwrap();\n    for stream in listener.incoming() {\n        handle(stream.unwrap());\n    }\n}\n";
    dated_commit(
        root,
        "src/server.rs",
        v1,
        "服务器：监听 8080 端口",
        zhang,
        120,
    );
    let v2 = v1.replace("127.0.0.1:8080", "0.0.0.0:8080");
    dated_commit(root, "src/server.rs", &v2, "服务器：对外网开放", alice, 30);
    let v3 = v2.replace(
        "        handle(stream.unwrap());\n",
        "        let stream = stream.unwrap();\n        log::info!(\"连接 {:?}\", stream.peer_addr());\n        handle(stream);\n",
    );
    dated_commit(root, "src/server.rs", &v3, "记录每个连接的来源", zhang, 1);
}

#[test]
fn blame_annotates_lines_and_jumps_to_the_commit_in_the_log() {
    use crate::app::git::blame::BLAME_W;

    let Some((mut app, root)) = git_app("git-blame", false) else {
        return;
    };
    blame_repo(&root);
    app.git_refresh();
    app.open_path(&root.join("src/server.rs"));
    paint(&mut app);
    let width = app.layout.gutter.w;

    // 打开：行号栏多出一栏 `BLAME_W`；每一行的作者对应到各自的提交。
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    assert_eq!(app.layout.gutter.w, width + BLAME_W);
    let id = app.editor.active().unwrap().id;
    let blame = app.blame_of(id).expect("算好了");
    let summary = |line: usize| blame.commits[blame.lines[line] as usize].summary.clone();
    assert_eq!(summary(0), "服务器：监听 8080 端口");
    assert_eq!(summary(3), "服务器：对外网开放");
    assert_eq!(summary(5), "记录每个连接的来源");

    // 修改一行但不保存：重算后该行标记为「未提交」，其余行的归属不变。
    let at = app.editor.active().unwrap().text().line_start(1);
    app.select(at, at);
    app.run(winc_editor::Command::Insert("// 新加的一行\n".into()));
    paint(&mut app);
    let blame = app.blame_of(id).unwrap();
    assert!(blame.commits[blame.lines[1] as usize].uncommitted);
    assert_eq!(
        blame.commits[blame.lines[4] as usize].summary,
        "服务器：对外网开放"
    );

    // 悬停提示写着提交号和说明；点了在日志里选中那次提交。
    let l = app.layout;
    let y = l.text.y + 4.5 * app.fonts.line_height;
    let x = l.gutter.x + 40.0;
    assert_eq!(app.hover_at(x, y), Hover::Blame(4));
    let tip = app.blame_tip(4).unwrap().1;
    assert!(tip.contains("服务器：对外网开放"), "{tip}");
    click(&mut app, (x, y));
    assert!(app.log_active());
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    let selected = view.selected.map(|i| view.commits[i].subject.clone());
    assert_eq!(selected.as_deref(), Some("服务器：对外网开放"));
    // 日志显示完整的历史，不按该提交筛选。
    assert!(view.query.text.is_empty() && app.log.filter.text().is_empty());
    assert!(view.commits.len() > 1);

    // 回到文件再关闭追溯：行号栏恢复原来的宽度。
    app.open_path(&root.join("src/server.rs"));
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    assert_eq!(app.layout.gutter.w, width);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn blame_click_finds_a_commit_older_than_the_first_log_batch() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::process::Stdio;
    use winc_editor::git::log::BATCH;

    let Some((mut app, root)) = git_app("git-blame-deep-log", false) else {
        return;
    };
    blame_repo(&root);
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    let head = std::str::from_utf8(&head.stdout).unwrap().trim();
    // 一次写入整批空提交，使目标超出首批日志，同时保留原文件的逐行归属。
    let mut history = String::new();
    for i in 0..BATCH {
        let time = 1_800_000_000 + i;
        writeln!(
            history,
            "commit refs/heads/main\ncommitter Test <t@t> {time} +0000\ndata 7\nhistory"
        )
        .unwrap();
        if i == 0 {
            writeln!(history, "from {head}").unwrap();
        }
        history.push('\n');
    }
    let mut import = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    import
        .stdin
        .take()
        .unwrap()
        .write_all(history.as_bytes())
        .unwrap();
    assert!(import.wait().unwrap().success());

    app.git_refresh();
    let path = root.join("src/server.rs");
    app.open_path(&path);
    assert!(app.run_action(Action::GitBlame));
    let target = app.blame_commit(0).unwrap();
    assert_eq!(target.summary, "服务器：监听 8080 端口");

    assert!(app.run_action(Action::GitShowLog));
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.commits.len(), BATCH);
    assert!(view.commits.iter().all(|c| c.oid != target.oid));

    app.open_path(&path);
    paint(&mut app);
    let x = app.layout.gutter.x + 40.0;
    let y = app.layout.text.y + app.fonts.line_height / 2.0;
    assert_eq!(app.hover_at(x, y), Hover::Blame(0));
    click(&mut app, (x, y));

    // 逐批读取到该提交后选中；日志仍是完整的历史，不按该提交筛选。
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    assert_eq!(view.selected_oid(), Some(target.oid.as_str()));
    assert!(view.query.text.is_empty() && app.log.filter.text().is_empty());
    assert!(view.commits.len() > BATCH);
    let details = view.details.as_ref().unwrap();
    assert_eq!(details.oid, target.oid);
    assert!(details.files.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：逐行追溯（`blame-light`、`blame-dark`）。
#[test]
#[ignore]
fn shoot_blame() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-blame-shot-{theme}"), false, dark)
        else {
            return;
        };
        blame_repo(&root);
        app.git_refresh();
        app.open_path(&root.join("src/server.rs"));
        app.run_action(Action::GitBlame);
        let at = app.editor.active().unwrap().text().line_start(1);
        app.select(at, at);
        app.run(winc_editor::Command::Insert(
            "// TODO：限制并发连接数\n".into(),
        ));
        paint(&mut app);
        // 鼠标停在第 7 行（「记录每个连接的来源」）上，提示已显示。
        let l = app.layout;
        let y = l.text.y + 6.5 * app.fonts.line_height;
        app.on_mouse_move(l.gutter.x + 40.0, y);
        app.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("blame-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
