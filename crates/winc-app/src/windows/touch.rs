//! 自绘顶栏的原生命中测试。
//!
//! 顶栏空白处与窗口边框在 `WM_NCHITTEST` 中返回非客户区结果，鼠标和触摸由此进入系统的移动、
//! 缩放与吸附流程；顶栏中的控件仍按客户区接收输入。

// 窗口子类回调与 Win32 调用；回调只在窗口线程执行，共享数据由 `Arc<Mutex<_>>` 持有。
#![allow(unsafe_code)]

use std::sync::{Arc, Mutex};

use winc_render::Rect;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, EnableMenuItem, GCL_STYLE, GetClassLongW, GetClientRect,
    GetMessageExtraInfo, GetSystemMenu, GetSystemMetrics, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
    HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, MF_BYCOMMAND,
    MF_ENABLED, MF_GRAYED, PostMessageW, SC_MAXIMIZE, SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE,
    SM_MENUDROPALIGNMENT, SetClassLongW, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MBUTTONDBLCLK,
    WM_MBUTTONDOWN, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
    WM_NCRBUTTONUP, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_SYSCOMMAND, WM_XBUTTONDBLCLK,
    WM_XBUTTONDOWN,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[cfg(test)]
mod native_tests;

/// 窗口的命中测试区域（系统逻辑像素，客户区坐标）。
///
/// 系统逻辑像素由物理像素除以窗口的 DPI 缩放系数得到，不再除以界面缩放倍数；布局坐标乘以
/// 界面缩放倍数后填入。顶栏中的控件按客户区处理；顶栏其余部分和窗口边框交给系统处理。
#[derive(Default)]
pub struct WindowHitTest {
    /// 顶栏高度（系统逻辑像素），从客户区顶边量起。
    pub caption_height: f32,
    /// 可拖动调整尺寸的边框宽度（系统逻辑像素）；为 0 或窗口最大化时不检测边框。
    pub resize_border: f32,
    /// 相邻控件之间的间距上限（系统逻辑像素），见 [`WindowHitTest::double_click_zooms`]。
    pub control_gap: f32,
    /// 顶栏中按客户区接收输入的控件矩形（系统逻辑像素）。
    pub client: Vec<Rect>,
}

impl WindowHitTest {
    /// 返回点 (`x`, `y`) 的命中测试结果（`HT*` 常量）。
    ///
    /// 坐标与窗口尺寸 `width`、`height` 均为系统逻辑像素；点位于窗口外时返回 `HTCLIENT`，
    /// 即保留系统的默认结果。
    pub fn hit(&self, x: f32, y: f32, width: f32, height: f32, maximized: bool) -> u32 {
        if !Rect::new(0.0, 0.0, width, height).contains(x, y) {
            return HTCLIENT;
        }
        let edge = self.resize_border;
        if edge > 0.0 && !maximized {
            let (left, right) = (x < edge, x >= width - edge);
            let (top, bottom) = (y < edge, y >= height - edge);
            let hit = match (top, bottom, left, right) {
                (true, _, true, _) => HTTOPLEFT,
                (true, _, _, true) => HTTOPRIGHT,
                (_, true, true, _) => HTBOTTOMLEFT,
                (_, true, _, true) => HTBOTTOMRIGHT,
                (true, ..) => HTTOP,
                (_, true, ..) => HTBOTTOM,
                (_, _, true, _) => HTLEFT,
                (_, _, _, true) => HTRIGHT,
                _ => HTCLIENT,
            };
            if hit != HTCLIENT {
                return hit;
            }
        }
        if y < self.caption_height && !self.client.iter().any(|r| r.contains(x, y)) {
            HTCAPTION
        } else {
            HTCLIENT
        }
    }
}

impl WindowHitTest {
    /// 是否由系统在双击顶栏空白处横坐标 `x`（系统逻辑像素）的位置时切换最大化。
    ///
    /// 位于某个控件所在的列时不切换，见 [`in_control_column`](crate::layout::in_control_column)。
    pub fn double_click_zooms(&self, x: f32) -> bool {
        !crate::layout::in_control_column(&self.client, self.control_gap, x)
    }
}

