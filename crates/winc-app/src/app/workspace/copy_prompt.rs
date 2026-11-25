//! 文件树的「复制为提示词」：在后台把选中的文件整理成提示词（[`winc_editor::prompt`]），完成后
//! 写入剪贴板。

use std::path::PathBuf;

use winc_editor::{MAX_PROMPT_FILES, Prompt, PromptFile};
use winc_i18n::t;

use super::{App, NoticeKind, UserEvent};

impl App {
    /// 把文件 `paths`（文件树中选中的文件，不含目录）在后台整理成提示词，完成后写入剪贴板。
    ///
    /// 名字是相对于所在目录的路径，不在打开的目录中的文件跳过；有未保存修改的文件用编辑器中的
    /// 内容。超过 [`MAX_PROMPT_FILES`] 个时不整理，提示原因。
    pub(in crate::app) fn copy_as_prompt(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if paths.len() > MAX_PROMPT_FILES {
            let message = t!("menu-copy-as-prompt-too-many", limit = MAX_PROMPT_FILES);
            self.notify(NoticeKind::Warning, message);
            return;
        }
        let remote = paths.iter().any(|path| winc_backend::is_remote(path));
        let mut outside = 0;
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let Some(name) = self.prompt_name(&path) else {
                outside += 1;
                continue;
            };
            let unsaved = self.dirty_rope(&path);
            files.push(PromptFile {
                path,
                name,
                unsaved,
            });
        }
        let read = move || {
            let mut prompt = winc_editor::prompt(files);
            prompt.skipped += outside;
            prompt
        };
        let Some(proxy) = self.link.proxy.clone() else {
            let prompt = winc_backend::remote::allow_blocking(read);
            self.prompt_copied(prompt);
            return;
        };
        let task = move || {
            let _ = proxy.send_event(UserEvent::PromptCopied(read()));
        };
        if remote {
            winc_editor::background::spawn_blocking(task);
        } else {
            winc_editor::background::spawn(task);
        }
    }

    /// 写在提示词中 `path` 之前的名字：相对于所在目录的路径，本机文件在 Windows 上同样以 `/`
    /// 分隔；不在打开的目录中时返回 `None`。
    fn prompt_name(&self, path: &std::path::Path) -> Option<String> {
        let name = self.relative_path(path)?;
        Some(if cfg!(windows) && !winc_backend::is_remote(path) {
            name.replace('\\', "/")
        } else {
            name
        })
    }

    /// 整理好的提示词 `prompt` 写入剪贴板，并提示跳过的文件数；没有可用的文件时剪贴板保持不变，
    /// 提示原因。
    pub(in crate::app) fn prompt_copied(&mut self, prompt: Prompt) {
        if prompt.text.is_empty() {
            self.notify(NoticeKind::Warning, t!("menu-copy-as-prompt-empty"));
            return;
        }
        self.set_clipboard(prompt.text, false);
        self.flash(match prompt.skipped {
            0 => t!("menu-copied-as-prompt").to_owned(),
            count => t!("menu-copied-as-prompt-skipped", count = count),
        });
    }
}
