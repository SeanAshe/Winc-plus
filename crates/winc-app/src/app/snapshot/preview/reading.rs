//! 阅读视图的测试：docx 文档与更新日志。

use super::*;

/// docx 打开后显示阅读视图：只读、铺满编辑区，标题出现在标题指示条中，包中的图片从内存读取；
/// 右键菜单没有「在源代码中显示」。文件损坏时显示原因和重试、用默认应用打开的按钮。
#[test]
fn docx_files_open_as_a_reading_view() {
    let dir = sample_folder("md-docx");
    let path = dir.join("季度报告.docx");
    std::fs::write(&path, sample_docx()).unwrap();
    let mut app = app_with(&path, false);
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.unsupported, Some(winc_editor::Unsupported::Docx));
    assert!(doc.readonly);
    assert_eq!(app.preview.markdown.area, app.layout.editor);
    assert!(app.preview.markdown.failed.is_none());
    assert_eq!(app.hits.markdown_toc_tops.len(), 2);
    // 图片按文档所在目录解析出路径，但不读磁盘：磁盘上没有这个文件。
    let opened = doc.path.clone().unwrap();
    let logo = opened.parent().unwrap().join("word/media/logo.svg");
    assert!(!logo.exists());
    assert!(matches!(
        app.preview.markdown.images.get(&logo),
        Some(crate::app::preview::markdown::ImageSlot::Ready { pixels, .. }) if !pixels.is_empty()
    ));
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert!(laid.chars() > 0);

    let area = app.preview.markdown.area;
    right_click(&mut app, (area.x + area.w / 2.0, area.y + 60.0));
    assert!(
        !menu_commands(&app)
            .iter()
            .any(|(c, _)| *c == MenuCommand::RevealSource)
    );

    let broken = dir.join("损坏.docx");
    std::fs::write(&broken, b"not a docx").unwrap();
    app.open_path(&broken);
    settle(&mut app);
    assert_eq!(
        app.preview.markdown.failed.as_deref(),
        Some(winc_i18n::t!("preview-docx-invalid"))
    );
    assert_eq!(app.hits.placeholder.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 打开更新日志时标签先出现并提示正在读取；内容到达后在同一个标签中排版显示：只读、铺满
/// 编辑区，没有可以切换到的源代码。未能读取时标签中说明，并给出重试和在浏览器中查看的按钮。
#[test]
fn release_notes_open_in_a_reading_tab() {
    use crate::release_notes::Scope;
    let mut app = blank_app(false);
    let tabs = app.editor.documents.len();
    app.open_release_notes(Scope::Since(Some("0.1.0".into())));
    settle(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.name(), winc_i18n::t!("workspace-release-notes-title"));
    assert_eq!(doc.reading, Some(winc_editor::Reading::Loading));
    assert!(doc.readonly && !doc.shows_text() && !doc.can_preview());
    assert_eq!(app.editor.documents.len(), tabs + 1);
    // 正在读取：没有排版的内容，也没有按钮。
    assert_eq!(app.preview.markdown.area, Rect::default());
    assert!(app.hits.placeholder.is_empty());

    app.show_release_notes(Some(
        "## 0.2.0 · 2026-02-01\n\n### 新功能\n\n- 甲\n- 乙\n".into(),
    ));
    settle(&mut app);
    assert_eq!(app.editor.documents.len(), tabs + 1);
    assert_eq!(app.preview.markdown.area, app.layout.editor);
    let shown = |app: &App| {
        let laid = app.preview.markdown.layout.as_ref().unwrap();
        laid.text(0, laid.chars())
    };
    assert!(shown(&app).contains("新功能") && shown(&app).contains('乙'));
    let area = app.preview.markdown.area;
    right_click(&mut app, (area.x + area.w / 2.0, area.y + 60.0));
    assert!(
        !menu_commands(&app)
            .iter()
            .any(|(c, _)| *c == MenuCommand::RevealSource)
    );

    // 未能读取：标签中有两个按钮，重试沿用上一次的范围并回到正在读取。
    app.show_release_notes(None);
    settle(&mut app);
    assert_eq!(app.preview.markdown.area, Rect::default());
    let actions: Vec<_> = app.hits.placeholder.iter().map(|(_, a)| *a).collect();
    assert_eq!(
        actions,
        [PlaceholderAction::Retry, PlaceholderAction::OpenInBrowser]
    );
    app.run_placeholder_action(PlaceholderAction::Retry);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.reading, Some(winc_editor::Reading::Loading));
    assert_eq!(app.release_notes.scope, Scope::Since(Some("0.1.0".into())));

    // 读取期间关闭了标签：之后到达的内容不再打开它。
    let index = app.editor.active.unwrap();
    app.editor.documents.remove(index);
    app.editor.active = None;
    app.show_release_notes(Some("## 0.3.0 · 2026-03-01\n\n- 丙\n".into()));
    assert_eq!(app.editor.documents.len(), tabs);
}

