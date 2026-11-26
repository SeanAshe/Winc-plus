//! 右键菜单的测试：菜单项编号、快捷键的显示，以及菜单项随点击目标变化。

use super::*;

#[test]
fn menu_items_are_numbered_in_display_order() {
    use crate::app::git::branch::BranchOp;
    let entries = vec![
        MenuEntry::Item(MenuCommand::GitFetch, true),
        MenuEntry::Separator,
        MenuEntry::Submenu(
            "分支".to_owned(),
            vec![
                MenuEntry::Item(MenuCommand::GitBranch(BranchOp::Rebase, 42), true),
                MenuEntry::Item(MenuCommand::GitBranch(BranchOp::Delete, 20_000), false),
            ],
        ),
        MenuEntry::Item(MenuCommand::LogAuthor(30_000), true),
    ];
    let at = |tag| MenuEntry::command_at(&entries, tag);
    assert_eq!(at(1), Some(MenuCommand::GitFetch));
    assert_eq!(at(2), Some(MenuCommand::GitBranch(BranchOp::Rebase, 42)));
    // 序号再大也不会串到别的命令上。
    assert_eq!(
        at(3),
        Some(MenuCommand::GitBranch(BranchOp::Delete, 20_000))
    );
    assert_eq!(at(4), Some(MenuCommand::LogAuthor(30_000)));
    assert_eq!(at(0), None);
    assert_eq!(at(5), None);
    assert_eq!(at(-1), None);
}

#[test]
fn menus_show_the_key_for_where_they_open() {
    crate::keymap::install(crate::keymap::Preset::VsCode, &Default::default());
    let text = |c: MenuCommand, context| {
        c.shortcut(context)
            .map(|s| s.label(crate::keymap::Os::CURRENT))
    };
    if cfg!(target_os = "macos") {
        assert_eq!(
            text(MenuCommand::CopyPath, Context::Tree).as_deref(),
            Some("⌥⌘C")
        );
        assert_eq!(
            text(MenuCommand::CopyRelativePath, Context::Other).as_deref(),
            Some("⌥⇧⌘C")
        );
        assert_eq!(
            text(MenuCommand::Delete, Context::Tree).as_deref(),
            Some("⌘⌫")
        );
        assert_eq!(
            text(MenuCommand::Rename, Context::Tree).as_deref(),
            Some("↩")
        );
        assert_eq!(
            text(MenuCommand::Close, Context::Other).as_deref(),
            Some("⌘W")
        );
        assert_eq!(
            text(MenuCommand::Copy, Context::Editor).as_deref(),
            Some("⌘C")
        );
        assert_eq!(
            text(MenuCommand::Format, Context::Editor).as_deref(),
            Some("⌥⇧F")
        );
        assert_eq!(
            text(MenuCommand::FindReferences, Context::Editor).as_deref(),
            Some("⇧F12")
        );
    }
    // 两步组合键无法显示；右键菜单中的「新建文件…」不是 `⌘N`，也不显示。
    assert_eq!(text(MenuCommand::FormatSelection, Context::Editor), None);
    assert_eq!(text(MenuCommand::NewFile, Context::Tree), None);
}

