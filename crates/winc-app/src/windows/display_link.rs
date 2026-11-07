//! 按屏幕刷新的节奏绘制动画。
//!
//! 桌面窗口管理器（DWM）每次屏幕刷新合成一次画面。后台线程用 `DwmFlush` 等到下一次合成，然后
//! 请求重绘：这一帧在之后的合成中显示，相邻两帧的间隔跟随屏幕刷新。不使用计时器：计时器与刷新
//! 不同步，部分刷新周期绘制两帧，部分周期没有新帧。线程平时等待，有动画时才运行。

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Graphics::Dwm::DwmFlush;

/// 后台线程与界面线程共享的状态。
#[derive(Default)]
struct State {
    /// 是否运行：有动画时为 `true`。
    running: bool,
    /// 是否结束线程。
    stop: bool,
}

/// 显示链接：屏幕每次刷新时唤醒界面；平时暂停，有动画时才运行。
pub struct DisplayLink {
    /// 共享状态和唤醒后台线程的条件变量。
    shared: Arc<(Mutex<State>, Condvar)>,
    /// 是否正在运行（界面线程一侧的记录，避免重复加锁）。
    running: bool,
    /// 后台线程；丢弃时等它结束。
    thread: Option<JoinHandle<()>>,
}

impl DisplayLink {
    /// 创建一个处于暂停状态的显示链接，每次刷新调用 `tick`；`refresh` 为屏幕的刷新间隔。
    ///
    /// 无法启动后台线程时返回 `None`，由调用方按计时器安排动画帧。
    pub fn new(refresh: Duration, tick: impl Fn() + Send + 'static) -> Option<DisplayLink> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let state = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("winc-display-link".into())
            .spawn(move || run(&state, refresh, &tick))
            .ok()?;
        Some(DisplayLink {
            shared,
            running: false,
            thread: Some(thread),
        })
    }

    /// 设置显示链接是否运行：有动画时运行，没有动画时暂停，避免无谓地唤醒界面。
    pub fn set_running(&mut self, running: bool) {
        if running == self.running {
            return;
        }
        self.running = running;
        let (lock, signal) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.running = running;
            signal.notify_one();
        }
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        let (lock, signal) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stop = true;
            signal.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 后台线程：运行时每等到一次合成调用一次 `tick`，暂停时等待。
///
/// 没有待合成的内容时 `DwmFlush` 可能立即返回；两次唤醒的间隔短于半个刷新间隔时，休眠至满
/// 一个刷新间隔，避免空转。
#[allow(unsafe_code)] // DwmFlush
fn run(shared: &(Mutex<State>, Condvar), refresh: Duration, tick: &dyn Fn()) {
    let (lock, signal) = shared;
    let mut last = Instant::now();
    loop {
        {
            let Ok(mut state) = lock.lock() else {
                return;
            };
            while !state.running && !state.stop {
                let Ok(next) = signal.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.stop {
                return;
            }
        }
        // SAFETY: `DwmFlush` 没有参数，只阻塞调用线程直到下一次合成。
        let flushed = unsafe { DwmFlush() } >= 0;
        let since = last.elapsed();
        if !flushed || since < refresh / 2 {
            std::thread::sleep(refresh.saturating_sub(since));
        }
        last = Instant::now();
        tick();
    }
}
