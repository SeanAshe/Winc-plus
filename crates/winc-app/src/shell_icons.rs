//! Windows 资源管理器里的文件图标。
//!
//! 图像直接用 [Notepads](https://github.com/0x7c13/Notepads) 的 `Assets/FileIcons`：
//! 16、32、48、512 各一张，不把同一张图缩小到每个尺寸。应用图标单独占资源编号 1。
//! 这里的每一项紧随其后，编号连续，外壳用 `exe,-编号` 引用。
//!
//! 构建脚本和程序各编译一次：构建脚本只拿清单去找图标文件，查扩展名和打开命令的函数在那边用不到。

#![allow(dead_code)]

/// 应用图标的资源编号。窗口、任务栏和「打开方式」都用它。
pub const APP_ICON_ID: u16 = 1;

/// 一种文件图标：属性窗口里的类型名、Notepads 图标名、共用这张图标的扩展名（不带点、小写）。
pub struct FileIcon {
    /// 属性窗口里的类型名。空字符串时用扩展名本身。
    pub label: &'static str,
    /// `assets/file-icons` 里的图标名，对应 Notepads 的 `FileIcons/<asset>.targetsize-*.png`。
    pub asset: &'static str,
    /// 使用这张图标的扩展名。
    pub extensions: &'static [&'static str],
}

/// 全部文件图标，顺序就是资源编号：第一项是 [`APP_ICON_ID`] 的下一个。
pub const FILE_ICONS: &[FileIcon] = &[
    FileIcon {
        label: "MD",
        asset: "md",
        extensions: &["md", "markdown", "mdx", "mkd", "mdown"],
    },
    FileIcon {
        label: "TXT",
        asset: "txt",
        extensions: &["txt", "text", "rst", "adoc"],
    },
    FileIcon {
        label: "LOG",
        asset: "log",
        extensions: &["log"],
    },
    FileIcon {
        label: "JSON",
        asset: "json",
        extensions: &["json", "jsonc", "json5", "jsonl"],
    },
    FileIcon {
        label: "YML",
        asset: "yml",
        extensions: &["yml", "yaml"],
    },
    FileIcon {
        label: "XML",
        asset: "xml",
        extensions: &["xml", "svg", "xsl"],
    },
    FileIcon {
        label: "HTML",
        asset: "html",
        extensions: &["html", "htm", "xhtml"],
    },
    FileIcon {
        label: "CSS",
        asset: "css",
        extensions: &["css", "scss", "less", "sass"],
    },
    FileIcon {
        label: "JS",
        asset: "js",
        extensions: &["js", "mjs", "cjs", "jsx"],
    },
    FileIcon {
        label: "PY",
        asset: "py",
        extensions: &["py", "pyi", "pyw"],
    },
    FileIcon {
        label: "C",
        asset: "c",
        extensions: &["c"],
    },
    FileIcon {
        label: "H",
        asset: "h",
        extensions: &["h", "hh", "hpp", "hxx"],
    },
    FileIcon {
        label: "C++",
        asset: "cpp",
        extensions: &["cpp", "cc", "cxx"],
    },
    FileIcon {
        label: "C#",
        asset: "cs",
        extensions: &["cs"],
    },
    FileIcon {
        label: "JAVA",
        asset: "java",
        extensions: &["java"],
    },
    FileIcon {
        label: "RB",
        asset: "rb",
        extensions: &["rb"],
    },
    FileIcon {
        label: "PHP",
        asset: "php",
        extensions: &["php"],
    },
    FileIcon {
        label: "SH",
        asset: "sh",
        // `.bat` 不在这里：它由系统的 `batfile` 直接执行，登记成打开方式以后双击就进编辑器。
        extensions: &["sh", "zsh", "ps1", "cmd"],
    },
    FileIcon {
        label: "BASH",
        asset: "bash",
        extensions: &["bash"],
    },
    FileIcon {
        label: "SQL",
        asset: "sql",
        extensions: &["sql"],
    },
    FileIcon {
        label: "INI",
        asset: "ini",
        extensions: &["ini"],
    },
    FileIcon {
        label: "CFG",
        asset: "cfg",
        extensions: &["cfg", "conf", "config"],
    },
    // Notepads 没有单独图标的类型用它的通用纸。必须放在最后：认不出的扩展名也用这一张。
    FileIcon {
        label: "",
        asset: "file",
        extensions: &[
            "toml",
            "ts",
            "tsx",
            "mts",
            "cts",
            "rs",
            "go",
            "kt",
            "kts",
            "lua",
            "csv",
            "tsv",
            "gitignore",
            "gitattributes",
            "gitmodules",
            "env",
        ],
    },
];

