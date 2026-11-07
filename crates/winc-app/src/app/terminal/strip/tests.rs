//! 标签栏的测试：标签放不下时从最宽的开始变窄，且不窄于最小宽度。

use super::*;

#[test]
fn crowded_tabs_shrink_the_widest_first() {
    let mut widths = [120.0, 200.0, 120.0];
    fit_tabs(&mut widths, 1000.0);
    assert_eq!(widths, [120.0, 200.0, 120.0]);
    // 放 400：宽的先窄到和别的一样，再一起窄。
    fit_tabs(&mut widths, 400.0 + 2.0 * TAB_GAP);
    assert_eq!(widths, [120.0, 160.0, 120.0]);
    fit_tabs(&mut widths, 300.0 + 2.0 * TAB_GAP);
    assert_eq!(widths, [100.0, 100.0, 100.0]);
    // 再窄也不窄过最窄的宽度。
    fit_tabs(&mut widths, 30.0);
    assert_eq!(widths, [TAB_SQUEEZED; 3]);
}

#[test]
fn tabs_keep_their_minimum_width_when_there_is_room() {
    let widths = fit_named_tabs(&[60.0, 300.0], 1000.0);
    assert_eq!(widths, [TAB_MIN, TAB_MAX]);
}

#[test]
fn blank_space_goes_before_any_name_is_cut() {
    // 放不下三个最小宽度，但名字都放得下：名字短的让出留白，长名字保持完整。
    let room = 300.0 + 2.0 * TAB_GAP;
    let widths = fit_named_tabs(&[80.0, 80.0, 118.0], room);
    assert!(widths.iter().sum::<f32>() <= 300.0);
    assert!(widths[2] >= 118.0);
    assert!(widths[0] >= 80.0 && widths[0] < TAB_MIN);
    assert_eq!(widths[0], widths[1]);
}

#[test]
fn only_the_longest_names_are_cut_when_blank_space_is_not_enough() {
    let room = 240.0 + TAB_GAP;
    let widths = fit_named_tabs(&[80.0, 200.0], room);
    assert_eq!(widths, [80.0, 160.0]);
}

#[test]
fn short_names_count_with_the_narrowest_tab_width() {
    // 短名字的标签不窄于 `TAB_SQUEEZED`：按该宽度计算空间，标签栏不会多滚出一段。
    let room = 400.0 + 4.0 * TAB_GAP;
    let widths = fit_named_tabs(&[40.0, 40.0, 40.0, 300.0, 300.0], room);
    assert!(widths.iter().sum::<f32>() <= 400.0, "{widths:?}");
    assert_eq!(widths[0], TAB_SQUEEZED);
}

#[test]
fn narrowed_tabs_ignore_the_rounding_of_their_widths() {
    // 名字短的按 `TAB_MIN` 算，取整少的不足 1 不算变窄。
    assert!(!tabs_narrowed(&[50.0, 130.5], &[TAB_MIN, 130.0]));
    assert!(tabs_narrowed(&[50.0, 50.0], &[TAB_MIN, 100.0]));
    // 名字本身超过上限时按上限算。
    assert!(!tabs_narrowed(&[500.0], &[TAB_MAX]));
    assert!(tabs_narrowed(&[500.0], &[TAB_SQUEEZED]));
}
