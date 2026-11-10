//! Git 的表单对话框：「新建工作树」「新建分支」。
//!
//! 窗口正中的功能对话框，填名称、选起点，按「创建」执行；和设置浮层同属模态卡片，各种表单共用一套
//! 状态、几何和绘制，只是行和选项不同（[`FormKind`]）。`git`、`ssh` 的输入请求同样使用该对话框。
//! 按「创建」执行的操作随输入的名称变化（[`FormPlan`]）：新建工作树时可能是新建分支、签出已有的、
//! 跟踪远程的，或者已经在别的工作树里签出着（主按钮换成「打开」）；不能做的原因写在框里。

use std::path::PathBuf;

use winc_editor::git::unique_dir;
use winc_i18n::t;

use super::branch::local_name;
use super::log::short;
use super::question::QuestionView;
use super::rebase::RebaseForm;
use super::worktree::NameTarget;
use crate::app::editor::find::ref_name_field;
use crate::app::{App, Focus, Hover, TextField};
use crate::askpass::Prompt;
use crate::layout::{DialogLayout, DialogPart};

/// 表单对话框是哪一种。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormKind {
    /// 新建工作树：分支、基于、位置，复制被忽略的文件、在新窗口中打开。
    Worktree,
    /// 新建分支：名称、基于，创建后签出。
    ///
    /// `at` 是日志里选的提交（完整的提交号），从它新建时「基于」不能改；`None` 时「基于」可以
    /// 选分支。
    Branch { at: Option<String> },
    /// `git`、`ssh` 的输入请求，见 [`super::question`]。
    Question(Prompt),
    /// 克隆仓库：地址、位置。
    Clone,
    /// 添加远程仓库，或修改 `editing` 的地址。
    Remote { editing: Option<String> },
    /// 交互式变基。
    Rebase(Box<RebaseForm>),
}

impl FormKind {
    /// 返回使用询问对话框版式的表单的内容；其他表单返回 `None`。
    pub(crate) fn view(&self) -> Option<QuestionView> {
        match self {
            FormKind::Question(prompt) => Some(QuestionView::of(prompt)),
            FormKind::Remote { editing } => Some(QuestionView::remote(editing.as_deref())),
            FormKind::Worktree
            | FormKind::Branch { .. }
            | FormKind::Clone
            | FormKind::Rebase(_) => None,
        }
    }
}

/// 表单对话框的状态。
pub(crate) struct FormDialog {
    /// 哪一种。
    pub kind: FormKind,
    /// 名称的输入框；打开对话框时就在这里输入。
    pub name: TextField,
    /// 询问对话框的第二个输入框：登录的密码（遮盖）、远程仓库的地址；其他表单不显示。
    pub second: TextField,
    /// 新分支从哪个分支开始；`None` 是当前分支（游离时是当前提交）。
    pub base: Option<String>,
    /// 用「位置」右端的目录按钮选的目录，工作树建在它下面；`None` 是默认位置（见
    /// [`App::new_worktree_dir`]）。
    pub folder: Option<PathBuf>,
    /// 是否复制配置 `git.worktreeCopy` 匹配的被忽略的文件。
    pub copy: bool,
    /// 工作树建好以后在新窗口打开（`false` 是当前窗口换过去）。
    pub new_window: bool,
    /// 分支建好以后签出它。
    pub checkout: bool,
    /// 鼠标悬停的部件。
    pub hover: Option<DialogPart>,
    /// 键盘焦点所在的部件。
    ///
    /// `Tab` 在名称、基于、选择目录、各个选项和两个按钮之间循环，空格、回车激活该部件；
    /// 焦点在名称（[`DialogPart::Branch`]）上时，键入的文字进入输入框。
    pub focus: DialogPart,
    /// 按下「创建」后仍在执行的 git 操作的编号（`BranchState::started`）。
    ///
    /// 操作完成后才关闭对话框；失败时保留对话框，并将原因写入 `error`。
    pub running: Option<u64>,
    /// 上一次创建失败的原因，显示在按钮左侧；修改名称时清空。
    pub error: Option<String>,
    /// 打开对话框前的焦点，关闭以后还回去。
    back: Focus,
}

