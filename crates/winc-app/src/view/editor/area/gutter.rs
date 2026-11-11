//! 一行在行号栏里的部分：行号、Git 改动标记、逐行追溯、书签和折叠箭头。

use winc_i18n::t;

use super::*;

impl Screen<'_> {
    /// 行号；有光标的行颜色深一些。
    ///
    /// `number` 是复用的缓冲区。
    pub(super) fn draw_number(&self, frame: &mut Frame<'_>, row: &Row<'_>, number: &mut String) {
        if self.app.config.line_numbers == Some(false) {
            return;
        }
        let (p, gutter, line) = (self.app.palette, self.gutter, row.line);
        number.clear();
        // 相对行号：当前行显示行号，其余各行显示与当前行的距离。
        let shown = match self.relative {
            true if line != self.caret_line => line.abs_diff(self.caret_line),
            _ => line + 1,
        };
        let _ = write!(number, "{shown}");
        let number_x = gutter.right() - GUTTER_PADDING - number.len() as f32 * self.cell;
        let number_color =
            if line == self.caret_line || self.extra_carets.iter().any(|c| c.0 == line) {
                p.gutter_text_current
            } else {
                p.gutter_text
            };
        frame.text(
            FontKind::Mono,
            self.app.fonts.code,
            number_x,
            row.gutter_baseline,
            number,
            number_color,
            gutter,
        );
    }

    /// Git 改动：新增、修改的行在行号右边一道细色条（自动换行时盖住这一行的所有段），删掉的
    /// 地方在两行之间绘制一个朝右的小三角。
    ///
    /// 颜色取差异视图的柔和色；卡片打开着的那一处、鼠标指着的那一处绘制宽一些。
    pub(super) fn draw_change_marks(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let (app, p, gutter, lh) = (self.app, self.app.palette, self.gutter, self.lh);
        let (line, y, hunks) = (row.line, row.y, self.hunks);
        let mark_x = mark_x(gutter);
        // 改动按位置排好，二分找到第一处可能落在这一行的，不用每行从头数。
        let skip = hunks.partition_point(|h| h.new.start + h.new.len().max(1) <= line);
        for (i, h) in hunks
            .iter()
            .enumerate()
            .skip(skip)
            .take_while(|(_, h)| h.new.start <= line)
        {
            let shade = diff::hunk_shade(p, h);
            let lit = app.hover == Hover::ChangeMark(i) || self.open == Some(i);
            if h.new.is_empty() {
                if h.new.start == line {
                    draw_deleted_mark(frame, (mark_x, y), shade.mark, lit, gutter);
                }
            } else if h.new.contains(&line) {
                let height = lh * row.breaks.rows() as f32;
                // 一块的第一行和最后一行上下各缩进 1 逻辑像素，相邻的两块不连成一条。
                let top = if line == h.new.start { 1.0 } else { 0.0 };
                let bottom = if line + 1 == h.new.end { 1.0 } else { 0.0 };
                let w = if lit { MARK_W + 1.0 } else { MARK_W };
                let bar = Rect::new(mark_x, y + top, w, height - top - bottom);
                frame.rect_clipped(bar, shade.mark, 0.0, gutter);
            }
        }
        // 删掉的是文件末尾的几行：三角绘制在最后一行的底下。
        if line + 1 == self.doc.text().line_count()
            && let Some((i, h)) = hunks.iter().enumerate().next_back()
            && h.new.is_empty()
            && h.new.start > line
        {
            let lit = app.hover == Hover::ChangeMark(i) || self.open == Some(i);
            let mark = diff::hunk_shade(p, h).mark;
            draw_deleted_mark(frame, (mark_x, y + lh), mark, lit, gutter);
        }
    }

    /// 逐行追溯：同一次提交连着的几行只在第一行写作者和时间，每一行左边一道按新旧深浅的色条；
    /// 鼠标指着的那一次提交的几行字深一些。
    pub(super) fn draw_blame(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let (app, p, gutter, lh) = (self.app, self.app.palette, self.gutter, self.lh);
        let (line, blame_w) = (row.line, self.blame_w);
        let (Some(blame), Some((oldest, newest))) = (self.blame, self.ages) else {
            return;
        };
        let Some(&index) = blame.lines.get(line) else {
            return;
        };
        let Some(commit) = blame.commits.get(index as usize) else {
            return;
        };
        let column = Rect::new(gutter.x, gutter.y, blame_w, gutter.h);
        let height = lh * row.breaks.rows() as f32;
        let strength = if commit.uncommitted || newest == oldest {
            1.0
        } else {
            (commit.time - oldest) as f32 / (newest - oldest) as f32
        };
        let bar_color = if commit.uncommitted {
            p.diff_modified.mark
        } else {
            p.accent.alpha(p.accent.a * (0.12 + 0.7 * strength))
        };
        frame.rect_clipped(
            Rect::new(gutter.x + 6.0, row.y, 3.0, height),
            bar_color,
            0.0,
            column,
        );
        let first = line == 0 || blame.lines.get(line - 1) != Some(&index);
        if first {
            let gutter_y = row.gutter_y;
            let hovered = self.blame_hover == Some(index);
            let band = Rect::new(gutter.x, gutter_y, blame_w, lh);
            let clip = Rect::new(gutter.x + 14.0, gutter_y, blame_w - 22.0, lh);
            let (who, color) = if commit.uncommitted {
                (t!("editor-blame-uncommitted").to_owned(), p.text_dim)
            } else if hovered {
                (commit.author.clone(), p.text)
            } else {
                (commit.author.clone(), p.text_muted)
            };
            let when = if commit.uncommitted {
                String::new()
            } else {
                crate::app::git::blame::blame_time(commit, self.now)
            };
            let when_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &when);
            let when_x = gutter.x + blame_w - 10.0 - when_w;
            let who_clip = Rect::new(clip.x, clip.y, (when_x - 8.0 - clip.x).max(0.0), lh);
            ui_text_small(frame, app, clip.x, band, &who, color, who_clip);
            let when_color = if hovered { p.text_muted } else { p.text_dim };
            ui_text_small(frame, app, when_x, band, &when, when_color, column);
        }
    }

    /// 书签绘制在行号左侧的空白处（开着逐行追溯时在那一栏右边）。
    pub(super) fn draw_bookmark(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        if self.doc.bookmarks().contains(row.line) {
            let gutter = self.gutter;
            let center = (
                gutter.x + self.blame_w + GUTTER_PADDING / 2.0 + 1.0,
                row.gutter_y + self.lh / 2.0,
            );
            icons::bookmark(frame, center, self.app.palette.accent, gutter);
        }
    }

    /// 折叠箭头：已折叠的行、鼠标在行号栏上时可折叠的行，以及动画中转动的那一个。
    pub(super) fn draw_fold_arrow(&self, frame: &mut Frame<'_>, row: &Row<'_>) {
        let line = row.line;
        let turn = self
            .turning
            .filter(|&(at, _)| at == line)
            .map(|(_, open)| open);
        if row.collapsed || turn.is_some() || self.foldable.contains(&line) {
            let gutter = self.gutter;
            let column = fold_column(gutter);
            // 折叠箭头位于行号栏中靠近文本的一侧，远离行号。
            let center = (
                column.x + column.w / 2.0 + 1.0,
                row.gutter_y + self.lh / 2.0,
            );
            // 展开时朝下，折叠时朝右，动画中在两者之间转动。
            let open = turn.unwrap_or(if row.collapsed { 0.0 } else { 1.0 });
            let angle = open * std::f32::consts::FRAC_PI_2;
            icons::chevron_turned(frame, center, 6.0, angle, self.app.palette.text_dim, gutter);
        }
    }
}
