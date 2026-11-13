//! 提交视图中文件行的测试：名称的省略、目录的显示和按钮的收起。

use super::*;

#[test]
fn commit_list_groups_changes_by_directory() {
    let Some((mut app, root)) = git_app("git-commit-tree", false) else {
        return;
    };
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    // 几层目录：`pkg` 与 `model` 之间没有其他文件，`core/model` 并成一行；`pkg` 下另有 `z.rs`。
    for path in [
        "pkg/core/model/a.rs",
        "pkg/core/model/b.rs",
        "pkg/z.rs",
        "top.rs",
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x\n").unwrap();
    }
    git_in(&root, &["add", "-A"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let index = |name: &str| {
        status
            .entries
            .iter()
            .position(|e| e.path.ends_with(name))
            .unwrap()
    };
    // 已暂存的新文件和改过的 `src/main.rs` 同在「更改」一组里。
    let changes: Vec<CommitRow> = app
        .commit_rows()
        .iter()
        .skip_while(|r| **r != CommitRow::Group(CommitGroup::Changes))
        .cloned()
        .collect();
    let dir = |dir: &str, label: &str, depth| CommitRow::Dir {
        group: CommitGroup::Changes,
        dir: PathBuf::from(dir),
        label: label.into(),
        depth,
    };
    let file = |name: &str, depth| CommitRow::File {
        index: index(name),
        depth,
    };
    assert_eq!(
        changes,
        [
            CommitRow::Group(CommitGroup::Changes),
            dir("pkg", "pkg", 0),
            dir("pkg/core/model", "core/model", 1),
            file("model/a.rs", 2),
            file("model/b.rs", 2),
            file("pkg/z.rs", 1),
            dir("src", "src", 0),
            file("src/main.rs", 1),
            file("src/new.rs", 1),
            file("top.rs", 0),
        ]
    );

    // 新文件默认勾着。目录的勾选框取消目录下的所有文件，再点一次全部勾上。
    let row = |app: &App, r: &CommitRow| app.commit_rows().iter().position(|x| x == r).unwrap();
    let pkg = row(&app, &dir("pkg", "pkg", 0));
    let checked = |app: &App, name: &str| app.git.commit.checked(&status.entries[index(name)]);
    assert!(checked(&app, "model/a.rs") && checked(&app, "top.rs"));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(!checked(&app, "model/a.rs") && !checked(&app, "pkg/z.rs"));
    assert!(checked(&app, "top.rs"));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(checked(&app, "model/a.rs") && checked(&app, "pkg/z.rs"));
    let rows = app.commit_rows();
    let members = rows.dir(CommitGroup::Changes, Path::new("pkg"));
    // `pkg` 目录这一组：合并后的 `core/model` 与 `pkg/z.rs` 里的文件都算在内。
    assert_eq!(members.len(), 3);
    assert_eq!(app.git.commit.members_state(&status, members), Some(true));
    // 组与目录的勾选框每帧都要查其下所有文件：结果按勾选的版本缓存，勾选不变时复用上次的结果。
    let cached = |app: &App| {
        let compute = || app.git.commit.members_state(&status, members);
        rows.members_state(members, app.git.commit.checks(), compute)
    };
    assert_eq!(cached(&app), Some(true));
    app.press_commit(CommitTarget::DirCheck(pkg));
    assert!(!checked(&app, "model/a.rs"));
    assert_eq!(cached(&app), None);
    // 勾选不重排行：已排好的行继续复用，收起或展开目录后才重新排。
    assert!(std::rc::Rc::ptr_eq(&rows, &app.commit_rows()));

    // 点击目录行收起：其下的行不再出现；「全部展开」恢复。
    app.press_commit(CommitTarget::Dir(pkg));
    assert!(!app.commit_rows().contains(&file("pkg/z.rs", 1)));
    app.run_commit_more_command(crate::app::input::context_menu::MenuCommand::CommitExpandAll);
    assert!(app.commit_rows().contains(&file("pkg/z.rs", 1)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scrolled_changes_keep_their_group_and_folders_pinned_on_top() {
    let Some(dir) = deep_changes_repo("git-sticky") else {
        return;
    };
    // `b` 目录之后再放一批文件：滚动范围足够大，下面滚到 `b` 目录中间的位置才能到达。
    let more = dir.join("assets/pets/c");
    std::fs::create_dir_all(&more).unwrap();
    for i in 0..40 {
        std::fs::write(more.join(format!("{i}.png")), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    paint(&mut app);
    let rows = app.commit_rows();
    // 没滚动时什么也不固定。
    assert!(app.commit_sticky(&rows).is_empty());

    // 滚到 `b` 目录中间：上面固定着组、`assets/pets`、`b` 三层。
    let b = rows
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "b"))
        .unwrap();
    app.git.commit.scroll = (b + 4) as f32 * COMMIT_ROW;
    assert!(app.git.commit.scroll <= app.commit_max_scroll());
    paint(&mut app);
    let sticky = app.commit_sticky(&rows);
    let pinned: Vec<usize> = sticky.iter().map(|s| s.row).collect();
    assert_eq!(pinned.len(), 3);
    assert!(matches!(
        rows[pinned[0]],
        CommitRow::Group(CommitGroup::Changes)
    ));
    assert_eq!(pinned[2], b);
    let list = app.commit_parts().list;
    assert_eq!(sticky[2].rect.y, list.y + 2.0 * COMMIT_ROW);

    // 点击固定的 `b` 行：列表滚到 `b` 所在行并保持不变，`b` 收起。
    let at = sticky[2].rect;
    app.on_mouse_move(at.x + 120.0, at.y + at.h / 2.0);
    assert_eq!(app.pointer.hover, Hover::Commit(CommitTarget::Sticky(b, 2)));
    click(&mut app, (at.x + 120.0, at.y + at.h / 2.0));
    // 收起后列表变短，内容放得下时滚回顶部。
    let settled = ((b - 2) as f32 * COMMIT_ROW).min(app.commit_max_scroll());
    assert_eq!(app.git.commit.scroll, settled);
    let CommitRow::Dir {
        group, dir: folder, ..
    } = &rows[b]
    else {
        panic!("第 {b} 行不是目录");
    };
    assert!(folder.ends_with("assets/pets/b"));
    assert!(app.git.commit.dir_collapsed(*group, folder));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn option_clicking_a_changed_folder_collapses_and_expands_it_all_the_way() {
    let Some(dir) = deep_changes_repo("git-recursive") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.git.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    app.press_commit(CommitTarget::Group(CommitGroup::Changes));
    paint(&mut app);
    let dirs = |app: &App| -> Vec<(PathBuf, bool)> {
        app.commit_rows()
            .iter()
            .filter_map(|row| match row {
                CommitRow::Dir { group, dir, .. } => {
                    Some((dir.clone(), app.git.commit.dir_collapsed(*group, dir)))
                }
                _ => None,
            })
            .collect()
    };
    let first = app
        .commit_rows()
        .iter()
        .position(|row| matches!(row, CommitRow::Dir { .. }))
        .unwrap();
    let inner = dirs(&app).len();
    assert!(inner > 1);
    let alt = winit::keyboard::ModifiersState::ALT;

    // 递归收起：只剩最上面的目录行，且收起。
    app.pointer.modifiers = alt;
    app.press_commit(CommitTarget::Dir(first));
    assert_eq!(dirs(&app).len(), 1);
    assert!(dirs(&app)[0].1);

    // 普通单击只展开一层：下一级目录还是收起的。
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    app.press_commit(CommitTarget::Dir(first));
    assert!(dirs(&app).iter().skip(1).all(|(_, collapsed)| *collapsed));

    // 先收起再递归展开：各级全部展开，行数回到最初。
    app.press_commit(CommitTarget::Dir(first));
    app.pointer.modifiers = alt;
    app.press_commit(CommitTarget::Dir(first));
    app.pointer.modifiers = winit::keyboard::ModifiersState::empty();
    assert_eq!(dirs(&app).len(), inner);
    assert!(dirs(&app).iter().all(|(_, collapsed)| !collapsed));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn showing_as_tree_switches_the_changes_between_a_tree_and_a_flat_list() {
    use crate::app::input::context_menu::{MenuCommand, MenuTarget};

    let Some(dir) = deep_changes_repo("git-flat") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 默认平铺：没有目录行，每个文件一行，按路径排；「刷新」在「Git 日志」后面。
    assert!(app.git.commit.flat());
    let flat = app.commit_rows();
    assert!(!flat.iter().any(|r| matches!(r, CommitRow::Dir { .. })));
    let files = flat
        .iter()
        .filter(|r| matches!(r, CommitRow::File { depth: 0, .. }))
        .count();
    assert_eq!(files, 27);
    let tools = app.commit_parts().tools;
    assert_eq!(tools[0].1, CommitTarget::Log);
    assert_eq!(tools[1].1, CommitTarget::Refresh);
    assert!(tools[1].0.x > tools[0].0.x && tools[1].0.x < tools[2].0.x);
    // 平铺时文件不缩进：勾选框和组标题的勾选框在同一列。
    let check_x = |app: &App, want: fn(&CommitTarget) -> bool| {
        let (rect, _) = app.hits.commit.iter().find(|(_, t)| want(t)).unwrap();
        rect.x + rect.w / 2.0
    };
    let group = check_x(&app, |t| matches!(t, CommitTarget::GroupCheck(_)));
    assert_eq!(
        check_x(&app, |t| matches!(t, CommitTarget::Check(_))),
        group
    );

    // 「⋯」里的「以树形显示」：按目录排成树，排成树时打勾；记进状态，以后打开的窗口、下次启动
    // 沿用。
    let toggle = |app: &mut App| {
        app.open_context_menu(MenuTarget::CommitMore, (0.0, 0.0));
        app.run_menu_command(MenuCommand::CommitTree);
    };
    assert!(!app.menu_checked(MenuCommand::CommitTree));
    toggle(&mut app);
    assert!(!app.git.commit.flat());
    assert!(app.menu_checked(MenuCommand::CommitTree));
    assert!(
        app.commit_rows()
            .iter()
            .any(|r| matches!(r, CommitRow::Dir { .. }))
    );
    assert!(app.state.commit_tree);
    // 再点回到平铺。
    toggle(&mut app);
    assert!(app.git.commit.flat());
    assert!(!app.state.commit_tree);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folding_a_changes_folder_slides_its_rows_like_the_file_tree() {
    use crate::app::Clock;
    use crate::app::git::commit::CommitMotion;

    let Some(dir) = deep_changes_repo("git-commit-motion") else {
        return;
    };
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let before = app.commit_rows();
    let a = before
        .iter()
        .position(|r| matches!(r, CommitRow::Dir { label, .. } if label == "a"))
        .unwrap();
    app.press_commit(CommitTarget::Dir(a));
    let after = app.commit_rows();
    assert_eq!(before.len() - after.len(), 12);

    // 收起到一半：`a` 下面的 12 行收回一半，`b` 上移 6 行。动画期间只记录 `a` 及其上方各行
    // 的命中区域，正在移动的行不记录。
    let half = Clock::frozen(
        std::time::Duration::from_millis(1000),
        std::time::Duration::from_millis(2000),
    );
    app.git.commit.motion = Some(CommitMotion {
        rows: before.clone(),
        parent: a,
        count: 12,
        expanding: false,
        clock: half,
    });
    let motion = app.git.commit.motion.as_ref().unwrap();
    let hidden = motion.hidden();
    assert!(hidden > 0.0 && hidden < 12.0 * COMMIT_ROW);
    paint(&mut app);
    assert!(
        app.hits
            .commit
            .iter()
            .any(|(_, t)| *t == CommitTarget::Dir(a))
    );
    assert!(
        !app.hits
            .commit
            .iter()
            .any(|(_, t)| matches!(t, CommitTarget::Dir(r) if *r > a))
    );
    shoot(&mut app, "commit-folding");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hovering_a_long_change_name_shows_it_in_full_with_its_buttons_after_it() {
    let Some((dir, long)) = long_name_repo("git-commit-overflow") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let rows = app.commit_rows();
    let status = app.git.status.clone().unwrap();
    let (i, index) = rows
        .iter()
        .enumerate()
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;

    // 未悬停时名称截短，没有伸出的部分。
    assert!(app.git.commit.overflow.is_none());

    // 停在这一行上：写全名，伸出侧栏；「打开文件」「回滚」跟在全名之后，也在伸出的那一段里。
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let overflow = app.git.commit.overflow.expect("long name spills");
    assert_eq!(overflow.row, i);
    let spill = overflow.spill.expect("past the sidebar");
    assert_eq!(spill.x, list.right());
    let (open, revert) = overflow.actions.expect("buttons after the name");
    assert!(open.x > list.right() && revert.right() <= spill.right());
    let open_hit = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::OpenFile(index))
        .map(|(r, _)| *r);
    assert_eq!(open_hit, Some(open));

    // 移到伸出去的按钮上：仍写全名，悬停在该按钮上。
    let (ox, oy) = (open.x + open.w / 2.0, open.y + open.h / 2.0);
    app.on_mouse_move(ox, oy);
    paint(&mut app);
    assert_eq!(
        app.pointer.hover,
        Hover::Commit(CommitTarget::OpenFile(index))
    );
    assert!(app.git.commit.overflow.is_some_and(|o| o.row == i));

    // 展开收起的动画途中：该行就是被收起的行时位置不变，仍写全名；位于被收起的行下方、正在
    // 移动时恢复截短显示。
    let motion = |parent: usize| crate::app::git::commit::CommitMotion {
        rows: rows.clone(),
        parent,
        count: 1,
        expanding: false,
        clock: crate::app::animation::Clock::frozen(
            std::time::Duration::from_millis(1000),
            std::time::Duration::from_millis(2000),
        ),
    };
    app.on_mouse_move(list.x + 120.0, y);
    app.git.commit.motion = Some(motion(i));
    paint(&mut app);
    assert!(app.git.commit.overflow.is_some_and(|o| o.row == i));
    if i > 0 {
        app.git.commit.motion = Some(motion(i - 1));
        paint(&mut app);
        assert!(app.git.commit.overflow.is_none());
    }
    app.git.commit.motion = None;

    // 移开：恢复截短显示。
    app.on_mouse_move(list.x + 120.0, list.bottom() - 4.0);
    paint(&mut app);
    assert!(app.git.commit.overflow.is_none_or(|o| o.row != i));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 改动行的长名字和文件树一样随鼠标滑出、收回：命中范围立即按全名，画出的宽度从截短的宽度
/// 缓动过去，截短的名字淡出、全名淡入；移开以后宽度缓动回去，这一行上不再唤出滚动条。
#[test]
fn a_long_change_name_slides_out_and_back_like_the_file_tree() {
    let Some((dir, long)) = long_name_repo("git-commit-name-slide") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git.commit.names.animate_without_window = true;
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let entry = (status.entries.iter())
        .find(|e| e.path.ends_with(&long))
        .unwrap();
    let key = CommitName::File(entry.path.clone());
    let rows = app.commit_rows();
    let i = (rows.iter())
        .position(|r| matches!(r, CommitRow::File { index, .. } if status.entries[*index].path == entry.path))
        .unwrap();
    let rest = app.git.commit.names.peek(&key).unwrap();
    assert!(!app.git.commit.names.moving());

    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let overflow = app.git.commit.overflow.expect("long name spills");
    let (_, revert) = overflow.actions.unwrap();
    let view = app.git.commit.names.peek(&key).unwrap();
    assert!(app.git.commit.names.moving());
    assert!(view.width < revert.x - list.x, "{}", view.width);
    assert!(
        (view.width - rest.width).abs() < 40.0,
        "{} {}",
        view.width,
        rest.width
    );
    assert!(!view.old.is_empty());
    assert!(app.over_commit_overflow_row(y));

    app.git.commit.names.finish();
    paint(&mut app);
    let out = app.git.commit.names.peek(&key).unwrap();
    assert!(out.width > rest.width + 40.0);

    // 移开：命中范围立即消失，宽度从全名开始缓动回去。
    app.on_mouse_move(list.x + 120.0, list.bottom() - 4.0);
    paint(&mut app);
    assert!(app.git.commit.overflow.is_none_or(|o| o.row != i));
    let back = app.git.commit.names.peek(&key).unwrap();
    assert!(app.git.commit.names.moving());
    assert!(
        back.width > rest.width + 40.0,
        "{} {}",
        back.width,
        rest.width
    );
    app.git.commit.names.finish();
    paint(&mut app);
    assert_eq!(app.git.commit.names.peek(&key).unwrap().width, rest.width);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 名字在侧栏里放得下、只需为「打开文件」「回滚」两个按钮让出位置时：名字不变，按钮从原位滑到
/// 名字后面，伸出侧栏的部分随按钮变宽；名字放不下、在截断处淡出的行，悬停时同样从截断处滑出
/// 全名。
#[test]
fn names_that_only_make_room_for_the_buttons_slide_the_buttons_out() {
    let Some(dir) = deep_changes_repo("git-commit-button-slide") else {
        return;
    };
    // 名字长短不一的文件放在根目录：没有可以折叠的目录，截短时只能在末尾淡出。
    let names: Vec<String> = (20..60).map(|n| format!("{}.rs", "x".repeat(n))).collect();
    for name in &names {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    git_in(&dir, &["add", "-A"]);
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.git.commit.names.animate_without_window = true;
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let list = app.commit_parts().list;
    let away = (app.layout.editor.x + 200.0, app.layout.editor.y + 200.0);

    // 依次悬停各行，记下按钮开始时的位置、终点，以及名字露出的宽度是否在变。
    let mut slight = None;
    let mut clipped = None;
    for name in &names {
        let rows = app.commit_rows();
        let Some(i) = rows.iter().position(|r| {
            matches!(r, CommitRow::File { index, .. } if status.entries[*index].path.ends_with(name))
        }) else {
            continue;
        };
        app.on_mouse_move(away.0, away.1);
        app.git.commit.scroll = ((i as f32 - 3.0) * COMMIT_ROW).clamp(0.0, app.commit_max_scroll());
        paint(&mut app);
        app.git.commit.names.finish();
        paint(&mut app);
        let path = (status.entries.iter())
            .find(|e| e.path.ends_with(name))
            .unwrap();
        let key = CommitName::File(path.path.clone());
        let rest = app.git.commit.names.peek(&key).unwrap().width;
        let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
        app.on_mouse_move(list.x + 120.0, y);
        paint(&mut app);
        let Some(overflow) = app.git.commit.overflow else {
            continue;
        };
        let (end, _) = overflow.actions.unwrap();
        let start = (app.hits.commit.iter())
            .find(|(_, t)| matches!(t, CommitTarget::OpenFile(_)))
            .unwrap()
            .0;
        let moving = app.git.commit.names.moving();
        app.git.commit.names.finish();
        let full = app.git.commit.names.peek(&key).unwrap().width;
        if rest == full && slight.is_none() {
            slight = Some((start.x, end.x, moving));
        }
        if rest < full && clipped.is_none() {
            clipped = Some(moving);
        }
    }

    // 只需让出位置：名字不动，按钮从原位出发，距终点仍有一段距离。
    let (start, end, moving) = slight.expect("a name that fits before the status letter");
    assert!(!moving);
    assert!(start < end - 1.0, "{start} {end}");
    // 截断处淡出的名字：悬停时宽度从截断处开始变化。
    assert_eq!(clipped, Some(true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_overflow() {
    for dark in [false, true] {
        let name = if dark {
            "overflow-dark"
        } else {
            "overflow-light"
        };
        let Some((dir, long)) = long_name_repo(name) else {
            return;
        };
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        let rows = app.commit_rows();
        let status = app.git.status.clone().unwrap();
        let i = rows
            .iter()
            .position(|r| {
                matches!(r, CommitRow::File { index, .. }
                    if status.entries[*index].path.ends_with(&long))
            })
            .unwrap();
        let list = app.commit_parts().list;
        // 滚到该行位于列表上部，悬停后截图。
        let scroll = (i as f32 - 3.0) * COMMIT_ROW;
        app.git.commit.scroll = scroll.clamp(0.0, app.commit_max_scroll());
        paint(&mut app);
        let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
        app.on_mouse_move(list.x + 120.0, y);
        shoot(&mut app, name);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn switching_folders_keeps_the_commit_view_while_there_is_a_repository() {
    let first = search_folder("switch-git-a");
    let second = search_folder("switch-git-b");
    let plain = search_folder("switch-git-plain");
    if !make_repo(&first) || !make_repo(&second) {
        return;
    }
    let mut app = app_with(&first, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.panels.sidebar.view, SidebarView::Commit);

    // 换到另一个仓库：还在提交视图，焦点在提交信息里。
    app.switch_folder(&second);
    assert_eq!(app.panels.sidebar.view, SidebarView::Commit);
    assert_eq!(app.focus, Focus::CommitMessage);

    // 换到不是仓库的目录：没有提交视图了，回到文件树。
    app.switch_folder(&plain);
    assert_eq!(app.panels.sidebar.view, SidebarView::Files);
    assert_eq!(app.focus, Focus::Tree);
    for dir in [first, second, plain] {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn long_commit_messages_scroll_with_the_wheel() {
    // 回归：提交信息超过最大行数时，滚轮无法滚动，每帧都回到光标所在的行。
    let Some((mut app, root)) = git_app("git-commit-message-scroll", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    let text: Vec<String> = (1..=30).map(|i| format!("第 {i} 行")).collect();
    app.git.commit.message.set_text(&text.join("\n"));
    paint(&mut app);
    // 光标在末尾，框内露出的是最后几行。
    let bottom = app.git.commit.message.view.scroll;
    assert!(bottom > 0.0);

    let message = app.commit_parts().message;
    app.pointer.mouse = (message.x + 20.0, message.y + 20.0);
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 2.0));
    paint(&mut app);
    let up = app.git.commit.message.view.scroll;
    assert!(up < bottom);
    // 往上滚过头停在第一行，不出现负的滚动量。
    for _ in 0..20 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    }
    paint(&mut app);
    assert_eq!(app.git.commit.message.view.scroll, 0.0);
    // 往下滚过头停在最后一行。
    for _ in 0..20 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, -5.0));
    }
    paint(&mut app);
    assert_eq!(app.git.commit.message.view.scroll, bottom);

    // 键入文字后回到光标所在的行。
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    paint(&mut app);
    app.focus = Focus::CommitMessage;
    let end = app.git.commit.message.text().len();
    app.edit_field(|f| {
        f.set_caret(end, false);
        f.insert("！")
    });
    paint(&mut app);
    assert_eq!(app.git.commit.message.view.scroll, bottom);
    let _ = std::fs::remove_dir_all(&root);
}

/// 在终端里只暂存了一部分的文件与其他文件的处理相同：勾选框为全选，提交的是工作区里的内容，
/// 与暂存区无关。
#[test]
fn files_staged_in_the_terminal_commit_what_is_in_the_working_tree() {
    let Some((mut app, root)) = git_app("git-commit-partly-staged", false) else {
        return;
    };
    let main = root.join("src/main.rs");
    std::fs::write(&main, "staged\n").unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    std::fs::write(&main, "staged\nnot staged\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let entry = status.entries.iter().find(|e| e.path == main).unwrap();
    assert!(entry.staged.is_some() && entry.unstaged.is_some());
    assert_eq!(app.git.commit.file_state(entry), Some(true));

    app.git.commit.message.set_text("提交工作区里的内容");
    app.git_commit();
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&head.stdout),
        "staged\nnot staged\n"
    );
    let status = app.git.status.clone().unwrap();
    assert!(status.entries.iter().all(|e| e.path != main));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：侧栏较窄时按目录排列的改动列表（`commit-narrow-dirs`），检查并起来的目录名截短后
/// 仍以淡色分隔符显示。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_narrow_dirs() {
    let Some((mut app, root)) = git_app("git-commit-narrow", false) else {
        return;
    };
    let deep = root.join("crates/winc-app/src/app/snapshot/git");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("commit.rs"), "a\n").unwrap();
    let view = root.join("crates/winc-app/src/view/git");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(view.join("commit.rs"), "b\n").unwrap();
    app.git_refresh();
    app.git.commit.set_flat(false);
    app.panels.sidebar.width = 200.0;
    app.relayout();
    app.run_action(Action::ShowCommit);
    shoot(&mut app, "commit-narrow-dirs");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_button_names_its_scope_only_for_a_plain_commit() {
    let Some((mut app, root)) = git_app("git-commit-scope", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.commit_scope(), Some(1));

    // 没有勾选、修改上一次提交时都不写范围。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    assert_eq!(app.commit_scope(), None);
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Changes));
    app.git.commit.amend = true;
    assert_eq!(app.commit_scope(), None);
    app.git.commit.amend = false;

    // 游离 `HEAD` 时也写文件数，分支写在上方的分支行里。
    git_in(&root, &["checkout", "-q", "--detach"]);
    app.git_refresh();
    assert_eq!(app.commit_scope(), Some(1));

    let _ = std::fs::remove_dir_all(&root);
}

/// 没有更改时不显示提交卡片：空状态下面是下一步（这里是推送领先的一个提交），焦点从看不见的
/// 提交信息移到改动列表上，`Tab` 也不再把焦点交给它。
#[test]
fn a_clean_tree_hides_the_commit_card_and_offers_the_next_step() {
    let Some((mut app, root)) = git_app("git-commit-clean-card", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert!(app.commit_parts().card.is_some());
    assert_eq!(app.focus, Focus::CommitMessage);

    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "base"]);
    git_in(&root, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
    git_in(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
    git_in(&root, &["branch", "-q", "--set-upstream-to=origin/main"]);
    app.git_refresh();
    paint(&mut app);
    let parts = app.commit_parts();
    assert!(parts.card.is_none());
    assert_eq!(app.focus, Focus::CommitList);
    let next = (app.hits.commit.iter()).find(|(_, t)| *t == CommitTarget::NextStep);
    let (rect, _) = *next.unwrap();
    assert!(parts.list.contains(rect.x, rect.y));
    app.commit_list_key(&winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab));
    assert_eq!(app.focus, Focus::CommitList);

    // 打开「修改上一次提交」时卡片回来，填上上一次的信息；关掉后又收起。
    app.press_commit(CommitTarget::LastCommit);
    app.run_menu_command(crate::app::input::context_menu::MenuCommand::CommitAmend);
    paint(&mut app);
    assert!(app.git.commit.amend);
    assert!(app.commit_parts().card.is_some());
    assert_eq!(app.focus, Focus::CommitMessage);
    app.press_commit(CommitTarget::Amend);
    paint(&mut app);
    assert!(!app.git.commit.amend);
    assert!(app.commit_parts().card.is_none());
    assert_eq!(app.focus, Focus::CommitList);
    let _ = std::fs::remove_dir_all(&root);
}

/// 显示全名的改动行覆盖列表的滚动条：鼠标移到这一行的右端时仍悬停在这一行上（全名保持显示），
/// 在伸出侧栏的部分上滚动时滚动的是改动列表。
#[test]
fn the_full_name_row_covers_the_scrollbar_like_the_file_tree() {
    use winit::event::MouseScrollDelta;
    let Some((dir, long)) = long_name_repo("git-commit-overflow-bar") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    let (i, index) = (rows.iter().enumerate())
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    assert!(app.commit_max_scroll() > 0.0, "list must scroll");
    let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    let spill = app.git.commit.overflow.and_then(|o| o.spill).unwrap();

    // 移到滚动条的可点击范围内：仍悬停在这一行上，全名保持显示。
    app.on_mouse_move(list.right() - 3.0, y);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::Commit(CommitTarget::File(index)));
    assert!(app.git.commit.overflow.is_some_and(|o| o.row == i));

    // 在伸出侧栏的部分上滚动：滚动的是改动列表。
    app.on_mouse_move(spill.x + 20.0, y);
    let before = app.git.commit.scroll;
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -2.0));
    assert!(
        app.git.commit.scroll > before,
        "{} {before}",
        app.git.commit.scroll
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 显示全名的改动行覆盖在侧栏与编辑区之间的分隔条之上，与文件树的处理相同：鼠标从名字经分隔条
/// 移到伸出侧栏的部分时，始终悬停在这一行上，全名不收回；在此处按下选中这一行，不拖动分隔条。
/// 其他行仍可拖动分隔条。
#[test]
fn the_full_name_row_covers_the_splitter_like_the_file_tree() {
    let Some((dir, long)) = long_name_repo("git-commit-overflow-splitter") else {
        return;
    };
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    let rows = app.commit_rows();
    let (i, index) = (rows.iter().enumerate())
        .find_map(|(i, r)| match r {
            CommitRow::File { index, .. } if status.entries[*index].path.ends_with(&long) => {
                Some((i, *index))
            }
            _ => None,
        })
        .unwrap();
    let list = app.commit_parts().list;
    let y = list.y + i as f32 * COMMIT_ROW - app.git.commit.scroll + COMMIT_ROW / 2.0;
    app.on_mouse_move(list.x + 120.0, y);
    paint(&mut app);
    assert!(app.git.commit.overflow.is_some_and(|o| o.row == i));

    let splitter = app.layout.splitter;
    let row = Hover::Commit(CommitTarget::File(index));
    for x in [
        splitter.x + 1.0,
        splitter.x + splitter.w / 2.0,
        splitter.right() - 1.0,
    ] {
        app.on_mouse_move(x, y);
        paint(&mut app);
        assert_eq!(app.pointer.hover, row, "x = {x}");
        assert!(
            app.git.commit.overflow.is_some_and(|o| o.row == i),
            "x = {x}"
        );
    }
    click(&mut app, (splitter.x + splitter.w / 2.0, y));
    assert_eq!(
        app.git.commit.selected.as_deref(),
        Some(status.entries[index].path.as_path())
    );
    assert_eq!(app.layout.splitter, splitter);

    // 其他行：分隔条正常响应。
    let other = y + 3.0 * COMMIT_ROW;
    app.on_mouse_move(splitter.x + splitter.w / 2.0, other);
    paint(&mut app);
    assert_eq!(app.pointer.hover, Hover::Splitter);
    let _ = std::fs::remove_dir_all(&dir);
}
