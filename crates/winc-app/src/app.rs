//! 应用主体：`App` 的状态、窗口和事件循环。
//!
//! 各项功能的方法按主题分在子模块里：子模块只读取和修改 `App` 的字段，不持有自己的长期状态；
//! `view/` 读取这些状态绘制一帧。事件从 `winit` 的事件循环进入，经 `keyboard`、`mouse` 转成
//! [`Action`](crate::keymap::Action)，或直接修改状态。

pub(crate) mod animation;
pub(crate) mod diff;
pub(crate) mod editor;
mod fields;
pub(crate) mod git;
pub(crate) mod hits;
mod input;
mod lifecycle;
pub(crate) mod preview;
mod release_notes;
pub(crate) mod remote;
pub(crate) mod scroll;
mod settings;
pub(crate) mod setup;
#[cfg(test)]
mod snapshot;
mod status;
pub(crate) mod terminal;
pub(crate) mod tickets;
pub(crate) mod vim;
pub(crate) mod workspace;

pub(crate) use crate::updates::UpdateClick;
pub use animation::Clock;
use animation::{
    EXIT_FADE, FOLD_ANIMATION, PICKER_ANIMATION, POPUP_IN, POPUP_OUT, SIDEBAR_ANIMATION, TAB_SLIDE,
    TREE_ANIMATION,
};
pub(crate) use diff::{DIFF_TEXT_PAD, DiffParts, DiffTarget, HunkKind, diff_parts};
pub use editor::conflict::{ConflictAction, OverwriteChoice};
pub use editor::editing::BraceKey;
pub use editor::folding::FoldMotion;
pub(crate) use editor::folding::fold_column;
pub use editor::format::{FormatState, ToolAnswer};
pub use editor::hint::NameHint;
pub(crate) use editor::scrollbar::{ScrollArea, Scrollbars, ThumbLook};
pub use editor::sticky::StickyRow;
pub(crate) use editor::sticky::{place_sticky_rows, sticky_limit};
pub(crate) use git::branch::picker::{GitAnchor, PickerPart};
pub(crate) use git::changes::{ChangeTarget, MARK_W, mark_x, ruler_rows};
pub(crate) use git::commit::{
    COMMIT_ROW, CommitGroup, CommitLabels, CommitName, CommitOverflow, CommitParts, CommitRow,
    CommitRows, CommitTarget, MESSAGE_LINE, MESSAGE_PAD, STASH_GAP, StashLine, StashView,
    SyncState,
};
pub(crate) use input::context_menu::{
    CHECK_COLUMN, ICON_COLUMN, MenuIcon, POPUP_PAD, POPUP_TEXT_X, PopupLevel, PopupMenu, PopupRow,
    folder_rows,
};
pub use input::context_menu::{ContextMenu, MenuButton, MenuCommand};
pub use input::palette::palette;
pub(crate) use lifecycle::alert::{AlertButton, AlertKey, AlertPart, AlertParts};
use lifecycle::appearance::{configured_font, forced_theme};
pub(crate) use lifecycle::backup::{restore_at_launch, restore_tabs};
pub use lifecycle::closing::{CloseRequest, SaveChoice, SavePrompt};
pub use lifecycle::notice::{NoticeAction, NoticeGhost};
pub use lifecycle::title_bar::WindowControl;
pub(crate) use lifecycle::title_bar::resize_cursor;
pub(crate) use lifecycle::wallpaper::Layer as WallpaperLayer;
use lifecycle::window::frame_interval;
pub(crate) use scroll::ScrollTarget;
pub use settings::{DialogCache, SettingsState};
pub use status::motion::{StatusEntry, StatusKey, StatusMotion};
pub use status::status_bar::{StatusItem, indent_label};
pub use workspace::file_drop::DropZone;
pub use workspace::files::{Pick, PlaceholderAction, TabBar, TabHit};
pub use workspace::global::{GlobalSearch, ResultRow};
pub(crate) use workspace::outline::{OUTLINE_GAP, has_children as outline_has_children};
pub use workspace::outline::{OutlineEmpty, OutlineState, OutlineTarget};
pub use workspace::picker::{
    PICKER_ICON_X, PICKER_MAX_ROWS, PICKER_PAD, PICKER_ROW, PICKER_TEXT_X, PLACES_FOOTER,
    PlacesGhost,
};
pub(crate) use workspace::recent::folder_label;
pub use workspace::session::StartAction;
pub use workspace::tab_drag::{TabDrag, TabSlide};
pub use workspace::tool_fold::ToolFold;
use workspace::tree::SidebarMotion;
pub(crate) use workspace::tree::tree_max_scroll;
pub use workspace::tree::{NameInput, TreeMotion};
pub use workspace::tree_drag::TreeDrag;
pub use workspace::tree_label::{LabelCause, LabelView, TreeLabels};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, mpsc};
// 只在 macOS 的系统回调（⌘Q 前是否询问）中读写。
#[cfg(target_os = "macos")]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Theme, Window, WindowId};

use winc_core::text::{self, MAX_LINE_BYTES, MAX_LINE_CHARS};
use winc_core::{Lines, Selection};
use winc_editor::{COPY_LIMIT, Command, DocumentId, Editor, Entry, EntryKind, FileIndex, Search};
use winc_render::{FontError, FontKind, FontSpec, Metrics, Rect, Renderer};

use winc_config::{Config, Session, State, ThemeChoice, resolve_font};

use crate::bench::{Bench, Next};
use crate::field::{TextField, Unit};
use crate::layout::{
    Layout, PanelButton, RowAction, Scrollbar, SearchButton, Setting, SettingControl,
    SettingsLayout, SettingsPage, SettingsPart, SidebarView, THEME_CHOICES, TreeTool,
    editor_content_height, scroll_to_show,
};
use crate::theme::*;
use crate::view;

/// 交给逐个执行的线程的一个任务（见 [`App::spawn_ordered_then`]）。
type OrderedTask = Box<dyn FnOnce() + Send>;

