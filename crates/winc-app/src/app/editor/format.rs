//! 格式化和修复文档（`⇧⌥F`、右键菜单、保存时）：在后台运行外部工具，结果返回时作为一次修改应用。

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};

use ropey::Rope;
use winc_core::Change;
use winc_editor::format::{
    Cancel, FormatError, FormatRange, Formatted, Formatter, Job, Plan, Program, Task,
};
use winc_i18n::t;

use super::*;

use crate::app::scroll::set_scroll_line;

/// 手动格式化、修复的最长等待时间。
///
/// 工具在后台运行，期间可以继续输入（输入后结果作废），因此上限放宽：新装的工具首次运行较慢
/// （系统需要先完成检查，node 需要先生成编译缓存），实测可能超过 10 秒。
const TIMEOUT: Duration = Duration::from_secs(30);
/// 保存前修复、格式化各自的最长等待时间。
///
/// 取自调参：按下 `⌘S` 后等待过久会拖延保存。
const SAVE_TIMEOUT: Duration = Duration::from_secs(3);
/// 任务运行超过该时长才在状态栏显示「正在格式化…」或「正在修复…」。
///
/// 取自调参：避免工具很快完成时，状态栏内容短暂出现又消失。
const SHOW_PROGRESS: Duration = Duration::from_millis(500);

/// 后台线程送回的结果：文档及其格式化结果。
type Outcome = (DocumentId, Result<Formatted, FormatError>);

/// 返回设置 `config` 中 `task`（格式化或修复）的各语言命令，键为语言 ID。
pub(in crate::app) fn task_commands(config: &Config, task: Task) -> &BTreeMap<String, String> {
    match task {
        Task::Format => &config.format_commands,
        Task::Fix => &config.fix_commands,
    }
}

/// 返回设置 `config` 中 `task`（格式化或修复）的各语言命令的可变引用。
pub(in crate::app) fn task_commands_mut(
    config: &mut Config,
    task: Task,
) -> &mut BTreeMap<String, String> {
    match task {
        Task::Format => &mut config.format_commands,
        Task::Fix => &mut config.fix_commands,
    }
}

/// 返回设置 `config` 中保存时执行 `task` 的开关（「保存时格式化」或「保存时修复问题」）；
/// 没有写时为 `None`。
fn save_switch(config: &Config, task: Task) -> Option<bool> {
    match task {
        Task::Format => config.format_on_save,
        Task::Fix => config.fix_on_save,
    }
}

/// 格式化的状态：正在后台运行的任务，以及后台线程送回结果的通道。
pub struct FormatState {
    /// 查找并运行外部格式化工具的格式化器。
    pub formatter: Formatter,
    /// 关闭标签或退出时选择了「保存」，正在为其中一个文件格式化：保存完成后继续处理其余文件。
    pub then_close: Option<SavePrompt>,
    running: Vec<Running>,
    /// 正在后台查找工具的文档，以及完成后是否继续保存：远端的文件要读那台主机上的项目文件
    /// 才知道用哪个工具，不能在界面线程上进行。
    planning: Vec<(DocumentId, bool)>,
    /// 后台查好的工具送回的通道。
    planned: (Sender<Planned>, Receiver<Planned>),
    /// 保存前完成修复和格式化的文件，保存后在状态栏显示的提示（例如「使用 ruff 修复了 2 行」）。
    notes: Vec<(DocumentId, String)>,
    /// 后台线程送回结果的通道的发送端。
    sender: Sender<Outcome>,
    /// 后台线程送回结果的通道的接收端。
    receiver: Receiver<Outcome>,
}

/// 一个正在后台格式化（修复）的文档。
struct Running {
    /// 正在处理的文档。
    id: DocumentId,
    /// 任务种类：格式化或修复。
    task: Task,
    /// 送去格式化时的内容。
    ///
    /// 结果返回时文档内容已变化则不应用结果。
    sent: Rope,
    /// 完成后是否继续保存（`⌘S` 时启用了「保存时格式化」或「保存时修复问题」）。
    save: bool,
    /// 是否只格式化选中的部分。
    partial: bool,
    /// 开始运行的时刻，用来决定何时在状态栏显示进度（[`SHOW_PROGRESS`]）。
    started: Instant,
    /// 状态栏是否已经显示「正在格式化…」。
    shown: bool,
    /// 取消令牌，用于停止工具进程。
    cancel: Cancel,
}

