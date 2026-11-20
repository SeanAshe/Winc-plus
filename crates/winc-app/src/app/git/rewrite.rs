//! 日志里提交菜单上改写历史的操作：编辑提交说明、修正或压缩到此提交、删除提交、推送此前所有
//! 提交。
//!
//! 范围和要推送的提交先在后台读出，读到以后再打开对话框或请用户确认。

use winc_editor::git::{Outgoing, RebaseAction, RebasePlan, RebaseStep};
use winc_i18n::t;

use super::branch::Then;
use super::commit::{PUSH_PREVIEW, PushIntent};
use super::form::FormKind;
use super::log::{LogConfirmed, short};
use crate::app::{App, NoticeKind};

/// 提交以后并入的历史提交：提交信息以 `fixup!` 或 `squash!` 开头，提交后变基并入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Autosquash {
    /// 并入的提交，完整的提交号。
    pub oid: String,
    /// 为真时压缩（两段说明都留下），否则修正（只留原来的说明）。
    pub squash: bool,
    /// 提交信息的第一行：「fixup! 原来的说明」。
    pub subject: String,
    /// 变基的基点；从根提交开始时为 `None`。
    pub base: Option<String>,
    /// 范围内已推送的提交数。
    pub pushed: usize,
}

/// 在后台读好、等用户处理的改写。
pub(in crate::app) enum Loaded {
    /// 编辑提交说明的范围：打开对话框。
    Reword(RebasePlan),
    /// 删除提交的范围：确认后删除。
    Drop(RebasePlan),
    /// 修正（`false`）或压缩（`true`）到此提交的范围：打开提交卡片。
    Into(RebasePlan, bool),
    /// 推送到提交 `oid` 为止：打开推送对话框，列出会送出的提交。
    PushUpTo {
        /// 推送还是发布。
        intent: PushIntent,
        /// 完整的提交号。
        oid: String,
        /// 会送出的提交的标题，最新的在前。
        subjects: Vec<String>,
        /// 会送出的提交数。
        total: usize,
    },
}

impl App {
    /// 读取改写提交 `oid` 的信息的范围，打开编辑提交说明的对话框。
    pub(in crate::app) fn start_reword(&mut self, oid: String) {
        self.git_operate_then(t!("git-op-rebase-load").into(), move |git, root| {
            let plan = git.reword_plan(root, &oid)?;
            Ok((String::new(), Then::History(Box::new(Loaded::Reword(plan)))))
        });
    }

    /// 读取删除提交 `oid` 要重新应用的范围，确认后删除。
    pub(in crate::app) fn start_drop_commit(&mut self, oid: String) {
        self.git_operate_then(t!("git-op-rebase-load").into(), move |git, root| {
            let plan = git.rebase_plan(root, &oid)?;
            Ok((String::new(), Then::History(Box::new(Loaded::Drop(plan)))))
        });
    }

    /// 读取修正（`squash` 为假）或压缩到提交 `oid` 的范围，在提交卡片里写好说明。
    pub(in crate::app) fn start_commit_into(&mut self, oid: String, squash: bool) {
        self.git_operate_then(t!("git-op-rebase-load").into(), move |git, root| {
            let plan = git.rebase_plan(root, &oid)?;
            Ok((
                String::new(),
                Then::History(Box::new(Loaded::Into(plan, squash))),
            ))
        });
    }

    /// 读取推送到提交 `oid` 为止会送出的提交，打开推送对话框。
    pub(in crate::app) fn start_push_up_to(&mut self, oid: String) {
        let Some(intent) = self.push_intent() else {
            return;
        };
        self.git_operate_then(t!("git-op-rebase-load").into(), move |git, root| {
            git.on_branch(root, &oid)?;
            let target = match intent {
                PushIntent::Publish(_) => Outgoing::Publish,
                _ => Outgoing::Upstream,
            };
            let (subjects, total) = git.outgoing(root, target, &oid, PUSH_PREVIEW)?;
            let loaded = Loaded::PushUpTo {
                intent,
                oid,
                subjects,
                total,
            };
            Ok((String::new(), Then::History(Box::new(loaded))))
        });
    }

