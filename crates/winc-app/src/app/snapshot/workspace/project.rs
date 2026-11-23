//! 项目文件的测试：打开项目、目录变化写回、另存为、从会话按项目文件恢复、无效的文件。

use super::*;
use crate::app::input::context_menu::{MenuCommand, MenuEntry, MenuTarget};

/// 规范化后的目录 `dir`。
fn canonical(dir: PathBuf) -> PathBuf {
    winc_config::files::canonicalize(&dir).unwrap()
}

/// 项目菜单中列出的命令。
fn project_menu(app: &mut App) -> Vec<MenuCommand> {
    app.open_context_menu(MenuTarget::Project, (10.0, 10.0));
    let menu = app.overlays.context_menu.take().unwrap();
    (menu.entries.into_iter())
        .filter_map(|entry| match entry {
            MenuEntry::Item(command, _) => Some(command),
            _ => None,
        })
        .collect()
}

/// 打开项目文件：第一个目录是打开的目录，其余为附加的目录，窗口用项目文件的名字；之后增减目录
/// 写回项目文件，注释、其他内容和显示名保留。
#[test]
fn a_project_file_opens_its_folders_and_records_changes() {
    let main = canonical(sample_folder("project-main"));
    let docs = canonical(sample_folder("project-docs"));
    let extra = canonical(sample_folder("project-extra"));
    let file = main.join("team.winc-project");
    let text = "{\n    // 团队项目\n    \"folders\": [\n        { \"path\": \".\" },\n        { \"path\": \"../project-docs\", \"name\": \"文档\" }\n    ],\n    \"settings\": {}\n}\n";
    std::fs::write(&file, text).unwrap();

    let mut app = blank_app(false);
    app.open_from_outside(&file);
    assert_eq!(app.editor.root(), Some(main.as_path()));
    assert_eq!(app.extra_folders(), std::slice::from_ref(&docs));
    let workspace = app.editor.workspace.as_ref().unwrap();
    assert_eq!(workspace.project(), Some(file.as_path()));
    assert_eq!(workspace.name, "team");
    assert!(
        app.editor
            .documents
            .iter()
            .all(|doc| doc.path.as_deref() != Some(&file))
    );

    // 项目的名字取自项目文件：没有起名的一项，可以另存为。
    let menu = project_menu(&mut app);
    assert!(menu.contains(&MenuCommand::SaveProjectAs));
    // 窗口内打开（文件树、快速打开）时作为文本编辑。
    app.open_path(&file);
    let active = app.editor.active().and_then(|doc| doc.path.clone());
    assert_eq!(active.as_deref(), Some(file.as_path()));
    assert!(!menu.contains(&MenuCommand::RenameGroup));

    app.add_folder(&extra);
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.starts_with("{\n    // 团队项目\n"), "{saved}");
    assert!(
        saved.contains("{ \"path\": \"../project-docs\", \"name\": \"文档\" },"),
        "{saved}"
    );
    assert!(
        saved.contains("{ \"path\": \"../project-extra\" }\n    ],\n    \"settings\": {}"),
        "{saved}"
    );
    // 现场和最近列表按项目文件记。
    let session = app.state.session_for(&file).unwrap();
    assert_eq!(session.project.as_deref(), Some(file.as_path()));
    assert_eq!(app.state.recent.first(), Some(&file));

    // 移除到只剩打开的目录：名字仍是项目的。
    app.remove_folder(&docs);
    app.remove_folder(&extra);
    assert_eq!(app.editor.workspace.as_ref().unwrap().name, "team");
    let folders = winc_config::read_project(&file).unwrap();
    assert_eq!(folders.len(), 1);
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_dir_all(&extra);
}

