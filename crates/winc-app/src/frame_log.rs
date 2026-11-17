//! 帧日志：设置环境变量 `WINC_FRAME_LOG` 后，每隔 [`REPORT_EVERY`] 向标准错误输出一行绘制统计。
//!
//! 用于排查卡顿和掉帧：在终端中启动 winc，复现问题后附上输出。只统计实际绘制的帧；画面静止、
//! 不绘制时不输出。
//!
//! 另记录鼠标移动到画面更新的间隔、查询光标下的窗口的耗时、跳过的帧数和被系统收回的呈现缓冲数。

use std::time::{Duration, Instant};

/// 两次输出之间的时长。
///
/// 取自调参：足以收集上百帧作统计，卡顿发生后 5 s 内即有输出。
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// 帧日志的状态；未启用时不记录。
pub struct FrameLog {
    /// 是否启用（设置了 `WINC_FRAME_LOG`）。
    enabled: bool,
    /// 本轮统计中第一帧的时刻。
    since: Option<Instant>,
    /// 本轮各帧的绘制与呈现耗时。
    times: Vec<Duration>,
    /// 本轮耗时超过一次屏幕刷新间隔的帧数；这些帧必然错过一次刷新。
    slow: usize,
    /// 尚未反映到画面上的最早一次鼠标移动的时刻。
    moved: Option<Instant>,
    /// 本轮各次鼠标移动到其后第一帧画完的间隔。
    waits: Vec<Duration>,
    /// 本轮查询光标下的窗口的次数和累计耗时。
    queries: std::cell::Cell<(u32, Duration)>,
    /// 本轮因呈现缓冲都被占用而没有画出的帧数。
    skipped: usize,
    /// 本轮取用时内容已被系统收回的呈现缓冲的块数。
    reclaimed: u32,
}

impl FrameLog {
    /// 按环境变量 `WINC_FRAME_LOG` 创建帧日志；变量未设置时不记录。
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var_os("WINC_FRAME_LOG").is_some(),
            since: None,
            times: Vec::new(),
            slow: 0,
            moved: None,
            waits: Vec::new(),
            queries: std::cell::Cell::new((0, Duration::ZERO)),
            skipped: 0,
            reclaimed: 0,
        }
    }

    /// 记录一次鼠标移动；已有尚未反映到画面上的移动时保留更早的时刻。
    pub fn pointer_moved(&mut self) {
        if self.enabled {
            self.moved.get_or_insert_with(Instant::now);
        }
    }

    /// 执行查询光标下的窗口的 `query`，启用时记录其耗时。
    #[cfg_attr(
        not(any(target_os = "macos", test)),
        allow(dead_code, reason = "仅 macOS 使用")
    )]
    pub fn time_query<T>(&self, query: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return query();
        }
        let started = Instant::now();
        let result = query();
        let (count, total) = self.queries.get();
        self.queries.set((count + 1, total + started.elapsed()));
        result
    }

    /// 记录一帧因呈现缓冲都被占用而没有画出。
    pub fn frame_skipped(&mut self) {
        self.skipped += usize::from(self.enabled);
    }

    /// 是否启用。
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 记录一帧：`elapsed` 为绘制与呈现的耗时，`refresh` 为屏幕的刷新间隔，`size` 为画面的物理
    /// 像素尺寸。距本轮第一帧满 [`REPORT_EVERY`] 时输出本轮统计并开始下一轮。
    ///
    /// `reclaimed` 是这一帧取用时内容已被系统收回的呈现缓冲的块数。
    pub fn record(
        &mut self,
        elapsed: Duration,
        refresh: Duration,
        size: (u32, u32),
        reclaimed: u32,
    ) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        self.reclaimed += reclaimed;
        if let Some(moved) = self.moved.take() {
            self.waits.push(now.duration_since(moved));
        }
        let since = *self.since.get_or_insert(now);
        self.times.push(elapsed);
        self.slow += usize::from(elapsed > refresh);
        if now.duration_since(since) < REPORT_EVERY {
            return;
        }

        self.times.sort_unstable();
        self.waits.sort_unstable();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        // 第 `q` 分位的值（毫秒）；没有记录时为 0。
        let at = |times: &[Duration], q: f64| {
            let index = (times.len().saturating_sub(1) as f64 * q) as usize;
            times.get(index).map_or(0.0, |&d| ms(d))
        };
        let (queries, query_time) = self.queries.take();
        eprintln!(
            "winc frame-log: frames={} draw_p50_ms={:.2} draw_p95_ms={:.2} draw_max_ms={:.2} \
             slow_frames={} refresh_ms={:.2} size={}x{} pointer_moves={} \
             pointer_to_frame_p50_ms={:.2} pointer_to_frame_p95_ms={:.2} \
             pointer_to_frame_max_ms={:.2} window_queries={} window_query_total_ms={:.2} \
             skipped_frames={} reclaimed_surfaces={}",
            self.times.len(),
            at(&self.times, 0.5),
            at(&self.times, 0.95),
            at(&self.times, 1.0),
            self.slow,
            ms(refresh),
            size.0,
            size.1,
            self.waits.len(),
            at(&self.waits, 0.5),
            at(&self.waits, 0.95),
            at(&self.waits, 1.0),
            queries,
            ms(query_time),
            self.skipped,
            self.reclaimed,
        );
        self.since = None;
        self.times.clear();
        self.waits.clear();
        (self.slow, self.skipped, self.reclaimed) = (0, 0, 0);
    }
}

#[cfg(test)]
mod tests;
