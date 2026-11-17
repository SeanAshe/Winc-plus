//! macOS 呈现：画面直接画进 `IOSurface`，作为窗口子层的内容。
//!
//! 画面由 CPU 画进 `IOSurface`，设为窗口视图下一个 `CALayer` 子层的 `contents`，由窗口合成器
//! 直接读取，不需要额外拷贝。

#![allow(unsafe_code)] // IOSurface 的裸指针和 Objective-C 消息发送

use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send};
use objc2_core_foundation::{
    CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
};
use objc2_core_graphics::kCGColorSpaceSRGB;
use objc2_io_surface::{
    IOSurfaceLockOptions, IOSurfacePurgeabilityState, IOSurfaceRef, kIOSurfaceBytesPerElement,
    kIOSurfaceColorSpace, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
};
use objc2_quartz_core::{CALayer, CATransaction, kCAGravityTopLeft};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::draw::canvas::Canvas;

/// 像素格式 `kCVPixelFormatType_32BGRA`（四字符码 `BGRA`），每像素 4 字节。
///
/// 小端机器上，一个像素的内存布局与 `0xAARRGGBB` 的 `u32` 一致。
const PIXEL_FORMAT_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// 呈现缓冲（surface）的最大块数（块）。
///
/// 合成器占用屏幕上显示的一块，刚交出的一块要到下一次垂直同步才换上去，而绘制按刷新间隔排帧，
/// 不与垂直同步对齐。只有两块时，动画中约三分之一的帧两块都被占用（经验值），此时只能把画面
/// 画进合成器正读的那块，可能读到画了一半的画面，整个窗口闪烁。取三块避免该冲突。
const MAX_SURFACES: usize = 3;

/// 全部 surface 都被占用时等待空闲的最长时间（毫秒，4 ms）。
///
/// 一次刷新里连画几帧时，合成器通常在 4 ms 内放开一块（滚动测量中最长 3–4 ms）；等不到就跳过
/// 这一帧（[`Self::frame`] 返回 `false`），下一次屏幕刷新再画。等满更长时间后改用最久未用的一块
/// 并不划算：界面线程要多阻塞两帧以上，而且画进合成器占着的那块时可能读到画了一半的画面。
const SURFACE_WAIT: Duration = Duration::from_millis(4);

/// 停止绘制多久后把不在屏幕上的 surface 标成可清除（毫秒，经验值）。
///
/// 可清除（purgeable volatile）的 surface 不计入进程内存，系统内存紧张时可以直接收回；内容被收回
/// 也没关系，每帧先铺满底色、整个重画。该值取 100 ms：长于 60 Hz 下的帧间隔（16.7 ms），动画
/// 中途不会来回切换；短于光标闪烁的间隔（530 ms）。
const PURGEABLE_IDLE: Duration = Duration::from_millis(100);

/// 停止绘制多久后把备用的 surface 标成可清除（2 s）：长于光标闪烁的间隔（530 ms）。
const SPARE_IDLE: Duration = Duration::from_secs(2);

/// 一块 `IOSurface` 及其行距。
struct Surface {
    /// `IOSurface` 对象。
    surface: CFRetained<IOSurfaceRef>,
    /// 一行的像素数，等于每行字节数除以 4。
    ///
    /// `IOSurface` 会把行宽对齐，因此可能大于画面宽度。
    stride: usize,
    /// 是否已标成可清除：不计入进程内存，系统可以收回内容。
    purgeable: bool,
}

impl Surface {
    /// 创建 `width` × `height`（物理像素）的 32 位 BGRA `IOSurface`。
    ///
    /// # Panics
    ///
    /// `IOSurfaceCreate` 失败时 panic。
    fn new(width: u32, height: u32) -> Self {
        let values =
            [width as i32, height as i32, 4, PIXEL_FORMAT_BGRA as i32].map(CFNumber::new_i32);
        // SAFETY: 这些是 IOSurface.framework 导出的全局常量字符串，在进程生命周期内有效，
        // 读取这些常量不涉及可变状态。
        let keys: [&CFString; 4] = unsafe {
            [
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfacePixelFormat,
            ]
        };
        let values: [&CFType; 4] = [&values[0], &values[1], &values[2], &values[3]];
        let properties = CFDictionary::from_slices(&keys, &values);
        // SAFETY: 属性字典由 `CFString` 键和 `CFNumber` 值组成，包含宽、高、每元素字节数和
        // 像素格式，符合 `IOSurfaceCreate` 的要求；字典在调用期间保持存活。
        let surface =
            unsafe { IOSurfaceRef::new(properties.as_opaque()) }.expect("IOSurfaceCreate 失败");
        // 标明像素是 sRGB。不标时合成器把这些数值当成显示器自己的色彩空间，P3 广色域屏上
        // 显示的颜色比配色里写的更艳（`#3574F0` 显示出来接近 `#0E75F9`）。

        // SAFETY: 两个都是系统框架导出的全局常量字符串；键和值都是 CFString，符合
        // `IOSurfaceSetValue` 的要求。
        unsafe { surface.set_value(kIOSurfaceColorSpace, kCGColorSpaceSRGB) };
        let stride = surface.bytes_per_row() / 4;
        Self {
            surface,
            stride,
            purgeable: false,
        }
    }

