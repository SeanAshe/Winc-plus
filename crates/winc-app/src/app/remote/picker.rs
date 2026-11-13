//! 连接远端主机的选择面板：先选主机，连接后浏览其上的目录并打开。
//!
//! 两个阶段都显示在快速打开的卡片里（`Picker::remote`）：
//!
//! - **主机**：最近打开过的远端目录、已连接的主机、`ssh` 配置中的主机；输入可以筛选，也可以
//!   直接写 `用户@主机` 或 `主机:目录`。选中最近的目录直接打开，选中主机则连接后进入下一阶段；
//!   `⇥` 在连接对话框中修改选中主机的地址和端口。最近的目录、只来自最近目录的主机可以用行尾
//!   的「✕」移除；配置中的主机在最后一行「编辑 SSH 配置文件」打开的文件里修改。
//! - **目录**：列出当前目录的子目录，第一行是「打开此目录」。输入用来筛选；输入 `/` 进入
//!   匹配的目录，也可以写绝对路径、`~/…` 或 `../`。`⇥` 进入选中的目录，输入为空时 `⌫` 回到
//!   上级，`↩` 执行选中的一行，按住 `⌘`（其他系统是 `Ctrl`）的 `↩` 直接打开选中的目录。
//!
//! 目录内容在后台读取并按目录缓存，选中的子目录提前读取，进入时通常不必等待。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use winc_backend::Target;
use winc_i18n::t;
use winit::keyboard::{Key, NamedKey};

use super::{After, parse_destination};
use crate::app::animation::Card;
use crate::app::{App, Focus, UserEvent};

/// 面板的内容。
pub(crate) struct RemoteList {
    pub(crate) stage: Stage,
    /// 当前显示的各行，由 [`App::remote_picker_refresh`] 按输入重新生成。
    pub(crate) rows: Vec<Row>,
    /// 读过的目录，按主机和路径。
    listings: HashMap<(Target, String), Listing>,
}

/// 面板所处的阶段。
pub(crate) enum Stage {
    /// 选择主机或最近的远端目录。
    Hosts,
    /// 浏览主机 `target` 上的目录 `dir`（绝对路径，根目录以外不以 `/` 结尾）；`home` 是主目录。
    Browse {
        target: Target,
        home: String,
        dir: String,
    },
}

/// 一个目录的读取状态。
enum Listing {
    Loading,
    /// 其中的子目录名，已排序。
    Ready(Vec<String>),
    Failed(String),
}

/// 面板中的一行。
pub(crate) struct Row {
    /// 主要的文字。
    pub(crate) title: String,
    /// 其后淡色的说明。
    pub(crate) detail: String,
    pub(crate) icon: RowIcon,
    /// 行尾是否有「✕」：从最近的记录中移除这一行。
    pub(crate) removable: bool,
    action: RowAction,
}

impl Row {
    /// 这一行选中时在右端提示的按键；没有特别的按键时为 `None`。
    pub(crate) fn hint(&self) -> Option<&'static str> {
        match self.action {
            RowAction::OpenHere => Some(t!("workspace-picker-remote-hint-open")),
            RowAction::Enter(_) => Some(t!("workspace-picker-remote-hint-enter")),
            RowAction::Host(_) => Some(t!("workspace-picker-remote-hint-edit")),
            _ => None,
        }
    }
}

/// 把目录 `dir` 写成便于阅读的样子：主目录 `home` 以内的部分以 `~` 开头。
fn shorten(dir: &str, home: &str) -> String {
    match dir.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') && home != "/" => format!("~{rest}"),
        _ => dir.to_owned(),
    }
}

/// 行首的图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowIcon {
    Host,
    Folder,
    /// 打开着的目录：「打开此目录」一行。
    Open,
    /// 「编辑 SSH 配置文件」一行。
    Config,
}

