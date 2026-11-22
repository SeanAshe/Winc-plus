//! 右键菜单和文件类型在注册表中的登记：写哪些项、怎样判断开着，不依赖具体的注册表。

use std::path::Path;

/// 安装程序登记的、Winc 能打开的文件类型（扩展名，写法同 `installer.nsi` 的
/// `ForEachFileType`，测试核对两处一致）。
pub(super) const FILE_TYPES: &[&str] = &[
    "ASH",
    "ASTRO",
    "ATOM",
    "BASH",
    "BB",
    "BZL",
    "C",
    "CC",
    "CFG",
    "CJS",
    "CLJ",
    "CLJC",
    "CLJS",
    "CLS",
    "CMAKE",
    "CONF",
    "CPP",
    "CPPM",
    "CS",
    "CSPROJ",
    "CSS",
    "CSV",
    "CSX",
    "CTS",
    "CU",
    "CUH",
    "CXX",
    "DART",
    "DESKTOP",
    "DIFF",
    "DOCKERFILE",
    "DTX",
    "EDN",
    "ENV",
    "EPUB",
    "ERL",
    "ESCRIPT",
    "ETS",
    "EX",
    "EXS",
    "FISH",
    "FS",
    "FSI",
    "FSPROJ",
    "FSX",
    "GEMSPEC",
    "GEOJSON",
    "GO",
    "GQL",
    "GRADLE",
    "GRAPHQL",
    "GRAPHQLS",
    "GROOVY",
    "H",
    "HCL",
    "HH",
    "HPP",
    "HRL",
    "HS",
    "HSC",
    "HTM",
    "HTML",
    "HXX",
    "INF",
    "INI",
    "INL",
    "INO",
    "IPP",
    "IXX",
    "JAVA",
    "JBUILDER",
    "JL",
    "JS",
    "JSON",
    "JSON5",
    "JSONC",
    "JSONL",
    "JSX",
    "KML",
    "KSH",
    "KT",
    "KTS",
    "LESS",
    "LOG",
    "LTX",
    "LUA",
    "LUAU",
    "M",
    "MARKDOWN",
    "MD",
    "MDOWN",
    "MJS",
    "MK",
    "MKD",
    "MKDN",
    "ML",
    "MLI",
    "MLL",
    "MLY",
    "MM",
    "MTS",
    "NDJSON",
    "NIX",
    "NOMAD",
    "NUSPEC",
    "PATCH",
    "PDF",
    "PHP",
    "PHP3",
    "PHP4",
    "PHP5",
    "PHP8",
    "PHPS",
    "PHTML",
    "PL",
    "PLIST",
    "PM",
    "POD",
    "PODSPEC",
    "PROPERTIES",
    "PROPS",
    "PROTO",
    "PS1",
    "PSD1",
    "PSGI",
    "PSM1",
    "PY",
    "PYI",
    "R",
    "RAKE",
    "RB",
    "RBI",
    "RESX",
    "ROCKSPEC",
    "RPROFILE",
    "RS",
    "RSS",
    "SASS",
    "SC",
    "SCALA",
    "SCSS",
    "SERVICE",
    "SH",
    "SQL",
    "STAR",
    "STORYBOARD",
    "STY",
    "SVELTE",
    "SVG",
    "SWIFT",
    "T",
    "TAB",
    "TARGETS",
    "TEX",
    "TF",
    "TFTEST",
    "TFVARS",
    "TOML",
    "TPP",
    "TS",
    "TSV",
    "TSX",
    "TXT",
    "VBPROJ",
    "VCXPROJ",
    "VUE",
    "WEBMANIFEST",
    "WSDL",
    "WXML",
    "WXS",
    "WXSS",
    "XAML",
    "XCSCHEME",
    "XHTML",
    "XIB",
    "XML",
    "XSD",
    "XSL",
    "XSLT",
    "YAML",
    "YML",
    "ZIG",
    "ZON",
    "ZSH",
];

/// 右键菜单所在的各类：所有文件、文件夹、文件夹里的空白处。
pub(super) const MENU_CLASSES: [&str; 3] = ["*", "Directory", r"Directory\Background"];

/// 安装程序记下用户选择的注册表项，静默升级时沿用。
pub(super) const CHOICES: &str = r"Software\DEEIX\winc";

/// 返回类 `class` 下右键菜单的注册表项。
pub(super) fn menu_key(class: &str) -> String {
    format!(r"Software\Classes\{class}\shell\winc")
}