#[test]
fn menus_depend_on_what_was_clicked() {
    use MenuCommand::*;
    let commands = |menu: Vec<MenuEntry>| -> Vec<(MenuCommand, bool)> {
        menu.into_iter()
            .filter_map(|e| match e {
                MenuEntry::Item(c, enabled) | MenuEntry::Alternate(c, enabled) => {
                    Some((c, enabled))
                }
                MenuEntry::Separator | MenuEntry::Submenu(..) => None,
            })
            .collect()
    };

    // 根目录不能复制、改名、删除，没有相对路径，但可以粘贴；文件没有「在目录中查找」。
    let root: Vec<_> = commands(tree_menu(None, true, false))
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(
        root,
        [
            NewFile,
            NewFolder,
            Reveal,
            OpenInTerminal,
            FindInFolder,
            Paste,
            CopyPath
        ]
    );
    let file: Vec<_> = commands(tree_menu(Some(EntryKind::File), false, false))
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert!(!file.contains(&FindInFolder) && file.contains(&Delete) && file.contains(&Copy));
    assert!(file.contains(&Cut));
    assert!(commands(tree_menu(Some(EntryKind::Dir), true, false)).contains(&(FindInFolder, true)));
    // 多选：正好两个文件时才能比较，没有文件时「打开」灰显。
    assert!(commands(tree_selection_menu(2, false)).contains(&(CompareFiles, true)));
    assert!(
        !commands(tree_selection_menu(3, false))
            .iter()
            .any(|(c, _)| *c == CompareFiles)
    );
    assert!(commands(tree_selection_menu(0, false)).contains(&(OpenFiles, false)));
    // 剪贴板中没有文件或图片时「粘贴」灰显。
    assert!(commands(tree_menu(Some(EntryKind::File), false, false)).contains(&(Paste, false)));
    // 「下载…」只在远端的文件、目录和多选上出现，本机的项和根目录没有。
    let download = |menu: Vec<MenuEntry>| commands(menu).contains(&(Download, true));
    assert!(download(tree_menu(Some(EntryKind::File), false, true)));
    assert!(download(tree_menu(Some(EntryKind::Dir), false, true)));
    assert!(download(tree_selection_menu(2, true)));
    assert!(!download(tree_menu(Some(EntryKind::File), false, false)));
    assert!(!download(tree_menu(None, false, true)));
    assert!(!download(tree_selection_menu(2, false)));

    // 只有一个未命名的标签：关闭其他、关闭右侧、路径相关的命令都灰显。
    let only = commands(tab_menu((0, 0), 1, 1, false, false, false));
    for command in [
        CloseOthers,
        CloseRight,
        CloseSaved,
        CopyPath,
        Reveal,
        RevealInTree,
    ] {
        assert!(only.contains(&(command, false)), "{command:?}");
    }
    assert!(only.contains(&(Close, true)) && only.contains(&(CloseAll, true)));

    // 最后一个标签没有右侧，第一个没有左侧。
    let last = commands(tab_menu((2, 2), 1, 3, true, true, true));
    assert!(last.contains(&(CloseRight, false)) && last.contains(&(CloseOthers, true)));
    assert!(last.contains(&(CloseLeft, true)));
    assert!(commands(tab_menu((0, 0), 1, 3, true, true, true)).contains(&(CloseLeft, false)));
    // 选中了全部标签时没有「其他」；选中的不相邻时有。
    assert!(commands(tab_menu((0, 2), 3, 3, true, true, true)).contains(&(CloseOthers, false)));
    assert!(commands(tab_menu((0, 2), 2, 3, true, true, true)).contains(&(CloseOthers, true)));

    // 可编辑的文件菜单项全部可用；只读的文件只能复制、复制代码引用，以及转到定义、查找引用。
    assert_eq!(
        commands(editor_menu(true, false, false, false, true)),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, true),
            (Fix, true),
            (Cut, true),
            (Copy, true),
            (Paste, true),
            (CopyReference, true)
        ]
    );
    assert_eq!(
        commands(editor_menu(false, false, false, false, false)),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, false),
            (Fix, false),
            (Cut, false),
            (Copy, true),
            (Paste, false),
            (CopyReference, true)
        ]
    );
    // 有选区时多一项「格式化选中部分」。
    assert_eq!(
        commands(editor_menu(true, true, true, false, true))[6],
        (FormatSelection, true)
    );
    // 「比较」子菜单：有选区时比较选中部分；没有存到磁盘上的文件不能与已保存的版本比较。
    let compare = |menu: Vec<MenuEntry>| {
        let found = menu.into_iter().find_map(|e| match e {
            MenuEntry::Submenu(_, entries) => Some(entries),
            _ => None,
        });
        commands(found.unwrap())
    };
    assert_eq!(
        compare(editor_menu(true, false, false, false, true)),
        [(CompareWithClipboard, true), (CompareWithSaved, true)]
    );
    assert_eq!(
        compare(editor_menu(false, true, false, false, false)),
        [
            (CompareSelectionWithClipboard, true),
            (CompareWithSaved, false)
        ]
    );
    // 光标所在行已有书签时只能删除，不能重复添加。
    let marked = commands(editor_menu(true, false, true, false, true));
    assert!(marked.contains(&(AddBookmark, false)));
    assert!(marked.contains(&(RemoveBookmark, true)));
}

