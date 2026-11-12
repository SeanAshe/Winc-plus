//! [`State`] 的测试：多进程读改写、容错读入、徽章颜色、现场与工具选择的记录。

use super::*;

#[test]
fn several_windows_keep_each_others_changes() {
    let dir = std::env::temp_dir().join(format!("winc-state-merge-{}", std::process::id()));
    let path = dir.join("state.json");
    let (x, y) = (Path::new("/p/x"), Path::new("/p/y"));

    // 模拟两个进程先后读改写同一个文件：第一个记下 X 的现场与书签，第二个打开 Y。
    let (_, ok) = State::update(&path, |s| s.remember(x));
    ok.unwrap();
    let (_, ok) = State::update(&path, |s| {
        s.set_session(Session {
            folder: x.to_path_buf(),
            bookmarks: vec![(x.join("a.rs"), vec![3])],
            ..Session::default()
        });
    });
    ok.unwrap();
    let (_, ok) = State::update(&path, |s| {
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

fn sample() -> State {
    let mut state = State {
        recent: vec![PathBuf::from("/a/项目"), PathBuf::from("/b")],
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
                pdf_places: vec![(PathBuf::from("/a/项目/手册.pdf"), (12, 0.375))],
                terminal: Some(TerminalSession {
                    tabs: vec![PathBuf::from("/a/项目"), PathBuf::from("/a/项目/src")],
                    names: vec![String::new(), "服务".into()],
                    shells: vec![String::new(), "/bin/bash".into()],
                    active: 1,
                    open: true,
                    height: 320.0,
                    width: 0.0,
                }),
                run: Some("后端".into()),
                sidebar_hidden: true,
            },
            Session {
                folder: PathBuf::from("/b"),
                tabs: vec![Tab::File(PathBuf::from("/b/x"))],
                active: None,
                bookmarks: Vec::new(),
                outline: Vec::new(),
                folds: Vec::new(),
                pdf_places: Vec::new(),
                terminal: None,
                run: None,
                sidebar_hidden: false,
            },
        ],
        last: Some(PathBuf::from("/a/项目")),
        others: vec![PathBuf::from("/b")],
        commit_tree: true,
        pdf_thumbnails_hidden: true,
        vim_marks: vec![('A', PathBuf::from("/a/项目/x.rs"), 12, 3)],
        vim_commands: vec!["%s/a/b/g".into(), "w".into()],
        vim_searches: vec!["\\<词\\>".into()],
        stay_outside_applications: true,
        panel_width: Some(360.0),
        preview_mode: Some("split".into()),
        window: Some(WindowFrame {
            position: Some((-1280.0, 40.0)),
            width: 900.5,
            height: 600.0,
            scale: 2.0,
            maximized: true,
        }),
        badges: Vec::new(),
        tool_choices: vec![(PathBuf::from("/a/项目"), true)],
        trusted_repos: vec![(PathBuf::from("/a/项目"), u64::MAX - 1)],
    };
    state.assign_badges();
    state
}

#[test]
fn round_trips_through_json() {
    let state = sample();
    assert_eq!(State::parse(&state.to_json()), state);
    assert_eq!(State::parse(&State::default().to_json()), State::default());
}

#[test]
fn malformed_pdf_places_are_skipped() {
    // 页号为负数或小数、比例不是数字、缺少一项的记录跳过，其余仍然读取。
    let text = r#"{"session": {"folder": "/a", "files": [], "pdfPlaces":
        {"/a/a.pdf": [2, 0.5], "/a/b.pdf": [-1, 0], "/a/c.pdf": [1.5, 0], "/a/d.pdf": [3],
         "/a/e.pdf": [0, "x"], "/a/f.pdf": 4, "": [1, 0]}}}"#;
    assert_eq!(
        State::parse(text).last_session().unwrap().pdf_places,
        [(PathBuf::from("/a/a.pdf"), (2, 0.5))]
    );
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
    for i in 0..15 {
        state.remember(Path::new(&format!("/p{i}")));
    }
    assert_eq!(state.recent.len(), MAX_RECENT);
    assert_eq!(state.recent[0], PathBuf::from("/p14"));

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
fn recent_folders_keep_their_own_badge_colors() {
    let mut state = State::default();
    for i in 0..MAX_RECENT {
        state.remember(Path::new(&format!("/projects/p{i}")));
    }

    // 最近列表中各目录的颜色互不相同。
    let colors = |state: &State| -> Vec<usize> {
        state.recent.iter().map(|p| state.badge_color(p)).collect()
    };
    let mut all = colors(&state);
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), MAX_RECENT);

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

    // 新目录替换最旧的目录并获得其释放的颜色；序列化后再读回，颜色保持不变。
    let oldest = state.recent.last().cloned().unwrap();
    let freed = state.badge_color(&oldest);
    state.remember(Path::new("/projects/new"));
    assert_eq!(state.badge_color(Path::new("/projects/new")), freed);
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
        pdf_places: Vec::new(),
        terminal: None,
        run: None,
        sidebar_hidden: false,
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
    assert_eq!(state.sessions.len(), MAX_RECENT);

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
