//! 测量钩子：设置环境变量 `WINC_BENCH` 后启用，供 `cargo xtask bench` 使用。
//!
//! 环境变量的值决定测量模式：`startup` 在首帧呈现后退出；`scroll`、`preview`、`terminal:<命令>`
//! 分别在滚动文本、滚动预览、终端输出期间统计帧耗时；`wheel` 按固定间隔送出鼠标滚轮滚动并统计
//! 帧间隔和画面的跟随情况；`search:<内容>`、`picker` 完成后保持运行；`find:<内容>`、`highlight`
//! 输出耗时、帧数和 CPU 时间后退出；其他值只输出时间戳并保持运行。保持运行的模式由外部进程
//! 采样内存；未设置时不测量。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 滚动测量采集的帧数（帧）。
///
/// 无外部出处与硬性约束，取自调参：按 60 Hz 刷新率计，该帧数对应约 5 秒的滚动时长，
/// 使 p50、p95 与最大耗时的统计覆盖数百帧。采集满该帧数后输出统计结果并退出。
const SCROLL_FRAMES: usize = 300;

/// 滚轮测量送出的格数（格）。
///
/// 取自调参：按 [`WHEEL_INTERVAL`] 送出约需 1.5 秒，使帧间隔和画面落后的统计覆盖上百帧。
const WHEEL_NOTCHES: usize = 60;

/// 滚轮测量中相邻两格的间隔。
///
/// 短于滚轮过渡的最短时长（50 ms）：各格的过渡必然重叠，测得的是快速转动滚轮时的跟随情况。
const WHEEL_INTERVAL: Duration = Duration::from_millis(25);

/// 滚轮测量在最后一格之后等待画面停止滚动的最长时长；超过后仍然输出统计，不计停止的耗时。
///
/// 远长于滚轮过渡的最长时长（125 ms），只在画面始终未停止时起作用。
const WHEEL_SETTLE_LIMIT: Duration = Duration::from_secs(2);

/// 测量模式，由环境变量 `WINC_BENCH` 决定。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// 不测量。
    Off,
    /// 输出启动时间戳后保持运行，供外部进程采样内存。
    Idle,
    /// 首帧呈现后退出，用于测量启动耗时。
    Startup,
    /// 连续滚动并统计每帧耗时，采集 [`SCROLL_FRAMES`] 帧后退出。
    Scroll,
    /// 首帧呈现后切换到只显示预览（Markdown、SVG），之后与 [`Mode::Scroll`] 相同。
    Preview,
    /// 首帧呈现后打开终端面板并运行给定的命令，统计此后 [`SCROLL_FRAMES`] 帧的耗时后退出。
    ///
    /// 帧由终端的输出触发：命令必须持续输出，否则采集不满，进程不会退出。
    Terminal(String),
    /// 按 [`WHEEL_INTERVAL`] 送出 [`WHEEL_NOTCHES`] 格鼠标滚轮滚动，画面停止后输出统计并退出。
    Wheel,
    /// 首帧呈现后在打开的目录里搜索给定的内容，之后保持运行。
    Search(String),
    /// 首帧呈现后打开快速打开面板，之后保持运行。
    QuickOpen,
    /// 首帧呈现后在打开的文件里查找给定的内容，搜到末尾后退出。
    Find(String),
    /// 首帧呈现后跳到文件末尾，高亮状态推算完成后退出。
    Highlight,
}

/// 每帧呈现之后要执行的动作。
pub enum Next {
    /// 不执行动作，继续正常运行。
    Continue,
    /// 滚动测量：调用方向下滚动一段并请求下一帧。
    Scroll,
    /// 把当前标签切换到只显示预览。
    Preview,
    /// 打开终端面板。
    Terminal,
    /// 测量完成，退出事件循环。
    Exit,
    /// 在打开的目录里搜索给定的内容。
    Search(String),
    /// 打开快速打开面板。
    QuickOpen,
    /// 在打开的文件里查找给定的内容。
    Find(String),
    /// 跳到文件末尾。
    Highlight,
}

/// 测量钩子的状态。
pub struct Bench {
    mode: Mode,
    /// 滚动测量中已采集的每帧耗时；只在 [`Mode::Scroll`] 下使用。
    frame_times: Vec<Duration>,
    /// 查找、高亮测量开始的时刻，当时进程已用的 CPU 时间，以及此后绘制的帧数。
    progress: Option<(Instant, Duration, usize)>,
    /// 滚轮测量的进度；只在 [`Mode::Wheel`] 下、首帧呈现之后存在。
    wheel: Option<WheelRun>,
    /// 终端测量中尚未写给终端的命令，见 [`Bench::terminal_command`]。
    command: Option<String>,
}

/// 滚轮测量的进度。
struct WheelRun {
    /// 下一格的送出时刻。
    next: Instant,
    /// 已送出的格数。
    sent: usize,
    /// 已送出的各格合计应滚动的行数。
    target: f64,
    /// 一格滚动的行数，用于把落后的行数换算为格数。
    notch: f32,
    /// 最后一格的送出时刻。
    last: Option<Instant>,
    /// 送出第一格之后各次呈现的时刻，以及当时画面落后的行数。
    frames: Vec<(Instant, f32)>,
}

