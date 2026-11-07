//! 其他平台的呈现：直接画进 `softbuffer` 的窗口缓冲并呈现。
//!
//! 窗口缓冲与画面一样大（4K 画面约 32 MB）。停止绘制 [`RELEASE_IDLE`] 后释放，下一帧重建：
//! 呈现时窗口系统已复制画面，释放后窗口仍显示最后一帧，需要重画时再重建。Wayland 上不释放：
//! 合成器在发出 `wl_buffer.release` 之前仍可能读取已提交的缓冲。

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::draw::canvas::Canvas;

/// 停止绘制后释放窗口缓冲之前等待的时长。
///
/// 长于光标闪烁的间隔（530 ms）：闪烁期间每次闪烁都会绘制，缓冲不会反复释放和重建。取自调参：
/// 停止操作约 2 s 后释放；重建的代价是下一帧多一次与画面同样大的分配。
const RELEASE_IDLE: Duration = Duration::from_secs(2);

/// 软件缓冲呈现器：持有窗口的 `softbuffer` 表面，空闲时释放。
pub(crate) struct Presenter {
    /// 重建表面所用的 `softbuffer` 上下文。
    context: softbuffer::Context<Arc<Window>>,
    /// 呈现目标窗口。
    window: Arc<Window>,
    /// 窗口对应的 `softbuffer` 表面，缓冲里每个像素为 `0xAARRGGBB`；空闲时释放为 `None`。
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// 窗口客户区宽度（物理像素），不小于 1。
    width: u32,
    /// 窗口客户区高度（物理像素），不小于 1。
    height: u32,
    /// 最近一次呈现的时刻；表面已释放或尚未呈现时为 `None`。
    presented: Option<Instant>,
    /// 空闲时能否释放表面：Wayland 上为 `false`（见模块文档）。
    releasable: bool,
}

impl Presenter {
    /// 创建呈现器，呈现目标是 `window`。
    ///
    /// # Errors
    ///
    /// 创建 `softbuffer` 上下文或表面失败时返回错误描述。
    pub(crate) fn new(window: &Arc<Window>) -> Result<Self, String> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        let wayland = window
            .window_handle()
            .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)));
        let mut presenter = Self {
            context,
            window: window.clone(),
            surface: Some(surface),
            width: 0,
            height: 0,
            presented: None,
            releasable: !wayland,
        };
        let size = window.inner_size();
        presenter.resize(size.width, size.height, window.scale_factor());
        Ok(presenter)
    }

    /// 更新窗口尺寸（物理像素，小于 1 的按 1 处理）；参数 `_scale` 不使用。
    ///
    /// 表面已释放时只记下尺寸，重建时使用。
    pub(crate) fn resize(&mut self, width: u32, height: u32, _scale: f64) {
        self.width = width.max(1);
        self.height = height.max(1);
        if let Some(surface) = &mut self.surface {
            let _ = surface.resize(nonzero(self.width), nonzero(self.height));
        }
    }

    /// 返回当前画面尺寸（物理像素）。
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 把 `paint` 画进 `softbuffer` 的窗口缓冲并呈现；表面已释放时先重建，取不到缓冲时跳过
    /// 本帧。
    ///
    /// 总是返回 `true`：这里不等待缓冲，取不到时在下一次屏幕刷新时重试也取不到。`paint` 直接
    /// 画进窗口缓冲（行距等于画面宽度），不另备一块同样大的像素缓冲再整块拷贝。
    pub(crate) fn frame(&mut self, paint: impl FnOnce(Canvas<'_>)) -> bool {
        let (width, height) = (self.width as usize, self.height as usize);
        if self.surface.is_none() {
            let Ok(mut surface) = softbuffer::Surface::new(&self.context, self.window.clone())
            else {
                return true;
            };
            let _ = surface.resize(nonzero(self.width), nonzero(self.height));
            self.surface = Some(surface);
        }
        let Some(Ok(mut buffer)) = self.surface.as_mut().map(|s| s.buffer_mut()) else {
            return true;
        };
        // 调整尺寸失败时缓冲还是原来的大小，装不下这一帧，跳过。
        if buffer.len() < width * height {
            return true;
        }
        paint(Canvas {
            px: &mut buffer[..],
            width,
            height,
            stride: width,
        });
        let _ = buffer.present();
        self.presented = Some(Instant::now());
        true
    }

    /// 返回应当释放空闲的窗口缓冲的时刻：最近一次呈现之后 [`RELEASE_IDLE`]；表面已释放、
    /// 尚未呈现或不能释放时为 `None`。
    pub(crate) fn purgeable_at(&self) -> Option<Instant> {
        let held = self.releasable && self.surface.is_some();
        self.presented.filter(|_| held).map(|at| at + RELEASE_IDLE)
    }

    /// 停止绘制超过 [`RELEASE_IDLE`] 后释放窗口缓冲；下一帧重建。
    pub(crate) fn make_idle_purgeable(&mut self) {
        if self.purgeable_at().is_some_and(|at| Instant::now() >= at) {
            self.surface = None;
            self.presented = None;
        }
    }

    /// 设置窗口是否在前台；`softbuffer` 的缓冲不区分前后台，什么也不做。
    pub(crate) fn set_foreground(&mut self, _foreground: bool) {}

    /// 设置窗口是否完全看不见；空闲的缓冲由 [`Self::make_idle_purgeable`] 释放，此处不做处理。
    pub(crate) fn set_occluded(&mut self, _occluded: bool) {}
}

/// 把不小于 1 的尺寸转为 `NonZeroU32`。
fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("调用方已保证不为 0")
}
