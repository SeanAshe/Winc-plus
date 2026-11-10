//! 磁盘冲突：文档有未保存的修改，磁盘上的文件又被其他程序（另一个编辑器、AI 工具、`git`）改了。
//!
//! 文件不加锁，两边都可以继续编辑；没有未保存修改的文档直接重新读取。两边都改了时不能自动覆盖
//! 任何一边：当前文档冲突时，编辑区右上角一直显示冲突卡片，等待用户选择「比较」「使用磁盘版本」
//! （算一次修改，`⌘Z` 可撤回）或「保留当前修改」；自动保存跳过冲突的文档，`⌘S` 保存时先确认。

use crate::app::workspace::watch::DiskBack;
use winc_editor::DiskTaken;
use winc_editor::diff_view::{DiffSide, DiffView};
use winc_i18n::t;

use super::*;

/// 冲突卡片上的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    /// 打开一个只读标签，列出磁盘版本与未保存的修改之间的差异。
    Compare,
    /// 换成磁盘上的内容。
    TakeDisk,
    /// 保留未保存的修改，保存时覆盖磁盘上的内容。
    KeepMine,
}

impl ConflictAction {
    /// 冲突卡片上从左到右的按钮。
    pub const ALL: [ConflictAction; 3] = [
        ConflictAction::Compare,
        ConflictAction::TakeDisk,
        ConflictAction::KeepMine,
    ];

    /// 按钮上的文字。
    pub fn label(self) -> &'static str {
        match self {
            ConflictAction::Compare => t!("git-disk-conflict-compare"),
            ConflictAction::TakeDisk => t!("git-disk-conflict-take-disk"),
            ConflictAction::KeepMine => t!("git-disk-conflict-keep-mine"),
        }
    }
}

/// `⌘S` 保存冲突的文档时，提示框上的选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteChoice {
    /// 先比较两边的差异，不保存。
    Compare,
    /// 用未保存的内容覆盖磁盘上的文件。
    Overwrite,
    /// 不保存。
    Cancel,
}

impl App {
    /// 当前文档处于冲突中时返回它的名字，冲突卡片据此显示。
    pub(crate) fn active_conflict(&self) -> Option<String> {
        self.editor
            .active()
            .filter(|doc| doc.in_conflict())
            .map(|doc| doc.name().into_owned())
    }