/// 选中一行后执行的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowAction {
    /// 打开最近列表中的远端目录。
    Recent(PathBuf),
    /// 打开连接对话框，填写新主机。
    NewHost,
    /// 在编辑器中打开用户的 `ssh` 配置文件。
    Config,
    /// 连接主机，再浏览其上的目录。
    Host(Target),
    /// 连接主机并直接打开输入中给出的目录。
    Destination(Target, String),
    /// 打开正在浏览的目录。
    OpenHere,
    /// 进入名为该值的子目录。
    Enter(String),
    /// 回到上级目录。
    Up,
}

/// 后台读好的目录：主机、路径，以及其中的子目录名或失败的原因。
type Listed = (Target, String, Result<Vec<String>, String>);

/// 后台读取目录的结果，由 [`App::poll_remote_listings`] 取走。
#[derive(Default)]
pub(crate) struct Listings(Arc<Mutex<Vec<Listed>>>);

/// 返回目录 `dir` 中名为 `name` 的子目录的路径。
fn child(dir: &str, name: &str) -> String {
    match dir {
        "/" => format!("/{name}"),
        _ => format!("{dir}/{name}"),
    }
}

/// 返回目录 `dir` 的上级目录；根目录的上级是它自己。
fn parent(dir: &str) -> String {
    match dir.rsplit_once('/') {
        Some(("", _)) | None => "/".to_owned(),
        Some((up, _)) => up.to_owned(),
    }
}

/// 从目录 `dir` 出发按 `path` 走到的目录：`/` 开头是绝对路径，`~` 开头相对于主目录 `home`，
/// 其余相对于 `dir`；`.` 忽略，`..` 回到上级。
fn walk(dir: &str, home: &str, path: &str) -> String {
    let (mut at, rest) = if path.starts_with('/') {
        ("/".to_owned(), path)
    } else if let Some(rest) = path.strip_prefix('~') {
        (home.to_owned(), rest)
    } else {
        (dir.to_owned(), path)
    };
    for segment in rest.split('/') {
        match segment {
            "" | "." => {}
            ".." => at = parent(&at),
            name => at = child(&at, name),
        }
    }
    at
}

/// 按输入 `typed` 筛选子目录 `names`：不区分大小写，以它开头的排在包含它的前面，各自保持原来
/// 的顺序。以 `.` 开头的目录只在输入也以 `.` 开头时列出。
fn matching<'a>(names: &'a [String], typed: &str) -> Vec<&'a str> {
    let needle = typed.to_lowercase();
    let hidden = typed.starts_with('.');
    let visible = names.iter().filter(|name| hidden || !name.starts_with('.'));
    let (mut leading, mut containing) = (Vec::new(), Vec::new());
    for name in visible {
        let lower = name.to_lowercase();
        if lower.starts_with(&needle) {
            leading.push(name.as_str());
        } else if lower.contains(&needle) {
            containing.push(name.as_str());
        }
    }
    leading.extend(containing);
    leading
}

/// 读取主机 `target` 上目录 `dir` 的子目录名，按文件树的顺序排列（在后台线程上调用）。
///
/// 指向目录的符号链接也列出；名字不是有效 UTF-8 的目录无法在输入框中表示，不列出。
fn subfolders(target: &Target, dir: &str) -> Result<Vec<String>, String> {
    let path = winc_backend::virtual_path(target, dir.as_bytes());
    let entries = winc_backend::fs::read_dir(&path).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            entry.kind.is_dir() || entry.kind.is_symlink() && winc_backend::fs::is_dir(entry.path())
        })
        .filter_map(|entry| entry.name.into_string().ok())
        .collect();
    names.sort_by(|a, b| winc_editor::natural_cmp(a, b));
    Ok(names)
}

impl App {
    /// 打开面板的主机阶段（命令「连接到 SSH 主机」、项目名菜单、开始页）。
    pub(in crate::app) fn open_remote_picker(&mut self) {
        super::refresh_hosts();
        self.show_remote_picker(Stage::Hosts);
    }

    /// 打开面板的目录阶段：浏览 `target` 上的目录，从主目录 `home` 开始。
    pub(in crate::app) fn open_remote_browser(&mut self, target: Target, home: String) {
        let dir = home.clone();
        self.show_remote_picker(Stage::Browse { target, home, dir });
    }

