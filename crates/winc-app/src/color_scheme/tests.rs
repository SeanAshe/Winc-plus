//! `color_scheme` 的测试：门户的 `color-scheme` 取值对应的外观。

use super::*;

#[test]
fn only_a_preference_for_dark_is_dark() {
    assert_eq!(theme_of(&Value::U32(0)), Some(Theme::Light));
    assert_eq!(theme_of(&Value::U32(1)), Some(Theme::Dark));
    assert_eq!(theme_of(&Value::U32(2)), Some(Theme::Light));
    let read = Value::Value(Box::new(Value::U32(1)));
    assert_eq!(theme_of(&read), Some(Theme::Dark));
    assert_eq!(theme_of(&Value::from("prefer-dark")), None);
}
