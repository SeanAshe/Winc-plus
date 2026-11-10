//! 文件监视的测试：外部修改如何反映到文件树、文件表、已打开的文档和搜索结果。

use std::time::{Duration, SystemTime};

use winc_editor::Changes;

use crate::app::workspace::watch::DiskBack;

use super::*;

/// 将 `path` 的修改时间设为当前时间之后 1 秒。
///
/// 测试中写入过快，修改时间可能与打开时相同，无法判断文件被改过。
fn touch_later(path: &Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(1))
        .unwrap();
}

fn changes(dir: &Path, paths: &[&str]) -> Changes {
    Changes {
        paths: paths.iter().map(|p| dir.join(p)).collect(),
        rescan: false,
        git: false,
    }
}

fn tree_names(app: &App) -> Vec<String> {
    let ws = app.editor.workspace.as_ref().unwrap();
    ws.rows().iter().map(|e| e.name.clone()).collect()
}

#[test]
fn outside_changes_reach_the_tree_the_index_and_open_files() {
    let dir = sample_folder("watch-outside");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("short.rs"));
    app.open_path(&dir.join("a.txt"));
    app.open_picker();
    app.close_picker();
    let indexed = app.picker.index.len();

    // 其他程序新建了一个文件；修改了已打开且没有未保存修改的 `short.rs`（直接重新读取）；
    // 修改了已打开且有未保存修改的 `a.txt`（提示冲突，不覆盖）。
    std::fs::write(dir.join("new.rs"), "fn new() {}\n").unwrap();
    std::fs::write(dir.join("short.rs"), "fn changed() {}\n").unwrap();
    touch_later(&dir.join("short.rs"));
    let a = app.editor.active.unwrap();
    app.run(Command::Insert("mine ".into()));
    std::fs::write(dir.join("a.txt"), "theirs\n").unwrap();
    touch_later(&dir.join("a.txt"));
    app.apply_changes(changes(&dir, &["new.rs", "short.rs", "a.txt"]));

    assert!(tree_names(&app).contains(&"new.rs".to_owned()));
    let short = app
        .editor
        .documents
        .iter()
        .find(|d| d.name() == "short.rs")
        .unwrap();
    assert_eq!(
        short.text().rope().unwrap().to_string(),
        "fn changed() {}\n"
    );
    let a = &app.editor.documents[a];
    assert!(a.text().rope().unwrap().to_string().starts_with("mine "));
    assert!(!a.can_save_quietly());
    // 冲突的是当前文档：由冲突卡片提示，不另弹提示条。
    assert_eq!(app.active_conflict().as_deref(), Some("a.txt"));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);

    // ⌘P 的文件表在后台更新：测试中没有事件循环，需要等待更新完成后手动取回。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !app.picker.index.is_ready() || app.picker.index.len() == indexed {
        assert!(std::time::Instant::now() < deadline, "⌘P 的文件表没改好");
        std::thread::sleep(Duration::from_millis(5));
        app.picker_indexed();
    }
    assert_eq!(app.picker.index.len(), indexed + 1);
    assert!(app.picker.index.position_of(&dir.join("new.rs")).is_some());

    // 删除文件后：文件树和文件表中都不再有该文件。
    std::fs::remove_file(dir.join("new.rs")).unwrap();
    app.apply_changes(changes(&dir, &["new.rs"]));
    assert!(!tree_names(&app).contains(&"new.rs".to_owned()));
    while !app.picker.index.is_ready() || app.picker.index.len() != indexed {
        assert!(std::time::Instant::now() < deadline, "⌘P 的文件表没改好");
        std::thread::sleep(Duration::from_millis(5));
        app.picker_indexed();
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changes_are_handled_together_a_moment_after_the_first() {
    let dir = sample_folder("watch-settle");
    let mut app = app_with(&dir, false);
    // 测试中没有事件循环，不监视目录；但仍可验证收到通知与到期处理的时序。
    assert!(app.watcher.is_none());
    let now = std::time::Instant::now();
    app.files_changed();
    let due = app.changes_due.unwrap();
    assert!(due > now);
    // 后续到达的通知不推迟到期时刻：持续有改动时也会按时处理。
    app.files_changed();
    assert_eq!(app.changes_due, Some(due));
    assert_eq!(app.changes_tick(now), Some(due));
    assert_eq!(app.changes_tick(due), None);
    assert!(app.changes_due.is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_results_follow_only_the_files_that_changed() {
    let dir = sample_folder("watch-search");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir, false);
    app.global.query.set_text("println");
    app.run_global_search();
    finish_search(&mut app);
    let files = |app: &App| -> Vec<String> {
        let results = &app.global.engine.results;
        results.iter().map(|f| f.relative.clone()).collect()
    };
    let before = files(&app);
    assert!(before.contains(&"short.rs".to_owned()));
    assert!(before.contains(&"a.txt".to_owned()));

    // `short.rs` 中不再有匹配、`a.txt` 已删除：只重新搜索这两个文件并从结果中移除；
    // 新增的文件不进入结果（需要手动重新搜索）。
    std::fs::write(dir.join("short.rs"), "fn main() {}\n").unwrap();
    std::fs::remove_file(dir.join("a.txt")).unwrap();
    std::fs::write(dir.join("new.rs"), "println!();\n").unwrap();
    app.apply_changes(changes(&dir, &["short.rs", "a.txt", "new.rs"]));
    // 两个文件在后台重新搜索，搜索完成之前结果列表保持不变。
    assert_eq!(files(&app), before);
    finish_search(&mut app);
    let after = files(&app);
    assert!(!after.contains(&"short.rs".to_owned()));
    assert!(!after.contains(&"a.txt".to_owned()));
    assert!(!after.contains(&"new.rs".to_owned()));
    assert_eq!(after.len(), before.len() - 2);

    // 系统报告有通知被丢弃（`rescan`）：整体重新搜索，新文件也能搜到。
    app.apply_changes(Changes {
        paths: Vec::new(),
        rescan: true,
        git: false,
    });
    finish_search(&mut app);
    assert!(files(&app).contains(&"new.rs".to_owned()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_made_here_reach_quick_open_without_walking_again() {
    let dir = sample_folder("watch-made-here");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir, false);
    app.open_picker();
    app.close_picker();
    let before = app.picker.index.len();

    // 在文件树中一次创建两级的目录和文件：`docs` → `guide` → `intro.md`。
    app.start_new_entry(false);
    app.edit_field(|f| f.insert("docs/guide/intro.md"));
    app.submit_name_input();
    assert!(dir.join("docs/guide/intro.md").is_file());

    // 后台只更新新增的目录与文件（测试中没有事件循环，需要手动取回结果）。
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app
        .picker
        .index
        .position_of(&dir.join("docs/guide/intro.md"))
        .is_none()
    {
        assert!(std::time::Instant::now() < deadline, "⌘P 的文件表没改好");
        std::thread::sleep(Duration::from_millis(5));
        app.picker_indexed();
    }
    assert_eq!(app.picker.index.len(), before + 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn logs_written_by_others_are_followed_at_the_end() {
    use std::io::Write as _;

    let dir = sample_folder("watch-follow");
    let log = dir.join("app.log");
    let lines =
        |from: usize, to: usize| -> String { (from..to).map(|i| format!("line {i}\n")).collect() };
    std::fs::write(&log, lines(0, 200)).unwrap();
    let mut app = app_with(&log, false);
    let root = app.editor.root().unwrap().to_path_buf();
    let log = root.join("app.log");
    let append = |from: usize, to: usize, n: u64| {
        let mut file = std::fs::File::options().append(true).open(&log).unwrap();
        file.write_all(lines(from, to).as_bytes()).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5 * n))
            .unwrap();
    };
    let caret_line = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text().line_of(doc.selection.head)
    };

    // 光标在最后一行：其他程序向文件末尾追加内容，光标跟随到新的末尾并滚动到可见，
    // 不提示「已重新加载」。
    let end = app.editor.active().unwrap().text().end_pos();
    app.select(end, end);
    append(200, 300, 1);
    app.apply_changes(changes(&root, &["app.log"]));
    assert_eq!(caret_line(&app), 300);
    assert!(app.status.message.is_none());
    let doc = app.editor.active().unwrap();
    assert!(!doc.is_dirty());
    assert!(doc.scroll_line + app.visible_lines() as f64 > 300.0);

    // 光标在上方：追加的内容同样载入，光标位置和滚动位置都不变。
    let at = app.editor.active().unwrap().text().line_start(10);
    app.select(at, at);
    let scroll = app.editor.active().unwrap().scroll_line;
    append(300, 310, 2);
    app.apply_changes(changes(&root, &["app.log"]));
    assert_eq!(caret_line(&app), 10);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    assert_eq!(app.editor.active().unwrap().text().line_count(), 311);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn conflicts_wait_for_the_user_to_pick_a_side() {
    let dir = sample_folder("watch-conflict");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir.join("a.txt"), false);
    app.open_path(&dir.join("short.rs"));
    let disk = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();

    // 两个标签都有未保存的修改，磁盘上的文件又被其他程序修改。
    app.run(Command::Insert("// mine\n".into()));
    app.open_path(&dir.join("a.txt"));
    app.run(Command::Insert("mine ".into()));
    std::fs::write(dir.join("a.txt"), "theirs\n").unwrap();
    std::fs::write(dir.join("short.rs"), "fn theirs() {}\n").unwrap();
    touch_later(&dir.join("a.txt"));
    touch_later(&dir.join("short.rs"));
    app.apply_changes(changes(&dir, &["a.txt", "short.rs"]));

    // 当前标签显示冲突卡片；另一个标签只在提示条中列出。
    paint(&mut app);
    let (card, buttons) = app.hits.conflict_card.clone().unwrap();
    assert_eq!(buttons.len(), 3);
    assert!(buttons.iter().all(|(b, _)| card.contains(b.x, b.y)));
    assert!(card.right() <= app.layout.editor.right());
    let notice = app.notice.text.clone().unwrap_or_default();
    assert!(
        notice.contains("short.rs") && !notice.contains("a.txt"),
        "{notice}"
    );

    // 卡片遮挡下方的文字：悬停只识别按钮，点击卡片空白处不移动光标。
    let caret = app.editor.active().unwrap().selection;
    app.on_mouse_move(card.x + 20.0, card.y + card.h / 2.0);
    assert_eq!(app.hover, Hover::None);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.active().unwrap().selection, caret);

    // ⌘S 不会静默覆盖：没有系统提示框时提示先选择，磁盘上仍是其他程序写入的内容。
    app.save_active();
    assert_eq!(disk("a.txt"), "theirs\n");

    // 比较：打开并排的差异标签，左边是磁盘版本，右边是自己的修改。
    let button = |app: &App, action: ConflictAction| {
        let (_, buttons) = app.hits.conflict_card.clone().unwrap();
        buttons.iter().find(|(_, a)| *a == action).unwrap().0
    };
    let compare = button(&app, ConflictAction::Compare);
    app.on_mouse_move(compare.x + 4.0, compare.y + 4.0);
    assert_eq!(app.hover, Hover::ConflictButton(ConflictAction::Compare));
    app.on_mouse_down(MouseButton::Left);
    let diff = app.editor.active().unwrap();
    assert_eq!(diff.title.as_deref(), Some("比较：a.txt"));
    assert!(diff.readonly && !diff.shows_text());
    let view = diff.diff.as_deref().unwrap();
    assert_eq!(view.old.text.to_string(), "theirs\n");
    assert!(view.new.text.to_string().starts_with("mine "));
    assert_eq!(view.hunks.len(), 1);
    paint(&mut app);

    // 回到 a.txt，使用磁盘版本：内容替换为磁盘上的版本，没有未保存的修改；⌘Z 恢复本地的修改。
    app.open_path(&dir.join("a.txt"));
    paint(&mut app);
    let take = button(&app, ConflictAction::TakeDisk);
    app.on_mouse_move(take.x + 4.0, take.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "theirs\n");
    assert!(!doc.is_dirty() && !doc.in_conflict());
    paint(&mut app);
    assert!(app.hits.conflict_card.is_none());
    app.run(Command::Undo);
    assert!(app.editor.active().unwrap().is_dirty());

    // short.rs 保留本地的修改：冲突解除，之后 ⌘S 直接覆盖磁盘上的文件。
    app.open_path(&dir.join("short.rs"));
    paint(&mut app);
    let keep = button(&app, ConflictAction::KeepMine);
    app.on_mouse_move(keep.x + 4.0, keep.y + 4.0);
    app.on_mouse_down(MouseButton::Left);
    assert!(app.active_conflict().is_none());
    app.save_active();
    assert!(disk("short.rs").starts_with("// mine\n"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_save_path_overwrites_a_conflict() {
    let dir = sample_folder("watch-no-overwrite");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir.join("a.txt"), false);
    app.open_path(&dir.join("short.rs"));
    let id = app.editor.documents[0].id;

    // a.txt 有未保存的修改；其他程序修改了该文件，文件监视尚未发出通知。
    app.open_path(&dir.join("a.txt"));
    app.run(Command::Insert("mine ".into()));
    std::fs::write(dir.join("a.txt"), "theirs\n").unwrap();
    touch_later(&dir.join("a.txt"));
    app.open_path(&dir.join("short.rs"));

    // 关闭时选择「保存」与保存时格式化都经过同一个保存入口：不写入，切回 a.txt 等待用户选择。
    assert!(!app.save_document(id, None));
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "theirs\n"
    );
    assert_eq!(app.editor.active().unwrap().id, id);
    assert_eq!(app.active_conflict().as_deref(), Some("a.txt"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn background_rereads_keep_only_the_latest_and_skip_edited_documents() {
    let dir = sample_folder("watch-reread");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir, false);
    let path = dir.join("short.rs");
    app.open_path(&path);
    let index = app.editor.active.unwrap();
    let id = app.editor.documents[index].id;
    // 按 `sync_documents` 的方式发出一次重新读取，但不立即接收结果：先取得读取的内容，
    // 再按给定的顺序交回。
    let reread = |app: &mut App, text: &str, later: u64| {
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(later))
            .unwrap();
        let winc_editor::DiskSync::Reread(reread) = app.editor.documents[index].check_disk() else {
            panic!("干净的文档被改了应该要重读");
        };
        (app.rereads.tickets.issue(id), DiskBack::Read(reread.read()))
    };
    let text = |app: &App| {
        app.editor.documents[index]
            .text()
            .rope()
            .unwrap()
            .to_string()
    };

    // 文件连续修改两次，先发出的读取后到达：采用后发出的结果，后到达的旧内容不覆盖新内容。
    let (first, old) = reread(&mut app, "fn first() {}\n", 1);
    let (second, new) = reread(&mut app, "fn second() {}\n", 2);
    let sender = app.rereads.results.0.clone();
    sender.send((id, second, new)).unwrap();
    sender.send((id, first, old)).unwrap();
    app.poll_rereads();
    assert_eq!(text(&app), "fn second() {}\n");
    assert!(app.editor.documents[index].can_save_quietly());

    // 读取期间用户修改了文档：不采用读取的内容，改为冲突，用户的修改保留。
    let (third, theirs) = reread(&mut app, "fn theirs() {}\n", 3);
    app.run(Command::Insert("mine ".into()));
    sender.send((id, third, theirs)).unwrap();
    app.poll_rereads();
    assert!(text(&app).starts_with("mine fn second"));
    assert_eq!(app.active_conflict().as_deref(), Some("short.rs"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn background_tree_scans_keep_only_the_latest_per_folder() {
    let dir = sample_folder("watch-tree-scan");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let mut app = app_with(&dir, false);
    // 按 `apply_changes` 的方式发出一次扫描，但不立即接收结果：先完成扫描，再按给定顺序交回。
    let scan = |app: &mut App| {
        let scan = app.editor.workspace.as_ref().unwrap().scan(None);
        let tickets: Vec<u64> = scan
            .dirs()
            .iter()
            .map(|d| app.tree.scans.tickets.issue(d.clone()))
            .collect();
        (tickets, scan.run())
    };

    // 目录连续变化两次，先发出的扫描后到达：采用后发出的结果，后到达的旧内容不覆盖新内容。
    std::fs::write(dir.join("first.rs"), "").unwrap();
    let old = scan(&mut app);
    std::fs::write(dir.join("second.rs"), "").unwrap();
    let new = scan(&mut app);
    let sender = app.tree.scans.results.0.clone();
    sender.send(new).unwrap();
    sender.send(old).unwrap();
    app.poll_tree_scans();
    let names = tree_names(&app);
    assert!(names.iter().any(|n| n == "first.rs"), "{names:?}");
    assert!(names.iter().any(|n| n == "second.rs"), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn large_files_save_in_the_background_and_skip_their_own_write() {
    let dir = sample_folder("watch-large-save");
    let dir = winc_config::files::canonicalize(&dir).unwrap();
    let path = dir.join("big.log");
    let line = "2026-10-29 ok\n";
    let count = winc_editor::LARGE_FILE as usize / line.len() + 10;
    std::fs::write(&path, line.repeat(count)).unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    let index = app.editor.active.unwrap();
    let id = app.editor.documents[index].id;
    assert!(app.editor.documents[index].is_large());

    // ⌘S 在后台保存（测试中没有事件循环，保存就地完成并立即接收结果）。
    app.run(Command::Insert("A".into()));
    app.save_active();
    assert!(!app.saving(id));
    assert!(!app.editor.documents[index].is_dirty());
    let head = std::fs::read(&path).unwrap()[..line.len() + 1].to_vec();
    assert_eq!(head, format!("A{line}").into_bytes());

    // 正在后台保存的文档不随磁盘同步：本程序写出的文件不会被视为外部修改而报告冲突。
    app.run(Command::Insert("B".into()));
    app.saves.running.insert(id);
    touch_later(&path);
    app.apply_changes(changes(&dir, &["big.log"]));
    assert!(app.active_conflict().is_none());
    app.saves.running.remove(&id);

    let _ = std::fs::remove_dir_all(&dir);
}
