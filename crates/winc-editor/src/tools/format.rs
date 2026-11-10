//! 格式化和修复：把文档通过标准输入交给外部工具，用工具的标准输出替换文档内容。
//!
//! 可用的工具见 [`tools::PRESETS`]。[`Plan`] 决定运行哪条命令（设置里写的命令优先，否则按
//! 项目里的迹象自动选择），[`Formatter`] 在后台线程里运行该命令，[`Job`] 携带一次运行所需的
//! 全部输入。工具不支持时返回 [`FormatError`]，不会修改文档。

mod command;
mod install;
mod tools;

pub use crate::tools::shell::Cancel;
pub use install::{guide as install_guide, knows as knows_how_to_install};
pub use tools::{Auto, PRESETS, Preset, presets, presets_for};

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ropey::Rope;
use winc_config::Json;
use winc_core::diff;
use winc_core::{Change, EditKind, LineEnding, Selection, Transaction, indent};
use winc_i18n::t;
use winc_syntax::Language;

use crate::document::content::Content;
use crate::tools::shell::process::{self, MAX_OUTPUT, RunError};
use crate::tools::shell::{Shell, is_executable};
use crate::{Document, display_name};
use tools::Detected;

/// 外部工具执行的任务。
///
/// 两种任务的输出都会替换文档内容，区别在于工具是否会改动程序的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// 只改排版，不改变程序行为。
    Format,
    /// 用 lint 工具修复可自动修复的问题（例如删除未使用的 import），可能改变代码行为。
    Fix,
}

impl Task {
    /// 返回任务的名称（「格式化」或「修复」），用于提示信息。
    pub fn verb(self) -> &'static str {
        match self {
            Task::Format => t!("format-task-format"),
            Task::Fix => t!("format-task-fix"),
        }
    }

    /// 返回任务在文案中的选择键（`format` 或 `fix`），供提示信息按任务选择措辞。
    pub fn key(self) -> &'static str {
        match self {
            Task::Format => "format",
            Task::Fix => "fix",
        }
    }
}

/// 格式化（修复）未能完成的原因，显示在「无法格式化：」之后。
///
/// 保存时自动格式化的场景只显示其中一部分原因，具体见各变体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// 无法执行：认不出语言，或工具不支持格式化所选内容。
    ///
    /// 保存时遇到这种情况不提示。
    Unavailable(String),
    /// 没有指定命令，自动选择时又选不出工具：项目里看不出用什么，或这种语言没有自动使用的
    /// 预设。
    ///
    /// 在设置中指定命令后即可使用；保存时遇到这种情况同样不提示。
    Unset(String),
    /// 找不到要运行的程序。
    Missing {
        /// 提示信息。
        message: String,
        /// 知道安装方法的工具（预设中的那些）。
        tools: Vec<String>,
    },
    /// 项目里看不出用什么工具，候选工具又都没有安装。
    NoTool {
        /// 提示信息。
        message: String,
        /// 候选工具。
        tools: Vec<String>,
    },
    /// 运行了，但没有得到可用的结果。
    Failed(String),
    /// 工具出错退出，或没有输出但标准错误里有内容。
    Tool {
        /// 提示信息。
        message: String,
        /// 完整记录（命令、退出码和标准错误），供「查看输出」显示。
        output: String,
    },
    /// 已取消（标签被关闭或程序退出）。
    Cancelled,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Unavailable(message)
            | FormatError::Unset(message)
            | FormatError::Missing { message, .. }
            | FormatError::NoTool { message, .. }
            | FormatError::Failed(message)
            | FormatError::Tool { message, .. } => f.write_str(message),
            FormatError::Cancelled => f.write_str(t!("format-cancelled")),
        }
    }
}

impl FormatError {
    /// 返回未安装、且知道安装方法的工具。
    pub fn tools_to_install(&self) -> &[String] {
        match self {
            FormatError::Missing { tools, .. } | FormatError::NoTool { tools, .. } => tools,
            _ => &[],
        }
    }

    /// 构造找不到程序 `name` 的错误。
    fn missing(message: String, name: &str) -> FormatError {
        let tools = install::knows(name)
            .then(|| name.to_owned())
            .into_iter()
            .collect();
        FormatError::Missing { message, tools }
    }
}

/// 要运行的程序。
///
/// 按程序所在的位置区分，决定是否需要在运行前询问用户、能否退回 `PATH` 查找。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// 在 `PATH` 中查找；macOS 上找不到 clang-format、swift-format 时再通过 xcrun 查找。
    Search(String),
    /// 项目自带的程序，运行前需要用户确认。
    ///
    /// 命令里写的相对路径不能退回 `PATH` 查找，此时 `searchable` 为 `false`。
    Project {
        /// 程序路径。
        path: PathBuf,
        /// 程序所在目录，用户的选择按该目录记录。
        folder: PathBuf,
        /// 不运行项目内的程序时，能否退回 `PATH` 查找同名程序。
        searchable: bool,
    },
    /// 命令里写的绝对路径。
    Path(PathBuf),
}

