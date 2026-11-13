//! `git`、`ssh` 在操作中的输入请求：登录凭据、私钥口令和主机密钥确认。
//!
//! 请求由 [`crate::askpass`] 从后台线程送达，通过表单对话框（[`FormKind::Question`]）答复。操作结束
//! 时关闭未答复的对话框；用户关闭对话框表示取消，该操作随后的失败不按错误提示。

use std::mem;
use std::path::Path;
use std::sync::mpsc::Sender;

use winc_i18n::t;

use super::form::FormKind;
use crate::app::lifecycle::notice::NoticeAction;
use crate::app::{App, Focus, TextField, UserEvent};
use crate::askpass::{CONFIRMED, InputKind, Listener, Prompt, Question, host_of};
use crate::layout::DialogPart;
use crate::winc::WindowProxy;

/// 询问对话框中的一行只读信息。
pub(crate) struct QuestionValue {
    /// 名称。
    pub label: &'static str,
    /// 内容。
    pub text: String,
    /// 是否使用等宽字体：指纹需要逐字符核对。
    pub mono: bool,
}

/// 询问对话框中的一个输入框。
pub(crate) struct QuestionInput {
    /// 名称。
    pub label: &'static str,
    /// 占位文字。
    pub placeholder: &'static str,
}

/// 询问对话框显示的内容。
pub(crate) struct QuestionView {
    /// 标题。
    pub title: String,
    /// 说明文字。
    pub message: Option<String>,
    /// 为说明文字保留的行数（行）。
    pub message_lines: usize,
    /// 只读信息。
    pub values: Vec<QuestionValue>,
    /// 输入框，顺序为 `name`、`password`。
    pub inputs: Vec<QuestionInput>,
    /// 主按钮的文字。
    pub confirm: &'static str,
}

/// 无法识别的提示在有输入框时保留的行数（行）：较长的提示折行后通常不超过 3 行。
const OTHER_LINES: usize = 3;
/// 无法识别的提示在无输入框时保留的行数（行）。
///
/// 主要用于带其他已知名称的主机密钥说明：主机、指纹、各个已知名称和确认语各占一至两行。
const OTHER_CONFIRM_LINES: usize = 8;
/// 主机密钥说明保留的行数（行）：两种界面语言的说明在对话框宽度内都不超过 1 行。
const HOST_KEY_LINES: usize = 1;

impl QuestionView {
    /// 返回「转到提交号、分支或标签」对话框的内容：一个输入框。
    pub(super) fn go_to_revision() -> Self {
        Self {
            title: t!("git-go-to-title").to_owned(),
            message: None,
            message_lines: 0,
            values: Vec::new(),
            inputs: vec![QuestionInput {
                label: t!("git-go-to-label"),
                placeholder: t!("git-go-to-placeholder"),
            }],
            confirm: t!("git-go-to-button"),
        }
    }

    /// 返回添加远程仓库（`editing` 为 `None`）或修改其地址的对话框内容。
    pub(crate) fn remote(editing: Option<&str>) -> QuestionView {
        let input = |label, placeholder| QuestionInput { label, placeholder };
        let url = input(t!("git-remote-url"), t!("git-remote-url-placeholder"));
        match editing {
            Some(name) => QuestionView {
                title: t!("git-remote-edit-title").to_owned(),
                message: None,
                message_lines: 0,
                values: vec![QuestionValue {
                    label: t!("git-remote-name"),
                    text: name.to_owned(),
                    mono: false,
                }],
                inputs: vec![url],
                confirm: t!("git-remote-save-button"),
            },
            None => QuestionView {
                title: t!("git-remote-add-title").to_owned(),
                message: None,
                message_lines: 0,
                values: Vec::new(),
                inputs: vec![input(t!("git-remote-name"), "origin"), url],
                confirm: t!("git-remote-add-button"),
            },
        }
    }