/// 更新之后的第一次启动提示一次，提示记着上次运行的版本；第一次运行、版本没有变化或变旧时
/// 不提示。每种情况都记下当前版本。第一次运行时开始首次设置，没有完成时下次启动接着显示。
#[test]
fn the_first_launch_after_an_update_offers_the_release_notes() {
    let current = winc_version::VERSION;
    let launch = |recent: bool, version: Option<&str>| {
        let mut app = blank_app(false);
        app.state = winc_config::State::default();
        if recent {
            app.state.recent = vec![PathBuf::from("/项目")];
        }
        app.state.version = version.map(str::to_owned);
        app.note_version();
        assert_eq!(app.state.version.as_deref(), Some(current));
        // 更新提示在绘制之前、右上角没有别的提示时才显示。
        assert!(app.notice.text.is_none());
        app.show_update_notice();
        app
    };
    // 第一次运行：状态为空，不提示，开始页显示「首次设置」并记入状态。
    let first = launch(false, None);
    assert!(first.notice.action.is_none() && first.state.setup_pending);
    // 上次没有完成首次设置：接着显示。
    let mut again = blank_app(false);
    again.state.version = Some(current.to_owned());
    again.state.setup_pending = true;
    again.note_version();
    assert!(again.state.setup_pending && again.setup_shown());
    // 版本没有变化。
    let same = launch(true, Some(current));
    assert!(same.notice.text.is_none() && !same.state.setup_pending);
    // 比当前版本新（换回了旧版本）。
    assert!(launch(true, Some("9999.0.0")).notice.text.is_none());
    // 从旧版本更新而来：提示更新日志，不显示「首次设置」。
    let app = launch(true, Some("0.0.1"));
    assert!(!app.state.setup_pending);
    assert!(app.notice.text.as_deref().unwrap().contains(current));
    assert!(matches!(
        &app.notice.action,
        Some(NoticeAction::ReleaseNotes(Some(previous))) if previous == "0.0.1"
    ));
    // 上次运行的版本不记录版本号。
    assert!(matches!(
        launch(true, None).notice.action,
        Some(NoticeAction::ReleaseNotes(None))
    ));

    // 启动时右上角已有别的提示（例如正在连接远端主机）：更新提示等它收起后再显示。
    let mut app = blank_app(false);
    app.state.recent = vec![PathBuf::from("/项目")];
    app.state.version = Some("0.0.1".into());
    app.note_version();
    app.notify_pinned("正在连接", NoticeAction::CancelConnect);
    app.show_update_notice();
    assert_eq!(app.notice.text.as_deref(), Some("正在连接"));
    app.dismiss_notice();
    app.notice.leaving = None;
    app.show_update_notice();
    assert!(matches!(
        &app.notice.action,
        Some(NoticeAction::ReleaseNotes(Some(previous))) if previous == "0.0.1"
    ));
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_release_notes() {
    let notes = "## 0.2.0 · 2026-02-01\n\n### ✨ 新功能\n\n\
- 新增「更新日志」，可在命令面板中打开（[#157](https://github.com/DEEIX-AI/Winc/issues/157)）\n\
- 输入 `- [ ` 创建任务项\n\n### 🐛 修复\n\n- 修复预览含中文的 SVG 后内存占用偏高\n\n\
## 0.1.0 · 2026-01-01\n\n- 第一个版本\n";
    for dark in [false, true] {
        let mut app = blank_app(dark);
        let scope = crate::release_notes::Scope::All;
        app.open_release_notes(scope.clone());
        app.show_release_notes(crate::release_notes::select(notes, &scope, "0.2.0", ""));
        settle(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("release-notes-{theme}"));
    }
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_docx() {
    for dark in [false, true] {
        let dir = sample_folder("md-docx-shot");
        let path = dir.join("季度报告.docx");
        std::fs::write(&path, sample_docx()).unwrap();
        let mut app = app_with(&path, dark);
        settle(&mut app);
        let theme = if dark { "dark" } else { "light" };
        shoot(&mut app, &format!("md-docx-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