/// 格式化一个文件要运行的命令计划。
///
/// 在界面线程上确定（只检查文件是否存在），在后台线程里运行。
#[derive(Debug, Clone)]
pub struct Plan {
    /// 拆分后的命令参数，占位符尚未替换；第一个是程序。
    args: Vec<String>,
    /// 只格式化这一区段；为 `None` 时处理整个文件。
    range: Option<FormatRange>,
    /// 要运行的程序。
    pub program: Program,
    /// 自动选择时命中的预设名和依据（例如 `.prettierrc`）；用户自定义命令时为 `None`。
    pub auto: Option<(&'static str, Option<String>)>,
    /// 被格式化的文件路径，占位符据此展开。
    file: PathBuf,
    /// 项目里看不出用什么工具时依次尝试的预设。
    ///
    /// 运行前（[`Formatter::resolve`]）确定为第一个
    /// 已安装的；在那之前命令取自第一个预设。
    guesses: Vec<&'static Preset>,
    /// 语言显示名，用于提示信息。
    language: &'static str,
    /// 这次要做的是格式化还是修复。
    task: Task,
}

impl Plan {
    /// 为格式化 `file` 创建计划：使用 `configured`（设置里写的命令），没有写则自动选择工具。
    ///
    /// `root` 是打开的目录，命令中的相对路径从该目录起算。
    ///
    /// # Errors
    ///
    /// 无法识别语言、项目里看不出该用哪个工具、命令为空或解析失败时返回 [`FormatError`]。
    pub fn new(
        language: &Language,
        file: &Path,
        root: Option<&Path>,
        configured: Option<&str>,
    ) -> Result<Plan, FormatError> {
        Plan::for_task(Task::Format, language, file, root, configured)
    }

    /// 与 [`Plan::new`] 相同，但执行 `task`（格式化或修复）。
    ///
    /// # Errors
    ///
    /// 与 [`Plan::new`] 相同。
    pub fn for_task(
        task: Task,
        language: &Language,
        file: &Path,
        root: Option<&Path>,
        configured: Option<&str>,
    ) -> Result<Plan, FormatError> {
        if language.id == "text" {
            return Err(FormatError::Unavailable(
                t!("format-unknown-language").to_owned(),
            ));
        }
        let dir = file.parent().unwrap_or(Path::new("/"));
        let (command, auto, guesses) = match configured {
            Some(command) => (command, None, Vec::new()),
            None => match tools::detect(task, language.id, dir) {
                Detected::Preset(preset, reason) => {
                    (preset.command, Some((preset.name, reason)), Vec::new())
                }
                Detected::Installed(presets) => {
                    // `detect` 只在候选列表非空时返回 `Installed`。
                    (presets[0].command, Some((presets[0].name, None)), presets)
                }
                Detected::NoSign(presets) => {
                    let names: Vec<&str> = presets.iter().map(|p| p.name).collect();
                    let names = names.join(t!("format-tool-list-separator"));
                    let language = language.name;
                    let message = t!(
                        "format-no-fixer-configured",
                        language = language,
                        names = names
                    );
                    return Err(FormatError::Unset(message));
                }
                Detected::NoPreset => {
                    let (language, task) = (language.name, task.key());
                    let message = t!("format-no-auto-tool", language = language, task = task);
                    return Err(FormatError::Unset(message));
                }
            },
        };
        let mut plan = Plan::with_command(command, auto, file, root)?;
        plan.language = language.name;
        plan.task = task;
        if !guesses.is_empty() {
            // `with_command` 已确认命令至少有一个参数。
            plan.program = Program::Search(plan.args[0].clone());
            plan.guesses = guesses;
        }
        Ok(plan)
    }

