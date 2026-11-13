//! 预设的格式化与修复工具，以及工具的自动选择和程序查找。
//!
//! 按项目中的配置文件、依赖声明等迹象判断项目在用哪个工具（[`detect`]），并在项目自带的目录、
//! `PATH` 和 macOS 开发者工具链中查找工具对应的可执行文件。

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use winc_config::Json;

use super::Task;
use crate::tools::shell::process::{self, Cancel};
use crate::tools::shell::{file_names, is_executable};

/// 一个预设的格式化或修复工具。
#[derive(Debug)]
pub struct Preset {
    /// 显示名称。
    pub name: &'static str,
    /// 该工具用于格式化还是修复。
    pub task: Task,
    /// 要运行的命令模板，占位符由 [`super::Plan`] 展开。
    pub command: &'static str,
    /// 支持的语言（语言 id）。
    pub languages: &'static [&'static str],
    /// 自动选用该工具的条件。
    pub auto: Auto,
}

/// 自动选用工具的条件。
#[derive(Debug)]
pub enum Auto {
    /// 该语言只有一种公认的格式，总是使用。
    Always,
    /// 项目里出现其中任一迹象时使用。
    ///
    /// 项目里看不出该用什么工具时按 [`Auto::Installed`] 处理：已安装就使用。
    When(&'static [Sign]),
    /// 没有自己的配置文件，无法判断项目是否在用：项目里看不出该用什么工具时，已安装就使用。
    Installed,
    /// 不自动使用：会改写内容（`jq` 会把 `"\u00e9"` 改成 `"é"`）。
    ///
    /// 只在设置中列出，供用户手动选择。
    Never,
}

/// 项目正在使用某个工具的迹象。
#[derive(Debug, Clone, Copy)]
pub enum Sign {
    /// 存在该文件。
    File(&'static str),
    /// `package.json` 中有该字段，或 `dependencies`、`devDependencies` 中有该包。
    ///
    /// 只看声明的依赖，不看 `node_modules/.bin`：npm 也会为间接依赖建立链接，
    /// 已安装不等于项目在使用。
    Package(&'static str),
    /// `pyproject.toml` 中有该节（如 `tool.black`）。
    Pyproject(&'static str),
    /// Python 虚拟环境（`.venv`、`venv`）中装有该程序。
    Venv(&'static str),
}

/// 表明项目使用 `ESLint` 的迹象（配置文件或 `package.json` 中的声明）。
const ESLINT_FILES: &[Sign] = &[
    Sign::File("eslint.config.js"),
    Sign::File("eslint.config.mjs"),
    Sign::File("eslint.config.cjs"),
    Sign::File("eslint.config.ts"),
    Sign::File("eslint.config.mts"),
    Sign::File("eslint.config.cts"),
    Sign::File(".eslintrc"),
    Sign::File(".eslintrc.js"),
    Sign::File(".eslintrc.cjs"),
    Sign::File(".eslintrc.json"),
    Sign::File(".eslintrc.yaml"),
    Sign::File(".eslintrc.yml"),
    Sign::Package("eslintConfig"),
    Sign::Package("eslint"),
];

/// 表明项目使用 Prettier 的迹象（配置文件或 `package.json` 中的声明）。
const PRETTIER_FILES: &[Sign] = &[
    Sign::File(".prettierrc"),
    Sign::File(".prettierrc.json"),
    Sign::File(".prettierrc.json5"),
    Sign::File(".prettierrc.yaml"),
    Sign::File(".prettierrc.yml"),
    Sign::File(".prettierrc.toml"),
    Sign::File(".prettierrc.js"),
    Sign::File(".prettierrc.cjs"),
    Sign::File(".prettierrc.mjs"),
    Sign::File(".prettierrc.ts"),
    Sign::File("prettier.config.js"),
    Sign::File("prettier.config.cjs"),
    Sign::File("prettier.config.mjs"),
    Sign::File("prettier.config.ts"),
    Sign::Package("prettier"),
];

/// 预设工具列表；同一种语言中排在前面的优先。
///
/// 修复工具只在项目里有迹象时才自动使用（见 `detect`）。
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "rustfmt",
        task: Task::Format,
        command: "rustfmt --edition {rust-edition}",
        languages: &["rust"],
        auto: Auto::Always,
    },
    Preset {
        name: "gofmt",
        task: Task::Format,
        command: "gofmt",
        languages: &["go"],
        auto: Auto::Always,
    },
    Preset {
        name: "Prettier",
        task: Task::Format,
        command: "prettier --stdin-filepath {file}",
        languages: &[
            "javascript",
            "typescript",
            "vue",
            "css",
            "scss",
            "less",
            "html",
            "markdown",
            "yaml",
            "json",
        ],
        auto: Auto::When(PRETTIER_FILES),
    },
    Preset {
        name: "Biome",
        task: Task::Format,
        command: "biome format --stdin-file-path={file}",
        languages: &["javascript", "typescript", "css", "json"],
        auto: Auto::When(&[
            Sign::File("biome.json"),
            Sign::File("biome.jsonc"),
            Sign::Package("@biomejs/biome"),
        ]),
    },
    // 排在 Ruff 前面：项目可能用 Ruff 做检查、用 Black 格式化，而 Black 只做格式化，
    // 项目配置了 Black 就说明格式化用的是它。
    Preset {
        name: "Black",
        task: Task::Format,
        command: "black --quiet --stdin-filename {file} -",
        languages: &["python"],
        auto: Auto::When(&[Sign::Pyproject("tool.black"), Sign::Venv("black")]),
    },
    Preset {
        name: "Ruff",
        task: Task::Format,
        command: "ruff format --stdin-filename {file} -",
        languages: &["python"],
        auto: Auto::When(&[
            Sign::File("ruff.toml"),
            Sign::File(".ruff.toml"),
            Sign::Pyproject("tool.ruff"),
            Sign::Venv("ruff"),
        ]),
    },
    Preset {
        name: "ClangFormat",
        task: Task::Format,
        command: "clang-format --assume-filename={file}",
        languages: &["c", "cpp", "objective-c", "objective-cpp"],
        auto: Auto::When(&[Sign::File(".clang-format"), Sign::File("_clang-format")]),
    },
    Preset {
        name: "swift-format",
        task: Task::Format,
        command: "swift-format format --assume-filename {file} -",
        languages: &["swift"],
        auto: Auto::When(&[Sign::File(".swift-format")]),
    },
    Preset {
        name: "SwiftFormat",
        task: Task::Format,
        command: "swiftformat --stdinpath {file}",
        languages: &["swift"],
        auto: Auto::When(&[Sign::File(".swiftformat")]),
    },
    Preset {
        name: "Taplo",
        task: Task::Format,
        command: "taplo fmt -",
        languages: &["toml"],
        auto: Auto::When(&[Sign::File(".taplo.toml"), Sign::File("taplo.toml")]),
    },
    Preset {
        name: "shfmt",
        task: Task::Format,
        command: "shfmt -filename {file}",
        languages: &["shell"],
        auto: Auto::Installed,
    },
    Preset {
        name: "google-java-format",
        task: Task::Format,
        command: "google-java-format -",
        languages: &["java"],
        auto: Auto::Installed,
    },
    Preset {
        name: "jq",
        task: Task::Format,
        command: "jq .",
        languages: &["json"],
        auto: Auto::Never,
    },
    Preset {
        name: "xmllint",
        task: Task::Format,
        command: "xmllint --format -",
        languages: &["xml", "svg"],
        auto: Auto::Never,
    },
    // 修复工具同样从标准输入读取、向标准输出写出修复后的内容。仍有无法自动修复的问题时不算失败：
    // Ruff 加 `--exit-zero`；ESLint 此时退出码为 1，修复后的内容在 JSON 结果里
    // （见 `super::eslint_output`）。
    Preset {
        name: "Ruff",
        task: Task::Fix,
        command: "ruff check --fix --exit-zero --stdin-filename {file} -",
        languages: &["python"],
        auto: Auto::When(&[
            Sign::File("ruff.toml"),
            Sign::File(".ruff.toml"),
            Sign::Pyproject("tool.ruff"),
            Sign::Venv("ruff"),
        ]),
    },
    Preset {
        name: "ESLint",
        task: Task::Fix,
        command: "eslint --stdin --stdin-filename {file} --fix-dry-run --format json",
        languages: &["javascript", "typescript", "vue"],
        auto: Auto::When(ESLINT_FILES),
    },
    Preset {
        name: "Biome",
        task: Task::Fix,
        command: "biome lint --write --stdin-file-path={file}",
        languages: &["javascript", "typescript", "css"],
        auto: Auto::When(&[
            Sign::File("biome.json"),
            Sign::File("biome.jsonc"),
            Sign::Package("@biomejs/biome"),
        ]),
    },
];

/// 支持只格式化一部分的工具（按程序名匹配），以及追加在命令末尾的参数；占位符见
/// [`super::FormatRange`]。
///
/// 参数追加在末尾，因为 `ruff format`、`swift-format format` 的选项必须跟在子命令之后。
const RANGE_ARGS: &[(&str, &[&str])] = &[
    ("clang-format", &["--lines={start-line}:{end-line}"]),
    ("swift-format", &["--lines", "{start-line}:{end-line}"]),
    (
        "prettier",
        &["--range-start={start-offset}", "--range-end={end-offset}"],
    ),
    // 结束位置不含在内：取最后一行最后一个字符之后的位置。
    ("ruff", &["--range={start-line}:1-{end-line}:{end-column}"]),
    ("black", &["--line-ranges={start-line}-{end-line}"]),
];

/// 返回程序 `name` 只格式化一部分时要追加的参数；不支持时返回 `None`。
pub(super) fn range_args(name: &str) -> Option<&'static [&'static str]> {
    RANGE_ARGS
        .iter()
        .find(|(program, _)| *program == name)
        .map(|(_, args)| *args)
}

/// 返回 `language` 可用的格式化工具预设，按优先顺序。
pub fn presets_for(language: &str) -> impl Iterator<Item = &'static Preset> + '_ {
    presets(Task::Format, language)
}

/// 返回 `language` 可用的 `task` 类预设，按优先顺序。
pub fn presets(task: Task, language: &str) -> impl Iterator<Item = &'static Preset> + '_ {
    PRESETS
        .iter()
        .filter(move |p| p.task == task && p.languages.contains(&language))
}

/// 自动选择工具的结果。
#[derive(Debug, PartialEq)]
pub(super) enum Detected {
    /// 选中的预设，以及依据（如 `.prettierrc`）；总是使用的预设没有依据。
    Preset(&'static Preset, Option<String>),
    /// 项目里看不出该用什么工具：按顺序取其中第一个已安装的预设。
    Installed(Vec<&'static Preset>),
    /// 项目里看不出该用什么修复工具。
    ///
    /// 修复可能改变代码的行为，因此不像格式化那样已安装就使用。
    NoSign(Vec<&'static Preset>),
    /// 该语言没有可自动使用的预设。
    NoPreset,
}

impl Preset {
    /// 返回要运行的程序（命令的第一个词），也是在 `PATH` 中查找的名字。
    pub fn program(&self) -> &'static str {
        self.command.split(' ').next().unwrap_or(self.command)
    }
}

impl PartialEq for Preset {
    fn eq(&self, other: &Self) -> bool {
        self.command == other.command
    }
}

/// 从 `dir` 向上逐级查找项目在用哪个工具：离文件最近的迹象优先，同一级内按预设顺序。
///
/// 没有迹象时，格式化回退到已安装的工具（[`Detected::Installed`]），修复则不自动使用
/// （[`Detected::NoSign`]）。
pub(super) fn detect(task: Task, language: &str, dir: &Path) -> Detected {
    let candidates: Vec<(&'static Preset, &'static [Sign])> = presets(task, language)
        .filter_map(|p| match p.auto {
            Auto::Always => Some((p, &[][..])),
            Auto::When(signs) => Some((p, signs)),
            Auto::Installed => Some((p, &[][..])),
            Auto::Never => None,
        })
        .collect();
    if let Some(&(always, _)) = candidates
        .iter()
        .find(|(p, _)| matches!(p.auto, Auto::Always))
    {
        return Detected::Preset(always, None);
    }
    if candidates.is_empty() {
        return Detected::NoPreset;
    }

    // 远端的目录：只看那台主机上的各级，每级的内容先一起列出，查看迹象时不再逐个文件询问。
    let remote = winc_backend::is_remote(dir);
    let levels: Vec<&Path> = (dir.ancestors())
        .take_while(|level| !remote || winc_backend::is_remote(level))
        .collect();
    let listed = match remote {
        true => list_levels(&levels),
        false => vec![None; levels.len()],
    };
    for (level, names) in levels.into_iter().zip(listed) {
        let mut files = ProjectFiles::new(level, names);
        for &(preset, signs) in &candidates {
            if let Some(reason) = signs.iter().find_map(|&sign| files.shows(sign)) {
                return Detected::Preset(preset, Some(reason));
            }
        }
    }
    let presets = candidates.into_iter().map(|(p, _)| p).collect();
    match task {
        Task::Format => Detected::Installed(presets),
        Task::Fix => Detected::NoSign(presets),
    }
}

/// 解析好的 `package.json` 是否在 `dependencies` 或 `devDependencies` 里声明了包 `name`。
pub(super) fn declares_dependency(package: &Json, name: &str) -> bool {
    ["dependencies", "devDependencies"].iter().any(|key| {
        package
            .as_object()
            .and_then(|o| o.get(*key))
            .and_then(Json::as_object)
            .is_some_and(|deps| deps.contains_key(name))
    })
}

/// 一级目录中各项的名字，以及它是不是目录。
type Names = HashMap<std::ffi::OsString, bool>;

/// 同时列出远端的各级目录 `levels`，每级一次往返；列不出来的（没有权限）为 `None`，查看迹象
/// 时照常逐个文件询问。当前线程不能等待远端的应答时全部为 `None`。
fn list_levels(levels: &[&Path]) -> Vec<Option<Names>> {
    if !winc_backend::remote::may_block() {
        return vec![None; levels.len()];
    }
    let list = |level: &Path| -> Option<Names> {
        let entries = winc_backend::fs::read_dir(level).ok()?;
        Some(
            entries
                .flatten()
                .map(|entry| (entry.name, entry.kind.is_dir()))
                .collect(),
        )
    };
    std::thread::scope(|scope| {
        let waiting: Vec<_> = levels
            .iter()
            .map(|level| scope.spawn(|| list(level)))
            .collect();
        waiting
            .into_iter()
            .map(|listed| {
                listed
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            })
            .collect()
    })
}

/// 某一级目录中与迹象有关的文件；读过的内容缓存下来，同一级不重复读取。
struct ProjectFiles<'a> {
    dir: &'a Path,
    /// 这一级目录的内容，已经列出时有：其中没有的文件不必再向文件系统询问。
    names: Option<Names>,
    /// `package.json` 的解析结果：外层 `None` 表示尚未读取，内层 `None` 表示文件不存在或无法解析。
    package: Option<Option<Json>>,
    /// `pyproject.toml` 的内容：外层 `None` 表示尚未读取，内层 `None` 表示文件不存在或无法读取。
    pyproject: Option<Option<String>>,
}

impl<'a> ProjectFiles<'a> {
    fn new(dir: &'a Path, names: Option<Names>) -> Self {
        ProjectFiles {
            dir,
            names,
            package: None,
            pyproject: None,
        }
    }

