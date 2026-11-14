//! 提交视图的布局：标题行、分支行、更改列表、提交卡片各自的位置。

use super::*;

impl App {
    /// 计算提交视图的布局。
    ///
    /// 自上而下依次为标题行、分支行、说明条（有进行中的操作时）、改动列表、还没推送的上一次
    /// 提交，最下面是提交卡片。先看改了什么、再写信息、最后提交，从上往下就是这个顺序。
    pub(crate) fn commit_parts(&self) -> CommitParts {
        let s = self.layout.sidebar;
        let panel = Rect::new(s.x, TOP_BAR_END, s.w, (s.bottom() - TOP_BAR_END).max(0.0));
        let header = Rect::new(panel.x, panel.y, panel.w, SIDEBAR_HEADER);
        let tool = |i: usize| {
            let x = header.right() - 8.0 - (3 - i) as f32 * TOOL;
            Rect::new(x, header.y + (header.h - TOOL) / 2.0, TOOL, TOOL)
        };
        let tools = [
            (tool(0), CommitTarget::Log),
            (tool(1), CommitTarget::Refresh),
            (tool(2), CommitTarget::More),
        ];
        // 浏览历史提交的文件时只有标题行和列表：列表从标题行下面一直到视图底部。
        if self.shown_commit_list().browse {
            let list_y = header.bottom() + SIDEBAR_LIST_GAP;
            let list = Rect::new(panel.x, list_y, panel.w, (panel.bottom() - list_y).max(0.0));
            return CommitParts {
                header,
                tools,
                location: None,
                branch: Rect::new(panel.x, header.bottom(), panel.w, 0.0),
                banner: None,
                card: None,
                message: Rect::default(),
                message_settled: Rect::default(),
                amend: None,
                commit: Rect::default(),
                commit_menu: Rect::default(),
                push: None,
                undo: None,
                stashes: None,
                list,
            };
        }
        let inner_x = panel.x + MARGIN;
        let inner_w = (panel.w - 2.0 * MARGIN).max(0.0);

        // Git 上下文：写在哪、在哪个分支、和上游差多少，提交前一眼能看到。
        let context_y = header.bottom();
        let branch = Rect::new(inner_x, context_y, inner_w, CONTEXT_ROW);
        // 工作树按钮在标题行上，选择面板从标题行下面展开，和各行同宽。
        let location = (self.git_has_locations_button())
            .then(|| Rect::new(inner_x, header.y, inner_w, header.h));
        let banner = (self.commit_banner_height() > 0.0).then(|| {
            let rect = Rect::new(inner_x, branch.bottom() + SIDEBAR_GAP, inner_w, BANNER_H);
            let w = BANNER_ABORT_W.min(rect.w);
            let abort = Rect::new(rect.right() - 2.0 - w, rect.y + 2.0, w, rect.h - 4.0);
            (rect, abort)
        });

        let share = self.commit_card_share();
        let card_share = share;
        let (card, message, message_settled, amend, buttons) = if share > 0.0 {
            self.commit_card_parts(panel, inner_x, inner_w, share)
        } else {
            Default::default()
        };
        let (commit, commit_menu, push) = buttons;
        // 「上一次提交」在视图底部，有提交卡片时紧接在卡片上沿：栏内上下各留 4，文字到上方分隔线
        // 和到卡片的距离相等，两者之间没有 `SIDEBAR_LIST_GAP`。改动列表在它上面结束。提交以后
        // 仍有未勾选的文件（未跟踪的文件默认不勾选）时卡片保持显示，栏同样显示。
        // 拉开、收起的途中栏按露出的高度算，里面的东西贴着上沿，从下沿底下露出来。
        let share = self.git.commit.undo.share(self.commit_undoable().is_some());
        let undo = (share > 0.0).then(|| {
            let bottom = card.map_or(panel.bottom(), |card| card.y);
            let bar = Rect::new(panel.x, bottom - UNDO_H * share, panel.w, UNDO_H * share);
            // 「⋯」比标题行的工具按钮小，中心和它们对齐，从上到下是一列。
            let h = UNDO_H - 8.0;
            let more = Rect::new(inner_x + inner_w - (TOOL + h) / 2.0, bar.y + 4.0, h, h);
            let w = self.git.commit.labels.undo + 2.0 * UNDO_PAD;
            let undo = Rect::new(more.x - BUTTON_GAP - w, more.y, w, h);
            (bar, undo, more)
        });
        let top = banner.map_or(branch.bottom(), |(rect, _)| rect.bottom());
        let list_y = top + SIDEBAR_LIST_GAP;
        let above = match (undo, card) {
            (Some((bar, ..)), _) => bar.y,
            // 卡片滑入、滑出时这段间隔按露出比例伸缩，卡片完全收起时列表的下沿与视图底部重合。
            (None, Some(card)) => card.y - SIDEBAR_LIST_GAP * card_share,
            (None, None) => panel.bottom(),
        };
        // 储藏一组在改动列表下面，标题和各条储藏各占一行，上下各留 `STASH_GAP`。一组的下沿紧贴
        // 卡片：`SIDEBAR_LIST_GAP` 是改动列表和卡片之间的间隔，再加上它时下方空隙远大于上方。
        let h = self.stash_section_height();
        let stash_bottom = match (undo, card) {
            (None, Some(card)) => card.y,
            _ => above,
        };
        let stashes = (h > 0.0).then(|| Rect::new(panel.x, stash_bottom - h, panel.w, h));
        let list_end = stashes.map_or(above, |rect| rect.y);
        let list = Rect::new(panel.x, list_y, panel.w, (list_end - list_y).max(0.0));
        CommitParts {
            header,
            tools,
            location,
            branch,
            banner,
            card,
            message,
            message_settled,
            amend,
            commit,
            commit_menu,
            push,
            undo,
            stashes,
            list,
        }
    }

