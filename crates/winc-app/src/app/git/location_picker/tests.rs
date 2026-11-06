//! `location_picker` 的测试：一个位置的分支、与上游的差异和未提交的更改数。

use super::*;

#[test]
fn place_state_reads_branch_and_counts() {
    let status = Status {
        head: Some(Head::Branch("main".into())),
        ahead: 2,
        behind: 1,
        ..Status::default()
    };
    let state = PlaceState::of(&status);
    assert_eq!(state.head.as_deref(), Some("main"));
    assert_eq!((state.ahead, state.behind, state.changes), (2, 1, 0));
}
