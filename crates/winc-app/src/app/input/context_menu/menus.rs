//! 各个菜单有哪些项：状态栏、文件树、标签、编辑区、终端、差异、日志、提交视图、项目、格式化工具。

use winc_editor::Encoding;
use winc_i18n::t;

use super::*;

/// 返回状态栏缩进菜单的菜单项：使用空格还是 Tab、宽度（子菜单，`size_title` 为其标题）、
/// 转换已有的缩进、从内容中重新检测。
///
/// `editable` 为 `false`（只读的文件）时不能转换。
pub fn indent_menu(size_title: &str, editable: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator, Submenu};

    let sizes = INDENT_SIZES.map(|n| Item(IndentSize(n), true)).collect();
    vec![
        Item(IndentUsingSpaces, true),
        Item(IndentUsingTabs, true),
        Separator,
        Submenu(size_title.to_owned(), sizes),
        Separator,
        Item(ConvertToSpaces, editable),
        Item(ConvertToTabs, editable),
        Separator,
        Item(DetectIndentation, true),
    ]
}

/// 返回状态栏编码菜单的菜单项：两个子菜单「通过编码重新打开」「通过编码保存」，内容是各种编码；
/// 「Unicode 转义」只在 `properties` 为真（`.properties` 文件）时列出。
///
/// `reopen` 返回能否按某种编码重新打开（未命名的文档都不能，大文件只能用 UTF-8），`hex` 表示能否
/// 改按十六进制查看（有对应的文件就能）；`save` 返回能否改用某种编码保存（只读的文档都不能，大
/// 文件只能用 UTF-8）。
pub fn encoding_menu(
    properties: bool,
    reopen: impl Fn(Encoding) -> bool,
    hex: bool,
    save: impl Fn(Encoding) -> bool,
) -> Vec<MenuEntry> {
    use MenuCommand::{ReopenEncoding, ReopenHex, SaveEncoding};
    use MenuEntry::{Item, Separator, Submenu};

    let listed = || {
        (CHOICES.iter().enumerate()).filter(move |(_, (encoding, _))| {
            properties || !matches!(encoding, Encoding::UnicodeEscapes { .. })
        })
    };
    let reopen = listed()
        .map(|(i, &(encoding, _))| Item(ReopenEncoding(i), reopen(encoding)))
        .chain([Separator, Item(ReopenHex, hex)]);
    let save = listed().map(|(i, &(encoding, _))| Item(SaveEncoding(i), save(encoding)));
    vec![
        Submenu(t!("menu-reopen-with-encoding").to_owned(), reopen.collect()),
        Submenu(t!("menu-save-with-encoding").to_owned(), save.collect()),
    ]
}

/// 返回状态栏行尾菜单的菜单项：LF、CRLF（文件当前是 CR 时还有 CR）。
///
/// `editable` 为 `false`（只读的文件）时不能更换。
pub fn line_ending_menu(current: LineEnding, editable: bool) -> Vec<MenuEntry> {
    LINE_ENDINGS
        .into_iter()
        .filter(|&eol| eol != LineEnding::Cr || current == LineEnding::Cr)
        .map(|eol| MenuEntry::Item(MenuCommand::LineEnding(eol), editable))
        .collect()
}

/// 返回状态栏只读菜单的菜单项。
///
/// `own` 表示文档自身已设为只读；`all` 表示窗口的全部文档已设为只读，此时文档自身的一项禁用。
pub fn read_only_menu(own: bool, all: bool) -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item(MenuCommand::ReadOnly(!own), !all),
        MenuEntry::Item(MenuCommand::ReadOnlyAll(!all), true),
    ]
}

/// 返回「快捷键」页中一行的菜单项：`has_keys` 表示命令现在有快捷键，`customized` 表示改过。
pub fn shortcut_menu(has_keys: bool, customized: bool) -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item(MenuCommand::ShortcutChange, true),
        MenuEntry::Item(MenuCommand::ShortcutAdd, has_keys),
        MenuEntry::Item(MenuCommand::ShortcutRemove, has_keys),
        MenuEntry::Separator,
        MenuEntry::Item(MenuCommand::ShortcutReset, customized),
    ]
}

/// 返回「键位方案」菜单的菜单项：各方案，从本机找到的 `sources` 处编辑器导入，以及恢复全部
/// 默认（`customized` 表示改过快捷键，否则不可选）。
pub fn keymap_menu(sources: usize, customized: bool) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = (crate::keymap::Preset::ALL.into_iter())
        .map(|preset| MenuEntry::Item(MenuCommand::KeymapPreset(preset), true))
        .collect();
    if sources > 0 {
        entries.push(MenuEntry::Separator);
        entries.extend((0..sources).map(|i| MenuEntry::Item(MenuCommand::ImportKeymap(i), true)));
    }
    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item(MenuCommand::ResetAllShortcuts, customized));
    entries
}

