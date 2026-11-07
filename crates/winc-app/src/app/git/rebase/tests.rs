//! `rebase` 的测试：能否开始的判断和步骤的生成。

use super::*;

fn plan(names: &[&str]) -> RebasePlan {
    RebasePlan {
        base: Some("base".into()),
        commits: names
            .iter()
            .map(|name| RebaseCommit {
                oid: (*name).to_owned(),
                subject: (*name).to_owned(),
                message: (*name).to_owned(),
            })
            .collect(),
        pushed: 0,
    }
}

#[test]
fn an_unchanged_list_has_nothing_to_do() {
    let mut form = RebaseForm::new(plan(&["a", "b"]));
    assert!(!form.changed());
    form.rows.swap(0, 1);
    assert!(form.changed());
    form.rows.swap(0, 1);
    assert!(!form.changed());
    form.rows[1].action = RebaseAction::Drop;
    assert!(form.changed());
}

#[test]
fn the_first_kept_commit_cannot_be_squashed() {
    let mut form = RebaseForm::new(plan(&["a", "b"]));
    form.rows[0].action = RebaseAction::Squash;
    assert!(form.problem("").is_some());
    form.rows[0].action = RebaseAction::Drop;
    form.rows[1].action = RebaseAction::Fixup;
    assert!(form.problem("").is_some());
    form.rows[1].action = RebaseAction::Pick;
    assert!(form.problem("").is_none());
    form.rows[1].action = RebaseAction::Squash;
    form.rows[0].action = RebaseAction::Pick;
    assert!(form.problem("").is_none());
}

#[test]
fn dropping_everything_is_refused() {
    let mut form = RebaseForm::new(plan(&["a"]));
    form.rows[0].action = RebaseAction::Drop;
    assert!(form.problem("").is_some());
}

#[test]
fn steps_follow_the_list_order() {
    let mut form = RebaseForm::new(plan(&["a", "b"]));
    form.rows.swap(0, 1);
    form.rows[0].action = RebaseAction::Reword;
    form.rows[0].message = Some("new".into());
    let steps = form.steps();
    assert_eq!(steps[0].oid, "b");
    assert_eq!(steps[0].message.as_deref(), Some("new"));
    assert_eq!(steps[1].oid, "a");
    assert_eq!(steps[1].action, RebaseAction::Pick);
}

/// 回归：信息为空时 `git commit` 会中止，变基会停在这个提交上。
#[test]
fn a_reworded_commit_needs_a_message() {
    let mut form = RebaseForm::new(plan(&["a", "b"]));
    form.rows[0].action = RebaseAction::Reword;
    form.rows[0].message = Some("kept".into());
    assert!(form.problem("kept").is_none());
    // 选中行以信息框的内容为准，其余行以存下的信息为准。
    assert!(form.problem("").is_some());
    assert!(form.problem(" \n ").is_some());
    form.selected = 1;
    assert!(form.problem("").is_none());
    form.rows[0].message = Some(String::new());
    assert!(form.problem("anything").is_some());
}
