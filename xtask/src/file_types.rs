//! 安装包声明的文件类型的测试：两种 Windows 安装包的列表一致，常用类型都在，双击即运行的类型
//! 不注册。

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

/// `installer.nsi` 的 `ForEachRetiredFileType` 里列出的扩展名。
fn nsis_retired_types() -> Vec<String> {
    let script = release_file("windows/installer.nsi");
    let (_, rest) = script
        .split_once("!macro ForEachRetiredFileType Action")
        .unwrap();
    let (body, _) = rest.split_once("!macroend").unwrap();
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("!insertmacro ${Action} "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn types_windows_runs_on_double_click_are_never_registered() {
    // 回归：注册了 BAT、CMD 后，Windows 双击这类文件时改为询问用哪个程序打开，脚本不再运行。
    // 这些类型的默认操作是运行：列进「打开方式」会使双击时改为询问。
    let runnable = [
        "BAT", "CMD", "COM", "CPL", "EXE", "HTA", "LNK", "MSC", "MSI", "PIF", "REG", "SCR", "VBE",
        "VBS", "WSF", "WSH",
    ];
    let types = nsis_types();
    for kind in runnable {
        assert!(!types.iter().any(|t| t == kind), "不能注册 {kind}");
    }
    // 较早版本注册过的类型在安装和卸载时清除，并且不再出现在注册的列表中。
    let retired = nsis_retired_types();
    for kind in ["BAT", "CMD"] {
        assert!(retired.iter().any(|t| t == kind), "清除列表缺少 {kind}");
    }
    assert!(retired.iter().all(|t| !types.contains(t)));
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
