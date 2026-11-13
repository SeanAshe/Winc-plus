//! Git 询问对话框的测试：登录、取消、遮盖文字、主机密钥确认、操作结束时的关闭和截图。

use std::sync::mpsc::{Receiver, channel};

use crate::app::Focus;
use crate::app::git::form::FormKind;
use crate::app::lifecycle::notice::NoticeAction;
use crate::askpass::{InputKind, Prompt, Question};
use crate::layout::DialogPart;

use super::*;

fn passphrase() -> Prompt {
    Prompt::Passphrase {
        key: "/home/me/.ssh/id_ed25519".into(),
    }
}

fn username() -> Prompt {
    Prompt::Username {
        server: "https://github.com".into(),
    }
}

fn password() -> Prompt {
    Prompt::Password {
        server: "https://github.com".into(),
        account: Some("me".into()),
    }
}

fn host_key() -> Prompt {
    Prompt::HostKey {
        host: "git.example.com (203.0.113.10)".into(),
        key_type: "ED25519".into(),
        fingerprint: "SHA256:Dq8fXk2mVYQw0kq1yq3B7sPqTn4hO9mZk3a1uLrG5cE".into(),
    }
}

/// 向 `app` 发送属于当前操作的请求，返回接收答复的通道。
fn ask(app: &mut App, prompt: Prompt) -> Receiver<String> {
    let (reply, answers) = channel();
    let id = app.git.branch.started;
    app.open_git_question(id, Question { prompt, reply });
    answers
}

/// 创建一个正在运行操作的应用并发送 `prompt`。
fn asking(name: &str, prompt: Prompt) -> (App, Receiver<String>) {
    let dir = sample_folder(name);
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    app.git.branch.running = Some("推送".into());
    let answers = ask(&mut app, prompt);
    paint(&mut app);
    (app, answers)
}

fn typed(app: &mut App, text: &str) {
    app.focused_field().unwrap().insert(text);
}

fn enter(app: &mut App) {
    assert!(app.dialog_key(&Key::Named(NamedKey::Enter), "", false));
}

fn focus(app: &App) -> DialogPart {
    app.overlays.form_dialog.as_ref().unwrap().focus
}

/// `git` 先请求用户名、后请求密码，用户只需要填写一个对话框。
#[test]
fn signing_in_collects_both_fields_and_answers_the_password_request() {
    let (mut app, usernames) = asking("question-sign-in", username());
    typed(&mut app, "me");
    enter(&mut app);
    assert_eq!(focus(&app), DialogPart::Second);
    assert!(usernames.try_recv().is_err());
    typed(&mut app, "s3cret");
    enter(&mut app);
    assert_eq!(usernames.try_recv().unwrap(), "me");
    assert!(app.overlays.form_dialog.is_none());

    let passwords = ask(&mut app, password());
    assert_eq!(passwords.try_recv().unwrap(), "s3cret");
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(app.git.branch.password, None);
}

#[test]
fn an_empty_username_cannot_be_submitted() {
    let (mut app, answers) = asking("question-no-user", username());
    enter(&mut app);
    assert_eq!(focus(&app), DialogPart::Branch);
    assert!(answers.try_recv().is_err());
    assert!(app.overlays.form_dialog.is_some());
}

/// 已填写的密码只答复同一地址，不能发送给其他服务器。
#[test]
fn a_collected_password_is_not_sent_to_another_server() {
    let (mut app, _usernames) = asking("question-other-server", username());
    typed(&mut app, "me");
    enter(&mut app);
    typed(&mut app, "s3cret");
    enter(&mut app);
    let other = Prompt::Password {
        server: "https://other.example".into(),
        account: None,
    };
    let answers = ask(&mut app, other);
    assert!(answers.try_recv().is_err());
    assert!(app.overlays.form_dialog.is_some());
}

#[test]
fn the_end_of_the_operation_forgets_the_password_and_closes_the_dialog() {
    let (mut app, _answers) = asking("question-end", username());
    let id = app.git.branch.started;
    app.end_git_questions(id + 1);
    assert!(app.overlays.form_dialog.is_some());
    app.git.branch.password = Some(("s".into(), "p".into()));
    app.end_git_questions(id);
    assert!(app.overlays.form_dialog.is_none());
    assert_eq!(app.git.branch.password, None);
    assert!(!app.git.branch.declined);
}

#[test]
fn escape_cancels_and_the_failure_is_not_reported_as_an_error() {
    let (mut app, answers) = asking("question-cancel", passphrase());
    assert!(app.dialog_key(&Key::Named(NamedKey::Escape), "", false));
    assert!(app.overlays.form_dialog.is_none());
    assert!(answers.try_recv().is_err());
    assert!(app.git.branch.declined);
    // 取消一次即取消整个操作：`ssh` 再问口令时不再弹出，同样按取消答复。
    let again = ask(&mut app, passphrase());
    assert!(app.overlays.form_dialog.is_none());
    assert!(again.recv().is_err());
    app.report_git_failure("推送", "无法推送：could not read Password".into());
    assert!(app.notice.text.is_none());
    assert!(!app.git.branch.declined);
}

