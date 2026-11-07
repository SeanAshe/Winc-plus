//! `api` 的测试：覆盖发布的 `conpty.dll` 的使用条件。

use super::*;

/// 两个文件都存在才使用发布的版本：只有库时它会启动系统的控制台，只有控制台程序时无库可加载。
#[test]
fn the_bundled_library_needs_both_files() {
    let dir = std::env::temp_dir().join(format!("winc-conpty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let library = dir.join("conpty.dll");
    let host = dir.join("OpenConsole.exe");
    assert_eq!(bundled_library(&dir), None);

    std::fs::write(&library, b"").unwrap();
    assert_eq!(bundled_library(&dir), None);

    std::fs::write(&host, b"").unwrap();
    assert_eq!(bundled_library(&dir), Some(library.clone()));

    std::fs::remove_file(&library).unwrap();
    assert_eq!(bundled_library(&dir), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