    /// 用 `command` 创建格式化 `file` 的计划。
    ///
    /// # Errors
    ///
    /// 命令为空或无法拆分时返回 [`FormatError::Failed`]。
    fn with_command(
        command: &str,
        auto: Option<(&'static str, Option<String>)>,
        file: &Path,
        root: Option<&Path>,
    ) -> Result<Plan, FormatError> {
        let dir = file.parent().unwrap_or(Path::new("/"));
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let args = command::split(command, home.as_deref()).map_err(FormatError::Failed)?;
        let Some(name) = args.first() else {
            return Err(FormatError::Failed(t!("format-command-empty").to_owned()));
        };
        let program = if name.contains('/') {
            let path = PathBuf::from(name);
            if path.is_absolute() {
                Program::Path(path)
            } else {
                let base = root.unwrap_or(dir);
                Program::Project {
                    path: base.join(path),
                    folder: base.to_path_buf(),
                    searchable: false,
                }
            }
        } else {
            match tools::find_in_project(name, dir) {
                Some((path, folder)) => Program::Project {
                    path,
                    folder,
                    searchable: true,
                },
                None => Program::Search(name.clone()),
            }
        };

        Ok(Plan {
            args,
            range: None,
            program,
            auto,
            file: file.to_path_buf(),
            guesses: Vec::new(),
            language: "",
            task: Task::Format,
        })
    }

    /// 是否要等到运行前才能确定工具：项目里看不出用什么工具，需要看装了哪个
    /// （见 [`Formatter::resolve`]）。
    pub fn is_guess(&self) -> bool {
        !self.guesses.is_empty()
    }

    /// 按顺序选出第一个已安装的预设，返回确定后的计划。
    ///
    /// `installed` 回答一个程序是否已安装，不知道时返回 `None`，此时也无法确定，本函数返回 `None`。
    /// 都没安装时返回 [`FormatError::NoTool`]。
    fn settle(
        &self,
        installed: impl Fn(&str) -> Option<bool>,
    ) -> Option<Result<Plan, FormatError>> {
        if self.guesses.is_empty() {
            return Some(Ok(self.clone()));
        }
        for &preset in &self.guesses {
            if installed(preset.program())? {
                // 预设的命令都是预先写好的，一定能拆分。使用系统中安装的程序，不去项目里找
                // （项目自带的程序运行前必须先询问用户）。
                let mut plan =
                    Plan::with_command(preset.command, Some((preset.name, None)), &self.file, None)
                        .ok()?;
                plan.program = Program::Search(preset.program().to_owned());
                plan.range = self.range.clone();
                plan.language = self.language;
                plan.task = self.task;
                return Some(Ok(plan));
            }
        }
        let names: Vec<&str> = self.guesses.iter().map(|p| p.name).collect();
        let (language, names) = (self.language, names.join(t!("format-tool-list-separator")));
        Some(Err(FormatError::NoTool {
            message: t!(
                "format-no-formatter-installed",
                language = language,
                names = names
            ),
            tools: self
                .guesses
                .iter()
                .map(|p| p.program().to_owned())
                .collect(),
        }))
    }

    /// 返回这次计划要做的任务。
    pub fn task(&self) -> Task {
        self.task
    }

    /// 返回程序的名字（例如 `prettier`），用于提示信息。
    pub fn tool(&self) -> Cow<'_, str> {
        match &self.program {
            Program::Search(name) => Cow::Borrowed(name),
            Program::Project { path, .. } | Program::Path(path) => display_name(path),
        }
    }

    /// 限定只格式化 `range` 这一区段；工具不支持时返回原因，命令保持不变。
    ///
    /// 需要运行前才能确定工具的计划留到那时再检查。
    ///
    /// # Errors
    ///
    /// 已知工具不支持区段格式化（见 `tools::range_args`）时返回 [`FormatError::Unavailable`]。
    pub fn restrict(&mut self, range: &FormatRange) -> Result<(), FormatError> {
        if !self.is_guess() {
            self.range_args()?;
        }
        self.range = Some(range.clone());
        Ok(())
    }

    /// 是否只格式化一部分。
    pub fn is_restricted(&self) -> bool {
        self.range.is_some()
    }

    /// 返回只格式化一部分时加在命令末尾的参数；工具不支持时返回原因。
    fn range_args(&self) -> Result<&'static [&'static str], FormatError> {
        let tool = self.tool();
        tools::range_args(&tool)
            .ok_or_else(|| FormatError::Unavailable(t!("format-no-range", tool = tool.as_ref())))
    }

    /// 不用项目自带的程序，改到 `PATH` 里找同名的程序。
    pub fn use_system(&mut self) {
        if let Program::Project {
            searchable: true, ..
        } = self.program
        {
            // 命令至少有一个参数，见 [`Plan::with_command`]。
            self.program = Program::Search(self.args[0].clone());
        }
    }

    /// 是否 `command` 把 `{file}` 当成要处理的文件交给了工具（`gofmt {file}`），而不是某个
    /// 选项的值（`--stdin-filepath {file}`）。
    ///
    /// 是则工具会读取磁盘上的文件，而不是编辑器里的内容。
    pub fn reads_file_from_disk(command: &str) -> bool {
        let args = command::split(command, None).unwrap_or_default();
        args.windows(2)
            .any(|pair| pair[1] == command::FILE && !pair[0].starts_with('-'))
    }
}

