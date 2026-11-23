//! 右键菜单和文件类型登记的测试：用内存里的注册表代替系统的。

use std::collections::BTreeMap;
use std::path::Path;

use super::registry::*;

/// 内存里的注册表：（是否全机之下，注册表项）到各个值；数字写成十进制的文字。
#[derive(Default)]
struct Fake(BTreeMap<(bool, String), BTreeMap<String, String>>);

impl Fake {
    /// 返回当前用户之下 `key` 中名为 `name` 的值。
    fn value(&self, key: &str, name: &str) -> Option<&str> {
        let values = self.0.get(&(false, key.to_owned()))?;
        values.get(name).map(String::as_str)
    }

    /// 在全机之下登记右键菜单，如同 MSI 安装的那样。
    fn with_machine_menu(mut self) -> Fake {
        for class in MENU_CLASSES {
            let key = format!(r"{}\command", menu_key(class));
            self.0.entry((true, key)).or_default();
        }
        self
    }
}

impl Registry for Fake {
    fn has_key(&self, machine: bool, key: &str) -> bool {
        self.0.contains_key(&(machine, key.to_owned()))
    }

    fn has_value(&self, key: &str, name: &str) -> bool {
        self.value(key, name).is_some()
    }

    fn set_text(&mut self, key: &str, name: &str, data: &str) {
        let values = self.0.entry((false, key.to_owned())).or_default();
        values.insert(name.to_owned(), data.to_owned());
    }

    fn set_number(&mut self, key: &str, name: &str, data: u32) {
        if let Some(values) = self.0.get_mut(&(false, key.to_owned())) {
            values.insert(name.to_owned(), data.to_string());
        }
    }

    fn delete_value(&mut self, key: &str, name: &str) {
        if let Some(values) = self.0.get_mut(&(false, key.to_owned())) {
            values.remove(name);
        }
    }

