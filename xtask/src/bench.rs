//! 基准（`cargo xtask bench`）：构建 release，测启动时间、空载内存和滚动帧耗时，对照预算。

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::samples::{samples, samples_dir};
use crate::{build_release, root};

/// 从外部启动进程到呈现文件首帧的启动预算（毫秒）。
///
/// 取自调参：把各样本文件的实测启动中位数向上整到 200 ms，超出即视为启动变慢。
/// 作用范围是 `bench` 对启动中位数的判定；样本或构建方式变化后需重新标定。
const BUDGET_STARTUP_MS: f64 = 200.0;
/// 空载物理内存预算（MB）。
///
/// 取自调参：各样本文件空载时的物理内存为 40–48 MB（M4 Pro，2× 缩放，120 Hz 显示器）。
/// 作用范围是 `bench` 对空载物理内存的判定；样本或缓存策略变化后需重新标定。
const BUDGET_IDLE_MB: f64 = 50.0;
/// 滚动单帧耗时的 p95 预算（毫秒）。
///
/// 取 120 Hz 下一帧间隔（8.3 ms）的四分之一，其余时间留给同一帧内的输入处理与合成器。
/// 各样本文件实测的 p95 为 1.0–1.6 ms（M4 Pro）。
const BUDGET_SCROLL_P95_MS: f64 = 2.0;
/// 一次按键从处理到出帧的 p95 耗时预算（毫秒），用于 Vim 模式的按键与编辑区的键入。
///
/// 取 120 Hz 下一帧间隔（8.3 ms）的一半：按键的处理与随后一帧的绘制在同一个刷新周期内完成。
const BUDGET_KEY_P95_MS: f64 = 4.0;

/// Markdown 预览滚动单帧耗时的 p95 预算（毫秒）。
///
/// 预览按比例字体排版，每帧绘制的文字与图形多于等宽的代码区，预算高于 [`BUDGET_SCROLL_P95_MS`]。
/// 本仓库的 `README.md` 实测 p95 为 1.6–1.9 ms（M4 Pro）。
const BUDGET_PREVIEW_P95_MS: f64 = 3.0;

