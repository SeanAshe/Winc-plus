use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::files::canonicalize(&dir).unwrap()
}

#[test]
fn folders_are_read_relative_to_the_file() {
    let text = r#"{
        // 注释
        "folders": [
            { "path": "." },
            { "path": "../docs", "name": "Docs" },
            { "name": "no path" },
            { "path": "/abs", "name": " " },
        ],
    }"#;
    let folders = parse_project(text, Path::new("/work/app")).unwrap();
    assert_eq!(
        folders,
        [
            ProjectFolder {
                path: PathBuf::from("/work/app/."),
                name: None
            },
            ProjectFolder {
                path: PathBuf::from("/work/app/../docs"),
                name: Some("Docs".into())
            },
            ProjectFolder {
                path: PathBuf::from("/abs"),
                name: None
            },
        ]
    );
    assert_eq!(parse_project(r#"{ "folders": [] }"#, Path::new("/")), None);
    assert_eq!(parse_project("not json", Path::new("/")), None);
}

#[test]
fn project_files_are_told_by_their_extension() {
    assert!(is_project_file(Path::new("/a/app.winc-project")));
    assert!(is_project_file(Path::new("/a/APP.WINC-PROJECT")));
    assert!(!is_project_file(Path::new("/a/app.code-workspace")));
    assert!(!is_project_file(Path::new("/a/winc-project")));
}

#[test]
fn paths_are_written_relative_with_slashes() {
    assert_eq!(
        relative(Path::new("/w/app"), Path::new("/w/app")),
        PathBuf::from(".")
    );
    assert_eq!(
        relative(Path::new("/w/app"), Path::new("/w/docs/x")),
        PathBuf::from("../docs/x")
    );
    assert_eq!(
        relative(Path::new("/w"), Path::new("/w/app/src")),
        PathBuf::from("app/src")
    );
    assert_eq!(portable_path(Path::new("../docs/x")), "../docs/x");
}

#[test]
fn writing_keeps_comments_names_and_other_keys() {
    let dir = scratch("write");
    let (app, docs, extra) = (dir.join("app"), dir.join("docs"), dir.join("extra"));
    for folder in [&app, &docs, &extra] {
        std::fs::create_dir(folder).unwrap();
    }
    let file = app.join("team.winc-project");

    // 新建：第一项相对项目文件所在的目录。
    assert!(write_project(&file, &[app.clone(), docs.clone()]).unwrap());
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        text,
        "{\n    \"folders\": [\n        { \"path\": \".\" },\n        { \"path\": \"../docs\" }\n    ]\n}\n"
    );
    // 目录没有变化时不改写。
    assert!(!write_project(&file, &[app.clone(), docs.clone()]).unwrap());

    // 已有的注释、其他键和显示名保留，只换掉 `folders` 的值。
    let edited = "{\n    // 团队项目\n    \"folders\": [\n        { \"path\": \".\" },\n        { \"path\": \"../docs\", \"name\": \"文档\" } // 文档\n    ],\n    \"settings\": { \"a\": 1 }\n}\n";
    std::fs::write(&file, edited).unwrap();
    assert!(write_project(&file, &[app.clone(), docs.clone(), extra.clone()]).unwrap());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("{\n    // 团队项目\n    \"folders\": ["),
        "{text}"
    );
    assert!(
        text.contains("{ \"path\": \"../docs\", \"name\": \"文档\" },"),
        "{text}"
    );
    assert!(
        text.contains("{ \"path\": \"../extra\" }\n    ],\n    \"settings\": { \"a\": 1 }"),
        "{text}"
    );
    let folders = read_project(&file).unwrap();
    assert_eq!(folders.len(), 3);

    // 没有 `folders` 时加在对象开头；无效的文件不覆盖。
    std::fs::write(&file, "{ \"settings\": {} }").unwrap();
    assert!(write_project(&file, std::slice::from_ref(&app)).unwrap());
    assert!(read_project(&file).is_ok());
    std::fs::write(&file, "{ broken").unwrap();
    let error = write_project(&file, std::slice::from_ref(&app)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_kinds_follow_the_extension() {
    let kind = |name: &str| project_kind(Path::new(name));
    assert_eq!(kind("a.winc-project"), Some(ProjectKind::Winc));
    assert_eq!(kind("a.Code-Workspace"), Some(ProjectKind::VsCode));
    assert_eq!(kind("a.sublime-project"), Some(ProjectKind::Sublime));
    assert_eq!(kind("a.ipr"), Some(ProjectKind::Idea));
    assert_eq!(kind("a.sublime-workspace"), None);
}

#[test]
fn vs_code_folders_may_be_uris() {
    let text = r#"{
        "folders": [
            { "path": "app" },
            { "uri": "file:///work/%E6%96%87%E6%A1%A3", "name": "Docs" },
            { "uri": "vscode-remote://ssh-remote+devbox/srv/app" },
        ],
        "settings": { "editor.tabSize": 2 },
    }"#;
    let folders = parse_project(text, Path::new("/work")).unwrap();
    let paths: Vec<_> = folders.iter().map(|f| f.path.clone()).collect();
    let devbox = winc_backend::Target::new("devbox").unwrap();
    let remote = winc_backend::virtual_path(&devbox, b"/srv/app");
    assert_eq!(
        paths,
        [
            PathBuf::from("/work/app"),
            PathBuf::from("/work/文档"),
            remote
        ]
    );
    assert_eq!(folders[1].name.as_deref(), Some("Docs"));
    assert_eq!(
        foreign::folder_uri("file:///C:/src/app"),
        Some(PathBuf::from("C:/src/app"))
    );
    assert_eq!(
        foreign::folder_uri("file://localhost/srv"),
        Some(PathBuf::from("/srv"))
    );
    assert_eq!(foreign::folder_uri("file:///bad%2"), None);
    assert_eq!(foreign::folder_uri("vscode-remote://wsl+Ubuntu/home"), None);
    assert_eq!(foreign::folder_uri("https://example.com/x"), None);
}

