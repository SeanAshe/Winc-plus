//! 删除本地分支，分支未完全合并时先报告、由调用方确认后强制删除。

use super::*;

/// [`Git::delete_branch`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchDeleted {
    /// 已删除，内容是 `git` 的结论。
    Done(String),
    /// 分支未完全合并，需要强制删除，尚未删除。
    NeedsForce,
}

impl Git {
    /// 删除本地分支 `name`。
    ///
    /// `force` 时用 `--force` 连同未合并的提交一起删除；否则分支未完全合并时返回
    /// [`BranchDeleted::NeedsForce`]，不改动仓库。
    ///
    /// # Errors
    ///
    /// 分支不存在、被某个工作树签出等其他失败时返回 `git` 给出的原因。
    pub fn delete_branch(
        &self,
        root: &Path,
        name: &str,
        force: bool,
    ) -> Result<BranchDeleted, String> {
        let mut args = vec!["branch", "--delete"];
        if force {
            args.push("--force");
        }
        args.extend(["--end-of-options", name]);
        if let Err(refused) = self.require_trust(root) {
            output::record(root, &args, None, refused.as_bytes());
            return Err(refused);
        }
        // 要按报错认出「尚未合并」，报错不能被翻译成用户的语言。
        let english = |command: &mut Command| {
            command.env("LC_ALL", "C");
        };
        match self.run_prepared(root, &args, Vec::new(), OPERATION_TIMEOUT, english) {
            Ok(summary) => Ok(BranchDeleted::Done(summary)),
            // 分支名不含空格，不会和这句话混淆。
            Err(e) if !force && e.contains("is not fully merged") => Ok(BranchDeleted::NeedsForce),
            Err(e) => Err(e),
        }
    }
}
