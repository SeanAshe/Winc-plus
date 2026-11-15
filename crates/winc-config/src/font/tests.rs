//! [`resolve_font`] 的测试：字体名与字体文件路径的区分。

use super::*;

#[test]
fn font_request_tells_names_from_paths() {
    assert_eq!(
        resolve_font(Some("Menlo"), "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
    assert_eq!(
        resolve_font(Some("Helvetica Neue"), "Menlo"),
        FontRequest::Name("Helvetica Neue".to_string())
    );
    assert_eq!(
        resolve_font(Some("/System/Library/Fonts/Menlo.ttc"), "Menlo"),
        FontRequest::Path(PathBuf::from("/System/Library/Fonts/Menlo.ttc"))
    );
    assert_eq!(
        resolve_font(Some("fonts/MyFont.ttf"), "Menlo"),
        FontRequest::Path(PathBuf::from("fonts/MyFont.ttf"))
    );
    assert_eq!(
        resolve_font(None, "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
    assert_eq!(
        resolve_font(Some("   "), "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
}

/// 字体列表以逗号分隔，按书写顺序返回；空白、引号和空项略去，各项仍区分字体名与路径。
#[test]
fn font_lists_are_split_on_commas() {
    let name = |n: &str| FontRequest::Name(n.to_string());
    assert_eq!(
        resolve_font_list(Some("JetBrains Mono, Noto Sans SC")),
        [name("JetBrains Mono"), name("Noto Sans SC")]
    );
    assert_eq!(
        resolve_font_list(Some(" 'Fira Code' ,, \"LXGW WenKai\", fonts/Emoji.ttf ,")),
        [
            name("Fira Code"),
            name("LXGW WenKai"),
            FontRequest::Path(PathBuf::from("fonts/Emoji.ttf"))
        ]
    );
    assert_eq!(resolve_font_list(Some("Menlo")), [name("Menlo")]);
    assert!(resolve_font_list(Some(" , ")).is_empty());
    assert!(resolve_font_list(None).is_empty());
}