/// 返回状态栏语言菜单的菜单项：有文件名（`named`）时第一项是按文件名识别，然后是纯文本和
/// 最近选择的语言 `recent`（语言的标识，最近的在前），其余语言按名字排序，不重复列出。
pub fn document_language_menu(named: bool, recent: &[String]) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    let mut languages: Vec<_> = winc_syntax::languages().iter().enumerate().collect();
    languages.sort_by_key(|(_, l)| l.name.to_lowercase());
    let mut menu = Vec::new();
    if named {
        menu.extend([Item(MenuCommand::DetectLanguage, true), Separator]);
    }
    menu.push(Item(MenuCommand::UsePlainText, true));
    let leading = menu.len();
    for id in recent {
        if let Some(at) = languages.iter().position(|(_, l)| l.id == id.as_str()) {
            menu.push(Item(MenuCommand::UseLanguage(languages.remove(at).0), true));
        }
    }
    if menu.len() > leading {
        menu.push(Separator);
    }
    menu.extend((languages.into_iter()).map(|(i, _)| Item(MenuCommand::UseLanguage(i), true)));
    menu
}

/// 去掉 `menu` 中只能在本机执行的项，并合并因此相邻的分隔线。
///
/// 远端主机上的文件无法交给本机的文件管理器，除非它所在的目录挂载在本机（`mounted`，见配置项
/// `remote.localFolders`），此时显示的是本机上的那一份；`terminal` 为 `false`（终端模块关闭）时
/// 「在终端中打开」只能启动本机的终端程序，同样去掉。
pub fn without_local_only(menu: Vec<MenuEntry>, terminal: bool, mounted: bool) -> Vec<MenuEntry> {
    use MenuCommand::{LocationReveal, OpenInTerminal, Reveal};

    let mut kept: Vec<MenuEntry> = Vec::with_capacity(menu.len());
    for entry in menu {
        let dropped = match &entry {
            MenuEntry::Item(Reveal | LocationReveal, _) => !mounted,
            MenuEntry::Item(OpenInTerminal, _) => !terminal,
            MenuEntry::Separator => matches!(kept.last(), None | Some(MenuEntry::Separator)),
            _ => false,
        };
        if !dropped {
            kept.push(entry);
        }
    }
    if matches!(kept.last(), Some(MenuEntry::Separator)) {
        kept.pop();
    }
    kept
}

/// 返回文件树菜单的菜单项。
///
/// `kind` 是右键点中的那一项的类型，`None` 表示空白处（根目录）：根目录不能改名、删除，
/// 也没有相对路径。「在目录中查找」只对目录和根目录出现。`remote` 表示这一项在远端主机上，
/// 此时可以下载到本机。
pub fn tree_menu(kind: Option<EntryKind>, can_paste: bool, remote: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(NewFile, true), Item(NewFolder, true), Separator];
    menu.push(Item(Reveal, true));
    menu.push(Item(OpenInTerminal, true));
    if kind != Some(EntryKind::File) {
        menu.push(Item(FindInFolder, true));
    }
    menu.push(Separator);
    if kind.is_some() {
        menu.extend([Item(Cut, true), Item(Copy, true)]);
    }
    menu.push(Item(Paste, can_paste));
    if remote && kind.is_some() {
        menu.push(Item(Download, true));
    }
    menu.push(Separator);
    menu.push(Item(CopyPath, true));
    if kind.is_some() {
        menu.push(Item(CopyRelativePath, true));
        if kind == Some(EntryKind::File) {
            menu.push(Item(CopyAsPrompt, true));
        }
        menu.extend([Separator, Item(Rename, true), Item(Delete, true)]);
    }
    menu
}

/// 返回文件树里选中了多项、右键点在其中一项上时的菜单项：打开（`files`：选中的文件数，没有
/// 文件时灰显）、比较所选文件（正好两个文件时才有）、剪切、复制、复制路径、复制相对路径和删除，
/// 都作用于选中的各项。
///
/// 新建、改名等在多项选择下没有意义的项不出现。`remote` 表示选中的各项在远端主机上，此时
/// 可以下载到本机。
pub fn tree_selection_menu(files: usize, remote: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(OpenFiles, files > 0)];
    if files == 2 {
        menu.push(Item(CompareFiles, true));
    }
    menu.extend([Separator, Item(Cut, true), Item(Copy, true)]);
    if remote {
        menu.push(Item(Download, true));
    }
    menu.extend([
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, true),
    ]);
    if files > 0 {
        menu.push(Item(CopyAsPrompt, true));
    }
    menu.extend([Separator, Item(Delete, true)]);
    menu
}