/// 只格式化文件的一部分时送进工具的区段。
///
/// 各个工具对范围的表示方式不同，因此预先全部算好。`{start-line}`、`{end-line}` 是第一行和最后一行
/// （行号从 1 起，含最后一行）；`{end-column}` 是最后一行最后一个字符之后的列（列号从 1 起，Ruff
/// 的结束位置不含该列）；`{start-offset}`、`{end-offset}` 是区段在送给工具的内容（换行符统一为
/// LF）中的位置，按 UTF-16 码元数计（Prettier 按 JavaScript 字符串下标计），不含结束位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRange {
    /// 第一行的行号（从 1 起）。
    start_line: usize,
    /// 最后一行的行号（从 1 起，含该行）。
    end_line: usize,
    /// 最后一行的结束列号（从 1 起）。
    end_column: usize,
    /// 区段起点的偏移（UTF-16 码元，从 0 起）。
    start_offset: usize,
    /// 区段终点的偏移（UTF-16 码元，从 0 起，不含该位置）。
    end_offset: usize,
}

impl FormatRange {
    /// 创建覆盖文本中 `from..to`（字符偏移）的区段；选区止于下一行行首时，那一行不计入。
    ///
    /// # Panics
    ///
    /// `from` 超出 `text` 的字符数时 panic。
    pub fn new(text: &Rope, from: usize, to: usize) -> FormatRange {
        let to = to.min(text.len_chars()).max(from);
        let start = text.char_to_line(from);
        let mut end = text.char_to_line(to);
        if to > from && end > start && to == text.line_to_char(end) {
            end -= 1;
        }
        let last = text.line(end);
        let last_len = last.chars().filter(|c| !matches!(c, '\n' | '\r')).count();
        // 送给工具的内容统一为 LF，因此每个 `\r\n` 少算一个字符。
        let utf16 = |pos: usize| {
            let before = text.slice(..pos);
            let crlf = before
                .chars()
                .zip(before.chars().skip(1))
                .filter(|&(a, b)| a == '\r' && b == '\n')
                .count();
            before.chars().map(char::len_utf16).sum::<usize>() - crlf
        };
        FormatRange {
            start_line: start + 1,
            end_line: end + 1,
            end_column: last_len + 1,
            start_offset: utf16(from),
            end_offset: utf16(to),
        }
    }

    fn expand(&self, arg: &str) -> String {
        arg.replace("{start-line}", &self.start_line.to_string())
            .replace("{end-line}", &self.end_line.to_string())
            .replace("{end-column}", &self.end_column.to_string())
            .replace("{start-offset}", &self.start_offset.to_string())
            .replace("{end-offset}", &self.end_offset.to_string())
    }
}

/// 一次格式化任务：要运行的命令，以及文档当时的内容。
#[derive(Debug, Clone)]
pub struct Job {
    /// 要运行的命令计划。
    pub plan: Plan,
    /// 送去格式化的文本。
    pub text: Rope,
    /// 文档的换行风格，工具输出的换行符统一转换为它。
    pub line_ending: LineEnding,
    /// 允许工具运行的最长时间，超时后连同子进程一起终止。
    pub timeout: Duration,
}

/// 格式化的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Formatted {
    /// 逐行的修改，按位置从后往前排列，可直接放进一个 `Transaction`；为空表示文档本来就已格式化。
    pub changes: Vec<Change>,
    /// 改动的行数（行），用于提示信息。
    pub lines: usize,
    /// 使用的工具名（例如 `ruff`），用于提示信息；项目里看不出用什么工具时，运行前才能确定。
    pub tool: String,
}

/// 运行格式化工具所用的环境（用户 shell 的 [`Shell`]，加上 macOS 上 xcrun 的查询结果），
/// 各线程共用同一个实例；克隆得到的是同一个实例。
#[derive(Debug, Clone)]
pub struct Formatter(Arc<Shared>);

#[derive(Debug)]
struct Shared {
    /// 用户 shell 的环境：在 `PATH` 里找工具、运行工具都用它。
    shell: Shell,
    /// 已通过 xcrun 查询过的程序及其结果，每个程序只查询一次。
    xcrun: Mutex<Vec<(String, Option<PathBuf>)>>,
}

impl Default for Formatter {
    fn default() -> Self {
        Formatter::with(Shell::default())
    }
}

impl Formatter {
    /// 使用给定的环境变量创建，不运行登录 shell（测试用）。
    pub fn with_vars(vars: Vec<(OsString, OsString)>) -> Self {
        Formatter::with(Shell::with_vars(vars))
    }