    /// 返回连接远端主机的对话框内容：主机、端口和密钥文件。
    pub(crate) fn connect() -> QuestionView {
        let input = |label, placeholder| QuestionInput { label, placeholder };
        QuestionView {
            title: t!("workspace-remote-dialog-title").to_owned(),
            message: None,
            message_lines: 0,
            values: Vec::new(),
            inputs: vec![
                input(
                    t!("workspace-remote-dialog-host"),
                    t!("workspace-remote-dialog-host-placeholder"),
                ),
                input(t!("workspace-remote-dialog-port"), "22"),
                input(
                    t!("workspace-remote-dialog-key"),
                    t!("workspace-remote-dialog-key-placeholder"),
                ),
            ],
            confirm: t!("workspace-remote-dialog-connect"),
        }
    }

    /// 返回 `prompt` 对应的对话框内容；`origin` 是提出请求的远端主机，排在只读信息的第一行。
    ///
    /// 远端主机上的命令提出的请求，答复会发到那台主机；提示的文字也由它给出，可以写成任何
    /// 样子。标明主机，用户才知道输入的内容交给了谁。
    pub(crate) fn of(prompt: &Prompt, origin: Option<&str>) -> QuestionView {
        let mut view = Self::of_prompt(prompt);
        if let Some(host) = origin {
            let from = QuestionValue {
                label: t!("git-question-origin"),
                text: host.to_owned(),
                mono: false,
            };
            view.values.insert(0, from);
        }
        view
    }

    /// 返回 `prompt` 本身对应的对话框内容。
    fn of_prompt(prompt: &Prompt) -> QuestionView {
        let value = |label, text: &str| QuestionValue {
            label,
            text: text.to_owned(),
            mono: false,
        };
        let input = |label| QuestionInput {
            label,
            placeholder: "",
        };
        let password = QuestionInput {
            label: t!("git-question-password"),
            placeholder: t!("git-question-password-placeholder"),
        };
        let base = QuestionView {
            title: String::new(),
            message: None,
            message_lines: 0,
            values: Vec::new(),
            inputs: Vec::new(),
            confirm: t!("git-question-sign-in-button"),
        };
        match prompt {
            Prompt::Username { server } => QuestionView {
                title: t!("git-question-sign-in", host = host_of(server)),
                values: vec![value(t!("git-question-server"), server)],
                inputs: vec![input(t!("git-question-username")), password],
                ..base
            },
            Prompt::Password { server, account } => QuestionView {
                title: t!("git-question-sign-in", host = host_of(server)),
                values: (account.iter())
                    .map(|account| value(t!("git-question-account"), account))
                    .chain([value(t!("git-question-server"), server)])
                    .collect(),
                inputs: vec![password],
                ..base
            },
            Prompt::SshPassword { account, host } => QuestionView {
                title: t!("git-question-sign-in", host = host.as_str()),
                values: vec![value(t!("git-question-account"), account)],
                inputs: vec![password],
                ..base
            },
            Prompt::Passphrase { key } => QuestionView {
                title: t!("git-question-unlock-key").to_owned(),
                values: vec![value(
                    t!("git-question-key"),
                    &winc_editor::display_path(Path::new(key), None),
                )],
                inputs: vec![input(t!("git-question-passphrase"))],
                confirm: t!("git-question-unlock-button"),
                ..base
            },
            Prompt::HostKey {
                host,
                key_type,
                fingerprint,
            } => QuestionView {
                title: t!("git-question-host-key").to_owned(),
                message: Some(t!("git-question-host-key-message").to_owned()),
                message_lines: HOST_KEY_LINES,
                values: vec![
                    value(t!("git-question-host"), host),
                    value(t!("git-question-key-type"), key_type),
                    QuestionValue {
                        mono: true,
                        ..value(t!("git-question-fingerprint"), fingerprint)
                    },
                ],
                confirm: t!("git-question-connect-button"),
                ..base
            },
            Prompt::Other { text, input: kind } => {
                let confirmation = *kind == InputKind::None;
                QuestionView {
                    title: if confirmation {
                        t!("git-question-confirm")
                    } else {
                        t!("git-question-input")
                    }
                    .to_owned(),
                    message: Some(text.clone()),
                    message_lines: if confirmation {
                        OTHER_CONFIRM_LINES
                    } else {
                        OTHER_LINES
                    },
                    inputs: if confirmation {
                        Vec::new()
                    } else {
                        vec![input(t!("git-question-response"))]
                    },
                    confirm: t!("git-question-ok-button"),
                    ..base
                }
            }
        }
    }
}