/// 返回标签栏菜单的菜单项：右键点的是第 `first` 到第 `last` 个标签（从 0 起，只点击了一个标签时
/// 两者相同；多选时是选中的最左、最右一个），其中 `selected` 个是菜单作用的（多选时选中的
/// 个数，否则是 1），共 `count` 个。
///
/// `has_path` 表示该文档有路径，`in_workspace` 表示该文档在已打开的目录内，`any_saved`
/// 表示是否有没有未保存修改的标签。
pub fn tab_menu(
    (first, last): (usize, usize),
    selected: usize,
    count: usize,
    has_path: bool,
    in_workspace: bool,
    any_saved: bool,
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    vec![
        Item(Close, true),
        Item(CloseOthers, selected < count),
        Item(CloseLeft, first > 0),
        Item(CloseRight, last + 1 < count),
        Item(CloseSaved, any_saved),
        Item(CloseAll, true),
        Separator,
        Item(SplitRight, true),
        Item(SplitDown, true),
        Item(MoveToSplitRight, count > 1),
        Item(MoveToSplitDown, count > 1),
        Separator,
        Item(CopyPath, has_path),
        Item(CopyRelativePath, in_workspace),
        Separator,
        Item(Reveal, has_path),
        Item(RevealInTree, in_workspace),
    ]
}

/// 提交菜单里哪些操作可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitActions {
    /// 没有别的 Git 操作在运行，也没有停在合并、变基中间：会修改仓库的操作可用。
    pub enabled: bool,
    /// 这个提交是可以撤销的上一次提交。
    pub undo: bool,
    /// 有可以推送到的远程。
    pub push: bool,
    /// 提交是否在当前分支上；日志里判断不出时为 `None`，操作时再检查。
    pub on_head: Option<bool>,
    /// 列表里有它的子提交。
    pub child: bool,
    /// 它有父提交（不是第一个提交）。
    pub parent: bool,
    /// 提交视图里有可以提交的更改：可以修正、压缩到这个提交。
    pub changes: bool,
}

/// 日志里一个提交的右键菜单，分组和顺序：复制与拣选；签出与比较；重置、还原与撤销；改写历史与推送；
/// 指向这个提交的各分支、标签的子菜单，新建分支、标签；转到。
///
/// `actions` 说明哪些操作可用，`branch` 是当前分支名（分离的 `HEAD` 时为「HEAD」），写进重置子菜单
/// 的标题；`branches` 是指向这个提交的分支及其操作（见 [`crate::app::git::branch::branch_ops`]）。
/// 分支、标签不超过 [`BRANCH_SUBMENUS`]、[`TAG_SUBMENUS`] 个时各自一个子菜单，多了收进一个。
pub fn log_commit_menu(
    actions: CommitActions,
    branch: &str,
    branches: Vec<(String, Vec<MenuEntry>)>,
    tags: &[&str],
) -> Vec<MenuEntry> {
    use crate::app::git::branch::TagOp;
    use MenuCommand::*;
    use MenuEntry::{Item, Separator, Submenu};

    let enabled = actions.enabled;
    // 改写历史、推送只对当前分支上的提交；已经在当前分支上的提交不用拣选。
    let (history, foreign) = (
        actions.on_head != Some(false),
        actions.on_head != Some(true),
    );
    let mut menu = vec![
        Item(LogCopyOid, true),
        Item(LogCopyShortOid, true),
        Item(LogCopyMessage, true),
        Item(LogCopyPatch, true),
        Item(LogCherryPick, enabled && foreign),
        Separator,
        Item(LogCheckout, enabled),
        Item(LogNewWorktree, enabled),
        Item(LogCompareRevision, true),
        Separator,
        Submenu(
            t!("menu-log-reset-branch", branch = branch),
            vec![
                Item(LogResetSoft, enabled),
                Item(LogResetMixed, enabled),
                Separator,
                Item(LogResetHard, enabled),
            ],
        ),
        Item(LogRevert, enabled),
        Item(LogUndoCommit, enabled && actions.undo),
        Separator,
        Item(LogReword, enabled && history),
        Item(LogFixup, enabled && history && actions.changes),
        Item(LogSquashInto, enabled && history && actions.changes),
        Item(LogDropCommit, enabled && history),
        Item(LogInteractiveRebase, enabled && history),
        Item(LogPushUpTo, enabled && history && actions.push),
        Separator,
    ];
    // 提交上的分支、标签：每个一个子菜单，多了收进一个子菜单。
    let branch_title = |name: &str| t!("menu-log-branch", name = name);
    let group = t!("menu-log-branches");
    menu.extend(ref_submenus(branches, BRANCH_SUBMENUS, branch_title, group));
    let tags = (tags.iter().enumerate())
        .map(|(i, name)| {
            let ops = TagOp::ALL.map(|op| Item(LogTag(op, i), enabled));
            ((*name).to_owned(), ops.into())
        })
        .collect();
    let tag_title = |name: &str| t!("menu-log-tag", name = name);
    menu.extend(ref_submenus(
        tags,
        TAG_SUBMENUS,
        tag_title,
        t!("menu-log-tags"),
    ));
    menu.extend([
        Item(LogNewBranch, enabled),
        Item(LogNewTag, enabled),
        Separator,
        Item(LogGoTo, true),
        Item(LogGoToChild, actions.child),
        Item(LogGoToParent, actions.parent),
    ]);
    menu
}

