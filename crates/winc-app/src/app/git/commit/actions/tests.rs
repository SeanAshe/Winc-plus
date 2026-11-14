//! 提交与回滚的测试：在指定仓库提交完成之后收下结果。

use super::*;

impl App {
    /// 测试用：在 `root` 里提交好了，收下结果。
    pub(in crate::app) fn committed_in(&mut self, root: &Path) {
        let outcome = Outcome::Committed(Ok("abc1234".into()), "msg".into(), root.into());
        let _ = self.git.commit.results.0.send(outcome);
        self.poll_commit();
    }
}

/// 依次提交几个仓库时中途失败：说明里写明失败的仓库，前面已经提交了的一并写出。
#[test]
fn a_partly_finished_commit_names_what_was_committed() {
    let mut done = CommittedIn::default();
    assert_eq!(done.failed(Path::new("/w/a"), "hook"), "a: hook");
    done.add(Path::new("/w/a"), "1111111".into());
    done.add(Path::new("/w/b"), "2222222".into());
    let text = done.failed(Path::new("/w/c"), "hook");
    assert!(
        text.contains("a") && text.contains("b") && text.contains("c"),
        "{text}"
    );
    assert!(text.ends_with("hook"), "{text}");
    assert_eq!(done.first.as_deref(), Some("1111111"));
}
