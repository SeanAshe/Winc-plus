//! 命令行打开不存在的文件的测试：打开为新文件、保存时才写入磁盘，其他入口仍说明文件不存在。

use std::time::{Duration, Instant};

use winc_editor::Document;
use winc_i18n::t;

use super::*;

/// 收下后台读取的结果，直到当前标签读好。
fn wait_loaded(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.editor.active().is_some_and(Document::is_pending) {
        assert!(Instant::now() < deadline, "读取没有完成");
        std::thread::sleep(Duration::from_millis(5));
        app.poll_loads();
    }
}

fn target(path: &Path) -> crate::cli::Target {
    crate::cli::Target {
        path: path.to_path_buf(),
        line: None,
        col: None,
    }
}

/// 交给正在运行的窗口打开的不存在的文件：新文件，状态栏说明保存后才写入磁盘；没改过时不记入
/// 现场，关闭不询问；保存时连同不存在的目录一起建出。
#[test]
fn missing_files_handed_over_open_as_new_files() {
    let dir = sample_folder("new-file-handed");
    let path = dir.join("notes/todo.md");
    let mut app = app_with(&dir, false);
    app.open_files(&[target(&path)]);
    wait_loaded(&mut app);

    let doc = app.editor.active().unwrap();
    assert!(doc.is_new_file() && !doc.is_dirty());
    assert_eq!(app.status.message.as_deref(), Some(t!("editor-new-file")));
    assert!(app.session_tab(doc).is_none() && !path.exists());

    app.run(Command::Insert("- 买牛奶".into()));
    app.save_active();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "- 买牛奶");
    assert!(!app.editor.active().unwrap().is_new_file());

    // 没改过的新文件关闭时不询问、不写入磁盘。
    let other = dir.join("draft.txt");
    app.open_files(&[target(&other)]);
    wait_loaded(&mut app);
    let count = app.editor.tab_count();
    app.close_tab(app.editor.active.unwrap());
    assert_eq!(app.editor.tab_count(), count - 1);
    assert!(!other.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文件树、快速打开等其他入口打开不存在的文件仍是说明原因的占位标签，不提示新文件。
#[test]
fn other_ways_of_opening_still_report_missing_files() {
    let dir = sample_folder("new-file-other");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("gone.txt"));
    wait_loaded(&mut app);
    assert!(app.editor.active().unwrap().unsupported.is_some());
    assert_ne!(app.status.message.as_deref(), Some(t!("editor-new-file")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 启动时命令行给出的不存在的文件同样打开为新文件；以分隔符结尾的不存在路径指目录，报告
/// 目录不存在。`--wait` 等待的新文件不保存就关闭时照常退出，磁盘上不留文件。
#[test]
fn missing_files_on_the_command_line_open_as_new_files_at_launch() {
    let dir = sample_folder("new-file-launch");
    let path = dir.join("COMMIT_NOTE");
    let folder = format!("{}/", dir.join("nowhere").display());
    let args = crate::cli::parse([path.clone().into_os_string(), folder.into()]).unwrap();
    let args = crate::cli::Args { wait: true, ..args };

    let mut editor = Editor::default();
    let opened = crate::open_targets(&mut editor, &args);
    assert_eq!(opened.problems.len(), 1, "{:?}", opened.problems);
    assert!(opened.problems[0].contains(t!("cmd-cli-no-such-folder")));
    assert_eq!(editor.documents.len(), 1);
    assert!(editor.active().unwrap().is_new_file());

    let mut app = App::new(editor, None, Bench::from_env(), Config::default());
    app.start_one_off(&args);
    let mut winc = crate::winc::Winc::new(None, None);
    let key = winc.add_detached(app);
    assert!(!winc.settle_for_test().1);
    winc.app(key).close_tab(0);
    assert!(winc.settle_for_test().1);
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
