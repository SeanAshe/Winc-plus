//! Git 模块整体的测试：开关、文件树装饰、上下文栏、视图切换、切换目录、损坏的仓库与耗时测量。

use super::*;

/// 目录本身不是仓库、下面有几个仓库时：都能找到，可以切换，打开文件时跟过去。
/// 有多个仓库时「新建工作树」写明建在当前仓库，切换仓库后跟着变；只有一个仓库时不写。
#[test]
fn the_new_worktree_button_names_the_repository_among_several() {
    let dir = search_folder("git-new-worktree-label");
    let (a, b) = (dir.join("a"), dir.join("b"));
    for repo in [&a, &b] {
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        if !make_repo(repo) {
            return;
        }
    }
    let mut app = app_with(&dir, false);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    assert_eq!(app.location_new_worktree_label(), "在「a」中新建工作树");
    app.git_switch_repository(1);
    assert_eq!(app.location_new_worktree_label(), "在「b」中新建工作树");
    let _ = std::fs::remove_dir_all(&dir);

    let Some((app, root)) = git_app("git-new-worktree-one", false) else {
        return;
    };
    assert_eq!(app.location_new_worktree_label(), "新建工作树");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_folder_of_repositories_can_switch_between_them() {
    let dir = search_folder("git-repositories");
    let (a, b, deep) = (dir.join("a"), dir.join("b"), dir.join("group/deep"));
    for repo in [&a, &b, &deep] {
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        if !make_repo(repo) {
            return;
        }
    }
    let mut app = app_with(&dir, false);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let dir = app.editor.root().unwrap().to_path_buf();
    let (a, b, deep) = (dir.join("a"), dir.join("b"), dir.join("group/deep"));

    // 查找层数为 1 时只看第一层子目录，按名字排序，默认用第一个。
    assert_eq!(app.git.repositories, [a.clone(), b.clone()]);
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert!(app.git.status.is_some());
    assert_eq!(app.git_repository_name(1), "b");

    // 从项目菜单切换。
    app.git_switch_repository(1);
    assert_eq!(app.git.root.as_ref(), Some(&b));
    assert!(app.git.status.is_some());

    // 打开另一个仓库的文件：跟过去。更深的仓库也加入列表。
    app.open_path(&a.join("src/main.rs"));
    app.git_follow_active();
    assert_eq!(app.git.root.as_ref(), Some(&a));
    app.open_path(&deep.join("src/main.rs"));
    app.git_follow_active();
    assert_eq!(app.git.root.as_ref(), Some(&deep));
    assert!(app.git.repositories.contains(&deep));
    assert_eq!(
        app.git_repository_name(2),
        "group/deep".replace('/', std::path::MAIN_SEPARATOR_STR)
    );

    // 切换仓库时沿用找到的 git，切回去时马上有状态。
    let tool = app.git.tool.get();
    assert!(tool.is_some());
    app.git_switch_repository(1);
    assert!(app.git.tool.get().is_some());
    assert!(app.git.status.is_some());
    assert_eq!(
        app.git.status.as_ref().unwrap().head,
        Some(Head::Branch("main".into()))
    );

    // 重新查找时保留已加入的仓库。
    app.git_discover();
    assert!(app.git.repositories.contains(&deep));

    // 正在看提交视图时不跟随，新仓库只加入列表。
    app.git_switch_repository(0);
    app.panels.sidebar.view = SidebarView::Commit;
    let shallow = dir.join("group/other");
    std::fs::create_dir_all(shallow.join("src")).unwrap();
    std::fs::write(shallow.join("src/main.rs"), "fn main() {}\n").unwrap();
    if !make_repo(&shallow) {
        return;
    }
    app.open_path(&shallow.join("src/main.rs"));
    app.git_follow_active();
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert!(app.git.repositories.contains(&shallow));
    app.panels.sidebar.view = SidebarView::Files;
    app.git_switch_repository(2);

    // 每个位置都能看到分支和改动数：另一个仓库改一个文件，后台读到以后显示。
    std::fs::write(b.join("src/main.rs"), "fn changed() {}\n").unwrap();
    app.git_discover();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline && app.place_state(&b).changes == 0 {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let state = app.place_state(&b);
    assert_eq!(state.head.as_deref(), Some("main"));
    assert!(state.changes > 0);
    // 当前仓库用最新的状态。
    let root = app.git.root.clone().unwrap();
    assert_eq!(app.place_state(&root).head.as_deref(), Some("main"));

    // 提交信息写了字时不跟随，文字不会丢。
    let root_before = app.git.root.clone();
    app.git.commit.message.insert("draft");
    app.open_path(&b.join("src/main.rs"));
    app.git_follow_active();
    assert_eq!(app.git.root.as_ref(), root_before.as_ref());
    assert_eq!(app.git.commit.message.text(), "draft");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 目录本身就是仓库时只用它，子目录没有仓库时不显示仓库一组。
#[test]
fn a_single_repository_lists_only_itself() {
    let dir = search_folder("git-one-repository");
    if !make_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    assert_eq!(app.git.repositories.len(), 1);
    assert_eq!(app.git.repositories.first(), app.git.root.as_ref());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn git_module_is_off_until_switched_on_and_clears_when_off() {
    let dir = search_folder("git-module");
    if !make_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    assert!(!app.git_enabled());
    assert!(app.git.root.is_none() && app.git.status.is_none());

    app.settings_toggle(Setting::GitModule);
    assert!(app.git_enabled());
    // 未安装开发者工具或找不到 git 时，只检查开关本身。
    if git_unusable(&app) {
        return;
    }
    let status = app.git.status.clone().expect("刷新是就地完成的");
    assert_eq!(status.head, Some(Head::Branch("main".into())));
    let main = app.editor.root().unwrap().join("src/main.rs");
    assert_eq!(app.git.change_of(&main), Some(Change::Modified));
    let new = app.editor.root().unwrap().join("src/new.rs");
    assert_eq!(app.git.change_of(&new), Some(Change::Untracked));
    assert_eq!(
        app.git.change_of(&app.editor.root().unwrap().join("src")),
        Some(Change::Modified)
    );

    // 绘制一帧：文件树和状态栏都能正常绘制，不会因为装饰出错。
    paint(&mut app);

    app.settings_toggle(Setting::GitModule);
    assert!(!app.git_enabled());
    assert!(app.git.root.is_none() && app.git.status.is_none());
    assert_eq!(app.git.change_of(&main), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ignored_files_are_known_to_the_tree() {
    let dir = search_folder("git-ignored");
    if !make_repo(&dir) {
        return;
    }
    std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
    std::fs::create_dir_all(dir.join("target/debug")).unwrap();
    std::fs::write(dir.join("target/debug/app"), "x").unwrap();
    std::fs::write(dir.join("src/run.log"), "x").unwrap();
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.editor.root().unwrap().to_path_buf();
    // 被忽略的目录本身及其中的项均视为忽略（`git` 只列出目录本身），被忽略的文件同样如此。
    assert!(app.git.is_ignored(&root.join("target")));
    assert!(app.git.is_ignored(&root.join("target/debug/app")));
    assert!(app.git.is_ignored(&root.join("src/run.log")));
    // 已跟踪的文件、未被忽略的未跟踪文件及其所在目录均不视为忽略。
    assert!(!app.git.is_ignored(&root.join("src/main.rs")));
    assert!(!app.git.is_ignored(&root.join("src/new.rs")));
    assert!(!app.git.is_ignored(&root.join("src")));
    assert!(!app.git.is_ignored(&root));
    paint(&mut app);

    // 关闭 Git 模块后不再提供忽略信息。
    app.settings_toggle(Setting::GitModule);
    assert!(!app.git.is_ignored(&root.join("target")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outside_changes_refresh_the_status_unless_git_ignores_them() {
    let dir = search_folder("git-ignored-tracked");
    if !make_repo(&dir) {
        return;
    }
    std::fs::write(dir.join(".gitignore"), "target/\n*.log\n").unwrap();
    std::fs::write(dir.join("keep.log"), "one\n").unwrap();
    git_in(&dir, &["add", "-f", ".gitignore", "keep.log"]);
    git_in(&dir, &["commit", "-q", "-m", "log"]);
    std::fs::create_dir_all(dir.join("target/debug")).unwrap();
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.editor.root().unwrap().to_path_buf();
    let changed = |path: PathBuf| winc_editor::Changes {
        paths: vec![path],
        rescan: false,
        git: false,
    };

    // 被忽略目录中的变化和仓库之外的变化不触发状态刷新。
    let generation = app.git.generation;
    std::fs::write(root.join("target/debug/app"), "x").unwrap();
    app.apply_changes(changed(root.join("target/debug/app")));
    app.apply_changes(changed(PathBuf::from("/elsewhere/file")));
    assert_eq!(app.git.generation, generation);

    // 已跟踪的文件即使匹配忽略规则，被其他程序修改后仍然触发状态刷新。
    let log = root.join("keep.log");
    assert_eq!(app.git.change_of(&log), None);
    std::fs::write(&log, "two\n").unwrap();
    app.apply_changes(changed(log.clone()));
    assert_eq!(app.git.change_of(&log), Some(Change::Modified));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：Git 模块启用时的文件树和状态栏，深浅两套配色。
#[test]
#[ignore]
fn shoot_git_decorations() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "git-shot-dark"
        } else {
            "git-shot-light"
        });
        if !make_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        shoot(&mut app, if dark { "git-dark" } else { "git-light" });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：每个主题各一张，左侧提交视图、右侧带语法高亮的 Rust 差异（三种改动齐全），
/// 光标与选区在右侧。
#[test]
#[ignore]
fn shoot_themes() {
    const BEFORE: &str = "use std::collections::HashMap;\n\n/// 按名字查用户的缓存。\n#[derive(Debug, Default)]\npub struct Cache {\n    users: HashMap<String, u32>,\n    hits: usize,\n}\n\nimpl Cache {\n    pub fn get(&mut self, name: &str) -> Option<u32> {\n        // 先查缓存\n        let id = self.users.get(name).copied();\n        if id.is_some() {\n            self.hits += 1;\n        }\n        id\n    }\n\n    pub fn clear(&mut self) {\n        self.users.clear();\n    }\n}\n";
    let after = BEFORE
        .replace(
            "hits: usize,\n",
            "hits: usize,\n    misses: usize,\n    limit: Option<usize>,\n",
        )
        .replace(
            "            self.hits += 1;\n",
            "            self.hits += 1;\n        } else {\n            self.misses += 1;\n",
        )
        .replace(
            "    pub fn clear(&mut self) {\n        self.users.clear();\n    }\n",
            "",
        );
    for dark in [false, true] {
        for theme in crate::theme::themes(dark) {
            let name = theme.id;
            let dir = search_folder(&format!("theme-{name}"));
            std::fs::write(dir.join("src/main.rs"), BEFORE).unwrap();
            if !make_repo(&dir) {
                return;
            }
            std::fs::write(dir.join("src/main.rs"), &after).unwrap();
            std::fs::write(dir.join("src/new.rs"), "pub fn new() {}\n").unwrap();
            let mut app = app_with(&dir, dark);
            if dark {
                app.config.dark_theme = Some(name.to_owned());
            } else {
                app.config.light_theme = Some(name.to_owned());
            }
            app.set_theme(if dark {
                winit::window::Theme::Dark
            } else {
                winit::window::Theme::Light
            });
            app.settings_toggle(Setting::GitModule);
            app.run_action(Action::ShowCommit);
            app.edit_field(|f| f.insert("缓存记下没命中的次数"));
            let main = app.editor.root().unwrap().join("src/main.rs");
            let index = app
                .git
                .status
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .position(|e| e.path == main);
            paint(&mut app);
            app.press_commit(CommitTarget::File(index.unwrap()));
            // 在右侧双击选中一个词，光标位于右侧。
            paint(&mut app);
            let view = app.editor.active().unwrap().diff.as_deref().unwrap();
            let parts = crate::app::diff_parts(app.layout.editor, view, app.metrics.cell);
            let lh = app.fonts.line_height;
            let at = (
                parts.panes[RIGHT].text.x
                    + crate::app::diff::DIFF_TEXT_PAD
                    + app.metrics.cell * 13.3,
                parts.body.y + lh * 12.5,
            );
            click(&mut app, at);
            click(&mut app, at);
            shoot(&mut app, &format!("theme-{name}"));
            // 文件视图加编辑区：文件树与代码并排。
            app.set_sidebar_view(crate::layout::SidebarView::Files);
            app.open_path(&dir.join("src/main.rs"));
            app.focus = Focus::Editor;
            shoot(&mut app, &format!("theme-{name}-editor"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// 截图：文件视图标题行的上下文栏（项目 ⌄、分支 ⌄），鼠标停在分支上，深浅两套配色。
#[test]
#[ignore]
fn shoot_context_bar() {
    for dark in [false, true] {
        let name = if dark {
            "context-dark"
        } else {
            "context-light"
        };
        let dir = search_folder(name);
        if !long_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        paint(&mut app);
        let project = app.hits.project;
        app.on_mouse_move(project.x + 4.0, project.y + 4.0);
        shoot(&mut app, name);
        // 悬停提示：「名字 … 快捷键」两栏，快捷键是右边的淡色字。
        let commit = app.sidebar_tabs()[2].0;
        app.on_mouse_move(commit.x + 4.0, commit.y + 4.0);
        app.overlays.tooltip.since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("{name}-tip"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 截图：窄侧栏里的空搜索页，分支名很长（从末尾截短，项目徽章一直在），深浅两套配色。
#[test]
#[ignore]
fn shoot_search_header() {
    for dark in [false, true] {
        let name = if dark {
            "search-head-dark"
        } else {
            "search-head-light"
        };
        let dir = search_folder(name);
        if !long_repo(&dir) {
            return;
        }
        git_in(&dir, &["checkout", "-q", "-b", "grok_build_1.0.40"]);
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.panels.sidebar.width = 270.0;
        app.run_action(Action::FindInFiles);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 尚未找到 git 时，由最先用到 git 的后台任务查找，结果留给之后的任务和界面线程，
/// 避免每个任务各查一次并在用完后丢弃。
#[test]
fn the_first_git_job_to_find_git_shares_it() {
    let dir = search_folder("git-shared-tool");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    // 状态刷新找到的 git 保留在缓存中；清除缓存后，打开日志的任务自行查找，结果同样保留。
    assert!(app.git.tool.get().is_some());
    app.git.tool.forget();
    assert!(app.git.tool.get().is_none());
    assert!(app.run_action(Action::GitShowLog));
    assert!(app.git.tool.get().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn broken_repositories_explain_why_instead_of_loading_forever() {
    let Some((mut app, root)) = git_app("git-broken", false) else {
        return;
    };
    std::fs::write(root.join(".git/HEAD"), "garbage\n").unwrap();
    app.git_refresh();
    assert!(app.git.status.is_none());
    assert!(app.git.problem.is_some());
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.open_settings();
    paint(&mut app);
    let _ = std::fs::remove_dir_all(&root);
}

/// 测量：在本仓库上打开 Git 模块后，各个会在界面线程上发生的事情花多长时间。
///
/// 运行方式：`cargo test -p winc-app measure_git_costs -- --ignored --nocapture`。
#[test]
#[ignore]
fn measure_git_costs() {
    use std::time::Instant;
    // 先克隆再测：本仓库可能正开在另一个 winc 窗口中，直接打开时会被转交给该窗口。
    let source =
        winc_config::files::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .unwrap();
    let repo = std::env::temp_dir().join(format!("winc-measure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let cloned = Command::new("git")
        .args(["clone", "-q", "--local"])
        .arg(&source)
        .arg(&repo)
        .status()
        .unwrap();
    assert!(cloned.success());
    let repo = winc_config::files::canonicalize(&repo).unwrap();
    // 有改动、有编译产物：改几个文件，在 target/ 下放几千个文件。
    std::fs::write(repo.join("crates/winc-app/src/lib.rs"), "// changed\n").unwrap();
    std::fs::create_dir_all(repo.join("target/debug/deps")).unwrap();
    for i in 0..4000 {
        std::fs::write(repo.join(format!("target/debug/deps/x{i}.o")), "x").unwrap();
    }
    let mut app = app_with(&repo, false);
    let file = repo.join("crates/winc-app/src/app.rs");
    app.open_path(&file);
    let time = |label: &str, n: u32, f: &mut dyn FnMut()| {
        let start = Instant::now();
        for _ in 0..n {
            f();
        }
        eprintln!("{label}: {:?} / 次", start.elapsed() / n);
    };
    paint(&mut app);
    time("画一帧（模块关）", 20, &mut || paint(&mut app));
    app.settings_toggle(Setting::GitModule);
    eprintln!(
        "root = {:?}, enabled = {}, editor root = {:?}",
        app.git.root,
        app.git_enabled(),
        app.editor.root()
    );
    eprintln!(
        "entries = {:?}, problem = {:?}",
        app.git.status.as_ref().map(|s| s.entries.len()),
        app.git.problem
    );
    time("画一帧（模块开）", 20, &mut || paint(&mut app));
    time("刷新状态（就地，含 git）", 3, &mut || {
        app.git_refresh()
    });
    time("打字后画一帧", 20, &mut || {
        app.run(winc_editor::Command::Insert("x".into()));
        paint(&mut app);
    });
    let paths: Vec<PathBuf> = (0..4000)
        .map(|i| repo.join(format!("target/debug/deps/x{i}.o")))
        .collect();
    let status = app.git.status.clone().unwrap();
    time("算文件树装饰", 20, &mut || {
        let _ = status.decorations(&repo);
    });
    app.run_action(Action::ShowCommit);
    time("画一帧（提交视图）", 20, &mut || paint(&mut app));
    let changes = winc_editor::Changes {
        paths: paths.clone(),
        rescan: false,
        git: false,
    };
    let mut changes = Some(changes);
    time("处理一批 4000 个 target 变化", 1, &mut || {
        app.apply_changes(changes.take().unwrap());
    });
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn the_project_leads_the_second_row_and_git_context_sits_below_it() {
    let Some((mut app, root)) = git_app("git-context-bar", false) else {
        return;
    };
    // 顶栏是侧栏开关和视图切换图标，第二行左边是项目名，三个视图里位置不变。
    app.panels.sidebar.width = 400.0;
    paint(&mut app);
    let project = app.hits.project;
    assert!(project.y >= app.layout.sidebar_header.y);
    let tabs = app.sidebar_tabs();
    assert!(tabs[0].0.x > app.layout.sidebar_button.right());
    assert!(tabs[0].0.bottom() <= app.layout.sidebar_header.y);
    app.run_action(Action::FindInFiles);
    paint(&mut app);
    assert_eq!(app.hits.project, project);

    app.run_action(Action::ShowCommit);
    app.panels.sidebar.width = 400.0;
    paint(&mut app);
    assert_eq!(app.hits.project, project);
    app.on_mouse_move(project.x + 4.0, project.y + 4.0);
    assert_eq!(app.pointer.hover, Hover::ProjectName);
    let parts = app.commit_parts();
    assert_eq!(
        parts.tools.map(|(_, t)| t),
        [CommitTarget::Log, CommitTarget::Refresh, CommitTarget::More]
    );
    assert!(parts.tools[0].0.x >= project.right());

    // 一个仓库、一个工作树：只有分支行，它在改动列表上方。
    assert!(parts.location.is_none());
    assert!(parts.branch.y >= parts.header.bottom());
    assert!(parts.branch.bottom() <= parts.list.y);
    let hit = |app: &App, target| {
        app.hits
            .commit
            .iter()
            .find(|(_, t)| *t == target)
            .map(|h| h.0)
    };
    let branch = hit(&app, CommitTarget::Branch).unwrap();
    app.on_mouse_move(branch.x + 4.0, branch.y + 4.0);
    assert_eq!(app.pointer.hover, Hover::Commit(CommitTarget::Branch));

    // 分支名很长时在行内截短，右端的按钮还在，不碰到它们。
    git_in(
        &root,
        &[
            "switch",
            "-q",
            "-c",
            "feature/a-very-long-branch-name-for-the-sidebar-that-does-not-fit",
        ],
    );
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    app.git_refresh();
    app.panels.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    let branch = hit(&app, CommitTarget::Branch).unwrap();
    let sync = hit(&app, CommitTarget::Sync).unwrap();
    assert!(branch.right() <= sync.x);
    assert!(sync.right() <= app.layout.sidebar.right());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn switching_views_slides_the_raised_segment() {
    use crate::app::animation::{Clock, VIEW_SLIDE};
    use std::time::Duration;

    let Some((mut app, root)) = git_app("view-slide", false) else {
        return;
    };
    // 没有窗口（测试中）时不做动画，直接到终点。
    app.set_sidebar_view(SidebarView::Search);
    assert!(app.panels.sidebar.view_slide.is_none());
    // 半路：动画进行中、下一帧继续绘制；到时间以后清掉。
    app.panels.sidebar.view_slide = Some((
        SidebarView::Files,
        Clock::frozen(VIEW_SLIDE / 2, VIEW_SLIDE),
    ));
    assert!(app.animating());
    app.settle_animations();
    assert!(app.panels.sidebar.view_slide.is_some());
    app.panels.sidebar.view_slide = Some((
        SidebarView::Files,
        Clock::frozen(VIEW_SLIDE + Duration::from_millis(1), VIEW_SLIDE),
    ));
    app.settle_animations();
    assert!(app.panels.sidebar.view_slide.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：从「文件」切到「提交」滑到一半（`view-slide-light`、`view-slide-dark`）。
#[test]
#[ignore]
fn shoot_view_slide() {
    use crate::app::animation::{Clock, VIEW_SLIDE};

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("view-slide-{theme}"), false, dark) else {
            return;
        };
        app.set_sidebar_view(SidebarView::Commit);
        app.panels.sidebar.view_slide = Some((
            SidebarView::Files,
            Clock::frozen(VIEW_SLIDE / 3, VIEW_SLIDE),
        ));
        shoot(&mut app, &format!("view-slide-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 切换目录：旧仓库的分支操作与写了一半的提交信息不带入新目录；旧仓库晚到的结果只在状态栏
/// 提示一句，不在当前目录打开差异，也不改动当前状态。
#[test]
fn switching_folders_leaves_the_old_repos_work_behind() {
    let Some((mut app, root)) = git_app("git-switch-away", false) else {
        return;
    };
    app.git.commit.message.set_text("写了一半");
    app.git.commit.amend = true;
    app.git.branch.running = Some("拉取".into());

    let other = search_folder("git-switch-target");
    app.open_path(&other);
    assert_ne!(app.git.root.as_deref(), Some(root.as_path()));
    assert!(app.git.branch.running.is_none());
    assert_eq!(app.git.commit.message.text(), "");
    assert!(!app.git.commit.amend);

    // 旧仓库的拉取完成：在状态栏说明在哪个目录完成；当前目录正在进行的操作不受影响。
    app.git.branch.running = Some("推送".into());
    app.pulled_in(&root);
    assert_eq!(app.git.branch.running.as_deref(), Some("推送"));
    let name = winc_editor::display_name(&root).into_owned();
    assert_eq!(
        app.status.message.as_deref(),
        Some(format!("已在「{name}」中拉取").as_str())
    );
    // 旧仓库的差异晚到了：不开标签，也不提示。
    app.status.message = None;
    let tabs = app.editor.documents.len();
    app.diff_failed_in(&root, &root.join("src/main.rs"));
    assert_eq!(app.editor.documents.len(), tabs);
    assert!(app.status.message.is_none());

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&other);
}

/// 把「提交」一段、终端开关、分支名的出现、消失动画停在 `at`，方向是 `on`。
///
/// 测试中没有窗口，`track_modules` 不做动画，因此直接给时钟。
fn freeze_modules(app: &mut App, on: bool, at: std::time::Duration) {
    use crate::app::animation::{Clock, EXIT_FADE, MODULE_FADE};

    // 消失时宽度在内容淡出结束后才开始收起，`at` 从收起开始计时。
    let at = if on { at } else { at + EXIT_FADE };
    let fx = &mut app.motion.module_fx;
    for p in [&mut fx.commit_tab, &mut fx.terminal, &mut fx.branch] {
        p.track(!on, None);
        p.track(on, Some(Clock::frozen(at, MODULE_FADE)));
    }
}

/// 关闭 Git、终端模块时「提交」一段缩回去、终端开关和分支名淡出。
///
/// 动画途中仍然绘制（分支名用最后一帧的内容），但不再可点；动画结束后消失。
#[test]
fn switching_modules_off_fades_their_chrome_out() {
    use crate::app::animation::MODULE_FADE;

    let Some((mut app, root)) = git_app("module-fade", false) else {
        return;
    };
    app.settings_toggle(Setting::TerminalModule);
    paint(&mut app);
    assert!(app.motion.module_fx.branch_label.is_some());
    let full = app.sidebar_tabs()[2].0;
    let hit = |app: &App, item| app.hits.status.iter().any(|h| h.1 == item);
    assert!(hit(&app, StatusItem::Branch) && hit(&app, StatusItem::Terminal));

    app.settings_toggle(Setting::GitModule);
    app.settings_toggle(Setting::TerminalModule);
    freeze_modules(&mut app, false, MODULE_FADE / 3);
    assert!(app.animating());
    paint(&mut app);
    let tabs = app.sidebar_tabs();
    assert_eq!(tabs.len(), 3);
    assert!(tabs[2].0.w > 0.0 && tabs[2].0.w < full.w);
    assert_eq!(tabs[2].0.right(), full.right());
    assert!(app.motion.module_fx.branch_label.is_some());
    assert!(!hit(&app, StatusItem::Branch) && !hit(&app, StatusItem::Terminal));

    freeze_modules(&mut app, false, MODULE_FADE);
    app.track_modules();
    assert!(!app.motion.module_fx.moving());
    assert!(app.motion.module_fx.branch_label.is_none());
    assert_eq!(app.sidebar_tabs().len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：打开 Git、终端模块时「提交」一段展开、终端开关和分支名淡入到四分之一
/// （`module-fade-in-*`），以及关闭时淡出到四分之一（`module-fade-out-*`）。
#[test]
#[ignore]
fn shoot_module_fade() {
    use crate::app::animation::MODULE_FADE;

    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("module-fade-{theme}"), false, dark) else {
            return;
        };
        app.settings_toggle(Setting::TerminalModule);
        freeze_modules(&mut app, true, MODULE_FADE / 4);
        shoot(&mut app, &format!("module-fade-in-{theme}"));
        app.settings_toggle(Setting::GitModule);
        app.settings_toggle(Setting::TerminalModule);
        freeze_modules(&mut app, false, MODULE_FADE / 4);
        shoot(&mut app, &format!("module-fade-out-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 建一个放着多个仓库的目录：`a`、`group/b`、`group/deep/c`，打开它并等后台查找完成。
fn nested_repositories(name: &str) -> Option<(App, PathBuf)> {
    let dir = search_folder(name);
    for repo in ["a", "group/b", "group/deep/c"] {
        let repo = dir.join(repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();
        if !make_repo(&repo) {
            return None;
        }
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.git.repositories.len() < 3 && std::time::Instant::now() < deadline {
        app.poll_git();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Some((app, dir))
}

#[test]
fn nested_repositories_are_found_in_the_background_without_moving_earlier_ones() {
    let Some((mut app, dir)) = nested_repositories("git-nested") else {
        return;
    };
    let names: Vec<String> = (0..app.git.repositories.len())
        .map(|i| app.git_repository_name(i))
        .collect();
    assert_eq!(names, ["a", "group/b", "group/deep/c"]);
    assert_eq!(app.git.root.as_ref(), app.git.repositories.first());

    // 配置的层数生效：只找一层时找不到更深的。
    app.config.repository_scan_depth = Some(1);
    app.git_folder_changed();
    std::thread::sleep(std::time::Duration::from_millis(200));
    app.poll_git();
    assert_eq!(app.git.repositories.len(), 1);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_location_picker_searches_and_switches() {
    use crate::app::git::location_picker::LocationRow;

    let Some((mut app, dir)) = nested_repositories("git-location-picker") else {
        return;
    };
    // 项目名只管项目：提交视图里也打开项目菜单。
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.open_project_menu();
    assert!(!app.picker_lists_locations());
    assert!(app.overlays.context_menu.is_some());
    app.overlays.context_menu = None;

    // 位置行展开位置选择器，选中的是当前位置，面板就在这一行下方。
    let row = app.commit_parts().location.unwrap();
    app.open_location_picker();
    assert!(app.picker_lists_locations());
    let place = |row: &LocationRow| match row {
        LocationRow::Place(place) => place.clone(),
        other => panic!("{other:?}"),
    };
    let rows = |app: &App| app.overlays.picker.location_rows.clone().unwrap();
    assert_eq!(rows(&app).len(), 3);
    assert!(place(&rows(&app)[app.overlays.picker.selected]).current);
    paint(&mut app);
    assert_eq!(app.layout.picker.x, row.x);
    assert!(app.layout.picker.y >= row.bottom());

    // 搜索只留下匹配的位置；回车切换并关闭。
    app.overlays.picker.input.set_text("deep");
    app.picker_refresh();
    assert_eq!(rows(&app).len(), 1);
    app.picker_accept();
    assert!(!app.overlays.picker.open);
    assert_eq!(app.git.root.as_ref(), app.git.repositories.get(2));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_repository_with_worktrees_is_a_group_and_expanding_it_does_not_switch() {
    use crate::app::git::location_picker::LocationRow;

    let Some((mut app, dir)) = nested_repositories("git-location-group") else {
        return;
    };
    let a = app.git.repositories[0].clone();
    let linked = dir.with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    git_in(
        &a,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.open_location_picker();
    let rows = |app: &App| app.overlays.picker.location_rows.clone().unwrap();
    // 当前仓库默认展开，工作树排在它下面。
    let labels = |app: &App| -> Vec<String> {
        rows(app)
            .iter()
            .map(|row| match row {
                LocationRow::Repository { label, .. } => label.clone(),
                LocationRow::Place(place) => place.label.clone(),
            })
            .collect()
    };
    assert_eq!(
        labels(&app)[..3],
        [
            "a".to_owned(),
            "主工作树".to_owned(),
            "git-location-group.linked".to_owned()
        ]
    );

    // 点仓库只是展开收起，当前仓库不变。
    let root = app.git.root.clone();
    app.overlays.picker.selected = 0;
    app.picker_accept();
    assert!(app.picker_lists_locations());
    assert_eq!(app.git.root, root);
    assert!(!labels(&app).contains(&"主工作树".to_owned()));
    // 再打开时当前仓库重新展开。
    app.close_picker();
    app.open_location_picker();
    assert!(labels(&app).contains(&"主工作树".to_owned()));

    std::fs::remove_dir_all(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&linked);
}

#[test]
fn drafts_stay_with_their_location() {
    let Some((mut app, dir)) = nested_repositories("git-location-drafts") else {
        return;
    };
    app.run_action(Action::ShowCommit);
    app.git.commit.message.set_text("a 的草稿");
    app.git_switch_repository(1);
    assert_eq!(app.git.commit.message.text(), "");
    app.git.commit.message.set_text("b 的草稿");
    app.git_switch_repository(0);
    assert_eq!(app.git.commit.message.text(), "a 的草稿");
    app.git_switch_repository(1);
    assert_eq!(app.git.commit.message.text(), "b 的草稿");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截图：位置选择器。
#[test]
#[ignore]
fn shoot_location_picker() {
    let Some((mut app, dir)) = nested_repositories("git-location-shot") else {
        return;
    };
    std::fs::write(dir.join("group/b/src/main.rs"), "fn changed() {}\n").unwrap();
    let hotfix = dir.join("hotfix");
    let hotfix = hotfix.to_str().unwrap();
    git_in(
        &dir.join("a"),
        &["worktree", "add", "-q", "-b", "hotfix", hotfix],
    );
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    app.git_discover();
    let b = dir.join("group/b");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline && app.place_state(&b).changes == 0 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    shoot(&mut app, "git-context");
    app.open_location_picker();
    shoot(&mut app, "location-picker");
    std::fs::remove_dir_all(&dir).unwrap();
}