impl Bench {
    /// 创建一个不测量的状态，供后续新建的窗口使用。
    ///
    /// 测量只针对启动时的窗口。
    pub fn off() -> Self {
        Self {
            mode: Mode::Off,
            frame_times: Vec::new(),
            progress: None,
            wheel: None,
            command: None,
        }
    }

    /// 按环境变量 `WINC_BENCH` 创建测量状态；变量未设置时不测量。
    pub fn from_env() -> Self {
        let mode = match std::env::var("WINC_BENCH").as_deref() {
            Err(_) => Mode::Off,
            Ok("startup") => Mode::Startup,
            Ok("scroll") => Mode::Scroll,
            Ok("preview") => Mode::Preview,
            Ok("wheel") => Mode::Wheel,
            Ok("picker") => Mode::QuickOpen,
            Ok("highlight") => Mode::Highlight,
            Ok(value) => match (value.strip_prefix("search:"), value.strip_prefix("find:")) {
                (Some(query), _) => Mode::Search(query.to_owned()),
                (_, Some(query)) => Mode::Find(query.to_owned()),
                _ => match value.strip_prefix("terminal:") {
                    Some(command) => Mode::Terminal(command.to_owned()),
                    None => Mode::Idle,
                },
            },
        };

        Self {
            mode,
            frame_times: Vec::new(),
            progress: None,
            wheel: None,
            command: None,
        }
    }

    /// 是否启用测量模式。
    ///
    /// 测量模式下窗口静默：不激活应用、不出现在程序坞、窗口完全透明且鼠标点击穿透。窗口仍然
    /// 上屏并呈现，因此测得的绘制与呈现耗时与正常运行时相同，仅是不可见、不响应点击。
    pub fn silent(&self) -> bool {
        self.mode != Mode::Off
    }

    /// 输出进程入口的时间戳（Unix 纪元起的毫秒数），由 `main` 的入口处调用；不测量时不输出。
    pub fn on_start(&self) {
        if self.mode != Mode::Off {
            println!("WINC main_entry_epoch_ms={:.3}", epoch_ms());
        }
    }

    /// 更新呈现计数与帧耗时，返回下一帧要执行的动作。
    ///
    /// 每次呈现之后调用。
    ///
    /// `presented` 是包括当前帧在内的累计呈现次数，从 1 起；第 1 帧绘制在尚未显示的窗口上，
    /// 因此以第 2 帧作为首次呈现，滚动测量也从第 3 帧起采样。`elapsed` 是当前帧的绘制与呈现耗时。
    pub fn on_present(&mut self, presented: usize, elapsed: Duration) -> Next {
        if self.mode == Mode::Off {
            return Next::Continue;
        }

        if presented == 2 {
            println!("WINC first_present_epoch_ms={:.3}", epoch_ms());

            match &self.mode {
                Mode::Startup => return Next::Exit,
                Mode::Search(query) => return Next::Search(query.clone()),
                Mode::QuickOpen => return Next::QuickOpen,
                Mode::Find(query) => {
                    self.progress = Some((Instant::now(), cpu_time(), 0));
                    return Next::Find(query.clone());
                }
                Mode::Highlight => {
                    self.progress = Some((Instant::now(), cpu_time(), 0));
                    return Next::Highlight;
                }
                Mode::Wheel => {
                    self.wheel = Some(WheelRun {
                        next: Instant::now(),
                        sent: 0,
                        target: 0.0,
                        notch: 0.0,
                        last: None,
                        frames: Vec::new(),
                    });
                }
                Mode::Preview => return Next::Preview,
                Mode::Terminal(command) => {
                    self.command = Some(command.clone());
                    return Next::Terminal;
                }
                Mode::Off | Mode::Idle | Mode::Scroll => {}
            }
        }

        let terminal = matches!(self.mode, Mode::Terminal(_));
        let timed = terminal || matches!(self.mode, Mode::Scroll | Mode::Preview);
        // 终端测量只统计命令写入终端之后的帧。
        if !timed || presented < 2 || self.command.is_some() {
            return Next::Continue;
        }

        if presented > 2 {
            self.frame_times.push(elapsed);
        }

        if self.frame_times.len() < SCROLL_FRAMES {
            return if terminal {
                Next::Continue
            } else {
                Next::Scroll
            };
        }

        self.frame_times.sort_unstable();
        let pick = |q: f64| {
            self.frame_times[((self.frame_times.len() - 1) as f64 * q) as usize].as_secs_f64()
                * 1000.0
        };
        println!(
            "WINC scroll_frames={} p50_ms={:.2} p95_ms={:.2} max_ms={:.2}",
            self.frame_times.len(),
            pick(0.5),
            pick(0.95),
            pick(1.0)
        );
        Next::Exit
    }
}

