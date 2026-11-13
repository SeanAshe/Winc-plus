//! 文件的打开与保存、标签的切换与多选，以及无法打开的文件在编辑区给出的操作。
//!
//! 系统文件面板由 [`Pick`] 描述用途，[`App::pick`] 弹出，选择结果经事件循环回到
//! [`App::on_picked`]。保存有界面线程与后台两条路径，后台保存见 `background_save.rs`。

use winc_i18n::t;

use super::*;
use crate::system;

/// 系统文件对话框的用途。
#[derive(Debug, Clone)]
pub enum Pick {
    /// 可选择文件和目录（`⌘O`）。
    Open,
    /// 只能选择目录。
    OpenFolder,
    /// 选择目录，初始位置是参数指定的 WSL 发行版的根目录。
    WslFolder(#[cfg_attr(not(windows), allow(dead_code))] String),
    /// 把文档另存为，参数为要保存的文档。
    SaveAs(DocumentId),
    /// 新建工作树对话框「位置」右端的目录按钮：选工作树放在哪个目录下面。
    WorktreeFolder,
    /// 设置中选择窗口的背景图片。
    BackgroundImage,
    /// 连接远端主机对话框中密钥文件右端的按钮：选密钥文件，从 `~/.ssh` 开始。
    KeyFile,
}

/// 无法打开的文件在编辑区中给出的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderAction {
    /// 以文本方式打开二进制文件，只读。
    OpenAsText,
    /// 读取失败时重试。
    Retry,
    /// 用系统默认应用打开。
    OpenWithDefaultApp,
    /// 在系统文件管理器中显示。
    Reveal,
    /// SVG 预览无法绘制时切回源代码。
    ShowSource,
    /// 阅读标签的内容未能取回时，在浏览器中查看。
    OpenInBrowser,
}

/// 一个标签在标签栏上的位置，绘制时记录，点击时使用。
#[derive(Debug, Clone, Copy)]
pub struct TabHit {
    /// 标签在 `documents` 中的下标。
    pub index: usize,
    /// 标签的矩形（窗口坐标，逻辑像素）。
    pub tab: Rect,
    /// 关闭按钮的矩形（窗口坐标，逻辑像素）。
    pub close: Rect,
    /// 标签上的名称是否被截短；没有对应文件的标签据此决定是否显示提示。
    pub elided: bool,
}

/// 标签栏的滚动位置，以及绘制时记录的几何信息，供点击和滚轮使用。
#[derive(Debug, Default)]
pub struct TabBar {
    /// 标签的可见范围；放不下全部标签时，右侧为新建按钮和「⌄」留出位置。
    pub view: Rect,
    /// 新建按钮的矩形。
    pub new_tab: Rect,
    /// 放不下全部标签时最右端的「⌄」按钮，点击后列出全部标签；放得下时为 `None`。
    pub list: Option<Rect>,
    /// 全部标签排开的总宽度（逻辑像素）。
    pub content: f32,
    /// 每个标签的宽度（逻辑像素），顺序与标签一致。
    pub widths: Vec<f32>,
    /// 用鼠标关闭标签时锁定的宽度：关掉的标签的下标和宽度。
    ///
    /// 标签宽度贴合文件名，关掉一个以后右边的标签移过来，它的关闭按钮一般不在鼠标下面。锁定期间
    /// 移到该下标上的标签暂用被关闭标签的宽度，关闭按钮正好在鼠标下面，可以继续关闭；
    /// 鼠标离开顶栏后解除。
    pub width_lock: Option<(usize, f32)>,
    /// 横向滚动位置（逻辑像素）。
    pub scroll: f32,
    /// 上次滚入可见范围的标签。
    ///
    /// 当前标签变化时才自动滚动，避免打断手动滚动；刚打开的标签展开完成后才记入。
    pub revealed: Option<DocumentId>,
    /// 按住的标签的拖动状态。
    pub drag: Option<TabDrag>,
    /// 正在滑动的标签。
    pub slides: Vec<TabSlide>,
    /// 刚关掉、正在原地淡出的标签（右边的标签同时滑过来补上它的位置）。
    pub closing: Vec<TabGhost>,
    /// 上一帧有哪些标签（`None`：还没画过）：多出来的一个是刚打开的，从零宽过渡到原宽并淡入。
    pub known: Option<Vec<DocumentId>>,
    /// 刚打开、正在展开的标签。
    pub opening: Vec<(DocumentId, crate::app::animation::Clock)>,
    /// `⌘` 单击、`⇧` 单击选中的各个标签。
    ///
    /// 当前标签在其中、且不止一个时才算多选（见
    /// `App::tab_selection`）：通过其他方式切换到别的标签后不再计入。
    pub marked: Vec<DocumentId>,
    /// 按 `⇧` 扩展选区时的起点。
    pub anchor: Option<DocumentId>,
}