    fn show_remote_picker(&mut self, stage: Stage) {
        self.finish_name_input();
        self.overlays.picker.places = None;
        self.overlays.picker.branch_rows = None;
        self.overlays.picker.location_rows = None;
        self.overlays.picker.folders = false;
        self.overlays.picker.vim_list = None;
        // 面板开着换阶段（从目录回到主机）：按换页过渡。
        if self.overlays.picker.open && self.overlays.picker.remote.is_some() {
            self.turn_card(Card::Picker);
        }
        // 已读过的目录留着：从目录阶段回到主机阶段再进来时不必重读。
        let listings =
            (self.overlays.picker.remote.take()).map_or_else(HashMap::new, |list| list.listings);
        self.overlays.picker.remote = Some(RemoteList {
            stage,
            rows: Vec::new(),
            listings,
        });
        self.overlays.picker.open = true;
        self.overlays.picker.doc = self.editor.active().map(|doc| doc.id);
        self.overlays.picker.input.set_text("");
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 按当前的输入重新生成各行。
    pub(in crate::app) fn remote_picker_refresh(&mut self) {
        let typed = self.overlays.picker.input.text().to_owned();
        let Some(list) = self.overlays.picker.remote.as_mut() else {
            return;
        };
        // 目录阶段的输入中出现 `/`：最后一个 `/` 之前的部分是要进入的目录，之后的留作筛选。
        if let Stage::Browse { home, dir, .. } = &mut list.stage
            && let Some((path, rest)) = typed.rsplit_once('/')
        {
            // 单独的 `/` 是根目录：`rsplit_once` 给出的前一部分是空的。
            let path = if path.is_empty() { "/" } else { path };
            *dir = walk(dir, home, path);
            let rest = rest.to_owned();
            self.overlays.picker.input.set_text("");
            self.overlays.picker.input.insert(&rest);
            return self.remote_picker_refresh();
        }
        let rows = match &list.stage {
            Stage::Hosts => self.host_rows(&typed),
            Stage::Browse { target, home, dir } => {
                let (target, home, dir) = (target.clone(), home.clone(), dir.clone());
                self.fetch_remote_listing(&target, &dir);
                self.folder_rows(&target, &home, &dir, &typed)
            }
        };
        self.overlays.picker.hits = (0..rows.len()).collect();
        if let Some(list) = self.overlays.picker.remote.as_mut() {
            list.rows = rows;
        }
        self.prefetch_remote_listing();
    }

    /// 主机阶段的各行：输入中明确写出的目标、最近的远端目录、已连接和已配置的主机。
    fn host_rows(&self, typed: &str) -> Vec<Row> {
        let needle = typed.trim().to_lowercase();
        let fits = |text: &str| needle.is_empty() || text.to_lowercase().contains(&needle);
        let mut rows = Vec::new();
        let mut listed: Vec<String> = Vec::new();
        // 没有输入时第一行是填写新主机的入口。
        if needle.is_empty() {
            rows.push(Row {
                title: t!("workspace-picker-remote-new").to_owned(),
                detail: String::new(),
                icon: RowIcon::Host,
                removable: false,
                action: RowAction::NewHost,
            });
        }

        for folder in &self.state.recent {
            let Some(location) = winc_backend::locate(folder) else {
                continue;
            };
            let full = winc_backend::display(folder).unwrap_or_default();
            if fits(&full) {
                rows.push(Row {
                    title: winc_editor::display_name(folder).into_owned(),
                    detail: full,
                    icon: RowIcon::Folder,
                    removable: true,
                    action: RowAction::Recent(folder.clone()),
                });
            }
            let host = location.target.as_str();
            if !listed.iter().any(|known| known == host) {
                listed.push(host.to_owned());
            }
        }
        // 只来自最近目录的主机可以移除；已连接的、配置里的不由这里的记录决定。
        let mut kept: Vec<String> = Vec::new();
        // 已连接的主机排在最前（不必等待连接），其后是最近用过的，再后是只出现在配置里的。
        let connected = winc_backend::remote::connected();
        for (at, target) in connected.iter().enumerate() {
            listed.retain(|known| known != target.as_str());
            listed.insert(at, target.as_str().to_owned());
            kept.push(target.as_str().to_owned());
        }
        for host in super::hosts() {
            kept.push(host.to_owned());
            if !listed.iter().any(|known| known == host) {
                listed.push(host.to_owned());
            }
        }
        for host in &listed {
            let Some(target) = Target::new(host).filter(|_| fits(host)) else {
                continue;
            };
            // 只标出已连接的主机；选中一行后可用的按键写在行尾，不在每一行重复说明。
            let detail = if winc_backend::remote::is_connected(&target) {
                t!("workspace-picker-remote-connected")
            } else {
                ""
            };
            rows.push(Row {
                title: host.clone(),
                detail: detail.to_owned(),
                icon: RowIcon::Host,
                removable: !kept.contains(host),
                action: RowAction::Host(target),
            });
        }

        // 输入本身就是一个目标（`用户@主机`、`主机:目录`），且不是已列出的主机：可以直接连接。
        // 写了 `@` 或 `:` 的是明确的目标，排在最前；否则排在筛选结果之后。
        let known = |host: &str| listed.iter().any(|known| known == host);
        if let Some((target, folder)) = parse_destination(typed)
            && (!folder.is_empty() || !known(target.as_str()))
        {
            let explicit = typed.contains(['@', ':']);
            let row = Row {
                title: t!("workspace-picker-remote-connect", host = typed.trim()),
                detail: String::new(),
                icon: RowIcon::Host,
                removable: false,
                action: match folder {
                    "" => RowAction::Host(target),
                    folder => RowAction::Destination(target, folder.to_owned()),
                },
            };
            if explicit {
                rows.insert(0, row);
            } else {
                rows.push(row);
            }
        }
        // 没有输入时最后一行通向配置文件：别名、密钥、跳板机都在那里设置。
        if needle.is_empty() {
            rows.push(Row {
                title: t!("workspace-picker-remote-config").to_owned(),
                detail: "~/.ssh/config".to_owned(),
                icon: RowIcon::Config,
                removable: false,
                action: RowAction::Config,
            });
        }
        rows
    }

    /// 目录阶段的各行：没有输入时是「打开此目录」、上级目录和各个子目录，有输入时是匹配的
    /// 子目录。目录尚未读好或读取失败时没有子目录的行，原因见 [`App::remote_picker_message`]。
    fn folder_rows(&self, target: &Target, home: &str, dir: &str, typed: &str) -> Vec<Row> {
        let mut rows = Vec::new();
        if typed.is_empty() {
            rows.push(Row {
                title: t!("workspace-picker-remote-open-here").to_owned(),
                detail: shorten(dir, home),
                icon: RowIcon::Open,
                removable: false,
                action: RowAction::OpenHere,
            });
            if dir != "/" {
                rows.push(Row {
                    title: "..".to_owned(),
                    detail: shorten(&parent(dir), home),
                    icon: RowIcon::Folder,
                    removable: false,
                    action: RowAction::Up,
                });
            }
        }
        let listing = (self.overlays.picker.remote.as_ref())
            .and_then(|list| list.listings.get(&(target.clone(), dir.to_owned())));
        if let Some(Listing::Ready(names)) = listing {
            rows.extend(matching(names, typed).into_iter().map(|name| Row {
                title: name.to_owned(),
                detail: String::new(),
                icon: RowIcon::Folder,
                removable: false,
                action: RowAction::Enter(name.to_owned()),
            }));
        }
        rows
    }

    /// 列表下方（或代替列表）显示的说明：正在读取、读取失败，或没有匹配的项。
    pub(crate) fn remote_picker_message(&self) -> Option<String> {
        let list = self.overlays.picker.remote.as_ref()?;
        let typed = self.overlays.picker.input.text();
        match &list.stage {
            Stage::Hosts if list.rows.is_empty() && typed.trim().is_empty() => {
                Some(t!("workspace-picker-remote-no-hosts").to_owned())
            }
            Stage::Hosts if list.rows.is_empty() => {
                Some(t!("workspace-picker-remote-invalid-host").to_owned())
            }
            Stage::Hosts => None,
            Stage::Browse { target, dir, .. } => {
                match list.listings.get(&(target.clone(), dir.clone())) {
                    None | Some(Listing::Loading) => {
                        Some(t!("workspace-picker-remote-loading").to_owned())
                    }
                    Some(Listing::Failed(error)) => {
                        Some(t!("workspace-picker-remote-failed", error = error.as_str()))
                    }
                    Some(Listing::Ready(names)) => {
                        let none = matching(names, typed).is_empty();
                        (none && !typed.is_empty())
                            .then(|| t!("workspace-picker-remote-no-folders").to_owned())
                    }
                }
            }
        }
    }

    /// 正在浏览的主机；主机阶段和面板未打开时为 `None`。显示在输入框的右端。
    pub(crate) fn remote_picker_host(&self) -> Option<&str> {
        match &self.overlays.picker.remote.as_ref()?.stage {
            Stage::Browse { target, .. } => Some(target.as_str()),
            Stage::Hosts => None,
        }
    }

    /// 执行选中的一行。按住 `⌘`（其他系统是 `Ctrl`）时直接打开选中的目录，不进入。
    pub(in crate::app) fn remote_picker_accept(&mut self) {
        let Some(list) = self.overlays.picker.remote.as_ref() else {
            return;
        };
        let row = (self.overlays.picker.hits.get(self.overlays.picker.selected))
            .and_then(|&i| list.rows.get(i));
        let Some(action) = row.map(|row| row.action.clone()) else {
            return;
        };
        let directly = if cfg!(target_os = "macos") {
            self.pointer.modifiers.super_key()
        } else {
            self.pointer.modifiers.control_key()
        };
        let browsing = match &list.stage {
            Stage::Browse { target, dir, .. } => Some((target.clone(), dir.clone())),
            Stage::Hosts => None,
        };
        match (action, browsing) {
            (RowAction::Recent(folder), _) => {
                self.close_picker();
                self.reopen_remote(&folder);
            }
            (RowAction::NewHost, _) => {
                self.close_picker();
                self.open_connect_dialog("", "", false);
            }
            (RowAction::Config, _) => {
                self.close_picker();
                self.open_ssh_config();
            }
            // 连接在对话框中进行：进度、需要的输入和失败的原因都在那里，成功后回到面板选目录。
            (RowAction::Host(target), _) => self.connect_from_picker(&target, true),
            (RowAction::Destination(target, folder), _) => {
                self.close_picker();
                self.remote_open(target, folder, After::Open);
            }
            (RowAction::OpenHere, Some((target, dir))) => {
                self.close_picker();
                self.remote_open(target, dir, After::Open);
            }
            (RowAction::Enter(name), Some((target, dir))) if directly => {
                self.close_picker();
                self.remote_open(target, child(&dir, &name), After::Open);
            }
            (RowAction::Enter(name), Some((_, dir))) => self.remote_picker_go(child(&dir, &name)),
            (RowAction::Up, Some((_, dir))) => self.remote_picker_go(parent(&dir)),
            (RowAction::OpenHere | RowAction::Enter(_) | RowAction::Up, None) => {}
        }
    }

    /// 关闭面板，在连接对话框中填上 `target`；`start` 为真时立即开始连接，否则留给用户修改。
    fn connect_from_picker(&mut self, target: &Target, start: bool) {
        self.close_picker();
        let port = target
            .port()
            .map(|port| port.to_string())
            .unwrap_or_default();
        self.open_connect_dialog(target.host(), &port, start);
    }

    /// 点了第 `row` 行行尾的「✕」：从最近的记录中移除这个目录；移除主机时移除其上的所有
    /// 目录。选中的位置保持不变。
    pub(in crate::app) fn forget_remote_row(&mut self, row: usize) {
        let list = self.overlays.picker.remote.as_ref();
        let item = (self.overlays.picker.hits.get(row))
            .and_then(|&i| list?.rows.get(i))
            .filter(|item| item.removable);
        let folders: Vec<PathBuf> = match item.map(|item| &item.action) {
            Some(RowAction::Recent(folder)) => vec![folder.clone()],
            Some(RowAction::Host(target)) => (self.state.recent.iter())
                .filter(|folder| {
                    winc_backend::locate(folder).is_some_and(|at| at.target == *target)
                })
                .cloned()
                .collect(),
            _ => return,
        };
        // 移除主机时连同为它指定的密钥文件。
        let host = match item.map(|item| &item.action) {
            Some(RowAction::Host(target)) => Some(target.clone()),
            _ => None,
        };
        let selected = self.overlays.picker.selected;
        self.update_state(|state| {
            folders.iter().for_each(|folder| state.forget(folder));
            if let Some(host) = &host {
                state.set_remote_key(host.as_str(), "");
            }
        });
        if let Some(host) = &host {
            winc_backend::remote::set_identity(host, None);
        }
        self.picker_refresh();
        self.overlays.picker.selected =
            selected.min(self.overlays.picker.hits.len().saturating_sub(1));
        self.relayout();
        self.request_redraw();
    }

    /// 目录阶段改为浏览 `dir`，清空输入。
    fn remote_picker_go(&mut self, to: String) {
        if let Some(Stage::Browse { dir, .. }) =
            self.overlays.picker.remote.as_mut().map(|l| &mut l.stage)
        {
            *dir = to;
        }
        self.overlays.picker.input.set_text("");
        self.picker_refresh();
        self.relayout();
        self.request_redraw();
    }

    /// 目录阶段按 `Esc`：回到主机面板（上一步），返回真；主机阶段返回假，由调用方关闭面板。
    pub(in crate::app) fn remote_picker_back(&mut self) -> bool {
        let browsing = (self.overlays.picker.remote.as_ref())
            .is_some_and(|list| matches!(list.stage, Stage::Browse { .. }));
        if browsing {
            self.open_remote_picker();
        }
        browsing
    }

    /// 处理面板特有的按键，返回是否已处理。主机阶段 `⇥` 在连接对话框中修改选中的主机；目录
    /// 阶段 `⇥` 进入选中的目录，输入为空时 `⌫` 回到上级。
    pub(in crate::app) fn remote_picker_key(&mut self, key: &Key) -> bool {
        let Some(list) = self.overlays.picker.remote.as_ref() else {
            return false;
        };
        let row = (self.overlays.picker.hits.get(self.overlays.picker.selected))
            .and_then(|&i| list.rows.get(i));
        let Stage::Browse { dir, .. } = &list.stage else {
            let edited = match (key, row.map(|row| &row.action)) {
                (Key::Named(NamedKey::Tab), Some(RowAction::Host(target))) => target.clone(),
                _ => return false,
            };
            self.connect_from_picker(&edited, false);
            return true;
        };
        match key {
            Key::Named(NamedKey::Tab) => {
                match row.map(|row| &row.action) {
                    Some(RowAction::Enter(name)) => self.remote_picker_go(child(dir, name)),
                    Some(RowAction::Up) => self.remote_picker_go(parent(dir)),
                    _ => {}
                }
                true
            }
            Key::Named(NamedKey::Backspace) if self.overlays.picker.input.text().is_empty() => {
                if dir != "/" {
                    self.remote_picker_go(parent(dir));
                }
                true
            }
            _ => false,
        }
    }

    /// 在后台读取 `target` 上的目录 `dir`；已读过或正在读取时不重复。
    fn fetch_remote_listing(&mut self, target: &Target, dir: &str) {
        let Some(list) = self.overlays.picker.remote.as_mut() else {
            return;
        };
        let key = (target.clone(), dir.to_owned());
        if list.listings.contains_key(&key) {
            return;
        }
        list.listings.insert(key, Listing::Loading);
        let results = Arc::clone(&self.remote.listed.0);
        let (target, dir) = (target.clone(), dir.to_owned());
        self.spawn_blocking_then(UserEvent::RemoteListed, move || {
            let names = subfolders(&target, &dir);
            let mut results = results.lock().unwrap_or_else(PoisonError::into_inner);
            results.push((target, dir, names));
        });
    }

    /// 提前读取选中的那个子目录：进入时通常已经读好。
    pub(in crate::app) fn prefetch_remote_listing(&mut self) {
        let Some(list) = self.overlays.picker.remote.as_ref() else {
            return;
        };
        let Stage::Browse { target, dir, .. } = &list.stage else {
            return;
        };
        let row = (self.overlays.picker.hits.get(self.overlays.picker.selected))
            .and_then(|&i| list.rows.get(i));
        let next = match row.map(|row| &row.action) {
            Some(RowAction::Enter(name)) => child(dir, name),
            Some(RowAction::Up) => parent(dir),
            _ => return,
        };
        let target = target.clone();
        self.fetch_remote_listing(&target, &next);
    }

    /// 收下后台读好的目录；正在浏览的目录读好时重新生成各行，选中的行保持不变。
    pub(in crate::app) fn poll_remote_listings(&mut self) {
        let listed: Vec<Listed> = {
            let mut results = (self.remote.listed.0.lock()).unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *results)
        };
        let Some(list) = self.overlays.picker.remote.as_mut() else {
            return;
        };
        let mut shown = false;
        for (target, dir, names) in listed {
            if let Stage::Browse {
                target: at,
                dir: here,
                ..
            } = &list.stage
            {
                shown |= *at == target && *here == dir;
            }
            let listing = match names {
                Ok(names) => Listing::Ready(names),
                Err(error) => Listing::Failed(error),
            };
            list.listings.insert((target, dir), listing);
        }
        if shown {
            self.picker_refresh();
            self.relayout();
            self.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_walk_from_the_folder_being_browsed() {
        let at = |path| walk("/home/me/src", "/home/me", path);
        assert_eq!(at("app"), "/home/me/src/app");
        assert_eq!(at("app/ui"), "/home/me/src/app/ui");
        assert_eq!(at(".."), "/home/me");
        assert_eq!(at("../../.."), "/");
        assert_eq!(at("../../../.."), "/");
        assert_eq!(at("./a//b/"), "/home/me/src/a/b");
        assert_eq!(at("/"), "/");
        assert_eq!(at("/etc/ssh"), "/etc/ssh");
        assert_eq!(at("~"), "/home/me");
        assert_eq!(at("~/notes"), "/home/me/notes");
        assert_eq!(
            (child("/", "etc"), parent("/etc"), parent("/")),
            ("/etc".into(), "/".into(), "/".into())
        );
    }

    #[test]
    fn folders_under_the_home_folder_are_shown_with_a_tilde() {
        assert_eq!(shorten("/home/me", "/home/me"), "~");
        assert_eq!(shorten("/home/me/src/app", "/home/me"), "~/src/app");
        // 只是名字的前缀相同，不在主目录之下。
        assert_eq!(shorten("/home/mercury", "/home/me"), "/home/mercury");
        assert_eq!(shorten("/etc", "/home/me"), "/etc");
        assert_eq!(shorten("/etc", "/"), "/etc");
    }

    #[test]
    fn folders_are_filtered_by_what_is_typed() {
        let names: Vec<String> = ["build", "Docs", ".git", "old-docs", "src"]
            .map(String::from)
            .into();
        assert_eq!(matching(&names, ""), ["build", "Docs", "old-docs", "src"]);
        // 以输入开头的在前，包含它的在后；不区分大小写。
        assert_eq!(matching(&names, "do"), ["Docs", "old-docs"]);
        assert_eq!(matching(&names, "D"), ["Docs", "build", "old-docs"]);
        // 以 `.` 开头的目录在输入 `.` 之后才列出。
        assert_eq!(matching(&names, "."), [".git"]);
        assert!(matching(&names, "zzz").is_empty());
    }
}
