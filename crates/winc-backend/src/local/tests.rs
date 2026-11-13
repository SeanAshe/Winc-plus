//! 本机基础操作的测试：临时文件名的长度，以及目标被打开时的替换。

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-backend-local-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 回归：文件名超过约 228 字节时，临时文件名曾超出文件名长度上限，导致无法保存。
#[test]
fn temporary_names_fit_for_long_file_names() {
    let dir = scratch("long");
    let name = format!("{}.txt", "名".repeat(80));
    assert!(name.len() > 228 && name.len() <= 255);
    let (temp, _) = create_temporary(&dir, &name, false).unwrap();
    assert!(temp.file_name().unwrap().len() <= 255);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 回归（Windows）：大文件文档一直开着原文件、文件系统不支持 POSIX 式的替换时，保存曾报拒绝访问。
///
/// 先移开旧文件再换上新文件：换完以后没有留下移开的文件；新文件换不上时旧文件回到原处。
#[test]
fn files_held_open_are_replaced_by_moving_them_aside() {
    let dir = scratch("aside");
    let path = dir.join("big.txt");
    std::fs::write(&path, "old").unwrap();
    let held = std::fs::File::open(&path).unwrap();
    let temp = dir.join("big.txt.tmp");
    std::fs::write(&temp, "new").unwrap();
    replace_aside(&temp, &path).unwrap();
    drop(held);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "移开的旧文件已删掉"
    );

    let missing = dir.join("missing.tmp");
    assert!(replace_aside(&missing, &path).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "new",
        "旧文件回到原处"
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
