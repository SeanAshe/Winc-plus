//! 状态栏：右侧可点击的项（缩进、行尾、编码、分支、终端开关）和左侧的路径。

use super::*;

pub(super) mod breadcrumb;
pub(super) mod motion;
pub(super) mod status_bar;

/// 状态栏的界面状态：临时消息及其期限、可点击的区域、内容切换的过渡。
#[derive(Default)]
pub struct StatusState {
    /// 状态栏上的临时消息。
    pub(crate) message: Option<String>,
    pub(in crate::app) until: Option<Instant>,
    /// 状态栏右侧各项的过渡状态。
    pub(crate) motion: StatusMotion,
}