    /// 对当前文档执行冲突卡片上的操作 `action`。
    pub(in crate::app) fn resolve_conflict(&mut self, action: ConflictAction) {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return;
        };
        match action {
            ConflictAction::Compare => self.compare_with_disk(id),
            ConflictAction::TakeDisk => self.take_disk_version(id),
            ConflictAction::KeepMine => {
                if let Some(doc) = self.editor.document_mut(id) {
                    doc.keep_mine();
                    self.flash(t!("git-disk-conflict-kept-mine"));
                }
            }
        }
        self.set_hover(Hover::None);
        self.request_redraw();
    }

    /// 打开（或刷新）一个并排差异的标签：左边是磁盘上的版本，右边是未保存的内容。
    ///
    /// 两边内容相同时（例如别的程序写入的正是这些内容）不打开标签，直接解除冲突。
    fn compare_with_disk(&mut self, id: DocumentId) {
        let Some(doc) = self.editor.document(id) else {
            return;
        };
        let name = doc.name().into_owned();
        let disk = match doc.disk_text() {
            Ok(text) => text,
            Err(e) => {
                let error = e.to_string();
                self.notify(t!("git-read-file-failed", name = &name, error = error));
                return;
            }
        };
        let mine = doc.text_range(0, usize::MAX).unwrap_or_default();
        let language = doc.highlighter.language();
        let path = doc.path.clone();
        let view = DiffView::new(
            DiffSide::new(
                t!("git-disk-conflict-side-disk"),
                &disk,
                path.clone(),
                language,
            ),
            DiffSide::new(t!("git-disk-conflict-side-unsaved"), &mine, None, language),
        );
        if view.is_identical() {
            self.take_disk_version(id);
            self.flash(t!("git-disk-conflict-identical", name = name.as_str()));
            return;
        }

        self.editor
            .show_diff(&t!("git-disk-conflict-compare-title", name = name), view);
        self.focus = Focus::Editor;
        self.relayout();
        self.clamp_scroll();
    }

    /// 把文档 `id` 换成磁盘上的内容，放弃未保存的修改（`⌘Z` 可以撤回）。
    ///
    /// 读取和比较在后台进行（大文件需要重新建索引，普通文件需要逐行比较），完成后由
    /// [`App::finish_take_disk_version`] 换上。
    fn take_disk_version(&mut self, id: DocumentId) {
        let Some(take) = self
            .editor
            .document(id)
            .and_then(|d| d.begin_take_disk_version())
        else {
            return;
        };
        let ticket = self.rereads.reencodes.issue(id);
        let sender = self.rereads.results.0.clone();
        self.spawn_then(UserEvent::Reread, move || {
            let _ = sender.send((id, ticket, DiskBack::Taken(take.read())));
        });
    }

    /// 换上后台读好的磁盘版本 `read`；读取期间文档被编辑过时不换，冲突仍然保留。
    pub(in crate::app) fn finish_take_disk_version(&mut self, id: DocumentId, read: DiskTaken) {
        let Some(doc) = self.editor.document_mut(id) else {
            return;
        };
        let name = doc.name().into_owned();
        match doc.finish_take_disk_version(read) {
            Ok(None) => {}
            Ok(Some(undoable)) => {
                let undo = crate::keymap::shortcut(crate::keymap::Action::Undo);
                self.flash(match undo {
                    Some(key) if undoable => {
                        t!(
                            "git-disk-conflict-took-disk-key",
                            name = name.as_str(),
                            key = key
                        )
                    }
                    None if undoable => t!("git-disk-conflict-took-disk", name = name.as_str()),
                    _ => t!("git-disk-conflict-reloaded", name = name.as_str()),
                });
                self.relayout();
                self.ensure_caret_visible();
            }
            Err(e) => {
                let error = e.to_string();
                self.notify(t!("git-read-file-failed", name = name, error = error));
            }
        }
    }

    /// 是否需要先确认才能保存文档 `id`：保存会覆盖其他程序对磁盘文件所做的修改时，弹出提示框
    /// 询问，返回 `true`，由回答决定是否继续保存。
    pub(in crate::app) fn confirm_overwrite(&mut self, id: DocumentId) -> bool {
        let Some(doc) = self.editor.document_mut(id) else {
            return false;
        };
        if !doc.check_overwrite() {
            return false;
        }
        let name = doc.name().into_owned();
        self.request_redraw();

        if self.window.handle.is_some() {
            use crate::app::AlertButton;

            let buttons = vec![
                AlertButton::new(t!("git-disk-conflict-compare")),
                AlertButton::cancel(t!("common-cancel")),
                AlertButton {
                    destructive: true,
                    ..AlertButton::new(t!("git-disk-conflict-overwrite"))
                },
            ];
            let message = t!("git-disk-conflict-modified", name = name.as_str());
            let detail = t!("git-disk-conflict-modified-detail");
            // 没有弹出时不保存，并说明原因。
            self.show_alert(&message, detail, buttons, None, move |index, _| {
                let choice = match index {
                    0 => OverwriteChoice::Compare,
                    2 => OverwriteChoice::Overwrite,
                    _ => OverwriteChoice::Cancel,
                };
                Some(UserEvent::Overwrite(id, choice))
            });
            return true;
        }

        // 没有窗口（测试中）时不保存：冲突卡片上可以选择保留哪一边，之后再保存。
        self.notify(t!("git-disk-conflict-choose-first", name = name));
        true
    }

    /// 处理保存冲突文档时提示框上的选择。
    pub(in crate::app) fn answer_overwrite(&mut self, id: DocumentId, choice: OverwriteChoice) {
        match choice {
            OverwriteChoice::Compare => {
                if let Some(index) = self.editor.index_of(id) {
                    self.editor.active = Some(index);
                }
                self.compare_with_disk(id);
            }
            OverwriteChoice::Overwrite => {
                if let Some(doc) = self.editor.document_mut(id) {
                    doc.keep_mine();
                }
                self.save_by_id(id);
            }
            OverwriteChoice::Cancel => {}
        }
        self.request_redraw();
    }
}
