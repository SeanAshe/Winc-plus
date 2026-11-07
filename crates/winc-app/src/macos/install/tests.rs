//! `install` 的测试：判断应用是否位于「应用程序」文件夹，以及应用所在的卷。

use super::*;

#[test]
fn applications_folders_include_the_user_one_and_subfolders() {
    let home = Path::new("/Users/me");
    for inside in [
        "/Applications/winc.app",
        "/Applications/Utilities/winc.app",
        "/Users/me/Applications/winc.app",
    ] {
        assert!(
            inside_applications(Path::new(inside), Some(home)),
            "{inside}"
        );
    }
    for outside in [
        "/Users/me/Downloads/winc.app",
        "/Volumes/winc/winc.app",
        "/ApplicationsOld/winc.app",
        "/Users/other/Applications/winc.app",
    ] {
        assert!(
            !inside_applications(Path::new(outside), Some(home)),
            "{outside}"
        );
    }
}

#[test]
fn the_volume_is_the_first_folder_under_volumes() {
    assert_eq!(
        volume_of(Path::new("/Volumes/winc 1/winc.app")),
        Some(PathBuf::from("/Volumes/winc 1"))
    );
    assert_eq!(volume_of(Path::new("/Users/me/Downloads/winc.app")), None);
}
