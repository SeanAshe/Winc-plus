//! `Document` 的测试：状态栏上的缩进与换行风格、打开与保存的编码、磁盘冲突与同步、
//! 末尾追加、大文件的后台保存、列对齐与粘滞滚动的缓存。

#[cfg(unix)]
use super::disk::write_atomic;
use super::*;
use crate::test_util::scratch;

#[cfg(unix)]
#[test]
fn atomic_writes_use_a_fresh_private_temporary_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("atomic");
    let path = dir.join(".env");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    // 临时文件新建时只给自己读写，写完换成原文件的权限。
    write_atomic(&path, false, true, |out| {
        let tmp = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .unwrap();
        assert_eq!(tmp.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        out.write_all(b"new")
    })
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "没有留下临时文件");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn indentation_and_line_endings_chosen_in_the_status_bar() {
    let mut doc = Document::from_text(DocumentId(1), "fn a() {\n    b();\n}\n");
    doc.detected_indent = indent::detect(&doc.text);
    doc.set_language("a.rs");
    assert_eq!(doc.indent(), Indent::Spaces(4));

    // 换成 Tab：宽度不变，一个 Tab 显示为 4 列；此时修改宽度改的是 Tab 的显示宽度。
    doc.use_indent(true, 8);
    assert_eq!(doc.indent(), Indent::Tabs);
    assert_eq!(doc.tab_width(8), 4);
    doc.set_indent_size(2);
    assert_eq!(doc.indent_size(8), 2);
    doc.detect_indentation();
    assert_eq!(doc.indent(), Indent::Spaces(4));
    assert_eq!(doc.tab_width(8), 8);

    // 转换缩进：文本中的缩进随之转换，一次撤销即可还原。
    doc.convert_indentation(true, 8).unwrap();
    assert_eq!(doc.text_range(0, 100).unwrap(), "fn a() {\n\tb();\n}\n");
    assert_eq!(doc.indent(), Indent::Tabs);
    doc.execute(Command::Undo, 10, 8);
    assert_eq!(doc.text_range(0, 100).unwrap(), "fn a() {\n    b();\n}\n");

    // 转换换行符：撤销后换行风格也恢复。
    doc.change_line_ending(LineEnding::CrLf).unwrap();
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    assert_eq!(
        doc.text_range(0, 100).unwrap(),
        "fn a() {\r\n    b();\r\n}\r\n"
    );
    doc.execute(Command::Undo, 10, 8);
    assert_eq!(doc.line_ending, LineEnding::Lf);
    doc.execute(Command::Redo, 10, 8);
    assert_eq!(doc.line_ending, LineEnding::CrLf);

    // 只读文档不能修改文本，但可以更改之后使用的缩进。
    doc.readonly = true;
    assert!(doc.change_line_ending(LineEnding::Lf).is_err());
    assert!(doc.convert_indentation(false, 8).is_err());
    doc.use_indent(false, 8);
    assert_eq!(doc.indent(), Indent::Spaces(4));
}

