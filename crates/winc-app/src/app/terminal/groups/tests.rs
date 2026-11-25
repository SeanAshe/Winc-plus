//! 分栏的数据：各栏的位置、换栏、加栏、去掉空栏和按排列顺序找标签。

use super::*;

/// 编号为 `id` 的空标签。
fn tab(id: u16) -> TermTab {
    let size = Size {
        cols: 80,
        rows: 24,
        width: 0,
        height: 0,
    };
    TermTab::new(id, size)
}

/// 各栏的标签编号，按排列顺序；正在用的一栏的标签在 `tabs` 里。
fn ids(t: &TerminalPanel) -> Vec<Vec<u16>> {
    (0..t.group_count())
        .map(|at| t.group_tabs(at).iter().map(|tab| tab.id).collect())
        .collect()
}

/// 面板里有标签 1、2，正在用的是第一栏。
fn panel() -> TerminalPanel {
    TerminalPanel {
        tabs: vec![tab(1), tab(2)],
        ..TerminalPanel::default()
    }
}

#[test]
fn groups_share_the_panel_by_their_shares_between_dividers() {
    let card = Rect::new(10.0, 20.0, 401.0, 100.0);
    let cards = group_cards(card, &[1.0, 1.0], false);
    assert_eq!(cards[0], Rect::new(10.0, 20.0, 200.0, 100.0));
    // 最后一栏铺到面板边上，不留舍入的零头。
    assert_eq!(cards[1].x, 10.0 + 200.0 + DIVIDER);
    assert_eq!(cards[1].right(), card.right());

    let cards = group_cards(card, &[3.0, 1.0], true);
    assert_eq!(cards[0].h, ((100.0 - DIVIDER) * 0.75_f32).floor());
    assert_eq!(cards[1].bottom(), card.bottom());
    assert_eq!(cards[0].w, card.w);
}

#[test]
fn a_new_group_goes_next_to_the_focused_one_and_all_share_equally() {
    let mut t = panel();
    t.insert_group(1);
    assert_eq!(t.group_count(), 2);
    assert_eq!(t.group_at, 0);
    t.groups[0].share = 3.0;
    // 在正在用的一栏前面加：正在用的一栏后移一位，份额重新平分。
    t.insert_group(0);
    assert_eq!(t.group_at, 1);
    assert_eq!(t.panel_shares(), [1.0, 1.0, 1.0]);
    assert_eq!(ids(&t), [vec![], vec![1, 2], vec![]]);
}

#[test]
fn swapping_groups_keeps_each_groups_tabs_and_active_tab() {
    let mut t = panel();
    t.active = 1;
    t.insert_group(1);
    t.place_tab(1, tab(3), None);
    t.place_tab(1, tab(4), None);
    assert_eq!(ids(&t), [vec![1, 2], vec![3, 4]]);
    assert_eq!(t.tab().map(|tab| tab.id), Some(2));

    t.swap_group(1);
    assert_eq!(t.group_at, 1);
    assert_eq!(t.tab().map(|tab| tab.id), Some(4));
    assert_eq!(ids(&t), [vec![1, 2], vec![3, 4]]);
    t.swap_group(0);
    assert_eq!(t.tab().map(|tab| tab.id), Some(2));
    assert_eq!(t.tab_count(), 4);
    assert_eq!(t.group_of(3), Some(1));
    assert_eq!(t.group_of(9), None);
}

#[test]
fn an_emptied_focused_group_hands_focus_to_its_neighbour() {
    let mut t = panel();
    t.insert_group(1);
    t.place_tab(1, tab(3), None);
    t.insert_group(2);
    t.place_tab(2, tab(4), None);
    t.swap_group(1);
    t.tabs.clear();
    assert!(t.drop_empty_groups());
    // 改用前面一栏，空了的那一栏去掉，其余的位置跟着前移。
    assert_eq!(ids(&t), [vec![1, 2], vec![4]]);
    assert_eq!(t.group_at, 0);

    // 只剩一栏时不再分栏。
    t.swap_group(1);
    t.tabs.clear();
    assert!(t.drop_empty_groups());
    assert!(t.groups.is_empty());
    assert_eq!(ids(&t), [vec![1, 2]]);
}

#[test]
fn empty_groups_stay_while_tabs_are_waiting_to_be_created() {
    let mut t = panel();
    t.insert_group(1);
    t.swap_group(1);
    t.want.push(Vec::new());
    assert!(!t.drop_empty_groups());
    assert_eq!(t.group_count(), 2);
    t.want.clear();
    assert!(t.drop_empty_groups());
    assert_eq!(t.group_count(), 1);
}

#[test]
fn tabs_are_counted_across_groups_in_order() {
    let mut t = panel();
    t.place_tab(2, tab(3), None);
    assert_eq!(t.group_count(), 3);
    assert_eq!(ids(&t), [vec![1, 2], vec![], vec![3]]);
    t.focus_shell(2);
    assert_eq!(t.group_at, 2);
    assert_eq!(t.tab().map(|tab| tab.id), Some(3));
    t.focus_shell(1);
    assert_eq!((t.group_at, t.active), (0, 1));
    let all: Vec<u16> = t.all_tabs().map(|tab| tab.id).collect();
    assert_eq!(all, [1, 2, 3]);
}