/// 按「创建」会做什么，跟着输入的名称、选的起点变。
pub(crate) struct FormPlan {
    /// 不能创建的原因，用错误色写在输入框右端；框里就是名称，原因里不再重复。
    pub problem: Option<String>,
    /// 主按钮的字。
    pub confirm: &'static str,
    /// 主按钮能不能按。
    pub enabled: bool,
    /// 「基于」右边写的字。
    pub base: String,
    /// 能不能选起点。
    pub base_enabled: bool,
    /// 工作树的目录；已经签出着时是那个工作树的目录，建不了、新建分支时为 `None`。
    pub dir: Option<PathBuf>,
}

impl App {
    /// 打开新建工作树对话框。
    pub(in crate::app) fn open_worktree_dialog(&mut self) {
        self.open_form_dialog(FormKind::Worktree, None);
    }

    /// 打开新建分支对话框：`from` 是从哪个分支开始（`None` 是当前分支），`at` 是从日志里哪个
    /// 提交开始（这时「基于」不能改）。
    pub(in crate::app) fn open_branch_dialog(&mut self, from: Option<String>, at: Option<String>) {
        self.open_form_dialog(FormKind::Branch { at }, from);
    }

    /// 打开表单对话框；不在 Git 仓库里时说明。
    ///
    /// 先关闭其他浮层：对话框是模态窗口，盖在最上面。
    fn open_form_dialog(&mut self, kind: FormKind, base: Option<String>) {
        if self.git.status.is_none() {
            self.flash(t!("git-not-a-repository"));
            return;
        }
        if self.form_dialog.is_some() {
            return;
        }
        self.show_form_dialog(kind, ref_name_field(), DialogPart::Branch);
        if let Some(dialog) = &mut self.form_dialog {
            dialog.base = base;
        }
    }

    /// 关闭其他浮层，显示表单对话框：`name` 是它的输入框，`focus` 是一开始的键盘焦点。
    pub(super) fn show_form_dialog(&mut self, kind: FormKind, name: TextField, focus: DialogPart) {
        if self.picker.open {
            self.close_picker();
        }
        if self.goto_input.is_some() {
            self.close_prompt();
        }
        self.finish_name_input();
        self.close_settings();

        let back = match self.focus {
            focus @ (Focus::Editor | Focus::Tree | Focus::CommitList | Focus::Terminal) => focus,
            _ => Focus::Editor,
        };
        self.form_dialog = Some(FormDialog {
            kind,
            name,
            second: TextField::masked(),
            base: None,
            folder: None,
            copy: true,
            new_window: false,
            checkout: true,
            hover: None,
            focus,
            running: None,
            error: None,
            back,
        });
        self.focus = Focus::Dialog;
        self.set_hover(Hover::None);
        self.request_redraw();
    }

    /// 关闭表单对话框，焦点还给打开之前的地方。
    pub(in crate::app) fn close_form_dialog(&mut self) {
        let Some(dialog) = self.form_dialog.take() else {
            return;
        };
        if matches!(dialog.kind, FormKind::Question(_)) {
            self.decline_git_question();
        }
        self.field_preedit.clear();
        if self.focus == Focus::Dialog {
            self.focus = dialog.back;
        }
        self.update_cursor_icon();
        self.request_redraw();
    }

    /// 对话框的几何；窗口太小时卡片缩到窗口以内。
    pub(crate) fn dialog_layout(&self) -> DialogLayout {
        let window = self.layout.window;
        match self.form_dialog.as_ref().map(|d| &d.kind) {
            Some(FormKind::Clone) => DialogLayout::clone_form(window),
            Some(FormKind::Rebase(form)) => {
                DialogLayout::rebase(window, form.rows.len(), form.pushed > 0)
            }
            Some(kind) if let Some(view) = kind.view() => DialogLayout::question(
                window,
                view.message_lines,
                view.values.len(),
                view.inputs.len(),
            ),
            Some(FormKind::Branch { .. }) => {
                DialogLayout::compute(window, false, &[DialogPart::Checkout])
            }
            _ if self.config.git_worktree_copy.is_empty() => {
                DialogLayout::compute(window, true, &[DialogPart::NewWindow])
            }
            _ => DialogLayout::compute(window, true, &[DialogPart::Copy, DialogPart::NewWindow]),
        }
    }

