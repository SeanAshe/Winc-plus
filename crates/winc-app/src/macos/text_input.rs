//! 保留输入法在按键时直接提交的文字。
//!
//! 输入法在没有组字时也可能提交文字，例如拼音输入法下按 `,` 提交 `，`，全角模式下按字母提交
//! 全角字母。这些文字经由内容视图的 `insertText:replacementRange:` 送达，此时没有标记文本；
//! `winit` 只在有标记文本时将其作为 `Ime::Commit` 派发，否则不予派发，随后的按键事件只携带
//! 按键本身的字符（`,`）。
//!
//! 本模块替换内容视图类的 `insertText:replacementRange:` 实现：按键过程中没有标记文本时保存
//! 提交的文字，再调用原实现；随后的按键事件由 `with_inserted_text` 替换为保存的文字。

use std::cell::RefCell;

use objc2::runtime::Bool;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::Key;

use super::files::rust_string;
use super::*;

/// `NSRange`（`<Foundation/NSRange.h>`）。
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRange {
    location: usize,
    length: usize,
}

type InsertText = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, NSRange);

/// 被替换前的 `insertText:replacementRange:` 实现。
static ORIGINAL: OnceLock<InsertText> = OnceLock::new();

thread_local! {
    /// 当前按键中输入法直接提交、尚未用于按键事件的文字。
    static INSERTED: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `NSEventTypeKeyDown`（`<AppKit/NSEvent.h>`）。
const KEY_DOWN: usize = 10;

/// 替换窗口内容视图类的 `insertText:replacementRange:`，保存输入法在按键时直接提交的文字。
///
/// 只替换一次；必须在窗口接收按键之前调用。
pub fn capture_inserted_text(window: &Window) {
    let Some(view) = ns_view(window) else {
        return;
    };
    let class = view.class();
    // SAFETY: `class` 是有效的类对象，`class_getInstanceMethod` 找不到方法时返回空指针，判空后
    // 再使用。`InsertText` 的签名与该方法的声明一致：无返回值，参数依次为 self、_cmd、文字对象
    // 和 `NSRange`；`InsertText` 与 `Imp` 同为 `extern "C-unwind"` 函数指针，大小相同。
    ORIGINAL.get_or_init(|| unsafe {
        let method = ffi::class_getInstanceMethod(class, sel!(insertText:replacementRange:));
        let imp = std::mem::transmute::<InsertText, Imp>(insert_text);
        let original = (!method.is_null())
            .then(|| ffi::method_setImplementation(method, imp))
            .flatten();
        match original {
            Some(original) => std::mem::transmute::<Imp, InsertText>(original),
            None => skip_insert_text,
        }
    });
}

/// 原实现不存在时使用的空实现。
extern "C-unwind" fn skip_insert_text(_: *mut AnyObject, _: Sel, _: *mut AnyObject, _: NSRange) {}

/// 实现 `- insertText:replacementRange:`：按键过程中没有标记文本时保存文字，再调用原实现。
extern "C-unwind" fn insert_text(
    this: *mut AnyObject,
    cmd: Sel,
    text: *mut AnyObject,
    range: NSRange,
) {
    // SAFETY: `AppKit` 在主线程调用本方法，`this` 是有效的内容视图，`text` 是 NSString 或
    // NSAttributedString；`currentEvent` 可能为空，判空后再使用；字符串由 `rust_string` 立即
    // 拷贝。
    unsafe {
        let marked: Bool = msg_send![this, hasMarkedText];
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let event: *mut AnyObject = msg_send![app, currentEvent];
        let key_down = event.as_ref().is_some_and(|event| {
            let kind: usize = msg_send![event, type];
            kind == KEY_DOWN
        });
        if !marked.as_bool()
            && key_down
            && let Some(text) = text.as_ref()
        {
            let attributed: Bool = msg_send![text, isKindOfClass: class!(NSAttributedString)];
            let string: *mut AnyObject = if attributed.as_bool() {
                msg_send![text, string]
            } else {
                (text as *const AnyObject).cast_mut()
            };
            let inserted = string.as_ref().and_then(|string| rust_string(string));
            INSERTED.with_borrow_mut(|slot| *slot = inserted);
        }
    }
    if let Some(original) = ORIGINAL.get() {
        original(this, cmd, text, range);
    }
}

/// 返回以输入法直接提交的文字替换后的按键事件 `event`。
///
/// 按下的键产生文字且与输入法提交的文字不同时，`text` 和 `logical_key` 替换为提交的文字；其他
/// 情况原样返回。每次按下都取出保存的文字，不保留到下一次按键。
pub fn with_inserted_text(mut event: KeyEvent) -> KeyEvent {
    if event.state != ElementState::Pressed {
        return event;
    }
    let inserted = INSERTED.with_borrow_mut(Option::take);
    if let Some(inserted) = inserted
        && let Some(text) = &event.text
        && text.as_str() != inserted
        && !inserted.chars().any(char::is_control)
    {
        let text = winit::keyboard::SmolStr::new(inserted);
        event.logical_key = Key::Character(text.clone());
        event.text = Some(text);
    }
    event
}