    /// 提交卡片的各部分：卡片、提交信息（当前的和过渡结束后的）、「修改上次提交」，以及各个按钮
    /// （提交、下拉箭头、并排的「提交并推送…」）。
    ///
    /// 卡片贴着视图底部，信息变长时往上长。自上而下是「修改上次提交」一行、提交信息、按钮一行
    /// （见 [`App::classic_card_buttons`]），两种样式相同。进行中的合并、变基不能修改上一次
    /// 提交，没有那一行。
    ///
    /// `share` 是卡片露出的比例（见 [`App::commit_card_share`]）：不到 1 时整张卡片往下移，没露
    /// 出的部分在视图底部下面。
    fn commit_card_parts(
        &self,
        panel: Rect,
        inner_x: f32,
        inner_w: f32,
        share: f32,
    ) -> (Option<Rect>, Rect, Rect, Option<Rect>, CardButtons) {
        let settled_h = self.commit_message_target(panel.h);
        let message_h = self
            .git
            .commit
            .message_motion
            .map_or(settled_h, |(from, clock)| clock.between(from, settled_h));
        // 「修改上次提交」单独占提交信息上方的一行，始终可见。
        let amendable = self.commit_amendable();
        let top_row = if amendable {
            CARD_INSET + FOOTER_H
        } else {
            0.0
        };
        let card_h = top_row + message_h + FOOTER_GAP + FOOTER_H + CARD_INSET;
        let sunk = (1.0 - share) * (card_h + MARGIN);
        let card = Rect::new(
            inner_x,
            panel.bottom() - MARGIN - card_h + sunk,
            inner_w,
            card_h,
        );
        let message = Rect::new(card.x, card.y + top_row, card.w, message_h);
        let message_settled = Rect::new(card.x, card.y + top_row, card.w, settled_h);
        let footer = Rect::new(
            card.x + CARD_INSET,
            message.bottom() + FOOTER_GAP,
            (card.w - 2.0 * CARD_INSET).max(0.0),
            FOOTER_H,
        );
        let labels = self.git.commit.labels;
        let buttons = self.classic_card_buttons(footer);
        // 「修改上次提交」的勾选框在这一段里离左边留 1，整段左移 1：勾选框的左边与按钮、信息文字
        // 的左边对齐。
        let amend_x = footer.x - 1.0;
        let amend =
            amendable.then(|| Rect::new(amend_x, card.y + CARD_INSET, labels.amend, FOOTER_H));
        (Some(card), message, message_settled, amend, buttons)
    }

    /// 勾选式下提交卡片底行 `footer` 的各个按钮：「提交」「提交并推送…」靠左并排，下拉箭头在
    /// 最右。放不下时「提交并推送…」收进下拉菜单。
    fn classic_card_buttons(&self, footer: Rect) -> CardButtons {
        let labels = self.git.commit.labels;
        let menu_w = BUTTON_MENU_W.min(footer.w);
        let commit_menu = Rect::new(footer.right() - menu_w, footer.y, menu_w, FOOTER_H);
        let room = (commit_menu.x - PUSH_GAP - footer.x).max(0.0);
        let natural = labels.button + 2.0 * BUTTON_PAD;
        let width = (self.git.commit.button_motion)
            .map_or(natural, |(from, clock)| clock.between(from, natural));
        let commit = Rect::new(footer.x, footer.y, width.min(room), FOOTER_H);
        let push_w = labels.push + 2.0 * BUTTON_PAD;
        let fits = commit.right() + PUSH_GAP + push_w + PUSH_GAP <= commit_menu.x;
        let push = (self.commit_push_beside() && fits)
            .then(|| Rect::new(commit.right() + PUSH_GAP, footer.y, push_w, FOOTER_H));
        (commit, commit_menu, push)
    }

    /// 这时是否把「提交并推送…」排在提交按钮旁边：有文字宽度可用，没有进行中的合并、变基，
    /// 也没有在修改上一次提交（这两种情况推送没有意义或需要强制推送）。
    fn commit_push_beside(&self) -> bool {
        self.git.commit.labels.push > 0.0
            && !self.git.commit.amend
            && (self.git.status.as_ref()).is_some_and(|s| s.operation.is_none())
    }
}
