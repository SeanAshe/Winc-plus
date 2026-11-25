//! 热退出备份的测试：备份间隔随内容大小放大，超过上限时封顶。

use super::*;

impl App {
    /// 测试用：本窗口是否持有备份锁。
    pub(in crate::app) fn holds_backup_lock(&self) -> bool {
        self.backup.lock.as_ref().is_some_and(|l| l.1.is_some())
    }

    /// 测试用：清除已排定的备份到期时刻，下一次 [`App::backup_tick`] 重新排定。
    pub(in crate::app) fn forget_backup_due(&mut self) {
        self.backup.due = None;
    }
}

#[test]
fn large_contents_are_backed_up_less_often() {
    // 小的内容按 1 秒的间隔写一次；越大等得越久，64 MiB 约 8.5 秒，再大也不超过 30 秒。
    assert_eq!(backup_delay(0), BACKUP_DELAY);
    assert_eq!(backup_delay(BACKUP_SMALL), BACKUP_DELAY);
    let big = backup_delay(64 << 20);
    assert!(big > Duration::from_secs(8) && big < Duration::from_secs(9));
    assert_eq!(backup_delay(1 << 30), BACKUP_MAX_DELAY);
    assert!(backup_delay(16 << 20) < backup_delay(32 << 20));
}

/// 回归：先写新备份再删旧备份，路径变化的文档在两步之间中止时仍保留旧备份；删除时跳过刚
/// 写好的文件和这次写失败的文档的旧备份。
#[test]
fn stale_backups_are_removed_only_when_safe() {
    let dir = std::env::temp_dir().join(format!("winc-stale-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, "x").unwrap();
        path
    };
    let (reused, kept, gone) = (file("reused"), file("kept"), file("gone"));
    let removes = vec![
        // 关闭的标签的旧备份，与新打开的标签刚写的备份同名。
        (DocumentId(1), reused.clone()),
        // 改了路径、新备份没写成的文档：旧备份是该文档仅有的备份。
        (DocumentId(2), kept.clone()),
        (DocumentId(3), gone.clone()),
    ];
    let failed = vec![(DocumentId(2), "磁盘已满".to_owned())];
    remove_stale(removes, &[reused.as_path()], &failed);
    assert!(reused.exists() && kept.exists());
    assert!(!gone.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

fn editor_group(pane: usize, tab: usize, active: bool) -> winc_config::TerminalGroup {
    winc_config::TerminalGroup {
        tabs: 1,
        share: 1.0,
        editor: Some(winc_config::TerminalEditor { pane, tab, active }),
    }
}

fn terminal_session(
    groups: Vec<winc_config::TerminalGroup>,
    names: &[&str],
) -> winc_config::TerminalSession {
    winc_config::TerminalSession {
        tabs: vec![PathBuf::from("/p"); groups.iter().map(|g| g.tabs).sum()],
        groups,
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        ..winc_config::TerminalSession::default()
    }
}

fn restored_session(session: &Session, present: &[usize]) -> (Editor, Restored) {
    let mut editor = Editor::default();
    let files = present
        .iter()
        .filter_map(|&at| match &session.tabs[at] {
            Tab::File(path) => Some(path.clone()),
            Tab::Untitled(_) => None,
        })
        .collect();
    let restored = restore_prepared(
        &mut editor,
        Some(session),
        Prepared {
            files,
            ..Prepared::default()
        },
    );
    (editor, restored)
}

#[test]
fn a_closed_panel_restores_its_single_editor_marker_without_starting_a_shell() {
    let session = Session {
        terminal: Some(terminal_session(
            vec![editor_group(0, 0, true)],
            &["server"],
        )),
        ..Session::default()
    };
    let (mut editor, restored) = restored_session(&session, &[]);
    assert_eq!(editor.tab_count(), 1);
    let marker = editor.active().unwrap();
    assert!(marker.terminal && marker.path.is_none() && marker.readonly_reason().is_some());
    assert_eq!(marker.name(), "server");
    let id = marker.id;
    assert_eq!(restored.terminal_groups, [(1.0, Some(id))]);
    assert!(session.tabs.is_empty());
    assert!(!session.terminal.as_ref().unwrap().open);
    let markers = restore_session_panes(&mut editor, &session, &[], &[Some(id)]);
    assert_eq!(markers, [Some(id)]);
    assert_eq!(
        editor.documents.len(),
        1,
        "the PTY restore must reuse this marker"
    );
}

#[test]
fn missing_files_do_not_shift_interleaved_or_adjacent_terminal_markers() {
    // 保存顺序：缺失文件、first、a、second、third、缺失文件、b。
    // 终端的原始存储栏顺序则是 third、first、second。
    let session = Session {
        tabs: ["gone-before.rs", "a.rs", "gone-between.rs", "b.rs"]
            .into_iter()
            .map(|name| Tab::File(Path::new("/p").join(name)))
            .collect(),
        active: Some(3),
        terminal: Some(terminal_session(
            vec![
                editor_group(0, 4, false),
                editor_group(0, 1, false),
                editor_group(0, 3, true),
            ],
            &["third", "first", "second"],
        )),
        ..Session::default()
    };
    let (editor, restored) = restored_session(&session, &[1, 3]);
    let names: Vec<_> = editor
        .tabs()
        .iter()
        .map(|doc| doc.name().into_owned())
        .collect();
    assert_eq!(names, ["first", "a.rs", "second", "third", "b.rs"]);
    assert_eq!(
        editor.active().unwrap().name(),
        "second",
        "file-active fallback must not override it"
    );
    for (at, name) in ["third", "first", "second"].into_iter().enumerate() {
        let id = restored.terminal_groups[at].1.unwrap();
        assert_eq!(editor.document(id).unwrap().name(), name);
    }
}

#[test]
fn terminal_only_split_panes_keep_the_tree_focus_and_each_active_marker() {
    use winc_config::{PaneNode, SessionPane, SessionPanes};
    use winc_editor::panes::PaneId;
    let tree = PaneNode::Split {
        column: true,
        share: 0.3,
        first: Box::new(PaneNode::Pane(0)),
        second: Box::new(PaneNode::Split {
            column: false,
            share: 0.6,
            first: Box::new(PaneNode::Pane(1)),
            second: Box::new(PaneNode::Pane(2)),
        }),
    };
    let mut split_group = editor_group(1, 0, true);
    split_group.tabs = 2;
    split_group.share = 0.4;
    let session = Session {
        panes: Some(SessionPanes {
            tree: tree.clone(),
            panes: vec![
                SessionPane {
                    tabs: Vec::new(),
                    active: None
                };
                3
            ],
            focused: 2,
        }),
        terminal: Some(terminal_session(
            vec![
                editor_group(2, 0, true),
                split_group,
                editor_group(0, 0, true),
            ],
            &["bottom", "split", "split-member", "left"],
        )),
        ..Session::default()
    };
    let (editor, restored) = restored_session(&session, &[]);
    assert_eq!(
        editor.documents.len(),
        3,
        "split shells share one editor marker"
    );
    assert_eq!(
        editor.panes.tree,
        crate::app::workspace::panes::pane_tree(&tree)
    );
    assert_eq!(editor.panes.focused, PaneId(2));
    assert_eq!(editor.active().unwrap().name(), "bottom");
    for (pane, name) in [(0, "left"), (1, "split"), (2, "bottom")] {
        let (tabs, active) = editor.pane_tabs(PaneId(pane));
        assert_eq!(tabs.len(), 1);
        assert_eq!(active, Some(tabs[0]));
        assert_eq!(editor.document(tabs[0]).unwrap().name(), name);
    }
    assert_eq!(restored.terminal_groups[1].0, 0.4);
}

#[test]
fn an_inactive_marker_does_not_override_the_active_file_fallback() {
    let session = Session {
        tabs: vec![Tab::File(PathBuf::from("/p/a.rs"))],
        active: Some(0),
        panes: Some(winc_config::SessionPanes {
            tree: winc_config::PaneNode::Pane(0),
            panes: vec![winc_config::SessionPane {
                tabs: vec![0],
                active: None,
            }],
            focused: 0,
        }),
        terminal: Some(terminal_session(
            vec![editor_group(0, 0, false)],
            &["terminal"],
        )),
        ..Session::default()
    };
    let (editor, _) = restored_session(&session, &[0]);
    assert_eq!(editor.active().unwrap().name(), "a.rs");
    assert_eq!(editor.tabs()[0].name(), "terminal");
}

#[test]
fn a_legacy_file_only_session_still_discards_missing_slots() {
    let session = Session {
        tabs: vec![
            Tab::File(PathBuf::from("/p/gone.rs")),
            Tab::File(PathBuf::from("/p/a.rs")),
        ],
        active: Some(1),
        ..Session::default()
    };
    let (editor, restored) = restored_session(&session, &[1]);
    assert_eq!(editor.documents.len(), 1);
    assert_eq!(editor.active().unwrap().name(), "a.rs");
    assert!(restored.terminal_groups.is_empty());
}

#[test]
fn terminal_editor_disabled_module_does_not_leave_restored_empty_tabs() {
    let session = Session {
        terminal: Some(terminal_session(
            vec![editor_group(0, 0, true)],
            &["restored"],
        )),
        ..Session::default()
    };
    let (editor, restored) = restored_session(&session, &[]);
    let mut app = App::new(
        editor,
        None,
        crate::bench::Bench::from_env(),
        winc_config::Config::default(),
    );
    app.adopt_restored(restored);
    assert!(app.editor.documents.is_empty());
    assert!(!app.terminal_in_editor());
    assert!(app.terminal.groups.is_empty());
}