    /// 后台读好了 `loaded`。
    pub(super) fn history_loaded(&mut self, loaded: Loaded) {
        match loaded {
            Loaded::Reword(plan) => self.open_reword_dialog(plan),
            Loaded::Drop(plan) if plan.base.is_none() && plan.commits.len() == 1 => {
                self.notify(NoticeKind::Warning, t!("git-rebase-all-dropped"));
            }
            Loaded::Drop(plan) => self.confirm_log(LogConfirmed::DropCommit(Box::new(plan))),
            Loaded::Into(plan, squash) => {
                let Some(target) = plan.commits.first() else {
                    return;
                };
                let word = if squash { "squash!" } else { "fixup!" };
                self.begin_commit_into(Autosquash {
                    oid: target.oid.clone(),
                    squash,
                    subject: format!("{word} {}", target.subject),
                    base: plan.base.clone(),
                    pushed: plan.pushed,
                });
            }
            Loaded::PushUpTo {
                intent,
                oid,
                subjects,
                total,
            } => self.open_push_up_to(intent, oid, subjects, total),
        }
    }

    /// 打开推送到提交 `oid` 为止的推送对话框；`subjects`、`total` 是会送出的提交。
    fn open_push_up_to(
        &mut self,
        intent: PushIntent,
        oid: String,
        subjects: Vec<String>,
        total: usize,
    ) {
        if self.overlays.form_dialog.is_some() {
            return;
        }
        self.open_push_dialog(intent, subjects, total);
        if let Some(dialog) = &mut self.overlays.form_dialog
            && let FormKind::Transfer(form) = &mut dialog.kind
        {
            form.upto = Some(oid);
        }
    }

    /// 提交了 `target` 的 `fixup!`、`squash!` 提交以后变基，把它并入目标。
    pub(in crate::app) fn run_autosquash(&mut self, target: Autosquash) {
        let what = match target.squash {
            true => t!("git-op-squash-into", commit = short(&target.oid)),
            false => t!("git-op-fixup", commit = short(&target.oid)),
        };
        self.git_operate_then(what, move |git, root| {
            let stopped = git.autosquash(root, &target.oid, target.base.as_deref())?;
            let then = match stopped {
                true => Then::Warn(t!("git-rebase-stopped").to_owned()),
                false => Then::Nothing,
            };
            Ok((String::new(), then))
        });
    }

    /// 删除 `plan` 的第一个提交，之后的提交重新应用。
    pub(in crate::app) fn drop_commit(&mut self, plan: RebasePlan) {
        let steps: Vec<RebaseStep> = (plan.commits.into_iter().enumerate())
            .map(|(i, commit)| RebaseStep {
                oid: commit.oid,
                action: if i == 0 {
                    RebaseAction::Drop
                } else {
                    RebaseAction::Pick
                },
                message: None,
            })
            .collect();
        let Some(first) = steps.first() else {
            return;
        };
        let what = t!("git-op-drop-commit", commit = short(&first.oid));
        let base = plan.base;
        self.git_operate_then(what, move |git, root| {
            let stopped = git.interactive_rebase(root, base.as_deref(), &steps)?;
            let then = match stopped {
                true => Then::Warn(t!("git-rebase-stopped").to_owned()),
                false => Then::Nothing,
            };
            Ok((String::new(), then))
        });
    }
}

/// 删除提交的确认框内容：（标题，说明，确认按钮的文字）。
pub(in crate::app) fn drop_prompt(plan: &RebasePlan) -> (String, String, &'static str) {
    let subject = plan.commits.first().map_or("", |c| c.subject.as_str());
    let mut detail = t!("git-drop-commit-confirm-detail").to_owned();
    let later = plan.commits.len().saturating_sub(1);
    if later > 0 {
        detail.push('\n');
        detail.push_str(&t!("git-drop-commit-confirm-later", count = later));
    }
    if plan.pushed > 0 {
        detail.push('\n');
        detail.push_str(t!("git-drop-commit-confirm-pushed"));
    }
    (
        t!("git-drop-commit-confirm", subject = subject),
        detail,
        t!("git-drop-commit-confirm-button"),
    )
}