impl FormatState {
    /// 创建格式化状态，`formatter` 为使用的格式化器。
    pub fn new(formatter: Formatter) -> Self {
        let (sender, receiver) = mpsc::channel();
        FormatState {
            formatter,
            then_close: None,
            running: Vec::new(),
            planning: Vec::new(),
            planned: mpsc::channel(),
            notes: Vec::new(),
            sender,
            receiver,
        }
    }
}

/// 一次格式化在选好工具之后要用到的内容。
struct Launch {
    task: Task,
    id: DocumentId,
    /// 完成后是否继续保存。
    save: bool,
    /// 只处理的区段；`None` 是整个文档。
    range: Option<FormatRange>,
    /// 送去格式化的内容。
    sent: Rope,
    line_ending: winc_core::LineEnding,
    /// 文件是否在远端主机上。
    remote: bool,
    root: Option<PathBuf>,
    /// 项目设置中为这种语言写的命令。
    project: Option<String>,
}

/// 后台为一次格式化选好的工具，或选不出的原因。
type Planned = (Launch, Result<Plan, FormatError>);

/// 运行前需要用户确认的对象：项目设置（`.winc/config.json`）中写的命令，或项目自带的程序
/// （`node_modules/.bin/prettier`）。
///
/// 打开陌生的仓库时，直接运行其中的程序不安全。
struct Approval {
    /// 用户的选择按此目录记录：项目自带程序所在的目录，或项目的 `.winc` 目录。
    folder: PathBuf,
    /// 提示框中对象的描述，例如「项目中的「prettier」」「项目设置中的修复命令」。
    what: String,
    /// 提示框主句下方的说明小字。
    detail: String,
    /// 用户不同意时是否可以改用系统中安装的同名程序。
    ///
    /// 命令中以相对路径指定的程序和项目设置中的命令不可以改用。
    searchable: bool,
    /// 是否询问项目设置开启的保存时修复或格式化（而非某个程序）。
    ///
    /// 此时按钮为「允许」「不允许」；选择「不允许」同样记录，之后保存时不再询问，也不执行。
    on_save: bool,
}

/// 提示框上对项目自带工具的回答。
///
/// `run` 为 `Some(true)` 表示运行该工具，`Some(false)` 表示改用系统中安装的同名程序，
/// `None` 表示取消。
#[derive(Debug, Clone)]
pub struct ToolAnswer {
    /// 发起询问的文档。
    pub id: DocumentId,
    /// 任务种类：格式化或修复。
    pub task: Task,
    /// 完成后是否继续保存。
    pub save: bool,
    /// 仅处理的区段（格式化选中部分时）。
    pub range: Option<FormatRange>,
    /// 记录用户选择的目录（见 `Approval::folder`）。
    pub folder: PathBuf,
    /// 用户的回答。
    pub run: Option<bool>,
}

impl App {
    /// 格式化当前文档（`⇧⌥F`、右键菜单中的「格式化文档」）。
    pub(in crate::app) fn format_active(&mut self) {
        self.format_current(false);
    }

    /// 格式化选中部分（右键菜单、命令面板）。
    pub(in crate::app) fn format_selection(&mut self) {
        self.format_current(true);
    }

