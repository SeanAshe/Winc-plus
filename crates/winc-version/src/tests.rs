use super::VERSION;

#[test]
fn version_is_three_numbers_with_an_optional_suffix() {
    let core = VERSION.split(['-', '+']).next().unwrap();
    let parts: Vec<&str> = core.split('.').collect();
    assert_eq!(parts.len(), 3, "{VERSION}");
    for part in parts {
        assert!(
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()),
            "{VERSION}"
        );
    }
}