/// 刚关闭的一个标签：内容在原位淡出，之后宽度收起，右侧的标签滑入（`App::animate_tab_close`）。
#[derive(Debug, Clone)]
pub struct TabGhost {
    /// 原来的左边在全部标签里的横坐标（从第一个标签的左边起算，逻辑像素）。
    pub left: f32,
    /// 原来的宽度。
    pub width: f32,
    /// 标签上的名字。
    pub name: String,
    /// 画图标用的文件名。
    pub icon: String,
    /// 是日志标签（画 Git 的图标）。
    pub log: bool,
    /// 关掉时是当前标签（凸起的底色跟着淡出）。
    pub active: bool,
    /// 内容淡出的时钟。
    pub fade: crate::app::animation::Clock,
    /// 宽度收起的时钟，在 `fade` 结束时开始。
    pub clock: crate::app::animation::Clock,
}

impl TabBar {
    /// 返回最大横向滚动位置（逻辑像素）：全部标签的总宽度超出可见范围的部分。
    pub fn max_scroll(&self) -> f32 {
        (self.content - self.view.w).max(0.0)
    }
}

impl App {
    /// 返回坐标 (`x`, `y`) 处的标签下标，以及该点是否在该标签的关闭按钮上。
    ///
    /// 不在任何标签上时返回 `None`。关闭按钮的判定范围向四周各扩大 2 逻辑像素。
    pub(in crate::app) fn tab_at(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        if !self.tabs.view.contains(x, y) {
            return None;
        }

        // 被拖动的标签最后绘制、压在其他标签之上，因此从后向前查找，先判断被拖动的标签。
        let hit = self.hits.tabs.iter().rev().find(|h| h.tab.contains(x, y))?;
        Some((hit.index, hit.close.inset(-2.0, -2.0).contains(x, y)))
    }

    /// 打开文件或目录 `path`（拖入窗口的，或在「打开」面板中选择的）。
    ///
    /// 文件总会打开一个标签，无法打开的在编辑区中说明原因；只打开该文件，还没有打开目录时
    /// 不把它所在的目录当作项目打开（那可能是「下载」或主目录）。目录见 [`Self::open_folder`]。
    /// 文件在预览标签中时，该标签成为普通标签。
    pub(in crate::app) fn open_path(&mut self, path: &Path) {
        self.open_file(path, false);
    }

    /// 在预览标签中打开文件 `path`。
    ///
    /// 文件已打开时切换到其标签；否则替换现有的预览标签，没有预览标签时新建一个。
    pub(in crate::app) fn preview_path(&mut self, path: &Path) {
        self.open_file(path, true);
    }

    /// 设置中是否启用了预览标签（`tree.previewOnClick`）。
    pub(in crate::app) fn preview_on_click(&self) -> bool {
        self.config.preview_on_click == Some(true)
    }

    /// 返回预览标签的序号；未启用预览标签、预览标签已关闭或已修改时为 `None`。
    ///
    /// 修改过（有可撤销或重做的修改）的预览标签成为普通标签，撤销修改后仍是普通标签。
    pub(crate) fn preview_index(&self) -> Option<usize> {
        let id = self.preview_tab.filter(|_| self.preview_on_click())?;
        let index = self.editor.index_of(id)?;
        let doc = &self.editor.documents[index];
        (!doc.history.has_changes()).then_some(index)
    }

    /// 第 `index` 个标签是预览标签时，使其成为普通标签。
    pub(in crate::app) fn keep_tab(&mut self, index: usize) {
        if self.preview_index() == Some(index) {
            self.preview_tab = None;
            self.request_redraw();
        }
    }

