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

/// 返回状态栏编码菜单的菜单项：两个子菜单「通过编码重新打开」「通过编码保存」，内容是各种编码。
///
/// `reopen` 表示能否重新打开（大文件、二进制文件、未命名的文档不能）；`save` 返回能否改用某种
/// 编码保存（只读的文档都不能，大文件只能用 UTF-8）。
pub fn encoding_menu(reopen: bool, save: impl Fn(Encoding) -> bool) -> Vec<MenuEntry> {
    use MenuCommand::{ReopenEncoding, SaveEncoding};
    use MenuEntry::{Item, Submenu};

    let reopen = (0..CHOICES.len()).map(|i| Item(ReopenEncoding(i), reopen));
    let save = CHOICES
        .iter()
        .enumerate()
        .map(|(i, &(encoding, _))| Item(SaveEncoding(i), save(encoding)));
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

/// 返回状态栏语言菜单的菜单项：有文件名（`named`）时第一项是按文件名识别，然后是纯文本，
/// 其余语言按名字排序。
pub fn document_language_menu(named: bool) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    let mut languages: Vec<_> = winc_syntax::languages().iter().enumerate().collect();
    languages.sort_by_key(|(_, l)| l.name.to_lowercase());
    let mut menu = Vec::new();
    if named {
        menu.extend([Item(MenuCommand::DetectLanguage, true), Separator]);
    }
    menu.push(Item(MenuCommand::UsePlainText, true));
    menu.extend((languages.into_iter()).map(|(i, _)| Item(MenuCommand::UseLanguage(i), true)));
    menu
}

/// 返回文件树菜单的菜单项。
///
/// `kind` 是右键点中的那一项的类型，`None` 表示空白处（根目录）：根目录不能改名、删除，
/// 也没有相对路径。「在目录中查找」只对目录和根目录出现。
pub fn tree_menu(kind: Option<EntryKind>, can_paste: bool) -> Vec<MenuEntry> {
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
    menu.push(Separator);
    menu.push(Item(CopyPath, true));
    if kind.is_some() {
        menu.extend([
            Item(CopyRelativePath, true),
            Separator,
            Item(Rename, true),
            Item(Delete, true),
        ]);
    }
    menu
}

/// 返回文件树里选中了多项、右键点在其中一项上时的菜单项：打开（`files`：选中的文件数，没有
/// 文件时灰显）、比较所选文件（正好两个文件时才有）、剪切、复制、复制路径、复制相对路径和删除，
/// 都作用于选中的各项。
///
/// 新建、改名等在多项选择下没有意义的项不出现。
pub fn tree_selection_menu(files: usize) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(OpenFiles, files > 0)];
    if files == 2 {
        menu.push(Item(CompareFiles, true));
    }
    menu.extend([
        Separator,
        Item(Cut, true),
        Item(Copy, true),
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, true),
        Separator,
        Item(Delete, true),
    ]);
    menu
}

/// 返回标签栏菜单的菜单项：右键点的是第 `first` 到第 `last` 个标签（从 0 起，只点了一个标签时
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
        Item(CopyPath, has_path),
        Item(CopyRelativePath, in_workspace),
        Separator,
        Item(Reveal, has_path),
        Item(RevealInTree, in_workspace),
    ]
}

/// 日志里一个提交的右键菜单：签出、在新工作树中签出、新建分支、新建标签，拣选、还原，把当前
/// 分支重置到此提交（软、混合、硬在子菜单里，各自做什么写在副标题里），以及复制提交号与说明。
///
/// `enabled` 为 `false`（另一个 Git 操作还在运行，或者正停在合并、变基中间）时，会修改
/// 仓库的几项灰显。`branch` 是当前分支名（分离的 `HEAD` 时为「HEAD」），写进重置子菜单
/// 的标题。
pub fn log_commit_menu(enabled: bool, branch: &str) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator, Submenu};

    vec![
        Item(LogCheckout, enabled),
        Item(LogNewWorktree, enabled),
        Item(LogNewBranch, enabled),
        Item(LogNewTag, enabled),
        Separator,
        Item(LogCherryPick, enabled),
        Item(LogRevert, enabled),
        Submenu(
            t!("menu-log-reset-branch", branch = branch),
            vec![
                Item(LogResetSoft, enabled),
                Item(LogResetMixed, enabled),
                Separator,
                Item(LogResetHard, enabled),
            ],
        ),
        Separator,
        Item(LogCopyOid, true),
        Item(LogCopyShortOid, true),
        Item(LogCopyMessage, true),
    ]
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