/// 后台线程和系统回调（面板、菜单、提示框）送回事件循环的事件。
#[derive(Debug)]
pub enum UserEvent {
    /// 打开或另存为面板已关闭；取消时路径为 `None`。
    Picked(Pick, Option<PathBuf>),
    /// 全局搜索产生了新结果，或已搜索完成。
    Searched,
    /// 快速打开的文件索引已建好。
    Indexed,
    /// macOS 上按下 ⌘Q（或程序坞中的「退出」）。
    ///
    /// 系统侧的退出已被取消，由事件循环按关闭窗口的规则处理。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    QuitRequested,
    /// 菜单栏的「设置…」（或 ⌘,）被触发；设置已打开时关闭设置。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenSettings,
    /// 菜单栏的「更新日志」被触发。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OpenReleaseNotes,
    /// 菜单栏的「安装 winc 命令…」被触发。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    InstallShellCommand,
    /// macOS 上全屏过渡中红绿灯的位置发生变化（见 `crate::macos::TrafficLights`），
    /// 需要重新计算布局并重绘。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    TrafficLightsMoved,
    /// 询问是否将应用移到「应用程序」文件夹（见 `crate::macos::install`）；在窗口显示后处理，
    /// 提示框依附于窗口。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    OfferMoveToApplications,
    /// 「移到「应用程序」文件夹」提示框中的选择：是否移动，以及是否勾选「不再询问」。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MoveToApplications(bool, bool),
    /// 选择面板无法打开：参数为对应的选择与原因（没有桌面门户时原因为 `None`）。
    ///
    /// 仅出现在通过桌面门户选择文件的平台上。
    #[cfg(all(unix, not(target_os = "macos")))]
    PickFailed(Pick, Option<String>),
    /// 后台已取回更新日志（整理好的 Markdown）；请求失败或内容格式不符时为 `None`。
    ReleaseNotes(Option<String>),
    /// 后台检查更新发现了新版本（参数为版本号），或状态栏上的更新提示需要收起（`None`）。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateFound(Option<String>),
    /// 定时检查更新的时刻已到；设置中开启自动检查时执行检查。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateTick,
    /// 应用内更新的状态发生变化。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    UpdateState(crate::winc::UpdateState),
    /// 右键菜单中选中一项，参数是 `AppKit` 菜单项的编号（`tag`）。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MenuCommand(isize),
    /// 删除确认框中选择了「移到废纸篓」，参数是要删除的各个路径。
    Delete(Vec<PathBuf>),
    /// 「要保存修改吗」提示框中的一个按钮被点击。
    SaveChoice(SaveChoice),
    /// 状态栏路径的目录菜单中选中了一个文件。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FolderItem(PathBuf),
    /// 文件树拖动后确认了「移动」：第一个参数中的各个路径移入第二个参数指定的目录；
    /// 勾选「不再询问」时第三个参数为 `true`。
    Move(Vec<PathBuf>, PathBuf, bool),
    /// 「要在哪里打开」提示框中做了选择：目录、是否在新窗口打开、是否勾选了「不再询问」。
    OpenFolderIn(PathBuf, bool, bool),
    /// 后台格式化已有结果。
    Formatted,
    /// 提示框中回答了是否允许运行项目自带的格式化工具。
    ToolChoice(ToolAnswer),
    /// 确认框中同意运行参数所指的运行配置。
    RunApproved(Box<winc_config::RunConfig>),
    /// 提示框中回答了是否信任 Git 仓库（第一个参数）指纹为第二个参数的配置。
    GitTrust(PathBuf, u64, bool),
    /// 已取得用户 shell 的环境，格式化工具在其 `PATH` 中查找。
    FormatterReady,
    /// 后台查找定义有新进展。
    Definitions,
    /// 已打开目录中有文件被外部修改。
    FilesChanged,
    /// 后台读到一段标准输入（`winc -`）。
    StdinRead,
    /// 新启动的 `winc` 进程交来要打开的目录、文件及其激活令牌（见 [`crate::instance`]）。
    ///
    /// 由应用层决定切换到哪个窗口或新建窗口，并经通道答复是否接收；该事件不会分发到窗口。
    Launched(crate::instance::Launch, std::sync::mpsc::Sender<bool>),
    /// 系统交来要打开的文件和目录（拖到程序坞图标上、文件管理器中的「打开方式」）。
    ///
    /// 处理方式同 [`UserEvent::Launched`]：由应用层决定在哪个窗口打开。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Opened(Vec<PathBuf>),
    /// 程序坞菜单中选择了「新建窗口」。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NewWindow,
    /// 提示框中选择切换到已打开参数所指目录的另一个窗口。
    FocusFolder(PathBuf),
    /// 保存磁盘上已被修改的文档时，提示框上选择了是否覆盖。
    Overwrite(DocumentId, OverwriteChoice),
    /// 用户确认放弃未保存的修改，按指定编码重新打开文档。
    ReopenEncoding(DocumentId, winc_editor::Encoding),
    /// 后台完成了一张图片的解码，或一个 SVG 的绘制。
    ImageReady,
    /// 粘贴到文件树中的文件已在后台复制完成：新路径，或失败的原因；以及这次复制。
    Pasted(Result<Vec<PathBuf>, String>, Arc<winc_editor::Copying>),
    /// 拖进文件树的文件和目录已在后台复制完成：各个副本的路径、复制失败的说明，以及这次复制。
    Copied(Vec<PathBuf>, Vec<String>, Arc<winc_editor::Copying>),
    /// 窗口已关闭时点击了程序坞中的图标（macOS）：重新显示窗口。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Reopen,
    /// 后台的 `git status` 已返回结果。
    GitStatus,
    /// 后台已取得某个文件在 Git 中的版本，可以打开差异。
    GitContent,
    /// 后台已重新比较完一个差异标签。
    Rediffed,
    /// 后台已提取完大纲的符号。
    Outline,
    /// 后台的提交、回滚已完成，或已取得上一次提交的信息。
    GitCommit,
    /// 确认框中确认回滚参数中的文件。
    GitRollback(Vec<PathBuf>),
    /// 确认框中确认中止进行中的合并、变基等操作。
    GitAbort,
    /// 提示框中选择切换到参数中的分支；为 `true` 时先储藏未提交的更改，否则把更改带到新分支。
    GitSwitchBranch(winc_editor::git::Branch, bool),
    /// 确认框中确认强制推送当前分支。
    GitForcePush,
    /// 提示框中确认发布、推送或同步当前分支；为 `true` 时勾选了「不再询问」。
    GitPushConfirmed(crate::app::git::commit::PushIntent, bool),
    /// 提示框中确认提交已勾选的文件并推送。
    GitCommitAndPush,
    /// 提示框中确认撤销上一次提交。
    GitUndoCommit,
    /// 后台已取得某个文件在 `HEAD` 中的内容（行号栏改动标记的基准）。
    GitBase,
    /// 后台的分支操作（获取、拉取、推送、签出等）已完成。
    GitOperated,
    /// 后台的 Git 操作发来了输入请求。
    GitQuestion,
    /// 后台到远端主机的连接已建立或已失败。
    RemoteConnected,
    /// 远程选择面板在后台读好了一个目录。
    RemoteListed,
    /// 后台对远端目录的监视已经启动。
    RemoteWatching,
    /// 到远端主机的连接意外中断。
    RemoteClosed(winc_backend::Target),
    /// 后台的文件操作（新建、改名、移动、删除）已完成。
    FileWorked,
    /// 后台为远端文件的格式化选好了工具。
    FormatPlanned,
    /// 连接远端主机的 `ssh` 发来了输入请求。
    RemoteQuestion,
    /// 后台到远端主机的连接到达了新的阶段。
    RemoteStage,
    /// 远端主机有无响应的状况变了：状态栏的连接按钮随之重绘。
    RemoteHealth,
    /// 后台已读取三栏合并的两侧。
    GitMergeSides,
    /// 后台已取得日志的一批提交、一个提交的详情或一个文件的差异。
    GitLog,
    /// 终端的辅助进程发来了消息。
    Terminal,
    /// 确认框中确认关闭参数所指的终端标签（其中仍有程序在运行）。
    TerminalClose(u16),
    /// 确认框中确认重新启动参数所指的终端标签（伪终端已断开，其中的程序可能仍在运行）。
    TerminalRestart(u16),
    /// 需要授权的 `winc` 命令安装或卸载已结束。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ShellCommandDone(crate::shell_command::Done),
    /// 本机可用 shell 的检测已完成。
    TerminalShells(Vec<terminal::shells::ShellChoice>),
    /// 退出或关闭窗口前的确认结果（终端中有程序在运行）：用户确认时为真，取消时为假。
    TerminalQuit(bool),
    /// 确认框中确认把文字粘贴到指定的终端标签（多行文字，且程序未开启括号粘贴）。
    TerminalPaste(u16, String),
    /// 确认框中确认日志中对一个提交的操作（签出该提交、硬重置）。
    GitLogConfirmed(git::log::LogConfirmed),
    /// 确认框中确认移除参数所指的工作树；第二个参数是一并删除的分支（勾选「同时删除分支」时）。
    GitRemoveWorktree(PathBuf, Option<String>),
    /// 后台已计算完一个文档的逐行追溯。
    GitBlame,
    /// 后台已检查或重读被其他程序修改的文件。
    Reread,
    /// 后台已重读文件树中被其他程序修改的目录。
    TreeScanned,
    /// 后台已保存一个大文件。
    Saved,
    /// 后台已读取一个刚打开的文件。
    Loaded,
    /// 后台已写入一批热退出备份。
    BackedUp,
    /// 跨文件替换已在后台写回全部未打开的文件。
    Replaced,
    /// 大文件的后台断行已完成。
    Wrapped,
    /// 确认框上确认了全部替换；参数为弹出确认框时结果列表中的匹配总数。
    ReplaceAllConfirmed(usize),
}

