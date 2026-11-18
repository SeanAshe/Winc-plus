//! 键位表中的一条绑定。

use super::*;

/// 一条绑定：在 `when` 范围内按下该键（或该组合键）执行 `action`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// 要执行的命令。
    pub action: Action,
    /// 第一段按键。
    pub first: Stroke,
    /// 组合键的第二段；单按即可触发时为 `None`。
    pub second: Option<Stroke>,
    /// 生效范围。
    pub when: When,
}

impl Binding {
    /// 返回绑定在 `os` 上的显示文字；两段组合键的两段之间用空格连接。
    pub fn label(&self, os: Os) -> String {
        match self.second {
            // 连按两下同一个修饰键：macOS 上写作 `⇧⇧`，其他系统上写作 `Shift Shift`。
            Some(second) if os == Os::Mac && self.first.key == KeyCode::Tap => {
                self.first.label(os) + &second.label(os)
            }
            Some(second) => format!("{} {}", self.first.label(os), second.label(os)),
            None => self.first.label(os),
        }
    }
}