    fn delete_tree(&mut self, key: &str) {
        let under = format!(r"{key}\");
        self.0
            .retain(|(machine, k), _| *machine || !(k == key || k.starts_with(&under)));
    }

    fn delete_if_empty(&mut self, key: &str) {
        let under = format!(r"{key}\");
        let children = self.0.keys().any(|(m, k)| !*m && k.starts_with(&under));
        let entry = (false, key.to_owned());
        if !children && self.0.get(&entry).is_some_and(BTreeMap::is_empty) {
            self.0.remove(&entry);
        }
    }
}

/// 测试用的菜单：程序在带空格的目录里，文字是中文的。
fn menu() -> Menu<'static> {
    let (file, folder) = menu_labels(true);
    Menu {
        exe: Path::new(r"C:\Program Files\Winc\winc.exe"),
        file,
        folder,
    }
}

#[test]
fn the_menu_text_is_the_installers() {
    let script = include_str!("../../../../release/windows/installer.nsi");
    let text = |name: &str, language: &str| {
        let start = format!("LangString {name} ${{{language}}} \"");
        let from = script.find(&start).unwrap() + start.len();
        &script[from..from + script[from..].find('"').unwrap()]
    };
    for (chinese, language) in [(true, "LANG_SIMPCHINESE"), (false, "LANG_ENGLISH")] {
        let labels = (
            text("OpenFileVerb", language),
            text("OpenFolderVerb", language),
        );
        assert_eq!(menu_labels(chinese), labels);
    }
}

#[test]
fn the_file_types_are_those_the_installer_registers() {
    let script = include_str!("../../../../release/windows/installer.nsi");
    let from = script.find("!macro ForEachFileType Action").unwrap();
    let list = &script[from..from + script[from..].find("!macroend").unwrap()];
    let listed: Vec<&str> = (list.lines())
        .filter_map(|line| line.trim().strip_prefix("!insertmacro ${Action} "))
        .filter_map(|rest| rest.split_whitespace().next())
        .collect();
    assert_eq!(listed, FILE_TYPES);
}

/// 回归：清单只列了 Winc 能着色的类型的一部分，LaTeX 等类型不出现在候选程序里。
#[test]
fn every_highlighted_type_is_registered() {
    // 双击即运行的类型不登记，带符号的扩展名（`c++`）也不登记。
    let runnable = ["bat", "cmd", "pyw"];
    for language in winc_syntax::languages() {
        for kind in language.extensions {
            let plain = kind.chars().all(|c| c.is_ascii_alphanumeric());
            if plain && !runnable.contains(kind) {
                let name = kind.to_uppercase();
                assert!(FILE_TYPES.contains(&name.as_str()), "{kind}");
            }
        }
    }
}

#[test]
fn the_menu_is_one_entry_for_files_and_one_for_folders() {
    let mut registry = Fake::default();
    registry.0.entry((false, CHOICES.to_owned())).or_default();
    assert!(!menu_on(&registry));

    set_menu(&mut registry, Some(&menu()), true);
    assert!(menu_on(&registry));
    let files = r"Software\Classes\*\shell\winc";
    assert_eq!(registry.value(files, ""), Some("通过 Winc 打开"));
    assert_eq!(
        registry.value(files, "Icon"),
        Some(r"C:\Program Files\Winc\winc.exe,0")
    );
    let command = registry.value(&format!(r"{files}\command"), "");
    assert_eq!(command, Some(r#""C:\Program Files\Winc\winc.exe" "%1""#));
    for class in ["Directory", r"Directory\Background"] {
        let key = menu_key(class);
        assert_eq!(registry.value(&key, ""), Some("在 Winc 中打开"));
        let command = registry.value(&format!(r"{key}\command"), "");
        assert_eq!(command, Some(r#""C:\Program Files\Winc\winc.exe" "%V""#));
    }
    // 不按类型逐个登记。
    assert!(
        registry
            .0
            .keys()
            .all(|(_, key)| !key.contains("SystemFileAssociations"))
    );
    assert_eq!(registry.value(CHOICES, "ContextMenu"), Some("1"));

    // 关闭：三项连同子项都去掉，选择记为关。
    set_menu(&mut registry, None, true);
    assert!(!menu_on(&registry));
    let left: Vec<_> = registry.0.keys().map(|(_, key)| key.as_str()).collect();
    assert_eq!(left, [CHOICES]);
    assert_eq!(registry.value(CHOICES, "ContextMenu"), Some("0"));
}

#[test]
fn a_portable_copy_leaves_no_record_of_the_choice() {
    let mut registry = Fake::default();
    set_menu(&mut registry, Some(&menu()), false);
    assert!(menu_on(&registry));
    assert!(!registry.has_key(false, CHOICES));
}

#[test]
fn a_machine_wide_menu_is_hidden_rather_than_removed() {
    let mut registry = Fake::default().with_machine_menu();
    assert!(menu_on(&registry));

    // 全机之下的登记改不了：在当前用户之下遮住它，不另写一份。
    set_menu(&mut registry, None, false);
    assert!(!menu_on(&registry));
    for class in MENU_CLASSES {
        let key = menu_key(class);
        assert_eq!(registry.value(&key, "LegacyDisable"), Some(""));
        assert!(registry.has_key(true, &format!(r"{key}\command")));
        assert!(!registry.has_key(false, &format!(r"{key}\command")));
    }

    set_menu(&mut registry, Some(&menu()), false);
    assert!(menu_on(&registry));
    for class in MENU_CLASSES {
        let key = menu_key(class);
        assert_eq!(registry.value(&key, "LegacyDisable"), None);
        assert_eq!(registry.value(&key, ""), None);
    }
}

#[test]
fn listing_the_file_types_keeps_other_programs_entries() {
    let mut registry = Fake::default();
    registry.0.entry((false, CHOICES.to_owned())).or_default();
    let json = r"Software\Classes\.JSON\OpenWithProgids";
    registry.set_text(json, "other.json", "");
    assert!(!types_on(&registry));

    set_types(&mut registry, true);
    assert!(types_on(&registry));
    for kind in FILE_TYPES {
        let (key, name) = listing(kind);
        assert_eq!(registry.value(&key, &name), Some(""), "{kind}");
    }
    assert_eq!(registry.value(CHOICES, "FileTypes"), Some("1"));

    // 去掉：只删 Winc 的那一项，别的程序的留着；空了的项一并去掉。
    set_types(&mut registry, false);
    assert!(!types_on(&registry));
    assert_eq!(registry.value(json, "other.json"), Some(""));
    assert_eq!(registry.value(json, "winc.JSON"), None);
    assert!(!registry.has_key(false, r"Software\Classes\.MD\OpenWithProgids"));
    assert!(!registry.has_key(false, r"Software\Classes\.MD"));
    // 记为关：安装程序默认会列入，升级时据此保持关着。
    assert_eq!(registry.value(CHOICES, "FileTypes"), Some("0"));
}

#[test]
fn the_settings_list_the_group_only_on_windows() {
    use crate::layout::{Setting, SettingControl, SettingsPage};

    let sections = SettingsPage::General.sections(&Default::default());
    let listed = sections
        .iter()
        .any(|s| s.items.contains(&Setting::ExplorerMenu));
    assert_eq!(listed, cfg!(windows));
    // 记在注册表里，不是配置文件中的项；同组各项都有说明。
    for setting in [Setting::ExplorerMenu, Setting::FileAssociations] {
        assert_eq!(setting.page(), SettingsPage::General);
        assert_eq!(setting.control(), SettingControl::Switch);
        assert_eq!(setting.key(), None);
        assert!(setting.description().is_some());
    }
}