    /// 用 lint 工具修复当前文档中能自动修复的问题（右键菜单中的「修复问题」）。
    pub(in crate::app) fn fix_active(&mut self) {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return;
        };
        if let Err(e) = self.start(Task::Fix, id, false, None) {
            self.task_failed(Task::Fix, id, &e);
        }
    }

    /// 格式化当前文档；`selection` 为 `true` 时只处理选中部分。
    fn format_current(&mut self, selection: bool) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        let range = selection
            .then(|| doc.text().rope())
            .flatten()
            .map(|rope| FormatRange::new(rope, doc.selection.from(), doc.selection.to()));
        if let Err(e) = self.start(Task::Format, id, false, range) {
            self.task_failed(Task::Format, id, &e);
        }
    }

    /// 是否需要在保存（`⌘S`）前先修复或格式化文档 `id`。
    pub(in crate::app) fn prepares_on_save(&self, id: DocumentId) -> bool {
        let project = self.project_format();
        self.on_save_with(project.as_ref(), Task::Fix, id)
            || self.on_save_with(project.as_ref(), Task::Format, id)
    }

    /// 是否需要在保存（`⌘S`）前对文档 `id` 先执行 `task`（见 [`App::on_save_with`]）。
    fn on_save(&self, task: Task, id: DocumentId) -> bool {
        self.on_save_with(self.project_format().as_ref(), task, id)
    }

    /// 是否需要在保存（`⌘S`）前对文档 `id` 先执行 `task`；`project` 是项目设置。
    ///
    /// 条件是：开启了对应的开关，且该语言没有被排除（被排除的语言修复和格式化都不做）。
    /// 项目设置（`.winc/config.json`）优先于用户设置，项目中排除的语言同样不做。
    fn on_save_with(&self, project: Option<&Config>, task: Task, id: DocumentId) -> bool {
        let Some(doc) = self.editor.document(id) else {
            return false;
        };
        let language = doc.highlighter.language().id;
        let switch = |config: &Config| save_switch(config, task);
        let on = project.and_then(switch).or(switch(&self.config));
        let excluded =
            |config: &Config| config.format_on_save_exclude.iter().any(|l| l == language);
        on == Some(true) && !excluded(&self.config) && !project.is_some_and(excluded)
    }

    /// 保存时的 `task` 由项目设置 `project` 开启、而用户未开启时，返回记录用户选择所用的键；
    /// 否则返回 `None`。
    ///
    /// 打开陌生的仓库后按 `⌘S` 不能自动运行用户不知情的工具：工具会加载项目中的配置，
    /// ESLint、Prettier 的配置本身是可执行的 JS。
    fn project_on_save(&self, project: Option<&Config>, task: Task) -> Option<PathBuf> {
        let on = save_switch(project?, task) == Some(true);
        (on && save_switch(&self.config, task) != Some(true)).then(|| {
            let key = match task {
                Task::Format => "formatOnSave",
                Task::Fix => "fixOnSave",
            };
            self.editor
                .root()
                .unwrap_or(Path::new(""))
                .join(".winc")
                .join(key)
        })
    }

    /// 返回已打开目录中的格式化设置（`.winc/config.json` 的 `format` 一节）。
    ///
    /// 未打开目录或没有该文件时返回 `None`；无法识别的内容在标准错误输出中提示。
    pub(in crate::app) fn project_format(&self) -> Option<Config> {
        let (config, problems) = Config::load_project(self.editor.root()?)?;
        for problem in problems {
            eprintln!("winc: .winc/config.json：{problem}");
        }
        Some(config)
    }

    /// 执行保存时的修复和格式化，之后保存。
    ///
    /// 先修复（修复后可能需要重新排版），再格式化；全部完成或无法执行后再保存。
    pub(in crate::app) fn format_then_save(&mut self, id: DocumentId) {
        if self.on_save(Task::Fix, id) {
            if let Err(e) = self.start(Task::Fix, id, true, None) {
                self.save_step_done(Task::Fix, id, note_for(Task::Fix, e));
            }
        } else {
            self.save_step_done(Task::Fix, id, None);
        }
    }

    /// 处理保存前某一步的结束（完成或未能执行），`note` 为保存后要显示的提示。
    ///
    /// 修复后继续格式化，最后执行保存。
    fn save_step_done(&mut self, task: Task, id: DocumentId, note: Option<String>) {
        if let Some(note) = note {
            self.format.notes.push((id, note));
        }
        if task == Task::Fix && self.on_save(Task::Format, id) {
            match self.start(Task::Format, id, true, None) {
                Ok(()) => return,
                Err(e) => {
                    if let Some(note) = note_for(Task::Format, e) {
                        self.format.notes.push((id, note));
                    }
                }
            }
        }
        self.save_with_notes(id);
    }

    /// 开始格式化（或修复）文档 `id`。
    ///
    /// `save` 为 `true` 时完成后继续保存，给出 `range` 时只处理该区段；
    /// 返回 `Ok` 表示后续步骤已交给后台任务。
    ///
    /// # Errors
    ///
    /// 未能开始时返回 [`FormatError`]：文档不可格式化、没有可用的工具、命令引用的程序不存在，
    /// 或用户选择不运行项目自带的工具。
    fn start(
        &mut self,
        task: Task,
        id: DocumentId,
        save: bool,
        range: Option<FormatRange>,
    ) -> Result<(), FormatError> {
        if let Some(running) = self.format.running.iter_mut().find(|r| r.id == id) {
            running.save |= save;
            return Ok(());
        }
        if let Some(planning) = self.format.planning.iter_mut().find(|p| p.0 == id) {
            planning.1 |= save;
            return Ok(());
        }

        let project = self.project_format();
        if save && let Some(folder) = self.project_on_save(project.as_ref(), task) {
            match self.state.tool_choice(&folder) {
                Some(true) => {}
                // 不允许：本次保存仍正常进行，但不修复、不格式化，也不提示。
                Some(false) => return Err(FormatError::Cancelled),
                None => {
                    let approval = Approval {
                        folder,
                        what: t!("format-approval-on-save-what", task = task.key()),
                        detail: t!("format-approval-on-save-detail", task = task.key()),
                        searchable: false,
                        on_save: true,
                    };
                    return self.ask_approval(task, id, save, None, approval);
                }
            }
        }

        let Some(doc) = self.editor.document(id) else {
            return Err(FormatError::Cancelled);
        };
        let sent = doc.format_source().map_err(FormatError::Unavailable)?;
        let language = doc.highlighter.language();
        let line_ending = doc.line_ending;
        let root = self.editor.root().map(Path::to_path_buf);
        let command = |config: &Config| task_commands(config, task).get(language.id).cloned();
        let project = project.as_ref().and_then(command);
        let configured = project.clone().or_else(|| command(&self.config));
        let path = match doc.path.clone() {
            Some(path) => path,
            None if language.id == "text" => {
                return Err(FormatError::Unavailable(
                    t!("format-unsaved-no-language").to_owned(),
                ));
            }
            // 工具需要读取磁盘上的文件，未保存过的文件在磁盘上不存在。
            None if configured
                .as_deref()
                .is_some_and(Plan::reads_file_from_disk) =>
            {
                let (language, task) = (language.name, task.key());
                let message = t!(
                    "format-unsaved-reads-disk",
                    language = language,
                    task = task
                );
                return Err(FormatError::Unavailable(message));
            }
            None => unsaved_path(language, root.as_deref()),
        };
        let launch = Launch {
            task,
            id,
            save,
            range,
            sent,
            line_ending,
            remote: winc_backend::is_remote(&path),
            root,
            project,
        };
        // 远端的文件：选工具要读那台主机上的项目文件，在后台进行，选好后由
        // [`App::poll_formatting`] 接着处理。
        if launch.remote {
            self.format.planning.push((id, save));
            let sender = self.format.planned.0.clone();
            self.spawn_blocking_then(UserEvent::FormatPlanned, move || {
                let root = launch.root.as_deref();
                let plan = Plan::for_task(task, language, &path, root, configured.as_deref());
                let _ = sender.send((launch, plan));
            });
            return Ok(());
        }
        let root = launch.root.as_deref();
        let plan = Plan::for_task(task, language, &path, root, configured.as_deref())?;
        self.launch(launch, plan)
    }

    /// 用选好的工具 `plan` 开始格式化：必要时先询问用户能否运行，再交给后台。
    ///
    /// # Errors
    ///
    /// 同 [`App::start`]。
    fn launch(&mut self, launch: Launch, mut plan: Plan) -> Result<(), FormatError> {
        let Launch {
            task,
            id,
            save,
            range,
            sent,
            line_ending,
            remote,
            root,
            project,
        } = launch;
        if let Some(range) = &range {
            plan.restrict(range)?;
        }

        // 命令中写的相对路径指向不存在的程序时，无需询问能否运行，直接报错。远端的程序不能在
        // 界面线程上查看，运行时找不到再报错。
        if matches!(plan.program, Program::Project { .. })
            && !remote
            && self.format.formatter.installed(&plan.program) == Some(false)
        {
            return Err(FormatError::Missing {
                message: t!("format-tool-not-found", tool = plan.tool().into_owned()),
                tools: Vec::new(),
            });
        }
        if let Some(approval) = approval(&plan, root.as_deref(), project.as_deref()) {
            match self.state.tool_choice(&approval.folder) {
                Some(true) => {}
                Some(false) if approval.searchable => plan.use_system(),
                Some(false) => {
                    let what = approval.what;
                    return Err(FormatError::Failed(t!(
                        "format-approval-declined",
                        what = what
                    )));
                }
                None => return self.ask_approval(task, id, save, range, approval),
            }
        }

        let cancel = Cancel::default();
        self.format.running.push(Running {
            id,
            task,
            sent: sent.clone(),
            save,
            partial: plan.is_restricted(),
            started: Instant::now(),
            shown: false,
            cancel: cancel.clone(),
        });
        let job = Job {
            plan,
            text: sent,
            line_ending,
            timeout: if save { SAVE_TIMEOUT } else { TIMEOUT },
        };
        let (formatter, sender) = (self.format.formatter.clone(), self.format.sender.clone());
        let proxy = self.link.proxy.clone();
        // 格式化工具最长运行 `TIMEOUT`（30 秒），在单独的线程上等待，不占用共用的线程池。
        winc_editor::background::spawn_blocking(move || {
            let result = formatter.run(job, &cancel);
            if sender.send((id, result)).is_ok()
                && let Some(proxy) = proxy
            {
                let _ = proxy.send_event(UserEvent::Formatted);
            }
        });
        Ok(())
    }

    /// 在项目设置中的命令或项目自带的程序首次运行前弹出提示框，询问是否允许运行。
    ///
    /// # Errors
    ///
    /// 没有窗口（测试中）、非 macOS 平台，或窗口上已有其他提示框、面板而无法弹出时，
    /// 返回 [`FormatError::Failed`]，本次不格式化。
    fn ask_approval(
        &mut self,
        task: Task,
        id: DocumentId,
        save: bool,
        range: Option<FormatRange>,
        approval: Approval,
    ) -> Result<(), FormatError> {
        if self.window.handle.is_some() {
            use crate::app::AlertButton;

            let message = if approval.on_save {
                t!(
                    "format-approval-allow-question",
                    what = approval.what.as_str()
                )
            } else {
                t!(
                    "format-approval-run-question",
                    what = approval.what.as_str()
                )
            };
            let mut buttons = vec![AlertButton::new(if approval.on_save {
                t!("format-approval-allow")
            } else {
                t!("format-approval-run")
            })];
            if approval.searchable {
                buttons.push(AlertButton::new(t!("format-approval-use-system")));
            }
            buttons.push(AlertButton::cancel(if approval.on_save {
                t!("format-approval-deny")
            } else {
                t!("common-cancel")
            }));
            // 保存时的询问没有「取消」：「不允许」同样是需要记录的回答。
            let cancel = if approval.on_save {
                usize::MAX
            } else {
                buttons.len() - 1
            };
            let detail = &approval.detail;
            let folder = approval.folder.clone();
            let asked = self.present_alert(&message, detail, buttons, None, move |i, _| {
                let answer = ToolAnswer {
                    id,
                    task,
                    save,
                    range: range.clone(),
                    folder: folder.clone(),
                    run: (i != cancel).then_some(i == 0),
                };
                Some(UserEvent::ToolChoice(answer))
            });
            // 窗口上已有其他提示框或面板时无法弹出，视为本次不格式化。
            if asked {
                return Ok(());
            }
        }

        let what = approval.what;
        Err(FormatError::Failed(t!(
            "format-approval-needed",
            what = what
        )))
    }

    /// 处理用户在提示框中对是否运行项目自带程序的回答 `answer`：记录该选择并继续格式化。
    ///
    /// 取消时仅本次不格式化，保存仍然进行。
    pub(in crate::app) fn answer_tool_choice(&mut self, answer: ToolAnswer) {
        let ToolAnswer { id, task, save, .. } = answer;
        let Some(run) = answer.run else {
            if save {
                self.save_step_done(task, id, None);
            }
            return;
        };

        self.update_state(|state| state.set_tool_choice(&answer.folder, run));
        if let Err(e) = self.start(task, id, save, answer.range) {
            if save {
                self.save_step_done(task, id, note_for(task, e));
            } else {
                self.task_failed(task, id, &e);
            }
        }
    }

    /// 接收后台线程送回的格式化结果。
    pub(in crate::app) fn poll_formatting(&mut self) {
        // 后台选好了工具：接着询问、运行；期间又要求保存的一并保存。
        while let Ok((mut launch, plan)) = self.format.planned.1.try_recv() {
            let (task, id) = (launch.task, launch.id);
            let waiting = self.format.planning.iter().position(|p| p.0 == id);
            launch.save |= waiting.is_some_and(|at| self.format.planning.swap_remove(at).1);
            let save = launch.save;
            if let Err(e) = plan.and_then(|plan| self.launch(launch, plan)) {
                if save {
                    self.save_step_done(task, id, note_for(task, e));
                } else {
                    self.task_failed(task, id, &e);
                }
            }
        }
        while let Ok((id, result)) = self.format.receiver.try_recv() {
            self.formatting_done(id, result);
        }
    }

    /// 处理文档 `id` 的格式化（修复）结果 `result`：应用修改并在状态栏提示。
    ///
    /// 作为保存流程一部分的任务，处理后继续保存流程。
    fn formatting_done(&mut self, id: DocumentId, result: Result<Formatted, FormatError>) {
        // 标签已关闭或任务已取消时，任务不在 `running` 中，丢弃结果。
        let Some(index) = self.format.running.iter().position(|r| r.id == id) else {
            return;
        };
        let running = self.format.running.remove(index);
        let task = running.task;

        let formatted = match result {
            Ok(formatted) => formatted,
            Err(e) if running.save => return self.save_step_done(task, id, note_for(task, e)),
            Err(e) => return self.task_failed(task, id, &e),
        };
        let (lines, tool) = (formatted.lines, formatted.tool);
        let applied = self.apply_formatted(id, &running.sent, formatted.changes);
        let (key, count) = (task.key(), lines);

        if running.save {
            let note = (applied && lines > 0).then(|| {
                t!(
                    "format-save-note",
                    tool = tool.as_str(),
                    task = key,
                    count = count
                )
            });
            return self.save_step_done(task, id, note);
        }

        let (clean, clean_part) = match task {
            Task::Format => (t!("format-clean"), t!("format-clean-selection")),
            Task::Fix => (t!("format-fix-clean"), t!("format-fix-clean")),
        };
        let clean_named = |name: String| match task {
            Task::Format => t!("format-clean-named", name = name),
            Task::Fix => t!("format-fix-clean-named", name = name),
        };
        let tool = tool.as_str();
        self.flash(match (applied, lines, self.background_name(id)) {
            (false, _, None) => t!("format-modified-during", task = key),
            (false, _, Some(name)) => t!("format-modified-during-named", name = name, task = key),
            (true, 0, None) if running.partial => clean_part.to_owned(),
            (true, 0, None) => clean.to_owned(),
            (true, 0, Some(name)) => clean_named(name),
            (true, _, None) if running.partial => {
                t!(
                    "format-applied-selection",
                    tool = tool,
                    task = key,
                    count = count
                )
            }
            (true, _, None) => t!("format-applied", tool = tool, task = key, count = count),
            (true, _, Some(name)) => {
                t!(
                    "format-applied-named",
                    tool = tool,
                    task = key,
                    name = name,
                    count = count
                )
            }
        });
    }

    /// 用户已切换到其他标签时返回文档 `id` 的名称，使提示能指明对应的文件；
    /// 文档仍为当前标签时返回 `None`。
    fn background_name(&self, id: DocumentId) -> Option<String> {
        if self.editor.active().map(|d| d.id) == Some(id) {
            return None;
        }
        self.editor.document(id).map(|d| d.name().into_owned())
    }

    /// 提示任务失败的原因；已取消的任务不提示。
    ///
    /// 工具出错时附带「查看输出」按钮，工具未安装时附带安装方法的按钮，
    /// 未指定工具时附带打开设置的按钮。
    fn task_failed(&mut self, task: Task, id: DocumentId, error: &FormatError) {
        if *error == FormatError::Cancelled {
            return;
        }
        let (task_key, reason) = (task.key(), error.to_string());
        let message = match self.background_name(id) {
            Some(name) => t!(
                "format-failed-named",
                task = task_key,
                name = name,
                error = reason
            ),
            None => t!("format-failed", task = task_key, error = reason),
        };
        match error {
            FormatError::Tool { output, .. } => {
                self.notify_with(message, NoticeAction::ShowOutput(output.clone()));
            }
            _ if !error.tools_to_install().is_empty() => {
                let dir = self
                    .editor
                    .document(id)
                    .and_then(|d| d.path.as_deref()?.parent().map(Path::to_path_buf))
                    .unwrap_or_default();
                let tools = error.tools_to_install().to_vec();
                self.notify_with(message, NoticeAction::Install { tools, dir });
            }
            FormatError::Unset(_) => self.notify_with(message, NoticeAction::FormatSettings),
            _ => self.notify(message),
        }
    }

    /// 在只读标签中显示 `tools` 的安装方法：只有一个工具时即该工具，有多个时安装其中任意一个即可。
    ///
    /// `dir` 为文件所在的目录，项目内可安装的方式排在前面。
    pub(in crate::app) fn show_install_guide(&mut self, tools: &[String], dir: &Path) {
        let tools: Vec<&str> = tools.iter().map(String::as_str).collect();
        let formatter = &self.format.formatter;
        let has = |program: &str| formatter.has(program) == Some(true);
        let Some(text) = winc_editor::format::install_guide(&tools, dir, has) else {
            return;
        };
        self.editor.show_output(t!("format-install-title"), &text);
        self.focus = Focus::Editor;
        self.relayout();
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 保存文档 `id`，之后在状态栏提示修复、格式化的结果。
    ///
    /// 保存失败时只提示保存的错误。关闭标签或退出的流程正在等待本次保存时，保存成功后继续关闭，
    /// 保存失败则放弃整个关闭流程（处理方式同直接保存失败）。
    fn save_with_notes(&mut self, id: DocumentId) {
        let closing = self.format.then_close.take();
        let (notes, others) = std::mem::take(&mut self.format.notes)
            .into_iter()
            .partition::<Vec<_>, _>(|(d, _)| *d == id);
        self.format.notes = others;
        let Some(name) = self.editor.document(id).map(|d| d.name().into_owned()) else {
            return;
        };
        if !self.save_document(id, None) {
            return;
        }
        if !notes.is_empty() {
            let notes: Vec<String> = notes.into_iter().map(|(_, note)| note).collect();
            let notes = notes.join(t!("format-note-separator"));
            self.flash(t!("format-saved-with-notes", name = name, notes = notes));
        }
        if let Some(prompt) = closing {
            self.save_then_close(prompt);
        }
    }

    /// 是否已把格式化结果 `changes`（相对于送去格式化时的内容 `sent`）应用到文档 `id`。
    ///
    /// 滚动位置保持稳定：光标原先在屏幕上时，仍留在屏幕上的同一显示行；不在屏幕上时，
    /// 原先最上方的行仍在最上方。
    fn apply_formatted(&mut self, id: DocumentId, sent: &Rope, changes: Vec<Change>) -> bool {
        let visible = self.visible_lines() as f64;
        let Some(doc) = self.editor.document_mut(id) else {
            return false;
        };

        // 位置都按显示行计算，已折叠的行不计入。
        let scroll = doc.scroll_line;
        let caret_row = doc.row_at(doc.selection.head) as f64;
        let on_screen = caret_row >= scroll.floor() && caret_row < scroll + visible;
        let top = doc.text().line_start(doc.line_of_row(scroll as usize));
        let Some(top) = doc.apply_formatted(sent, changes, top) else {
            return false;
        };

        let scroll = if on_screen {
            doc.row_at(doc.selection.head) as f64 - (caret_row - scroll)
        } else {
            doc.row_at(top) as f64 + scroll.fract()
        };
        set_scroll_line(
            doc,
            scroll.clamp(0.0, (doc.row_count() as f64 - 2.0).max(0.0)),
        );

        if self.editor.active().map(|d| d.id) == Some(id) {
            self.relayout();
            self.clamp_scroll();
            self.reset_blink();
        }
        self.request_redraw();
        true
    }

    /// 标签关闭时停止该标签的格式化，并丢弃结果。
    pub(in crate::app) fn cancel_format(&mut self, id: DocumentId) {
        if let Some(index) = self.format.running.iter().position(|r| r.id == id) {
            self.format.running.remove(index).cancel.cancel();
        }
        self.format.notes.retain(|(d, _)| *d != id);
    }

    /// 退出前停止所有格式化，避免工具进程残留。
    pub(in crate::app) fn cancel_all_formatting(&mut self) {
        for running in self.format.running.drain(..) {
            running.cancel.cancel();
        }
        self.format.notes.clear();
    }

    /// 文档 `id` 是否正在格式化（修复）。
    pub(in crate::app) fn is_formatting(&self, id: DocumentId) -> bool {
        self.format.running.iter().any(|r| r.id == id)
    }

    /// 返回状态栏应显示进度的任务种类（「正在格式化…」或「正在修复…」）；
    /// 没有需要显示的任务时返回 `None`。
    pub(crate) fn format_in_progress(&self) -> Option<Task> {
        self.format.running.iter().find(|r| r.shown).map(|r| r.task)
    }

    /// 对运行超过 [`SHOW_PROGRESS`] 仍未结束的任务重绘一次，使状态栏显示「正在格式化…」。
    ///
    /// 返回下一次需要唤醒的时刻；没有待显示的任务时返回 `None`。
    pub(in crate::app) fn format_progress_tick(&mut self, now: Instant) -> Option<Instant> {
        let mut next: Option<Instant> = None;
        let mut show = false;
        for running in self.format.running.iter_mut().filter(|r| !r.shown) {
            let at = running.started + SHOW_PROGRESS;
            if now >= at {
                running.shown = true;
                show = true;
            } else {
                next = Some(next.map_or(at, |n| n.min(at)));
            }
        }
        if show {
            self.request_redraw();
        }
        next
    }
}