/// 输入请求的来源：决定请求何时作废，以及用户取消后由谁提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Asker {
    /// 第几个 Git 操作。
    Git(u64),
    /// 到远端主机的连接。
    Remote,
}

/// 开始接收第 `id` 个操作的输入请求：写入 `questions` 并唤醒界面线程。
pub(super) fn listen(
    proxy: WindowProxy,
    questions: Sender<(u64, Question)>,
    id: u64,
) -> Option<Listener> {
    Listener::bind(move |question| {
        let _ = questions.send((id, question));
        let _ = proxy.send_event(UserEvent::GitQuestion);
    })
}

impl App {
    /// 处理后台送达的输入请求；所属操作已结束的请求被丢弃，askpass 程序按取消处理。
    pub(in crate::app) fn poll_git_questions(&mut self) {
        while let Ok((id, question)) = self.git.branch.questions.1.try_recv() {
            if self.git.branch.running.is_some() && self.git.branch.started == id {
                self.open_git_question(id, question);
            }
        }
    }

    /// 答复第 `id` 个操作的输入请求：优先使用登录对话框中已填写的密码，否则打开对话框。
    ///
    /// `git` 依次请求用户名和密码，登录对话框一次收集两项，其中的密码用于答复紧随其后的密码请求。
    /// 已打开的表单被关闭，因为操作正在等待答复。无需输入的请求没有默认答复：键盘焦点不落在按钮上，
    /// 接受主机密钥必须由用户明确选择。
    pub(in crate::app) fn open_git_question(&mut self, id: u64, question: Question) {
        // 用户已取消过这个操作的询问：之后的询问同样按取消答复，不再弹出。
        if self.git.branch.declined {
            return;
        }
        self.open_question(Asker::Git(id), question);
    }

    /// 答复来自 `asker` 的输入请求，规则见 [`App::open_git_question`]。
    pub(in crate::app) fn open_question(&mut self, asker: Asker, question: Question) {
        let Question { prompt, reply } = question;
        if let Prompt::Password { server, .. } = &prompt
            && let Some((known, password)) = self.git.branch.password.take()
            && known == *server
        {
            let _ = reply.send(password);
            return;
        }
        // 连接远端主机的对话框让位给询问，答复之后再回来（见 `connect_dialog_closed`）。
        if asker == Asker::Remote {
            self.connect_dialog_steps_aside();
            // 同一张卡片换了一页：旧的一页淡出，询问淡入。
            self.turn_card(crate::app::animation::Card::Dialog);
        }
        self.close_form_dialog();
        let (name, focus) = match &prompt {
            Prompt::Username { .. } => (TextField::default(), DialogPart::Branch),
            Prompt::Other {
                input: InputKind::Visible,
                ..
            } => (TextField::default(), DialogPart::Branch),
            prompt if prompt.is_confirmation() => (TextField::default(), DialogPart::Close),
            Prompt::Passphrase { .. }
            | Prompt::Password { .. }
            | Prompt::SshPassword { .. }
            | Prompt::Other { .. }
            | Prompt::HostKey { .. } => (TextField::masked(), DialogPart::Branch),
        };
        self.git.branch.asking = Some((asker, reply));
        // 远端仓库的操作提出的请求：答复会发到那台主机上，对话框标明它。
        let origin = match asker {
            Asker::Git(_) => (self.git.root.as_deref())
                .and_then(winc_backend::locate)
                .map(|at| at.target.as_str().to_owned()),
            Asker::Remote => None,
        };
        self.show_form_dialog(FormKind::Question(prompt, origin), name, focus);
    }