    /// 返回「基于」一行要显示的文字：选了起点时是它的名字，否则是当前分支或当前提交。
    fn base_label(&self, base: Option<&str>) -> String {
        let current = self.git.branches.iter().find(|b| b.current && !b.remote);
        match (base, current) {
            (Some(base), _) => base.to_owned(),
            (None, Some(b)) => t!("git-form-base-current-branch", name = b.name.as_str()),
            (None, None) => t!("git-form-base-current-commit").to_owned(),
        }
    }

    /// 按对话框当前填写的内容，说明「创建」会做什么。
    pub(crate) fn form_plan(&self, dialog: &FormDialog) -> FormPlan {
        match &dialog.kind {
            FormKind::Worktree => self.worktree_plan(dialog),
            FormKind::Branch { at } => self.branch_plan(dialog, at.as_deref()),
            FormKind::Rebase(form) => self.rebase_plan(form, dialog.name.text()),
            FormKind::Clone => self.clone_plan(dialog.name.text().trim(), dialog.folder.as_deref()),
            FormKind::Remote { editing } => {
                self.remote_plan(editing.as_deref(), dialog.name.text(), dialog.second.text())
            }
            FormKind::Question(prompt) => FormPlan {
                problem: None,
                confirm: QuestionView::of(prompt).confirm,
                // 用户名为空的登录请求必然失败；密码和口令允许为空。
                enabled: !matches!(prompt, Prompt::Username { .. })
                    || !dialog.name.text().is_empty(),
                base: String::new(),
                base_enabled: false,
                dir: None,
            },
        }
    }

    /// 新建分支：名称不能和已有的本地分支重名。
    fn branch_plan(&self, dialog: &FormDialog, at: Option<&str>) -> FormPlan {
        let name = dialog.name.text().trim();
        let (base, base_enabled) = match at {
            Some(at) => (t!("git-form-base-commit", commit = short(at)), false),
            None => (self.base_label(dialog.base.as_deref()), true),
        };
        let problem = if name.starts_with('-') {
            Some(t!("git-form-name-leading-dash"))
        } else if (self.git.branches.iter()).any(|b| !b.remote && b.name == name) {
            Some(t!("git-form-name-exists"))
        } else {
            None
        };
        FormPlan {
            enabled: !name.is_empty() && problem.is_none(),
            problem: problem.map(str::to_owned),
            confirm: t!("git-form-create"),
            base,
            base_enabled,
            dir: None,
        }
    }

    /// 新建工作树：按 [`App::worktree_name_target`] 看名称指的是什么。
    fn worktree_plan(&self, dialog: &FormDialog) -> FormPlan {
        let name = dialog.name.text().trim();
        let base = self.base_label(dialog.base.as_deref());
        let dir = |local: &str| {
            let local = if local.is_empty() { "…" } else { local };
            match &dialog.folder {
                Some(folder) => Some(unique_dir(folder, local)),
                None => self.new_worktree_dir(local),
            }
        };
        let plan = |problem: Option<String>| FormPlan {
            enabled: problem.is_none(),
            problem,
            confirm: t!("git-form-create"),
            base: base.clone(),
            base_enabled: true,
            dir: dir(name),
        };
        // 不是新建分支时没有起点可选，「基于」写成灰的。
        let fixed = |plan: FormPlan, text: &str| FormPlan {
            base: text.to_owned(),
            base_enabled: false,
            ..plan
        };
        // 所选目录与仓库不在同一系统中时不允许新建工作树：一侧的 `git` 无法使用另一侧的路径。
        let elsewhere = (dialog.folder.as_deref())
            .zip(self.git.root.as_deref())
            .is_some_and(|(folder, root)| !winc_editor::wsl::same_system(root, folder));
        let creating = |plan: FormPlan| {
            if elsewhere {
                FormPlan {
                    enabled: false,
                    problem: Some(t!("git-form-other-system").to_owned()),
                    ..plan
                }
            } else {
                plan
            }
        };

        if name.is_empty() {
            return creating(FormPlan {
                enabled: false,
                ..plan(None)
            });
        }
        match self.worktree_name_target(name) {
            // `git worktree add -b` 把名字原样交给 `git branch`，「-」开头的会被当成选项。
            NameTarget::New if name.starts_with('-') => FormPlan {
                dir: None,
                ..plan(Some(t!("git-form-name-leading-dash").to_owned()))
            },
            NameTarget::New => creating(plan(None)),
            // 远程分支：签出同名的本地分支，「基于」写跟踪的远程分支。
            NameTarget::Branch(branch) if branch.remote => creating(FormPlan {
                dir: dir(local_name(&branch)),
                ..fixed(plan(None), &branch.name)
            }),
            NameTarget::Branch(_) => {
                creating(fixed(plan(None), t!("git-form-base-existing-branch")))
            }
            // 已在别的工作树中签出：主按钮换成「打开」，位置写那个工作树的目录。
            NameTarget::Held(i) => FormPlan {
                confirm: t!("git-form-open"),
                dir: Some(self.git.worktrees[i].path.clone()),
                ..fixed(plan(None), t!("git-form-base-existing-branch"))
            },
            NameTarget::Current => FormPlan {
                dir: None,
                ..fixed(
                    plan(Some(t!("git-form-current-branch-held").to_owned())),
                    t!("git-form-base-current"),
                )
            },
            NameTarget::Stale => FormPlan {
                dir: None,
                ..fixed(
                    plan(Some(t!("git-form-stale-worktree").to_owned())),
                    t!("git-form-base-existing-branch"),
                )
            },
        }
    }