    /// 打开文件或目录 `path`；`preview` 为真时在预览标签中打开文件（见 [`Self::preview_path`]）。
    fn open_file(&mut self, path: &Path, preview: bool) {
        if winc_backend::fs::is_dir(path) {
            self.open_folder(path);
            return;
        }

        let replace = self.preview_index();
        if replace.is_none() {
            self.preview_tab = None;
        }
        self.auto_save_active();
        let count = self.editor.documents.len();
        let index = self.editor.open_tab(path);
        let id = self.editor.documents[index].id;
        if !preview {
            // 以其他方式打开预览标签中的文件时，该标签成为普通标签。
            if self.preview_tab == Some(id) {
                self.preview_tab = None;
            }
        } else if self.editor.documents.len() > count {
            if let Some(old) = replace {
                self.replace_preview_tab(old, index);
            }
            self.preview_tab = Some(id);
        }
        self.load_active_in_background();

        self.focus = Focus::Editor;
        self.note_recent_file();
        self.relayout();
        self.reveal_active();
        self.announce_readonly();
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 用新打开的第 `new` 个标签替换预览标签（第 `old` 个）：新标签移到原位置，原标签关闭，
    /// 不播放动画，不记入最近关闭的标签。
    fn replace_preview_tab(&mut self, old: usize, new: usize) {
        let id = self.editor.documents[new].id;
        let replaced = self.editor.documents[old].id;
        self.editor.move_document(new, old);
        // 标签栏将新标签视为原标签，不播放打开动画。
        if let Some(known) = &mut self.tabs.known
            && let Some(slot) = known.iter_mut().find(|k| **k == replaced)
        {
            *slot = id;
        }
        self.discard_tab(replaced);
    }

    /// 在 `from`（文件或目录）移到 `to` 之后，同步更新各处的记录。
    ///
    /// 已打开的文档、`⌘P` 面板的最近文件、已关闭标签（`⇧⌘T`）的记录和后退前进历史都换成新路径；
    /// `⌘P` 的文件表只更新 `from` 和 `to` 这两项；全局搜索结果重新搜索。
    pub(in crate::app) fn paths_moved(&mut self, from: &Path, to: &Path) {
        self.editor.moved(from, to);
        let relocate = |path: &mut PathBuf| {
            if let Ok(rest) = path.strip_prefix(from) {
                *path = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
            }
        };
        self.history.files.iter_mut().for_each(relocate);
        self.history
            .closed
            .iter_mut()
            .for_each(|(path, _)| relocate(path));
        self.jumps.navigation.relocate(from, to);
        self.picker_touched(vec![from.to_path_buf(), to.to_path_buf()]);
        self.files_changed_under_results();
    }

    /// 别的窗口把 `from` 移到了 `to`（在那里粘贴了本窗口中剪切的文件）：更新打开的文件、
    /// 最近的文件和文件树。
    pub(crate) fn moved_elsewhere(&mut self, from: &Path, to: &Path) {
        self.paths_moved(from, to);
        if let Some(ws) = self.editor.workspace.as_mut()
            && ws.refresh_paths(&[from.to_path_buf(), to.to_path_buf()])
        {
            self.relayout();
        }
        self.request_redraw();
    }

    /// 当前文档只读时，在状态栏说明原因；无法打开的文件已在编辑区说明，不重复提示。
    pub(in crate::app) fn announce_readonly(&mut self) {
        if let Some(reason) = self
            .editor
            .active()
            // 尚未读取完成的文档不提示：读取完成后再判断（`loading.rs`）。
            .filter(|d| d.shows_text() && !d.is_pending())
            .and_then(|d| d.readonly_reason())
        {
            self.flash(reason);
        }
    }

    /// 当前标签的编辑区是否显示文本（不是图片、无法打开的文件或 SVG、Markdown 的预览）。
    pub(in crate::app) fn active_shows_text(&self) -> bool {
        self.editor.active().is_some_and(|d| d.shows_text())
    }

    /// 执行无法打开的文件在编辑区中的按钮操作 `action`。
    pub(in crate::app) fn run_placeholder_action(&mut self, action: PlaceholderAction) {
        let Some(index) = self.editor.active else {
            return;
        };
        // 「更新日志」标签没有对应的文件：重新取回或在浏览器中查看。
        if self.is_release_notes_tab(index) {
            match action {
                PlaceholderAction::Retry => self.reload_release_notes(),
                PlaceholderAction::OpenInBrowser => self.open_release_notes_page(),
                _ => {}
            }
            return;
        }
        let Some(path) = self.editor.documents[index].path.clone() else {
            return;
        };

        match action {
            PlaceholderAction::ShowSource => self.toggle_preview(),
            PlaceholderAction::OpenAsText | PlaceholderAction::Retry => {
                let as_text = action == PlaceholderAction::OpenAsText;
                // 读好后由 `poll_loads` 换入标签并提示只读的原因。
                if self.editor.reopen(index, as_text) {
                    self.load_active_in_background();
                }
            }
            PlaceholderAction::OpenWithDefaultApp => {
                if let Err(e) = system::open_with_default_app(&path) {
                    self.notify(t!(
                        "workspace-open-default-app-failed",
                        error = e.to_string()
                    ));
                }
            }
            PlaceholderAction::Reveal => self.reveal_in_file_manager(&path),
            PlaceholderAction::OpenInBrowser => {}
        }

        self.relayout();
        self.request_redraw();
    }

    /// 在系统文件管理器中显示 `path`；失败时提示原因。
    pub(in crate::app) fn reveal_in_file_manager(&mut self, path: &Path) {
        if let Err(e) = system::reveal_in_file_manager(path) {
            self.notify(t!(
                "workspace-reveal-failed",
                action = system::reveal_label(),
                error = e.to_string()
            ));
        }
    }

    /// 在外部终端中打开 `path`。
    ///
    /// 目录打开它自己，文件打开它所在的目录；`path` 为 `None`（没有选中项，也没有打开的文件）
    /// 时打开工作区的目录，没有工作区时打开环境变量 `HOME` 指定的主目录（未设置时为 `/`）。
    /// 终端程序由配置项 `terminal.external` 指定。基准测量和测试中不启动终端，只记录目录。
    pub(in crate::app) fn open_in_terminal(&mut self, path: Option<&Path>) {
        let dir = match path {
            Some(path) if winc_backend::fs::is_dir(path) => path.to_path_buf(),
            Some(path) => path.parent().unwrap_or(path).to_path_buf(),
            None => match self.editor.root() {
                Some(root) => root.to_path_buf(),
                None => std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from),
            },
        };
        if self.open_in_builtin_terminal(&dir) {
            return;
        }
        if self.frame.bench.silent() || cfg!(test) {
            self.opened_terminal = Some(dir);
            return;
        }
        if let Err(e) = system::open_terminal(&dir, self.config.external_terminal.as_deref()) {
            let app = self.config.external_terminal.as_deref();
            let app = app.unwrap_or(t!("workspace-terminal-app-fallback"));
            self.notify(t!(
                "workspace-open-app-failed",
                app = app,
                error = e.to_string()
            ));
        }
    }