#[test]
fn a_real_failure_offers_the_full_output() {
    let (mut app, _answers) = asking("question-failure", passphrase());
    app.end_git_questions(app.git.branch.started);
    app.report_git_failure("推送", "无法推送：denied".into());
    assert_eq!(app.notice.text.as_deref(), Some("无法推送：denied"));
    assert_eq!(app.notice.action, Some(NoticeAction::GitOutput));
    app.run_notice_action();
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn a_secret_cannot_be_copied_or_cut() {
    let (mut app, _answers) = asking("question-mask", passphrase());
    typed(&mut app, "s3cret");
    let field = app.focused_field().unwrap();
    assert!(field.is_masked());
    field.select_all();
    app.set_clipboard("before".into(), false);
    app.copy_field(true);
    assert_eq!(app.focused_field().unwrap().text(), "s3cret");
    assert_eq!(app.clipboard_text().as_deref(), Some("before"));
    paint(&mut app);
}

#[test]
fn accepting_a_host_key_takes_a_deliberate_choice() {
    // 打开时没有部件持有焦点，回车按取消处理。
    let (mut app, answers) = asking("question-confirm", host_key());
    assert_eq!(focus(&app), DialogPart::Close);
    assert_eq!(app.dialog_layout().branch.w, 0.0);
    enter(&mut app);
    assert!(app.overlays.form_dialog.is_none());
    assert!(answers.try_recv().is_err());

    let (mut app, answers) = asking("question-accept", host_key());
    assert!(app.dialog_key(&Key::Named(NamedKey::Tab), "", false));
    assert_eq!(focus(&app), DialogPart::Cancel);
    assert!(app.dialog_key(&Key::Named(NamedKey::Tab), "", false));
    assert_eq!(focus(&app), DialogPart::Confirm);
    enter(&mut app);
    assert_eq!(answers.try_recv().unwrap(), "yes");
}

#[test]
fn clicking_outside_does_not_cancel_the_operation() {
    let (mut app, answers) = asking("question-outside", passphrase());
    app.click_form_dialog(0.0, 0.0);
    assert!(app.overlays.form_dialog.is_some());
    assert!(matches!(
        answers.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn clicking_the_password_field_moves_the_typing_there() {
    let (mut app, _answers) = asking("question-click", username());
    let rect = app.dialog_layout().second;
    app.click_form_dialog(rect.x + 5.0, rect.y + 5.0);
    assert_eq!(focus(&app), DialogPart::Second);
    typed(&mut app, "s");
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.second.text(), "s");
    assert_eq!(dialog.name.text(), "");
}

#[test]
fn a_question_replaces_a_form_being_filled_in() {
    let (mut app, _answers) = asking("question-form", passphrase());
    app.end_git_questions(app.git.branch.started);
    app.git.status = Some(Default::default());
    app.open_worktree_dialog();
    let _answers = ask(&mut app, passphrase());
    let dialog = app.overlays.form_dialog.as_ref().unwrap();
    assert!(matches!(dialog.kind, FormKind::Question(..)));
    paint(&mut app);
}

#[test]
#[ignore = "截图，手动运行"]
fn snapshot_question_dialogs() {
    let other = Prompt::Other {
        text: "(me@host) Verification code:".into(),
        input: InputKind::Secret,
    };
    let cases = [
        ("sign-in", username()),
        ("password", password()),
        ("key", passphrase()),
        ("host", host_key()),
        ("other", other),
    ];
    for (name, prompt) in cases {
        let (mut app, _answers) = asking(&format!("question-shot-{name}"), prompt);
        if name == "sign-in" {
            typed(&mut app, "me-example");
            enter(&mut app);
        }
        typed(&mut app, "s3cret");
        // 等待卡片的入场动画结束。
        std::thread::sleep(std::time::Duration::from_millis(400));
        shoot(&mut app, &format!("question-{name}"));
    }
}

#[test]
fn a_question_from_a_remote_repository_names_the_host() {
    let (mut app, _answers) = asking("git-question-origin", password());
    let view = |app: &App| {
        app.overlays
            .form_dialog
            .as_ref()
            .unwrap()
            .kind
            .view()
            .unwrap()
    };
    // 本机仓库的请求不带来源。
    assert_eq!(view(&app).values[0].text, "me");

    // 远端仓库的操作提出的请求：只读信息的第一行是那台主机。
    let target = winc_backend::Target::new("me@devbox").unwrap();
    app.git.root = Some(winc_backend::virtual_path(&target, b"/srv/app"));
    let _answers = ask(&mut app, password());
    paint(&mut app);
    let view = view(&app);
    assert_eq!(
        (view.values[0].label, view.values[0].text.as_str()),
        ("来自远端", "me@devbox")
    );
    assert_eq!(view.values.len(), 3);
    shoot(&mut app, "git-question-origin");
}

#[test]
fn cancelling_a_question_from_ssh_ends_the_connection_at_once() {
    let mut app = app_with(&sample_folder("question-remote").join("short.rs"), false);
    let abort = app.begin_connection_for_test();
    let (reply, answers) = channel();
    let prompt = Prompt::SshPassword {
        account: "me".into(),
        host: "devbox".into(),
    };
    app.open_question(
        crate::app::git::question::Asker::Remote,
        Question { prompt, reply },
    );
    assert!(app.overlays.form_dialog.is_some() && !abort.is_aborted());
    // 取消口令的询问：连接随即中止，`ssh` 没有机会再问第二次、第三次。
    assert!(app.dialog_key(&Key::Named(NamedKey::Escape), "", false));
    assert!(abort.is_aborted());
    assert!(answers.recv().is_err());
}
