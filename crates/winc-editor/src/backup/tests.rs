//! `backup` 模块的测试：备份文件的格式与往返恢复、按目录加锁、临时文件与损坏的头、
//! 备份之后磁盘文件被修改或删除。

use super::*;
use crate::document::content::Content;
use crate::test_util::scratch;
use winc_core::edit::{EditKind, Transaction};

fn type_into(doc: &mut Document, at: usize, text: &str) {
    let before = doc.selection;
    let change = winc_core::edit::Change {
        start: at,
        removed: String::new(),
        inserted: text.into(),
    };
    let after = Selection::point(at + text.chars().count());
    doc.apply(Transaction::new(
        vec![change],
        before,
        after,
        EditKind::Other,
    ));
}

#[test]
fn folder_keys_stay_the_same_across_versions() {
    // 备份子目录以它命名：值变了，升级前留下的备份就找不到了。
    assert_eq!(
        folder_key(Path::new("/Users/me/project")),
        "7a1ddf583e7aabcf"
    );
}

#[test]
fn large_unsaved_buffers_are_backed_up_too() {
    // 超过 64 MiB 的未命名文档也会备份，读回来一字不差。
    let dir = scratch("large");
    let backups = Backups::new(&dir.join("backups"), None);
    let mut editor = Editor::default();
    editor.new_untitled();
    let line = "0123456789abcdef".repeat(8) + "\n";
    let big = line.repeat((65 << 20) / line.len() + 1);
    type_into(&mut editor.documents[0], 0, &big);
    assert!(editor.documents[0].text().rope().unwrap().len_bytes() > 64 << 20);
    assert!(Backups::wanted(&editor.documents[0]));
    let name = untitled_name();
    backups
        .snapshot(&editor.documents[0], &name, 0)
        .unwrap()
        .write()
        .unwrap();
    let read = backups.read_all();
    assert_eq!(read.len(), 1);
    let text = read[0].text().unwrap();
    assert_eq!(text.len(), big.len());
    assert!(text == big);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unsaved_files_come_back_as_they_were() {
    let dir = scratch("round");
    let folder = dir.join("项目");
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    let backups = Backups::new(&dir.join("backups"), Some(&folder));

    // 修改一个文件，再新建一个未保存的文档，内容含换行、中文和 CRLF。
    let mut editor = Editor::default();
    editor.open(&file);
    type_into(&mut editor.documents[0], 0, "// 改了\r\n");
    editor.documents[0].selection = Selection::new(2, 4);
    editor.new_untitled();
    type_into(&mut editor.documents[1], 0, "草稿\n第二行");
    let name = untitled_name();
    for (tab, doc) in editor.documents.iter().enumerate() {
        backups.snapshot(doc, &name, tab).unwrap().write().unwrap();
    }

    // 没有修改的文档不备份。
    let mut clean = Editor::default();
    clean.open(&file);
    assert!(backups.snapshot(&clean.documents[0], &name, 0).is_none());

    // 下次启动：按标签序号读回，恢复后的内容和选区与备份前相同，且仍是未保存状态。
    let read = backups.read_all();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].path.as_deref(), Some(file.as_path()));
    assert_eq!(read[1].path, None);
    let mut next = Editor::default();
    for backup in &read {
        assert!(!next.restore_backup(backup).unwrap().1);
    }
    for (old, new) in editor.documents.iter().zip(&next.documents) {
        assert_eq!(old.text().rope().unwrap(), new.text().rope().unwrap());
        assert_eq!(old.path, new.path);
        assert!(new.is_dirty());
    }
    assert_eq!(next.documents[0].selection, Selection::new(2, 4));

    // 撤销后回到磁盘上的内容。
    let doc = &mut next.documents[0];
    let mut history = std::mem::take(&mut doc.history);
    if let Content::Rope(rope) = &mut doc.text {
        history.undo(rope);
    }
    assert_eq!(doc.text().rope().unwrap().to_string(), "fn main() {}\n");
    assert!(!history.is_dirty());

    // 删除备份后，目录也随之删除。
    for backup in &read {
        remove(&backup.file).unwrap();
    }
    assert!(!backups.dir().exists());
    remove(&read[0].file).unwrap();

    let _ = fs::remove_dir_all(&dir);
}

