//! 放置目标的判定：放进自身之内、放回原处，以及不同主机之间。

use super::*;

#[test]
fn a_drop_without_effect_has_no_target() {
    let dir = PathBuf::from("/work/src");
    let file = vec![dir.join("a.txt")];
    assert_eq!(drop_dir(&file, dir.clone(), false), None, "移回原处");
    assert_eq!(
        drop_dir(&file, dir.clone(), true),
        Some(dir.clone()),
        "原处复制"
    );
    let folder = vec![dir.clone()];
    assert_eq!(drop_dir(&folder, dir.join("deep"), true), None, "放进自身");
}

/// 不同主机之间只能复制：放进同名的目录也有效果。
#[test]
fn items_from_another_host_are_copied_into_the_folder() {
    let target = winc_backend::Target::new("drop-host").unwrap();
    let remote = winc_backend::virtual_path(&target, b"/work/src");
    let file = vec![PathBuf::from("/work/src/a.txt")];
    assert!(!same_host(&file, &remote));
    assert_eq!(drop_dir(&file, remote.clone(), false), Some(remote));
}