/// 远端主机上的文件：各菜单不列出「在文件管理器中显示」，终端模块关闭时也不列出「在终端中
/// 打开」，其余的项和顺序不变，不留下多余的分隔线。
#[test]
fn local_only_items_are_left_out_for_remote_files() {
    use MenuCommand::*;

    let items = |menu: &[MenuEntry]| -> Vec<MenuCommand> {
        (menu.iter())
            .filter_map(|entry| match entry {
                MenuEntry::Item(command, _) => Some(*command),
                _ => None,
            })
            .collect()
    };
    let tidy = |menu: &[MenuEntry]| {
        let separator = |entry: &MenuEntry| matches!(entry, MenuEntry::Separator);
        !menu.first().is_some_and(separator)
            && !menu.last().is_some_and(separator)
            && !menu
                .windows(2)
                .any(|pair| separator(&pair[0]) && separator(&pair[1]))
    };
    let menus = [
        tree_menu(Some(EntryKind::File), true, true),
        tree_selection_menu(2, true),
        tree_menu(None, true, false),
        tab_menu((0, 0), 1, 3, true, true, true),
        commit_file_menu(true, true, &[]),
        log_file_menu(true, true),
    ];
    for menu in menus {
        let before = items(&menu);
        for terminal in [false, true] {
            let kept = without_local_only(menu.clone(), terminal, false);
            let after = items(&kept);
            assert!(!after.contains(&Reveal), "{after:?}");
            assert_eq!(
                after.contains(&OpenInTerminal),
                terminal && before.contains(&OpenInTerminal)
            );
            let expected: Vec<_> = (before.iter())
                .filter(|c| **c != Reveal && (terminal || **c != OpenInTerminal))
                .cloned()
                .collect();
            assert_eq!(after, expected);
            assert!(tidy(&kept), "{kept:?}");
        }
        // 挂载在本机的目录（`remote.localFolders`）之内：「在文件管理器中显示」保留。
        let mounted = items(&without_local_only(menu.clone(), true, true));
        assert_eq!(mounted.contains(&Reveal), before.contains(&Reveal));
        // 应用内的定位不受影响。
        if before.contains(&RevealInTree) {
            assert!(items(&without_local_only(menu, false, false)).contains(&RevealInTree));
        }
    }
}

#[test]
fn a_tag_on_a_commit_can_still_copy_its_name_while_git_is_busy() {
    use crate::app::git::branch::TagOp;
    let actions = CommitActions {
        enabled: false,
        undo: false,
        push: false,
        on_head: None,
        child: false,
        parent: false,
        changes: false,
        in_log: true,
    };
    let menu = log_commit_menu(actions, "main", Vec::new(), &["v1"]);
    let tag = menu.iter().find_map(|entry| match entry {
        MenuEntry::Submenu(_, items)
            if matches!(
                items.first(),
                Some(MenuEntry::Item(MenuCommand::LogTag(..), _))
            ) =>
        {
            Some(items)
        }
        _ => None,
    });
    let enabled: Vec<_> = (tag.unwrap().iter())
        .filter(|entry| matches!(entry, MenuEntry::Item(_, true)))
        .collect();
    let copy = MenuEntry::Item(MenuCommand::LogTag(TagOp::CopyName, 0), true);
    assert_eq!(enabled, [&copy]);
}