/// 编辑区顶部输入条的当前用途。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// 跳转到行（`⌃G`）。
    GoTo,
    /// 储藏改动的说明（可为空）。
    Stash,
    /// 新 Git 标签的名称；`at` 为标签指向的提交（完整的提交号）。
    NewTag { at: String },
    /// 分支的新名称；`old` 为当前名称。
    RenameBranch { old: String },
    /// Vim 模式下查找的模式；`forward` 为真时向后文查找。
    VimSearch { forward: bool },
    /// Vim 模式的命令行。
    VimCommand,
}

impl PromptKind {
    /// 是否为 Vim 模式的命令行或查找：输入条显示在状态栏所在的一行。
    pub fn is_vim(&self) -> bool {
        matches!(self, Self::VimSearch { .. } | Self::VimCommand)
    }
}

/// 键盘焦点所在的区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Tree,
    /// 查找条的查找输入。
    Search,
    /// 查找条第二行的替换输入。
    Replace,
    /// 跳转条，或文件树中输入名字的那一行。
    Prompt,
    /// 侧栏搜索面板的查找输入。
    GlobalQuery,
    /// 侧栏搜索面板的替换输入。
    GlobalReplace,
    /// 侧栏搜索面板的「包含」输入。
    GlobalInclude,
    /// 侧栏搜索面板的「排除」输入。
    GlobalExclude,
    /// 侧栏搜索面板的结果列表。
    Results,
    /// 终端面板。
    Terminal,
    /// 终端的查找条。
    TerminalFind,
    /// 终端标签上的重命名输入框。
    TerminalRename,
    /// 快速打开的输入框（⌘P）。
    Picker,
    /// 设置浮层中正在编辑的字体名输入框。
    Setting,
    /// 设置浮层已打开且没有输入框在编辑：方向键在设置项之间移动并修改值。
    Settings,
    /// 设置中「快捷键」页的搜索框。
    KeySearch,
    /// 设置浮层分类栏顶部的搜索框。
    SettingsSearch,
    /// 侧栏提交视图的提交信息输入框。
    CommitMessage,
    /// 侧栏提交视图的改动列表。
    CommitList,
    /// 文件视图底部大纲的符号列表。
    Outline,
    /// 日志标签标题栏的筛选输入框。
    LogFilter,
    /// 功能对话框（新建工作树）里的输入框。
    Dialog,
}

