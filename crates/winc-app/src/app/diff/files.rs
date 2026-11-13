//! 差异导航跨文件的移动：到达首尾时等待再次按键，之后切换到相邻的文件。

use super::*;

impl App {
    /// 切换到下一个（`forward`）或上一个文件，从第一处改动开始显示；没有更多文件时给出提示。
    pub(in crate::app) fn step_diff_file(&mut self, forward: bool) {
        if !self.enter_diff_file(forward, false) {
            self.flash(if forward {
                t!("git-no-next-file")
            } else {
                t!("git-no-previous-file")
            });
        }
    }

    /// 打开同一批文件中的下一个（`forward`）或上一个文件，返回是否已打开。
    ///
    /// 差异由储藏打开时为同一条储藏中的文件，由日志打开时为同一提交中的文件，否则为提交视图
    /// 列出的文件，均在当前标签中切换。`last` 为 `true` 时打开后停在最后一处改动，否则停在第一处。
    pub(super) fn enter_diff_file(&mut self, forward: bool, last: bool) -> bool {
        self.diff.land = Some(last);
        let entered = self.step_stash_file(forward)
            || self.step_log_file(forward)
            || self.step_changed_file(forward);
        if !entered {
            self.diff.land = None;
        }
        entered
    }

    /// 切换文件后的差异载入后，按 [`App::enter_diff_file`] 的要求跳转到第一处或最后一处改动。
    pub(in crate::app) fn land_diff_file(&mut self) {
        let Some(last) = self.diff.land.take() else {
            return;
        };
        let hunks = self
            .editor
            .active()
            .and_then(|d| d.diff.as_deref())
            .map_or(0, |v| v.hunks.len());
        if hunks > 0 {
            self.go_to_hunk(if last { hunks - 1 } else { 0 });
        }
    }

    /// 返回下一个（`forward`）或上一个文件的文件名；没有更多文件时为 `None`。
    pub(super) fn next_diff_file_name(&self, forward: bool) -> Option<String> {
        if let Some((_, name, _)) = self.next_stash_file(forward) {
            return Some(name);
        }
        if let Some((_, _, name, _)) = self.next_log_file(forward) {
            return Some(name);
        }
        let (path, _, _) = self.next_changed_file(forward)?;
        Some(winc_editor::display_name(&path).into_owned())
    }

    /// 处理导航到达首尾时的按键：第一次提示「再按一次」，提示仍在显示时第二次按键才切换文件。
    ///
    /// 切换到上一个文件时停在其最后一处改动；没有更多文件时提示 `none`。差异可能正在编辑，
    /// 因此不在一次按键中替换用户正在查看的文件。
    pub(super) fn cross_diff_file(&mut self, forward: bool, none: &str) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        let cross = DiffCross {
            doc: doc.id,
            forward,
            hunk: view.focus.map(|(hunk, _)| hunk),
            scroll: doc.scroll_line,
        };
        let armed = self.diff.cross.take();
        if armed.is_some_and(|(at, armed)| armed == cross && at.elapsed() < FLASH_DURATION)
            && self.enter_diff_file(forward, !forward)
        {
            return;
        }
        let Some(name) = self.next_diff_file_name(forward) else {
            self.flash(none);
            return;
        };
        self.diff.cross = Some((Instant::now(), cross));
        let name = name.as_str();
        self.flash(if forward {
            t!("git-press-again-next-file", name = name)
        } else {
            t!("git-press-again-previous-file", name = name)
        });
    }

    /// 当前差异标签是否可以切换文件：由日志、提交视图或储藏打开，或为工作区文件的 Git 差异。
    pub(crate) fn diff_steps_files(&self) -> bool {
        let Some(id) = self.editor.active().map(|d| d.id) else {
            return false;
        };
        let stash =
            self.git.commit.stash_view.is_some() && self.git.commit.stash_preview == Some(id);
        stash || self.steps_log_files(id) || self.steps_changed_files(id)
    }

    /// 滚动到下一处（`forward`）或上一处改动。
    ///
    /// 没有更多改动时按 [`App::cross_diff_file`] 切换文件。
    pub(in crate::app) fn jump_difference(&mut self, forward: bool) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        // 判断当前所在段：刚跳转、尚未滚动时取跳转到的那一段（末尾几段无法滚到视口顶部，
        // 不能按滚动位置判断），否则按视口第四行判断。
        let here = doc.scroll_line.max(0.0).floor() + 3.0;
        let target = match (view.focus, view.current_hunk(doc.scroll_line)) {
            (Some((_, at)), Some(current)) if at == doc.scroll_line => {
                let next = if forward {
                    Some(current + 1)
                } else {
                    current.checked_sub(1)
                };
                next.filter(|&i| i < view.hunks.len())
            }
            _ => {
                let mut hunks = 0..view.hunks.len();
                if forward && view.focus.is_none() {
                    // 尚无当前段时从视口顶部开始查找，不跳过靠近文件开头、已在视口内的改动。
                    let top = doc.scroll_line.max(0.0).floor();
                    hunks.find(|&i| view.hunk_pos(i) >= top)
                } else if forward {
                    hunks.find(|&i| view.hunk_pos(i) > here)
                } else {
                    hunks.rev().find(|&i| view.hunk_pos(i) < here)
                }
            }
        };
        match target {
            // 跳转成功后，上一次「没有更多改动」的提示不再成立。
            Some(hunk) => {
                self.clear_flash();
                self.go_to_hunk(hunk);
            }
            None => {
                let message = if view.is_identical() {
                    t!("git-diff-identical-sides")
                } else if forward {
                    t!("git-change-no-next")
                } else {
                    t!("git-change-no-previous")
                };
                self.cross_diff_file(forward, message);
            }
        }
    }
}