    /// 在标签栏最右端的「⌄」下方弹出标签列表，菜单右端与按钮对齐；选择一项即切换到该标签。
    pub(in crate::app) fn open_tab_list_menu(&mut self) {
        use crate::app::input::context_menu::{MenuButton, MenuTarget, menu_below};
        let Some(list) = self.tabs.list else {
            return;
        };
        let (_, y) = menu_below(list);
        self.open_button_menu(MenuButton::TabList, MenuTarget::TabList, (list.right(), y));
    }

    /// 切换到第 `index` 个标签；下标超出范围时不处理。
    pub(in crate::app) fn activate_tab(&mut self, index: usize) {
        if index >= self.editor.documents.len() {
            return;
        }

        if self.editor.active != Some(index) {
            self.auto_save_active();
        }
        self.editor.active = Some(index);
        self.load_active_in_background();
        self.focus = Focus::Editor;
        self.note_recent_file();
        self.relayout();
        self.reveal_active();
        self.request_redraw();
    }

    /// 返回选中的标签，按标签顺序排列：多选时是 `TabBar::marked` 中仍然打开的标签，否则只含当前
    /// 标签；没有标签时为空。
    pub(crate) fn tab_selection(&self) -> Vec<DocumentId> {
        let active = self.editor.active().map(|d| d.id);
        let marked = &self.tabs.marked;
        if marked.len() > 1 && active.is_some_and(|a| marked.contains(&a)) {
            return self
                .editor
                .documents
                .iter()
                .map(|d| d.id)
                .filter(|id| marked.contains(id))
                .collect();
        }
        active.into_iter().collect()
    }

    /// 是否选中了不止一个标签。
    pub(crate) fn tabs_multi_selected(&self) -> bool {
        self.tab_selection().len() > 1
    }

    /// 把第 `index` 个标签加入或移出选区（`⌘` 单击）；加入时切到它，移出的是当前标签时切到选区里
    /// 剩下的最后一个。
    ///
    /// 只剩一个时不移出。
    pub(in crate::app) fn toggle_tab_selected(&mut self, index: usize) {
        let Some(id) = self.editor.documents.get(index).map(|d| d.id) else {
            return;
        };
        let mut marked = self.tab_selection();
        self.tabs.anchor = Some(id);
        if let Some(at) = marked.iter().position(|m| *m == id) {
            if marked.len() == 1 {
                return;
            }
            marked.remove(at);
            let active = self.editor.active().map(|d| d.id);
            let next = marked.last().copied();
            self.tabs.marked = marked;
            if active == Some(id)
                && let Some(next) = next.and_then(|n| self.editor.index_of(n))
            {
                self.activate_tab(next);
            }
        } else {
            marked.push(id);
            self.tabs.marked = marked;
            self.activate_tab(index);
        }
        self.request_redraw();
    }

