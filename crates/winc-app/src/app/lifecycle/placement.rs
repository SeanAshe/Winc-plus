//! 窗口的大小和位置：关闭时记下，建窗口时按当前的显示器校验以后沿用。

use winc_config::WindowFrame;
use winit::dpi::{LogicalPosition, PhysicalPosition, Position};
use winit::event_loop::ActiveEventLoop;

use crate::app::App;

/// 窗口内容区的默认宽度（逻辑像素）。
pub(super) const DEFAULT_WIDTH: f64 = 1280.0;
/// 窗口内容区的默认高度（逻辑像素）。
pub(super) const DEFAULT_HEIGHT: f64 = 820.0;
/// 窗口内容区的最小宽度（逻辑像素）。
pub(in crate::app) const MIN_WIDTH: f64 = 640.0;
/// 窗口内容区的最小高度（逻辑像素）。
pub(in crate::app) const MIN_HEIGHT: f64 = 420.0;
/// 窗口至少要有这么多留在显示器内（逻辑像素），用户才能把它拖回来。
///
/// 取 128，与主流编辑器恢复窗口时的边距一致。
const VISIBLE_MARGIN: f64 = 128.0;

/// 一台显示器。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Screen {
    /// 左上角的横坐标（物理像素）。
    pub x: f64,
    /// 左上角的纵坐标（物理像素）。
    pub y: f64,
    /// 宽度（物理像素）。
    pub width: f64,
    /// 高度（物理像素）。
    pub height: f64,
    /// 缩放比例。
    pub scale: f64,
    /// 可用区域（显示器减去任务栏等系统保留区域）的宽度和高度（物理像素）；只在 Windows 上
    /// 读取，其他系统为 `None`。
    pub work: Option<(f64, f64)>,
}

/// 校验之后的窗口大小和位置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Placement {
    /// 内容区的宽度（逻辑像素）。
    pub width: f64,
    /// 内容区的高度（逻辑像素）。
    pub height: f64,
    /// 外框左上角的（横，纵）坐标（物理像素）；不恢复位置时为 `None`，由系统决定。
    pub position: Option<(f64, f64)>,
}

/// 按当前的显示器校验 `frame`。
///
/// 显示器拔掉、分辨率变小以后，窗口的顶边或者足够宽的一段不在任何显示器上时不恢复位置；大小
/// 不超过窗口所在的显示器（不恢复位置时是第一台），也不小于窗口的最小尺寸。
///
/// 记录的大小盖满所在显示器的可用区域（见 [`covers`]）时，大小和位置都不恢复，使用默认大小。
pub(super) fn place(frame: &WindowFrame, screens: &[Screen]) -> Placement {
    let margin = VISIBLE_MARGIN * frame.scale;
    let right = |x: f64| x + frame.width * frame.scale;
    let reachable = |screen: &&Screen, (x, y): (f64, f64)| {
        let overlap = right(x).min(screen.x + screen.width) - x.max(screen.x);
        overlap >= margin && y >= screen.y && y <= screen.y + screen.height - margin
    };
    let found = frame
        .position
        .and_then(|at| Some((screens.iter().find(|s| reachable(s, at))?, at)));
    let screen = found.map(|(screen, _)| screen).or(screens.first());
    let size = (frame.width * frame.scale, frame.height * frame.scale);
    let (mut width, mut height, position) = if covers(size, screen.and_then(|s| s.work)) {
        (DEFAULT_WIDTH, DEFAULT_HEIGHT, None)
    } else {
        (frame.width, frame.height, found.map(|(_, at)| at))
    };
    if let Some(screen) = screen {
        width = width.min(screen.width / screen.scale);
        height = height.min(screen.height / screen.scale);
    }
    Placement {
        width: width.max(MIN_WIDTH),
        height: height.max(MIN_HEIGHT),
        position,
    }
}

