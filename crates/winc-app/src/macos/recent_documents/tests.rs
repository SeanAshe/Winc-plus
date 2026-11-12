//! 最近文档列表的测试：发出的消息与 `AppKit` 的声明一致。

use super::super::tests::method_encoding as encoding;
use super::*;

#[test]
fn recent_document_messages_match_appkit() {
    let controller = c"NSDocumentController";
    assert_eq!(
        encoding(controller, sel!(sharedDocumentController), false),
        "@@:"
    );
    assert_eq!(
        encoding(controller, sel!(clearRecentDocuments:), true),
        "v@:@"
    );
    assert_eq!(
        encoding(controller, sel!(noteNewRecentDocumentURL:), true),
        "v@:@"
    );
}