    /// 选中从扩展选区的起点（没有时是当前标签）到第 `index` 个之间的标签（`⇧` 单击），切到第
    /// `index` 个。
    pub(in crate::app) fn select_tab_range(&mut self, index: usize) {
        let docs = &self.editor.documents;
        if index >= docs.len() {
            return;
        }
        let from = self
            .tabs
            .anchor
            .and_then(|a| self.editor.index_of(a))
            .or(self.editor.active)
            .unwrap_or(index);
        self.tabs.anchor = Some(docs[from].id);
        self.tabs.marked = docs[from.min(index)..=from.max(index)]
            .iter()
            .map(|d| d.id)
            .collect();
        self.activate_tab(index);
    }

    /// 新建未命名文档并切换到该文档。
    pub(in crate::app) fn new_file(&mut self) {
        self.editor.new_untitled();
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 循环切换标签：`forward` 为 `true` 时切到下一个，否则切到上一个；到两端后绕回。
    pub(in crate::app) fn cycle_tab(&mut self, forward: bool) {
        let n = self.editor.documents.len();

        if n > 0 {
            let current = self.editor.active.unwrap_or(0);
            self.activate_tab(if forward {
                (current + 1) % n
            } else {
                (current + n - 1) % n
            });
        }
    }

    /// 保存当前文档（`⌘S`）。
    ///
    /// 未命名的文档弹出「另存为」面板；开启了「保存时格式化」时先格式化再保存。
    pub(in crate::app) fn save_active(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;

        if doc.title.is_some() {
            // 格式化工具的输出标签只用于查看，不可保存。
            if let Some(reason) = doc.readonly_reason() {
                self.flash(reason);
            }
        } else if doc.path.is_none() {
            self.pick(Pick::SaveAs(id));
        } else if !self.confirm_overwrite(id) {
            self.save_by_id(id);
        }
    }

    /// 保存有路径的文档 `id`：开启了「保存时格式化」时先格式化再保存。
    ///
    /// 不检查磁盘冲突，调用方已确认可以覆盖。
    pub(in crate::app) fn save_by_id(&mut self, id: DocumentId) {
        if self.prepares_on_save(id) {
            self.format_then_save(id);
        } else {
            self.write_document_with(id, None, false, true);
        }
    }

    /// 开启「自动保存」时，在切换离开之前保存当前文档。
    fn auto_save_active(&mut self) {
        if let Some(id) = self.editor.active().map(|d| d.id) {
            self.auto_save(&[id]);
        }
    }

    /// 开启「自动保存」时，保存 `ids` 中有修改的文档。
    ///
    /// 只保存有路径且可写的文档，跳过几种情况：不格式化，因为用户不在操作，不应产生额外改动；正在
    /// 格式化的文档，格式化结果返回时会修改内容；磁盘上已被其他程序修改的文档，避免未经提示覆盖；
    /// 窗口上有提示框时也不保存，提示框会使窗口失去焦点，用户随后点「不保存」就不再有效。保存成功
    /// 时不提示，失败时仍然提示。
    pub(in crate::app) fn auto_save(&mut self, ids: &[DocumentId]) {
        if self.config.auto_save != Some(true) {
            return;
        }
        #[cfg(target_os = "macos")]
        if self
            .window
            .handle
            .as_deref()
            .is_some_and(crate::macos::has_sheet)
        {
            return;
        }
        for &id in ids {
            let Some(doc) = self.editor.document(id) else {
                continue;
            };
            let candidate =
                doc.is_dirty() && doc.title.is_none() && doc.readonly_reason().is_none();
            // 文件已从磁盘上删除：不自动写回，改为提示一次重新创建的方法；文件恢复后被再次删除时
            // 重新提示。
            if candidate && doc.deleted_on_disk() {
                if self.deleted_told.insert(id) {
                    let name = doc.name().into_owned();
                    let message = match crate::keymap::shortcut(crate::keymap::Action::Save) {
                        Some(key) => t!("workspace-deleted-on-disk-key", name = name, key = key),
                        None => t!("workspace-deleted-on-disk", name = name),
                    };
                    self.notify(message);
                }
                continue;
            }
            self.deleted_told.remove(&id);
            if candidate && doc.can_save_quietly() && !self.is_formatting(id) {
                self.write_document_with(id, None, true, true);
            }
        }
    }

    /// 返回是否保存成功：保存标签 `id`（给出 `to` 时另存为到该路径），并在状态栏提示结果。
    ///
    /// 另存为会产生新文件，文件树和 `⌘P` 的文件表随之更新。
    pub(in crate::app) fn save_document(&mut self, id: DocumentId, to: Option<PathBuf>) -> bool {
        self.write_document(id, to, false)
    }

    /// 返回是否保存成功；行为与 [`App::save_document`] 相同，但 `quiet` 为 `true` 时保存成功
    /// 不提示。
    pub(in crate::app) fn write_document(
        &mut self,
        id: DocumentId,
        to: Option<PathBuf>,
        quiet: bool,
    ) -> bool {
        self.write_document_with(id, to, quiet, false)
    }

    /// 返回是否保存成功；行为与 [`App::write_document`] 相同，但 `background` 为 `true` 时在后台
    /// 写出（见 `background_save.rs`），返回值表示是否已经开始写，保存结果在完成后再提示。
    ///
    /// 只有 `⌘S` 和自动保存使用后台写出：其他保存随后要关闭、提交，必须等写完。
    pub(in crate::app) fn write_document_with(
        &mut self,
        id: DocumentId,
        to: Option<PathBuf>,
        quiet: bool,
        background: bool,
    ) -> bool {
        // 后台还在存上一次：需要在后台再存的，等这次存完再存一次；需要马上存的，先等这次保存完成，
        // 两次写入不会先后颠倒。
        if self.saving(id) {
            if background && to.is_none() {
                self.disk.saves.again.insert(id, quiet);
                return true;
            }
            self.wait_for_save(id);
        }
        // 另存为到已在其他标签中打开的文件时，两个标签对应同一个文件，但内容各自独立，
        // 先保存哪一个都会覆盖另一个的内容。因此对方有未保存的修改时不保存；对方没有修改时
        // 关闭对方的标签，内容以本次保存的为准。
        let to = to.map(|path| winc_editor::canonical_path(&path));
        let other = to.as_ref().and_then(|path| {
            self.editor
                .documents
                .iter()
                .find(|d| d.id != id && d.path.as_ref() == Some(path))
                .map(|d| (d.id, d.is_dirty()))
        });
        if let Some((_, true)) = other {
            let name = to
                .as_deref()
                .map(winc_editor::display_name)
                .unwrap_or_default();
            self.notify(t!("workspace-save-as-dirty-elsewhere", name = name));
            return false;
        }

        let Some(doc) = self.editor.document_mut(id) else {
            return false;
        };
        let saved_as = to.is_some();
        // 所有保存都经过这里：磁盘上的文件已被其他程序修改、用户还没选择保留哪一边时不写，
        // 切到该标签，由冲突卡片决定。关闭时选「保存」、保存时格式化期间才发现冲突，都在这里拦下。
        if !saved_as && doc.check_overwrite() {
            let name = doc.name().into_owned();
            if let Some(index) = self.editor.index_of(id) {
                self.activate_tab(index);
            }
            self.notify(t!("workspace-save-conflict", name = name));
            return false;
        }
        let Some(doc) = self.editor.document_mut(id) else {
            return false;
        };
        doc.tidy_for_save(to.as_deref(), quiet);
        // 远端主机上的文档总是由后台线程写出：界面线程不发出远端的请求。必须等写完的保存在此
        // 等待后台完成。
        let remote = doc.path.as_deref().is_some_and(winc_backend::is_remote);
        // 远端的改名、移动或删除尚未完成：文档的路径可能即将改变，后台保存推迟到它完成之后。
        if remote && background && to.is_none() && self.file_work_pending() {
            if !self
                .disk
                .file_work
                .deferred
                .iter()
                .any(|(known, _)| *known == id)
            {
                self.disk.file_work.deferred.push((id, quiet));
            }
            return true;
        }
        let Some(doc) = self.editor.document_mut(id) else {
            return false;
        };
        if (background || remote) && to.is_none() {
            match doc.begin_save() {
                Ok(Some(job)) => {
                    self.save_in_background(id, job, quiet);
                    if background {
                        return true;
                    }
                    self.wait_for_save(id);
                    return (self.editor.document(id)).is_some_and(|doc| !doc.is_dirty());
                }
                Ok(None) => {}
                Err(e) => {
                    let name = doc.name().into_owned();
                    self.notify(t!(
                        "workspace-save-failed",
                        name = name,
                        error = e.to_string()
                    ));
                    return false;
                }
            }
        }
        let result = match to {
            Some(path) => doc.save_as(path),
            None => doc.save(),
        };
        let name = doc.name().into_owned();

        match result {
            Ok(()) => {
                if !quiet {
                    self.flash(t!("workspace-saved", name = name));
                }
                self.blame_saved(id);
                self.run_file_saved(id);
                if saved_as {
                    if let Some((other, _)) = other {
                        self.remove_tabs(&[other]);
                    }
                    let path = self.editor.document(id).and_then(|d| d.path.clone());
                    if let (Some(ws), Some(path)) = (&mut self.editor.workspace, &path) {
                        ws.refresh_paths(std::slice::from_ref(path));
                    }
                    self.picker_touched(path.into_iter().collect());
                }
                true
            }
            Err(e) => {
                self.notify(t!(
                    "workspace-save-failed",
                    name = name,
                    error = e.to_string()
                ));
                false
            }
        }
    }

    /// 弹出系统的打开或另存为面板，选择完成后经事件循环回到 `on_picked`。
    ///
    /// macOS 上是系统的面板；Windows 上是系统的文件对话框；其他 Unix 平台通过 XDG 桌面门户由
    /// 桌面环境显示面板（`portal.rs`），门户不可用时说明原因（`App::pick_failed`）。
    pub(in crate::app) fn pick(&mut self, kind: Pick) {
        let Some(proxy) = self.link.proxy.clone() else {
            return;
        };
        // 另存为：默认使用原来的名字，从原来所在的目录开始；新文件从已打开的目录开始。
        let (name, dir) = match &kind {
            Pick::SaveAs(id) => {
                let doc = self.editor.document(*id);
                let name = doc.map_or_else(|| "untitled.txt".to_owned(), |d| d.name().into_owned());
                let dir = doc
                    .and_then(|d| d.path.as_ref()?.parent().map(Path::to_path_buf))
                    .or_else(|| self.editor.root().map(Path::to_path_buf));
                (name, dir)
            }
            Pick::KeyFile => (String::new(), crate::app::remote::key_dir()),
            Pick::Open
            | Pick::OpenFolder
            | Pick::WslFolder(_)
            | Pick::WorktreeFolder
            | Pick::BackgroundImage => (String::new(), None),
        };

        #[cfg(target_os = "macos")]
        {
            use crate::macos::{Panel, show_panel};

            let Some(window) = self.window.handle.clone() else {
                return;
            };
            let panel = match &kind {
                Pick::Open | Pick::BackgroundImage | Pick::KeyFile => Panel::Open {
                    files: true,
                    dir: dir.as_deref(),
                },
                Pick::OpenFolder | Pick::WslFolder(_) | Pick::WorktreeFolder => Panel::Open {
                    files: false,
                    dir: None,
                },
                Pick::SaveAs(_) => Panel::Save {
                    name: &name,
                    dir: dir.as_deref(),
                },
            };

            let unshown = kind.clone();
            let shown = show_panel(&window, panel, move |path| {
                let _ = proxy.send_event(UserEvent::Picked(kind.clone(), path));
            });
            // 没有弹出时按取消处理：关闭流程里等这次另存为的部分随之放弃，不一直等下去。
            if !self.dialog_shown(shown) {
                self.on_picked(unshown, None);
            }
        }

        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use crate::portal::Choose;
            // 门户的面板只能选文件或者只能选目录：「打开…」选文件，目录从「打开目录…」选。
            let (choose, title) = match &kind {
                Pick::Open | Pick::BackgroundImage => {
                    let choose = Choose::Open {
                        directory: false,
                        dir: None,
                    };
                    (choose, t!("cmd-open-file"))
                }
                Pick::KeyFile => {
                    let choose = Choose::Open {
                        directory: false,
                        dir: dir.as_deref(),
                    };
                    (choose, t!("workspace-remote-dialog-key-choose"))
                }
                Pick::OpenFolder | Pick::WslFolder(_) | Pick::WorktreeFolder => {
                    let choose = Choose::Open {
                        directory: true,
                        dir: None,
                    };
                    (choose, t!("cmd-open-folder"))
                }
                Pick::SaveAs(_) => (
                    Choose::Save {
                        name: &name,
                        dir: dir.as_deref(),
                    },
                    t!("cmd-save-as"),
                ),
            };
            // 面板的标题不带省略号。
            let title = title.trim_end_matches('…');
            crate::portal::choose(choose, title, move |result| {
                let event = match result {
                    Ok(path) => UserEvent::Picked(kind, path),
                    Err(error) => UserEvent::PickFailed(kind, error),
                };
                let _ = proxy.send_event(event);
            });
        }

        #[cfg(windows)]
        {
            // 系统的对话框只能选文件或者只能选目录：「打开…」选文件，目录从「打开目录…」选。
            let title = match &kind {
                Pick::Open | Pick::BackgroundImage => t!("cmd-open-file"),
                Pick::KeyFile => t!("workspace-remote-dialog-key-choose"),
                Pick::OpenFolder | Pick::WorktreeFolder => t!("cmd-open-folder"),
                Pick::WslFolder(_) => t!("cmd-open-wsl-folder"),
                Pick::SaveAs(_) => t!("cmd-save-as"),
            };
            // 对话框的标题不带省略号；挂在窗口上，打开期间窗口不响应。
            let mut dialog = rfd::FileDialog::new().set_title(title.trim_end_matches('…'));
            if let Some(window) = &self.window.handle {
                dialog = dialog.set_parent(&**window);
            }
            if let Some(dir) = &dir {
                dialog = dialog.set_directory(dir);
            }
            // 对话框有自己的消息循环，在后台线程上等待关闭，事件循环正常运行。
            std::thread::spawn(move || {
                // 检查发行版的根目录可能要等发行版启动，不在界面线程上进行。
                if let Pick::WslFolder(distro) = &kind {
                    dialog = dialog.set_directory(winc_editor::wsl::distro_root(distro));
                }
                let path = match &kind {
                    Pick::Open | Pick::BackgroundImage | Pick::KeyFile => dialog.pick_file(),
                    Pick::OpenFolder | Pick::WslFolder(_) | Pick::WorktreeFolder => {
                        dialog.pick_folder()
                    }
                    Pick::SaveAs(_) => dialog.set_file_name(&name).save_file(),
                };
                let _ = proxy.send_event(UserEvent::Picked(kind, path));
            });
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (kind, name, dir, proxy);
            self.flash(t!("workspace-drop-to-open"));
        }
    }

