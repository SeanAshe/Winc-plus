//! 生成格式化工具的安装说明：工具未安装时，告诉用户怎么安装。
//!
//! 本模块只提供命令，不代替用户运行：是否使用 `sudo`、安装位置、使用哪个包管理器都应由用户决定，
//! 安装出错时也只能由用户自己的包管理器处理。

use std::path::{Path, PathBuf};

use winc_config::Json;
use winc_i18n::t;

/// 一种安装方式：系统中有程序（包管理器）`via` 时，运行 `command`。
struct Way {
    /// 包管理器（例如 `brew`）。
    via: &'static str,
    /// 在终端里运行的安装命令。
    command: &'static str,
}

const fn way(via: &'static str, command: &'static str) -> Way {
    Way { via, command }
}

/// 一种工具的安装信息。
struct Install {
    /// 程序名，与预设命令的第一个词相同。
    program: &'static str,
    /// 安装方式，按推荐顺序排列。
    ways: &'static [Way],
    /// 返回安装方式之外的补充说明（例如「gofmt 随 Go 提供」）。
    note: Option<fn() -> &'static str>,
    /// 官方安装文档的链接，显示在说明末尾。
    url: &'static str,
    /// 在项目中安装时使用的包名和所属生态（npm 或 Python）。
    package: Option<Package>,
}

