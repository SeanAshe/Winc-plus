//! 后台读取新打开文件的测试：读取期间的跳转、切换标签、关闭标签，以及磁盘上的外部修改。

use std::time::{Duration, Instant, SystemTime};

use winc_editor::LoadJob;

use super::*;

/// 按 `open_path` 的方式打开标签并发起读取，但不立即收下结果。
///
/// 返回文档 ID、读取凭证（ticket）与任务，由测试决定何时交回结果。
fn open_unloaded(app: &mut App, path: &Path) -> (DocumentId, u64, LoadJob) {
    let index = app.editor.open_tab(path);
    let job = app.editor.begin_load(index).unwrap();
    let id = job.id();
    let ticket = app.loads.tickets.issue(id);
    app.loads.since = Some((id, Instant::now()));
    (id, ticket, job)
}

fn deliver(app: &mut App, (id, ticket, job): (DocumentId, u64, LoadJob)) {
    app.loads.results.0.send((id, ticket, job.run())).unwrap();
    app.poll_loads();
}

/// 返回打开着 `path` 的标签序号。
fn tab_of(app: &App, path: &Path) -> Option<usize> {
    let path = winc_config::files::canonicalize(path).ok()?;
    app.editor
        .documents
        .iter()
        .position(|d| d.path.as_deref() == Some(path.as_path()))
}

/// 生成 `lines` 行 `line <行号>` 的样本，每行都不同，便于断言跳转到的行。
fn numbered(lines: usize) -> String {
    (0..lines).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn jumps_wait_until_the_file_is_read() {
    let dir = sample_folder("loading-jump");
    let path = dir.join("long.txt");
    std::fs::write(&path, numbered(100)).unwrap();
    let mut app = app_with(&dir, false);

    let load = open_unloaded(&mut app, &path);
    let id = load.0;
    // 还在读：标签是空的，不显示「只读」，跳转先记下。
    assert!(app.editor.active().unwrap().is_pending());
    assert!(app.holding_frame(Instant::now()).is_some());
    app.jump_to(42, 2, 0);
    assert_eq!(app.loads.then.len(), 1);

    deliver(&mut app, load);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.id, id);
    assert!(!doc.is_pending());
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (42, 2));
    assert!(app.loads.then.is_empty());
    assert!(app.holding_frame(Instant::now()).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_jump_waits_for_its_tab_and_is_dropped_when_the_tab_closes() {
    let dir = sample_folder("loading-switch");
    let (first, second) = (dir.join("first.txt"), dir.join("second.txt"));
    std::fs::write(&first, numbered(50)).unwrap();
    std::fs::write(&second, numbered(50)).unwrap();
    let mut app = app_with(&first, false);
    let other = app.editor.active.unwrap();

    // 切换标签后读完：内容换上，当前标签不受影响；跳转留到再次切到该标签时执行
    // （例如 `winc a.rs:10 b.rs:20` 中 `a.rs` 的跳转）。
    let load = open_unloaded(&mut app, &second);
    app.jump_to(30, 0, 0);
    app.activate_tab(other);
    deliver(&mut app, load);
    let index = tab_of(&app, &second).unwrap();
    assert!(!app.editor.documents[index].is_pending());
    assert_eq!(app.editor.documents[index].selection.head, 0);
    assert_eq!(app.editor.active, Some(other));
    assert_eq!(app.editor.active().unwrap().selection.head, 0);
    assert_eq!(app.loads.then.len(), 1);
    app.activate_tab(index);
    app.run_when_loaded();
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().pos_to_line_col(doc.selection.head), (30, 0));
    assert!(app.loads.then.is_empty());

    // 关掉以后读好：什么也不做。
    app.close_tab(index);
    let third = dir.join("third.txt");
    std::fs::write(&third, numbered(50)).unwrap();
    let load = open_unloaded(&mut app, &third);
    app.jump_to(30, 0, 0);
    let index = app.editor.active.unwrap();
    app.close_tab(index);
    deliver(&mut app, load);
    assert!(tab_of(&app, &third).is_none());
    assert!(app.loads.then.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changes_on_disk_while_reading_are_picked_up() {
    let dir = sample_folder("loading-changed");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let path = dir.join("file.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut app = app_with(&dir, false);

    // 读到的是旧内容，交回之前文件又被改了：换上以后再对一次磁盘，换成新的。
    let (id, ticket, job) = open_unloaded(&mut app, &path);
    let read = job.run();
    std::fs::write(&path, "after, and longer\n").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(1))
        .unwrap();
    app.loads.results.0.send((id, ticket, read)).unwrap();
    app.poll_loads();
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert_eq!(text, "after, and longer\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_slow_read_shows_what_is_being_read() {
    let dir = sample_folder("loading-slow");
    let path = dir.join("slow.txt");
    std::fs::write(&path, "x\n").unwrap();
    let mut app = app_with(&dir, false);

    // 超过留帧的时间还没读好：画出「正在读取」，不画空白的编辑区。
    let load = open_unloaded(&mut app, &path);
    app.loads.since = Some((load.0, Instant::now() - Duration::from_secs(1)));
    assert!(app.holding_frame(Instant::now()).is_none());
    shoot(&mut app, "loading-slow");
    assert!(app.hits.placeholder.is_empty());

    deliver(&mut app, load);
    assert!(!app.editor.active().unwrap().is_pending());
    let _ = std::fs::remove_dir_all(&dir);
}
