//! Windows 资源管理器里的文件图标：一种扩展名一条色带，样式参照 Notepads 的文件图标。
//!
//! 应用图标单独占资源编号 1。这里的每一项紧随其后，编号连续，外壳用 `exe,-编号` 引用。
//!
//! 构建脚本和程序各编译一次：构建脚本只拿清单去画图标，查扩展名和打开命令的函数在那边用不到。

#![allow(dead_code)]

/// 应用图标的资源编号。窗口、任务栏和「打开方式」都用它。
pub const APP_ICON_ID: u16 = 1;

/// 一种文件图标：色带上的字、色带颜色（`0xRR_GG_BB`）、共用这张图标的扩展名（不带点、小写）。
pub struct FileIcon {
    /// 色带上的字。空字符串表示一张只有横线的通用纸。
    pub label: &'static str,
    /// 色带颜色。
    pub color: u32,
    /// 使用这张图标的扩展名。
    pub extensions: &'static [&'static str],
}

/// 全部文件图标，顺序就是资源编号：第一项是 [`APP_ICON_ID`] 的下一个。
pub const FILE_ICONS: &[FileIcon] = &[
    FileIcon {
        label: "MD",
        color: 0x44_70_9E,
        extensions: &["md", "markdown", "mdx", "mkd", "mdown"],
    },
    FileIcon {
        label: "TXT",
        color: 0x5F_65_70,
        extensions: &["txt", "text", "log", "rst", "adoc"],
    },
    FileIcon {
        label: "JSON",
        color: 0x93_70_0F,
        extensions: &["json", "jsonc", "json5", "jsonl"],
    },
    FileIcon {
        label: "YML",
        color: 0xB2_3A_3A,
        extensions: &["yml", "yaml"],
    },
    FileIcon {
        label: "TOML",
        color: 0x9A_5A_25,
        extensions: &["toml"],
    },
    FileIcon {
        label: "XML",
        color: 0xB0_4F_24,
        extensions: &["xml", "svg", "xsl"],
    },
    FileIcon {
        label: "HTML",
        color: 0xB0_4F_24,
        extensions: &["html", "htm", "xhtml"],
    },
    FileIcon {
        label: "CSS",
        color: 0x2F_68_A8,
        extensions: &["css", "scss", "less", "sass"],
    },
    FileIcon {
        label: "JS",
        color: 0x9A_7A_0E,
        extensions: &["js", "mjs", "cjs", "jsx"],
    },
    FileIcon {
        label: "TS",
        color: 0x2D_62_B0,
        extensions: &["ts", "tsx", "mts", "cts"],
    },
    FileIcon {
        label: "PY",
        color: 0x2F_6F_B0,
        extensions: &["py", "pyi", "pyw"],
    },
    FileIcon {
        label: "RS",
        color: 0xB4_55_1E,
        extensions: &["rs"],
    },
    FileIcon {
        label: "GO",
        color: 0x1C_7F_99,
        extensions: &["go"],
    },
    FileIcon {
        label: "C",
        color: 0x4A_65_A0,
        extensions: &["c", "h"],
    },
    FileIcon {
        label: "C++",
        color: 0x4A_65_A0,
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
    },
    FileIcon {
        label: "C#",
        color: 0x5A_47_B0,
        extensions: &["cs"],
    },
    FileIcon {
        label: "JAVA",
        color: 0xA9_52_1D,
        extensions: &["java"],
    },
    FileIcon {
        label: "KT",
        color: 0x72_46_B5,
        extensions: &["kt", "kts"],
    },
    FileIcon {
        label: "RB",
        color: 0xB2_3A_3A,
        extensions: &["rb"],
    },
    FileIcon {
        label: "PHP",
        color: 0x4D_5A_A0,
        extensions: &["php"],
    },
    FileIcon {
        label: "LUA",
        color: 0x2A_4F_B0,
        extensions: &["lua"],
    },
    FileIcon {
        label: "SH",
        color: 0x3D_7F_35,
        extensions: &["sh", "bash", "zsh", "ps1", "bat", "cmd"],
    },
    FileIcon {
        label: "SQL",
        color: 0x93_70_0F,
        extensions: &["sql"],
    },
    FileIcon {
        label: "CSV",
        color: 0x2E_85_40,
        extensions: &["csv", "tsv"],
    },
    FileIcon {
        label: "CFG",
        color: 0x5F_65_70,
        extensions: &["ini", "cfg", "conf", "config"],
    },
    FileIcon {
        label: "GIT",
        color: 0xC4_44_2E,
        extensions: &["gitignore", "gitattributes", "gitmodules"],
    },
    FileIcon {
        label: "ENV",
        color: 0x93_70_0F,
        extensions: &["env"],
    },
    FileIcon {
        label: "",
        color: 0,
        extensions: &[],
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
    std::path::Path::new(path)
        .file_name()
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