    /// 选择面板打不开：按取消处理，并说明原因；没有门户（`error` 为 `None`）时说明替代的办法。
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(in crate::app) fn pick_failed(&mut self, kind: Pick, error: Option<&str>) {
        self.on_picked(kind, None);
        self.notify(match error {
            Some(error) => t!("workspace-picker-failed", error = error),
            None => t!("workspace-picker-unavailable").to_owned(),
        });
    }

    /// 处理系统面板的选择结果：`path` 为选中的路径，用户取消时为 `None`。
    pub(in crate::app) fn on_picked(&mut self, kind: Pick, path: Option<PathBuf>) {
        // 关闭流程中的「保存」在等待这次另存为：保存成功后继续处理剩下的文档；
        // 取消或保存失败时放弃整个关闭。
        let continuation = match kind {
            Pick::SaveAs(_) => self.closing.continuation.take(),
            Pick::Open
            | Pick::OpenFolder
            | Pick::WslFolder(_)
            | Pick::WorktreeFolder
            | Pick::BackgroundImage
            | Pick::KeyFile => None,
        };

        let Some(path) = path else { return };

        match kind {
            Pick::Open | Pick::OpenFolder | Pick::WslFolder(_) => self.open_path(&path),
            Pick::WorktreeFolder => self.worktree_folder_picked(path),
            Pick::BackgroundImage => self.set_background_image(Some(path)),
            Pick::KeyFile => self.key_file_picked(&path),
            Pick::SaveAs(id) => {
                if self.save_document(id, Some(path))
                    && let Some(prompt) = continuation
                {
                    self.save_then_close(prompt);
                }
            }
        }

        self.request_redraw();
    }

