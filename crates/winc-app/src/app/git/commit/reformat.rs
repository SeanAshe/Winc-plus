//! 提交前格式化：开着时（提交按钮的菜单），提交前先用格式化工具整理要提交的文件，再提交。
//!
//! 只整理整个文件都提交的：勾选式下有取消勾选的改动、暂存视图中只暂存了一部分的文件照原样
//! 提交，否则整理后的内容与选好的改动对不上。工具与手动格式化相同；还没有允许运行的项目工具
//! 不在提交途中询问，跳过对应的文件。格式化在磁盘上进行，打开着的文件先保存。

use ropey::Rope;
use winc_core::LineEnding;
use winc_editor::format::{Cancel as FormatCancel, Job, Plan};
use winc_editor::git::Change;

use super::*;

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
            let skipped = &self.git.commit.skipped;
            let picked = self.git.commit.picked_entries(status);
            picked
                .filter(|e| whole(e) && skipped.get(&e.path).is_none_or(|s| s.is_empty()))
                .map(|e| e.path.clone())
                .collect()
        };
        // 其他仓库里一并提交的文件同样先格式化，记下所在的仓库：暂存区视图中格式化后要重新暂存。
        let root = self.git.root.clone().unwrap_or_default();
        let mut targets: Vec<(PathBuf, PathBuf)> =
            paths.into_iter().map(|p| (p, root.clone())).collect();
        targets.extend(self.other_reformat_paths());
        let paths: Vec<PathBuf> = targets.iter().map(|(p, _)| p.clone()).collect();
        if !self.save_dirty_documents(&paths) {
            return false;
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
            for (path, repo, plan, timeout) in jobs {
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
                    continue;
                };
                if formatted.changes.is_empty() {
                    continue;
                }
                for change in &formatted.changes {
                    change.apply(&mut rope);
                }
                if winc_backend::fs::write(&path, rope.to_string()).is_ok() {
                    changed.push((path, repo));
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
                    let _ = git.stage(repo, &files);
                }
            }
            let _ = sender.send(Outcome::Reformatted(changed.len(), root.to_path_buf()));
        });
        true
    }

    /// 提交前的整理做完了：说明整理了几个文件，接着提交。
    pub(super) fn reformatted(&mut self, count: usize, root: &Path) {
        self.git.commit.busy = false;
        if self.git.root.as_deref() != Some(root) {
            self.git.commit.push_after = false;
            return;
        }
        if count > 0 {
            self.flash(t!("git-commit-reformatted", count = count));
        }
        self.git.commit.reformat_passed = true;
        self.git_commit();
    }

    /// 打开或关闭提交前格式化（提交按钮的菜单）。
    pub(super) fn toggle_reformat(&mut self) {
        self.update_state(|state| state.commit_reformat = !state.commit_reformat);
    }
}
