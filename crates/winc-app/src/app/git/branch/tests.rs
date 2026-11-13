//! 分支操作的测试：提交视图「⋯」里的拉取、被占用的分支不能签出到新工作树。

use super::*;

impl App {
    /// 测试用：在 `root` 里做完了「拉取」，收下结果。
    #[cfg(unix)] // 只有 Unix 上的 Git 测试用到
    pub(in crate::app) fn pulled_in(&mut self, root: &Path) {
        let what = "拉取".to_owned();
        // 换目录之前起的操作：编号比后来起的都小。
        let _ = self.git.branch.results.0.send(Done {
            id: 0,
            root: Some(root.into()),
            what,
            result: Ok((String::new(), Then::Nothing)),
        });
        self.poll_branch_ops();
    }
}

fn branch(name: &str, remote: bool, current: bool, upstream: Option<&str>) -> Branch {
    Branch {
        name: name.to_owned(),
        remote,
        current,
        upstream: upstream.map(str::to_owned),
        gone: false,
        oid: String::new(),
        time: 0,
        local: String::new(),
    }
}

/// 拉取、推送、获取在分支选择器里，「⋯」里不再重复；最上面是储藏。
#[test]
fn the_actions_menu_leaves_syncing_to_the_branch_picker() {
    let menu = branch_menu(&[], None, false);
    assert_eq!(
        menu.first(),
        Some(&MenuEntry::Item(MenuCommand::GitStash, true))
    );
    for command in [
        MenuCommand::GitFetch,
        MenuCommand::GitPull,
        MenuCommand::GitPullRebase,
        MenuCommand::GitPush,
    ] {
        assert!(
            !menu
                .iter()
                .any(|e| matches!(e, MenuEntry::Item(c, _) if *c == command)),
            "{command:?}"
        );
    }
}

#[test]
fn branches_held_by_another_worktree_cannot_go_into_a_new_one() {
    use MenuCommand::GitBranch;
    let branches = [
        branch("main", false, true, None),
        branch("dev", false, false, None),
        branch("feat", false, false, None),
        branch("origin/feat", true, false, None),
        branch("origin/main", true, false, None),
        branch("gone", false, false, None),
    ];
    let items: Vec<MenuEntry> = (0..branches.len())
        .flat_map(|i| branch_ops(i, &branches, &["feat"], &["gone"], false))
        .collect();
    // 当前分支、被占用的（远程分支看同名的本地分支，`origin/main` 在 `main` 上）没有「在新
    // 工作树中签出」。
    // 被目录已不在的工作树占着：签出、删除都灰掉（git 不让），要先清理。
    for op in [BranchOp::Checkout, BranchOp::Delete] {
        assert!(items.contains(&MenuEntry::Item(GitBranch(op, 5), false)));
    }
    let offered: Vec<usize> = items
        .iter()
        .filter_map(|e| match e {
            MenuEntry::Item(GitBranch(BranchOp::NewWorktree, i), true) => Some(*i),
            _ => None,
        })
        .collect();
    assert_eq!(offered, [1]);
}

#[test]
fn branch_ops_are_grouped_with_separators() {
    use BranchOp::*;
    use MenuCommand::GitBranch;

    let branches = [
        branch("main", false, true, None),
        branch("feat", false, false, None),
        branch("origin/feat", true, false, None),
    ];
    let item = |op, i| MenuEntry::Item(GitBranch(op, i), true);
    assert_eq!(
        branch_ops(1, &branches, &[], &[], false),
        [
            item(Checkout, 1),
            item(NewWorktree, 1),
            item(NewFrom, 1),
            MenuEntry::Separator,
            item(Compare, 1),
            MenuEntry::Separator,
            item(Merge, 1),
            item(Rebase, 1),
            MenuEntry::Separator,
            item(Rename, 1),
            item(Delete, 1),
        ]
    );
    // 空的组不留分隔线：当前分支只能重命名，远程分支最后没有删除。
    assert_eq!(branch_ops(0, &branches, &[], &[], false), [item(Rename, 0)]);
    let remote = branch_ops(2, &branches, &[], &[], false);
    assert_eq!(remote.last(), Some(&item(Rebase, 2)));
}