/// 提交菜单中直接列出的分支子菜单最多几个，多了收进「分支」子菜单。
pub const BRANCH_SUBMENUS: usize = 2;
/// 提交菜单中直接列出的标签子菜单最多几个，多了收进「标签」子菜单。
pub const TAG_SUBMENUS: usize = 1;

/// 把引用 `refs`（名字和操作）排成子菜单：不超过 `max` 个时每个一个以 `title(名字)` 为题的
/// 子菜单，多了放进以 `group` 为题的一个子菜单，里面每个引用一个以名字为题的子菜单。
fn ref_submenus(
    refs: Vec<(String, Vec<MenuEntry>)>,
    max: usize,
    title: impl Fn(&str) -> String,
    group: &str,
) -> Vec<MenuEntry> {
    if refs.len() <= max {
        let each = |(name, ops): (String, Vec<MenuEntry>)| MenuEntry::Submenu(title(&name), ops);
        return refs.into_iter().map(each).collect();
    }
    let each = |(name, ops)| MenuEntry::Submenu(name, ops);
    vec![MenuEntry::Submenu(
        group.to_owned(),
        refs.into_iter().map(each).collect(),
    )]
}

/// 日志详情里一个改动文件的右键菜单：显示这次的差异、打开工作区里的文件、显示文件历史，
/// 在文件树、文件管理器中显示，复制路径。
///
/// `exists` 为 `false`（文件已不在磁盘上）时，打开、定位的几项灰显。`in_workspace` 表示
/// 文件在打开的目录里。
pub fn log_file_menu(exists: bool, in_workspace: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    vec![
        Item(GitShowDiff, true),
        Item(LogCompareWithLocal, exists),
        Item(GitOpenFile, exists),
        Item(FileHistory, true),
        Separator,
        Item(RevealInTree, exists && in_workspace),
        Item(Reveal, exists),
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, in_workspace),
    ]
}

/// 新建工作树对话框「基于」的菜单项：当前分支，然后是其余的本地分支、远程分支两组。
pub fn form_base_menu(branches: &[winc_editor::git::Branch]) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(FormBase(0), true)];
    for remote in [false, true] {
        let group: Vec<usize> = (0..branches.len())
            .filter(|&i| branches[i].remote == remote && !branches[i].current)
            .collect();
        if group.is_empty() {
            continue;
        }
        menu.push(Separator);
        menu.push(Item(GitHeading(remote), false));
        menu.extend(group.into_iter().map(|i| Item(FormBase(i + 1), true)));
    }
    menu
}

/// 返回提交视图里一个文件的菜单项：显示差异、打开文件，回滚、储藏、加入 `.gitignore`，在文件
/// 树中显示、在文件管理器中显示，复制路径。
///
/// `exists` 表示文件还在磁盘上，删掉的文件不能打开、不能在文件管理器中显示；`in_workspace`
/// 表示它在打开的目录里。`ignore` 是可以加入 `.gitignore` 的各种范围，只有未跟踪的文件才有；
/// 为空时没有这个子菜单。
pub fn commit_file_menu(
    exists: bool,
    in_workspace: bool,
    ignore: &[crate::app::git::commit::IgnoreKind],
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator, Submenu};

    let mut changes = vec![Item(GitRollbackFile, true), Item(GitStashFiles, true)];
    if !ignore.is_empty() {
        let kinds = ignore.iter().map(|&kind| Item(GitIgnore(kind), true));
        changes.push(Submenu(t!("menu-git-ignore").to_owned(), kinds.collect()));
    }
    let mut menu = vec![
        Item(GitShowDiff, true),
        Item(GitOpenFile, exists),
        Separator,
    ];
    menu.extend(changes);
    menu.extend([
        Separator,
        Item(RevealInTree, exists && in_workspace),
        Item(Reveal, exists),
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, in_workspace),
    ]);
    menu
}

/// 返回提交视图里选中了多个文件、右键点在其中一个上时的菜单项：打开（`any_exists`：有还在
/// 磁盘上的）、暂存、取消暂存、回滚、储藏、复制路径、复制相对路径，都作用于选中的各个文件。
pub fn commit_selection_menu(any_exists: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    vec![
        Item(GitOpenFile, any_exists),
        Separator,
        Item(CommitCheck(true), true),
        Item(CommitCheck(false), true),
        Separator,
        Item(GitRollbackFile, true),
        Item(GitStashFiles, true),
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, true),
    ]
}

/// 返回全局搜索结果里一行的菜单项：打开（目录行没有）；目录、文件行是复制路径、复制相对路径、
/// 在文件树中显示，匹配行是复制这一行的文字；最后是替换（`replace`：替换输入展开着）和从结果中
/// 移除。
///
/// `file` 表示右键点着的是目录或文件行，`dir` 表示是目录行（以树形显示时）。结果都来自打开的
/// 目录，因此相对路径、在文件树中显示总是可用。
pub fn search_result_menu(file: bool, dir: bool, replace: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = Vec::new();
    if !dir {
        menu.extend([Item(OpenFiles, true), Separator]);
    }
    if file {
        menu.extend([
            Item(CopyPath, true),
            Item(CopyRelativePath, true),
            Separator,
            Item(RevealInTree, true),
        ]);
    } else {
        menu.push(Item(Copy, true));
    }
    menu.push(Separator);
    if replace {
        menu.push(Item(ReplaceResult(file), true));
    }
    menu.push(Item(DismissResult, true));
    menu
}

