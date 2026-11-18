//! 导入快捷键的测试：IDEA 的自定方案和继承链、VS Code 的 `keybindings.json`、本机来源的查找。

use super::*;

/// 在系统临时目录下建一个空的测试目录。
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn keys(custom: &Custom, action: Action) -> Option<Vec<String>> {
    let keys = custom.keys(action)?;
    Some(
        keys.iter()
            .map(|&(a, b)| binding_spec(a, b, Os::Mac))
            .collect(),
    )
}

/// 自定方案沿继承链合入：下层改过的操作取代上层的，没有键的操作去掉全部键，认不出的操作跳过。
#[test]
fn idea_keymaps_follow_their_parents() {
    let dir = scratch("idea");
    write(
        &dir.join("options/mac/keymap.xml"),
        r#"<component name="KeymapManager"><active_keymap name="Mine" /></component>"#,
    );
    write(
        &dir.join("keymaps/Base.xml"),
        r#"<keymap version="1" name="Base" parent="Mac OS X 10.5+">
  <action id="GotoFile"><keyboard-shortcut first-keystroke="meta P" /></action>
  <action id="ReformatCode"><keyboard-shortcut first-keystroke="meta shift L" /></action>
</keymap>"#,
    );
    write(
        &dir.join("keymaps/Mine.xml"),
        r#"<keymap version="1" name="Mine" parent="Base">
  <action id="ReformatCode"><keyboard-shortcut first-keystroke="control alt F" /></action>
  <action id="EditorDuplicate" />
  <action id="SomePluginAction"><keyboard-shortcut first-keystroke="meta Y" /></action>
  <action id="FindUsages"><keyboard-shortcut first-keystroke="meta U" /></action>
  <action id="ShowUsages"><keyboard-shortcut first-keystroke="meta shift U" /></action>
  <action id="ShowSettings">
    <keyboard-shortcut first-keystroke="meta K" second-keystroke="meta OPEN_BRACKET" />
  </action>
</keymap>"#,
    );
    let imported = idea::read(&dir, Os::Mac).unwrap();
    assert_eq!(imported.preset, Preset::Idea);
    assert_eq!(imported.skipped, 1);
    let custom = &imported.custom;
    assert_eq!(keys(custom, Action::QuickOpen).unwrap(), ["cmd+p"]);
    assert_eq!(keys(custom, Action::Format).unwrap(), ["ctrl+alt+f"]);
    assert_eq!(
        keys(custom, Action::CopyLinesDown).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(keys(custom, Action::Settings).unwrap(), ["cmd+k cmd+["]);
    // 对应同一个命令的几个操作，键合在一起。
    assert_eq!(
        keys(custom, Action::FindReferences).unwrap(),
        ["cmd+u", "shift+cmd+u"]
    );

    // 当前方案是内置方案：只换方案；内置的其他编辑器方案不支持。
    let default = scratch("idea-default");
    write(
        &default.join("options/keymap.xml"),
        r#"<component name="KeymapManager"><active_keymap name="VSCode OSX"/></component>"#,
    );
    let imported = idea::read(&default, Os::Mac).unwrap();
    assert_eq!(imported.preset, Preset::VsCode);
    assert!(imported.custom.is_empty());
    write(
        &default.join("options/keymap.xml"),
        r#"<component name="KeymapManager"><active_keymap name="Emacs"/></component>"#,
    );
    assert!(idea::read(&default, Os::Mac).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&default);
}

/// `keybindings.json` 在 VS Code 方案的键上增加、去掉键；允许注释和末尾的逗号。
#[test]
fn vscode_keybindings_add_and_remove_keys() {
    let dir = scratch("vscode");
    write(
        &dir.join("keybindings.json"),
        r#"// 用户的快捷键
[
  { "key": "cmd+e", "command": "workbench.action.quickOpen" },
  { "key": "cmd+p", "command": "-workbench.action.quickOpen" },
  { "command": "-editor.action.formatDocument" },
  { "key": "cmd+k cmd+u", "command": "editor.action.commentLine", "when": "editorTextFocus" },
  { "key": "cmd+shift+x", "command": "someExtension.command" },
  { "key": "[KeyA]", "command": "workbench.action.gotoLine" },
  { "key": "cmd+j", "command": "workbench.action.navigateBack" },
  { "key": "cmd+j", "command": "-workbench.action.navigateBack" },
]"#,
    );
    let imported = vscode::read(&dir, Os::Mac).unwrap();
    assert_eq!(imported.preset, Preset::VsCode);
    assert_eq!(imported.skipped, 2);
    let custom = &imported.custom;
    assert_eq!(keys(custom, Action::QuickOpen).unwrap(), ["cmd+e"]);
    assert_eq!(keys(custom, Action::Format).unwrap(), Vec::<String>::new());
    assert_eq!(
        keys(custom, Action::ToggleComment).unwrap(),
        ["cmd+/", "cmd+k cmd+u"]
    );
    assert_eq!(keys(custom, Action::GoToLine), None);
    // 增删之后与方案相同的命令不算改过。
    assert_eq!(keys(custom, Action::GoBack), None);

    // 没有 `keybindings.json`：只换用 VS Code 方案；格式不对时说明原因。
    let empty = scratch("vscode-empty");
    let imported = vscode::read(&empty, Os::Mac).unwrap();
    assert!(imported.preset == Preset::VsCode && imported.custom.is_empty());
    write(&empty.join("keybindings.json"), "{ \"key\": 1 }");
    assert!(vscode::read(&empty, Os::Mac).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&empty);
}

/// 每个 IDEA 系列产品只取最新的版本，前缀相近的产品分得开。
#[test]
fn idea_sources_keep_the_newest_version_of_each_product() {
    let root = scratch("sources");
    for name in [
        "IntelliJIdea2024.3",
        "IntelliJIdea2025.1",
        "IntelliJIdea2024.10",
        "PyCharm2025.2",
        "PyCharmCE2025.3",
        "IntelliJIdea",
        "consentOptions",
    ] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    let names: Vec<String> = idea_sources(&root).into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "IntelliJ IDEA 2025.1",
            "PyCharm 2025.2",
            "PyCharm CE 2025.3"
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}