/// 扩展名 `ext`（前面可以有一个点）对应的图标资源编号。认不出时用最后那张通用纸。
pub fn id_for_extension(ext: &str) -> u16 {
    let ext = ext.strip_prefix('.').unwrap_or(ext);
    let index = FILE_ICONS
        .iter()
        .position(|icon| {
            icon.extensions
                .iter()
                .any(|name| name.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(FILE_ICONS.len() - 1);
    APP_ICON_ID + 1 + index as u16
}

/// 安装时是否把扩展名的默认程序设成 `ours`。
///
/// 还没有默认程序，或者已经是 winc（包括这条 `ProgID` 本身）时可以登记。别的程序占着时不抢。
pub fn claims_extension(current: Option<&str>, command: Option<&str>, ours: &str) -> bool {
    match current.map(str::trim).filter(|id| !id.is_empty()) {
        None => true,
        Some(id) if id.eq_ignore_ascii_case(ours) => true,
        Some(_) => command.is_some_and(command_targets_winc),
    }
}

/// 打开命令里的可执行文件路径。
pub fn command_path(command: &str) -> Option<&str> {
    command_target(command)
}

/// `command` 是不是用 winc 打开文件的命令（`"路径\winc.exe" "%1"` 这种）。
pub fn command_targets_winc(command: &str) -> bool {
    let Some(path) = command_path(command) else {
        return false;
    };
    // 注册表里是 Windows 路径。测试也在 macOS、Linux 上跑，不能用本机的路径分隔符。
    path.rsplit(['\\', '/'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("winc.exe"))
}

/// 取出打开命令里的可执行文件路径：有引号时取第一对引号里的，否则取第一个词。
fn command_target(command: &str) -> Option<&str> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(&rest[..end]);
    }
    let token = command.split_whitespace().next()?;
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_and_json_use_different_icons_and_neither_is_the_app_icon() {
        let md = id_for_extension(".md");
        let json = id_for_extension("JSON");
        assert_ne!(md, json);
        assert_ne!(md, APP_ICON_ID);
        assert_eq!(md, id_for_extension("markdown"));
        assert_eq!(id_for_extension("no-such-type"), id_for_extension(""));
    }

    #[test]
    fn only_winc_commands_count() {
        assert!(command_targets_winc(
            r#""C:\Program Files\winc\winc.exe" "%1""#
        ));
        assert!(command_targets_winc(r"C:\winc\Winc.EXE %1"));
        assert!(!command_targets_winc(
            r#""C:\Program Files\Microsoft VS Code\Code.exe" "%1""#
        ));
        assert!(!command_targets_winc(""));
        assert!(!command_targets_winc("\""));
    }

    #[test]
    fn installation_claims_unowned_types_and_leaves_other_apps() {
        assert!(claims_extension(None, None, "Winc.md"));
        assert!(claims_extension(Some("Winc.md"), None, "Winc.md"));
        assert!(claims_extension(
            Some("md_auto_file"),
            Some(r#""C:\Program Files\winc\winc.exe" "%1""#),
            "Winc.md"
        ));
        assert!(!claims_extension(
            Some("VSCode.md"),
            Some(r#""C:\Program Files\Microsoft VS Code\Code.exe" "%1""#),
            "Winc.md"
        ));
        assert!(claims_extension(Some("  "), None, "Winc.md"));
    }
}