    /// 按「创建」（或回车）：按 [`FormPlan`] 执行。
    ///
    /// 不能做时什么都不做，原因已经写在输入框里。
    pub(in crate::app) fn submit_form_dialog(&mut self) {
        let Some(dialog) = &self.form_dialog else {
            return;
        };
        if dialog.running.is_some() || !self.form_plan(dialog).enabled {
            return;
        }
        if matches!(dialog.kind, FormKind::Question(_)) {
            self.answer_git_question();
            return;
        }
        // 须在发起操作前记录编号：没有窗口时（测试中）后台操作会同步完成。
        let next = self.branch.started + 1;
        let kind = dialog.kind.clone();
        if let Some(dialog) = &mut self.form_dialog {
            dialog.running = Some(next);
            dialog.error = None;
        }
        let Some(dialog) = &self.form_dialog else {
            return;
        };
        match kind {
            FormKind::Worktree => self.submit_worktree(),
            FormKind::Branch { at } => {
                let name = dialog.name.text().trim().to_owned();
                let from = at.or_else(|| dialog.base.clone());
                let checkout = dialog.checkout;
                self.git_new_branch(name, from, checkout);
            }
            FormKind::Question(_) => {}
            FormKind::Clone => self.submit_clone(),
            FormKind::Rebase(_) => self.submit_rebase(),
            FormKind::Remote { editing } => self.submit_remote(editing),
        }
        // 已发起 git 操作时，待其完成后再关闭（[`App::form_operation_done`]），期间按钮显示
        // 「正在创建…」；未发起操作（上一个操作未完成、直接打开已有的工作树）时不等待。
        if let Some(dialog) = &mut self.form_dialog
            && self.branch.started != next
        {
            dialog.running = None;
        }
        self.request_redraw();
    }

    /// 处理后台完成的编号为 `id` 的 git 操作；操作由对话框发起时返回 `true`。
    ///
    /// 由对话框发起的操作成功时关闭对话框，失败时保留对话框并在按钮左侧显示原因，
    /// 不再另行通知。
    pub(in crate::app) fn form_operation_done(&mut self, id: u64, error: Option<&str>) -> bool {
        let Some(dialog) = self.form_dialog.as_mut().filter(|d| d.running == Some(id)) else {
            return false;
        };
        dialog.running = None;
        match error {
            Some(e) => {
                dialog.error = Some(e.to_owned());
                dialog.focus = DialogPart::Branch;
                self.request_redraw();
            }
            None => self.close_form_dialog(),
        }
        true
    }

