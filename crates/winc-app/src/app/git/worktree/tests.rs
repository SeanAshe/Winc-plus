//! 工作树列表和菜单的测试：主工作树、被占用的分支、菜单项的可用状态。

use std::path::PathBuf;

use winc_editor::git::Head;

use super::*;

fn worktree(path: &str, head: Option<Head>) -> Worktree {
    Worktree {
        path: PathBuf::from(path),
        head,
        locked: false,
        prunable: false,
    }
}

fn branch(name: &str) -> Option<Head> {
    Some(Head::Branch(name.to_owned()))
}

#[test]
fn the_current_worktree_main_and_held_branches() {
    // 打开的是主工作树：不算附属的，别的工作树签出的分支算被占用。
    let mut git = GitState {
        root: Some(PathBuf::from("/r/winc")),
        worktrees: vec![
            worktree("/r/winc", branch("main")),
            worktree("/r/winc.worktrees/feat", branch("feat")),
            worktree("/r/fix", Some(Head::Detached("abc1234".into()))),
        ],
        ..GitState::default()
    };
    assert_eq!(git.current_worktree(), Some(0));
    assert!(git.main_worktree().is_none());
    assert_eq!(git.held_branches(), ["feat"]);
    assert_eq!(git.worktree_holding("feat"), Some(1));
    assert_eq!(git.worktree_holding("main"), None);

    // 打开的是附属工作树：主工作树是第一项，自己签出的分支不算被占用。
    git.root = Some(PathBuf::from("/r/winc.worktrees/feat"));
    assert_eq!(git.current_worktree(), Some(1));
    assert_eq!(
        git.main_worktree().map(worktree_name).as_deref(),
        Some("winc")
    );
    assert_eq!(git.held_branches(), ["main"]);
    assert_eq!(git.worktree_holding("feat"), None);

    // 只有一个工作树：列表为空，什么都不算。
    git.worktrees.clear();
    assert_eq!(git.current_worktree(), None);
    assert!(git.held_branches().is_empty());
}

#[test]
fn a_worktree_is_named_after_its_folder() {
    assert_eq!(worktree_name(&worktree("/r/winc.git", None)), "winc");
}