/// 在项目中安装时的包名及所属的包管理生态。
#[derive(Clone, Copy)]
enum Package {
    /// npm 生态的包名。
    Node(&'static str),
    /// Python 生态的包名。
    Python(&'static str),
}

/// 所有已知工具的安装信息，覆盖 `PRESETS` 中的全部预设（见测试
/// `every_preset_says_how_to_install_it`）。
const INSTALLS: &[Install] = &[
    Install {
        program: "rustfmt",
        ways: &[way("rustup", "rustup component add rustfmt")],
        note: Some(|| t!("format-install-note-rustfmt")),
        url: "https://rust-lang.github.io/rustup/",
        package: None,
    },
    Install {
        program: "gofmt",
        ways: &[
            way("brew", "brew install go"),
            way("winget", "winget install GoLang.Go"),
            way("apt", "sudo apt install golang-go"),
        ],
        note: Some(|| t!("format-install-note-gofmt")),
        url: "https://go.dev/doc/install",
        package: None,
    },
    Install {
        program: "prettier",
        ways: &[
            way("npm", "npm install --global prettier"),
            way("brew", "brew install prettier"),
        ],
        note: None,
        url: "https://prettier.io/docs/install",
        package: Some(Package::Node("prettier")),
    },
    Install {
        program: "biome",
        ways: &[
            way("npm", "npm install --global @biomejs/biome"),
            way("brew", "brew install biome"),
        ],
        note: None,
        url: "https://biomejs.dev/guides/manual-installation/",
        package: Some(Package::Node("@biomejs/biome")),
    },
    Install {
        program: "eslint",
        ways: &[way("npm", "npm init @eslint/config@latest")],
        note: Some(|| t!("format-install-note-eslint")),
        url: "https://eslint.org/docs/latest/use/getting-started",
        package: Some(Package::Node("eslint")),
    },
    Install {
        program: "black",
        ways: &[
            way("uv", "uv tool install black"),
            way("pipx", "pipx install black"),
            way("brew", "brew install black"),
        ],
        note: None,
        url: "https://black.readthedocs.io/en/stable/getting_started.html",
        package: Some(Package::Python("black")),
    },
    Install {
        program: "ruff",
        ways: &[
            way("uv", "uv tool install ruff"),
            way("pipx", "pipx install ruff"),
            way("brew", "brew install ruff"),
        ],
        note: None,
        url: "https://docs.astral.sh/ruff/installation/",
        package: Some(Package::Python("ruff")),
    },
    Install {
        program: "clang-format",
        ways: &[
            way("xcode-select", "xcode-select --install"),
            way("brew", "brew install clang-format"),
            way("apt", "sudo apt install clang-format"),
            way("winget", "winget install LLVM.LLVM"),
            way("pipx", "pipx install clang-format"),
        ],
        note: Some(|| t!("format-install-note-clang-format")),
        url: "https://clang.llvm.org/docs/ClangFormat.html",
        package: None,
    },
    Install {
        program: "swift-format",
        ways: &[
            way("xcode-select", "xcode-select --install"),
            way("brew", "brew install swift-format"),
        ],
        note: Some(|| t!("format-install-note-swift-format")),
        url: "https://github.com/swiftlang/swift-format",
        package: None,
    },
    Install {
        program: "swiftformat",
        ways: &[way("brew", "brew install swiftformat")],
        note: None,
        url: "https://github.com/nicklockwood/SwiftFormat",
        package: None,
    },
    Install {
        program: "taplo",
        ways: &[
            way("brew", "brew install taplo"),
            way("cargo", "cargo install taplo-cli --locked"),
            way("npm", "npm install --global @taplo/cli"),
        ],
        note: None,
        url: "https://taplo.tamasfe.dev/cli/installation/binary.html",
        package: None,
    },
    Install {
        program: "shfmt",
        ways: &[
            way("brew", "brew install shfmt"),
            way("apt", "sudo apt install shfmt"),
            way("scoop", "scoop install shfmt"),
            way("go", "go install mvdan.cc/sh/v3/cmd/shfmt@latest"),
        ],
        note: None,
        url: "https://github.com/mvdan/sh",
        package: None,
    },
    Install {
        program: "google-java-format",
        ways: &[way("brew", "brew install google-java-format")],
        note: None,
        url: "https://github.com/google/google-java-format",
        package: None,
    },
    Install {
        program: "jq",
        ways: &[
            way("brew", "brew install jq"),
            way("apt", "sudo apt install jq"),
            way("winget", "winget install jqlang.jq"),
        ],
        note: None,
        url: "https://jqlang.org/download/",
        package: None,
    },
    Install {
        program: "xmllint",
        ways: &[way("apt", "sudo apt install libxml2-utils")],
        note: Some(|| t!("format-install-note-xmllint")),
        url: "https://gitlab.gnome.org/GNOME/libxml2",
        package: None,
    },
];

/// 包管理器 `via` 是否可能存在于当前操作系统。
///
/// `apt` 只在 Linux 上，`brew` 只在 macOS 和 Linux 上，`winget`、`scoop` 只在 Windows 上，
/// `xcode-select` 只在 macOS 上，其他一律按可能存在处理。列出「其他方式」时，用本函数过滤掉
/// 其他操作系统的安装方式。`in_distro` 为真时文件位于 WSL 发行版中，工具也要装在发行版里，按
/// Linux 判断。
pub(super) fn fits_this_system(via: &str, in_distro: bool) -> bool {
    if in_distro {
        return !matches!(via, "winget" | "scoop" | "xcode-select");
    }
    match via {
        "apt" => cfg!(target_os = "linux"),
        "winget" | "scoop" => cfg!(windows),
        "brew" => cfg!(any(target_os = "macos", target_os = "linux")),
        "xcode-select" => cfg!(target_os = "macos"),
        _ => true,
    }
}

/// 是否有程序 `program` 的安装说明。
pub fn knows(program: &str) -> bool {
    INSTALLS.iter().any(|i| i.program == program)
}

/// 生成 `programs` 中工具的安装说明，写成给用户阅读的一段文本。
///
/// `programs` 只有一个工具时说明它的安装方法；有多个时表示安装其中任一个即可。`dir` 是文件
/// 所在的目录：能在项目中安装的方式（`package.json`、uv、Poetry）排在最前。`has` 判断一个
/// 程序（包管理器）是否已安装，已安装的包管理器对应的命令列在「本机可用」之下；`dir` 位于 WSL
/// 发行版中时不使用 `has`（它查询的是 Windows）。`programs` 中没有任何已知工具时返回 `None`。
pub fn guide(programs: &[&str], dir: &Path, has: impl Fn(&str) -> bool) -> Option<String> {
    let installs: Vec<&Install> = programs
        .iter()
        .filter_map(|&p| INSTALLS.iter().find(|i| i.program == p))
        .collect();
    let mut text = match installs.as_slice() {
        [] => return None,
        [one] => t!("format-install-not-found-one", tool = one.program),
        _ => {
            let tools = installs
                .iter()
                .map(|i| t!("format-install-quoted", name = i.program))
                .collect::<Vec<_>>()
                .join(t!("format-install-list-separator"));
            t!("format-install-not-found-many", tools = tools)
        }
    };
    // 说明文本显示在编辑区中，自动换行可能处于关闭状态，因此每行写得短一些。
    text.push('\n');
    text.push_str(t!("format-install-intro"));
    text.push('\n');

    let in_distro = crate::wsl::locate_here(dir).is_some();
    let several = installs.len() > 1;
    let indent = if several { "    " } else { "" };
    for install in installs {
        text.push('\n');
        if several {
            text.push_str(&format!("{}\n", install.program));
        }
        let mut section = |title: &str, lines: &[String]| {
            if lines.is_empty() {
                return;
            }
            text.push_str(&format!("{indent}{title}\n"));
            for line in lines {
                text.push_str(&format!("{indent}    {line}\n"));
            }
        };

        if let Some(project) = install.package.and_then(|p| project_install(p, dir)) {
            // 命令在发行版的终端里运行，要写发行版内的路径。
            let target = crate::wsl::locate_here(&project.dir)
                .map_or_else(|| project.dir.display().to_string(), |place| place.path);
            let cd = format!("cd {}", shell_quote(&target));
            section(project.title, &[cd, project.command]);
        }
        let (here, elsewhere): (Vec<&Way>, Vec<&Way>) = install
            .ways
            .iter()
            .filter(|w| fits_this_system(w.via, in_distro))
            .partition(|w| !in_distro && has(w.via));
        let commands = |ways: Vec<&Way>| ways.iter().map(|w| w.command.to_owned()).collect();
        let (here, elsewhere): (Vec<String>, Vec<String>) = (commands(here), commands(elsewhere));
        if here.is_empty() {
            section(t!("format-install-run-any"), &elsewhere);
        } else {
            section(t!("format-install-run-here"), &here);
            section(t!("format-install-other"), &elsewhere);
        }
        if let Some(note) = install.note {
            text.push_str(&format!("{indent}{}\n", note()));
        }
        let docs = t!("format-install-docs", url = install.url);
        text.push_str(&format!("{indent}{docs}\n"));
    }
    Some(text)
}

/// 在项目中安装的方式：一句标题说明，以及在哪个目录中运行什么命令。
struct ProjectInstall {
    title: &'static str,
    /// 运行命令的目录，即含有 `package.json` 或 `pyproject.toml` 的项目目录。
    dir: PathBuf,
    command: String,
}

/// 返回在项目中安装 `package` 的方式；没有适用的项目环境时返回 `None`。
///
/// Node：向上找到含 `package.json` 的目录，`dependencies` 或 `devDependencies`
/// 已声明该包时安装项目依赖，否则添加为开发依赖，包管理器按锁文件判断。Python：
/// 向上找到含 `pyproject.toml` 的目录，仅当存在 `uv.lock` 或 `poetry.lock` 时添加为开发依赖。
fn project_install(package: Package, dir: &Path) -> Option<ProjectInstall> {
    match package {
        Package::Node(name) => {
            let level = dir
                .ancestors()
                .find(|level| winc_backend::fs::is_file(level.join("package.json")))?;
            let manager = node_manager(level);
            let text = winc_backend::fs::read_to_string(level.join("package.json")).ok()?;
            let json = Json::parse(&text)?;
            Some(if super::tools::declares_dependency(&json, name) {
                ProjectInstall {
                    title: t!("format-install-project-declared"),
                    dir: level.to_path_buf(),
                    command: format!("{manager} install"),
                }
            } else {
                let add = match manager {
                    "npm" => "npm install --save-dev",
                    "pnpm" => "pnpm add --save-dev",
                    "yarn" => "yarn add --dev",
                    _ => "bun add --dev",
                };
                ProjectInstall {
                    title: t!("format-install-add-to-project"),
                    dir: level.to_path_buf(),
                    command: format!("{add} {name}"),
                }
            })
        }
        Package::Python(name) => {
            let level = dir
                .ancestors()
                .find(|level| winc_backend::fs::is_file(level.join("pyproject.toml")))?;
            let command = if winc_backend::fs::is_file(level.join("uv.lock")) {
                format!("uv add --dev {name}")
            } else if winc_backend::fs::is_file(level.join("poetry.lock")) {
                format!("poetry add --group dev {name}")
            } else {
                return None;
            };
            Some(ProjectInstall {
                title: t!("format-install-add-to-project"),
                dir: level.to_path_buf(),
                command,
            })
        }
    }
}

/// 为 shell 引用路径：含空格、引号等字符时用单引号括起来，使路径可以直接粘贴到命令行中使用；
/// 其中的单引号写成 `'\''`。
fn shell_quote(path: &str) -> String {
    let plain = |c: char| c.is_alphanumeric() || "/._-~+".contains(c);
    if path.chars().all(plain) {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

/// 按锁文件判断项目使用的 Node 包管理器（`pnpm`、`yarn`、`bun`），没有锁文件时为 `npm`。
fn node_manager(dir: &Path) -> &'static str {
    [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
    ]
    .into_iter()
    .find(|(lock, _)| winc_backend::fs::is_file(dir.join(lock)))
    .map_or("npm", |(_, manager)| manager)
}

#[cfg(test)]
mod tests;
