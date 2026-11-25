//! 提交前格式化：开着时（提交按钮的菜单），提交前先用格式化工具整理要提交的文件，再提交。
//!
//! 只整理整个文件都提交的：勾选式下有取消勾选的改动、暂存视图中只暂存了一部分的文件照原样
//! 提交，否则整理后的内容与选好的改动对不上。工具与手动格式化相同；还没有允许运行的项目工具
//! 不在提交途中询问，跳过对应的文件。格式化在磁盘上进行：勾选式下打开着的文件先保存；暂存
//! 视图提交的是暂存区，有未保存修改、或磁盘上与暂存区不同的文件不整理，暂存区只多出格式的变化。

use ropey::Rope;
use winc_core::LineEnding;
use winc_editor::format::{Cancel as FormatCancel, Job, Plan};
use winc_editor::git::Change;

use super::*;
use crate::app::NoticeKind;

impl App {
    /// 提交前格式化：开着、这次还没整理过时在后台整理要提交的文件，返回 `true`，提交等整理完
    /// 再继续（[`App::reformatted`]）。整理过一次后放行这一次提交。
    pub(super) fn reformat_first(&mut self) -> bool {
        if std::mem::take(&mut self.git.commit.reformat_passed) || !self.state.commit_reformat {
            return false;
        }
        let Some(status) = self.git.status.as_ref() else {
            return false;
        };
        let staging = self.git.commit.staging();
        let whole = |entry: &Entry| {
            let gone = [entry.staged, entry.unstaged].contains(&Some(Change::Deleted));
            !gone && !entry.is_dir && !entry.conflicted()
        };
        let paths: Vec<PathBuf> = if staging {
            let staged = status.entries.iter().filter(|e| e.staged.is_some());
            staged
                .filter(|e| e.unstaged.is_none() && whole(e))
                .map(|e| e.path.clone())
                .collect()
        } else {
            let commit = &self.git.commit;
            let picked = commit.picked_entries(status);
            picked
                .filter(|e| whole(e) && !commit.skips_hunks(e))
                .map(|e| e.path.clone())
                .collect()
        };
        // 其他仓库里一并提交的文件同样先格式化，记下所在的仓库：暂存区视图中格式化后要重新暂存。
        let root = self.git.root.clone().unwrap_or_default();
        let mut targets: Vec<(PathBuf, PathBuf)> =
            paths.into_iter().map(|p| (p, root.clone())).collect();
        targets.extend(self.other_reformat_paths());
        if staging {
            // 不保存也不整理有未保存修改的文件：保存后重新暂存会把没有暂存的修改带进提交。
            let dirty: Vec<&Path> = (self.editor.documents.iter())
                .filter(|d| d.is_dirty())
                .filter_map(|d| d.path.as_deref())
                .collect();
            targets.retain(|(p, _)| !dirty.contains(&self.editor.document_path(p).as_path()));
        } else {
            let paths: Vec<PathBuf> = targets.iter().map(|(p, _)| p.clone()).collect();
            if !self.save_dirty_documents(&paths) {
                return false;
            }
        }
        let jobs: Vec<(PathBuf, PathBuf, Plan, std::time::Duration)> = (targets.into_iter())
            .filter_map(|(path, repo)| {
                let (plan, timeout) = self.commit_format_plan(&path)?;
                Some((path, repo, plan, timeout))
            })
            .collect();
        if jobs.is_empty() {
            return false;
        }
        let formatter = self.format.formatter.clone();
        let sender = self.git.commit.results.0.clone();
        self.git.commit.busy = true;
        self.run_git_job(move |git, root| {
            let mut changed: Vec<(PathBuf, PathBuf)> = Vec::new();
            let mut failed = 0;
            for (path, repo, plan, timeout) in jobs {
                // 状态可能已经过时：磁盘上的内容此刻与暂存区不同的文件照原样提交。
                let matches = || (git.as_ref()).is_ok_and(|git| git.matches_index(&repo, &path));
                if staging && !matches() {
                    continue;
                }
                let Ok(bytes) = winc_backend::fs::read(&path) else {
                    continue;
                };
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                let mut rope = Rope::from_str(&text);
                let line_ending = LineEnding::detect(&rope);
                let job = Job {
                    plan,
                    text: rope.clone(),
                    line_ending,
                    timeout,
                };
                let Ok(formatted) = formatter.run(job, &FormatCancel::default()) else {
                    failed += 1;
                    continue;
                };
                if formatted.changes.is_empty() {
                    continue;
                }
                for change in &formatted.changes {
                    change.apply(&mut rope);
                }
                match winc_backend::fs::write(&path, rope.to_string()) {
                    Ok(()) => changed.push((path, repo)),
                    Err(_) => failed += 1,
                }
            }
            // 暂存区视图提交的是暂存区：格式化过的文件在各自的仓库里重新暂存。
            if staging && let Ok(git) = git {
                let mut repos: Vec<&PathBuf> = changed.iter().map(|(_, r)| r).collect();
                repos.sort();
                repos.dedup();
                for repo in repos {
                    let files: Vec<PathBuf> = (changed.iter())
                        .filter(|(_, r)| r == repo)
                        .map(|(p, _)| p.clone())
                        .collect();
                    // 没能重新暂存：整理过的内容留在工作区，提交的仍是原来暂存的。
                    if git.stage(repo, &files).is_err() {
                        failed += files.len();
                    }
                }
            }
            let done = changed.len().saturating_sub(failed);
            let _ = sender.send(Outcome::Reformatted(done, failed, root.to_path_buf()));
        });
        true
    }

    /// 提交前的整理做完了：说明整理了 `count` 个文件、`failed` 个没能整理而照原样提交，随后提交。
    pub(super) fn reformatted(&mut self, count: usize, failed: usize, root: &Path) {
        self.git.commit.busy = false;
        if self.git.root.as_deref() != Some(root) {
            self.git.commit.drop_push_intent();
            return;
        }
        // 用提示而不是状态栏：状态栏随后会换成提交完成的消息。
        if failed > 0 {
            let message = t!("git-commit-reformat-failed", count = failed);
            self.notify(NoticeKind::Warning, message);
        } else if count > 0 {
            let message = t!("git-commit-reformatted", count = count);
            self.notify(NoticeKind::Done, message);
        }
        self.git.commit.reformat_passed = true;
        self.git_commit_resumed();
    }

    /// 打开或关闭提交前格式化（提交按钮的菜单）。
    pub(super) fn toggle_reformat(&mut self) {
        self.update_state(move |state| state.commit_reformat = !state.commit_reformat);
    }
}
