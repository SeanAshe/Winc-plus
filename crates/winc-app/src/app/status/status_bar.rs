//! 状态栏右侧可点击的项：缩进、行尾、编码、语言、工作树、分支、终端开关和设置。
//!
//! 点击缩进、行尾、编码、语言、工作树、分支时，在该项处弹出系统菜单；文档的几项由
//! [`App::run_status_command`] 执行。

use winc_core::{Indent, LineEnding};
use winc_editor::encoding::CHOICES;
use winc_editor::{DocumentId, Encoding, EncodingRead};
use winc_i18n::t;

use super::*;
use crate::app::input::context_menu::{MenuTarget, indent_size_title};
use crate::app::status::breadcrumb::MENU_TITLE_INSET;
use crate::app::workspace::watch::DiskBack;

/// 状态栏右侧可点击的项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusItem {
    /// 缩进：空格还是制表符，以及缩进宽度。
    Indent,
    /// 换行符：`LF` 或 `CRLF`。
    LineEnding,
    /// 文档的保存编码。
    Encoding,
    /// 文档的语言：手动改用别的语言（没保存过的文件选了语言才能格式化），或按文件名重新识别。
    Language,
    /// 当前的仓库或工作树（有多个位置时才有）：点了弹出系统菜单切换。
    Worktree,
    /// Git 的当前分支：点了弹出系统菜单切换。
    Branch,
    /// 终端面板的开关（终端模块开着时在设置按钮左边）。
    Terminal,
    /// 设置按钮（始终在最右边）：点了打开设置。
    Settings,
    /// 后台检查找到了新版本：点了打开更新窗口。
    Update,
}

/// 返回状态栏上显示的缩进文案，例如「空格: 4」「制表符长度: 4」。
///
/// 制表符缩进时，数字为一个制表符占的列数 `tab_width`。
pub fn indent_label(indent: Indent, tab_width: usize) -> String {
    match indent {
        Indent::Spaces(n) => t!("workspace-status-spaces", size = n),
        Indent::Tabs => format!("{}: {tab_width}", indent_size_title(indent)),
    }
}

impl App {
    /// 响应状态栏上 `item` 的点击：在该项处弹出菜单；终端开关直接开关面板，设置按钮直接打开设置。
    ///
    /// 分支名弹出的是提交视图里同一个分支选择器（往上展开），不另做一套菜单：切换、新建、同步都
    /// 在一处。
    pub(in crate::app) fn open_status_menu(&mut self, item: StatusItem) {
        if item == StatusItem::Terminal {
            return self.click_terminal_button();
        }
        // 面板开着时点在面板外先关掉它（`click_picker`），再点一下才展开：点分支名就是开和关。
        if item == StatusItem::Branch {
            return self.open_branch_picker(crate::app::GitAnchor::Status);
        }
        if item == StatusItem::Settings {
            return self.open_settings();
        }
        if item == StatusItem::Update {
            #[cfg(target_os = "macos")]
            crate::macos::check_for_updates(true);
            #[cfg(not(target_os = "macos"))]
            self.click_update();
            return;
        }
        let Some(&(rect, _)) = self.hits.status.iter().find(|&&(_, i)| i == item) else {
            return;
        };
        let target = match (item, self.editor.active().map(|d| d.id)) {
            // 这个菜单不依赖当前文档。
            (StatusItem::Worktree, _) => MenuTarget::StatusPlaces(self.location_places()),
            (_, None) => return,
            (StatusItem::Indent, Some(id)) => MenuTarget::Indent(id),
            (StatusItem::LineEnding, Some(id)) => MenuTarget::LineEnding(id),
            (StatusItem::Encoding, Some(id)) => MenuTarget::Encoding(id),
            (StatusItem::Language, Some(id)) => MenuTarget::Language(id),
            (
                StatusItem::Branch
                | StatusItem::Terminal
                | StatusItem::Settings
                | StatusItem::Update,
                _,
            ) => return,
        };
        self.open_button_menu(
            MenuButton::Status(item),
            target,
            (rect.x - MENU_TITLE_INSET, rect.y),
        );
    }

