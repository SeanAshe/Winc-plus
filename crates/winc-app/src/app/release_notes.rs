//! 更新日志：更新之后第一次启动时提示，按需从发布站点读取，在阅读标签中显示。

use winc_config::{State, compare_versions};
use winc_editor::Reading;
use winc_i18n::t;

use super::*;
use crate::release_notes::{self, Scope};

/// 更新日志在一个窗口中的状态。
#[derive(Debug, Default)]
pub(crate) struct ReleaseNotes {
    /// 尚未显示的更新提示：外层为 `Some` 表示有提示待显示，其中是上次运行的版本（没有记录时
    /// 为 `None`）。
    pub notice: Option<Option<String>>,
    /// 「更新日志」标签的文档；尚未打开过时为 `None`。取回的内容按它换入。
    pub tab: Option<DocumentId>,
    /// 最近一次打开更新日志时要显示的版本范围；重试时沿用。
    pub scope: Scope,
}

impl App {
    /// 记下当前运行的版本；这是更新之后的第一次启动时记下一条待显示的更新提示（见
    /// [`App::show_update_notice`]）。
    ///
    /// 状态为空（第一次运行）时不提示，改为开始首次设置：开始页只显示「首次设置」，直到用户
    /// 点击「完成」。版本与上次运行时相同或更旧时不提示。
    pub(crate) fn note_version(&mut self) {
        let current = winc_version::VERSION;
        let previous = self.state.version.clone();
        if previous.as_deref() == Some(current) {
            return;
        }
        let first_run = self.state == State::default();
        self.update_state(|state| {
            state.version = Some(current.to_owned());
            state.setup_pending |= first_run;
        });
        let older = previous
            .as_deref()
            .is_some_and(|previous| compare_versions(current, previous).is_lt());
        if first_run || older {
            return;
        }
        self.release_notes.notice = Some(previous);
    }

    /// 显示待显示的更新提示：右上角提示「已更新到某版本」，提示上的按钮打开更新日志。
    ///
    /// 右上角正显示着别的提示（例如启动时正在连接远端主机）时先不显示，等它收起之后再显示，
    /// 两条提示都不会被对方顶掉。每帧绘制之前调用。
    pub(in crate::app) fn show_update_notice(&mut self) {
        let free = self.notice.text.is_none() && self.notice.leaving.is_none();
        if !free {
            return;
        }
        let Some(previous) = self.release_notes.notice.take() else {
            return;
        };
        self.notify_with(
            t!(
                "workspace-release-notes-updated",
                version = winc_version::VERSION
            ),
            NoticeAction::ReleaseNotes(previous),
        );
    }

    /// 打开「更新日志」标签并在后台读取：标签先显示正在读取，取回后换成 `scope` 范围内的各版
    /// （见 [`App::show_release_notes`]）。
    ///
    /// 没有事件循环（测试）时只打开标签，不请求网络。
    pub(in crate::app) fn open_release_notes(&mut self, scope: Scope) {
        let title = t!("workspace-release-notes-title");
        let index = self.editor.show_reading(title, Reading::Loading);
        self.release_notes.tab = Some(self.editor.documents[index].id);
        self.release_notes.scope = scope.clone();
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
        let Some(proxy) = self.link.proxy.clone() else {
            return;
        };
        let locale = winc_i18n::locale();
        let current = winc_version::VERSION;
        let missing = t!("workspace-release-notes-missing", version = current);
        winc_editor::background::spawn_blocking(move || {
            let notes = release_notes::load(locale, &scope, &missing);
            let _ = proxy.send_event(UserEvent::ReleaseNotes(notes));
        });
    }

    /// 重新读取更新日志，范围与上一次相同。
    pub(in crate::app) fn reload_release_notes(&mut self) {
        self.open_release_notes(self.release_notes.scope.clone());
    }

    /// 第 `index` 个标签是否为「更新日志」标签。
    pub(in crate::app) fn is_release_notes_tab(&self, index: usize) -> bool {
        let id = self.editor.documents.get(index).map(|doc| doc.id);
        id.is_some() && id == self.release_notes.tab
    }

    /// 把取回的更新日志 `notes` 换入「更新日志」标签，不切换标签；为 `None`（未能读取）时标签
    /// 中说明原因。标签在读取期间已经关闭时不再打开。
    pub(in crate::app) fn show_release_notes(&mut self, notes: Option<String>) {
        let reading = notes.map_or(Reading::Failed, |notes| Reading::Ready(notes.into()));
        let Some(tab) = self.release_notes.tab else {
            return;
        };
        if self.editor.update_reading(tab, reading) {
            self.relayout();
            self.request_redraw();
        }
    }

    /// 设置的「更新日志」：关闭设置，在阅读标签中打开全部版本。
    pub(in crate::app) fn settings_open_release_notes(&mut self) {
        self.close_settings();
        self.open_release_notes(Scope::All);
    }

    /// 在浏览器中打开完整的更新日志。
    pub(in crate::app) fn open_release_notes_page(&mut self) {
        let page = release_notes::page(winc_i18n::locale());
        if let Err(e) = crate::system::open_url(page) {
            self.flash(t!(
                "preview-markdown-link-open-failed",
                error = e.to_string()
            ));
        }
    }
}