    /// 提交询问对话框。
    ///
    /// 登录对话框在密码为空时先把键盘焦点移到密码框，密码可能确实为空，再次提交时按空密码答复。
    pub(super) fn answer_git_question(&mut self) {
        let Some(dialog) = &mut self.overlays.form_dialog else {
            return;
        };
        let FormKind::Question(prompt, _) = &dialog.kind else {
            return;
        };
        let answer = if prompt.is_confirmation() {
            CONFIRMED.to_owned()
        } else {
            dialog.name.text().replace(['\r', '\n'], "")
        };
        if let Prompt::Username { server } = prompt {
            if dialog.second.text().is_empty() && dialog.focus == DialogPart::Branch {
                dialog.focus = DialogPart::Second;
                self.request_redraw();
                return;
            }
            let password = dialog.second.text().replace(['\r', '\n'], "");
            self.git.branch.password = Some((server.clone(), password));
        }
        let asker = self.git.branch.asking.take().map(|(asker, reply)| {
            let _ = reply.send(answer);
            asker
        });
        self.close_form_dialog();
        // 答复的是连接远端主机时的询问：连接仍在进行，回到连接的对话框。
        if asker == Some(Asker::Remote) {
            self.restore_connect_dialog();
        }
    }

    /// 关闭询问对话框时调用：尚未答复的请求按用户取消处理。
    ///
    /// 取消一次即取消整件事：`ssh` 在一次询问没有答复后会再问（口令默认问三次）。连接远端
    /// 主机时直接中止连接；Git 操作此后的询问不再弹出（见 [`App::open_git_question`]）。
    pub(super) fn decline_git_question(&mut self) {
        match self.git.branch.asking.take() {
            Some((Asker::Git(_), _)) => self.git.branch.declined = true,
            Some((Asker::Remote, _)) => self.cancel_remote(),
            None => {}
        }
    }

    /// 第 `id` 个操作结束时调用：清除已填写的密码，关闭未答复的对话框，不按用户取消处理。
    pub(in crate::app) fn end_git_questions(&mut self, id: u64) {
        if self.git.branch.started == id {
            self.git.branch.password = None;
        }
        self.end_questions(Asker::Git(id));
    }

    /// 连接结束时调用：关闭未答复的对话框，不按用户取消处理。
    pub(in crate::app) fn end_remote_questions(&mut self) {
        self.end_questions(Asker::Remote);
    }

    fn end_questions(&mut self, asker: Asker) {
        if self
            .git
            .branch
            .asking
            .as_ref()
            .is_some_and(|(at, _)| *at == asker)
        {
            self.git.branch.asking = None;
            self.close_form_dialog();
        }
    }

    /// 报告操作失败：用户取消输入时提示已取消，否则显示 `message` 并提供查看完整输出的按钮。
    pub(in crate::app) fn report_git_failure(&mut self, what: &str, message: String) {
        if mem::take(&mut self.git.branch.declined) {
            self.flash(t!("git-op-canceled", what = what));
        } else {
            self.notify_with(message, NoticeAction::GitOutput);
        }
    }

    /// 在只读标签中打开 Git 操作的输出。
    pub(in crate::app) fn show_git_output(&mut self) {
        let text = winc_editor::git::output_text();
        let text = if text.is_empty() {
            t!("git-output-empty").to_owned()
        } else {
            text
        };
        self.editor.show_output(t!("git-output-title"), &text);
        self.focus = Focus::Editor;
        self.relayout();
        self.ensure_caret_visible();
        self.request_redraw();
    }
}