impl Focus {
    /// 是否由某个输入框持有焦点（光标闪烁、输入法、剪贴板都按输入框处理）。
    pub fn is_field(self) -> bool {
        !matches!(
            self,
            Focus::Editor
                | Focus::Tree
                | Focus::Results
                | Focus::Settings
                | Focus::CommitList
                | Focus::Outline
                | Focus::Terminal
        )
    }

    /// 是否由侧栏搜索面板持有焦点（四个输入框或结果列表）。
    pub fn in_panel(self) -> bool {
        matches!(
            self,
            Focus::GlobalQuery
                | Focus::GlobalReplace
                | Focus::GlobalInclude
                | Focus::GlobalExclude
                | Focus::Results
        )
    }
    /// 是否由终端面板持有焦点（网格、查找框、标签上的重命名输入框）。
    pub fn in_terminal(self) -> bool {
        matches!(
            self,
            Focus::Terminal | Focus::TerminalFind | Focus::TerminalRename
        )
    }
}

/// 鼠标悬停的目标，用于绘制悬停效果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hover {
    None,
    /// 文件视图底部大纲里的一处（符号行、行首箭头、标题行及其右端的按钮）。
    Outline(OutlineTarget),
    /// 编辑区右侧的缩略图。
    Minimap,
    /// 应用绘制的标题栏上的窗口按钮。
    WindowControl(WindowControl),
    /// 可以拖动改变窗口大小的边缘，参数是方向。
    ResizeEdge(winit::window::ResizeDirection),
    /// 解决合并冲突的按钮。
    Merge(git::merge::MergeTarget),
    /// 快速打开面板或位置浮窗中的一行，参数为结果的序号（从 0 起）。
    PickerRow(usize),
    /// 选择面板里的一个按钮。
    PickerPart(PickerPart),
    ProjectName,
    SidebarButton,
    NewTab,
    /// 标签栏无法容纳全部标签时显示在最右端的「⌄」，用于列出全部标签。
    TabList,
    /// 标签栏右端「源代码 / 分栏 / 预览」三段切换中的一段。
    Preview(winc_editor::PreviewMode),
    /// 标签栏右端「在浏览器中打开」。
    Browser,
    /// Markdown 预览里的链接（`MarkdownPreview::links` 的序号）。
    MarkdownLink(usize),
    /// Markdown 预览左侧的标题指示条或展开的标题列表。
    MarkdownToc(crate::app::preview::markdown::TocTarget),
    /// PDF 预览右下角工具栏上的按钮。
    PdfTool(crate::app::preview::pdf::PdfTool),
    Search(SearchButton),
    TreeRow(usize),
    /// 历史差异侧栏标题行中切换树形与列表的按钮。
    LogDiffLayout,
    /// 标签序号，以及是否悬停在该标签的关闭按钮上。
    Tab(usize, bool),
    Splitter,
    /// 编辑区和终端面板之间的间距（拖动可调整面板大小）。
    TerminalDivider,
    /// 文件视图中大纲标题行的上沿（上下拖动可调整大纲的高度）。
    OutlineDivider,
    /// 分栏预览中间的分隔线（左右拖动可调整两侧的宽度）。
    SplitDivider,
    /// 终端面板标签栏上的标签、关闭按钮、「+」和位置按钮。
    TerminalHeader(terminal::HeaderHit),
    /// 终端中命令块的色带与吸顶的命令行。
    TerminalBlock(terminal::BlockHit),
    Placeholder(PlaceholderAction),
    /// 开始页上的一个可点击项。
    Start(StartAction),
    /// 文件树工具栏上的按钮。
    TreeTool(TreeTool),
    /// 文件树工具栏收起后的「⋯」按钮。
    TreeMore,
    /// 侧栏顶部的「文件」「搜索」切换按钮。
    ViewTab(SidebarView),
    /// 搜索面板中的按钮。
    Panel(PanelButton),
    /// 搜索结果的行序号。
    Result(usize),
    /// 悬停某一行搜索结果时出现的按钮，参数依次为行序号和按钮。
    RowAction(usize, RowAction),
    /// 状态栏路径中的一段，参数是段序号。
    Crumb(usize),
    /// 状态栏右侧可点击的一项（缩进、行尾）。
    Status(StatusItem),
    /// 编辑区顶部钉住的一层，参数是层级。
    Sticky(usize),
    /// 编辑区行号栏：显示可折叠行的箭头。
    Gutter,
    /// 已折叠行末尾的 `⋯`，参数是行号（从 0 起）。
    Folded(usize),
    /// 超长行截断说明中的「显示整行」，参数是行号（从 0 起）。
    LongLine(usize),
    /// 滚动条的可点击区域。
    Scrollbar(Scrollbar),
    /// 提示条上的按钮。
    NoticeButton,
    /// 冲突卡片上的按钮。
    ConflictButton(ConflictAction),
    /// 差异标签中的可点击目标。
    Diff(DiffTarget),
    /// 提交视图中的可点击目标。
    Commit(CommitTarget),
    /// 行号栏改动卡片上的按钮。
    Change(ChangeTarget),
    /// 行号栏的改动标记，参数为改动的序号。
    ChangeMark(usize),
    /// 滚动条（概览标尺）上的改动标记，参数为改动的序号。
    RulerChange(usize),
    /// 日志标签中的可点击目标。
    Log(git::log::LogTarget),
    /// 逐行追溯栏中的一行，参数为行序号（从 0 起）。
    Blame(usize),
}