/// 取不到锁时按占用者决定是否等待：另一个实例占用时立即放弃，本进程启动的外部程序在 exec 之前
/// 继承的描述符占用时等到它关闭。
#[test]
fn waiting_for_the_lock_is_only_for_descriptors_inherited_by_child_processes() {
    use std::time::Instant;

    let dir = scratch("lock-wait");
    let backups = Backups::new(&dir.join("backups"), Some(&dir));
    let (wait, soon) = (Duration::from_secs(5), Duration::from_secs(1));

    // 本进程中另一把锁占用（同一进程中的另一个实例）：不等待。
    let first = backups.lock().unwrap();
    let started = Instant::now();
    assert!(backups.lock_within(wait).is_none());
    assert!(started.elapsed() < soon);

    // 只有 Unix 上子进程会继承建议锁；其他平台到此为止。
    #[cfg(not(unix))]
    drop(first);
    #[cfg(unix)]
    {
        // 本进程放开锁以后，复制出的描述符仍然占用，与子进程继承的描述符相同：等到它关闭。
        let inherited = first._file.try_clone().unwrap();
        drop(first);
        assert!(backups.lock().is_none());
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(inherited);
        });
        let lock = backups.lock_within(wait).expect("描述符关闭后取到锁");
        closer.join().unwrap();

        // 锁文件中记着其他进程的编号：占用者是另一个 winc 进程，不等待。
        let other = lock._file.try_clone().unwrap();
        drop(lock);
        fs::write(backups.lock_path(), "1").unwrap();
        let started = Instant::now();
        assert!(backups.lock_within(wait).is_none());
        assert!(started.elapsed() < soon);
        drop(other);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn only_one_winc_owns_a_folders_backups() {
    let dir = scratch("lock");
    let backups = Backups::new(&dir.join("backups"), Some(&dir));
    let first = backups.lock().expect("没人占着");
    assert!(backups.lock().is_none(), "另一个 winc 占着");
    let other = Backups::new(&dir.join("backups"), None);
    assert!(other.lock().is_some(), "别的目录不受影响");
    drop(first);
    assert!(backups.lock().is_some(), "放开以后能占");

    // 写备份的临时文件每次使用新名字，写完后不遗留。
    let mut editor = Editor::default();
    editor
        .documents
        .push(Document::from_text(crate::DocumentId(1), ""));
    type_into(&mut editor.documents[0], 0, "x");
    let snapshot = backups.snapshot(&editor.documents[0], "u", 0).unwrap();
    snapshot.write().unwrap();
    snapshot.write().unwrap();
    let names: Vec<_> = fs::read_dir(backups.dir()).unwrap().flatten().collect();
    assert_eq!(names.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn leftover_temp_files_and_bad_headers_are_skipped() {
    let dir = scratch("bad");
    let folder = dir.join("src");
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("a.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    let backups = Backups::new(&dir.join("backups"), Some(&folder));

    let mut editor = Editor::default();
    editor.open(&file);
    type_into(&mut editor.documents[0], 0, "// 改\n");
    let snapshot = backups.snapshot(&editor.documents[0], "", 0).unwrap();
    snapshot.write().unwrap();

    // 写到一半断电留下的临时文件：内容可能不完整，不能当作备份恢复。
    let tmp = snapshot.file.with_extension("tmp");
    fs::write(&tmp, "winc-backup 1 {\"anchor\":0,").unwrap();
    let mut good = fs::read(&snapshot.file).unwrap();
    good.truncate(good.len() - 3);
    fs::write(&tmp, &good).unwrap();
    assert_eq!(backups.read_all().len(), 1, "只认改好名的那一份");
    assert!(!tmp.exists(), "留下的临时文件读的时候删掉");

    // 头中的日期损坏：转换为 `SystemTime` 会溢出。读备份在每次启动时执行，不能 panic。
    fs::write(
        snapshot.file.with_extension("1"),
        "winc-backup 1 {\"mtime\":[1e19,0],\"anchor\":0,\"head\":0}\nx\n",
    )
    .unwrap();
    fs::write(
        snapshot.file.with_extension("2"),
        "winc-backup 1 {\"mtime\":[-1,0],\"anchor\":0,\"head\":0}\nx\n",
    )
    .unwrap();
    let read = backups.read_all();
    assert_eq!(read.len(), 3, "临时文件跳过，正文还在的三份都读出来");
    for backup in read.iter().filter(|b| b.file != snapshot.file) {
        assert!(
            backup.mtime.is_none(),
            "{} 的日期应当当没有",
            backup.file.display()
        );
        assert_eq!(backup.text().unwrap(), "x\n");
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn files_changed_or_gone_meanwhile_are_still_restored() {
    let dir = scratch("changed");
    let file = dir.join("a.txt");
    let gone = dir.join("gone.txt");
    fs::write(&file, "one\n").unwrap();
    fs::write(&gone, "x\n").unwrap();
    let backups = Backups::new(&dir.join("backups"), None);

    let mut editor = Editor::default();
    editor.open(&file);
    editor.open(&gone);
    type_into(&mut editor.documents[0], 0, "mine ");
    type_into(&mut editor.documents[1], 0, "kept ");
    for (tab, doc) in editor.documents.iter().enumerate() {
        backups.snapshot(doc, "", tab).unwrap().write().unwrap();
    }

    // winc 关闭期间，其他程序修改了 a.txt、删除了 gone.txt。
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&file, "theirs\n").unwrap();
    fs::remove_file(&gone).unwrap();

    let mut next = Editor::default();
    let read = backups.read_all();
    let (a, conflict) = next.restore_backup(&read[0]).unwrap();
    assert!(conflict, "磁盘上的文件在这期间被改过");
    assert_eq!(next.documents[a].text().rope().unwrap(), "mine one\n");
    assert!(
        !next.documents[a].can_save_quietly(),
        "自动保存不能悄悄盖掉"
    );

    let (g, conflict) = next.restore_backup(&read[1]).unwrap();
    assert!(!conflict);
    let doc = &mut next.documents[g];
    assert_eq!(doc.path.as_deref(), Some(gone.as_path()));
    assert_eq!(doc.text().rope().unwrap(), "kept x\n");
    assert!(doc.is_dirty());
    // 已删除的文件（例如期间切换了分支）不由自动保存重建，用户手动保存时才写入。
    assert!(doc.deleted_on_disk() && !doc.can_save_quietly());
    doc.save().unwrap();
    assert!(!doc.deleted_on_disk() && doc.can_save_quietly());
    assert_eq!(fs::read_to_string(&gone).unwrap(), "kept x\n");

    // 文件被替换成二进制而无法打开：内容放进一个未命名的标签，不丢失。
    fs::write(&file, b"\0\0\0").unwrap();
    let mut next = Editor::default();
    let (u, _) = next.restore_backup(&read[0]).unwrap();
    assert_eq!(next.documents[u].path, None);
    assert_eq!(next.documents[u].text().rope().unwrap(), "mine one\n");
    assert!(next.documents[u].is_dirty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unresolved_conflicts_survive_a_restart() {
    let dir = scratch("conflict-kept");
    let file = dir.join("a.txt");
    fs::write(&file, "one\n").unwrap();
    let backups = Backups::new(&dir.join("backups"), None);

    // 有未保存的修改时别的程序改了文件，冲突还没处理就退出：备份时记下的修改时间已经晚于
    // 对方写入的时间，只看修改时间会以为没有冲突。
    let mut editor = Editor::default();
    editor.open(&file);
    type_into(&mut editor.documents[0], 0, "mine ");
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&file, "theirs\n").unwrap();
    let doc = &mut editor.documents[0];
    assert_eq!(doc.sync_with_disk().unwrap(), crate::DiskChange::Conflict);
    backups.snapshot(doc, "", 0).unwrap().write().unwrap();

    let mut next = Editor::default();
    let read = backups.read_all();
    assert!(read[0].conflict);
    let (a, conflict) = next.restore_backup(&read[0]).unwrap();
    assert!(conflict, "冲突跟着备份回来");
    assert!(next.documents[a].in_conflict());
    assert!(!next.documents[a].can_save_quietly());

    // 选择保留以后再备份：不再记为冲突。
    next.documents[a].keep_mine();
    let snapshot = backups.snapshot(&next.documents[a], "", 0).unwrap();
    snapshot.write().unwrap();
    assert!(!backups.read_all()[0].conflict);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn broken_backups_are_skipped() {
    let dir = scratch("broken");
    let backups = Backups::new(&dir, Some(Path::new("/p")));
    fs::create_dir_all(backups.dir()).unwrap();
    for (name, bytes) in [
        ("a", &b"not a backup"[..]),
        ("b", b"winc-backup 1 {\"head\": 0}\nx"),
        ("c", b"winc-backup 1 {bad\nx"),
        ("d", b"winc-backup 1 {\"anchor\": 0, \"head\": 1}"),
        ("e", b"winc-backup 1 {\"anchor\": 0, \"head\": 1}\n\xff"),
        ("f", b"winc-backup 1 {\"anchor\": 0, \"head\": 1}\nok"),
    ] {
        fs::write(backups.dir().join(name), bytes).unwrap();
    }
    // 只读头：正文不是 UTF-8 的备份（`e`）列出来，恢复时才发现读不出，不恢复，也不开标签。
    let read = backups.read_all();
    assert_eq!(read.len(), 2);
    assert!(read[0].text().is_err());
    let mut editor = Editor::default();
    assert_eq!(editor.restore_backup(&read[0]), None);
    assert!(editor.documents.is_empty());
    assert_eq!(read[1].text().unwrap(), "ok");
    assert_eq!(read[1].path, None);

    // 同一个目录每次计算得到的子目录相同，不同目录得到的不同。
    assert_eq!(Backups::new(&dir, Some(Path::new("/p"))), backups);
    assert_ne!(Backups::new(&dir, Some(Path::new("/q"))), backups);
    assert_ne!(Backups::new(&dir, None), backups);
    assert_ne!(untitled_name(), untitled_name());

    let _ = fs::remove_dir_all(&dir);
}

/// 回归：备份头未记编码和行尾时，磁盘上被删除的 GBK、CRLF 文件恢复后保存成 UTF-8 和 LF。
#[test]
fn encoding_and_line_endings_survive_a_restore() {
    use winc_core::text::LineEnding;
    let dir = scratch("encoding");
    let file = dir.join("gbk.txt");
    let (bytes, _, _) = encoding_rs::GBK.encode("你好，世界。这是一段用 GBK 写的中文。\r\n");
    fs::write(&file, &bytes).unwrap();
    let backups = Backups::new(&dir.join("backups"), None);

    let mut editor = Editor::default();
    editor.open(&file);
    let doc = &mut editor.documents[0];
    let encoding = doc.encoding;
    assert!(matches!(encoding, Encoding::Legacy(_)));
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    type_into(doc, 0, "改：");
    backups.snapshot(doc, "", 0).unwrap().write().unwrap();
    fs::remove_file(&file).unwrap();

    let mut next = Editor::default();
    let read = backups.read_all();
    assert_eq!(read[0].encoding, Some(encoding));
    let (i, _) = next.restore_backup(&read[0]).unwrap();
    let doc = &mut next.documents[i];
    assert_eq!(doc.encoding, encoding);
    assert_eq!(doc.line_ending, LineEnding::CrLf);
    doc.save().unwrap();
    let (expected, _, _) = encoding_rs::GBK.encode("改：你好，世界。这是一段用 GBK 写的中文。\r\n");
    assert_eq!(fs::read(&file).unwrap(), &*expected);

    let _ = fs::remove_dir_all(&dir);
}