/// 日志「分支」下拉的菜单项：全部分支，然后是本地分支、远程分支两组（组标题不能点）。
pub fn log_branch_menu(branches: &[winc_editor::git::Branch]) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(LogBranch(0), true)];
    for remote in [false, true] {
        let group: Vec<usize> = (0..branches.len())
            .filter(|&i| branches[i].remote == remote)
            .collect();
        if group.is_empty() {
            continue;
        }
        menu.push(Separator);
        menu.push(Item(GitHeading(remote), false));
        menu.extend(group.into_iter().map(|i| Item(LogBranch(i + 1), true)));
    }
    menu
}

/// 状态栏分支菜单的菜单项：本地分支，同一前缀的两个以上收进子菜单；本地没有的远程分支在子菜单
/// 里；然后是同步、获取、拉取、推送，新建。`stale` 是签出它的工作树目录已不在、不能签出的分支，
/// `[sync, fetch, pull, push]` 是这四项能不能用。
pub fn status_branch_menu(
    branches: &[winc_editor::git::Branch],
    stale: &[&str],
    [sync, fetch, pull, push]: [bool; 4],
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator, Submenu};

    let named = |remote: bool| -> Vec<(usize, &str)> {
        (branches.iter().enumerate())
            .filter(|(_, b)| b.remote == remote)
            .filter(|(_, b)| {
                !remote || !crate::app::git::branch::picker::has_local_branch(branches, b)
            })
            .map(|(i, b)| (i, b.name.as_str()))
            .collect()
    };
    let mut menu = branch_tree(&named(false), branches, stale);
    let remote = named(true);
    if !remote.is_empty() {
        menu.push(Submenu(
            t!("menu-git-remote-branches").to_owned(),
            branch_tree(&remote, branches, stale),
        ));
    }
    menu.extend([
        Separator,
        Item(StatusSync, sync),
        Item(GitFetch, fetch),
        Item(GitPull, pull),
        Item(GitPush, push),
        Separator,
        Item(StatusNewBranch, true),
        Item(NewWorktree, true),
    ]);
    menu
}

/// 把 `(下标, 名字)` 排成菜单：本地分支当前的在最前，其余最近提交的在前（和分支选择器一样），远程
/// 分支按名字；第一段相同的两个以上收进以它为标题的子菜单，放在其中最靠前的一个的位置，层层如此。
fn branch_tree(
    names: &[(usize, &str)],
    branches: &[winc_editor::git::Branch],
    stale: &[&str],
) -> Vec<MenuEntry> {
    fn build(names: &[(usize, &str, &str)], stale: &[&str]) -> Vec<MenuEntry> {
        let head = |rest: &'_ str| rest.split_once('/').map(|(head, _)| head.to_owned());
        let mut seen: Vec<String> = Vec::new();
        let mut menu = Vec::new();
        for &(index, full, rest) in names {
            let folder = head(rest);
            let group: Vec<(usize, &str, &str)> = folder.as_ref().map_or_else(Vec::new, |folder| {
                (names.iter())
                    .filter(|n| head(n.2).as_ref() == Some(folder))
                    .map(|&(i, f, r)| (i, f, r.split_once('/').map_or(r, |(_, tail)| tail)))
                    .collect()
            });
            if group.len() < 2 {
                let usable = !stale.contains(&full);
                menu.push(MenuEntry::Item(MenuCommand::StatusBranch(index), usable));
                continue;
            }
            let folder = folder.unwrap_or_default();
            if !seen.contains(&folder) {
                seen.push(folder.clone());
                menu.push(MenuEntry::Submenu(folder, build(&group, stale)));
            }
        }
        menu
    }
    let time = |index: usize| {
        (branches.get(index)).map_or(0, crate::app::git::branch::picker::branch_rank)
    };
    let mut sorted: Vec<(usize, &str, &str)> = names
        .iter()
        .map(|&(index, name)| (index, name, name))
        .collect();
    sorted.sort_by(|a, b| time(b.0).cmp(&time(a.0)).then(a.1.cmp(b.1)));
    build(&sorted, stale)
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

/// 返回提交视图里一个文件的菜单项：显示差异、打开文件，回滚，在文件树中显示、
/// 在文件管理器中显示，复制路径。
///
/// `exists` 表示文件还在磁盘上，删掉的文件不能打开、不能在文件管理器中显示；`in_workspace`
/// 表示它在打开的目录里。
pub fn commit_file_menu(exists: bool, in_workspace: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    vec![
        Item(GitShowDiff, true),
        Item(GitOpenFile, exists),
        Separator,
        Item(GitRollbackFile, true),
        Separator,
        Item(RevealInTree, exists && in_workspace),
        Item(Reveal, exists),
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, in_workspace),
    ]
}

/// 返回提交视图里选中了多个文件、右键点在其中一个上时的菜单项：打开（`any_exists`：有还在
/// 磁盘上的）、勾选、取消勾选、回滚、复制路径、复制相对路径，都作用于选中的各个文件。
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
        Separator,
        Item(CopyPath, true),
        Item(CopyRelativePath, true),
    ]
}