impl Bench {
    /// 返回终端测量中待写入终端的命令：终端可以写入（`ready`）后返回一次，其余情况返回 `None`。
    pub fn terminal_command(&mut self, ready: bool) -> Option<String> {
        self.command.take_if(|_| ready)
    }

    /// 返回滚轮测量中是否已到送出下一格的时刻，以及之后需要唤醒的时刻；不在滚轮测量中或已
    /// 全部送出时返回 `(false, None)`。
    pub fn wheel_due(&self, now: Instant) -> (bool, Option<Instant>) {
        match &self.wheel {
            Some(run) if run.sent < WHEEL_NOTCHES => {
                let send = now >= run.next;
                let next = if send { now + WHEEL_INTERVAL } else { run.next };
                (send, Some(next))
            }
            _ => (false, None),
        }
    }

    /// 记录已送出一格滚轮滚动，该格应滚动 `lines` 行。
    pub fn wheel_sent(&mut self, lines: f32) {
        if let Some(run) = &mut self.wheel {
            let now = Instant::now();
            run.next = run.next.max(now) + WHEEL_INTERVAL;
            run.sent += 1;
            run.target += f64::from(lines);
            run.notch = lines;
            run.last = Some(now);
        }
    }

    /// 滚轮测量中记录一次呈现，`scroll` 是画面滚动到的行；返回测量是否已完成。
    ///
    /// 送完全部格数且画面停在目标位置时，或最后一格之后超过 [`WHEEL_SETTLE_LIMIT`] 时，输出
    /// 帧间隔的 p50、p95，画面落后格数的 p95 和最大值，以及最后一格之后画面停止的耗时。
    pub fn on_wheel_frame(&mut self, scroll: f64) -> bool {
        let Some(run) = &mut self.wheel else {
            return false;
        };
        if run.sent == 0 {
            return false;
        }
        let now = Instant::now();
        let behind = (run.target - scroll).max(0.0) as f32;
        run.frames.push((now, behind));
        let Some(last) = run.last.filter(|_| run.sent == WHEEL_NOTCHES) else {
            return false;
        };
        let settled = behind < 0.01;
        if !settled && now < last + WHEEL_SETTLE_LIMIT {
            return false;
        }

        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mut intervals: Vec<f64> = (run.frames.windows(2))
            .map(|w| ms(w[1].0 - w[0].0))
            .collect();
        intervals.sort_by(f64::total_cmp);
        let notch = f64::from(run.notch.max(f32::EPSILON));
        let mut lags: Vec<f64> = (run.frames.iter())
            .map(|f| f64::from(f.1) / notch)
            .collect();
        lags.sort_by(f64::total_cmp);
        let pick = |v: &[f64], q: f64| v.get(((v.len() - 1) as f64 * q) as usize).copied();
        println!(
            "WINC wheel_frames={} interval_p50_ms={:.2} interval_p95_ms={:.2} \
             lag_p95_notches={:.2} lag_max_notches={:.2} settle_ms={:.1}",
            run.frames.len(),
            pick(&intervals, 0.5).unwrap_or(0.0),
            pick(&intervals, 0.95).unwrap_or(0.0),
            pick(&lags, 0.95).unwrap_or(0.0),
            pick(&lags, 1.0).unwrap_or(0.0),
            if settled { ms(now - last) } else { f64::NAN },
        );
        true
    }

    /// 记录一帧进度，返回测量是否已完成。
    ///
    /// 查找、高亮测量中每绘制完一帧调用；`done` 表示目标动作是否完成。完成时输出从开始到当前的
    /// 耗时、累计帧数和期间进程消耗的 CPU 时间，返回 `true`，调用方应退出事件循环。
    pub fn on_progress(&mut self, done: bool) -> bool {
        let Some((started, cpu, frames)) = &mut self.progress else {
            return false;
        };
        *frames += 1;
        // 开始测量的一帧中刚发出查找或跳转，`done` 仍是上一帧的结果，不作为完成的依据。
        if *frames == 1 || !done {
            return false;
        }
        println!(
            "WINC progress_ms={:.1} progress_frames={} progress_cpu_ms={:.1}",
            started.elapsed().as_secs_f64() * 1000.0,
            frames,
            cpu_time().saturating_sub(*cpu).as_secs_f64() * 1000.0
        );
        true
    }
}

/// 返回进程启动以来消耗的 CPU 时间（用户态加内核态，包含所有线程）。
#[allow(unsafe_code)] // getrusage
fn cpu_time() -> Duration {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `getrusage` 只写入传入的结构体。
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            // SAFETY: `getrusage` 返回 0 表示调用成功，此时结构体已完整写入。
            let usage = unsafe { usage.assume_init() };
            let time = |t: libc::timeval| {
                Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
            };
            return time(usage.ru_utime) + time(usage.ru_stime);
        }
    }
    Duration::ZERO
}

/// 返回当前时间距 Unix 纪元的毫秒数；系统时钟早于纪元时返回 0。
fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}
