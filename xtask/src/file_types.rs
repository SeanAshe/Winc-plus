//! 安装包声明的文件类型的测试：两种 Windows 安装包的列表一致，常用类型都在。

use std::path::{Path, PathBuf};

fn release_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let path: PathBuf = root.join("release").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `installer.nsi` 的 `ForEachFileType` 里列出的扩展名。
fn nsis_types() -> Vec<String> {
    let script = release_file("windows/installer.nsi");
    let (_, rest) = script.split_once("!macro ForEachFileType Action").unwrap();
    let (body, _) = rest.split_once("!macroend").unwrap();
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("!insertmacro ${Action} "))
        .map(str::to_owned)
        .collect()
}

/// `installer.wxs` 的 `FileTypes` 里列出的扩展名。
fn wix_types() -> Vec<String> {
    let source = release_file("windows/installer.wxs");
    let (_, rest) = source.split_once("<?define FileTypes = ").unwrap();
    let (list, _) = rest.split_once("?>").unwrap();
    list.trim().split(';').map(str::to_owned).collect()
}

#[test]
fn both_windows_installers_register_the_same_types() {
    let types = nsis_types();
    assert_eq!(types, wix_types());
    // 类型列显示的名称取自这个写法（`JSON File`）。
    assert!(types.iter().all(|t| *t == t.to_uppercase()), "要写成大写");
    let mut sorted = types.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(types, sorted, "列表要按字母排序、不重复");
    for common in ["JSON", "MD", "PY", "RS", "TS", "TXT", "YAML"] {
        assert!(types.iter().any(|t| t == common), "缺少 {common}");
    }
}

#[test]
fn the_linux_desktop_entry_declares_types_outside_plain_text() {
    let entry = release_file("linux/winc.desktop");
    let mime = entry
        .lines()
        .find_map(|line| line.strip_prefix("MimeType="))
        .unwrap();
    assert!(mime.ends_with(';'));
    // `application/json` 不是 `text/plain` 的子类型，要单独声明。
    for common in ["text/plain", "inode/directory", "application/json"] {
        assert!(mime.split(';').any(|t| t == common), "缺少 {common}");
    }
}