    /// 执行状态栏菜单中选中的 `command`；`id` 不再是当前文档时忽略。
    pub(in crate::app) fn run_status_command(&mut self, command: MenuCommand, id: DocumentId) {
        if self.editor.active().map(|d| d.id) != Some(id) {
            return;
        }
        match command {
            MenuCommand::IndentUsingSpaces => self.use_indent(false),
            MenuCommand::IndentUsingTabs => self.use_indent(true),
            MenuCommand::IndentSize(size) => {
                if let Some(doc) = self.editor.active_mut() {
                    doc.set_indent_size(size);
                }
                self.relayout();
            }
            MenuCommand::ConvertToSpaces => self.convert_indentation(false),
            MenuCommand::ConvertToTabs => self.convert_indentation(true),
            MenuCommand::DetectIndentation => self.detect_indentation(),
            MenuCommand::LineEnding(eol) => self.change_line_ending(eol),
            MenuCommand::ReopenEncoding(i) => {
                if let Some(&(encoding, _)) = CHOICES.get(i) {
                    self.ask_reopen_with_encoding(id, encoding);
                }
            }
            MenuCommand::SaveEncoding(i) => {
                if let Some(&(encoding, _)) = CHOICES.get(i) {
                    self.save_with_encoding(encoding);
                }
            }
            MenuCommand::UseLanguage(i) => {
                if let Some(&language) = winc_syntax::languages().get(i) {
                    self.use_language(language);
                }
            }
            MenuCommand::UsePlainText => self.use_language(winc_syntax::detect("")),
            MenuCommand::DetectLanguage => {
                let name = self
                    .editor
                    .active()
                    .and_then(|d| d.path.as_deref()?.file_name());
                if let Some(name) = name.and_then(|n| n.to_str()) {
                    self.use_language(winc_syntax::detect(name));
                }
            }
            _ => {}
        }
    }

    /// 请求按 `encoding` 重新打开文档 `id`。
    ///
    /// 文档有未保存的修改时，先弹出确认框，用户选择「放弃更改并重新打开」后才执行。
    pub(super) fn ask_reopen_with_encoding(&mut self, id: DocumentId, encoding: Encoding) {
        let Some(doc) = self.editor.document(id) else {
            return;
        };
        if !doc.is_dirty() {
            self.reopen_with_encoding(id, encoding);
            return;
        }
        if self.window.handle.is_some() {
            let message = t!("workspace-reopen-confirm");
            let detail = t!("workspace-reopen-confirm-detail");
            let button = t!("workspace-reopen-discard");
            self.show_confirm(message, detail, button, move |ok| {
                ok.then_some(UserEvent::ReopenEncoding(id, encoding))
            });
            return;
        }
        // 没有窗口（测试中）无法弹出确认框，改为在状态栏提示原因。
        self.flash(t!("workspace-reopen-dirty"));
    }

    /// 按 `encoding` 重新读取文档 `id`，丢弃未保存的修改。
    ///
    /// 读取和解码在后台进行，只采用该文档最近一次发出的这类读取。读好后由
    /// [`App::finish_reopen_with_encoding`] 换入。
    pub(in crate::app) fn reopen_with_encoding(&mut self, id: DocumentId, encoding: Encoding) {
        let Some(doc) = self.editor.document(id) else {
            return;
        };
        match doc.begin_reopen_with_encoding(encoding) {
            Ok(job) => {
                let ticket = self.rereads.reencodes.issue(id);
                let sender = self.rereads.results.0.clone();
                self.spawn_then(UserEvent::Reread, move || {
                    let _ = sender.send((id, ticket, DiskBack::Reencoded(job.read())));
                });
            }
            Err(reason) => {
                self.notify(t!("workspace-reopen-failed", error = reason.to_string()));
                self.request_redraw();
            }
        }
    }