impl Hover {
    /// 是否为悬停足够久后会显示提示文字的按钮。
    pub fn has_tooltip(self) -> bool {
        matches!(
            self,
            Hover::TreeTool(_)
                | Hover::TerminalHeader(
                    terminal::HeaderHit::Position
                        | terminal::HeaderHit::ShellMenu
                        | terminal::HeaderHit::Find(_)
                        | terminal::HeaderHit::Hide
                        | terminal::HeaderHit::Run(_),
                )
                | Hover::Status(StatusItem::Terminal | StatusItem::Remote | StatusItem::Settings)
                | Hover::TreeMore
                | Hover::SidebarButton
                | Hover::LogDiffLayout
                | Hover::Tab(..)
                | Hover::TabList
                | Hover::Preview(_)
                | Hover::Browser
                | Hover::ViewTab(_)
                | Hover::Outline(OutlineTarget::FoldAll)
                | Hover::Search(_)
                | Hover::Panel(
                    PanelButton::Tool(_)
                        | PanelButton::Details
                        | PanelButton::Expand
                        | PanelButton::Case
                        | PanelButton::Word
                        | PanelButton::Regex
                        | PanelButton::ReplaceAll
                )
                | Hover::RowAction(..)
                | Hover::Change(_)
                | Hover::RulerChange(_)
                | Hover::Log(
                    git::log::LogTarget::CopyOid
                        | git::log::LogTarget::Refresh
                        | git::log::LogTarget::Avatar(_)
                        | git::log::LogTarget::Row(_)
                        | git::log::LogTarget::File(_)
                        | git::log::LogTarget::Line(..)
                )
                | Hover::Blame(_)
                | Hover::PickerPart(PickerPart::Pull | PickerPart::Push | PickerPart::Fetch)
                | Hover::Commit(
                    CommitTarget::Log
                        | CommitTarget::Sync
                        | CommitTarget::Location
                        | CommitTarget::Refresh
                        | CommitTarget::More
                        | CommitTarget::Commit
                        | CommitTarget::CommitMenu
                        | CommitTarget::OpenFile(_)
                        | CommitTarget::RevertFile(_)
                        | CommitTarget::GroupRevert(_)
                        | CommitTarget::Check(_)
                )
                | Hover::Diff(
                    DiffTarget::Previous
                        | DiffTarget::Next
                        | DiffTarget::PreviousFile
                        | DiffTarget::NextFile
                        | DiffTarget::Layout
                        | DiffTarget::Collapse
                        | DiffTarget::Revert(_)
                        | DiffTarget::Include(_)
                        | DiffTarget::IncludeLine(..)
                )
                | Hover::Merge(git::merge::MergeTarget::Previous | git::merge::MergeTarget::Next)
        )
    }
}

/// 正在进行的鼠标拖动。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    None,
    /// 拖选文字。
    ///
    /// `by_word` 表示双击后按词扩展；`origin` 是起点（按词时是起始词的范围）。
    Select {
        by_word: bool,
        origin: (usize, usize),
    },
    /// 拖选整行：三击或点击行号栏后，按整行扩大选区；`line` 是按下的那一行。
    SelectLine {
        line: usize,
    },
    /// 列选择：`⇧⌥` 拖动或中键拖动；`anchor` 是按下处的（行，显示列）。
    Columns {
        anchor: (usize, usize),
    },
    /// `⌥` 拖动：添加一个选区；`origin` 是按下处的位置（字符偏移）。
    AddSelect {
        origin: usize,
    },
    Splitter {
        start_x: f32,
        start_width: f32,
    },
    /// 在日志详情中拖选文字（`App::drag_log_text`）。
    LogText,
    /// 在 Markdown 预览中拖选文字（`App::drag_markdown`）。
    MarkdownText,
    /// 在表格中拖选单元格（`App::drag_table`）。
    TableCells,
    /// 拖动图片预览中的图片（`App::drag_picture`）；`at` 是上一次的鼠标位置（窗口坐标）。
    Picture {
        at: (f32, f32),
    },
    /// 在终端中拖选文字。
    TerminalSelect,
    /// 终端中的程序启用了鼠标上报：按住参数所示的按键拖动，松开时上报。
    TerminalMouse(winc_pty::input::Button),
    /// 拖动编辑区与终端面板之间的间距：按下时的坐标与面板当时的尺寸。
    ///
    /// 面板位于下方时为纵坐标和高度，位于右侧时为横坐标和宽度。
    TerminalDivider {
        start: f32,
        size: f32,
    },
    /// 拖动大纲标题行的上沿：按下时的纵坐标、符号列表当时的高度（逻辑像素）。
    OutlineDivider {
        start: f32,
        size: f32,
    },
    /// 拖动分栏预览中间的分隔线：按下时的横坐标、源代码一侧当时的宽度（逻辑像素）。
    SplitDivider {
        start_x: f32,
        start_left: f32,
    },
    /// 拖动日志标签中列表与详情之间的分隔线：按下时的横坐标、详情栏当时的宽度。
    LogDivider {
        start_x: f32,
        start_width: f32,
    },
    /// 拖动缩略图的滑块；指针在纵坐标 `from`（逻辑像素）时，滚动位置是 `scroll`。
    Minimap {
        from: f32,
        scroll: f64,
    },
    /// 拖动滚动条。
    ///
    /// `grab` 是按下点到滑块起点（顶边或左边）的距离（像素）。
    Scrollbar {
        bar: Scrollbar,
        grab: f32,
    },
    /// 在当前持有焦点的输入框中拖选文字。
    Field,
    /// 在差异右侧拖选：`unit` 为扩展单位（字、词或行），`origin` 为按下时选中的范围
    /// （右侧文档中的字符偏移）；`add` 为 `true` 时表示 `⌥` 拖动，只修改主选区并保留其余光标。
    DiffSelect {
        unit: diff::SelectUnit,
        origin: (usize, usize),
        add: bool,
    },
    /// 在差异右侧进行列选择（`⇧⌥` 拖动、中键拖动）；`anchor` 是按下处的（行，显示列）。
    DiffColumns {
        anchor: (usize, usize),
    },
    /// 在差异的只读一侧拖选；`unit`、`origin` 的含义同 [`Drag::DiffSelect`]，
    /// 位置按第 `side` 侧的文本计算。
    DiffSelectRead {
        side: usize,
        unit: diff::SelectUnit,
        origin: (usize, usize),
    },
}

/// 一次鼠标按下的记录，用于识别连击。
#[derive(Debug, Clone, Copy)]
struct Click {
    /// 按下的时刻。
    at: Instant,
    /// 按下位置的横坐标（逻辑像素）。
    x: f32,
    /// 按下位置的纵坐标（逻辑像素）。
    y: f32,
    /// 连击次数：1 为单击，2 为双击选词，3 为三击选行。
    count: u8,
}

