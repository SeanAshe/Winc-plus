//! 导出与导入设置：在系统的文件对话框中选定位置后，复制或替换用户级配置文件；导入后各窗口改用
//! 文件中的设置。

use std::path::Path;

use super::*;

/// 导出文件的默认文件名。
pub(in crate::app) const EXPORTED_SETTINGS: &str = "winc-settings.json";

/// 选择要导入的文件时列出的扩展名：导出的文件是 JSON，配置文件允许注释。
pub(in crate::app) const SETTINGS_EXTENSIONS: &[&str] = &["json", "jsonc"];

impl App {
    /// 把配置文件写入 `to`，在状态栏提示已导出；失败时提示原因。
    pub(in crate::app) fn export_settings(&mut self, to: &Path) {
        let Some(config) = self.store.config_path.clone() else {
            return;
        };
        match winc_config::export_config(&config, to) {
            Ok(()) => {
                let name = winc_editor::display_name(to);
                self.flash(t!("settings-exported", name = name));
            }
            Err(e) => self.notify(
                NoticeKind::Error,
                t!("settings-export-failed", error = e.to_string()),
            ),
        }
    }

    /// 用文件 `from` 替换配置文件，各窗口改用其中的设置，并提示原配置另存的位置和忽略的项；
    /// 文件不是有效的设置文件或无法读写时提示原因，设置不变。
    pub(in crate::app) fn import_settings(&mut self, from: &Path) {
        let Some(config) = self.store.config_path.clone() else {
            return;
        };
        // 正在编辑的输入框先提交：提交写入的配置随原文件一起另存，随后被导入的设置替换。
        self.settings_leave_field();
        let imported = match winc_config::import_config(from, &config) {
            Ok(imported) => imported,
            Err(e) => {
                let message = if e.kind() == std::io::ErrorKind::InvalidData {
                    t!(
                        "settings-import-invalid",
                        name = winc_editor::display_name(from)
                    )
                } else {
                    t!("settings-import-failed", error = e.to_string())
                };
                self.notify(NoticeKind::Error, message);
                return;
            }
        };
        // 版本号加一：应用层把导入的配置发给其他窗口。
        self.store.saved_config = imported.config.clone();
        self.store.config_rev += 1;
        self.adopt_config(imported.config);
        let (kind, message) = match (imported.problems.len(), imported.backup) {
            (0, Some(backup)) => {
                let path = backup.display().to_string();
                (NoticeKind::Info, t!("settings-imported", path = path))
            }
            (0, None) => (NoticeKind::Info, t!("settings-imported-new").to_owned()),
            (count, _) => (
                NoticeKind::Warning,
                t!("settings-imported-ignored", count = count),
            ),
        };
        self.notify(kind, message);
    }
}
