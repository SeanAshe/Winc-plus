//! `sparkle` 模块的测试：`键=值` 文本的解析、环境变量优先，以及 `Info.plist` 中的更新键。

use super::*;

const TEXT: &str = r#"
# 注释
WINC_UPDATE_CHANNELS="global=https://dl.example.com/winc/ cn=https://dl.example.cn/winc"
WINC_UPDATE_PUBLIC_KEY="abc+/="
"#;

// `TEXT` 中的地址前缀以 `/` 结尾，解析后必须去掉；两个通道的顺序保持不变。
#[test]
fn channels_and_key_come_from_the_file_unless_the_environment_sets_them() {
    let config = UpdateConfig::parse(TEXT, |_| None);
    assert_eq!(
        config.channels,
        [
            (
                "global".to_owned(),
                "https://dl.example.com/winc".to_owned()
            ),
            ("cn".to_owned(), "https://dl.example.cn/winc".to_owned()),
        ]
    );
    assert_eq!(config.public_key, "abc+/=");
    assert!(config.enabled());

    let env =
        |key: &str| (key == "WINC_UPDATE_CHANNELS").then(|| "a=http://127.0.0.1:1".to_owned());
    // 环境变量只提供通道，公钥仍然取自文本。
    let config = UpdateConfig::parse(TEXT, env);
    assert_eq!(
        config.channels,
        [("a".to_owned(), "http://127.0.0.1:1".to_owned())]
    );
    assert_eq!(config.public_key, "abc+/=");
}

#[cfg(unix)]
#[test]
fn framework_is_intact_only_when_its_layout_symlinks_remain() {
    let root = std::env::temp_dir().join(format!("winc-sparkle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let framework = root.join("Sparkle.framework");
    std::fs::create_dir_all(framework.join("Versions/B")).unwrap();
    std::os::unix::fs::symlink("B", framework.join("Versions/Current")).unwrap();
    std::os::unix::fs::symlink("Versions/Current/Sparkle", framework.join("Sparkle")).unwrap();
    assert!(framework_intact(&root));

    std::fs::remove_file(framework.join("Sparkle")).unwrap();
    std::fs::write(framework.join("Sparkle"), "not a symlink").unwrap();
    assert!(!framework_intact(&root));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn info_plist_gets_update_keys_only_when_configured() {
    let empty = UpdateConfig::parse("WINC_UPDATE_CHANNELS=\"\"\n", |_| None);
    assert!(!empty.enabled());
    assert_eq!(empty.plist_entries("arm64"), "");
    // 只有通道、没有公钥也不启用：Sparkle 不安装无法校验的更新。
    let no_key = UpdateConfig::parse("WINC_UPDATE_CHANNELS=\"a=https://x\"\n", |_| None);
    assert_eq!(no_key.plist_entries("arm64"), "");

    let entries = UpdateConfig::parse(TEXT, |_| None).plist_entries("arm64");
    // 每个通道写一条 `WincUpdateFeeds`，地址带本机架构后缀。
    for want in [
        "<string>https://dl.example.com/winc/appcast-macos-arm64.xml</string>",
        "<string>https://dl.example.cn/winc/appcast-macos-arm64.xml</string>",
        "<key>SUPublicEDKey</key>\n\t<string>abc+/=</string>",
        "<key>SURequireSignedFeed</key>\n\t<true/>",
        "<key>SUEnableAutomaticChecks</key>\n\t<false/>",
    ] {
        assert!(entries.contains(want), "{want}\n{entries}");
    }
    // 地址里的 `&` 要转义，否则 Info.plist 不是合法的 XML。
    let amp = UpdateConfig::parse(
        "WINC_UPDATE_CHANNELS=\"a=https://x/?a=1&b=2\"\nWINC_UPDATE_PUBLIC_KEY=\"k\"\n",
        |_| None,
    );
    assert!(
        amp.plist_entries("arm64")
            .contains("a=1&amp;b=2/appcast-macos-arm64.xml")
    );
}