/// 把消息 `lparam` 中的屏幕坐标换算为客户区坐标，返回 `[x, y, 宽, 高]`（系统逻辑像素），
/// 宽、高为客户区的尺寸；换算失败时返回 `None`。
///
/// 每次按窗口当前 DPI 换算（96 DPI 对应缩放系数 1），系统拖动中跨屏、最大化或调整尺寸后
/// 结果仍与布局一致。
fn client_point(hwnd: HWND, lparam: LPARAM) -> Option<[f32; 4]> {
    // `lparam` 的两个屏幕坐标（物理像素）是有符号 16 位值；副屏位于主屏左侧或上方时为负。
    let mut point = POINT {
        x: lparam as i16 as i32,
        y: (lparam >> 16) as i16 as i32,
    };
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `hwnd` 是调用方回调所属的有效窗口；两个输出参数都指向可写的栈变量。
    if unsafe { ScreenToClient(hwnd, &mut point) == 0 || GetClientRect(hwnd, &mut bounds) == 0 } {
        return None;
    }
    // SAFETY: `hwnd` 是调用方回调所属的有效窗口，`GetDpiForWindow` 没有指针参数。
    let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    Some([point.x, point.y, bounds.right, bounds.bottom].map(|v| v as f32 / scale))
}

/// 在屏幕坐标 `lparam` 处弹出系统的窗口菜单，并执行选中的命令。
///
/// 菜单项的可用状态按窗口是否最大化设置：系统只在自行弹出该菜单时更新这些状态。
fn show_window_menu(hwnd: HWND, lparam: LPARAM) {
    // SAFETY: `hwnd` 是调用方回调所属的有效窗口。第二个参数为 0 时返回窗口菜单的句柄而不重置
    // 菜单；句柄为空时不再使用。其余调用只传入该句柄、常量和空指针，弹出菜单期间不持有锁。
    unsafe {
        let menu = GetSystemMenu(hwnd, 0);
        if menu.is_null() {
            return;
        }
        let maximized = IsZoomed(hwnd) != 0;
        let state = |enabled: bool| MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
        EnableMenuItem(menu, SC_RESTORE, state(maximized));
        EnableMenuItem(menu, SC_MOVE, state(!maximized));
        EnableMenuItem(menu, SC_SIZE, state(!maximized));
        EnableMenuItem(menu, SC_MAXIMIZE, state(!maximized));
        EnableMenuItem(menu, SC_MINIMIZE, state(true));
        // 菜单的对齐方向跟随系统设置（从右向左的界面中菜单向左展开）。
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) == 0 {
            TPM_LEFTALIGN
        } else {
            TPM_RIGHTALIGN
        };
        let (x, y) = (lparam as i16 as i32, (lparam >> 16) as i16 as i32);
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | align;
        let command = TrackPopupMenu(menu, flags, x, y, 0, hwnd, std::ptr::null());
        if command != 0 {
            PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
        }
    }
}

/// 窗口命中测试的更新句柄。
///
/// 窗口子类回调持有同一份数据的独立 `Arc` 引用，并在 `WM_NCDESTROY` 中释放，
/// 因此回调不借用应用状态，窗口与句柄的销毁顺序不受限制。
pub struct WindowInput(Arc<Mutex<WindowHitTest>>);

impl WindowInput {
    /// 为 `window` 安装命中测试回调；无法取得 Win32 句柄或安装失败时返回 `None`。
    ///
    /// 必须在创建该窗口的事件循环线程上、窗口首次显示前调用。
    pub fn new(window: &Window) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let state = Arc::new(Mutex::new(WindowHitTest::default()));
        let data = Arc::into_raw(state.clone());
        // SAFETY: 句柄属于当前线程创建且仍存活的 `window`；`data` 是转交给回调的一个 `Arc`
        // 引用，在 `WM_NCDESTROY` 释放之前始终有效。
        if unsafe { SetWindowSubclass(handle.hwnd.get() as _, Some(window_proc), 1, data as usize) }
            == 0
        {
            // SAFETY: 安装失败时回调不会收到 `data`，此处收回 `Arc::into_raw` 转出的唯一引用。
            drop(unsafe { Arc::from_raw(data) });
            return None;
        }
        // winit 注册的窗口类不含 `CS_DBLCLKS`，加上后由系统处理顶栏双击。同类窗口都安装了
        // `window_proc`，客户区双击在回调中还原为第二次按下，winit 收到的消息序列不变。
        // SAFETY: 句柄属于当前线程上存活的窗口；`GCL_STYLE` 只读写该窗口类的样式位。
        unsafe {
            let hwnd = handle.hwnd.get() as _;
            SetClassLongW(
                hwnd,
                GCL_STYLE,
                (GetClassLongW(hwnd, GCL_STYLE) | CS_DBLCLKS) as i32,
            );
        }
        Some(Self(state))
    }

    /// 替换命中区域；锁已中毒时忽略本次更新。
    pub fn update(&self, hit: WindowHitTest) {
        if let Ok(mut state) = self.0.lock() {
            *state = hit;
        }
    }
}

