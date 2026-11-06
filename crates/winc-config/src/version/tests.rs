//! `version` 的测试：预发布版本与旧格式版本号的排序稳定。

use super::*;

#[test]
fn prereleases_and_legacy_versions_have_stable_order() {
    let ordered = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.0.1",
    ];
    for pair in ordered.windows(2) {
        assert_eq!(
            compare_versions(pair[0], pair[1]),
            Ordering::Less,
            "{pair:?}"
        );
        assert_eq!(compare_versions(pair[1], pair[0]), Ordering::Greater);
    }
    assert_eq!(
        compare_versions("1.0.0+build.1", "1.0.0+build.2"),
        Ordering::Equal
    );
    assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
    assert_eq!(compare_versions("0.1.368.1", "0.1.368"), Ordering::Greater);
    assert_eq!(compare_versions("0.1.502_116", "0.1.502.116"), Ordering::Equal);
    assert_eq!(compare_versions("0.1.502_116", "0.1.502_115"), Ordering::Greater);
    assert_eq!(compare_versions("0.1.503_1", "0.1.502_999"), Ordering::Greater);
    assert_eq!(compare_versions("0.1.502_116", "0.1.495"), Ordering::Greater);
}