/// 返回十六进制视图的右键菜单：复制（写成十六进制）、按文字复制、全选、转到偏移。
///
/// `selected` 为 `false`（空文件）时不能复制。
pub fn hex_menu(selected: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};
    vec![
        Item(Copy, selected),
        Item(HexCopyText, selected),
        Separator,
        Item(SelectAll, true),
        Item(HexGoTo, true),
    ]
}

/// 返回编辑区菜单的菜单项：转到定义与引用、书签、格式化与修复、「比较」子菜单、剪切复制粘贴。
///
/// `bookmarked` 决定添加、删除书签哪个可用；只读时不能格式化、剪切、粘贴，`saved` 为假时不能与
/// 已保存的版本比较。格式化和修复无法执行时点击后说明原因；没有选区时剪切、复制作用于整行。
/// 「粘贴」「与剪贴板比较」不检查剪贴板：弹出菜单不应读取其他应用复制的内容。
pub fn editor_menu(
    editable: bool,
    selected: bool,
    bookmarked: bool,
    markdown: bool,
    saved: bool,
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![
        Item(GoToDefinition, true),
        Item(FindReferences, true),
        Separator,
        Item(AddBookmark, !bookmarked),
        Item(RemoveBookmark, bookmarked),
        Item(ListBookmarks, true),
        Separator,
        Item(Format, editable),
    ];
    if selected {
        menu.push(Item(FormatSelection, editable));
    }
    menu.push(Item(Fix, editable));
    // 有选区时比较选中部分，菜单项的文字随之改变。
    let clipboard = match selected {
        true => CompareSelectionWithClipboard,
        false => CompareWithClipboard,
    };
    let compare = vec![Item(clipboard, true), Item(CompareWithSaved, saved)];
    menu.extend([
        Separator,
        MenuEntry::Submenu(t!("menu-compare").to_owned(), compare),
        Separator,
        Item(Cut, editable),
        Item(Copy, true),
        Item(Paste, editable),
    ]);
    if markdown {
        menu.push(Item(PastePlain, editable));
    }
    menu.push(Item(CopyReference, true));
    menu
}

/// 返回终端里的菜单项：右键点在命令块上时（`block`：报了命令行，能不能重新运行）先是复制命令、
/// 复制输出、重新运行；然后复制（有选区时）、粘贴（程序还在运行时）、全选，查找、清屏，最大化
/// 面板，新建、拆分、移到新分栏（`group`：面板是否在右侧，即移到下方；这一栏是否不止一个
/// 标签）、关闭终端。
pub fn terminal_menu(
    selected: bool,
    running: bool,
    block: Option<(bool, bool)>,
    group: (bool, bool),
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = match block {
        Some((text, rerun)) => vec![
            Item(CopyCommand, text),
            Item(CopyOutput, true),
            Item(Rerun, rerun),
            Separator,
        ],
        None => Vec::new(),
    };
    menu.extend([
        Item(Copy, selected),
        Item(Paste, running),
        Item(SelectAll, true),
        Separator,
        Item(TerminalFind, true),
        Item(TerminalClear, running),
        Separator,
        Item(TerminalMaximize, true),
        Item(TerminalDock, true),
        Separator,
        Item(NewTerminal, true),
        Item(SplitTerminal, running),
        Item(move_group_command(group.0), group.1),
        Item(MoveTerminalToEditor, true),
        Item(CloseTerminal, true),
    ]);
    menu
}

/// 返回差异标签里文字的菜单项：右侧（`right`）是剪切、复制、粘贴，光标在改动上
/// （`on_hunk`）时还有「回滚这处改动」；左侧只读，只有复制。差异是 Git 中的版本（`blame`）时
/// 最后是同时开关两侧的「逐行追溯」。
pub fn diff_menu(right: bool, on_hunk: bool, blame: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = if right {
        vec![
            Item(Cut, true),
            Item(Copy, true),
            Item(Paste, true),
            Separator,
            Item(GitRevertHunk, on_hunk),
        ]
    } else {
        vec![Item(Copy, true)]
    };
    if blame {
        menu.extend([Separator, Item(ToggleBlame, true)]);
    }
    menu
}

