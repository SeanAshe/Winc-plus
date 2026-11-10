//! 详情栏：选中和复制文字、改动的文件（提示、菜单、打开）。

use super::*;

impl App {
    /// 绘制前记下详情一栏这一帧要画哪个提交（`oid`）：换成了另一个提交（不是从没有到有）时，
    /// 内容从四成不透明淡入（`DETAILS_FADE`）；动态效果关闭、没有窗口时不淡入。
    pub(crate) fn note_log_details_shown(&mut self, oid: Option<String>) {
        if oid == self.log.shown {
            return;
        }
        if self.log.shown.is_some() && oid.is_some() {
            self.log.fade = self.clock(crate::app::animation::DETAILS_FADE);
        }
        self.log.shown = oid;
    }

    /// 详情里 `(x, y)` 处的字的位置。
    ///
    /// `near` 为 `true` 时（拖着选的时候）不在任何一行上也取
    /// 最近的一行，拖到头尾以外时停在头尾；否则只认正落在某一行上的位置。
    pub(crate) fn log_text_at(&self, x: f32, y: f32, near: bool) -> Option<TextPoint> {
        let runs = &self.log.runs;
        // 同一行上可能有几段（作者、邮箱）：取开头在鼠标左边的最后一段。
        let on = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.row.y <= y && y < r.row.bottom())
            .filter(|(_, r)| r.x - 2.0 <= x)
            .map(|(i, _)| i)
            .next_back()
            .or_else(|| {
                runs.iter()
                    .position(|r| r.row.y <= y && y < r.row.bottom())
                    .filter(|_| near)
            });
        let run = match on {
            Some(i) => i,
            None if near => {
                let first = runs.first()?;
                if y < first.row.y {
                    return Some(TextPoint { run: 0, ch: 0 });
                }
                let last = runs.len() - 1;
                if y >= runs[last].row.bottom() {
                    return Some(TextPoint {
                        run: last,
                        ch: runs[last].edges.len(),
                    });
                }
                // 两行之间的空隙：算上面那一行的行尾。
                let above = runs.iter().rposition(|r| r.row.bottom() <= y)?;
                return Some(TextPoint {
                    run: above,
                    ch: runs[above].edges.len(),
                });
            }
            None => return None,
        };
        // 只在字上（不在行尾以外的空白里）才算按在字上；拖着的时候行尾以外算到行尾。
        let r = &runs[run];
        let end = r.edges.last().copied().unwrap_or(r.x);
        if !near && (x < r.x - 2.0 || x > end + 4.0) {
            return None;
        }
        Some(TextPoint {
            run,
            ch: r.index_at(x),
        })
    }

    /// 第 `run` 行选中了第几个字到第几个字（`len` 是这一行的字数）；这一行没选中时为 `None`。
    pub(crate) fn log_selected_chars(&self, run: usize, len: usize) -> Option<(usize, usize)> {
        let (a, b) = self.log.text_sel?;
        let (from, to) = (a.min(b), a.max(b));
        if run < from.run || run > to.run {
            return None;
        }
        let start = if run == from.run { from.ch } else { 0 };
        let end = if run == to.run { to.ch } else { len };
        (start < end).then_some((start, end))
    }

    /// 详情里选中的字；没选中时为 `None`。
    ///
    /// 一段话折成的几行直接连起来，不同的行之间换行；截短显示的邮箱在整行选中时给出完整内容。
    pub(crate) fn log_selected_text(&self) -> Option<String> {
        let (a, b) = self.log.text_sel?;
        let (from, to) = (a.min(b), a.max(b));
        let mut out = String::new();
        for run in from.run..=to.run.min(self.log.runs.len().saturating_sub(1)) {
            let r = self.log.runs.get(run)?;
            let len = r.edges.len();
            let (start, end) = self.log_selected_chars(run, len).unwrap_or((0, 0));
            if run > from.run && !r.soft {
                out.push('\n');
            }
            match &r.full {
                Some(full) if start == 0 && end == len => out.push_str(full),
                _ => out.extend(r.text.chars().skip(start).take(end - start)),
            }
        }
        (!out.trim().is_empty()).then_some(out)
    }

    /// 在详情里按下鼠标（没按在可以点的东西上）：按在字上开始选字，`count` 为 2（双击）时选中
    /// 这一整行；按在空白处清掉选中。
    pub(in crate::app) fn press_log_text(&mut self, x: f32, y: f32, count: u8) -> bool {
        self.focus = Focus::Editor;
        let Some(at) = self.log_text_at(x, y, false) else {
            self.log.text_sel = None;
            self.request_redraw();
            return false;
        };
        self.log.text_sel = Some(if count >= 2 {
            let len = self.log.runs[at.run].edges.len();
            (
                TextPoint { run: at.run, ch: 0 },
                TextPoint {
                    run: at.run,
                    ch: len,
                },
            )
        } else {
            (at, at)
        });
        self.request_redraw();
        count < 2
    }

    /// 拖着选字时鼠标到了 `(x, y)`。
    pub(in crate::app) fn drag_log_text(&mut self, x: f32, y: f32) {
        if let (Some((anchor, _)), Some(head)) = (self.log.text_sel, self.log_text_at(x, y, true)) {
            self.log.text_sel = Some((anchor, head));
            self.request_redraw();
        }
    }

    /// 从第 `row` 行的线 `line` 顺着往下（往父提交的方向）走到的提交：这条线上的下一个提交，
    /// 或者这条线合进别的线时合进去的那个提交。
    ///
    /// 线从第 `row` 行自己的圆点出去时不算这一行。
    /// 还没取回来时为 `None`。
    pub(crate) fn log_line_target(&self, row: usize, line: u32) -> Option<usize> {
        let view = self.editor.active()?.log.as_deref()?;
        view.graph
            .iter()
            .enumerate()
            .skip(row)
            .find(|(j, g)| {
                if g.line == line {
                    *j > row
                } else {
                    g.up.iter().any(|s| s.line == line && s.to == g.lane)
                }
            })
            .map(|(j, _)| j)
    }

    /// 详情（或比较）里第 `i` 个改动的文件。
    pub(crate) fn log_file(&self, i: usize) -> Option<&CommitFile> {
        let view = self.editor.active()?.log.as_deref()?;
        if let Some(compare) = &view.compare {
            return compare.files.as_ref()?.get(i);
        }
        let oid = view.selected_oid()?;
        view.details
            .as_ref()
            .filter(|d| d.oid == oid)?
            .files
            .as_ref()?
            .get(i)
    }

    /// 详情里文件悬停提示的完整路径（相对仓库根目录）；改过名的写「旧路径 → 新路径」。
    pub(crate) fn log_file_tip(&self, i: usize) -> Option<String> {
        let file = self.log_file(i)?;
        let root = self.git.root.as_deref()?;
        let rel = |p: &Path| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        };
        Some(match &file.from {
            Some(from) => format!("{} → {}", rel(from), rel(&file.path)),
            None => rel(&file.path),
        })
    }

    /// 在日志详情里右键的位置：点着的文件（下标和路径）。
    ///
    /// 点着了就让键盘也选上它。
    pub(in crate::app) fn log_file_context_target(
        &mut self,
        x: f32,
        y: f32,
    ) -> Option<(usize, PathBuf)> {
        if !self.log_active() {
            return None;
        }
        let LogTarget::File(i) = crate::layout::hit(&self.hits.log, x, y)? else {
            return None;
        };
        let path = self.log_file(i)?.path.clone();
        self.focus = Focus::Editor;
        self.log.file = Some(i);
        Some((i, path))
    }

    /// 日志详情里一个文件的右键菜单命令。
    pub(in crate::app) fn run_log_file_command(
        &mut self,
        command: MenuCommand,
        i: usize,
        path: PathBuf,
    ) {
        match command {
            MenuCommand::GitShowDiff => self.open_log_file(i),
            MenuCommand::GitOpenFile => {
                if path.is_file() {
                    self.open_path(&path);
                    self.focus = Focus::Editor;
                }
            }
            MenuCommand::FileHistory => self.git_file_history(&path),
            MenuCommand::CopyPath | MenuCommand::CopyRelativePath => {
                self.copy_path(&path, command == MenuCommand::CopyRelativePath);
            }
            MenuCommand::Reveal => self.reveal_in_file_manager(&path),
            MenuCommand::RevealInTree => {
                self.show_files();
                self.reveal_path(&path);
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// 详情里列着几个改动的文件（比较两个版本时是两者之间的）；还没取回来时是 0。
    pub(crate) fn log_files_len(&self) -> usize {
        let Some(view) = self.editor.active().and_then(|d| d.log.as_deref()) else {
            return 0;
        };
        if let Some(compare) = &view.compare {
            return compare.files.as_ref().map_or(0, Vec::len);
        }
        let Some(oid) = view.selected_oid() else {
            return 0;
        };
        view.details
            .as_ref()
            .filter(|d| d.oid == oid)
            .and_then(|d| d.files.as_ref())
            .map_or(0, Vec::len)
    }

    /// 让详情里第 `i` 个文件露出来：按上一帧画出来的文件行推算它在哪，在详情一栏外面就滚。
    pub(super) fn reveal_log_file(&mut self, i: usize) {
        let area = self.log_parts().details;
        // 最下面一行小字（比较的提示）占 30 逻辑像素。
        let bottom = area.bottom() - 30.0;
        let Some(top) = self.hits.log.iter().find_map(|(rect, t)| match t {
            LogTarget::File(j) => Some(rect.y + (i as f32 - *j as f32) * LOG_FILE_ROW),
            _ => None,
        }) else {
            return;
        };
        let Some(view) = self.editor.active_mut().and_then(|d| d.log.as_deref_mut()) else {
            return;
        };
        if top < area.y {
            view.details_scroll = (view.details_scroll - (area.y - top) - 8.0).max(0.0);
        } else if top + LOG_FILE_ROW > bottom {
            view.details_scroll += top + LOG_FILE_ROW - bottom + 8.0;
        }
    }

    /// 双击了第 `row` 个提交：打开它改动的第一个文件；文件还没取回来就等取回来再开。
    pub(in crate::app) fn open_log_row_first_file(&mut self, row: usize) {
        let Some(oid) = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref()?.commits.get(row).map(|c| c.oid.clone()))
        else {
            return;
        };
        if self.log_files_len() > 0 {
            self.open_log_file(0);
        } else {
            self.log.open_first = Some(oid);
        }
    }
}