#[test]
fn save_round_trip_preserves_bytes() {
    let dir = scratch("save");
    let path = dir.join("a.py");
    let original = "\u{feff}x = 1\r\ny = 2\r\n";
    fs::write(&path, original).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Utf8Bom);
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    assert_eq!(doc.highlighter.language().id, "python");
    assert_eq!(doc.text.rope().unwrap().char(0), 'x');

    doc.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rejects_binary_and_marks_invalid_utf8_readonly() {
    let dir = scratch("bin");
    let binary = dir.join("a.bin");
    fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
    assert!(matches!(
        Document::open(DocumentId(1), binary),
        Err(OpenError::Binary(4))
    ));

    // 以文本方式打开：只读，编码记为 `Binary`。
    let forced = Document::open_binary(DocumentId(3), dir.join("a.bin")).unwrap();
    assert_eq!(forced.encoding, Encoding::Binary);
    assert_eq!(
        forced.readonly_reason(),
        Some("二进制文件，已以只读方式打开")
    );

    // UTF-8 中有一个非法字节：仍按 UTF-8 显示，非法字节显示为 U+FFFD，文档只读。
    let damaged = dir.join("b.txt");
    let mut bytes = "中文中文中文中文中文".as_bytes().to_vec();
    bytes.push(0xFF);
    fs::write(&damaged, &bytes).unwrap();
    let doc = Document::open(DocumentId(2), damaged).unwrap();
    assert_eq!(doc.encoding, Encoding::Lossy);
    assert!(doc.readonly);
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "中文中文中文中文中文\u{fffd}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn other_encodings_open_edit_and_save_back() {
    let dir = scratch("encodings");

    // GBK：能识别和编辑，按原编码写回（编码记为 GB18030，与 GBK 使用同一解码器）。
    let path = dir.join("gbk.txt");
    let (bytes, _, _) = encoding_rs::GBK.encode("你好，世界。这是一段用 GBK 写的中文。\n");
    fs::write(&path, &bytes).unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Legacy(encoding_rs::GB18030));
    assert!(!doc.readonly);
    doc.selection = Selection::point(0);
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "改：",
        winc_core::EditKind::Other,
    ));
    doc.save().unwrap();
    let (expected, _, _) = encoding_rs::GBK.encode("改：你好，世界。这是一段用 GBK 写的中文。\n");
    assert_eq!(fs::read(&path).unwrap(), expected.as_ref());

    // GB18030 可以表示所有 Unicode 字符：emoji 也能保存（用四字节序列表示）。
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "😀",
        winc_core::EditKind::Other,
    ));
    doc.save().unwrap();
    let (expected, _, _) =
        encoding_rs::GB18030.encode("😀改：你好，世界。这是一段用 GBK 写的中文。\n");
    assert_eq!(fs::read(&path).unwrap(), expected.as_ref());

    // Big5 无法表示的字符：不写入，错误信息指出该字符，磁盘文件保持原样。
    let path = dir.join("big5.txt");
    let (bytes, _, _) = encoding_rs::BIG5.encode("繁體中文的檔案，用來測試編碼寫不進去的字。\n");
    fs::write(&path, &bytes).unwrap();
    let mut doc = Document::open(DocumentId(3), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Legacy(encoding_rs::BIG5));
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "😀",
        winc_core::EditKind::Other,
    ));
    let error = doc.save().unwrap_err().to_string();
    assert!(
        error.contains("「😀」") && error.contains("Big5"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes.as_ref());
    assert!(doc.is_dirty());

    // 带 BOM 的 UTF-16：不视为二进制文件，可以编辑，写回时仍用 UTF-16。
    let path = dir.join("utf16.txt");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "Hello 中文\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&path, &bytes).unwrap();
    let mut doc = Document::open(DocumentId(2), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Utf16Le);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "Hello 中文\r\n");
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    doc.save().unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reload_keeps_the_encoding_the_user_picked() {
    let dir = scratch("reload-encoding");
    let path = dir.join("gbk.txt");
    // 很短的文本无法可靠识别编码：先按 GB18030 重新打开，再由用户改选 Big5。重新读取时
    // 必须沿用用户选择的编码，而不是自动猜测的编码。
    fs::write(&path, b"\xC4\xE3\xBA\xC3\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.reopen_with_encoding(Encoding::Legacy(encoding_rs::GB18030))
        .unwrap();
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "你好\n");
    doc.reopen_with_encoding(Encoding::Legacy(encoding_rs::BIG5))
        .unwrap();
    assert_eq!(doc.encoding, Encoding::Legacy(encoding_rs::BIG5));

    // 其他程序在末尾追加了一行：重新读取后仍按用户选择的编码解码，不会退回自动猜测的编码。
    fs::write(&path, b"\xC4\xE3\xBA\xC3\n\xCA\xC0\xBD\xE7\n").unwrap();
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    // 按 Big5 解码得到的是另一组字符：保留用户选择的编码，不退回自动猜测的 GB18030。
    assert_eq!(doc.encoding, Encoding::Legacy(encoding_rs::BIG5));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_deleted_on_disk_is_not_saved_quietly() {
    let dir = scratch("deleted");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert!(!doc.deleted_on_disk() && doc.can_save_quietly());

    // 其他程序删除了文件：不算修改，文档保留；自动保存不写入，用户手动保存时重新创建。
    fs::remove_file(&path).unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Unchanged);
    assert!(doc.deleted_on_disk() && !doc.can_save_quietly());
    assert!(!doc.check_overwrite());
    doc.save().unwrap();
    assert!(path.exists());
    assert!(!doc.deleted_on_disk() && doc.can_save_quietly());
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn read_only_files_are_not_overwritten() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("readonly");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();

    // 目录可写、文件只读：保存返回错误，自动保存跳过该文件。
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(!doc.can_save_quietly());
    let err = doc.save().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "没有留下临时文件");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(doc.can_save_quietly());
    doc.save().unwrap();
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn saving_a_hard_linked_file_keeps_the_link() {
    use std::os::unix::fs::MetadataExt;

    let dir = scratch("hardlink");
    let (path, link) = (dir.join("a.txt"), dir.join("b.txt"));
    fs::write(&path, "one\n").unwrap();
    fs::hard_link(&path, &link).unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.execute(Command::SelectAll, 10, 4);
    doc.execute(Command::Insert("two\n".into()), 10, 4);

    // 改名替换只会换掉其中一个名字：有多个硬链接时原地写入，两个名字都看到新内容。
    doc.save().unwrap();
    assert_eq!(fs::read_to_string(&link).unwrap(), "two\n");
    assert_eq!(fs::metadata(&path).unwrap().nlink(), 2);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "没有留下临时文件");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn sync_reloads_clean_and_flags_dirty() {
    let dir = scratch("sync");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Unchanged);

    // 修改时间的精度可能只到秒：显式设置一个更晚的时间，保证能检测到变化。
    let later = SystemTime::now() + std::time::Duration::from_secs(5);
    fs::write(&path, "two\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "two\n");

    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "x",
        winc_core::EditKind::Other,
    ));
    let later = later + std::time::Duration::from_secs(5);
    fs::write(&path, "three\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(!doc.can_save_quietly(), "改过了，还没告诉用户");
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Conflict);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Unchanged);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "xtwo\n");
    // 已提示过冲突，自动保存仍不能覆盖磁盘内容；用户手动保存后冲突解除。
    assert!(!doc.can_save_quietly());
    doc.save().unwrap();
    assert!(doc.can_save_quietly());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn conflicts_can_take_the_disk_version_or_keep_mine() {
    let dir = scratch("conflict-choice");
    let path = dir.join("a.py");
    fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let touch = |text: &str, secs: u64| {
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + std::time::Duration::from_secs(secs))
            .unwrap();
    };
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    let edit = |doc: &mut Document, at: usize, text: &str| {
        doc.apply(winc_core::edit::replace_selection(
            doc.text.rope().unwrap(),
            Selection::point(at),
            text,
            winc_core::EditKind::Other,
        ));
    };

    // 改了一处尚未保存，磁盘上的文件又被其他程序修改：保存前检查出来，标记为冲突。
    edit(&mut doc, 4, "TWO ");
    doc.selection = Selection::point(12);
    touch("one\ntwo\nthree\nfour\n", 5);
    assert!(!doc.in_conflict());
    assert!(doc.check_overwrite());
    assert!(doc.in_conflict());
    assert_eq!(doc.disk_text().unwrap(), "one\ntwo\nthree\nfour\n");

    // 后台读取期间文档又被编辑：读到的结果不再对应，不换内容，冲突仍然保留。
    let take = doc.begin_take_disk_version().unwrap();
    edit(&mut doc, 0, "x");
    assert_eq!(doc.finish_take_disk_version(take.read()).unwrap(), None);
    assert!(doc.in_conflict());
    doc.execute(Command::Undo, 10, 4);
    doc.selection = Selection::point(12);

    // 用磁盘上的版本：内容换掉，没有未保存的修改，光标还在 three 上；⌘Z 回到自己的修改。
    assert!(doc.take_disk_version().unwrap());
    assert!(!doc.in_conflict() && !doc.is_dirty());
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "one\ntwo\nthree\nfour\n"
    );
    assert_eq!(doc.selection.head, 8);
    assert!(doc.can_save_quietly());
    doc.execute(Command::Undo, 10, 4);
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "one\nTWO two\nthree\n"
    );
    assert!(doc.is_dirty());

    // 又冲突一次，这次保留自己的：冲突解除，之后的保存覆盖磁盘内容。
    touch("changed again\n", 10);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Conflict);
    doc.keep_mine();
    assert!(!doc.in_conflict() && doc.is_dirty());
    assert!(doc.can_save_quietly());
    assert!(!doc.check_overwrite());
    doc.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "one\nTWO two\nthree\n");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tidying_keeps_every_cursor_and_its_line() {
    let dir = scratch("tidy-cursors");
    fs::write(
        dir.join(".editorconfig"),
        "root = true\n[*]\ntrim_trailing_whitespace = true\n",
    )
    .unwrap();
    let path = dir.join("a.py");
    fs::write(&path, "a = \nb = \nc = \n").unwrap();
    let mut doc = Document::open(DocumentId(1), path).unwrap();
    let cursors = vec![Selection::point(4), Selection::point(14)];
    doc.set_selections(cursors.clone(), 1);

    // 自动保存：两个光标所在的行都保留行尾空白，只清理中间那一行；两个光标都还在。
    assert!(doc.tidy_for_save(None, true));
    assert_eq!(doc.text_range(0, 100).unwrap(), "a = \nb =\nc = \n");
    assert_eq!(
        doc.selections(),
        (vec![Selection::point(4), Selection::point(13)], 1)
    );
    doc.execute(Command::Undo, 10, 4);
    assert_eq!(doc.text_range(0, 100).unwrap(), "a = \nb = \nc = \n");
    assert_eq!(doc.selections(), (cursors, 1));

    // 手动保存：全部清理，光标随之移动。
    assert!(doc.tidy_for_save(None, false));
    assert_eq!(doc.text_range(0, 100).unwrap(), "a =\nb =\nc =\n");
    assert_eq!(
        doc.selections(),
        (vec![Selection::point(3), Selection::point(11)], 1)
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_save_as_keeps_the_old_path() {
    let dir = scratch("save-as-fails");
    let path = dir.join("gbk.txt");
    fs::write(&path, b"\xC4\xE3\xBA\xC3\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.reopen_with_encoding(Encoding::Legacy(encoding_rs::GB18030))
        .unwrap();
    doc.set_save_encoding(Encoding::Legacy(encoding_rs::GBK));
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "😀",
        winc_core::EditKind::Other,
    ));

    // GBK 无法表示 emoji：另存为失败，文档仍对应原来的文件，新文件没有创建。
    let target = dir.join("copy.md");
    assert!(doc.save_as(target.clone()).is_err());
    assert_eq!(doc.path.as_deref(), Some(path.as_path()));
    assert_eq!(doc.highlighter.language().id, "text");
    assert!(!target.exists());
    assert!(doc.is_dirty());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn taking_a_damaged_disk_version_makes_the_document_read_only() {
    let dir = scratch("take-damaged");
    let path = dir.join("gbk.txt");
    // 按 GBK 打开的文件，未保存的修改与其他程序写入的内容冲突。
    fs::write(&path, b"\xC4\xE3\xBA\xC3\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.reopen_with_encoding(Encoding::Legacy(encoding_rs::GBK))
        .unwrap();
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "mine ",
        winc_core::EditKind::Other,
    ));
    // 写入的内容里有 GBK 解不开的字节：换成磁盘版本后必须只读，否则保存会把这些字节
    // 改写成替换字符。
    let damaged = b"\xC4\xE3\x81\x20\n";
    fs::write(&path, damaged).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Conflict);
    assert!(!doc.take_disk_version().unwrap(), "整个重读，不能撤回");
    assert_eq!(doc.encoding, Encoding::Lossy);
    assert!(doc.readonly && !doc.is_dirty() && !doc.in_conflict());
    assert!(doc.save().is_err());
    assert_eq!(fs::read(&path).unwrap(), damaged);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changes_within_the_same_timestamp_are_still_noticed() {
    let dir = scratch("same-mtime");
    let path = dir.join("a.txt");
    fs::write(&path, "one\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    let known = doc.disk_mtime().unwrap();

    // 时间只精确到秒的文件系统：其他程序在同一秒内改写，修改时间没有变化而大小改变了。
    let rewrite = |text: &str| {
        fs::write(&path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(known)
            .unwrap();
    };
    rewrite("one two\n");
    assert!(!doc.can_save_quietly());
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "one two\n");

    // 有未保存的修改时同样算作冲突，按 ⌘S 之前也能检测到。
    doc.apply(winc_core::edit::replace_selection(
        doc.text.rope().unwrap(),
        Selection::point(0),
        "x",
        winc_core::EditKind::Other,
    ));
    rewrite("theirs\n");
    assert!(doc.check_overwrite());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn edits_before_an_append_are_not_mistaken_for_one() {
    let dir = scratch("edit-then-append");
    let path = dir.join("app.log");
    fs::write(&path, "first line\nsecond line\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    // 开头原地改了同样长度的内容，又在末尾追加：不能只接上追加的部分。
    fs::write(&path, "FIRST LINE\nsecond line\nthird\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "FIRST LINE\nsecond line\nthird\n"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn growing_files_only_read_what_was_added() {
    use std::io::Write as _;

    let dir = scratch("append");
    let path = dir.join("app.log");
    fs::write(&path, "\u{feff}第一行\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.selection = Selection::point(2);
    let later = |n: u64| SystemTime::now() + std::time::Duration::from_secs(5 * n);
    let append = |text: &[u8], n: u64| {
        let mut file = fs::File::options().append(true).open(&path).unwrap();
        file.write_all(text).unwrap();
        file.set_modified(later(n)).unwrap();
    };

    // 末尾追加了两行：接到文档后面，光标不动，文档仍是未修改状态。
    append("第二行\nthird\n".as_bytes(), 1);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Appended);
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "第一行\n第二行\nthird\n"
    );
    assert_eq!(doc.selection, Selection::point(2));
    assert!(!doc.is_dirty());

    // 写入未完成，最后一个字只写入了第一个字节：先追加前面完整的部分，写完后再追加其余。
    append(&"4 四".as_bytes()[..3], 2);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Appended);
    assert!(
        doc.text_range(0, usize::MAX)
            .unwrap()
            .ends_with("third\n4 ")
    );
    append(&"4 四".as_bytes()[3..], 3);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Appended);
    assert!(
        doc.text_range(0, usize::MAX)
            .unwrap()
            .ends_with("third\n4 四")
    );
    assert!(!doc.readonly);

    // 末尾之前的内容被修改（不只是末尾追加）：整个重新读取。
    fs::write(&path, "换掉了\n还多了一行\n很多字\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later(4))
        .unwrap();
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Reloaded);
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "换掉了\n还多了一行\n很多字\n"
    );

    // 保存之后，从保存时的文件末尾继续追加。
    doc.save().unwrap();
    append(b"after save\n", 5);
    assert_eq!(doc.sync_with_disk().unwrap(), DiskChange::Appended);
    assert!(
        doc.text_range(0, usize::MAX)
            .unwrap()
            .ends_with("很多字\nafter save\n")
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn large_utf16_opens_as_text_and_saves_back() {
    let dir = scratch("large-utf16");
    let path = dir.join("CommonLocalization.txt");
    let line = format!("KEY_{}\t平宝\r\n", "a".repeat(240));
    let mut units: Vec<u8> = Vec::new();
    units.extend_from_slice(&0xFEFFu16.to_le_bytes());
    let encoded: Vec<u8> = line.encode_utf16().flat_map(u16::to_le_bytes).collect();
    while units.len() as u64 <= LARGE_FILE {
        units.extend_from_slice(&encoded);
    }
    let tail = "NEEDLE\t找到了\r\n";
    units.extend(tail.encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&path, &units).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert_eq!(doc.encoding, Encoding::Utf16Le);
    assert!(!doc.is_large() && !doc.readonly);
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    assert!(doc.text().line_content(0).slice().starts_with("KEY_"));
    let needle = doc.text().line_count() - 2;
    assert_eq!(doc.text().line_content(needle).slice(), "NEEDLE\t找到了");

    doc.selection = Selection::point(doc.text().line_start(needle));
    doc.execute(Command::Insert("改".into()), 10, 4);
    doc.save().unwrap();
    let saved = fs::read(&path).unwrap();
    assert!(saved.starts_with(&[0xFF, 0xFE]));
    let text = String::from_utf16(
        &saved[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(text.contains("改NEEDLE\t找到了\r\n"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn large_files_edit_save_and_undo() {
    let dir = scratch("large");
    let path = dir.join("big.py");
    let line = "if status == 200:  # 请求完成\n";
    let count = LARGE_FILE as usize / line.len() + 10;
    fs::write(&path, line.repeat(count)).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert!(doc.is_large() && !doc.readonly);
    assert_eq!(doc.text().line_count(), count + 1);

    // 在第 2 行开头输入，再在文件末尾追加一行。
    doc.selection = Selection::point(doc.text().line_start(1));
    doc.execute(Command::Insert("x".into()), 10, 4);
    doc.selection = Selection::point(doc.text().end_pos());
    doc.execute(Command::Insert("end\n".into()), 10, 4);
    assert!(doc.is_dirty());
    assert_eq!(
        doc.text().line_content(1).slice(),
        format!("x{}", line.trim_end())
    );

    doc.save().unwrap();
    assert!(!doc.is_dirty() && doc.is_large());
    let saved = fs::read_to_string(&path).unwrap();
    assert_eq!(saved.len(), line.len() * count + 5);
    assert!(saved.starts_with(&format!("{line}x{line}")) && saved.ends_with("end\n"));

    // 保存后文档改为引用新文件，撤销仍然正常。
    doc.execute(Command::Undo, 10, 4);
    doc.execute(Command::Undo, 10, 4);
    assert_eq!(doc.text().line_content(1).slice(), line.trim_end());
    assert_eq!(doc.text().line_count(), count + 1);

    let mut search = crate::Search::default();
    search.query = "STATUS".into();
    // 查找按 8 ms 的时间片分段进行，机器繁忙（例如并行运行测试）时一次调用搜不完；
    // 反复调用直到搜完或结果达到上限，使断言不依赖机器当时的负载。
    search.update(&doc);
    while !search.is_complete() {
        search.update(&doc);
    }
    assert!(search.is_truncated());
    let (start, end) = search.matches()[1];
    assert_eq!(doc.text().pos_to_line_col(start), (1, 3));
    assert_eq!(doc.text_range(start, end).unwrap(), "status");

    // 选区超过 `EDIT_LIMIT`：拒绝编辑，但撤销不受限制。
    doc.selection = Selection::new(0, doc.text().end_pos());
    assert!(doc.edit_blocked(&Command::Delete { word: false }).is_some());
    assert!(doc.edit_blocked(&Command::Undo).is_none());
    // 超过上限的选区不是主选区（主选区只是个光标）时同样拒绝。
    let end = doc.text().end_pos();
    doc.set_selections(vec![Selection::new(0, end - 1), Selection::point(end)], 1);
    assert_eq!(doc.selection, Selection::point(end));
    assert!(doc.edit_blocked(&Command::Delete { word: false }).is_some());
    doc.set_selections(vec![Selection::point(0), Selection::point(end)], 1);
    assert!(doc.edit_blocked(&Command::Delete { word: false }).is_none());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn large_file_refuses_to_save_after_external_change() {
    let dir = scratch("large-ext");
    let path = dir.join("big.log");
    fs::write(&path, "x".repeat(LARGE_FILE as usize + 1)).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.execute(Command::Insert("a".into()), 10, 4);
    fs::write(&path, "changed").unwrap();
    assert!(doc.save().is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "changed");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn large_files_still_save_after_the_original_grows_but_only_as_utf8() {
    use std::io::Write as _;

    let dir = scratch("large-grow");
    let path = dir.join("big.log");
    let content = "x\n".repeat(LARGE_FILE as usize / 2 + 1);
    fs::write(&path, &content).unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.execute(Command::Insert("a".into()), 10, 4);

    // 编辑期间日志在末尾追加内容：另存为和保存仍然可以执行，写出的是文档中的内容。
    let mut log = fs::OpenOptions::new().append(true).open(&path).unwrap();
    log.write_all(b"more\n").unwrap();
    let copy = dir.join("copy.log");
    doc.save_as(copy.clone()).unwrap();
    let saved = fs::read(&copy).unwrap();
    assert_eq!(saved, format!("a{content}").as_bytes());

    // 片段原样写出原文件的字节：只能用 UTF-8 保存，带或不带 BOM。
    assert!(!doc.can_save_with(Encoding::Utf16Le));
    assert!(!doc.set_save_encoding(Encoding::Legacy(encoding_rs::GBK)));
    assert!(doc.set_save_encoding(Encoding::Utf8Bom));
    assert_eq!(doc.encoding, Encoding::Utf8Bom);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changed_lines_cover_only_the_lines_that_differ() {
    let case = |old: &str, new: &str| {
        changed_lines(&ropey::Rope::from_str(old), &ropey::Rope::from_str(new))
    };
    assert_eq!(case("a\nb\nc\n", "a\nB\nc\n"), (1, 2, 2));
    assert_eq!(case("a\nb\nc\n", "a\nb\nx\nc\n"), (2, 2, 3));
    assert_eq!(case("a\nb\nc\n", "a\nc\n"), (1, 2, 1));
    assert_eq!(case("a\nb", "a\nbc"), (1, 2, 2));
    assert_eq!(case("a\nb\n", "a\nb\n"), (2, 2, 2));
    assert_eq!(case("", "x"), (0, 1, 1));
    // 换行符变了：`\r` 和 `\r\n` 都算一处换行，两边按各自的行切分。
    let (head, old_end, new_end) = case("a\rb\nc", "a\r\nb\nc");
    assert!(head == 0 && old_end >= 1 && new_end >= 1);
}

#[test]
fn aligned_columns_follow_edits_without_drifting() {
    let mut text = String::from("id,name,city\n");
    for i in 0..300 {
        text.push_str(&format!(
            "{i},name{},\"c, {}\"\n",
            "x".repeat(i % 17),
            i % 5
        ));
    }
    let mut d = Document::from_text(DocumentId(1), &text);
    d.set_language("a.csv");
    let full = |d: &Document| d.measure_columns(4);
    assert_eq!(d.columns(4).as_deref().cloned(), full(&d));
    // 加宽、删除最宽的一格、加一列、删行、在行中间断行：结果每次都与整个重新测量相同。
    let edits: [(usize, &str, usize); 6] = [
        (5, "wider-than-anything-else", 0),
        (5, "", 24),
        (40, ",extra", 0),
        (100, "", 300),
        (2000, "\n", 0),
        (0, "\r\n", 0),
    ];
    for (at, insert, remove) in edits {
        let at = at.min(d.text().rope().unwrap().len_chars());
        let to = (at + remove).min(d.text().rope().unwrap().len_chars());
        d.set_selections(vec![Selection::new(at, to)], 0);
        d.execute(Command::Insert(insert.into()), 10, 4);
        assert_eq!(
            d.columns(4).as_deref().cloned(),
            full(&d),
            "{insert:?} at {at}"
        );
    }
}

#[test]
fn sticky_scopes_keep_their_memo_across_edits_far_below() {
    let mut text = String::from("class A:\n    def f(self):\n");
    for i in 0..400 {
        text.push_str(&format!("        x{i} = {i}\n"));
    }
    let mut d = Document::from_text(DocumentId(1), &text);
    d.set_language("a.py");
    let rows: Vec<usize> = (200..210).collect();
    let (before, settled) = d.sticky_scopes(&rows, false, 5, 4);
    assert!(settled && !d.sticky_memo.1.is_empty());
    // 在下方很远处输入：记下的结果保留，算出的结果与新打开的文档相同。
    let at = d.text().line_start(380);
    d.set_selections(vec![Selection::point(at)], 0);
    d.execute(Command::Insert("        y = 1\n".into()), 10, 4);
    assert!(!d.sticky_memo.1.is_empty());
    let mut fresh = Document::from_text(DocumentId(2), &d.text().rope().unwrap().to_string());
    fresh.set_language("a.py");
    assert_eq!(
        d.sticky_scopes(&rows, false, 5, 4),
        fresh.sticky_scopes(&rows, false, 5, 4)
    );
    assert_eq!(d.sticky_scopes(&rows, false, 5, 4).0, before);
    assert_eq!(d.scope_depth(300, 4), fresh.scope_depth(300, 4));
    // 在上面改：记下的结果作废。
    let at = d.text().line_start(1);
    d.set_selections(vec![Selection::point(at)], 0);
    d.execute(Command::Insert("    pass\n".into()), 10, 4);
    assert!(d.sticky_memo.1.is_empty());
}

#[test]
fn unchanged_lines_are_only_known_across_plain_edits() {
    let mut d = Document::from_text(DocumentId(1), "a\nb\nc\nd\n");
    let start = d.revision;
    assert_eq!(d.unchanged_lines_since(start), usize::MAX);
    for line in [3, 1, 2] {
        let at = d.text().line_start(line);
        d.set_selections(vec![Selection::point(at)], 0);
        d.execute(Command::Insert("x".into()), 10, 4);
    }
    assert_eq!(d.unchanged_lines_since(start), 1);
    assert_eq!(d.unchanged_lines_since(d.revision - 1), 2);
    // 换语言也推进了版本，但不是编辑：不知道哪些行没变。
    let before = d.revision;
    d.set_language("a.py");
    assert_eq!(d.unchanged_lines_since(before), 0);
    assert_eq!(d.unchanged_lines_since(start), 0);
}

#[test]
fn large_files_save_in_the_background_while_editing() {
    let dir = scratch("large-background");
    let path = dir.join("big.log");
    let line = "2026-10-29 ok\n";
    let count = LARGE_FILE as usize / line.len() + 10;
    fs::write(&path, line.repeat(count)).unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert!(doc.is_large());
    // 普通文件不经过后台：返回 `None`，调用方按正常流程保存。
    let small = dir.join("small.txt");
    fs::write(&small, "x").unwrap();
    let mut plain = Document::open(DocumentId(2), small).unwrap();
    assert!(plain.begin_save().unwrap().is_none());

    // 改了一处，发起保存；写的时候又改了一处。
    doc.selection = Selection::point(0);
    doc.execute(Command::Insert("A".into()), 10, 4);
    let job = doc.begin_save().unwrap().unwrap();
    doc.execute(Command::Insert("B".into()), 10, 4);
    let saved = job.run();
    doc.finish_save(saved).unwrap();

    // 磁盘上是发起那一刻的内容；文档里两处都在，仍算未保存，撤销回那一刻算已保存。
    let disk = fs::read_to_string(&path).unwrap();
    assert!(disk.starts_with(&format!("A{line}")));
    assert_eq!(
        doc.text().line_content(0).slice(),
        format!("AB{}", line.trim_end())
    );
    assert!(doc.is_dirty() && doc.can_save_quietly());
    doc.execute(Command::Undo, 10, 4);
    assert!(!doc.is_dirty());
    doc.execute(Command::Redo, 10, 4);

    // 被替换的原文件还开着，文档仍然能读、能再存；期间没再改时存完就是已保存。
    assert_eq!(doc.text().line_content(count).slice(), "");
    let job = doc.begin_save().unwrap().unwrap();
    doc.finish_save(job.run()).unwrap();
    assert!(!doc.is_dirty() && doc.is_large());
    let disk = fs::read_to_string(&path).unwrap();
    assert!(disk.starts_with(&format!("AB{line}")));
    assert_eq!(disk.len(), line.len() * count + 2);

    // 写入期间文档的路径改变：新路径上没有保存，返回错误，文档仍有未保存的修改。
    doc.execute(Command::Insert("C".into()), 10, 4);
    let job = doc.begin_save().unwrap().unwrap();
    doc.moved_to(dir.join("moved.log"));
    assert!(doc.finish_save(job.run()).is_err());
    assert!(doc.is_dirty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sizes_read_like_finder() {
    assert_eq!(size_text(0), "0 字节");
    assert_eq!(size_text(999), "999 字节");
    assert_eq!(size_text(1000), "1.0 KB");
    assert_eq!(size_text(2_400_000), "2.4 MB");
    // 进位到下一个单位，不写成「1000.0 KB」。
    assert_eq!(size_text(999_999), "1.0 MB");
    assert_eq!(size_text(3_200_000_000), "3.2 GB");
}

/// 回归：「按编码重新打开」把整个文件读进内存再解码，打开以后变大的文件也不例外。
#[test]
fn reopening_with_an_encoding_refuses_files_grown_past_the_large_limit() {
    let dir = scratch("reopen-large");
    let path = dir.join("grown.txt");
    fs::write(&path, "short\n").unwrap();
    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    fs::write(&path, "x".repeat(LARGE_FILE as usize + 1)).unwrap();
    let error = doc
        .reopen_with_encoding(Encoding::Legacy(encoding_rs::GBK))
        .unwrap_err();
    assert_eq!(error, t!("editor-large-utf8-only"));
    assert_eq!(doc.text_range(0, usize::MAX).unwrap(), "short\n");
    fs::remove_dir_all(&dir).unwrap();
}

/// 回归：有多个硬链接的大文件保存时改名替换，断开了链接（属于其他用户的文件同理会换掉属主）。
#[cfg(unix)]
#[test]
fn large_files_with_hard_links_are_not_replaced() {
    let dir = scratch("large-link");
    let path = dir.join("big.log");
    let content = "x\n".repeat(LARGE_FILE as usize / 2 + 1);
    fs::write(&path, &content).unwrap();
    let other = dir.join("other-name.log");
    fs::hard_link(&path, &other).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    assert!(doc.is_large());
    doc.execute(Command::Insert("a".into()), 10, 4);
    let error = doc.save().unwrap_err();
    assert_eq!(error.to_string(), t!("editor-large-identity"));
    assert!(doc.is_dirty());
    // 两个名字仍指向同一个、没有被改动的文件。
    use std::os::unix::fs::MetadataExt;
    assert_eq!(fs::metadata(&path).unwrap().nlink(), 2);
    assert_eq!(fs::read_to_string(&other).unwrap(), content);
    fs::remove_dir_all(&dir).unwrap();
}

/// 回归：改名替换以后文件的属组变成当前用户的默认组；当前用户只属于一个组时跳过。
#[cfg(unix)]
#[test]
fn saving_keeps_the_group() {
    use std::os::unix::fs::MetadataExt;
    let dir = scratch("group");
    let path = dir.join("shared.txt");
    fs::write(&path, "old\n").unwrap();
    let current = fs::metadata(&path).unwrap().gid();
    let groups = std::process::Command::new("id").arg("-G").output().unwrap();
    let other = String::from_utf8_lossy(&groups.stdout)
        .split_whitespace()
        .filter_map(|g| g.parse::<u32>().ok())
        .find(|&g| g != current);
    let Some(other) = other else {
        fs::remove_dir_all(&dir).unwrap();
        return;
    };
    std::os::unix::fs::chown(&path, None, Some(other)).unwrap();

    let mut doc = Document::open(DocumentId(1), path.clone()).unwrap();
    doc.execute(Command::Insert("new ".into()), 10, 4);
    doc.save().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new old\n");
    assert_eq!(fs::metadata(&path).unwrap().gid(), other);
    fs::remove_dir_all(&dir).unwrap();
}