    fn with(shell: Shell) -> Self {
        Formatter(Arc::new(Shared {
            shell,
            xcrun: Mutex::new(Vec::new()),
        }))
    }

    /// 运行外部程序所用的环境。
    ///
    /// 格式化以外的外部程序（如 git）也用它，与格式化共用同一次登录 shell 的查询。
    pub fn shell(&self) -> &Shell {
        &self.0.shell
    }

    /// 是否已取到环境：已取到时在 `PATH` 里找程序无需再等待。
    pub fn is_ready(&self) -> bool {
        self.0.shell.is_ready()
    }

    /// 在后台获取环境并更新 `PATH` 的清单，完成后调用 `done`。
    ///
    /// 环境已取到时重新列出清单，反映期间安装或卸载的工具。同时通过 xcrun 查询需要查询才能
    /// 确定是否安装的工具，使设置页能显示 C 和 Swift 所用的工具。
    pub fn prepare(&self, done: impl FnOnce() + Send + 'static) {
        let formatter = self.clone();
        crate::background::spawn(move || {
            // 第一次取到环境时已列出清单，不必再列一次。
            if formatter.is_ready() {
                formatter.0.shell.list();
            } else {
                formatter.0.shell.resolve();
            }
            for name in ["clang-format", "swift-format"] {
                if tools::needs_xcrun(name) && formatter.in_path(name).is_none() {
                    formatter.xcrun(name);
                }
            }
            done();
        });
    }

    /// 返回 `program` 是否已安装；不知道时返回 `None`。
    ///
    /// 按 `PATH` 的清单判断（见 [`Shell::peek`]），不逐个目录探测，不运行别的程序，也不等待，
    /// 因此可以在界面线程上调用（设置页使用）。尚未取到用户 shell 的 `PATH`，或需要先通过
    /// xcrun 查询才能确定的，都算不知道。
    pub fn installed(&self, program: &Program) -> Option<bool> {
        let Program::Search(name) = program else {
            return Some(self.locate(program).is_some());
        };
        if self.0.shell.peek(name)?.is_some() {
            return Some(true);
        }
        if !tools::needs_xcrun(name) {
            return Some(false);
        }
        self.xcrun_asked(name).map(|found| found.is_some())
    }

    /// 返回程序 `name` 是否已安装（在 `PATH` 里找）；不知道时返回 `None`，见
    /// [`Formatter::installed`]。
    pub fn has(&self, name: &str) -> Option<bool> {
        self.installed(&Program::Search(name.to_owned()))
    }

    /// 为看不出用哪个工具的项目确定第一个已安装的工具，其他计划原样返回，需在后台线程调用。
    ///
    /// # Errors
    /// 候选工具一个都没有安装时返回 [`FormatError::NoTool`]。
    ///
    /// # Panics
    /// `Plan::settle` 在回调始终返回 `Some` 时必定返回 `Some`，因此这里的 `expect` 不会触发。
    pub fn resolve(&self, plan: &Plan) -> Result<Plan, FormatError> {
        let found = |name: &str| {
            let program = Program::Search(name.to_owned());
            Some(self.locate_for(&plan.file, &program).is_some())
        };
        plan.settle(found).expect("每个程序都问到了")
    }

    /// 与 [`Formatter::resolve`] 相同，但不等待：还不知道装了哪个时返回 `None`（设置页使用）。
    pub fn peek(&self, plan: &Plan) -> Option<Result<Plan, FormatError>> {
        // 查询发行版要启动进程，这里不等待。
        if crate::wsl::locate_here(&plan.file).is_some() {
            return plan.settle(|_| None);
        }
        plan.settle(|name| self.has(name))
    }

    /// 返回要运行的程序所在的路径；找不到时返回 `None`。
    ///
    /// 在 `PATH` 里找时需要先取到环境，第一次调用会等待。
    pub fn locate(&self, program: &Program) -> Option<PathBuf> {
        match program {
            Program::Project { path, .. } | Program::Path(path) => {
                is_executable(path).then(|| path.clone())
            }
            Program::Search(name) => self.in_path(name).or_else(|| self.xcrun(name)),
        }
    }

    /// 返回格式化 `file` 时要运行的程序：`file` 位于 WSL 发行版中时在发行版内查找，返回发行版内的
    /// 路径；其余情况同 [`Formatter::locate`]。
    fn locate_for(&self, file: &Path, program: &Program) -> Option<PathBuf> {
        let Some(place) = crate::wsl::locate_here(file) else {
            return self.locate(program);
        };
        match program {
            Program::Project { path, .. } | Program::Path(path) => is_executable(path)
                .then(|| crate::wsl::locate_here(path))
                .flatten()
                .map(|program| PathBuf::from(program.path)),
            Program::Search(name) => {
                let path = crate::wsl::login_path(&place.distro);
                crate::wsl::find_program(&place.distro, path.as_deref(), name)
            }
        }
    }

