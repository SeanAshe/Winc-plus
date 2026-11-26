//! 提交视图的布局：标题行、分支行、更改列表、提交卡片各自的位置。

use super::*;

impl App {
    /// 计算提交视图的布局。
    ///
    /// 自上而下依次为标题行、分支行、说明条（有进行中的操作时）、改动列表、提交历史、工作树、
    /// 储藏、提交卡片和尚未推送的上一次提交。
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
        let inner_x = panel.x + MARGIN;
        let inner_w = (panel.w - 2.0 * MARGIN).max(0.0);

        // Git 上下文：写在哪、在哪个分支、和上游差多少，提交前一眼能看到。
        let context_y = header.bottom();
        let branch = Rect::new(inner_x, context_y, inner_w, CONTEXT_ROW);
        // 仓库按钮在上下文一行的左端，选择面板从这一行下面展开，和各行同宽。
        let location = (self.git.repositories.len() >= 2).then_some(branch);
        let banner = (self.commit_banner_height() > 0.0).then(|| {
            let rect = Rect::new(inner_x, branch.bottom() + SIDEBAR_GAP, inner_w, BANNER_H);
            let w = BANNER_ABORT_W.min(rect.w);
            let abort = Rect::new(rect.right() - 2.0 - w, rect.y + 2.0, w, rect.h - 4.0);
            (rect, abort)
        });

        // 「上一次提交」在视图的最底部，栏内上下各留 4；显示、
        // 隐藏的过渡中按露出的高度计算，卡片随之移动。
        let undo_share = self.git.commit.undo.share(self.commit_undoable().is_some());
        let undo = (undo_share > 0.0).then(|| {
            let h = UNDO_H * undo_share;
            let bar = Rect::new(panel.x, panel.bottom() - h, panel.w, h);
            // 「⋯」比标题行的工具按钮小，中心和它们对齐，从上到下是一列。
            let h = UNDO_H - 8.0;
            let more = Rect::new(inner_x + inner_w - (TOOL + h) / 2.0, bar.y + 4.0, h, h);
            let w = self.git.commit.labels.undo + 2.0 * UNDO_PAD;
            let undo = Rect::new(more.x - BUTTON_GAP - w, more.y, w, h);
            (bar, undo, more)
        });
        let card_share = self.commit_card_share();
        let (card, message, message_settled, amend, buttons) = if card_share > 0.0 {
            // 卡片的下沿与「上一次提交」一栏的上沿重合：栏内文字上方已留 4。
            // 没有该栏时卡片离视图底部一个 `MARGIN`。
            let bottom = undo.map_or(panel.bottom(), |(bar, ..)| bar.y + MARGIN * undo_share);
            let room = Rect::new(panel.x, panel.y, panel.w, (bottom - panel.y).max(0.0));
            self.commit_card_parts(room, inner_x, inner_w, card_share)
        } else {
            Default::default()
        };
        let (commit, commit_menu, push) = buttons;
        let top = banner.map_or(branch.bottom(), |(rect, _)| rect.bottom());
        let list_y = top + SIDEBAR_LIST_GAP;
        let above = match (card, undo) {
            // 卡片滑入、滑出时这段间隔按露出比例伸缩，卡片完全收起时列表的下沿与视图底部重合。
            (Some(card), _) => card.y - SIDEBAR_LIST_GAP * card_share,
            (None, Some((bar, ..))) => bar.y,
            (None, None) => panel.bottom(),
        };
        // 储藏、工作树两组紧贴卡片：标题行到卡片的留白与到上沿分隔线的相同。
        let h = self.stash_section_height();
        let stash_bottom = card.map_or(above, |card| card.y);
        let stashes = (h > 0.0).then(|| Rect::new(panel.x, stash_bottom - h, panel.w, h));
        // 工作树一组在储藏上面，两组之间没有间隔。
        let h = self.worktree_section_height();
        let bottom = stashes.map_or(stash_bottom, |rect| rect.y);
        let worktrees = (h > 0.0).then(|| Rect::new(panel.x, bottom - h, panel.w, h));
        // 提交历史一组在工作树上面，同样没有间隔。侧栏较矮时它先让出高度：给改动列表留足
        // `HISTORY_LIST_KEEP`，组内露出的行变少（可以滚动）；连标题都放不下时不显示。
        let bottom = worktrees.or(stashes).map_or(stash_bottom, |rect| rect.y);
        let room = bottom - list_y - HISTORY_LIST_KEEP;
        let h = self.history_section_height().min(room);
        let history =
            (h > 0.0 && room >= DRAWER_CLOSED).then(|| Rect::new(panel.x, bottom - h, panel.w, h));
        let list_end = (history.or(worktrees).or(stashes)).map_or(above, |rect| rect.y);
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
            worktrees,
            history,
            list,
        }
    }

    /// 提交卡片的各部分：卡片、提交信息（当前的和过渡结束后的）、「修改上次提交」，以及各个按钮。
    ///
    /// 卡片贴着视图底部，信息变长时往上长。自上而下是「修改上次提交」一行、提交信息、按钮一行（见
    /// [`App::classic_card_buttons`]）；进行中的合并、变基不能修改上一次提交，没有那一行。`share`
    /// 是卡片露出的比例（见 [`App::commit_card_share`]）：不到 1 时整张卡片往下移，没露出的部分在视
    /// 图底部下面。
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
        // 「修改上次提交」（修正、压缩到历史提交时是那一项）单独占提交信息上方的一行，始终可见。
        let amendable = self.commit_amendable() || self.git.commit.into.is_some();
        let top_row = if amendable { AMEND_TOP + AMEND_H } else { 0.0 };
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
            amendable.then(|| Rect::new(amend_x, card.y + AMEND_TOP, labels.amend, AMEND_H));
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
    /// 也没有在修改上一次提交或并入历史提交（这些情况推送没有意义或需要强制推送）。
    fn commit_push_beside(&self) -> bool {
        self.git.commit.labels.push > 0.0
            && !self.git.commit.amend
            && self.git.commit.into.is_none()
            && (self.git.status.as_ref()).is_some_and(|s| s.operation.is_none())
    }
}