/// 「将项目另存为」写出相对路径的项目文件，缺少扩展名时补上；项目文件被外部修改后按新内容恢复。
#[test]
fn a_window_is_saved_as_a_project_and_reopened_from_it() {
    let main = canonical(sample_folder("project-save-main"));
    let other = canonical(sample_folder("project-save-other"));
    let later = canonical(sample_folder("project-save-later"));
    let mut app = app_with(&main.join("short.rs"), false);
    app.add_folder(&other);
    app.save_project_as(main.join("mine"));
    let file = main.join("mine.winc-project");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "{\n    \"folders\": [\n        { \"path\": \".\" },\n        { \"path\": \"../project-save-other\" }\n    ]\n}\n"
    );
    let workspace = app.editor.workspace.as_ref().unwrap();
    assert_eq!(workspace.project(), Some(file.as_path()));
    assert_eq!(workspace.name, "mine");
    assert_eq!(
        app.status.message.as_deref(),
        Some("已保存项目「mine.winc-project」")
    );

    // 外部修改项目文件后，从最近列表重新打开时按新内容恢复；最近列表显示项目名。
    let edited =
        "{ \"folders\": [ { \"path\": \".\" }, { \"path\": \"../project-save-later\" } ] }";
    std::fs::write(&file, edited).unwrap();
    let mut again = blank_app(false);
    again.state = app.state.clone();
    assert_eq!(again.state.recent.first(), Some(&file));
    assert_eq!(again.recent_title(&file), "mine");
    assert_eq!(again.recent_tag(&file), Some("项目"));
    assert_eq!(again.recent_label(0).title, "mine（项目）");
    assert_eq!(again.recent_tag(&main), None);
    again.run_start_action(crate::app::StartAction::Recent(0));
    assert_eq!(again.editor.root(), Some(main.as_path()));
    assert_eq!(again.extra_folders(), std::slice::from_ref(&later));
    assert_eq!(again.editor.workspace.as_ref().unwrap().name, "mine");
    // 单独打开第一个目录时不带项目。
    let mut alone = blank_app(false);
    alone.state = app.state.clone();
    alone.open_folder_now(&main);
    let workspace = alone.editor.workspace.as_ref().unwrap();
    assert_eq!(workspace.project(), None);
    for dir in [&main, &other, &later] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// 打开的目录已开在窗口里时打开它的项目文件：就地换上项目的附加目录，不重新打开。
#[test]
fn opening_the_project_of_the_open_folder_applies_it_in_place() {
    let main = canonical(sample_folder("project-here-main"));
    let docs = canonical(sample_folder("project-here-docs"));
    let file = main.join("here.winc-project");
    let text = "{ \"folders\": [ { \"path\": \".\" }, { \"path\": \"../project-here-docs\" } ] }";
    std::fs::write(&file, text).unwrap();
    let mut app = app_with(&main.join("short.rs"), false);
    let tabs = app.editor.tab_count();
    app.open_from_outside(&file);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&docs));
    assert_eq!(app.editor.tab_count(), tabs);
    assert_eq!(app.editor.workspace.as_ref().unwrap().name, "here");
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&docs);
}