#[test]
fn foreign_projects_are_read_by_kind() {
    let dir = scratch("foreign");
    let (app, lib) = (dir.join("app"), dir.join("lib"));
    std::fs::create_dir_all(app.join(".idea")).unwrap();
    std::fs::create_dir_all(&lib).unwrap();

    // Sublime Text：与本格式同样列出 `folders`，其余各项不读。
    let sublime = app.join("app.sublime-project");
    let text = "{ \"folders\": [ { \"path\": \".\" }, { \"path\": \"../lib\", \"name\": \"Lib\", \"folder_exclude_patterns\": [\"target\"] } ], \"build_systems\": [] }";
    std::fs::write(&sublime, text).unwrap();
    let folders = read_project(&sublime).unwrap();
    assert_eq!(folders[1].path, app.join("../lib"));
    assert_eq!(folders[1].name.as_deref(), Some("Lib"));

    // IDEA：放在 `.idea` 中的模块以项目目录为内容根目录；在项目目录之外的内容根目录作为附加
    // 的目录，`<modules>` 本身不算模块。
    std::fs::write(
        app.join(".idea/app.iml"),
        "<module type=\"JAVA_MODULE\"><component name=\"NewModuleRootManager\"><content url=\"file://$MODULE_DIR$\" /></component></module>",
    )
    .unwrap();
    std::fs::write(
        lib.join("lib.iml"),
        "<module><component><content url=\"file://$MODULE_DIR$\"><sourceFolder url=\"file://$MODULE_DIR$/src\" /></content></component></module>",
    )
    .unwrap();
    let modules = "<project version=\"4\"><component name=\"ProjectModuleManager\"><modules>\n<module fileurl=\"file://$PROJECT_DIR$/.idea/app.iml\" filepath=\"$PROJECT_DIR$/.idea/app.iml\" />\n<module filepath=\"$PROJECT_DIR$/../lib/lib.iml\" />\n<module filepath=\"$PROJECT_DIR$/missing.iml\" />\n</modules></component></project>";
    std::fs::write(app.join(".idea/modules.xml"), modules).unwrap();
    assert_eq!(idea_folders(&app), std::slice::from_ref(&lib));
    std::fs::write(app.join("app.ipr"), modules).unwrap();
    let folders = read_project(&app.join("app.ipr")).unwrap();
    let paths: Vec<_> = folders.iter().map(|f| f.path.clone()).collect();
    assert_eq!(paths, [app.clone(), lib.clone()]);
    assert!(idea_folders(&lib).is_empty());
    std::fs::write(app.join("bad.ipr"), "not xml").unwrap();
    assert!(matches!(
        read_project(&app.join("bad.ipr")),
        Err(ProjectError::Invalid)
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端主机上的目录写成 `host` 和其上的绝对路径，读回来是同一个虚拟路径。
#[test]
fn remote_folders_round_trip_with_their_host() {
    let dir = scratch("remote");
    let file = dir.join("ops.winc-project");
    let devbox = winc_backend::Target::new("deploy@devbox:2222").unwrap();
    let (app, logs) = (
        winc_backend::virtual_path(&devbox, b"/srv/app"),
        winc_backend::virtual_path(&devbox, b"/var/log"),
    );
    assert!(write_project(&file, &[app.clone(), logs.clone()]).unwrap());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("{ \"host\": \"deploy@devbox:2222\", \"path\": \"/srv/app\" }"),
        "{text}"
    );
    let paths: Vec<_> = read_project(&file)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, [app, logs]);
    assert_eq!(
        parse_project(
            r#"{ "folders": [ { "host": "bad host", "path": "/x" } ] }"#,
            &dir
        ),
        None
    );
    let _ = std::fs::remove_dir_all(&dir);
}
