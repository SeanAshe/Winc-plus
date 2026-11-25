//! 导出与导入设置的测试。

use super::*;

/// 返回本测试独用的空目录。
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-transfer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn exporting_copies_the_config_file_with_its_comments() {
    let dir = scratch("export");
    let config = dir.join("config.json");
    let to = dir.join("exported.json");
    let text = "{\n  // 字号\n  \"editor\": { \"size\": 15 }\n}\n";
    std::fs::write(&config, text).unwrap();
    export_config(&config, &to).unwrap();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), text);

    // 没有配置文件：导出的文件为空对象，导入后全部为默认值。
    std::fs::remove_file(&config).unwrap();
    export_config(&config, &to).unwrap();
    assert_eq!(
        Config::parse(&std::fs::read_to_string(&to).unwrap()).0,
        Config::default()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn importing_replaces_the_config_file_and_keeps_a_backup() {
    let dir = scratch("import");
    let config = dir.join("config.json");
    let from = dir.join("settings.json");
    let old = "{\"editor\":{\"size\":13}}\n";
    let new = "{\n  // 字号\n  \"editor\": { \"size\": 15, \"unknown\": 1 },\n}\n";
    std::fs::write(&config, old).unwrap();
    std::fs::write(&from, new).unwrap();

    let imported = import_config(&from, &config).unwrap();
    assert_eq!(imported.config.editor_size, Some(15.0));
    assert_eq!(imported.problems.len(), 1, "{:?}", imported.problems);
    assert_eq!(std::fs::read_to_string(&config).unwrap(), new);
    let backup = imported.backup.unwrap();
    assert_eq!(backup, dir.join("config.json.bak"));
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), old);

    // 原来没有配置文件时不另存。
    std::fs::remove_file(&config).unwrap();
    std::fs::remove_file(&backup).unwrap();
    assert_eq!(import_config(&from, &config).unwrap().backup, None);
    assert!(!backup.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_a_json_object_is_not_imported() {
    let dir = scratch("invalid");
    let config = dir.join("config.json");
    let from = dir.join("settings.json");
    let old = "{\"editor\":{\"size\":13}}\n";
    std::fs::write(&config, old).unwrap();
    for content in [&b"not json"[..], b"[1, 2]", b"{\"editor\": \xff}"] {
        std::fs::write(&from, content).unwrap();
        let error = import_config(&from, &config).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(&config).unwrap(), old);
        assert!(!dir.join("config.json.bak").exists());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