    /// 换入按用户选择的编码读好的文档 `id` 的内容 `read`。
    ///
    /// 内容无法完整解码时文档以只读方式打开，并在状态栏提示；读取失败时在编辑区提示原因。读取
    /// 期间文档被编辑过时不换入。
    pub(in crate::app) fn finish_reopen_with_encoding(
        &mut self,
        id: DocumentId,
        read: EncodingRead,
    ) {
        let Some(doc) = self.editor.document_mut(id) else {
            return;
        };
        let encoding = read.encoding();
        match doc.finish_reopen_with_encoding(read) {
            Ok(true) => {
                let damaged = doc.encoding == Encoding::Lossy;
                if self.editor.active().is_some_and(|d| d.id == id) {
                    self.sticky_cache = None;
                    self.relayout();
                    self.clamp_scroll();
                    self.ensure_caret_visible();
                }
                if damaged {
                    self.flash(t!("workspace-reopen-lossy", encoding = encoding.label()));
                }
            }
            Ok(false) => {}
            Err(reason) => self.notify(t!("workspace-reopen-failed", error = reason.to_string())),
        }
        self.request_redraw();
    }

    /// 将当前文档的保存编码改为 `encoding`，并立即保存。
    pub(super) fn save_with_encoding(&mut self, encoding: Encoding) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if doc.set_save_encoding(encoding) {
            self.save_active();
        }
        self.request_redraw();
    }

    /// 将之后输入的缩进改为空格（`tabs` 为真时改为制表符），缩进宽度不变；已有文本不改动。
    pub(in crate::app) fn use_indent(&mut self, tabs: bool) {
        let tab_width = self.editor.tab_width;
        if let Some(doc) = self.editor.active_mut() {
            doc.use_indent(tabs, tab_width);
            self.request_redraw();
        }
    }

    /// 将已有的缩进全部转换为空格（`to_tabs` 为真时转换为制表符）；失败时在状态栏提示原因。
    pub(in crate::app) fn convert_indentation(&mut self, to_tabs: bool) {
        let tab_width = self.editor.tab_width;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        match doc.convert_indentation(to_tabs, tab_width) {
            Ok(()) => self.after_whole_file_edit(),
            Err(reason) => self.flash(reason),
        }
    }

    /// 清除手动选择的缩进设置，改为根据文档内容检测，并在状态栏提示检测结果。
    pub(in crate::app) fn detect_indentation(&mut self) {
        let tab_width = self.editor.tab_width;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.detect_indentation();
        let label = indent_label(doc.indent(), doc.tab_width(tab_width));
        self.relayout();
        self.flash(t!("workspace-indent-detected", indent = label));
    }

    /// 将文档中所有换行符转换为 `eol`；失败时在状态栏提示原因。
    pub(super) fn change_line_ending(&mut self, eol: LineEnding) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        match doc.change_line_ending(eol) {
            Ok(()) => self.after_whole_file_edit(),
            Err(reason) => self.flash(reason),
        }
    }

    /// 把当前文档改用 `language`：高亮、折叠、格式化都跟着变。
    ///
    /// 文件之后改名、另存为时按新的文件名重新识别。
    pub(in crate::app) fn use_language(&mut self, language: &'static winc_syntax::Language) {
        if let Some(doc) = self.editor.active_mut() {
            doc.choose_language(language);
        }
        self.relayout();
        self.request_redraw();
    }

    /// 执行命令面板中的「更改文件的语言」：弹出状态栏上的语言菜单。
    ///
    /// 输出标签（格式化的结果）没有这一项，不弹出该菜单。
    pub(in crate::app) fn open_language_menu(&mut self) {
        if self.editor.active().is_none_or(|d| d.title.is_some()) {
            return;
        }
        self.open_status_menu(StatusItem::Language);
    }

    /// 执行命令面板中的「更改行尾序列」：弹出状态栏上的行尾菜单。
    pub(in crate::app) fn open_line_ending_menu(&mut self) {
        self.open_status_menu(StatusItem::LineEnding);
    }

    fn after_whole_file_edit(&mut self) {
        self.relayout();
        self.ensure_caret_visible();
        self.request_redraw();
    }
}

impl App {
    /// 点了状态栏上的更新提示（macOS 以外的平台）：新版本已经准备好时重新启动，正在下载时不
    /// 处理，否则开始下载安装。
    #[cfg(not(target_os = "macos"))]
    fn click_update(&mut self) {
        use crate::winc::UpdateState;
        let state = self.shared.update_state.borrow().clone();
        match state {
            UpdateState::Ready(_) => {
                crate::updates::restart();
                self.request_quit();
            }
            UpdateState::Downloading(..) => {}
            _ => crate::updates::install(),
        }
    }
}
