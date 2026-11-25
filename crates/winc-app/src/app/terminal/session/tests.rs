//! 终端现场的测试：混合标签位置、原始栏份额和连续的 shell 启动条目。

use super::*;
use winc_editor::Editor;

#[test]
fn mixed_positions_skip_ephemeral_tabs_and_shell_entries_stay_grouped() {
    let mut editor = Editor::default();
    let a = editor.open_pending(std::path::Path::new("/p/a.rs"));
    let a = editor.documents[a].id;
    editor.show_output("temporary", "output");
    let first = editor.new_terminal("first");
    let first = editor.documents[first].id;
    editor.show_output("unpersisted", "output");
    let b = editor.open_pending(std::path::Path::new("/p/b.rs"));
    let b = editor.documents[b].id;
    let second = editor.new_terminal("second");
    let second = editor.documents[second].id;
    let mut app = App::new(
        editor,
        None,
        crate::bench::Bench::from_env(),
        winc_config::Config::default(),
    );
    let mut second_group = groups::TermGroup::empty(0.4);
    second_group.editor = Some(second);
    let mut first_group = groups::TermGroup::empty(0.7);
    first_group.editor = Some(first);
    app.terminal.groups = vec![second_group, first_group];
    app.terminal.want = vec![
        b"/p/first-one".to_vec(),
        b"/p/second".to_vec(),
        b"/p/first-two".to_vec(),
    ];
    app.terminal.want_groups = vec![1, 0, 1];
    app.terminal.want_names = vec!["one".into(), "second".into(), "two".into()];
    app.terminal.want_shells = vec![Some("sh".into()), None, Some("bash".into())];

    let saved = app.terminal_session_with_tabs(&[(a, 0), (b, 1)]).unwrap();
    assert_eq!(
        saved.tabs,
        ["/p/second", "/p/first-one", "/p/first-two"].map(PathBuf::from)
    );
    assert_eq!(saved.names, ["second", "one", "two"]);
    assert_eq!(saved.shells, ["", "sh", "bash"]);
    assert_eq!(saved.groups[0].tabs, 1);
    assert_eq!(saved.groups[1].tabs, 2);
    assert_eq!(saved.groups[0].share, 0.4);
    assert_eq!(saved.groups[1].share, 0.7);
    assert_eq!(
        saved.groups[0].editor,
        Some(TerminalEditor {
            pane: 0,
            tab: 3,
            active: true
        })
    );
    assert_eq!(
        saved.groups[1].editor,
        Some(TerminalEditor {
            pane: 0,
            tab: 1,
            active: false
        })
    );
    assert!(!saved.open);
}
