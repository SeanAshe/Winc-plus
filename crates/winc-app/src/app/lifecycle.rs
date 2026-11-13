//! 应用和窗口的生命周期。
//!
//! 启动、事件循环、窗口和绘制、外观、提示条、关闭和退出、热退出的备份、临时打开模式。

use super::*;

pub(super) mod alert;
pub(super) mod appearance;
pub(super) mod backup;
pub(super) mod closing;
pub(super) mod events;
#[cfg(target_os = "macos")]
mod install;
pub(super) mod notice;
pub(super) mod one_off;
pub(super) mod placement;
mod shell_command;
pub(super) mod startup;
pub(super) mod title_bar;
pub(super) mod wallpaper;
pub(super) mod window;

/// 帧的绘制与呈现：渲染器、下一帧的安排和测量。
pub(crate) struct FrameState {
    /// 渲染器；尚未创建时为 `None`。
    pub(in crate::app) renderer: Option<Renderer>,
    /// 动画下一帧的绘制时刻（没有显示链接时按计时器安排）。
    pub(in crate::app) next_frame: Option<Instant>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(target_os = "macos")]
    pub(in crate::app) display_link: Option<crate::macos::DisplayLink>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(windows)]
    pub(in crate::app) display_link: Option<crate::windows::DisplayLink>,
    /// 相邻两帧的间隔；动画开始时按窗口所在显示器的刷新率计算。
    pub(in crate::app) frame_interval: Duration,
    /// 测量钩子的状态。
    pub(in crate::app) bench: Bench,
    /// 帧日志（`WINC_FRAME_LOG`）。
    frame_log: crate::frame_log::FrameLog,
    /// 已呈现的帧数，供测量钩子使用。
    presented: usize,
    /// 是否需要重新绘制。
    pub(in crate::app) dirty: bool,
}

/// 各字体的行高、基线等尺寸和字符格的宽度（逻辑像素），随字体设置和缩放比例更新。
pub(crate) struct TextMetrics {
    /// 界面字体的行高、基线等尺寸（逻辑像素）。
    pub ui: Metrics,
    /// 等宽字体的行高、基线等尺寸（逻辑像素）。
    pub mono: Metrics,
    /// 等宽字体单个字符格的宽度（逻辑像素）。
    pub cell: f32,
    /// 终端字体的行高、基线等尺寸（逻辑像素）。
    pub terminal: Metrics,
    /// 终端里一个字符格的宽度（逻辑像素）。
    pub terminal_cell: f32,
}

/// 创建窗口时使用一次的参数：位置、是否全屏、共用的字形缓存和激活令牌。
pub(crate) struct Opening {
    /// 新窗口的位置（外框左上角，物理像素）与来源窗口的缩放比例；新窗口相对来源窗口向右下错开。
    ///
    /// 创建窗口时按当前显示器校验后使用一次，见 `App::cascade_from`。
    pub(crate) cascade: Option<((f64, f64), f64)>,
    /// 窗口创建并显示后是否进入全屏（来源窗口处于全屏时），使用一次。
    pub(crate) fullscreen: bool,
    /// 新窗口与来源窗口共用的字体和字形缓存（创建渲染器时使用一次）。
    pub(crate) glyphs_from: Option<winc_render::SharedGlyphs>,
    /// 待使用的激活令牌（见 [`App::set_activation`]）。
    #[cfg(all(unix, not(target_os = "macos")))]
    activation: Option<winit::window::ActivationToken>,
}

/// 本窗口与应用层的联系：事件代理、窗口编号和各窗口共用的状态。
pub(crate) struct AppLink {
    /// 后台线程和系统回调通过该代理把事件送回事件循环（附带本窗口的编号，见 [`crate::winc`]）；
    /// 测试中没有事件循环，为 `None`。
    pub(in crate::app) proxy: Option<crate::winc::WindowProxy>,
    /// 窗口的红绿灯（见 [`crate::macos::TrafficLights`]），窗口创建后才有，随窗口一同释放。
    #[cfg(target_os = "macos")]
    traffic_lights: Option<crate::macos::TrafficLights>,
    /// 本窗口在进程内的编号；测试中单独创建的 `App` 为 `None`。
    pub(crate) key: Option<crate::winc::WindowKey>,
    /// 各窗口共用的状态：其他窗口打开的内容、剪切的文件，以及请求应用层执行的操作。
    pub(crate) shared: std::rc::Rc<crate::winc::Shared>,
}