    /// 标成可清除或不可清除，返回标回不可清除时内容是否已被系统收回；已是该状态时不做处理。
    fn set_purgeable(&mut self, purgeable: bool) -> bool {
        if self.purgeable == purgeable {
            return false;
        }
        let state = if purgeable {
            IOSurfacePurgeabilityState::PurgeableVolatile
        } else {
            IOSurfacePurgeabilityState::PurgeableNonVolatile
        };
        let mut old = 0u32;
        // SAFETY: `surface` 是有效的 IOSurface；`old` 指向有效的 `u32`，调用返回后不再使用。
        unsafe { self.surface.set_purgeable(state.0, &mut old) };
        self.purgeable = purgeable;
        !purgeable && old == IOSurfacePurgeabilityState::PurgeableEmpty.0
    }

    /// 是否仍被合成器占用。
    fn in_use(&self) -> bool {
        self.surface.is_in_use()
    }
}

/// macOS 的呈现器：持有挂在窗口视图下的子层和一组 surface。
pub(crate) struct Presenter {
    /// 显示画面的专用子层。
    layer: Retained<CALayer>,
    /// 已创建的 surface，数量不超过 [`MAX_SURFACES`]；窗口尺寸变化后清空。
    ///
    /// 不再需要的 surface 不释放，空闲时标成可清除（见 [`PURGEABLE_IDLE`]）：每块约 16 MB，
    /// 新建并清零约 1.8 ms，隔几秒一次的动画会让内存来回跳。
    surfaces: Vec<Surface>,
    /// 下一次优先考虑的 surface 下标，按轮转递增。
    next: usize,
    /// 画面宽度（物理像素），不小于 1。
    width: u32,
    /// 画面高度（物理像素），不小于 1。
    height: u32,
    /// 屏幕上显示着的 surface 的下标，它一直被合成器占用，不标成可清除。
    shown: Option<usize>,
    /// 备用的 surface（上一帧显示的那块）的下标，下一帧优先使用。
    spare: Option<usize>,
    /// 从这时起没有再绘制（或者上次没能把空闲的都标成可清除），过 [`PURGEABLE_IDLE`] 再标。
    idle_since: Option<Instant>,
    /// 取用时内容已被系统收回的 surface 的块数。
    reclaimed: u32,
}

impl Presenter {
    /// 创建呈现器：在窗口的视图上挂一个专用子层，之后的画面都显示在该子层上。
    ///
    /// # Errors
    ///
    /// 不在主线程、窗口句柄不是 `AppKit` 句柄，或视图没有 `layer` 时返回错误描述。
    pub(crate) fn new(window: &Window) -> Result<Self, String> {
        MainThreadMarker::new().ok_or("必须在主线程创建")?;

        let RawWindowHandle::AppKit(handle) =
            window.window_handle().map_err(|e| e.to_string())?.as_raw()
        else {
            return Err("macOS 上需要 AppKit 窗口句柄".into());
        };

        // SAFETY: `handle` 来自仍然存活的 winit 窗口，`ns_view` 指向有效的 `NSView`；上面已确认
        // 当前在主线程，满足 AppKit 的线程要求；`setWantsLayer:` 和 `layer` 的参数、返回类型与
        // AppKit 的声明一致。
        let root: Retained<CALayer> = unsafe {
            let view: &AnyObject = handle.ns_view.cast().as_ref();
            let _: () = msg_send![view, setWantsLayer: Bool::YES];
            let layer: Option<Retained<CALayer>> = msg_send![view, layer];
            layer.ok_or("视图没有 layer")?
        };

        let layer = CALayer::new();
        layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        layer.setGeometryFlipped(true);
        // SAFETY: `kCAGravityTopLeft` 是 QuartzCore 导出的全局常量，在进程生命周期内有效。
        layer.setContentsGravity(unsafe { kCAGravityTopLeft });
        layer.setOpaque(true);
        root.addSublayer(&layer);

        let mut presenter = Self {
            layer,
            surfaces: Vec::new(),
            next: 0,
            width: 0,
            height: 0,
            shown: None,
            idle_since: None,
            spare: None,
            reclaimed: 0,
        };
        let size = window.inner_size();
        presenter.resize(size.width, size.height, window.scale_factor());
        Ok(presenter)
    }

