use super::*;

/// 贴边拆分的一条按窗格大小取一成，大窗格不超过上限，小窗格不小于下限，且不超过窗格的一半。
#[test]
fn the_split_edge_is_a_narrow_band() {
    assert_eq!(drop_edge(2000.0), DROP_EDGE_MAX);
    assert_eq!(drop_edge(600.0), 60.0);
    assert_eq!(drop_edge(200.0), DROP_EDGE_MIN);
    assert_eq!(drop_edge(40.0), 20.0);
}