/// 返回类型 `kind` 的候选程序列表所在的注册表项，以及 Winc 在其中的名字。
pub(super) fn listing(kind: &str) -> (String, String) {
    let key = format!(r"Software\Classes\.{kind}\OpenWithProgids");
    (key, format!("winc.{kind}"))
}

/// 读写注册表的接口：`machine` 为真时读全机之下的项，其余都在当前用户之下。
pub(super) trait Registry {
    /// 是否有注册表项 `key`。
    fn has_key(&self, machine: bool, key: &str) -> bool;
    /// 当前用户之下的 `key` 是否有名为 `name` 的值。
    fn has_value(&self, key: &str, name: &str) -> bool;
    /// 把 `key` 下名为 `name` 的值写成字符串 `data`，项不存在时创建；`name` 为空表示默认值。
    fn set_text(&mut self, key: &str, name: &str, data: &str);
    /// 把 `key` 下名为 `name` 的值写成整数 `data`，项不存在时不创建。
    fn set_number(&mut self, key: &str, name: &str, data: u32);
    /// 删除 `key` 下名为 `name` 的值。
    fn delete_value(&mut self, key: &str, name: &str);
    /// 删除 `key` 和它的全部子项。
    fn delete_tree(&mut self, key: &str);
    /// `key` 没有值也没有子项时删除它。
    fn delete_if_empty(&mut self, key: &str);
}

/// 右键菜单上显示的文字和要运行的程序。
pub(super) struct Menu<'a> {
    /// 程序的路径。
    pub exe: &'a Path,
    /// 文件上的那一项的文字。
    pub file: &'a str,
    /// 文件夹上的那一项的文字。
    pub folder: &'a str,
}

/// 返回右键菜单上文件和文件夹两项的文字，同安装程序的 `OpenFileVerb`、`OpenFolderVerb`：
/// `chinese` 为系统的语言是否是中文，安装程序也按系统的语言选。
pub(super) fn menu_labels(chinese: bool) -> (&'static str, &'static str) {
    if chinese {
        ("通过 Winc 打开", "在 Winc 中打开")
    } else {
        ("Open with Winc", "Open in Winc")
    }
}

/// 右键菜单是否显示：当前用户或全机之下登记了它，并且没有被盖住。
pub(super) fn menu_on(registry: &impl Registry) -> bool {
    let key = menu_key(MENU_CLASSES[0]);
    let command = format!(r"{key}\command");
    let added = registry.has_key(false, &command) || registry.has_key(true, &command);
    added && !registry.has_value(&key, "LegacyDisable")
}

/// 打开或关闭右键菜单，`record` 为真时记下这一选择（安装程序静默升级时沿用）。
///
/// 全机之下已经登记时不重复登记，关闭时只盖住它。
pub(super) fn set_menu(registry: &mut impl Registry, menu: Option<&Menu<'_>>, record: bool) {
    let machine = registry.has_key(true, &format!(r"{}\command", menu_key(MENU_CLASSES[0])));
    for class in MENU_CLASSES {
        let key = menu_key(class);
        match menu {
            Some(_) if machine => registry.delete_value(&key, "LegacyDisable"),
            Some(menu) => {
                let (label, target) = match class {
                    "*" => (menu.file, "%1"),
                    _ => (menu.folder, "%V"),
                };
                let exe = menu.exe.display();
                registry.delete_value(&key, "LegacyDisable");
                registry.set_text(&key, "", label);
                registry.set_text(&key, "Icon", &format!("{exe},0"));
                let command = format!(r#""{exe}" "{target}""#);
                registry.set_text(&format!(r"{key}\command"), "", &command);
            }
            None if machine => registry.set_text(&key, "LegacyDisable", ""),
            None => registry.delete_tree(&key),
        }
    }
    if record {
        registry.set_number(CHOICES, "ContextMenu", u32::from(menu.is_some()));
    }
}

/// Winc 是否列在各文件类型的候选程序中。
pub(super) fn types_on(registry: &impl Registry) -> bool {
    FILE_TYPES.iter().any(|kind| {
        let (key, name) = listing(kind);
        registry.has_value(&key, &name)
    })
}

/// 把 Winc 列入各文件类型的候选程序，或从中去掉，并记下这一选择。
pub(super) fn set_types(registry: &mut impl Registry, on: bool) {
    for kind in FILE_TYPES {
        let (key, name) = listing(kind);
        if on {
            registry.set_text(&key, &name, "");
        } else {
            registry.delete_value(&key, &name);
            registry.delete_if_empty(&key);
            registry.delete_if_empty(&format!(r"Software\Classes\.{kind}"));
        }
    }
    registry.set_number(CHOICES, "FileTypes", u32::from(on));
}
