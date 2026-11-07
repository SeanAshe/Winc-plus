//! Wayland 上凭激活令牌把已有窗口切到前台（xdg-activation-v1）。
//!
//! 合成器只接受附带激活令牌的切换请求。令牌由新启动的 winc 随目标交来（见
//! `crate::instance::launch_token`）；`winit` 只在新建窗口时使用令牌，已有窗口由本模块激活。
//! 本模块接入 `winit` 的 `wl_display`，在自己的事件队列上绑定 `xdg_activation_v1`。

// 范围：本模块；每个 `unsafe` 块都有 `// SAFETY:` 说明。
#![allow(unsafe_code)] // 接入 `winit` 的 Wayland 连接和窗口表面（libwayland 指针）

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::{ActivationToken, Window};

/// 首次激活时建立的对象；合成器不提供 `xdg_activation_v1` 时为 `None`。
///
/// 存放在静态变量中，永不丢弃：丢弃时会释放事件队列，而进程退出时 `wl_display` 可能已关闭。
static ACTIVATION: OnceLock<Option<Activation>> = OnceLock::new();

/// 接入 `winit` 连接后绑定的 `xdg_activation_v1`。
struct Activation {
    /// 与 `winit` 共用 `wl_display` 的连接，不拥有该 `wl_display`。
    connection: Connection,
    /// 合成器的 `xdg_activation_v1` 全局对象。
    manager: XdgActivationV1,
    /// 本模块的事件队列，只收到注册表的事件，每次激活时清空。
    queue: Mutex<EventQueue<State>>,
}

/// 本模块事件队列的分发状态；事件均无需处理。
struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: xdg_activation_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// 凭激活令牌 `token` 请合成器把 `window` 切到前台，返回请求是否已发出。
///
/// `window` 不是 Wayland 窗口、合成器不支持 xdg-activation 或连接出错时返回 `false`。合成器
/// 可以拒绝请求（例如令牌已过期）。
pub fn activate(window: &Window, token: ActivationToken) -> bool {
    let (Ok(display), Ok(handle)) = (window.display_handle(), window.window_handle()) else {
        return false;
    };
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(handle)) =
        (display.as_raw(), handle.as_raw())
    else {
        return false;
    };
    let activation = ACTIVATION.get_or_init(|| Activation::bind(display.display));
    activation
        .as_ref()
        .is_some_and(|activation| activation.activate(handle.surface, token))
}

impl Activation {
    /// 接入 `display` 并绑定 `xdg_activation_v1`；合成器不提供该对象或连接出错时返回 `None`。
    fn bind(display: NonNull<c_void>) -> Option<Activation> {
        // SAFETY: `display` 取自存活的窗口，在事件循环存续期间有效；该连接不拥有
        // `wl_display`，只在有存活窗口时使用，且永不丢弃。
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&connection).ok()?;
        let manager = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        Some(Activation {
            connection,
            manager,
            queue: Mutex::new(queue),
        })
    }

    /// 以令牌 `token` 激活表面 `surface`，返回请求是否已发出。
    fn activate(&self, surface: NonNull<c_void>, token: ActivationToken) -> bool {
        // SAFETY: `surface` 是存活窗口的 `wl_surface`，属于同一个 `wl_display`，在本函数内有效；
        // 接口不符时返回错误。
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) };
        let Some(surface) = id
            .ok()
            .and_then(|id| WlSurface::from_id(&self.connection, id).ok())
        else {
            return false;
        };
        self.manager.activate(token.into_raw(), &surface);
        if let Ok(mut queue) = self.queue.lock() {
            let _ = queue.dispatch_pending(&mut State);
        }
        self.connection.flush().is_ok()
    }
}