/// 返回项目名菜单的菜单项：最近的目录（不含当前目录；按住 `⌥` 时是「在新窗口中打开」），之后是「新
/// 建窗口」「打开目录…」「关闭目录」；有 WSL 发行版时另有「打开 WSL 目录」子菜单。
///
/// `recent`、`current` 为最近列表的长度和当前目录的位置，`opened` 表示是否已打开目录。`hosts` 为
/// `ssh` 配置中的主机数，远程模块关闭时为 `None`；有主机时「连接到 SSH 主机」是按主机列出的子
/// 菜单。`connected` 为真时「关闭目录」之前有「断开 SSH 连接」。
pub fn project_menu(
    recent: usize,
    current: Option<usize>,
    opened: bool,
    distros: usize,
    hosts: Option<usize>,
    connected: bool,
) -> Vec<MenuEntry> {
    use MenuEntry::{Alternate, Item, Separator, Submenu};

    let mut menu: Vec<MenuEntry> = (0..recent)
        .filter(|&i| Some(i) != current)
        .flat_map(|i| {
            [
                Item(MenuCommand::OpenRecent(i), true),
                Alternate(MenuCommand::OpenRecentInNewWindow(i), true),
            ]
        })
        .collect();
    if !menu.is_empty() {
        menu.push(Separator);
    }
    menu.push(Item(MenuCommand::NewWindow, true));
    menu.push(Item(MenuCommand::OpenFolder, true));
    if distros > 0 {
        let items = (0..distros).map(|i| Item(MenuCommand::OpenWslFolder(i), true));
        menu.push(Submenu(
            t!("cmd-open-wsl-folder").to_owned(),
            items.collect(),
        ));
    }
    match hosts {
        None => {}
        Some(0) => menu.push(Item(MenuCommand::RemoteConnect, true)),
        Some(hosts) => {
            let mut items: Vec<MenuEntry> = (0..hosts)
                .map(|i| Item(MenuCommand::RemoteHost(i), true))
                .collect();
            items.push(Separator);
            items.push(Item(MenuCommand::RemoteOther, true));
            menu.push(Submenu(t!("workspace-remote-submenu").to_owned(), items));
        }
    }
    if connected {
        menu.push(Item(MenuCommand::RemoteDisconnect, true));
    }
    menu.push(Item(MenuCommand::CloseFolder, opened));
    menu
}

/// 返回 `task`（格式化或修复）那一组「添加语言…」菜单的菜单项：这一组尚未列出的语言，有这类
/// 预设工具的排在前面，与其余语言之间用分隔线隔开。
///
/// `listed` 为这一组已列出的语言 ID。
pub fn language_menu(task: Task, listed: &[&str]) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    let (with_presets, others): (Vec<_>, Vec<_>) = winc_syntax::languages()
        .iter()
        .enumerate()
        .filter(|(_, l)| !listed.contains(&l.id))
        .partition(|(_, l)| winc_editor::format::presets(task, l.id).next().is_some());
    let item = |(i, _)| Item(MenuCommand::Language(i), true);

    let mut menu: Vec<MenuEntry> = with_presets.into_iter().map(item).collect();
    if !menu.is_empty() && !others.is_empty() {
        menu.push(Separator);
    }
    menu.extend(others.into_iter().map(item));
    menu
}

/// 返回设置中 `id` 语言的 `task`（格式化或修复）工具所在的设置项。
pub(in crate::app) fn tool_setting(task: Task, id: &'static str) -> Setting {
    match task {
        Task::Format => Setting::FormatCommand(id),
        Task::Fix => Setting::FixCommand(id),
    }
}

/// 返回在按钮下方弹出菜单时传给系统的位置：菜单外框的上沿与按钮相距 [`MENU_GAP`]，左边与按钮对齐。
pub(in crate::app) fn menu_below(button: Rect) -> (f32, f32) {
    (button.x, button.bottom() + MENU_GAP + MENU_TOP_PADDING)
}

/// 返回「语言」菜单的菜单项：跟随系统和各种语言，当前的选择打勾。
pub fn locale_menu() -> Vec<MenuEntry> {
    (0..winc_i18n::LocaleChoice::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::Locale(i), true))
        .collect()
}

/// 返回「打开工作树」菜单的菜单项，当前的选择打勾。
pub fn open_worktree_menu() -> Vec<MenuEntry> {
    (0..winc_config::WorktreeOpen::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::OpenWorktree(i), true))
        .collect()
}

/// 返回「Vim 剪贴板」菜单的菜单项，当前的选择打勾。
pub fn vim_clipboard_menu() -> Vec<MenuEntry> {
    (0..winc_config::VimClipboard::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::VimClipboard(i), true))
        .collect()
}

/// 返回「空白符」菜单的菜单项，当前的选择打勾。
pub fn whitespace_menu() -> Vec<MenuEntry> {
    (0..winc_config::RenderWhitespace::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::Whitespace(i), true))
        .collect()
}

/// 返回「界面缩放」菜单的菜单项：各个档位，当前的选择打勾。
pub fn zoom_menu() -> Vec<MenuEntry> {
    (0..crate::app::settings::ZOOM_STEPS.len())
        .map(|i| MenuEntry::Item(MenuCommand::Zoom(i), true))
        .collect()
}