    /// 判断该级目录是否有迹象 `sign`；有时返回展示给用户的依据（文件名），没有时返回 `None`。
    fn shows(&mut self, sign: Sign) -> Option<String> {
        let dir = self.dir;
        // 目录的内容已经列出时，其中没有的一项肯定不存在。
        let names = self.names.as_ref();
        let absent = |name: &str| names.is_some_and(|names| !names.contains_key(OsStr::new(name)));
        match sign {
            Sign::File(name) => match names.and_then(|names| names.get(OsStr::new(name))) {
                Some(is_dir) => (!is_dir).then(|| name.to_owned()),
                None if names.is_some() => None,
                None => winc_backend::fs::is_file(dir.join(name)).then(|| name.to_owned()),
            },
            Sign::Package(_) if absent("package.json") => None,
            Sign::Pyproject(_) if absent("pyproject.toml") => None,
            Sign::Venv(_) if absent(".venv") && absent("venv") => None,
            Sign::Package(name) => {
                let package = self.package.get_or_insert_with(|| {
                    let text = winc_backend::fs::read_to_string(dir.join("package.json")).ok()?;
                    Json::parse(&text)
                });
                let package = package.as_ref()?;
                let declared =
                    package.as_object()?.contains_key(name) || declares_dependency(package, name);
                declared.then(|| "package.json".to_owned())
            }
            Sign::Pyproject(section) => {
                let text = self
                    .pyproject
                    .get_or_insert_with(|| {
                        winc_backend::fs::read_to_string(dir.join("pyproject.toml")).ok()
                    })
                    .as_deref()?;
                let header = format!("[{section}]");
                let nested = format!("[{section}.");
                text.lines()
                    .map(str::trim)
                    .any(|line| line == header || line.starts_with(&nested))
                    .then(|| "pyproject.toml".to_owned())
            }
            Sign::Venv(program) => {
                let bin = if windows_programs(dir) {
                    "Scripts"
                } else {
                    "bin"
                };
                let names = program_names(program, dir);
                [".venv", "venv"]
                    .into_iter()
                    .find(|venv| {
                        let bin = dir.join(venv).join(bin);
                        names.iter().any(|name| is_executable(&bin.join(name)))
                    })
                    .map(str::to_owned)
            }
        }
    }
}

/// 目录 `dir` 中的程序是否按 Windows 的规则组织：WSL 发行版中的目录按 Linux 的规则。
fn windows_programs(dir: &Path) -> bool {
    cfg!(windows) && crate::wsl::locate_here(dir).is_none()
}

/// 目录 `dir` 中程序 `name` 可能使用的文件名。
fn program_names(name: &str, dir: &Path) -> Vec<String> {
    if windows_programs(dir) {
        file_names(name, std::env::var_os("PATHEXT").as_deref())
    } else {
        vec![name.to_owned()]
    }
}

/// 项目内存放可执行程序的目录（相对于每一级目录）。
///
/// Windows 上 Python 虚拟环境的程序位于 `Scripts` 中。
fn project_bins(windows: bool) -> &'static [&'static str] {
    if windows {
        &["node_modules/.bin", ".venv/Scripts", "venv/Scripts"]
    } else {
        &["node_modules/.bin", ".venv/bin", "venv/bin"]
    }
}

/// 查找项目自带的程序：从 `dir` 向上，逐级查看 `node_modules/.bin`、`.venv/bin`、`venv/bin`，
/// 一直找到文件系统根，因此 monorepo 中安装在上层的程序也能找到。
///
/// 返回程序的路径，以及它所在的那一级项目目录。
pub(super) fn find_in_project(name: &str, dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let names = program_names(name, dir);
    let bins = project_bins(windows_programs(dir));
    // 远端的目录：各级的候选位置一次问完（一次往返），只看那台主机上的各级。
    if winc_backend::is_remote(dir) {
        let mut candidates: Vec<(PathBuf, &Path)> = Vec::new();
        for level in (dir.ancestors()).take_while(|level| winc_backend::is_remote(level)) {
            for bin in bins {
                let found = names.iter().map(|name| (level.join(bin).join(name), level));
                candidates.extend(found);
            }
        }
        let paths: Vec<&Path> = candidates.iter().map(|(path, _)| path.as_path()).collect();
        let found = winc_backend::fs::metadata_many(&paths);
        return (candidates.iter().zip(found)).find_map(|((path, level), metadata)| {
            let metadata = metadata.ok()?;
            let runs =
                metadata.kind.is_file() && metadata.unix.is_some_and(|unix| unix.mode & 0o111 != 0);
            runs.then(|| (path.clone(), level.to_path_buf()))
        });
    }
    dir.ancestors().find_map(|level| {
        bins.iter()
            .flat_map(|bin| names.iter().map(move |name| level.join(bin).join(name)))
            .find(|path| is_executable(path))
            .map(|path| (path, level.to_path_buf()))
    })
}

/// 是否需要通过 `xcrun` 查找程序 `name`：macOS 上 `clang-format`、`swift-format` 只存在于
/// 开发者工具链中，`PATH` 和 `/usr/bin` 里都没有。
pub(super) fn needs_xcrun(name: &str) -> bool {
    cfg!(target_os = "macos") && matches!(name, "clang-format" | "swift-format")
}

/// 通过 `xcrun` 查找工具链中的程序；不需要 `xcrun` 或未安装开发者工具时返回 `None`。
///
/// 未安装开发者工具时运行 `xcrun` 会弹出系统的安装提示，因此先用 `xcode-select -p` 确认已安装。
pub(super) fn find_with_xcrun(name: &str) -> Option<PathBuf> {
    if !needs_xcrun(name) {
        return None;
    }

    let quiet = |program: &str, args: &[&str]| {
        let mut command = Command::new(program);
        command.args(args).stderr(Stdio::null());
        // 每次查询最多等 5 秒（经验值）：xcode-select 和 xcrun 正常在几十毫秒内返回，超时按
        // 没有该程序处理。
        process::run(
            &mut command,
            Vec::new(),
            Duration::from_secs(5),
            &Cancel::default(),
        )
        .ok()
        .filter(|output| output.status.success())
    };
    quiet("/usr/bin/xcode-select", &["-p"])?;
    let output = quiet("/usr/bin/xcrun", &["--find", name])?;
    let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    is_executable(&path).then_some(path)
}

/// 返回 `dir` 中的文件所属 crate 的 Rust 版次（edition）；无法确定时返回 `None`。
///
/// 值取自 Cargo 清单：最近的 `Cargo.toml` 中 `[package]` 的 `edition`；写的是
/// `edition.workspace = true` 时再向上找 `[workspace.package]` 中的值；没有写 `edition` 时为 2015。
/// 没有 `Cargo.toml`（单独的 `.rs` 文件）时取 `rustfmt.toml` 中的设置；都没有时返回 `None`。
///
/// 上述文件按行读取，只识别列出的写法，以避免为此引入 TOML 解析器。
pub(super) fn rust_edition(dir: &Path) -> Option<String> {
    let mut cache: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut read = |path: PathBuf| {
        cache
            .entry(path.clone())
            .or_insert_with(|| winc_backend::fs::read_to_string(&path).ok())
            .clone()
    };

    for level in dir.ancestors() {
        let Some(text) = read(level.join("Cargo.toml")) else {
            continue;
        };
        match edition_in(&text, "package") {
            Some(Edition::Value(edition)) => return Some(edition),
            Some(Edition::Missing) => return Some("2015".to_owned()),
            Some(Edition::Workspace) => {
                // 继承工作区的设置：从当前目录向上找含 `[workspace.package]` 的清单（工作区的根
                // 本身也可以是一个 crate）。
                let from_workspace = level.ancestors().find_map(|up| {
                    match edition_in(&read(up.join("Cargo.toml"))?, "workspace.package")? {
                        Edition::Value(edition) => Some(edition),
                        _ => None,
                    }
                });
                return from_workspace.or_else(|| Some("2015".to_owned()));
            }
            // 只有 `[workspace]` 的根清单，不是该文件所属的 crate，继续向上查找。
            None => {}
        }
    }

    dir.ancestors().find_map(|level| {
        [".rustfmt.toml", "rustfmt.toml"].iter().find_map(|name| {
            match edition_in(&read(level.join(name))?, "")? {
                Edition::Value(edition) => Some(edition),
                _ => None,
            }
        })
    })
}

/// 清单中 `edition` 的写法。
#[derive(Debug, PartialEq)]
enum Edition {
    /// 直接写出的版次，如 `2024`。
    Value(String),
    /// `edition.workspace = true`。
    Workspace,
    /// 有该节，但没有写 `edition`。
    Missing,
}

/// 返回 `section` 节中的 `edition`；空的 `section` 表示文件开头不在任何节内的部分。
///
/// 没有该节时返回 `None`。
fn edition_in(text: &str, section: &str) -> Option<Edition> {
    let mut current = String::new();
    let mut found = section.is_empty();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = header.trim().to_owned();
            found |= current == section;
            continue;
        }
        if current != section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let workspace = value.trim_matches(['{', '}', ' ']).replace(' ', "") == "workspace=true";
        match key {
            "edition.workspace" if value == "true" => return Some(Edition::Workspace),
            "edition" if workspace => return Some(Edition::Workspace),
            "edition" => return Some(Edition::Value(value.trim_matches(['"', '\'']).to_owned())),
            _ => {}
        }
    }
    found.then_some(Edition::Missing)
}

#[cfg(test)]
mod tests;