    /// 窗口重新获得焦点时检查外部修改。
    ///
    /// 所有已打开的文档都与磁盘比对，因为不在已打开目录中的文件不受监视。目录处于监视状态时，
    /// 文件树和 `⌘P` 的文件表已经保持最新；没有监视（启动监视失败）时，重读文件树，并让
    /// `⌘P` 的文件表在下次使用时于后台重新遍历。
    pub(in crate::app) fn check_disk(&mut self) {
        self.check_remote();
        self.sync_documents(|_| true);
        // 在终端里提交、切换分支时 `.git` 可能不在监视范围内：打开的是仓库中的子目录，或者没有
        // 启动监视。工作树、子模块的 `.git` 是文件，真正的目录在其他位置。仓库根位于已监视的目录
        // 中、`.git` 是目录时，它的变化已由监视报告，不必每次窗口重新获得焦点都启动 git。
        // `.git` 的位置取自上一次读到的状态，不访问文件系统：远端的仓库不能在界面线程上查询。
        self.git_discover();
        self.restart_exited_watcher();
        let live = self
            .disk
            .watcher
            .as_ref()
            .is_some_and(winc_editor::Watcher::reliable);
        let watched = (self.disk.watcher.as_ref().filter(|_| live)).zip(self.git.root.as_deref());
        let git_dir = (self.git.status.as_ref()).and_then(|status| status.git_dir.as_deref());
        if !watched.is_some_and(|(w, root)| {
            root.starts_with(w.root()) && git_dir == Some(root.join(".git").as_path())
        }) {
            self.git_refresh();
        }

        if !live {
            // 发行版无法访问时目录都读不到，保留现有的文件树。
            let reachable = self.editor.root().is_none_or(winc_editor::wsl::reachable);
            if let Some(ws) = (self.editor.workspace.as_mut()).filter(|_| reachable) {
                ws.refresh();
            }
            self.overlays.picker.stale = true;
        }

        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }
}