/// 返回全局搜索结果里一行的菜单项：打开；文件行是复制路径、复制相对路径、在文件树中显示，
/// 匹配行是复制这一行的文字；最后是替换（`replace`：替换输入展开着）和从结果中移除。
///
/// `file` 表示右键点着的是文件行。结果都来自打开的目录，因此相对路径、在文件树中显示总是可用。
pub fn search_result_menu(file: bool, replace: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    let mut menu = vec![Item(OpenFiles, true), Separator];
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

/// 返回编辑区菜单的菜单项：转到定义、查找引用，添加/删除书签、列出书签，格式化（有选区时多一项
/// 「格式化选中部分」）、修复问题，最后是剪切、复制、粘贴。
///
/// `bookmarked` 决定「添加书签」「删除书签」哪个可用。没有选区时剪切、复制作用于光标所在的整行，
/// 与 `⌘X`、`⌘C` 相同；只读的文件不能格式化、剪切、粘贴。格式化和修复总是可以点击，无法执行时
/// 点击后说明原因。「粘贴」不检查剪贴板里有没有文字：弹出菜单不应读取其他应用复制的内容。
pub fn editor_menu(editable: bool, selected: bool, bookmarked: bool) -> Vec<MenuEntry> {
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
    menu.extend([
        Separator,
        Item(Cut, editable),
        Item(Copy, true),
        Item(Paste, editable),
        Item(CopyReference, true),
    ]);
    menu
}

/// 返回终端里的菜单项：右键点在命令块上时（`block`：报了命令行，能不能重新运行）先是复制命令、
/// 复制输出、重新运行；然后复制（有选区时）、粘贴（程序还在运行时）、全选，查找、清屏，最大化
/// 面板，新建、关闭终端。
pub fn terminal_menu(selected: bool, running: bool, block: Option<(bool, bool)>) -> Vec<MenuEntry> {
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
        Item(CloseTerminal, true),
    ]);
    menu
}

/// 返回差异标签里文字的菜单项：右侧（`right`）是剪切、复制、粘贴，光标在改动上
/// （`on_hunk`）时还有「回滚这处改动」；左侧只读，只有复制。
pub fn diff_menu(right: bool, on_hunk: bool) -> Vec<MenuEntry> {
    use MenuCommand::*;
    use MenuEntry::{Item, Separator};

    if !right {
        return vec![Item(Copy, true)];
    }
    vec![
        Item(Cut, true),
        Item(Copy, true),
        Item(Paste, true),
        Separator,
        Item(GitRevertHunk, on_hunk),
    ]
}

/// 返回项目名菜单的菜单项：最近的目录（不含当前目录；按住 `⌥` 时是「在新窗口中打开」），之后是
/// 「打开目录…」「关闭目录」；有 WSL 发行版时另有「打开 WSL 目录」子菜单，按发行版列出。
///
/// `recent` 为最近列表的长度，`current` 为当前目录在最近列表中的位置，`opened` 表示是否已经
/// 打开目录（没有打开时「关闭目录」不可用）。
pub fn project_menu(
    recent: usize,
    current: Option<usize>,
    opened: bool,
    distros: usize,
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
    menu.push(Item(MenuCommand::OpenFolder, true));
    if distros > 0 {
        let items = (0..distros).map(|i| Item(MenuCommand::OpenWslFolder(i), true));
        menu.push(Submenu(
            t!("cmd-open-wsl-folder").to_owned(),
            items.collect(),
        ));
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

/// 返回「空白符」菜单的菜单项，当前的选择打勾。
pub fn whitespace_menu() -> Vec<MenuEntry> {
    (0..winc_config::RenderWhitespace::ALL.len())
        .map(|i| MenuEntry::Item(MenuCommand::Whitespace(i), true))
        .collect()
}

/// 返回字体菜单的菜单项：第一项是默认，然后是本机已安装的字族，当前的选择打勾。
///
/// 编辑器字体把等宽字体排在前面，其余字体紧随其后，都在同一级菜单里，不用再进子菜单。
/// 界面字体按名字排列。
pub fn font_menu(setting: Setting) -> Vec<MenuEntry> {
    use MenuEntry::{Item, Separator};

    let item = |(i, _)| Item(MenuCommand::Font(i), true);
    let fonts = winc_render::installed_fonts().iter().enumerate();
    let (mono, rest): (Vec<_>, Vec<_>) = if setting == Setting::EditorFont {
        fonts.partition(|(_, family)| family.monospace)
    } else {
        (fonts.collect(), Vec::new())
    };
    let mut menu = vec![Item(MenuCommand::FontDefault, true)];
    if !mono.is_empty() {
        menu.push(Separator);
        menu.extend(mono.into_iter().map(item));
    }
    if !rest.is_empty() {
        menu.push(Separator);
        menu.extend(rest.into_iter().map(item));
    }
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