    /// 更新窗口尺寸 `width` × `height`（物理像素）和缩放系数 `scale`。
    ///
    /// 子层与根层对齐；尺寸变化时旧 surface 作废，下次绘制时按新尺寸创建。
    pub(crate) fn resize(&mut self, width: u32, height: u32, scale: f64) {
        // 关闭隐式动画：Core Animation 默认对属性变化加 0.25 秒的过渡，改 frame 时会出现动画。
        CATransaction::begin();
        CATransaction::setDisableActions(true);

        if let Some(root) = self.layer.superlayer() {
            self.layer.setFrame(CGRect {
                origin: CGPoint::new(0.0, 0.0),
                size: root.bounds().size,
            });
        }

        self.layer.setContentsScale(scale);
        CATransaction::commit();

        let size = (width.max(1), height.max(1));

        if size != (self.width, self.height) {
            (self.width, self.height) = size;
            self.surfaces.clear();
            self.next = 0;
            (self.shown, self.spare) = (None, None);
        }
    }

    /// 返回当前画面尺寸（物理像素）。
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 把 `paint` 画进一块空闲的 surface，并把它设为子层的内容。
    ///
    /// 没有空闲 surface 且未达到 [`MAX_SURFACES`] 时新建一块；已达上限时最多等待
    /// [`SURFACE_WAIT`]，仍无空闲则不画，不调用 `paint`，返回 `false`，由调用方在下一次屏幕
    /// 刷新时再画。
    ///
    /// # Panics
    ///
    /// `IOSurfaceCreate` 失败，或基址不满足 `u32` 对齐要求时 panic（正常情况不会发生）。
    pub(crate) fn frame(&mut self, paint: impl FnOnce(Canvas<'_>)) -> bool {
        let Some(index) = self.acquire() else {
            return false;
        };
        self.reclaimed += u32::from(self.surfaces[index].set_purgeable(false));
        let surface = &self.surfaces[index];
        // SAFETY: `surface` 是有效的 IOSurface；`seed` 参数允许传 null；上锁后直到本函数下面
        // 解锁之前，只有交给 `paint` 的画布会写入该 surface。
        let locked = unsafe {
            surface
                .surface
                .lock(IOSurfaceLockOptions::empty(), std::ptr::null_mut())
        };
        /// `kern_return_t` 的成功值（`<mach/kern_return.h>`）。
        const KERN_SUCCESS: i32 = 0;
        // 上锁失败时基址不一定可用，不能往里画。换掉这一块：若留着，每一帧都选到它、每一帧都
        // 画不出来；这一帧不画，下一次屏幕刷新时用新的一块再画。
        if locked != KERN_SUCCESS {
            self.surfaces.remove(index);
            let after = |at: Option<usize>| match at {
                Some(k) if k == index => None,
                Some(k) if k > index => Some(k - 1),
                at => at,
            };
            (self.shown, self.spare) = (after(self.shown), after(self.spare));
            self.next = 0;
            return false;
        }

        let len = surface.stride * self.height as usize;
        let base = surface.surface.base_address().as_ptr().cast::<u32>();
        assert!(base.is_aligned(), "IOSurface 的基址不满足 `u32` 的对齐要求");
        // SAFETY: surface 已上锁，基址在解锁前有效；上面的断言保证基址满足 `u32` 的对齐要求。
        // 长度取自 IOSurface 报告的每行字节数和画面高度，不超出映射范围。`paint` 对任意生命周期
        // 的画布都要能用，存不下这个切片，切片在 `paint` 返回后、解锁之前就不再被使用。
        let px = unsafe { std::slice::from_raw_parts_mut(base, len) };
        paint(Canvas {
            px,
            width: self.width as usize,
            height: self.height as usize,
            stride: surface.stride,
        });

        let surface = &surface.surface;
        // SAFETY: 该 surface 由上面上锁且尚未解锁；`seed` 参数允许传 null。
        unsafe { surface.unlock(IOSurfaceLockOptions::empty(), std::ptr::null_mut()) };

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // 对同一个对象重复设置 `contents` 不会触发刷新，因此先清空再设置。
        // SAFETY: `IOSurfaceRef` 与 Objective-C 的 `IOSurface` 免费桥接，指针有效，且 `surface`
        // 在本函数内保持存活；`CALayer` 接受该对象作为 `contents`。
        unsafe {
            let contents: &AnyObject = &*CFRetained::as_ptr(surface).as_ptr().cast::<AnyObject>();
            self.layer.setContents(None);
            self.layer.setContents(Some(contents));
        }
        CATransaction::commit();
        self.spare = self.shown.filter(|&k| k != index);
        self.shown = Some(index);
        self.idle_since = Some(Instant::now());
        true
    }

    /// 返回这一帧要画的 surface 在 `surfaces` 中的下标；需要时新建一块或等待空闲，等到
    /// [`SURFACE_WAIT`] 后仍全部被占用时返回 `None`。
    fn acquire(&mut self) -> Option<usize> {
        if self.surfaces.iter().all(Surface::in_use) {
            if self.surfaces.len() < MAX_SURFACES {
                self.surfaces.push(Surface::new(self.width, self.height));
            } else {
                self.wait_for_free();
            }
        }

        let count = self.surfaces.len();
        let spare = self
            .spare
            .filter(|&k| k < count && !self.surfaces[k].in_use());
        let mut others = (0..count).map(|k| (self.next + k) % count);
        let index = spare.or_else(|| others.find(|&k| !self.surfaces[k].in_use()))?;
        self.next = (index + 1) % count;
        Some(index)
    }

    /// 等到有 surface 空闲，最多等待 [`SURFACE_WAIT`]。
    fn wait_for_free(&self) {
        let deadline = Instant::now() + SURFACE_WAIT;
        while self.surfaces.iter().all(Surface::in_use) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// 返回上次调用之后取用时内容已被系统收回的 surface 的块数，并清零。
    pub(crate) fn take_reclaimed(&mut self) -> u32 {
        std::mem::take(&mut self.reclaimed)
    }

    /// 设置窗口是否在前台（有焦点且没有被完全遮住）；转入后台时马上把空闲的 surface 标成可清除，
    /// 不等 [`PURGEABLE_IDLE`]。
    pub(crate) fn set_foreground(&mut self, foreground: bool) {
        if !foreground {
            self.idle_since = None;
            self.make_idle_purgeable();
        }
    }

    /// 设置窗口是否完全看不见（被别的窗口整个挡住、最小化、在别的桌面上）。
    ///
    /// 看不见时屏幕上显示着的那块也标成可清除：一块为整个窗口大小（2× 屏上约 16 MB），多个
    /// 窗口同时可见时只有前台窗口的那块需要常驻内存。重新可见时标回不可清除；内容被系统收回
    /// 也没关系，调用方马上重画一帧。
    pub(crate) fn set_occluded(&mut self, occluded: bool) {
        if occluded {
            self.idle_since = None;
            self.make_idle_purgeable();
        }
        if let Some(surface) = self.shown.and_then(|k| self.surfaces.get_mut(k)) {
            surface.set_purgeable(occluded);
        }
    }

    /// 返回应当把空闲的 surface 标成可清除的时刻；都已标好时返回 `None`。
    ///
    /// 动画结束后可能不再绘制新的一帧，所以由调用方按该时刻安排唤醒，到时调用
    /// [`Self::make_idle_purgeable`]，不必等到下一次绘制。
    pub(crate) fn purgeable_at(&self) -> Option<Instant> {
        let pending = |k: usize| Some(k) != self.shown && !self.surfaces[k].purgeable;
        let spare = self
            .spare
            .filter(|&k| k < self.surfaces.len() && pending(k));
        let others = (0..self.surfaces.len()).any(|k| Some(k) != spare && pending(k));
        let wait = match (others, spare) {
            (true, _) => PURGEABLE_IDLE,
            (false, Some(_)) => SPARE_IDLE,
            (false, None) => return None,
        };
        Some(self.idle_since.map_or_else(Instant::now, |t| t + wait))
    }

    /// 把空闲的 surface 标成可清除：备用的一块在停止绘制 [`SPARE_IDLE`] 后，其余的在
    /// [`PURGEABLE_IDLE`] 后；没有记录停止绘制的时刻时全部立即标。
    ///
    /// 合成器尚未释放上一帧的那块先跳过，再过 [`PURGEABLE_IDLE`] 重试。
    pub(crate) fn make_idle_purgeable(&mut self) {
        let now = Instant::now();
        let idle = self.idle_since.map(|t| now.duration_since(t));
        if idle.is_some_and(|idle| idle < PURGEABLE_IDLE) {
            return;
        }
        let (mut retry, mut kept) = (false, false);
        for (k, surface) in self.surfaces.iter_mut().enumerate() {
            if Some(k) == self.shown || surface.purgeable {
                continue;
            }
            if Some(k) == self.spare && idle.is_some_and(|idle| idle < SPARE_IDLE) {
                kept = true;
            } else if surface.in_use() {
                retry = true;
            } else {
                surface.set_purgeable(true);
            }
        }
        if retry {
            self.idle_since = Some(now);
        } else if !kept {
            self.idle_since = None;
        }
    }
}
