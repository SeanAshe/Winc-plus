//! 命令面板的「安装 winc 命令」「卸载 winc 命令」：执行安装或卸载，提示结果。
//!
//! 没有写入权限时，macOS 在后台线程弹出系统授权对话框，结果由事件送回；其他平台提示失败。

use winc_i18n::t;

use super::App;
use crate::shell_command::{self, Done, Outcome, Placement};

impl App {
    /// 安装 `winc` 命令。
    pub(in crate::app) fn install_shell_command(&mut self) {
        if let Some(placement) = shell_command::placement() {
            let outcome = shell_command::install(&placement);
            self.shell_command_outcome(placement, outcome, true);
        }
    }

    /// 卸载 `winc` 命令。
    pub(in crate::app) fn uninstall_shell_command(&mut self) {
        if let Some(placement) = shell_command::placement() {
            let outcome = shell_command::uninstall(&placement);
            self.shell_command_outcome(placement, outcome, false);
        }
    }

    /// 处理第一次尝试的结果：没有权限时改为请求授权，其余直接提示。
    fn shell_command_outcome(&mut self, placement: Placement, outcome: Outcome, installing: bool) {
        #[cfg(target_os = "macos")]
        if outcome == Outcome::Denied
            && let Some(proxy) = self.link.proxy.clone()
        {
            std::thread::spawn(move || {
                let outcome = shell_command::run_authorized(&placement, installing);
                let done = Done { placement, outcome };
                let _ = proxy.send_event(crate::app::UserEvent::ShellCommandDone(done));
            });
            return;
        }
        let _ = installing;
        self.shell_command_done(Done {
            placement,
            outcome: Some(outcome),
        });
    }

    /// 提示安装或卸载的结果；用户取消授权时不提示。
    pub(in crate::app) fn shell_command_done(&mut self, done: Done) {
        let Some(outcome) = done.outcome else {
            return;
        };
        let path = done.placement.link.display().to_string();
        let message = match outcome {
            Outcome::Installed => match done.placement.link.parent() {
                // 图形程序的 `PATH` 与用户 shell 的不同，无法判断目录是否在其中：不属于系统默认
                // `PATH` 的目录一律提示。
                Some(dir) if !dir.starts_with("/usr/local") => t!(
                    "shell-command-installed-hint",
                    path = path.as_str(),
                    dir = dir.display().to_string()
                ),
                _ => t!("shell-command-installed", path = path.as_str()),
            },
            Outcome::Uninstalled => t!("shell-command-uninstalled").to_owned(),
            Outcome::NotInstalled => t!("shell-command-not-installed").to_owned(),
            Outcome::Occupied => t!("shell-command-occupied", path = path.as_str()),
            Outcome::Denied => {
                let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
                t!(
                    "shell-command-failed",
                    path = path.as_str(),
                    error = error.to_string()
                )
            }
            Outcome::Failed(error) => {
                t!("shell-command-failed", path = path.as_str(), error = error)
            }
        };
        self.notify(message);
        self.request_redraw();
    }
}