    /// 返回键盘焦点可停留的部件，按 `Tab` 的顺序排列。
    ///
    /// 跳过不可用的部件：不可选的起点、不可按的主按钮。
    fn form_focus_order(&self) -> Vec<DialogPart> {
        let Some(dialog) = &self.form_dialog else {
            return Vec::new();
        };
        let plan = self.form_plan(dialog);
        if let FormKind::Rebase(form) = &dialog.kind {
            let mut order = vec![DialogPart::RebaseList];
            if form.rewording() {
                order.push(DialogPart::Branch);
            }
            order.push(DialogPart::Cancel);
            if plan.enabled {
                order.push(DialogPart::Confirm);
            }
            return order;
        }
        let layout = self.dialog_layout();
        let mut order = Vec::new();
        // 无需输入的请求没有输入框；第二个输入框只用于登录和添加远程仓库。
        if layout.branch.w > 0.0 {
            order.push(DialogPart::Branch);
        }
        if layout.second.w > 0.0 {
            order.push(DialogPart::Second);
        }
        if plan.base_enabled {
            order.push(DialogPart::Base);
        }
        if layout.location.is_some() {
            order.push(DialogPart::Change);
        }
        order.extend(layout.options.iter().map(|&(part, _)| part));
        order.push(DialogPart::Cancel);
        if plan.enabled && dialog.running.is_none() {
            order.push(DialogPart::Confirm);
        }
        order
    }