/// 是否盖满可用区域：宽 `width`、高 `height` 均不小于可用区域 `work` 的宽、高（物理像素）；
/// 可用区域未知时为否。
///
/// 最大化的窗口占满可用区域。未最大化的窗口使用这样的大小时与最大化的窗口无法区分，超出可用
/// 区域的部分被任务栏遮挡，因此这样的大小既不记录，也不恢复。
fn covers((width, height): (f64, f64), work: Option<(f64, f64)>) -> bool {
    work.is_some_and(|(w, h)| width >= w && height >= h)
}

/// 新窗口相对打开它的窗口往右下错开的距离（逻辑像素）：与 macOS 标题栏的高度接近，层叠的
/// 各窗口露出标题栏。
const CASCADE: f64 = 28.0;

/// 返回从外框左上角位于 `at`（物理像素）、缩放比例为 `scale` 的窗口打开的新窗口的外框左上角
/// 位置（物理像素）：往右下错开 [`CASCADE`]。
pub(super) fn cascaded((x, y): (f64, f64), scale: f64) -> (f64, f64) {
    (x + CASCADE * scale, y + CASCADE * scale)
}

/// 物理像素的坐标换成建窗口用的位置。
///
/// macOS 的坐标按各显示器自己的缩放比例换算，建窗口时不知道窗口会落在哪一台，用记录时的比例
/// 换回逻辑坐标；其他系统的物理坐标在所有显示器之间是连续的，直接使用。
pub(super) fn window_position((x, y): (f64, f64), scale: f64) -> Position {
    if cfg!(target_os = "macos") {
        LogicalPosition::new(x / scale, y / scale).into()
    } else {
        PhysicalPosition::new(x, y).into()
    }
}

/// 返回显示器 `monitor` 可用区域的宽度和高度（物理像素）；只在 Windows 上读取。
fn work_size(monitor: &winit::monitor::MonitorHandle) -> Option<(f64, f64)> {
    #[cfg(windows)]
    {
        use winit::platform::windows::MonitorHandleExtWindows;
        crate::windows::work_size(monitor.hmonitor())
    }
    #[cfg(not(windows))]
    {
        let _ = monitor;
        None
    }
}

/// 当前连接的显示器。
pub(super) fn screens(event_loop: &ActiveEventLoop) -> Vec<Screen> {
    event_loop
        .available_monitors()
        .map(|monitor| {
            let (at, size) = (monitor.position(), monitor.size());
            Screen {
                x: f64::from(at.x),
                y: f64::from(at.y),
                width: f64::from(size.width),
                height: f64::from(size.height),
                scale: monitor.scale_factor(),
                work: work_size(&monitor),
            }
        })
        .collect()
}

impl App {
    /// 窗口大小或位置变了：没有最大化、全屏、最小化时记下来，还原后回到这个大小。
    ///
    /// 盖满所在显示器可用区域的大小（见 [`covers`]）不记录，保留此前记录的大小。
    pub(super) fn note_window_frame(&mut self) {
        let Some(window) = &self.window.handle else {
            return;
        };
        if self.window.hidden
            || window.is_minimized() == Some(true)
            || window.is_maximized()
            || window.fullscreen().is_some()
        {
            return;
        }
        let inner = window.inner_size();
        let work = window.current_monitor().and_then(|m| work_size(&m));
        if covers((f64::from(inner.width), f64::from(inner.height)), work) {
            return;
        }
        let scale = window.scale_factor();
        let size = inner.to_logical::<f64>(scale);
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        let at = window.outer_position().ok();
        self.window.frame = Some(WindowFrame {
            position: at.map(|at| (f64::from(at.x), f64::from(at.y))),
            width: size.width,
            height: size.height,
            scale,
            maximized: false,
        });
    }

    /// 窗口要关了：记下它的大小和位置；有多个窗口时最后关闭的算数。
    pub(super) fn remember_window(&mut self) {
        if self.config.restore_window == Some(false) || self.window.hidden {
            return;
        }
        let (Some(window), Some(frame)) = (&self.window.handle, self.window.frame) else {
            return;
        };
        let frame = WindowFrame {
            maximized: window.is_maximized(),
            ..frame
        };
        self.update_state(|state| state.window = Some(frame));
    }
}

#[cfg(test)]
mod tests;