/// 返回「正文宽度」菜单的菜单项，当前的选择打勾。
pub fn text_width_menu() -> Vec<MenuEntry> {
    (0..winc_config::TextWidth::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::TextWidth(i), true))
        .collect()
}

/// 返回「背景显示方式」菜单的菜单项，当前的选择打勾。
pub fn background_fit_menu() -> Vec<MenuEntry> {
    (0..winc_config::BackgroundFit::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::BackgroundFit(i), true))
        .collect()
}

/// 返回「背景图片」菜单的菜单项：选择图片、无，没有背景图片时「无」打勾。
pub fn background_menu() -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item(MenuCommand::BackgroundChoose, true),
        MenuEntry::Item(MenuCommand::BackgroundNone, true),
    ]
}

/// 返回终端「位置」菜单的菜单项：底部、右侧，当前的选择打勾。
pub fn terminal_position_menu() -> Vec<MenuEntry> {
    [false, true]
        .map(|right| MenuEntry::Item(MenuCommand::TerminalPosition(right), true))
        .into()
}

/// 返回终端「标签位置」菜单的菜单项：顶部、右侧，当前的选择打勾。
pub fn terminal_tab_position_menu() -> Vec<MenuEntry> {
    [false, true]
        .map(|right| MenuEntry::Item(MenuCommand::TerminalTabPosition(right), true))
        .into()
}

/// 返回「Meta 键」菜单的菜单项，当前的选择打勾。
pub fn option_as_meta_menu() -> Vec<MenuEntry> {
    (0..winc_config::OptionAsMeta::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::OptionAsMeta(i), true))
        .collect()
}

/// 返回「默认 Shell」菜单的菜单项：系统默认、检测到的 `shells` 个 shell 和「自定义命令…」，
/// 当前的选择打勾。
pub fn default_shell_menu(shells: usize) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};
    let mut menu = vec![Item(MenuCommand::DefaultShell(None), true)];
    if shells > 0 {
        menu.push(Separator);
        menu.extend((0..shells).map(|i| Item(MenuCommand::DefaultShell(Some(i)), true)));
    }
    menu.extend([Separator, Item(MenuCommand::DefaultShellCustom, true)]);
    menu
}

/// 字体菜单中直接列出的常用编程字体：各平台自带的等宽字体和常见的开源编程字体，按字母顺序。
///
/// 系统声明等宽的字体还包括泰文、日文字体和符号字体，全部直接列出时难以找到编程字体，因此只
/// 直接列出这些，其余的等宽字体排在「其他字体」的最前面。Maple Mono 是本分支未配置编辑器字体时
/// 的默认字体，因此也列在这里。
pub(crate) const COMMON_FONTS: &[&str] = &[
    "Cascadia Code",
    "Cascadia Mono",
    "Consolas",
    "Courier New",
    "DejaVu Sans Mono",
    "Fira Code",
    "Hack",
    "IBM Plex Mono",
    "Iosevka",
    "JetBrains Mono",
    "Liberation Mono",
    "Maple Mono",
    "Maple Mono NF CN",
    "Menlo",
    "Monaco",
    "Roboto Mono",
    "SF Mono",
    "Source Code Pro",
    "Ubuntu Mono",
];

/// 界面字体菜单中直接列出的常用界面字体：各平台的系统界面字体及其中文字体和常见的开源界面
/// 字体，按字母顺序。
pub(crate) const COMMON_UI_FONTS: &[&str] = &[
    "Cantarell",
    "DejaVu Sans",
    "Helvetica Neue",
    "Hiragino Sans GB",
    "Inter",
    "Microsoft YaHei",
    "Noto Sans",
    "Noto Sans SC",
    "PingFang SC",
    "Roboto",
    "SF Pro",
    "Segoe UI",
    "Source Han Sans SC",
    "Ubuntu",
];

/// 返回字体菜单的菜单项：第一项是「系统默认」（终端字体为「跟随编辑器」），其后是已安装的
/// 字族，当前的选择打勾；最后一组是子菜单「其他字体」和「后备字体」（见 [`fallback_font_menu`]）。
///
/// 直接列出已安装的常用字体（编辑器和终端为 [`COMMON_FONTS`]，界面和预览正文为
/// [`COMMON_UI_FONTS`]）和当前的字体（`current` 中的第一个）。编辑器和终端的「其他字体」中先列
/// 其余的等宽字体，再列非等宽字体。
pub fn font_menu(setting: Setting, current: Option<&str>) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator, Submenu};

    let item = |(i, _)| Item(MenuCommand::Font(i), true);
    let fonts = winc_render::installed_fonts().iter().enumerate();
    let primary = winc_config::font_list(current).first().copied();
    let ui = matches!(setting, Setting::UiFont | Setting::MarkdownFont);
    let common = if ui { COMMON_UI_FONTS } else { COMMON_FONTS };
    let (first, rest): (Vec<_>, Vec<_>) = fonts.partition(|(_, family)| {
        common.contains(&family.name.as_str()) || primary == Some(family.name.as_str())
    });
    let (mono, other): (Vec<_>, Vec<_>) = rest.into_iter().partition(|(_, f)| !ui && f.monospace);
    let mut menu = vec![Item(MenuCommand::FontDefault, true)];
    if !first.is_empty() {
        menu.push(Separator);
        menu.extend(first.into_iter().map(item));
    }
    menu.push(Separator);
    if !mono.is_empty() || !other.is_empty() {
        let mut others: Vec<MenuEntry> = mono.into_iter().map(item).collect();
        if !others.is_empty() && !other.is_empty() {
            others.push(Separator);
        }
        others.extend(other.into_iter().map(item));
        menu.push(Submenu(t!("settings-font-others").to_owned(), others));
    }
    menu.push(Submenu(
        t!("settings-font-fallbacks").to_owned(),
        fallback_font_menu(current),
    ));
    menu
}

