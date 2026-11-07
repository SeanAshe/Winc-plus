//! 帧日志：设置环境变量 `WINC_FRAME_LOG` 后，每隔 [`REPORT_EVERY`] 向标准错误输出一行绘制统计。
//!
//! 用于排查卡顿和掉帧：在终端中启动 winc，复现问题后附上输出。只统计实际绘制的帧；画面静止、
//! 不绘制时不输出。

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
}

impl FrameLog {
    /// 按环境变量 `WINC_FRAME_LOG` 创建帧日志；变量未设置时不记录。
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var_os("WINC_FRAME_LOG").is_some(),
            since: None,
            times: Vec::new(),
            slow: 0,
        }
    }

    /// 是否启用。
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 记录一帧：`elapsed` 为绘制与呈现的耗时，`refresh` 为屏幕的刷新间隔，`size` 为画面的物理
    /// 像素尺寸。距本轮第一帧满 [`REPORT_EVERY`] 时输出本轮统计并开始下一轮。
    pub fn record(&mut self, elapsed: Duration, refresh: Duration, size: (u32, u32)) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let since = *self.since.get_or_insert(now);
        self.times.push(elapsed);
        self.slow += usize::from(elapsed > refresh);
        if now.duration_since(since) < REPORT_EVERY {
            return;
        }

        self.times.sort_unstable();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let pick = |q: f64| ms(self.times[((self.times.len() - 1) as f64 * q) as usize]);
        eprintln!(
            "winc frame-log: frames={} draw_p50_ms={:.2} draw_p95_ms={:.2} draw_max_ms={:.2} \
             slow_frames={} refresh_ms={:.2} size={}x{}",
            self.times.len(),
            pick(0.5),
            pick(0.95),
            pick(1.0),
            self.slow,
            ms(refresh),
            size.0,
            size.1,
        );
        self.since = None;
        self.times.clear();
        self.slow = 0;
    }
}

#[cfg(test)]
mod tests;