/// 无效的项目文件不打开任何目录，说明原因，也不作为文本打开。
#[test]
fn an_invalid_project_file_is_reported() {
    let dir = canonical(sample_folder("project-invalid"));
    let file = dir.join("broken.winc-project");
    std::fs::write(&file, "{ broken").unwrap();
    let mut app = blank_app(false);
    app.open_from_outside(&file);
    assert_eq!(app.editor.root(), None);
    assert_eq!(app.editor.tab_count(), 0);
    assert_eq!(
        app.notice.text.as_deref(),
        Some("「broken.winc-project」不是有效的项目文件")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 其他编辑器的项目文件只读：按其中的目录打开，以文件名为这一组的名字，增减目录不改写原文件；
/// 可以另存为 Winc 的项目文件。
#[test]
fn foreign_project_files_open_read_only() {
    let main = canonical(sample_folder("project-vscode-main"));
    let docs = canonical(sample_folder("project-vscode-docs"));
    let extra = canonical(sample_folder("project-vscode-extra"));
    let file = main.join("team.code-workspace");
    let text = "{\n  // VS Code\n  \"folders\": [{ \"path\": \".\" }, { \"path\": \"../project-vscode-docs\" }],\n  \"settings\": {}\n}\n";
    std::fs::write(&file, text).unwrap();
    let mut app = blank_app(false);
    app.open_from_outside(&file);
    assert_eq!(app.editor.root(), Some(main.as_path()));
    assert_eq!(app.extra_folders(), std::slice::from_ref(&docs));
    let workspace = app.editor.workspace.as_ref().unwrap();
    assert_eq!(workspace.project(), None);
    assert_eq!(workspace.name, "team");

    app.add_folder(&extra);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    let session = app.state.session_for(&main).unwrap();
    assert_eq!(session.folders, [docs.clone(), extra.clone()]);
    assert_eq!(session.project, None);
    app.save_project_as(main.join("team"));
    let saved = winc_config::read_project(&main.join("team.winc-project")).unwrap();
    assert_eq!(saved.len(), 3);
    for dir in [&main, &docs, &extra] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// 基于目录的 IDEA 项目：打开目录时，在它之外的模块目录作为附加的目录。
#[test]
fn idea_modules_outside_the_folder_are_added() {
    let main = canonical(sample_folder("project-idea-main"));
    let lib = canonical(sample_folder("project-idea-lib"));
    std::fs::create_dir_all(main.join(".idea")).unwrap();
    let iml = "<module><component><content url=\"file://$MODULE_DIR$\" /></component></module>";
    std::fs::write(lib.join("lib.iml"), iml).unwrap();
    std::fs::write(main.join(".idea/main.iml"), iml).unwrap();
    let modules = "<project><component><modules><module filepath=\"$PROJECT_DIR$/.idea/main.iml\" /><module filepath=\"$PROJECT_DIR$/../project-idea-lib/lib.iml\" /></modules></component></project>";
    std::fs::write(main.join(".idea/modules.xml"), modules).unwrap();
    let mut app = blank_app(false);
    app.open_folder_now(&main);
    assert_eq!(app.extra_folders(), std::slice::from_ref(&lib));
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&lib);
}

/// 加了第二个目录、还没有项目文件时是未保存的项目，菜单里是「保存项目…」；保存后是「将项目另存
/// 为…」。没有打开目录时也可以「打开项目…」。
#[test]
fn an_unsaved_project_is_marked_until_saved() {
    let main = canonical(sample_folder("project-unsaved-main"));
    let other = canonical(sample_folder("project-unsaved-other"));
    assert!(project_menu(&mut blank_app(false)).contains(&MenuCommand::OpenProject));
    let mut app = app_with(&main.join("short.rs"), false);
    assert!(!app.project_unsaved());
    let menu = project_menu(&mut app);
    assert!(menu.contains(&MenuCommand::CloseFolder));
    app.add_folder(&other);
    assert!(app.project_unsaved());
    let menu = project_menu(&mut app);
    assert!(menu.contains(&MenuCommand::SaveProject));
    assert!(!menu.contains(&MenuCommand::SaveProjectAs));
    // 窗口中的是项目时「关闭目录」是「关闭项目」。
    assert!(menu.contains(&MenuCommand::CloseProject));
    assert!(!menu.contains(&MenuCommand::CloseFolder));

    app.save_project_as(main.join("team"));
    assert!(!app.project_unsaved());
    let menu = project_menu(&mut app);
    assert!(menu.contains(&MenuCommand::SaveProjectAs));
    assert!(!menu.contains(&MenuCommand::SaveProject));
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&other);
}

/// 在窗口内以文本打开其他项目的项目文件时提示可打开该项目：作为当前标签时常驻，切走收起、
/// 切回再现，按钮打开项目；当前项目的文件不提示。
#[test]
fn a_project_file_opened_as_text_offers_to_open_it() {
    let main = canonical(sample_folder("project-offer-main"));
    let docs = canonical(sample_folder("project-offer-docs"));
    let file = docs.join("docs.code-workspace");
    std::fs::write(&file, "{ \"folders\": [ { \"path\": \".\" } ] }").unwrap();
    let mut app = app_with(&main.join("short.rs"), false);
    app.open_path(&file);
    let offered = Some("「docs.code-workspace」是项目文件");
    assert_eq!(app.notice.text.as_deref(), offered);
    assert_eq!(app.notice.until, None);
    app.activate_tab(0);
    assert_eq!(app.notice.text, None);
    app.activate_tab(1);
    assert_eq!(app.notice.text.as_deref(), offered);
    // 关掉以后，在换标签之前不再提示。
    app.dismiss_notice();
    app.relayout();
    assert_eq!(app.notice.text, None);
    app.activate_tab(0);
    app.activate_tab(1);
    assert_eq!(app.notice.text.as_deref(), offered);
    app.run_notice_action();
    assert_eq!(app.editor.root(), Some(docs.as_path()));

    let own = main.join("own.winc-project");
    std::fs::write(&own, "{ \"folders\": [ { \"path\": \".\" } ] }").unwrap();
    let mut app = blank_app(false);
    app.open_from_outside(&own);
    app.dismiss_notice();
    app.open_path(&own);
    assert_eq!(app.notice.text, None);
    let _ = std::fs::remove_dir_all(&main);
    let _ = std::fs::remove_dir_all(&docs);
}
