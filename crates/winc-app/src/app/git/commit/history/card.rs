//! 提交历史中悬停一个提交时的详情卡片：完整的说明和改动的统计在后台读取，按提交号缓存。

use super::*;

/// 一个提交的详情卡片的读取状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardState {
    /// 正在后台读取。
    Loading,
    /// 读取完成。
    Ready(CommitCard),
    /// 读取失败；不再重新读取，直到换了仓库。
    Failed,
}

/// 详情卡片在后台读取的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitCard {
    /// 完整的提交说明。
    pub(crate) message: String,
    /// 改动的文件数（个）。
    pub(crate) files: usize,
    /// 增加的行数；二进制文件不计。
    pub(crate) added: usize,
    /// 删除的行数；二进制文件不计。
    pub(crate) removed: usize,
}

impl App {
    /// 每帧绘制之前调用：鼠标停在一个提交上、该提交的详情尚未读取时在后台读取。
    pub(super) fn track_history_card(&mut self) {
        let Hover::Commit(CommitTarget::History(HistoryTarget::Commit(i))) = self.pointer.hover
        else {
            return;
        };
        let Some((commit, _)) = self.git.commit.history.commit(i) else {
            return;
        };
        if self.git.commit.history.cards.contains_key(&commit.oid) {
            return;
        }
        let (oid, parent) = (commit.oid.clone(), commit.parents.first().cloned());
        (self.git.commit.history.cards).insert(oid.clone(), CardState::Loading);
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, root| {
            let cancel = winc_editor::format::Cancel::default();
            let card = git.and_then(|git| {
                let message = git.commit_message(root, &oid, &cancel)?;
                let files = git.commit_files(root, parent.as_deref(), &oid, &cancel)?;
                let lines = files.iter().filter_map(|f| f.lines);
                let (added, removed) = lines.fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
                Ok(CommitCard {
                    message,
                    files: files.len(),
                    added,
                    removed,
                })
            });
            let _ = sender.send(Outcome::History(HistoryOutcome::Card(oid, card)));
        });
    }

    /// 收到提交 `oid` 的详情：记下；失败时记为失败，卡片中写明，不反复读取。
    pub(in crate::app) fn history_card_arrived(
        &mut self,
        oid: String,
        card: Result<CommitCard, String>,
    ) {
        let state = card.map_or(CardState::Failed, CardState::Ready);
        self.git.commit.history.cards.insert(oid, state);
    }

    /// 第 `i` 个提交的详情卡片的读取状态；尚未开始读取时为 `None`。
    pub(crate) fn history_card(&self, i: usize) -> Option<&CardState> {
        let (commit, _) = self.git.commit.history.commit(i)?;
        self.git.commit.history.cards.get(&commit.oid)
    }
}
