//! 在后台保存文档（见 winc-editor 的 `Document::begin_save`）。
//!
//! [`App::write_document_with`] 传入 `background` 时走后台，`⌘S` 和自动保存不会阻塞界面。关闭标签、
//! 关闭窗口、退出、提交和合并之前的保存需要立即知道结果，仍在界面线程上完成，开始之前先等该文档在
//! 后台的那次保存完成（[`App::wait_for_save`]），两次写入不会颠倒。后台保存期间不同步磁盘，否则
//! 自己写出的文件会被当成其他程序的修改而报冲突，此时文档仍算未保存。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use winc_editor::{SaveJob, Saved};

use super::*;

/// 后台保存完成的结果：文档、成功时是否不提示、保存结果。
type Done = (DocumentId, bool, Saved);

/// 在后台保存的文档。
pub(crate) struct BackgroundSaves {
    /// 正在后台保存的文档：同一个文档同一时刻只保存一次。
    pub(in crate::app) running: HashSet<DocumentId>,
    /// 保存期间又请求保存的文档，以及那次请求是否不提示：本次保存完成后再保存一次。
    pub(in crate::app) again: HashMap<DocumentId, bool>,
    results: (mpsc::Sender<Done>, mpsc::Receiver<Done>),
}

impl Default for BackgroundSaves {
    fn default() -> Self {
        Self {
            running: HashSet::new(),
            again: HashMap::new(),
            results: mpsc::channel(),
        }
    }
}

impl App {
    /// 文档 `id` 是否正在后台保存。
    pub(in crate::app) fn saving(&self, id: DocumentId) -> bool {
        self.disk.saves.running.contains(&id)
    }

    /// 在后台写出 `job`（文档 `id` 的保存），完成后由 [`App::poll_saves`] 收取。
    pub(in crate::app) fn save_in_background(&mut self, id: DocumentId, job: SaveJob, quiet: bool) {
        self.disk.saves.running.insert(id);
        let sender = self.disk.saves.results.0.clone();
        let save = move || {
            let _ = sender.send((id, quiet, job.run()));
        };
        // 远端的文件与改名、移动、删除在同一个线程上按顺序执行。
        let remote = (self.editor.document(id))
            .and_then(|doc| doc.path.as_deref())
            .is_some_and(winc_backend::is_remote);
        if remote {
            self.spawn_ordered_then(UserEvent::Saved, save);
        } else {
            self.spawn_then(UserEvent::Saved, save);
        }
    }

    /// 收取后台保存完成的文档。
    pub(in crate::app) fn poll_saves(&mut self) {
        while let Ok(done) = self.disk.saves.results.1.try_recv() {
            self.saved_in_background(done);
        }
    }

    /// 等文档 `id` 在后台的那次保存完成（没有正在保存时立即返回）；等待期间完成的其他文档一并收取。
    pub(in crate::app) fn wait_for_save(&mut self, id: DocumentId) {
        while self.saving(id) {
            let Ok(done) = self.disk.saves.results.1.recv() else {
                return;
            };
            self.saved_in_background(done);
        }
    }

    /// 等待所有后台保存完成：关闭、退出之前未保存的状态必须准确。
    pub(in crate::app) fn wait_for_saves(&mut self) {
        while !self.disk.saves.running.is_empty() {
            let Ok(done) = self.disk.saves.results.1.recv() else {
                return;
            };
            self.saved_in_background(done);
        }
    }

    fn saved_in_background(&mut self, (id, quiet, saved): Done) {
        self.disk.saves.running.remove(&id);
        let Some(doc) = self.editor.document_mut(id) else {
            return;
        };
        let name = doc.name().into_owned();
        let path = doc.path.clone();
        match doc.finish_save(saved) {
            Ok(()) => {
                if !quiet {
                    self.flash(winc_i18n::t!("workspace-saved", name = name));
                }
                self.blame_saved(id);
                self.run_file_saved(id);
                self.request_redraw();
            }
            Err(e) => self.notify(winc_i18n::t!(
                "workspace-save-failed",
                name = name,
                error = e.to_string()
            )),
        }
        // 后台保存期间没有同步磁盘：现在补一次，期间其他程序确实修改过就报冲突。
        if let Some(path) = path {
            self.sync_documents(|p| p == path);
        }
        if let Some(quiet) = self.disk.saves.again.remove(&id)
            && self.editor.document(id).is_some_and(|d| d.is_dirty())
        {
            self.write_document_with(id, None, quiet, true);
        }
    }
}