/// 跑一遍全部基准，返回所有指标是否都在预算内。
///
/// # Errors
///
/// 构建 release、生成样本或启动被测程序失败时返回错误描述。
pub fn bench(runs: usize) -> Result<bool, String> {
    build_release(&[])?;

    samples().map_err(|e| e.to_string())?;

    let exe = root()
        .join("target/release")
        .join(if cfg!(windows) { "winc.exe" } else { "winc" });
    let dir = samples_dir();
    let large = dir.join("large.rs");
    let mut ok = true;

    println!();
    println!("{:<34} {:>10} {:>10}  结果", "指标", "实测", "预算");

    let mut startup = Vec::with_capacity(runs);
    for _ in 0..runs {
        startup.push(startup_once(&exe, &large)?);
        std::thread::sleep(Duration::from_millis(200));
    }
    startup.sort_by(f64::total_cmp);
    let median = startup[startup.len() / 2];
    ok &= report(
        &format!("启动中位数（{runs} 次，large.rs）"),
        median,
        BUDGET_STARTUP_MS,
        "ms",
    );

    // 大文件：启动时先建行索引，高亮从可见区域附近开始推算。
    for name in ["huge.txt", "huge.rs"] {
        let path = dir.join(name);
        let mut times: Vec<f64> = (0..5)
            .map(|_| startup_once(&exe, &path))
            .collect::<Result<_, _>>()?;
        times.sort_by(f64::total_cmp);
        ok &= report(
            &format!("启动中位数（5 次，{name}）"),
            times[2],
            BUDGET_STARTUP_MS,
            "ms",
        );
    }

    let (p50, p95) = scroll_once(&exe, &large, "scroll")?;
    println!("{:<34} {:>8.2}ms", "滚动 p50（large.rs）", p50);
    ok &= report("滚动 p95（large.rs）", p95, BUDGET_SCROLL_P95_MS, "ms");

    for name in ["huge.txt", "huge.rs"] {
        let (_, p95) = scroll_once(&exe, &dir.join(name), "scroll")?;
        ok &= report(
            &format!("滚动 p95（{name}）"),
            p95,
            BUDGET_SCROLL_P95_MS,
            "ms",
        );
    }

    // Vim 模式：按键处理加随后一帧的耗时；键入的耗时用于核对关闭 Vim 模式时没有额外开销。
    let (p50, p95) = scroll_once(&exe, &large, "vim")?;
    println!("{:<34} {:>8.2}ms", "Vim 按键到出帧 p50（large.rs）", p50);
    ok &= report(
        "Vim 按键到出帧 p95（large.rs）",
        p95,
        BUDGET_KEY_P95_MS,
        "ms",
    );
    let (p50, p95) = scroll_once(&exe, &large, "typing")?;
    println!("{:<34} {:>8.2}ms", "键入到出帧 p50（large.rs）", p50);
    ok &= report("键入到出帧 p95（large.rs）", p95, BUDGET_KEY_P95_MS, "ms");

    let (_, p95) = scroll_once(&exe, &root().join("README.md"), "preview")?;
    ok &= report(
        "预览滚动 p95（README.md）",
        p95,
        BUDGET_PREVIEW_P95_MS,
        "ms",
    );

    // 快速转动鼠标滚轮：帧间隔、画面落后于滚轮的格数、最后一格之后画面停止的耗时。只列出，
    // 不对照预算。
    let wheel = wheel_once(&exe, &large)?;
    for (name, key, unit) in [
        (
            "滚轮帧间隔 p50（large.rs，仅供参考）",
            "interval_p50_ms",
            "ms",
        ),
        (
            "滚轮帧间隔 p95（large.rs，仅供参考）",
            "interval_p95_ms",
            "ms",
        ),
        (
            "滚轮落后 p95（large.rs，仅供参考）",
            "lag_p95_notches",
            "格",
        ),
        (
            "滚轮落后最大（large.rs，仅供参考）",
            "lag_max_notches",
            "格",
        ),
        ("滚轮停下后到位（large.rs，仅供参考）", "settle_ms", "ms"),
    ] {
        let value = field(&wheel, key).unwrap_or(f64::NAN);
        println!("{name:<34} {value:>8.2}{unit}");
    }

    // 分帧完成的任务：大文件查找（每帧一个时间片）、跳到末尾后推算高亮。只列出，不对照预算。
    for (name, mode, file) in [
        (
            "查找不存在的词（huge.txt，仅供参考）",
            "find:zqxjwv",
            "huge.txt",
        ),
        (
            "跳到末尾推算高亮（large.rs，仅供参考）",
            "highlight",
            "large.rs",
        ),
    ] {
        let (ms, frames) = progress_once(&exe, &dir.join(file), mode)?;
        println!("{name:<34} {ms:>8.1}ms  {frames:.0} 帧");
    }

    for name in ["large.rs", "minified.js", "huge.txt", "huge.rs"] {
        if let Some((footprint, rss)) = memory_once(&exe, &dir.join(name), "idle")? {
            ok &= report(
                &format!("空载物理内存（{name}）"),
                footprint,
                BUDGET_IDLE_MB,
                "MB",
            );
            println!("{:<34} {:>8.0}MB", format!("RSS（{name}，仅供参考）"), rss);
        }
    }

    // 打开本仓库后在全部文件中搜索、建立 ⌘P 的文件表：仓库会变大，只列出，不对照预算。
    for (name, mode) in [
        ("搜索「fn」后物理内存（本仓库，仅供参考）", "search:fn"),
        ("建好 ⌘P 文件表后物理内存（本仓库，仅供参考）", "picker"),
    ] {
        if let Some((footprint, _)) = memory_once(&exe, &root(), mode)? {
            println!("{name:<34} {footprint:>8.1}MB");
        }
    }

    println!();
    println!(
        "{}",
        if ok {
            "全部达标"
        } else {
            "有指标超出预算"
        }
    );
    Ok(ok)
}

fn report(name: &str, value: f64, budget: f64, unit: &str) -> bool {
    let pass = value <= budget;
    println!(
        "{name:<34} {value:>8.1}{unit} {budget:>8.0}{unit}  {}",
        if pass { "✓" } else { "✗ 超出" }
    );
    pass
}

fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// 返回从 winc 的输出里取到的 `WINC <key>=<value>` 中的数值。
fn field(output: &str, key: &str) -> Option<f64> {
    output
        .split_whitespace()
        .find_map(|w| w.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// 从外部启动一次 `exe`，返回进程启动到首帧呈现的毫秒数。
///
/// # Errors
///
/// 进程启动失败，或输出里没有首帧时间戳 `first_present_epoch_ms` 时返回错误描述。
fn startup_once(exe: &Path, file: &Path) -> Result<f64, String> {
    let started = epoch_ms();
    let output = Command::new(exe)
        .arg(file)
        .env("WINC_BENCH", "startup")
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let present = field(&stdout, "first_present_epoch_ms").ok_or("winc 没有输出首帧时间戳")?;
    Ok(present - started)
}

/// 启动后连续滚动，返回（单帧耗时的 p50，毫秒；p95，毫秒）。
///
/// `mode` 是测量模式：`scroll` 滚动文本，`preview` 先切换到只显示预览再滚动。
///
/// # Errors
///
/// 进程启动失败，或输出里缺少滚动耗时与帧率时返回错误描述。
fn scroll_once(exe: &Path, file: &Path, mode: &str) -> Result<(f64, f64), String> {
    let output = Command::new(exe)
        .arg(file)
        .env("WINC_BENCH", mode)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let p50 = field(&stdout, "p50_ms").ok_or("winc 没有输出滚动耗时")?;
    let p95 = field(&stdout, "p95_ms").ok_or("winc 没有输出滚动耗时")?;
    Ok((p50, p95))
}

/// 以滚轮测量模式打开 `file`，返回 winc 的输出（含 `WINC wheel_frames=…` 一行）。
///
/// # Errors
///
/// 进程启动失败，或输出里缺少滚轮测量的结果时返回错误描述。
fn wheel_once(exe: &Path, file: &Path) -> Result<String, String> {
    let output = Command::new(exe)
        .arg(file)
        .env("WINC_BENCH", "wheel")
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    field(&stdout, "wheel_frames").ok_or("winc 没有输出滚轮测量结果")?;
    Ok(stdout)
}

/// 以测量模式 `mode`（`find:…`、`highlight`）打开 `file`，返回（做完用的毫秒数，帧数）。
///
/// # Errors
///
/// 进程启动失败，或输出里缺少耗时或帧数时返回错误描述。
fn progress_once(exe: &Path, file: &Path, mode: &str) -> Result<(f64, f64), String> {
    let output = Command::new(exe)
        .arg(file)
        .env("WINC_BENCH", mode)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ms = field(&stdout, "progress_ms").ok_or("winc 没有输出耗时")?;
    let frames = field(&stdout, "progress_frames").ok_or("winc 没有输出帧数")?;
    Ok((ms, frames))
}

/// 以测量模式 `mode`（即传给 `WINC_BENCH` 的值）打开 `path`，等 2.5 秒后采样。
///
/// 返回（物理内存 MB，RSS MB）；不在 macOS 上时返回 `None`。
///
/// # Errors
///
/// 进程启动失败，或 `footprint`、`ps` 的输出无法解析时返回错误描述。
fn memory_once(exe: &Path, path: &Path, mode: &str) -> Result<Option<(f64, f64)>, String> {
    if !cfg!(target_os = "macos") {
        return Ok(None);
    }

    let mut child = Command::new(exe)
        .arg(path)
        .env("WINC_BENCH", mode)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(2500));

    let pid = child.id().to_string();
    let footprint = Command::new("footprint")
        .args(["-p", &pid])
        .output()
        .map_err(|e| e.to_string())?;
    let rss = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .map_err(|e| e.to_string())?;
    let _ = child.kill();
    let _ = child.wait();

    let footprint = parse_footprint(&String::from_utf8_lossy(&footprint.stdout))
        .ok_or("无法解析 footprint 输出")?;
    let rss_kb: f64 = String::from_utf8_lossy(&rss.stdout)
        .trim()
        .parse()
        .map_err(|_| "无法解析 ps 输出")?;
    Ok(Some((footprint, rss_kb / 1024.0)))
}

/// 解析 `footprint` 的一行输出，把内存量换算为 MB。
///
/// 输入形如 `winc [123]: 64-bit    Footprint: 49 MB (16384 bytes per page)`。
fn parse_footprint(output: &str) -> Option<f64> {
    let rest = output.split("Footprint:").nth(1)?.trim_start();
    let mut parts = rest.split_whitespace();
    let value: f64 = parts.next()?.parse().ok()?;
    match parts.next()? {
        "KB" => Some(value / 1024.0),
        "MB" => Some(value),
        "GB" => Some(value * 1024.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
