//! 编辑器状态的测试：标签的打开、移动与关闭，占位标签与尚未读取的标签，书签在改名和重试后的
//! 归属。

use super::*;

#[cfg(unix)]
#[test]
fn documents_under_a_symlinked_folder_follow_a_rename_in_the_tree() {
    let base = crate::test_util::scratch("moved-link");
    std::fs::create_dir_all(base.join("shared/docs")).unwrap();
    std::fs::create_dir_all(base.join("ws")).unwrap();
    std::fs::write(base.join("shared/docs/a.md"), "x").unwrap();
    std::os::unix::fs::symlink(base.join("shared/docs"), base.join("ws/docs")).unwrap();

    // 经文件树打开：树中的路径经过符号链接，文档的路径是链接指向的规范路径。
    let mut editor = Editor::default();
    editor.open(&base.join("ws/docs/a.md"));
    std::fs::rename(base.join("ws/docs/a.md"), base.join("ws/docs/b.md")).unwrap();
    editor.moved(&base.join("ws/docs/a.md"), &base.join("ws/docs/b.md"));

    let real = winc_config::files::canonicalize(base.join("shared/docs/b.md")).unwrap();
    assert_eq!(
        editor.active().unwrap().path.as_deref(),
        Some(real.as_path())
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn bookmarks_survive_a_placeholder_tab_until_the_file_opens() {
    let base = crate::test_util::scratch("marks-retry");
    let path = canonical_path(&base).join("a.txt");
    let marks = Bookmarks::from_lines(vec![0, 2]);

    // 文件暂时不在（例如切到了没有它的分支）：占位标签不移走书签，保存会话时也不丢。
    let mut editor = Editor::default();
    editor.bookmarks.insert(path.clone(), marks.clone());
    let index = editor.open(&path);
    assert!(editor.documents[index].unsupported.is_some());
    assert_eq!(editor.all_bookmarks().get(&path), Some(&marks));

    // 文件回来了，「重试」打开后书签移进文档。
    std::fs::write(&path, "a\nb\nc\n").unwrap();
    assert!(editor.reopen(index, false) && editor.load(index));
    assert_eq!(editor.documents[index].bookmarks(), &marks);
    assert!(editor.bookmarks.is_empty());
    assert_eq!(editor.all_bookmarks().get(&path), Some(&marks));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn folded_blocks_come_back_when_the_file_is_opened_again() {
    let base = crate::test_util::scratch("folds-reopen");
    std::fs::create_dir_all(&base).unwrap();
    let path = canonical_path(&base).join("notes.md");
    std::fs::write(&path, "# A\na1\n## B\nb1\nb2\n# C\nc1\n").unwrap();

    // 折叠两个标题后关闭标签：折叠的块留在编辑器中，保存会话时一并给出。
    let mut editor = Editor::default();
    let index = editor.open(&path);
    let doc = &mut editor.documents[index];
    assert!(doc.toggle_fold_at(2, 4) && doc.toggle_fold_at(5, 4));
    let folded = doc.folded();
    assert_eq!(folded.len(), 2);
    assert_eq!(editor.all_folds().get(&path), Some(&folded));
    editor.close(index);
    assert_eq!(editor.all_folds().get(&path), Some(&folded));

    // 再次打开：恢复折叠，编辑器中不再留有记录。
    let index = editor.open(&path);
    assert_eq!(editor.documents[index].folded(), folded);
    assert!(editor.folds.is_empty());

    // 全部展开后关闭：不再记录该文件。
    let doc = &mut editor.documents[index];
    assert!(doc.unfold_at(2) && doc.unfold_at(5));
    editor.close(index);
    assert!(editor.all_folds().is_empty());

    // 文件在此期间被改动：仍对得上的块恢复，对不上的丢弃。
    editor.folds.insert(path.clone(), folded.clone());
    std::fs::write(&path, "# A\na1\n## B\nb1\nb2\n# C\nc1\nc2\n").unwrap();
    let index = editor.open(&path);
    assert_eq!(editor.documents[index].folded(), [folded[0]]);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn moving_a_tab_keeps_the_active_document() {
    let mut editor = Editor::default();
    for _ in 0..4 {
        editor.new_untitled();
    }
    let ids: Vec<_> = editor.documents.iter().map(|d| d.id).collect();
    let order = |e: &Editor| e.documents.iter().map(|d| d.id).collect::<Vec<_>>();

    // 活动标签是第 2 个；把第 0 个移到末尾后，活动标签前移一位，仍指向同一个文档。
    editor.active = Some(2);
    editor.move_document(0, 3);
    assert_eq!(order(&editor), [ids[1], ids[2], ids[3], ids[0]]);
    assert_eq!(editor.active().unwrap().id, ids[2]);

    // 移动活动标签本身。
    editor.move_document(1, 0);
    assert_eq!(order(&editor), [ids[2], ids[1], ids[3], ids[0]]);
    assert_eq!(editor.active().unwrap().id, ids[2]);

    // 目标位置越界时不移动。
    editor.move_document(0, 9);
    assert_eq!(order(&editor), [ids[2], ids[1], ids[3], ids[0]]);
}

#[test]
fn output_tabs_are_read_only_and_reused() {
    let mut editor = Editor::default();
    editor.new_untitled();
    let first = editor.show_output("格式化输出", "a\n");
    let doc = &editor.documents[first];
    assert_eq!(doc.name(), "格式化输出");
    assert!(doc.readonly_reason().is_some());
    assert!(!doc.is_dirty());

    // 再次显示同名输出：替换内容，复用同一个标签。
    editor.active = Some(0);
    assert_eq!(editor.show_output("格式化输出", "b\n"), first);
    assert_eq!(editor.active, Some(first));
    assert_eq!(editor.documents.len(), 2);
    assert_eq!(
        editor.documents[first].text_range(0, 10).as_deref(),
        Some("b\n")
    );

    // 内容替换后 `revision` 继续递增：按（`DocumentId`，`revision`）缓存的查找结果
    // 不会用于新内容。
    let mut search = Search::default();
    search.query = "c".into();
    search.update(&editor.documents[first]);
    assert!(search.matches().is_empty());
    editor.show_output("格式化输出", "c\n");
    search.update(&editor.documents[first]);
    assert_eq!(search.matches(), &[(0, 1)]);
}

#[test]
fn close_keeps_active_index_valid() {
    let mut editor = Editor::default();
    editor.new_untitled();
    editor.new_untitled();
    editor.new_untitled();
    editor.active = Some(2);
    editor.close(0);
    assert_eq!(editor.active, Some(1));
    editor.close(1);
    assert_eq!(editor.active, Some(0));
    editor.close(0);
    assert_eq!(editor.active, None);
}

#[test]
fn open_deduplicates_by_canonical_path() {
    let dir = crate::test_util::scratch("editor");
    let path = dir.join("a.txt");
    std::fs::write(&path, "x").unwrap();

    let mut editor = Editor::default();
    assert_eq!(editor.open(&path), 0);
    editor.new_untitled();
    assert_eq!(editor.open(&dir.join(".").join("a.txt")), 0);
    assert_eq!(editor.documents.len(), 2);
    assert_eq!(editor.active, Some(0));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unopenable_files_get_a_placeholder_tab() {
    let dir = crate::test_util::scratch("placeholder");
    let binary = dir.join("a.bin");
    std::fs::write(&binary, [1u8, 0, 2]).unwrap();

    let mut editor = Editor::default();
    let index = editor.open(&binary);
    let doc = &mut editor.documents[index];
    assert_eq!(doc.unsupported, Some(Unsupported::Binary(3)));
    assert_eq!(
        doc.unsupported.as_ref().unwrap().to_string(),
        "二进制文件或不支持的文本编码（3 字节），无法显示预览"
    );
    assert_eq!(winc_core::Lines::end_pos(doc.text()), 0);

    // 占位标签不能保存，原文件不会被空内容覆盖。
    assert!(doc.save().is_err());
    assert_eq!(std::fs::read(&binary).unwrap(), [1, 0, 2]);

    // 以文本方式打开后成为只读的普通文档：标签和 `DocumentId` 不变，`revision` 大于占位标签的。
    let (id, revision) = (doc.id, doc.revision);
    assert!(editor.reopen(index, true) && editor.load(index));
    let doc = &editor.documents[index];
    assert_eq!((doc.id, doc.unsupported.clone()), (id, None));
    assert!(doc.revision > revision);
    assert_eq!(doc.encoding, Encoding::Binary);
    assert!(doc.readonly);

    // 文件不存在：占位标签记录原因；重试仍然失败时原因不变。
    let missing = editor.open(&dir.join("missing.txt"));
    let reason = Some(Unsupported::Unreadable("文件不存在".into()));
    assert_eq!(editor.documents[missing].unsupported, reason);
    // 不存在的文件，路径由规范化后的所在目录与文件名拼成。
    let expected = winc_config::files::canonicalize(&dir)
        .unwrap()
        .join("missing.txt");
    assert_eq!(editor.documents[missing].path, Some(expected));
    assert!(editor.reopen(missing, false) && editor.load(missing));
    assert_eq!(editor.documents[missing].unsupported, reason);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn pending_tabs_read_the_file_only_when_loaded() {
    let dir = crate::test_util::scratch("pending");
    let path = canonical_path(&dir).join("a.txt");
    std::fs::write(&path, "a\nb\nc\n").unwrap();
    let marks = Bookmarks::from_lines(vec![1]);

    let mut editor = Editor::default();
    editor.bookmarks.insert(path.clone(), marks.clone());
    let index = editor.open_pending(&path);
    assert_eq!(editor.open_pending(&dir.join(".").join("a.txt")), index);
    assert_eq!(editor.active, None);

    // 尚未读取：文本为空、只读，不能保存，不随磁盘同步，书签仍留在编辑器里。
    let doc = &mut editor.documents[index];
    assert!(doc.is_pending() && doc.readonly);
    assert_eq!(winc_core::Lines::end_pos(doc.text()), 0);
    assert!(doc.save().is_err());
    std::fs::write(&path, "a\nb\nc\nd\n").unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Unchanged);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\nc\nd\n");
    assert_eq!(editor.all_bookmarks().get(&path), Some(&marks));

    // 读入后是普通文档：标识不变、版本递增，书签移进文档。
    let (id, revision) = (editor.documents[index].id, editor.documents[index].revision);
    assert!(editor.load(index));
    assert!(!editor.load(index));
    let doc = &editor.documents[index];
    assert!(!doc.is_pending() && !doc.readonly);
    assert_eq!((doc.id, winc_core::Lines::line_count(doc.text())), (id, 5));
    assert!(doc.revision > revision);
    assert_eq!(doc.bookmarks(), &marks);

    // 用 `open` 切换到尚未读取的标签时同样读入。
    let other = canonical_path(&dir).join("b.txt");
    std::fs::write(&other, "x").unwrap();
    let pending = editor.open_pending(&other);
    assert_eq!(editor.open(&other), pending);
    assert!(!editor.documents[pending].is_pending());

    std::fs::remove_dir_all(&dir).unwrap();
}