    /// 在 `PATH` 里找程序 `name`，不通过 xcrun 查找；逐个目录检查，应在后台线程调用。
    fn in_path(&self, name: &str) -> Option<PathBuf> {
        self.0.shell.find(name)
    }

    /// 返回之前通过 xcrun 查询得到的结果；没有查询过时返回 `None`。
    fn xcrun_asked(&self, name: &str) -> Option<Option<PathBuf>> {
        let asked = self.0.xcrun.lock().unwrap_or_else(PoisonError::into_inner);
        asked
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, path)| path.clone())
    }

    fn xcrun(&self, name: &str) -> Option<PathBuf> {
        if let Some(path) = self.xcrun_asked(name) {
            return path;
        }
        let path = tools::find_with_xcrun(name);
        let mut asked = self.0.xcrun.lock().unwrap_or_else(PoisonError::into_inner);
        asked.push((name.to_owned(), path.clone()));
        path
    }

    /// 运行格式化工具，检查并整理输出，算出逐行的修改。
    ///
    /// 会阻塞到工具结束、超时或被取消，应在后台线程调用。
    ///
    /// # Errors
    ///
    /// 找不到程序、工具出错退出、超时、取消，或输出无法使用时返回 [`FormatError`]。
    pub fn run(&self, job: Job, cancel: &Cancel) -> Result<Formatted, FormatError> {
        let plan = &self.resolve(&job.plan)?;
        let tool = plan.tool().into_owned();
        let range_args = match &plan.range {
            Some(range) => plan.range_args()?.iter().map(|a| range.expand(a)).collect(),
            None => Vec::new(),
        };
        let program = self.locate_for(&plan.file, &plan.program).ok_or_else(|| {
            let message = match &plan.auto {
                Some((preset, Some(reason))) => {
                    let (preset, reason, tool) = (*preset, reason.as_str(), tool.as_str());
                    t!(
                        "format-preset-tool-not-found",
                        preset = preset,
                        reason = reason,
                        tool = tool
                    )
                }
                _ => t!("format-tool-not-found", tool = tool.as_str()),
            };
            match &plan.program {
                Program::Search(name) => FormatError::missing(message, name),
                _ => FormatError::missing(message, ""),
            }
        })?;

        // 发行版内的工具只接受发行版内的路径。
        let place = crate::wsl::locate_here(&plan.file);
        let file = place
            .as_ref()
            .map_or_else(|| plan.file.clone(), |place| PathBuf::from(&place.path));
        let dir = plan.file.parent().unwrap_or(Path::new("/"));
        let edition = plan
            .args
            .iter()
            .any(|arg| arg.contains(command::RUST_EDITION))
            .then(|| tools::rust_edition(dir))
            .flatten();
        let args: Vec<_> = plan.args[1..]
            .iter()
            .chain(&range_args)
            .map(|arg| command::expand(arg, &file, edition.as_deref()))
            .collect();
        // 工作目录取文件所在的目录（工具从这里向上查找配置）；该目录已被删除时取向上第一个
        // 仍存在的目录。
        let cwd = dir
            .ancestors()
            .find(|d| d.is_dir())
            .unwrap_or(Path::new("/"));

        let mut command = match place.zip(crate::wsl::locate_here(cwd)) {
            Some((place, cwd)) => {
                let mut command = self.0.shell.command(Path::new("wsl.exe"));
                let path = crate::wsl::login_path(&place.distro);
                crate::wsl::enter_login(&mut command, &place.distro, &cwd.path, path.as_deref());
                command.env("WSLENV", crate::wsl::FORMAT_ENV).arg(&program);
                command
            }
            None => {
                let mut command = self.0.shell.command(&program);
                command.current_dir(cwd);
                command
            }
        };
        command
            .args(&args)
            .env("WINC_FILE", &file)
            .stderr(Stdio::piped());

        // 工具收到的内容统一为 LF，使只用 CR 换行的旧文件不会被当作一整行；输出再转换回文件
        // 原有的换行符。`old` 留着和输出比较；送去格式化的输入直接移交给写标准输入的线程。
        let old = job.text.to_string();
        let input = unify_newlines(&old, "\n").into_owned().into_bytes();
        let output = match process::run(&mut command, input, job.timeout, cancel) {
            Ok(output) => output,
            Err(RunError::Spawn(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                let message = t!("format-tool-not-found", tool = tool.as_str());
                return Err(FormatError::missing(message, &tool));
            }
            Err(RunError::Spawn(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(FormatError::Failed(t!(
                    "format-no-permission",
                    tool = tool.as_str()
                )));
            }
            Err(RunError::Spawn(e)) => {
                let (tool, error) = (tool.as_str(), e.to_string());
                return Err(FormatError::Failed(t!(
                    "format-spawn-failed",
                    tool = tool,
                    error = error
                )));
            }
            Err(RunError::Timeout) => {
                let (tool, count) = (tool.as_str(), job.timeout.as_secs_f32().round() as u64);
                return Err(FormatError::Failed(t!(
                    "format-timeout",
                    tool = tool,
                    count = count
                )));
            }
            Err(RunError::TooLarge) => {
                let (tool, size) = (tool.as_str(), MAX_OUTPUT >> 20);
                let message = t!("format-output-too-large", tool = tool, size = size);
                return Err(FormatError::Failed(message));
            }
            Err(RunError::Cancelled) => return Err(FormatError::Cancelled),
        };

        let stderr = String::from_utf8_lossy(&output.stderr);
        let name = display_name(&plan.file);
        let transcript = || {
            let status = match output.status.code() {
                Some(code) => t!("format-exit-code", code = code),
                None => t!("format-killed-by-signal").to_owned(),
            };
            let command = std::iter::once(tool.as_str().into())
                .chain(args.iter().map(|a| a.to_string_lossy()))
                .map(|a| {
                    if a.contains(' ') {
                        format!("'{a}'").into()
                    } else {
                        a
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let note = t!("format-transcript-note", status = status);
            format!("$ {command} < {name}\n{note}\n\n{stderr}")
        };
        // ESLint 不直接输出修复后的内容，见 `eslint_output`。仍有无法自动修复的问题时退出码是 1，
        // 不算失败。按参数 `--fix-dry-run` 识别，而不是按程序名：项目里的路径是
        // `node_modules/.bin/eslint`，Windows 上是 `eslint.cmd`，用户也可能在外面套一层 `npx`。
        let eslint = plan.args.iter().any(|a| a == "--fix-dry-run");
        let finished = output.status.success() || (eslint && output.status.code() == Some(1));
        if !finished {
            for line in stderr.lines() {
                eprintln!("winc: {tool}: {line}");
            }
            let message =
                summary(&stderr, &tool, &name).unwrap_or_else(|| match output.status.code() {
                    Some(code) => t!("format-tool-error", tool = tool.as_str(), code = code),
                    None => t!("format-tool-exited", tool = tool.as_str()),
                });
            return Err(FormatError::Tool {
                message,
                output: transcript(),
            });
        }
        if output.stdout.is_empty() && !old.is_empty() {
            let message = t!("format-no-output", tool = tool.as_str());
            return Err(if stderr.trim().is_empty() {
                FormatError::Failed(message)
            } else {
                FormatError::Tool {
                    message,
                    output: transcript(),
                }
            });
        }
        let Ok(formatted) = String::from_utf8(output.stdout) else {
            let message = t!("format-output-not-utf8", tool = tool.as_str());
            return Err(FormatError::Failed(message));
        };

        let formatted = if eslint {
            eslint_output(&formatted, &unify_newlines(&old, "\n"), &tool)?
        } else {
            formatted
        };
        let formatted = formatted.strip_prefix('\u{feff}').unwrap_or(&formatted);
        let new = unify_newlines(formatted, job.line_ending.as_str());
        let changes = diff::line_changes(&old, &new);
        let lines = changes
            .iter()
            .map(|c| line_count(&c.removed).max(line_count(&c.inserted)))
            .sum();
        Ok(Formatted {
            changes,
            lines,
            tool,
        })
    }
}

/// 解析 `ESLint` 的 `--fix-dry-run --format json` 输出：修复过的内容在 `output` 字段里，
/// 没有该字段表示没有可修复的问题；无法解析的文件（语法错误）在 `messages` 中标记为 `fatal`。
///
/// # Errors
///
/// 输出不是预期的 JSON，或文件有语法错误（`fatal`）时返回 [`FormatError::Failed`]。
fn eslint_output(json: &str, input: &str, tool: &str) -> Result<String, FormatError> {
    let parsed = Json::parse(json);
    let result = parsed
        .as_ref()
        .and_then(|json| json.as_array()?.first()?.as_object())
        .ok_or_else(|| FormatError::Failed(t!("format-eslint-unreadable", tool = tool)))?;
    if let Some(output) = result.get("output").and_then(Json::as_str) {
        return Ok(output.to_owned());
    }
    let fatal = result
        .get("messages")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Json::as_object)
        .find(|m| m.get("fatal").and_then(Json::as_bool) == Some(true));
    if let Some(fatal) = fatal {
        let number = |key| fatal.get(key).and_then(Json::as_number).unwrap_or(0.0);
        let message = fatal
            .get("message")
            .and_then(Json::as_str)
            .unwrap_or(t!("format-eslint-parse-error"));
        let (line, column) = (number("line"), number("column"));
        let message = t!(
            "format-eslint-fatal",
            message = message,
            line = line,
            column = column
        );
        return Err(FormatError::Failed(message));
    }
    Ok(input.to_owned())
}

/// 把 `\r\n`、`\r`、`\n` 都换成 `ending`。
fn unify_newlines<'a>(text: &'a str, ending: &str) -> Cow<'a, str> {
    if !text.contains('\r') && ending == "\n" {
        return Cow::Borrowed(text);
    }
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    Cow::Owned(if ending == "\n" {
        unified
    } else {
        unified.replace('\n', ending)
    })
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// 返回工具出错时提示给用户的一句话，取自标准错误中第一行有内容的。
///
/// 去掉颜色控制符和 `error:` 这类前缀，把 `<standard input>` 换成文件名；这一行没写位置、
/// 而后几行里有位置时（`rustfmt` 写在第二行），把位置补在末尾。标准错误为空时返回 `None`。
fn summary(stderr: &str, tool: &str, file_name: &str) -> Option<String> {
    // 只需要前几行：标准错误最多可达 `MAX_OUTPUT`（64 MiB），不整个复制。
    let lines: Vec<String> = stderr
        .lines()
        .map(strip_ansi)
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();
    let first = lines.first()?.trim();

    let mut message = first;
    loop {
        let lower = message.to_ascii_lowercase();
        let prefix = [
            format!("{}:", tool.to_ascii_lowercase()),
            "error:".to_owned(),
            "[error]".to_owned(),
        ]
        .into_iter()
        .find(|p| lower.starts_with(p.as_str()));
        match prefix {
            Some(p) => message = message[p.len()..].trim_start(),
            None => break,
        }
    }
    let mut message = message
        .replace("<standard input>", file_name)
        .replace("<stdin>", file_name);

    if position(first).is_none()
        && let Some((line, column)) = lines[1..].iter().take(4).find_map(|l| position(l))
    {
        message.push_str(&t!("format-error-position", line = line, column = column));
    }
    Some(message)
}

/// 返回一行中形如 `<stdin>:12:5` 的位置（行，列）；没有时返回 `None`。
fn position(line: &str) -> Option<(usize, usize)> {
    ["<stdin>:", "<standard input>:", "stdin:"]
        .iter()
        .find_map(|marker| {
            let rest = &line[line.find(marker)? + marker.len()..];
            let mut parts = rest.split(':');
            let line = parts.next()?.trim().parse().ok()?;
            let column = parts
                .next()?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()?;
            Some((line, column))
        })
}

/// 去掉 ANSI 颜色控制符（`ESC [ … 字母`）。
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

impl Document {
    /// 返回要交给格式化工具的内容。
    ///
    /// # Errors
    ///
    /// 文档只读（含预览、二进制、无法解码）或属于大文件时返回原因。
    pub fn format_source(&self) -> Result<Rope, String> {
        if let Some(reason) = self.readonly_reason() {
            return Err(reason.to_owned());
        }
        match &self.text {
            Content::Rope(rope) => Ok(rope.clone()),
            Content::Large(_) => Err(t!("format-file-too-large").to_owned()),
        }
    }

    /// 用格式化的结果替换内容：算一次修改，⌘Z 一次还原；选区跟随原来的文字移动，并重新检测缩进。
    ///
    /// `top` 是窗口第一行的行首位置（字符偏移），与选区一起换算，返回换算后的位置。
    ///
    /// 内容与送去格式化时不一样（期间又被修改，或从磁盘重新加载过）时不做任何改动，返回 `None`。
    /// 不能用撤销记录的版本号判断：重新加载会清空撤销记录，版本号可能回到原来的值。
    pub fn apply_formatted(
        &mut self,
        sent: &Rope,
        changes: Vec<Change>,
        top: usize,
    ) -> Option<usize> {
        if self.text.rope() != Some(sent) || self.readonly {
            return None;
        }
        if changes.is_empty() {
            return Some(top);
        }

        let before = self.selection;
        // `map_positions` 按输入的数量返回位置，因此这里正好取到三个。
        let mapped = diff::map_positions(&changes, &[before.anchor, before.head, top]);
        let after = Selection::new(mapped[0], mapped[1]);
        self.apply(Transaction::new(changes, before, after, EditKind::Other));
        self.goal_cell = None;
        self.detected_indent = indent::detect(&self.text);
        Some(mapped[2])
    }
}

#[cfg(test)]
pub(crate) mod tests;