/// 返回未保存过、语言为 `language` 的文件交给格式化工具时使用的路径。
///
/// 路径形如打开的目录（没有时为临时目录）中的 `untitled.json`。工具只把该路径用作提示
/// （`--stdin-filepath`），并在其所在目录查找配置，不会读取该文件。
fn unsaved_path(language: &winc_syntax::Language, root: Option<&Path>) -> PathBuf {
    let name = match (language.extensions.first(), language.file_names.first()) {
        (Some(ext), _) => format!("untitled.{ext}"),
        (None, Some(name)) => (*name).to_owned(),
        (None, None) => "untitled".to_owned(),
    };
    root.map_or_else(std::env::temp_dir, Path::to_path_buf)
        .join(name)
}

/// 返回本次格式化需要先向用户询问的内容；无需询问时返回 `None`。
///
/// 项目设置中的命令只询问一次，该命令用到的项目自带程序不再另外询问。
fn approval(plan: &Plan, root: Option<&Path>, project: Option<&str>) -> Option<Approval> {
    if let (Some(command), Some(root)) = (project, root) {
        return Some(Approval {
            folder: root.join(".winc"),
            what: t!("format-approval-project-command", task = plan.task().key()),
            detail: t!("format-approval-project-command-detail", command = command),
            searchable: false,
            on_save: false,
        });
    }
    let Program::Project {
        path,
        folder,
        searchable,
    } = &plan.program
    else {
        return None;
    };
    let shown = path.strip_prefix(folder).unwrap_or(path);
    Some(Approval {
        folder: folder.clone(),
        what: t!(
            "format-approval-project-tool",
            tool = plan.tool().into_owned()
        ),
        detail: t!(
            "format-approval-project-tool-detail",
            path = shown.display().to_string()
        ),
        searchable: *searchable,
        on_save: false,
    })
}

/// 返回保存前的修复或格式化没有开始、或失败时，保存后应显示的提示。
///
/// 无法执行（认不出语言、项目没有配置、没装工具）和已取消的情况不提示，返回 `None`。
fn note_for(task: Task, error: FormatError) -> Option<String> {
    match error {
        FormatError::Unavailable(_)
        | FormatError::Unset(_)
        | FormatError::NoTool { .. }
        | FormatError::Cancelled => None,
        error => Some(t!(
            "format-save-step-failed",
            task = task.key(),
            error = error.to_string()
        )),
    }
}

// 仅 Unix 上的测试使用：这些测试用 sh 脚本代替格式化工具，Git 相关的测试也只在 Unix 上运行。
#[cfg(all(test, unix))]
mod tests;