/// 当前消息是否为由触摸或触控笔提升的鼠标事件。
///
/// 系统在这类消息的附加信息中写入固定签名：高 24 位为 `0xFF5157`。
fn promoted_from_touch() -> bool {
    const SIGNATURE_MASK: u32 = 0xFFFF_FF00;
    const PROMOTED: u32 = 0xFF51_5700;
    // SAFETY: `GetMessageExtraInfo` 没有参数，只读取当前线程最近一条消息的附加信息。
    let info = unsafe { GetMessageExtraInfo() } as u32;
    info & SIGNATURE_MASK == PROMOTED
}

/// 窗口子类回调：为客户区补充顶栏命中测试，并把客户区双击还原为按下。
///
/// `data` 是 [`WindowInput::new`] 转出的 `Arc<Mutex<WindowHitTest>>` 引用。
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let caption = wparam == HTCAPTION as usize;
    // SAFETY: `WM_NCDESTROY` 之前窗口始终持有该 `Arc` 引用，指针有效。
    let state = unsafe { &*(data as *const Mutex<WindowHitTest>) };
    // 顶栏空白处的双击位于控件所在的列时还原为按下：仍可拖动窗口，不切换最大化。持锁期间不调用
    // 可能重入回调的 Win32 函数。
    let zooms = |x: f32| state.lock().is_ok_and(|hit| hit.double_click_zooms(x));
    let beside_control = message == WM_NCLBUTTONDBLCLK
        && caption
        && client_point(hwnd, lparam).is_some_and(|[x, ..]| !zooms(x));
    let message = if beside_control {
        WM_NCLBUTTONDOWN
    } else {
        message
    };
    if message == WM_NCRBUTTONUP && caption {
        show_window_menu(hwnd, lparam);
        return 0;
    }
    // SAFETY: `hwnd` 是当前回调所属的有效窗口，`IsZoomed` 没有指针参数。
    let maximized = || unsafe { IsZoomed(hwnd) } != 0;
    if message == WM_NCLBUTTONDOWN && caption && (promoted_from_touch() || maximized()) {
        // winit 处理此消息时补发一个客户区坐标为 (0, 0) 的鼠标移动，用于消除拖动开始时的停顿。
        // 以下两种情况中系统把该移动当作真实位移，因此直接交给系统默认处理，保留原始输入序列；
        // 其余鼠标消息仍沿子类链处理。
        // - 由触摸提升的鼠标事件：点按也可能触发吸附。
        // - 窗口已最大化：客户区原点与按下点的距离超过系统的拖动阈值，单击即还原窗口。
        // SAFETY: 参数原样来自系统传给当前回调的消息，`hwnd` 是当前回调所属的有效窗口。
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if message == WM_NCDESTROY {
        // SAFETY: `data` 是安装时转交给该窗口的唯一 `Arc` 引用。先移除回调，此后不再访问
        // `data`；每个窗口只收到一次 `WM_NCDESTROY`，因此引用只释放一次。
        unsafe {
            RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Arc::from_raw(data as *const Mutex<WindowHitTest>));
        }
    } else if message == WM_NCHITTEST {
        // 系统已判定为标题栏、边框等非客户区时保留原结果，只对客户区补充顶栏命中测试。
        // SAFETY: 参数原样来自当前回调收到的消息，`hwnd` 是当前回调所属的有效窗口。
        let original = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if original != HTCLIENT as isize {
            return original;
        }
        let Some([x, y, width, height]) = client_point(hwnd, lparam) else {
            return original;
        };
        // 持锁期间不调用可能重入回调的 Win32 函数。
        if let Ok(hit) = state.lock() {
            return hit.hit(x, y, width, height, maximized()) as isize;
        }
        return original;
    }
    // 窗口类启用了 `CS_DBLCLKS`；客户区双击还原为第二次按下，由 winit 自行识别双击。
    let message = match message {
        WM_LBUTTONDBLCLK => WM_LBUTTONDOWN,
        WM_MBUTTONDBLCLK => WM_MBUTTONDOWN,
        WM_RBUTTONDBLCLK => WM_RBUTTONDOWN,
        WM_XBUTTONDBLCLK => WM_XBUTTONDOWN,
        message => message,
    };
    // 其余消息沿子类链传递。`WM_NCPOINTERDOWN` 由系统提升为非客户区鼠标消息后进入系统的
    // 移动与缩放循环，吸附预览、布局、还原尺寸和取消均由系统决定。
    // SAFETY: 参数来自系统传给当前回调的消息；双击改写为按下后 `wparam` 与 `lparam` 的格式不变。
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