/// 返回「后备字体」子菜单的菜单项：第一个字体缺字时依次使用的字体，点击一项即加入或移出字体
/// 列表。
///
/// 已在列表中的字体按列表的顺序排在前面并打勾，其后是其余已安装的字族；第一个字体本身不列出。
/// 没有设置字体时各项不可用：此时用的是平台默认字体，后备字体由系统决定，要先选择字体。
pub fn fallback_font_menu(current: Option<&str>) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    let list = winc_config::font_list(current);
    let (first, listed) = list
        .split_first()
        .map_or((None, &[][..]), |(f, l)| (Some(*f), l));
    let fonts = winc_render::installed_fonts();
    let index = |name: &str| fonts.iter().position(|family| family.name == name);
    let chosen: Vec<usize> = listed.iter().filter_map(|&name| index(name)).collect();
    let item = |i| Item(MenuCommand::FallbackFont(i), first.is_some());
    let mut menu: Vec<MenuEntry> = chosen.iter().map(|&i| item(i)).collect();
    let primary = first.and_then(index);
    let rest = (0..fonts.len()).filter(|&i| !chosen.contains(&i) && Some(i) != primary);
    if !menu.is_empty() {
        menu.push(Separator);
    }
    menu.extend(rest.map(item));
    menu
}

/// 返回「排除的语言」菜单的菜单项：格式化页列出的语言，被排除的打勾。
///
/// 点击一项即切换该语言是否被排除，菜单随即关闭。
pub fn skip_menu(listed: &[&str]) -> Vec<MenuEntry> {
    winc_syntax::languages()
        .iter()
        .enumerate()
        .filter(|(_, l)| listed.contains(&l.id))
        .map(|(i, _)| MenuEntry::Item(MenuCommand::SkipOnSave(i), true))
        .collect()
}

/// 返回一种语言的格式化（修复）工具菜单的菜单项：自动、该语言的预设、自定义命令。
///
/// 没有可自动使用的预设（没有预设，或预设都不自动使用）时不列出「自动」，选了也不会生效。
/// `install` 为 `true`（工具没装且知道安装方法）时有「安装方法…」；`removable` 为 `true`
/// （用户自己加的语言）时最后一项是「移除」。
pub fn tool_menu(task: Task, language: &str, install: bool, removable: bool) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    // 各组之间用分隔线隔开。
    fn group(menu: &mut Vec<MenuEntry>, items: Vec<MenuEntry>) {
        if items.is_empty() {
            return;
        }
        if !menu.is_empty() {
            menu.push(Separator);
        }
        menu.extend(items);
    }

    let mut menu = Vec::new();
    let presets = || {
        PRESETS
            .iter()
            .enumerate()
            .filter(|(_, p)| p.task == task && p.languages.contains(&language))
    };
    if presets().any(|(_, p)| !matches!(p.auto, Auto::Never)) {
        group(&mut menu, vec![Item(MenuCommand::ToolAuto, true)]);
    }
    let items = presets()
        .map(|(i, _)| Item(MenuCommand::ToolPreset(i), true))
        .collect();
    group(&mut menu, items);
    group(&mut menu, vec![Item(MenuCommand::ToolCustom, true)]);
    if install {
        group(&mut menu, vec![Item(MenuCommand::ToolInstall, true)]);
    }
    if removable {
        group(&mut menu, vec![Item(MenuCommand::ToolRemove, true)]);
    }
    menu
}

/// 返回缩进宽度子菜单的标题：空格缩进时为「缩进大小」（宽度是空格数），制表符缩进时为「制表符长度」
/// （宽度是一个制表符占的列数）。
pub fn indent_size_title(indent: Indent) -> &'static str {
    match indent {
        Indent::Spaces(_) => t!("menu-indent-size"),
        Indent::Tabs => t!("menu-tab-size"),
    }
}

/// 移到新分栏的菜单项：面板在右侧（`right`）时各栏上下排，移到下方，否则移到右侧。
pub fn move_group_command(right: bool) -> MenuCommand {
    match right {
        true => MenuCommand::MoveTerminalDown,
        false => MenuCommand::MoveTerminalRight,
    }
}
