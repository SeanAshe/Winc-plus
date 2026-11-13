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