/// 光标闪烁的半周期，即显示与隐藏各持续的时间。
///
/// 取自调参：半周期为 530 ms 时闪烁清晰可辨，且不显得跳动。
const BLINK: Duration = Duration::from_millis(530);
/// 光标停止闪烁、保持显示之前的无输入时长。
///
/// 闪烁停止后不再定时重绘，窗口长时间无变化时不再按 `BLINK` 的间隔唤醒事件循环。
const BLINK_STOP: Duration = Duration::from_secs(10);
/// 两次按下判定为连击的最大间隔。
///
/// 400 ms 不得短于系统双击间隔的常见设置，否则正常的双击会被判定为两次单击。
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// 全局搜索进行中，界面主动收取一次结果的间隔。
///
/// 取自调参：结果每 100 ms 并入界面一次时，搜索期间的输入仍能及时响应。
const GLOBAL_POLL: Duration = Duration::from_millis(100);

/// 「只显示匹配的行」的缓存键，各项依次为：文档、版本号、匹配的行数、已搜索到的位置、是否有查询。
type FilterKey = (DocumentId, u64, usize, usize, bool);

/// 同时固定的高亮的最大数量（个），等于高亮调色板的颜色数。
///
/// 超出时移除最早的一个并沿用其颜色，同一屏中的颜色互不相同。
pub(crate) const WORD_HIGHLIGHT_COUNT: usize = 6;

/// 固定高亮的一个查找内容：编辑区与差异两栏中，各处字面相同的文字铺上同一种颜色。
#[derive(PartialEq, Eq)]
pub(crate) struct WordHighlight {
    /// 要标注的文字。
    pub chars: Vec<char>,
    /// 是否只标注完整的词：由光标所在的词生成时为真，由选中的文字生成时为假。
    pub whole_word: bool,
}

/// 应用状态：编辑器、界面状态、绘制时记录的几何，以及窗口和事件循环的句柄。
///
/// 各功能的事件处理方法分布在 `app/` 的子模块中，均只读写本结构的字段。
pub struct App {
    /// 本帧绘制时记录的可点击范围与位置。
    pub hits: hits::Hits,
    // 编辑器、配置和历史
    /// 打开的文档、工作区、光标和选区。
    pub editor: Editor,
    /// 终端面板（终端模块）。
    pub(crate) terminal: terminal::TerminalPanel,
    /// 当前目录的运行配置。
    pub(crate) run: terminal::run::RunConfigs,
    /// 当前生效的配置。
    pub config: Config,
    /// 配置与状态文件的写回：文件的路径、配置文件当前的内容和配置的版本号。
    pub(crate) store: settings::Store,
    /// 配置中的主题选择；`Auto` 时跟随系统外观。
    pub theme_choice: ThemeChoice,
    /// 最近打开的目录和各目录的现场。
    pub state: State,
    /// 未保存内容的备份，用于退出后恢复。
    pub backup: lifecycle::backup::BackupState,
    /// 临时打开（`--wait`、`-`）时需要等待的标签和标准输入的读取状态。
    pub(crate) one_off: lifecycle::one_off::OneOff,
    /// 测试和测量中请求打开终端的目录；此时不真正打开终端。
    pub opened_terminal: Option<PathBuf>,

    // 外观
    /// 当前主题的颜色。
    pub palette: &'static Palette,
    /// 界面字体和等宽字体的字体、字号。
    pub fonts: ViewFonts,
    /// 各字体的行高、基线等尺寸和字符格的宽度（逻辑像素），随字体设置和缩放比例更新。
    pub(crate) metrics: lifecycle::TextMetrics,

    // 焦点、鼠标和键盘
    /// 键盘焦点所在的区域。
    pub focus: Focus,
    /// 鼠标与修饰键的状态：位置、悬停与拖动的目标、按下时所在的控件。
    pub(crate) pointer: input::Pointer,
    /// 当前的触摸接触点与正在识别的触摸手势。
    touch: input::touch::TouchState,
    /// 系统窗口及其状态。
    pub window: lifecycle::window::WindowState,

    // 光标和输入法
    /// 光标闪烁的状态。
    pub blink: animation::Blink,
    /// 输入法的状态。
    pub ime: input::keyboard::ImeState,

    // 布局，以及绘制时记录、点击时使用的几何
    /// 上一帧的布局：各区域的位置与大小，点击时据此确定点击目标。
    pub layout: Layout,
    /// 标签栏的标签和滚动状态。
    pub tabs: TabBar,
    /// 差异视图的界面状态。
    pub diff: diff::DiffState,
    /// 状态栏的界面状态。
    pub status: status::StatusState,
    /// 编辑区跨帧复用的缓存和待采用的计算结果。
    pub(crate) caches: editor::EditorCaches,
    /// 各类预览的状态：图片、表格、分栏、Markdown 与 PDF。
    pub(crate) preview: preview::PreviewState,
    /// 界面各处出现、消失与折叠的过渡动画。
    pub(crate) motion: animation::Motions,
    /// 更新日志的状态：待显示的更新提示、所在的标签和要显示的版本范围。
    pub(crate) release_notes: release_notes::ReleaseNotes,
    /// 窗口背景图的状态。
    pub(crate) wallpaper: lifecycle::wallpaper::WallpaperState,
    /// 状态栏键路径的缓存与扫描记录。
    pub key_path: status::breadcrumb::KeyPath,
    /// 侧栏及其中的面板：文件树、搜索、大纲，以及文件树中的重命名和拖入。
    pub(crate) panels: workspace::Panels,
    /// 滚动的状态：滚轮与触控板的输入、平滑滚动的过渡、惯性滚动和滚动条的显示。
    pub(crate) scroll: scroll::ScrollState,

    // 侧栏和文件树
    /// 跳转：位置历史、后台的定义查找和名字旁的提示。
    pub(crate) jumps: editor::Jumps,
    /// 覆盖在界面之上的浮层：右键菜单、快速打开、对话框、提示框和悬停提示。
    pub(crate) overlays: input::Overlays,
    /// 编辑区水平滚动条的内容宽度（逻辑像素），绘制时更新；没有水平滚动条时为 0。
    pub text_content_w: f32,
    /// 预览标签的文档；是否仍是预览标签见 [`App::preview_index`]。
    preview_tab: Option<winc_editor::DocumentId>,