    /// 将键盘焦点移到下一个部件（`back` 时为上一个，对应 `⇧Tab`），到达末尾时从头循环。
    pub(in crate::app) fn form_focus_move(&mut self, back: bool) {
        let order = self.form_focus_order();
        let Some(dialog) = &mut self.form_dialog else {
            return;
        };
        let n = order.len();
        // 焦点不在任何部件上时（无需输入的请求刚打开时），`Tab` 从第一个部件开始。
        let next = match order.iter().position(|&p| p == dialog.focus) {
            Some(at) if back => (at + n - 1) % n,
            Some(at) => (at + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        dialog.focus = order[next];
        self.field_preedit.clear();
        self.request_redraw();
    }

    /// 以空格、回车激活键盘焦点所在的部件（名称除外）。
    pub(in crate::app) fn form_activate(&mut self) {
        let Some(part) = self.form_dialog.as_ref().map(|d| d.focus) else {
            return;
        };
        self.press_form_part(part);
    }

    /// 新建工作树：新建分支、签出已有的，或者打开已经签出着的工作树。
    fn submit_worktree(&mut self) {
        let Some(dialog) = &self.form_dialog else {
            return;
        };
        let name = dialog.name.text().trim().to_owned();
        let (base, folder, new_window) = (
            dialog.base.clone(),
            dialog.folder.clone(),
            dialog.new_window,
        );
        let globs = if dialog.copy {
            self.config.git_worktree_copy.clone()
        } else {
            Vec::new()
        };
        let target = self.worktree_name_target(&name);

        let dir = |app: &App, local: &str| match &folder {
            Some(folder) => Some(unique_dir(folder, local)),
            None => app.new_worktree_dir(local),
        };
        match target {
            NameTarget::Held(i) => {
                self.close_form_dialog();
                self.open_worktree(i, new_window);
            }
            NameTarget::Branch(branch) => {
                let Some(dir) = dir(self, local_name(&branch)) else {
                    return;
                };
                self.checkout_in_worktree(&branch, dir, globs, new_window);
            }
            NameTarget::New => {
                let Some(dir) = dir(self, &name) else {
                    return;
                };
                let target = self.git_path_arg(&dir);
                self.add_worktree(
                    t!("git-op-new-worktree", name = name.as_str()),
                    dir,
                    globs,
                    new_window,
                    move |git, root| {
                        let mut args =
                            vec!["worktree", "add", "-b", &name, "--end-of-options", &target];
                        args.extend(base.as_deref());
                        git.operate(root, &args)
                    },
                );
            }
            NameTarget::Stale | NameTarget::Current => {}
        }
    }

    /// 对话框中窗口坐标 (`x`, `y`) 处的部件；主按钮不能按、起点不能选时不算。
    pub(in crate::app) fn dialog_part_at(&self, x: f32, y: f32) -> Option<DialogPart> {
        let dialog = self.form_dialog.as_ref()?;
        let layout = self.dialog_layout();
        if let FormKind::Rebase(form) = &dialog.kind
            && let Some(part) = self.rebase_part_at(&layout, form, x, y)
        {
            return Some(part);
        }
        let part = layout.part_at(x, y)?;
        let plan = self.form_plan(dialog);
        match part {
            DialogPart::Branch if matches!(&dialog.kind, FormKind::Rebase(form) if !form.rewording()) => {
                None
            }
            DialogPart::Confirm if !plan.enabled || dialog.running.is_some() => None,
            DialogPart::Base if !plan.base_enabled => None,
            part => Some(part),
        }
    }

    /// 处理对话框打开时的鼠标按下：点击卡片外部时关闭对话框，点击部件时执行其操作。
    pub(in crate::app) fn click_form_dialog(&mut self, x: f32, y: f32) {
        let layout = self.dialog_layout();
        if !layout.card.contains(x, y) {
            // 点击卡片外部不关闭询问对话框：关闭表示取消，会使正在进行的操作失败。
            let asking = (self.form_dialog.as_ref())
                .is_some_and(|d| matches!(d.kind, FormKind::Question(_)));
            if !asking {
                self.close_form_dialog();
            }
            return;
        }
        match self.dialog_part_at(x, y) {
            Some(part @ (DialogPart::Branch | DialogPart::Second)) => {
                self.focus = Focus::Dialog;
                if let Some(dialog) = &mut self.form_dialog {
                    dialog.focus = part;
                }
                self.click_field(x, y);
            }
            Some(part) => self.press_form_part(part),
            None => {}
        }
        self.request_redraw();
    }

    /// 执行点击（或以键盘激活）对话框中 `part` 的操作。
    fn press_form_part(&mut self, part: DialogPart) {
        match part {
            DialogPart::Close | DialogPart::Cancel => self.close_form_dialog(),
            DialogPart::Confirm => self.submit_form_dialog(),
            DialogPart::Branch | DialogPart::Second => self.submit_form_dialog(),
            DialogPart::Base => {
                use crate::app::input::context_menu::{MenuTarget, menu_below};
                let base = self.dialog_layout().base;
                self.open_context_menu(MenuTarget::FormBase, menu_below(base));
            }
            DialogPart::Change => self.pick(crate::app::Pick::WorktreeFolder),
            DialogPart::RebaseList => self.submit_form_dialog(),
            part @ (DialogPart::RebaseRow(_)
            | DialogPart::RebaseAction(_)
            | DialogPart::RebaseUp
            | DialogPart::RebaseDown) => self.press_rebase_part(part),
            DialogPart::Copy | DialogPart::NewWindow | DialogPart::Checkout => {
                if let Some(dialog) = &mut self.form_dialog {
                    let option = match part {
                        DialogPart::Copy => &mut dialog.copy,
                        DialogPart::NewWindow => &mut dialog.new_window,
                        _ => &mut dialog.checkout,
                    };
                    *option = !*option;
                }
            }
        }
        self.request_redraw();
    }

    /// 对话框打开时移动鼠标：只认对话框里的部件；在输入框里拖动时选择文字。
    pub(in crate::app) fn hover_form_dialog(&mut self, x: f32, y: f32) {
        let part = self.dialog_part_at(x, y);
        if let Some(dialog) = &mut self.form_dialog
            && dialog.hover != part
        {
            dialog.hover = part;
            self.request_redraw();
        }
        self.update_cursor_icon();
        if self.drag == crate::app::Drag::Field
            && let Some(field) = self.focused_field()
        {
            let pos = field.view.position_at_point(x, y);
            field.set_caret(pos, true);
            self.request_redraw();
        }
    }

    /// 把对话框的起点设为「基于」菜单中选的分支 `base`；`None` 是当前分支。
    pub(in crate::app) fn choose_form_base(&mut self, base: Option<String>) {
        if let Some(dialog) = &mut self.form_dialog {
            dialog.base = base;
        }
        self.request_redraw();
    }

    /// 「基于」菜单里第 `i` 项是不是现在选的起点。
    pub(in crate::app) fn form_base_chosen(&self, i: usize) -> bool {
        let Some(dialog) = &self.form_dialog else {
            return false;
        };
        match i.checked_sub(1) {
            None => dialog.base.is_none(),
            Some(i) => self
                .git
                .branches
                .get(i)
                .is_some_and(|b| dialog.base.as_deref() == Some(b.name.as_str())),
        }
    }

    /// 目录按钮选好了目录：工作树建在它下面。
    pub(in crate::app) fn worktree_folder_picked(&mut self, folder: PathBuf) {
        if let Some(dialog) = &mut self.form_dialog {
            dialog.folder = Some(folder);
        }
        self.request_redraw();
    }
}
