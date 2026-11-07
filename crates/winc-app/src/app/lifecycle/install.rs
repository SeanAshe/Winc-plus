//! macOS 上从磁盘映像或被系统复制到随机位置运行时，询问是否将应用移到「应用程序」文件夹（见
//! `crate::macos::install`）。

use std::sync::atomic::{AtomicBool, Ordering};

use winc_i18n::t;

use super::*;
use crate::app::AlertButton;

/// 本进程是否已询问过：每次启动只询问一次，之后新开的窗口不询问。
static ASKED: AtomicBool = AtomicBool::new(false);

impl App {
    /// 应用需要安装到「应用程序」文件夹时询问是否移动；用户曾选择不移动并勾选「不再询问」时
    /// 跳过。
    ///
    /// 按钮为「移到「应用程序」文件夹」（↩）和「不移动」（Esc），上方是「不再询问」勾选框。
    pub(super) fn offer_move_to_applications(&mut self) {
        if self.state.stay_outside_applications || ASKED.swap(true, Ordering::Relaxed) {
            return;
        }
        let Some(location) = crate::macos::install::needs_install() else {
            return;
        };
        let detail = if location.image.is_some() {
            t!("app-move-detail-image").to_owned()
        } else {
            let folder = location.bundle.parent().unwrap_or(&location.bundle);
            t!("app-move-detail", folder = folder.display().to_string())
        };
        let buttons = vec![
            AlertButton::new(t!("app-move-button")),
            AlertButton::cancel(t!("app-move-stay")),
        ];
        let asked = Some((t!("workspace-tree-dont-ask"), false));
        self.present_alert(
            t!("app-move-title"),
            &detail,
            buttons,
            asked,
            |index, never| Some(UserEvent::MoveToApplications(index == 0, never)),
        );
    }

    /// 处理是否移到「应用程序」文件夹的选择：`accept` 为真时复制到该文件夹，退出后从新位置重新
    /// 启动；选择不移动并勾选「不再询问」（`never`）时记入状态，以后启动时不询问。
    pub(super) fn answer_move_to_applications(&mut self, accept: bool, never: bool) {
        if !accept {
            if never {
                self.update_state(|state| state.stay_outside_applications = true);
            }
            return;
        }
        let Some(location) = crate::macos::install::needs_install() else {
            return;
        };
        let moved = crate::macos::install::move_to_applications(&location)
            .and_then(|target| crate::macos::install::relaunch_from(&target, &location));
        match moved {
            Ok(()) => self.request_quit(),
            Err(error) => self.notify(t!("app-move-failed", error = error)),
        }
    }
}
