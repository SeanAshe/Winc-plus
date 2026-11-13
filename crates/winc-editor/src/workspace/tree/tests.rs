//! 文件树 [`Workspace`] 的测试：按变化只重读受影响的目录、改名与移动保持展开、复制不覆盖、
//! 选中跟随行变化、自然排序与按需展开的缓存，以及连续单子目录的合并显示和递归展开。

use std::fs;

use super::*;

#[test]
fn outside_changes_reread_only_the_folders_they_touch() {
    let root = crate::test_util::scratch("ws-outside");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join("closed")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("src/a.rs"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/a.rs"));
    let names =
        |ws: &Workspace| -> Vec<String> { ws.rows().iter().map(|e| e.name.clone()).collect() };
    assert_eq!(names(&ws), ["closed", "src", "deep", "a.rs"]);

    // 只修改文件内容、在从未展开过的目录里新增文件：文件树不变。
    fs::write(root.join("src/a.rs"), "changed").unwrap();
    fs::write(root.join("closed/new.txt"), "").unwrap();
    assert!(!ws.refresh_paths(&[root.join("src/a.rs"), root.join("closed/new.txt")]));

    // 新建文件：只重新读取 src，选中项不变。
    fs::write(root.join("src/b.rs"), "").unwrap();
    assert!(ws.refresh_paths(&[root.join("src/b.rs")]));
    assert_eq!(names(&ws), ["closed", "src", "deep", "a.rs", "b.rs"]);
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src/a.rs"));

    // 已展开的目录被删除：它从树中消失，重新创建同名目录后不会自动展开。
    ws.reveal(&root.join("src/deep/x.py"));
    fs::remove_dir_all(root.join("src/deep")).unwrap();
    assert!(ws.refresh_paths(&[root.join("src/deep"), root.join("src/deep/x.py")]));
    assert_eq!(names(&ws), ["closed", "src", "a.rs", "b.rs"]);
    fs::create_dir_all(root.join("src/deep")).unwrap();
    assert!(ws.refresh_paths(&[root.join("src/deep")]));
    let deep = ws.rows().iter().find(|e| e.name == "deep").unwrap();
    assert!(!deep.expanded);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn rename_keeps_expansion_and_rejects_bad_names() {
    let root = crate::test_util::scratch("ws-rename");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("readme.md"), "").unwrap();
    fs::write(root.join("other.md"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    ws.set_compact(false);
    let root = ws.root.clone();
    ws.reveal(&root.join("src/deep/x.py"));

    // 目录改名：已展开的子目录仍然展开，并选中新名字所在的行。
    let lib = ws.rename(&root.join("src"), "lib").unwrap();
    assert_eq!(lib, root.join("lib"));
    assert!(root.join("lib/deep/x.py").is_file());
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["lib", "deep", "x.py", "other.md", "readme.md"]);
    assert_eq!(ws.selected_entry().unwrap().path, lib);

    // 只改大小写。
    let upper = ws.rename(&root.join("readme.md"), "README.md").unwrap();
    assert!(ws.rows().iter().any(|e| e.name == "README.md"));
    assert_eq!(upper, root.join("README.md"));

    // 重名、空名字、含斜杠、根目录本身都被拒绝，原文件保持不变。
    for bad in ["other.md", "", "  ", "a/b", "..", "."] {
        assert!(ws.rename(&upper, bad).is_err(), "{bad:?}");
    }
    assert!(ws.rename(&root, "x").is_err());
    assert!(upper.is_file());

    // 同名：什么都不做。
    assert_eq!(ws.rename(&upper, "README.md").unwrap(), upper);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn move_into_keeps_expansion_and_refuses_what_cannot_be_moved() {
    let root = crate::test_util::scratch("ws-move");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("readme.md"), "a").unwrap();
    fs::write(root.join("lib/readme.md"), "b").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    ws.set_compact(false);
    let root = ws.root.clone();
    ws.reveal(&root.join("src/deep/x.py"));

    // 目录移入另一个目录：其中已展开的子目录仍然展开，树展开到新位置并选中它。
    let moved = ws.move_into(&root.join("src"), &root.join("lib")).unwrap();
    assert_eq!(moved, root.join("lib/src"));
    assert!(root.join("lib/src/deep/x.py").is_file());
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["lib", "src", "deep", "x.py", "readme.md", "readme.md"]
    );
    assert_eq!(ws.selected_entry().unwrap().path, moved);

    // 移回根目录。已在目标目录中时什么都不做。
    let back = ws.move_into(&moved, &root).unwrap();
    assert_eq!(back, root.join("src"));
    assert_eq!(ws.move_into(&back, &root).unwrap(), back);

    // 不能移入自身及其子目录，根目录本身不能移动，同名项不被覆盖。
    assert!(ws.move_into(&back, &back).is_err());
    assert!(ws.move_into(&back, &back.join("deep")).is_err());
    assert!(ws.move_into(&root, &back).is_err());
    let clash = ws.move_into(&root.join("readme.md"), &root.join("lib"));
    assert_eq!(clash.unwrap_err().to_string(), "「lib」中已有同名项");
    assert_eq!(fs::read_to_string(root.join("lib/readme.md")).unwrap(), "b");
    assert!(root.join("readme.md").is_file());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn copy_into_copies_folders_and_never_overwrites() {
    let base = crate::test_util::scratch("ws-copy");
    let (root, outside) = (base.join("project"), base.join("outside"));
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::create_dir_all(outside.join("assets/icons")).unwrap();
    fs::write(root.join("lib/readme.md"), "old").unwrap();
    fs::write(outside.join("readme.md"), "new").unwrap();
    fs::write(outside.join("assets/icons/a.svg"), "<svg/>").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();

    // 复制文件：原文件还在，树展开到副本并选中它。
    let copied = ws.copy_into(&outside.join("readme.md"), &root).unwrap();
    assert_eq!(copied, root.join("readme.md"));
    assert_eq!(fs::read_to_string(&copied).unwrap(), "new");
    assert!(outside.join("readme.md").is_file());
    assert_eq!(ws.selected_entry().unwrap().path, copied);

    // 同名项不被覆盖，副本依次改名。
    let second = ws
        .copy_into(&outside.join("readme.md"), &root.join("lib"))
        .unwrap();
    assert_eq!(second, root.join("lib/readme 2.md"));
    assert_eq!(
        fs::read_to_string(root.join("lib/readme.md")).unwrap(),
        "old"
    );
    let third = ws
        .copy_into(&outside.join("readme.md"), &root.join("lib"))
        .unwrap();
    assert_eq!(third, root.join("lib/readme 3.md"));

    // 复制目录：连同其中的内容；不能复制进自身内部，也不能复制到工作区外。
    let assets = ws
        .copy_into(&outside.join("assets"), &root.join("lib"))
        .unwrap();
    assert!(assets.join("icons/a.svg").is_file());
    let lib = root.join("lib");
    assert!(ws.copy_into(&lib, &lib.join("assets")).is_err());
    assert!(ws.copy_into(&root.join("readme.md"), &outside).is_err());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn selection_follows_its_entry_when_rows_change() {
    let root = crate::test_util::scratch("ws-select");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("assets/a.png"), "").unwrap();
    fs::write(root.join("assets/b.png"), "").unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/main.rs"));
    let row = |ws: &Workspace, name: &str| ws.rows().iter().position(|e| e.name == name);

    // 展开上方的目录：行号变化，选中项仍是 main.rs。
    ws.toggle(row(&ws, "assets").unwrap());
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src/main.rs"));

    // 折叠它所在的目录：改为选中该目录。
    ws.toggle(row(&ws, "src").unwrap());
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src"));

    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn renaming_onto_a_different_file_that_differs_only_in_case_is_refused() {
    let root = crate::test_util::scratch("ws-case");
    fs::write(root.join("makefile"), "lower").unwrap();
    // 在不区分大小写的文件系统上，这里写入的是同一个文件，无法覆盖被测情形。
    if fs::write(root.join("Makefile"), "upper").is_ok()
        && fs::read_to_string(root.join("makefile")).unwrap() == "lower"
    {
        let mut ws = Workspace::open(&root).unwrap();
        let root = ws.root.clone();
        assert!(ws.rename(&root.join("makefile"), "Makefile").is_err());
        assert_eq!(fs::read_to_string(root.join("Makefile")).unwrap(), "upper");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn natural_order() {
    let mut names = vec!["file10", "File2", "file1", "a", "B"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["a", "B", "file1", "File2", "file10"]);

    // 只差大小写或前导零的名字也有确定的先后，相同的名字才相等。
    let mut names = vec!["file01", "a.rs", "file1", "A.rs", "file001"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["A.rs", "a.rs", "file1", "file01", "file001"]);
    assert_eq!(natural_cmp("x01y2", "x1y02"), Ordering::Greater);
    assert_eq!(natural_cmp("a.rs", "a.rs"), Ordering::Equal);
}

#[test]
fn lazy_expand_cache_and_reveal() {
    let root = crate::test_util::scratch("ws");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("README.md"), "").unwrap();
    fs::write(root.join("src/main.py"), "").unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();

    let mut ws = Workspace::open(&root).unwrap();
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["src", "README.md"]);

    ws.toggle(0);
    let names: Vec<_> = ws
        .rows()
        .iter()
        .map(|e| (e.name.as_str(), e.depth))
        .collect();
    assert_eq!(
        names,
        vec![("src", 0), ("deep", 1), ("main.py", 1), ("README.md", 0)]
    );

    // 缓存生效：磁盘上新增的文件在 `refresh` 之前看不到。
    fs::write(root.join("src/new.py"), "").unwrap();
    ws.toggle(0);
    ws.toggle(0);
    assert_eq!(ws.rows().len(), 4);
    ws.refresh();
    assert_eq!(ws.rows().len(), 5);

    let target = winc_config::files::canonicalize(root.join("src/deep/x.py")).unwrap();
    ws.reveal(&target);
    assert_eq!(
        ws.selected.map(|i| ws.rows()[i].name.as_str()),
        Some("x.py")
    );
    // 每一行记着上级和子孙的范围，应与按缩进推算的结果一致。
    let rows = ws.rows();
    for (i, e) in rows.iter().enumerate() {
        let parent = rows[..i].iter().rposition(|p| p.depth < e.depth);
        assert_eq!(e.parent, parent.filter(|_| e.depth > 0), "{}", e.name);
        let end = i
            + 1
            + rows[i + 1..]
                .iter()
                .take_while(|c| c.depth > e.depth)
                .count();
        assert_eq!(e.end, end, "{}", e.name);
    }

    // 新建：选中文件时建在它旁边，可以带多级目录，已存在时不覆盖。
    let dir = ws.target_dir();
    assert_eq!(dir, ws.root.join("src/deep"));
    let made = ws.create(&dir, "a/b.py", EntryKind::File).unwrap();
    assert_eq!(made, ws.root.join("src/deep/a/b.py"));
    assert_eq!(ws.selected_entry().map(|e| e.name.as_str()), Some("b.py"));
    assert!(ws.create(&dir, "a/b.py", EntryKind::File).is_err());
    assert!(ws.create(&dir, "../x", EntryKind::Dir).is_err());
    ws.create(&ws.root.clone(), "docs", EntryKind::Dir).unwrap();
    assert!(ws.root.join("docs").is_dir());

    // 全部折叠后选中最外层的目录。
    ws.reveal(&made);
    ws.collapse_all();
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["docs", "src", "README.md"]);
    assert_eq!(ws.selected_entry().map(|e| e.name.as_str()), Some("src"));

    fs::remove_dir_all(&root).unwrap();
}

mod measure {
    use super::*;

    /// 测量：展开着上万行的文件树里切换一个目录（重新拍平全部可见行）要多久。
    ///
    /// `cargo test -p winc-editor --release measure_tree_toggle -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_tree_toggle() {
        let root = crate::test_util::scratch("ws-measure");
        for d in 0..50 {
            let dir = root.join(format!("node_modules/package-{d}"));
            fs::create_dir_all(&dir).unwrap();
            for f in 0..200 {
                fs::write(dir.join(format!("module_file_{f}.js")), "").unwrap();
            }
        }
        let mut ws = Workspace::open(&root).unwrap();
        let root = ws.root.clone();
        for d in 0..50 {
            ws.reveal(&root.join(format!("node_modules/package-{d}/module_file_0.js")));
        }
        let rows = ws.rows().len();
        let last = rows - 201;
        assert_eq!(ws.rows()[last].name, "package-49");
        let start = std::time::Instant::now();
        for _ in 0..100 {
            ws.toggle(last);
        }
        eprintln!("{rows} 行：{:?} / 次", start.elapsed() / 100);
        fs::remove_dir_all(&root).unwrap();
    }
}

/// 回归：去掉上级目录也选中了的项时两两比较，全选上万项时是平方级。
///
/// 结果按显示顺序，只留最上层的项。
#[test]
fn selection_roots_drop_entries_under_selected_folders() {
    let root = crate::test_util::scratch("ws-roots");
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::create_dir_all(root.join("a2")).unwrap();
    fs::write(root.join("a/b/x.txt"), "").unwrap();
    fs::write(root.join("a/y.txt"), "").unwrap();
    fs::write(root.join("a2/z.txt"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("a/b/x.txt"));
    ws.reveal(&root.join("a2/z.txt"));
    // `a2` 的名字以 `a` 开头，但不在 `a` 下面；`a/b/x.txt` 和 `a/y.txt` 都在选中的 `a` 下面。
    for path in ["a", "a/b", "a/b/x.txt", "a/y.txt", "a2/z.txt"] {
        ws.marked.insert(root.join(path));
    }
    assert_eq!(
        ws.selection_roots(),
        [root.join("a"), root.join("a2/z.txt")]
    );
    ws.marked.remove(&root.join("a"));
    assert_eq!(
        ws.selection_roots(),
        [
            root.join("a/b"),
            root.join("a/y.txt"),
            root.join("a2/z.txt")
        ]
    );
    let _ = fs::remove_dir_all(&root);
}

/// 回归：只能移动本工作区里的文件，在另一个窗口的文件树里剪切的文件粘贴过来总是失败。
#[test]
fn entries_from_outside_the_workspace_can_be_moved_in() {
    let root = crate::test_util::scratch("ws-move-in");
    let outside = crate::test_util::scratch("ws-move-in-other");
    fs::write(outside.join("notes.txt"), "笔记").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();

    let moved = ws.move_into(&outside.join("notes.txt"), &root).unwrap();
    assert_eq!(moved, root.join("notes.txt"));
    assert_eq!(fs::read_to_string(&moved).unwrap(), "笔记");
    assert_eq!(ws.selected_entry().unwrap().path, moved);
    // 目标仍须在工作区里。
    fs::write(root.join("back.txt"), "").unwrap();
    assert!(ws.move_into(&root.join("back.txt"), &outside).is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

/// 回归：拖动时每帧在全部行里逐个找放置目标所在的行。
///
/// 逐层往下找的结果与逐个找相同；没有展开到的、不在工作区里的找不到。
#[test]
fn rows_are_found_by_walking_down_the_tree() {
    let root = crate::test_util::scratch("ws-row-of");
    for path in [
        "a/b/c/x.txt",
        "a/b/y.txt",
        "a/z.txt",
        "a2/w.txt",
        "d/e/v.txt",
        "top.txt",
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("a/b/c/x.txt"));
    ws.reveal(&root.join("a2/w.txt"));
    for (i, entry) in ws.rows().iter().enumerate() {
        assert_eq!(ws.row_of(&entry.path), Some(i), "{}", entry.path.display());
    }
    // `d` 没展开：它下面的行不在可见行里。
    assert_eq!(ws.row_of(&root.join("d/e")), None);
    assert_eq!(ws.row_of(&root.join("nowhere")), None);
    assert_eq!(ws.row_of(Path::new("/elsewhere")), None);
    let _ = fs::remove_dir_all(&root);
}

/// 返回各行显示的名字和缩进层级。
fn labels(ws: &Workspace) -> Vec<(String, usize)> {
    ws.rows()
        .iter()
        .map(|e| (e.label().into_owned(), e.depth))
        .collect()
}

/// 创建 `src/main/java/com/{config,model}` 和 `other` 目录，返回打开的工作区。
fn java_tree(name: &str) -> Workspace {
    let root = crate::test_util::scratch(name);
    fs::create_dir_all(root.join("src/main/java/com/config")).unwrap();
    fs::create_dir_all(root.join("src/main/java/com/model")).unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    Workspace::open(&root).unwrap()
}

#[test]
fn single_child_folders_merge_into_one_row_when_expanded() {
    let mut ws = java_tree("ws-compact");
    let root = ws.root.clone();

    // 收起时不读取目录内容，只显示自己的名字。
    assert_eq!(
        labels(&ws),
        [("other".to_owned(), 0), ("src".to_owned(), 0)]
    );

    // 展开一次就到最下一级目录：这一行的路径是最下一级，子项紧跟其后。
    ws.toggle(1);
    assert_eq!(
        labels(&ws),
        [
            ("other".to_owned(), 0),
            ("src/main/java/com".to_owned(), 0),
            ("config".to_owned(), 1),
            ("model".to_owned(), 1),
        ]
    );
    let row = &ws.rows()[1];
    assert_eq!(row.path, root.join("src/main/java/com"));
    assert!(row.expanded);
    assert_eq!(row.chain().len(), 4);
    assert_eq!(ws.rows()[2].parent, Some(1));

    // 收起后仍是一行，再展开不必逐级点击。
    ws.toggle(1);
    assert_eq!(labels(&ws)[1], ("src/main/java/com".to_owned(), 0));
    assert_eq!(ws.rows().len(), 2);
    ws.toggle(1);
    assert_eq!(ws.rows().len(), 4);

    // 全部折叠后回到只有自己的名字。
    ws.collapse_all();
    assert_eq!(labels(&ws)[1], ("src".to_owned(), 0));
}

#[test]
fn folders_with_files_or_several_children_do_not_merge() {
    let root = crate::test_util::scratch("ws-compact-stop");
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::write(root.join("a/b/note.txt"), "").unwrap();
    fs::create_dir_all(root.join("only/file")).unwrap();
    fs::write(root.join("only/file/x.rs"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();

    // `a` 只有 `b`，合并；`b` 里还有文件，合并到此为止。
    ws.toggle(0);
    assert_eq!(
        labels(&ws),
        [
            ("a/b".to_owned(), 0),
            ("c".to_owned(), 1),
            ("note.txt".to_owned(), 1),
            ("only".to_owned(), 0),
        ]
    );

    // 唯一的子项是文件时，文件不并入。
    ws.toggle(3);
    assert_eq!(labels(&ws)[3], ("only/file".to_owned(), 0));
    assert_eq!(labels(&ws)[4], ("x.rs".to_owned(), 1));
}

#[cfg(unix)]
#[test]
fn a_symlink_cycle_does_not_expand_forever() {
    let root = crate::test_util::scratch("ws-compact-link");
    fs::create_dir_all(root.join("a")).unwrap();
    std::os::unix::fs::symlink(root.join("a"), root.join("a/again")).unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    ws.toggle(0);
    assert_eq!(labels(&ws), [("a".to_owned(), 0), ("again".to_owned(), 1)]);
}

#[test]
fn compact_folders_can_be_turned_off() {
    let mut ws = java_tree("ws-compact-off");
    ws.toggle(1);
    ws.set_compact(false);
    let names: Vec<_> = labels(&ws).into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        ["other", "src", "main", "java", "com", "config", "model"]
    );
    ws.set_compact(true);
    assert_eq!(labels(&ws)[1].0, "src/main/java/com");
}

#[test]
fn a_merged_folder_can_be_selected_on_its_own() {
    let mut ws = java_tree("ws-compact-segment");
    let root = ws.root.clone();
    ws.toggle(1);
    let (main, com) = (root.join("src/main"), root.join("src/main/java/com"));
    assert_eq!(ws.row_of(&main), Some(1));
    assert_eq!(ws.row_of(&com), Some(1));
    assert_eq!(ws.row_of(&root.join("src/main/java/com/model")), Some(3));

    // 选中合并的某一级：新建、删除、复制都作用于它。
    ws.select_segment(1, Some(&main));
    assert_eq!(ws.segment(), Some(main.as_path()));
    assert_eq!(ws.current_path(), Some(main.as_path()));
    assert_eq!(ws.target_dir(), main);
    assert_eq!(ws.selection(), std::slice::from_ref(&main));
    assert_eq!(ws.selection_roots(), std::slice::from_ref(&main));

    // 选中最下一级等同于选中这一行；选另一行清掉这一级。
    ws.select_segment(1, Some(&com));
    assert_eq!(ws.segment(), None);
    assert_eq!(ws.target_dir(), com);
    ws.select_segment(1, Some(&main));
    ws.select(Some(0));
    assert_eq!(ws.segment(), None);

    // 定位到合并的某一级。
    ws.reveal(&main);
    assert_eq!(ws.selected, Some(1));
    assert_eq!(ws.segment(), Some(main.as_path()));
}

#[test]
fn merged_rows_split_and_rejoin_with_the_disk() {
    let mut ws = java_tree("ws-compact-change");
    let root = ws.root.clone();
    ws.toggle(1);
    let main = root.join("src/main");
    ws.select_segment(1, Some(&main));

    // 中间的目录多了文件：合并在那里断开，选中的目录有了自己的一行，仍然选中它。
    fs::write(root.join("src/main/note.txt"), "").unwrap();
    assert!(ws.refresh_paths(&[root.join("src/main/note.txt")]));
    let names: Vec<_> = labels(&ws).into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            "other", "src/main", "java/com", "config", "model", "note.txt"
        ]
    );
    assert_eq!(ws.segment(), None);
    assert_eq!(ws.selected_entry().unwrap().path, main);

    // 文件删除后重新合并，选中的目录回到合并的一行里。
    fs::remove_file(root.join("src/main/note.txt")).unwrap();
    assert!(ws.refresh_paths(&[root.join("src/main/note.txt")]));
    assert_eq!(labels(&ws)[1].0, "src/main/java/com");
    assert_eq!(ws.segment(), Some(main.as_path()));

    // 改名后展开状态跟着新路径。
    ws.rename(&root.join("src/main/java"), "kotlin").unwrap();
    assert_eq!(labels(&ws)[1].0, "src/main/kotlin/com");
}

/// 创建含 `a/{b/c,d}` 和被忽略的 `ignored/x/y` 的版本库，返回打开的工作区。
fn nested_tree(name: &str) -> Workspace {
    let root = crate::test_util::scratch(name);
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::create_dir_all(root.join("a/d")).unwrap();
    fs::create_dir_all(root.join("ignored/x/y")).unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    ws.set_compact(false);
    ws
}

/// 返回各行的名字。
fn row_names(ws: &Workspace) -> Vec<String> {
    ws.rows().iter().map(|e| e.name.clone()).collect()
}

#[test]
fn expanding_recursively_opens_every_level_but_skips_ignored_folders() {
    let mut ws = nested_tree("ws-recursive");
    let root = ws.root.clone();
    fs::create_dir_all(root.join("a/ignored/deep")).unwrap();

    assert!(!ws.expand_recursively(0));
    let names = row_names(&ws);
    assert_eq!(
        names[..5],
        ["a", "b", "c", "d", "ignored"].map(str::to_owned),
        "{names:?}"
    );
    // `a/ignored` 符合忽略规则：显示，但不展开。
    assert!(!ws.rows()[4].expanded);
    assert!(ws.rows()[1].expanded && ws.rows()[2].expanded);

    // 递归收起后，下次普通展开只露出一层。
    ws.collapse_recursively(0);
    assert_eq!(row_names(&ws)[..2], ["a", "ignored"].map(str::to_owned));
    ws.toggle(0);
    assert!(!ws.rows()[1].expanded);
}

#[test]
fn a_folder_that_is_itself_ignored_expands_only_one_level() {
    let mut ws = nested_tree("ws-recursive-ignored");
    let index = ws.rows().iter().position(|e| e.name == "ignored").unwrap();
    assert!(!ws.expand_recursively(index));
    assert!(ws.rows()[index].expanded);
    assert_eq!(ws.rows()[index + 1].name, "x");
    assert!(!ws.rows()[index + 1].expanded);
}

#[test]
fn expanding_recursively_stops_at_the_limits() {
    let mut ws = nested_tree("ws-recursive-limit");
    assert!(ws.expand_within(0, 2, 1000));
    let expanded = ws.rows().iter().filter(|e| e.expanded).count();
    assert_eq!(expanded, 2);

    let mut ws = nested_tree("ws-recursive-entries");
    assert!(ws.expand_within(0, 1000, 2));
}

#[cfg(unix)]
#[test]
fn expanding_recursively_does_not_follow_symlinks() {
    let mut ws = nested_tree("ws-recursive-link");
    let root = ws.root.clone();
    std::os::unix::fs::symlink(&root, root.join("a/loop")).unwrap();
    ws.refresh();
    assert!(!ws.expand_recursively(0));
    let link = ws.rows().iter().find(|e| e.name == "loop").unwrap();
    assert!(!link.expanded);
}

/// 不能等待读取的线程（界面线程）上的远端目录：文件树先按空目录显示，缺少的目录交给后台读取
/// 后换入；重读时读不了的目录保持原样，同样交给后台。
#[cfg(unix)]
#[test]
fn folders_that_cannot_be_read_on_this_thread_are_filled_in_later() {
    use std::os::unix::ffi::OsStrExt;
    use winc_backend::remote;

    let local = crate::test_util::scratch("ws-remote");
    fs::create_dir_all(local.join("only/inner")).unwrap();
    fs::write(local.join("only/inner/a.rs"), "").unwrap();
    fs::write(local.join("b.rs"), "").unwrap();
    let target = crate::test_util::loopback("ws-remote");
    let root = winc_backend::virtual_path(&target, local.as_os_str().as_bytes());

    // 限制只作用于当前线程，放在单独的线程上，不影响其他测试。
    std::thread::spawn(move || {
        remote::forbid_blocking();
        let names = |ws: &Workspace| -> Vec<String> {
            ws.rows().iter().map(|e| e.label().into_owned()).collect()
        };
        // 交给「后台」：这里就地执行，执行期间允许等待。
        let fetch = |ws: &mut Workspace| {
            let mut rounds = 0;
            while let Some(scan) = ws.take_missing() {
                let listing = remote::allow_blocking(|| scan.run());
                assert!(ws.apply_scan(listing));
                rounds += 1;
            }
            rounds
        };

        let mut ws = Workspace::open(&root).unwrap();
        assert!(names(&ws).is_empty());
        assert_eq!(fetch(&mut ws), 1);
        assert_eq!(names(&ws), ["only", "b.rs"]);
        // 同一个目录在读好之前只交出一次。
        ws.toggle(0);
        let scan = ws.take_missing().unwrap();
        ws.toggle(0);
        ws.toggle(0);
        assert!(ws.take_missing().is_none());
        assert!(ws.apply_scan(remote::allow_blocking(|| scan.run())));
        // 合并显示的下一级目录在换入后才知道需要，再读一轮。
        assert_eq!(fetch(&mut ws), 1);
        assert_eq!(names(&ws), ["only/inner", "a.rs", "b.rs"]);

        // 重读：读不了不等于目录已不存在，文件树和展开状态保持，变化在后台读取后出现。
        fs::write(local.join("only/inner/c.rs"), "").unwrap();
        ws.refresh();
        assert_eq!(names(&ws), ["only/inner", "a.rs", "b.rs"]);
        assert_eq!(fetch(&mut ws), 1);
        assert_eq!(names(&ws), ["only/inner", "a.rs", "c.rs", "b.rs"]);

        // 连接断开后重读：同样保持原样。
        remote::disconnect(&target);
        ws.refresh();
        while let Some(scan) = ws.take_missing() {
            ws.apply_scan(remote::allow_blocking(|| scan.run()));
        }
        assert_eq!(names(&ws), ["only/inner", "a.rs", "c.rs", "b.rs"]);
    })
    .join()
    .unwrap();
}