    // 编辑区
    /// 剪贴板。
    pub(crate) clipboard: editor::clipboard::Clipboard,
    /// Vim 模式的状态；Vim 模式关闭时为 `None`。
    pub(crate) vim: Option<Box<vim::VimState>>,

    // 查找条和跳转条
    /// 编辑区顶部的输入条：查找条与跳转条。
    pub(crate) bars: editor::InputBars,
    /// 阅读时固定的词或短选区及其颜色编号，按标记时间从早到晚排列。
    pub(crate) word_highlights: Vec<(usize, WordHighlight)>,

    // 快速打开
    /// 与磁盘的同步：目录的监视，以及文件在后台的读取、保存和改动。
    pub(crate) disk: workspace::DiskState,
    /// 本窗口最近关闭的标签、最近切换到的文件和最近使用的命令。
    pub(crate) history: workspace::History,
    /// 两段组合键（如 `⌘K ⌘S`）的第一段，正在等待第二段。
    pub(crate) pending_chord: Option<crate::keymap::Stroke>,

    /// 设置浮层：当前页、分类和各输入框。
    pub settings: SettingsState,

    /// 保存时格式化和修复的状态。
    pub format: FormatState,
    /// Git 模块：仓库状态、文件树装饰，以及提交、日志、追溯、分支、合并各视图的状态。
    ///
    /// 模块关闭时为空。
    pub git: git::GitState,
    /// 远程模块：正在进行的连接。
    pub remote: remote::RemoteState,

    // 右键菜单、关闭和退出
    /// 删除文件使用的函数：正常运行时移到废纸篓，测试中替换为直接删除，避免写入用户的废纸篓。
    pub trash: fn(&Path) -> std::io::Result<()>,
    /// 关闭与退出流程的状态。
    pub closing: lifecycle::closing::Closing,
    /// 退出前是否需要询问用户（各窗口共用同一个值，见 [`crate::winc::Shared::ask_on_quit`]）。
    pub(crate) ask_on_quit: Arc<AtomicBool>,

    // 状态栏消息和悬停提示
    /// 文件已从磁盘删除、自动保存因此未写入且已提示过用户的文档；失去焦点时不再重复提示。
    deleted_told: HashSet<DocumentId>,
    /// 右上角提示条的状态。
    pub notice: lifecycle::notice::Notice,

    // 窗口和事件循环
    /// 帧的绘制与呈现：渲染器、下一帧的安排和测量。
    pub(crate) frame: lifecycle::FrameState,
    /// 本窗口与应用层的联系：事件代理、窗口编号和各窗口共用的状态。
    pub(crate) link: lifecycle::AppLink,
    /// 创建窗口时使用一次的参数：位置、是否全屏、共用的字形缓存和激活令牌。
    pub(crate) opening: lifecycle::Opening,
}

impl App {
    /// 请求在下一帧重绘本窗口。
    ///
    /// 文件树的可见行变化后都会请求重绘：其中尚未读取的目录在此交给后台（[`App::fetch_tree`]）。
    pub(crate) fn request_redraw(&mut self) {
        self.frame.dirty = true;
        self.fetch_tree();
    }

    /// 在后台线程上执行 `job`，完成后发送事件 `done` 唤醒界面线程处理结果（[`App::finished`]）。
    ///
    /// 测试中没有事件循环：在当前线程上执行 `job`，之后直接调用 [`App::finished`] 处理结果。
    ///
    /// 打开的目录在远端主机上时改用单独的线程（[`App::spawn_blocking_then`]）：这时的任务大多
    /// 要等待网络，耗时没有上限，放进线程池会占住池中的线程，各窗口的本机任务只能排队。
    fn spawn_then(&mut self, done: UserEvent, job: impl FnOnce() + Send + 'static) {
        if self.editor.root().is_some_and(winc_backend::is_remote) {
            return self.spawn_blocking_then(done, job);
        }
        match self.link.proxy.clone() {
            Some(proxy) => winc_editor::background::spawn(move || {
                job();
                let _ = proxy.send_event(done);
            }),
            None => {
                winc_backend::remote::allow_blocking(job);
                self.finished(done);
            }
        }
    }

    /// 在单独创建的线程上执行 `job`，完成后发送事件 `done`；其余行为同 [`App::spawn_then`]。
    ///
    /// 用于运行子进程并等待其结束、耗时没有上限的任务
    /// （[`winc_editor::background::spawn_blocking`]）。
    ///
    /// 测试中没有事件循环：在当前线程上执行 `job`，之后直接调用 [`App::finished`] 处理结果。
    fn spawn_blocking_then(&mut self, done: UserEvent, job: impl FnOnce() + Send + 'static) {
        match self.link.proxy.clone() {
            Some(proxy) => winc_editor::background::spawn_blocking(move || {
                job();
                let _ = proxy.send_event(done);
            }),
            None => {
                winc_backend::remote::allow_blocking(job);
                self.finished(done);
            }
        }
    }

    /// 在本窗口改动远端文件的线程上执行 `job`，完成后发送事件 `done`；其余行为同
    /// [`App::spawn_then`]。
    ///
    /// 这个线程按提交的顺序逐个执行：保存与改名、移动、删除不会交错，先发起的保存一定在随后
    /// 的改名之前写完。线程在第一次使用时创建，窗口关闭时结束。
    fn spawn_ordered_then(&mut self, done: UserEvent, job: impl FnOnce() + Send + 'static) {
        let Some(proxy) = self.link.proxy.clone() else {
            winc_backend::remote::allow_blocking(job);
            self.finished(done);
            return;
        };
        let task: OrderedTask = Box::new(move || {
            job();
            let _ = proxy.send_event(done);
        });
        let task = match &self.disk.ordered {
            Some(queue) => match queue.send(task) {
                Ok(()) => return,
                // 线程已经结束：重新创建。
                Err(mpsc::SendError(task)) => task,
            },
            None => task,
        };
        let (queue, tasks) = mpsc::channel::<OrderedTask>();
        let worker = std::thread::Builder::new()
            .name("winc-remote".into())
            .spawn(move || tasks.into_iter().for_each(|task| task()));
        // 线程无法创建时退回单独的线程：任务仍然执行，只是不再保证先后。
        self.disk.ordered = None;
        if worker.is_err() {
            winc_editor::background::spawn_blocking(task);
        } else if let Err(mpsc::SendError(task)) = queue.send(task) {
            winc_editor::background::spawn_blocking(task);
        } else {
            self.disk.ordered = Some(queue);
        }
    }

