//! 名字旁边的提示气泡：转到定义没有找到定义、未找到其他用法、光标处没有可以查找的名字时，
//! 在光标所在的名字上方弹出一句话，而不是写在状态栏里。
//!
//! 气泡挂在文档中的位置上，随文档滚动；停留 [`HINT_DURATION`] 后淡出，按键、点击、滚动或
//! 打开浮窗时立即淡出。

use std::time::{Duration, Instant};

use winc_core::movement;
use winc_editor::DocumentId;

use super::*;

/// 提示气泡停留的时长（经验值）：够读完一句话。
const HINT_DURATION: Duration = Duration::from_millis(3000);

/// 名字旁边的提示气泡。
pub struct NameHint {
    /// 提示文字。
    pub text: String,
    /// 气泡挂在哪个文档上。
    pub doc: DocumentId,
    /// 气泡挂在文档中的位置（字符偏移）：光标所在名字的开头。
    pub anchor: usize,
    /// 到这一刻开始淡出。
    until: Instant,
    /// 出现的动画；结束后为 `None`。
    pub fade_in: Option<Clock>,
    /// 消失的动画；还没开始消失时为 `None`。
    pub fade_out: Option<Clock>,
}

impl App {
    /// 在光标所在的名字上方弹出提示 `text`；没有打开文档时改为写在状态栏。
    pub(super) fn hint_at_name(&mut self, text: impl Into<String>) {
        let text = text.into();
        let Some(doc) = self.editor.active() else {
            self.flash(text);
            return;
        };
        let anchor = self.name_span().map_or_else(
            || movement::word_at(doc.text(), doc.selection.head).0,
            |s| s.0,
        );
        let id = doc.id;
        let fade_in = self.clock(POPUP_IN);
        self.name_hint = Some(NameHint {
            text,
            doc: id,
            anchor,
            until: Instant::now() + HINT_DURATION,
            fade_in,
            fade_out: None,
        });
        self.request_redraw();
    }

    /// 让提示气泡淡出；没有窗口（测试中）或动态效果关闭时直接去掉。
    pub(in crate::app) fn dismiss_hint(&mut self) {
        let Some(hint) = &self.name_hint else {
            return;
        };
        if hint.fade_out.is_some() {
            return;
        }
        match self.clock(POPUP_OUT) {
            Some(clock) => {
                if let Some(hint) = &mut self.name_hint {
                    hint.fade_out = Some(clock);
                }
            }
            None => self.name_hint = None,
        }
        self.request_redraw();
    }

    /// 停留时间到了就开始淡出；返回下一次需要检查的时刻。
    pub(in crate::app) fn hint_tick(&mut self, now: Instant) -> Option<Instant> {
        let hint = self.name_hint.as_ref()?;
        if hint.fade_out.is_some() {
            return None;
        }
        if now >= hint.until {
            self.dismiss_hint();
            None
        } else {
            Some(hint.until)
        }
    }

    /// 清除已经结束的出现、消失动画；消失结束后去掉气泡。
    pub(in crate::app) fn settle_hint(&mut self) {
        let Some(hint) = &mut self.name_hint else {
            return;
        };
        if hint.fade_in.is_some_and(|c| c.done()) {
            hint.fade_in = None;
        }
        if hint.fade_out.is_some_and(|c| c.done()) {
            self.name_hint = None;
        }
    }

    /// 提示气泡是否在动画中。
    pub(in crate::app) fn hint_animating(&self) -> bool {
        self.name_hint
            .as_ref()
            .is_some_and(|h| h.fade_in.is_some() || h.fade_out.is_some())
    }
}