    /// 处理后台任务完成事件 `done`（见 [`App::spawn_then`]）对应的结果。
    ///
    /// 事件循环与测试中就地运行的任务都经由此处，事件与结果处理函数的对应关系只在此处定义。
    fn finished(&mut self, done: UserEvent) {
        match done {
            UserEvent::GitStatus => self.poll_git(),
            UserEvent::GitContent => self.poll_git_content(),
            UserEvent::GitCommit => self.poll_commit(),
            UserEvent::GitBase => self.poll_bases(),
            UserEvent::GitOperated => self.poll_branch_ops(),
            UserEvent::RemoteConnected => self.poll_remote(),
            UserEvent::RemoteListed => self.poll_remote_listings(),
            UserEvent::RemoteWatching => self.poll_remote_watcher(),
            UserEvent::FormatPlanned => self.poll_formatting(),
            UserEvent::FileWorked => self.poll_file_work(),
            UserEvent::GitMergeSides => self.poll_merge_sides(),
            UserEvent::GitLog => self.poll_log(),
            UserEvent::GitBlame => self.poll_blame(),
            UserEvent::Reread => self.poll_rereads(),
            UserEvent::TreeScanned => self.poll_tree_scans(),
            UserEvent::Saved => self.poll_saves(),
            UserEvent::Loaded => self.poll_loads(),
            UserEvent::BackedUp => self.collect_backups(false),
            UserEvent::Replaced => self.poll_replace_all(),
            UserEvent::Wrapped => self.poll_wraps(),
            other => debug_assert!(false, "{other:?} 不是后台任务做完的事件"),
        }
    }

    /// 返回本窗口的系统窗口编号；窗口尚未创建时为 `None`。
    pub(crate) fn window_id(&self) -> Option<winit::window::WindowId> {
        self.window.handle.as_ref().map(|w| w.id())
    }

    /// 返回鼠标位置（逻辑像素）。
    pub(crate) fn mouse(&self) -> (f32, f32) {
        self.pointer.mouse
    }

    /// 是否正在用鼠标拖动（拖选文字、拖动分隔条等）。
    pub(crate) fn dragging(&self) -> bool {
        self.pointer.drag != Drag::None
    }

    /// 是否按下主修饰键：macOS 上为 ⌘，其他系统上为 Ctrl（用于单击多选、比较、打开链接）。
    pub(crate) fn command_held(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.pointer.modifiers.super_key()
        } else {
            self.pointer.modifiers.control_key()
        }
    }

    /// 设置悬停目标。
    ///
    /// 目标是带提示的按钮时记录悬停开始的时刻，悬停足够久后才显示提示。
    pub(super) fn set_hover(&mut self, hover: Hover) {
        // 窗口内的菜单覆盖在上方时，下方的部件均不视为悬停（`hover_at` 同样如此）。
        let hover = if self.popup().is_some() {
            Hover::None
        } else {
            hover
        };
        if hover == self.pointer.hover {
            return;
        }

        // 鼠标移到预览的标题指示条上时，标题列表展开并淡入。
        let toc = |h: Hover| matches!(h, Hover::MarkdownToc(_));
        if toc(hover) && !toc(self.pointer.hover) {
            self.preview.markdown.toc_fade = self.clock(animation::TOC_FADE);
        }
        self.pointer.hover = hover;
        self.overlays.tooltip.since = hover.has_tooltip().then(Instant::now);
        self.overlays.tooltip.shown = false;
        self.request_redraw();
    }

    /// 结束所有进行中的拖动。
    ///
    /// 按住鼠标时切换到其他应用不会收到松开事件；若不结束拖动，切回后即使未按键，
    /// 移动鼠标时分隔条、选区、滚动条和标签仍会跟随移动。
    fn drop_drags(&mut self) {
        self.cancel_tree_drag();
        self.release_tab();
        self.pointer.drag = Drag::None;
        self.pointer.close_pressed = None;
        self.pointer.control_pressed = None;
    }

    fn reset_blink(&mut self) {
        self.blink.epoch = Instant::now();
        self.blink.visible = true;
    }

    /// 在每帧绘制之前推进搜索：继续搜索一个时间片；等待中的跳转已有结果时执行跳转，并按
    /// 搜索结果过滤行。
    fn advance_search(&mut self) {
        // 查找条关闭时也可能有等待中的跳转：⌘G、F3 在大文件中一个时间片内没有搜到下一处；
        // Vim 层的查找在查找条关闭时同样高亮匹配。
        let searching =
            self.bars.find.open || self.bars.find.pending_jump.is_some() || self.vim_highlights();
        if searching && let Some((id, _)) = self.update_diff_find() {
            // 记录的跳转只对当时的文档有效，切换标签后作废。
            match self.bars.find.pending_jump {
                Some((pending, pos, backwards)) if pending == id => {
                    self.jump_to_match(pos, backwards);
                }
                Some(_) => self.bars.find.pending_jump = None,
                None => {}
            }
            self.apply_line_filter();
            return;
        }
        // 焦点在预览上时查找预览的文字。
        let preview = searching && self.markdown_focused() && self.markdown_find_doc().is_some();
        let doc = if preview {
            self.preview.markdown.find_doc.as_ref()
        } else {
            self.editor.active()
        };
        match doc {
            Some(doc) if searching => {
                self.bars.search.update(doc);

                // 记录的跳转只对当时的文档有效，切换标签后作废。
                match self.bars.find.pending_jump {
                    Some((id, pos, backwards)) if id == doc.id => {
                        self.jump_to_match(pos, backwards);
                    }
                    Some(_) => self.bars.find.pending_jump = None,
                    None => {}
                }
            }
            // 标签全部关闭时，等待中的跳转作废；否则会一直按「尚未搜完」处理，每帧都继续绘制。
            None => self.bars.find.pending_jump = None,
            Some(_) => {}
        }
        self.apply_line_filter();
    }
}
